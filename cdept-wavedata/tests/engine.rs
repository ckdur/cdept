//! The binding over the real engine (skipped, with a note, in a stub build).

use std::io::Write;
use std::path::Path;
use std::sync::Arc;

use cdept_wavedata::{Dataset, Mode, OpenOptions, Phase, Prepared, Status, Text, Trace};

/// A binary ngspice raw file: x = i ns, `v(out)` = f(i).
fn write_raw(path: &Path, points: u64, f: impl Fn(u64) -> f64) {
    let mut out = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
    write!(
        out,
        "Title: t\nDate: d\nPlotname: Transient Analysis\nFlags: real\nNo. Variables: 2\n\
         No. Points: {points}\nVariables:\n\t0\ttime\ttime\n\t1\tv(out)\tvoltage\nBinary:\n"
    )
    .unwrap();
    for i in 0..points {
        out.write_all(&(i as f64 * 1e-9).to_le_bytes()).unwrap();
        out.write_all(&f(i).to_le_bytes()).unwrap();
    }
}

fn skip() -> bool {
    if !cdept_wavedata::available() {
        eprintln!("the data engine is not built in: skipped");
        return true;
    }
    false
}

/// CI's job `uberwave` sets `CDEPT_REQUIRE_ENGINES=1`: there a stub is a failure (a
/// stub build only prints a cargo:warning).
#[test]
fn the_engine_is_built_in_when_required() {
    if std::env::var_os("CDEPT_REQUIRE_ENGINES").is_some_and(|v| v == "1") {
        assert!(
            cdept_wavedata::available(),
            "CDEPT_REQUIRE_ENGINES=1 but the waveform data engine is the stub (see the cargo:warning of cdept-wavedata's build)"
        );
    }
}

#[test]
fn stub_or_real_open_reports() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("none.raw");
    let err = Dataset::open(&missing, &OpenOptions::default(), |_, _| true).unwrap_err();
    if cdept_wavedata::available() {
        assert_eq!(err.status, Status::Io, "{err}");
    } else {
        assert_eq!(err.status, Status::Unavailable);
        assert!(err.message.contains("not built"));
    }
}

#[test]
fn opens_queries_and_keeps_a_spike() {
    if skip() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let raw = dir.path().join("spike.raw");
    const N: u64 = 1_500_000;
    const AT: u64 = 1_000_003;
    write_raw(
        &raw,
        N,
        |i| if i == AT { 5.0 } else { (i % 7) as f64 * 1e-3 },
    );
    let mut phases = vec![];
    let ds = Dataset::open(&raw, &OpenOptions::default(), |p, _| {
        if phases.last() != Some(&p) {
            phases.push(p);
        }
        true
    })
    .unwrap();
    assert!(phases.contains(&Phase::Convert), "{phases:?}");
    assert!(ds.converted());
    assert_eq!(ds.signal_count(), 2);
    assert_eq!(ds.point_count(), N);
    assert_eq!(ds.signal_name(1), "v(out)");
    assert_eq!(ds.signal_unit(1), "V");
    assert_eq!(ds.text(Text::XName), "time");
    assert!(ds.text(Text::Cache).ends_with("spike.uwraw"));
    let (x0, x1) = ds.x_range().unwrap();
    assert_eq!(x0, 0.0);

    let mut t = Trace::default();
    let info = ds.query(1, x0, x1, 800, &mut t).unwrap();
    assert!(t.x.len() <= cdept_wavedata::query_capacity(800));
    assert_eq!(t.y.iter().cloned().fold(f64::MIN, f64::max), 5.0);
    assert_ne!(info.mode, Mode::Direct);

    ds.prepare(&[1]).unwrap();
    for _ in 0..2000 {
        if ds.prepared(1) == Prepared::Ready {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(ds.prepared(1), Prepared::Ready);
    let info = ds.query(1, x0, x1, 800, &mut t).unwrap();
    assert_eq!(info.mode, Mode::Pyramid);
    assert_eq!(t.y.iter().cloned().fold(f64::MIN, f64::max), 5.0);

    let (y, i) = ds.value_at(1, AT as f64 * 1e-9).unwrap();
    assert_eq!((y, i), (5.0, AT));
    let (lo, hi, n) = ds.exact_minmax(1, x0, x1).unwrap().unwrap();
    assert_eq!((lo, hi, n), (0.0, 5.0, N));
    let mut s = [0.0; 3];
    ds.samples(1, AT - 1, &mut s).unwrap();
    assert_eq!(s[1], 5.0);
    assert!(!ds.source_changed());

    // A second dataset on the same file, used from another thread, reuses the cache.
    let other = Arc::new(Dataset::open(&raw, &OpenOptions::default(), |_, _| true).unwrap());
    assert!(!other.converted());
    let o = other.clone();
    let max = std::thread::spawn(move || {
        let mut t = Trace::default();
        o.query(1, x0, x1, 300, &mut t).unwrap();
        t.y.iter().cloned().fold(f64::MIN, f64::max)
    })
    .join()
    .unwrap();
    assert_eq!(max, 5.0);
}

#[test]
fn cancel_stops_the_conversion() {
    if skip() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let raw = dir.path().join("big.raw");
    write_raw(&raw, 2_000_000, |i| i as f64);
    let err = Dataset::open(&raw, &OpenOptions::default(), |p, _| p != Phase::Convert).unwrap_err();
    assert_eq!(err.status, Status::Cancelled);
    assert!(!dir.path().join("big.uwraw").exists());
    // A panic in the callback cancels too, it does not cross into C++.
    let err = Dataset::open(&raw, &OpenOptions::default(), |p, _| {
        if p == Phase::Convert {
            panic!("callback panics");
        }
        true
    })
    .unwrap_err();
    assert_eq!(err.status, Status::Cancelled);
}

#[test]
fn corrupt_file_is_an_error() {
    if skip() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let raw = dir.path().join("junk.raw");
    std::fs::write(&raw, b"No. Variables: 99999999999\nBinary:\n").unwrap();
    let err = Dataset::open(&raw, &OpenOptions::default(), |_, _| true).unwrap_err();
    assert_eq!(err.status, Status::Format, "{err}");
    assert!(!err.message.is_empty());
}
