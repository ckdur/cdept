//! The 5-transistor OTA of examples/ihp-inv (cell `ota5t`) and the converter example
//! (`flash3`). `mod hermetic`: the testbench views, the suite and the project template
//! `ac_load` load and expand without a PDK. `mod real_pdk`: the OTA suite run for real at
//! the typical corner through the library API (xschem and ngspice on IHP SG13G2), every
//! spec passing and gain, unity-gain frequency and phase margin in sane windows; the load
//! sweep that comes from the template; the flash ADC's INL and DNL against the values the
//! ladder gives by hand. The real tests skip (with a note) without ngspice, the xschem
//! helper or the PDK.

use std::path::Path;

use libcdept::Project;

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let target = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            if e.file_name() != "simulations" {
                copy_dir(&e.path(), &target);
            }
        } else {
            std::fs::copy(e.path(), target).unwrap();
        }
    }
}

fn example() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/ihp-inv")
}

/// The OTA's testbench views.
const OTA_VIEWS: [&str; 7] = [
    "ac_open",
    "tran_step",
    "dc_op",
    "power",
    "psrr",
    "cmrr",
    "ac_cload",
];

mod hermetic {
    use super::*;
    use libcdept::sim;

    #[test]
    fn ota_testbenches_suite_and_template_load() {
        let _env = libcdept::testutil::hermetic_env();
        let dir = tempfile::tempdir().unwrap();
        copy_dir(&example(), dir.path());
        let project = Project::load(dir.path()).unwrap();
        let expect = [
            ("ac_open", "ac_response", &["gain_db", "ugbw", "pm"][..]),
            (
                "tran_step",
                "startup_settle",
                &["settle_time", "overshoot_pct", "final_err"],
            ),
            ("dc_op", "dc_op", &["vout", "isupply", "sat_margin"]),
            ("power", "power", &["i_avg", "p_avg"]),
            ("psrr", "psrr_cmrr", &["rejection_db"]),
            ("cmrr", "psrr_cmrr", &["rejection_db"]),
            ("ac_cload", "ac_load", &["pm", "ugbw"]),
        ];
        for (view, template, specs) in expect {
            let tb = sim::open(&project, "cells", "ota5t", view)
                .unwrap_or_else(|e| panic!("{view}: {e:#}"));
            assert_eq!(tb.setup.template, template, "{view}");
            let names: Vec<&str> = tb.setup.specs.iter().map(|s| s.name.as_str()).collect();
            for s in specs {
                assert!(names.contains(s), "{view}: no spec {s} in {names:?}");
            }
        }
        // the grid of the suite's tests: five corners, three temperatures
        let tb = sim::open(&project, "cells", "ota5t", "ac_open").unwrap();
        assert_eq!(tb.setup.corners, ["tt", "ff", "ss", "fs", "sf"]);
        assert_eq!(tb.setup.temperatures, [-40.0, 27.0, 125.0]);
        // the sweep of the load comes from the project template's [[sweep]]
        let tb = sim::open(&project, "cells", "ota5t", "ac_cload").unwrap();
        assert_eq!(tb.setup.sweeps.len(), 1);
        assert_eq!(tb.setup.sweeps[0].variable, "cl");
        assert_eq!(
            tb.setup.sweeps[0].values,
            ["50f", "100f", "200f", "500f", "1p"]
        );
        assert!(tb.setup.montecarlo.is_none());
        let suite = sim::suite::open(&project, "cells", "ota5t", "suite").unwrap();
        let views: Vec<&str> = suite.tests.iter().map(|t| t.view.as_str()).collect();
        assert_eq!(views, &OTA_VIEWS[..6]);
        let tb = sim::open(&project, "cells", "flash3", "inl_dnl").unwrap();
        assert_eq!(tb.setup.template, "inl_dnl");
        assert!(matches!(tb.setup.analyses[..], [sim::Analysis::Dc { .. }]));
    }
}

mod real_pdk {
    use super::*;
    use libcdept::sim::results::Overall;
    use libcdept::sim::suite::{self, SuiteOptions};
    use libcdept::sim::{self, Simulator};
    use libcdept::testutil::{RealPdk, real_pdk};

    struct Scratch {
        dir: tempfile::TempDir,
        _pdk: RealPdk,
    }

    fn open_example() -> Option<(Scratch, Project, Simulator)> {
        let Some(simulator) = Simulator::find() else {
            eprintln!("skipped: ngspice not found");
            return None;
        };
        if libcdept::xschem::Engine::find_helper().is_none() {
            eprintln!("skipped: the xschem helper is not built");
            return None;
        }
        let pdk = real_pdk("ihp-sg13g2")?;
        let root = pdk.root.display().to_string();
        let dir = tempfile::tempdir().unwrap();
        copy_dir(&example(), dir.path());
        std::fs::write(
            dir.path().join("cdept.local.toml"),
            format!("pdk_root = \"{root}\"\n"),
        )
        .unwrap();
        let project = Project::load(dir.path()).unwrap();
        Some((Scratch { dir, _pdk: pdk }, project, simulator))
    }

    fn measures(records: &[sim::results::RunRecord]) -> &std::collections::BTreeMap<String, f64> {
        assert_eq!(records.len(), 1, "one point: tt at 27 C");
        &records[0].measures
    }

