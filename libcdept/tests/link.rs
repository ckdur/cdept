//! Linking schematics with layouts (docs/LINKING.md), with the real KLayout helper.
//! Skipped when KLayout is not found; the IHP test (`real_pdk::`) also needs the IHP
//! SG13G2 PDK.

use std::path::{Path, PathBuf};

use libcdept::Project;
use libcdept::klayout::{Engine, setup};
use libcdept::link::{self, ApplyOptions, NAME_PROPERTY};

fn available() -> bool {
    let ok = setup::find_klayout().is_some() && setup::find_helper_dir().is_some();
    if !ok {
        eprintln!("skipped: KLayout or the cdept-klayout helper was not found");
    }
    ok
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

fn example(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../examples")
        .join(name)
}

/// Names bound in the top cell of a layout file, sorted.
fn bound(file: &Path, top: &str) -> Vec<String> {
    let mut names: Vec<String> = libcdept::layout::read_placements(file, NAME_PROPERTY)
        .unwrap()
        .into_iter()
        .filter(|p| p.structure == top)
        .filter_map(|p| p.value)
        .collect();
    names.sort();
    names
}

/// The buffer example: buf's schematic places inv twice and its layout has two inv
/// instances that are not linked yet. Generating takes them over instead of adding.
#[test]
fn buffer_links_its_existing_instances() {
    let _env = libcdept::testutil::hermetic_env();
    if !available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    copy_tree(&example("buffer"), dir.path());
    let project = Project::load(dir.path()).unwrap();
    let (map, sch) = link::prepare(&project, "logic", "buf", None).unwrap();
    let names: Vec<&str> = sch.wanted.iter().map(|w| w.name.as_str()).collect();
    assert_eq!(names, ["x1", "x2"]);
    assert_eq!(sch.pins.len(), 4);
    let file = libcdept::klayout::cell_layout(project.library("logic").unwrap(), "buf").unwrap();
    let before = link::check(&sch, &file).unwrap();
    assert_eq!(before.add, ["x1", "x2"]);
    assert_eq!(before.unbound, 2);
    assert!(!before.in_sync());

    let mut engine = Engine::for_project(&project).unwrap();
    let applied = link::generate(
        &project,
        &mut engine,
        &map,
        &sch,
        &file,
        &ApplyOptions::default(),
    )
    .unwrap();
    engine.shutdown().unwrap();
    assert_eq!(applied.bound, ["x1", "x2"], "{applied}");
    assert!(applied.added.is_empty(), "{applied}");
    assert_eq!(bound(&file, "buf"), ["x1", "x2"]);
    let after = link::check(&sch, &file).unwrap();
    assert!(after.in_sync(), "{after}");
    // inv's own schematic has generic devices only: nothing in the device map.
    let (_, inv) = link::prepare(&project, "logic", "inv", None).unwrap();
    assert!(inv.wanted.is_empty());
    assert_eq!(inv.problems.len(), 2, "{:?}", inv.problems);
}

/// Tests that need an installed PDK.
mod real_pdk {
    use super::*;

    const PDK: &str = "ihp-sg13g2";

    /// The IHP example: generate the inverter from its schematic (two SG13_dev PCells),
    /// change a width in the schematic, update.
    #[test]
    fn ihp_inverter_is_generated_and_updated() {
        if !available() {
            return;
        }
        let Some(pdk) = libcdept::testutil::real_pdk(PDK) else {
            return;
        };
        let root = &pdk.root;
        let dir = tempfile::tempdir().unwrap();
        copy_tree(&example("ihp-inv"), dir.path());
        for cell in ["inv", "buf"] {
            let _ = std::fs::remove_dir_all(dir.path().join(format!("libs/cells/{cell}/layout")));
        }
        let toml = dir.path().join("cdept.toml");
        let text = std::fs::read_to_string(&toml).unwrap().replace(
            "# pdk_root = \"/opt/ext/OpenPDKs\"",
            &format!("pdk_root = \"{}\"", root.display()),
        );
        std::fs::write(&toml, text).unwrap();
        let mut project = Project::load(dir.path()).unwrap();

        let (map, sch) = link::prepare(&project, "cells", "inv", None).unwrap();
        assert!(sch.problems.is_empty(), "{:?}", sch.problems);
        let devices: Vec<(String, String)> = sch
            .wanted
            .iter()
            .map(|w| (w.name.clone(), w.target.label()))
            .collect();
        assert_eq!(
            devices,
            [
                ("M2".to_string(), "SG13_dev/pmos".to_string()),
                ("M1".to_string(), "SG13_dev/nmos".to_string())
            ]
        );
        let file = link::layout_view(&mut project, "cells", "inv", None, true).unwrap();
        let mut engine = Engine::for_project(&project).unwrap();
        let applied = link::generate(
            &project,
            &mut engine,
            &map,
            &sch,
            &file,
            &ApplyOptions::default(),
        )
        .unwrap();
        assert_eq!(applied.added, ["M1", "M2"], "{applied}");
        assert_eq!(applied.pins.len(), 4);
        assert_eq!(bound(&file, "inv"), ["M1", "M2"]);
        assert!(link::check(&sch, &file).unwrap().in_sync());
        let w = engine
            .python(
                "ly = pya.Layout(); ly.technology_name = 'sg13g2'; ly.read(doc.root)\n\
                 sorted((i.property(61), i.pcell_parameters_by_name()['w']) \
                 for i in ly.cell('inv').each_inst())",
            )
            .unwrap();
        assert_eq!(w, "[('M1', '0.74u'), ('M2', '1.5u')]");

        // A wider NMOS in the schematic: the plan says update, generate changes it.
        let sch_file = &sch.file;
        let text = std::fs::read_to_string(sch_file)
            .unwrap()
            .replace("w=0.74u", "w=1u");
        std::fs::write(sch_file, text).unwrap();
        let (map, sch) = link::prepare(&project, "cells", "inv", None).unwrap();
        let plan = link::check(&sch, &file).unwrap();
        assert_eq!(plan.update.len(), 1, "{plan}");
        assert!(plan.update[0].what.contains("w 0.74u → 1u"), "{plan}");
        let applied = link::generate(
            &project,
            &mut engine,
            &map,
            &sch,
            &file,
            &ApplyOptions::default(),
        )
        .unwrap();
        assert_eq!(applied.updated, ["M1"], "{applied}");
        assert_eq!(applied.pins.len(), 0, "{applied}");
        assert!(link::check(&sch, &file).unwrap().in_sync());

        // buf: two references to inv's layout.
        project.rescan();
        let (map, buf) = link::prepare(&project, "cells", "buf", None).unwrap();
        let labels: Vec<String> = buf.wanted.iter().map(|w| w.target.label()).collect();
        assert_eq!(labels, ["cells/inv/layout", "cells/inv/layout"]);
        let buf_file = link::layout_view(&mut project, "cells", "buf", None, true).unwrap();
        let applied = link::generate(
            &project,
            &mut engine,
            &map,
            &buf,
            &buf_file,
            &ApplyOptions::default(),
        )
        .unwrap();
        engine.shutdown().unwrap();
        assert_eq!(applied.added, ["x1", "x2"], "{applied}");
        let meta = libcdept::layout::Meta::read_for(&buf_file)
            .unwrap()
            .unwrap();
        assert_eq!(meta.references["inv"].cell, "inv");
        assert_eq!(meta.instances["x1"].kind, "cell");
        assert!(link::check(&buf, &buf_file).unwrap().in_sync());
    }
}
