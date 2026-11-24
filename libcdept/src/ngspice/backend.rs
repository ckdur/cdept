//! The in-process backend of `sim` (`simulator_backend = "inproc"`, `cdept sim run
//! --backend inproc`): a run point simulated by libngspice inside CDePT instead of an
//! `ngspice -b` process, with the same files left behind (`deck.sp`, `out.log`,
//! `out.raw`) and the same `.meas` results, so results, specs, statistics and the
//! calculator do not see a difference.
//!
//! libngspice is one instance per process and is not re-entrant across threads, so a
//! single worker thread (`cdept-ngspice`) owns it: a run point is a request sent to that
//! thread, which runs the points one after the other. The job that sent it waits for
//! the answer; its cancel flag is read by the worker, which stops the simulation with
//! `bg_halt`. The library is initialised once; before every point the previous circuit
//! and plots are removed (`destroy all`, `remcirc`).
//!
//! A point runs like this: the folder's `.spiceinit` lines as commands, the deck without
//! its `.control` blocks loaded (`ngSpice_Circ`), then the control lines one by one,
//! each analysis (`tran`, `ac`, ...) as `bg_<analysis>` on ngspice's background thread so
//! that it reports progress (`tran: 45.2%`) and can be halted. A control block with
//! control flow (`if`, `while`, `foreach`, ...) cannot be run line by line: such a deck
//! is loaded whole and its control block runs while loading, without progress or
//! cancel until it ends.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use eyre::{Result, WrapErr, bail, eyre};

use super::{Event, LIB_ENV, Session, spiceinit_commands, strip_control_blocks, tail};
use crate::sim::run::{Outcome, parse_measures};

/// Whether the in-process backend can run here: an error that says why not (no
/// libngspice) and how to point CDePT at the library.
pub fn check() -> Result<()> {
    super::api().map(|_| ()).map_err(|e| {
        eyre!(
            "the in-process ngspice backend needs libngspice, which cannot be loaded: {e}. \
             Set {LIB_ENV} to the library (libngspice.so.0) or use the process backend \
             (--backend process, or simulator_backend = \"process\")"
        )
    })
}

/// What the worker tells the job that waits for a point.
enum Msg {
    /// 0..1 over the point's analyses.
    Progress(f32),
    /// A line ngspice printed.
    Line(String),
    Done(Result<Outcome>),
}

struct Request {
    deck: String,
    dir: PathBuf,
    env: Vec<(OsString, OsString)>,
    cancel: Arc<AtomicBool>,
    /// The time limit of the point, counted from when the worker starts it.
    limit: Option<Duration>,
    /// Set by the watchdog of the worker when the point ran past `limit`.
    timed_out: Arc<AtomicBool>,
    reply: Sender<Msg>,
}

fn worker() -> &'static Sender<Request> {
    static WORKER: OnceLock<Sender<Request>> = OnceLock::new();
    WORKER.get_or_init(|| {
        let (tx, rx) = channel::<Request>();
        std::thread::Builder::new()
            .name("cdept-ngspice".into())
            .spawn(move || serve(rx))
            .expect("cannot start the ngspice worker thread");
        tx
    })
}

fn serve(requests: Receiver<Request>) {
    for req in requests {
        let outcome = if req.cancel.load(Ordering::SeqCst) {
            Err(eyre!("cancelled"))
        } else {
            // The time limit: a watchdog sets `timed_out`, which the point reads like
            // the cancel flag (and halts the simulation the same way).
            let (done, finished) = channel::<()>();
            let flag = req.timed_out.clone();
            let watchdog = req.limit.filter(|l| !l.is_zero()).map(|limit| {
                std::thread::spawn(move || {
                    if finished.recv_timeout(limit)
                        == Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                    {
                        flag.store(true, Ordering::SeqCst);
                    }
                })
            });
            let outcome =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_point(&req)))
                    .unwrap_or_else(|_| Err(eyre!("the in-process ngspice run panicked")));
            drop(done);
            if let Some(w) = watchdog {
                let _ = w.join();
            }
            outcome
        };
        let _ = req.reply.send(Msg::Done(outcome));
    }
}

/// Run a deck in `dir` in-process, the way [`crate::sim::run::run`] does with the
/// program. `cancel` stops it (`bg_halt`); `progress` and `line` get the progress (0..1)
/// and every line ngspice prints, on the calling thread. Waits for the points queued
/// before it (one libngspice runs one point at a time).
pub fn run(
    deck: &str,
    dir: &Path,
    env: &[(OsString, OsString)],
    cancel: Option<Arc<AtomicBool>>,
    progress: impl FnMut(f32),
    line: impl FnMut(&str),
) -> Result<Outcome> {
    run_limited(deck, dir, env, cancel, None, progress, line)
}

