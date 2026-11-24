//! Starting UberWave on one or several files, and finding the viewer that already runs.
//!
//! A viewer with IPC (`--version` prints `... ipc 1`) is started as
//! `UberWaveViewer --ipc <socket> [--analysis A] [--tag T ...] FILE...`, with the socket
//! named after the project and the run ([`ipc::socket_path`]); a second request for the
//! same run connects to it and sends `open` / `import` instead of starting another
//! window. An older viewer gets the old command line with one file.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use eyre::{Result, WrapErr, bail};

use super::backend::{self, Backend, Connection, ViewerOps};
use super::embedded;
use super::ipc::{self, Client};
use super::show::{self, Applied, ShowRequest};
use super::{PointRaw, analyses, analysis_arg, command_line, require, spawn_detached};
use crate::Project;

/// How long `--version` may take before the viewer is taken for an old one.
pub const VERSION_TIMEOUT: Duration = Duration::from_secs(2);
/// How long a freshly started viewer has to open its socket.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

/// What `--version` said.
#[derive(Debug, Clone, PartialEq)]
pub struct Version {
    /// The first line, such as `UberWaveViewer 0.9.1 ipc 1`.
    pub line: String,
    /// The line says `ipc 1`.
    pub ipc: bool,
}

/// Ask the viewer for its version, never opening a window: no display is given to it
/// (and Qt-style toolkits get the offscreen platform), the working folder is `/`, and
/// it is killed after [`VERSION_TIMEOUT`]. An old viewer that does not know the flag
/// ends up here as `None` (it printed nothing, failed or was killed). The answer is
/// remembered for the program file (path and modification time).
pub fn version(program: &Path) -> Option<Version> {
    type Key = (PathBuf, Option<std::time::SystemTime>);
    static CACHE: Mutex<Option<HashMap<Key, Option<Version>>>> = Mutex::new(None);
    let key = (
        program.to_path_buf(),
        std::fs::metadata(program).and_then(|m| m.modified()).ok(),
    );
    if let Some(hit) = CACHE.lock().unwrap().as_ref().and_then(|c| c.get(&key)) {
        return hit.clone();
    }
    let found = probe(program);
    CACHE
        .lock()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .insert(key, found.clone());
    found
}

fn probe(program: &Path) -> Option<Version> {
    use std::io::Read;
    use std::os::unix::process::CommandExt;
    let mut child = Command::new(program)
        .arg("--version")
        .current_dir("/")
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .env("QT_QPA_PLATFORM", "offscreen")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .ok()?;
    let mut out = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = out.by_ref().take(64 * 1024).read_to_string(&mut text);
        text
    });
    let start = Instant::now();
    let done = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) if start.elapsed() < VERSION_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(10))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break false;
            }
        }
    };
    let text = reader.join().ok()?;
    if !done {
        return None;
    }
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())?
        .to_string();
    if !line.contains("UberWave") {
        return None;
    }
    Some(Version {
        ipc: line.contains("ipc 1"),
        line,
    })
}

/// One file to show: its tag in the viewer (the run point's name) and the raw file.
#[derive(Debug, Clone, PartialEq)]
pub struct ViewFile {
    pub tag: String,
    pub raw: PathBuf,
}

impl From<&PointRaw> for ViewFile {
    fn from(p: &PointRaw) -> Self {
        ViewFile {
            tag: p.point.clone(),
            raw: p.raw.clone(),
        }
    }
}

/// What [`open_files`] did.
pub struct Opened {
    /// The command line started, empty when a running viewer was reused.
    pub command: Vec<OsString>,
    /// The viewer ran already and was told to open the files over its socket.
    pub reused: bool,
    /// The viewer speaks IPC (the embedded one does everything IPC does).
    pub ipc: bool,
    /// The viewer is part of this executable ([`Backend::Embedded`]).
    pub embedded: bool,
    /// The socket, with IPC.
    pub socket: Option<PathBuf>,
    /// With an old viewer: files that could not be opened (only the first is).
    pub not_opened: Vec<ViewFile>,
    /// The connection (only when asked for).
    pub client: Option<Connection>,
    /// `tag:name` of the signals sent to the viewer with `show`.
    pub shown: Vec<String>,
    /// What went wrong without stopping the open (a signal a file lacks, a refused `show`).
    pub notes: Vec<String>,
}

