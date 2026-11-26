//! Opening views in their tools, and the external tool windows CDePT starts
//! (docs/tools.md, "Opening views").
//!
//! [`choices`] says which tools can open a view: the embedded editors (a tab of
//! CDePT) and the tools' own windows. [`Launcher`] starts and supervises the windows:
//! every one gets a log file (`<project>/.cdept/logs/<tool>-<n>.log`, its standard
//! output and error), a limit of [`MAX_PER_TOOL`] at a time per tool, and
//! restart/stop. A file already open in a tool's window is not opened twice; a GDS or
//! OASIS file goes to a KLayout that already runs (klive, port 8082) before a new
//! window is started.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::SystemTime;

use eyre::{Result, WrapErr, bail, eyre};

use crate::entry::Format;
use crate::{Entry, Project};

/// Most windows of one tool CDePT keeps running at once.
pub const MAX_PER_TOOL: usize = 8;

/// One way to open a view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Choice {
    /// Stable name: `xschem`, `klayout`, `magic` (tabs), `xschem-window`,
    /// `klayout-window`, `magic-window`, `qucs-window`, `editor`.
    pub id: &'static str,
    pub label: &'static str,
    /// Opens in a CDePT tab (else a window of the tool).
    pub embedded: bool,
}

const XSCHEM_TAB: Choice = Choice {
    id: "xschem",
    label: "xschem (tab)",
    embedded: true,
};
const XSCHEM_WINDOW: Choice = Choice {
    id: "xschem-window",
    label: "xschem window",
    embedded: false,
};
const KLAYOUT_TAB: Choice = Choice {
    id: "klayout",
    label: "KLayout (tab)",
    embedded: true,
};
const KLAYOUT_WINDOW: Choice = Choice {
    id: "klayout-window",
    label: "KLayout window (a running one through klive, else a new one)",
    embedded: false,
};
const MAGIC_TAB: Choice = Choice {
    id: "magic",
    label: "Magic (tab)",
    embedded: true,
};
const MAGIC_WINDOW: Choice = Choice {
    id: "magic-window",
    label: "Magic window",
    embedded: false,
};
const QUCS_WINDOW: Choice = Choice {
    id: "qucs-window",
    label: "Qucs-S window",
    embedded: false,
};
const EDITOR: Choice = Choice {
    id: "editor",
    label: "Text editor (the desktop's)",
    embedded: false,
};

/// The tools that can open a file of `format`, the usual one first.
pub fn choices(format: Format) -> Vec<Choice> {
    match format {
        Format::XschemSchematic | Format::XschemSymbol => vec![XSCHEM_TAB, XSCHEM_WINDOW],
        Format::Gds | Format::Oasis => vec![KLAYOUT_TAB, MAGIC_TAB, KLAYOUT_WINDOW, MAGIC_WINDOW],
        Format::Lef => vec![KLAYOUT_TAB, KLAYOUT_WINDOW, EDITOR],
        Format::Magic => vec![MAGIC_TAB, MAGIC_WINDOW],
        Format::QucsSchematic => vec![QUCS_WINDOW, EDITOR],
        Format::KlayoutPcell
        | Format::Spice
        | Format::Cdl
        | Format::Verilog
        | Format::SystemVerilog
        | Format::Vhdl
        | Format::VerilogA
        | Format::Liberty => vec![EDITOR],
    }
}

/// The choices for a view (none for a view without a file).
pub fn choices_for(entry: &Entry) -> Vec<Choice> {
    match (entry.format, &entry.main_file) {
        (Some(format), Some(_)) => choices(format),
        _ => Vec::new(),
    }
}

/// A choice by its id.
pub fn choice(id: &str) -> Option<Choice> {
    [
        XSCHEM_TAB,
        XSCHEM_WINDOW,
        KLAYOUT_TAB,
        KLAYOUT_WINDOW,
        MAGIC_TAB,
        MAGIC_WINDOW,
        QUCS_WINDOW,
        EDITOR,
    ]
    .into_iter()
    .find(|c| c.id == id)
}

/// What a launch did.
#[derive(Debug, Clone, PartialEq)]
pub enum Launched {
    /// A new window, its process id and log.
    Started { pid: u32, log: PathBuf },
    /// The file went to a window that already runs (klive), or is open in one.
    Reused(String),
}

