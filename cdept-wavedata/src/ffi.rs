//! Declarations of `uwdata.h` (written by hand, they are few) and the safe handle.

use std::ffi::{CString, c_char, c_int, c_void};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::ptr::NonNull;

use crate::{Error, Mode, OpenOptions, Phase, Prepared, QueryInfo, Result, Status, Text, Trace};

#[repr(C)]
struct UwdDataset {
    _private: [u8; 0],
}

type ProgressFn = extern "C" fn(user: *mut c_void, phase: c_int, fraction: f64) -> c_int;

#[repr(C)]
struct UwdOpenOptions {
    size: u32,
    plot: i32,
    cache_dir: *const c_char,
    flags: u32,
    progress: Option<ProgressFn>,
    user: *mut c_void,
}

#[repr(C)]
#[derive(Default)]
struct UwdQueryInfo {
    first_index: u64,
    last_index: u64,
    source_points: u64,
    bucket: u64,
    mode: i32,
    level: i32,
    ms: f64,
}

const OPEN_REBUILD: u32 = 1;
const OPEN_CACHE_DIR_ONLY: u32 = 2;
const API_VERSION: u32 = 1;

unsafe extern "C" {
    fn uwd_open_options_init(options: *mut UwdOpenOptions);
    fn uwd_api_version() -> u32;
    fn uwd_open(
        path: *const c_char,
        options: *const UwdOpenOptions,
        out: *mut *mut UwdDataset,
        error: *mut c_char,
        error_size: usize,
    ) -> c_int;
    fn uwd_close(dataset: *mut UwdDataset);
    fn uwd_signal_count(dataset: *mut UwdDataset) -> u32;
    fn uwd_point_count(dataset: *mut UwdDataset) -> u64;
    fn uwd_signal_name(
        dataset: *mut UwdDataset,
        signal: u32,
        buf: *mut c_char,
        size: usize,
    ) -> usize;
    fn uwd_signal_unit(
        dataset: *mut UwdDataset,
        signal: u32,
        buf: *mut c_char,
        size: usize,
    ) -> usize;
    fn uwd_text(dataset: *mut UwdDataset, key: c_int, buf: *mut c_char, size: usize) -> usize;
    fn uwd_x_log(dataset: *mut UwdDataset) -> c_int;
    fn uwd_x_range(dataset: *mut UwdDataset, first: *mut f64, last: *mut f64) -> c_int;
    fn uwd_converted(dataset: *mut UwdDataset) -> c_int;
    fn uwd_prepare(dataset: *mut UwdDataset, signals: *const u32, count: usize) -> c_int;
    fn uwd_prepare_status(dataset: *mut UwdDataset, signal: u32, progress: *mut f32) -> c_int;
    fn uwd_query(
        dataset: *mut UwdDataset,
        signal: u32,
        x_min: f64,
        x_max: f64,
        pixels: u32,
        xs: *mut f64,
        ys: *mut f64,
        capacity: usize,
        n: *mut usize,
        info: *mut UwdQueryInfo,
    ) -> c_int;
    fn uwd_value_at(
        dataset: *mut UwdDataset,
        signal: u32,
        x: f64,
        y: *mut f64,
        index: *mut u64,
    ) -> c_int;
    fn uwd_exact_minmax(
        dataset: *mut UwdDataset,
        signal: u32,
        x_min: f64,
        x_max: f64,
        min: *mut f64,
        max: *mut f64,
        count: *mut u64,
    ) -> c_int;
    fn uwd_samples(
        dataset: *mut UwdDataset,
        signal: u32,
        first: u64,
        count: usize,
        out: *mut f64,
    ) -> c_int;
    fn uwd_source_changed(dataset: *mut UwdDataset) -> c_int;
}

fn check(code: c_int, what: &str) -> Result<()> {
    match Status::from_code(code) {
        None => Ok(()),
        Some(status) => Err(Error {
            status,
            message: format!("{what}: {}", status_text(status)),
        }),
    }
}

fn status_text(status: Status) -> &'static str {
    match status {
        Status::Argument => "bad argument",
        Status::Io => "cannot read the file",
        Status::Format => "not a readable waveform file",
        Status::Cancelled => "cancelled",
        Status::NoMemory => "out of memory",
        Status::Buffer => "buffer too small",
        Status::Cache => "cannot write the cache",
        Status::Internal => "internal error",
        Status::Unavailable => "not built in",
    }
}

