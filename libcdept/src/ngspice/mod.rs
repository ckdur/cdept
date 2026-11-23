//! ngspice in-process, through `libngspice` (the "shared" API), loaded at run time.
//!
//! There is no link-time dependency: the library is found with dlopen
//! (`$CDEPT_LIBNGSPICE`, else `libngspice.so.0`, `libngspice.so`, the platform's
//! other names, and the `lib` folder next to the `ngspice` program on `PATH`).
//! Without it [`available`] is false and [`Session::open`] is an error.
//!
//! **libngspice is a process-global singleton**: one circuit, one set of plots,
//! one set of callbacks. [`Session`] holds a process-wide lock, so only one exists
//! at a time ([`Session::open`] waits, [`Session::try_open`] does not). Never use
//! another copy of libngspice in the same process. Details: docs/NGSPICE.md.
//!
//! [`backend`] runs testbench points through it (`simulator_backend = "inproc"`).

pub mod backend;
mod sys;

use std::collections::BTreeMap;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use eyre::{Result, WrapErr, bail, eyre};

use crate::sim::run::{Outcome, parse_measures};

/// Environment variable that names the libngspice shared library.
pub const LIB_ENV: &str = "CDEPT_LIBNGSPICE";

/// What the simulator reports while it works.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// A line of the simulator's standard output.
    Stdout(String),
    /// A line of its standard error.
    Stderr(String),
    /// A status line (`--ready--`, `tran 45.2%`).
    Status(String),
    /// A simulation is about to deliver data: plot name and the vector names.
    InitData { plot: String, vectors: Vec<String> },
    /// One point of every vector, by name (the real part; complex ones also as
    /// `(re, im)` in `complex`). The scale vector is among them.
    Data { index: i32, count: i32, values: BTreeMap<String, f64>, complex: BTreeMap<String, (f64, f64)> },
    /// The background thread started (`true`) or ended (`false`).
    BackgroundRunning(bool),
    /// ngspice asked to exit (`quit`, or a fatal error); the process is NOT exited.
    Exit { status: i32, immediate: bool, quit: bool },
}

/// The values of one vector.
#[derive(Debug, Clone, PartialEq)]
pub enum VectorData {
    Real(Vec<f64>),
    /// `(re, im)` pairs.
    Complex(Vec<(f64, f64)>),
}

