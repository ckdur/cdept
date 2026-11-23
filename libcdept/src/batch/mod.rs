//! Batch adapters: external tools run to completion as separate processes (Xyce,
//! netgen, Yosys, OpenROAD), each with a working folder, a log file and the project's
//! tool environment. Only the process boundary is shared with the GPL tools; nothing
//! is linked. Design and options: docs/BATCH.md.
//!
//! An [`Adapter`] says how to find a program; a [`Job`] is one run of it; the modules
//! below build jobs and read what the tools print.

pub mod netgen;
pub mod openroad;
pub mod xyce;
pub mod yosys;

use std::ffi::OsString;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};

use eyre::{Result, WrapErr, bail};

/// How a tool is found.
#[derive(Debug, Clone, Copy)]
pub struct Adapter {
    /// For messages.
    pub name: &'static str,
    /// Environment variable that names the program, first choice.
    pub env_var: &'static str,
    /// Name of the entry in [`crate::tools::KNOWN_TOOLS`]: its binary names are tried
    /// on `PATH` after the variable.
    pub tool: &'static str,
}

impl Adapter {
    /// The program: `$env_var`, else the first binary of the tool's entry in
    /// [`crate::tools::KNOWN_TOOLS`] found on `PATH`.
    pub fn locate(&self) -> Option<PathBuf> {
        self.locate_with(|name| std::env::var_os(name))
    }

    fn locate_with(&self, var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
        if let Some(p) = var(self.env_var).filter(|p| !p.is_empty()) {
            return Some(PathBuf::from(p));
        }
        let spec = crate::tools::KNOWN_TOOLS.iter().find(|s| s.name == self.tool)?;
        match spec.locate {
            crate::tools::Locate::Bin(names) => names.iter().find_map(|b| crate::tools::find_in_path(b)),
            _ => None,
        }
    }

    /// [`Self::locate`], or an error that says how to point CDePT at the program.
    pub fn require(&self) -> Result<PathBuf> {
        self.locate().ok_or_else(|| {
            eyre::eyre!("{} not found: put it on PATH or set {}", self.name, self.env_var)
        })
    }
}

/// One run of a program.
#[derive(Debug, Clone)]
pub struct Job {
    /// For messages.
    pub name: String,
    pub program: PathBuf,
    pub args: Vec<OsString>,
    /// The working folder (created when missing).
    pub dir: PathBuf,
    /// On top of the process environment; see [`crate::Project::tool_env`].
    pub env: Vec<(OsString, OsString)>,
    /// Defaults that apply only when neither the process nor `env` sets the variable.
    pub env_defaults: Vec<(OsString, OsString)>,
    /// Where stdout and stderr go. Relative: inside `dir`.
    pub log: PathBuf,
}

impl Job {
    pub fn new(name: &str, program: &Path, dir: &Path, log: &str) -> Job {
        Job {
            name: name.to_string(),
            program: program.to_path_buf(),
            args: vec![],
            dir: dir.to_path_buf(),
            env: vec![],
            env_defaults: vec![],
            log: PathBuf::from(log),
        }
    }

    pub fn args<I, S>(mut self, args: I) -> Job
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    pub fn env(mut self, env: &[(OsString, OsString)]) -> Job {
        self.env.extend(env.iter().cloned());
        self
    }

    pub fn log_path(&self) -> PathBuf {
        self.dir.join(&self.log)
    }