/// A window CDePT started.
#[derive(Debug)]
struct Proc {
    tool: &'static str,
    file: Option<PathBuf>,
    command: Vec<String>,
    child: Child,
    log: PathBuf,
    started: SystemTime,
    exit: Option<String>,
}

/// What [`Launcher::processes`] reports of one window.
#[derive(Debug, Clone, PartialEq)]
pub struct ProcInfo {
    pub index: usize,
    pub tool: &'static str,
    pub file: Option<PathBuf>,
    pub pid: u32,
    pub log: PathBuf,
    pub started: SystemTime,
    /// `None` while it runs, else how it ended.
    pub exit: Option<String>,
}

/// The tool windows started by this CDePT.
#[derive(Debug, Default)]
pub struct Launcher {
    procs: Mutex<Vec<Proc>>,
}

impl Launcher {
    pub fn new() -> Launcher {
        Launcher::default()
    }

    /// Open `file` with the external tool of `choice` (an embedded choice is the
    /// caller's to open in a tab).
    pub fn open(
        &self,
        project: Option<&Project>,
        choice: Choice,
        file: Option<&Path>,
    ) -> Result<Launched> {
        if choice.embedded {
            bail!("{} opens in a tab of CDePT", choice.label);
        }
        let file = file.map(|f| std::path::absolute(f).unwrap_or_else(|_| f.to_path_buf()));
        self.reap();
        if let Some(f) = &file {
            let procs = self.lock();
            if let Some(p) = procs
                .iter()
                .find(|p| p.exit.is_none() && p.tool == choice.id && p.file.as_deref() == Some(f))
            {
                return Ok(Launched::Reused(format!(
                    "{} is open in {} already (process {})",
                    f.display(),
                    choice.label,
                    p.child.id()
                )));
            }
        }
        // A GDS for KLayout: a running KLayout with klive takes it.
        if choice.id == "klayout-window"
            && let Some(f) = &file
            && matches!(Format::of_file(f), Some(Format::Gds))
            && crate::klayout::klive::send(f).is_ok()
        {
            return Ok(Launched::Reused(format!(
                "{} sent to the KLayout that runs (klive)",
                f.display()
            )));
        }
        let running = self
            .lock()
            .iter()
            .filter(|p| p.exit.is_none() && p.tool == choice.id)
            .count();
        if running >= MAX_PER_TOOL {
            bail!(
                "{} windows of {} run already (at most {MAX_PER_TOOL}): close one first",
                running,
                choice.label
            );
        }
        let mut command = command_for(project, choice, file.as_deref())?;
        let log = log_file(project, choice.id)?;
        let out = std::fs::File::create(&log)
            .wrap_err_with(|| format!("cannot write {}", log.display()))?;
        let err = out.try_clone()?;
        let words: Vec<String> = std::iter::once(command.get_program())
            .chain(command.get_args())
            .map(|w| w.to_string_lossy().into_owned())
            .collect();
        command
            .stdin(Stdio::null())
            .stdout(Stdio::from(out))
            .stderr(Stdio::from(err));
        let child = command
            .spawn()
            .wrap_err_with(|| format!("cannot start {}", words.join(" ")))?;
        let pid = child.id();
        tracing::info!(target: "cdept::tools", "{}: started {} (log {})", choice.label, words.join(" "), log.display());
        self.lock().push(Proc {
            tool: choice.id,
            file,
            command: words,
            child,
            log: log.clone(),
            started: SystemTime::now(),
            exit: None,
        });
        Ok(Launched::Started { pid, log })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Proc>> {
        self.procs.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Note the windows that have ended.
    pub fn reap(&self) {
        for p in self.lock().iter_mut().filter(|p| p.exit.is_none()) {
            if let Ok(Some(status)) = p.child.try_wait() {
                p.exit = Some(status.to_string());
                tracing::info!(target: "cdept::tools", "{} ended ({status}), log {}", p.tool, p.log.display());
            }
        }
    }

    pub fn processes(&self) -> Vec<ProcInfo> {
        self.reap();
        self.lock()
            .iter()
            .enumerate()
            .map(|(index, p)| ProcInfo {
                index,
                tool: p.tool,
                file: p.file.clone(),
                pid: p.child.id(),
                log: p.log.clone(),
                started: p.started,
                exit: p.exit.clone(),
            })
            .collect()
    }

    /// Stop window `index` (asked politely with SIGTERM, then killed).
    pub fn stop(&self, index: usize) -> Result<()> {
        let mut procs = self.lock();
        let p = procs
            .get_mut(index)
            .ok_or_else(|| eyre!("no window {index}"))?;
        if p.exit.is_none() {
            terminate(&mut p.child);
            p.exit = Some("stopped by CDePT".into());
        }
        Ok(())
    }

    /// Start window `index` again with the same command (after it ended or crashed).
    pub fn restart(&self, index: usize) -> Result<u32> {
        self.stop(index)?;
        let mut procs = self.lock();
        let p = procs
            .get_mut(index)
            .ok_or_else(|| eyre!("no window {index}"))?;
        let out = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&p.log)?;
        let err = out.try_clone()?;
        let mut command = Command::new(&p.command[0]);
        command
            .args(&p.command[1..])
            .stdin(Stdio::null())
            .stdout(Stdio::from(out))
            .stderr(Stdio::from(err));
        if let Some(dir) = p.file.as_deref().and_then(Path::parent) {
            command.current_dir(dir);
        }
        p.child = command.spawn().wrap_err("cannot start it again")?;
        p.exit = None;
        p.started = SystemTime::now();
        Ok(p.child.id())
    }

    /// Stop every window still running (CDePT quits).
    pub fn stop_all(&self) {
        for p in self.lock().iter_mut().filter(|p| p.exit.is_none()) {
            terminate(&mut p.child);
            p.exit = Some("stopped by CDePT".into());
        }
    }
}

fn terminate(child: &mut Child) {
    // SAFETY: plain libc call on our own child's pid.
    unsafe {
        libc_kill(child.id() as i32, 15);
    }
    for _ in 0..20 {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let _ = child.kill();
    let _ = child.wait();
}

unsafe extern "C" {
    #[link_name = "kill"]
    fn libc_kill(pid: i32, sig: i32) -> i32;
}

/// `<project>/.cdept/logs/<tool>-<n>.log` (a scratch folder without a project).
fn log_file(project: Option<&Project>, tool: &str) -> Result<PathBuf> {
    let dir = match project {
        Some(p) => p.root.join(".cdept").join("logs"),
        None => std::env::temp_dir().join("cdept-logs"),
    };
    std::fs::create_dir_all(&dir).wrap_err_with(|| format!("cannot create {}", dir.display()))?;
    for n in 1..100_000 {
        let f = dir.join(format!("{tool}-{n}.log"));
        if !f.exists() {
            return Ok(f);
        }
    }
    Err(eyre!("too many logs in {}", dir.display()))
}

fn require(tool: &str, env: &str, names: &[&str]) -> Result<PathBuf> {
    if let Some(p) = std::env::var_os(env).filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(p));
    }
    names
        .iter()
        .find_map(|n| crate::tools::find_in_path(n))
        .ok_or_else(|| eyre!("{tool} was not found on PATH (or set {env})"))
}

