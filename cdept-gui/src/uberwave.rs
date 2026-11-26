//! UberWave in the Simulation tab (`libcdept::wave`): "Open in UberWave" on a point,
//! "Open family in UberWave" (all points in one viewer), and the script commands `wave
//! ...`. The calculator panel is `views/simulation/calc.rs`.

use std::path::{Path, PathBuf};

use egui::Ui;
use libcdept::Project;
use libcdept::wave::{self, PointRaw, calc as wcalc};

/// Start UberWave on the `out.raw` of a run point folder: the action of the button. The
/// folder is `simulations/<lib>/<cell>/<view>/<point>`; a viewer of that run that is
/// open already gets the file (IPC), else a new one is started.
pub(crate) fn open_dir(project: &Project, dir: &Path) -> Result<String, String> {
    let raw = dir.join("out.raw");
    if !raw.is_file() {
        return Err(format!(
            "{} has no out.raw (the run left no raw file)",
            dir.display()
        ));
    }
    let opened = match run_of(project, dir) {
        Some((lib, cell, view, _)) => {
            let point = PointRaw {
                point: file_name(dir),
                raw,
            };
            wave::open_point_raw(
                project,
                &lib,
                &cell,
                &view,
                &point,
                None,
                &wave::default_show(project, &lib, &cell, &view),
            )
        }
        None => wave::session::open_files(
            Some(project),
            "files",
            &[wave::session::ViewFile {
                tag: file_name(dir),
                raw,
            }],
            None,
            wave::session::Mode::Open,
            &wave::show::ShowRequest::default(),
        ),
    }
    .map_err(|e| format!("{e:#}"))?;
    Ok(with_note(opened.text(), opened.old_viewer_note()))
}

fn with_note(line: String, note: Option<String>) -> String {
    match note {
        Some(n) => format!("{line} (note: {n})"),
        None => line,
    }
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// `(library, cell, view, point)` of a run point folder under the project's `simulations/`.
fn run_of(project: &Project, dir: &Path) -> Option<(String, String, String, String)> {
    let rel = dir.strip_prefix(project.root.join("simulations")).ok()?;
    let parts: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    match parts.as_slice() {
        [l, c, v, p] => Some((l.clone(), c.clone(), v.clone(), p.clone())),
        _ => None,
    }
}

/// The button: failures (no viewer, with how to get it; no raw file) go to the log. In a
/// thread: a viewer that is started is waited for to plot the default signals.
pub(crate) fn open_run_dir(project: &Project, dir: &Path) {
    let (project, dir) = (project.clone(), dir.to_path_buf());
    std::thread::spawn(move || match open_dir(&project, &dir) {
        Ok(line) => tracing::info!(target: "cdept::sim", "UberWave: {line}"),
        Err(e) => tracing::warn!(target: "cdept::sim", "Open in UberWave: {e}"),
    });
}

fn points_of(
    project: &Project,
    run: (&str, &str, &str),
    chosen: Option<&str>,
) -> Result<Vec<PointRaw>, String> {
    let names: Vec<String> = chosen.map(String::from).into_iter().collect();
    wave::select_points(project, run.0, run.1, run.2, &names).map_err(|e| format!("{e:#}"))
}

/// Open all the points (or the one named) of a view in one viewer.
pub(crate) fn open_family(
    project: &Project,
    run: (&str, &str, &str),
    chosen: Option<&str>,
) -> Result<String, String> {
    let points = points_of(project, run, chosen)?;
    let show = wave::default_show(project, run.0, run.1, run.2);
    let opened = wave::open_family(project, run.0, run.1, run.2, &points, None, &show)
        .map_err(|e| format!("{e:#}"))?;
    Ok(with_note(opened.text(), opened.old_viewer_note()))
}

/// Evaluate `expr` on the points and send the waveforms. The lines to show: the scalars
/// (`point: value`), the traces sent, the errors; the flag says something went wrong.
pub(crate) fn calc_lines(
    project: &Project,
    run: (&str, &str, &str),
    chosen: Option<&str>,
    expr: &str,
) -> (Vec<String>, bool) {
    let points = match points_of(project, run, chosen) {
        Ok(p) => p,
        Err(e) => return (vec![e], true),
    };
    let req = wcalc::Request {
        expr: expr.to_string(),
        ..Default::default()
    };
    let sent = wcalc::send(project, run.0, run.1, run.2, &points, &req);
    let mut lines = Vec::new();
    let mut bad = false;
    for o in &sent.outcomes {
        match &o.result {
            Ok(wcalc::Evaluated::Scalar(s)) => lines.push(format!("{}: {s}", o.point)),
            Ok(wcalc::Evaluated::Traces(_)) => {}
            Err(e) => {
                bad = true;
                lines.push(format!("{}: {}", o.point, e.replace('\n', " | ")));
            }
        }
    }
    for (name, n) in &sent.pushed {
        lines.push(format!("sent '{name}' ({n} points)"));
    }
    if let Some(p) = &sent.problem {
        bad = true;
        lines.push(format!("not sent: {p}"));
    }
    (lines, bad)
}

/// The "Open family in UberWave" button of the Results area, for the testbench `run`
/// (library, cell, view) and the names of the points that left a raw file. The
/// calculator is the panel of `views/simulation/calc.rs`.
pub(crate) fn results_ui<'a>(
    ui: &mut Ui,
    project: &Project,
    run: (&str, &str, &str),
    points: impl Iterator<Item = &'a str>,
) {
    if points.count() == 0 {
        return;
    }
    if ui
        .button("Open family in UberWave")
        .on_hover_text("All the points of this run in one UberWave window (needs a viewer with IPC; an older one opens the first)")
        .clicked()
    {
        let (project, run) = (
            project.clone(),
            (run.0.to_string(), run.1.to_string(), run.2.to_string()),
        );
        std::thread::spawn(
            move || match open_family(&project, (&run.0, &run.1, &run.2), None) {
                Ok(line) => tracing::info!(target: "cdept::sim", "UberWave: {line}"),
                Err(e) => tracing::warn!(target: "cdept::sim", "Open family in UberWave: {e}"),
            },
        );
    }
}