impl Opened {
    /// One line for the user.
    pub fn text(&self) -> String {
        if self.embedded {
            return if self.reused {
                "reused the running UberWave (built in)".into()
            } else {
                "UberWave (built in) is open".into()
            };
        }
        if self.reused {
            return format!(
                "reused the running UberWave ({})",
                self.socket
                    .as_deref()
                    .map(|s| s.display().to_string())
                    .unwrap_or_default()
            );
        }
        self.command
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// What an old viewer left out, or nothing.
    pub fn old_viewer_note(&self) -> Option<String> {
        if self.ipc {
            return None;
        }
        let mut note = String::from(
            "this UberWave has no IPC (`UberWaveViewer --version` does not say 'ipc 1'): it opens one file, \
             and a calculator result cannot be sent to it (docs/howto/view-waveforms.md)",
        );
        if !self.not_opened.is_empty() {
            note.push_str("; File > Import adds the others:");
            for f in &self.not_opened {
                note.push_str(&format!("\n  {}\t{}", f.tag, f.raw.display()));
            }
        }
        Some(note)
    }
}

/// What to do with a viewer that runs already.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// A running viewer gets the files (`open` then `import`: the session is replaced);
    /// a new one is started and not waited for.
    Open,
    /// A running viewer is left as it is; a new one is started on the files and waited
    /// for, and the connection returned (to push traces).
    Connect,
}

/// Show `files` (one tag each) in the viewer of `run` (`library/cell/view`; any text
/// for files that belong to no run). `analysis` as for [`analysis_arg`], per file. `show`
/// says what the viewer plots (nothing is plotted until `show`); a viewer that is started
/// and has to be sent `show` is connected to and waited for, which is not done for
/// `Show::Off` in `Mode::Open`. In `Mode::Connect` a running viewer is left as it is.
pub fn open_files(
    project: Option<&Project>,
    run: &str,
    files: &[ViewFile],
    analysis: Option<&str>,
    mode: Mode,
    show: &ShowRequest,
) -> Result<Opened> {
    let Some(first) = files.first() else {
        bail!("no files to open");
    };
    for f in files {
        if !f.raw.exists() {
            bail!("{} does not exist", f.raw.display());
        }
    }
    let per_file: Vec<Option<String>> = files
        .iter()
        .map(|f| {
            let found = if f.raw.is_file() {
                analyses(&f.raw).unwrap_or_default()
            } else {
                vec![]
            };
            analysis_arg(&found, analysis)
        })
        .collect::<Result<_>>()?;
    if backend::choose(project)? == Backend::Embedded {
        return open_embedded(files, &per_file, mode, show);
    }
    let program = require(project)?;
    let dir = std::path::absolute(&first.raw)
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf));
    let dir = dir.filter(|d| d.is_dir());
    let caps = version(&program);
    if !caps.as_ref().is_some_and(|v| v.ipc) {
        let line = command_line(&program, &first.raw, per_file[0].as_deref());
        spawn_detached(&line, dir.as_deref(), project)?;
        return Ok(Opened {
            command: line,
            reused: false,
            ipc: false,
            embedded: false,
            socket: None,
            not_opened: files[1..].to_vec(),
            client: None,
            shown: vec![],
            notes: vec![],
        });
    }
    let root = project
        .map(|p| p.root.clone())
        .or_else(|| dir.clone())
        .unwrap_or_else(|| PathBuf::from("."));
    let socket = ipc::socket_path(&root, run);
    // A viewer of this run that already runs.
    let mut applied = Applied::default();
    if let Some(mut client) = existing(&socket)? {
        if mode == Mode::Open {
            for (i, (f, a)) in files.iter().zip(&per_file).enumerate() {
                let (path, tag) = (absolute(&f.raw), Some(f.tag.as_str()));
                if i == 0 {
                    client.open(&path, a.as_deref(), tag)?;
                } else {
                    client.import(&path, a.as_deref(), tag)?;
                }
            }
            applied = show::apply(&mut client, files, show, show::LOAD_TIMEOUT);
        }
        return Ok(Opened {
            command: vec![],
            reused: true,
            ipc: true,
            embedded: false,
            socket: Some(socket),
            not_opened: vec![],
            client: (mode == Mode::Connect).then_some(Connection::Ipc(client)),
            shown: applied.shown,
            notes: applied.notes,
        });
    }
    // Nobody listens: a socket file left by a viewer that died would stop the new one.
    let _ = std::fs::remove_file(&socket);
    let mut line: Vec<OsString> = vec![
        program.as_os_str().to_os_string(),
        "--ipc".into(),
        socket.as_os_str().to_os_string(),
    ];
    if let Some(a) = &per_file[0] {
        line.push("--analysis".into());
        line.push(a.into());
    }
    for f in files {
        line.push("--tag".into());
        line.push((&f.tag).into());
    }
    line.extend(files.iter().map(|f| absolute(&f.raw).into_os_string()));
    spawn_detached(&line, dir.as_deref(), project)?;
    let connect = mode == Mode::Connect || show.enabled();
    let mut client = None;
    if connect {
        let started = Client::connect(&socket, CONNECT_TIMEOUT).and_then(|mut c| {
            c.hello()
                .wrap_err("the new viewer does not answer hello")
                .map(|_| c)
        });
        match started {
            Ok(mut c) => {
                applied = show::apply(&mut c, files, show, show::LOAD_TIMEOUT);
                client = Some(Connection::Ipc(c));
            }
            // The viewer runs, only `show` could not be sent: not an error to open.
            Err(e) if mode == Mode::Open => applied.notes.push(format!("{e:#}")),
            Err(e) => return Err(e),
        }
    }
    let client = if mode == Mode::Connect { client } else { None };
    Ok(Opened {
        command: line,
        reused: false,
        ipc: true,
        embedded: false,
        socket: Some(socket),
        not_opened: vec![],
        client,
        shown: applied.shown,
        notes: applied.notes,
    })
}

