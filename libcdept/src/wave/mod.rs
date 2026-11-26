//! Waveforms: CDePT has no waveform viewer of its own. It uses UberWave, Symbiotic EDA's
//! viewer (the `uberwave` repository), in one of two ways ([`backend`]):
//!
//! * built into the cdept executable ([`embedded`], crate `cdept-uberwave`): the window
//!   runs on a thread of this process, commands and data are plain calls;
//! * as a separate process (`UberWaveViewer`), the fallback when it is not built in.
//!
//! The rest of this text is about the process: an older viewer takes one path and an optional `--analysis <name>` (`tran`, `ac`, `dc`,
//! `noise`, `op` for a multi-plot ngspice raw file): one raw file at a time. A viewer
//! with IPC protocol 1 (`--version` says `ipc 1`) takes several files and a socket
//! ([`ipc`], [`session`]): families of run points open in one window and the calculator
//! pushes traces into it ([`calc`]). Design: docs/howto/view-waveforms.md.

#[cfg(unix)]
pub mod backend;
#[cfg(unix)]
pub mod calc;
pub mod decimate;
#[cfg(unix)]
pub mod embedded;
#[cfg(unix)]
pub mod ipc;
#[cfg(unix)]
pub mod other;
#[cfg(unix)]
pub mod session;
#[cfg(unix)]
pub mod show;

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use eyre::{Result, WrapErr, bail, eyre};

use crate::Project;
use crate::raw::RawFile;
use crate::tools::find_in_path;

/// Environment variable that names the viewer program.
pub const UBERWAVE_ENV: &str = "CDEPT_UBERWAVE";
/// Binary names looked for on `PATH`.
pub const BINARIES: &[&str] = &["UberWaveViewer", "uberwave"];
/// Where the build instructions are.
pub const DOCS: &str = "docs/howto/view-waveforms.md";

/// Sibling builds of the `uberwave` repository, as a convenience for developers:
/// `<root>/../uberwave/build/wp5-viewer/UberWaveViewer` and `build*` next to it.
pub fn sibling_builds(root: &Path) -> Vec<PathBuf> {
    let Some(parent) = root.parent() else {
        return vec![];
    };
    let repo = parent.join("uberwave");
    let mut found = vec![repo.join("build/wp5-viewer/UberWaveViewer")];
    if let Ok(read) = std::fs::read_dir(&repo) {
        let mut others: Vec<PathBuf> = read
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("build"))
            .map(|e| e.path().join("wp5-viewer/UberWaveViewer"))
            .filter(|p| !found.contains(p))
            .collect();
        others.sort();
        found.extend(others);
    }
    found
}

/// The viewer: `$CDEPT_UBERWAVE`, else `PATH`, else a sibling build next to the project
/// or the CDePT repository.
pub fn locate(project: Option<&Project>) -> Option<PathBuf> {
    let mut roots: Vec<PathBuf> = project.map(|p| p.root.clone()).into_iter().collect();
    if let Some(repo) = Path::new(env!("CARGO_MANIFEST_DIR")).parent() {
        roots.push(repo.to_path_buf());
    }
    locate_with(|n| std::env::var_os(n), find_in_path, &roots)
}

/// [`locate`] with the environment, the `PATH` search and the roots given.
pub fn locate_with(
    var: impl Fn(&str) -> Option<OsString>,
    on_path: impl Fn(&str) -> Option<PathBuf>,
    roots: &[PathBuf],
) -> Option<PathBuf> {
    if let Some(p) = var(UBERWAVE_ENV).filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(p));
    }
    if let Some(p) = BINARIES.iter().find_map(|b| on_path(b)) {
        return Some(p);
    }
    roots
        .iter()
        .flat_map(|r| sibling_builds(r))
        .find(|p| p.is_file())
}

/// [`locate`], or an error that says how to get the viewer.
pub fn require(project: Option<&Project>) -> Result<PathBuf> {
    locate(project).ok_or_else(|| {
        eyre!(
            "UberWave (UberWaveViewer) not found: build it (cmake -B build -DCMAKE_BUILD_TYPE=Release && \
             cmake --build build --target UberWaveViewer, in the uberwave repository), put it on PATH or set \
             {UBERWAVE_ENV}; see {DOCS}"
        )
    })
}

