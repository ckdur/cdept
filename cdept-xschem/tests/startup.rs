//! Starting the engine with a drawing window in awkward conditions, through `cdx-tcl`.

use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

/// `cdx-tcl --display 300x200` with these further arguments, in a scratch home.
/// Panics if it has not ended after `limit`.
fn cdx_tcl(home: &Path, arguments: &[&str], limit: Duration) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_cdx-tcl"))
        .args(["--display", "300x200"])
        .args(arguments)
        .env("HOME", home)
        .env_remove("CDEPT_XSCHEM_TK")
        .current_dir(home)
        .stdin(Stdio::null())
        // The widget updates on stdout (menus, icons) would fill a pipe nobody reads.
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("cannot start cdx-tcl");
    let deadline = Instant::now() + limit;
    while child.try_wait().unwrap().is_none() {
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("cdx-tcl {arguments:?} did not end within {limit:?}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    child.wait_with_output().unwrap()
}

/// xschem reports an error in the start-up file with a message box. Nobody is
/// connected yet who could press OK: the start must fail with the message, not wait.
#[test]
fn an_error_in_the_startup_file_fails_the_start_instead_of_waiting() {
    let home = tempfile::tempdir().unwrap();
    let rcfile = home.path().join("rc.tcl");
    std::fs::write(&rcfile, "error {this start-up file is broken}\n").unwrap();
    let output = cdx_tcl(
        home.path(),
        &["--rcfile", rcfile.to_str().unwrap(), "-e", "expr {1 + 1}"],
        Duration::from_secs(20),
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "the start should fail:\n{stderr}");
    assert!(
        stderr.contains("this start-up file is broken"),
        "the reason is missing:\n{stderr}"
    );
}

/// xschem names its temporary folders after a random number seeded with the time in
/// seconds. Several engines starting in the same second (CDePT restoring its tabs)
/// must still each get their folders: one without loses undo.
#[test]
fn engines_starting_together_all_get_their_temporary_folders() {
    let example = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../examples/buffer/libs/logic/inv/schematic/inv.sch");
    let load = format!(
        "xschem load {{{}}}; xschem get schname",
        example.canonicalize().unwrap().display()
    );
    let threads: Vec<_> = (0..12)
        .map(|_| {
            let load = load.clone();
            std::thread::spawn(move || {
                let home = tempfile::tempdir().unwrap();
                cdx_tcl(home.path(), &["-e", &load], Duration::from_secs(60))
            })
        })
        .collect();
    for thread in threads {
        let output = thread.join().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{stderr}");
        assert!(
            !stderr.contains("create_tmpdir") && !stderr.contains("Undo will be disabled"),
            "an engine did not get its temporary folder:\n{stderr}"
        );
    }
}