/// [`run`] with a time limit, counted from when the point starts (not while it waits
/// for the points before it): the simulation is halted (`bg_halt`) when it runs longer
/// and the error is [`crate::sim::run::time_limit_error`]. A deck whose control block
/// has control flow runs whole and cannot be halted; for it the limit only applies
/// between its parts.
pub fn run_limited(
    deck: &str,
    dir: &Path,
    env: &[(OsString, OsString)],
    cancel: Option<Arc<AtomicBool>>,
    limit: Option<Duration>,
    mut progress: impl FnMut(f32),
    mut line: impl FnMut(&str),
) -> Result<Outcome> {
    check()?;
    let (reply, answers) = channel();
    let req = Request {
        deck: deck.to_string(),
        dir: dir.to_path_buf(),
        env: env.to_vec(),
        cancel: cancel.unwrap_or_default(),
        limit,
        timed_out: Arc::default(),
        reply,
    };
    worker()
        .send(req)
        .map_err(|_| eyre!("the ngspice worker thread has stopped"))?;
    for msg in answers {
        match msg {
            Msg::Progress(p) => progress(p),
            Msg::Line(l) => line(&l),
            Msg::Done(outcome) => return outcome,
        }
    }
    Err(eyre!("the ngspice worker thread has stopped"))
}

/// [`run`] as part of a job: progress goes to the job, ngspice's lines to its log, and
/// cancelling the job halts the simulation.
pub fn run_in_job(
    deck: &str,
    dir: &Path,
    env: &[(OsString, OsString)],
    ctx: &crate::jobs::JobCtx,
) -> Result<Outcome> {
    run_in_job_limited(deck, dir, env, ctx, None)
}

/// [`run_in_job`] with a time limit ([`run_limited`]).
pub fn run_in_job_limited(
    deck: &str,
    dir: &Path,
    env: &[(OsString, OsString)],
    ctx: &crate::jobs::JobCtx,
    limit: Option<Duration>,
) -> Result<Outcome> {
    ctx.log("in-process ngspice (libngspice)");
    run_limited(
        deck,
        dir,
        env,
        Some(ctx.cancel_flag()),
        limit,
        |p| ctx.progress(p),
        |l| ctx.log(l),
    )
}

/// Commands that run an analysis: sent as `bg_<command>`.
const ANALYSES: &[&str] = &[
    "op", "tran", "ac", "dc", "noise", "run", "pz", "sens", "tf", "disto", "pss", "sp", "resume",
];

/// Control-flow words: a block with one of them is not run line by line.
const CONTROL_FLOW: &[&str] = &[
    "if", "else", "elseif", "end", "while", "repeat", "dowhile", "foreach", "break", "continue",
    "label", "goto",
];

