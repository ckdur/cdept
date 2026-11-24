//! The real thing: hand-written declarations of `uberwave_shim.h` and the safe wrapper.

use std::ffi::{CStr, CString, c_char, c_double, c_int};
use std::marker::PhantomData;

use crate::{Cursors, Error, Options, Pane, Result};

#[repr(C)]
struct UwViewer {
    _private: [u8; 0],
}

#[repr(C)]
struct UwOptions {
    struct_size: u32,
    title: *const c_char,
    width: i32,
    height: i32,
    frame_cap_fps: i32,
    headless: i32,
    platform: *const c_char,
    write_ini: i32,
}

unsafe extern "C" {
    fn uw_version() -> *const c_char;
    fn uw_viewer_start(options: *const UwOptions) -> *mut UwViewer;
    fn uw_start_error() -> *const c_char;
    fn uw_error(viewer: *mut UwViewer) -> *const c_char;
    fn uw_viewer_is_running(viewer: *mut UwViewer) -> c_int;
    fn uw_viewer_wait(viewer: *mut UwViewer) -> c_int;
    fn uw_viewer_close(viewer: *mut UwViewer);
    fn uw_viewer_destroy(viewer: *mut UwViewer);
    fn uw_open(
        viewer: *mut UwViewer,
        path: *const c_char,
        analysis: *const c_char,
        tag: *const c_char,
    ) -> c_int;
    fn uw_import(
        viewer: *mut UwViewer,
        path: *const c_char,
        analysis: *const c_char,
        tag: *const c_char,
    ) -> c_int;
    fn uw_list_files(viewer: *mut UwViewer, json_out: *mut *mut c_char) -> c_int;
    fn uw_list_signals(
        viewer: *mut UwViewer,
        file: *const c_char,
        filter: *const c_char,
        json_out: *mut *mut c_char,
    ) -> c_int;
    fn uw_set_analysis(viewer: *mut UwViewer, name: *const c_char, file: *const c_char) -> c_int;
    fn uw_show(
        viewer: *mut UwViewer,
        signals: *const *const c_char,
        n: usize,
        pane_in: c_int,
        pane_out: *mut c_int,
    ) -> c_int;
    fn uw_hide(viewer: *mut UwViewer, signals: *const *const c_char, n: usize) -> c_int;
    fn uw_set_cursors(viewer: *mut UwViewer, a: *const c_double, b: *const c_double) -> c_int;
    fn uw_clear_cursors(viewer: *mut UwViewer) -> c_int;
    fn uw_get_cursors(
        viewer: *mut UwViewer,
        a: *mut c_double,
        b: *mut c_double,
        values_json: *mut *mut c_char,
    ) -> c_int;
    fn uw_set_range(viewer: *mut UwViewer, x0: c_double, x1: c_double) -> c_int;
    fn uw_fit(viewer: *mut UwViewer) -> c_int;
    fn uw_add_trace(
        viewer: *mut UwViewer,
        name: *const c_char,
        x: *const c_double,
        y: *const c_double,
        n: usize,
        unit: *const c_char,
        x_unit: *const c_char,
        pane: c_int,
    ) -> c_int;
    fn uw_remove_trace(viewer: *mut UwViewer, name: *const c_char) -> c_int;
    fn uw_screenshot(viewer: *mut UwViewer, path: *const c_char) -> c_int;
    fn uw_command(
        viewer: *mut UwViewer,
        request_json: *const c_char,
        reply_out: *mut *mut c_char,
    ) -> c_int;
    fn uw_free_string(text: *mut c_char);
}

pub(crate) fn version() -> String {
    // SAFETY: a static NUL-terminated string.
    unsafe { CStr::from_ptr(uw_version()) }
        .to_string_lossy()
        .into_owned()
}

fn cstring(text: &str) -> Result<CString> {
    CString::new(text).map_err(|_| Error(format!("'{}' contains a NUL byte", text.escape_debug())))
}

fn opt_cstring(text: Option<&str>) -> Result<Option<CString>> {
    text.map(cstring).transpose()
}

fn ptr(text: &Option<CString>) -> *const c_char {
    text.as_ref().map_or(std::ptr::null(), |c| c.as_ptr())
}

/// Text the library allocated; freed with `uw_free_string`.
fn take(text: *mut c_char) -> String {
    if text.is_null() {
        return String::new();
    }
    // SAFETY: a malloc'ed NUL-terminated string from the shim, freed once.
    let owned = unsafe { CStr::from_ptr(text) }
        .to_string_lossy()
        .into_owned();
    unsafe { uw_free_string(text) };
    owned
}

