//! `kind = "slurm"`: the simulator runs as a batch job. A `job.sbatch` is written in the point
//! folder, submitted with `sbatch --parsable`, and `squeue` (then `sacct` once the job has left
//! the queue, then the `exit_status` file the script writes) is polled until it ends.
//!
//! States: PENDING/CONFIGURING and RUNNING/COMPLETING are logged as they change. COMPLETED
//! returns the simulator's exit status (the `exit_status` file: the script ends 0 itself, so a
//! failing simulator is a COMPLETED job with a non-zero status); FAILED, CANCELLED, TIMEOUT,
//! NODE_FAIL, OUT_OF_MEMORY, ... are errors that carry SLURM's state and reason.

use std::path::Path;
use std::time::{Duration, Instant};

use eyre::{Result, WrapErr, bail, eyre};

use super::{Remote, exit_status, quote};
use crate::batch::{Job, Output, Watch, tail};

/// File the script writes the simulator's exit status to.
const EXIT_FILE: &str = "exit_status";
/// The batch script, in the point folder.
const SCRIPT: &str = "job.sbatch";
/// Polls in a row that learn nothing (squeue and sacct both failing or empty, no exit file)
/// before the job is given up.
const MAX_BLIND_POLLS: u32 = 6;
const HELPER_LIMIT: Duration = Duration::from_secs(60);

/// What one poll found out.
#[derive(Debug, PartialEq)]
enum Poll {
    /// Not over (state and reason as SLURM prints them).
    Active(String, String),
    /// Over: the state and the job's exit code when known.
    Final(String, Option<i32>),
    /// Nothing could be learnt (the message says why).
    Blind(String),
}

pub(super) fn launch(
    r: &Remote,
    job: &Job,
    watch: &Watch,
    log: &dyn Fn(&str),
    rdir: &str,
) -> Result<Output> {
    let exit_file = job.dir.join(EXIT_FILE);
    // A stale result must not pass for a new one.
    let _ = std::fs::remove_file(&exit_file);
    let _ = std::fs::remove_file(job.log_path());
    let script_file = job.dir.join(SCRIPT);
    std::fs::write(&script_file, script(r, job, rdir))
        .wrap_err_with(|| format!("cannot write {}", script_file.display()))?;
    r.push(&job.dir, rdir)?;
    if r.workdir_mode == super::WorkdirMode::Rsync {
        // The remote copy may hold the last run's exit file.
        let _ = r.run(
            &["rm".into(), "-f".into(), format!("{rdir}/{EXIT_FILE}")],
            None,
            HELPER_LIMIT,
        );
    }

    let sub = r.run(
        &[r.sbatch.clone(), "--parsable".into(), SCRIPT.into()],
        Some(rdir),
        HELPER_LIMIT,
    )?;
    if sub.code != 0 {
        bail!("sbatch failed ({}): {}", sub.code, sub.text());
    }
    let id = parse_job_id(&sub.stdout)
        .ok_or_else(|| eyre!("sbatch printed no job id: '{}'", sub.stdout.trim()))?;
    log(&format!("slurm job {id} submitted ({})", r.describe()));

    let start = Instant::now();
    let stop = |why: &str| {
        let c = r.run(&[r.scancel.clone(), id.clone()], None, HELPER_LIMIT);
        match c {
            Ok(c) if c.code == 0 => log(&format!("slurm job {id}: scancel ({why})")),
            Ok(c) => log(&format!("scancel {id} failed ({}): {}", c.code, c.text())),
            Err(e) => log(&format!("scancel {id} failed: {e:#}")),
        }
    };
    let (mut last, mut reason, mut blind) = (String::new(), String::new(), 0u32);
    let result = loop {
        if watch
            .cancel
            .as_ref()
            .is_some_and(|c| c.load(std::sync::atomic::Ordering::SeqCst))
        {
            stop("cancelled");
            bail!(
                "{} cancelled (slurm job {id} cancelled with scancel)",
                job.name
            );
        }
        if let Some(limit) = watch.timeout
            && start.elapsed() >= limit
        {
            stop("time limit");
            bail!(
                "{} timed out after {} and was stopped (slurm job {id} cancelled with scancel)\nlog: {}\n{}",
                job.name,
                crate::jobs::format_duration(limit),
                job.log_path().display(),
                tail(&read_log(&job.log_path()), 12)
            );
        }
        match query(r, &id, rdir, &exit_file) {
            Poll::Active(state, why) => {
                blind = 0;
                if !why.is_empty() && why != "None" {
                    reason = why.clone();
                }
                if state != last {
                    log(&format!(
                        "slurm job {id}: {state}{}",
                        if why.is_empty() || why == "None" {
                            String::new()
                        } else {
                            format!(" ({why})")
                        }
                    ));
                    last = state;
                }
            }
            Poll::Final(state, code) => break (state, code),
            Poll::Blind(why) => {
                blind += 1;
                if blind >= MAX_BLIND_POLLS {
                    bail!(
                        "slurm job {id} of {}: its state cannot be found ({why}); it may still be running",
                        job.name
                    );
                }
            }
        }
        sleep_watching(r.poll, watch);
    };
    let (state, sacct_code) = result;
    log(&format!("slurm job {id}: {state}"));

    // The files of the job (rsync mode: copied back first) and the script's own verdict.
    r.pull(&job.dir, rdir, log);
    let text = read_log(&job.log_path());
    if state != "COMPLETED" {
        let why = if reason.is_empty() {
            String::new()
        } else {
            format!(", reason {reason}")
        };
        let code = sacct_code.filter(|c| *c != 0);
        bail!(
            "slurm job {id} of {} ended {state}{}{why}\nlog: {}\n{}",
            job.name,
            code.map(|c| format!(" (exit code {c})"))
                .unwrap_or_default(),
            job.log_path().display(),
            tail(&text, 12)
        );
    }
    // COMPLETED: the simulator's status is in the file the script wrote (a network
    // filesystem may be late with it).
    let code = if r.wait_for_file(&exit_file) {
        read_code(&exit_file)
    } else {
        log(&format!(
            "no {EXIT_FILE} after {}: taking {}",
            crate::jobs::format_duration(r.file_wait),
            sacct_code.unwrap_or(0)
        ));
        None
    }
    .or(sacct_code)
    .unwrap_or(0);
    Ok(Output {
        name: job.name.clone(),
        status: exit_status(code),
        log: job.log_path(),
        text,
    })
}

