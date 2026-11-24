//! Time limits of the simulators and of OpenVAF, and whether VACASK can run a
//! technology (docs/VACASK.md, docs/JOBS.md). Hermetic: the programs are small shell
//! scripts and the technology is written here; nothing needs a PDK or a real simulator.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use libcdept::Project;
use libcdept::batch::openvaf::{self, Compile};
use libcdept::batch::{Cancelled, TimedOut, Watch};
use libcdept::jobs::{JobSpec, JobState, Queue};
use libcdept::sim::run::{self, SimKind, Simulator};
use libcdept::testutil::{hermetic_env, hermetic_env_with};
use libcdept::vacask::{Availability, availability, models};

fn fake(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// Run `deck` with `sim` as a job of a queue with a time limit; the error text of the
/// point and how long it took.
fn run_limited(
    sim: Simulator,
    limit: Option<Duration>,
) -> (Result<(), String>, JobState, Duration) {
    let dir = tempfile::tempdir().unwrap();
    let q = Queue::new(1);
    let slot: Arc<Mutex<Option<Result<(), String>>>> = Arc::default();
    let s = slot.clone();
    let run_dir = dir.path().join("run");
    let deck = if sim.kind == SimKind::Vacask {
        "ground 0\n"
    } else {
        ".end\n"
    };
    let t = Instant::now();
    let id = q.submit(JobSpec::func("point", move |ctx| {
        let r = run::run_in_job_limited(&sim, deck, &run_dir, &[], ctx, limit)
            .map(|_| ())
            .map_err(|e| format!("{e:#}"));
        *s.lock().unwrap() = Some(r.clone());
        r.map_err(|e| eyre::eyre!(e))
    }));
    let info = q.wait(id).unwrap();
    let r = slot.lock().unwrap().take().unwrap();
    (r, info.state, t.elapsed())
}

#[test]
fn a_point_that_runs_too_long_is_killed_and_ends_as_an_error_for_every_simulator() {
    let dir = tempfile::tempdir().unwrap();
    // Each program prints something, then sleeps for a long time. ngspice writes its
    // log itself (`-o out.log`); Xyce and VACASK print to the job's own file.
    let ng = fake(
        dir.path(),
        "slow-ngspice",
        "echo 'Doing analysis at TEMP = 27' > out.log; echo 'tran: 12%' >> out.log; exec sleep 30",
    );
    let xyce = fake(
        dir.path(),
        "slow-xyce",
        "echo 'Xyce: step 12'; exec sleep 30",
    );
    let vacask = fake(
        dir.path(),
        "slow-vacask",
        "echo 'Running analysis tran1'; exec sleep 30",
    );
    for (kind, program, last) in [
        (SimKind::Ngspice, ng, "tran: 12%"),
        (SimKind::Xyce, xyce, "Xyce: step 12"),
        (SimKind::Vacask, vacask, "Running analysis tran1"),
    ] {
        let (result, state, took) = run_limited(
            Simulator { kind, program },
            Some(Duration::from_millis(400)),
        );
        let err = result.unwrap_err();
        assert!(
            err.starts_with("run point exceeded its time limit of 400ms") && err.contains(last),
            "{kind:?}: {err}"
        );
        assert_eq!(state, JobState::Failed, "{kind:?}");
        assert!(
            took < Duration::from_secs(10),
            "{kind:?}: killed, not waited for"
        );
    }
}

#[test]
fn a_fast_point_and_a_zero_limit_are_not_stopped() {
    let dir = tempfile::tempdir().unwrap();
    let quick = fake(
        dir.path(),
        "quick-ngspice",
        "sleep 0.3; echo 'gain = 2.5' > out.log; echo raw > out.raw",
    );
    let sim = Simulator::ngspice(quick);
    let (r, state, _) = run_limited(sim.clone(), Some(Duration::from_secs(60)));
    assert!(r.is_ok() && state == JobState::Done, "{r:?}");
    // Zero (and None) is no limit at all.
    let (r, _, _) = run_limited(sim.clone(), Some(Duration::ZERO));
    assert!(r.is_ok(), "{r:?}");
    let (r, _, _) = run_limited(sim, None);
    assert!(r.is_ok(), "{r:?}");
}

#[test]
fn the_limit_comes_from_the_command_line_the_testbench_or_the_project() {
    let _env = hermetic_env();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("cdept.toml"),
        "name = \"t\"\n[sim]\ntimeout = \"90s\"\nopenvaf_timeout = 120\n",
    )
    .unwrap();
    let project = Project::load(dir.path()).unwrap();
    assert!(project.warnings.is_empty(), "{:?}", project.warnings);
    assert_eq!(project.sim.timeout, Some(Duration::from_secs(90)));
    assert_eq!(project.sim.openvaf_timeout, Some(Duration::from_secs(120)));
    let setup = |t: &str| libcdept::sim::Setup::parse(t).unwrap();
    let opts = libcdept::sim::RunOptions::default();
    // The project's.
    assert_eq!(
        opts.time_limit(&project, &setup("")).unwrap(),
        Some(Duration::from_secs(90))
    );
    // The testbench's wins over the project's: a string, or a number of seconds.
    assert_eq!(
        opts.time_limit(&project, &setup("timeout = \"10m\""))
            .unwrap(),
        Some(Duration::from_secs(600))
    );
    assert_eq!(
        opts.time_limit(&project, &setup("timeout = 45")).unwrap(),
        Some(Duration::from_secs(45))
    );
    // "0" is none.
    assert_eq!(
        opts.time_limit(&project, &setup("timeout = \"0\""))
            .unwrap(),
        None
    );
    // The command line wins over both.
    let cli = libcdept::sim::RunOptions {
        timeout: Some(Duration::from_secs(5)),
        ..Default::default()
    };
    assert_eq!(
        cli.time_limit(&project, &setup("timeout = \"10m\""))
            .unwrap(),
        Some(Duration::from_secs(5))
    );
    // Nothing set anywhere: thirty minutes.
    let other = tempfile::tempdir().unwrap();
    std::fs::write(other.path().join("cdept.toml"), "name = \"u\"\n").unwrap();
    let plain = Project::load(other.path()).unwrap();
    assert_eq!(
        opts.time_limit(&plain, &setup("")).unwrap(),
        Some(Duration::from_secs(30 * 60))
    );
    // A bad value is an error of the testbench.
    assert!(libcdept::sim::Setup::parse("timeout = \"soon\"").is_err());
    // And a bad value in cdept.toml a warning, not a failure.
    let bad = tempfile::tempdir().unwrap();
    std::fs::write(
        bad.path().join("cdept.toml"),
        "name = \"b\"\n[sim]\ntimeout = \"soon\"\n",
    )
    .unwrap();
    let project = Project::load(bad.path()).unwrap();
    assert!(project.warnings.iter().any(|w| w.contains("[sim] timeout")));
    assert_eq!(project.sim.timeout, None);
    // The setup round-trips.
    let s = setup("timeout = \"10m\"");
    assert_eq!(s.timeout, "10m");
    assert!(toml::to_string(&s).unwrap().contains("timeout = \"10m\""));
}

