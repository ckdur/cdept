//! Safe wrapper around the C side (`shim/cdm.h`) and the start-up of Magic inside the
//! helper's Tcl interpreter.

use std::cell::RefCell;
use std::ffi::{CStr, CString, c_char, c_int, c_uchar, c_ulong};
use std::marker::PhantomData;
use std::os::fd::RawFd;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};

use libcdept::xschem::frame::Rect;
use libcdept::xschem::input::Input;
use libcdept::xschem::tcl;

unsafe extern "C" {
    fn cdm_init(argv0: *const c_char, display: c_int, sw: c_int, sh: c_int) -> c_int;
    fn cdm_eval(script: *const c_char) -> c_int;
    fn cdm_result() -> *const c_char;
    fn cdm_watch_fd(fd: c_int, readable: extern "C" fn());
    fn cdm_when_idle(call: extern "C" fn());
    fn cdm_do_one_event(wait: c_int) -> c_int;
    fn cdm_every(ms: c_int, tick: extern "C" fn());
    fn cdm_set_ui_sink(sink: extern "C" fn(*const c_char));
    fn cdm_display_resize(width: c_int, height: c_int);
    fn cdm_display_event(kind: c_int, x: c_int, y: c_int, keysym: c_ulong, button: c_int, state: c_int);
    fn cdm_display_size(width: *mut c_int, height: *mut c_int) -> c_int;
    fn cdm_display_take_damage(x: *mut c_int, y: *mut c_int, w: *mut c_int, h: *mut c_int) -> c_int;
    fn cdm_display_read(dst: *mut c_uchar, stride: c_int, x: c_int, y: c_int, w: c_int, h: c_int);
    fn cdm_display_title() -> *const c_char;
}

static STARTED: AtomicBool = AtomicBool::new(false);

type Callback = Rc<dyn Fn()>;
type Sink = Rc<dyn Fn(&str)>;

thread_local! {
    static READABLE: RefCell<Option<Callback>> = const { RefCell::new(None) };
    static IDLE: RefCell<Option<Callback>> = const { RefCell::new(None) };
    static TICK: RefCell<Option<Callback>> = const { RefCell::new(None) };
    static UI_SINK: RefCell<Option<Sink>> = const { RefCell::new(None) };
}

fn call(slot: &'static std::thread::LocalKey<RefCell<Option<Callback>>>) {
    let callback = slot.with(|s| s.borrow().clone());
    if let Some(callback) = callback {
        callback();
    }
}

extern "C" fn readable_trampoline() {
    call(&READABLE);
}
extern "C" fn idle_trampoline() {
    call(&IDLE);
}
extern "C" fn tick_trampoline() {
    call(&TICK);
}
extern "C" fn ui_trampoline(json: *const c_char) {
    // SAFETY: Tcl hands a valid NUL-terminated string that lives through this call.
    let json = unsafe { CStr::from_ptr(json) }.to_string_lossy();
    let sink = UI_SINK.with(|s| s.borrow().clone());
    if let Some(sink) = sink {
        sink(&json);
    }
}

/// Where things are and how to start.
#[derive(Debug, Clone, Default)]
pub struct StartOptions {
    /// Magic's Tcl folder (`<CAD_ROOT>/magic/tcl`, holding `magic.tcl` and
    /// `tclmagic.so`).
    pub magic_tcl_dir: PathBuf,
    /// Magic's start-up file (the PDK's magicrc); `None`: Magic's own search.
    pub magic_rc: Option<PathBuf>,
    /// CDePT's Tcl, run after Magic started (search paths, the cell to load).
    pub rcfile: Option<PathBuf>,
    /// The Tk stand-in (faketk.tcl) and our own Tcl files.
    pub faketk: PathBuf,
    pub own_tcl_dir: PathBuf,
    /// Draw: Magic gets its Cairo display (`-d XR`) and a window.
    pub display: bool,
    /// Largest window, in pixels.
    pub screen: (u32, u32),
}

/// Magic in this process. Not `Send`: Magic and its Tcl interpreter stay on the thread
/// that started them.
pub struct Magic {
    _one_thread: PhantomData<*mut ()>,
}

