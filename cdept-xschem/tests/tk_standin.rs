//! Runs the Tcl tests of the Tk stand-in and of CDePT's adjustments to xschem
//! (`shim/tcl/tests/t_*.tcl`) in `cdx-tcl`, as `shim/tcl/tests/run.sh` does, without
//! and with the drawing window. Every widget update the Tcl side sends during those
//! tests must also read as a `UiUpdate`: that is the check that the hand-written JSON
//! matches the Rust types.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use libcdept::xschem::protocol::UiUpdate;

fn tests_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("shim/tcl/tests")
}

/// Run one test script and check its output. Returns the number of checks passed.
fn run(script: &str, display: bool) -> usize {
    let dir = tests_dir();
    // xschem writes into ~/.xschem and some commands into the current folder.
    let scratch = tempfile::tempdir().unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_cdx-tcl"));
    if display {
        command.args(["--display", "900x600"]);
    }
    let stand_in = dir.parent().unwrap();
    command
        .arg("-e")
        .arg(format!("set ::faketk_dir {{{}}}", stand_in.display()))
        .arg(dir.join("boot.tcl"))
        .arg(dir.join("lib.tcl"))
        .arg(dir.join(script))
        .env("HOME", scratch.path())
        .env_remove("CDEPT_XSCHEM_TK")
        .env_remove("CDEPT_FAKETK_DEBUG")
        .current_dir(scratch.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("cannot start cdx-tcl");

    // Read both pipes on threads so that a full pipe never blocks the child.
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let read = |mut pipe: Box<dyn std::io::Read + Send>| {
        std::thread::spawn(move || {
            let mut text = String::new();
            let _ = pipe.read_to_string(&mut text);
            text
        })
    };
    let (stdout, stderr) = (read(Box::new(stdout)), read(Box::new(stderr)));
    let deadline = Instant::now() + Duration::from_secs(120);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("{script}: no end after two minutes");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let stdout = stdout.join().unwrap();
    let stderr = stderr.join().unwrap();

    let mut passed = 0;
    let mut problems = Vec::new();
    for line in stdout.lines() {
        if let Some(json) = line.strip_prefix("ui: ") {
            if let Err(error) = serde_json::from_str::<UiUpdate>(json) {
                let start: String = json.chars().take(300).collect();
                problems.push(format!("not a UiUpdate ({error}): {start}"));
            }
        } else if line.starts_with("PASS") {
            passed += 1;
        } else if line.starts_with("FAIL") {
            problems.push(line.to_string());
        }
    }
    problems.extend(
        stderr
            .lines()
            .filter(|line| line.starts_with("error:"))
            .map(str::to_string),
    );
    if !status.success() {
        problems.push(format!("cdx-tcl ended with {status}"));
    }
    assert!(
        problems.is_empty(),
        "{script} (display: {display}):\n{}\n--- stderr ---\n{stderr}",
        problems.join("\n")
    );
    assert!(passed > 0, "{script}: no check ran");
    passed
}

macro_rules! tcl_tests {
    ($($name:ident: $script:literal, $display:literal, at least $checks:literal;)*) => {
        $(
            #[test]
            fn $name() {
                let passed = run($script, $display);
                assert!(passed >= $checks, "only {passed} checks ran");
            }
        )*
    };
}

tcl_tests! {
    widgets: "t_widgets.tcl", false, at least 90;
    widgets_with_display: "t_widgets.tcl", true, at least 90;
    reports: "t_ui.tcl", false, at least 50;
    reports_with_display: "t_ui.tcl", true, at least 50;
    dialogs: "t_dialogs.tcl", false, at least 60;
    dialogs_with_display: "t_dialogs.tcl", true, at least 60;
    flows_with_display: "t_flow.tcl", true, at least 30;
    sweep: "t_sweep.tcl", false, at least 30;
    sweep_with_display: "t_sweep.tcl", true, at least 150;
    cdept_adjustments_with_display: "t_cdept.tcl", true, at least 140;
    simulation: "t_sim.tcl", false, at least 34;
    simulation_with_display: "t_sim.tcl", true, at least 65;
    panes_and_clipboard: "t_preview.tcl", false, at least 9;
    panes_and_clipboard_with_display: "t_preview.tcl", true, at least 22;
}

/// Every test script is listed above.
#[test]
fn no_script_is_left_out() {
    let source = std::fs::read_to_string(file!().replace("cdept-xschem/", "")).unwrap();
    for entry in std::fs::read_dir(tests_dir()).unwrap() {
        let name = entry.unwrap().file_name().to_string_lossy().into_owned();
        if name.starts_with("t_") && name.ends_with(".tcl") {
            assert!(source.contains(&name), "{name} is not run by this test");
        }
    }
}
