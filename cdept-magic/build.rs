//! Builds the Tk and X stand-ins (`shim/cdm.c`) that let Magic's `tclmagic.so` run
//! inside the helper. Magic itself is not compiled: the helper loads the `tclmagic.so`
//! of the Magic installation at run time.
//!
//! Needs on the build host: a C compiler and the development files of Tcl, Tk (only
//! `tk.h`, for the layout of Tk's stub table and window record; libtk is not linked),
//! X11 (headers only) and cairo. Debian/Ubuntu: `tcl-dev tk-dev libx11-dev libcairo2-dev`.

use std::env;
use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let tcl = ["tcl", "tcl8.6", "tcl86"]
        .iter()
        .find_map(|name| pkg_config::Config::new().probe(name).ok())
        .expect("Tcl development files not found (pkg-config: tcl). Install tcl-dev.");
    // Headers only: the stub table and Tk_FakeWin of the Tk that Magic was built for.
    let tk = ["tk", "tk8.6"]
        .iter()
        .find_map(|name| {
            pkg_config::Config::new()
                .cargo_metadata(false)
                .probe(name)
                .ok()
        })
        .expect("Tk development files not found (pkg-config: tk). Install tk-dev.");
    let cairo = pkg_config::Config::new()
        .probe("cairo")
        .expect("cairo development files not found (pkg-config: cairo). Install libcairo2-dev.");

    let mut build = cc::Build::new();
    build
        .file(manifest.join("shim/cdm.c"))
        .include(manifest.join("shim"))
        .includes(&tcl.include_paths)
        .includes(&tk.include_paths)
        .includes(&cairo.include_paths)
        .flag("-std=gnu99")
        .flag_if_supported("-Wno-unused-parameter");
    build.compile("cdmshim");

    // The X functions Magic calls must be visible to the dynamic linker, so that they
    // win over libX11's when tclmagic.so is loaded.
    println!("cargo:rustc-link-arg-bins=-Wl,--export-dynamic");
    println!("cargo:rustc-link-lib=m");
    for path in ["build.rs", "shim", "tcl"] {
        println!("cargo:rerun-if-changed={path}");
    }
    // The Tk stand-in written in Tcl, shared with the xschem helper, and our own Tcl.
    let faketk = manifest.join("../cdept-xschem/shim/tcl/faketk.tcl");
    println!(
        "cargo:rustc-env=CDEPT_MAGIC_DEFAULT_FAKETK={}",
        faketk.display()
    );
    println!(
        "cargo:rustc-env=CDEPT_MAGIC_DEFAULT_TCLDIR={}",
        manifest.join("tcl").display()
    );
}
