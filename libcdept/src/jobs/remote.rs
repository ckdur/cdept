//! Remote execution of run points (SYMBI-82): the simulator of a run point is started on
//! another machine through `ssh`, or as a SLURM batch job, instead of as a local process.
//! SMOKE-TESTED ONLY: against fake `ssh`/`rsync`/`sbatch`/`squeue`/`sacct`/`scancel`
//! programs, never against a cluster. Design and limits: docs/JOBS.md, docs/howto/remote-runs.md.
//!
//! The run point is prepared locally as always (deck, `.spiceinit` in its folder). A
//! [`Remote`] then [`launch`](Remote::launch)es the simulator's [`Job`] (program,
//! arguments, folder, environment) and returns the same [`Output`] a local run gives, so
//! the files the simulator wrote (out.raw, out.log, ...) are read back from the point folder
//! exactly as for a local run. The folder is visible at the same path on the remote
//! (`workdir_mode = "shared"`, the default) or copied with rsync (`"rsync"`).
//!
//! The remotes are named in `cdept.toml` ([`JobsSettings`]):
//!
//! ```toml
//! [jobs]
//! default = "cluster"            # optional: used when a testbench names none
//!
//! [jobs.remote.cluster]
//! kind = "slurm"                 # or "ssh"
//! host = "login1"                # ssh host; for slurm the login node (empty: sbatch is local)
//! partition = "short"
//! time = "00:30:00"
//! cpus = 2
//! mem = "4G"
//! modules = ["ngspice/45"]
//! max_jobs = 16
//! ```

mod slurm;
mod ssh;
#[cfg(all(test, unix))]
mod tests;

use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use eyre::{Result, WrapErr, bail, eyre};
use serde::Deserialize;

use crate::batch::{Job, Output, Watch};

/// How a remote starts the simulator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// `ssh host '<command>'`, one ssh session per run point.
    Ssh,
    /// `sbatch` a batch script and poll `squeue`/`sacct` until it ends.
    Slurm,
}

/// Where the point folder is on the remote.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkdirMode {
    /// The project folder is visible at the same path on the remote (a shared filesystem).
    #[default]
    Shared,
    /// The point folder is copied to `remote_dir` before the run and back after it.
    Rsync,
}

/// A named remote: `[jobs.remote.NAME]`.
#[derive(Debug, Clone, PartialEq)]
pub struct Remote {
    pub name: String,
    pub kind: Kind,
    /// The ssh host (`user@host` works). Slurm: the login node, empty when `sbatch` is local.
    pub host: String,
    /// Extra `ssh` arguments (`["-p", "2222", "-i", "key"]`); `-o BatchMode=yes` always comes first.
    pub ssh_options: Vec<String>,
    /// Shell lines run on the remote before the simulator (`source /etc/profile`).
    pub command_prefix: Vec<String>,
    /// `module load` of these, after the prefix.
    pub modules: Vec<String>,
    /// Environment of the simulator on the remote, on top of the project's tool environment.
    pub env: BTreeMap<String, String>,
    pub partition: String,
    pub account: String,
    /// SLURM `--time` (`"00:30:00"`), passed as written.
    pub time: String,
    pub cpus: Option<u32>,
    pub mem: String,
    /// More `#SBATCH` arguments, one per entry (`"--gres=gpu:1"`).
    pub sbatch_options: Vec<String>,
    pub workdir_mode: WorkdirMode,
    /// rsync mode: the folder on the remote the point folders are copied under.
    pub remote_dir: String,
    /// Run points started at once on this remote (a local run uses the cores).
    pub max_jobs: usize,
    /// The simulator on the remote when it is not the bare name; `programs` wins per simulator.
    pub program: String,
    pub programs: BTreeMap<String, String>,
    /// How often SLURM is asked for the job state.
    pub poll: Duration,
    /// How long files the remote wrote are waited for (a lagging network filesystem).
    pub file_wait: Duration,
    /// Program names (a path, for a stand-in or a site's wrapper).
    pub ssh: String,
    pub rsync: String,
    pub sbatch: String,
    pub squeue: String,
    pub sacct: String,
    pub scancel: String,
}

/// The `[jobs]` table of `cdept.toml`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct JobsSettings {
    /// The remote a testbench uses when it names none (`default`).
    pub default: Option<String>,
    pub remotes: BTreeMap<String, Remote>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Lines {
    One(String),
    Many(Vec<String>),
}

