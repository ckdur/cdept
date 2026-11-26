//! Magic views with a real Magic and sky130A (examples/sky130-inv): layout from
//! schematic with the PDK's device generators, check-source, GDS export and import,
//! the references, LVS with netgen. Skipped when Magic or the PDK is missing; LVS also
//! needs the xschem helper ($CDEPT_XSCHEM) and netgen.

use std::fs;
use std::path::{Path, PathBuf};

use libcdept::Project;
use libcdept::magic::{Magic, Source, flows, refs};

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for e in fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let target = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &target);
        } else {
            fs::copy(e.path(), &target).unwrap();
        }
    }
}

fn setup() -> Option<(tempfile::TempDir, PathBuf, libcdept::testutil::RealPdk)> {
    let magic = PathBuf::from("/opt/ext/OpenLane/bin/magic");
    if Magic::find().is_none() && !magic.is_file() {
        eprintln!("skipped: no magic");
        return None;
    }
    let pdk = libcdept::testutil::real_pdk("sky130A")?;
    if std::env::var_os("CDEPT_MAGIC").is_none() {
        // SAFETY: the PDK guard serialises environment changes.
        unsafe { std::env::set_var("CDEPT_MAGIC", &magic) };
    }
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("p");
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/sky130-inv");
    copy_dir(&example, &dir);
    let _ = fs::remove_dir_all(dir.join("libs/cells/inv/layout"));
    fs::write(
        dir.join("cdept.local.toml"),
        format!("pdk_root = \"{}\"\n", pdk.root.display()),
    )
    .unwrap();
    Some((tmp, dir, pdk))
}

#[test]
fn real_generate_update_export_import_and_refs() {
    let Some((_tmp, dir, _pdk)) = setup() else {
        return;
    };
    let p = Project::load(&dir).unwrap();
    let magic = Magic::for_project(&p).unwrap();

    // Out of sync: the cell has no layout yet (check-source does not make one).
    assert!(flows::generate(&magic, &p, "cells", "inv", None, false, true).is_err());

    // Generate: both devices, named as in the schematic.
    let g = flows::generate(&magic, &p, "cells", "inv", None, false, false).unwrap();
    assert_eq!(g.plan.actions.len(), 2, "{:?}", g.plan.lines());
    let p = Project::load(&dir).unwrap();
    let placed = libcdept::magic::link::placed(&p, &g.layout).unwrap();
    let mut names: Vec<&str> = placed.iter().map(|x| x.instance.as_str()).collect();
    names.sort();
    assert_eq!(names, ["M1", "M2"]);
    let m1 = placed.iter().find(|x| x.instance == "M1").unwrap();
    assert_eq!(
        m1.generator,
        Some(("sky130".to_string(), "sky130_fd_pr__nfet_01v8".to_string()))
    );
    assert_eq!(m1.params["w"], "1");
    let check = flows::generate(&magic, &p, "cells", "inv", None, false, true).unwrap();
    assert!(check.plan.in_sync(), "{:?}", check.plan.lines());

    // The schematic changes: one update, made in place; the old device cell goes.
    let sch = dir.join("libs/cells/inv/schematic/inv.sch");
    let text = fs::read_to_string(&sch).unwrap().replacen("W=1\n", "W=2.5\n", 1);
    fs::write(&sch, text).unwrap();
    let check = flows::generate(&magic, &p, "cells", "inv", None, false, true).unwrap();
    assert_eq!(check.plan.actions.len(), 1);
    flows::generate(&magic, &p, "cells", "inv", None, false, false).unwrap();
    let p = Project::load(&dir).unwrap();
    let check = flows::generate(&magic, &p, "cells", "inv", None, false, true).unwrap();
    assert!(check.plan.in_sync(), "{:?}", check.plan.lines());
    let devices = fs::read_dir(dir.join("libs/cells/inv/layout"))
        .unwrap()
        .filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().starts_with("sky130_fd_pr"))
        .count();
    assert_eq!(devices, 2, "the old version of M1 is removed");
    // The sidecar records the link.
    let meta = libcdept::layout::meta::Meta::read_for(&g.layout).unwrap().unwrap();
    assert_eq!(meta.source.unwrap().cell, "inv");
    assert_eq!(meta.instances["M1"].params["w"], "2.5");

    // GDS out, and into another library as Magic views with relative paths.
    let gds = dir.join("out/inv.gds");
    flows::export_gds(&magic, &Source::Mag { file: g.layout.clone() }, &gds).unwrap();
    assert!(fs::metadata(&gds).unwrap().len() > 1000);
    fs::write(
        dir.join("cdept.toml"),
        fs::read_to_string(dir.join("cdept.toml"))
            .unwrap()
            .replace("[libraries]\n", "[libraries]\nimported = { path = \"libs/imported\", layout = \"cells\" }\n"),
    )
    .unwrap();
    fs::create_dir_all(dir.join("libs/imported")).unwrap();
    let p = Project::load(&dir).unwrap();
    let lib = p.library("imported").unwrap();
    let done = flows::import_gds(&magic, &p, &gds, lib, &["inv".into()], false).unwrap();
    assert!(done.cells.contains(&"inv".to_string()), "{done:?}");
    assert!(done.cells.len() >= 3, "the devices come along: {done:?}");
    let p = Project::load(&dir).unwrap();
    let inv = dir.join("libs/imported/inv/layout/inv.mag");
    let r = refs::resolve_file(&p, &inv).unwrap();
    assert!(r.iter().all(|x| matches!(x.target, refs::Target::Project(_))), "{r:?}");
    assert!(refs::check(&p, false).is_empty(), "{:?}", refs::check(&p, false));
    // Again: the cells are there already.
    let again = flows::import_gds(&magic, &p, &gds, lib, &["inv".into()], false).unwrap();
    assert!(again.cells.is_empty() && !again.skipped.is_empty());
}

#[test]
fn real_lvs_runs_and_reports() {
    let Some((_tmp, dir, _pdk)) = setup() else {
        return;
    };
    if std::env::var_os("CDEPT_XSCHEM").is_none() || libcdept::batch::netgen::ADAPTER.require().is_err() {
        eprintln!("skipped: no xschem helper or netgen");
        return;
    }
    let p = Project::load(&dir).unwrap();
    let magic = Magic::for_project(&p).unwrap();
    flows::generate(&magic, &p, "cells", "inv", None, false, false).unwrap();
    let p = Project::load(&dir).unwrap();
    let r = flows::lvs(&magic, &p, "cells", "inv").unwrap();
    // The devices are placed, not wired: netgen finds the nets different.
    assert!(!r.report.verdict.clean(), "{}", r.report.result);
    assert_eq!(r.report.devices, Some((2, 2)), "{:?}", r.report);
    assert!(r.layout_netlist.is_file() && r.schematic_netlist.is_file());
}
