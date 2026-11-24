//! Tests of the remote backends against fake `ssh`, `rsync`, `sbatch`, `squeue`, `sacct` and
//! `scancel` programs (shell scripts in a temp folder).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::*;
use crate::batch::testutil::fake;

fn remote(toml_text: &str) -> Remote {
    Remote::parse("t", toml::from_str(toml_text).unwrap()).unwrap()
}

/// A fake SLURM in `dir`: `states` are what `squeue` prints on its 1st, 2nd, ... call (`ERR`: it
/// fails); after them the job is gone from the queue and `sacct` prints `sacct` (when given).
/// `scancel` leaves a `cancelled` marker, after which the job is CANCELLED in `sacct`.
/// `submit` is run by `sbatch` in the job folder before it prints the id.
fn fake_slurm(dir: &Path, states: &[&str], sacct: Option<&str>, submit: &str) -> Remote {
    let d = dir.display();
    std::fs::write(dir.join("states"), states.join("\n") + "\n").unwrap();
    if let Some(s) = sacct {
        std::fs::write(dir.join("sacct_out"), format!("{s}\n")).unwrap();
    }
    fake(
        dir,
        "sbatch",
        &format!("echo \"$@\" >> {d}/sbatch.args\n{submit}\necho 4242"),
    );
    fake(
        dir,
        "squeue",
        &format!(
            "n=$(cat {d}/cnt 2>/dev/null || echo 0); n=$((n+1)); echo $n > {d}/cnt\n\
             [ -f {d}/cancelled ] && exit 0\n\
             line=$(sed -n \"${{n}}p\" {d}/states)\n\
             [ -z \"$line\" ] && exit 0\n\
             if [ \"$line\" = ERR ]; then echo 'slurm_load_jobs error: Socket timed out' >&2; exit 1; fi\n\
             echo \"$line\""
        ),
    );
    fake(
        dir,
        "sacct",
        &format!(
            "if [ -f {d}/cancelled ]; then echo 'CANCELLED by 1000|0:15'; exit 0; fi\n\
             n=$(cat {d}/cnt 2>/dev/null || echo 0)\n\
             [ -f {d}/sacct_out ] && [ \"$n\" -gt \"$(wc -l < {d}/states)\" ] && cat {d}/sacct_out && exit 0\n\
             echo 'sacct: accounting storage is disabled' >&2; exit 1"
        ),
    );
    fake(
        dir,
        "scancel",
        &format!("echo \"$@\" >> {d}/scancel.args\ntouch {d}/cancelled"),
    );
    remote(&format!(
        "kind = \"slurm\"\npoll = \"20ms\"\nfile_wait = \"300ms\"\npartition = \"short\"\n\
         time = \"00:10:00\"\ncpus = 2\nmem = \"2G\"\nmodules = [\"ngspice/45\"]\n\
         command_prefix = \"source /etc/site\"\nenv = {{ OMP_NUM_THREADS = \"1\" }}\n\
         sbatch = \"{d}/sbatch\"\nsqueue = \"{d}/squeue\"\nsacct = \"{d}/sacct\"\nscancel = \"{d}/scancel\"\n"
    ))
}

fn job(dir: &Path) -> Job {
    let mut j = Job::new("p0", Path::new("ngspice"), &dir.join("point"), "sim.log")
        .args(["-b", "-o", "out.log", "deck.sp"]);
    j.env = vec![("PDK_ROOT".into(), "/pdks".into())];
    j
}

type Lines = Mutex<Vec<String>>;

fn collect(lines: &Lines) -> impl Fn(&str) + '_ {
    move |l| lines.lock().unwrap().push(l.to_string())
}

fn run(r: &Remote, dir: &Path, watch: &Watch) -> (Result<Output>, Vec<String>) {
    let lines = Lines::default();
    let out = r.launch(&job(dir), watch, &collect(&lines));
    (out, lines.into_inner().unwrap())
}

fn err_text(r: Result<Output>) -> String {
    format!("{:#}", r.expect_err("an error"))
}