fn first_word(line: &str) -> String {
    line.split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// The lines of the deck's `.control` blocks, in order, or None when they cannot be run
/// one at a time (control flow). Comments and blanks are left out; `quit`/`exit` end
/// the list (in-process ngspice must not quit).
fn control_lines(deck: &str) -> Option<Vec<String>> {
    let mut out = vec![];
    let mut inside = false;
    for line in deck.lines() {
        let t = line.trim();
        let lower = t.to_ascii_lowercase();
        if lower.starts_with(".control") {
            inside = true;
        } else if lower.starts_with(".endc") {
            inside = false;
        } else if inside && !t.is_empty() && !t.starts_with('*') {
            let word = first_word(t);
            if CONTROL_FLOW.contains(&word.as_str()) || t.starts_with('+') {
                return None;
            }
            if word == "quit" || word == "exit" {
                break;
            }
            out.push(t.to_string());
        }
    }
    Some(out)
}

/// The percentage of a status line (`tran: 45.2%`), as a fraction.
fn status_fraction(status: &str) -> Option<f32> {
    let (before, _) = status.split_once('%')?;
    let number = before
        .rsplit(|c: char| c.is_whitespace() || c == ':')
        .next()?;
    let p: f32 = number.trim().parse().ok()?;
    p.is_finite().then(|| (p / 100.0).clamp(0.0, 1.0))
}

/// The worker's side: one point, on the thread that owns libngspice.
fn run_point(req: &Request) -> Result<Outcome> {
    let dir = &req.dir;
    std::fs::create_dir_all(dir).wrap_err_with(|| format!("cannot create {}", dir.display()))?;
    let dir = dir.canonicalize()?;
    let deck_file = dir.join("deck.sp");
    let raw = dir.join("out.raw");
    let log = dir.join("out.log");
    std::fs::write(&deck_file, &req.deck)
        .wrap_err_with(|| format!("cannot write {}", deck_file.display()))?;
    // A stale result must not pass for a new one.
    for old in [&raw, &log] {
        let _ = std::fs::remove_file(old);
    }
    // Initialises the library the first time; clears the previous circuit and plots.
    let session = Session::open()?;
    let mut point = Point {
        session: &session,
        req,
        text: String::new(),
        exit: None,
        halted: false,
    };
    // ngspice resolves `write out.raw` and relative includes against the process's
    // working directory, and `$VAR` in control lines against its environment.
    let previous_dir = std::env::current_dir().ok();
    let saved_env: Vec<_> = req
        .env
        .iter()
        .map(|(k, _)| (k.clone(), std::env::var_os(k)))
        .collect();
    // SAFETY: the worker thread is the only one that runs ngspice; the variables are the
    // project's tool environment, the same for every point (docs/NGSPICE.md).
    unsafe {
        for (k, v) in &req.env {
            if std::env::var_os(k).as_ref() != Some(v) {
                std::env::set_var(k, v);
            }
        }
    }
    let result = std::env::set_current_dir(&dir)
        .map_err(Into::into)
        .and_then(|()| point.simulate());
    point.pump();
    if let Some(p) = previous_dir {
        let _ = std::env::set_current_dir(p);
    }
    // SAFETY: as above.
    unsafe {
        for (k, old) in saved_env {
            match old {
                Some(v) if std::env::var_os(&k).as_ref() != Some(&v) => std::env::set_var(&k, v),
                Some(_) => {}
                None => std::env::remove_var(&k),
            }
        }
    }
    let (text, exit, halted) = (std::mem::take(&mut point.text), point.exit, point.halted);
    drop(session);
    std::fs::write(&log, &text).wrap_err_with(|| format!("cannot write {}", log.display()))?;
    if req.timed_out.load(Ordering::SeqCst)
        && halted
        && !req.cancel.load(Ordering::SeqCst)
        && let Some(limit) = req.limit
    {
        return Err(crate::sim::run::time_limit_error(limit, &tail(&text)));
    }
    if halted || req.cancel.load(Ordering::SeqCst) {
        bail!("cancelled");
    }
    if let Some(status) = exit {
        bail!(
            "ngspice asked to exit (status {status}); the in-process instance may be unusable now \
             (use the process backend)\n{}",
            tail(&text)
        );
    }
    result.map_err(|e| eyre!("the simulator failed: {e:#}\n{}", tail(&text)))?;
    if !raw.is_file() {
        bail!("the simulator failed, no raw file\n{}", tail(&text));
    }
    Ok(Outcome {
        deck: deck_file,
        raw,
        log,
        measures: parse_measures(&text),
    })
}

struct Point<'a> {
    session: &'a Session,
    req: &'a Request,
    /// What ngspice printed: `out.log`.
    text: String,
    exit: Option<i32>,
    halted: bool,
}

