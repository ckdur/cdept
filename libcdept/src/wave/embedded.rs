//! The embedded backend: UberWave's window on a thread of this process
//! (`cdept-uberwave`). There is one viewer window per process (the toolkit has one global
//! state); every open of a project run reuses it while it is open, a closed one is started
//! again by the next open.
//!
//! Everything here is a plain function call with Rust/C values: no JSON, no socket, no
//! files in between (the calculator's `f64` arrays are handed over as they are).

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub use cdept_uberwave::{CursorValue, Cursors, FileInfo, Marker, Options, SignalList};
use cdept_uberwave::{Pane, TraceStyle, Viewer};
use eyre::{Result, eyre};

use super::ipc::{PaneSel, Signal, Trace};

/// A handle on the process's viewer; clones share it.
#[derive(Clone)]
pub struct Shared(Arc<Mutex<Viewer>>);

static CURRENT: Mutex<Option<Shared>> = Mutex::new(None);
static SETTINGS: Mutex<Option<Options>> = Mutex::new(None);

/// What the GUI wants to hear about cursor moves, for every viewer that is started.
type Listener = Arc<dyn Fn(Option<f64>, Option<f64>) + Send + Sync>;
static LISTENER: Mutex<Option<Listener>> = Mutex::new(None);

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Window settings for the next viewer that is started (`cdept wave --screenshot`: hidden
/// and of a known size). A running viewer keeps its own.
pub fn configure(options: Options) {
    *lock(&SETTINGS) = Some(options);
}

fn options() -> Options {
    lock(&SETTINGS).clone().unwrap_or_else(|| Options {
        title: Some("UberWave (cdept)".into()),
        ..Options::default()
    })
}

/// Call `listener` when a cursor of the viewer moves (on the viewer's thread: keep it
/// short, never call the viewer from it). Set on the running viewer and on every one
/// started later; `None` removes it.
pub fn set_cursor_listener(listener: Option<Listener>) {
    *lock(&LISTENER) = listener.clone();
    if let Some(v) = running() {
        let _ = v.attach_listener(listener);
    }
}

/// The running viewer, if its window is open.
pub fn running() -> Option<Shared> {
    let current = lock(&CURRENT).clone()?;
    current.is_running().then_some(current)
}

/// The running viewer (`true`: it was open already), else a new window.
pub fn acquire() -> Result<(Shared, bool)> {
    let mut slot = lock(&CURRENT);
    if let Some(v) = slot.as_ref()
        && v.is_running()
    {
        return Ok((v.clone(), true));
    }
    // A viewer that was closed: drop it (joins nothing, the thread ended) before the next.
    *slot = None;
    let viewer = Viewer::start(&options()).map_err(|e| eyre!("{e}"))?;
    let shared = Shared(Arc::new(Mutex::new(viewer)));
    if let Some(l) = lock(&LISTENER).clone() {
        let _ = shared.attach_listener(Some(l));
    }
    *slot = Some(shared.clone());
    Ok((shared, false))
}

/// Block until the viewer window is closed (the end of `cdept wave`). Returns at once
/// when no viewer runs.
pub fn wait_closed() {
    while let Some(v) = running() {
        let _ = &v;
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Close the window of the viewer, if one is open.
pub fn close() {
    let current = lock(&CURRENT).take();
    if let Some(v) = current {
        lock(&v.0).close();
    }
}

fn err(e: cdept_uberwave::Error) -> eyre::Report {
    eyre!("{e}")
}

fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn pane_of(pane: PaneSel) -> Pane {
    match pane {
        PaneSel::Default => Pane::Default,
        PaneSel::New => Pane::New,
        PaneSel::Index(i) => Pane::Index(i),
    }
}

impl Shared {
    pub fn is_running(&self) -> bool {
        lock(&self.0).is_running()
    }

    fn attach_listener(&self, listener: Option<Listener>) -> Result<()> {
        let boxed: Option<cdept_uberwave::CursorListener> =
            listener.map(|l| Box::new(move |a, b| l(a, b)) as cdept_uberwave::CursorListener);
        lock(&self.0).set_cursor_listener(boxed).map_err(err)
    }

    pub fn open(&self, path: &Path, analysis: Option<&str>, tag: Option<&str>) -> Result<()> {
        lock(&self.0).open(&text(path), analysis, tag).map_err(err)
    }

    pub fn import(&self, path: &Path, analysis: Option<&str>, tag: Option<&str>) -> Result<()> {
        lock(&self.0)
            .import(&text(path), analysis, tag)
            .map_err(err)
    }

    /// The files of the session.
    pub fn list_files(&self) -> Result<Vec<FileInfo>> {
        lock(&self.0).list_files().map_err(err)
    }

    /// The signals, typed as the viewer gives them.
    pub fn signals(&self, file: Option<&str>, filter: Option<&str>) -> Result<SignalList> {
        lock(&self.0).list_signals(file, filter).map_err(err)
    }

    pub fn list_signals(
        &self,
        file: Option<&str>,
        filter: Option<&str>,
    ) -> Result<(Vec<Signal>, bool)> {
        let list = self.signals(file, filter)?;
        Ok((
            list.signals
                .into_iter()
                .map(|s| Signal {
                    name: s.name,
                    file: s.file,
                    unit: s.unit,
                    complex: s.complex,
                })
                .collect(),
            list.truncated,
        ))
    }

    pub fn show(&self, signals: &[String], pane: PaneSel) -> Result<Option<u64>> {
        lock(&self.0)
            .show(signals, pane_of(pane))
            .map(|p| Some(p as u64))
            .map_err(err)
    }

    /// The raw `f64` data goes to the viewer as it is: no JSON, no thinning. A point the
    /// viewer cannot draw (x not finite, y infinite) is left out; a trace whose x runs
    /// backwards anywhere is refused by the viewer with its reason.
    pub fn add_trace(&self, t: &Trace) -> Result<()> {
        let pane = pane_of(t.pane);
        let style = TraceStyle {
            x_log: t.x_log,
            fold: t.fold,
        };
        let v = lock(&self.0);
        let clean = t.x.iter().all(|x| x.is_finite()) && t.y.iter().all(|y| !y.is_infinite());
        if clean {
            return v
                .add_trace_styled(
                    &t.name,
                    &t.x,
                    &t.y,
                    t.unit.as_deref(),
                    t.x_unit.as_deref(),
                    pane,
                    style,
                )
                .map_err(err);
        }
        let (mut x, mut y) = (Vec::with_capacity(t.x.len()), Vec::with_capacity(t.y.len()));
        for (a, b) in t.x.iter().zip(&t.y) {
            if a.is_finite() && !b.is_infinite() {
                x.push(*a);
                y.push(*b);
            }
        }
        if x.is_empty() {
            return Err(eyre!("trace '{}' has no point that can be drawn", t.name));
        }
        v.add_trace_styled(
            &t.name,
            &x,
            &y,
            t.unit.as_deref(),
            t.x_unit.as_deref(),
            pane,
            style,
        )
        .map_err(err)
    }

    pub fn remove_trace(&self, name: &str) -> Result<()> {
        lock(&self.0).remove_trace(name).map_err(err)
    }

    pub fn screenshot(&self, path: &Path) -> Result<()> {
        lock(&self.0).screenshot(&text(path)).map_err(err)
    }

    pub fn set_cursors(&self, a: Option<f64>, b: Option<f64>) -> Result<()> {
        lock(&self.0).set_cursors(a, b).map_err(err)
    }

    /// The cursors and the values of the plotted traces under them.
    pub fn cursors(&self) -> Result<Cursors> {
        lock(&self.0).cursors().map_err(err)
    }

    pub fn add_marker(&self, marker: &Marker) -> Result<()> {
        lock(&self.0).add_marker(marker).map_err(err)
    }

    pub fn clear_markers(&self, group: Option<&str>) -> Result<u32> {
        lock(&self.0).clear_markers(group).map_err(err)
    }
}