#[test]
fn reads_a_remote_table() {
    let r = remote("kind = \"ssh\"\nhost = \"box\"");
    assert_eq!((r.kind, r.max_jobs, r.poll), (Kind::Ssh, 8, DEFAULT_POLL));
    assert_eq!(r.workdir_mode, WorkdirMode::Shared);
    assert_eq!(r.program_for("ngspice"), "ngspice");
    assert_eq!(r.program_for("xyce"), "Xyce");
    assert_eq!(r.program_for("vacask"), "vacask");
    let r = remote(
        "kind = \"slurm\"\nprogram = \"/opt/sim\"\nprograms = { xyce = \"/opt/Xyce\" }\npoll = 0.5\nmax_jobs = 3",
    );
    assert_eq!(r.program_for("ngspice"), "/opt/sim");
    assert_eq!(r.program_for("xyce"), "/opt/Xyce");
    assert_eq!((r.max_jobs, r.poll), (3, Duration::from_millis(500)));

    let bad = |t: &str| {
        format!(
            "{:#}",
            Remote::parse("t", toml::from_str(t).unwrap()).unwrap_err()
        )
    };
    assert!(bad("host = \"x\"").contains("kind is missing"));
    assert!(bad("kind = \"ssh\"").contains("needs a host"));
    assert!(bad("kind = \"ssh\"\nhost=\"h\"\nworkdir_mode = \"rsync\"").contains("remote_dir"));
    assert!(bad("kind = \"slurm\"\npartiton = \"x\"").contains("partiton"));
    assert!(bad("kind = \"lsf\"").contains("lsf"));
}

#[test]
fn settings_warn_and_pick() {
    let root: toml::Table = toml::from_str(
        "[jobs]\ndefault = \"a\"\nbogus = 1\n[jobs.remote.a]\nkind = \"ssh\"\nhost = \"h\"\n\
         [jobs.remote.b]\nkind = \"slurm\"\n[jobs.remote.c]\nkind = \"ssh\"\n",
    )
    .unwrap();
    let mut warnings = vec![];
    let s = JobsSettings::load(&root, &mut warnings);
    assert_eq!(s.remotes.keys().collect::<Vec<_>>(), ["a", "b"]);
    assert!(warnings.iter().any(|w| w.contains("bogus")), "{warnings:?}");
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("jobs.remote.c") && w.contains("host")),
        "{warnings:?}"
    );
    assert_eq!(s.pick(None, "").unwrap().unwrap().name, "a");
    assert_eq!(s.pick(None, "b").unwrap().unwrap().name, "b");
    assert_eq!(s.pick(Some("a"), "b").unwrap().unwrap().name, "a");
    let e = format!("{}", s.pick(Some("zzz"), "").unwrap_err());
    assert!(e.contains("there are: a, b"), "{e}");
    let none = JobsSettings::default();
    assert!(none.pick(None, "").unwrap().is_none());
    assert!(format!("{}", none.pick(None, "x").unwrap_err()).contains("no [jobs.remote"));
}

#[test]
fn quotes_for_the_shell() {
    assert_eq!(quote("deck.sp"), "deck.sp");
    assert_eq!(quote("a b"), "'a b'");
    assert_eq!(quote("it's"), "'it'\\''s'");
    assert_eq!(quote(""), "''");
}

#[test]
fn writes_the_batch_script() {
    let tmp = tempfile::tempdir().unwrap();
    let r = fake_slurm(
        tmp.path(),
        &[],
        Some("COMPLETED|0:0"),
        "echo 0 > exit_status",
    );
    let (out, _) = run(&r, tmp.path(), &Watch::default());
    out.unwrap();
    let s = std::fs::read_to_string(tmp.path().join("point/job.sbatch")).unwrap();
    for want in [
        "#!/bin/bash",
        "#SBATCH --job-name=cdept-p0",
        "#SBATCH --partition=short",
        "#SBATCH --time=00:10:00",
        "#SBATCH --cpus-per-task=2",
        "#SBATCH --mem=2G",
        "source /etc/site",
        "module load ngspice/45",
        "export PDK_ROOT=/pdks",
        "export OMP_NUM_THREADS=1",
        "ngspice -b -o out.log deck.sp > sim.log 2>&1",
        "echo $? > exit_status",
    ] {
        assert!(s.contains(want), "'{want}' is not in\n{s}");
    }
    let cd = s.find("cd ").unwrap();
    assert!(cd < s.find("ngspice -b").unwrap());
    let args = std::fs::read_to_string(tmp.path().join("sbatch.args")).unwrap();
    assert_eq!(args.trim(), "--parsable job.sbatch");
}

#[test]
fn a_slurm_job_runs_through_its_states_to_completed() {
    let tmp = tempfile::tempdir().unwrap();
    let r = fake_slurm(
        tmp.path(),
        &[
            "PENDING|Priority",
            "PENDING|Priority",
            "RUNNING|None",
            "RUNNING|None",
        ],
        Some("COMPLETED|0:0"),
        "echo 0 > exit_status; echo 'sim output' > sim.log",
    );
    let (out, lines) = run(&r, tmp.path(), &Watch::default());
    let out = out.unwrap();
    assert!(out.status.success());
    assert!(out.text.contains("sim output"), "{}", out.text);
    let all = lines.join("\n");
    for want in ["submitted", "PENDING (Priority)", "RUNNING", "COMPLETED"] {
        assert!(all.contains(want), "'{want}' is not in\n{all}");
    }
    // Each state is logged once, not once per poll.
    assert_eq!(all.matches("PENDING").count(), 1, "{all}");
}

