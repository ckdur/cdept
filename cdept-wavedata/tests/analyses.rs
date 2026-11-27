//! AC (complex) files of the real simulators, several plots in one file, sweeps that go
//! back, Spectre PSF folders: through the binding, against the analytic answers. The
//! simulator parts run when `CDEPT_NGSPICE`, `CDEPT_XYCE`, `CDEPT_VACASK` are set.

use std::path::{Path, PathBuf};
use std::process::Command;

use cdept_wavedata::{Dataset, OpenOptions, Text, Trace, XOrder, list_analyses};

fn skip() -> bool {
    if !cdept_wavedata::available() {
        eprintln!("the data engine is not built in: skipped");
        return true;
    }
    false
}

fn tool(var: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.is_file())
}

fn open(path: &Path, plot: u32) -> Dataset {
    Dataset::open(
        path,
        &OpenOptions {
            plot,
            ..Default::default()
        },
        |_, _| true,
    )
    .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn index(ds: &Dataset, name: &str) -> u32 {
    (0..ds.signal_count())
        .find(|i| ds.signal_name(*i).eq_ignore_ascii_case(name))
        .unwrap_or_else(|| {
            let names: Vec<String> = (0..ds.signal_count()).map(|i| ds.signal_name(i)).collect();
            panic!("no {name} in {names:?}")
        })
}

fn all(ds: &Dataset, sig: u32) -> Vec<f64> {
    let mut v = vec![0.0; ds.point_count() as usize];
    ds.samples(sig, 0, &mut v).unwrap();
    v
}

/// R = 1k, C = 1n: H = 1 / (1 + j f / fc), fc = 159.155 kHz.
fn check_rc(ds: &Dataset, out: &str, what: &str) {
    let fc = 1.0 / (2.0 * std::f64::consts::PI * 1e3 * 1e-9);
    assert!(ds.x_log(), "{what}: x not log");
    assert_eq!(ds.x_order(), XOrder::Increasing, "{what}");
    let f = all(ds, 0);
    let db = all(ds, index(ds, &format!("dB20({out})")));
    let ph = all(ds, index(ds, &format!("uphase({out})")));
    let re = all(ds, index(ds, &format!("re({out})")));
    let im = all(ds, index(ds, &format!("im({out})")));
    let mut worst = (0.0f64, 0.0f64);
    for i in 0..f.len() {
        let w = f[i] / fc;
        let want_db = -10.0 * (1.0 + w * w).log10();
        let want_ph = -w.atan().to_degrees();
        worst.0 = worst.0.max((db[i] - want_db).abs());
        worst.1 = worst.1.max((ph[i] - want_ph).abs());
        let (wr, wi) = (1.0 / (1.0 + w * w), -w / (1.0 + w * w));
        assert!(
            (re[i] - wr).abs() < 1e-6 && (im[i] - wi).abs() < 1e-6,
            "{what}: re/im at {}",
            f[i]
        );
    }
    // Xyce's ascii has 9 digits: within 1e-6 dB and 1e-5 degrees
    assert!(
        worst.0 < 1e-5 && worst.1 < 1e-4,
        "{what}: dB off {} , phase off {}",
        worst.0,
        worst.1
    );
    // -3.01 dB at fc
    let (y, _) = ds.value_at(index(ds, &format!("dB20({out})")), fc).unwrap();
    assert!((y + 3.0103).abs() < 0.05, "{what}: {y} dB at fc");
    eprintln!(
        "{what}: {} points, dB off {:.2e}, phase off {:.2e} deg, {} dB at fc",
        f.len(),
        worst.0,
        worst.1,
        y
    );
}

const RC: &str = "RC lowpass\nV1 in 0 dc 0 ac 1\nR1 in out 1k\nC1 out 0 1n\n.ac dec 10 1 100meg\n";

#[test]
fn ngspice_ac_ascii_and_binary() {
    if skip() {
        return;
    }
    let Some(ng) = tool("CDEPT_NGSPICE") else {
        eprintln!("CDEPT_NGSPICE not set: skipped");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("rc.cir"), format!("{RC}.end\n")).unwrap();
    for (name, ascii) in [("bin.raw", false), ("ascii.raw", true)] {
        let mut cmd = Command::new(&ng);
        cmd.current_dir(dir.path())
            .args(["-b", "-r", name, "rc.cir"])
            .stdin(std::process::Stdio::null());
        if ascii {
            cmd.env("SPICE_ASCIIRAWFILE", "1");
        }
        assert!(cmd.output().unwrap().status.success());
        let ds = open(&dir.path().join(name), 0);
        assert_eq!(ds.text(Text::Analysis), "ac");
        check_rc(&ds, "v(out)", &format!("ngspice {name}"));
    }
}

#[test]
fn xyce_ac_ascii_and_binary() {
    if skip() {
        return;
    }
    let Some(xyce) = tool("CDEPT_XYCE") else {
        eprintln!("CDEPT_XYCE not set: skipped");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("rc.cir"),
        format!("{RC}.print ac v(out)\n.end\n"),
    )
    .unwrap();
    for (name, ascii) in [("bin.raw", false), ("ascii.raw", true)] {
        let mut cmd = Command::new(&xyce);
        cmd.current_dir(dir.path())
            .env("OMPI_MCA_plm", "isolated")
            .env("OMPI_MCA_btl", "self")
            .env("PMIX_MCA_gds", "hash")
            .stdin(std::process::Stdio::null())
            .arg("-r")
            .arg(name);
        if ascii {
            cmd.arg("-a");
        }
        let out = cmd.arg("rc.cir").output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stdout)
        );
        let ds = open(&dir.path().join(name), 0);
        check_rc(&ds, "OUT", &format!("Xyce {name}"));
    }
}