    /// Run it and wait. The exit status is in the result whatever it is; an error is
    /// only for a program that cannot be started or a log that cannot be made.
    pub fn run_unchecked(&self) -> Result<Output> {
        std::fs::create_dir_all(&self.dir)
            .wrap_err_with(|| format!("cannot create {}", self.dir.display()))?;
        let log = self.log_path();
        let out = File::create(&log).wrap_err_with(|| format!("cannot write {}", log.display()))?;
        let err = out.try_clone()?;
        let mut cmd = Command::new(&self.program);
        cmd.current_dir(&self.dir)
            .args(&self.args)
            .stdin(Stdio::null())
            .stdout(out)
            .stderr(err);
        for (k, v) in &self.env_defaults {
            let set = self.env.iter().any(|(name, _)| name == k) || std::env::var_os(k).is_some();
            if !set {
                cmd.env(k, v);
            }
        }
        cmd.envs(self.env.iter().map(|(k, v)| (k, v)));
        tracing::debug!(job = %self.name, program = %self.program.display(), args = ?self.args, "batch run");
        // A program that was written a moment ago may still be open for writing in a
        // process forked meanwhile (ETXTBSY, os error 26): try again shortly.
        let mut tries = 0;
        let status = loop {
            match cmd.status() {
                Err(e) if e.raw_os_error() == Some(26) && tries < 20 => {
                    tries += 1;
                    std::thread::sleep(std::time::Duration::from_millis(25));
                }
                other => break other,
            }
        }
        .wrap_err_with(|| format!("cannot start {} ({})", self.name, self.program.display()))?;
        let text = String::from_utf8_lossy(&std::fs::read(&log).unwrap_or_default()).into_owned();
        Ok(Output { name: self.name.clone(), status, log, text })
    }

    /// Run it; an error when it exits with a failure status. The end of the log is in
    /// the message.
    pub fn run(&self) -> Result<Output> {
        let out = self.run_unchecked()?;
        out.check()?;
        Ok(out)
    }
}

/// What a run printed and how it ended.
#[derive(Debug, Clone)]
pub struct Output {
    pub name: String,
    pub status: ExitStatus,
    /// The log file (stdout and stderr, in order).
    pub log: PathBuf,
    /// Its text.
    pub text: String,
}

impl Output {
    /// An error with the status and the tail of the log when the program failed.
    pub fn check(&self) -> Result<()> {
        if !self.status.success() {
            bail!("{} failed ({})\nlog: {}\n{}", self.name, self.status, self.log.display(), tail(&self.text, 12));
        }
        Ok(())
    }
}

/// The last `n` non-empty lines of `text`.
pub fn tail(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

#[cfg(test)]
pub(crate) mod testutil {
    use std::path::{Path, PathBuf};

    /// A shell script that stands in for a program.
    #[cfg(unix)]
    pub fn fake(dir: &Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locates_by_variable_first() {
        let a = Adapter { name: "X", env_var: "CDEPT_X", tool: "no such tool" };
        assert_eq!(a.locate_with(|_| Some("/opt/x".into())), Some(PathBuf::from("/opt/x")));
        assert_eq!(a.locate_with(|_| Some("".into())), None);
        assert_eq!(a.locate_with(|_| None), None);
        assert!(a.require().is_err() || std::env::var_os("CDEPT_X").is_some());
    }

    #[test]
    fn falls_back_to_the_tool_table() {
        let a = Adapter { name: "Yosys", env_var: "CDEPT_YOSYS", tool: "Yosys" };
        // Whatever PATH holds, the lookup goes through the table without panicking.
        let _ = a.locate_with(|_| None);
    }

    #[test]
    fn tail_keeps_the_end() {
        assert_eq!(tail("a\n\nb\nc\n", 2), "b\nc");
        assert_eq!(tail("", 3), "");
    }

    #[cfg(unix)]
    #[test]
    fn runs_logs_and_reports_failure() {
        let dir = tempfile::tempdir().unwrap();
        let ok = testutil::fake(dir.path(), "ok", "echo \"hello $1 $CDEPT_T $CDEPT_D\"; echo warn >&2");
        let mut job = Job::new("fake", &ok, &dir.path().join("w"), "run.log").args(["arg"]);
        job.env = vec![("CDEPT_T".into(), "e".into())];
        job.env_defaults = vec![("CDEPT_T".into(), "ignored".into()), ("CDEPT_D".into(), "d".into())];
        let out = job.run().unwrap();
        assert!(out.text.contains("hello arg e d") && out.text.contains("warn"), "{}", out.text);
        assert!(dir.path().join("w/run.log").is_file());

        let bad = testutil::fake(dir.path(), "bad", "echo one; echo boom; exit 3");
        let err = Job::new("fake", &bad, dir.path(), "bad.log").run().unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("boom") && msg.contains("exit status: 3"), "{msg}");
        assert!(Job::new("none", &dir.path().join("missing"), dir.path(), "m.log").run().is_err());
    }
}
