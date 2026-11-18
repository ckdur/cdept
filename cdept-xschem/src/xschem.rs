//! Safe wrapper around the C side (`shim/cdx.h`: `engine.c`, `display.c` and xschem
//! itself).

use std::cell::RefCell;
use std::ffi::{CStr, CString, c_char, c_int, c_uchar, c_ulong};
use std::marker::PhantomData;
use std::os::fd::RawFd;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};

use libcdept::xschem::frame::Rect;
use libcdept::xschem::input::Input;

unsafe extern "C" {
    fn cdx_init(argc: c_int, argv: *mut *mut c_char, display: c_int) -> c_int;
    fn cdx_eval(script: *const c_char) -> c_int;
    fn cdx_result() -> *const c_char;
    fn cdx_xschem_version() -> *const c_char;
    fn cdx_watch_fd(fd: c_int, readable: extern "C" fn());
    fn cdx_when_idle(call: extern "C" fn());
    fn cdx_do_one_event(wait: c_int) -> c_int;
    fn cdx_set_ui_sink(sink: extern "C" fn(*const c_char));
    fn cdx_display_resize(width: c_int, height: c_int);
    fn cdx_display_event(
        kind: c_int,
        x: c_int,
        y: c_int,
        keysym: c_ulong,
        button: c_int,
        state: c_int,
    );
    fn cdx_display_size(width: *mut c_int, height: *mut c_int) -> c_int;
    fn cdx_display_take_damage(
        x: *mut c_int,
        y: *mut c_int,
        width: *mut c_int,
        height: *mut c_int,
    ) -> c_int;
    fn cdx_display_take_damage_list(rects: *mut c_int, max: c_int) -> c_int;
    fn cdx_display_read(
        dst: *mut c_uchar,
        dst_stride: c_int,
        x: c_int,
        y: c_int,
        width: c_int,
        height: c_int,
    );
    fn cdx_preview_list() -> c_int;
    fn cdx_preview_info(
        index: c_int,
        width: *mut c_int,
        height: *mut c_int,
        changed: *mut c_int,
    ) -> *const c_char;
    fn cdx_preview_read(index: c_int, dst: *mut c_uchar, dst_stride: c_int);
}

/// A picture pane of a dialog: a Tk frame whose window xschem's C code draws a
/// schematic or symbol into (its `preview_window`). See [`Xschem::previews`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pane {
    index: c_int,
    /// Tk path of the frame (or top-level window) the picture belongs to.
    pub path: String,
    pub width: u32,
    pub height: u32,
    /// The pixels changed since the previous [`Xschem::previews`] call.
    pub changed: bool,
}

/// xschem keeps its state in C globals: there can be one per process.
static STARTED: AtomicBool = AtomicBool::new(false);

type Callback = Rc<dyn Fn()>;
type UiSink = Rc<dyn Fn(&str)>;

thread_local! {
    static READABLE: RefCell<Option<Callback>> = const { RefCell::new(None) };
    static IDLE: RefCell<Option<Callback>> = const { RefCell::new(None) };
    static UI_SINK: RefCell<Option<UiSink>> = const { RefCell::new(None) };
}

// Both callbacks call back into Tcl, where a nested event loop (xschem waiting for a
// dialog) may get here again: they are shared, never borrowed while running.

extern "C" fn readable_trampoline() {
    let callback = READABLE.with(|slot| slot.borrow().clone());
    if let Some(callback) = callback {
        callback();
    }
}

extern "C" fn idle_trampoline() {
    let callback = IDLE.with(|slot| slot.borrow().clone());
    if let Some(callback) = callback {
        callback();
    }
}

extern "C" fn ui_trampoline(json: *const c_char) {
    // SAFETY: Tcl hands a valid NUL-terminated string that lives through this call.
    let json = unsafe { CStr::from_ptr(json) }.to_string_lossy();
    let sink = UI_SINK.with(|slot| slot.borrow().clone());
    if let Some(sink) = sink {
        sink(&json);
    }
}

/// The xschem engine of this process. Not `Send`: xschem and its Tcl interpreter must
/// stay on the thread that started them.
pub struct Xschem {
    _one_thread: PhantomData<*mut ()>,
}

