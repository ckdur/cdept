//! Starting xschem inside this process: where its runtime files are and which of its
//! command line options the helper uses.

use std::path::{Path, PathBuf};

use libcdept::xschem::tcl;

use crate::xschem::Xschem;

#[derive(Debug, Clone, Default)]
pub struct StartOptions {
    /// Start-up file (Tcl) to read instead of the user's `xschemrc`.
    pub rcfile: Option<PathBuf>,
    /// xschem's runtime files; found automatically when `None` (see [`find_sharedir`]).
    pub sharedir: Option<PathBuf>,
    /// Draw: xschem gets a window (see [`Xschem::resize`]). Without it xschem runs as
    /// with `xschem --no_x`.
    pub display: bool,
}

/// Locate xschem's runtime files and start it.
pub fn start(options: &StartOptions) -> Result<Xschem, String> {
    let sharedir = find_sharedir(options.sharedir.as_deref())?;
    let libraries = sharedir.join("xschem_library");

    // SAFETY: no other thread exists yet.
    unsafe { std::env::set_var("XSCHEM_SHAREDIR", &sharedir) };
    create_user_conf_dir(&sharedir)?;

    // Runs before the rc file: start the symbol search path with xschem's own devices
    // library, reachable both as `res.sym` and as `devices/res.sym` (schematics in the
    // wild use both). The rc file appends the project's libraries.
    let search_path = format!(
        "{}:{}",
        libraries.join("devices").to_string_lossy(),
        libraries.to_string_lossy()
    );
    let mut argv: Vec<String> = vec!["cdept-xschem".into(), "--no_readline".into()];
    if !options.display {
        argv.push("--no_x".into());
    }
    argv.push("--preinit".into());
    argv.push(format!(
        "set XSCHEM_LIBRARY_PATH {}",
        tcl::quote(&search_path)
    ));
    match &options.rcfile {
        Some(rcfile) => {
            argv.push("--rcfile".into());
            argv.push(rcfile.to_string_lossy().into_owned());
        }
        // Without a start-up file, do not pick up the user's own xschemrc either: the
        // engine should behave the same for everybody.
        None => argv.push("--no_rcload".into()),
    }
    let xschem = Xschem::start(&argv, options.display)?;
    // CDePT's own adjustments to xschem's Tcl side (file dialogs become the library
    // picker, symbols made from schematics go into the symbol view, …), read after
    // xschem.tcl and the start-up file. Without a display only the parts that need no
    // windows apply.
    let adjust = sharedir.join("cdept/cdept.tcl");
    if adjust.is_file() {
        xschem
            .eval(&format!("source {}", tcl::quote(&adjust.to_string_lossy())))
            .map_err(|e| format!("{}: {e}", adjust.display()))?;
    }
    Ok(xschem)
}

/// Create xschem's folder in the user's home (`USER_CONF_DIR` in `xschem_config.h`) if
/// it is missing, with the template `xschemrc`, as xschem itself does at start-up.
///
/// xschem checks for the folder and then creates it, and exits when the folder appeared
/// in between. That happens when several helpers start at the same moment on a machine
/// where xschem never ran. Creating it here first is safe against that.
fn create_user_conf_dir(sharedir: &Path) -> Result<(), String> {
    use std::os::unix::fs::DirBuilderExt;

    let Some(home) = std::env::var_os("HOME") else {
        return Ok(()); // xschem asks the system for the home folder itself
    };
    let dir = Path::new(&home).join(".xschem");
    match std::fs::DirBuilder::new().mode(0o700).create(&dir) {
        Ok(()) => {
            // Only the process that created the folder fills it.
            let _ = std::fs::copy(sharedir.join("xschemrc"), dir.join("xschemrc"));
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(format!("cannot create {}: {error}", dir.display())),
    }
}

/// The folder holding `xschem.tcl` and `xschem_library/devices`.
pub fn find_sharedir(explicit: Option<&Path>) -> Result<PathBuf, String> {
    let is_sharedir = |dir: &Path| {
        dir.join("xschem.tcl").is_file() && dir.join("xschem_library/devices").is_dir()
    };
    if let Some(dir) = explicit {
        return if is_sharedir(dir) {
            Ok(dir.to_path_buf())
        } else {
            Err(format!(
                "--sharedir {}: xschem.tcl or xschem_library/devices not found",
                dir.display()
            ))
        };
    }
    let from_env = std::env::var_os("CDEPT_XSCHEM_SHAREDIR").map(PathBuf::from);
    let installed = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.parent()?.join("share/cdept-xschem")));
    let build_tree = Some(PathBuf::from(env!("CDEPT_XSCHEM_DEFAULT_SHAREDIR")));
    [from_env, installed, build_tree]
        .into_iter()
        .flatten()
        .find(|dir| is_sharedir(dir))
        .ok_or_else(|| {
            "xschem's runtime files (xschem.tcl) were not found; set CDEPT_XSCHEM_SHAREDIR".into()
        })
}
