//! VACASK against ngspice on examples/ihp-inv (IHP SG13G2: PSP103 through OSDI compiled by
//! OpenVAF, converted statistical models): the inverter's tests, the OTA suite at the
//! typical corner, Monte Carlo with CDePT's own draws (statistics against ngspice's, seed
//! reproducibility, the three modes). The tolerances are the differences seen when the
//! tests were written (docs/VACASK.md, "Parity with ngspice"), with some room.
//! Skipped, with a note, when VACASK, OpenVAF, python3, ngspice, the xschem helper or the
//! PDK is missing.

use std::collections::BTreeMap;
use std::path::Path;

use libcdept::Project;
use libcdept::sim::run::SimKind;
use libcdept::sim::setup::McMode;
use libcdept::sim::suite::{self, SuiteOptions};
use libcdept::sim::{self, RunOptions, Simulator};
use libcdept::testutil::{RealPdk, real_pdk};

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

struct Scratch {
    dir: tempfile::TempDir,
    _pdk: RealPdk,
}

struct Tools {
    vacask: Simulator,
    ngspice: Simulator,
}

fn open_example() -> Option<(Scratch, Project, Tools)> {
    let (Some(vacask), Some(ngspice)) = (Simulator::find_kind(SimKind::Vacask), Simulator::find())
    else {
        eprintln!("skipped: VACASK or ngspice not found");
        return None;
    };
    if libcdept::batch::openvaf::ADAPTER.locate().is_none()
        || libcdept::tools::find_in_path("python3").is_none()
    {
        eprintln!("skipped: OpenVAF or python3 not found");
        return None;
    }
    if libcdept::xschem::Engine::find_helper().is_none() {
        eprintln!("skipped: the xschem helper is not built");
        return None;
    }
    let pdk = real_pdk("ihp-sg13g2")?;
    if !pdk.dir.join("libs.tech/verilog-a").is_dir() {
        eprintln!("skipped: ihp-sg13g2 has no libs.tech/verilog-a");
        return None;
    }
    let root = pdk.root.display().to_string();
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/ihp-inv");
    let dir = tempfile::tempdir().unwrap();
    copy_dir(&example, dir.path());
    std::fs::write(
        dir.path().join("cdept.local.toml"),
        format!("pdk_root = \"{root}\"\n"),
    )
    .unwrap();
    let project = Project::load(dir.path()).unwrap();
    Some((
        Scratch { dir, _pdk: pdk },
        project,
        Tools { vacask, ngspice },
    ))
}

/// The measures of a view at the typical corner (first temperature) with one simulator.
fn run_view(
    project: &Project,
    tools: &Tools,
    cell: &str,
    view: &str,
    simulator: &str,
) -> Vec<BTreeMap<String, f64>> {
    let mut tb = sim::open(project, "cells", cell, view).unwrap();
    tb.setup.set_simulator(simulator).unwrap();
    let program = if simulator == "vacask" {
        &tools.vacask
    } else {
        &tools.ngspice
    };
    let results = sim::run_testbench_with(
        project,
        &tb,
        program,
        &["tt".to_string()],
        &RunOptions {
            no_store: true,
            ..Default::default()
        },
    )
    .unwrap();
    results
        .into_iter()
        .map(|r| {
            let o = r
                .outcome
                .unwrap_or_else(|e| panic!("{cell}/{view} with {simulator}: {e:#}"));
            o.measures
        })
        .collect()
}

/// Relative tolerance of a measurement (docs/VACASK.md has the observed differences).
fn tolerance(name: &str) -> f64 {
    match name {
        // Transient edge shapes depend on the time steps the two simulators choose.
        "overshoot" | "overshoot_pct" => 0.12,
        "t_rise" | "t_fall" | "slew_rise" | "slew_fall" | "settle_time" => 0.06,
        "i_avg" | "p_avg" | "e_cycle" | "i_peak" | "p_peak" => 0.02,
        // thd_fft: a distortion of 10 % read from 1024 points of a coarse transient.
        "thd_db" | "thd_pct" | "sndr_db" | "sfdr_db" | "enob" => 0.05,
        _ => 0.005,
    }
}