impl Lines {
    fn into_vec(self) -> Vec<String> {
        match self {
            Lines::One(s) => s.lines().map(str::to_string).collect(),
            Lines::Many(v) => v,
        }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Seconds {
    Number(f64),
    Text(String),
}

impl Seconds {
    fn duration(&self) -> Result<Duration> {
        match self {
            Seconds::Number(n) if *n >= 0.0 && n.is_finite() => Ok(Duration::from_secs_f64(*n)),
            Seconds::Number(_) => Err(eyre!("a duration cannot be negative")),
            Seconds::Text(t) => super::parse_duration(t),
        }
    }
}

/// `[jobs.remote.NAME]` as written.
#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct Raw {
    kind: Option<Kind>,
    host: String,
    ssh_options: Vec<String>,
    command_prefix: Option<Lines>,
    modules: Vec<String>,
    env: BTreeMap<String, String>,
    partition: String,
    account: String,
    time: String,
    cpus: Option<u32>,
    mem: String,
    sbatch_options: Vec<String>,
    workdir_mode: WorkdirMode,
    remote_dir: String,
    max_jobs: Option<usize>,
    program: String,
    programs: BTreeMap<String, String>,
    poll: Option<Seconds>,
    file_wait: Option<Seconds>,
    ssh: Option<String>,
    rsync: Option<String>,
    sbatch: Option<String>,
    squeue: Option<String>,
    sacct: Option<String>,
    scancel: Option<String>,
}

/// Run points at once on a remote that does not say.
pub const DEFAULT_MAX_JOBS: usize = 8;
/// How often SLURM is polled when the remote does not say.
pub const DEFAULT_POLL: Duration = Duration::from_secs(5);
/// How long output files are waited for when the remote does not say.
pub const DEFAULT_FILE_WAIT: Duration = Duration::from_secs(10);

impl Remote {
    /// Read `[jobs.remote.<name>]`. The error names the problem (unknown key, missing `host`).
    pub fn parse(name: &str, value: toml::Value) -> Result<Remote> {
        let raw: Raw = value.try_into().map_err(|e| eyre!("{}", e.message()))?;
        let kind = raw
            .kind
            .ok_or_else(|| eyre!("kind is missing (\"slurm\" or \"ssh\")"))?;
        if kind == Kind::Ssh && raw.host.is_empty() {
            bail!("an ssh remote needs a host");
        }
        if raw.workdir_mode == WorkdirMode::Rsync {
            if raw.host.is_empty() {
                bail!("workdir_mode = \"rsync\" needs a host");
            }
            if raw.remote_dir.is_empty() {
                bail!("workdir_mode = \"rsync\" needs a remote_dir");
            }
        }
        let duration = |what: &str, v: &Option<Seconds>, default: Duration| match v {
            Some(s) => s.duration().wrap_err_with(|| what.to_string()),
            None => Ok(default),
        };
        Ok(Remote {
            name: name.to_string(),
            kind,
            host: raw.host,
            ssh_options: raw.ssh_options,
            command_prefix: raw.command_prefix.map(Lines::into_vec).unwrap_or_default(),
            modules: raw.modules,
            env: raw.env,
            partition: raw.partition,
            account: raw.account,
            time: raw.time,
            cpus: raw.cpus,
            mem: raw.mem,
            sbatch_options: raw.sbatch_options,
            workdir_mode: raw.workdir_mode,
            remote_dir: raw.remote_dir,
            max_jobs: raw.max_jobs.filter(|n| *n > 0).unwrap_or(DEFAULT_MAX_JOBS),
            program: raw.program,
            programs: raw.programs,
            poll: duration("poll", &raw.poll, DEFAULT_POLL)?,
            file_wait: duration("file_wait", &raw.file_wait, DEFAULT_FILE_WAIT)?,
            ssh: raw.ssh.unwrap_or_else(|| "ssh".into()),
            rsync: raw.rsync.unwrap_or_else(|| "rsync".into()),
            sbatch: raw.sbatch.unwrap_or_else(|| "sbatch".into()),
            squeue: raw.squeue.unwrap_or_else(|| "squeue".into()),
            sacct: raw.sacct.unwrap_or_else(|| "sacct".into()),
            scancel: raw.scancel.unwrap_or_else(|| "scancel".into()),
        })
    }

    /// The simulator program on the remote: the remote's own, never the local path. `sim` is
    /// `ngspice`, `xyce` or `vacask`; the default is the bare name the remote's `PATH` knows.
    pub fn program_for(&self, sim: &str) -> String {
        if let Some(p) = self.programs.get(sim).filter(|p| !p.is_empty()) {
            return p.clone();
        }
        if !self.program.is_empty() {
            return self.program.clone();
        }
        match sim {
            "xyce" => "Xyce".into(),
            other => other.into(),
        }
    }