/// The batch script.
fn script(r: &Remote, job: &Job, rdir: &str) -> String {
    let mut s = String::from("#!/bin/bash\n# written by CDePT (jobs::remote)\n");
    let mut sbatch = |line: String| s.push_str(&format!("#SBATCH {line}\n"));
    let name: String = job
        .name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    sbatch(format!("--job-name=cdept-{name}"));
    sbatch(format!("--output={rdir}/slurm.out"));
    if !r.partition.is_empty() {
        sbatch(format!("--partition={}", r.partition));
    }
    if !r.account.is_empty() {
        sbatch(format!("--account={}", r.account));
    }
    if !r.time.is_empty() {
        sbatch(format!("--time={}", r.time));
    }
    if let Some(n) = r.cpus {
        sbatch(format!("--cpus-per-task={n}"));
    }
    if !r.mem.is_empty() {
        sbatch(format!("--mem={}", r.mem));
    }
    for o in &r.sbatch_options {
        sbatch(o.clone());
    }
    for line in r.preamble(job, rdir) {
        s.push_str(&line);
        s.push('\n');
    }
    s.push_str(&format!("rm -f {EXIT_FILE}\n"));
    // The simulator's output goes to the job log, the status to the exit file.
    s.push_str(&format!(
        "{} > {} 2>&1\n",
        r.command_line(job),
        quote(&job.log.to_string_lossy())
    ));
    s.push_str(&format!("echo $? > {EXIT_FILE}\n"));
    s
}

/// The job id of `sbatch --parsable` output: `123` or `123;cluster`.
fn parse_job_id(out: &str) -> Option<String> {
    let id = out.trim().lines().next()?.split(';').next()?.trim();
    (!id.is_empty() && id.chars().all(|c| c.is_ascii_digit() || c == '_')).then(|| id.to_string())
}

/// The first word of a state (`CANCELLED by 1000` is CANCELLED), without the `+` of a
/// requeued or cancelled job.
fn base_state(s: &str) -> String {
    s.split_whitespace()
        .next()
        .unwrap_or("")
        .trim_end_matches('+')
        .to_ascii_uppercase()
}

