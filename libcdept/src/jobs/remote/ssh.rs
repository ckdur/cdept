//! `kind = "ssh"`: the simulator runs in an ssh session; the local `ssh` process is what
//! is watched (killing it hangs the session up), and a cancel also tries to kill the
//! remote process through the pid it wrote.

use std::fs::File;
use std::process::{Command, Stdio};
use std::time::Duration;

use eyre::{Result, WrapErr, bail, eyre};

use super::{Remote, exit_status, quote, spawn_retry};
use crate::batch::{Job, Output, Waited, Watch, tail};

/// The remote process id file, in the point folder (rsync mode: the remote copy).
const PID_FILE: &str = ".cdept-remote.pid";

pub(super) fn launch(
    r: &Remote,
    job: &Job,
    watch: &Watch,
    log: &dyn Fn(&str),
    rdir: &str,
) -> Result<Output> {
    r.push(&job.dir, rdir)?;
    // One script for the remote shell: environment, `cd`, then the simulator in place of the
    // shell (so the pid written first is the simulator's).
    let mut script = vec![format!(
        "mkdir -p {d} && echo $$ > {d}/{PID_FILE}",
        d = quote(rdir)
    )];
    script.extend(r.preamble(job, rdir));
    script.push(format!("exec {}", r.command_line(job)));
    let script = script.join("\n");

    let path = job.log_path();
    let out = File::create(&path).wrap_err_with(|| format!("cannot write {}", path.display()))?;
    let err = out.try_clone()?;
    let mut cmd = Command::new(&r.ssh);
    cmd.args(r.ssh_args())
        .arg(&r.host)
        .arg(&script)
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err);
    log(&format!("ssh {}: {}", r.host, r.command_line(job)));
    let mut child =
        spawn_retry(&mut cmd).wrap_err_with(|| format!("cannot start {} ({})", r.ssh, job.name))?;
    let text = || String::from_utf8_lossy(&std::fs::read(&path).unwrap_or_default()).into_owned();
    match watch.wait(&mut child)? {
        Waited::Exited(status) => {
            if status.code() == Some(255) {
                bail!(
                    "ssh to {} failed (exit status 255): the connection or the login did not work\n{}",
                    r.host,
                    tail(&text(), 6)
                );
            }
            Ok(Output {
                name: job.name.clone(),
                status: status.code().map_or_else(|| exit_status(1), exit_status),
                log: path.clone(),
                text: text(),
            })
        }
        Waited::Cancelled => {
            kill_remote(r, rdir, log);
            Err(eyre!("{} cancelled", job.name))
        }
        Waited::TimedOut(limit) => {
            kill_remote(r, rdir, log);
            Err(eyre!(
                "{} timed out after {} and was stopped\nlog: {}\n{}",
                job.name,
                crate::jobs::format_duration(limit),
                path.display(),
                tail(&text(), 12)
            ))
        }
    }
}

/// Best effort: kill the simulator by the pid it wrote. A short limit: the connection may be gone.
fn kill_remote(r: &Remote, rdir: &str, log: &dyn Fn(&str)) {
    let line = format!(
        "kill $(cat {d}/{PID_FILE}) 2>/dev/null; true",
        d = quote(rdir)
    );
    let mut cmd = Command::new(&r.ssh);
    cmd.args(r.ssh_args()).arg(&r.host).arg(line);
    match super::capture_command(&mut cmd, Duration::from_secs(10)) {
        Ok(c) if c.code == 0 => log("remote process killed"),
        Ok(c) => log(&format!("remote kill failed ({}): {}", c.code, c.text())),
        Err(e) => log(&format!("remote kill failed: {e:#}")),
    }
}