    /// One line for lists: `ssh to host, shared folder, max 8 jobs`.
    pub fn describe(&self) -> String {
        let mut s = match self.kind {
            Kind::Ssh => format!("ssh to {}", self.host),
            Kind::Slurm if self.host.is_empty() => "slurm (local sbatch)".to_string(),
            Kind::Slurm => format!("slurm via {}", self.host),
        };
        if !self.partition.is_empty() {
            s.push_str(&format!(", partition {}", self.partition));
        }
        if self.workdir_mode == WorkdirMode::Rsync {
            s.push_str(&format!(", rsync to {}", self.remote_dir));
        } else {
            s.push_str(", shared folder");
        }
        s.push_str(&format!(", max {} jobs", self.max_jobs));
        s
    }

    /// Start `job` (a simulator, its arguments, folder and environment) on the remote and wait
    /// for it. `watch` stops it early (the cancel flag: the local ssh is killed and a remote
    /// kill is tried; SLURM: `scancel`); `log` gets progress lines for the job log. The result
    /// is the simulator's own exit status and output (`job.log`), as for a local run; an error
    /// when the remote could not be reached, the job did not run to its end, or was stopped.
    pub fn launch(&self, job: &Job, watch: &Watch, log: &dyn Fn(&str)) -> Result<Output> {
        std::fs::create_dir_all(&job.dir)
            .wrap_err_with(|| format!("cannot create {}", job.dir.display()))?;
        let rdir = self.remote_dir_of(&job.dir);
        let out = match self.kind {
            Kind::Ssh => ssh::launch(self, job, watch, log, &rdir),
            Kind::Slurm => slurm::launch(self, job, watch, log, &rdir),
        };
        // Whatever the remote wrote comes back, also after a failure (the logs).
        self.pull(&job.dir, &rdir, log);
        out
    }

    /// Does the remote answer? `ssh host true`, or `sbatch --test-only` for SLURM (a one-line
    /// summary, else the error).
    pub fn check(&self) -> Result<String> {
        match self.kind {
            Kind::Ssh => {
                let r = self.run(&["true".into()], None, Duration::from_secs(30))?;
                if r.code != 0 {
                    bail!("ssh {} failed ({}): {}", self.host, r.code, r.text());
                }
                Ok(format!("ssh {} works", self.host))
            }
            Kind::Slurm => {
                let mut argv = vec![self.sbatch.clone(), "--test-only".into()];
                if !self.partition.is_empty() {
                    argv.push(format!("--partition={}", self.partition));
                }
                if !self.time.is_empty() {
                    argv.push(format!("--time={}", self.time));
                }
                argv.push("--wrap=true".into());
                let r = self.run(&argv, None, Duration::from_secs(60))?;
                if r.code != 0 {
                    bail!("sbatch --test-only failed ({}): {}", r.code, r.text());
                }
                Ok(format!("sbatch accepts a job ({})", r.text()))
            }
        }
    }

    // ---- shared by the kinds ----

    /// The point folder as the remote sees it.
    pub(crate) fn remote_dir_of(&self, dir: &Path) -> String {
        let abs = std::path::absolute(dir).unwrap_or_else(|_| dir.to_path_buf());
        match self.workdir_mode {
            WorkdirMode::Shared => abs.to_string_lossy().into_owned(),
            WorkdirMode::Rsync => {
                // lib/cell/view/point: the last four names keep testbenches apart.
                let names: Vec<String> = abs
                    .components()
                    .filter_map(|c| match c {
                        std::path::Component::Normal(n) => Some(n.to_string_lossy().into_owned()),
                        _ => None,
                    })
                    .collect();
                let tail = &names[names.len().saturating_sub(4)..];
                format!(
                    "{}/{}",
                    self.remote_dir.trim_end_matches('/'),
                    tail.join("/")
                )
            }
        }
    }

    /// The arguments every ssh call of this remote gets.
    pub(crate) fn ssh_args(&self) -> Vec<String> {
        let mut v: Vec<String> = vec!["-o".into(), "BatchMode=yes".into()];
        v.extend(self.ssh_options.iter().cloned());
        v
    }