/// [`open_files`] with the viewer of this process: the window is reused while it is open
/// and started again after it was closed.
fn open_embedded(
    files: &[ViewFile],
    per_file: &[Option<String>],
    mode: Mode,
    show: &ShowRequest,
) -> Result<Opened> {
    let (viewer, reused) = embedded::acquire()?;
    let mut conn = Connection::Embedded(viewer);
    let mut applied = Applied::default();
    if !reused || mode == Mode::Open {
        for (i, (f, a)) in files.iter().zip(per_file).enumerate() {
            let (path, tag) = (absolute(&f.raw), Some(f.tag.as_str()));
            if i == 0 {
                ViewerOps::open(&mut conn, &path, a.as_deref(), tag)?;
            } else {
                ViewerOps::import(&mut conn, &path, a.as_deref(), tag)?;
            }
        }
        applied = show::apply(&mut conn, files, show, show::LOAD_TIMEOUT);
    }
    Ok(Opened {
        command: vec![],
        reused,
        ipc: true,
        embedded: true,
        socket: None,
        not_opened: vec![],
        client: (mode == Mode::Connect).then_some(conn),
        shown: applied.shown,
        notes: applied.notes,
    })
}

/// The connection to a viewer that listens on `socket`, if one does. Something that
/// accepts but does not say hello (a viewer still starting) is tried again a few times
/// and then is an error: its socket is never taken away.
fn existing(socket: &Path) -> Result<Option<Client>> {
    let mut last = None;
    for attempt in 0..5 {
        let Ok(mut client) = Client::connect_once(socket) else {
            if attempt == 0 {
                return Ok(None);
            }
            std::thread::sleep(Duration::from_millis(200));
            continue;
        };
        match client.hello() {
            Ok(_) => return Ok(Some(client)),
            Err(e) => {
                last = Some(e);
                std::thread::sleep(Duration::from_millis(200));
            }
        }
    }
    match last {
        Some(e) => Err(e.wrap_err(format!(
            "the viewer on {} does not answer hello",
            socket.display()
        ))),
        None => Ok(None),
    }
}

fn absolute(p: &Path) -> PathBuf {
    std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf())
}

/// The run key of a testbench view, `library/cell/view`.
pub fn run_key(library: &str, cell: &str, view: &str) -> String {
    format!("{library}/{cell}/{view}")
}