/// The `cdept tools` line of UberWave: `built in (embedded)`, `process: <path>`, or
/// `not available`.
pub fn tool_line(project: Option<&Project>) -> String {
    #[cfg(unix)]
    if backend::embedded_available() {
        let version = cdept_uberwave::version().unwrap_or_default();
        return format!("built in (embedded)  {version}");
    }
    match locate(project) {
        Some(p) => {
            let version = describe(&p).map(|v| format!("{v}  ")).unwrap_or_default();
            format!("process: {version}({})", p.display())
        }
        None => "not available".into(),
    }
}

/// What the "version" column of `cdept tools` shows: the viewer's own `--version` line
/// (with `ipc 1` when it has IPC), else, for a viewer that does not answer it (an older
/// one: the probe never opens a window and is killed after 2 s), the age of the file.
pub fn describe(program: &Path) -> Option<String> {
    #[cfg(unix)]
    if let Some(v) = session::version(program) {
        return Some(v.line);
    }
    let modified = std::fs::metadata(program).ok()?.modified().ok()?;
    let when: chrono::DateTime<chrono::Local> = modified.into();
    Some(format!("found, file of {}", when.format("%Y-%m-%d")))
}

/// One analysis (plot) of a raw file.
#[derive(Debug, Clone, PartialEq)]
pub struct Analysis {
    pub index: usize,
    /// `tran`, `ac`, `dc`, `noise`, `op`, `other`: the name UberWave's `--analysis` knows.
    pub kind: String,
    /// As the simulator wrote it (`Transient Analysis`).
    pub name: String,
    pub signals: usize,
    pub points: usize,
}

/// The analyses of a raw file, by CDePT's own reader.
pub fn analyses(raw: &Path) -> Result<Vec<Analysis>> {
    let file = RawFile::open(raw)?;
    Ok(file
        .plots()
        .iter()
        .enumerate()
        .map(|(index, p)| Analysis {
            index,
            kind: p.kind().to_string(),
            name: p.name.clone(),
            signals: p.vars.len(),
            points: p.points,
        })
        .collect())
}

