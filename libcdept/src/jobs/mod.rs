//! A small job queue and process supervisor (Phase 3/4): a pool of worker threads that
//! run jobs (a closure, or a command with cwd and environment), track their state,
//! capture a per-job log file, report progress, and cancel (kill the child, drop the
//! queued). Plain `std` threads; no async runtime. Design: docs/JOBS.md.
//!
//! ```no_run
//! use libcdept::jobs::{JobSpec, Queue};
//! let queue = Queue::new(4);
//! let id = queue.submit(JobSpec::command("hello", "echo", ["hi"]).log("/tmp/hello.log"));
//! queue.wait_all();
//! println!("{:?}", queue.info(id).unwrap().state);
//! ```

pub mod remote;

use std::collections::VecDeque;
use std::ffi::OsString;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use eyre::{Result, WrapErr, eyre};

/// Identifies a job within its [`Queue`] (numbered from 0 in submission order).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct JobId(pub usize);

/// Where a job is in its life. `Done`, `Failed` and `Cancelled` are final.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobState {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
}

impl JobState {
    pub fn is_final(self) -> bool {
        !matches!(self, JobState::Queued | JobState::Running)
    }
}

/// A copy of one job's status; what a UI polls ([`Queue::snapshot`]).
#[derive(Debug, Clone)]
pub struct JobInfo {
    pub id: JobId,
    pub name: String,
    pub state: JobState,
    /// 0.0 to 1.0, as reported by the job ([`JobCtx::progress`]); 1.0 once `Done`.
    pub progress: f32,
    /// The error, for a `Failed` job.
    pub error: Option<String>,
    /// The log file, when the job has one.
    pub log: Option<PathBuf>,
    /// Time spent running (so far, for a running job).
    pub elapsed: Duration,
}

/// Sent to listeners ([`Queue::on_event`]) on every state or progress change, from a
/// worker thread (or the thread that submits or cancels). Keep listeners quick and do
/// not call `submit`/`cancel` from them.
#[derive(Debug, Clone)]
pub struct JobEvent {
    pub info: JobInfo,
}

type Work = Box<dyn FnOnce(&JobCtx) -> Result<()> + Send + 'static>;

/// What to run. Build with [`JobSpec::command`] or [`JobSpec::func`].
pub struct JobSpec {
    name: String,
    log: Option<PathBuf>,
    work: Work,
}

impl JobSpec {
    /// A job that runs a closure. It should poll [`JobCtx::is_cancelled`] if it is long.
    pub fn func(
        name: impl Into<String>,
        work: impl FnOnce(&JobCtx) -> Result<()> + Send + 'static,
    ) -> JobSpec {
        JobSpec {
            name: name.into(),
            log: None,
            work: Box::new(work),
        }
    }

    /// A job that runs `program args…`; stdout and stderr go to the log. Fails when it
    /// exits with a non-zero status. Add [`CommandSpec::cwd`]/[`CommandSpec::env`] as needed.
    pub fn command<I, S>(
        name: impl Into<String>,
        program: impl Into<OsString>,
        args: I,
    ) -> CommandSpec
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        CommandSpec {
            name: name.into(),
            program: program.into(),
            args: args.into_iter().map(Into::into).collect(),
            cwd: None,
            env: vec![],
            log: None,
        }
    }

    /// Capture the job's log (see [`JobCtx::log`]) into this file (created at start).
    pub fn log(mut self, path: impl Into<PathBuf>) -> JobSpec {
        self.log = Some(path.into());
        self
    }
}

/// A command job under construction; [`Queue::submit`] takes it as is.
pub struct CommandSpec {
    name: String,
    program: OsString,
    args: Vec<OsString>,
    cwd: Option<PathBuf>,
    env: Vec<(OsString, OsString)>,
    log: Option<PathBuf>,
}

impl CommandSpec {
    pub fn cwd(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cwd = Some(dir.into());
        self
    }
    /// Variables set on top of the inherited environment.
    pub fn env(mut self, vars: impl IntoIterator<Item = (OsString, OsString)>) -> Self {
        self.env.extend(vars);
        self
    }
    pub fn log(mut self, path: impl Into<PathBuf>) -> Self {
        self.log = Some(path.into());
        self
    }
}

impl From<CommandSpec> for JobSpec {
    fn from(c: CommandSpec) -> JobSpec {
        let log = c.log.clone();
        let mut spec = JobSpec::func(c.name.clone(), move |ctx| {
            let mut cmd = Command::new(&c.program);
            cmd.args(&c.args).envs(c.env.iter().map(|(k, v)| (k, v)));
            if let Some(d) = &c.cwd {
                cmd.current_dir(d);
            }
            let status = ctx.run_command(&mut cmd)?;
            if status.success() {
                Ok(())
            } else {
                Err(eyre!(
                    "{} exited with {status}",
                    c.program.to_string_lossy()
                ))
            }
        });
        spec.log = log;
        spec
    }
}

