//! Nets of a layout against its schematic (docs/LINKING.md §8), with the real KLayout,
//! xschem and the IHP SG13G2 PDK (its LVS deck extracts the layout). Skipped when one
//! of them is not found.

use std::path::{Path, PathBuf};

use libcdept::Project;
use libcdept::klayout::{Engine, setup, verify};
use libcdept::link::{self, nets::NetStatus};

const PDK: &str = "ihp-sg13g2";

fn pdk_root() -> Option<PathBuf> {
    if setup::find_klayout().is_none() || setup::find_helper_dir().is_none() {
        eprintln!("skipped: KLayout or the cdept-klayout helper was not found");
        return None;
    }
    if libcdept::xschem::Engine::find_helper().is_none() {
        // The test runs from target/<profile>/deps; the helper is built next to it
        // (cargo build -p cdept-xschem, or a workspace `cargo test`).
        let built = std::env::current_exe()
            .ok()
            .and_then(|exe| Some(exe.parent()?.parent()?.join("cdept-xschem")))
            .filter(|p| p.is_file());
        match built {
            // SAFETY: set before any engine starts; every test sets the same value.
            Some(path) => unsafe { std::env::set_var("CDEPT_XSCHEM", path) },
            None => {
                eprintln!("skipped: the xschem helper was not found (cargo build -p cdept-xschem)");
                return None;
            }
        }
    }
    let roots = [
        std::env::var_os("PDK_ROOT").map(PathBuf::from),
        Some(PathBuf::from("/opt/ext/OpenPDKs")),
    ];
    let found = roots
        .into_iter()
        .flatten()
        .find(|r| r.join(PDK).join("libs.tech/klayout").is_dir());
    if found.is_none() {
        eprintln!("skipped: the {PDK} PDK was not found");
    }
    found
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

fn example(root: &Path) -> (tempfile::TempDir, Project) {
    let dir = tempfile::tempdir().unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/ihp-inv");
    copy_tree(&source, dir.path());
    let toml = dir.path().join("cdept.toml");
    let text = std::fs::read_to_string(&toml).unwrap().replace(
        "# pdk_root = \"/opt/ext/OpenPDKs\"",
        &format!("pdk_root = \"{}\"", root.display()),
    );
    std::fs::write(&toml, text).unwrap();
    let project = Project::load(dir.path()).unwrap();
    (dir, project)
}

fn check(project: &Project, cell: &str, work: &Path) -> link::nets::NetRun {
    let (source, schematic) = link::schematic_view(project, "cells", cell, None).unwrap();
    let file = libcdept::klayout::cell_layout(project.library("cells").unwrap(), cell).unwrap();
    let dir = work.join(cell);
    std::fs::create_dir_all(&dir).unwrap();
    let stream = verify::export_stream(project, &file, &dir.join(format!("{cell}.gds"))).unwrap();
    link::nets::run(project, source, &schematic, &stream, &dir).unwrap()
}

/// The example's inverter is wired at A and Y, not at VDD and VSS: two nets ok, two
/// open with flylines; with a wire from A to Y, a short.
#[test]
fn the_inverter_has_opens_and_then_a_short() {
    let Some(root) = pdk_root() else {
        return;
    };
    let (dir, project) = example(&root);
    let work = tempfile::tempdir().unwrap();
    let run = check(&project, "inv", work.path());
    let report = &run.report;
    let status = |name: &str| report.net(name).map(|n| n.status);
    assert_eq!(status("A"), Some(NetStatus::Ok), "{:#?}", report);
    assert_eq!(status("Y"), Some(NetStatus::Ok));
    for net in ["VDD", "VSS"] {
        let result = report.net(net).unwrap();
        assert_eq!(result.status, NetStatus::Open);
        assert_eq!(result.pieces.len(), 2);
        assert_eq!(result.flylines.len(), 1, "{net}");
    }
    let pins: Vec<(&str, &str)> = report
        .pins
        .iter()
        .map(|p| (p.name.as_str(), p.status.as_str()))
        .collect();
    assert!(
        pins.contains(&("A", "ok")) && pins.contains(&("Y", "ok")),
        "{pins:?}"
    );
    assert!(report.shorts.is_empty() && report.unplaced.is_empty());
    assert!(!report.clean());
    assert!(run.lvsdb.is_file());

    // A wire from A to Y (examples/ihp-inv/scripts/wire-inv.py short=1).
    let script = dir.path().join("scripts/wire-inv.py");
    let file = dir.path().join("libs/cells/inv/layout/inv.gds");
    let mut engine = Engine::for_project(&project).unwrap();
    engine
        .python(&format!(
            "exec(open({:?}).read(), {{'file': {:?}, 'short': 1}})",
            script.display().to_string(),
            file.display().to_string()
        ))
        .unwrap();
    engine.shutdown().unwrap();
    let run = check(&project, "inv", work.path());
    let report = &run.report;
    assert_eq!(report.shorts.len(), 1, "{:#?}", report.shorts);
    let mut joined = report.shorts[0].schematic_nets.clone();
    joined.sort();
    assert_eq!(joined, ["A", "Y"]);
    assert!(matches!(
        report.net("A").unwrap().status,
        NetStatus::Short | NetStatus::OpenShort
    ));
}

/// buf places the wired inverter twice: the net between them (x1's Y, x2's A) is open
/// between the two cells, with a flyline from one to the other.
#[test]
fn the_buffer_is_open_between_its_cells() {
    let Some(root) = pdk_root() else {
        return;
    };
    let (_dir, project) = example(&root);
    let work = tempfile::tempdir().unwrap();
    let run = check(&project, "buf", work.path());
    let report = &run.report;
    let middle = report
        .nets
        .iter()
        .find(|n| {
            n.pieces.iter().any(|p| {
                p.terminals
                    .iter()
                    .any(|t| t.path == "x2/M1" && t.pin == "G")
            })
        })
        .expect("x2's gate is on a net");
    assert_eq!(middle.status, NetStatus::Open, "{middle:#?}");
    assert_eq!(middle.pieces.len(), 2);
    assert_eq!(middle.flylines.len(), 1);
    let line = middle.flylines[0];
    // From x1 (x < 5.43 µm) to x2 (x > 7.43 µm).
    let (left, right) = (line.from[0].min(line.to[0]), line.from[0].max(line.to[0]));
    assert!(left < 5.5 && right > 7.4, "{line:?}");
    assert_eq!(report.net("A").map(|n| n.status), Some(NetStatus::Ok));
    assert!(report.unplaced.is_empty(), "{:?}", report.unplaced);
}
