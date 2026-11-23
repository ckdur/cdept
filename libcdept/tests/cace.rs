//! CACE datasheets: import into a scratch project, export back, compare; and an imported
//! testbench run with ngspice on a made-up technology (skipped, with a note, without
//! ngspice or the xschem helper). Hermetic (docs/TESTS.md section 8).

use std::path::Path;

use libcdept::Project;
use libcdept::cace::{self, Mapping, Value, export, import};
use libcdept::sim::{self, Setup, Simulator};

const DEMO: &str = include_str!("data/cace/demo_ota.yaml");

/// A project with one `cells` library, optionally with a made-up technology whose
/// corners tt/ff/ss only change a resistor.
fn project(root: &Path, tech: bool) -> Project {
    std::fs::create_dir_all(root.join("libs/cells")).unwrap();
    std::fs::write(
        root.join("cdept.toml"),
        "name = \"cace\"\n[libraries]\ncells = { path = \"libs/cells\", layout = \"cells\" }\n",
    )
    .unwrap();
    if tech {
        let models = root.join("models");
        std::fs::create_dir_all(&models).unwrap();
        for (corner, r) in [("tt", "1k"), ("ff", "800"), ("ss", "1.2k")] {
            std::fs::write(
                models.join(format!("{corner}.lib")),
                format!("* corner {corner}\n.param rval={r}\n"),
            )
            .unwrap();
        }
        std::fs::write(
            root.join("tech.toml"),
            format!(
                "[pdk]\nname = \"made-up\"\n[models]\ndefault = \"ngspice\"\ncorner = \"tt\"\n\
                 [models.ngspice]\nsimulator = \"ngspice\"\ndir = \"{}\"\n\
                 [models.ngspice.corners]\ntt = [\"tt.lib\"]\nff = [\"ff.lib\"]\nss = [\"ss.lib\"]\nmc = [\"tt.lib\"]\n",
                models.display()
            ),
        )
        .unwrap();
    }
    Project::load(root).unwrap()
}

fn options(cell: &str) -> import::ImportOptions {
    import::ImportOptions {
        library: "cells".into(),
        cell: cell.into(),
        source: "demo_ota.yaml".into(),
        ..Default::default()
    }
}

fn params(sheet: &Mapping) -> &Mapping {
    sheet["parameters"].as_mapping().unwrap()
}

fn setup_of(project: &Project, cell: &str, view: &str) -> Setup {
    Setup::load(
        &project
            .library("cells")
            .unwrap()
            .path
            .join(cell)
            .join(view)
            .join("setup.toml"),
    )
    .unwrap()
}

