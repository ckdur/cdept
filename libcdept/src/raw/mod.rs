//! SPICE raw files: the output of ngspice (`write`, `-r`) and Xyce (`-r`), binary and
//! ascii, with several plots in one file (a run's analyses are appended to `out.raw`),
//! real and complex vectors.
//!
//! [`RawFile::open`] maps the file and reads only the headers (and, for ascii, the
//! numbers: ascii files are small); a binary plot's values stay in the file until
//! [`RawFile::vector`] copies one column out, so a 100 MB file costs the size of the
//! vectors that are asked for. A file that ends early (a simulator that died) gives the
//! points that are there.
//!
//! Names are matched without regard to case (Xyce prints `OUT`, ngspice `v(out)`).
//!
//! [`RawFile::open`] also reads CSV tables ([`csv`]) and digital waveforms (VCD, FST, GHW,
//! [`digital`]) into the same plots and vectors, so the calculator and the listing of a
//! file's analyses work on them unchanged.

mod csv;
pub(crate) mod digital;

use std::path::Path;
use std::sync::Arc;

use eyre::{Result, WrapErr, bail, eyre};

use crate::calc::{C, Samples, Waveform};

/// One variable (column) of a plot.
#[derive(Debug, Clone, PartialEq)]
pub struct Var {
    pub name: String,
    /// `voltage`, `current`, `time`, `frequency` (anything the simulator wrote, e.g.
    /// `frequency grid=3`).
    pub unit: String,
}

/// Header of one plot of a raw file.
#[derive(Debug, Clone, PartialEq)]
pub struct Plot {
    pub title: String,
    pub date: String,
    /// `Transient Analysis`, `AC Analysis`, … as the simulator wrote it.
    pub name: String,
    pub complex: bool,
    pub vars: Vec<Var>,
    /// The points that are in the file.
    pub points: usize,
    /// What the simulator announced (more than `points` when the file is cut).
    pub declared_points: usize,
    layout: Layout,
}

#[derive(Debug, Clone, PartialEq)]
enum Layout {
    /// Binary: values start at this byte offset, row after row.
    Binary(usize),
    /// Ascii, parsed: `vars × points` rows of `(re, im)` pairs (im is 0 for real plots).
    Ascii(Vec<f64>),
    /// A digital file: each variable is loaded on demand with its own time steps.
    Digital(Arc<digital::Source>),
}

impl Plot {
    /// The analysis kind: `tran`, `ac`, `dc`, `noise`, `op`, or `other`.
    pub fn kind(&self) -> &'static str {
        let n = self.name.to_lowercase();
        if n.contains("transient") || n.starts_with("tran") {
            "tran"
        } else if n.contains("ac ") || n.starts_with("ac") {
            "ac"
        } else if n.contains("noise") {
            "noise"
        } else if n.contains("operating") || n.starts_with("op") {
            "op"
        } else if n.contains("dc") {
            "dc"
        } else {
            "other"
        }
    }

    /// Index of a variable, ignoring case. A digital or table signal is also found by
    /// the end of its hierarchical name (`clk` for `tb.dut.clk`) when only one ends so.
    pub fn var_index(&self, name: &str) -> Option<usize> {
        let lower = name.to_lowercase();
        if let Some(i) = self
            .vars
            .iter()
            .position(|v| v.name.to_lowercase() == lower)
        {
            return Some(i);
        }
        if lower.is_empty() || !matches!(self.layout, Layout::Digital(_)) {
            return None;
        }
        let tail = format!(".{lower}");
        let mut found = self
            .vars
            .iter()
            .enumerate()
            .skip(1)
            .filter(|(_, v)| v.name.to_lowercase().ends_with(&tail));
        match (found.next(), found.next()) {
            (Some((i, _)), None) => Some(i),
            _ => None,
        }
    }

    /// The file is a digital waveform (VCD, FST, GHW).
    pub fn is_digital(&self) -> bool {
        matches!(self.layout, Layout::Digital(_))
    }

    /// Does this plot answer to `selector`: its index (`0`), its kind (`tran`), or a
    /// part of its name (`Transient`)?
    pub fn matches(&self, index: usize, selector: &str) -> bool {
        let s = selector.trim().to_lowercase();
        s == index.to_string()
            || s == self.kind()
            || (!s.is_empty() && self.name.to_lowercase().contains(&s))
    }
}

