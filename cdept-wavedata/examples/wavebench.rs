//! Measurements of the data engine on a real file (docs/NOTES.md, waveform viewer
//! step 2): first and second open, cache size, memory, query latency across zoom
//! levels, decimated extremes against every sample, two readers on one file.
//!
//! ```sh
//! cargo run --release -p cdept-wavedata --example wavebench -- FILE.raw [--signals N] [--pixels P]
//! ```
//!
//! The cache beside the file (`FILE.uwraw`) is deleted first, so the first open converts.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use cdept_wavedata::{Dataset, Mode, OpenOptions, Phase, Prepared, Text, Trace};

fn status() -> (u64, u64, u64, u64) {
    let text = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let field = |name: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(name))
            .and_then(|v| v.split_whitespace().next()?.parse().ok())
            .unwrap_or(0u64)
    };
    (
        field("VmRSS:") / 1024,
        field("VmHWM:") / 1024,
        field("RssFile:") / 1024,
        field("RssAnon:") / 1024,
    )
}

/// memory.current and memory.peak of this process's cgroup (the systemd scope it runs
/// in), MiB: what the machine pays, page cache counted once.
fn cgroup() -> Option<(u64, u64)> {
    let line = std::fs::read_to_string("/proc/self/cgroup").ok()?;
    let rel = line.lines().find_map(|l| l.strip_prefix("0::"))?;
    let dir = Path::new("/sys/fs/cgroup").join(rel.trim_start_matches('/'));
    let read = |f: &str| -> Option<u64> {
        std::fs::read_to_string(dir.join(f))
            .ok()?
            .trim()
            .parse()
            .ok()
    };
    Some((
        read("memory.current")? / 1048576,
        read("memory.peak")? / 1048576,
    ))
}

fn mem(label: &str) {
    let (rss, hwm, file, anon) = status();
    let cg = cgroup().map_or(String::new(), |(c, p)| {
        format!("; cgroup {c} MiB, cgroup peak {p} MiB")
    });
    println!(
        "  memory {label}: RSS {rss} MiB (file-backed {file}, anon {anon}), peak RSS {hwm} MiB{cg}"
    );
}

fn open(path: &Path) -> (Dataset, f64) {
    let t = Instant::now();
    let mut last = -1i32;
    let ds = Dataset::open(path, &OpenOptions::default(), |phase, f| {
        if phase == Phase::Convert {
            let pct = (f * 10.0) as i32;
            if pct != last {
                last = pct;
                eprint!("\r  converting {:3.0}%", f * 100.0);
            }
        }
        true
    })
    .unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    if last >= 0 {
        eprintln!();
    }
    (ds, t.elapsed().as_secs_f64())
}

fn pct(v: &mut [f64], p: f64) -> f64 {
    v.sort_by(f64::total_cmp);
    if v.is_empty() {
        return 0.0;
    }
    v[((v.len() - 1) as f64 * p).round() as usize]
}