impl Xschem {
    /// Start xschem with its own command line options (`args[0]` is the program name).
    /// With `display`, xschem draws into the drawing window (see [`Xschem::resize`]).
    pub fn start(args: &[String], display: bool) -> Result<Xschem, String> {
        if STARTED.swap(true, Ordering::SeqCst) {
            return Err("xschem was already started in this process".into());
        }
        let owned: Vec<CString> = args
            .iter()
            .map(|arg| CString::new(arg.as_str()))
            .collect::<Result<_, _>>()
            .map_err(|_| "an argument contains a NUL byte".to_string())?;
        // xschem keeps pointers into argv for its whole life: leak the strings.
        let mut argv: Vec<*mut c_char> = owned.into_iter().map(CString::into_raw).collect();
        argv.push(std::ptr::null_mut());
        let argv = Box::leak(argv.into_boxed_slice());

        let xschem = Xschem {
            _one_thread: PhantomData,
        };
        // SAFETY: argv is a NULL-terminated array of valid C strings that live forever,
        // and STARTED guarantees this runs once.
        let status = unsafe {
            cdx_init(
                (argv.len() - 1) as c_int,
                argv.as_mut_ptr(),
                c_int::from(display),
            )
        };
        if status != 0 {
            return Err(xschem.result());
        }
        Ok(xschem)
    }

    /// Evaluate a Tcl script at global level: `Ok(result)` or `Err(message)`.
    pub fn eval(&self, script: &str) -> Result<String, String> {
        let script =
            CString::new(script).map_err(|_| "the script contains a NUL byte".to_string())?;
        // SAFETY: the interpreter exists (we hold an Xschem) and we are on its thread.
        let status = unsafe { cdx_eval(script.as_ptr()) };
        let result = self.result();
        if status == 0 { Ok(result) } else { Err(result) }
    }

    fn result(&self) -> String {
        // SAFETY: Tcl returns a valid NUL-terminated string, copied before the next
        // call into Tcl.
        unsafe { CStr::from_ptr(cdx_result()) }
            .to_string_lossy()
            .into_owned()
    }

    /// Call `readable` from the Tcl event loop whenever `fd` has data, also from event
    /// loops nested inside a call into xschem. One watch per process: a second call
    /// replaces the callback but must name the same `fd`.
    pub fn watch(&self, fd: RawFd, readable: impl Fn() + 'static) {
        READABLE.with(|slot| *slot.borrow_mut() = Some(Rc::new(readable)));
        // SAFETY: on the interpreter's thread; the trampoline is a plain function.
        unsafe { cdx_watch_fd(fd, readable_trampoline) }
    }

    /// The callback [`when_idle`](Self::when_idle) runs.
    pub fn on_idle(&self, callback: impl Fn() + 'static) {
        IDLE.with(|slot| *slot.borrow_mut() = Some(Rc::new(callback)));
    }

    /// Run the [`on_idle`](Self::on_idle) callback once, from the Tcl event loop, when
    /// it has nothing else to do. Like the file watch this also happens in an event
    /// loop nested inside a call into xschem.
    pub fn when_idle(&self) {
        // SAFETY: on the interpreter's thread; the trampoline is a plain function.
        unsafe { cdx_when_idle(idle_trampoline) }
    }

    /// Run one round of the Tcl event loop (timers, idle work, the watched file).
    /// `wait` blocks until there is something to do. Returns whether work was done.
    pub fn do_one_event(&self, wait: bool) -> bool {
        // SAFETY: as in `eval`.
        unsafe { cdx_do_one_event(c_int::from(wait)) != 0 }
    }

    /// Run the Tcl events that are due, without waiting.
    pub fn run_pending_events(&self) {
        while self.do_one_event(false) {}
    }

    /// Give the drawing window this size in pixels; the first call creates it.
    pub fn resize(&self, width: u32, height: u32) {
        // SAFETY: as in `eval`.
        unsafe { cdx_display_resize(width as c_int, height as c_int) }
    }

    /// Deliver pointer or keyboard input to the drawing window.
    pub fn input(&self, input: Input) {
        let e = input.to_x();
        // SAFETY: as in `eval`.
        unsafe {
            cdx_display_event(
                e.kind,
                e.x,
                e.y,
                c_ulong::from(e.keysym),
                e.button as c_int,
                e.state as c_int,
            )
        }
    }

    /// Size of the drawing window, once it exists.
    pub fn display_size(&self) -> Option<(u32, u32)> {
        let (mut width, mut height) = (0, 0);
        // SAFETY: both pointers are valid for the call.
        let exists = unsafe { cdx_display_size(&mut width, &mut height) } != 0;
        (exists && width > 0 && height > 0).then_some((width as u32, height as u32))
    }

    /// The rectangle of the drawing window that changed since the last call.
    pub fn take_damage(&self) -> Option<Rect> {
        let (mut x, mut y, mut width, mut height) = (0, 0, 0, 0);
        // SAFETY: all pointers are valid for the call.
        let any = unsafe { cdx_display_take_damage(&mut x, &mut y, &mut width, &mut height) } != 0;
        (any && x >= 0 && y >= 0 && width > 0 && height > 0)
            .then(|| Rect::new(x as u32, y as u32, width as u32, height as u32))
    }

