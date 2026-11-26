//! The embedded viewer on its own thread, driven from a Rust test binary. Runs only when
//! UberWave is built in (`uberwave_embedded`) and a display exists.

#![cfg(uberwave_embedded)]

use std::sync::{Arc, Mutex};

use cdept_uberwave::{Marker, Options, Pane, TraceStyle, Viewer};

/// One viewer window per process: the tests take turns.
static ONE_WINDOW: Mutex<()> = Mutex::new(());

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
    let _turn = ONE_WINDOW.lock().unwrap_or_else(|e| e.into_inner());
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
        let ext = viewer.list_signals(Some("(external)"), None).unwrap();
        assert!(
            ext.signals
                .iter()
                .any(|s| s.name == "calc" && s.file == "(external)")
        );
        let files = viewer.list_files().unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].tag, "A");
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

/// Typed lists, the cursor listener, markers, an eye diagram and calls after the window
/// closed.
#[test]
fn listener_markers_eye_and_a_closed_window() {
    if !display() {
        eprintln!("no display: skipped");
        return;
    }
    let _turn = ONE_WINDOW.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let mut viewer = Viewer::start(&Options {
        headless: true,
        width: 900,
        height: 500,
        ..Default::default()
    })
    .unwrap();
    let a = fixture(dir.path(), "a.raw", 1.0);
    viewer.open(&a, None, Some("A")).unwrap();
    viewer.show(&["A:v(out)".into()], Pane::New).unwrap();
    type Seen = Arc<Mutex<Vec<(Option<f64>, Option<f64>)>>>;
    let seen: Seen = Arc::default();
    let log = seen.clone();
    viewer
        .set_cursor_listener(Some(Box::new(move |a, b| log.lock().unwrap().push((a, b)))))
        .unwrap();
    viewer.set_cursors(Some(3.0), Some(7.5)).unwrap();
    let c = viewer.cursors().unwrap();
    assert_eq!((c.a, c.b), (Some(3.0), Some(7.5)));
    let v = c.values.iter().find(|v| v.signal == "v(out)").unwrap();
    assert_eq!((v.a, v.b), (Some(3.0), Some(7.5)));
    for _ in 0..100 {
        if seen.lock().unwrap().last() == Some(&(Some(3.0), Some(7.5))) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(seen.lock().unwrap().last(), Some(&(Some(3.0), Some(7.5))));
    viewer.set_cursor_listener(None).unwrap();
    let marker = Marker {
        group: "m".into(),
        label: "tpd".into(),
        x: 4.0,
        ..Default::default()
    };
    viewer.add_marker(&marker).unwrap();
    viewer
        .add_marker(&Marker {
            y: Some(2.0),
            ..marker.clone()
        })
        .unwrap();
    assert!(
        viewer
            .add_marker(&Marker {
                x: f64::NAN,
                ..marker.clone()
            })
            .is_err()
    );
    assert_eq!(viewer.clear_markers(Some("m")).unwrap(), 2);
    let t: Vec<f64> = (0..5000).map(|i| i as f64 * 1e-11).collect();
    let y: Vec<f64> = t.iter().map(|t| (t * 3e9).sin()).collect();
    viewer
        .add_trace_styled(
            "eye",
            &t,
            &y,
            Some("V"),
            Some("s"),
            Pane::New,
            TraceStyle {
                x_log: false,
                fold: Some((2e-9, 0.0)),
            },
        )
        .unwrap();
    assert!(
        viewer
            .add_trace_styled(
                "eye2",
                &t,
                &y,
                None,
                None,
                Pane::Default,
                TraceStyle {
                    x_log: false,
                    fold: Some((-1.0, 0.0))
                }
            )
            .is_err()
    );
    assert!(
        viewer
            .add_trace("nul\0name", &t, &y, None, None, Pane::Default)
            .is_err()
    );
    assert!(
        viewer
            .add_trace("short", &t, &y[..3], None, None, Pane::Default)
            .is_err()
    );
    viewer.close();
    assert!(!viewer.is_running());
    assert!(viewer.list_files().is_err());
    assert!(viewer.cursors().is_err());
    assert!(viewer.set_cursor_listener(None).is_err());
}