/// The command line of an external tool for `file`.
pub fn command_for(
    project: Option<&Project>,
    choice: Choice,
    file: Option<&Path>,
) -> Result<Command> {
    let mut cmd;
    match choice.id {
        "xschem-window" => {
            cmd = Command::new(require("xschem", "CDEPT_XSCHEM_PROGRAM", &["xschem"])?);
            if let Some(p) = project
                && let Some(rc) = p.xschem_rc.first()
            {
                cmd.arg("--rcfile").arg(rc);
            }
            if let Some(f) = file {
                cmd.arg(f);
            }
        }
        "klayout-window" => {
            cmd = Command::new(require("KLayout", "CDEPT_KLAYOUT", &["klayout"])?);
            cmd.arg("-e");
            if let Some(p) = project {
                let path: Vec<String> = p
                    .klayout_path()
                    .iter()
                    .map(|d| d.to_string_lossy().into_owned())
                    .collect();
                if !path.is_empty() {
                    cmd.env("KLAYOUT_PATH", path.join(":"));
                }
            }
            if let Some(f) = file {
                cmd.arg(f);
            }
        }
        "magic-window" => {
            cmd = Command::new(crate::magic::Magic::find().ok_or_else(|| {
                eyre!(
                    "magic was not found on PATH (or set {})",
                    crate::magic::MAGIC_ENV
                )
            })?);
            cmd.args(["-d", "XR"]);
            if let Some(p) = project
                && let Some(rc) = crate::magic::rc_file(p)
            {
                cmd.arg("-rcfile").arg(rc);
            }
            if let Some(f) = file {
                match Format::of_file(f) {
                    Some(Format::Gds) => {
                        cmd.arg(f);
                    }
                    _ => {
                        cmd.arg(f.with_extension(""));
                    }
                }
            }
        }
        "qucs-window" => {
            cmd = Command::new(crate::batch::qucs::ADAPTER.require()?);
            if let Some(f) = file {
                cmd.arg(f);
            }
        }
        "editor" => {
            cmd = Command::new(require("a text editor", "VISUAL", &["xdg-open"])?);
            if let Some(f) = file {
                cmd.arg(f);
            }
        }
        other => return Err(eyre!("no external tool '{other}'")),
    }
    if let Some(dir) = file.and_then(Path::parent).filter(|d| d.is_dir()) {
        cmd.current_dir(dir);
    }
    if let Some(p) = project {
        cmd.envs(p.tool_env().iter().map(|(k, v)| (k, v)));
    }
    Ok(cmd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_format_has_a_tool_and_layouts_have_a_choice() {
        for (_, format) in crate::formats::NAMES.iter() {
            assert!(!choices(*format).is_empty(), "{format:?}");
        }
        let gds = choices(Format::Gds);
        assert!(gds.iter().any(|c| c.id == "klayout") && gds.iter().any(|c| c.id == "magic"));
        assert_eq!(choices(Format::Magic)[0].id, "magic");
        assert_eq!(choice("magic-window"), Some(MAGIC_WINDOW));
        assert_eq!(choice("nope"), None);
    }

    #[test]
    fn a_window_is_started_logged_reused_and_stopped() {
        let tmp = tempfile::tempdir().unwrap();
        let _env = crate::testutil::hermetic_env();
        let editor = tmp.path().join("editor.sh");
        std::fs::write(&editor, "#!/bin/sh\necho opened \"$1\"\nsleep 30\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&editor, std::fs::Permissions::from_mode(0o755)).unwrap();
        // SAFETY: the hermetic guard serialises environment changes.
        unsafe { std::env::set_var("VISUAL", &editor) };
        let file = tmp.path().join("a.spice");
        std::fs::write(&file, "* x\n").unwrap();
        let l = Launcher::new();
        let first = l.open(None, EDITOR, Some(&file)).unwrap();
        let Launched::Started { log, .. } = first else {
            panic!("{first:?}")
        };
        // The same file again: not a second window.
        assert!(matches!(
            l.open(None, EDITOR, Some(&file)).unwrap(),
            Launched::Reused(_)
        ));
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(std::fs::read_to_string(&log).unwrap().contains("opened"));
        assert_eq!(l.processes().len(), 1);
        assert!(l.processes()[0].exit.is_none());
        l.stop(0).unwrap();
        assert!(l.processes()[0].exit.is_some());
        let pid = l.restart(0).unwrap();
        assert!(pid > 0);
        assert!(l.processes()[0].exit.is_none());
        // The limit per tool.
        for n in 0..MAX_PER_TOOL - 1 {
            let f = tmp.path().join(format!("{n}.v"));
            std::fs::write(&f, "").unwrap();
            l.open(None, EDITOR, Some(&f)).unwrap();
        }
        let f = tmp.path().join("over.v");
        std::fs::write(&f, "").unwrap();
        assert!(l.open(None, EDITOR, Some(&f)).is_err());
        l.stop_all();
        assert!(l.processes().iter().all(|p| p.exit.is_some()));
        unsafe { std::env::remove_var("VISUAL") };
        assert!(l.open(None, KLAYOUT_TAB, Some(&file)).is_err());
    }
}
