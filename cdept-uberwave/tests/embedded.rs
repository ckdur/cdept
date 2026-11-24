//! The embedded viewer on its own thread, driven from a Rust test binary. Runs only when
//! UberWave is built in (`uberwave_embedded`) and a display exists.

#![cfg(uberwave_embedded)]

use cdept_uberwave::{Options, Pane, Viewer};

fn display() -> bool {
    ["DISPLAY", "WAYLAND_DISPLAY"]
        .iter()
        .any(|v| std::env::var_os(v).is_some_and(|x| !x.is_empty()))
}

fn fixture(dir: &std::path::Path, name: &str, scale: f64) -> String {
    let mut text = String::from(
        "Title: t\nDate: d\nPlotname: Transient Analysis\nFlags: real\nNo. Variables: 2\nNo. Points: 32\n\
         Variables:\n\t0\ttime\ttime\n\t1\tv(out)\tvoltage\nValues:\n",
    );
    for i in 0..32 {
        text.push_str(&format!("{i}\t{i}\n\t{}\n", scale * i as f64));
    }
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    path.to_string_lossy().into_owned()
}

#[test]
fn a_viewer_opens_shows_plots_and_closes_twice() {
    if !display() {
        eprintln!("no display: skipped");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    for round in 0..2 {
        let viewer = Viewer::start(&Options {
            headless: true,
            width: 900,
            height: 500,
            ..Default::default()
        })
        .unwrap_or_else(|e| panic!("round {round}: {e}"));
        let a = fixture(dir.path(), "a.raw", 1.0);
        viewer.open(&a, None, Some("A")).unwrap();
        let pane = viewer.show(&["A:v(out)".into()], Pane::New).unwrap();
        assert_eq!(pane, 0);
        let x: Vec<f64> = (0..1000).map(|i| i as f64 * 0.03).collect();
        let y: Vec<f64> = x.iter().map(|v| v.sin() * 10.0).collect();
        viewer
            .add_trace("calc", &x, &y, Some("V"), Some("s"), Pane::Default)
            .unwrap();
        assert!(
            viewer
                .list_signals(Some("(external)"), None)
                .unwrap()
                .contains("calc")
        );
        let err = viewer.show(&["nope".into()], Pane::Default).unwrap_err();
        assert!(err.0.contains("not found"), "{err}");
        let png = dir.path().join(format!("s{round}.png"));
        viewer.screenshot(png.to_str().unwrap()).unwrap();
        let bytes = std::fs::read(&png).unwrap();
        assert_eq!(&bytes[1..4], b"PNG");
        assert!(bytes.len() > 2000);
        assert!(viewer.is_running());
        drop(viewer);
    }
}