/// The fake viewer's log: `$FAKE_UBERWAVE_LOG`.
fn fake_log() -> Result<PathBuf, String> {
    std::env::var_os("FAKE_UBERWAVE_LOG")
        .map(PathBuf::from)
        .ok_or_else(|| "FAKE_UBERWAVE_LOG is not set (the log of fake-uberwave-ipc.py)".to_string())
}

/// The records of the fake's log whose `key` object exists, as JSON text lines.
fn log_records(key: &str) -> Result<Vec<serde_json::Value>, String> {
    let file = fake_log()?;
    let text = std::fs::read_to_string(&file)
        .map_err(|e| format!("cannot read {}: {e}", file.display()))?;
    Ok(text
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter_map(|v| v.get(key).cloned())
        .collect())
}

/// Wait up to 5 s for a record to appear (a viewer started a moment ago).
fn wait_for<T>(
    mut find: impl FnMut() -> Result<Option<T>, String>,
    what: &str,
) -> Result<T, String> {
    let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if let Some(v) = find()? {
            return Ok(v);
        }
        if std::time::Instant::now() >= end {
            return Err(format!("the fake viewer's log has no {what}"));
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// `wave ...` of a test script (documented in docs/GUI-TESTING.md): the same actions as
/// the buttons, and checks of what a fake viewer (`CDEPT_UBERWAVE`) recorded.
pub(crate) fn script(project: Option<&Project>, args: &[String]) -> Result<(), String> {
    const USAGE: &str = "usage: wave open <lib> <cell> <view> <point> | wave family <lib> <cell> <view> [point] | \
         wave calc <lib> <cell> <view> <point|all> <expr> | wave expect_start <text> | \
         wave expect_request <cmd> [text] | wave expect_trace <name> <n|>n> | wave quit_viewer <lib> <cell> <view> | \
         wave backend <embedded|process> | wave embedded_files <tag>... | wave embedded_signal <file> <name> | \
         wave embedded_screenshot <png> | wave embedded_close";
    let project_or = || project.ok_or("no project is open");
    match args {
        [open, lib, cell, view, point] if open == "open" => {
            let dir = libcdept::sim::runs_dir(project_or()?, lib, cell, view).join(point);
            let line = open_dir(project_or()?, &dir)?;
            tracing::info!(target: "cdept::script", "wave: {line}");
            Ok(())
        }
        [family, lib, cell, view, rest @ ..] if family == "family" && rest.len() <= 1 => {
            let line = open_family(
                project_or()?,
                (lib, cell, view),
                rest.first().map(String::as_str),
            )?;
            tracing::info!(target: "cdept::script", "wave: {line}");
            Ok(())
        }
        [calc, lib, cell, view, point, expr @ ..] if calc == "calc" && !expr.is_empty() => {
            let chosen = (point != "all").then_some(point.as_str());
            let (lines, bad) =
                calc_lines(project_or()?, (lib, cell, view), chosen, &expr.join(" "));
            for l in &lines {
                tracing::info!(target: "cdept::script", "wave calc: {l}");
            }
            if bad { Err(lines.join("; ")) } else { Ok(()) }
        }
        [expect, text] if expect == "expect_start" => wait_for(
            || {
                Ok(log_records("start")?
                    .into_iter()
                    .find(|r| r.to_string().contains(text.as_str())))
            },
            &format!("start with '{text}'"),
        )
        .map(drop),
        [expect, cmd, text @ ..] if expect == "expect_request" => {
            let text = text.join(" ");
            wait_for(
                || {
                    Ok(log_records("request")?
                        .into_iter()
                        .find(|r| r["cmd"] == cmd.as_str() && r.to_string().contains(&text)))
                },
                &format!("request '{cmd}' with '{text}'"),
            )
            .map(drop)
        }
        [expect, rest @ .., n] if expect == "expect_trace" && !rest.is_empty() => {
            // The name may hold spaces (`v("out") @ tt_27`); `N` points, or `>N` for more than N.
            let name = rest.join(" ");
            let (more, digits) = n
                .strip_prefix('>')
                .map_or((false, n.as_str()), |d| (true, d));
            let n: u64 = digits
                .parse()
                .map_err(|_| format!("'{n}' is not a number"))?;
            wait_for(
                || {
                    Ok(log_records("request")?.into_iter().find(|r| {
                        let got = r["n"].as_u64().unwrap_or(0);
                        r["cmd"] == "add_trace"
                            && r["name"] == name.as_str()
                            && if more { got > n } else { got == n }
                    }))
                },
                &format!("add_trace '{name}' with {n} points"),
            )
            .map(drop)
        }
        [quit, lib, cell, view] if quit == "quit_viewer" => {
            let project = project_or()?;
            let socket =
                wave::ipc::socket_path(&project.root, &wave::session::run_key(lib, cell, view));
            let mut client =
                wave::ipc::Client::connect_once(&socket).map_err(|e| format!("{e:#}"))?;
            client.quit().map_err(|e| format!("{e:#}"))
        }
        [backend, name] if backend == "backend" => {
            let b = wave::backend::Backend::parse(name)
                .ok_or_else(|| format!("'{name}' is not a backend (embedded, process)"))?;
            wave::backend::set_override(Some(b));
            Ok(())
        }
        [files, tags @ ..] if files == "embedded_files" && !tags.is_empty() => {
            let viewer = embedded_viewer()?;
            let files = viewer.list_files().map_err(|e| format!("{e:#}"))?;
            let have: Vec<&str> = files.iter().map(|f| f.tag.as_str()).collect();
            for t in tags {
                if !have.contains(&t.as_str()) {
                    return Err(format!(
                        "the embedded viewer has no file '{t}' (it has {})",
                        have.join(", ")
                    ));
                }
            }
            Ok(())
        }
        [signal, file, name @ ..] if signal == "embedded_signal" && !name.is_empty() => {
            let viewer = embedded_viewer()?;
            let name = name.join(" ");
            let list = viewer
                .signals(Some(file), None)
                .map_err(|e| format!("{e:#}"))?;
            if list.signals.iter().any(|s| s.name == name) {
                Ok(())
            } else {
                let have: Vec<&str> = list.signals.iter().map(|s| s.name.as_str()).collect();
                Err(format!(
                    "the embedded viewer has no signal '{name}' in '{file}' (it has {})",
                    have.join(", ")
                ))
            }
        }
        [shot, png] if shot == "embedded_screenshot" => {
            let viewer = embedded_viewer()?;
            let path = std::path::absolute(png).map_err(|e| e.to_string())?;
            viewer.screenshot(&path).map_err(|e| format!("{e:#}"))?;
            check_png(&path)
        }
        [close] if close == "embedded_close" => {
            wave::embedded::close();
            Ok(())
        }
        _ => Err(USAGE.into()),
    }
}

fn embedded_viewer() -> Result<wave::embedded::Shared, String> {
    wave::embedded::running().ok_or_else(|| "no embedded UberWave window is open".to_string())
}

/// The screenshot is a PNG with something drawn in it: not (nearly) all dark.
fn check_png(path: &Path) -> Result<(), String> {
    let img = image::open(path)
        .map_err(|e| format!("{} is not a PNG: {e}", path.display()))?
        .to_rgb8();
    let bright = img
        .pixels()
        .filter(|p| p.0.iter().any(|c| *c > 150))
        .count();
    if bright < 200 {
        return Err(format!(
            "{} looks empty: {bright} bright pixels of {}",
            path.display(),
            img.width() * img.height()
        ));
    }
    tracing::info!(target: "cdept::script", "wave: screenshot {} ({}x{}, {bright} bright pixels)", path.display(), img.width(), img.height());
    Ok(())
}
