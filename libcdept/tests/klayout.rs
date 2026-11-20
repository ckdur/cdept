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
                ..Default::default()
            },
        )]
        .into(),
        ..Default::default()
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

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

/// Editing in place (docs/KLAYOUT.md §3.4): from buf, descend into an inv instance,
/// add a box; the save writes inv.gds too, and buf.gds still only references inv.
#[test]
fn edit_inv_in_place_from_buf() {
    if !available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/buffer");
    copy_tree(&example, dir.path());
    let project = libcdept::Project::load(dir.path()).unwrap();
    let logic = project.library("logic").unwrap();
    let buf = libcdept::klayout::cell_layout(logic, "buf").unwrap();
    let inv = libcdept::klayout::cell_layout(logic, "inv").unwrap();
    let before = std::fs::read(&inv).unwrap();
    let refs = libcdept::layout::refs::resolve(&project, &buf);
    assert!(!refs.files[&inv].read_only);
    let mut engine = Engine::for_project(&project).unwrap();
    // Shapes of `inv` in inv.gds, read by KLayout.
    let count = format!(
        "ly = pya.Layout(); ly.read({:?})\n\
         c = ly.cell('inv'); sum(c.shapes(li).size() for li in ly.layer_indexes())",
        inv.display().to_string()
    );
    let inv_before: u32 = engine.python(&count).unwrap().parse().unwrap();
    engine.open(&refs, true).unwrap();
    // Select the first instance of inv, descend, draw a box (as one undo step).
    engine
        .python(
            "inst = next(layout.cell('buf').each_inst())\n\
             p = pya.ObjectInstPath()\n\
             p.top = layout.cell('buf').cell_index()\n\
             p.append_path(pya.InstElement.new(inst))\n\
             view.select_object(p)",
        )
        .unwrap();
    let value = engine.call(method::DESCEND, json!({})).unwrap();
    assert_eq!(value["cell"], "inv", "{value}");
    assert_eq!(value["context"]["writable"], true, "{value}");
    engine
        .python(
            "view.transaction('box')\n\
             view.cellview(0).cell.shapes(layout.layer(1, 0)).insert(pya.DBox(0, 0, 0.1, 0.1))\n\
             view.commit()",
        )
        .unwrap();
    let report = engine.save(None).unwrap();
    assert_eq!(report.files(), [buf.as_path(), inv.as_path()]);
    libcdept::layout::refs::update_after_save(&project, &report).unwrap();
    let inv_after: u32 = engine.python(&count).unwrap().parse().unwrap();
    engine.shutdown().unwrap();

    assert_ne!(std::fs::read(&inv).unwrap(), before);
    assert_eq!(inv_after, inv_before + 1);
    let summary = libcdept::layout::gds::read_structure(&buf).unwrap();
    assert!(!summary.defines("inv"), "buf.gds must only reference inv");
    assert_eq!(summary.ghosts(), ["inv"]);
    let meta = libcdept::layout::Meta::read_for(&inv).unwrap().unwrap();
    assert_eq!(meta.label(), "logic/inv/layout");
    let meta = libcdept::layout::Meta::read_for(&buf).unwrap().unwrap();
    assert_eq!(meta.references["inv"].cell, "inv");
}

/// A PCell view of the project (`logic/rpoly`, docs/KLAYOUT.md §3.6): the layout that
/// places it (`logic/rdiv`) reads back live PCells of the KLayout library `logic`; a
/// change to the PCell file rebuilds them; the export has plain geometry only.
#[test]
fn project_pcells_are_live() {
    if !available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/buffer");
    copy_tree(&example, dir.path());
    let project = libcdept::Project::load(dir.path()).unwrap();
    let logic = project.library("logic").unwrap();
    let rdiv = libcdept::klayout::cell_layout(logic, "rdiv").unwrap();
    let pcell = logic.cell("rpoly").unwrap().entry("pcell").unwrap();
    assert_eq!(pcell.kind, libcdept::ViewKind::Pcell);
    let file = pcell.main_file.clone().unwrap();
    let refs = libcdept::layout::refs::resolve(&project, &rdiv);
    assert_eq!(refs.pcells["logic"]["rpoly"], file);
    assert!(refs.files[&rdiv].refs.is_empty(), "{refs:?}");
    let mut engine = Engine::for_project(&project).unwrap();
    engine.open(&refs, true).unwrap();
    let lengths = "sorted(round(i.pcell_parameters_by_name()['l'], 3) for i in \
                   layout.cell('rdiv').each_inst() if i.is_pcell() \
                   and i.cell.library().name() == 'logic')";
    assert_eq!(engine.python(lengths).unwrap(), "[2.0, 4.0]");
    let height = "round(layout.cell('rdiv').dbbox().height(), 3)";
    assert_eq!(engine.python(height).unwrap(), "4.1");
    // The PCell draws twice as long: both instances follow.
    let text = std::fs::read_to_string(&file).unwrap();
    std::fs::write(
        &file,
        text.replace("pya.DBox(0, 0, l, w)", "pya.DBox(0, 0, 2 * l, w)"),
    )
    .unwrap();
    let later = std::time::SystemTime::now() + Duration::from_secs(2);
    std::fs::File::options()
        .write(true)
        .open(&file)
        .unwrap()
        .set_modified(later)
        .unwrap();
    let value = engine.call(method::REFRESH_PCELLS, json!({})).unwrap();
    assert_eq!(value["changed"], true, "{value}");
    assert_eq!(engine.python(height).unwrap(), "8.1");
    let out = dir.path().join("flat.gds");
    engine
        .call(method::EXPORT, json!({"out": out.display().to_string()}))
        .unwrap();
    let flat = std::fs::read(&out).unwrap();
    assert!(!flat.windows(18).any(|w| w == b"$$$CONTEXT_INFO$$$"));
    let summary = libcdept::layout::gds::read_structure(&rdiv).unwrap();
    assert_eq!(summary.top_cells(), ["rdiv"]);
    assert!(summary.ghosts().is_empty());
}
