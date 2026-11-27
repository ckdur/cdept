//! Waveform data for CDePT's own viewer: UberWave's data engine (`wp3-data-engine` of
//! the uberwave repository) through its C interface `uwdata.h`.
//!
//! The engine reads ngspice and Xyce raw files (binary and ascii) and `.uwraw` files,
//! keeps a `.uwraw` cache beside a raw file (or in a cache folder when that folder is not
//! writable), and answers queries decimated to a pixel width with the minimum and
//! maximum of every bucket kept, so a one-sample spike is never lost. It draws nothing:
//! the viewer is CDePT's (`cdept-gui`, `views/wave`).
//!
//! A [`Dataset`] is one open file. Any number may be open at once, also on the same
//! file; a dataset may be shared between threads (the C side serialises its calls).
//! [`Dataset::open`] blocks (it may convert a large file): call it from a worker thread,
//! it reports progress and can be cancelled.
//!
//! Without the engine (see `build.rs`) the crate is a stub with the same API:
//! [`available`] is false and [`Dataset::open`] fails with a message.
//!
//! [`Series`] is a waveform held in memory (a calculator result) with the same queries,
//! answered in Rust (also in the stub build).

use std::fmt;
use std::path::{Path, PathBuf};

#[cfg(wavedata_engine)]
mod ffi;
mod series;

pub use series::Series;

/// Whether the engine is built in.
pub fn available() -> bool {
    cfg!(wavedata_engine)
}

/// What failed, as the C interface reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Argument,
    Io,
    Format,
    Cancelled,
    NoMemory,
    Buffer,
    Cache,
    Internal,
    /// The engine is not built in.
    Unavailable,
}

impl Status {
    #[cfg_attr(not(wavedata_engine), allow(dead_code))]
    fn from_code(code: i32) -> Option<Status> {
        Some(match code {
            0 => return None,
            1 => Status::Argument,
            2 => Status::Io,
            3 => Status::Format,
            4 => Status::Cancelled,
            5 => Status::NoMemory,
            6 => Status::Buffer,
            7 => Status::Cache,
            _ => Status::Internal,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub status: Status,
    pub message: String,
}

impl Error {
    #[cfg_attr(wavedata_engine, allow(dead_code))]
    fn unavailable() -> Error {
        Error {
            status: Status::Unavailable,
            message: "the waveform data engine is not built into this cdept (it needs the uberwave source, \
                      cmake and a C++ compiler when cdept is built)"
                .into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// How to open a file.
#[derive(Debug, Clone, Default)]
pub struct OpenOptions {
    /// The plot of a raw file with several, 0 = the first.
    pub plot: u32,
    /// Where caches go when the folder of the file is not writable.
    pub cache_dir: Option<PathBuf>,
    /// Convert again even when a fresh cache exists.
    pub rebuild: bool,
    /// Never write the cache beside the file (only into `cache_dir`).
    pub cache_dir_only: bool,
}

/// What [`Dataset::open`] is doing, for its progress callback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Header,
    /// Another reader writes this file's cache: waiting for it.
    Wait,
    /// Writing the `.uwraw` cache.
    Convert,
    Open,
}

impl Phase {
    #[cfg_attr(not(wavedata_engine), allow(dead_code))]
    fn from_code(code: i32) -> Phase {
        match code {
            1 => Phase::Wait,
            2 => Phase::Convert,
            3 => Phase::Open,
            _ => Phase::Header,
        }
    }
}

/// Texts of a dataset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Text {
    Title = 0,
    PlotName = 1,
    Analysis = 2,
    XName = 3,
    XUnit = 4,
    Source = 5,
    Cache = 6,
    Reader = 7,
}

/// The min/max pyramid of a signal.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Prepared {
    None,
    Building(f32),
    Ready,
    Error,
}

/// How a query was answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Every sample of the range.
    #[default]
    Direct,
    /// Min/max per bucket read from the file.
    MinMax,
    /// Min/max per bucket from the pyramid.
    Pyramid,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct QueryInfo {
    pub first_index: u64,
    pub last_index: u64,
    pub source_points: u64,
    pub bucket: u64,
    pub mode: Mode,
    pub level: i32,
    pub ms: f64,
}

/// The points of one query, reused from query to query.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Trace {
    pub x: Vec<f64>,
    pub y: Vec<f64>,
}

/// The most points a query returns for `pixels` columns.
pub fn query_capacity(pixels: u32) -> usize {
    let p = pixels.clamp(1, 65536) as usize;
    (9 * p + 64).max(128)
}

/// One open waveform file.
pub struct Dataset {
    #[cfg(wavedata_engine)]
    inner: ffi::Handle,
    #[cfg(not(wavedata_engine))]
    _never: std::convert::Infallible,
}

impl fmt::Debug for Dataset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Dataset")
            .field("source", &self.text(Text::Source))
            .finish()
    }
}

