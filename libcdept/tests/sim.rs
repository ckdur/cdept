//! A testbench run for real: examples/ihp-inv/tb_inv through xschem and ngspice at
//! three corners. Skipped (with a note) when ngspice, the xschem helper or the PDK is
//! not found. ngspice: `$CDEPT_NGSPICE` or PATH; the PDK: `$PDK_ROOT` or /opt/ext/OpenPDKs.

use std::path::Path;

use libcdept::Project;
use libcdept::sim::{self, Simulator, spec::Verdict};

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let target = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &target);
        } else {
            std::fs::copy(e.path(), target).unwrap();
        }
    }
}

#[test]
fn inverter_testbench_runs_at_three_corners() {
    let Some(simulator) = Simulator::find() else {
        eprintln!("skipped: ngspice not found");
        return;
    };
    if libcdept::xschem::Engine::find_helper().is_none() {
        eprintln!("skipped: the xschem helper is not built");
        return;
    }
    let root = std::env::var("PDK_ROOT").unwrap_or_else(|_| "/opt/ext/OpenPDKs".into());
    if !Path::new(&root).join("ihp-sg13g2/libs.tech/ngspice").is_dir() {
        eprintln!("skipped: ihp-sg13g2 not found in {root}");
        return;
    }
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/ihp-inv");
    let dir = tempfile::tempdir().unwrap();
    copy_dir(&example, dir.path());
    std::fs::write(dir.path().join("cdept.local.toml"), format!("pdk_root = \"{root}\"\n")).unwrap();
    let project = Project::load(dir.path()).unwrap();
    let tb = sim::open(&project, "cells", "tb_inv", "testbench").unwrap();
    let results = sim::run_testbench(&project, &tb, &simulator, &[]).unwrap();
    let names: Vec<String> = results.iter().map(|r| r.point.name()).collect();
    assert_eq!(names, ["tt_27", "ff_27", "ss_27"]);
    let mut tpd = vec![];
    for r in &results {
        let out = r.outcome.as_ref().unwrap_or_else(|e| panic!("{}: {e:#}", r.point.name()));
        assert!(out.raw.metadata().unwrap().len() > 1000);
        assert_eq!(r.specs[0].verdict, Verdict::Pass, "{:?}", r.specs);
        tpd.push(out.measures["tpd"]);
    }
    // Fast corner is faster than typical, slow corner slower.
    assert!(tpd[1] < tpd[0] && tpd[0] < tpd[2], "{tpd:?}");
}