    /// Shell lines that set up the environment and then `cd` to the folder: the prefix, the
    /// modules, the variables (the project's tool environment, the remote's own on top; a
    /// default only when the remote does not set the variable).
    pub(crate) fn preamble(&self, job: &Job, rdir: &str) -> Vec<String> {
        let mut lines = self.command_prefix.clone();
        lines.extend(self.modules.iter().map(|m| format!("module load {m}")));
        for (k, v) in &job.env_defaults {
            let k = k.to_string_lossy();
            lines.push(format!(
                "[ -n \"${{{k}+x}}\" ] || {k}={}; export {k}",
                quote(&v.to_string_lossy())
            ));
        }
        for (k, v) in &job.env {
            let (k, v) = (k.to_string_lossy(), v.to_string_lossy());
            if self.env.contains_key(k.as_ref()) {
                continue;
            }
            lines.push(format!("export {k}={}", quote(&v)));
        }
        for (k, v) in &self.env {
            lines.push(format!("export {k}={}", quote(v)));
        }
        lines.push(format!("cd {}", quote(rdir)));
        lines
    }

    /// The simulator's command line, quoted.
    pub(crate) fn command_line(&self, job: &Job) -> String {
        std::iter::once(job.program.to_string_lossy().into_owned())
            .chain(job.args.iter().map(|a| a.to_string_lossy().into_owned()))
            .map(|a| quote(&a))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// The command that runs `argv` on the remote side: through ssh when there is a host,
    /// else here (in `cwd`).
    pub(crate) fn command(&self, argv: &[String], cwd: Option<&str>) -> Command {
        if self.host.is_empty() {
            let mut c = Command::new(&argv[0]);
            c.args(&argv[1..]);
            if let Some(d) = cwd {
                c.current_dir(d);
            }
            c
        } else {
            let mut line = argv.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" ");
            if let Some(d) = cwd {
                line = format!("cd {} && {line}", quote(d));
            }
            let mut c = Command::new(&self.ssh);
            c.args(self.ssh_args()).arg(&self.host).arg(line);
            c
        }
    }

    /// [`Remote::command`], run to its end (at most `limit`).
    pub(crate) fn run(
        &self,
        argv: &[String],
        cwd: Option<&str>,
        limit: Duration,
    ) -> Result<Captured> {
        capture_command(&mut self.command(argv, cwd), limit)
    }

    /// rsync mode: copy the point folder to the remote (and make the folder there).
    pub(crate) fn push(&self, dir: &Path, rdir: &str) -> Result<()> {
        if self.workdir_mode != WorkdirMode::Rsync {
            return Ok(());
        }
        let mk = self.run(
            &["mkdir".into(), "-p".into(), rdir.into()],
            None,
            Duration::from_secs(60),
        )?;
        if mk.code != 0 {
            bail!("cannot make {rdir} on {}: {}", self.host, mk.text());
        }
        let r = self.rsync_run(
            &format!("{}/", dir.display()),
            &format!("{}:{rdir}/", self.host),
        )?;
        if r.code != 0 {
            bail!("rsync to {} failed ({}): {}", self.host, r.code, r.text());
        }
        Ok(())
    }

    /// rsync mode: copy the remote folder back over the local one. Best effort.
    pub(crate) fn pull(&self, dir: &Path, rdir: &str, log: &dyn Fn(&str)) {
        if self.workdir_mode != WorkdirMode::Rsync {
            return;
        }
        let r = self.rsync_run(
            &format!("{}:{rdir}/", self.host),
            &format!("{}/", dir.display()),
        );
        match r {
            Ok(r) if r.code == 0 => {}
            Ok(r) => log(&format!(
                "rsync from {} failed ({}): {}",
                self.host,
                r.code,
                r.text()
            )),
            Err(e) => log(&format!("rsync from {} failed: {e:#}", self.host)),
        }
    }

    fn rsync_run(&self, from: &str, to: &str) -> Result<Captured> {
        let ssh = std::iter::once(self.ssh.clone())
            .chain(self.ssh_args())
            .map(|a| quote(&a))
            .collect::<Vec<_>>()
            .join(" ");
        let mut cmd = Command::new(&self.rsync);
        cmd.args(["-a", "-e", &ssh, from, to]);
        capture_command(&mut cmd, Duration::from_secs(600))
    }