#[test]
fn the_simulators_own_exit_status_comes_through() {
    let tmp = tempfile::tempdir().unwrap();
    let r = fake_slurm(
        tmp.path(),
        &["RUNNING|None"],
        Some("COMPLETED|0:0"),
        "echo 3 > exit_status",
    );
    let (out, _) = run(&r, tmp.path(), &Watch::default());
    assert_eq!(out.unwrap().status.code(), Some(3));
}

#[test]
fn a_failed_job_says_so_with_its_exit_code() {
    let tmp = tempfile::tempdir().unwrap();
    let r = fake_slurm(
        tmp.path(),
        &["RUNNING|None"],
        Some("FAILED|2:0"),
        "echo 'cd failed' > sim.log",
    );
    let msg = err_text(run(&r, tmp.path(), &Watch::default()).0);
    assert!(
        msg.contains("FAILED") && msg.contains("exit code 2"),
        "{msg}"
    );
    assert!(msg.contains("cd failed"), "the job log is in it: {msg}");
}

#[test]
fn slurm_states_that_are_not_completed_are_errors_with_the_reason() {
    for (acct, want) in [
        ("TIMEOUT|0:0", "TIMEOUT"),
        ("NODE_FAIL|0:0", "NODE_FAIL"),
        ("OUT_OF_MEMORY|0:125", "OUT_OF_MEMORY"),
        ("CANCELLED by 1000|0:0", "CANCELLED"),
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let r = fake_slurm(tmp.path(), &["RUNNING|TimeLimit"], Some(acct), "");
        let msg = err_text(run(&r, tmp.path(), &Watch::default()).0);
        assert!(
            msg.contains(want) && msg.contains("reason TimeLimit"),
            "{msg}"
        );
    }
}

#[test]
fn cancelling_runs_scancel() {
    let tmp = tempfile::tempdir().unwrap();
    let states = vec!["RUNNING|None"; 500];
    let r = fake_slurm(tmp.path(), &states, Some("COMPLETED|0:0"), "");
    let flag = Arc::new(AtomicBool::new(false));
    let f = flag.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        f.store(true, Ordering::SeqCst);
    });
    let t = Instant::now();
    let (out, lines) = run(&r, tmp.path(), &Watch::cancel(flag));
    let msg = err_text(out);
    assert!(msg.contains("cancelled"), "{msg}");
    assert!(t.elapsed() < Duration::from_secs(10));
    let args = std::fs::read_to_string(tmp.path().join("scancel.args")).unwrap();
    assert_eq!(args.trim(), "4242");
    assert!(lines.iter().any(|l| l.contains("scancel")), "{lines:?}");
}

#[test]
fn a_time_limit_runs_scancel() {
    let tmp = tempfile::tempdir().unwrap();
    let states = vec!["RUNNING|None"; 500];
    let r = fake_slurm(tmp.path(), &states, None, "echo 'still going' > sim.log");
    let watch = Watch {
        cancel: None,
        timeout: Some(Duration::from_millis(300)),
    };
    let msg = err_text(run(&r, tmp.path(), &watch).0);
    assert!(
        msg.contains("timed out after 300ms") && msg.contains("still going"),
        "{msg}"
    );
    assert!(tmp.path().join("scancel.args").is_file());
}

#[test]
fn a_squeue_that_fails_now_and_then_is_asked_again() {
    let tmp = tempfile::tempdir().unwrap();
    let r = fake_slurm(
        tmp.path(),
        &["ERR", "RUNNING|None", "ERR", "ERR", "RUNNING|None"],
        Some("COMPLETED|0:0"),
        "",
    );
    let (out, _) = run(&r, tmp.path(), &Watch::default());
    assert!(out.unwrap().status.success());
}

#[test]
fn a_squeue_that_never_answers_ends_the_wait() {
    let tmp = tempfile::tempdir().unwrap();
    let states = vec!["ERR"; 50];
    let r = fake_slurm(tmp.path(), &states, None, "");
    let msg = err_text(run(&r, tmp.path(), &Watch::default()).0);
    assert!(
        msg.contains("state cannot be found") && msg.contains("Socket timed out"),
        "{msg}"
    );
}

