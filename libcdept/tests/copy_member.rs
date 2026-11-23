//! Copying a cell that is one cell of a multi-cell file (a standard cell of the PDK's
//! reference library) into a library of the project. Skipped when the PDK is absent.

use std::path::PathBuf;

use libcdept::layout::gds;
use libcdept::refactor::{self, Operation, Options};
use libcdept::Project;

const PDK: &str = "ihp-sg13g2";

fn pdk_root() -> Option<PathBuf> {
    let roots = std::env::var_os("PDK_ROOT")
        .map(PathBuf::from)
        .into_iter()
        .chain(["/foss/pdks", "/opt/ext/OpenPDKs"].map(PathBuf::from));
    roots
        .into_iter()
        .find(|r| r.join(PDK).join("libs.ref/sg13g2_stdcell/gds").is_dir())
}

#[test]
fn a_standard_cell_is_copied_as_files_of_its_own() {
    let Some(root) = pdk_root() else {
        eprintln!("skipped: the {PDK} PDK was not found");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("libs/cells")).unwrap();
    std::fs::write(
        tmp.path().join("cdept.toml"),
        format!(
            "name = \"t\"\ntechnology = \"{PDK}\"\npdk_root = \"{}\"\n[libraries]\ncells = \"libs/cells\"\n",
            root.display()
        ),
    )
    .unwrap();
    // The snapshot of the PDK (tech.toml) brings its libraries into the project.
    let mut project = Project::load(tmp.path()).unwrap();
    project.scan_technology(false).unwrap();
    let std = project
        .library("sg13g2_stdcell")
        .expect("the technology brings the standard cells");
    assert!(std
        .cell("sg13g2_inv_1")
        .is_some_and(|c| c.entries.iter().any(|e| e.is_member())));

    let operation = Operation::CopyCell {
        library: "sg13g2_stdcell".into(),
        cell: "sg13g2_inv_1".into(),
        to_library: "cells".into(),
        new: "inv_copy".into(),
        hierarchy: false,
    };
    let plan = refactor::plan(&project, &operation, &Options::default()).unwrap();
    let lines = plan.lines(tmp.path()).join("\n");
    assert!(
        lines.contains("libs/cells/inv_copy/layout/inv_copy.gds"),
        "{lines}"
    );
    assert!(lines.contains("note: view 'liberty'"), "{lines}");
    // Nothing is written by planning.
    assert!(!tmp.path().join("libs/cells/inv_copy").exists());
    refactor::apply(&project, &plan).unwrap();

    let layout = tmp.path().join("libs/cells/inv_copy/layout/inv_copy.gds");
    let summary = gds::read_structure(&layout).unwrap();
    assert_eq!(summary.top_cells(), ["inv_copy"]);
    assert!(summary.ghosts().is_empty());
    let source = gds::read_structure(
        &root
            .join(PDK)
            .join("libs.ref/sg13g2_stdcell/gds/sg13g2_stdcell.gds"),
    )
    .unwrap();
    assert!(summary.structures.len() < source.structures.len() / 10);
    assert!((summary.user_per_dbu - source.user_per_dbu).abs() < 1e-12);

    let netlist = std::fs::read_to_string(
        tmp.path()
            .join("libs/cells/inv_copy/netlist/inv_copy.spice"),
    )
    .unwrap();
    assert!(netlist.contains(".subckt inv_copy "), "{netlist}");
    assert_eq!(netlist.matches(".subckt").count(), 1, "{netlist}");

    // The project sees the copy as a cell of its own, not as a member.
    let project = Project::load(tmp.path()).unwrap();
    let copy = project.library("cells").unwrap().cell("inv_copy").unwrap();
    assert!(copy.entries.iter().all(|e| !e.is_member()));

    // The original still cannot be renamed, moved or deleted.
    let refused = refactor::plan(
        &project,
        &Operation::RenameCell {
            library: "sg13g2_stdcell".into(),
            cell: "sg13g2_inv_1".into(),
            new: "x".into(),
        },
        &Options::default(),
    )
    .unwrap_err()
    .to_string();
    assert!(refused.contains("holds several cells"), "{refused}");
}