/// A viewer window running on its own thread in this process. Closed (and the thread
/// joined) when dropped. Methods may be called from any thread that owns the value; the
/// type is `Send` but not `Sync`.
pub struct Viewer {
    raw: *mut UwViewer,
    _not_sync: PhantomData<std::cell::Cell<()>>,
}

// SAFETY: the C API is thread safe (every call is marshalled onto the viewer thread).
unsafe impl Send for Viewer {}

impl Viewer {
    /// Open the window; returns when it exists. Only one window can exist per process at
    /// a time.
    pub fn start(options: &Options) -> Result<Viewer> {
        let title = opt_cstring(options.title.as_deref())?;
        let platform = opt_cstring(options.platform.as_deref())?;
        let raw_options = UwOptions {
            struct_size: size_of::<UwOptions>() as u32,
            title: ptr(&title),
            width: options.width,
            height: options.height,
            frame_cap_fps: options.frame_cap_fps,
            headless: options.headless as i32,
            platform: ptr(&platform),
            write_ini: 0,
        };
        // SAFETY: the options and the strings they point to live through the call.
        let raw = unsafe { uw_viewer_start(&raw_options) };
        if raw.is_null() {
            // SAFETY: a NUL-terminated string valid until the next call on this thread.
            let why = unsafe { CStr::from_ptr(uw_start_error()) }
                .to_string_lossy()
                .into_owned();
            return Err(Error(why));
        }
        Ok(Viewer {
            raw,
            _not_sync: PhantomData,
        })
    }

    fn fail(&self) -> Error {
        // SAFETY: valid until the next uw_error call on this thread; copied at once.
        Error(
            unsafe { CStr::from_ptr(uw_error(self.raw)) }
                .to_string_lossy()
                .into_owned(),
        )
    }

    fn check(&self, code: c_int) -> Result<()> {
        if code == 0 { Ok(()) } else { Err(self.fail()) }
    }

    /// The window is still open.
    pub fn is_running(&self) -> bool {
        // SAFETY: self.raw is a live handle.
        unsafe { uw_viewer_is_running(self.raw) != 0 }
    }

    /// Block until the window is closed.
    pub fn wait(&self) {
        // SAFETY: as above.
        unsafe { uw_viewer_wait(self.raw) };
    }

    /// Close the window and join the viewer thread.
    pub fn close(&self) {
        // SAFETY: as above.
        unsafe { uw_viewer_close(self.raw) };
    }

    pub fn open(&self, path: &str, analysis: Option<&str>, tag: Option<&str>) -> Result<()> {
        let (p, a, t) = (cstring(path)?, opt_cstring(analysis)?, opt_cstring(tag)?);
        // SAFETY: valid handle and strings for the call.
        self.check(unsafe { uw_open(self.raw, p.as_ptr(), ptr(&a), ptr(&t)) })
    }

    pub fn import(&self, path: &str, analysis: Option<&str>, tag: Option<&str>) -> Result<()> {
        let (p, a, t) = (cstring(path)?, opt_cstring(analysis)?, opt_cstring(tag)?);
        // SAFETY: as above.
        self.check(unsafe { uw_import(self.raw, p.as_ptr(), ptr(&a), ptr(&t)) })
    }

    /// The files of the session as JSON (`[{"tag","path","analyses"}]`).
    pub fn list_files(&self) -> Result<String> {
        let mut out: *mut c_char = std::ptr::null_mut();
        // SAFETY: out is a valid location.
        self.check(unsafe { uw_list_files(self.raw, &mut out) })?;
        Ok(take(out))
    }

    /// The signals as JSON (`[{"name","file","unit","complex","analysis"}]`), of the file
    /// with this tag (`(external)`: the traces added with [`Viewer::add_trace`]) or of all.
    pub fn list_signals(&self, file: Option<&str>, filter: Option<&str>) -> Result<String> {
        let (f, g) = (opt_cstring(file)?, opt_cstring(filter)?);
        let mut out: *mut c_char = std::ptr::null_mut();
        // SAFETY: as above.
        self.check(unsafe { uw_list_signals(self.raw, ptr(&f), ptr(&g), &mut out) })?;
        Ok(take(out))
    }

    pub fn set_analysis(&self, name: &str, file: Option<&str>) -> Result<()> {
        let (n, f) = (cstring(name)?, opt_cstring(file)?);
        // SAFETY: as above.
        self.check(unsafe { uw_set_analysis(self.raw, n.as_ptr(), ptr(&f)) })
    }

