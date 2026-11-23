//! VACASK for real: generic devices (RC, diode, level-1 MOS) through the translator
//! and the simulator, and examples/ihp-inv/tb_inv (IHP SG13G2, PSP103 through OSDI
//! compiled by OpenVAF) against ngspice. Each test is skipped, with a note, when a
//! tool is missing: VACASK (`$CDEPT_VACASK` or PATH), OpenVAF (`$CDEPT_OPENVAF`),
//! python3, ngspice (`$CDEPT_NGSPICE`), the xschem helper, the PDK (`$PDK_ROOT` or
//! /opt/ext/OpenPDKs).

use std::path::Path;

use libcdept::Project;
use libcdept::sim::deck::RunPoint;
use libcdept::sim::run::SimKind;
use libcdept::sim::{self, Setup, Simulator};
use libcdept::vacask::{self, deck, raw};

fn vacask() -> Option<Simulator> {
    let s = Simulator::find_kind(SimKind::Vacask);
    if s.is_none() {
        eprintln!("skipped: vacask not found");
    }
    s
}

/// Build and run a deck for `netlist` with `setup`, return the outcome's folder.
fn run_generic(sim: &Simulator, setup_toml: &str, netlist: &str) -> (tempfile::TempDir, sim::Outcome) {
    let setup = Setup::parse(setup_toml).unwrap();
    let point = RunPoint { corner: "tt".into(), ..Default::default() };
    let i = deck::Inputs { setup: &setup, point: &point, title: "generic", models: &[], netlist, defined: None, pdk: "", notes: &[] };
    let d = deck::build(&i).unwrap_or_else(|e| panic!("{e:#}"));
    let dir = tempfile::tempdir().unwrap();
    let out = vacask::run_vacask(&sim.program, &d.text, dir.path(), &[]).unwrap_or_else(|e| panic!("{e:#}\n{}", d.text));
    (dir, out)
}

const RC: &str = "* rc\nV1 in 0 PULSE(0 1 1n 1n 1n 5n 20n) ac 1 dc 0\nR1 in out 1k\nC1 out 0 1p\n.end\n";

#[test]
fn rc_transient_dc_ac_and_noise() {
    let Some(sim) = vacask() else { return };
    let (dir, out) = run_generic(
        &sim,
        "simulator=\"vacask\"\nmeasures=[\"tran dly trig v(in) val=0.5 rise=1 targ v(out) val=0.5 rise=1\", \"tran vmax max v(out)\"]\n\
         [[analysis]]\nkind=\"tran\"\nstep=\"0.1n\"\nstop=\"20n\"\n\
         [[analysis]]\nkind=\"ac\"\npoints=10\nstart=\"1meg\"\nstop=\"1g\"\n\
         [[analysis]]\nkind=\"noise\"\noutput=\"v(out)\"\nsource=\"V1\"\npoints=5\nstart=\"1k\"\nstop=\"1meg\"\n\
         [[analysis]]\nkind=\"dc\"\nsource=\"V1\"\nstart=\"0\"\nstop=\"1\"\nstep=\"0.25\"\n",
        RC,
    );
    // The same RC delay as the Xyce test (7.35e-10).
    assert!((out.measures["dly"] - 7.35e-10).abs() < 5e-11, "{:?}", out.measures);
    assert!(out.measures["vmax"] > 0.99 && out.measures["vmax"] < 1.001, "{:?}", out.measures);
    assert!(out.raw.is_file());
    // AC: 1/sqrt(1+(f/fc)^2), fc = 1/(2 pi R C) = 159.15 MHz.
    let ac = &raw::read(&dir.path().join("ac1.raw")).unwrap()[0];
    assert!(ac.complex);
    let (f, re, im) = (ac.x(), ac.get("out").unwrap(), &ac.imag[ac.index("out").unwrap()]);
    let fc = 1.0 / (2.0 * std::f64::consts::PI * 1e3 * 1e-12);
    for k in 0..f.len() {
        let want = 1.0 / (1.0 + (f[k] / fc).powi(2)).sqrt();
        let got = re[k].hypot(im[k]);
        assert!((got - want).abs() < 0.01 * want + 1e-6, "f={} {got} vs {want}", f[k]);
    }
    // Noise: the resistor's 4kTR at the output at low frequency, 4.07 nV/rtHz at 27 C.
    let noise = &raw::read(&dir.path().join("noise1.raw")).unwrap()[0];
    let names = noise.names.join(" ");
    let vn = noise.index("onoise").unwrap_or_else(|| panic!("no output noise in {names}"));
    let first = noise.data[vn][0];
    assert!(first > 3.5e-9 && first < 4.6e-9 || first > 1.2e-17 && first < 2.1e-17, "{first} ({names})");
    // DC sweep of the source: out follows in (no dc current through the capacitor).
    let dc = &raw::read(&dir.path().join("dc1.raw")).unwrap()[0];
    let out_v = dc.get("out").unwrap();
    assert_eq!(out_v.len(), 5, "to= is reached");
    assert!((out_v[4] - 1.0).abs() < 1e-6 && out_v[0].abs() < 1e-9, "{out_v:?}");
}