/// A text of the C side: asked once with a buffer, again when it was too small.
fn text_of(get: impl Fn(*mut c_char, usize) -> usize) -> String {
    let mut buf = vec![0u8; 256];
    let n = get(buf.as_mut_ptr().cast(), buf.len());
    if n >= buf.len() {
        buf = vec![0u8; n + 1];
        get(buf.as_mut_ptr().cast(), buf.len());
    }
    buf.truncate(n.min(buf.len()));
    String::from_utf8_lossy(&buf).into_owned()
}

/// The owner of a `uwd_dataset*`.
pub(crate) struct Handle(NonNull<UwdDataset>);

// The C side locks every call on a dataset; the handle is closed only on drop.
unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { uwd_close(self.0.as_ptr()) }
    }
}

struct Callback<'a> {
    f: &'a mut dyn FnMut(Phase, f64) -> bool,
}

extern "C" fn trampoline(user: *mut c_void, phase: c_int, fraction: f64) -> c_int {
    // A panic must not unwind into C++: it cancels instead.
    let go_on = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let cb = unsafe { &mut *user.cast::<Callback<'_>>() };
        (cb.f)(Phase::from_code(phase), fraction)
    }));
    match go_on {
        Ok(true) => 0,
        _ => 1,
    }
}

impl Handle {
    pub(crate) fn open(
        path: &Path,
        options: &OpenOptions,
        mut progress: impl FnMut(Phase, f64) -> bool,
    ) -> Result<Handle> {
        if unsafe { uwd_api_version() } != API_VERSION {
            return Err(Error {
                status: Status::Internal,
                message: "the data engine's interface has another version".into(),
            });
        }
        let arg = |what: &str| Error {
            status: Status::Argument,
            message: format!("{what} holds a NUL byte"),
        };
        let c_path = CString::new(path.as_os_str().as_bytes()).map_err(|_| arg("the path"))?;
        let c_cache = match &options.cache_dir {
            Some(d) => {
                Some(CString::new(d.as_os_str().as_bytes()).map_err(|_| arg("the cache folder"))?)
            }
            None => None,
        };
        let mut cb = Callback { f: &mut progress };
        let mut opt = UwdOpenOptions {
            size: 0,
            plot: 0,
            cache_dir: std::ptr::null(),
            flags: 0,
            progress: None,
            user: std::ptr::null_mut(),
        };
        unsafe { uwd_open_options_init(&mut opt) };
        opt.plot = i32::try_from(options.plot).unwrap_or(i32::MAX);
        opt.cache_dir = c_cache.as_ref().map_or(std::ptr::null(), |c| c.as_ptr());
        opt.flags = if options.rebuild { OPEN_REBUILD } else { 0 }
            | if options.cache_dir_only {
                OPEN_CACHE_DIR_ONLY
            } else {
                0
            };
        opt.progress = Some(trampoline);
        opt.user = (&mut cb as *mut Callback<'_>).cast();
        let mut out: *mut UwdDataset = std::ptr::null_mut();
        let mut error = vec![0u8; 2048];
        let code = unsafe {
            uwd_open(
                c_path.as_ptr(),
                &opt,
                &mut out,
                error.as_mut_ptr().cast(),
                error.len(),
            )
        };
        let end = error.iter().position(|b| *b == 0).unwrap_or(error.len());
        let message = String::from_utf8_lossy(&error[..end]).into_owned();
        match (Status::from_code(code), NonNull::new(out)) {
            (None, Some(ptr)) => Ok(Handle(ptr)),
            (status, _) => {
                let status = status.unwrap_or(Status::Internal);
                Err(Error {
                    status,
                    message: if message.is_empty() {
                        status_text(status).to_string()
                    } else {
                        message
                    },
                })
            }
        }
    }

    fn ptr(&self) -> *mut UwdDataset {
        self.0.as_ptr()
    }

    pub(crate) fn signal_count(&self) -> u32 {
        unsafe { uwd_signal_count(self.ptr()) }
    }

    pub(crate) fn point_count(&self) -> u64 {
        unsafe { uwd_point_count(self.ptr()) }
    }

    pub(crate) fn signal_name(&self, signal: u32) -> String {
        text_of(|buf, size| unsafe { uwd_signal_name(self.ptr(), signal, buf, size) })
    }

    pub(crate) fn signal_unit(&self, signal: u32) -> String {
        text_of(|buf, size| unsafe { uwd_signal_unit(self.ptr(), signal, buf, size) })
    }

    pub(crate) fn text(&self, key: Text) -> String {
        text_of(|buf, size| unsafe { uwd_text(self.ptr(), key as c_int, buf, size) })
    }

    pub(crate) fn x_log(&self) -> bool {
        unsafe { uwd_x_log(self.ptr()) != 0 }
    }

    pub(crate) fn converted(&self) -> bool {
        unsafe { uwd_converted(self.ptr()) != 0 }
    }

    pub(crate) fn x_range(&self) -> Result<(f64, f64)> {
        let (mut a, mut b) = (0.0, 0.0);
        check(
            unsafe { uwd_x_range(self.ptr(), &mut a, &mut b) },
            "x range",
        )?;
        Ok((a, b))
    }

    pub(crate) fn prepare(&self, signals: &[u32]) -> Result<()> {
        check(
            unsafe { uwd_prepare(self.ptr(), signals.as_ptr(), signals.len()) },
            "prepare",
        )
    }

    pub(crate) fn prepared(&self, signal: u32) -> Prepared {
        let mut p = 0.0f32;
        match unsafe { uwd_prepare_status(self.ptr(), signal, &mut p) } {
            0 => Prepared::None,
            1 => Prepared::Building(p),
            2 => Prepared::Ready,
            _ => Prepared::Error,
        }
    }

    pub(crate) fn query(
        &self,
        signal: u32,
        x_min: f64,
        x_max: f64,
        pixels: u32,
        out: &mut Trace,
    ) -> Result<QueryInfo> {
        let mut cap = crate::query_capacity(pixels);
        loop {
            out.x.resize(cap, 0.0);
            out.y.resize(cap, 0.0);
            let mut n = 0usize;
            let mut info = UwdQueryInfo::default();
            let code = unsafe {
                uwd_query(
                    self.ptr(),
                    signal,
                    x_min,
                    x_max,
                    pixels,
                    out.x.as_mut_ptr(),
                    out.y.as_mut_ptr(),
                    cap,
                    &mut n,
                    &mut info,
                )
            };
            if Status::from_code(code) == Some(Status::Buffer) && n > cap {
                cap = n;
                continue;
            }
            out.x.truncate(n.min(cap));
            out.y.truncate(n.min(cap));
            if let Err(e) = check(code, "query") {
                out.x.clear();
                out.y.clear();
                return Err(e);
            }
            return Ok(QueryInfo {
                first_index: info.first_index,
                last_index: info.last_index,
                source_points: info.source_points,
                bucket: info.bucket,
                mode: match info.mode {
                    1 => Mode::MinMax,
                    2 => Mode::Pyramid,
                    _ => Mode::Direct,
                },
                level: info.level,
                ms: info.ms,
            });
        }
    }

    pub(crate) fn value_at(&self, signal: u32, x: f64) -> Result<(f64, u64)> {
        let (mut y, mut i) = (0.0, 0u64);
        check(
            unsafe { uwd_value_at(self.ptr(), signal, x, &mut y, &mut i) },
            "value",
        )?;
        Ok((y, i))
    }

    pub(crate) fn exact_minmax(
        &self,
        signal: u32,
        x_min: f64,
        x_max: f64,
    ) -> Result<Option<(f64, f64, u64)>> {
        let (mut lo, mut hi, mut n) = (0.0, 0.0, 0u64);
        check(
            unsafe { uwd_exact_minmax(self.ptr(), signal, x_min, x_max, &mut lo, &mut hi, &mut n) },
            "extremes",
        )?;
        Ok((n > 0).then_some((lo, hi, n)))
    }

    pub(crate) fn samples(&self, signal: u32, first: u64, out: &mut [f64]) -> Result<()> {
        check(
            unsafe { uwd_samples(self.ptr(), signal, first, out.len(), out.as_mut_ptr()) },
            "samples",
        )
    }

    pub(crate) fn source_changed(&self) -> bool {
        unsafe { uwd_source_changed(self.ptr()) == 1 }
    }
}