/// The `--analysis` to pass: the one asked for (a name or kind of the file's analyses,
/// or anything for a file CDePT cannot read, such as a PSF folder), else for a file
/// with several analyses the first transient one (the first one when there is none).
/// `None` for a single analysis.
pub fn analysis_arg(found: &[Analysis], asked: Option<&str>) -> Result<Option<String>> {
    if let Some(a) = asked {
        let hit = found
            .iter()
            .find(|x| x.kind.eq_ignore_ascii_case(a) || x.name.eq_ignore_ascii_case(a));
        return match hit {
            Some(x) => Ok(Some(x.kind.clone())),
            None if found.is_empty() => Ok(Some(a.to_string())),
            None => Err(eyre!(
                "the file has no analysis '{a}' (it has: {})",
                found
                    .iter()
                    .map(|x| x.kind.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        };
    }
    if found.len() < 2 {
        return Ok(None);
    }
    Ok(found
        .iter()
        .find(|a| a.kind == "tran")
        .or(found.first())
        .map(|a| a.kind.clone()))
}

/// The command line: `<program> <raw> [--analysis <name>]`.
pub fn command_line(program: &Path, raw: &Path, analysis: Option<&str>) -> Vec<OsString> {
    let mut line = vec![
        program.as_os_str().to_os_string(),
        absolute(raw).into_os_string(),
    ];
    if let Some(a) = analysis {
        line.push("--analysis".into());
        line.push(a.into());
    }
    line
}

fn absolute(p: &Path) -> PathBuf {
    std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf())
}

/// What was started.
#[derive(Debug, Clone)]
pub struct LaunchInfo {
    pub command: Vec<OsString>,
    /// Opened in the viewer built into this process (no command line).
    pub embedded: bool,
}

impl LaunchInfo {
    pub fn text(&self) -> String {
        if self.embedded {
            return "opened in the built-in UberWave".into();
        }
        self.command
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Start the viewer on `raw` and return at once; it keeps running after this process.
/// Its working folder is the folder of the file. `analysis` as for [`analysis_arg`].
pub fn open_raw(
    project: Option<&Project>,
    raw: &Path,
    analysis: Option<&str>,
) -> Result<LaunchInfo> {
    if !raw.exists() {
        bail!("{} does not exist", raw.display());
    }
    #[cfg(unix)]
    if backend::choose(project)? == backend::Backend::Embedded {
        let tag = raw
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "file".into());
        session::open_files(
            project,
            "files",
            &[session::ViewFile {
                tag,
                raw: raw.to_path_buf(),
            }],
            analysis,
            session::Mode::Open,
            &show::ShowRequest::default(),
        )?;
        return Ok(LaunchInfo {
            command: vec![],
            embedded: true,
        });
    }
    let program = require(project)?;
    let found = if raw.is_file() {
        analyses(raw).unwrap_or_default()
    } else {
        vec![]
    };
    let analysis = analysis_arg(&found, analysis)?;
    let line = command_line(&program, raw, analysis.as_deref());
    spawn_detached(
        &line,
        absolute(raw).parent().filter(|d| d.is_dir()),
        project,
    )?;
    Ok(LaunchInfo {
        command: line,
        embedded: false,
    })
}

pub(crate) fn spawn_detached(
    line: &[OsString],
    dir: Option<&Path>,
    project: Option<&Project>,
) -> Result<()> {
    use std::process::{Command, Stdio};
    let mut cmd = Command::new(&line[0]);
    cmd.args(&line[1..]);
    if let Some(d) = dir {
        cmd.current_dir(d);
    }
    if let Some(p) = project {
        cmd.envs(p.tool_env().iter().map(|(k, v)| (k, v)));
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Its own process group: a Ctrl-C or the end of this process leaves it running.
        cmd.process_group(0);
    }
    let child = cmd
        .spawn()
        .wrap_err_with(|| format!("cannot start {}", line[0].to_string_lossy()))?;
    // Not waited for: it outlives us.
    drop(child);
    Ok(())
}

/// A run point of a testbench with a raw file.
#[derive(Debug, Clone, PartialEq)]
pub struct PointRaw {
    pub point: String,
    pub raw: PathBuf,
}

/// The testbench views of `library/cell` that have run points with a raw file.
pub fn views_with_runs(project: &Project, library: &str, cell: &str) -> Vec<String> {
    let base = project.root.join("simulations").join(library).join(cell);
    let mut views: Vec<String> = std::fs::read_dir(base)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir() && !points_in(&e.path()).is_empty())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    views.sort();
    views
}

/// The run points of a testbench view that have an `out.raw`, by name.
pub fn points(project: &Project, library: &str, cell: &str, view: &str) -> Vec<PointRaw> {
    points_in(&crate::sim::runs_dir(project, library, cell, view))
}

fn points_in(dir: &Path) -> Vec<PointRaw> {
    let mut points: Vec<PointRaw> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| PointRaw {
            point: e.file_name().to_string_lossy().into_owned(),
            raw: e.path().join("out.raw"),
        })
        .filter(|p| p.raw.is_file())
        .collect();
    points.sort_by(|a, b| a.point.cmp(&b.point));
    points
}

/// The view to use: the one named, else the only testbench view with runs.
pub fn pick_view(
    project: &Project,
    library: &str,
    cell: &str,
    view: Option<&str>,
) -> Result<String> {
    if let Some(v) = view {
        return Ok(v.to_string());
    }
    let mut views = views_with_runs(project, library, cell);
    match views.len() {
        0 => Err(eyre!(
            "{library}/{cell} has no simulation results (run its testbench first)"
        )),
        1 => Ok(views.remove(0)),
        _ => Err(eyre!(
            "{library}/{cell} has results in several views ({}): pass --view",
            views.join(", ")
        )),
    }
}

/// The raw file of one point; `point` None: the only one, an error that lists them when
/// there are several.
pub fn pick_point(
    project: &Project,
    library: &str,
    cell: &str,
    view: &str,
    point: Option<&str>,
) -> Result<PointRaw> {
    let all = points(project, library, cell, view);
    match point {
        Some(p) => {
            if let Some(hit) = all.iter().find(|x| x.point == p) {
                return Ok(hit.clone());
            }
            Err(eyre!(
                "no run point '{p}' with a raw file in {library}/{cell}/{view} (points: {})",
                if all.is_empty() {
                    "none, run the testbench first".to_string()
                } else {
                    names(&all)
                }
            ))
        }
        None => match all.len() {
            0 => Err(eyre!(
                "{library}/{cell}/{view} has no run point with a raw file (run the testbench first)"
            )),
            1 => Ok(all.into_iter().next().unwrap()),
            n => Err(eyre!(
                "{library}/{cell}/{view} has {n} run points: pass --point, or --family to open them all in \
                 one viewer. Points: {}",
                names(&all)
            )),
        },
    }
}

/// Names of points, comma separated.
pub fn names(points: &[PointRaw]) -> String {
    points
        .iter()
        .map(|p| p.point.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Open one run point (the GUI's button and script command): in the viewer of the run
/// when one runs and has IPC, else in a new one. `show` as for [`session::open_files`]:
/// [`default_show`] is what the buttons use.
#[cfg(unix)]
pub fn open_point(
    project: &Project,
    library: &str,
    cell: &str,
    view: &str,
    point: &str,
    show: &show::ShowRequest,
) -> Result<session::Opened> {
    let p = pick_point(project, library, cell, view, Some(point))?;
    session::open_files(
        Some(project),
        &session::run_key(library, cell, view),
        &[(&p).into()],
        None,
        session::Mode::Open,
        show,
    )
}

/// What the viewer plots by default for the testbench view `library/cell/view`: the
/// signals its `save` names, else the ones its measures use, else the first voltages.
#[cfg(unix)]
pub fn default_show(project: &Project, library: &str, cell: &str, view: &str) -> show::ShowRequest {
    show::ShowRequest::for_testbench(project, library, cell, view, show::Show::Default)
}

/// Open the raw file of a point already picked (the viewer of the run is reused).
#[cfg(unix)]
pub fn open_point_raw(
    project: &Project,
    library: &str,
    cell: &str,
    view: &str,
    point: &PointRaw,
    analysis: Option<&str>,
    show: &show::ShowRequest,
) -> Result<session::Opened> {
    session::open_files(
        Some(project),
        &session::run_key(library, cell, view),
        &[point.into()],
        analysis,
        session::Mode::Open,
        show,
    )
}

/// The points of a view to open together: `chosen` (an error for an unknown one), or all
/// of them when empty.
pub fn select_points(
    project: &Project,
    library: &str,
    cell: &str,
    view: &str,
    chosen: &[String],
) -> Result<Vec<PointRaw>> {
    let all = points(project, library, cell, view);
    if all.is_empty() {
        return Err(eyre!(
            "{library}/{cell}/{view} has no run point with a raw file (run the testbench first)"
        ));
    }
    if chosen.is_empty() {
        return Ok(all);
    }
    chosen
        .iter()
        .map(|n| {
            all.iter().find(|p| &p.point == n).cloned().ok_or_else(|| {
                eyre!(
                    "no run point '{n}' with a raw file in {library}/{cell}/{view} (points: {})",
                    names(&all)
                )
            })
        })
        .collect()
}

/// Open several run points in one viewer, tags = point names (the GUI's "Open family"
/// and `cdept wave --family`).
#[cfg(unix)]
pub fn open_family(
    project: &Project,
    library: &str,
    cell: &str,
    view: &str,
    chosen: &[PointRaw],
    analysis: Option<&str>,
    show: &show::ShowRequest,
) -> Result<session::Opened> {
    let files: Vec<session::ViewFile> = chosen.iter().map(Into::into).collect();
    session::open_files(
        Some(project),
        &session::run_key(library, cell, view),
        &files,
        analysis,
        session::Mode::Open,
        show,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none(_: &str) -> Option<PathBuf> {
        None
    }

    fn fake_build(root: &Path, dir: &str) -> PathBuf {
        let p = root
            .join("uberwave")
            .join(dir)
            .join("wp5-viewer/UberWaveViewer");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "x").unwrap();
        p
    }

    #[test]
    fn the_variable_wins_then_path_then_a_sibling_build() {
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("proj");
        std::fs::create_dir_all(&proj).unwrap();
        let sibling = fake_build(tmp.path(), "build");
        let roots = [proj];
        let env = |n: &str| (n == UBERWAVE_ENV).then(|| OsString::from("/x/viewer"));
        let on_path = |b: &str| (b == "uberwave").then(|| PathBuf::from("/usr/bin/uberwave"));

        assert_eq!(
            locate_with(env, on_path, &roots),
            Some(PathBuf::from("/x/viewer"))
        );
        assert_eq!(
            locate_with(|_| None, on_path, &roots),
            Some(PathBuf::from("/usr/bin/uberwave"))
        );
        assert_eq!(locate_with(|_| None, none, &roots), Some(sibling.clone()));
        // An empty variable does not count.
        let empty = |n: &str| (n == UBERWAVE_ENV).then(OsString::new);
        assert_eq!(locate_with(empty, none, &roots), Some(sibling));
        assert_eq!(
            locate_with(|_| None, none, &[tmp.path().join("nowhere/x")]),
            None
        );
    }

    #[test]
    fn other_build_folders_are_tried_in_order() {
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("proj");
        std::fs::create_dir_all(&proj).unwrap();
        let b = fake_build(tmp.path(), "build-release");
        assert_eq!(
            locate_with(|_| None, none, std::slice::from_ref(&proj)),
            Some(b)
        );
        let plain = fake_build(tmp.path(), "build");
        assert_eq!(locate_with(|_| None, none, &[proj]), Some(plain));
    }

    fn an(kind: &str, name: &str) -> Analysis {
        Analysis {
            index: 0,
            kind: kind.into(),
            name: name.into(),
            signals: 3,
            points: 10,
        }
    }

    #[test]
    fn analysis_choice() {
        let two = [
            an("op", "Operating Point"),
            an("tran", "Transient Analysis"),
        ];
        assert_eq!(analysis_arg(&[an("tran", "T")], None).unwrap(), None);
        assert_eq!(analysis_arg(&two, None).unwrap().as_deref(), Some("tran"));
        assert_eq!(
            analysis_arg(&two, Some("op")).unwrap().as_deref(),
            Some("op")
        );
        assert_eq!(
            analysis_arg(&two, Some("transient analysis"))
                .unwrap()
                .as_deref(),
            Some("tran")
        );
        assert!(
            analysis_arg(&two, Some("ac"))
                .unwrap_err()
                .to_string()
                .contains("op, tran")
        );
        // A PSF folder is not read by CDePT: the name goes through.
        assert_eq!(
            analysis_arg(&[], Some("runTran-tran")).unwrap().as_deref(),
            Some("runTran-tran")
        );
    }

    #[test]
    fn the_command_line() {
        let line = command_line(
            Path::new("/b/UberWaveViewer"),
            Path::new("/p/out.raw"),
            Some("tran"),
        );
        let text: Vec<_> = line
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            text,
            ["/b/UberWaveViewer", "/p/out.raw", "--analysis", "tran"]
        );
        assert_eq!(
            command_line(Path::new("v"), Path::new("/p/out.raw"), None).len(),
            2
        );
    }

    #[test]
    fn points_of_a_view() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("simulations/l/c/tb");
        for p in ["tt_27", "ss_27"] {
            std::fs::create_dir_all(base.join(p)).unwrap();
            std::fs::write(base.join(p).join("out.raw"), "x").unwrap();
        }
        std::fs::create_dir_all(base.join("ff_27")).unwrap(); // failed run: no raw
        let got = points_in(&base);
        assert_eq!(
            got.iter().map(|p| p.point.as_str()).collect::<Vec<_>>(),
            ["ss_27", "tt_27"]
        );
        assert_eq!(names(&got), "ss_27, tt_27");
    }
}