fn main() {
    let mut args = std::env::args().skip(1);
    let path = PathBuf::from(
        args.next()
            .expect("usage: wavebench FILE [--signals N] [--pixels P]"),
    );
    let mut n_signals = usize::MAX;
    let mut pixels = 1600u32;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--signals" => n_signals = args.next().unwrap().parse().unwrap(),
            "--pixels" => pixels = args.next().unwrap().parse().unwrap(),
            _ => panic!("unknown argument {a}"),
        }
    }
    let source_size = std::fs::metadata(&path).unwrap().len();
    println!(
        "file {} ({:.1} MiB)",
        path.display(),
        source_size as f64 / 1048576.0
    );
    let cache = path.with_extension("uwraw");
    let _ = std::fs::remove_file(&cache);
    mem("at start");

    // 1. First open: converts.
    let (ds, first) = open(&path);
    let cache_path = PathBuf::from(ds.text(Text::Cache));
    let cache_size = std::fs::metadata(&cache_path).map(|m| m.len()).unwrap_or(0);
    println!(
        "first open: {first:.2} s (converted: {}), {} signals x {} points, cache {} ({:.1} MiB), reader {}",
        ds.converted(),
        ds.signal_count(),
        ds.point_count(),
        cache_path.display(),
        cache_size as f64 / 1048576.0,
        ds.text(Text::Reader)
    );
    mem("after the first open");
    drop(ds);
    mem("after closing it");

    // 2. Second open: the cache.
    let (ds, second) = open(&path);
    println!(
        "second open: {:.1} ms (converted: {})",
        second * 1e3,
        ds.converted()
    );
    mem("after the second open");
    let ds = Arc::new(ds);
    let (x0, x1) = ds.x_range().unwrap();
    let span = x1 - x0;
    let signals: Vec<u32> = (1..ds.signal_count()).take(n_signals).collect();

    // 3. Query latency, before any pyramid (min/max read from the file), then with them.
    let windows = [1.0, 0.1, 0.01, 1e-3, 1e-4, 1e-5];
    let probe: Vec<u32> = signals.iter().copied().take(4).collect();
    let mut t = Trace::default();
    for pass in ["no pyramid", "pyramid ready"] {
        if pass == "pyramid ready" {
            let t0 = Instant::now();
            ds.prepare(&probe).unwrap();
            while probe
                .iter()
                .any(|s| matches!(ds.prepared(*s), Prepared::Building(_) | Prepared::None))
            {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            println!(
                "pyramids of {} signals built in {:.0} ms (in parallel)",
                probe.len(),
                t0.elapsed().as_secs_f64() * 1e3
            );
            mem("with 4 pyramids");
        }
        println!("query latency at {pixels} px, {pass} (4 signals x 25 positions per zoom):");
        for w in windows {
            let mut ms = vec![];
            let mut pts = 0usize;
            let mut modes = [0usize; 3];
            for k in 0..25 {
                let start = x0 + (span * (1.0 - w)) * (k as f64 / 24.0);
                for &s in &probe {
                    let t0 = Instant::now();
                    let info = ds
                        .query(s, start, start + span * w, pixels, &mut t)
                        .unwrap();
                    ms.push(t0.elapsed().as_secs_f64() * 1e3);
                    pts = pts.max(t.x.len());
                    modes[match info.mode {
                        Mode::Direct => 0,
                        Mode::MinMax => 1,
                        Mode::Pyramid => 2,
                    }] += 1;
                }
            }
            println!(
                "  window {:>8}: p50 {:7.3} ms  p95 {:7.3} ms  max {:7.3} ms  (<= {pts} points; direct {} min/max {} pyramid {})",
                format!("{w:e}"),
                pct(&mut ms.clone(), 0.5),
                pct(&mut ms.clone(), 0.95),
                pct(&mut ms, 1.0),
                modes[0],
                modes[1],
                modes[2]
            );
        }
    }

    // 4. Decimated extremes against every sample, all signals, several zooms, with
    // and without pyramids.
    println!(
        "decimated min/max against every sample ({} signals):",
        signals.len()
    );
    let mut checked = 0;
    let mut bad = 0;
    let t0 = Instant::now();
    for pass in 0..2 {
        if pass == 1 {
            let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let st = stop.clone();
            let sampler = std::thread::spawn(move || {
                let (mut f, mut a, mut r) = (0, 0, 0);
                while !st.load(std::sync::atomic::Ordering::Relaxed) {
                    let (rss, _, file, anon) = status();
                    r = r.max(rss);
                    f = f.max(file);
                    a = a.max(anon);
                    std::thread::sleep(std::time::Duration::from_micros(500));
                }
                (r, f, a)
            });
            let t1 = Instant::now();
            ds.prepare(&signals).unwrap();
            while signals
                .iter()
                .any(|s| matches!(ds.prepared(*s), Prepared::Building(_) | Prepared::None))
            {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            stop.store(true, std::sync::atomic::Ordering::Relaxed);
            let (r, f, a) = sampler.join().unwrap();
            println!(
                "  {} pyramids built in {:.0} ms; sampled during the build: RSS max {r} MiB (file-backed max {f}, anon max {a})",
                signals.len(),
                t1.elapsed().as_secs_f64() * 1e3
            );
            mem(&format!("with {} pyramids", signals.len()));
        }
        for &s in &signals {
            for (wi, w) in [1.0, 0.1, 0.01, 1e-3].iter().enumerate() {
                let start = x0 + span * (1.0 - w) * ((s as f64 * 0.37 + wi as f64 * 0.11) % 1.0);
                let info = ds
                    .query(s, start, start + span * w, pixels, &mut t)
                    .unwrap();
                let lo = t.y.iter().cloned().fold(f64::INFINITY, f64::min);
                let hi = t.y.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
                // The exact extremes over the samples the query covered.
                let mut xa = [0.0];
                let mut xb = [0.0];
                ds.samples(0, info.first_index, &mut xa).unwrap();
                ds.samples(0, info.last_index, &mut xb).unwrap();
                let (elo, ehi, _) = ds.exact_minmax(s, xa[0], xb[0]).unwrap().unwrap();
                checked += 1;
                if lo != elo || hi != ehi {
                    bad += 1;
                    println!(
                        "  MISMATCH {} window {w:e} mode {:?}: drawn {lo:e}/{hi:e} exact {elo:e}/{ehi:e}",
                        ds.signal_name(s),
                        info.mode
                    );
                }
            }
        }
    }
    println!(
        "  {checked} queries checked, {bad} mismatches ({:.1} s)",
        t0.elapsed().as_secs_f64()
    );
    drop(ds);
    mem("after closing");

    // 5. Two readers open the file at once with no cache: one converts, one waits.
    let _ = std::fs::remove_file(&cache);
    let t0 = Instant::now();
    let p1 = path.clone();
    let p2 = path.clone();
    let a = std::thread::spawn(move || {
        let (d, s) = open(&p1);
        (d.converted(), s)
    });
    let b = std::thread::spawn(move || {
        let (d, s) = open(&p2);
        (d.converted(), s)
    });
    let (ca, ta) = a.join().unwrap();
    let (cb, tb) = b.join().unwrap();
    println!(
        "two readers at once without a cache: {ta:.2} s (converted {ca}) and {tb:.2} s (converted {cb}), {:.2} s in all",
        t0.elapsed().as_secs_f64()
    );
    mem("at the end");
}
