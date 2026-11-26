//! Which UberWave CDePT talks to, and the one interface to both.
//!
//! * [`Backend::Embedded`]: the viewer is part of this executable (`cdept-uberwave`, built
//!   with `$UBERWAVE_SRC`), its window runs on a thread of this process. No socket, no
//!   JSON for data, no decimation.
//! * [`Backend::Process`]: `UberWaveViewer` as a separate program over IPC protocol 1
//!   ([`super::ipc`]). The fallback when UberWave is not built in.
//!
//! The default is `Embedded` when it is built in, else `Process`. Override, strongest
//! first: [`set_override`] (`cdept wave --backend`), `$CDEPT_UBERWAVE_BACKEND`, `[wave]
//! backend` of `cdept.toml`.

use std::path::Path;
use std::sync::Mutex;

use eyre::{Result, eyre};
use serde_json::{Value, json};

use super::embedded::{CursorValue, Cursors, Marker};
use super::ipc::{Client, PaneSel, Signal, Trace};
use crate::Project;

/// Environment variable that chooses the backend: `embedded` or `process`.
pub const BACKEND_ENV: &str = "CDEPT_UBERWAVE_BACKEND";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Embedded,
    Process,
}

impl Backend {
    pub fn name(self) -> &'static str {
        match self {
            Backend::Embedded => "embedded",
            Backend::Process => "process",
        }
    }

    pub fn parse(text: &str) -> Option<Backend> {
        match text.trim().to_ascii_lowercase().as_str() {
            "embedded" | "builtin" | "built-in" => Some(Backend::Embedded),
            "process" | "ipc" => Some(Backend::Process),
            _ => None,
        }
    }
}

static OVERRIDE: Mutex<Option<Backend>> = Mutex::new(None);

/// What `cdept wave --backend` says; strongest of all.
pub fn set_override(backend: Option<Backend>) {
    *OVERRIDE.lock().unwrap() = backend;
}

/// UberWave is built into this executable.
pub fn embedded_available() -> bool {
    cdept_uberwave::available()
}

/// The backend to use. An explicit `embedded` that is not built in is an error that says
/// how to get it.
pub fn choose(project: Option<&Project>) -> Result<Backend> {
    choose_with(
        project.and_then(|p| p.wave.backend),
        std::env::var(BACKEND_ENV).ok().as_deref(),
        *OVERRIDE.lock().unwrap(),
        embedded_available(),
    )
}

/// [`choose`] with every input given.
pub fn choose_with(
    config: Option<Backend>,
    env: Option<&str>,
    over: Option<Backend>,
    available: bool,
) -> Result<Backend> {
    let from_env = match env.filter(|e| !e.trim().is_empty()) {
        Some(text) => Some(
            Backend::parse(text)
                .ok_or_else(|| eyre!("{BACKEND_ENV}={text}: it must be `embedded` or `process`"))?,
        ),
        None => None,
    };
    match over.or(from_env).or(config) {
        Some(Backend::Embedded) if !available => Err(eyre!(
            "the embedded UberWave was asked for, but {}; use the process backend (--backend process) or build it in \
             (docs/howto/view-waveforms.md)",
            cdept_uberwave::NOT_BUILT_IN
        )),
        Some(b) => Ok(b),
        None if available => Ok(Backend::Embedded),
        None => Ok(Backend::Process),
    }
}

