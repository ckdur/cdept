//! Real-PDK test: a CACE datasheet for the inverter of examples/ihp-inv imported next to
//! its testbench, run with ngspice at the datasheet's IHP SG13G2 corners and exported
//! with the results. Skipped (with a note) without ngspice, the xschem helper or the PDK
//! (`$CDEPT_TEST_PDK_ROOT` or /opt/ext/OpenPDKs).

use std::path::Path;

use libcdept::Project;
use libcdept::cace::{self, Value, export, import};
use libcdept::sim::{self, Simulator};
use libcdept::testutil::real_pdk;

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

const SHEET: &str = r#"name: inv
description: Inverter delay
PDK: ihp-sg13g2
cace_format: 5.2
paths:
  root: ..
default_conditions:
  vdd:
    unit: V
    typical: 1.2
parameters:
  delay:
    display: Propagation delay
    unit: ps
    spec:
      tpd:
        maximum:
          value: 300
    tool:
      ngspice:
        template: tb_inv.sch
    conditions:
      corner:
        enumerate: [tt, ff, ss]
      temperature:
        typical: 27
"#;

#[test]
fn a_cace_parameter_runs_at_the_ihp_corners() {
    let Some(simulator) = Simulator::find() else {
        eprintln!("skipped: ngspice not found");
        return;
    };
    if libcdept::xschem::Engine::find_helper().is_none() {
        eprintln!("skipped: the xschem helper is not built");
        return;
    }
    let Some(pdk) = real_pdk("ihp-sg13g2") else { return };
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/ihp-inv");
    let dir = tempfile::tempdir().unwrap();
    copy_dir(&example, dir.path());
    std::fs::write(dir.path().join("cdept.local.toml"), format!("pdk_root = \"{}\"\n", pdk.root.display())).unwrap();
    let mut project = Project::load(dir.path()).unwrap();
    let options = import::ImportOptions {
        library: "cells".into(),
        cell: "tb_inv".into(),
        prefix: "cace_".into(),
        source: "inv.yaml".into(),
        ..Default::default()
    };
    let plan = import::plan(&project, SHEET, &options).unwrap();
    let lines = plan.lines(&project.root).join("\n");
    assert!(plan.ok(), "{lines}");
    // The corners are the technology's: no warning about them.
    assert!(!lines.contains("not a corner set"), "{lines}");
    import::apply(&plan).unwrap();
    // CACE's template measured tpd; the testbench says how.
    let file = project.library("cells").unwrap().path.join("tb_inv/cace_delay/setup.toml");
    let text = std::fs::read_to_string(&file).unwrap();
    let at = text.find("\n[").unwrap();
    let text = format!(
        "{}\nmeasures = [\"tran tpd trig v(in) val=0.6 rise=1 targ v(out) val=0.6 fall=1\"]\n\n[[analysis]]\nkind = \"tran\"\nstep = \"5p\"\nstop = \"1.2n\"\n{}",
        &text[..at],
        &text[at..]
    );
    std::fs::write(&file, text).unwrap();
    project.rescan();
    let tb = sim::open(&project, "cells", "tb_inv", "cace_delay").unwrap();
    let results = sim::run_testbench(&project, &tb, &simulator, &[]).unwrap();
    let names: Vec<String> = results.iter().map(|r| r.point.name()).collect();
    assert_eq!(names, ["tt_27", "ff_27", "ss_27"]);
    for r in &results {
        assert!(r.outcome.is_ok(), "{}: {:?}", r.point.name(), r.outcome.as_ref().err());
    }
    let out = export::export(
        &project,
        "cells",
        "tb_inv",
        &export::ExportOptions { view: Some("cace_delay".into()), results: true },
    )
    .unwrap();
    let back = cace::parse(&out.text).unwrap();
    let param = &back["parameters"]["delay"];
    assert_eq!(param["spec"], cace::parse(SHEET).unwrap()["parameters"]["delay"]["spec"]);
    let r = &param["results"][0];
    assert_eq!(r["runs"], Value::from(3));
    assert_eq!(r["status"], Value::from("pass"), "{}", out.text);
    // In ps, like the spec: the inverter of the example is well below 300 ps.
    let max = r["tpd"]["maximum"]["value"].as_f64().unwrap();
    assert!(max > 1.0 && max < 300.0, "{max}");
}