#[test]
fn import_maps_the_datasheet_and_reports_the_rest() {
    let _env = libcdept::testutil::hermetic_env();
    let dir = tempfile::tempdir().unwrap();
    let mut p = project(dir.path(), true);
    let plan = import::plan(&p, DEMO, &options("ota")).unwrap();
    let lines = plan.lines(&p.root).join("\n");
    assert!(plan.ok(), "{lines}");
    // Four simulated parameters and the suite; the layout checks are kept in the suite.
    let views: Vec<&str> = plan.files.iter().map(|f| f.view.as_str()).collect();
    assert_eq!(
        views,
        ["dc_params", "offset", "gain_mc", "transient", "datasheet"],
        "{lines}"
    );
    for needle in [
        "corner ss, tt, ff -> corners",
        "vdd 1.08, 1.2, 1.32 -> [[sweep]]",
        "ib = 1.2e-5 -> [variables] (uA scaled to SI, as CACE does)",
        "corner mc collated over iterations (20 values) -> [montecarlo] runs = 20",
        "spec vos [mV] -> [[spec]] -0.005 .. 0.005",
        "spec ugf [MHz] -> [[spec]] >= 10000000",
        "magic_area: tool magic_area: not a simulation",
        "klayout_lvs: tool klayout_lvs: not a simulation",
        "ibias: maximum 15 is not checked (fail: false)",
        "transient: status skip: not a test of the suite",
        "plot: {gain_vs_temp}",
        "pins: datasheet header",
        "the testbench must measure a0, ugf, pm",
        "has no xschem schematic",
    ] {
        assert!(lines.contains(needle), "no '{needle}' in\n{lines}");
    }
    // Nothing is written by a plan.
    assert!(!p.library("cells").unwrap().path.join("ota").exists());
    import::apply(&plan).unwrap();
    p.rescan();
    let dc = setup_of(&p, "ota", "dc_params");
    assert_eq!(dc.corners, ["ss", "tt", "ff"]);
    assert_eq!(dc.temperatures, [-40.0, 27.0, 125.0]);
    assert_eq!(dc.sweeps[0].variable, "vdd");
    assert_eq!(dc.sweeps[0].values, ["1.08", "1.2", "1.32"]);
    assert_eq!(dc.variables["vcm"], "0.6");
    assert_eq!(dc.variables["ib"], "1e-5");
    let ugf = dc.specs.iter().find(|s| s.name == "ugf").unwrap();
    assert_eq!((ugf.min, ugf.max), (Some(1e7), None));
    let cace_table = dc.cace.as_ref().unwrap();
    assert_eq!(cace_table["template"].as_str(), Some("ac.sch"));
    assert_eq!(cace_table["units"]["pm"].as_str(), Some("°"));
    let mc = setup_of(&p, "ota", "gain_mc").montecarlo.unwrap();
    assert_eq!(mc.runs, 20);
    assert!(
        mc.corner.is_empty(),
        "the technology's mc corner is the default"
    );
    // The suite lists the tests that are not skipped and loads.
    let suite = sim::suite::open(&p, "cells", "ota", "datasheet").unwrap();
    let tests: Vec<&str> = suite.tests.iter().map(|t| t.view.as_str()).collect();
    assert_eq!(tests, ["dc_params", "offset", "gain_mc"]);
    assert_eq!(suite.tests[0].name, "DC Params");
    // Again: the views exist.
    let again = import::plan(&p, DEMO, &options("ota")).unwrap();
    assert!(!again.ok());
    assert!(again.blockers[0].contains("exists"), "{:?}", again.blockers);
    let over = import::plan(
        &p,
        DEMO,
        &import::ImportOptions {
            overwrite: true,
            ..options("ota")
        },
    )
    .unwrap();
    assert!(over.ok() && over.files.iter().all(|f| f.exists));
}