#[test]
fn vacask_ac_and_noise() {
    if skip() {
        return;
    }
    let Some(vacask) = tool("CDEPT_VACASK") else {
        eprintln!("CDEPT_VACASK not set: skipped");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("rc.sim"),
        "RC lowpass\n\nload \"resistor.osdi\"\nload \"capacitor.osdi\"\ninclude \"builtins.inc\"\n\n\
         model r resistor\nmodel c capacitor\n\nv1 (in 0) vsource dc=0 mag=1\nr1 (in out) r r=1k\nc1 (out 0) c c=1n\n\n\
         control\n  analysis ac1 ac from=1 to=100e6 mode=\"dec\" points=10\n\
         analysis noise1 noise out=\"out\" in=\"v1\" from=1 to=100e6 mode=\"dec\" points=10\nendc\n",
    )
    .unwrap();
    let lib = vacask
        .parent()
        .and_then(Path::parent)
        .map(|p| p.join("lib"))
        .unwrap();
    let out = Command::new(&vacask)
        .current_dir(dir.path())
        .env("LD_LIBRARY_PATH", &lib)
        .arg("rc.sim")
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let ds = open(&dir.path().join("ac1.raw"), 0);
    assert_eq!(ds.text(Text::Analysis), "ac");
    check_rc(&ds, "out", "VACASK ac1.raw");
    // the noise of a 1k resistor through the RC: 4kTR at low frequency, V^2/Hz
    let n = open(&dir.path().join("noise1.raw"), 0);
    assert_eq!(n.text(Text::Analysis), "noise");
    let (y, _) = n.value_at(index(&n, "onoise"), 1.0).unwrap();
    let four_ktr = 4.0 * 1.380649e-23 * 300.15 * 1e3;
    assert!(
        (y / four_ktr - 1.0).abs() < 0.02,
        "VACASK onoise at 1 Hz {y} vs 4kTR {four_ktr}"
    );
}

