//! The real thing: hand-written declarations of `uberwave_shim.h` and the safe wrapper.
//! Every call is a plain C call with C values; lists come back as C structs the library
//! owns, copied into Rust values and freed at once. No JSON anywhere.

use std::ffi::{CStr, CString, c_char, c_double, c_int, c_void};
use std::marker::PhantomData;

use crate::{
    CursorListener, CursorValue, Cursors, Error, FileInfo, Marker, Options, Pane, Result,
    SignalInfo, SignalList, TraceStyle,
};

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

#[repr(C)]
struct UwFileInfo {
    tag: *const c_char,
    path: *const c_char,
    analyses: *const *const c_char,
    n_analyses: usize,
}

#[repr(C)]
struct UwFileList {
    files: *const UwFileInfo,
    n: usize,
}

#[repr(C)]
struct UwSignalInfo {
    name: *const c_char,
    file: *const c_char,
    unit: *const c_char,
    analysis: *const c_char,
    complex: i32,
}

#[repr(C)]
struct UwSignalList {
    signals: *const UwSignalInfo,
    n: usize,
    truncated: i32,
}

#[repr(C)]
struct UwCursorValue {
    signal: *const c_char,
    a: c_double,
    b: c_double,
}

#[repr(C)]
struct UwCursorState {
    a: c_double,
    b: c_double,
    values: *const UwCursorValue,
    n: usize,
}

#[repr(C)]
struct UwTrace {
    struct_size: u32,
    name: *const c_char,
    x: *const c_double,
    y: *const c_double,
    n: usize,
    unit: *const c_char,
    x_unit: *const c_char,
    pane: i32,
    x_log: i32,
    fold_period: c_double,
    fold_offset: c_double,
}

#[repr(C)]
struct UwMarker {
    struct_size: u32,
    group: *const c_char,
    label: *const c_char,
    x: c_double,
    y: c_double,
    pane: i32,
    rgba: u32,
    x_unit: *const c_char,
}

type UwCursorFn = unsafe extern "C" fn(user: *mut c_void, a: c_double, b: c_double);

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
    fn uw_list_files(viewer: *mut UwViewer, out: *mut *mut UwFileList) -> c_int;
    fn uw_free_file_list(list: *mut UwFileList);
    fn uw_list_signals(
        viewer: *mut UwViewer,
        file: *const c_char,
        filter: *const c_char,
        out: *mut *mut UwSignalList,
    ) -> c_int;
    fn uw_free_signal_list(list: *mut UwSignalList);
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
    fn uw_get_cursors(viewer: *mut UwViewer, out: *mut *mut UwCursorState) -> c_int;
    fn uw_free_cursor_state(state: *mut UwCursorState);
    fn uw_set_cursor_callback(
        viewer: *mut UwViewer,
        f: Option<UwCursorFn>,
        user: *mut c_void,
    ) -> c_int;
    fn uw_set_range(viewer: *mut UwViewer, x0: c_double, x1: c_double) -> c_int;
    fn uw_fit(viewer: *mut UwViewer) -> c_int;
    fn uw_add_trace_ex(viewer: *mut UwViewer, trace: *const UwTrace) -> c_int;
    fn uw_remove_trace(viewer: *mut UwViewer, name: *const c_char) -> c_int;
    fn uw_add_marker(viewer: *mut UwViewer, marker: *const UwMarker) -> c_int;
    fn uw_clear_markers(viewer: *mut UwViewer, group: *const c_char, removed: *mut c_int) -> c_int;
    fn uw_screenshot(viewer: *mut UwViewer, path: *const c_char) -> c_int;
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

/// A string of a library-owned struct.
///
/// # Safety
/// `p` is null or a NUL-terminated string that lives while it is read.
unsafe fn text(p: *const c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: as documented.
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

/// A slice of a library-owned array (empty for null).
///
/// # Safety
/// `p` is null or points to `n` valid items that live while the slice is used.
unsafe fn items<'a, T>(p: *const T, n: usize) -> &'a [T] {
    if p.is_null() || n == 0 {
        return &[];
    }
    // SAFETY: as documented.
    unsafe { std::slice::from_raw_parts(p, n) }
}

fn finite(v: f64) -> Option<f64> {
    v.is_finite().then_some(v)
}

/// The listener's box, kept alive while the viewer may call it.
struct ListenerBox(CursorListener);

unsafe extern "C" fn trampoline(user: *mut c_void, a: c_double, b: c_double) {
    if user.is_null() {
        return;
    }
    // SAFETY: `user` is the ListenerBox the Viewer keeps until the callback is removed;
    // a panic must not cross into C.
    let listener = unsafe { &*(user as *const ListenerBox) };
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        (listener.0)(finite(a), finite(b))
    }));
}

