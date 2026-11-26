//! Magic inside a CDePT tab (docs/MAGIC.md, "Embedded Magic").
//!
//! The helper program `cdept-magic` (GPL, a process of its own) loads Magic's
//! `tclmagic.so` with stand-ins for Tk and X and speaks the protocol of the xschem
//! helper ([`crate::xschem::protocol`], frame files): a tab drives it with a
//! [`crate::xschem::Session`] made from [`session_options`].

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use super::refs;
use super::tcl::word;
use crate::Project;
use crate::xschem::session::Wake;
use crate::xschem::{EngineOptions, SessionOptions};

/// File name of the helper program.
pub const HELPER_NAME: &str = "cdept-magic";
/// Environment variable naming the helper program explicitly.
pub const HELPER_ENV: &str = "CDEPT_MAGIC_HELPER";

/// Where the helper is: `$CDEPT_MAGIC_HELPER`, next to the running program, or on
/// `PATH`.
pub fn find_helper() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(HELPER_ENV).filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(path));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        // `cargo test` runs from target/debug/deps: the helper is one folder up.
        for dir in [Some(dir), dir.parent()].into_iter().flatten() {
            let sibling = dir.join(HELPER_NAME);
            if sibling.is_file() {
                return Some(sibling);
            }
        }
    }
    crate::tools::find_in_path(HELPER_NAME)
}

/// The cell name Magic knows a `.mag` file by.
pub fn cell_of(file: &Path) -> String {
    file.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// CDePT's Tcl, run in the helper once Magic has started: the search path for the
/// project's Magic views, then the cell `file` in the window.
pub fn startup_script(project: &Project, file: &Path) -> String {
    let mut t = String::new();
    for dir in refs::search_dirs(project) {
        t += &format!("catch {{addpath {}}}\n", word(&dir.to_string_lossy()));
    }
    let dir = file.parent().unwrap_or(Path::new("."));
    t += &format!(
        "set ::cdept::magic::own_dir {}\n",
        word(&dir.to_string_lossy())
    );
    t += &load_script(file);
    t
}

/// Load `file` (a `.mag` view) as the edit cell and fit it in the window. A file that
/// does not exist yet is created as an empty cell by Magic when it is saved.
pub fn load_script(file: &Path) -> String {
    if crate::Format::of_file(file) == Some(crate::Format::Gds) {
        // A GDS view looked at in Magic: read, its (first) top cell shown.
        let stem = cell_of(file);
        return format!(
            "gds read {f}\n\
             if {{[cellname list exists {c}] ne {{0}}}} {{load {c}}} else {{\n\
               load [lindex [lsearch -inline -all -not -exact [cellname list topcells] (UNNAMED)] 0]\n\
             }}\nselect clear\ncatch {{view}}\n",
            f = word(&file.to_string_lossy()),
            c = word(&stem)
        );
    }
    let base = file.with_extension("");
    format!(
        "load {}\nselect clear\ncatch {{view}}\n",
        word(&base.to_string_lossy())
    )
}

/// How to start the helper for `project`. `screen` bounds the window (pixels).
pub fn engine_options(project: &Project, file: &Path) -> EngineOptions {
    let mut env: Vec<(OsString, OsString)> = project.tool_env();
    if let Some(rc) = super::rc_file(project) {
        env.push(("CDEPT_MAGIC_RC".into(), rc.into_os_string()));
    }
    if let Some(magic) = super::Magic::find() {
        env.push((super::MAGIC_ENV.into(), magic.into_os_string()));
    }
    EngineOptions {
        helper: find_helper(),
        startup: Some(startup_script(project, file)),
        cwd: file.parent().map(Path::to_path_buf),
        env,
        display: true,
        // Magic reads its technology and the PDK's start-up script first.
        startup_timeout: std::time::Duration::from_secs(60),
    }
}

/// A session for the tab showing `file` (the helper is restarted with the same file
/// if it stops).
pub fn session_options(project: &Project, file: &Path, wake: Wake) -> SessionOptions {
    SessionOptions {
        engine: engine_options(project, file),
        file: None,
        wake,
    }
}

/// The Tcl that saves the edit cell, and the cells of the same view folder that
/// changed, and answers with what is still modified afterwards.
pub const SAVE_SCRIPT: &str = "cdept::magic::save; cdept::magic::modified";
