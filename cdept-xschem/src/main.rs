//! `cdept-xschem`: xschem's schematic engine as a helper process for CDePT.
//!
//! CDePT starts this program and talks to it over a Unix socket
//! (`libcdept::xschem::protocol`). xschem's own code does the work; this program is
//! therefore GPL-2.0-or-later, while CDePT itself stays MIT.
//!
//! Step 0 of docs/XSCHEM.md: no drawing yet. xschem runs as with `xschem --no_x`:
//! load, save, netlist, SVG export and queries.

mod xschem;

use std::io::BufReader;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use libcdept::xschem::protocol::{self, Message, PROTOCOL_VERSION, Request};
use libcdept::xschem::tcl;

use crate::xschem::Xschem;

const USAGE: &str = "\
cdept-xschem: xschem engine for CDePT (started by cdept, not by hand)

Usage: cdept-xschem --connect <socket> [--rcfile <file>] [--sharedir <folder>]
       cdept-xschem --version

  --connect <socket>   Unix socket CDePT listens on
  --rcfile <file>      start-up file (Tcl) to read instead of the user's xschemrc
  --sharedir <folder>  xschem's runtime files: xschem.tcl, the awk scripts and
                       xschem_library/devices (default: $CDEPT_XSCHEM_SHAREDIR, then
                       ../share/cdept-xschem next to this program, then the build tree)
";

struct Args {
    connect: PathBuf,
    rcfile: Option<PathBuf>,
    sharedir: Option<PathBuf>,
}

fn main() -> ExitCode {
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(Some(args)) => args,
        Ok(None) => return ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("cdept-xschem: {error}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("cdept-xschem: {error}");
            ExitCode::FAILURE
        }
    }
}

fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Option<Args>, String> {
    let mut connect = None;
    let mut rcfile = None;
    let mut sharedir = None;
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or(format!("{name} needs a value"));
        match arg.as_str() {
            "--connect" => connect = Some(PathBuf::from(value("--connect")?)),
            "--rcfile" => rcfile = Some(PathBuf::from(value("--rcfile")?)),
            "--sharedir" => sharedir = Some(PathBuf::from(value("--sharedir")?)),
            "--version" => {
                println!(
                    "cdept-xschem {} (xschem {}, protocol {PROTOCOL_VERSION})",
                    env!("CARGO_PKG_VERSION"),
                    xschem::version()
                );
                return Ok(None);
            }
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(None);
            }
            other => return Err(format!("unknown argument '{other}'")),
        }
    }
    let connect = connect.ok_or("--connect is required")?;
    Ok(Some(Args {
        connect,
        rcfile,
        sharedir,
    }))
}

fn run(args: Args) -> Result<(), String> {
    let stream = UnixStream::connect(&args.connect)
        .map_err(|e| format!("cannot connect to {}: {e}", args.connect.display()))?;
    let mut writer = stream.try_clone().map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(stream);
    let mut send = move |message: Message| {
        protocol::write_message(&mut writer, &message).map_err(|e| format!("socket: {e}"))
    };

    let xschem = match start_xschem(&args) {
        Ok(xschem) => xschem,
        Err(error) => {
            send(Message::Failed {
                error: error.clone(),
            })?;
            return Err(error);
        }
    };
    send(Message::Ready {
        protocol: PROTOCOL_VERSION,
        xschem: xschem::version(),
    })?;

    loop {
        // `None`: CDePT closed the connection (or died). Leave like on Shutdown.
        let request =
            protocol::read_message::<Request>(&mut reader).map_err(|e| format!("socket: {e}"))?;
        match request {
            Some(Request::Tcl { id, script }) => {
                let (ok, value) = match xschem.eval(&script) {
                    Ok(value) => (true, value),
                    Err(error) => (false, error),
                };
                send(Message::Reply { id, ok, value })?;
                xschem.run_pending_events();
            }
            Some(Request::Shutdown { id }) => {
                send(Message::Reply {
                    id,
                    ok: true,
                    value: String::new(),
                })?;
                break;
            }
            None => break,
        }
    }

    // Tcl's `exit` runs xschem's exit handler (removes its undo folder) and ends the
    // process; it does not return.
    let _ = xschem.eval("exit 0");
    Ok(())
}

/// Locate xschem's runtime files and start it without a display.
fn start_xschem(args: &Args) -> Result<Xschem, String> {
    let sharedir = find_sharedir(args.sharedir.as_deref())?;
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
    let mut argv: Vec<String> = vec![
        "cdept-xschem".into(),
        "--no_x".into(),
        "--no_readline".into(),
        "--preinit".into(),
        format!("set XSCHEM_LIBRARY_PATH {}", tcl::quote(&search_path)),
    ];
    match &args.rcfile {
        Some(rcfile) => {
            argv.push("--rcfile".into());
            argv.push(rcfile.to_string_lossy().into_owned());
        }
        // Without a start-up file, do not pick up the user's own xschemrc either: the
        // engine should behave the same for everybody.
        None => argv.push("--no_rcload".into()),
    }
    Xschem::start(&argv)
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
fn find_sharedir(explicit: Option<&Path>) -> Result<PathBuf, String> {
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
