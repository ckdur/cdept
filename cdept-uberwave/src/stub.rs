//! The stand-in when UberWave is not built in: same API, nothing runs.

use crate::{
    CursorListener, Cursors, Error, FileInfo, Marker, NOT_BUILT_IN, Options, Pane, Result,
    SignalList, TraceStyle,
};

/// A viewer window. Never exists in the stub: [`Viewer::start`] fails.
pub struct Viewer {
    // Not constructible, not Sync, like the real one.
    _never: std::marker::PhantomData<std::cell::Cell<()>>,
}

fn no<T>() -> Result<T> {
    Err(Error(NOT_BUILT_IN.into()))
}

#[allow(clippy::unused_self, clippy::missing_const_for_fn)]
impl Viewer {
    pub fn start(_options: &Options) -> Result<Viewer> {
        no()
    }
    pub fn is_running(&self) -> bool {
        false
    }
    pub fn wait(&self) {}
    pub fn close(&self) {}
    pub fn open(&self, _path: &str, _analysis: Option<&str>, _tag: Option<&str>) -> Result<()> {
        no()
    }
    pub fn import(&self, _path: &str, _analysis: Option<&str>, _tag: Option<&str>) -> Result<()> {
        no()
    }
    pub fn list_files(&self) -> Result<Vec<FileInfo>> {
        no()
    }
    pub fn list_signals(&self, _file: Option<&str>, _filter: Option<&str>) -> Result<SignalList> {
        no()
    }
    pub fn set_analysis(&self, _name: &str, _file: Option<&str>) -> Result<()> {
        no()
    }
    pub fn show(&self, _signals: &[String], _pane: Pane) -> Result<u32> {
        no()
    }
    pub fn hide(&self, _signals: &[String]) -> Result<()> {
        no()
    }
    pub fn set_cursors(&self, _a: Option<f64>, _b: Option<f64>) -> Result<()> {
        no()
    }
    pub fn clear_cursors(&self) -> Result<()> {
        no()
    }
    pub fn cursors(&self) -> Result<Cursors> {
        no()
    }
    pub fn set_cursor_listener(&mut self, _listener: Option<CursorListener>) -> Result<()> {
        no()
    }
    pub fn set_range(&self, _x0: f64, _x1: f64) -> Result<()> {
        no()
    }
    pub fn fit(&self) -> Result<()> {
        no()
    }
    pub fn add_trace(
        &self,
        _name: &str,
        _x: &[f64],
        _y: &[f64],
        _unit: Option<&str>,
        _x_unit: Option<&str>,
        _pane: Pane,
    ) -> Result<()> {
        no()
    }
    #[allow(clippy::too_many_arguments)]
    pub fn add_trace_styled(
        &self,
        _name: &str,
        _x: &[f64],
        _y: &[f64],
        _unit: Option<&str>,
        _x_unit: Option<&str>,
        _pane: Pane,
        _style: TraceStyle,
    ) -> Result<()> {
        no()
    }
    pub fn remove_trace(&self, _name: &str) -> Result<()> {
        no()
    }
    pub fn add_marker(&self, _marker: &Marker) -> Result<()> {
        no()
    }
    pub fn clear_markers(&self, _group: Option<&str>) -> Result<u32> {
        no()
    }
    pub fn screenshot(&self, _path: &str) -> Result<()> {
        no()
    }
}
