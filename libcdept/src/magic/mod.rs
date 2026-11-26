//! Magic in batch mode (docs/MAGIC.md): extraction to SPICE, DRC, LEF export and GDS
//! import.
//!
//! Magic (its licence is permissive, docs/MAGIC.md) runs as a process of its own:
//! `magic -dnull -noconsole -rcfile <rc>` with a Tcl script on its standard input, in
//! the PDK environment of the project ([`Project::tool_env`]). The scripts
//! ([`tcl`]) print `CDEPT:` lines that are read back ([`parse`]); a failed run is an
//! error that carries the tail of Magic's output.

pub mod embed;
pub mod flows;
pub mod link;
pub mod mag;
pub mod parse;
pub mod refs;
pub mod tcl;

use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use eyre::{Result, WrapErr, bail, eyre};
use serde::{Deserialize, Serialize};

pub use parse::{LefPin, LefSummary, SpiceSummary, Subckt, Violation};

use crate::{Format, Project};

/// Environment variable that names the Magic program.
pub const MAGIC_ENV: &str = "CDEPT_MAGIC";

/// Lines of Magic's output kept in an error.
const TAIL: usize = 25;

/// How long a batch run may take unless `[magic] timeout` or `--timeout` says otherwise.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// What Magic reads the cell from.
#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    /// A `.mag` file; its folder joins the search path for sub-cells.
    Mag { file: PathBuf },
    /// One cell of a GDSII file.
    Gds { file: PathBuf, cell: String },
}

impl Source {
    /// The layout view of `library/cell` of the project: a `.mag` view first, else a
    /// GDS one (the cell inside is `cell`, or the file's only top cell).
    pub fn of_cell(project: &Project, library: &str, cell: &str) -> Result<Source> {
        let found = project
            .library(library)
            .and_then(|l| l.cell(cell))
            .ok_or_else(|| eyre!("no cell {library}/{cell}"))?;
        let pick = |format: Format| {
            found
                .entries
                .iter()
                .find(|e| e.format == Some(format) && e.main_file.is_some())
        };
        if let Some(e) = pick(Format::Magic) {
            let file = e.main_file.clone().expect("checked");
            return Ok(Source::Mag { file });
        }
        if let Some(e) = pick(Format::Gds) {
            let file = e.main_file.clone().expect("checked");
            let cell = crate::klayout::verify::stream_top(&file, e.member_or(cell));
            return Ok(Source::Gds { file, cell });
        }
        Err(eyre!("{library}/{cell} has no Magic or GDS layout view"))
    }

    /// The name of the cell Magic works on.
    pub fn cell(&self) -> String {
        tcl::cell_name(self)
    }
}

/// Parasitics of an extraction.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Parasitics {
    /// Devices and connectivity only (`ext2spice lvs`).
    #[default]
    None,
    /// Substrate and coupling capacitances too.
    Capacitance,
}

#[derive(Debug, Clone, Default)]
pub struct ExtractOptions {
    /// The PDK's extraction style (`extract style`); the start-up file's default when `None`.
    pub style: Option<String>,
    pub parasitics: Parasitics,
}

/// The result of an extraction.
#[derive(Debug, Clone, PartialEq)]
pub struct Extraction {
    pub cell: String,
    pub spice: PathBuf,
    pub summary: SpiceSummary,
    /// Magic's output.
    pub log: String,
}

/// The result of a DRC run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DrcReport {
    pub cell: String,
    /// The DRC style Magic used.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    pub violations: Vec<Violation>,
    #[serde(skip)]
    pub log: String,
}

impl DrcReport {
    /// Number of error boxes.
    pub fn count(&self) -> usize {
        self.violations.iter().map(|v| v.boxes.len()).sum()
    }

    pub fn is_clean(&self) -> bool {
        self.violations.is_empty()
    }

    /// `(rule or message, boxes)`, the most frequent first.
    pub fn by_rule(&self) -> Vec<(String, usize)> {
        let mut rows: Vec<(String, usize)> = self
            .violations
            .iter()
            .map(|v| {
                (
                    v.rule.clone().unwrap_or_else(|| v.why.clone()),
                    v.boxes.len(),
                )
            })
            .collect();
        rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        rows
    }
}