/// The operations CDePT needs from a running viewer, whatever it is.
pub trait ViewerOps {
    /// Replace the session by one file.
    fn open(&mut self, path: &Path, analysis: Option<&str>, tag: Option<&str>) -> Result<()>;
    /// Add a file to the session.
    fn import(&mut self, path: &Path, analysis: Option<&str>, tag: Option<&str>) -> Result<()>;
    fn list_signals(
        &mut self,
        file: Option<&str>,
        filter: Option<&str>,
    ) -> Result<(Vec<Signal>, bool)>;
    /// Show signals in a pane; the pane they went to.
    fn show(&mut self, signals: &[String], pane: PaneSel) -> Result<Option<u64>>;
    fn add_trace(&mut self, trace: &Trace) -> Result<()>;
    fn remove_trace(&mut self, name: &str) -> Result<()>;
    /// Save the window as PNG (absolute path).
    fn screenshot(&mut self, path: &Path) -> Result<()>;
    /// Traces should be thinned before they are sent (a socket carries them as JSON).
    fn wants_decimation(&self) -> bool;
    /// Place the cursors (`None` leaves one alone).
    fn set_cursors(&mut self, a: Option<f64>, b: Option<f64>) -> Result<()>;
    /// The cursors and the values of the plotted traces under them.
    fn cursors(&mut self) -> Result<Cursors>;
    /// CDePT's markers (measured instants and points) on the plot.
    fn add_marker(&mut self, marker: &Marker) -> Result<()>;
    /// Remove the markers of a group (`None`: all).
    fn clear_markers(&mut self, group: Option<&str>) -> Result<u32>;
}