enum Bytes {
    Mapped(memmap2::Mmap),
    Owned(Vec<u8>),
}

impl Bytes {
    fn get(&self) -> &[u8] {
        match self {
            Bytes::Mapped(m) => m,
            Bytes::Owned(v) => v,
        }
    }
}

/// An open raw file.
pub struct RawFile {
    bytes: Bytes,
    plots: Vec<Plot>,
}

impl std::fmt::Debug for RawFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RawFile")
            .field("plots", &self.plots)
            .finish()
    }
}

impl RawFile {
    /// Open a waveform file: a SPICE raw file (ngspice, Xyce), a CSV table (`.csv`,
    /// `.tsv`, `.txt` and the like) or a digital waveform (VCD, FST, GHW, told by content).
    pub fn open(file: &Path) -> Result<RawFile> {
        let ext = file
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if matches!(ext.as_str(), "csv" | "tsv" | "txt" | "dat" | "tab") {
            let bytes =
                std::fs::read(file).wrap_err_with(|| format!("cannot read {}", file.display()))?;
            return Self::from_table(bytes)
                .wrap_err_with(|| format!("{} is not a waveform table", file.display()));
        }
        if matches!(ext.as_str(), "vcd" | "fst" | "ghw") || !Self::looks_like_raw(file) {
            if let Some(format) = digital::detect(file) {
                let plot = digital::open(file, format)
                    .wrap_err_with(|| format!("cannot read {} ({format})", file.display()))?;
                return Ok(RawFile {
                    bytes: Bytes::Owned(Vec::new()),
                    plots: vec![plot],
                });
            }
            if matches!(ext.as_str(), "vcd" | "fst" | "ghw") {
                bail!("{} is not a {ext} file", file.display());
            }
        }
        Self::open_raw(file)
    }

    /// The first bytes read like the header of a SPICE raw file.
    fn looks_like_raw(file: &Path) -> bool {
        use std::io::Read;
        let mut head = [0u8; 64];
        let n = std::fs::File::open(file)
            .and_then(|mut f| f.read(&mut head))
            .unwrap_or(0);
        let text = String::from_utf8_lossy(&head[..n]).to_lowercase();
        text.starts_with("title:") || text.starts_with("plotname:") || text.starts_with("date:")
    }

    /// A CSV/TSV table held in memory (see the module [`csv`]).
    pub fn from_table(bytes: Vec<u8>) -> Result<RawFile> {
        let plots = csv::parse(&bytes)?;
        Ok(RawFile {
            bytes: Bytes::Owned(Vec::new()),
            plots,
        })
    }

    /// Map `file` (a SPICE raw file) and read its plot headers.
    fn open_raw(file: &Path) -> Result<RawFile> {
        let f = std::fs::File::open(file)
            .wrap_err_with(|| format!("cannot open {}", file.display()))?;
        // SAFETY: the map is read only; a simulator that rewrites the file under us is
        // the caller's race (the runner reads after the simulator has ended).
        let map = unsafe { memmap2::Mmap::map(&f) }
            .wrap_err_with(|| format!("cannot map {}", file.display()))?;
        Self::from_bytes_inner(Bytes::Mapped(map))
            .wrap_err_with(|| format!("{} is not a raw file", file.display()))
    }

