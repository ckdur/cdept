//! The embedded backend: UberWave's window on a thread of this process
//! (`cdept-uberwave`). There is one viewer window per process (the toolkit has one global
//! state); every open of a project run reuses it while it is open, a closed one is started
//! again by the next open.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub use cdept_uberwave::Options;
use cdept_uberwave::{Pane, Viewer};
use eyre::{Result, eyre};
use serde_json::Value;

use super::ipc::{Signal, Trace};

/// A handle on the process's viewer; clones share it.
#[derive(Clone)]
pub struct Shared(Arc<Mutex<Viewer>>);

static CURRENT: Mutex<Option<Shared>> = Mutex::new(None);
static SETTINGS: Mutex<Option<Options>> = Mutex::new(None);

/// Window settings for the next viewer that is started (`cdept wave --screenshot`: hidden
/// and of a known size). A running viewer keeps its own.
pub fn configure(options: Options) {
    *SETTINGS.lock().unwrap() = Some(options);
}

fn options() -> Options {
    SETTINGS.lock().unwrap().clone().unwrap_or_else(|| Options {
        title: Some("UberWave (cdept)".into()),
        ..Options::default()
    })
}

/// The running viewer, if its window is open.
pub fn running() -> Option<Shared> {
    let current = CURRENT.lock().unwrap().clone()?;
    current.is_running().then_some(current)
}

/// The running viewer (`true`: it was open already), else a new window.
pub fn acquire() -> Result<(Shared, bool)> {
    let mut slot = CURRENT.lock().unwrap();
    if let Some(v) = slot.as_ref()
        && v.is_running()
    {
        return Ok((v.clone(), true));
    }
    // A viewer that was closed: drop it (joins nothing, the thread ended) before the next.
    *slot = None;
    let viewer = Viewer::start(&options()).map_err(|e| eyre!("{e}"))?;
    let shared = Shared(Arc::new(Mutex::new(viewer)));
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
    if let Some(v) = CURRENT.lock().unwrap().take() {
        v.0.lock().unwrap().close();
    }
}

fn err(e: cdept_uberwave::Error) -> eyre::Report {
    eyre!("{e}")
}

fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn pane_of(pane: &Option<Value>) -> Pane {
    match pane {
        Some(Value::String(s)) if s == "new" => Pane::New,
        Some(Value::Number(n)) => n.as_u64().map_or(Pane::Default, |i| Pane::Index(i as u32)),
        _ => Pane::Default,
    }
}

impl Shared {
    pub fn is_running(&self) -> bool {
        self.0.lock().unwrap().is_running()
    }

    pub fn open(&self, path: &Path, analysis: Option<&str>, tag: Option<&str>) -> Result<()> {
        let v = self.0.lock().unwrap();
        v.open(&text(path), analysis, tag).map_err(err)
    }

    pub fn import(&self, path: &Path, analysis: Option<&str>, tag: Option<&str>) -> Result<()> {
        let v = self.0.lock().unwrap();
        v.import(&text(path), analysis, tag).map_err(err)
    }

    pub fn list_signals(
        &self,
        file: Option<&str>,
        filter: Option<&str>,
    ) -> Result<(Vec<Signal>, bool)> {
        let json = self
            .0
            .lock()
            .unwrap()
            .list_signals(file, filter)
            .map_err(err)?;
        let list: Value = serde_json::from_str(&json)?;
        let field = |s: &Value, k: &str| s.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let signals = list
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|s| Signal {
                        name: field(s, "name"),
                        file: field(s, "file"),
                        unit: field(s, "unit"),
                        complex: s.get("complex").and_then(Value::as_bool).unwrap_or(false),
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok((signals, false))
    }

    pub fn show(&self, signals: &[String], pane: Option<Value>) -> Result<Option<u64>> {
        let v = self.0.lock().unwrap();
        v.show(signals, pane_of(&pane))
            .map(|p| Some(p as u64))
            .map_err(err)
    }

    /// The raw `f64` data goes to the viewer as it is: no JSON, no thinning. A point the
    /// viewer cannot draw (x not finite, y infinite) is left out.
    pub fn add_trace(&self, t: &Trace) -> Result<()> {
        let pane = pane_of(&t.pane);
        let v = self.0.lock().unwrap();
        let clean = t.x.iter().all(|x| x.is_finite()) && t.y.iter().all(|y| !y.is_infinite());
        if clean {
            return v
                .add_trace(
                    &t.name,
                    &t.x,
                    &t.y,
                    t.unit.as_deref(),
                    t.x_unit.as_deref(),
                    pane,
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
        v.add_trace(
            &t.name,
            &x,
            &y,
            t.unit.as_deref(),
            t.x_unit.as_deref(),
            pane,
        )
        .map_err(err)
    }

    pub fn remove_trace(&self, name: &str) -> Result<()> {
        self.0.lock().unwrap().remove_trace(name).map_err(err)
    }

    pub fn screenshot(&self, path: &Path) -> Result<()> {
        self.0.lock().unwrap().screenshot(&text(path)).map_err(err)
    }

    /// The session's files as JSON, and its signals of one file: what the script checks read.
    pub fn list_files_json(&self) -> Result<String> {
        self.0.lock().unwrap().list_files().map_err(err)
    }

    pub fn list_signals_json(&self, file: Option<&str>) -> Result<String> {
        self.0.lock().unwrap().list_signals(file, None).map_err(err)
    }
}