/// The cursors of a `get_cursors` answer (`{a, b, values:{sig:{a,b}}}`).
fn cursors_of(v: &Value) -> Cursors {
    let num = |x: Option<&Value>| x.and_then(Value::as_f64).filter(|f| f.is_finite());
    Cursors {
        a: num(v.get("a")),
        b: num(v.get("b")),
        values: v
            .get("values")
            .and_then(Value::as_object)
            .map(|m| {
                m.iter()
                    .map(|(k, x)| CursorValue {
                        signal: k.clone(),
                        a: num(x.get("a")),
                        b: num(x.get("b")),
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
}

const NO_MARKERS: &str =
    "markers need the built-in UberWave (the UberWaveViewer process has no markers)";

impl ViewerOps for Client {
    fn open(&mut self, path: &Path, analysis: Option<&str>, tag: Option<&str>) -> Result<()> {
        Client::open(self, path, analysis, tag)
    }
    fn import(&mut self, path: &Path, analysis: Option<&str>, tag: Option<&str>) -> Result<()> {
        Client::import(self, path, analysis, tag)
    }
    fn list_signals(
        &mut self,
        file: Option<&str>,
        filter: Option<&str>,
    ) -> Result<(Vec<Signal>, bool)> {
        Client::list_signals(self, file, filter)
    }
    fn show(&mut self, signals: &[String], pane: PaneSel) -> Result<Option<u64>> {
        Client::show(self, signals, pane)
    }
    fn add_trace(&mut self, trace: &Trace) -> Result<()> {
        Client::add_trace(self, trace)
    }
    fn remove_trace(&mut self, name: &str) -> Result<()> {
        Client::remove_trace(self, name)
    }
    fn screenshot(&mut self, path: &Path) -> Result<()> {
        self.request("screenshot", json!({ "path": path.to_string_lossy() }))
            .map(drop)
    }
    fn wants_decimation(&self) -> bool {
        true
    }
    fn set_cursors(&mut self, a: Option<f64>, b: Option<f64>) -> Result<()> {
        Client::set_cursors(self, a, b)
    }
    fn cursors(&mut self) -> Result<Cursors> {
        Ok(cursors_of(&self.get_cursors()?))
    }
    fn add_marker(&mut self, _marker: &Marker) -> Result<()> {
        Err(eyre!(NO_MARKERS))
    }
    fn clear_markers(&mut self, _group: Option<&str>) -> Result<u32> {
        Err(eyre!(NO_MARKERS))
    }
}

/// A connection to a viewer, of either backend.
pub enum Connection {
    Ipc(Client),
    Embedded(super::embedded::Shared),
}

impl Connection {
    pub fn backend(&self) -> Backend {
        match self {
            Connection::Ipc(_) => Backend::Process,
            Connection::Embedded(_) => Backend::Embedded,
        }
    }
}

impl ViewerOps for Connection {
    fn open(&mut self, path: &Path, analysis: Option<&str>, tag: Option<&str>) -> Result<()> {
        match self {
            Connection::Ipc(c) => ViewerOps::open(c, path, analysis, tag),
            Connection::Embedded(v) => v.open(path, analysis, tag),
        }
    }
    fn import(&mut self, path: &Path, analysis: Option<&str>, tag: Option<&str>) -> Result<()> {
        match self {
            Connection::Ipc(c) => ViewerOps::import(c, path, analysis, tag),
            Connection::Embedded(v) => v.import(path, analysis, tag),
        }
    }
    fn list_signals(
        &mut self,
        file: Option<&str>,
        filter: Option<&str>,
    ) -> Result<(Vec<Signal>, bool)> {
        match self {
            Connection::Ipc(c) => ViewerOps::list_signals(c, file, filter),
            Connection::Embedded(v) => v.list_signals(file, filter),
        }
    }
    fn show(&mut self, signals: &[String], pane: PaneSel) -> Result<Option<u64>> {
        match self {
            Connection::Ipc(c) => ViewerOps::show(c, signals, pane),
            Connection::Embedded(v) => v.show(signals, pane),
        }
    }
    fn add_trace(&mut self, trace: &Trace) -> Result<()> {
        match self {
            Connection::Ipc(c) => ViewerOps::add_trace(c, trace),
            Connection::Embedded(v) => v.add_trace(trace),
        }
    }
    fn remove_trace(&mut self, name: &str) -> Result<()> {
        match self {
            Connection::Ipc(c) => ViewerOps::remove_trace(c, name),
            Connection::Embedded(v) => v.remove_trace(name),
        }
    }
    fn screenshot(&mut self, path: &Path) -> Result<()> {
        match self {
            Connection::Ipc(c) => ViewerOps::screenshot(c, path),
            Connection::Embedded(v) => v.screenshot(path),
        }
    }
    fn wants_decimation(&self) -> bool {
        matches!(self, Connection::Ipc(_))
    }
    fn set_cursors(&mut self, a: Option<f64>, b: Option<f64>) -> Result<()> {
        match self {
            Connection::Ipc(c) => ViewerOps::set_cursors(c, a, b),
            Connection::Embedded(v) => v.set_cursors(a, b),
        }
    }
    fn cursors(&mut self) -> Result<Cursors> {
        match self {
            Connection::Ipc(c) => ViewerOps::cursors(c),
            Connection::Embedded(v) => v.cursors(),
        }
    }
    fn add_marker(&mut self, marker: &Marker) -> Result<()> {
        match self {
            Connection::Ipc(c) => ViewerOps::add_marker(c, marker),
            Connection::Embedded(v) => v.add_marker(marker),
        }
    }
    fn clear_markers(&mut self, group: Option<&str>) -> Result<u32> {
        match self {
            Connection::Ipc(c) => ViewerOps::clear_markers(c, group),
            Connection::Embedded(v) => v.clear_markers(group),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_is_embedded_when_built_in() {
        assert_eq!(
            choose_with(None, None, None, true).unwrap(),
            Backend::Embedded
        );
        assert_eq!(
            choose_with(None, None, None, false).unwrap(),
            Backend::Process
        );
    }

    #[test]
    fn override_beats_environment_beats_config() {
        let (e, p) = (Backend::Embedded, Backend::Process);
        assert_eq!(
            choose_with(Some(e), Some("process"), None, true).unwrap(),
            p
        );
        assert_eq!(
            choose_with(Some(p), Some("process"), Some(e), true).unwrap(),
            e
        );
        assert_eq!(choose_with(Some(p), Some(""), None, true).unwrap(), p);
        assert!(choose_with(None, Some("qt"), None, true).is_err());
    }

    #[test]
    fn embedded_asked_for_but_not_built_in_says_how() {
        let err = choose_with(Some(Backend::Embedded), None, None, false)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("UBERWAVE_SRC") && err.contains("--backend process"),
            "{err}"
        );
        // The process backend never needs it.
        assert_eq!(
            choose_with(Some(Backend::Process), None, None, false).unwrap(),
            Backend::Process
        );
    }

    #[test]
    fn names() {
        assert_eq!(Backend::parse(" Embedded "), Some(Backend::Embedded));
        assert_eq!(Backend::parse("process"), Some(Backend::Process));
        assert_eq!(Backend::parse("x"), None);
    }
}