#[test]
fn openvaf_is_cancellable_and_bound_by_a_time_limit() {
    let dir = tempfile::tempdir().unwrap();
    let slow = fake(
        dir.path(),
        "openvaf-slow",
        "echo 'compiling psp103.va'; echo partial > \"$2\"; exec sleep 30",
    );
    let source = dir.path().join("psp103.va");
    std::fs::write(&source, "// va\n").unwrap();
    let compile = |name: &str| Compile {
        source: source.clone(),
        out: dir.path().join(name).join("psp103.osdi"),
        ..Default::default()
    };
    // The time limit.
    let c = compile("a");
    let t = Instant::now();
    let watch = Watch {
        timeout: Some(Duration::from_millis(300)),
        ..Default::default()
    };
    let err = openvaf::compile_watched(&slow, &c, &[], &watch).unwrap_err();
    let text = format!("{err:#}");
    assert!(
        text.contains("OpenVAF ran past its time limit of 300ms")
            && text.contains("compiling psp103.va")
            && text.contains("openvaf_timeout"),
        "{text}"
    );
    assert!(err.downcast_ref::<TimedOut>().is_some());
    assert!(!c.out.exists(), "a half-written library is removed");
    // The cancel flag (a job's).
    let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let f = flag.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        f.store(true, std::sync::atomic::Ordering::SeqCst);
    });
    let err =
        openvaf::compile_watched(&slow, &compile("b"), &[], &Watch::cancel(flag)).unwrap_err();
    assert!(err.downcast_ref::<Cancelled>().is_some(), "{err:#}");
    // The cached compile stops the same way and caches nothing.
    let cache = dir.path().join("cache");
    let err =
        openvaf::compile_cached_watched(&slow, &compile("c"), &cache, &[], &watch).unwrap_err();
    assert!(err.downcast_ref::<TimedOut>().is_some());
    assert!(!cache.join("psp103.osdi").exists());
    assert!(
        t.elapsed() < Duration::from_secs(10),
        "killed, not waited for"
    );
}