    /// What changed in the drawing window since the last call, as a few rectangles
    /// that together cover it. Use this instead of [`Xschem::take_damage`] (either one
    /// forgets the changes): far-apart changes, like the two strips of a cross-hair
    /// cursor, stay small instead of becoming one rectangle around both.
    pub fn take_damage_rects(&self) -> Vec<Rect> {
        const MAX: usize = 8;
        let mut raw = [0 as c_int; 4 * MAX];
        // SAFETY: `raw` has room for MAX rectangles of four numbers.
        let count = unsafe { cdx_display_take_damage_list(raw.as_mut_ptr(), MAX as c_int) };
        raw.as_chunks::<4>()
            .0
            .iter()
            .take(count.clamp(0, MAX as c_int) as usize)
            .filter(|r| r[0] >= 0 && r[1] >= 0 && r[2] > 0 && r[3] > 0)
            .map(|r| Rect::new(r[0] as u32, r[1] as u32, r[2] as u32, r[3] as u32))
            .collect()
    }

    /// Copy `rect` of the drawing window into `frame`: rows of `stride` bytes, R G B A
    /// premultiplied, the frame having the size of the window. A rectangle reaching
    /// outside the window or the buffer is clipped.
    pub fn read_pixels(&self, frame: &mut [u8], stride: u32, rect: Rect) {
        let Some((width, height)) = self.display_size() else {
            return;
        };
        let rect = rect.clamp(width, height);
        if rect.is_empty() || stride < width * 4 {
            return;
        }
        let needed = (rect.y + rect.height - 1) as usize * stride as usize
            + (rect.x + rect.width) as usize * 4;
        if frame.len() < needed {
            return;
        }
        let offset = rect.y as usize * stride as usize + rect.x as usize * 4;
        // SAFETY: the destination range was checked against `frame`, and the rectangle
        // against the window.
        unsafe {
            cdx_display_read(
                frame.as_mut_ptr().add(offset),
                stride as c_int,
                rect.x as c_int,
                rect.y as c_int,
                rect.width as c_int,
                rect.height as c_int,
            )
        }
    }
}

impl Xschem {
    /// The picture panes that exist now. Valid until the next call into xschem.
    pub fn previews(&self) -> Vec<Pane> {
        // SAFETY: no arguments; refreshes the list the other calls index.
        let count = unsafe { cdx_preview_list() };
        (0..count)
            .filter_map(|index| {
                let (mut width, mut height, mut changed) = (0, 0, 0);
                // SAFETY: the index is below the count just returned, pointers valid.
                let path =
                    unsafe { cdx_preview_info(index, &mut width, &mut height, &mut changed) };
                if path.is_null() || width <= 0 || height <= 0 {
                    return None;
                }
                // SAFETY: a NUL-terminated string owned by the C side, valid for now.
                let path = unsafe { CStr::from_ptr(path) }
                    .to_string_lossy()
                    .into_owned();
                Some(Pane {
                    index,
                    path,
                    width: width as u32,
                    height: height as u32,
                    changed: changed != 0,
                })
            })
            .collect()
    }

    /// Copy all pixels of `pane` into `frame`: rows of `stride` bytes, R G B A
    /// premultiplied. Nothing happens when `frame` is too small. Call it right after
    /// [`Xschem::previews`], before anything else calls into xschem.
    pub fn read_preview(&self, pane: &Pane, frame: &mut [u8], stride: u32) {
        if stride < pane.width * 4
            || frame.len() < (pane.height as usize - 1) * stride as usize + pane.width as usize * 4
        {
            return;
        }
        // SAFETY: the buffer holds `height` rows of `stride` bytes (checked above), and
        // the index came from the latest list, whose window has this size.
        unsafe { cdx_preview_read(pane.index, frame.as_mut_ptr(), stride as c_int) }
    }
}

/// Receive the widget updates the Tcl side reports with `cdept::ui <json>`. Call it
/// before [`Xschem::start`] to also get what xschem's start-up reports (the menus). The
/// sink runs on xschem's thread, also while a call into xschem is in progress.
pub fn on_ui(sink: impl Fn(&str) + 'static) {
    UI_SINK.with(|slot| *slot.borrow_mut() = Some(Rc::new(sink)));
    // SAFETY: stores a plain function pointer; no interpreter is needed yet.
    unsafe { cdx_set_ui_sink(ui_trampoline) }
}

/// Version of the xschem sources this program was built from.
pub fn version() -> String {
    // SAFETY: returns a string literal.
    unsafe { CStr::from_ptr(cdx_xschem_version()) }
        .to_string_lossy()
        .into_owned()
}
