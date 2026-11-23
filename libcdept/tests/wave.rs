//! Opening waveforms: UberWave is a separate program; here it is a fake shell script that
//! records the arguments and the working folder it was started with (hermetic), plus a
//! smoke test with the real viewer when one is found (`$CDEPT_UBERWAVE`, PATH or a
//! sibling build) and a display exists.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use libcdept::testutil::hermetic_env_with;
use libcdept::wave;

const OP: &str = "Title: t\nPlotname: Operating Point\nFlags: real\nNo. Variables: 2\nNo. Points: 1\nVariables:\n\t0\tv(a)\tvoltage\n\t1\ti(v1)\tcurrent\nValues:\n 0\t1.0\n\t0.5\n\n";
const TRAN: &str = "Title: t\nPlotname: Transient Analysis\nFlags: real\nNo. Variables: 3\nNo. Points: 2\nVariables:\n\t0\ttime\ttime\n\t1\tv(a)\tvoltage\n\t2\tv(b)\tvoltage\nValues:\n 0\t0\n\t1\n\t2\n 1\t1e-9\n\t1\n\t0\n\n";

fn fake_viewer(dir: &Path) -> PathBuf {
    let program = dir.join("fake-uberwave");
    std::fs::write(&program, "#!/bin/sh\ncase \"$1\" in --version) exit 0;; esac\nd=$(dirname \"$0\")\nprintf '%s\\n' \"$@\" > \"$d/args.txt\"\npwd > \"$d/cwd.txt\"\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    program
}

fn wait_for(file: &Path) -> String {
    let end = Instant::now() + Duration::from_secs(10);
    while Instant::now() < end {
        if let Ok(text) = std::fs::read_to_string(file)
            && !text.is_empty()
        {
            return text;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("{} was not written", file.display());
}

#[cfg(unix)]
#[test]
fn a_raw_file_is_opened_with_the_transient_analysis_chosen() {
    let tmp = tempfile::tempdir().unwrap();
    let program = fake_viewer(tmp.path());
    let _env = hermetic_env_with(&[("CDEPT_UBERWAVE", Some(program.as_os_str()))]);
    let run = tmp.path().join("run");
    std::fs::create_dir_all(&run).unwrap();
    let raw = run.join("out.raw");
    std::fs::write(&raw, format!("{OP}{TRAN}")).unwrap();

    let found = wave::analyses(&raw).unwrap();
    assert_eq!(
        found
            .iter()
            .map(|a| (a.kind.as_str(), a.signals))
            .collect::<Vec<_>>(),
        [("op", 2), ("tran", 3)]
    );

    let started = wave::open_raw(None, &raw, None).unwrap();
    assert!(
        started.text().ends_with("out.raw --analysis tran"),
        "{}",
        started.text()
    );
    let args: Vec<String> = wait_for(&tmp.path().join("args.txt"))
        .lines()
        .map(String::from)
        .collect();
    assert_eq!(args, [raw.to_str().unwrap(), "--analysis", "tran"]);
    // The working folder is the run point's.
    let cwd = wait_for(&tmp.path().join("cwd.txt"));
    assert_eq!(
        Path::new(cwd.trim()).canonicalize().unwrap(),
        run.canonicalize().unwrap()
    );

    // An explicit analysis, by its kind; one the file does not have is an error.
    std::fs::remove_file(tmp.path().join("args.txt")).unwrap();
    wave::open_raw(None, &raw, Some("op")).unwrap();
    assert_eq!(
        wait_for(&tmp.path().join("args.txt")).lines().last(),
        Some("op")
    );
    assert!(wave::open_raw(None, &raw, Some("ac")).is_err());
}

#[cfg(unix)]
#[test]
fn a_single_analysis_gets_no_analysis_flag_and_a_point_is_found() {
    let tmp = tempfile::tempdir().unwrap();
    let program = fake_viewer(tmp.path());
    let _env = hermetic_env_with(&[("CDEPT_UBERWAVE", Some(program.as_os_str()))]);
    let project_dir = tmp.path().join("proj");
    std::fs::create_dir_all(project_dir.join("libs")).unwrap();
    std::fs::write(project_dir.join("cdept.toml"), "").unwrap();
    let project = libcdept::Project::load(&project_dir).unwrap();
    let base = project_dir.join("simulations/cells/tb/testbench");
    for point in ["tt_27", "ss_27"] {
        std::fs::create_dir_all(base.join(point)).unwrap();
        std::fs::write(base.join(point).join("out.raw"), TRAN).unwrap();
    }

    assert_eq!(
        wave::pick_view(&project, "cells", "tb", None).unwrap(),
        "testbench"
    );
    // Several points and none named: the error lists them.
    let e = wave::pick_point(&project, "cells", "tb", "testbench", None)
        .unwrap_err()
        .to_string();
    assert!(
        e.contains("2 run points") && e.contains("ss_27, tt_27") && e.contains("--family"),
        "{e}"
    );
    let e = wave::pick_point(&project, "cells", "tb", "testbench", Some("nope"))
        .unwrap_err()
        .to_string();
    assert!(e.contains("ss_27, tt_27"), "{e}");

    let started = wave::open_point(&project, "cells", "tb", "testbench", "tt_27").unwrap();
    assert!(
        started.text().ends_with("tt_27/out.raw"),
        "{}",
        started.text()
    );
    let args = wait_for(&tmp.path().join("args.txt"));
    assert_eq!(args.lines().count(), 1, "{args}");
}

#[test]
fn a_missing_viewer_says_how_to_get_it() {
    let missing = OsStr::new("");
    let _env = hermetic_env_with(&[
        ("CDEPT_UBERWAVE", Some(missing)),
        ("PATH", Some(OsStr::new("/nonexistent"))),
    ]);
    let tmp = tempfile::tempdir().unwrap();
    let raw = tmp.path().join("out.raw");
    std::fs::write(&raw, TRAN).unwrap();
    // A sibling build next to the repository may exist on a developer's machine: only
    // the message is checked when there is none.
    if wave::locate(None).is_none() {
        let e = wave::open_raw(None, &raw, None).unwrap_err().to_string();
        assert!(
            e.contains("CDEPT_UBERWAVE") && e.contains("view-waveforms.md"),
            "{e}"
        );
    }
}

/// The real viewer starts on a raw file and stays up (it opens a window: needs a display).
#[cfg(unix)]
#[test]
fn the_real_viewer_starts_and_stays_alive() {
    // The guard is only held to look for the program: a hermetic environment has no
    // XDG_RUNTIME_DIR, which the viewer needs to reach the display.
    let program = {
        let _env = hermetic_env_with(&[]);
        wave::locate(None).filter(|p| p.is_file())
    };
    let Some(program) = program else {
        eprintln!("skipped: UberWave not found");
        return;
    };
    if std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none() {
        eprintln!("skipped: no display");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let raw = tmp.path().join("out.raw");
    std::fs::write(&raw, TRAN).unwrap();
    let mut child = std::process::Command::new(&program)
        .arg(&raw)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_secs(2));
    let status = child.try_wait().unwrap();
    let alive = status.is_none();
    let _ = child.kill();
    let _ = child.wait();
    assert!(alive, "UberWave exited within 2 seconds ({:?})", status);
}

// ---- A viewer with IPC (the fake of the GUI scripts, a Python program) --------------------

const FAKE_IPC: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../cdept-gui/scripts/fake-uberwave-ipc.py"
);

fn python_ok() -> bool {
    std::process::Command::new("python3")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

struct Bench {
    tmp: tempfile::TempDir,
    project: libcdept::Project,
    log: PathBuf,
}

fn bench(points: &[&str]) -> Bench {
    let tmp = tempfile::tempdir().unwrap();
    let project_dir = tmp.path().join("proj");
    std::fs::create_dir_all(project_dir.join("libs")).unwrap();
    std::fs::write(project_dir.join("cdept.toml"), "").unwrap();
    let project = libcdept::Project::load(&project_dir).unwrap();
    let base = project_dir.join("simulations/cells/tb/testbench");
    for point in points {
        std::fs::create_dir_all(base.join(point)).unwrap();
        std::fs::write(base.join(point).join("out.raw"), format!("{OP}{TRAN}")).unwrap();
    }
    let log = tmp.path().join("fake.log");
    Bench { tmp, project, log }
}

fn log_lines(log: &Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

fn wait_log(
    log: &Path,
    what: &str,
    pred: impl Fn(&[serde_json::Value]) -> bool,
) -> Vec<serde_json::Value> {
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        let lines = log_lines(log);
        if pred(&lines) {
            return lines;
        }
        assert!(
            Instant::now() < end,
            "the fake's log never had {what}: {lines:?}"
        );
        std::thread::sleep(Duration::from_millis(30));
    }
}

fn quit_viewer(project: &libcdept::Project) {
    let socket = wave::ipc::socket_path(&project.root, "cells/tb/testbench");
    if let Ok(mut c) = wave::ipc::Client::connect_once(&socket) {
        let _ = c.quit();
    }
}

fn v_a() -> wave::calc::Request {
    wave::calc::Request {
        expr: "v(\"a\")".into(),
        ..Default::default()
    }
}

#[cfg(unix)]
#[test]
fn a_family_opens_in_one_viewer_a_second_open_reuses_it_and_the_calculator_pushes_traces() {
    if !python_ok() {
        eprintln!("skipped: no python3");
        return;
    }
    let b = bench(&["tt_27", "ff_27", "ss_27"]);
    let run = b.tmp.path().join("run");
    std::fs::create_dir_all(&run).unwrap();
    let _env = hermetic_env_with(&[
        ("CDEPT_UBERWAVE", Some(OsStr::new(FAKE_IPC))),
        ("XDG_RUNTIME_DIR", Some(run.as_os_str())),
        ("FAKE_UBERWAVE_LOG", Some(b.log.as_os_str())),
        ("FAKE_UBERWAVE_IDLE", Some(OsStr::new("30"))),
    ]);

    let v = wave::session::version(Path::new(FAKE_IPC)).unwrap();
    assert!(v.ipc && v.line == "UberWaveViewer fake ipc 1", "{v:?}");
    assert_eq!(
        wave::describe(Path::new(FAKE_IPC)).as_deref(),
        Some("UberWaveViewer fake ipc 1")
    );

    // The family: one start with every point, tags = point names, the transient analysis.
    let all = wave::select_points(&b.project, "cells", "tb", "testbench", &[]).unwrap();
    assert_eq!(all.len(), 3);
    let opened = wave::open_family(&b.project, "cells", "tb", "testbench", &all, None).unwrap();
    assert!(
        opened.ipc
            && !opened.reused
            && opened.not_opened.is_empty()
            && opened.old_viewer_note().is_none()
    );
    let lines = wait_log(&b.log, "a start", |l| {
        l.iter().any(|r| r.get("start").is_some())
    });
    let start = &lines[0]["start"];
    assert_eq!(
        start["tags"],
        serde_json::json!(["ff_27", "ss_27", "tt_27"])
    );
    assert_eq!(start["files"].as_array().unwrap().len(), 3);
    assert_eq!(start["analysis"], "tran");
    assert!(
        start["files"][0]
            .as_str()
            .unwrap()
            .ends_with("ff_27/out.raw")
    );
    let socket = wave::ipc::socket_path(&b.project.root, "cells/tb/testbench");
    assert_eq!(start["argv"][0], "--ipc");
    assert_eq!(start["argv"][1], socket.to_str().unwrap());

    // A second open of the same run goes to the running viewer: open, then import.
    let again = wave::open_family(&b.project, "cells", "tb", "testbench", &all[..2], None).unwrap();
    assert!(again.reused && again.command.is_empty(), "{}", again.text());
    let lines = wait_log(&b.log, "open and import", |l| {
        l.iter().filter(|r| r.get("request").is_some()).count() >= 3
    });
    let cmds: Vec<&str> = lines
        .iter()
        .filter_map(|r| r["request"]["cmd"].as_str())
        .collect();
    assert_eq!(cmds, ["hello", "open", "import"]);
    assert_eq!(lines.iter().filter(|r| r.get("start").is_some()).count(), 1);
    let one = wave::open_point(&b.project, "cells", "tb", "testbench", "tt_27").unwrap();
    assert!(one.reused);

    // The calculator on one point: a trace named by the expression and the point.
    let tt = vec![all.iter().find(|p| p.point == "tt_27").unwrap().clone()];
    let sent = wave::calc::send(&b.project, "cells", "tb", "testbench", &tt, &v_a());
    assert_eq!(sent.problem, None, "{sent:?}");
    assert_eq!(sent.pushed, [("v(\"a\") @ tt_27".to_string(), 2)]);
    // Sending again replaces the trace (the fake refuses a second one of the same name).
    let sent = wave::calc::send(&b.project, "cells", "tb", "testbench", &tt, &v_a());
    assert_eq!(sent.problem, None, "{sent:?}");
    // The family: one trace per point.
    let named = wave::calc::Request {
        expr: "v(\"a\")".into(),
        name: Some("va".into()),
        plot: None,
    };
    let sent = wave::calc::send(&b.project, "cells", "tb", "testbench", &all, &named);
    assert_eq!(sent.problem, None, "{sent:?}");
    assert_eq!(
        sent.pushed
            .iter()
            .map(|(n, _)| n.as_str())
            .collect::<Vec<_>>(),
        ["va @ ff_27", "va @ ss_27", "va @ tt_27"]
    );
    // Scalars come back as a table and nothing is sent.
    let scalar = wave::calc::Request {
        expr: "value_at(v(\"a\"), 0)".into(),
        ..Default::default()
    };
    let sent = wave::calc::send(&b.project, "cells", "tb", "testbench", &all, &scalar);
    assert!(sent.pushed.is_empty() && sent.problem.is_none());
    assert_eq!(sent.table().lines().count(), 3, "{}", sent.table());
    assert!(sent.table().starts_with("ff_27\t"), "{}", sent.table());
    // A bad expression is an outcome, not a panic.
    let bad = wave::calc::Request {
        expr: "nofunc(1)".into(),
        ..Default::default()
    };
    let sent = wave::calc::send(&b.project, "cells", "tb", "testbench", &tt, &bad);
    assert!(
        sent.table().contains("error") && sent.pushed.is_empty(),
        "{}",
        sent.table()
    );

    let lines = log_lines(&b.log);
    let traces: Vec<&serde_json::Value> = lines
        .iter()
        .map(|r| &r["request"])
        .filter(|r| r["cmd"] == "add_trace")
        .collect();
    assert_eq!(traces.len(), 2 + 3, "{traces:?}");
    assert_eq!(traces[0]["name"], "v(\"a\") @ tt_27");
    assert_eq!(traces[0]["n"], 2);
    assert_eq!(traces[0]["x_unit"], "s");
    assert_eq!(
        lines.iter().filter(|r| r.get("start").is_some()).count(),
        1,
        "one viewer all along"
    );

    // The typed client against the running fake.
    let mut c = wave::ipc::Client::connect(&socket, Duration::from_secs(2)).unwrap();
    assert_eq!(c.hello().unwrap().protocol, 1);
    let files = c.list_files().unwrap();
    assert_eq!(
        files.iter().map(|f| f.tag.as_str()).collect::<Vec<_>>(),
        ["tt_27"]
    );
    assert_eq!(files[0].analyses, ["op", "tran"]);
    let (sigs, _) = c.list_signals(Some("tt_27"), Some("v(")).unwrap();
    assert!(
        sigs.iter().any(|s| s.name == "v(b)" && s.file == "tt_27")
            && sigs.iter().any(|s| s.file == "external"),
        "{sigs:?}"
    );
    let e = c
        .request("frobnicate", serde_json::Value::Null)
        .unwrap_err()
        .to_string();
    assert!(e.contains("unknown command"), "{e}");
    c.quit().unwrap();
    quit_viewer(&b.project);
}

#[cfg(unix)]
#[test]
fn a_viewer_that_refuses_is_reported_and_a_dead_socket_is_replaced() {
    if !python_ok() {
        eprintln!("skipped: no python3");
        return;
    }
    let b = bench(&["tt_27"]);
    let run = b.tmp.path().join("run");
    std::fs::create_dir_all(&run).unwrap();
    let _env = hermetic_env_with(&[
        ("CDEPT_UBERWAVE", Some(OsStr::new(FAKE_IPC))),
        ("XDG_RUNTIME_DIR", Some(run.as_os_str())),
        ("FAKE_UBERWAVE_LOG", Some(b.log.as_os_str())),
        ("FAKE_UBERWAVE_IDLE", Some(OsStr::new("30"))),
        ("FAKE_UBERWAVE_REFUSE", Some(OsStr::new("add_trace"))),
    ]);
    // A socket file nobody listens on (a viewer that died) does not stop a new start.
    let socket = wave::ipc::socket_path(&b.project.root, "cells/tb/testbench");
    drop(std::os::unix::net::UnixListener::bind(&socket).unwrap());
    assert!(socket.exists());
    let all = wave::select_points(&b.project, "cells", "tb", "testbench", &[]).unwrap();
    let sent = wave::calc::send(&b.project, "cells", "tb", "testbench", &all, &v_a());
    let problem = sent.problem.unwrap();
    assert!(problem.contains("refused 'add_trace'"), "{problem}");
    assert!(sent.pushed.is_empty());
    quit_viewer(&b.project);
}

#[cfg(unix)]
#[test]
fn an_older_viewer_opens_the_first_file_and_the_rest_are_listed() {
    let b = bench(&["tt_27", "ff_27", "ss_27"]);
    let program = fake_viewer(b.tmp.path());
    let _env = hermetic_env_with(&[("CDEPT_UBERWAVE", Some(program.as_os_str()))]);
    assert!(wave::session::version(&program).is_none());
    let all = wave::select_points(&b.project, "cells", "tb", "testbench", &[]).unwrap();
    let opened = wave::open_family(&b.project, "cells", "tb", "testbench", &all, None).unwrap();
    assert!(!opened.ipc && !opened.reused && opened.client.is_none());
    assert_eq!(
        opened
            .not_opened
            .iter()
            .map(|f| f.tag.as_str())
            .collect::<Vec<_>>(),
        ["ss_27", "tt_27"]
    );
    let note = opened.old_viewer_note().unwrap();
    assert!(
        note.contains("no IPC") && note.contains("ss_27") && note.contains("tt_27/out.raw"),
        "{note}"
    );
    let args = wait_for(&b.tmp.path().join("args.txt"));
    assert_eq!(args.lines().count(), 3, "{args}");
    assert!(args.contains("ff_27/out.raw"));

    // The calculator cannot push to it: the scalars still come, with the reason.
    let sent = wave::calc::send(&b.project, "cells", "tb", "testbench", &all[..1], &v_a());
    assert!(sent.pushed.is_empty());
    assert!(sent.problem.unwrap().contains("no IPC"));
    let scalar = wave::calc::Request {
        expr: "value_at(v(\"a\"), 0)".into(),
        ..Default::default()
    };
    let sent = wave::calc::send(&b.project, "cells", "tb", "testbench", &all, &scalar);
    assert_eq!(sent.table().lines().count(), 3);
    assert!(sent.problem.is_none());
}