/// The result of a LEF export.
#[derive(Debug, Clone, PartialEq)]
pub struct LefExport {
    pub cell: String,
    pub file: PathBuf,
    pub summary: Option<LefSummary>,
    pub log: String,
}

/// The result of a GDS import.
#[derive(Debug, Clone, PartialEq)]
pub struct Import {
    /// Cells read, all written as `<dir>/<cell>.mag`.
    pub cells: Vec<String>,
    pub log: String,
}

/// A Magic to run, with the start-up file and environment it runs in.
#[derive(Debug, Clone)]
pub struct Magic {
    pub program: PathBuf,
    /// The PDK's start-up file (`-rcfile`).
    pub rc: Option<PathBuf>,
    pub env: Vec<(OsString, OsString)>,
    /// A run that takes longer is killed ([`DEFAULT_TIMEOUT`]; `None`: no limit).
    pub timeout: Option<Duration>,
    /// Set to kill the running Magic (a job's [`crate::jobs::JobCtx::cancel_flag`]).
    pub cancel: Option<Arc<AtomicBool>>,
    /// Folders added to Magic's search path before every script (`addpath`): the
    /// folders of the project's Magic views, for cells placed without a path.
    pub search: Vec<PathBuf>,
}

impl Magic {
    /// `$CDEPT_MAGIC`, else `magic` on `PATH`.
    pub fn find() -> Option<PathBuf> {
        match std::env::var_os(MAGIC_ENV) {
            Some(p) if !p.is_empty() => Some(PathBuf::from(p)),
            _ => crate::tools::find_in_path("magic"),
        }
    }

    pub fn new(program: PathBuf) -> Magic {
        Magic {
            program,
            rc: None,
            env: Vec::new(),
            timeout: Some(DEFAULT_TIMEOUT),
            cancel: None,
            search: Vec::new(),
        }
    }

    /// This Magic with another time limit (zero: none).
    pub fn with_timeout(mut self, limit: Duration) -> Magic {
        self.timeout = (!limit.is_zero()).then_some(limit);
        self
    }

    /// This Magic, killed when the job is cancelled. Run from a job of the queue.
    pub fn in_job(mut self, ctx: &crate::jobs::JobCtx) -> Magic {
        self.cancel = Some(ctx.cancel_flag());
        self
    }

    /// Magic for `project`: the program found by [`Magic::find`], the start-up file of
    /// the snapshot's `[magic]` section (else `$PDK_ROOT/$PDK/libs.tech/magic/$PDK.magicrc`),
    /// the PDK environment and the time limit of `[magic] timeout` in `cdept.toml`.
    pub fn for_project(project: &Project) -> Result<Magic> {
        let program = Magic::find()
            .ok_or_else(|| eyre!("magic not found: put it on PATH or set {MAGIC_ENV}"))?;
        Ok(Magic {
            rc: rc_file(project),
            env: project.tool_env(),
            search: refs::search_dirs(project),
            ..Magic::new(program)
        }
        .with_timeout(project.magic.timeout.unwrap_or(DEFAULT_TIMEOUT)))
    }

