//! The KLayout helper, for real: these start `klayout -zz` with the helper script.
//! Skipped (with a note) when KLayout or the helper script is not found, as in CI.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use libcdept::Template;
use libcdept::klayout::protocol::{Button, Event, Input, Modifiers, method};
use libcdept::klayout::{Engine, EngineOptions, Session, SessionOptions, Status, setup};
use libcdept::layout::{FileRefs, RefTable};
use serde_json::json;

fn available() -> bool {
    let ok = setup::find_klayout().is_some() && setup::find_helper_dir().is_some();
    if !ok {
        eprintln!("skipped: KLayout or the cdept-klayout helper was not found");
    }
    ok
}

/// A GDS file with one empty cell `name`, and its reference table.
fn empty_layout(dir: &Path, name: &str) -> RefTable {
    let file = dir.join(format!("{name}.gds"));
    std::fs::write(&file, Template::GdsLayout.contents(name)).unwrap();
    RefTable {
        root: file.clone(),
        files: [(
            file,
            FileRefs {
                label: format!("test/{name}/layout"),
                refs: Default::default(),
            },
        )]
        .into(),
    }
}

#[test]
fn version_python_open_save_render() {
    if !available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let refs = empty_layout(dir.path(), "top");
    let mut engine = Engine::start(EngineOptions::default()).unwrap();
    assert!(
        engine.klayout_version().starts_with("KLayout"),
        "{}",
        engine.klayout_version()
    );
    assert_eq!(engine.python("1 + 1").unwrap(), "2");
    assert!(engine.call("no_such_method", json!({})).is_err());

    let info = engine.open(&refs, true).unwrap();
    assert_eq!(info.top, "top");
    assert!(info.missing.is_empty());
    let report = engine.save(None).unwrap();
    assert_eq!(report.top, "top");
    assert!(report.references.is_empty());

    let png = dir.path().join("top.png");
    engine.render_png(&refs, 64, 48, &png).unwrap();
    let bytes = std::fs::read(&png).unwrap();
    assert!(bytes.starts_with(b"\x89PNG"), "not a PNG");

    let out = dir.path().join("flat.gds");
    engine.export(&refs, &out).unwrap();
    assert!(out.is_file());
    engine.shutdown().unwrap();
}

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_session_draws_and_answers() {
    if !available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let refs = empty_layout(dir.path(), "cell");
    let session = Session::open(SessionOptions {
        klayout: None,
        cwd: None,
        env: Vec::new(),
        refs,
        editable: true,
        wake: Arc::new(|| {}),
    });
    session.resize(320, 200);
    wait_for("the helper", || session.status() == Status::Running);
    wait_for("a frame", || {
        session
            .frame(|f| f.width() == 320 && f.height() == 200)
            .unwrap_or(false)
    });
    // A new frame file starts out blank; wait until KLayout has painted it (opaque).
    let mut pixels = Vec::new();
    wait_for("a painted frame", || {
        let copied = session.frame(|f| f.copy(f.full(), &mut pixels)).unwrap();
        assert_eq!((copied.width, copied.height), (320, 200));
        assert_eq!(pixels.len(), 320 * 200 * 4);
        pixels[3] == 255
    });

    let id = session.call(method::INFO, json!({}));
    let mut events = Vec::new();
    let mut reply = None;
    wait_for("the info reply", || {
        events.extend(session.take_events());
        reply = session.take_replies().into_iter().find(|r| r.id == id);
        reply.is_some()
    });
    let info = reply.unwrap().result.unwrap();
    assert_eq!(info["top"], "cell", "{info}");

    // Input goes through without an answer; the helper keeps running.
    session.input(Input::Move {
        x: 10.0,
        y: 10.0,
        buttons: vec![],
        modifiers: Modifiers::default(),
    });
    session.input(Input::Press {
        x: 10.0,
        y: 10.0,
        button: Button::Left,
        modifiers: Modifiers::default(),
    });
    session.input(Input::Release {
        x: 10.0,
        y: 10.0,
        button: Button::Left,
        modifiers: Modifiers::default(),
    });
    wait_for("the cell tree", || {
        events.extend(session.take_events());
        events.iter().any(|e| matches!(e, Event::Cells { .. }))
    });
    assert_eq!(session.status(), Status::Running);
    let version = session.klayout_version().unwrap();
    assert!(version.starts_with("KLayout"), "{version}");
}
