//! A testbench run for real: examples/ihp-inv/tb_inv through xschem and ngspice at
//! three corners, a supply sweep and Monte Carlo, with the results stored. Skipped
//! (with a note) when ngspice, the xschem helper or the PDK is not found. ngspice:
//! `$CDEPT_NGSPICE` or PATH; the PDK: `$PDK_ROOT` or /opt/ext/OpenPDKs.

use std::path::Path;

use libcdept::Project;
use libcdept::sim::results::{self, RunRecord, Store};
use libcdept::sim::{self, RunOptions, Simulator, spec::Verdict};

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

/// A scratch copy of the example project, or None (with a note) when the tools are missing.
fn open_example() -> Option<(tempfile::TempDir, Project, Simulator)> {
    let Some(simulator) = Simulator::find() else {
        eprintln!("skipped: ngspice not found");
        return None;
    };
    if libcdept::xschem::Engine::find_helper().is_none() {
        eprintln!("skipped: the xschem helper is not built");
        return None;
    }
    let root = std::env::var("PDK_ROOT").unwrap_or_else(|_| "/opt/ext/OpenPDKs".into());
    if !Path::new(&root).join("ihp-sg13g2/libs.tech/ngspice").is_dir() {
        eprintln!("skipped: ihp-sg13g2 not found in {root}");
        return None;
    }
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/ihp-inv");
    let dir = tempfile::tempdir().unwrap();
    copy_dir(&example, dir.path());
    std::fs::write(dir.path().join("cdept.local.toml"), format!("pdk_root = \"{root}\"\n")).unwrap();
    let project = Project::load(dir.path()).unwrap();
    Some((dir, project, simulator))
}

#[test]
fn inverter_testbench_runs_at_three_corners() {
    let Some((_dir, project, simulator)) = open_example() else { return };
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
        // Every run is stored: result.json in its folder.
        let rec = RunRecord::read(r.record.as_ref().expect("result.json")).unwrap();
        assert_eq!(rec.measures["tpd"], out.measures["tpd"]);
        assert_eq!(rec.point.name, r.point.name());
        assert_eq!(rec.overall(), results::Overall::Pass);
    }
    // Fast corner is faster than typical, slow corner slower.
    assert!(tpd[1] < tpd[0] && tpd[0] < tpd[2], "{tpd:?}");
    // ... and indexed.
    let db = Store::open(&project.root).unwrap();
    let latest = db.latest("cells", "tb_inv", "testbench").unwrap();
    assert_eq!(latest.len(), 3);
    let agg = db.stats("cells", "tb_inv", "testbench").unwrap();
    assert_eq!((agg.runs, agg.pass), (3, 3));
    assert_eq!(agg.measures["tpd"].min, tpd[1]);
    assert_eq!(agg.measures["tpd"].max, tpd[2]);
}

#[test]
fn supply_sweep_over_corners_and_temperatures() {
    let Some((_dir, project, simulator)) = open_example() else { return };
    let tb = sim::open(&project, "cells", "tb_inv", "sweep").unwrap();
    let results = sim::run_testbench(&project, &tb, &simulator, &[]).unwrap();
    assert_eq!(results.len(), 20);
    assert_eq!(results[0].point.name(), "tt_27_vdd1");
    assert_eq!(results[1].point.variables["vdd"], "1.1");
    let tpd = |i: usize| results[i].outcome.as_ref().unwrap_or_else(|e| panic!("{i}: {e:#}")).measures["tpd"];
    // A higher supply is faster, at each corner and temperature.
    for group in 0..4 {
        for k in 0..4 {
            assert!(tpd(group * 5 + k + 1) < tpd(group * 5 + k), "group {group} step {k}");
        }
    }
    let db = Store::open(&project.root).unwrap();
    let agg = db.stats("cells", "tb_inv", "sweep").unwrap();
    assert_eq!(agg.runs, 20);
    let s = &agg.measures["tpd"];
    assert_eq!(s.count, 20);
    assert!(s.min < s.mean && s.mean < s.max && s.std > 0.0);
    // The variables are in the stored point.
    let rec = &db.latest("cells", "tb_inv", "sweep").unwrap()[7];
    assert_eq!(rec.point.variables["vdd"], "1.2");
    assert_eq!(db.series("cells", "tb_inv", "sweep", "tpd").unwrap().len(), 20);
}

#[test]
fn montecarlo_runs_are_independent_and_repeatable() {
    let Some((_dir, project, simulator)) = open_example() else { return };
    let tb = sim::open(&project, "cells", "tb_inv", "montecarlo").unwrap();
    let options = RunOptions { mc_runs: Some(5), ..Default::default() };
    let results = sim::run_testbench_with(&project, &tb, &simulator, &[], &options).unwrap();
    assert_eq!(results.len(), 5);
    let values = |rs: &[sim::PointResult]| -> Vec<f64> {
        rs.iter().map(|r| r.outcome.as_ref().unwrap_or_else(|e| panic!("{e:#}")).measures["tpd"]).collect()
    };
    let first = values(&results);
    let mut distinct = first.clone();
    distinct.dedup();
    assert_eq!(distinct.len(), 5, "the runs sample differently: {first:?}");
    assert!(results.iter().all(|r| r.point.seed.is_some() && r.specs[0].verdict == Verdict::Pass));
    // The same base seed gives the same samples, another one other samples.
    let again = sim::run_testbench_with(&project, &tb, &simulator, &[], &options).unwrap();
    assert_eq!(values(&again), first);
    let other = RunOptions { mc_runs: Some(5), mc_seed: Some(99), ..Default::default() };
    let other = sim::run_testbench_with(&project, &tb, &simulator, &[], &other).unwrap();
    assert_ne!(values(&other), first);
    // Statistics over the latest run of each point; the history keeps all 15.
    let db = Store::open(&project.root).unwrap();
    assert_eq!(db.runs("cells", "tb_inv", "montecarlo").unwrap().len(), 15);
    let agg = db.stats("cells", "tb_inv", "montecarlo").unwrap();
    assert_eq!((agg.runs, agg.pass), (5, 5));
    assert!(agg.measures["tpd"].std > 0.0);
    let v = values(&other);
    assert_eq!(agg.measures["tpd"].max, v.iter().copied().fold(f64::MIN, f64::max));
    // The grid still runs when asked for.
    let grid = RunOptions { no_mc: true, no_store: true, ..Default::default() };
    let grid = sim::run_testbench_with(&project, &tb, &simulator, &[], &grid).unwrap();
    assert_eq!(grid.len(), 1);
    assert!(grid[0].record.is_none());
}