/// A project whose technology has `ngspice_lib` as its only model file: a snapshot
/// written here, for a PDK that is not installed. `osdi` names Verilog-A models to
/// compile.
fn project_with_models(dir: &Path, model_text: &str, osdi: &[&str]) -> Project {
    let models = dir.join("models");
    std::fs::create_dir_all(&models).unwrap();
    std::fs::write(models.join("corner.lib"), model_text).unwrap();
    std::fs::write(
        dir.join("cdept.toml"),
        "name = \"t\"\ntechnology = \"fakepdk\"\n",
    )
    .unwrap();
    let osdi_list = osdi
        .iter()
        .map(|o| format!("\"/nowhere/{o}.osdi\""))
        .collect::<Vec<_>>()
        .join(", ");
    std::fs::write(
        dir.join("tech.toml"),
        format!(
            "schema = \"cdept-tech\"\nversion = 1\n[pdk]\nname = \"fakepdk\"\n\
             [models]\ndefault = \"ngspice\"\ncorner = \"tt\"\n\
             [models.ngspice]\nsimulator = \"ngspice\"\ndir = \"{}\"\nlibraries = [\"corner.lib\"]\n\
             osdi = [{osdi_list}]\n\
             [models.ngspice.corners]\ntt = [\"corner.lib tt\"]\n",
            models.display()
        ),
    )
    .unwrap();
    Project::load(dir).unwrap()
}

const PSP_LIB: &str = ".lib tt\n.model nm nmos level=1 vto=0.4\n.endl\n";
const SKY_LIB: &str = ".lib tt\n.model nfet_01v8__model.0 nmos\n+ level = 54\n+ lmin = 1.0e-07 lmax = 1.0e-06 wmin = 4.2e-07\n+ wmax = 1e-6\n.endl\n";