#[test]
fn sacct_answers_when_the_job_left_the_queue() {
    let tmp = tempfile::tempdir().unwrap();
    // The queue is empty at once; only the accounting knows (and says the exit code).
    let r = fake_slurm(
        tmp.path(),
        &[],
        Some("COMPLETED|0:0\nCOMPLETED|0:0"),
        "echo 1 > exit_status",
    );
    let (out, _) = run(&r, tmp.path(), &Watch::default());
    assert_eq!(out.unwrap().status.code(), Some(1));
}

#[test]
fn without_accounting_the_exit_file_tells() {
    let tmp = tempfile::tempdir().unwrap();
    let r = fake_slurm(tmp.path(), &["RUNNING|None"], None, "echo 0 > exit_status");
    let (out, _) = run(&r, tmp.path(), &Watch::default());
    assert!(out.unwrap().status.success());
}

#[test]
fn sbatch_failing_is_an_error() {
    let tmp = tempfile::tempdir().unwrap();
    let r = fake_slurm(
        tmp.path(),
        &[],
        None,
        "echo 'sbatch: error: invalid partition' >&2; exit 1",
    );
    let msg = err_text(run(&r, tmp.path(), &Watch::default()).0);
    assert!(
        msg.contains("sbatch failed") && msg.contains("invalid partition"),
        "{msg}"
    );
    assert!(
        !tmp.path().join("cnt").exists(),
        "no polling after a failed submit"
    );
}

#[test]
fn checks_slurm_and_ssh() {
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path().display();
    let mut r = fake_slurm(tmp.path(), &[], None, "");
    fake(
        tmp.path(),
        "sbatch-test",
        "echo \"Job 1 to start at 2030-01-01 using 1 CPUs ($*)\"",
    );
    r.sbatch = format!("{d}/sbatch-test");
    let msg = r.check().unwrap();
    assert!(
        msg.contains("--test-only") && msg.contains("--partition=short"),
        "{msg}"
    );
    fake(
        tmp.path(),
        "sbatch-bad",
        "echo 'sbatch: error: Batch job submission failed' >&2; exit 1",
    );
    r.sbatch = format!("{d}/sbatch-bad");
    assert!(format!("{}", r.check().unwrap_err()).contains("submission failed"));

    let s = fake_ssh(tmp.path(), "");
    assert!(s.check().unwrap().contains("works"));
    let bad = remote(&format!(
        "kind = \"ssh\"\nhost = \"nowhere\"\nssh = \"{}\"",
        fake(
            tmp.path(),
            "ssh-down",
            "echo 'ssh: connect to host nowhere: refused' >&2; exit 255"
        )
        .display()
    ));
    assert!(format!("{}", bad.check().unwrap_err()).contains("refused"));
}

// ---- ssh ----

/// A fake `ssh` that skips its options and the host and runs the command here, as the
/// remote shell would. Calls are logged to `ssh.calls`.
fn fake_ssh(dir: &Path, extra: &str) -> Remote {
    let d = dir.display();
    fake(
        dir,
        "ssh",
        &format!(
            "while [ \"$1\" = -o ] || [ \"$1\" = -p ] || [ \"$1\" = -i ]; do shift 2; done\n\
             host=$1; shift\necho \"$host\" >> {d}/ssh.calls\nexec sh -c \"$*\""
        ),
    );
    remote(&format!(
        "kind = \"ssh\"\nhost = \"cluster1\"\nssh = \"{d}/ssh\"\n{extra}"
    ))
}

fn fake_sim(dir: &Path, body: &str) -> PathBuf {
    fake(dir, "sim", body)
}

#[test]
fn ssh_runs_the_command_with_its_environment_and_returns_the_status() {
    let tmp = tempfile::tempdir().unwrap();
    let r = fake_ssh(
        tmp.path(),
        "command_prefix = [\"PREFIX_SET=yes\", \"export PREFIX_SET\"]\nenv = { REMOTE_ONLY = \"a b\" }",
    );
    let sim = fake_sim(
        tmp.path(),
        "echo \"hello $PDK_ROOT $PREFIX_SET [$REMOTE_ONLY] $* in $(pwd)\"; echo data > produced.txt; exit 3",
    );
    let mut j = job(tmp.path());
    j.program = sim;
    let lines = Lines::default();
    let out = r.launch(&j, &Watch::default(), &collect(&lines)).unwrap();
    assert_eq!(out.status.code(), Some(3), "the simulator's own status");
    let want = format!(
        "hello /pdks yes [a b] -b -o out.log deck.sp in {}",
        tmp.path().join("point").display()
    );
    assert!(out.text.contains(&want), "{}\nwant {want}", out.text);
    assert_eq!(std::fs::read_to_string(&out.log).unwrap(), out.text);
    assert!(tmp.path().join("point/produced.txt").is_file());
    let calls = std::fs::read_to_string(tmp.path().join("ssh.calls")).unwrap();
    assert_eq!(calls.trim(), "cluster1");
}

