//! UberWave, the waveform viewer of Symbiotic EDA, as part of the cdept executable.
//!
//! The viewer (a C++ / Dear ImGui / GLFW window, `uberwave_embed` of the uberwave
//! repository) runs on its own thread inside this process; [`Viewer`] is the safe wrapper
//! of its C API (`wp5-viewer/shim/uberwave_shim.h`). Every call is marshalled onto the
//! viewer thread and may be made from any thread; the ones that wait for the viewer (a
//! file that loads, a screenshot) block the caller.
//!
//! Without the library (see `build.rs`: no source, no cmake, no OpenGL/GTK, no network)
//! this crate is a stub with the same API: [`available`] is false and [`Viewer::start`]
//! fails with a message that says how to build it in.

use std::fmt;

/// Which pane a signal or trace goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Pane {
    /// The active pane.
    #[default]
    Default,
    /// A new pane (the first one while it is empty).
    New,
    /// This pane, counting from 0.
    Index(u32),
}

impl Pane {
    #[cfg_attr(not(uberwave_embedded), allow(dead_code))]
    fn code(self) -> i32 {
        match self {
            Pane::Default => -1,
            Pane::New => -2,
            Pane::Index(i) => i.min(i32::MAX as u32) as i32,
        }
    }
}

/// A failed call: the viewer's message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// Window and loop settings.
#[derive(Debug, Clone, Default)]
pub struct Options {
    pub title: Option<String>,
    /// Pixels; 0: 80% of the monitor.
    pub width: i32,
    pub height: i32,
    /// Frame rate ceiling, 0: 60.
    pub frame_cap_fps: i32,
    /// A hidden window: rendering and screenshots work, nothing is shown.
    pub headless: bool,
    /// The GLFW backend: `x11` or `wayland`; `None`: it chooses (Wayland when there is one).
    pub platform: Option<String>,
}

/// One file of the viewer's session.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileInfo {
    pub tag: String,
    pub path: String,
    pub analyses: Vec<String>,
}

/// One signal the viewer has: of a file (`file` = its tag) or a host trace (`file` =
/// `(external)`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SignalInfo {
    pub name: String,
    pub file: String,
    pub unit: String,
    pub analysis: String,
    pub complex: bool,
}

/// The signals that matched, and whether there were more than the list holds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SignalList {
    pub signals: Vec<SignalInfo>,
    pub truncated: bool,
}

/// The value of one plotted trace under the cursors (`None`: that cursor is off or
/// outside the trace).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CursorValue {
    /// `name` when one file is open, else `tag:name`; host traces by their name.
    pub signal: String,
    pub a: Option<f64>,
    pub b: Option<f64>,
}

/// The cursors of the viewer.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Cursors {
    pub a: Option<f64>,
    pub b: Option<f64>,
    pub values: Vec<CursorValue>,
}

/// How a host trace is drawn.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TraceStyle {
    /// X on a log scale (a Bode plot), when no file trace in the pane decides.
    pub x_log: bool,
    /// An eye diagram: every `period` from `offset` on is drawn on top of the others.
    pub fold: Option<(f64, f64)>,
}

/// A marker of CDePT on the plot: a measured instant (`y` = `None`: a dashed vertical
/// line) or point.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Marker {
    /// Markers of a group are cleared together.
    pub group: String,
    pub label: String,
    pub x: f64,
    pub y: Option<f64>,
    /// The X unit (`s`, `Hz`): drawn only in panes of that X unit; empty: every pane.
    pub x_unit: String,
    /// `None`: every pane.
    pub pane: Option<u32>,
    /// `0xRRGGBBAA`, 0: the default colour.
    pub rgba: u32,
}

/// What is called when a cursor moves: `(a, b)` in X units, `None` when off. It runs
/// on the viewer's thread and must return quickly; it must not call the viewer.
pub type CursorListener = Box<dyn Fn(Option<f64>, Option<f64>) + Send + Sync + 'static>;

/// UberWave is built into this executable (not the stub).
pub const fn available() -> bool {
    cfg!(uberwave_embedded)
}

/// Why [`Viewer::start`] fails in the stub, and what to do about it.
pub const NOT_BUILT_IN: &str =
    "UberWave was not built into this cdept: set UBERWAVE_SRC and rebuild";

#[cfg(uberwave_embedded)]
mod real;
#[cfg(uberwave_embedded)]
pub use real::Viewer;

#[cfg(not(uberwave_embedded))]
mod stub;
#[cfg(not(uberwave_embedded))]
pub use stub::Viewer;

/// The version line of the viewer built in (`UberWaveViewer 0.1.0 ipc 1`).
pub fn version() -> Option<String> {
    #[cfg(uberwave_embedded)]
    {
        Some(real::version())
    }
    #[cfg(not(uberwave_embedded))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stub_says_how_to_build_it_in() {
        if available() {
            return;
        }
        let err = Viewer::start(&Options::default()).err().unwrap();
        assert!(err.0.contains("UBERWAVE_SRC"), "{err}");
        assert_eq!(version(), None);
    }

    #[test]
    fn pane_codes() {
        assert_eq!(Pane::Default.code(), -1);
        assert_eq!(Pane::New.code(), -2);
        assert_eq!(Pane::Index(3).code(), 3);
    }
}
