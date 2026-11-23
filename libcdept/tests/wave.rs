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
    std::fs::write(&program, "#!/bin/sh\nd=$(dirname \"$0\")\nprintf '%s\\n' \"$@\" > \"$d/args.txt\"\npwd > \"$d/cwd.txt\"\n").unwrap();
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
    assert_eq!(found.iter().map(|a| (a.kind.as_str(), a.signals)).collect::<Vec<_>>(), [("op", 2), ("tran", 3)]);

    let started = wave::open_raw(None, &raw, None).unwrap();
    assert!(started.text().ends_with("out.raw --analysis tran"), "{}", started.text());
    let args: Vec<String> = wait_for(&tmp.path().join("args.txt")).lines().map(String::from).collect();
    assert_eq!(args, [raw.to_str().unwrap(), "--analysis", "tran"]);
    // The working folder is the run point's.
    let cwd = wait_for(&tmp.path().join("cwd.txt"));
    assert_eq!(Path::new(cwd.trim()).canonicalize().unwrap(), run.canonicalize().unwrap());

    // An explicit analysis, by its kind; one the file does not have is an error.
    std::fs::remove_file(tmp.path().join("args.txt")).unwrap();
    wave::open_raw(None, &raw, Some("op")).unwrap();
    assert_eq!(wait_for(&tmp.path().join("args.txt")).lines().last(), Some("op"));
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

    assert_eq!(wave::pick_view(&project, "cells", "tb", None).unwrap(), "testbench");
    // Several points and none named: the error lists them.
    let e = wave::pick_point(&project, "cells", "tb", "testbench", None).unwrap_err().to_string();
    assert!(e.contains("2 run points") && e.contains("ss_27, tt_27") && e.contains("one file at a time"), "{e}");
    let e = wave::pick_point(&project, "cells", "tb", "testbench", Some("nope")).unwrap_err().to_string();
    assert!(e.contains("ss_27, tt_27"), "{e}");

    let started = wave::open_point(&project, "cells", "tb", "testbench", "tt_27").unwrap();
    assert!(started.text().ends_with("tt_27/out.raw"), "{}", started.text());
    let args = wait_for(&tmp.path().join("args.txt"));
    assert_eq!(args.lines().count(), 1, "{args}");
}

#[test]
fn a_missing_viewer_says_how_to_get_it() {
    let missing = OsStr::new("");
    let _env = hermetic_env_with(&[("CDEPT_UBERWAVE", Some(missing)), ("PATH", Some(OsStr::new("/nonexistent")))]);
    let tmp = tempfile::tempdir().unwrap();
    let raw = tmp.path().join("out.raw");
    std::fs::write(&raw, TRAN).unwrap();
    // A sibling build next to the repository may exist on a developer's machine: only
    // the message is checked when there is none.
    if wave::locate(None).is_none() {
        let e = wave::open_raw(None, &raw, None).unwrap_err().to_string();
        assert!(e.contains("CDEPT_UBERWAVE") && e.contains("view-waveforms.md"), "{e}");
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
