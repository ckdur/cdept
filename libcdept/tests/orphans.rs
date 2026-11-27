//! A program a job waits for ends when CDePT does, also when CDePT is killed and cannot
//! clean up. The test binary starts itself as the stand-in for CDePT, kills it with
//! SIGKILL and looks for the job's program.

#![cfg(target_os = "linux")]

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use libcdept::batch::Job;
use libcdept::jobs::{JobSpec, Queue};

const ROLE: &str = "CDEPT_ORPHAN_ROLE";
const PIDFILE: &str = "CDEPT_ORPHAN_PIDFILE";

/// Writes its pid, then becomes `sleep`: what a long simulation looks like.
fn long_program(pidfile: &str) -> [String; 2] {
    ["-c".into(), format!("echo $$ > '{pidfile}'; exec sleep 60")]
}

/// Not a test by itself: the stand-in for CDePT, when started with `ROLE` set.
#[test]
fn stand_in() {
    let Ok(role) = std::env::var(ROLE) else {
        return;
    };
    let pidfile = std::env::var(PIDFILE).unwrap();
    let args = long_program(&pidfile);
    match role.as_str() {
        "queue" => {
            let queue = Queue::new(1);
            let id = queue.submit(JobSpec::command("long", "/bin/sh", args));
            queue.wait(id);
        }
        "batch" => {
            let dir = Path::new(&pidfile).parent().unwrap();
            let _ = Job::new("long", Path::new("/bin/sh"), dir, "long.log")
                .args(args)
                .run();
        }
        other => panic!("unknown role {other}"),
    }
}

/// Running (a zombie, waiting for whoever adopted it, has ended).
fn alive(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|s| s.rsplit_once(") ").map(|(_, rest)| !rest.starts_with('Z')))
        .unwrap_or(false)
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn killed_parent_takes_the_program_along(role: &str) {
    let dir = tempfile::tempdir().unwrap();
    let pidfile = dir.path().join("pid");
    let mut parent = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "stand_in", "--test-threads=1"])
        .env(ROLE, role)
        .env(PIDFILE, &pidfile)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut pid = 0;
    wait_until("the job's program", || {
        pid = std::fs::read_to_string(&pidfile)
            .ok()
            .and_then(|t| t.trim().parse().ok())
            .unwrap_or(0);
        pid != 0
    });
    assert!(alive(pid));
    parent.kill().unwrap(); // SIGKILL: no clean-up of its own
    parent.wait().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while alive(pid) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let survived = alive(pid);
    if survived {
        let _ = Command::new("kill").args(["-9", &pid.to_string()]).status();
    }
    assert!(!survived, "{role}: the program outlived its killed parent");
}

#[test]
fn a_queue_command_does_not_outlive_cdept() {
    killed_parent_takes_the_program_along("queue");
}

#[test]
fn a_batch_program_does_not_outlive_cdept() {
    killed_parent_takes_the_program_along("batch");
}