impl VectorData {
    pub fn len(&self) -> usize {
        match self {
            VectorData::Real(v) => v.len(),
            VectorData::Complex(v) => v.len(),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

static API: OnceLock<std::result::Result<sys::Api, String>> = OnceLock::new();
static INITIALISED: OnceLock<()> = OnceLock::new();
static SESSION_LOCK: Mutex<()> = Mutex::new(());
static SINK: Mutex<Option<Sender<Event>>> = Mutex::new(None);

/// Candidate library files, best first.
pub fn candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(p) = std::env::var_os(LIB_ENV)
        && !p.is_empty()
    {
        out.push(PathBuf::from(p));
        return out; // an explicit choice is the only one
    }
    for name in ["libngspice.so.0", "libngspice.so", "libngspice.dylib", "ngspice.dll", "libngspice-0.dll"] {
        out.push(PathBuf::from(name));
    }
    if let Some(prog) = crate::tools::find_in_path("ngspice")
        && let Some(root) = prog.canonicalize().ok().and_then(|p| p.parent()?.parent().map(Path::to_path_buf))
    {
        for lib in ["lib", "lib64"] {
            for name in ["libngspice.so.0", "libngspice.so"] {
                let p = root.join(lib).join(name);
                if p.is_file() {
                    out.push(p);
                }
            }
        }
    }
    out
}

fn api() -> Result<&'static sys::Api> {
    let loaded = API.get_or_init(|| {
        let mut errors = Vec::new();
        for path in candidates() {
            match sys::Api::load(&path) {
                Ok(api) => return Ok(api),
                Err(e) => errors.push(format!("{e:#}")),
            }
        }
        Err(format!("libngspice not found (set ${LIB_ENV}): {}", errors.join("; ")))
    });
    loaded.as_ref().map_err(|e| eyre!("{e}"))
}

/// Whether libngspice can be loaded here.
pub fn available() -> bool {
    api().is_ok()
}

fn emit(event: Event) {
    if let Ok(guard) = SINK.lock()
        && let Some(tx) = guard.as_ref()
    {
        let _ = tx.send(event);
    }
}

/// # Safety
/// `p` is null or a NUL-terminated string.
unsafe fn text(p: *const c_char) -> String {
    if p.is_null() {
        String::new()
    } else {
        // SAFETY: by the contract above.
        unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
    }
}

unsafe extern "C" fn on_char(s: *mut c_char, _id: c_int, _u: *mut c_void) -> c_int {
    // SAFETY: ngspice passes NUL-terminated strings.
    let s = unsafe { text(s) };
    match s.strip_prefix("stderr ") {
        Some(rest) => emit(Event::Stderr(rest.to_string())),
        None => emit(Event::Stdout(s.strip_prefix("stdout ").unwrap_or(&s).to_string())),
    }
    0
}

unsafe extern "C" fn on_stat(s: *mut c_char, _id: c_int, _u: *mut c_void) -> c_int {
    // SAFETY: as above.
    emit(Event::Status(unsafe { text(s) }));
    0
}

unsafe extern "C" fn on_exit(status: c_int, immediate: bool, quit: bool, _id: c_int, _u: *mut c_void) -> c_int {
    emit(Event::Exit { status, immediate, quit });
    0
}

unsafe extern "C" fn on_data(all: *mut sys::VecValuesAll, _n: c_int, _id: c_int, _u: *mut c_void) -> c_int {
    // SAFETY: ngspice passes a valid array of `count` pointers for the call.
    unsafe {
        let all = &*all;
        let mut values = BTreeMap::new();
        let mut complex = BTreeMap::new();
        for i in 0..all.count.max(0) as usize {
            let v = &**all.vecs.add(i);
            let name = text(v.name);
            if v.is_complex {
                complex.insert(name.clone(), (v.creal, v.cimag));
            }
            values.insert(name, v.creal);
        }
        emit(Event::Data { index: all.index, count: all.count, values, complex });
    }
    0
}

unsafe extern "C" fn on_init_data(all: *mut sys::VecInfoAll, _id: c_int, _u: *mut c_void) -> c_int {
    // SAFETY: as above.
    unsafe {
        let all = &*all;
        let vectors = (0..all.count.max(0) as usize).map(|i| text((**all.vecs.add(i)).name)).collect();
        emit(Event::InitData { plot: text(all.name), vectors });
    }
    0
}

unsafe extern "C" fn on_bg(not_running: bool, _id: c_int, _u: *mut c_void) -> c_int {
    emit(Event::BackgroundRunning(!not_running));
    0
}

/// Read a NULL-terminated `char**`.
///
/// # Safety
/// `p` is null or a NULL-terminated array of NUL-terminated strings.
unsafe fn strings(mut p: *mut *mut c_char) -> Vec<String> {
    let mut out = Vec::new();
    if p.is_null() {
        return out;
    }
    // SAFETY: by the contract above.
    unsafe {
        while !(*p).is_null() {
            out.push(text(*p));
            p = p.add(1);
        }
    }
    out
}

fn cstring(s: &str) -> Result<CString> {
    CString::new(s).map_err(|_| eyre!("text with a NUL byte"))
}

/// An open ngspice. See the module docs: only one at a time per process.
pub struct Session {
    api: &'static sys::Api,
    events: Receiver<Event>,
    _lock: MutexGuard<'static, ()>,
}

impl Session {
    /// Load the library if needed and take the process-wide session, waiting for
    /// another one to end.
    pub fn open() -> Result<Session> {
        let lock = SESSION_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        Session::start(lock)
    }

    /// As [`Session::open`], but `Ok(None)` when another session is open.
    pub fn try_open() -> Result<Option<Session>> {
        let lock = match SESSION_LOCK.try_lock() {
            Ok(l) => l,
            Err(std::sync::TryLockError::Poisoned(e)) => e.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return Ok(None),
        };
        Session::start(lock).map(Some)
    }

    fn start(lock: MutexGuard<'static, ()>) -> Result<Session> {
        let api = api()?;
        let (tx, rx) = channel();
        *SINK.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
        if INITIALISED.get().is_none() {
            // SAFETY: the callbacks match sharedspice.h; they only touch `SINK`.
            let rc = unsafe {
                (api.init)(
                    Some(on_char),
                    Some(on_stat),
                    Some(on_exit),
                    Some(on_data),
                    Some(on_init_data),
                    Some(on_bg),
                    std::ptr::null_mut(),
                )
            };
            if rc != 0 {
                bail!("ngSpice_Init failed ({rc})");
            }
            let _ = INITIALISED.set(());
        }
        let session = Session { api, events: rx, _lock: lock };
        // A clean slate: the previous session's circuit and plots.
        let _ = session.command("destroy all");
        let _ = session.command("remcirc");
        // Leave out the banner and the clean-up's own messages.
        while session.events.try_recv().is_ok() {}
        Ok(session)
    }