fn compare(what: &str, vc: &BTreeMap<String, f64>, ng: &BTreeMap<String, f64>) {
    assert!(!ng.is_empty(), "{what}: ngspice measured nothing");
    for (k, n) in ng {
        let v = vc
            .get(k)
            .unwrap_or_else(|| panic!("{what}: VACASK has no '{k}' ({vc:?})"));
        let scale = n.abs().max(1e-30);
        let diff = (v - n).abs() / scale;
        let abs_ok =
            (v - n).abs() < 1e-9 * (1.0 + n.abs()) || (k == "peaking_db" && (v - n).abs() < 0.05);
        assert!(
            diff <= tolerance(k) || abs_ok,
            "{what}: {k} vacask {v} ngspice {n} ({:.3} %, allowed {:.1} %)",
            diff * 100.0,
            tolerance(k) * 100.0
        );
    }
}

#[test]
fn the_inverter_tests_agree_with_ngspice() {
    let Some((_dir, project, tools)) = open_example() else {
        return;
    };
    for (cell, view) in [
        ("tb_inv", "tran_delay"),
        ("tb_inv", "dc_transfer"),
        ("tb_inv", "power"),
        ("tb_inv_ac", "ac_response"),
    ] {
        let vc = run_view(&project, &tools, cell, view, "vacask");
        let ng = run_view(&project, &tools, cell, view, "ngspice");
        for (v, n) in vc.iter().zip(&ng) {
            compare(&format!("{cell}/{view}"), v, n);
        }
    }
}