    /// A raw file held in memory.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<RawFile> {
        Self::from_bytes_inner(Bytes::Owned(bytes))
    }

    fn from_bytes_inner(bytes: Bytes) -> Result<RawFile> {
        let plots = parse_plots(bytes.get())?;
        Ok(RawFile { bytes, plots })
    }

    pub fn plots(&self) -> &[Plot] {
        &self.plots
    }

    /// The plot a selector names (see [`Plot::matches`]); the first of several.
    pub fn find_plot(&self, selector: &str) -> Option<usize> {
        self.plots
            .iter()
            .enumerate()
            .find(|(i, p)| p.matches(*i, selector))
            .map(|(i, _)| i)
    }

    /// Copy one variable of plot `plot` out as a waveform against the plot's first
    /// variable (time, frequency, the swept source).
    pub fn vector(&self, plot: usize, name: &str) -> Result<Waveform> {
        let p = self
            .plots
            .get(plot)
            .ok_or_else(|| eyre!("no plot {plot}"))?;
        let col = p
            .var_index(name)
            .ok_or_else(|| eyre!("no vector '{name}' in plot '{}'", p.name))?;
        if let Layout::Digital(source) = &p.layout {
            return source.vector(p, col);
        }
        let x: Vec<f64> = self.column(plot, 0)?.into_iter().map(|c| c.re).collect();
        let y = self.column(plot, col)?;
        let samples = if p.complex {
            Samples::Complex(y)
        } else {
            Samples::Real(y.into_iter().map(|c| c.re).collect())
        };
        Ok(Waveform {
            x: std::sync::Arc::new(x),
            y: samples,
            x_name: p.vars[0].name.clone(),
            x_unit: p.vars[0].unit.clone(),
            name: p.vars[col].name.clone(),
            unit: p.vars[col].unit.clone(),
        })
    }

    fn column(&self, plot: usize, col: usize) -> Result<Vec<C>> {
        let p = &self.plots[plot];
        let nv = p.vars.len();
        let mut out = Vec::with_capacity(p.points);
        match &p.layout {
            Layout::Digital(_) => return Err(eyre!("a digital signal has no common time column")),
            Layout::Ascii(rows) => {
                for i in 0..p.points {
                    let k = (i * nv + col) * 2;
                    out.push(C::new(rows[k], rows[k + 1]));
                }
            }
            Layout::Binary(start) => {
                let data = self.bytes.get();
                let width = if p.complex { 16 } else { 8 };
                let stride = nv * width;
                for i in 0..p.points {
                    let at = start + i * stride + col * width;
                    let re = f64::from_le_bytes(data[at..at + 8].try_into().unwrap());
                    let im = if p.complex {
                        f64::from_le_bytes(data[at + 8..at + 16].try_into().unwrap())
                    } else {
                        0.0
                    };
                    out.push(C::new(re, im));
                }
            }
        }
        Ok(out)
    }

    /// Every vector of a plot by name.
    pub fn vector_names(&self, plot: usize) -> Vec<&str> {
        self.plots[plot]
            .vars
            .iter()
            .map(|v| v.name.as_str())
            .collect()
    }
}