/// What a running job gets: log, progress, cancellation, and a cancellable way to run a process.
pub struct JobCtx {
    id: JobId,
    cancel: Arc<AtomicBool>,
    log: Mutex<Option<File>>,
    log_path: Option<PathBuf>,
    shared: Arc<Shared>,
}

impl JobCtx {
    pub fn id(&self) -> JobId {
        self.id
    }
    /// True once [`Queue::cancel`] was called for this job.
    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }
    /// The flag [`JobCtx::is_cancelled`] reads, for code that polls it on its own (a
    /// batch program, Magic, the in-process ngspice).
    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.cancel.clone()
    }
    /// The job's log file, if it has one.
    pub fn log_path(&self) -> Option<&Path> {
        self.log_path.as_deref()
    }
    /// Append a line to the job's log (ignored without a log file).
    pub fn log(&self, line: &str) {
        if let Some(f) = self.log.lock().unwrap().as_mut() {
            let _ = writeln!(f, "{line}");
        }
    }
    /// Report progress (clamped to 0..=1); listeners get an event.
    pub fn progress(&self, fraction: f32) {
        let ev = {
            let mut g = self.shared.lock();
            let rec = &mut g.jobs[self.id.0];
            rec.progress = fraction.clamp(0.0, 1.0);
            rec.info()
        };
        self.shared.emit(ev);
    }
    /// Run `cmd` with stdout and stderr appended to the job log, and wait for it. If the
    /// job is cancelled meanwhile, the child is killed and this returns an error.
    pub fn run_command(&self, cmd: &mut Command) -> Result<ExitStatus> {
        Ok(self
            .run_command_limited(cmd, None)?
            .expect("no limit, no time-out"))
    }

    /// [`JobCtx::run_command`] with a time limit: the child is killed when it runs
    /// longer than `limit` and the result is `Ok(None)` (`None` or zero: no limit).
    pub fn run_command_limited(
        &self,
        cmd: &mut Command,
        limit: Option<Duration>,
    ) -> Result<Option<ExitStatus>> {
        let limit = limit.filter(|l| !l.is_zero());
        let start = std::time::Instant::now();
        let (out, err) = match self
            .log
            .lock()
            .unwrap()
            .as_ref()
            .map(|f| (f.try_clone(), f.try_clone()))
        {
            Some((Ok(a), Ok(b))) => (Stdio::from(a), Stdio::from(b)),
            _ => (Stdio::null(), Stdio::null()),
        };
        let mut child = cmd
            .stdin(Stdio::null())
            .stdout(out)
            .stderr(err)
            .spawn()
            .wrap_err_with(|| format!("cannot start {}", cmd.get_program().to_string_lossy()))?;
        loop {
            if self.is_cancelled() {
                let _ = child.kill();
                let _ = child.wait();
                return Err(eyre!("cancelled"));
            }
            if let Some(status) = child.try_wait()? {
                return Ok(Some(status));
            }
            if limit.is_some_and(|l| start.elapsed() >= l) {
                let _ = child.kill();
                let _ = child.wait();
                return Ok(None);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

/// A duration as written in a settings file or on the command line: `90s`, `10m`,
/// `1h30m`, `2h`, `500ms`, or a plain number of seconds. `0` is zero (callers read it
/// as "no limit").
pub fn parse_duration(text: &str) -> Result<Duration> {
    let t = text.trim();
    if t.is_empty() {
        return Err(eyre!("an empty duration"));
    }
    if let Ok(secs) = t.parse::<f64>() {
        if !(secs.is_finite() && secs >= 0.0) {
            return Err(eyre!("'{text}' is not a duration"));
        }
        return Ok(Duration::from_secs_f64(secs));
    }
    let mut total = Duration::ZERO;
    let mut rest = t;
    while !rest.is_empty() {
        let n_end = rest
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .unwrap_or(rest.len());
        let unit_end = rest[n_end..]
            .find(|c: char| c.is_ascii_digit())
            .map_or(rest.len(), |i| n_end + i);
        let (number, unit) = (&rest[..n_end], rest[n_end..unit_end].trim());
        let value: f64 = number
            .parse()
            .map_err(|_| eyre!("'{text}' is not a duration (like 90s, 10m, 1h30m)"))?;
        let scale = match unit {
            "ms" => 0.001,
            "s" | "sec" => 1.0,
            "m" | "min" => 60.0,
            "h" => 3600.0,
            _ => return Err(eyre!("'{text}' is not a duration (units: ms, s, m, h)")),
        };
        total += Duration::from_secs_f64(value * scale);
        rest = rest[unit_end..].trim_start();
    }
    Ok(total)
}

/// A duration the way [`parse_duration`] reads it (`15m`, `1h30m`, `45s`).
pub fn format_duration(d: Duration) -> String {
    let s = d.as_secs();
    if d.subsec_millis() != 0 && s < 60 {
        return format!("{}ms", d.as_millis());
    }
    let (h, m, s) = (s / 3600, (s / 60) % 60, s % 60);
    let mut out = String::new();
    if h > 0 {
        out.push_str(&format!("{h}h"));
    }
    if m > 0 {
        out.push_str(&format!("{m}m"));
    }
    if s > 0 || out.is_empty() {
        out.push_str(&format!("{s}s"));
    }
    out
}

struct Rec {
    id: JobId,
    name: String,
    state: JobState,
    progress: f32,
    error: Option<String>,
    log: Option<PathBuf>,
    cancel: Arc<AtomicBool>,
    work: Option<Work>,
    started: Option<Instant>,
    finished: Option<Instant>,
}

impl Rec {
    fn info(&self) -> JobInfo {
        JobInfo {
            id: self.id,
            name: self.name.clone(),
            state: self.state,
            progress: self.progress,
            error: self.error.clone(),
            log: self.log.clone(),
            elapsed: match (self.started, self.finished) {
                (Some(s), Some(f)) => f - s,
                (Some(s), None) => s.elapsed(),
                _ => Duration::ZERO,
            },
        }
    }
}

#[derive(Default)]
struct Inner {
    jobs: Vec<Rec>,
    pending: VecDeque<usize>,
    shutdown: bool,
}

/// A shareable [`Queue::on_event`] callback.
pub type Listener = Arc<dyn Fn(&JobEvent) + Send + Sync>;

#[derive(Default)]
struct Shared {
    inner: Mutex<Inner>,
    work: Condvar,
    done: Condvar,
    listeners: Mutex<Vec<Listener>>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
    fn emit(&self, info: JobInfo) {
        let ls: Vec<Listener> = self.listeners.lock().unwrap().clone();
        let ev = JobEvent { info };
        for l in ls {
            l(&ev);
        }
    }
}

/// A pool of worker threads and the jobs given to it. Dropping the queue cancels what is
/// left and joins the workers.
pub struct Queue {
    shared: Arc<Shared>,
    workers: Vec<JoinHandle<()>>,
}

impl Queue {
    /// A queue with `workers` threads (at least 1).
    pub fn new(workers: usize) -> Queue {
        let shared = Arc::new(Shared::default());
        let workers = (0..workers.max(1))
            .map(|_| {
                let s = shared.clone();
                std::thread::spawn(move || worker(s))
            })
            .collect();
        Queue { shared, workers }
    }

    /// The number of cores, the usual default for [`Queue::new`].
    pub fn default_workers() -> usize {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
    }

    /// Call `f` on every state or progress change of any job. See [`JobEvent`].
    pub fn on_event(&self, f: impl Fn(&JobEvent) + Send + Sync + 'static) {
        self.shared.listeners.lock().unwrap().push(Arc::new(f));
    }

    /// Queue a job (a [`JobSpec`] or a [`CommandSpec`]); it starts when a worker is free, in submission order.
    pub fn submit(&self, spec: impl Into<JobSpec>) -> JobId {
        let spec = spec.into();
        let info = {
            let mut g = self.shared.lock();
            let id = JobId(g.jobs.len());
            g.jobs.push(Rec {
                id,
                name: spec.name,
                state: JobState::Queued,
                progress: 0.0,
                error: None,
                log: spec.log,
                cancel: Arc::new(AtomicBool::new(false)),
                work: Some(spec.work),
                started: None,
                finished: None,
            });
            g.pending.push_back(id.0);
            g.jobs[id.0].info()
        };
        self.shared.work.notify_one();
        let id = info.id;
        self.shared.emit(info);
        id
    }

    /// Cancel a job: a queued one is dropped, a running one has its child killed (a
    /// closure should notice [`JobCtx::is_cancelled`]) and ends `Cancelled`. No effect on a final job.
    pub fn cancel(&self, id: JobId) {
        let ev = {
            let mut g = self.shared.lock();
            let Some(rec) = g.jobs.get_mut(id.0) else {
                return;
            };
            match rec.state {
                JobState::Queued => {
                    rec.state = JobState::Cancelled;
                    rec.work = None;
                    rec.finished = Some(Instant::now());
                    let info = rec.info();
                    g.pending.retain(|&i| i != id.0);
                    Some(info)
                }
                JobState::Running => {
                    rec.cancel.store(true, Ordering::SeqCst);
                    None
                }
                _ => None,
            }
        };
        if let Some(ev) = ev {
            self.shared.done.notify_all();
            self.shared.emit(ev);
        }
    }

    /// Cancel every job that is not final.
    pub fn cancel_all(&self) {
        let ids: Vec<JobId> = self
            .shared
            .lock()
            .jobs
            .iter()
            .filter(|j| !j.state.is_final())
            .map(|j| j.id)
            .collect();
        for id in ids {
            self.cancel(id);
        }
    }

    /// Block until every job submitted so far is final.
    pub fn wait_all(&self) {
        let mut g = self.shared.lock();
        while g.jobs.iter().any(|j| !j.state.is_final()) {
            g = self.shared.done.wait(g).unwrap_or_else(|e| e.into_inner());
        }
    }

    /// Block until job `id` is final; returns its last status (None for an unknown id).
    pub fn wait(&self, id: JobId) -> Option<JobInfo> {
        let mut g = self.shared.lock();
        while !g.jobs.get(id.0)?.state.is_final() {
            g = self.shared.done.wait(g).unwrap_or_else(|e| e.into_inner());
        }
        Some(g.jobs[id.0].info())
    }

    /// The status of one job.
    pub fn info(&self, id: JobId) -> Option<JobInfo> {
        self.shared.lock().jobs.get(id.0).map(Rec::info)
    }

    /// The status of all jobs, in submission order. Cheap; meant to be polled by a UI.
    pub fn snapshot(&self) -> Vec<JobInfo> {
        self.shared.lock().jobs.iter().map(Rec::info).collect()
    }
}

impl Drop for Queue {
    fn drop(&mut self) {
        self.cancel_all();
        self.shared.lock().shutdown = true;
        self.shared.work.notify_all();
        for w in self.workers.drain(..) {
            let _ = w.join();
        }
    }
}

fn worker(shared: Arc<Shared>) {
    loop {
        let (id, work, cancel, log, info) = {
            let mut g = shared.lock();
            let idx = loop {
                if let Some(i) = g.pending.pop_front() {
                    break i;
                }
                if g.shutdown {
                    return;
                }
                g = shared.work.wait(g).unwrap_or_else(|e| e.into_inner());
            };
            let rec = &mut g.jobs[idx];
            rec.state = JobState::Running;
            rec.started = Some(Instant::now());
            (
                rec.id,
                rec.work.take(),
                rec.cancel.clone(),
                rec.log.clone(),
                rec.info(),
            )
        };
        shared.emit(info);
        let Some(work) = work else { continue };
        let file = log.as_ref().and_then(|p| {
            if let Some(d) = p.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            File::create(p).ok()
        });
        let ctx = JobCtx {
            id,
            cancel: cancel.clone(),
            log: Mutex::new(file),
            log_path: log,
            shared: shared.clone(),
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&ctx)));
        let (state, error) = if cancel.load(Ordering::SeqCst) {
            (JobState::Cancelled, None)
        } else {
            match result {
                Ok(Ok(())) => (JobState::Done, None),
                Ok(Err(e)) => (JobState::Failed, Some(format!("{e:#}"))),
                Err(_) => (JobState::Failed, Some("the job panicked".to_string())),
            }
        };
        if let Some(e) = &error {
            ctx.log(&format!("error: {e}"));
        }
        let info = {
            let mut g = shared.lock();
            let rec = &mut g.jobs[id.0];
            rec.state = state;
            rec.error = error;
            rec.finished = Some(Instant::now());
            if state == JobState::Done {
                rec.progress = 1.0;
            }
            rec.info()
        };
        shared.done.notify_all();
        shared.emit(info);
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::AtomicUsize;

    fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    #[test]
    fn command_runs_logs_and_fails() {
        let dir = tempfile::tempdir().unwrap();
        let ok = script(
            dir.path(),
            "ok",
            "echo hello; echo oops >&2; echo $FOO; pwd",
        );
        let bad = script(dir.path(), "bad", "echo nope; exit 3");
        let q = Queue::new(2);
        let a = q.submit(
            JobSpec::command("ok", ok, ["x"])
                .cwd(dir.path())
                .env([("FOO".into(), "bar".into())])
                .log(dir.path().join("logs/a.log")),
        );
        let b = q.submit(
            JobSpec::command("bad", bad, Vec::<String>::new()).log(dir.path().join("b.log")),
        );
        q.wait_all();
        let (ia, ib) = (q.info(a).unwrap(), q.info(b).unwrap());
        assert_eq!(ia.state, JobState::Done);
        assert_eq!(ia.progress, 1.0);
        let log = std::fs::read_to_string(ia.log.unwrap()).unwrap();
        assert!(
            log.contains("hello") && log.contains("oops") && log.contains("bar"),
            "{log}"
        );
        assert!(
            log.contains(dir.path().file_name().unwrap().to_str().unwrap()),
            "{log}"
        );
        assert_eq!(ib.state, JobState::Failed);
        assert!(ib.error.unwrap().contains("status"), "exit code in error");
        assert!(
            std::fs::read_to_string(dir.path().join("b.log"))
                .unwrap()
                .contains("nope")
        );
    }

    #[test]
    fn runs_in_parallel_up_to_the_pool_size() {
        let running = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let q = Queue::new(3);
        for i in 0..9 {
            let (r, p) = (running.clone(), peak.clone());
            q.submit(JobSpec::func(format!("j{i}"), move |_| {
                let n = r.fetch_add(1, Ordering::SeqCst) + 1;
                p.fetch_max(n, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(40));
                r.fetch_sub(1, Ordering::SeqCst);
                Ok(())
            }));
        }
        q.wait_all();
        assert_eq!(peak.load(Ordering::SeqCst), 3);
        assert!(q.snapshot().iter().all(|j| j.state == JobState::Done));
    }

    #[test]
    fn cancel_kills_running_and_drops_queued() {
        let dir = tempfile::tempdir().unwrap();
        let slow = script(dir.path(), "slow", "echo started; exec sleep 30");
        let q = Queue::new(1);
        let running = q.submit(
            JobSpec::command("slow", &slow, Vec::<String>::new()).log(dir.path().join("s.log")),
        );
        let queued = q.submit(JobSpec::command("later", &slow, Vec::<String>::new()));
        let t = Instant::now();
        while q.info(running).unwrap().state != JobState::Running {
            assert!(t.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(5));
        }
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(q.info(queued).unwrap().state, JobState::Queued);
        q.cancel(queued);
        q.cancel(running);
        q.wait_all();
        assert!(
            t.elapsed() < Duration::from_secs(10),
            "the child was killed, not waited for"
        );
        assert_eq!(q.info(running).unwrap().state, JobState::Cancelled);
        assert_eq!(q.info(queued).unwrap().state, JobState::Cancelled);
        assert!(
            std::fs::read_to_string(dir.path().join("s.log"))
                .unwrap()
                .contains("started")
        );
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration("90").unwrap(), Duration::from_secs(90));
        assert_eq!(parse_duration("10m").unwrap(), Duration::from_secs(600));
        assert_eq!(parse_duration("1h30m").unwrap(), Duration::from_secs(5400));
        assert_eq!(
            parse_duration(" 2h 5s ").unwrap(),
            Duration::from_secs(7205)
        );
        assert_eq!(parse_duration("250ms").unwrap(), Duration::from_millis(250));
        assert_eq!(parse_duration("0").unwrap(), Duration::ZERO);
        for bad in ["", "ten", "5x", "-3", "m"] {
            assert!(parse_duration(bad).is_err(), "{bad}");
        }
        assert_eq!(format_duration(Duration::from_secs(900)), "15m");
        assert_eq!(format_duration(Duration::from_secs(5405)), "1h30m5s");
        assert_eq!(format_duration(Duration::from_millis(300)), "300ms");
    }

    #[test]
    fn events_progress_and_panics() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let q = Queue::new(1);
        let e = events.clone();
        q.on_event(move |ev| {
            e.lock()
                .unwrap()
                .push((ev.info.id, ev.info.state, ev.info.progress))
        });
        let a = q.submit(JobSpec::func("p", |ctx| {
            ctx.progress(0.5);
            ctx.log("half");
            Ok(())
        }));
        let b = q.submit(JobSpec::func("boom", |_| panic!("x")));
        q.wait_all();
        assert_eq!(q.info(b).unwrap().state, JobState::Failed);
        let ev = events.lock().unwrap().clone();
        assert!(ev.contains(&(a, JobState::Running, 0.5)), "{ev:?}");
        assert!(ev.contains(&(a, JobState::Done, 1.0)));
        assert_eq!(q.wait(a).unwrap().state, JobState::Done);
    }
}
