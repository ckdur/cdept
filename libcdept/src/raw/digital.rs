//! Digital waveforms (VCD, FST, GHW) read with `wellen` (BSD-3-Clause), as one plot whose
//! variables are the file's signals by their full hierarchical names (`tb.dut.clk`).
//!
//! A signal becomes a step waveform in seconds: at each change two points, the old value
//! and the new one at the same time, so a crossing or a frequency measured by the
//! calculator lands on the edge. Bits and vectors are their unsigned value (`x`/`z` in
//! any bit: a gap, NaN), real variables their value, strings and events gaps. Signals are
//! loaded when they are asked for; the hierarchy is read when the file is opened.

use std::io::{BufReader, Read, Seek};
use std::path::Path;
use std::sync::{Arc, Mutex};

use eyre::{Result, bail, eyre};

use super::{Layout, Plot, Var};
use crate::calc::{Samples, Waveform};

/// Is this a VCD, FST or GHW file (by its content)?
pub(crate) fn detect(path: &Path) -> Option<&'static str> {
    let file = std::fs::File::open(path).ok()?;
    let mut reader = BufReader::new(file);
    let format = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        wellen::viewers::detect_file_format(&mut reader)
    }))
    .ok()?;
    match format {
        wellen::FileFormat::Vcd => Some("VCD"),
        wellen::FileFormat::Fst => Some("FST"),
        wellen::FileFormat::Ghw => Some("GHW"),
        wellen::FileFormat::Unknown => None,
    }
}

/// The open file, shared by the plot.
pub(crate) struct Source {
    wave: Mutex<wellen::simple::Waveform>,
    /// The signal of each variable (index 0, the time axis, has none).
    signals: Vec<Option<wellen::SignalRef>>,
    /// Seconds per time unit of the file, as `mul / div` (exact for 1 ns = 1 / 1e9).
    mul: f64,
    div: f64,
}

impl std::fmt::Debug for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("digital::Source")
    }
}

impl PartialEq for Source {
    fn eq(&self, other: &Source) -> bool {
        std::ptr::eq(self, other)
    }
}

fn read(path: &Path) -> Result<wellen::simple::Waveform> {
    // wellen is not written for hostile input: a panic in it is the file's error.
    let options = wellen::LoadOptions {
        multi_thread: false,
        remove_scopes_with_empty_name: false,
    };
    let path = path.to_path_buf();
    std::panic::catch_unwind(move || wellen::simple::read_with_options(&path, &options))
        .map_err(|_| eyre!("the waveform reader gave up on this file (it is damaged)"))?
        .map_err(|e| eyre!("{e}"))
}

/// Open a digital waveform file: its plot (the hierarchy; values come later).
pub(crate) fn open(path: &Path, format: &str) -> Result<Plot> {
    // A file that is not even as long as its smallest header is refused before wellen.
    let mut head = [0u8; 8];
    let mut f = std::fs::File::open(path)?;
    if f.read(&mut head)? == 0 {
        bail!("the file is empty");
    }
    f.rewind()?;
    let wave = read(path)?;
    let h = wave.hierarchy();
    let (mul, div) = match h
        .timescale()
        .and_then(|ts| Some((ts.factor, ts.unit.to_exponent()?)))
    {
        Some((factor, e)) if e < 0 => (factor as f64, 10f64.powi(-(e as i32))),
        Some((factor, e)) => (factor as f64 * 10f64.powi(e as i32), 1.0),
        None => (1.0, 1.0),
    };
    let mut vars = vec![Var {
        name: "time".into(),
        unit: if h.timescale().is_some() {
            "time".into()
        } else {
            "tick".into()
        },
    }];
    let mut signals = vec![None];
    let mut seen = std::collections::HashSet::new();
    for v in h.all_vars() {
        let var = &h[v];
        let name = var.full_name(h);
        if !seen.insert(name.to_lowercase()) {
            continue; // the same signal under the same name twice (aliases)
        }
        let unit = match var.signal_encoding(h) {
            wellen::SignalEncoding::Real => "real".to_string(),
            wellen::SignalEncoding::String => "string".to_string(),
            wellen::SignalEncoding::BitVector(n) => format!("digital {n} bit"),
        };
        vars.push(Var { name, unit });
        signals.push(Some(var.signal_ref()));
    }
    if vars.len() == 1 {
        bail!("the {format} file has no signals");
    }
    let points = wave.time_table().len();
    let source = Arc::new(Source {
        wave: Mutex::new(wave),
        signals,
        mul,
        div,
    });
    Ok(Plot {
        title: format!("{format} file"),
        date: String::new(),
        name: format!("Transient (digital, {format})"),
        complex: false,
        vars,
        points,
        declared_points: points,
        layout: Layout::Digital(source),
    })
}