    #[test]
    fn the_ota_suite_passes_at_the_typical_corner() {
        let Some((scratch, _, _)) = open_example() else {
            return;
        };
        // Only the typical corner at 27 C (the suite's own grid is 5 corners x 3 temperatures).
        let file = scratch.dir.path().join("libs/cells/ota5t/suite/suite.toml");
        let text = std::fs::read_to_string(&file).unwrap();
        let text = text
            .replace(
                "corners = [\"tt\", \"ff\", \"ss\", \"fs\", \"sf\"]",
                "corners = [\"tt\"]",
            )
            .replace("temperatures = [-40, 27, 125]", "temperatures = [27]");
        assert!(
            text.contains("corners = [\"tt\"]") && text.contains("temperatures = [27]"),
            "{text}"
        );
        std::fs::write(&file, text).unwrap();
        let project = Project::load(scratch.dir.path()).unwrap();

        let report = suite::run(
            &project,
            "cells",
            "ota5t",
            "suite",
            &SuiteOptions::default(),
        )
        .unwrap();
        assert_eq!(report.tests.len(), 6);
        for t in &report.tests {
            assert!(t.error.is_none(), "{}: {:?}", t.id, t.error);
            assert!(
                t.records.iter().all(|r| r.overall() == Overall::Pass),
                "{}: {:?}",
                t.id,
                t.records
            );
        }
        let s = report.summary();
        assert!(s.ok(), "{s:?}");
        assert_eq!((s.runs, s.pass), (6, 6));
        assert!(suite::format_report(&report, false).contains("SUITE PASS"));

        let by = |view: &str| {
            measures(
                &report
                    .tests
                    .iter()
                    .find(|t| t.id.ends_with(view))
                    .unwrap()
                    .records,
            )
        };
        // Open loop: 35.8 dB, UGBW 145 MHz, phase margin 59 degrees when this was written.
        let ac = by("/ac_open");
        assert!(ac["gain_db"] > 32.0 && ac["gain_db"] < 42.0, "{ac:?}");
        assert!(ac["ugbw"] > 8e7 && ac["ugbw"] < 4e8, "{ac:?}");
        assert!(ac["pm"] > 50.0 && ac["pm"] < 80.0, "{ac:?}");
        assert!(
            ac["bw_3db"] * 10f64.powf(ac["gain_db"] / 20.0) > 0.5 * ac["ugbw"],
            "gain x bandwidth ~ UGBW: {ac:?}"
        );
        // Unity-gain step: settles in about 9 ns, small overshoot, lands on the input.
        let step = by("/tran_step");
        assert!(
            step["settle_time"] > 1e-9 && step["settle_time"] < 2e-8,
            "{step:?}"
        );
        assert!(
            step["overshoot_pct"] < 15.0 && step["final_err"] < 0.025,
            "{step:?}"
        );
        // Operating point: out follows the 0.42 V common mode, 39 uA, all six saturated.
        let op = by("/dc_op");
        assert!((op["vout"] - 0.42).abs() < 0.03, "{op:?}");
        assert!(op["isupply"] > 3e-5 && op["isupply"] < 5e-5, "{op:?}");
        assert!(op["sat_margin"] > 0.03, "{op:?}");
        let p = by("/power");
        assert!(
            (p["p_avg"] - 1.2 * p["i_avg"]).abs() < 1e-12
                && (p["i_avg"] - op["isupply"]).abs() < 5e-6,
            "{p:?} {op:?}"
        );
        assert!(by("/psrr")["rejection_db"] > 50.0 && by("/cmrr")["rejection_db"] > 40.0);
    }

    #[test]
    fn the_load_sweep_comes_from_the_template() {
        let Some((_dir, project, simulator)) = open_example() else {
            return;
        };
        let tb = sim::open(&project, "cells", "ota5t", "ac_cload").unwrap();
        let results = sim::run_testbench(&project, &tb, &simulator, &[]).unwrap();
        assert_eq!(results.len(), 5);
        let pm: Vec<f64> = results
            .iter()
            .map(|r| r.outcome.as_ref().unwrap().measures["pm"])
            .collect();
        let ugbw: Vec<f64> = results
            .iter()
            .map(|r| r.outcome.as_ref().unwrap().measures["ugbw"])
            .collect();
        // a heavier load: lower unity-gain frequency, more phase margin
        assert!(pm.windows(2).all(|w| w[1] > w[0]), "{pm:?}");
        assert!(ugbw.windows(2).all(|w| w[1] < w[0]), "{ugbw:?}");
        assert!(results.iter().all(|r| {
            r.specs
                .iter()
                .all(|s| s.verdict == sim::spec::Verdict::Pass)
        }));
        assert_eq!(results[0].point.variables["cl"], "50f");
    }

    #[test]
    fn the_flash_adc_linearity_matches_its_ladder() {
        let Some((_dir, project, simulator)) = open_example() else {
            return;
        };
        let tb = sim::open(&project, "cells", "flash3", "inl_dnl").unwrap();
        let results = sim::run_testbench(&project, &tb, &simulator, &[]).unwrap();
        let m = &results[0]
            .outcome
            .as_ref()
            .unwrap_or_else(|e| panic!("{e:#}"))
            .measures;
        // Ladder 10k x 7 and R5 = 11k: transitions at 10, 20, 30, 40, 51, 61, 71 of 81
        // (x 1.2 V); end-point LSB 61/6, so the widest code is 11/(61/6) - 1 = 0.082 LSB too
        // wide and the largest INL is 0.5/(61/6) = 0.049 LSB.
        assert!((m["dnl_lsb"] - 0.0820).abs() < 0.003, "{m:?}");
        assert!((m["inl_lsb"] - 0.0492).abs() < 0.003, "{m:?}");
        assert!(
            results[0]
                .specs
                .iter()
                .all(|s| s.verdict == sim::spec::Verdict::Pass),
            "{:?}",
            results[0].specs
        );
    }
}