    /// Plot signals (`name` or `tag:name`); returns the pane they went to.
    pub fn show(&self, signals: &[String], pane: Pane) -> Result<u32> {
        let owned: Vec<CString> = signals.iter().map(|s| cstring(s)).collect::<Result<_>>()?;
        let ptrs: Vec<*const c_char> = owned.iter().map(|c| c.as_ptr()).collect();
        let mut out: c_int = 0;
        // SAFETY: ptrs point into owned, which lives through the call.
        self.check(unsafe { uw_show(self.raw, ptrs.as_ptr(), ptrs.len(), pane.code(), &mut out) })?;
        Ok(out.max(0) as u32)
    }

    pub fn hide(&self, signals: &[String]) -> Result<()> {
        let owned: Vec<CString> = signals.iter().map(|s| cstring(s)).collect::<Result<_>>()?;
        let ptrs: Vec<*const c_char> = owned.iter().map(|c| c.as_ptr()).collect();
        // SAFETY: as above.
        self.check(unsafe { uw_hide(self.raw, ptrs.as_ptr(), ptrs.len()) })
    }

    /// Place the cursors (X units); `None` leaves that one alone.
    pub fn set_cursors(&self, a: Option<f64>, b: Option<f64>) -> Result<()> {
        let pa = a.as_ref().map_or(std::ptr::null(), |v| v as *const f64);
        let pb = b.as_ref().map_or(std::ptr::null(), |v| v as *const f64);
        // SAFETY: the pointers are null or point to locals that live through the call.
        self.check(unsafe { uw_set_cursors(self.raw, pa, pb) })
    }

    pub fn clear_cursors(&self) -> Result<()> {
        // SAFETY: valid handle.
        self.check(unsafe { uw_clear_cursors(self.raw) })
    }

    pub fn cursors(&self) -> Result<Cursors> {
        let (mut a, mut b) = (f64::NAN, f64::NAN);
        let mut values: *mut c_char = std::ptr::null_mut();
        // SAFETY: valid locations.
        self.check(unsafe { uw_get_cursors(self.raw, &mut a, &mut b, &mut values) })?;
        Ok(Cursors {
            a: a.is_finite().then_some(a),
            b: b.is_finite().then_some(b),
            values: take(values),
        })
    }

    pub fn set_range(&self, x0: f64, x1: f64) -> Result<()> {
        // SAFETY: valid handle.
        self.check(unsafe { uw_set_range(self.raw, x0, x1) })
    }

    pub fn fit(&self) -> Result<()> {
        // SAFETY: valid handle.
        self.check(unsafe { uw_fit(self.raw) })
    }

    /// Add a trace of raw `f64` data (`x` finite and not decreasing, `y` may hold NaN for
    /// a gap). The data is copied once into the viewer. A trace of that name is replaced.
    pub fn add_trace(
        &self,
        name: &str,
        x: &[f64],
        y: &[f64],
        unit: Option<&str>,
        x_unit: Option<&str>,
        pane: Pane,
    ) -> Result<()> {
        if x.len() != y.len() {
            return Err(Error(format!(
                "trace '{name}': {} x values but {} y values",
                x.len(),
                y.len()
            )));
        }
        let (n, u, xu) = (cstring(name)?, opt_cstring(unit)?, opt_cstring(x_unit)?);
        // SAFETY: the slices and strings live through the call; the library copies the data.
        self.check(unsafe {
            uw_add_trace(
                self.raw,
                n.as_ptr(),
                x.as_ptr(),
                y.as_ptr(),
                x.len(),
                ptr(&u),
                ptr(&xu),
                pane.code(),
            )
        })
    }

    pub fn remove_trace(&self, name: &str) -> Result<()> {
        let n = cstring(name)?;
        // SAFETY: as above.
        self.check(unsafe { uw_remove_trace(self.raw, n.as_ptr()) })
    }

    /// Save the window as a PNG (absolute path).
    pub fn screenshot(&self, path: &str) -> Result<()> {
        let p = cstring(path)?;
        // SAFETY: as above.
        self.check(unsafe { uw_screenshot(self.raw, p.as_ptr()) })
    }

    /// Any protocol command (`{"cmd": ..., ...}` without `id`); the reply object as JSON.
    pub fn command(&self, request_json: &str) -> Result<String> {
        let r = cstring(request_json)?;
        let mut out: *mut c_char = std::ptr::null_mut();
        // SAFETY: as above.
        self.check(unsafe { uw_command(self.raw, r.as_ptr(), &mut out) })?;
        Ok(take(out))
    }
}

impl Drop for Viewer {
    fn drop(&mut self) {
        // SAFETY: closes the window, joins the thread and frees the handle once.
        unsafe { uw_viewer_destroy(self.raw) };
    }
}