impl Point<'_> {
    fn cancelled(&self) -> bool {
        self.req.cancel.load(Ordering::SeqCst) || self.req.timed_out.load(Ordering::SeqCst)
    }

    fn handle(&mut self, ev: Event, analysis: Option<(usize, usize)>) {
        match ev {
            Event::Stdout(l) | Event::Stderr(l) => {
                self.text.push_str(&l);
                self.text.push('\n');
                let _ = self.req.reply.send(Msg::Line(l));
            }
            Event::Status(s) => {
                if let (Some((done, of)), Some(f)) = (analysis, status_fraction(&s)) {
                    let _ = self
                        .req
                        .reply
                        .send(Msg::Progress((done as f32 + f) / of as f32));
                }
            }
            Event::Exit { status, .. } => self.exit = Some(status),
            _ => {}
        }
    }

    /// Take the events so far.
    fn pump(&mut self) {
        while let Ok(ev) = self.session.events().try_recv() {
            self.handle(ev, None);
        }
    }

    fn simulate(&mut self) -> Result<()> {
        if let Ok(init) = std::fs::read_to_string(".spiceinit") {
            for cmd in spiceinit_commands(&init) {
                // A failing start-up line (an OSDI library already loaded) is not fatal.
                let _ = self.session.command(&cmd);
            }
        }
        self.pump();
        let Some(lines) = control_lines(&self.req.deck) else {
            // Control flow: the whole deck, its control block running while it loads.
            self.session.load_deck(&self.req.deck)?;
            return Ok(());
        };
        self.session
            .load_deck(&strip_control_blocks(&self.req.deck))?;
        self.pump();
        let of = lines
            .iter()
            .filter(|l| ANALYSES.contains(&first_word(l).as_str()))
            .count()
            .max(1);
        let mut done = 0;
        for line in &lines {
            if self.cancelled() || self.exit.is_some() {
                self.halted = self.halted || self.cancelled();
                break;
            }
            if !ANALYSES.contains(&first_word(line).as_str()) {
                // As in batch mode, a failing command is reported by ngspice and the
                // script goes on.
                let _ = self.session.command(line);
                self.pump();
                continue;
            }
            self.session.command(&format!("bg_{line}"))?;
            self.wait_background(done, of);
            done += 1;
            let _ = self.req.reply.send(Msg::Progress(done as f32 / of as f32));
        }
        Ok(())
    }

    /// Follow the background thread until it ends; halt it when the job is cancelled.
    fn wait_background(&mut self, done: usize, of: usize) {
        loop {
            match self
                .session
                .events()
                .recv_timeout(Duration::from_millis(20))
            {
                Ok(ev) => self.handle(ev, Some((done, of))),
                Err(_) if !self.session.is_running() => break,
                Err(_) => {}
            }
            if self.cancelled() && !self.halted && self.session.is_running() {
                let _ = self.session.halt();
                self.halted = true;
            }
        }
        // Whatever arrived before the thread ended.
        while let Ok(ev) = self.session.events().try_recv() {
            self.handle(ev, Some((done, of)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_control_lines() {
        let deck = "t\nr1 a 0 1\n.control\n* c\nset filetype=binary\ntran 1n 1u\nwrite out.raw all\nquit\nmeas x\n.endc\n.end\n";
        assert_eq!(
            control_lines(deck).unwrap(),
            ["set filetype=binary", "tran 1n 1u", "write out.raw all"]
        );
        assert!(control_lines("t\n.control\nforeach v 1 2\ntran 1n 1u\nend\n.endc\n").is_none());
        assert_eq!(
            control_lines("t\nr1 a 0 1\n.end\n").unwrap(),
            Vec::<String>::new()
        );
    }

    #[test]
    fn reads_status_percentages() {
        assert_eq!(status_fraction("tran: 45.0%"), Some(0.45));
        assert_eq!(status_fraction("tran 100%"), Some(1.0));
        assert_eq!(status_fraction("--ready--"), None);
    }

    /// Without the library `inproc` is an error that says what to do, never a fallback.
    #[test]
    fn a_missing_library_is_a_clear_error() {
        if super::super::available() {
            return;
        }
        let err = format!("{}", check().unwrap_err());
        assert!(err.contains(LIB_ENV) && err.contains("process"), "{err}");
    }

    /// Progress, cancel and the files, with the real library (skipped without it). The
    /// library is a process singleton: everything real is in this one test.
    #[test]
    fn real_library_runs_points_and_halts() {
        if !super::super::available() {
            eprintln!("skipped: no libngspice");
            return;
        }
        let deck = "t\nv1 in 0 pulse(0 1 0 1n 1n 1u 2u)\nr1 in out 1k\nc1 out 0 1n\n.control\nset filetype=binary\n\
                    set appendwrite\ntran 1n 20u\nwrite out.raw all\nmeas tran vmax max v(out)\n.endc\n.end\n";
        let dir = tempfile::tempdir().unwrap();
        let mut progress = vec![];
        let mut lines = 0;
        let o = run(
            deck,
            &dir.path().join("a"),
            &[],
            None,
            |p| progress.push(p),
            |_| lines += 1,
        )
        .unwrap();
        assert!(o.raw.is_file() && o.log.is_file() && o.deck.is_file());
        assert!(o.measures["vmax"] > 0.5, "{:?}", o.measures);
        assert_eq!(progress.last(), Some(&1.0));
        assert!(lines > 0);
        // The same deck again: a clean slate, the same numbers.
        let again = run(deck, &dir.path().join("b"), &[], None, |_| {}, |_| {}).unwrap();
        assert_eq!(again.measures, o.measures);
        // Cancel halts a long transient.
        let long = deck.replace("tran 1n 20u", "tran 0.1n 20m");
        let flag = Arc::new(AtomicBool::new(false));
        let f = flag.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            f.store(true, Ordering::SeqCst);
        });
        let t = std::time::Instant::now();
        let err = run(
            &long,
            &dir.path().join("c"),
            &[],
            Some(flag),
            |_| {},
            |_| {},
        )
        .unwrap_err();
        assert!(format!("{err}").contains("cancelled"), "{err}");
        assert!(t.elapsed() < Duration::from_secs(20), "{:?}", t.elapsed());
        // A time limit halts it the same way, and says so.
        let t = std::time::Instant::now();
        let err = run_limited(
            &long,
            &dir.path().join("e"),
            &[],
            None,
            Some(Duration::from_millis(300)),
            |_| {},
            |_| {},
        )
        .unwrap_err();
        let text = format!("{err}");
        assert!(
            text.starts_with("run point exceeded its time limit of 300ms"),
            "{text}"
        );
        assert!(t.elapsed() < Duration::from_secs(20), "{:?}", t.elapsed());
        // And the instance still works afterwards.
        let after = run(deck, &dir.path().join("d"), &[], None, |_| {}, |_| {}).unwrap();
        assert_eq!(after.measures, o.measures);
    }
}
