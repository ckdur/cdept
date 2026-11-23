//! Reader for the raw files VACASK (and ngspice, Xyce) write: `Title:`, `Plotname:`,
//! `Flags: real|complex`, a variable list and `Binary:` (8-byte floats, one record per
//! point) or `Values:` (text). A file may hold several plots.

use std::path::Path;

use eyre::{Result, WrapErr, bail, eyre};

/// One plot of a raw file.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Plot {
    pub title: String,
    pub plotname: String,
    pub complex: bool,
    /// Variable names, the sweep variable (`time`, `frequency`, …) first.
    pub names: Vec<String>,
    /// `data[var][point]`: the real part.
    pub data: Vec<Vec<f64>>,
    /// The imaginary part of a complex plot, same shape as `data`.
    pub imag: Vec<Vec<f64>>,
}

impl Plot {
    pub fn points(&self) -> usize {
        self.data.first().map_or(0, |d| d.len())
    }

    /// The index of a variable. A SPICE vector name (`v(out)`, `i(vdd)`) finds the
    /// variable VACASK calls `out`, `vdd:flow(br)`; case does not matter.
    pub fn index(&self, name: &str) -> Option<usize> {
        let want = name.trim().to_lowercase();
        let inner = |p: &str| want.strip_prefix(p).and_then(|r| r.strip_suffix(')')).map(str::to_string);
        let mut candidates = vec![want.clone()];
        if let Some(n) = inner("v(") {
            candidates.push(n);
        }
        if let Some(n) = inner("i(") {
            candidates.push(format!("{n}:flow(br)"));
        }
        candidates.iter().find_map(|c| self.names.iter().position(|n| n.to_lowercase() == *c))
    }

    /// The real part of a variable.
    pub fn get(&self, name: &str) -> Option<&[f64]> {
        self.index(name).map(|i| self.data[i].as_slice())
    }

    /// The sweep variable (first column).
    pub fn x(&self) -> &[f64] {
        self.data.first().map_or(&[], |d| d.as_slice())
    }
}

/// Every plot in `bytes`.
pub fn parse(bytes: &[u8]) -> Result<Vec<Plot>> {
    let mut plots = vec![];
    let mut pos = 0;
    while pos < bytes.len() {
        // Skip blank space between plots.
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if pos >= bytes.len() {
            break;
        }
        let mut plot = Plot::default();
        let (mut nvars, mut npoints) = (0usize, 0usize);
        loop {
            let end = bytes[pos..].iter().position(|b| *b == b'\n').map(|e| pos + e).unwrap_or(bytes.len());
            let line = String::from_utf8_lossy(&bytes[pos..end]).into_owned();
            pos = (end + 1).min(bytes.len());
            let (key, rest) = line.split_once(':').map(|(k, r)| (k.trim().to_lowercase(), r.trim())).unwrap_or_default();
            match key.as_str() {
                "title" => plot.title = rest.into(),
                "plotname" => plot.plotname = rest.into(),
                "flags" => plot.complex = rest.to_lowercase().contains("complex"),
                "no. variables" => nvars = rest.parse().wrap_err("bad 'No. Variables'")?,
                "no. points" => npoints = rest.parse().wrap_err("bad 'No. Points'")?,
                "variables" => {
                    for _ in 0..nvars {
                        let end = bytes[pos..].iter().position(|b| *b == b'\n').map(|e| pos + e).unwrap_or(bytes.len());
                        let l = String::from_utf8_lossy(&bytes[pos..end]).into_owned();
                        pos = (end + 1).min(bytes.len());
                        let mut it = l.split_whitespace();
                        let _index = it.next();
                        plot.names.push(it.next().ok_or_else(|| eyre!("bad variable line '{l}'"))?.to_string());
                    }
                }
                "binary" => {
                    let width = if plot.complex { 16 } else { 8 };
                    let need = nvars * npoints * width;
                    if bytes.len() < pos + need {
                        bail!("raw file is short: {} bytes of data, {} expected", bytes.len() - pos, need);
                    }
                    plot.data = vec![Vec::with_capacity(npoints); nvars];
                    plot.imag = if plot.complex { vec![Vec::with_capacity(npoints); nvars] } else { vec![] };
                    let f = |at: usize| f64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
                    let mut at = pos;
                    for _ in 0..npoints {
                        for v in 0..nvars {
                            plot.data[v].push(f(at));
                            at += 8;
                            if plot.complex {
                                plot.imag[v].push(f(at));
                                at += 8;
                            }
                        }
                    }
                    pos = at;
                    break;
                }
                "values" => {
                    plot.data = vec![Vec::with_capacity(npoints); nvars];
                    plot.imag = if plot.complex { vec![Vec::with_capacity(npoints); nvars] } else { vec![] };
                    let text = String::from_utf8_lossy(&bytes[pos..]).into_owned();
                    let mut consumed = 0;
                    let mut nums = text.lines().map(|l| (l.len() + 1, l)).peekable();
                    for _ in 0..npoints {
                        for v in 0..nvars {
                            let (len, l) = nums.next().ok_or_else(|| eyre!("ascii raw file is short"))?;
                            consumed += len;
                            // The first value of a point starts with the point number.
                            let l = if v == 0 { l.split_whitespace().nth(1).unwrap_or("") } else { l.trim() };
                            if plot.complex {
                                let (re, im) = l.split_once(',').ok_or_else(|| eyre!("bad complex value '{l}'"))?;
                                plot.data[v].push(re.trim().parse()?);
                                plot.imag[v].push(im.trim().parse()?);
                            } else {
                                plot.data[v].push(l.parse()?);
                            }
                        }
                    }
                    pos = (pos + consumed).min(bytes.len());
                    break;
                }
                _ => {
                    if pos >= bytes.len() {
                        bail!("raw file ends before its data");
                    }
                }
            }
        }
        plots.push(plot);
    }
    if plots.is_empty() {
        bail!("no plot in the raw file");
    }
    Ok(plots)
}