#[test]
fn ssh_that_cannot_connect_is_an_error() {
    let tmp = tempfile::tempdir().unwrap();
    let mut r = fake_ssh(tmp.path(), "");
    r.ssh = fake(
        tmp.path(),
        "ssh-down",
        "echo 'Permission denied (publickey)' >&2; exit 255",
    )
    .to_string_lossy()
    .into_owned();
    let msg = err_text(run(&r, tmp.path(), &Watch::default()).0);
    assert!(
        msg.contains("ssh to cluster1 failed") && msg.contains("Permission denied"),
        "{msg}"
    );
}

#[test]
fn cancelling_an_ssh_run_kills_it_here_and_there() {
    let tmp = tempfile::tempdir().unwrap();
    let r = fake_ssh(tmp.path(), "");
    let sim = fake_sim(tmp.path(), "echo started; exec sleep 30");
    let mut j = job(tmp.path());
    j.program = sim;
    let flag = Arc::new(AtomicBool::new(false));
    let f = flag.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(400));
        f.store(true, Ordering::SeqCst);
    });
    let lines = Lines::default();
    let t = Instant::now();
    let out = r.launch(&j, &Watch::cancel(flag), &collect(&lines));
    assert!(err_text(out).contains("cancelled"));
    assert!(t.elapsed() < Duration::from_secs(10));
    let lines = lines.into_inner().unwrap();
    assert!(
        lines.iter().any(|l| l.contains("remote process killed")),
        "{lines:?}"
    );
    // The pid it wrote is of a process that is gone.
    let pid = std::fs::read_to_string(tmp.path().join("point/.cdept-remote.pid")).unwrap();
    assert!(!Path::new(&format!("/proc/{}", pid.trim())).exists());
}

#[test]
fn an_ssh_run_has_a_time_limit() {
    let tmp = tempfile::tempdir().unwrap();
    let r = fake_ssh(tmp.path(), "");
    let mut j = job(tmp.path());
    j.program = fake_sim(tmp.path(), "echo working; exec sleep 30");
    let watch = Watch {
        cancel: None,
        timeout: Some(Duration::from_millis(400)),
    };
    let msg = err_text(r.launch(&j, &watch, &|_| {}));
    assert!(
        msg.contains("timed out after 400ms") && msg.contains("working"),
        "{msg}"
    );
}

#[test]
fn rsync_mode_copies_the_folder_there_and_back() {
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path().display();
    // The remote's folder is a local one, `host:` is dropped by the fake rsync.
    fake(
        tmp.path(),
        "rsync",
        "while [ \"$1\" = -a ] || [ \"$1\" = -e ]; do if [ \"$1\" = -e ]; then shift 2; else shift; fi; done\n\
         from=${1#*:}; to=${2#*:}\n\
         case \"$1\" in *:*) from=${1#*:};; esac\n\
         mkdir -p \"$to\" && cp -a \"$from\"/. \"$to\"/",
    );
    let r = fake_ssh(
        tmp.path(),
        &format!("workdir_mode = \"rsync\"\nremote_dir = \"{d}/remote\"\nrsync = \"{d}/rsync\""),
    );
    let sim = fake_sim(
        tmp.path(),
        "cat deck.sp > out.copy; echo remote-wrote > out.log; echo sim-out",
    );
    let mut j = job(tmp.path());
    j.program = sim;
    std::fs::create_dir_all(&j.dir).unwrap();
    std::fs::write(j.dir.join("deck.sp"), "the deck\n").unwrap();
    let rd = r.remote_dir_of(&j.dir);
    assert!(
        rd.starts_with(&format!("{d}/remote/")) && rd.ends_with("/point"),
        "{rd}"
    );
    let out = r.launch(&j, &Watch::default(), &|_| {}).unwrap();
    assert!(out.status.success(), "{}", out.text);
    // The remote folder got the deck; the local one got what the simulator wrote.
    assert_eq!(
        std::fs::read_to_string(j.dir.join("out.copy")).unwrap(),
        "the deck\n"
    );
    assert_eq!(
        std::fs::read_to_string(j.dir.join("out.log"))
            .unwrap()
            .trim(),
        "remote-wrote"
    );
    assert!(out.text.contains("sim-out"));
}