#[test]
fn export_gives_back_what_was_imported() {
    let _env = libcdept::testutil::hermetic_env();
    let dir = tempfile::tempdir().unwrap();
    let mut p = project(dir.path(), true);
    import::apply(&import::plan(&p, DEMO, &options("ota")).unwrap()).unwrap();
    p.rescan();
    let original = cace::parse(DEMO).unwrap();
    let out = export::export(&p, "cells", "ota", &export::ExportOptions::default()).unwrap();
    assert!(out.suite && out.view == "datasheet");
    let back = cace::parse(&out.text).unwrap();
    // Every parameter, in order, as it was: conditions, spec limits, tools, plots.
    let keys = |m: &Mapping| {
        m.keys()
            .map(|k| k.as_str().unwrap().to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(keys(params(&back)), keys(params(&original)));
    for (k, v) in params(&original) {
        assert_eq!(&params(&back)[k], v, "parameter {k:?}:\n{}", out.text);
    }
    // The header too.
    for (k, v) in &original {
        if k.as_str() != Some("parameters") {
            assert_eq!(&back[k], v, "{k:?}");
        }
    }
    assert!(
        out.notes.iter().any(|n| n.contains("has not been run")),
        "{:?}",
        out.notes
    );

    // Edit the testbenches, export again: the CDePT values win.
    let dc_file = p
        .library("cells")
        .unwrap()
        .path
        .join("ota/dc_params/setup.toml");
    let text = std::fs::read_to_string(&dc_file).unwrap();
    let text = text.replace(
        "temperatures = [-40.0, 27.0, 125.0]",
        "temperatures = [27.0]",
    );
    let text = text.replace(
        "name = \"a0\"\nmin = 50.0",
        "name = \"a0\"\nmin = 55.0\nmax = 1000.0",
    );
    std::fs::write(&dc_file, text).unwrap();
    let mc_file = p
        .library("cells")
        .unwrap()
        .path
        .join("ota/gain_mc/setup.toml");
    let text = std::fs::read_to_string(&mc_file)
        .unwrap()
        .replace("runs = 20", "runs = 5");
    std::fs::write(&mc_file, text).unwrap();
    let out = export::export(
        &p,
        "cells",
        "ota",
        &export::ExportOptions {
            results: false,
            ..Default::default()
        },
    )
    .unwrap();
    let back = cace::parse(&out.text).unwrap();
    let dc = params(&back)["dc_params"].as_mapping().unwrap();
    let temp = dc["conditions"]["temperature"].as_mapping().unwrap();
    assert_eq!(temp.get("typical"), Some(&Value::from(27)));
    assert!(temp.get("minimum").is_none());
    assert_eq!(dc["spec"]["a0"]["minimum"]["value"], Value::from(55));
    assert_eq!(dc["spec"]["a0"]["maximum"]["value"], Value::from(1000));
    // The untouched parts stay as read.
    assert_eq!(
        dc["conditions"]["vdd"],
        params(&original)["dc_params"]["conditions"]["vdd"]
    );
    assert_eq!(
        dc["spec"]["ugf"],
        params(&original)["dc_params"]["spec"]["ugf"]
    );
    let iterations = params(&back)["gain_mc"]["conditions"]["iterations"]
        .as_mapping()
        .unwrap();
    assert_eq!(iterations.get("maximum"), Some(&Value::from(5)));
    assert!(
        out.notes
            .iter()
            .any(|n| n.contains("condition temperature is 27")),
        "{:?}",
        out.notes
    );

    // A testbench alone takes the header of the suite it came from.
    let one = export::export(
        &p,
        "cells",
        "ota",
        &export::ExportOptions {
            view: Some("offset".into()),
            results: false,
        },
    )
    .unwrap();
    let back = cace::parse(&one.text).unwrap();
    assert_eq!(one.parameters, ["offset"]);
    assert_eq!(back["name"], Value::from("demo_ota"));
    assert_eq!(params(&back)["offset"], params(&original)["offset"]);
}

#[test]
fn a_testbench_that_was_not_imported_becomes_a_parameter() {
    let _env = libcdept::testutil::hermetic_env();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut p = project(root, true);
    let view = p.library("cells").unwrap().path.join("tb/testbench");
    std::fs::create_dir_all(&view).unwrap();
    std::fs::create_dir_all(view.parent().unwrap().join("schematic")).unwrap();
    std::fs::write(
        view.parent().unwrap().join("schematic/tb.sch"),
        "v {xschem version=3.4.5 file_version=1.2\n}\n",
    )
    .unwrap();
    std::fs::write(
        view.join("setup.toml"),
        "corners = [\"tt\", \"ss\"]\ntemperatures = [27]\n[variables]\nvdd = \"1.2\"\n[[sweep]]\nvariable = \"cl\"\nvalues = [\"1p\", \"2p\"]\n[[spec]]\nname = \"tpd\"\nmax = 3e-10\n",
    )
    .unwrap();
    p.rescan();
    let out = export::export(&p, "cells", "tb", &export::ExportOptions::default()).unwrap();
    assert!(!out.suite);
    let back = cace::parse(&out.text).unwrap();
    assert_eq!(back["cace_format"], Value::from(5.2));
    let param = params(&back)["testbench"].as_mapping().unwrap();
    assert_eq!(param["tool"]["ngspice"]["template"], Value::from("tb.sch"));
    let corner: Vec<&str> = param["conditions"]["corner"]["enumerate"]
        .as_sequence()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(corner, ["tt", "ss"]);
    assert_eq!(param["conditions"]["vdd"]["typical"], Value::from(1.2));
    assert_eq!(param["conditions"]["cl"]["enumerate"][0], Value::from("1p"));
    assert_eq!(param["spec"]["tpd"]["maximum"]["value"], Value::from(3e-10));
    // And it imports again into the same table.
    let plan = import::plan(
        &p,
        &out.text,
        &import::ImportOptions {
            prefix: "again_".into(),
            ..options("tb")
        },
    )
    .unwrap();
    assert!(plan.ok(), "{:?}", plan.blockers);
    let s = Setup::parse(&plan.files[0].text).unwrap();
    assert_eq!(
        (s.corners.clone(), s.temperatures.clone()),
        (vec!["tt".to_string(), "ss".into()], vec![27.0])
    );
    assert_eq!(s.specs[0].max, Some(3e-10));
}

#[test]
fn an_old_datasheet_or_a_corner_the_technology_lacks_is_reported() {
    let _env = libcdept::testutil::hermetic_env();
    let dir = tempfile::tempdir().unwrap();
    let p = project(dir.path(), true);
    let e = import::plan(
        &p,
        "electrical_parameters: []\nparameters: {}\n",
        &options("x"),
    )
    .unwrap_err();
    assert!(format!("{e:#}").contains("4.x"));
    let sheet = "name: x\ndescription: d\nPDK: made-up\ncace_format: 5.2\nparameters:\n  p:\n    tool:\n      ngspice:\n        template: t.sch\n    conditions:\n      corner:\n        enumerate: [tt, fs]\n";
    let plan = import::plan(&p, sheet, &options("x")).unwrap();
    let lines = plan.lines(&p.root).join("\n");
    assert!(
        lines.contains("corner 'fs' is not a corner set of the technology"),
        "{lines}"
    );
    let only_drc = "name: x\nparameters:\n  drc:\n    tool: magic_drc\n";
    let plan = import::plan(&p, only_drc, &options("x")).unwrap();
    assert!(
        !plan.ok() && plan.blockers[0].contains("no parameter is simulated"),
        "{:?}",
        plan.blockers
    );
}

/// An RC delay: the datasheet's corners only change the resistor (the made-up
/// technology), the load is a condition in pF.
const RC_SHEET: &str = r#"name: rc
description: RC delay
PDK: made-up
cace_format: 5.2
paths:
  root: ..
default_conditions:
  vdd:
    unit: V
    typical: 1.8
  temperature:
    unit: °C
    typical: 27
parameters:
  delay:
    display: RC delay
    unit: ns
    spec:
      tdly:
        maximum:
          value: 1
    tool:
      ngspice:
        template: rc.sch
    conditions:
      corner:
        enumerate: [tt, ff, ss]
      cl:
        unit: pF
        typical: 1
"#;

const RC_SCH: &str = "v {xschem version=3.4.5 file_version=1.2\n}\nG {}\nK {}\nV {}\nS {}\nE {}\n\
N -220 0 -70 0 {lab=in}\nN -10 0 120 0 {lab=out}\n\
C {vsource.sym} -220 30 0 0 {name=Vin value=\"pulse(0 \\{vdd\\} 1n 10p 10p 10n 20n)\" savecurrent=false}\n\
C {gnd.sym} -220 60 0 0 {name=l1 lab=0}\n\
C {res.sym} -40 0 1 0 {name=R1 value=\\{rval\\} m=1}\n\
C {capa.sym} 120 30 0 0 {name=C1 value=\\{cl\\} m=1}\n\
C {gnd.sym} 120 60 0 0 {name=l2 lab=0}\n\
C {lab_wire.sym} -160 0 0 0 {name=l3 lab=in}\nC {lab_wire.sym} 80 0 0 0 {name=l4 lab=out}\n";

#[test]
fn an_imported_testbench_runs_with_ngspice() {
    let _env = libcdept::testutil::hermetic_env();
    let Some(simulator) = Simulator::find() else {
        eprintln!("skipped: ngspice not found");
        return;
    };
    if libcdept::xschem::Engine::find_helper().is_none() {
        eprintln!("skipped: the xschem helper is not built");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let mut p = project(dir.path(), true);
    let sch = p.library("cells").unwrap().path.join("rc/schematic");
    std::fs::create_dir_all(&sch).unwrap();
    std::fs::write(sch.join("rc.sch"), RC_SCH).unwrap();
    p.rescan();
    let plan = import::plan(
        &p,
        RC_SHEET,
        &import::ImportOptions {
            source: "rc.yaml".into(),
            ..options("rc")
        },
    )
    .unwrap();
    assert!(plan.ok(), "{:?}", plan.blockers);
    import::apply(&plan).unwrap();
    // The datasheet's template measured tdly; here the setup does.
    let file = p.library("cells").unwrap().path.join("rc/delay/setup.toml");
    let text = std::fs::read_to_string(&file).unwrap();
    let at = text.find("\n[").unwrap();
    let text = format!(
        "{}\nmeasures = [\"tran tdly trig v(in) val=0.9 rise=1 targ v(out) val=0.9 rise=1\"]\n\n[[analysis]]\nkind = \"tran\"\nstep = \"10p\"\nstop = \"8n\"\n{}",
        &text[..at],
        &text[at..]
    );
    std::fs::write(&file, text).unwrap();
    p.rescan();
    let tb = sim::open(&p, "cells", "rc", "delay").unwrap();
    assert_eq!(tb.setup.variables["cl"], "1e-12");
    let results = sim::run_testbench(&p, &tb, &simulator, &[]).unwrap();
    let names: Vec<String> = results.iter().map(|r| r.point.name()).collect();
    assert_eq!(names, ["tt_27", "ff_27", "ss_27"]);
    let tdly: Vec<f64> = results
        .iter()
        .map(|r| {
            let out = r
                .outcome
                .as_ref()
                .unwrap_or_else(|e| panic!("{}: {e:#}", r.point.name()));
            *out.measures.get("tdly").unwrap_or_else(|| {
                panic!(
                    "no tdly: {}\n{}",
                    std::fs::read_to_string(&out.log).unwrap_or_default(),
                    std::fs::read_to_string(&out.deck).unwrap_or_default()
                )
            })
        })
        .collect();
    // ln 2 R C: 0.69 ns at 1k, less at 800, more at 1.2k; all within 1 ns.
    assert!((tdly[0] - 0.693e-9).abs() < 0.05e-9, "{tdly:?}");
    assert!(tdly[1] < tdly[0] && tdly[0] < tdly[2], "{tdly:?}");
    // The export has the results: min/typ/max in ns and the verdicts.
    let out = export::export(&p, "cells", "rc", &export::ExportOptions::default()).unwrap();
    let back = cace::parse(&out.text).unwrap();
    let r = &params(&back)["delay"]["results"][0];
    assert_eq!(r["status"], Value::from("pass"), "{}", out.text);
    assert_eq!(r["runs"], Value::from(3));
    let max = r["tdly"]["maximum"]["value"].as_f64().unwrap();
    assert!((max - tdly[2] * 1e9).abs() < 1e-6, "{max} {tdly:?}");
    assert_eq!(r["tdly"]["maximum"]["status"], Value::from("pass"));
    // Spec and conditions are those of the datasheet.
    assert_eq!(
        params(&back)["delay"]["spec"],
        cace::parse(RC_SHEET).unwrap()["parameters"]["delay"]["spec"]
    );
}