/// A noise plot of VACASK (`onoise`: output power spectral density in V^2/Hz, `gain`:
/// power gain from the input source) with ngspice's two spectra added, which the test
/// templates read: `onoise_spectrum` = sqrt(onoise) and `inoise_spectrum` =
/// sqrt(onoise / gain), both in V/sqrt(Hz). A plot that is not VACASK's noise result,
/// or has them already, is returned as it is.
pub fn with_noise_spectra(mut plot: Plot) -> Plot {
    let (Some(o), Some(g)) = (plot.index("onoise"), plot.index("gain")) else { return plot };
    if plot.complex || plot.index("onoise_spectrum").is_some() {
        return plot;
    }
    let out: Vec<f64> = plot.data[o].iter().map(|p| p.max(0.0).sqrt()).collect();
    let inp: Vec<f64> = plot.data[o].iter().zip(&plot.data[g]).map(|(n, g)| if *g > 0.0 { (n / g).max(0.0).sqrt() } else { f64::NAN }).collect();
    for (name, v) in [("onoise_spectrum", out), ("inoise_spectrum", inp)] {
        plot.names.push(name.to_string());
        plot.data.push(v);
    }
    plot
}

/// A real plot as a binary raw file (what [`parse`] reads and ngspice's tools too).
pub fn write_real(plot: &Plot) -> Vec<u8> {
    let mut out = format!(
        "Title: {}\nDate: CDePT\nPlotname: {}\nFlags: real\nNo. Variables: {}\nNo. Points: {}\nVariables:\n",
        plot.title,
        plot.plotname,
        plot.names.len(),
        plot.points()
    );
    for (i, n) in plot.names.iter().enumerate() {
        let kind = if n == "frequency" { "frequency" } else { "notype" };
        let _ = i;
        out.push_str(&format!("\t{i}\t{n}\t{kind}\n"));
    }
    out.push_str("Binary:\n");
    let mut bytes = out.into_bytes();
    for p in 0..plot.points() {
        for c in &plot.data {
            bytes.extend_from_slice(&c[p].to_le_bytes());
        }
    }
    bytes
}

