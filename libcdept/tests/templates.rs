//! Test templates and suites for real: the inverter's family of tests in
//! examples/ihp-inv (tran_delay, dc_transfer, power, ac_response) through xschem and
//! ngspice. Skipped (with a note) when ngspice, the xschem helper or the PDK is missing.

use std::path::Path;

use libcdept::Project;
use libcdept::sim::results::Overall;
use libcdept::sim::suite::{self, SuiteOptions};
use libcdept::sim::{self, Simulator};

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

fn run_view(project: &Project, simulator: &Simulator, cell: &str, view: &str) -> Vec<sim::PointResult> {
    let tb = sim::open(project, "cells", cell, view).unwrap();
    sim::run_testbench(project, &tb, simulator, &[]).unwrap()
}

#[test]
fn tran_delay_matches_a_hand_written_meas() {
    let Some((_dir, project, simulator)) = open_example() else { return };
    let results = run_view(&project, &simulator, "tb_inv", "tran_delay");
    assert_eq!(results.len(), 3);
    let by_hand = run_view(&project, &simulator, "tb_inv", "testbench");
    for (t, h) in results.iter().zip(&by_hand) {
        let m = &t.outcome.as_ref().unwrap_or_else(|e| panic!("{e:#}")).measures;
        // The `.meas` of testbench is the output-falling delay at vdd/2.
        let meas = h.outcome.as_ref().unwrap().measures["tpd"];
        let fall = m["tpd_fall"];
        assert!((fall - meas).abs() < 1e-12, "{fall} vs {meas}");
        assert!(m["tpd"] > 1e-11 && m["tpd"] < 3e-10, "{m:?}");
        assert!(m["t_rise"] > 0.0 && m["slew_rise"] > 1e9, "{m:?}");
        assert!(t.specs.iter().all(|s| s.verdict == sim::spec::Verdict::Pass), "{:?}", t.specs);
        assert!(t.run.calc.contains(&"tpd".to_string()));
    }
}

#[test]
fn dc_transfer_finds_the_trip_point() {
    let Some((_dir, project, simulator)) = open_example() else { return };
    let results = run_view(&project, &simulator, "tb_inv", "dc_transfer");
    for r in &results {
        let m = &r.outcome.as_ref().unwrap_or_else(|e| panic!("{e:#}")).measures;
        assert!(m["vout_start"] > 1.19 && m["vout_end"] < 0.01, "{m:?}");
        assert!(m["vtrip"] > 0.55 && m["vtrip"] < 0.7, "{m:?}");
        assert!(m["gain_max"] > 10.0, "{m:?}");
        assert!(r.specs.iter().all(|s| s.verdict == sim::spec::Verdict::Pass), "{:?}", r.specs);
    }
}

#[test]
fn power_is_consistent() {
    let Some((_dir, project, simulator)) = open_example() else { return };
    let results = run_view(&project, &simulator, "tb_inv", "power");
    assert_eq!(results.len(), 6);
    for r in &results {
        let m = &r.outcome.as_ref().unwrap_or_else(|e| panic!("{e:#}")).measures;
        assert!(m["i_peak"] > m["i_avg"] && m["i_avg"] > 0.0, "{m:?}");
        assert!((m["p_avg"] - 1.2 * m["i_avg"]).abs() < 1e-12);
        assert!((m["e_cycle"] - m["p_avg"] * 1e-9).abs() < 1e-18);
        // CV^2 of the 10 fF load alone is 14.4 fJ per cycle.
        assert!(m["e_cycle"] > 1.0e-14 && m["e_cycle"] < 5e-14, "{m:?}");
    }
}

#[test]
fn ac_response_of_the_biased_inverter() {
    let Some((_dir, project, simulator)) = open_example() else { return };
    let results = run_view(&project, &simulator, "tb_inv_ac", "ac_response");
    let m = &results[0].outcome.as_ref().unwrap_or_else(|e| panic!("{e:#}")).measures;
    assert!(m["gain_db"] > 15.0 && m["gain_db"] < 30.0, "{m:?}");
    assert!(m["bw_3db"] > 1e8 && m["bw_3db"] < 1e9, "{m:?}");
    assert!(m["ugbw"] > m["bw_3db"], "{m:?}");
    assert!(m["pm"] > 60.0 && m["pm"] < 100.0, "{m:?}");
    assert!(results[0].specs.iter().all(|s| s.verdict == sim::spec::Verdict::Pass), "{:?}", results[0].specs);
}

#[test]
fn the_suite_runs_all_tests_over_shared_corners_and_is_stored() {
    let Some((_dir, project, _)) = open_example() else { return };
    let options = SuiteOptions::default();
    let report = suite::run(&project, "cells", "tb_inv", "suite", &options).unwrap();
    assert_eq!(report.tests.len(), 4);
    for t in &report.tests {
        assert!(t.error.is_none(), "{}: {:?}", t.id, t.error);
        assert_eq!(t.records.len(), 3, "{}: shared corners tt, ff, ss at 27 C", t.id);
        assert!(t.records.iter().all(|r| r.overall() == Overall::Pass), "{}", t.id);
    }
    // The power test has two temperatures of its own; the suite sets one.
    let s = report.summary();
    assert!(s.ok(), "{s:?}");
    assert_eq!((s.runs, s.pass), (12, 12));
    let text = suite::format_report(&report, false);
    assert!(text.contains("SUITE PASS") && text.contains("tpd") && text.contains("pm"), "{text}");
    // stored: suite.json, and the runs of every test in results.db
    assert_eq!(suite::latest(&project, "cells", "tb_inv", "suite").unwrap().summary(), s);
    let db = sim::results::Store::open(&project.root).unwrap();
    assert_eq!(db.latest("cells", "tb_inv_ac", "ac_response").unwrap().len(), 3);
    let conn = rusqlite::Connection::open(sim::results::Store::path(&project.root)).unwrap();
    let rows: i64 = conn.query_row("SELECT COUNT(*) FROM suite_runs WHERE view = 'suite'", [], |r| r.get(0)).unwrap();
    assert_eq!(rows, 1);
}

#[test]
fn a_failing_spec_and_a_missing_net_fail_the_suite() {
    let Some((dir, project, _)) = open_example() else { return };
    // A spec that cannot be met, and a test with a net the schematic does not have.
    std::fs::write(
        dir.path().join("libs/cells/tb_inv/tran_delay/setup.toml"),
        "template = \"tran_delay\"\ncorners = [\"tt\"]\n[template_args]\ntpd_max = \"1p\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("libs/cells/tb_inv/bad_net")).unwrap();
    std::fs::write(
        dir.path().join("libs/cells/tb_inv/bad_net/setup.toml"),
        "template = \"tran_delay\"\n[template_args]\nout = \"nope\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("libs/cells/tb_inv/suite/suite.toml"),
        "[[test]]\nview = \"tran_delay\"\n[[test]]\nview = \"bad_net\"\n",
    )
    .unwrap();
    let project = Project::load(&project.root).unwrap();
    let report = suite::run(&project, "cells", "tb_inv", "suite", &SuiteOptions { no_store: true, ..Default::default() }).unwrap();
    let s = report.summary();
    assert!(!s.ok());
    assert_eq!((s.fail, s.spec_fail, s.test_errors), (1, 1, 1), "{s:?}");
    let err = report.tests[1].error.as_deref().unwrap();
    assert!(err.contains("net 'nope' does not exist"), "{err}");
}