    /// The events so far and to come. Unbounded: drain it on long runs.
    pub fn events(&self) -> &Receiver<Event> {
        &self.events
    }

    /// Load a circuit (a whole deck; the title is its first line). Note that
    /// libngspice RUNS the deck's `.control` block while loading it (synchronously,
    /// as the batch program does); to stream a run with [`Session::run_background`]
    /// load [`strip_control_blocks`]`(deck)` instead.
    pub fn load_deck(&self, deck: &str) -> Result<()> {
        let mut lines: Vec<CString> = Vec::new();
        for l in deck.lines() {
            lines.push(cstring(l)?);
        }
        if !deck.lines().any(|l| l.trim().eq_ignore_ascii_case(".end")) {
            lines.push(cstring(".end")?);
        }
        let mut ptrs: Vec<*mut c_char> = lines.iter().map(|c| c.as_ptr() as *mut c_char).collect();
        ptrs.push(std::ptr::null_mut());
        // SAFETY: NULL-terminated array of NUL-terminated strings, alive for the call.
        let rc = unsafe { (self.api.circ)(ptrs.as_mut_ptr()) };
        if rc != 0 {
            bail!("ngSpice_Circ failed ({rc})");
        }
        Ok(())
    }

    /// Run one ngspice command (`run`, `osdi path`, `set x=1`, `bg_run`, ...).
    /// `Err` when ngspice reports failure.
    pub fn command(&self, cmd: &str) -> Result<()> {
        let c = cstring(cmd)?;
        // SAFETY: valid NUL-terminated string.
        let rc = unsafe { (self.api.command)(c.as_ptr() as *mut c_char) };
        if rc != 0 {
            bail!("ngspice command '{cmd}' failed ({rc})");
        }
        Ok(())
    }

    /// Run the loaded circuit's analyses and wait (`run`, synchronous).
    pub fn run(&self) -> Result<()> {
        self.command("run")
    }

    /// Start the analyses on ngspice's own thread and return; follow
    /// [`Session::events`] (or [`Session::wait_background`]) until
    /// [`Event::BackgroundRunning`]`(false)`.
    pub fn run_background(&self) -> Result<()> {
        self.command("bg_run")
    }

    /// Ask the background thread to stop (`bg_halt`).
    pub fn halt(&self) -> Result<()> {
        self.command("bg_halt")
    }

    /// Whether the background thread is running.
    pub fn is_running(&self) -> bool {
        // SAFETY: plain query.
        unsafe { (self.api.running)() }
    }

    /// Hand every event to `each` until the background thread has ended.
    pub fn wait_background(&self, mut each: impl FnMut(&Event)) {
        let mut started = false;
        loop {
            match self.events.recv_timeout(Duration::from_millis(100)) {
                Ok(ev) => {
                    each(&ev);
                    match ev {
                        Event::BackgroundRunning(true) => started = true,
                        Event::BackgroundRunning(false) => return,
                        _ => {}
                    }
                }
                // Quiet: finished before we looked, or never started.
                Err(_) if !self.is_running() && (started || self.events.try_recv().is_err()) => {
                    for ev in self.events.try_iter() {
                        each(&ev);
                    }
                    return;
                }
                Err(_) => {}
            }
        }
    }

    /// Run the deck's `.control` section by hand: every line between `.control` and
    /// `.endc`, as commands (for a deck loaded without it, see [`strip_control_blocks`]).
    pub fn run_control_blocks(&self, deck: &str) -> Result<()> {
        let mut inside = false;
        for line in deck.lines() {
            let t = line.trim();
            let lower = t.to_ascii_lowercase();
            if lower.starts_with(".control") {
                inside = true;
            } else if lower.starts_with(".endc") {
                inside = false;
            } else if inside && !t.is_empty() && !t.starts_with('*') {
                self.command(t)?;
            }
        }
        Ok(())
    }