/// The value of a change: an unsigned number, the real value, or NaN.
fn value_of(v: &wellen::SignalValueRef) -> f64 {
    match v {
        wellen::SignalValueRef::Real(r) => {
            if r.is_finite() {
                *r
            } else {
                f64::NAN
            }
        }
        wellen::SignalValueRef::BitVec(_) => {
            let Some(bits) = v.to_bit_string() else {
                return f64::NAN;
            };
            let mut out = 0f64;
            for b in bits.bytes() {
                out = match b {
                    b'0' => out * 2.0,
                    b'1' => out * 2.0 + 1.0,
                    _ => return f64::NAN,
                };
            }
            out
        }
        _ => f64::NAN,
    }
}

impl Source {
    /// The step waveform of variable `col`.
    pub(crate) fn vector(&self, plot: &Plot, col: usize) -> Result<Waveform> {
        let Some(Some(sig)) = self.signals.get(col) else {
            bail!("'{}' is the time axis", plot.vars[0].name);
        };
        let mut wave = self.wave.lock().unwrap_or_else(|e| e.into_inner());
        let sig = *sig;
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| wave.load_signals(&[sig])))
            .map_err(|_| {
                eyre!(
                    "the waveform reader gave up on '{}' (the file is damaged)",
                    plot.vars[col].name
                )
            })?;
        let table: Vec<u64> = wave.time_table().to_vec();
        let signal = wave
            .get_signal(sig)
            .ok_or_else(|| eyre!("'{}' has no values", plot.vars[col].name))?;
        let (mut x, mut y) = (Vec::new(), Vec::new());
        let mut last: Option<f64> = None;
        for (idx, v) in signal.iter_changes() {
            let Some(&t) = table.get(idx as usize) else {
                continue;
            };
            let t = t as f64 * self.mul / self.div;
            if x.last().is_some_and(|&lx| t < lx) {
                continue; // a file whose time runs backwards: keep x in order
            }
            let value = value_of(&v);
            if let Some(prev) = last {
                if prev.to_bits() == value.to_bits() {
                    continue;
                }
                x.push(t);
                y.push(prev);
            }
            x.push(t);
            y.push(value);
            last = Some(value);
        }
        drop(wave);
        if x.is_empty() {
            bail!("'{}' never changes in the file", plot.vars[col].name);
        }
        // hold the last value to the end of the file
        if let (Some(&end), Some(&lastv)) = (table.last(), y.last()) {
            let end = end as f64 * self.mul / self.div;
            if end > *x.last().unwrap() {
                x.push(end);
                y.push(lastv);
            }
        }
        Ok(Waveform {
            x: Arc::new(x),
            y: Samples::Real(y),
            x_name: plot.vars[0].name.clone(),
            x_unit: plot.vars[0].unit.clone(),
            name: plot.vars[col].name.clone(),
            unit: plot.vars[col].unit.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::RawFile;

    const VCD: &str = "$timescale 1ns $end\n$scope module tb $end\n$var wire 1 ! clk $end\n\
$var wire 4 \" cnt [3:0] $end\n$var real 64 # vin $end\n$upscope $end\n$enddefinitions $end\n\
#0\n0!\nb0000 \"\nr0.5 #\n#5\n1!\n#10\n0!\nb0001 \"\n#15\n1!\nbx001 \"\n#20\n0!\nb0011 \"\nr1.25 #\n#25\n";

    fn write(dir: &std::path::Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, bytes).unwrap();
        p
    }

    #[test]
    fn reads_a_vcd_as_step_waveforms() {
        let dir = tempfile::tempdir().unwrap();
        let raw = RawFile::open(&write(dir.path(), "t.vcd", VCD.as_bytes())).unwrap();
        let p = &raw.plots()[0];
        assert_eq!(p.kind(), "tran");
        assert!(p.vars.iter().any(|v| v.name == "tb.clk"));
        let clk = raw.vector(0, "tb.clk").unwrap();
        assert_eq!(clk.x[0], 0.0);
        // the first rising edge is at 5 ns, as two points
        assert_eq!((clk.x[1], clk.x[2]), (5e-9, 5e-9));
        assert_eq!(clk.y.re()[1..3], [0.0, 1.0]);
        // short names find the signal when only one has it
        let cnt = raw.vector(0, "cnt").unwrap().y.re();
        assert!(cnt.contains(&1.0) && cnt.contains(&3.0) && cnt.iter().any(|v| v.is_nan()));
        let vin = raw.vector(0, "tb.vin").unwrap();
        assert_eq!(*vin.y.re().last().unwrap(), 1.25);
        assert_eq!(*vin.x.last().unwrap(), 25e-9);
        let env = crate::calc::Env::new(&raw);
        let f = crate::calc::evaluate("frequency(v(\"clk\"), level=0.5)", &env).unwrap();
        assert!((f.as_f64().unwrap() - 1e8).abs() < 1.0, "{f:?}");
    }

    /// `testdata/counter.fst`: Icarus Verilog's FST of `testdata/counter.v` (clock 10 ns,
    /// a 4-bit counter on its rising edges).
    #[test]
    fn reads_an_fst() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(dir.path(), "c.fst", include_bytes!("testdata/counter.fst"));
        let raw = RawFile::open(&p).unwrap();
        assert!(raw.plots()[0].name.contains("FST"));
        let env = crate::calc::Env::new(&raw);
        let f = crate::calc::evaluate("frequency(v(\"tb.clk\"), level=0.5)", &env).unwrap();
        assert!((f.as_f64().unwrap() - 1e8).abs() < 1.0, "{f:?}");
        let cnt = raw.vector(0, "cnt").unwrap();
        assert_eq!(cnt.y.re().iter().cloned().fold(0.0, f64::max), 10.0);
    }

    #[test]
    fn damaged_digital_files_are_errors() {
        let dir = tempfile::tempdir().unwrap();
        // every prefix of a good file, and garbage after a good header
        for n in [0, 10, 40, 90, 140, 200, VCD.len() - 3] {
            let p = write(dir.path(), "cut.vcd", &VCD.as_bytes()[..n.min(VCD.len())]);
            if let Ok(raw) = RawFile::open(&p) {
                for v in raw.plots()[0]
                    .vars
                    .iter()
                    .skip(1)
                    .map(|v| v.name.clone())
                    .collect::<Vec<_>>()
                {
                    let _ = raw.vector(0, &v);
                }
            }
        }
        let mut bad = VCD.as_bytes()[..150].to_vec();
        bad.extend((0..2000u32).map(|i| (i * 131) as u8));
        let p = write(dir.path(), "noise.vcd", &bad);
        if let Ok(raw) = RawFile::open(&p) {
            let _ = raw.vector(0, "tb.clk");
        }
        // an FST / GHW that is only noise
        for name in ["n.fst", "n.ghw"] {
            let p = write(
                dir.path(),
                name,
                &(0..5000u32)
                    .map(|i| ((i * 7919) >> 3) as u8)
                    .collect::<Vec<_>>(),
            );
            assert!(RawFile::open(&p).is_err());
        }
        assert!(RawFile::open(&write(dir.path(), "e.vcd", b"")).is_err());
        // a VCD whose time goes backwards
        let back = "$timescale 1ps $end\n$var wire 1 ! a $end\n$enddefinitions $end\n#10\n1!\n#5\n0!\n#20\n1!\n";
        if let Ok(raw) = RawFile::open(&write(dir.path(), "b.vcd", back.as_bytes()))
            && let Ok(w) = raw.vector(0, "a")
        {
            assert!(w.x.windows(2).all(|p| p[0] <= p[1]), "{:?}", w.x);
        }
    }
}