/// Every plot of a raw file.
pub fn read(path: &Path) -> Result<Vec<Plot>> {
    let bytes = std::fs::read(path).wrap_err_with(|| format!("cannot read {}", path.display()))?;
    parse(&bytes).wrap_err_with(|| path.display().to_string())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A binary real raw file with the given variables and points.
    pub fn binary(names: &[&str], columns: &[Vec<f64>]) -> Vec<u8> {
        let mut out = format!(
            "Title: t\nDate: d\nPlotname: Transient Analysis\nFlags: real\nNo. Variables: {}\nNo. Points: {}\nVariables:\n",
            names.len(),
            columns[0].len()
        );
        for (i, n) in names.iter().enumerate() {
            out.push_str(&format!("\t{i}\t{n}\tnotype\n"));
        }
        out.push_str("Binary:\n");
        let mut bytes = out.into_bytes();
        for p in 0..columns[0].len() {
            for c in columns {
                bytes.extend_from_slice(&c[p].to_le_bytes());
            }
        }
        bytes
    }

    #[test]
    fn reads_binary_plots() {
        let mut bytes = binary(&["time", "out", "Vdd:flow(br)"], &[vec![0.0, 1.0, 2.0], vec![0.0, 0.5, 1.0], vec![-1.0, -2.0, -3.0]]);
        bytes.extend(binary(&["time", "x"], &[vec![0.0], vec![7.0]]));
        let plots = parse(&bytes).unwrap();
        assert_eq!(plots.len(), 2);
        let p = &plots[0];
        assert_eq!(p.points(), 3);
        assert_eq!(p.get("v(OUT)").unwrap(), [0.0, 0.5, 1.0]);
        assert_eq!(p.get("i(vdd)").unwrap(), [-1.0, -2.0, -3.0]);
        assert_eq!(p.x(), [0.0, 1.0, 2.0]);
        assert!(p.get("v(nope)").is_none());
        assert_eq!(plots[1].get("x").unwrap(), [7.0]);
    }

    #[test]
    fn reads_complex_and_short_files() {
        let mut head = b"Title: t\nPlotname: AC\nFlags: complex\nNo. Variables: 2\nNo. Points: 1\nVariables:\n\t0\tfrequency\tnotype\n\t1\tout\tnotype\nBinary:\n".to_vec();
        for v in [10.0f64, 0.0, 0.5, -0.5] {
            head.extend_from_slice(&v.to_le_bytes());
        }
        let p = &parse(&head).unwrap()[0];
        assert!(p.complex);
        assert_eq!((p.data[1][0], p.imag[1][0]), (0.5, -0.5));
        head.truncate(head.len() - 4);
        assert!(parse(&head).unwrap_err().to_string().contains("short"));
    }

    #[test]
    fn noise_plots_get_ngspices_spectra() {
        let bytes = binary(&["frequency", "onoise", "gain"], &[vec![1.0, 10.0], vec![4e-18, 16e-18], vec![4.0, 4.0]]);
        let p = with_noise_spectra(parse(&bytes).unwrap().remove(0));
        assert_eq!(p.get("onoise_spectrum").unwrap(), [2e-9, 4e-9]);
        let inp = p.get("inoise_spectrum").unwrap();
        assert!((inp[0] - 1e-9).abs() < 1e-21 && (inp[1] - 2e-9).abs() < 1e-21, "{inp:?}");
        // Written and read again.
        let again = parse(&write_real(&p)).unwrap().remove(0);
        assert_eq!(again.names, p.names);
        assert_eq!(again.data, p.data);
        // Not a noise plot: unchanged.
        let tran = parse(&binary(&["time", "out"], &[vec![0.0], vec![1.0]])).unwrap().remove(0);
        assert_eq!(with_noise_spectra(tran.clone()).names, tran.names);
    }

    #[test]
    fn reads_ascii_plots() {
        let text = "Title: t\nPlotname: DC\nFlags: real\nNo. Variables: 2\nNo. Points: 2\nVariables:\n\t0\tv-sweep\tvoltage\n\t1\tout\tvoltage\nValues:\n 0\t0\n\t1\n 1\t0.5\n\t2\n";
        let p = &parse(text.as_bytes()).unwrap()[0];
        assert_eq!(p.data[0], [0.0, 0.5]);
        assert_eq!(p.data[1], [1.0, 2.0]);
    }
}