fn is_final(state: &str) -> bool {
    matches!(
        state,
        "COMPLETED"
            | "FAILED"
            | "CANCELLED"
            | "TIMEOUT"
            | "NODE_FAIL"
            | "OUT_OF_MEMORY"
            | "BOOT_FAIL"
            | "DEADLINE"
            | "PREEMPTED"
            | "REVOKED"
    )
}

/// The exit code of sacct's `ExitCode` (`2:0`).
fn sacct_code(field: &str) -> Option<i32> {
    field.split(':').next()?.trim().parse().ok()
}

fn read_code(file: &Path) -> Option<i32> {
    std::fs::read_to_string(file).ok()?.trim().parse().ok()
}

fn read_log(file: &Path) -> String {
    String::from_utf8_lossy(&std::fs::read(file).unwrap_or_default()).into_owned()
}

/// Ask SLURM: squeue while the job is in the queue, sacct after, the exit file at last.
fn query(r: &Remote, id: &str, rdir: &str, exit_file: &Path) -> Poll {
    let squeue = r.run(
        &[
            r.squeue.clone(),
            "-j".into(),
            id.into(),
            "-h".into(),
            "-o".into(),
            "%T|%r".into(),
        ],
        Some(rdir),
        HELPER_LIMIT,
    );
    let mut why = match &squeue {
        Ok(c) if c.code == 0 => String::new(),
        Ok(c) => format!("squeue failed ({}): {}", c.code, c.text()),
        Err(e) => format!("squeue: {e:#}"),
    };
    if let Ok(c) = &squeue
        && c.code == 0
        && let Some(line) = c.stdout.lines().find(|l| !l.trim().is_empty())
    {
        let (state, reason) = line.split_once('|').unwrap_or((line, ""));
        let state = base_state(state);
        // The queue still knows the job: a final state here (a job on its way out) is believed.
        return if is_final(&state) {
            Poll::Final(state, None)
        } else {
            Poll::Active(state, reason.trim().to_string())
        };
    }
    // Not in the queue (or the queue could not be asked): the accounting.
    let sacct = r.run(
        &[
            r.sacct.clone(),
            "-j".into(),
            id.into(),
            "-n".into(),
            "-P".into(),
            "-o".into(),
            "State,ExitCode".into(),
        ],
        Some(rdir),
        HELPER_LIMIT,
    );
    match &sacct {
        Ok(c) if c.code == 0 => {
            if let Some(line) = c.stdout.lines().find(|l| !l.trim().is_empty()) {
                let (state, code) = line.split_once('|').unwrap_or((line, ""));
                let state = base_state(state);
                return if is_final(&state) {
                    Poll::Final(state, sacct_code(code))
                } else {
                    Poll::Active(state, String::new())
                };
            }
            if why.is_empty() {
                why = "the job is not in squeue and sacct knows nothing of it".into();
            }
        }
        Ok(c) => why = format!("{why}; sacct failed ({}): {}", c.code, c.text()),
        Err(e) => why = format!("{why}; sacct: {e:#}"),
    }
    // No accounting: the script's own exit file says that it ran to its end.
    if r.workdir_mode == super::WorkdirMode::Shared
        && let Some(code) = read_code(exit_file)
    {
        return Poll::Final("COMPLETED".into(), Some(code));
    }
    Poll::Blind(why)
}

/// Sleep for `total`, in short steps so that a cancel is answered soon.
fn sleep_watching(total: Duration, watch: &Watch) {
    let start = Instant::now();
    while start.elapsed() < total {
        if watch
            .cancel
            .as_ref()
            .is_some_and(|c| c.load(std::sync::atomic::Ordering::SeqCst))
        {
            return;
        }
        std::thread::sleep(Duration::from_millis(10).min(total));
    }
}

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn ids_and_states() {
        assert_eq!(parse_job_id("4242\n").as_deref(), Some("4242"));
        assert_eq!(parse_job_id("17;cluster2\n").as_deref(), Some("17"));
        assert_eq!(parse_job_id("sbatch: error\n"), None);
        assert_eq!(parse_job_id(""), None);
        assert_eq!(base_state("CANCELLED by 1000"), "CANCELLED");
        assert_eq!(base_state("CANCELLED+"), "CANCELLED");
        assert!(is_final("TIMEOUT") && !is_final("RUNNING") && !is_final("PENDING"));
        assert_eq!(sacct_code("2:0"), Some(2));
        assert_eq!(sacct_code(""), None);
    }
}
