//! Builds xschem's C sources (git submodule `xschem/`) into a static library, against
//! the stand-in headers in `shim/include` instead of Xlib and Tk.
//!
//! Needs on the build host: a C compiler, flex, bison, and the development files of Tcl
//! and cairo (Debian/Ubuntu: `tcl-dev libcairo2-dev flex bison`).

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

/// xschem's sources as listed in `xschem/src/Makefile.in`, without `main.c` (replaced
/// by `shim/engine.c`) and `cairo_jpg.c` (needs libjpeg, left out for now).
const XSCHEM_SOURCES: &[&str] = &[
    "icon.c",
    "callback.c",
    "actions.c",
    "move.c",
    "check.c",
    "clip.c",
    "draw.c",
    "globals.c",
    "netlist.c",
    "hash_iterator.c",
    "findnet.c",
    "scheduler.c",
    "store.c",
    "xinit.c",
    "select.c",
    "font.c",
    "editprop.c",
    "save.c",
    "paste.c",
    "token.c",
    "psprint.c",
    "node_hash.c",
    "hilight.c",
    "options.c",
    "vhdl_netlist.c",
    "svgdraw.c",
    "spice_netlist.c",
    "spectre_netlist.c",
    "tedax_netlist.c",
    "verilog_netlist.c",
    "in_memory_undo.c",
];

fn main() {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let src = manifest.join("xschem/src");
    let shim = manifest.join("shim");
    let include = shim.join("include");

    if !src.join("xschem.h").is_file() {
        panic!(
            "xschem sources not found in {}.\nRun: git submodule update --init",
            src.display()
        );
    }

    // Parsers. Same commands as xschem's Makefile.
    generate(
        "bison",
        &["-d", "-o"],
        &out.join("expandlabel.c"),
        &src.join("expandlabel.y"),
    );
    generate(
        "bison",
        &["-p", "kk", "-o"],
        &out.join("eval_expr.c"),
        &src.join("eval_expr.y"),
    );
    generate(
        "flex",
        &["-l", "-o"],
        &out.join("parselabel.c"),
        &src.join("parselabel.l"),
    );

    let tcl = ["tcl", "tcl8.6", "tcl86", "tcl9.0"]
        .iter()
        .find_map(|name| pkg_config::Config::new().probe(name).ok())
        .expect("Tcl development files not found (pkg-config: tcl). Install tcl-dev.");
    let cairo = pkg_config::Config::new()
        .probe("cairo")
        .expect("cairo development files not found (pkg-config: cairo). Install libcairo2-dev.");
    println!("cargo:rustc-link-lib=m");

    let configure = |build: &mut cc::Build| {
        build
            // Our stand-ins come first, so <X11/Xlib.h>, <tk.h> and <cairo-xlib.h>
            // resolve to them.
            .include(&include)
            .include(&out)
            .include(&src)
            .includes(&tcl.include_paths)
            .includes(&cairo.include_paths)
            .define("NO_SCCONFIG", None)
            .flag("-include")
            .flag(include.join("xschem_config.h"))
            .flag("-std=gnu99");
    };

    // xschem itself: upstream code, compiled as upstream does (no extra warnings).
    let mut xschem = cc::Build::new();
    configure(&mut xschem);
    xschem.warnings(false).flag_if_supported("-w");
    for file in XSCHEM_SOURCES {
        xschem.file(src.join(file));
    }
    for file in ["expandlabel.c", "eval_expr.c", "parselabel.c"] {
        xschem.file(out.join(file));
    }
    xschem.compile("xschem");

    // Our own C code, with warnings on.
    let mut glue = cc::Build::new();
    configure(&mut glue);
    for file in ["xshim.c", "engine.c"] {
        glue.file(shim.join(file));
    }
    glue.compile("cdxshim");

    for path in ["build.rs", "shim", "xschem/src"] {
        println!("cargo:rerun-if-changed={path}");
    }

    // xschem's runtime files, laid out as `make install` does (PDK start-up files
    // expect $XSCHEM_SHAREDIR/xschem_library/devices). Used when running from the
    // build tree; an installed helper finds its own copy.
    let share = out.join("share/xschem");
    assemble_share(&manifest.join("xschem"), &share);
    println!(
        "cargo:rustc-env=CDEPT_XSCHEM_DEFAULT_SHAREDIR={}",
        share.display()
    );
}

/// Build an xschem share folder out of symbolic links into the submodule: the Tcl and
/// awk scripts and `systemlib` from `src/`, and `xschem_library/`.
fn assemble_share(xschem: &Path, share: &Path) {
    use std::os::unix::fs::symlink;

    let _ = std::fs::remove_dir_all(share);
    std::fs::create_dir_all(share).unwrap();
    let src = xschem.join("src");
    for entry in std::fs::read_dir(&src).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        let name_text = name.to_string_lossy();
        let wanted = if entry.file_type().unwrap().is_dir() {
            matches!(&*name_text, "systemlib" | "utile")
        } else {
            let extension = Path::new(&name).extension().and_then(|e| e.to_str());
            // Not sources or build files, and not a locally built `xschem` program:
            // its presence makes xschem believe it runs from its source folder.
            !matches!(extension, Some("c" | "h" | "o" | "y" | "l" | "in"))
                && !matches!(&*name_text, "xschem" | "rawtovcd" | "Makefile")
        };
        if wanted {
            symlink(entry.path(), share.join(&name)).unwrap();
        }
    }
    symlink(xschem.join("xschem_library"), share.join("xschem_library")).unwrap();
}

fn generate(tool: &str, args: &[&str], output: &Path, input: &Path) {
    let mut command = Command::new(tool);
    // The last argument is "-o"; flex wants the file name glued to it.
    let (flag, options) = args.split_last().unwrap();
    command.args(options);
    if tool == "flex" {
        let mut glued = std::ffi::OsString::from(flag);
        glued.push(output);
        command.arg(glued);
    } else {
        command.arg(flag).arg(output);
    }
    command.arg(input);
    run(tool, command);
}

fn run(tool: &str, mut command: Command) {
    let status = command
        .status()
        .unwrap_or_else(|e| panic!("cannot run {tool}: {e}. Install {tool}."));
    if !status.success() {
        panic!("{tool} failed: {command:?}");
    }
}