#[test]
fn the_ota_suite_at_the_typical_corner_agrees_with_ngspice() {
    let Some((dir, project, _tools)) = open_example() else {
        return;
    };
    // The suite over one corner and one temperature (the file has 15 points per test).
    let file = dir.dir.path().join("libs/cells/ota5t/suite/suite.toml");
    let text = std::fs::read_to_string(&file).unwrap();
    let text: String = text
        .lines()
        .map(|l| match l.split('=').next().map(str::trim) {
            Some("corners") => "corners = [\"tt\"]".to_string(),
            Some("temperatures") => "temperatures = [27]".to_string(),
            _ => l.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&file, text).unwrap();
    let run = |sim: &str| {
        let options = SuiteOptions {
            no_store: true,
            simulator: Some(sim.to_string()),
            ..Default::default()
        };
        suite::run(&project, "cells", "ota5t", "suite", &options).unwrap()
    };
    let (vc, ng) = (run("vacask"), run("ngspice"));
    assert_eq!(vc.tests.len(), 6);
    for (v, n) in vc.tests.iter().zip(&ng.tests) {
        assert!(v.error.is_none(), "{}: {:?}", v.id, v.error);
        assert_eq!(v.records.len(), 1);
        let (rv, rn) = (&v.records[0], &n.records[0]);
        assert!(rv.error.is_none(), "{}: {:?}", v.id, rv.error);
        assert!(rv.calc_errors.is_empty(), "{}: {:?}", v.id, rv.calc_errors);
        assert_eq!(rv.simulator, "vacask");
        compare(&v.id, &rv.measures, &rn.measures);
        // Same verdicts: the specs are met by both or by neither.
        let verdicts = |r: &libcdept::sim::results::RunRecord| {
            r.specs.iter().map(|s| s.verdict).collect::<Vec<_>>()
        };
        assert_eq!(verdicts(rv), verdicts(rn), "{}", v.id);
    }
}

/// An operating point of `netlist` at the typical corner: Ok, or the error text.
fn op_point(
    project: &Project,
    tools: &Tools,
    netlist: &str,
    dir: &Path,
) -> Result<std::collections::BTreeMap<String, f64>, String> {
    let ready = libcdept::vacask::models::ready(project).map_err(|e| format!("{e:#}"))?;
    let mut setup =
        sim::Setup::parse("simulator = \"vacask\"\n[[analysis]]\nkind = \"op\"\n").unwrap();
    setup.save.clear();
    let point = sim::deck::RunPoint {
        corner: "tt".into(),
        ..Default::default()
    };
    let d = libcdept::vacask::build_point(
        project, &ready, &setup, "vacask", &point, "devices", netlist,
    )
    .map_err(|e| format!("{e:#}"))?;
    let out = libcdept::vacask::run_vacask(&tools.vacask.program, &d.text, dir, &[])
        .map_err(|e| format!("{e:#}"))?;
    let plot = libcdept::vacask::raw::read(&out.raw)
        .map_err(|e| format!("{e:#}"))?
        .remove(0);
    Ok(plot
        .names
        .iter()
        .cloned()
        .zip(plot.data.iter().map(|d| d[0]))
        .collect())
}

#[test]
fn the_devices_of_the_pdk_run_or_are_refused_by_name() {
    let Some((dir, project, tools)) = open_example() else {
        return;
    };
    let run = |name: &str, net: &str| {
        op_point(
            &project,
            &tools,
            net,
            &dir.dir.path().join("dev").join(name),
        )
    };
    // Run: the MOS transistors, the bipolar transistors (VBIC, 4 and 5 terminals), a resistor,
    // a capacitor, a diode.
    let nmos = run(
        "nmos",
        "t\nVd d 0 1.2\nVg g 0 0.9\nXM1 d g 0 0 sg13_lv_nmos w=1u l=0.13u ng=1 m=1 mm_ok=0\n.end\n",
    )
    .unwrap();
    assert!(nmos["Vd:flow(br)"] < -1e-5, "{nmos:?}");
    let hv = run(
        "hv",
        "t\nVd d 0 3.3\nVg g 0 1.5\nXM1 d g 0 0 sg13_hv_nmos w=1u l=0.45u ng=1 m=1 mm_ok=0\n.end\n",
    )
    .unwrap();
    assert!(hv["Vd:flow(br)"] < -1e-6, "{hv:?}");
    let npn = run(
        "npn",
        "t\nVc c 0 1.2\nVb b 0 0.8\nXQ1 c b 0 0 npn13G2 Nx=1 mm_ok=0\n.end\n",
    )
    .unwrap();
    assert!(npn["Vc:flow(br)"] < -1e-6, "{npn:?}");
    let npn5 = run(
        "npn5",
        "t\nVc c 0 1.2\nVb b 0 0.8\nXQ1 c b 0 0 0 npn13G2_5t Nx=1 mm_ok=0\n.end\n",
    )
    .unwrap();
    assert!(npn5["Vc:flow(br)"] < -1e-6, "{npn5:?}");
    let res = run(
        "res",
        "t\nV1 a 0 1\nXR1 a b 0 rppd w=0.5u l=5u m=1 b=0 mm_ok=0\nR2 b 0 1k\n.end\n",
    )
    .unwrap();
    assert!(res["b"] > 0.0 && res["b"] < 1.0, "{res:?}");
    let cap = run(
        "cap",
        "t\nV1 a 0 1\nXC1 a b cap_cmim w=10u l=10u m=1 mm_ok=0\nR2 b 0 1k\n.end\n",
    )
    .unwrap();
    assert!(cap["b"].abs() < 1e-3, "{cap:?}");
    // The bond pad's symbol brings `.include sg13g2_bondpad.lib` (an empty subcircuit, in no
    // corner): the include is the converted file of the set.
    let include = dir
        ._pdk
        .dir
        .join("libs.tech/ngspice/models/sg13g2_bondpad.lib");
    let pad = run(
        "pad",
        &format!(
            "t\n.include {}\nV1 a 0 1\nR1 a b 1k\nXB1 b bondpad size=80u shape=0 padtype=0\n.end\n",
            include.display()
        ),
    )
    .unwrap();
    assert!((pad["b"] - 1.0).abs() < 1e-6, "{pad:?}");
    // Refused, naming the device: the varicap's resistor depends on a node voltage.
    let e = run(
        "varicap",
        "t\nV1 a 0 1\nV2 g 0 0\nX1 g a 0 sg13_hv_svaricap w=3.74u l=0.8u Nx=1 mm_ok=0\n.end\n",
    )
    .unwrap_err();
    assert!(e.contains("sg13_hv_svaricap") && e.contains("v()"), "{e}");
}

/// 200 runs, seed 1, process variation of examples/ihp-inv tb_inv/montecarlo (tpd at 0.5 V):
/// what ngspice gave when this was written (docs/VACASK.md). The generators differ, so the
/// statistics agree, not the runs.
const NGSPICE_MC_TPD: (f64, f64) = (5.4884e-11, 4.1530e-12);

fn mc_tpd(project: &Project, tools: &Tools, mode: McMode, runs: u32, seed: u64) -> Vec<f64> {
    let mut tb = sim::open(project, "cells", "tb_inv", "montecarlo").unwrap();
    tb.setup.set_simulator("vacask").unwrap();
    let options = RunOptions {
        no_store: true,
        mc_runs: Some(runs),
        mc_seed: Some(seed),
        mc_mode: Some(mode),
        ..Default::default()
    };
    let results = sim::run_testbench_with(project, &tb, &tools.vacask, &[], &options).unwrap();
    results
        .iter()
        .map(|r| {
            r.outcome
                .as_ref()
                .unwrap_or_else(|e| panic!("{e:#}"))
                .measures["tpd"]
        })
        .collect()
}

fn mean_std(v: &[f64]) -> (f64, f64) {
    let n = v.len() as f64;
    let mean = v.iter().sum::<f64>() / n;
    (
        mean,
        (v.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0)).sqrt(),
    )
}