#[cfg(wavedata_engine)]
impl Dataset {
    /// Open a raw or `.uwraw` file (see the module documentation). `progress` is called
    /// with the phase and a fraction (0..1); returning false cancels.
    pub fn open(
        path: &Path,
        options: &OpenOptions,
        progress: impl FnMut(Phase, f64) -> bool,
    ) -> Result<Dataset> {
        ffi::Handle::open(path, options, progress).map(|inner| Dataset { inner })
    }

    pub fn signal_count(&self) -> u32 {
        self.inner.signal_count()
    }

    pub fn point_count(&self) -> u64 {
        self.inner.point_count()
    }

    /// Signal 0 is the x axis.
    pub fn signal_name(&self, signal: u32) -> String {
        self.inner.signal_name(signal)
    }

    pub fn signal_unit(&self, signal: u32) -> String {
        self.inner.signal_unit(signal)
    }

    pub fn text(&self, key: Text) -> String {
        self.inner.text(key)
    }

    pub fn x_log(&self) -> bool {
        self.inner.x_log()
    }

    /// Whether opening it converted the source (no fresh cache was there).
    pub fn converted(&self) -> bool {
        self.inner.converted()
    }

    pub fn x_range(&self) -> Result<(f64, f64)> {
        self.inner.x_range()
    }

    /// Build the pyramids of these signals in the background (others are dropped).
    pub fn prepare(&self, signals: &[u32]) -> Result<()> {
        self.inner.prepare(signals)
    }

    pub fn prepared(&self, signal: u32) -> Prepared {
        self.inner.prepared(signal)
    }

    /// The samples of `signal` over `x_min..x_max`, reduced to about `pixels` columns with
    /// every bucket's minimum and maximum kept, into `out`.
    pub fn query(
        &self,
        signal: u32,
        x_min: f64,
        x_max: f64,
        pixels: u32,
        out: &mut Trace,
    ) -> Result<QueryInfo> {
        self.inner.query(signal, x_min, x_max, pixels, out)
    }

    /// The value at `x`, interpolated linearly, and the sample at or before `x`.
    pub fn value_at(&self, signal: u32, x: f64) -> Result<(f64, u64)> {
        self.inner.value_at(signal, x)
    }

    /// Exact minimum and maximum over the samples with `x_min <= x <= x_max` (reads them
    /// all) and how many there were; `None` when there were none.
    pub fn exact_minmax(
        &self,
        signal: u32,
        x_min: f64,
        x_max: f64,
    ) -> Result<Option<(f64, f64, u64)>> {
        self.inner.exact_minmax(signal, x_min, x_max)
    }

    pub fn samples(&self, signal: u32, first: u64, out: &mut [f64]) -> Result<()> {
        self.inner.samples(signal, first, out)
    }

    /// The source file is not the one that was read any more (rewritten or gone).
    pub fn source_changed(&self) -> bool {
        self.inner.source_changed()
    }
}

// The real dataset closes its file on drop; the stub has the same shape.
#[cfg(not(wavedata_engine))]
impl Drop for Dataset {
    fn drop(&mut self) {}
}

#[cfg(not(wavedata_engine))]
impl Dataset {
    pub fn open(
        _path: &Path,
        _options: &OpenOptions,
        _progress: impl FnMut(Phase, f64) -> bool,
    ) -> Result<Dataset> {
        Err(Error::unavailable())
    }

    pub fn signal_count(&self) -> u32 {
        match self._never {}
    }

    pub fn point_count(&self) -> u64 {
        match self._never {}
    }

    pub fn signal_name(&self, _signal: u32) -> String {
        match self._never {}
    }

    pub fn signal_unit(&self, _signal: u32) -> String {
        match self._never {}
    }

    pub fn text(&self, _key: Text) -> String {
        match self._never {}
    }

    pub fn x_log(&self) -> bool {
        match self._never {}
    }

    pub fn converted(&self) -> bool {
        match self._never {}
    }

    pub fn x_range(&self) -> Result<(f64, f64)> {
        match self._never {}
    }

    pub fn prepare(&self, _signals: &[u32]) -> Result<()> {
        match self._never {}
    }

    pub fn prepared(&self, _signal: u32) -> Prepared {
        match self._never {}
    }

    pub fn query(
        &self,
        _signal: u32,
        _x_min: f64,
        _x_max: f64,
        _pixels: u32,
        _out: &mut Trace,
    ) -> Result<QueryInfo> {
        match self._never {}
    }

    pub fn value_at(&self, _signal: u32, _x: f64) -> Result<(f64, u64)> {
        match self._never {}
    }

    pub fn exact_minmax(
        &self,
        _signal: u32,
        _x_min: f64,
        _x_max: f64,
    ) -> Result<Option<(f64, f64, u64)>> {
        match self._never {}
    }

    pub fn samples(&self, _signal: u32, _first: u64, _out: &mut [f64]) -> Result<()> {
        match self._never {}
    }

    pub fn source_changed(&self) -> bool {
        match self._never {}
    }
}