    /// Wait (up to `file_wait`) for a file the remote wrote to show up here: a network
    /// filesystem may lag. True when it is there.
    pub fn wait_for_file(&self, file: &Path) -> bool {
        let start = Instant::now();
        loop {
            if file.is_file() {
                return true;
            }
            if start.elapsed() >= self.file_wait {
                return false;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}

/// What a helper command printed.
pub(crate) struct Captured {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Captured {
    /// stderr, else stdout, trimmed.
    pub fn text(&self) -> String {
        let t = if self.stderr.trim().is_empty() {
            &self.stdout
        } else {
            &self.stderr
        };
        t.trim().to_string()
    }
}

/// Run a helper to its end (at most `limit`, then it is killed), capturing its output. A
/// program that cannot be started is an error; any exit code is a result.
pub(crate) fn capture_command(cmd: &mut Command, limit: Duration) -> Result<Captured> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = spawn_retry(cmd)
        .wrap_err_with(|| format!("cannot start {}", cmd.get_program().to_string_lossy()))?;
    fn drain(mut r: impl std::io::Read + Send + 'static) -> std::thread::JoinHandle<String> {
        std::thread::spawn(move || {
            let mut b = vec![];
            let _ = r.read_to_end(&mut b);
            String::from_utf8_lossy(&b).into_owned()
        })
    }
    let (so, se) = (
        drain(child.stdout.take().unwrap()),
        drain(child.stderr.take().unwrap()),
    );
    let start = Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break Some(s);
        }
        if start.elapsed() >= limit {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    Ok(Captured {
        code: status.and_then(|s| s.code()).unwrap_or(-1),
        stdout: so.join().unwrap_or_default(),
        stderr: se.join().unwrap_or_default(),
    })
}

/// Spawn, trying again when a program written a moment ago is still open for writing in a
/// process forked meanwhile (ETXTBSY, os error 26).
pub(crate) fn spawn_retry(cmd: &mut Command) -> std::io::Result<std::process::Child> {
    // Both callers wait on this thread (the ssh/rsync here, not what runs on the remote).
    crate::orphan::die_with_parent(cmd);
    let mut tries = 0;
    loop {
        match cmd.spawn() {
            Err(e) if e.raw_os_error() == Some(26) && tries < 40 => {
                tries += 1;
                std::thread::sleep(Duration::from_millis(25));
            }
            other => return other,
        }
    }
}

/// An exit status with this code (a remote run's result has no process of ours to ask).
pub(crate) fn exit_status(code: i32) -> ExitStatus {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        ExitStatus::from_raw((code & 0xff) << 8)
    }
    #[cfg(not(unix))]
    {
        Command::new("cmd")
            .args(["/C", &format!("exit {code}")])
            .status()
            .expect("cmd")
    }
}

/// Quote a word for a POSIX shell (left as it is when it needs none).
pub fn quote(s: &str) -> String {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "_-./:=@%+,".contains(c))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

impl JobsSettings {
    /// Read `[jobs]` of a parsed `cdept.toml`. A remote that cannot be read is left out with a
    /// warning (unknown keys included).
    pub fn load(root: &toml::Table, warnings: &mut Vec<String>) -> JobsSettings {
        let mut out = JobsSettings::default();
        let Some(jobs) = root.get("jobs").and_then(|v| v.as_table()) else {
            if root.contains_key("jobs") {
                warnings.push("[jobs]: it must be a table".into());
            }
            return out;
        };
        for (key, value) in jobs {
            match (key.as_str(), value) {
                ("default", toml::Value::String(s)) => out.default = Some(s.clone()),
                ("remote", toml::Value::Table(t)) => {
                    for (name, v) in t {
                        match Remote::parse(name, v.clone()) {
                            Ok(r) => {
                                out.remotes.insert(name.clone(), r);
                            }
                            Err(e) => warnings.push(format!("[jobs.remote.{name}]: {e:#}")),
                        }
                    }
                }
                (other, _) => warnings.push(format!(
                    "[jobs]: '{other}' is not known or has the wrong type (default, remote.NAME)"
                )),
            }
        }
        if let Some(d) = &out.default
            && !out.remotes.contains_key(d)
        {
            warnings.push(format!("[jobs] default: there is no remote '{d}'"));
        }
        out
    }

    /// The remote called `name`; the error lists the known ones.
    pub fn get(&self, name: &str) -> Result<&Remote> {
        self.remotes.get(name).ok_or_else(|| {
            if self.remotes.is_empty() {
                eyre!("no remote '{name}': the project has no [jobs.remote.*] in cdept.toml")
            } else {
                eyre!(
                    "no remote '{name}' (there are: {})",
                    self.remotes.keys().cloned().collect::<Vec<_>>().join(", ")
                )
            }
        })
    }

    /// The remote a run uses: `chosen` (`--remote`), else the testbench's `remote`, else the
    /// project's `default`. `Ok(None)`: a local run.
    pub fn pick(&self, chosen: Option<&str>, testbench: &str) -> Result<Option<&Remote>> {
        let name = chosen
            .filter(|n| !n.is_empty())
            .or(Some(testbench).filter(|n| !n.is_empty()))
            .or(self.default.as_deref());
        name.map(|n| self.get(n)).transpose()
    }
}
