//! UberWave in the Simulation tab: the waveform viewer is a separate program
//! (`libcdept::wave`). The Results area has "Open family in UberWave" (all points in one
//! viewer, with a viewer that has IPC) and an expression line: the calculator evaluates
//! the expression on the selected point or on each point and the waveforms go to the
//! viewer as traces; scalars and errors are shown under the line.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use egui::{Id, Ui};
use libcdept::Project;
use libcdept::calc::funcs;
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

/// The result of a send in its thread: the lines to show and whether it went wrong.
type Slot = Arc<Mutex<Option<(Vec<String>, bool)>>>;

/// What the expression line remembers between frames (in egui's memory, per testbench).
#[derive(Clone, Default)]
struct State {
    expr: String,
    /// `None`: every point.
    point: Option<String>,
    lines: Vec<String>,
    bad: bool,
    /// The send in progress, in its thread.
    pending: Option<Slot>,
}

/// The widgets in the Results area, for the testbench `run` (library, cell, view) and
/// the names of the points that left a raw file.
pub(crate) fn results_ui<'a>(
    ui: &mut Ui,
    project: &Project,
    run: (&str, &str, &str),
    points: impl Iterator<Item = &'a str>,
) {
    let points: Vec<&str> = points.collect();
    if points.is_empty() {
        return;
    }
    let id = Id::new(("uberwave-calc", run.0, run.1, run.2));
    let mut st: State = ui.data_mut(|d| d.get_temp(id)).unwrap_or_default();
    if let Some(slot) = st.pending.clone()
        && let Some((lines, bad)) = slot.lock().unwrap().take()
    {
        st.lines = lines;
        st.bad = bad;
        st.pending = None;
    }
    let busy = st.pending.is_some();
    let target = st.point.clone().filter(|p| points.contains(&p.as_str()));
    ui.horizontal(|ui| {
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
        egui::ComboBox::from_id_salt("uw-calc-point")
            .selected_text(target.clone().unwrap_or_else(|| "all points".into()))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut st.point, None, "all points");
                for p in &points {
                    ui.selectable_value(&mut st.point, Some(p.to_string()), *p);
                }
            });
    });
    let mut send = false;
    ui.horizontal(|ui| {
        let edit = ui.add(
            egui::TextEdit::singleline(&mut st.expr)
                .hint_text("calculator expression, e.g. vdb(v(\"out\"))")
                .desired_width(ui.available_width() - 70.0),
        );
        if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            send = true;
        }
        if ui
            .add_enabled(
                !busy && !st.expr.trim().is_empty(),
                egui::Button::new("Send"),
            )
            .on_hover_text("Evaluate on the point(s) and send the waveforms to UberWave as traces")
            .clicked()
        {
            send = true;
        }
    });
    suggestions(ui, &mut st.expr);
    if send && !busy && !st.expr.trim().is_empty() {
        let slot: Slot = Arc::default();
        st.pending = Some(slot.clone());
        st.lines = vec!["sending...".into()];
        let (project, expr, chosen) = (project.clone(), st.expr.clone(), target);
        let (l, c, v) = (run.0.to_string(), run.1.to_string(), run.2.to_string());
        let ctx = ui.ctx().clone();
        std::thread::spawn(move || {
            let out = calc_lines(&project, (&l, &c, &v), chosen.as_deref(), &expr);
            *slot.lock().unwrap() = Some(out);
            ctx.request_repaint();
        });
    }
    if busy {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(100));
    }
    for line in &st.lines {
        if st.bad {
            ui.colored_label(ui.visuals().error_fg_color, line);
        } else {
            ui.label(line);
        }
    }
    ui.data_mut(|d| d.insert_temp(id, st));
}

/// The function names that start like the word at the end of the expression, as a row
/// of small buttons; a click completes the word.
fn suggestions(ui: &mut Ui, expr: &mut String) {
    let start = expr
        .rfind(|c: char| !(c.is_alphanumeric() || c == '_'))
        .map_or(0, |i| i + 1);
    let word = expr[start..].to_lowercase();
    if word.len() < 2 {
        return;
    }
    let mut names: Vec<&str> = funcs::all()
        .iter()
        .flat_map(|f| f.names.iter().copied())
        .filter(|n| n.starts_with(&word) && *n != word)
        .collect();
    names.sort_unstable();
    names.dedup();
    if names.is_empty() {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        for n in names.into_iter().take(10) {
            if ui.small_button(n).clicked() {
                expr.truncate(start);
                expr.push_str(n);
                expr.push('(');
            }
        }
    });
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
         wave expect_request <cmd> [text] | wave expect_trace <name> <n|>n> | wave quit_viewer <lib> <cell> <view>";
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
        _ => Err(USAGE.into()),
    }
}