impl Magic {
    pub fn start(options: &StartOptions) -> Result<Magic, String> {
        if STARTED.swap(true, Ordering::SeqCst) {
            return Err("Magic was already started in this process".into());
        }
        let argv0 = CString::new("cdept-magic").expect("no NUL");
        // SAFETY: once per process (STARTED), on this thread.
        let status = unsafe {
            cdm_init(
                argv0.as_ptr(),
                c_int::from(options.display),
                options.screen.0 as c_int,
                options.screen.1 as c_int,
            )
        };
        let magic = Magic {
            _one_thread: PhantomData,
        };
        if status != 0 {
            return Err(magic.result());
        }
        let q = |p: &Path| tcl::quote(&p.to_string_lossy());
        let mut setup = String::new();
        // Nobody can answer a dialog while Magic starts (faketk.tcl reads this).
        setup.push_str("set ::cdept::starting 1\n");
        if options.display {
            setup.push_str(&format!("source {}\n", q(&options.faketk)));
        }
        setup.push_str(&format!(
            "source {}\n",
            q(&options.own_tcl_dir.join("cdept-magic.tcl"))
        ));
        magic.eval(&setup).map_err(|e| format!("Tcl set-up: {e}"))?;

        // Magic's own start-up script, unchanged, with the arguments `magic` would
        // pass it: no GUI wrapper (CDePT has its own), the Cairo display.
        let mut args = vec!["-nowrapper".to_string(), "-noconsole".to_string()];
        if options.display {
            // The window is opened after initialisation (cdept-magic.tcl), once its
            // caption and scroll bars are off.
            args.extend(["-nowindow".into(), "-d".into(), "XR".into()]);
        } else {
            args.push("-dnull".into());
        }
        if let Some(rc) = &options.magic_rc {
            args.extend(["-rcfile".into(), rc.to_string_lossy().into_owned()]);
        }
        let list: Vec<String> = args.iter().map(|a| tcl::quote(a)).collect();
        let magic_tcl = options.magic_tcl_dir.join("magic.tcl");
        let start = format!(
            "set argv0 magic\nset argc {}\nset argv [list {}]\n\
             cdept::magic::start {}",
            args.len(),
            list.join(" "),
            q(&magic_tcl)
        );
        magic.eval(&start).map_err(|e| format!("Magic did not start: {e}"))?;
        if let Some(rcfile) = &options.rcfile {
            magic
                .eval(&format!("source {}", q(rcfile)))
                .map_err(|e| format!("{}: {e}", rcfile.display()))?;
        }
        magic.eval("set ::cdept::starting 0").ok();
        Ok(magic)
    }

    pub fn eval(&self, script: &str) -> Result<String, String> {
        let script = CString::new(script).map_err(|_| "the script contains a NUL byte".to_string())?;
        // SAFETY: the interpreter exists and we are on its thread.
        let status = unsafe { cdm_eval(script.as_ptr()) };
        let result = self.result();
        if status == 0 { Ok(result) } else { Err(result) }
    }

    fn result(&self) -> String {
        // SAFETY: Tcl returns a valid string, copied before the next call into Tcl.
        unsafe { CStr::from_ptr(cdm_result()) }
            .to_string_lossy()
            .into_owned()
    }

    pub fn version(&self) -> String {
        self.eval("cdept::magic::version").unwrap_or_default()
    }

    pub fn watch(&self, fd: RawFd, readable: impl Fn() + 'static) {
        READABLE.with(|s| *s.borrow_mut() = Some(Rc::new(readable)));
        // SAFETY: on the interpreter's thread; the trampoline is a plain function.
        unsafe { cdm_watch_fd(fd, readable_trampoline) }
    }

    pub fn on_idle(&self, callback: impl Fn() + 'static) {
        IDLE.with(|s| *s.borrow_mut() = Some(Rc::new(callback)));
    }

    pub fn when_idle(&self) {
        // SAFETY: as above.
        unsafe { cdm_when_idle(idle_trampoline) }
    }

    pub fn every(&self, ms: u32, tick: impl Fn() + 'static) {
        TICK.with(|s| *s.borrow_mut() = Some(Rc::new(tick)));
        // SAFETY: as above.
        unsafe { cdm_every(ms as c_int, tick_trampoline) }
    }