fn parse_plots(data: &[u8]) -> Result<Vec<Plot>> {
    let mut plots = vec![];
    let mut pos = 0;
    loop {
        // Skip blank space between plots.
        while pos < data.len() && data[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if pos >= data.len() {
            break;
        }
        let (plot, next) =
            parse_plot(data, pos).wrap_err_with(|| format!("plot {} (byte {pos})", plots.len()))?;
        plots.push(plot);
        pos = next;
    }
    if plots.is_empty() {
        bail!("no plots");
    }
    Ok(plots)
}

fn line_at(data: &[u8], pos: usize) -> (String, usize) {
    let end = memchr::memchr(b'\n', &data[pos..]).map_or(data.len(), |i| pos + i);
    let line = String::from_utf8_lossy(&data[pos..end])
        .trim_end_matches('\r')
        .to_string();
    (line, (end + 1).min(data.len()))
}

fn parse_plot(data: &[u8], mut pos: usize) -> Result<(Plot, usize)> {
    let mut plot = Plot {
        title: String::new(),
        date: String::new(),
        name: String::new(),
        complex: false,
        vars: vec![],
        points: 0,
        declared_points: 0,
        layout: Layout::Binary(0),
    };
    let (mut nvars, mut npoints) = (0usize, 0usize);
    let mut seen_header = false;
    loop {
        if pos >= data.len() {
            bail!("the header ends the file");
        }
        let (line, next) = line_at(data, pos);
        pos = next;
        let (key, value) = match line.split_once(':') {
            Some((k, v)) => (k.trim().to_lowercase(), v.trim()),
            None => (line.trim().to_lowercase(), ""),
        };
        match key.as_str() {
            "title" => plot.title = value.to_string(),
            "date" => plot.date = value.to_string(),
            "plotname" => plot.name = value.to_string(),
            "flags" => {
                plot.complex = value
                    .to_lowercase()
                    .split_whitespace()
                    .any(|f| f == "complex")
            }
            "no. variables" => nvars = value.parse().wrap_err("bad 'No. Variables'")?,
            "no. points" => npoints = value.parse().wrap_err("bad 'No. Points'")?,
            "command" | "option" | "dimensions" | "offset" => {}
            "variables" => {
                for _ in 0..nvars {
                    if pos >= data.len() {
                        bail!("the variable list ends the file");
                    }
                    let (l, n) = line_at(data, pos);
                    pos = n;
                    let mut it = l.split_whitespace();
                    let _index = it.next();
                    let name = it
                        .next()
                        .ok_or_else(|| eyre!("bad variable line '{l}'"))?
                        .to_string();
                    let unit = it.collect::<Vec<_>>().join(" ");
                    plot.vars.push(Var { name, unit });
                }
            }
            "binary" => {
                if nvars == 0 || plot.vars.len() != nvars {
                    bail!("binary data before the variables");
                }
                let width = nvars * if plot.complex { 16 } else { 8 };
                let available = (data.len() - pos) / width;
                plot.declared_points = npoints;
                plot.points = npoints.min(available);
                plot.layout = Layout::Binary(pos);
                // A file cut short has fewer points; the next plot cannot be told, so stop.
                let end = if plot.points < npoints {
                    data.len()
                } else {
                    pos + plot.points * width
                };
                return Ok((plot, end));
            }
            "values" => {
                if nvars == 0 || plot.vars.len() != nvars {
                    bail!("values before the variables");
                }
                // The counts are the file's word only: reserve no more than its bytes
                // can hold (two numbers per value need at least two bytes each).
                let mut rows = Vec::with_capacity(
                    npoints
                        .saturating_mul(nvars)
                        .saturating_mul(2)
                        .min(data.len() - pos),
                );
                let mut points = 0;
                'points: for _ in 0..npoints {
                    // Point index, then one value per variable.
                    let Some((_, p)) = token(data, pos) else {
                        break;
                    };
                    pos = p;
                    for _ in 0..nvars {
                        let Some((tok, p)) = token(data, pos) else {
                            break 'points;
                        };
                        pos = p;
                        let (re, im) = match tok.split_once(',') {
                            Some((a, b)) => (a, b),
                            None => (tok, "0"),
                        };
                        rows.push(
                            re.parse::<f64>()
                                .wrap_err_with(|| format!("bad number '{tok}'"))?,
                        );
                        rows.push(
                            im.parse::<f64>()
                                .wrap_err_with(|| format!("bad number '{tok}'"))?,
                        );
                    }
                    points += 1;
                }
                plot.declared_points = npoints;
                plot.points = points;
                plot.layout = Layout::Ascii(rows);
                return Ok((plot, pos));
            }
            _ => {
                if !seen_header && key.is_empty() {
                    continue;
                }
                bail!("unexpected header line '{line}'");
            }
        }
        seen_header = true;
    }
}