#[test]
fn monte_carlo_on_vacask_has_ngspices_statistics_and_repeats() {
    let Some((_dir, project, tools)) = open_example() else {
        return;
    };
    let tpd = mc_tpd(&project, &tools, McMode::Process, 200, 1);
    assert_eq!(tpd.len(), 200);
    let (mean, std) = mean_std(&tpd);
    let (ref_mean, ref_std) = NGSPICE_MC_TPD;
    assert!(
        ((mean - ref_mean) / ref_mean).abs() < 0.02,
        "mean {mean} against ngspice {ref_mean}"
    );
    assert!(
        ((std - ref_std) / ref_std).abs() < 0.25,
        "std {std} against ngspice {ref_std}"
    );

    // The same seed repeats the numbers, another one does not.
    let a = mc_tpd(&project, &tools, McMode::Process, 6, 7);
    let b = mc_tpd(&project, &tools, McMode::Process, 6, 7);
    let c = mc_tpd(&project, &tools, McMode::Process, 6, 8);
    assert_eq!(a, b);
    assert_ne!(a, c);
    // The first runs of a longer run are the same runs (a draw depends on the run, not on the count).
    assert_eq!(
        &mc_tpd(&project, &tools, McMode::Process, 3, 7)[..],
        &a[..3]
    );

    // Mismatch alone spreads less than the process; both together at least as much as the process.
    let mm = mean_std(&mc_tpd(&project, &tools, McMode::Mismatch, 60, 1));
    let both = mean_std(&mc_tpd(&project, &tools, McMode::Both, 60, 1));
    assert!(
        mm.1 > 0.0 && mm.1 < std,
        "mismatch std {} process std {std}",
        mm.1
    );
    assert!(both.1 > mm.1, "both {} mismatch {}", both.1, mm.1);
    assert!(
        ((mm.0 - mean) / mean).abs() < 0.03,
        "mismatch mean {} process mean {mean}",
        mm.0
    );
}