    pub fn do_one_event(&self, wait: bool) -> bool {
        // SAFETY: as above.
        unsafe { cdm_do_one_event(c_int::from(wait)) != 0 }
    }

    pub fn run_pending_events(&self) {
        while self.do_one_event(false) {}
    }

    pub fn resize(&self, width: u32, height: u32) {
        // SAFETY: as above.
        unsafe { cdm_display_resize(width as c_int, height as c_int) }
    }

    pub fn input(&self, input: Input) {
        let e = input.to_x();
        // SAFETY: as above.
        unsafe {
            cdm_display_event(
                e.kind,
                e.x,
                e.y,
                c_ulong::from(e.keysym),
                e.button as c_int,
                e.state as c_int,
            )
        }
    }

    pub fn display_size(&self) -> Option<(u32, u32)> {
        let (mut width, mut height) = (0, 0);
        // SAFETY: valid pointers.
        let exists = unsafe { cdm_display_size(&mut width, &mut height) } != 0;
        (exists && width > 0 && height > 0).then_some((width as u32, height as u32))
    }

    pub fn take_damage(&self) -> Option<Rect> {
        let (mut x, mut y, mut w, mut h) = (0, 0, 0, 0);
        // SAFETY: valid pointers.
        let any = unsafe { cdm_display_take_damage(&mut x, &mut y, &mut w, &mut h) } != 0;
        (any && w > 0 && h > 0).then(|| Rect::new(x as u32, y as u32, w as u32, h as u32))
    }

    /// Copy `rect` of the drawing window into `frame` (rows of `stride` bytes, R G B A,
    /// the frame having the window's size). Clipped to window and buffer.
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
        // SAFETY: destination range checked against `frame`, rectangle against the
        // window.
        unsafe {
            cdm_display_read(
                frame.as_mut_ptr().add(offset),
                stride as c_int,
                rect.x as c_int,
                rect.y as c_int,
                rect.width as c_int,
                rect.height as c_int,
            )
        }
    }

    /// The window title Magic set (the cell and its state).
    pub fn title(&self) -> String {
        // SAFETY: a static buffer of the C side.
        unsafe { CStr::from_ptr(cdm_display_title()) }
            .to_string_lossy()
            .into_owned()
    }
}

/// Receive `cdept::ui <json>` reports. Call before [`Magic::start`] to also get those
/// of the start-up.
pub fn on_ui(sink: impl Fn(&str) + 'static) {
    UI_SINK.with(|s| *s.borrow_mut() = Some(Rc::new(sink)));
    // SAFETY: stores a plain function pointer.
    unsafe { cdm_set_ui_sink(ui_trampoline) }
}

/// Magic's Tcl folder from the `magic` start script, as `magic --prefix` reports
/// it (`<prefix>/magic/tcl`).
pub fn find_magic_tcl_dir(explicit: Option<&Path>) -> Result<PathBuf, String> {
    let ok = |dir: &Path| dir.join("magic.tcl").is_file() && dir.join("tclmagic.so").is_file();
    if let Some(dir) = explicit {
        return if ok(dir) {
            Ok(dir.to_path_buf())
        } else {
            Err(format!("{}: no magic.tcl and tclmagic.so there", dir.display()))
        };
    }
    if let Some(dir) = std::env::var_os("CDEPT_MAGIC_TCLDIR").map(PathBuf::from)
        && ok(&dir)
    {
        return Ok(dir);
    }
    let program = std::env::var_os("CDEPT_MAGIC")
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "magic".into());
    let output = std::process::Command::new(&program)
        .arg("--prefix")
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("cannot run {}: {e}", Path::new(&program).display()))?;
    let prefix = String::from_utf8_lossy(&output.stdout);
    let dir = PathBuf::from(prefix.lines().last().unwrap_or("").trim()).join("magic/tcl");
    if ok(&dir) {
        Ok(dir)
    } else {
        Err(format!(
            "Magic's Tcl files (magic.tcl, tclmagic.so) were not found: `{} --prefix` \
             gave {}; set CDEPT_MAGIC_TCLDIR",
            Path::new(&program).display(),
            dir.display()
        ))
    }
}
