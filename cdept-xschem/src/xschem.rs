//! Safe wrapper around the C side (`shim/engine.c` and xschem itself).

use std::ffi::{CStr, CString, c_char, c_int};
use std::marker::PhantomData;
use std::sync::atomic::{AtomicBool, Ordering};

unsafe extern "C" {
    fn cdx_init(argc: c_int, argv: *mut *mut c_char) -> c_int;
    fn cdx_eval(script: *const c_char) -> c_int;
    fn cdx_result() -> *const c_char;
    fn cdx_run_pending_events();
    fn cdx_xschem_version() -> *const c_char;
}

/// xschem keeps its state in C globals: there can be one per process.
static STARTED: AtomicBool = AtomicBool::new(false);

/// The xschem engine of this process. Not `Send`: xschem and its Tcl interpreter must
/// stay on the thread that started them.
pub struct Xschem {
    _one_thread: PhantomData<*mut ()>,
}

impl Xschem {
    /// Start xschem with its own command line options (`args[0]` is the program name).
    pub fn start(args: &[String]) -> Result<Xschem, String> {
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
        let status = unsafe { cdx_init((argv.len() - 1) as c_int, argv.as_mut_ptr()) };
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

    /// Run timers and file events that are due.
    pub fn run_pending_events(&self) {
        // SAFETY: as in `eval`.
        unsafe { cdx_run_pending_events() }
    }

    fn result(&self) -> String {
        // SAFETY: Tcl returns a valid NUL-terminated string, copied before the next
        // call into Tcl.
        unsafe { CStr::from_ptr(cdx_result()) }
            .to_string_lossy()
            .into_owned()
    }
}

/// Version of the xschem sources this program was built from.
pub fn version() -> String {
    // SAFETY: returns a string literal.
    unsafe { CStr::from_ptr(cdx_xschem_version()) }
        .to_string_lossy()
        .into_owned()
}
