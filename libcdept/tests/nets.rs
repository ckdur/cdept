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
    let stream =
        verify::export_stream(project, &file, cell, &dir.join(format!("{cell}.gds"))).unwrap();
    let pins = libcdept::layout::Meta::read_for(&file)
        .unwrap()
        .map(|m| m.pins)
        .unwrap_or_default();
    link::nets::run(project, source, &schematic, &stream, &dir, &[], &pins).unwrap()
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

/// A second nmos in parallel with M1 (S and D the other way round) is found as a
/// parallel group from xschem's own connectivity, and recorded in the sidecar.
#[test]
fn parallel_devices_are_found_in_the_schematic() {
    let Some(root) = pdk_root() else {
        return;
    };
    let (dir, project) = example(&root);
    let sch = dir.path().join("libs/cells/inv/schematic/inv.sch");
    let mut text = std::fs::read_to_string(&sch).unwrap();
    text.push_str(
        "C {lab_pin.sym} 100 20 0 0 {name=l5 sig_type=std_logic lab=VSS}\n\
         C {lab_pin.sym} 100 80 0 0 {name=l6 sig_type=std_logic lab=Y}\n\
         C {lab_pin.sym} 60 50 0 0 {name=l7 sig_type=std_logic lab=A}\n\
         C {lab_pin.sym} 100 50 2 0 {name=l8 sig_type=std_logic lab=VSS}\n\
         C {sg13g2_pr/sg13_lv_nmos.sym} 80 50 0 0 {name=M3\nl=0.13u\nw=0.74u\nng=1\nm=1\n\
         model=sg13_lv_nmos\nspiceprefix=X\n}\n",
    );
    std::fs::write(&sch, text).unwrap();
    let (source, file) = link::schematic_view(&project, "cells", "inv", None).unwrap();
    let map = link::DeviceMap::for_project(&project).unwrap();
    let flat = link::nets::read_flat(&project, &map, source, &file).unwrap();
    let groups = link::nets::parallel_groups(&flat);
    assert_eq!(
        groups,
        vec![vec!["M1".to_string(), "M3".into()]],
        "{flat:#?}"
    );
    let layout = libcdept::klayout::cell_layout(project.library("cells").unwrap(), "inv").unwrap();
    let merged = link::nets::merge_parallel(&groups, &link::nets::recorded_parallel(&layout));
    link::nets::record_parallel(&layout, &merged).unwrap();
    assert_eq!(link::nets::recorded_parallel(&layout), merged);
}

/// The example's pins (docs/LINKING.md §9): in the sidecar, on IHP's pin layers in the
/// stream; the net check takes them (A and Y on their wires, VDD and VSS on none); a
/// pin moved onto the VSS wire is found there; IHP's LVS sees the pins' labels.
#[test]
fn pins_are_in_the_sidecar_the_stream_and_the_checks() {
    let Some(root) = pdk_root() else {
        return;
    };
    let (dir, project) = example(&root);
    let file = dir.path().join("libs/cells/inv/layout/inv.gds");
    let meta = libcdept::layout::Meta::read_for(&file).unwrap().unwrap();
    assert_eq!(
        meta.pins.keys().map(String::as_str).collect::<Vec<_>>(),
        ["A", "VDD", "VSS", "Y"]
    );
    assert_eq!(meta.pins["A"].direction, "input");
    let map = link::DeviceMap::for_project(&project).unwrap();
    let rules = map.pins.rules();
    let (source, sch_file) = link::schematic_view(&project, "cells", "inv", None).unwrap();
    let sch = link::read_schematic(&project, &map, source, &sch_file).unwrap();
    let wanted: Vec<link::pins::Wanted<'_>> = sch
        .pins
        .iter()
        .map(|p| link::pins::Wanted {
            name: &p.name,
            direction: &p.direction,
        })
        .collect();
    let checks = link::pins::check_file(&file, Some(&meta), Some(&wanted), Some(&rules)).unwrap();
    assert!(checks.iter().all(|c| c.is_ok()), "{checks:#?}");
    let tagged = link::pins::tagged_in_stream(&file, "inv", &rules)
        .unwrap()
        .unwrap();
    assert_eq!(tagged.len(), 4);

    let work = tempfile::tempdir().unwrap();
    let run = check(&project, "inv", work.path());
    let status: std::collections::BTreeMap<&str, &str> = run
        .report
        .pins
        .iter()
        .map(|p| (p.name.as_str(), p.status.as_str()))
        .collect();
    assert_eq!(status["A"], "ok", "{status:?}");
    assert_eq!(status["Y"], "ok");
    assert_eq!(status["VSS"], "the pin is on no wire");

    // VSS moved onto M1's source (the Metal1 of the nmos' S terminal): on its net now.
    let source = run
        .connectivity
        .devices
        .iter()
        .find(|d| d.class.contains("nmos"))
        .and_then(|d| d.terminals.iter().find(|t| t.name == "S"))
        .and_then(|t| t.point)
        .expect("the nmos has a source");
    let mut moved = meta.clone();
    let pin = moved.pins.get_mut("VSS").unwrap();
    let (x, y) = (source[0], source[1]);
    pin.shapes[0].r#box = Some([x - 0.05, y - 0.05, x + 0.05, y + 0.05]);
    pin.label = Some([x, y]);
    moved.write_for(&file).unwrap();
    let stream_dir = work.path().join("moved");
    std::fs::create_dir_all(&stream_dir).unwrap();
    let stream = verify::export_stream(&project, &file, "inv", &stream_dir.join("inv.gds")).unwrap();
    let (source_view, schematic) = link::schematic_view(&project, "cells", "inv", None).unwrap();
    let run = link::nets::run(
        &project,
        source_view,
        &schematic,
        &stream,
        &stream_dir,
        &[],
        &moved.pins,
    )
    .unwrap();
    let vss = run.report.pins.iter().find(|p| p.name == "VSS").unwrap();
    assert_eq!(vss.status, "ok", "{:#?}", run.report.pins);

    // LVS: the extracted inverter has the pins A and Y (the labels on their wires).
    let deck = verify::find_deck(&project, verify::Check::Lvs).unwrap();
    let lvs_dir = work.path().join("lvs");
    std::fs::create_dir_all(&lvs_dir).unwrap();
    let stream = verify::export_stream(&project, &file, "inv", &lvs_dir.join("inv.gds")).unwrap();
    let netlist = lvs_dir.join("inv.spice");
    verify::schematic_netlist(&project, &schematic, &netlist).unwrap();
    let lvs = verify::run_lvs(&project, &deck, &stream, &netlist, &lvs_dir, &[]).unwrap();
    let extracted = std::fs::read_to_string(&lvs.extracted).unwrap();
    let subckt = extracted
        .lines()
        .find(|l| l.to_ascii_lowercase().starts_with(".subckt inv"))
        .unwrap_or_default()
        .to_string();
    for pin in ["A", "Y"] {
        assert!(subckt.split_whitespace().any(|w| w == pin), "{subckt}");
    }
}