/// A viewer window running on its own thread in this process. Closed (and the thread
/// joined) when dropped. Methods may be called from any thread that owns the value; the
/// type is `Send` but not `Sync`.
pub struct Viewer {
    raw: *mut UwViewer,
    listener: Option<Box<ListenerBox>>,
    _not_sync: PhantomData<std::cell::Cell<()>>,
}

// SAFETY: the C API is thread safe (every call is marshalled onto the viewer thread);
// the listener is Send + Sync.
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
            let why = unsafe { text(uw_start_error()) };
            return Err(Error(why));
        }
        Ok(Viewer {
            raw,
            listener: None,
            _not_sync: PhantomData,
        })
    }

    fn fail(&self) -> Error {
        // SAFETY: valid until the next uw_error call on this thread; copied at once.
        Error(unsafe { text(uw_error(self.raw)) })
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

    /// The files of the session.
    pub fn list_files(&self) -> Result<Vec<FileInfo>> {
        let mut out: *mut UwFileList = std::ptr::null_mut();
        // SAFETY: out is a valid location.
        self.check(unsafe { uw_list_files(self.raw, &mut out) })?;
        if out.is_null() {
            return Ok(Vec::new());
        }
        // SAFETY: the list and everything it points to live until uw_free_file_list.
        let files = unsafe {
            items((*out).files, (*out).n)
                .iter()
                .map(|f| FileInfo {
                    tag: text(f.tag),
                    path: text(f.path),
                    analyses: items(f.analyses, f.n_analyses)
                        .iter()
                        .map(|a| text(*a))
                        .collect(),
                })
                .collect()
        };
        // SAFETY: freed once, nothing of it is used afterwards.
        unsafe { uw_free_file_list(out) };
        Ok(files)
    }

    /// The signals of the file with this tag (`(external)`: the traces added with
    /// [`Viewer::add_trace`]) or of all, whose name contains `filter` (case-blind).
    pub fn list_signals(&self, file: Option<&str>, filter: Option<&str>) -> Result<SignalList> {
        let (f, g) = (opt_cstring(file)?, opt_cstring(filter)?);
        let mut out: *mut UwSignalList = std::ptr::null_mut();
        // SAFETY: as above.
        self.check(unsafe { uw_list_signals(self.raw, ptr(&f), ptr(&g), &mut out) })?;
        if out.is_null() {
            return Ok(SignalList::default());
        }
        // SAFETY: the list lives until uw_free_signal_list.
        let list = unsafe {
            SignalList {
                signals: items((*out).signals, (*out).n)
                    .iter()
                    .map(|s| SignalInfo {
                        name: text(s.name),
                        file: text(s.file),
                        unit: text(s.unit),
                        analysis: text(s.analysis),
                        complex: s.complex != 0,
                    })
                    .collect(),
                truncated: (*out).truncated != 0,
            }
        };
        // SAFETY: freed once.
        unsafe { uw_free_signal_list(out) };
        Ok(list)
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

    /// The cursors and the value of every plotted trace under them.
    pub fn cursors(&self) -> Result<Cursors> {
        let mut out: *mut UwCursorState = std::ptr::null_mut();
        // SAFETY: valid location.
        self.check(unsafe { uw_get_cursors(self.raw, &mut out) })?;
        if out.is_null() {
            return Ok(Cursors::default());
        }
        // SAFETY: the state lives until uw_free_cursor_state.
        let cursors = unsafe {
            Cursors {
                a: finite((*out).a),
                b: finite((*out).b),
                values: items((*out).values, (*out).n)
                    .iter()
                    .map(|v| CursorValue {
                        signal: text(v.signal),
                        a: finite(v.a),
                        b: finite(v.b),
                    })
                    .collect(),
            }
        };
        // SAFETY: freed once.
        unsafe { uw_free_cursor_state(out) };
        Ok(cursors)
    }

    /// Call `listener` when a cursor moves (`None`: no listener). The old listener is
    /// never called after this returns.
    pub fn set_cursor_listener(&mut self, listener: Option<CursorListener>) -> Result<()> {
        let new = listener.map(|l| Box::new(ListenerBox(l)));
        let user = new.as_ref().map_or(std::ptr::null_mut(), |b| {
            &**b as *const ListenerBox as *mut c_void
        });
        let f: Option<UwCursorFn> = new.as_ref().map(|_| trampoline as UwCursorFn);
        // SAFETY: `user` stays alive in self.listener until it is replaced, which only
        // happens after the library stopped calling the old one (this call returned).
        let code = unsafe { uw_set_cursor_callback(self.raw, f, user) };
        if code != 0 {
            return Err(self.fail());
        }
        self.listener = new;
        Ok(())
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
        self.add_trace_styled(name, x, y, unit, x_unit, pane, TraceStyle::default())
    }

    /// [`Viewer::add_trace`] with a log X axis or folded into an eye diagram.
    #[allow(clippy::too_many_arguments)]
    pub fn add_trace_styled(
        &self,
        name: &str,
        x: &[f64],
        y: &[f64],
        unit: Option<&str>,
        x_unit: Option<&str>,
        pane: Pane,
        style: TraceStyle,
    ) -> Result<()> {
        if x.len() != y.len() {
            return Err(Error(format!(
                "trace '{name}': {} x values but {} y values",
                x.len(),
                y.len()
            )));
        }
        let (n, u, xu) = (cstring(name)?, opt_cstring(unit)?, opt_cstring(x_unit)?);
        let (fold_period, fold_offset) = style.fold.unwrap_or((0.0, 0.0));
        let trace = UwTrace {
            struct_size: size_of::<UwTrace>() as u32,
            name: n.as_ptr(),
            x: x.as_ptr(),
            y: y.as_ptr(),
            n: x.len(),
            unit: ptr(&u),
            x_unit: ptr(&xu),
            pane: pane.code(),
            x_log: style.x_log as i32,
            fold_period,
            fold_offset,
        };
        // SAFETY: the slices and strings live through the call; the library copies the data.
        self.check(unsafe { uw_add_trace_ex(self.raw, &trace) })
    }

    pub fn remove_trace(&self, name: &str) -> Result<()> {
        let n = cstring(name)?;
        // SAFETY: as above.
        self.check(unsafe { uw_remove_trace(self.raw, n.as_ptr()) })
    }

    /// Put a marker on the plot.
    pub fn add_marker(&self, marker: &Marker) -> Result<()> {
        let (g, l, u) = (
            cstring(&marker.group)?,
            cstring(&marker.label)?,
            cstring(&marker.x_unit)?,
        );
        let raw = UwMarker {
            struct_size: size_of::<UwMarker>() as u32,
            group: g.as_ptr(),
            label: l.as_ptr(),
            x: marker.x,
            y: marker.y.unwrap_or(f64::NAN),
            pane: marker.pane.map_or(-1, |p| p.min(i32::MAX as u32) as i32),
            rgba: marker.rgba,
            x_unit: u.as_ptr(),
        };
        // SAFETY: the strings live through the call.
        self.check(unsafe { uw_add_marker(self.raw, &raw) })
    }

    /// Remove the markers of a group (`None`: all); how many went.
    pub fn clear_markers(&self, group: Option<&str>) -> Result<u32> {
        let g = opt_cstring(group)?;
        let mut removed: c_int = 0;
        // SAFETY: as above.
        self.check(unsafe { uw_clear_markers(self.raw, ptr(&g), &mut removed) })?;
        Ok(removed.max(0) as u32)
    }

    /// Save the window as a PNG (absolute path).
    pub fn screenshot(&self, path: &str) -> Result<()> {
        let p = cstring(path)?;
        // SAFETY: as above.
        self.check(unsafe { uw_screenshot(self.raw, p.as_ptr()) })
    }
}

impl Drop for Viewer {
    fn drop(&mut self) {
        // SAFETY: closes the window, joins the thread and frees the handle once; the
        // listener box outlives every call of it (it is dropped after this).
        unsafe { uw_viewer_destroy(self.raw) };
        self.listener = None;
    }
}