    /// The name of the current plot (`tran1`).
    pub fn current_plot(&self) -> String {
        // SAFETY: returns a NUL-terminated string owned by ngspice.
        unsafe { text((self.api.cur_plot)()) }
    }

    /// All plots, newest first.
    pub fn plot_names(&self) -> Vec<String> {
        // SAFETY: NULL-terminated array owned by ngspice.
        unsafe { strings((self.api.all_plots)()) }
    }

    /// The vector names of a plot.
    pub fn vectors(&self, plot: &str) -> Vec<String> {
        let Ok(p) = cstring(plot) else { return Vec::new() };
        // SAFETY: as above.
        unsafe { strings((self.api.all_vecs)(p.as_ptr() as *mut c_char)) }
    }

    /// The values of `plot.name` (an empty plot means the current one).
    pub fn vector(&self, plot: &str, name: &str) -> Result<VectorData> {
        let full = if plot.is_empty() { name.to_string() } else { format!("{plot}.{name}") };
        let c = cstring(&full)?;
        // SAFETY: the info and its arrays belong to ngspice and live while the plot does;
        // they are copied out before returning.
        unsafe {
            let info = (self.api.get_vec_info)(c.as_ptr() as *mut c_char);
            if info.is_null() {
                bail!("no vector {full}");
            }
            let info = &*info;
            let n = info.length.max(0) as usize;
            if !info.real.is_null() {
                Ok(VectorData::Real(std::slice::from_raw_parts(info.real, n).to_vec()))
            } else if !info.comp.is_null() {
                let s = std::slice::from_raw_parts(info.comp, n);
                Ok(VectorData::Complex(s.iter().map(|c| (c.re, c.im)).collect()))
            } else {
                Ok(VectorData::Real(Vec::new()))
            }
        }
    }