/// The next whitespace-separated token at or after `pos`, and where it ends.
fn token(data: &[u8], mut pos: usize) -> Option<(&str, usize)> {
    while pos < data.len() && data[pos].is_ascii_whitespace() {
        pos += 1;
    }
    if pos >= data.len() {
        return None;
    }
    let start = pos;
    while pos < data.len() && !data[pos].is_ascii_whitespace() {
        pos += 1;
    }
    std::str::from_utf8(&data[start..pos])
        .ok()
        .map(|s| (s, pos))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A binary plot of real columns (first column x).
    pub fn binary_plot(
        name: &str,
        vars: &[(&str, &str)],
        cols: &[Vec<f64>],
        complex: bool,
    ) -> Vec<u8> {
        let n = cols[0].len();
        let mut out = format!(
            "Title: test\nDate: today\nPlotname: {name}\nFlags: {}\nNo. Variables: {}\nNo. Points: {n}\nVariables:\n",
            if complex { "complex" } else { "real" },
            vars.len()
        )
        .into_bytes();
        for (i, (v, u)) in vars.iter().enumerate() {
            out.extend(format!("\t{i}\t{v}\t{u}\n").bytes());
        }
        out.extend(b"Binary:\n");
        for i in 0..n {
            for c in cols {
                out.extend(c[i].to_le_bytes());
                if complex {
                    out.extend(0f64.to_le_bytes());
                }
            }
        }
        out
    }

    #[test]
    fn reads_binary_multi_plot() {
        let mut bytes = binary_plot(
            "Transient Analysis",
            &[("time", "time"), ("v(out)", "voltage")],
            &[vec![0.0, 1.0, 2.0], vec![5.0, 6.0, 7.0]],
            false,
        );
        bytes.extend(binary_plot(
            "AC Analysis",
            &[("frequency", "frequency"), ("V(OUT)", "voltage")],
            &[vec![1.0, 10.0], vec![1.0, 0.5]],
            true,
        ));
        let raw = RawFile::from_bytes(bytes).unwrap();
        assert_eq!(raw.plots().len(), 2);
        assert_eq!(raw.plots()[0].kind(), "tran");
        assert_eq!(raw.plots()[1].kind(), "ac");
        assert_eq!(raw.find_plot("ac"), Some(1));
        assert_eq!(raw.find_plot("1"), Some(1));
        let w = raw.vector(0, "V(out)").unwrap();
        assert_eq!(*w.x, vec![0.0, 1.0, 2.0]);
        assert_eq!(w.y.re(), vec![5.0, 6.0, 7.0]);
        let ac = raw.vector(1, "v(out)").unwrap();
        assert!(matches!(ac.y, Samples::Complex(_)));
        assert!(raw.vector(0, "nothing").is_err());
    }

    #[test]
    fn reads_ascii_complex_and_truncated_binary() {
        let ascii = "Title: rc\nPlotname: AC Analysis\nFlags: complex\nNo. Variables: 2\nNo. Points: 2\nVariables:\n\t0\tfrequency\tfrequency grid=3\n\t1\tv(out)\tvoltage\nValues:\n 0\t1.0e+00,0.0e+00\n\t0.5,-0.5\n\n 1\t10,0\n\t0.1,-0.2\n";
        let raw = RawFile::from_bytes(ascii.as_bytes().to_vec()).unwrap();
        let w = raw.vector(0, "v(out)").unwrap();
        assert_eq!(*w.x, vec![1.0, 10.0]);
        let Samples::Complex(y) = &w.y else { panic!() };
        assert_eq!((y[0].re, y[0].im, y[1].im), (0.5, -0.5, -0.2));
        assert_eq!(raw.plots()[0].vars[0].unit, "frequency grid=3");

        let mut bytes = binary_plot(
            "Transient Analysis",
            &[("time", "time"), ("v(a)", "voltage")],
            &[vec![0.0, 1.0, 2.0], vec![1.0, 2.0, 3.0]],
            false,
        );
        bytes.truncate(bytes.len() - 24 - 8);
        let raw = RawFile::from_bytes(bytes).unwrap();
        assert_eq!(
            (raw.plots()[0].points, raw.plots()[0].declared_points),
            (1, 3)
        );
    }

    #[test]
    fn rejects_garbage() {
        assert!(RawFile::from_bytes(b"hello world\n".to_vec()).is_err());
        assert!(RawFile::from_bytes(vec![]).is_err());
    }

    /// Headers that lie about their counts (the file that once took 51 GB in the
    /// viewer), truncated files and binary noise: errors or few points, never a huge
    /// allocation, a panic or a hang.
    #[test]
    fn hostile_headers_and_noise() {
        let head = |vars: &str, points: &str, list: &str, mode: &str| {
            format!(
                "Title: t\nPlotname: Transient Analysis\nFlags: real\nNo. Variables: {vars}\nNo. Points: {points}\nVariables:\n{list}{mode}\n"
            )
            .into_bytes()
        };
        let two = "\t0\ttime\ttime\n\t1\tv(out)\tvoltage\n";
        let noise: Vec<u8> = (0..1000u32).map(|i| (i * 37) as u8).collect();
        let with = |mut h: Vec<u8>, tail: &[u8]| {
            h.extend_from_slice(tail);
            h
        };
        // more variables announced than listed: the list runs into the data
        assert!(
            RawFile::from_bytes(with(
                head("99999999", "18446744073709551615", "", "Binary:"),
                &noise
            ))
            .is_err()
        );
        // the points are cut to what the bytes hold (1000 / 16)
        let raw = RawFile::from_bytes(with(
            head("2", "18446744073709551615", two, "Binary:"),
            &noise,
        ))
        .unwrap();
        assert_eq!(raw.plots()[0].points, 62);
        assert_eq!(raw.vector(0, "v(out)").unwrap().len(), 62);
        // ascii claiming 2^64 points: no capacity overflow, the points that are there
        let raw = RawFile::from_bytes(with(
            head("2", "18446744073709551615", two, "Values:"),
            b" 0\t1\n\t2\n 1\t3\n",
        ))
        .unwrap();
        assert_eq!(raw.plots()[0].points, 1);
        for bad in ["-5", "1e3", "two", ""] {
            assert!(RawFile::from_bytes(with(head("2", bad, two, "Binary:"), &noise)).is_err());
        }
        assert!(RawFile::from_bytes(with(head("0", "5", "", "Binary:"), &noise)).is_err());
        // every prefix of a good file: an error or a plot, never a panic
        let good = binary_plot(
            "Transient Analysis",
            &[("time", "time"), ("v(a)", "voltage")],
            &[vec![0.0, 1.0, 2.0], vec![1.0, 2.0, 3.0]],
            false,
        );
        for n in 0..good.len() {
            if let Ok(raw) = RawFile::from_bytes(good[..n].to_vec()) {
                let _ = raw.vector(0, "v(a)");
            }
        }
        // binary noise of many shapes
        let mut seed = 12345u64;
        for _ in 0..200 {
            let len = (seed % 4000) as usize;
            let bytes: Vec<u8> = (0..len)
                .map(|_| {
                    seed = seed
                        .wrapping_mul(6364136223846793005)
                        .wrapping_add(1442695040888963407);
                    (seed >> 33) as u8
                })
                .collect();
            let _ = RawFile::from_bytes(bytes);
        }
    }

    /// Xyce's own raw file (names in upper case, `V1#branch`), when there is a Xyce.
    #[test]
    fn real_xyce_raw_reads_like_ngspice() {
        use crate::sim::run::{SimKind, Simulator, run};
        let Some(sim) = Simulator::find_kind(SimKind::Xyce) else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let deck = "* rc\nV1 in 0 PULSE(0 1 1n 1n 1n 5n 20n)\nR1 in out 1k\nC1 out 0 1p\n.tran 0.1n 20n\n.print tran v(*)\n.end\n";
        let out = run(&sim, deck, dir.path(), &[]).unwrap();
        let raw = RawFile::open(&out.raw).unwrap();
        assert_eq!(raw.plots().len(), 1);
        assert_eq!(raw.plots()[0].kind(), "tran");
        let env = crate::calc::Env::new(&raw);
        let v = |src: &str| crate::calc::evaluate(src, &env).unwrap().as_f64().unwrap();
        // 50 % delay of an RC (1 ns) behind a 1 ns edge: the .meas of the run says 0.735 ns.
        let d = v("delay(v(\"in\"), v(\"out\"), 0.5)");
        assert!((d - 7.35e-10).abs() < 3e-11, "{d}");
        assert!(
            v("min(i(\"v1\"))") < -1e-5,
            "the source current is found as V1#branch"
        );
    }
}