/// One deck, all analyses: op, a parametric dc whose inner sweep runs backwards, ac,
/// noise, tran: six plots in one raw file.
#[test]
fn ngspice_plots_of_one_file() {
    if skip() {
        return;
    }
    let Some(ng) = tool("CDEPT_NGSPICE") else {
        eprintln!("CDEPT_NGSPICE not set: skipped");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("multi.cir"),
        "* multi\nV1 in 0 dc 0.9 ac 1 sin(0.9 0.5 1meg)\nV2 vb 0 dc 1\nR1 in out 1k\nR2 out vb 2k\nC1 out 0 1n\n\
         .option sparse\n.op\n.dc V1 1.8 0 -0.05 V2 0 2 1\n.ac dec 20 1 1g\n.noise v(out) V1 dec 20 1 1g\n.tran 10n 5u\n.end\n",
    )
    .unwrap();
    for ascii in [false, true] {
        let name = if ascii { "ascii.raw" } else { "bin.raw" };
        let mut cmd = Command::new(&ng);
        cmd.current_dir(dir.path())
            .args(["-b", "-r", name, "multi.cir"])
            .stdin(std::process::Stdio::null());
        if ascii {
            cmd.env("SPICE_ASCIIRAWFILE", "1");
        }
        assert!(cmd.output().unwrap().status.success());
        let path = dir.path().join(name);
        let list = list_analyses(&path).unwrap();
        let kinds: Vec<&str> = list.iter().map(|a| a.kind.as_str()).collect();
        assert_eq!(
            kinds,
            ["ac", "dc", "op", "tran", "noise", "noise"],
            "{name}: {list:?}"
        );
        assert!(list[0].complex && !list[1].complex);
        // dc: three sweeps of V1 from 1.8 down to 0 (V2 = 0, 1, 2)
        let dc = open(&path, 1);
        assert_eq!(dc.x_order(), XOrder::Segmented, "{name}");
        let segs = dc.segments();
        assert_eq!(segs.len(), 3, "{name}: {segs:?}");
        assert!(
            segs.iter()
                .all(|s| s.direction == -1 && s.last - s.first == 36)
        );
        // v(out) = (2 V1 + V2) / 3 (R1 = 1k to in, R2 = 2k to vb)
        let out = index(&dc, "v(out)");
        for (k, _) in segs.iter().enumerate() {
            let (y, _) = dc.value_at_segment(out, k as u32, 0.9).unwrap();
            let want = (2.0 * 0.9 + k as f64) / 3.0;
            assert!((y - want).abs() < 1e-6, "{name} seg {k}: {y} vs {want}");
            let mut t = Trace::default();
            dc.query_segment(out, k as u32, 0.0, 1.8, 500, &mut t)
                .unwrap();
            assert_eq!(t.x.len(), 37);
            assert!(t.x.windows(2).all(|w| w[1] < w[0]), "decreasing x");
        }
        let (lo, hi) = dc.x_extent().unwrap();
        assert!(lo.abs() < 1e-9 && (hi - 1.8).abs() < 1e-9);
        // op: one point
        let op = open(&path, 2);
        assert_eq!(op.point_count(), 1);
        let (y, _) = op.value_at(index(&op, "v(out)"), 0.0).unwrap();
        assert!((y - (2.0 * 0.9 + 1.0) / 3.0).abs() < 1e-9, "op {y}");
        // noise: V/sqrt(Hz), against the integrated noise of the last plot
        let nz = open(&path, 4);
        let spec = index(&nz, "onoise_spectrum");
        assert_eq!(nz.signal_unit(spec), "V/\u{221a}Hz");
        let f = all(&nz, 0);
        let s = all(&nz, spec);
        let mut total = 0.0;
        for i in 1..f.len() {
            // the noise power is close to constant over each log step: trapezoid of S^2
            total += 0.5 * (s[i] * s[i] + s[i - 1] * s[i - 1]) * (f[i] - f[i - 1]);
        }
        let tot = open(&path, 5);
        let (want, _) = tot.value_at(index(&tot, "v(onoise_total)"), 0.0).unwrap();
        assert!(
            (total.sqrt() / want - 1.0).abs() < 0.01,
            "{name}: integrated {} vs ngspice {want}",
            total.sqrt()
        );
        eprintln!(
            "{name}: integrated noise {:.4e} V, ngspice onoise_total {want:.4e} V",
            total.sqrt()
        );
    }
}

/// The Spectre PSF fixtures of the uberwave repository.
#[test]
fn psf_folders() {
    if skip() {
        return;
    }
    let src = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../cdept-uberwave/uberwave/wp3-data-engine/tests/psf");
    if !src.is_dir() {
        eprintln!("no PSF fixtures: skipped");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    copy_dir(&src, dir.path());
    let bin = dir.path().join("psf-parser/binary");
    let list = list_analyses(&bin).unwrap();
    let ac = list.iter().find(|a| a.name == "myac-ac").expect("myac");
    assert!(ac.plottable && ac.complex, "{ac:?}");
    let ds = open(&bin, ac.index);
    assert!(ds.x_log());
    let names: Vec<String> = (0..ds.signal_count()).map(|i| ds.signal_name(i)).collect();
    assert!(names.iter().any(|n| n.starts_with("uphase(")), "{names:?}");
    // magnitude from re/im equals the mag form
    let s = names.iter().position(|n| n.starts_with("re(")).unwrap() as u32;
    let base = &names[s as usize][3..names[s as usize].len() - 1];
    let (re, im, mag) = (
        all(&ds, s),
        all(&ds, index(&ds, &format!("im({base})"))),
        all(&ds, index(&ds, &format!("mag({base})"))),
    );
    for i in 0..re.len() {
        assert!((re[i].hypot(im[i]) - mag[i]).abs() <= 1e-12 * mag[i].abs().max(1.0));
    }
    // a file of a folder without a logFile: its own analysis is preferred
    let file = dir.path().join("spectre-examples/frequencySweep.ac");
    let list = list_analyses(&file).unwrap();
    let pref = list.iter().find(|a| a.preferred).unwrap();
    assert_eq!(pref.name, "frequencySweep.ac");
    assert_eq!(pref.kind, "ac");
    let ds = open(&file, pref.index);
    assert_eq!(ds.text(Text::Analysis), "ac");
    // a tran of the inverter folder
    let inv = dir.path().join("inverter_test.raw");
    let list = list_analyses(&inv).unwrap();
    let tran = list.iter().find(|a| a.kind == "tran").unwrap();
    let ds = open(&inv, tran.index);
    assert_eq!(ds.point_count(), 680);
    assert_eq!(ds.x_order(), XOrder::Increasing);
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let p = e.path();
        let t = to.join(e.file_name());
        if p.is_dir() {
            copy_dir(&p, &t);
        } else {
            std::fs::copy(&p, &t).unwrap();
        }
    }
}