    /// The text ngspice printed, from the events not yet taken (other events are dropped).
    pub fn take_output(&self) -> String {
        let mut out = String::new();
        for ev in self.events.try_iter() {
            if let Event::Stdout(l) | Event::Stderr(l) = ev {
                out.push_str(&l);
                out.push('\n');
            }
        }
        out
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if self.is_running() {
            let _ = self.halt();
            while self.is_running() {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        let _ = self.command("destroy all");
        let _ = self.command("remcirc");
        *SINK.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

/// The deck without its `.control` ... `.endc` blocks.
pub fn strip_control_blocks(deck: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for line in deck.lines() {
        let lower = line.trim().to_ascii_lowercase();
        if lower.starts_with(".control") {
            inside = true;
        } else if lower.starts_with(".endc") {
            inside = false;
        } else if !inside {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// The lines of a `.spiceinit` as ngspice commands (comments and blanks dropped).
fn spiceinit_commands(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('*') && !l.starts_with('#'))
        .map(str::to_string)
        .collect()
}

fn tail(text: &str) -> String {
    let lines: Vec<&str> = text.lines().rev().take(12).collect();
    lines.into_iter().rev().collect::<Vec<_>>().join("\n")
}

/// Run a deck in-process, as [`crate::sim::run::run`] does with the program: `deck.sp`
/// is written into `dir`, a `.spiceinit` found there is applied as commands (the
/// library does not read one), the process's working directory is `dir` for the
/// run (relative `write`/`include` paths), the deck's `.control` block runs, and
/// the log (`out.log`) and the raw file (`out.raw`, written by the control block)
/// are left in `dir`. `env` is set for the process during the run.
///
/// Holds the session lock for the whole run, and changes the process working
/// directory and environment meanwhile: do not call it while other threads rely on either.
pub fn run_deck(deck: &str, dir: &Path, env: &[(std::ffi::OsString, std::ffi::OsString)]) -> Result<Outcome> {
    std::fs::create_dir_all(dir).wrap_err_with(|| format!("cannot create {}", dir.display()))?;
    let dir = dir.canonicalize()?;
    let deck_file = dir.join("deck.sp");
    let raw = dir.join("out.raw");
    let log = dir.join("out.log");
    std::fs::write(&deck_file, deck)?;
    // A stale result must not pass for a new one.
    for old in [&raw, &log] {
        let _ = std::fs::remove_file(old);
    }
    let session = Session::open()?;
    let previous_dir = std::env::current_dir().ok();
    let saved_env: Vec<_> = env.iter().map(|(k, _)| (k.clone(), std::env::var_os(k))).collect();
    // SAFETY: documented above; the caller accepts process-global changes.
    unsafe {
        for (k, v) in env {
            std::env::set_var(k, v);
        }
    }
    let result = (|| -> Result<()> {
        std::env::set_current_dir(&dir)?;
        if let Ok(init) = std::fs::read_to_string(dir.join(".spiceinit")) {
            for cmd in spiceinit_commands(&init) {
                // A failing start-up line (an OSDI library already loaded) is not fatal.
                let _ = session.command(&cmd);
            }
        }
        // Loading runs the deck's control block.
        session.load_deck(deck)
    })();
    let output = session.take_output();
    if let Some(p) = previous_dir {
        let _ = std::env::set_current_dir(p);
    }
    // SAFETY: as above.
    unsafe {
        for (k, old) in saved_env {
            match old {
                Some(v) => std::env::set_var(k, v),
                None => std::env::remove_var(k),
            }
        }
    }
    drop(session);
    std::fs::write(&log, &output)?;
    if let Err(e) = result {
        bail!("the simulator failed: {e:#}\n{}", tail(&output));
    }
    if !raw.is_file() {
        bail!("the simulator left no raw file\n{}", tail(&output));
    }
    Ok(Outcome { deck: deck_file, raw, log, measures: parse_measures(&output) })
}

#[cfg(test)]
mod tests {
    use super::*;

    const RC: &str = "rc lowpass\nv1 in 0 dc 0 pulse(0 1 0 1n 1n 1u 2u)\nr1 in out 1k\nc1 out 0 1n\n.tran 10n 2u\n.end\n";

    #[test]
    fn spiceinit_lines() {
        let c = spiceinit_commands("* c\n\nosdi /x/psp103.osdi\nset ngbehavior=hsa\n");
        assert_eq!(c, ["osdi /x/psp103.osdi", "set ngbehavior=hsa"]);
    }

    #[test]
    fn strips_control() {
        let d = strip_control_blocks("t\nr1 a 0 1\n.control\nrun\n.endc\n.end\n");
        assert_eq!(d, "t\nr1 a 0 1\n.end\n");
    }

    #[test]
    fn lookup_does_not_panic_without_the_library() {
        let _ = available();
        let _ = candidates();
    }

    // One test for everything real: the library is a process singleton.
    #[test]
    fn real_library() {
        if !available() {
            eprintln!("skipped: no libngspice");
            return;
        }
        let s = Session::open().unwrap();
        assert!(Session::try_open().is_ok_and(|o| o.is_none()), "a second session must be refused");
        s.load_deck(RC).unwrap();
        s.run().unwrap();
        let plot = s.current_plot();
        assert!(plot.starts_with("tran"), "{plot}");
        assert!(s.plot_names().contains(&plot));
        let names = s.vectors(&plot);
        assert!(names.iter().any(|n| n == "time") && names.iter().any(|n| n == "out"), "{names:?}");
        let VectorData::Real(out) = s.vector(&plot, "out").unwrap() else { panic!("complex") };
        assert!(out.len() > 50 && out.iter().cloned().fold(0.0, f64::max) > 0.5);

        s.load_deck(RC).unwrap();
        s.run_background().unwrap();
        let (mut init, mut points) = (false, 0);
        s.wait_background(|e| match e {
            Event::InitData { vectors, .. } => init = vectors.iter().any(|v| v == "time"),
            Event::Data { .. } => points += 1,
            _ => {}
        });
        assert!(init && points > 50, "init {init} points {points}");

        s.load_deck("rc ac\nv1 in 0 dc 0 ac 1\nr1 in out 1k\nc1 out 0 1n\n.ac dec 5 1k 1meg\n.end\n").unwrap();
        s.run().unwrap();
        let plot = s.current_plot();
        assert!(plot.starts_with("ac"), "{plot}");
        assert!(matches!(s.vector(&plot, "out").unwrap(), VectorData::Complex(_)));
        drop(s);

        let deck = "t\nv1 in 0 pulse(0 1 0 1n 1n 1u 2u)\nr1 in out 1k\nc1 out 0 1n\n.control\ntran 10n 2u\nwrite out.raw all\n\
                    meas tran vmax max v(out)\n.endc\n.end\n";
        let dir = tempfile::tempdir().unwrap();
        let o = run_deck(deck, &dir.path().join("r"), &[]).unwrap();
        assert!(o.raw.is_file() && o.log.is_file());
        assert!(o.measures["vmax"] > 0.5, "{:?}", o.measures);
    }
}
