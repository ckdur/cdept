//! Real-PDK tests of KLayout LVS on the PDKs' standard cells, with no manual step: the
//! PDK's own SPICE netlist (devices written as `X` calls) is compared as it is, and the
//! switches of the deck come from the technology snapshot. Skipped (with a note) when
//! KLayout, the helper or the PDK is not found.

use std::path::{Path, PathBuf};

use libcdept::Project;
use libcdept::batch::netgen;
use libcdept::klayout::verify::{self, Check};
use libcdept::klayout::{Engine, EngineOptions, setup};
use libcdept::tech::{self, PdkDir, ScanContext};
use libcdept::testutil::{RealPdk, real_pdk};

fn tools() -> bool {
    if setup::find_klayout().is_none() || setup::find_helper_dir().is_none() {
        eprintln!("skipped: KLayout or the cdept-klayout helper was not found");
        return false;
    }
    true
}

/// The `.subckt` of `cell` in a SPICE file of a standard-cell library.
fn subckt(file: &Path, cell: &str) -> String {
    let text = std::fs::read_to_string(file).unwrap();
    let mut out = String::new();
    let mut inside = false;
    for line in text.lines() {
        let words: Vec<&str> = line.split_whitespace().collect();
        if words.len() > 1 && words[0].eq_ignore_ascii_case(".subckt") && words[1] == cell {
            inside = true;
        }
        if inside {
            out.push_str(line);
            out.push('\n');
            if words.first().is_some_and(|w| w.eq_ignore_ascii_case(".ends")) {
                break;
            }
        }
    }
    assert!(!out.is_empty(), "no .subckt {cell} in {}", file.display());
    out
}

/// A project of the PDK (with its snapshot) and the layout of `cell` cut out of `gds`.
fn project_with_cell(
    dir: &Path,
    real: &RealPdk,
    gds: &Path,
    cell: &str,
) -> (Project, PathBuf) {
    let pdk = PdkDir::new(real.root.clone(), &real.name);
    let (tech, catalog) = tech::scan(&ScanContext::new(pdk).files_only());
    std::fs::write(
        dir.join("cdept.toml"),
        format!(
            "name = \"t\"\ntechnology = \"{}\"\npdk_root = \"{}\"\n[libraries]\ncells = \"libs/cells\"\n",
            real.name,
            real.root.display()
        ),
    )
    .unwrap();
    tech::file::save(dir, &tech, &catalog).unwrap();
    let layout = dir.join(format!("libs/cells/{cell}/layout"));
    std::fs::create_dir_all(&layout).unwrap();
    let out = layout.join(format!("{cell}.gds"));
    let mut engine = Engine::start(EngineOptions::default()).unwrap();
    engine
        .python(&format!(
            "ly = pya.Layout(); ly.read({:?})\n\
             o = pya.Layout(); o.dbu = ly.dbu\n\
             o.create_cell({cell:?}).copy_tree(ly.cell({cell:?}))\n\
             o.write({:?})",
            gds.to_str().unwrap(),
            out.to_str().unwrap()
        ))
        .unwrap();
    engine.shutdown().unwrap();
    (Project::load(dir).unwrap(), out)
}

/// LVS of `cell` against the PDK's netlist as it is; the extracted netlist and the
/// project, for what comes next.
fn lvs_of(
    real: &RealPdk,
    gds: &Path,
    spice: &Path,
    cell: &str,
) -> (verify::LvsRun, Project, tempfile::TempDir) {
    let guard = tempfile::tempdir().unwrap();
    let tmp = guard.path().to_path_buf();
    let (project, layout) = project_with_cell(&tmp, real, gds, cell);
    let deck = verify::find_deck(&project, Check::Lvs).expect("the PDK has an LVS deck");
    let dir = tmp.join("lvs");
    std::fs::create_dir_all(&dir).unwrap();
    let stream = verify::export_stream(&project, &layout, cell, &dir.join(format!("{cell}.gds")))
        .unwrap();
    let netlist = tmp.join(format!("{cell}.spice"));
    std::fs::write(&netlist, subckt(spice, cell)).unwrap();
    let run = verify::run_lvs(&project, &deck, &stream, &netlist, &dir, &[]).unwrap();
    (run, project, guard)
}

