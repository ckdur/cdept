//! The C ABI of `sharedspice.h`, hand-written and loaded with dlopen.

use std::ffi::{c_char, c_int, c_short, c_void};
use std::path::Path;

use eyre::{Result, WrapErr};
use libloading::Library;

#[repr(C)]
pub struct NgComplex {
    pub re: f64,
    pub im: f64,
}

#[repr(C)]
pub struct VectorInfo {
    pub name: *mut c_char,
    pub typ: c_int,
    pub flags: c_short,
    pub real: *mut f64,
    pub comp: *mut NgComplex,
    pub length: c_int,
}

#[repr(C)]
pub struct VecValues {
    pub name: *mut c_char,
    pub creal: f64,
    pub cimag: f64,
    pub is_scale: bool,
    pub is_complex: bool,
}

#[repr(C)]
pub struct VecValuesAll {
    pub count: c_int,
    pub index: c_int,
    pub vecs: *mut *mut VecValues,
}

#[repr(C)]
pub struct VecInfo {
    pub number: c_int,
    pub name: *mut c_char,
    pub is_real: bool,
    pub pdvec: *mut c_void,
    pub pdvecscale: *mut c_void,
}

#[repr(C)]
pub struct VecInfoAll {
    pub name: *mut c_char,
    pub title: *mut c_char,
    pub date: *mut c_char,
    pub typ: *mut c_char,
    pub count: c_int,
    pub vecs: *mut *mut VecInfo,
}

pub type SendChar = unsafe extern "C" fn(*mut c_char, c_int, *mut c_void) -> c_int;
pub type SendStat = unsafe extern "C" fn(*mut c_char, c_int, *mut c_void) -> c_int;
pub type ControlledExit = unsafe extern "C" fn(c_int, bool, bool, c_int, *mut c_void) -> c_int;
pub type SendData = unsafe extern "C" fn(*mut VecValuesAll, c_int, c_int, *mut c_void) -> c_int;
pub type SendInitData = unsafe extern "C" fn(*mut VecInfoAll, c_int, *mut c_void) -> c_int;
pub type BgRunning = unsafe extern "C" fn(bool, c_int, *mut c_void) -> c_int;

type InitFn = unsafe extern "C" fn(
    Option<SendChar>,
    Option<SendStat>,
    Option<ControlledExit>,
    Option<SendData>,
    Option<SendInitData>,
    Option<BgRunning>,
    *mut c_void,
) -> c_int;

/// The entry points, resolved once. The library stays loaded for the life of the
/// process (ngspice cannot be unloaded safely), so the pointers stay valid.
pub struct Api {
    pub init: InitFn,
    pub circ: unsafe extern "C" fn(*mut *mut c_char) -> c_int,
    pub command: unsafe extern "C" fn(*mut c_char) -> c_int,
    pub cur_plot: unsafe extern "C" fn() -> *mut c_char,
    pub all_plots: unsafe extern "C" fn() -> *mut *mut c_char,
    pub all_vecs: unsafe extern "C" fn(*mut c_char) -> *mut *mut c_char,
    pub get_vec_info: unsafe extern "C" fn(*mut c_char) -> *mut VectorInfo,
    pub running: unsafe extern "C" fn() -> bool,
}

impl Api {
    pub fn load(path: &Path) -> Result<Api> {
        // SAFETY: loading a shared library runs its initialisers; that is what was asked.
        let lib = unsafe { Library::new(path) }
            .wrap_err_with(|| format!("cannot load {}", path.display()))?;
        // Never unloaded: the function pointers below must stay valid.
        let lib: &'static Library = Box::leak(Box::new(lib));
        macro_rules! sym {
            ($name:literal) => {
                // SAFETY: the signature is that of sharedspice.h.
                *unsafe { lib.get(concat!($name, "\0").as_bytes()) }
                    .wrap_err_with(|| format!("{} has no {}", path.display(), $name))?
            };
        }
        Ok(Api {
            init: sym!("ngSpice_Init"),
            circ: sym!("ngSpice_Circ"),
            command: sym!("ngSpice_Command"),
            cur_plot: sym!("ngSpice_CurPlot"),
            all_plots: sym!("ngSpice_AllPlots"),
            all_vecs: sym!("ngSpice_AllVecs"),
            get_vec_info: sym!("ngGet_Vec_Info"),
            running: sym!("ngSpice_running"),
        })
    }
}