#[test]
fn without_openvaf_the_models_that_need_it_are_unavailable_with_the_fix_in_the_message() {
    let tmp = tempfile::tempdir().unwrap();
    let vacask = fake(tmp.path(), "vacask", "exit 0");
    let nowhere = tmp.path().join("no-such-openvaf");
    let _env = hermetic_env_with(&[
        ("CDEPT_VACASK", Some(vacask.as_os_str())),
        ("CDEPT_OPENVAF", Some(nowhere.as_os_str())),
        ("PATH", Some(tmp.path().join("empty").as_os_str())),
    ]);
    let proj_dir = tmp.path().join("p");
    std::fs::create_dir_all(&proj_dir).unwrap();
    let project = project_with_models(&proj_dir, PSP_LIB, &["psp103", "psp103_nqs"]);
    let a = availability(&project);
    let why = a.reason().unwrap_or_else(|| panic!("{a:?}"));
    assert_eq!(
        why,
        "OpenVAF not found: put openvaf-r on PATH or set CDEPT_OPENVAF; needed to compile psp103, psp103_nqs"
    );
    // The run is refused with the same words, before the netlist or VACASK.
    let tb = libcdept::sim::Testbench {
        library: "l".into(),
        cell: "c".into(),
        view: "v".into(),
        setup: libcdept::sim::Setup::parse("simulator = \"vacask\"").unwrap(),
        schematic: tmp.path().join("none.sch"),
    };
    let sim = Simulator {
        kind: SimKind::Vacask,
        program: vacask.clone(),
    };
    let err = match libcdept::sim::run_testbench_with(&project, &tb, &sim, &[], &Default::default())
    {
        Ok(_) => panic!("must be refused"),
        Err(e) => format!("{e:#}"),
    };
    assert_eq!(err, why, "one clear error, not Device 'psp103va' not found");
}

#[test]
fn availability_follows_the_tools_and_the_technology() {
    let tmp = tempfile::tempdir().unwrap();
    let vacask = fake(tmp.path(), "vacask", "exit 0");
    let openvaf = fake(tmp.path(), "openvaf-r", "exit 0");
    fake(tmp.path(), "python3", "exit 0");
    let path = std::env::join_paths([tmp.path()]).unwrap();
    let _env = hermetic_env_with(&[
        ("CDEPT_VACASK", Some(vacask.as_os_str())),
        ("CDEPT_OPENVAF", Some(openvaf.as_os_str())),
        ("PATH", Some(path.as_os_str())),
    ]);
    let a = tmp.path().join("a");
    std::fs::create_dir_all(&a).unwrap();
    let ok = project_with_models(&a, PSP_LIB, &["psp103"]);
    assert_eq!(availability(&ok), Availability::Ready);
    assert!(availability(&ok).line().starts_with("VACASK: available"));
    // sky130's way of writing transistors: binned BSIM4, which ng2vc cannot convert.
    let b = tmp.path().join("b");
    std::fs::create_dir_all(&b).unwrap();
    let sky = project_with_models(&b, SKY_LIB, &[]);
    let why = availability(&sky).reason().unwrap().to_string();
    assert!(
        why.contains("'fakepdk'") && why.contains("binned BSIM4") && why.contains("corner.lib"),
        "{why}"
    );
    // A program that is not there.
    drop(_env);
    let gone = tmp.path().join("gone");
    let _env = hermetic_env_with(&[
        ("CDEPT_VACASK", Some(gone.as_os_str())),
        ("CDEPT_OPENVAF", Some(openvaf.as_os_str())),
    ]);
    let why = availability(&ok).reason().unwrap().to_string();
    assert!(
        why.starts_with("VACASK not found") && why.contains("CDEPT_VACASK"),
        "{why}"
    );
}

#[test]
fn missing_compiled_libraries_stop_the_run_with_one_clear_error() {
    let g = models::Generated {
        pdk: "ihp-sg13g2".into(),
        missing: vec![
            "psp103: did not compile (error: bad)".into(),
            "r3_cmc: no Verilog-A source".into(),
        ],
        ..Default::default()
    };
    let err = format!("{}", models::check_libraries(&g).unwrap_err());
    assert!(
        err.contains("compiled model libraries of the technology 'ihp-sg13g2' are missing")
            && err.contains("psp103: did not compile")
            && err.contains("r3_cmc"),
        "{err}"
    );
    let g = models::Generated {
        openvaf_missing: true,
        ..g
    };
    let err = format!("{}", models::check_libraries(&g).unwrap_err());
    assert_eq!(
        err,
        "OpenVAF not found: put openvaf-r on PATH or set CDEPT_OPENVAF; needed to compile psp103, r3_cmc"
    );
    assert!(models::check_libraries(&models::Generated::default()).is_ok());
}