const DEVICES: &str = "* generic devices\n.model nm nmos (level=1 vto=0.5 kp=2e-4 lambda=0.02)\n.model pm pmos (level=1 vto=-0.5 kp=1e-4 lambda=0.02)\n\
.model dd d (is=1e-14 n=1.0 rs=1)\nVdd vdd 0 1.8\nVin in 0 PULSE(0 1.8 1n 0.1n 0.1n 4n 10n)\n\
M1 out in 0 0 nm w=10u l=1u\nM2 out in vdd vdd pm w=20u l=1u\nCl out 0 20f\nVd a 0 0.7\nRd a b 10\nD1 b 0 dd\n.end\n";

#[test]
fn level_1_inverter_and_a_diode_agree_with_ngspice() {
    let Some(sim) = vacask() else { return };
    let (_dir, out) = run_generic(
        &sim,
        "simulator=\"vacask\"\nmeasures=[\"tran tpd trig v(in) val=0.9 rise=1 targ v(out) val=0.9 fall=1\", \"tran vlow min v(out)\", \"tran vhigh max v(out)\", \"op idiode min i(Vd)\"]\n\
         [[analysis]]\nkind=\"op\"\n[[analysis]]\nkind=\"tran\"\nstep=\"10p\"\nstop=\"8n\"\n",
        DEVICES,
    );
    let (tpd, lo, hi) = (out.measures["tpd"], out.measures["vlow"], out.measures["vhigh"]);
    assert!(tpd > 1e-11 && tpd < 5e-9, "tpd {tpd}");
    assert!(lo.abs() < 0.05 && (hi - 1.8).abs() < 0.05, "swing {lo} .. {hi}");
    // Forward diode current at 0.7 V through 10 ohm: about 30 mA, taken out of Vd.
    assert!(out.measures["idiode"] < -1e-3, "{:?}", out.measures);

    // The same circuit in ngspice: the delays agree to a few percent (two
    // independent implementations of the same level-1 equations).
    let Some(ng) = Simulator::find() else { return };
    let setup = Setup::parse(
        "measures=[\"tran tpd trig v(in) val=0.9 rise=1 targ v(out) val=0.9 fall=1\"]\n[[analysis]]\nkind=\"tran\"\nstep=\"10p\"\nstop=\"8n\"\n",
    )
    .unwrap();
    let point = RunPoint { corner: "tt".into(), ..Default::default() };
    let text = sim::build_deck(&setup, &point, "generic", &[], DEVICES);
    let dir = tempfile::tempdir().unwrap();
    let n = sim::run(&ng, &text, dir.path(), &[]).unwrap();
    let rel = (tpd - n.measures["tpd"]).abs() / n.measures["tpd"];
    assert!(rel < 0.05, "vacask {tpd} ngspice {} ({:.1} %)", n.measures["tpd"], rel * 100.0);
}

fn tools_for_pdk() -> bool {
    if vacask().is_none() {
        return false;
    }
    if libcdept::batch::openvaf::ADAPTER.locate().is_none() {
        eprintln!("skipped: OpenVAF not found");
        return false;
    }
    if libcdept::tools::find_in_path("python3").is_none() {
        eprintln!("skipped: python3 not found");
        return false;
    }
    true
}

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

