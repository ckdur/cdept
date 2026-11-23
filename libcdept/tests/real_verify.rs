//! Real-PDK tests: DRC and LVS for real, KLayout runs the IHP SG13G2 decks on the PDK's own
//! inverter. Skipped (with a note) when KLayout, the helper or the PDK is not found, as in
//! CI. The PDK is looked for in `$CDEPT_TEST_PDK_ROOT`, else `/opt/ext/OpenPDKs`.

use std::path::{Path, PathBuf};

use libcdept::testutil::{RealPdk, real_pdk};

use libcdept::Project;
use libcdept::klayout::verify::{self, Check};
use libcdept::klayout::{Engine, EngineOptions, setup};

const PDK: &str = "ihp-sg13g2";
const CELL: &str = "sg13g2_inv_1";

/// The PDK folder, the inverter's test case folder (layout/, netlist/) and the guard that
/// keeps the environment for the PDK.
fn pdk() -> Option<(PathBuf, PathBuf, RealPdk)> {
    if setup::find_klayout().is_none() || setup::find_helper_dir().is_none() {
        eprintln!("skipped: KLayout or the cdept-klayout helper was not found");
        return None;
    }
    let real = real_pdk(PDK)?;
    let case = real.dir.join(format!(
        "libs.tech/klayout/tech/lvs/testing/testcases/sg13g2_cells/{CELL}"
    ));
    if !case.join("layout").is_dir() {
        eprintln!("skipped: the {PDK} PDK has no KLayout LVS test cases");
        return None;
    }
    Some((real.root.clone(), case, real))
}

/// A project using the PDK, with library `std` holding the inverter's layout.
fn project(dir: &Path, root: &Path, case: &Path) -> Project {
    let layout = dir.join(format!("libs/std/{CELL}/layout"));
    std::fs::create_dir_all(&layout).unwrap();
    std::fs::copy(
        case.join(format!("layout/{CELL}.gds")),
        layout.join(format!("{CELL}.gds")),
    )
    .unwrap();
    std::fs::write(
        dir.join("cdept.toml"),
        format!(
            "name = \"t\"\ntechnology = \"{PDK}\"\npdk_root = \"{}\"\n[libraries]\nstd = \"libs/std\"\n",
            root.display()
        ),
    )
    .unwrap();
    Project::load(dir).unwrap()
}

fn layout_file(project: &Project) -> PathBuf {
    libcdept::klayout::cell_layout(project.library("std").unwrap(), CELL).unwrap()
}

#[test]
fn drc_of_the_pdk_inverter_and_of_a_narrow_wire() {
    let Some((root, case, _env)) = pdk() else { return };
    let tmp = tempfile::tempdir().unwrap();
    let project = project(tmp.path(), &root, &case);
    let deck = verify::find_deck(&project, Check::Drc).expect("the PDK has a DRC deck");
    assert!(
        deck.ends_with("tech/drc/ihp-sg13g2.drc"),
        "{}",
        deck.display()
    );

    let dir = tmp.path().join("drc");
    std::fs::create_dir_all(&dir).unwrap();
    let stream =
        verify::export_stream(&project, &layout_file(&project), CELL, &dir.join("inv.gds"))
            .unwrap();
    assert_eq!(stream.top, CELL);
    assert!(stream.missing.is_empty());
    let run = verify::run_drc(&project, &deck, &stream, &dir, &[]).unwrap();
    assert_eq!(run.rdb.top_cell, CELL);
    assert!(!run.rdb.categories.is_empty());

    // A Metal1 wire 50 nm wide (the rule is 160 nm).
    let wire = dir.join("wire.gds");
    let mut engine = Engine::start(EngineOptions::default()).unwrap();
    engine
        .python(&format!(
            "ly = pya.Layout(); ly.dbu = 0.001; c = ly.create_cell('wire')\n\
             c.shapes(ly.layer(8, 0)).insert(pya.Box(0, 0, 50, 5000))\n\
             ly.write({:?})",
            wire.to_str().unwrap()
        ))
        .unwrap();
    engine.shutdown().unwrap();
    let stream = verify::Stream {
        file: wire,
        top: "wire".into(),
        missing: Vec::new(),
    };
    let run = verify::run_drc(&project, &deck, &stream, &dir, &[]).unwrap();
    let width: Vec<_> = run
        .rdb
        .counts()
        .into_iter()
        .filter(|(c, _)| c.name() == "M1.a")
        .collect();
    assert_eq!(width.len(), 1, "{:?}", run.rdb.counts());
    let path = width[0].0.path.clone();
    let item = run.rdb.items_of(&path).next().unwrap();
    let [l, b, r, t] = item.bbox().unwrap();
    assert!(
        l >= -0.001 && r <= 0.051 && b >= -0.001 && t <= 5.001,
        "{l} {b} {r} {t}"
    );
}

#[test]
fn lvs_of_the_pdk_inverter() {
    let Some((root, case, _env)) = pdk() else { return };
    let tmp = tempfile::tempdir().unwrap();
    let project = project(tmp.path(), &root, &case);
    let deck = verify::find_deck(&project, Check::Lvs).expect("the PDK has an LVS deck");
    let dir = tmp.path().join("lvs");
    std::fs::create_dir_all(&dir).unwrap();
    let stream =
        verify::export_stream(&project, &layout_file(&project), CELL, &dir.join("inv.gds"))
            .unwrap();
    let netlist = case.join(format!("netlist/{CELL}.cdl"));

    let run = verify::run_lvs(&project, &deck, &stream, &netlist, &dir, &[]).unwrap();
    assert!(run.summary.matched, "{:?}", run.summary);
    assert!(run.extracted.is_file());

    // The output wire of the schematic renamed: the nets cannot match.
    let text = std::fs::read_to_string(&netlist).unwrap();
    let bad = dir.join("bad.cdl");
    std::fs::write(&bad, text.replace("MX1 Y A VSS", "MX1 Q A VSS")).unwrap();
    let run = verify::run_lvs(&project, &deck, &stream, &bad, &dir, &[]).unwrap();
    assert!(!run.summary.matched);
    let circuit = &run.summary.circuits[0];
    assert_eq!(circuit.layout, CELL);
    assert!(circuit.pairs().count() > 0, "{circuit:?}");
}
