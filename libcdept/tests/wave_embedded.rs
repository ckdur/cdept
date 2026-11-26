//! The embedded UberWave (`cdept-uberwave`): a family opens in the window of this process,
//! a second open reuses it, the calculator pushes raw traces, a screenshot shows them.
//! Runs only when UberWave is built in (`UBERWAVE_SRC` at build time) and a display
//! exists; skipped otherwise (the fake-viewer tests of wave.rs cover the process backend).

#![cfg(unix)]

use std::ffi::OsStr;
use std::path::Path;

use libcdept::testutil::hermetic_env_with;
use libcdept::wave;

const OP: &str = "Title: t\nPlotname: Operating Point\nFlags: real\nNo. Variables: 2\nNo. Points: 1\nVariables:\n\t0\tv(a)\tvoltage\n\t1\ti(v1)\tcurrent\nValues:\n 0\t1.0\n\t0.5\n\n";

/// A transient raw file: v(a) rises, v(b) falls, `n` points.
fn tran(n: usize, scale: f64) -> String {
    let mut text = format!(
        "Title: t\nPlotname: Transient Analysis\nFlags: real\nNo. Variables: 3\nNo. Points: {n}\n\
         Variables:\n\t0\ttime\ttime\n\t1\tv(a)\tvoltage\n\t2\tv(b)\tvoltage\nValues:\n"
    );
    for i in 0..n {
        let t = i as f64 / (n - 1) as f64;
        text.push_str(&format!(
            " {i}\t{:e}\n\t{}\n\t{}\n",
            t * 1e-9,
            scale * t,
            scale * (1.0 - t)
        ));
    }
    text.push('\n');
    text
}

fn display() -> bool {
    ["DISPLAY", "WAYLAND_DISPLAY"]
        .iter()
        .any(|v| std::env::var_os(v).is_some_and(|x| !x.is_empty()))
}

fn png_has_content(path: &Path) {
    let bytes = std::fs::read(path).unwrap();
    assert!(bytes.len() > 2000 && &bytes[1..4] == b"PNG", "not a PNG");
    let img = image::open(path).unwrap().to_rgb8();
    let bright = img
        .pixels()
        .filter(|p| p.0.iter().any(|c| *c > 150))
        .count();
    assert!(
        bright > 200,
        "the screenshot is dark: {bright} bright pixels"
    );
}

#[test]
fn a_family_opens_in_this_process_is_reused_and_takes_calculator_traces() {
    if !wave::backend::embedded_available() || !display() {
        eprintln!("skipped: UberWave is not built in, or there is no display");
        return;
    }
    // The display connection needs XDG_RUNTIME_DIR (Wayland): keep the real one.
    let runtime = std::env::var_os("XDG_RUNTIME_DIR");
    let _env = hermetic_env_with(&[
        ("CDEPT_UBERWAVE_BACKEND", Some(OsStr::new("embedded"))),
        ("XDG_RUNTIME_DIR", runtime.as_deref()),
    ]);
    assert_eq!(
        wave::backend::choose(None).unwrap(),
        wave::backend::Backend::Embedded
    );
    wave::embedded::configure(wave::embedded::Options {
        title: Some("cdept test".into()),
        width: 1100,
        height: 650,
        headless: true,
        ..Default::default()
    });

    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("proj");
    std::fs::create_dir_all(dir.join("libs")).unwrap();
    std::fs::write(dir.join("cdept.toml"), "").unwrap();
    let project = libcdept::Project::load(&dir).unwrap();
    let base = dir.join("simulations/cells/tb/testbench");
    for (point, scale) in [("tt_27", 1.2), ("ff_27", 1.32), ("ss_27", 1.08)] {
        std::fs::create_dir_all(base.join(point)).unwrap();
        // 300000 points: more than the process backend's default limit of 200000, which
        // the embedded one must not apply.
        std::fs::write(
            base.join(point).join("out.raw"),
            format!("{OP}{}", tran(300_000, scale)),
        )
        .unwrap();
    }

    let all = wave::select_points(&project, "cells", "tb", "testbench", &[]).unwrap();
    let show = wave::default_show(&project, "cells", "tb", "testbench");
    let opened =
        wave::open_family(&project, "cells", "tb", "testbench", &all, None, &show).unwrap();
    assert!(opened.embedded && !opened.reused, "{}", opened.text());
    assert!(
        opened.shown.iter().any(|s| s == "tt_27:v(a)"),
        "{:?}",
        opened.shown
    );

    // Again: the same window.
    let again = wave::open_family(&project, "cells", "tb", "testbench", &all, None, &show).unwrap();
    assert!(again.embedded && again.reused, "{}", again.text());

    // The calculator: raw data, not thinned.
    let req = wave::calc::Request {
        expr: "v(\"a\") * 2".into(),
        ..Default::default()
    };
    let sent = wave::calc::send(&project, "cells", "tb", "testbench", &all, &req);
    assert!(sent.problem.is_none(), "{:?}", sent.problem);
    assert!(sent.embedded);
    assert_eq!(sent.pushed.len(), 3, "{:?}", sent.pushed);
    assert!(
        sent.pushed.iter().all(|(_, n)| *n == 300_000),
        "not decimated: {:?}",
        sent.pushed
    );
    assert!(
        sent.notes.iter().all(|n| !n.contains("envelope")),
        "{:?}",
        sent.notes
    );

    let viewer = wave::embedded::running().expect("the window is open");
    let files = viewer.list_files().unwrap();
    for tag in ["tt_27", "ff_27", "ss_27"] {
        assert!(files.iter().any(|f| f.tag == tag), "{files:?}");
    }
    let ext = viewer.signals(Some("(external)"), None).unwrap();
    assert!(
        ext.signals
            .iter()
            .any(|s| s.name.contains("@ tt_27") && s.file == "(external)"),
        "{ext:?}"
    );

    let png = tmp.path().join("shot.png");
    viewer.screenshot(&png).unwrap();
    png_has_content(&png);

    // A closed window is started again by the next open.
    wave::embedded::close();
    assert!(wave::embedded::running().is_none());
    let reopened =
        wave::open_family(&project, "cells", "tb", "testbench", &all, None, &show).unwrap();
    assert!(reopened.embedded && !reopened.reused);
    wave::embedded::close();
}