fn open_example() -> Option<(tempfile::TempDir, Project)> {
    if !tools_for_pdk() || libcdept::xschem::Engine::find_helper().is_none() {
        eprintln!("skipped: xschem helper not built");
        return None;
    }
    let root = std::env::var("PDK_ROOT").unwrap_or_else(|_| "/opt/ext/OpenPDKs".into());
    if !Path::new(&root).join("ihp-sg13g2/libs.tech/verilog-a").is_dir() {
        eprintln!("skipped: ihp-sg13g2 not found in {root}");
        return None;
    }
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/ihp-inv");
    let dir = tempfile::tempdir().unwrap();
    copy_dir(&example, dir.path());
    std::fs::write(dir.path().join("cdept.local.toml"), format!("pdk_root = \"{root}\"\n")).unwrap();
    let project = Project::load(dir.path()).unwrap();
    Some((dir, project))
}

#[test]
fn ihp_inverter_with_psp103_agrees_with_ngspice() {
    let Some((dir, project)) = open_example() else { return };
    let sim = vacask().unwrap();
    let tb = sim::open(&project, "cells", "tb_inv", "vacask").unwrap();
    let results = sim::run_testbench(&project, &tb, &sim, &[]).unwrap();
    let vc: Vec<f64> = results
        .iter()
        .map(|r| r.outcome.as_ref().unwrap_or_else(|e| panic!("{}: {e:#}", r.point.name())).measures["tpd"])
        .collect();
    assert!(vc[1] < vc[0] && vc[0] < vc[2], "ff < tt < ss: {vc:?}");
    // The models were generated, recorded in tech.toml and cached in the project.
    let tech = libcdept::tech::file::load_tech(dir.path()).unwrap().unwrap();
    let set = &tech.models.sets["vacask"];
    assert!(set.generated.is_some() && !set.osdi.is_empty() && set.corners.contains_key("tt"));
    assert!(!set.corners.contains_key("mc"), "statistical corners are not generated");
    assert!(tech.notes.iter().any(|n| n.starts_with("vacask:")));
    assert!(dir.path().join(".cdept/osdi").is_dir());

    // ngspice on the same testbench.
    let Some(ng) = Simulator::find() else { return };
    let ngtb = sim::open(&project, "cells", "tb_inv", "testbench").unwrap();
    let ngres = sim::run_testbench(&project, &ngtb, &ng, &[]).unwrap();
    for (v, n) in vc.iter().zip(&ngres) {
        let n = n.outcome.as_ref().unwrap().measures["tpd"];
        assert!(((v - n) / n).abs() < 0.02, "vacask {v} ngspice {n}");
    }
    eprintln!("tpd vacask {vc:?}");

    // A second run reads the cache and the recorded set: no conversion, no compile.
    let before = std::fs::read_to_string(dir.path().join("tech.toml")).unwrap();
    let again = sim::run_testbench(&project, &tb, &sim, &["tt".into()]).unwrap();
    assert!(again[0].outcome.is_ok());
    assert_eq!(before, std::fs::read_to_string(dir.path().join("tech.toml")).unwrap());
}

#[test]
fn a_device_the_converted_models_lack_is_an_error_that_names_it() {
    let Some((dir, project)) = open_example() else { return };
    let sim = vacask().unwrap();
    // A resistor of the PDK in the testbench netlist: its subcircuit is not defined by
    // the converted files, so the run is refused instead of starting VACASK.
    let tb = sim::open(&project, "cells", "tb_inv", "vacask").unwrap();
    let ready = vacask::models::ready(&project).unwrap();
    let setup = tb.setup.clone();
    let point = RunPoint { corner: "tt".into(), ..Default::default() };
    let net = "t\nx1 a b sg13_hv_nonexistent w=1u l=1u\n.end\n";
    let e = vacask::build_point(&project, &ready, &setup, "vacask", &point, "t", net).unwrap_err();
    let text = format!("{e:#}");
    assert!(text.contains("ihp-sg13g2") && text.contains("sg13_hv_nonexistent") && text.contains("x1"), "{text}");
    let _ = (dir, sim);
}