    /// Run `script` in `dir` and return Magic's output (both streams). Errors: the
    /// program does not start, it fails, it reports a Tcl error, it ends before the
    /// script does, it runs out of time ([`Magic::timeout`]) or it is cancelled
    /// ([`Magic::cancel`]; then it is killed); each one carries the end of the output.
    pub fn run(&self, script: &str, dir: &Path) -> Result<String> {
        use crate::batch::{Waited, Watch};
        std::fs::create_dir_all(dir)?;
        let mut cmd = Command::new(&self.program);
        cmd.args(["-dnull", "-noconsole"]);
        if let Some(rc) = &self.rc {
            cmd.arg("-rcfile").arg(rc);
        }
        cmd.current_dir(dir)
            .envs(self.env.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd
            .spawn()
            .wrap_err_with(|| format!("cannot start {}", self.program.display()))?;
        let mut stdin = child.stdin.take().expect("piped");
        let mut script = script.to_string();
        if !self.search.is_empty() {
            let mut paths = String::new();
            for dir in &self.search {
                paths += &format!("catch {{addpath {}}}\n", tcl::word(&dir.to_string_lossy()));
            }
            script.insert_str(0, &paths);
        }
        // Written on a thread: a script bigger than the pipe must not block the reads.
        let writer = std::thread::spawn(move || {
            let _ = stdin.write_all(script.as_bytes());
        });
        // Read on threads too, so that the output so far is there when Magic is stopped.
        let (out, out_reader) = collect(child.stdout.take());
        let (err, err_reader) = collect(child.stderr.take());
        let watch = Watch {
            cancel: self.cancel.clone(),
            timeout: self.timeout,
        };
        let waited = watch.wait(&mut child)?;
        // After a kill, a process Magic started may keep the pipes open: do not wait
        // for it.
        if matches!(waited, Waited::Exited(_)) {
            let _ = writer.join();
            let _ = out_reader.join();
            let _ = err_reader.join();
        }
        let mut text = String::from_utf8_lossy(&out.lock().unwrap()).into_owned();
        text.push_str(&String::from_utf8_lossy(&err.lock().unwrap()));
        let status = match waited {
            Waited::Exited(status) => status,
            Waited::Cancelled => {
                return Err(eyre!(
                    "magic was cancelled and stopped\n{}",
                    parse::tail(&text, TAIL)
                ));
            }
            Waited::TimedOut(limit) => {
                return Err(eyre!(
                    "magic did not finish within {} and was stopped \
                     (raise [magic] timeout in cdept.toml, or --timeout)\n{}",
                    crate::jobs::format_duration(limit),
                    parse::tail(&text, TAIL)
                ));
            }
        };
        if let Some(err) = parse::script_error(&text) {
            bail!("magic: {err}\n{}", parse::tail(&text, TAIL));
        }
        if !status.success() || !parse::finished(&text) {
            bail!(
                "magic stopped before the end of the script ({})\n{}",
                status,
                parse::tail(&text, TAIL)
            );
        }
        Ok(text)
    }

    /// Extract `src` to SPICE: `<dir>/<cell>.spice`, with Magic's `.ext` files in `dir`.
    pub fn extract(
        &self,
        src: &Source,
        dir: &Path,
        options: &ExtractOptions,
    ) -> Result<Extraction> {
        let cell = src.cell();
        let spice = dir.join(format!("{cell}.spice"));
        let _ = std::fs::remove_file(&spice);
        let log = self.run(&tcl::extract(src, dir, &spice, options), dir)?;
        let text = std::fs::read_to_string(&spice).map_err(|e| {
            eyre!(
                "magic left no {}: {e}\n{}",
                spice.display(),
                parse::tail(&log, TAIL)
            )
        })?;
        Ok(Extraction {
            cell,
            spice,
            summary: parse::spice(&text),
            log,
        })
    }

    /// Check `src` against the PDK's rules (`style`: a DRC style, e.g. `drc(full)`).
    pub fn drc(&self, src: &Source, dir: &Path, style: Option<&str>) -> Result<DrcReport> {
        let log = self.run(&tcl::drc(src, style), dir)?;
        Ok(DrcReport {
            cell: src.cell(),
            style: parse::drc_style(&log),
            violations: parse::violations(&log),
            log,
        })
    }

    /// Write the LEF of `src` to `<dir>/<cell>.lef`.
    pub fn lef(&self, src: &Source, dir: &Path, hide: bool) -> Result<LefExport> {
        let cell = src.cell();
        let file = dir.join(format!("{cell}.lef"));
        let _ = std::fs::remove_file(&file);
        let log = self.run(&tcl::lef(src, &file, hide), dir)?;
        let text = std::fs::read_to_string(&file).map_err(|e| {
            eyre!(
                "magic left no {}: {e}\n{}",
                file.display(),
                parse::tail(&log, TAIL)
            )
        })?;
        Ok(LefExport {
            cell,
            file,
            summary: parse::lef(&text),
            log,
        })
    }

    /// Read every cell of a GDS file and write them as `.mag` files into `dir`.
    pub fn import_gds(&self, gds: &Path, dir: &Path) -> Result<Import> {
        let gds = std::path::absolute(gds)?;
        let log = self.run(&tcl::import_gds(&gds, dir), dir)?;
        let cells = parse::marked(&log, "cell")
            .into_iter()
            .map(String::from)
            .collect();
        Ok(Import { cells, log })
    }
}

/// Read a pipe to its end on a thread, into a buffer that can be looked at meanwhile.
fn collect(
    pipe: Option<impl Read + Send + 'static>,
) -> (Arc<Mutex<Vec<u8>>>, std::thread::JoinHandle<()>) {
    let buf = Arc::new(Mutex::new(Vec::new()));
    let b = buf.clone();
    let handle = std::thread::spawn(move || {
        let Some(mut pipe) = pipe else { return };
        let mut chunk = [0u8; 8192];
        while let Ok(n) = pipe.read(&mut chunk) {
            if n == 0 {
                break;
            }
            b.lock().unwrap().extend_from_slice(&chunk[..n]);
        }
    });
    (buf, handle)
}

/// The start-up file for the project's PDK.
pub fn rc_file(project: &Project) -> Option<PathBuf> {
    if let Some(rc) = project.snapshot().and_then(|t| t.magic.rc.as_ref())
        && let Ok(path) = rc.resolve(|n| project.var(n))
        && path.is_file()
    {
        return Some(path);
    }
    let pdk = project.pdk_dir()?;
    let rc = pdk
        .dir()
        .join("libs.tech/magic")
        .join(format!("{}.magicrc", pdk.name));
    rc.is_file().then_some(rc)
}

/// Where the files of a Magic job on `library/cell` go: `<project>/magic/<library>/<cell>/`.
pub fn out_dir(project: &Project, library: &str, cell: &str) -> PathBuf {
    project.root.join("magic").join(library).join(cell)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mag() -> Source {
        Source::Mag {
            file: "/x/lib/inv.mag".into(),
        }
    }

    #[test]
    fn extract_script() {
        let t = tcl::extract(
            &mag(),
            Path::new("/w"),
            Path::new("/w/inv.spice"),
            &ExtractOptions {
                style: Some("ngspice(si)".into()),
                parasitics: Parasitics::None,
            },
        );
        assert!(t.contains("addpath \"/x/lib\"\nload \"inv\""));
        assert!(t.contains("extract style \"ngspice(si)\""));
        assert!(t.contains("extract all\next2spice lvs"));
        assert!(t.contains("ext2spice -p \"/w\" -o \"/w/inv.spice\""));
        assert!(t.trim_end().ends_with("quit -noprompt"));
        assert!(t.contains("CDEPT: error $cdept_err"));
    }

    #[test]
    fn gds_scripts() {
        let src = Source::Gds {
            file: "/p/lib.gds".into(),
            cell: "a$b".into(),
        };
        let t = tcl::drc(&src, Some("drc(full)"));
        assert!(t.contains("gds read \"/p/lib.gds\""));
        assert!(t.contains("cellname list exists \"a\\$b\"] eq"));
        assert!(t.contains("drc style \"drc(full)\""));
        assert!(t.contains("units microns"));
        let l = tcl::lef(&src, Path::new("/w/c.lef"), true);
        assert!(l.contains("lef write \"/w/c.lef\" -hide"));
        let i = tcl::import_gds(Path::new("/p/lib.gds"), Path::new("/w"));
        assert!(i.contains("writeall force"));
    }

    #[test]
    fn capacitance_extraction() {
        let t = tcl::extract(
            &mag(),
            Path::new("/w"),
            Path::new("/w/inv.spice"),
            &ExtractOptions {
                style: None,
                parasitics: Parasitics::Capacitance,
            },
        );
        assert!(t.contains("extract do capacitance"));
        assert!(!t.contains("ext2spice lvs"));
    }

    #[test]
    fn report_counts() {
        let r = DrcReport {
            cell: "c".into(),
            style: None,
            violations: parse::violations(
                "CDEPT: drc\tA (a.1)\t0 0 1 1\nCDEPT: drc\tA (a.1)\t0 0 2 2\nCDEPT: drc\tB\t0 0 1 1\n",
            ),
            log: String::new(),
        };
        assert_eq!(r.count(), 3);
        assert!(!r.is_clean());
        assert_eq!(r.by_rule(), [("a.1".to_string(), 2), ("B".to_string(), 1)]);
    }

    #[cfg(unix)]
    fn fake_magic(dir: &Path, body: &str) -> Magic {
        Magic::new(crate::batch::testutil::fake(dir, "fake-magic", body))
    }

    #[cfg(unix)]
    #[test]
    fn a_hung_magic_is_stopped_with_its_last_lines() {
        let dir = tempfile::tempdir().unwrap();
        let magic = fake_magic(
            dir.path(),
            "echo 'Magic 8.3 starting'\necho 'Loading cell inv'\nexec sleep 30",
        )
        .with_timeout(Duration::from_millis(400));
        let t = std::time::Instant::now();
        let msg = format!("{}", magic.run("puts hi\n", dir.path()).unwrap_err());
        assert!(
            t.elapsed() < Duration::from_secs(10),
            "killed, not waited for"
        );
        assert!(
            msg.contains("did not finish within 400ms") && msg.contains("Loading cell inv"),
            "{msg}"
        );
        // A Magic that ends in time is not affected.
        let ok = fake_magic(dir.path(), "cat >/dev/null\necho 'CDEPT: done'")
            .with_timeout(Duration::from_secs(30));
        assert!(
            ok.run("puts hi\n", dir.path())
                .unwrap()
                .contains("CDEPT: done")
        );
        assert_eq!(Magic::new("m".into()).timeout, Some(DEFAULT_TIMEOUT));
        assert_eq!(
            Magic::new("m".into()).with_timeout(Duration::ZERO).timeout,
            None
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_cancelled_job_kills_magic() {
        use crate::jobs::{JobSpec, JobState, Queue};
        let dir = tempfile::tempdir().unwrap();
        let magic = fake_magic(dir.path(), "echo 'Magic 8.3 starting'\nexec sleep 30");
        let q = Queue::new(1);
        let error = Arc::new(Mutex::new(String::new()));
        let (e, d) = (error.clone(), dir.path().to_path_buf());
        let id = q.submit(JobSpec::func("drc", move |ctx| {
            let r = magic.in_job(ctx).run("drc check\n", &d);
            *e.lock().unwrap() = format!("{:#}", r.as_ref().unwrap_err());
            r.map(|_| ())
        }));
        let t = std::time::Instant::now();
        while q.info(id).unwrap().state != JobState::Running {
            std::thread::sleep(Duration::from_millis(5));
        }
        std::thread::sleep(Duration::from_millis(200));
        q.cancel(id);
        assert_eq!(q.wait(id).unwrap().state, JobState::Cancelled);
        assert!(
            t.elapsed() < Duration::from_secs(10),
            "killed, not waited for"
        );
        let msg = error.lock().unwrap().clone();
        assert!(
            msg.contains("cancelled") && msg.contains("Magic 8.3 starting"),
            "{msg}"
        );
    }

    // -- against a real Magic and PDK, when they are installed ------------------------

    /// A Magic and a PDK of this machine (the environment is the PDK's while the second lives),
    /// else `None` (the test is skipped).
    fn real(pdk: &str) -> Option<(Magic, crate::testutil::RealPdk)> {
        let program = Magic::find().or_else(|| {
            let p = PathBuf::from("/opt/ext/OpenLane/bin/magic");
            p.is_file().then_some(p)
        })?;
        let installed = crate::testutil::real_pdk(pdk)?;
        let root = installed.root.clone();
        let rc = root
            .join(pdk)
            .join(format!("libs.tech/magic/{pdk}.magicrc"));
        if !rc.is_file() {
            return None;
        }
        let mut m = Magic::new(program);
        m.rc = Some(rc);
        m.env = vec![
            ("PDK_ROOT".into(), root.clone().into()),
            ("PDK".into(), pdk.into()),
            ("PDKPATH".into(), root.join(pdk).into()),
        ];
        Some((m, installed))
    }

    fn inverter(root: &Path) -> Option<PathBuf> {
        let p = root.join("sky130A/libs.ref/sky130_fd_sc_hd/mag/sky130_fd_sc_hd__inv_1.mag");
        p.is_file().then_some(p)
    }

    #[test]
    fn real_extract_drc_lef() {
        let Some((magic, pdk)) = real("sky130A") else {
            eprintln!("skipped: no magic or sky130A");
            return;
        };
        let Some(file) = inverter(&pdk.root) else {
            eprintln!("skipped: no inverter in sky130A");
            return;
        };
        let tmp = tempfile::tempdir().unwrap();
        let src = Source::Mag { file };
        let e = magic
            .extract(&src, tmp.path(), &ExtractOptions::default())
            .unwrap();
        assert_eq!(e.cell, "sky130_fd_sc_hd__inv_1");
        assert_eq!(e.summary.subckts[0].name, "sky130_fd_sc_hd__inv_1");
        assert!(e.summary.subckts[0].ports.contains(&"Y".to_string()));
        assert_eq!(
            e.summary.devices.values().sum::<usize>(),
            2,
            "{:?}",
            e.summary
        );

        let d = magic.drc(&src, tmp.path(), None).unwrap();
        assert!(d.is_clean(), "{:?}", d.violations);
        assert!(d.style.is_some());

        let l = magic.lef(&src, tmp.path(), false).unwrap();
        let s = l.summary.unwrap();
        assert_eq!(s.name, "sky130_fd_sc_hd__inv_1");
        assert!(s.pins.iter().any(|p| p.name == "A"));
    }

    #[test]
    fn real_ihp_extract_from_gds() {
        let Some((magic, pdk)) = real("ihp-sg13g2") else {
            eprintln!("skipped: no magic or ihp-sg13g2");
            return;
        };
        let gds = pdk
            .dir
            .join("libs.ref/sg13g2_stdcell/gds/sg13g2_stdcell.gds");
        if !gds.is_file() {
            eprintln!("skipped: no standard cell GDS in ihp-sg13g2");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let src = Source::Gds {
            file: gds,
            cell: "sg13g2_inv_1".into(),
        };
        let e = magic
            .extract(&src, tmp.path(), &ExtractOptions::default())
            .unwrap();
        assert_eq!(e.summary.subckts[0].name, "sg13g2_inv_1");
        assert_eq!(
            e.summary.devices.values().sum::<usize>(),
            2,
            "{:?}",
            e.summary
        );
        assert!(magic.drc(&src, tmp.path(), None).unwrap().is_clean());
    }

    #[test]
    fn real_drc_finds_violations() {
        let Some((magic, _pdk)) = real("sky130A") else {
            eprintln!("skipped: no magic or sky130A");
            return;
        };
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("bad.mag");
        std::fs::write(
            &file,
            "magic\ntech sky130A\ntimestamp 1\n<< metal1 >>\nrect 0 0 100 100\nrect 110 0 210 100\nrect 300 0 305 100\n<< end >>\n",
        )
        .unwrap();
        let work = tmp.path().join("w");
        let d = magic.drc(&Source::Mag { file }, &work, None).unwrap();
        let rules = d.by_rule();
        assert!(rules.iter().any(|(r, _)| r == "met1.2"), "{rules:?}");
        assert!(rules.iter().any(|(r, _)| r == "met1.6"), "{rules:?}");
        assert!(d.count() >= 3);
    }

    #[test]
    fn real_gds_import_and_errors() {
        let Some((magic, pdk)) = real("sky130A") else {
            eprintln!("skipped: no magic or sky130A");
            return;
        };
        let Some(file) = inverter(&pdk.root) else {
            eprintln!("skipped: no inverter in sky130A");
            return;
        };
        let tmp = tempfile::tempdir().unwrap();
        // GDS made by Magic from the .mag, then read back.
        let gds = tmp.path().join("inv.gds");
        let script = format!(
            "load {}\ngds write {}\nputs \"CDEPT: done\"\nquit -noprompt\n",
            tcl::word(&file.with_extension("").to_string_lossy()),
            tcl::word(&gds.to_string_lossy())
        );
        magic.run(&script, tmp.path()).unwrap();
        assert!(gds.is_file());
        let out = tmp.path().join("mag");
        let imp = magic.import_gds(&gds, &out).unwrap();
        assert!(
            imp.cells.contains(&"sky130_fd_sc_hd__inv_1".to_string()),
            "{:?}",
            imp.cells
        );
        assert!(out.join("sky130_fd_sc_hd__inv_1.mag").is_file());

        // A cell that is not in the file is an error with Magic's output.
        let src = Source::Gds {
            file: gds,
            cell: "nope".into(),
        };
        let err = magic.lef(&src, &out, false).unwrap_err().to_string();
        assert!(err.contains("no such cell"), "{err}");
    }
}