#[test]
fn ihp_standard_cells_match_the_pdk_netlist_as_it_is() {
    if !tools() {
        return;
    }
    let Some(real) = real_pdk("ihp-sg13g2") else { return };
    let lib = real.dir.join("libs.ref/sg13g2_stdcell");
    let gds = lib.join("gds/sg13g2_stdcell.gds");
    let spice = lib.join("spice/sg13g2_stdcell.spice");
    if !gds.is_file() || !spice.is_file() {
        eprintln!("skipped: no sg13g2_stdcell library");
        return;
    }
    for cell in ["sg13g2_inv_1", "sg13g2_nand2_1"] {
        let (run, _, _guard) = lvs_of(&real, &gds, &spice, cell);
        assert!(run.summary.matched, "{cell}: {:?}", run.summary);
        assert_ne!(run.reference, run.schematic, "the PDK's file is not the deck's");
        assert!(
            std::fs::read_to_string(&run.schematic)
                .unwrap()
                .contains("XN0"),
            "the original netlist is not edited"
        );
        assert!(run.notes.iter().any(|n| n.contains("written as devices")), "{:?}", run.notes);
    }
}

#[test]
fn sky130_standard_cell_matches_with_the_deck_variables_of_the_technology() {
    if !tools() {
        return;
    }
    let Some(real) = real_pdk("sky130A") else { return };
    let lib = real.dir.join("libs.ref/sky130_fd_sc_hd");
    let gds = lib.join("gds/sky130_fd_sc_hd.gds");
    let spice = lib.join("spice/sky130_fd_sc_hd.spice");
    if !gds.is_file() || !spice.is_file() {
        eprintln!("skipped: no sky130_fd_sc_hd library");
        return;
    }
    let (run, project, _) = lvs_of(&real, &gds, &spice, "sky130_fd_sc_hd__inv_1");
    assert!(run.summary.matched, "{:?}", run.summary);
    assert_eq!(
        project.klayout.lvs_variables.get("scale").map(String::as_str),
        Some("true")
    );
    assert!(run.notes.iter().any(|n| n.contains("scale=true")), "{:?}", run.notes);
}

#[test]
fn netgen_accepts_the_netlist_klayout_extracted() {
    if !tools() {
        return;
    }
    let Ok(program) = netgen::ADAPTER.require() else {
        eprintln!("skipped: netgen not found");
        return;
    };
    let Some(real) = real_pdk("ihp-sg13g2") else { return };
    let lib = real.dir.join("libs.ref/sg13g2_stdcell");
    let gds = lib.join("gds/sg13g2_stdcell.gds");
    let spice = lib.join("spice/sg13g2_stdcell.spice");
    if !gds.is_file() || !spice.is_file() {
        eprintln!("skipped: no sg13g2_stdcell library");
        return;
    }
    let cell = "sg13g2_inv_1";
    let (run, project, tmp) = lvs_of(&real, &gds, &spice, cell);
    let tmp = tmp.path();
    let dir = tmp.join("netgen");
    let (layout, note) = netgen::prepare_netlist(&project, &run.extracted, &dir).unwrap();
    assert!(note.is_some());
    let pdk_netlist = tmp.join("pdk.spice");
    std::fs::write(&pdk_netlist, subckt(&spice, cell)).unwrap();
    let (schematic, note) = netgen::prepare_netlist(&project, &pdk_netlist, &dir).unwrap();
    assert!(note.is_none(), "the PDK's netlist has calls already");
    let lvs = netgen::Lvs {
        layout,
        layout_cell: cell.into(),
        schematic,
        schematic_cell: cell.into(),
        setup: netgen::project_setup(&project),
    };
    let report = netgen::run(&program, &lvs, &dir, &project.tool_env()).unwrap();
    assert!(report.verdict.clean(), "{report:?}");
}
