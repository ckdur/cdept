//! The calculator feeding the viewer: an expression is evaluated on the raw file of a
//! run point (or of each point of a family) and the waveform results are pushed to
//! UberWave as traces (`add_trace`). Scalars are only reported.
//!
//! A complex waveform is sent as two traces, `<name> (mag dB)` (20 log10 |y|) and
//! `<name> (phase)` (degrees, unwrapped), since the viewer's plots are real.

use std::path::Path;

use eyre::{Result, eyre};

use super::backend::ViewerOps;
use super::decimate;
use super::ipc::{PaneSel, Trace};
use super::session::{Mode, ViewFile, open_files, run_key};
use super::show::{Show, ShowRequest};
use super::{PointRaw, analyses};
use crate::Project;
use crate::calc::{self, Env, Value, Waveform};
use crate::raw::RawFile;
use crate::sim::results::{RESULT_FILE, RunRecord};

/// What an expression gave on one point.
#[derive(Debug, Clone, PartialEq)]
pub enum Evaluated {
    /// A number, complex number or string, as text.
    Scalar(String),
    /// A waveform: one trace, two for a complex one.
    Traces(Vec<Trace>),
}

/// The evaluation of one point.
#[derive(Debug, Clone, PartialEq)]
pub struct PointOutcome {
    pub point: String,
    pub result: std::result::Result<Evaluated, String>,
    /// Where the expression measured (crossings, edges, extremes): markers in the viewer.
    pub marks: Vec<calc::Mark>,
}

/// Evaluate `expr` on a point's raw file. The environment has the point's `corner`,
/// `temp`, `seed`, `run` and swept variables when its `result.json` is next to the
/// raw file. `plot` limits the search to one analysis (as `cdept calc --plot`).
pub fn evaluate_raw(raw_file: &Path, expr: &str, plot: Option<&str>) -> Result<Value> {
    evaluate_raw_marked(raw_file, expr, plot).map(|(v, _)| v)
}

/// [`evaluate_raw`] and the places it measured at.
pub fn evaluate_raw_marked(
    raw_file: &Path,
    expr: &str,
    plot: Option<&str>,
) -> Result<(Value, Vec<calc::Mark>)> {
    evaluate_raw_with(raw_file, expr, plot, &[])
}

/// [`evaluate_raw_marked`] with more names (`cursor_a`, `cursor_b`, ...).
pub fn evaluate_raw_with(
    raw_file: &Path,
    expr: &str,
    plot: Option<&str>,
    extra: &[(String, f64)],
) -> Result<(Value, Vec<calc::Mark>)> {
    let raw = RawFile::open(raw_file)?;
    let mut env = Env::new(&raw);
    for (k, v) in extra {
        env.set(k, Value::Num(*v));
    }
    if let Some(p) = plot {
        env = env.with_plot(p);
    }
    if let Some(rec) = raw_file
        .parent()
        .and_then(|d| RunRecord::read(&d.join(RESULT_FILE)).ok())
    {
        for (k, v) in &rec.point.variables {
            env.set_text(k, v);
        }
        env.set("corner", Value::Str(rec.point.corner.clone()));
        if let Some(t) = rec.point.temperature {
            env.set("temp", Value::Num(t));
            env.set("temperature", Value::Num(t));
        }
        if let Some(s) = rec.point.seed {
            env.set("seed", Value::Num(s as f64));
        }
        if let Some(r) = rec.point.run {
            env.set("run", Value::Num(r as f64));
        }
    }
    match calc::evaluate(expr, &env) {
        Ok(v) => Ok((v, env.take_marks())),
        Err(first) => {
            // `v(out)` for `v("out")`: tried only when the expression as written failed,
            // so what is valid keeps its meaning (a bare name there is a variable).
            if let Some(quoted) = quote_bare_nodes(expr) {
                env.take_marks();
                if let Ok(v) = calc::evaluate(&quoted, &env) {
                    return Ok((v, env.take_marks()));
                }
            }
            let mut text = first.render(expr);
            if text.contains("unknown name") && expr.contains('(') {
                text.push_str(&format!("\n  hint: {QUOTE_HINT}"));
            }
            Err(eyre!("{text}"))
        }
    }
}

/// What to tell when a signal is written without quotes.
pub const QUOTE_HINT: &str = "write node names in quotes: v(\"out\"), i(\"vdd\")";

/// `expr` with the bare names of `v( )` and `i( )` calls quoted (`v(out)` as `v("out")`,
/// `i(x1.vdd)` as `i("x1.vdd")`), or `None` when there is none. Only a call with one
/// plain name inside is touched.
pub fn quote_bare_nodes(expr: &str) -> Option<String> {
    let chars: Vec<char> = expr.chars().collect();
    let mut out = String::new();
    let mut changed = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let call = matches!(c, 'v' | 'V' | 'i' | 'I')
            && (i == 0 || !(chars[i - 1].is_alphanumeric() || chars[i - 1] == '_'))
            && chars.get(i + 1) == Some(&'(');
        if call && let Some(close) = chars[i + 2..].iter().position(|c| *c == ')') {
            let inner: String = chars[i + 2..i + 2 + close].iter().collect();
            let name = inner.trim();
            let plain = !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_alphanumeric() || "_.:#$[]/<>!".contains(c));
            if plain {
                out.push_str(&format!("{c}(\"{name}\")"));
                changed = true;
                i += 2 + close + 1;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    changed.then_some(out)
}

/// The trace name of a point: `name` as given when it is the only point, else
/// `<name or expression> @ <point>`.
pub fn trace_name(expr: &str, name: Option<&str>, point: &str, single: bool) -> String {
    match name {
        Some(n) if single => n.to_string(),
        Some(n) => format!("{n} @ {point}"),
        None => format!("{} @ {point}", expr.trim()),
    }
}

/// A value as what the viewer gets: traces for a waveform, text otherwise.
pub fn to_evaluated(value: &Value, name: &str) -> Evaluated {
    match value {
        Value::Wave(w) => Evaluated::Traces(wave_traces(w, name)),
        other => Evaluated::Scalar(other.display()),
    }
}

/// The raw file names a vector's kind (`time`, `voltage`), the viewer wants a unit.
fn si_unit(kind: &str) -> Option<String> {
    let lower = kind.to_lowercase();
    // ngspice writes `frequency grid=3`, `time grid=..`: the first word is the kind
    let first = lower.split_whitespace().next().unwrap_or("");
    let u = match first {
        "" => return None,
        "time" => "s",
        "voltage" => "V",
        "current" => "A",
        "frequency" => "Hz",
        _ => kind,
    };
    Some(u.to_string())
}

fn wave_traces(w: &Waveform, name: &str) -> Vec<Trace> {
    let x = w.x.to_vec();
    let x_unit = si_unit(&w.x_unit);
    if !w.y.is_complex() {
        let mut t = Trace::new(name, x, w.y.re());
        t.unit = si_unit(&w.unit);
        t.x_unit = x_unit;
        return vec![t];
    }
    let ys: Vec<_> = (0..w.len()).map(|i| w.y.at(i)).collect();
    let mag: Vec<f64> = ys.iter().map(|c| 20.0 * c.abs().log10()).collect();
    let mut phase: Vec<f64> = ys.iter().map(|c| c.arg().to_degrees()).collect();
    unwrap_degrees(&mut phase);
    let mut m = Trace::new(format!("{name} (mag dB)"), x.clone(), mag);
    m.unit = Some("dB".into());
    m.x_unit = x_unit.clone();
    let mut p = Trace::new(format!("{name} (phase)"), x, phase);
    p.unit = Some("deg".into());
    p.x_unit = x_unit;
    vec![m, p]
}

/// Remove the 360 degree jumps of a phase in degrees.
fn unwrap_degrees(p: &mut [f64]) {
    let mut offset = 0.0;
    let mut prev = match p.first() {
        Some(v) => *v,
        None => return,
    };
    for v in p.iter_mut().skip(1) {
        let raw = *v;
        let d = raw - prev;
        if d > 180.0 {
            offset -= 360.0 * ((d + 180.0) / 360.0).floor();
        } else if d < -180.0 {
            offset += 360.0 * ((-d + 180.0) / 360.0).floor();
        }
        prev = raw;
        *v = raw + offset;
    }
}

/// Evaluate `expr` on every point.
pub fn evaluate_points(
    points: &[PointRaw],
    expr: &str,
    name: Option<&str>,
    plot: Option<&str>,
) -> Vec<PointOutcome> {
    evaluate_points_with(points, expr, name, plot, &[])
}

/// [`evaluate_points`] with more names for the expression.
pub fn evaluate_points_with(
    points: &[PointRaw],
    expr: &str,
    name: Option<&str>,
    plot: Option<&str>,
    extra: &[(String, f64)],
) -> Vec<PointOutcome> {
    let single = points.len() == 1;
    points
        .iter()
        .map(|p| match evaluate_raw_with(&p.raw, expr, plot, extra) {
            Ok((v, marks)) => PointOutcome {
                point: p.point.clone(),
                result: Ok(to_evaluated(&v, &trace_name(expr, name, &p.point, single))),
                marks,
            },
            Err(e) => PointOutcome {
                point: p.point.clone(),
                result: Err(format!("{e:#}")),
                marks: vec![],
            },
        })
        .collect()
}

/// What to evaluate: the expression, a name for its traces, the analysis to look in.
#[derive(Debug, Clone, Default)]
pub struct Request {
    pub expr: String,
    pub name: Option<String>,
    pub plot: Option<String>,
    /// What a viewer started for the push plots (`None`: [`Show::Default`]).
    pub show: Option<Show>,
    /// Traces with more points are decimated: `None` the project's `[wave] max_points`
    /// (default [`decimate::DEFAULT_MAX_POINTS`]), `Some(0)` never.
    pub max_points: Option<usize>,
    /// Put markers where scalar results were measured (embedded UberWave only).
    pub markers: bool,
    /// How the traces are laid out in the viewer.
    pub layout: Layout,
    /// More names for the expression (`cursor_a`, `cursor_b` from the viewer's cursors).
    pub extra: Vec<(String, f64)>,
}

/// How calculator traces are laid out.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum Layout {
    /// Into the viewer's active pane (a complex result as `(mag dB)` and `(phase)`).
    #[default]
    Plain,
    /// A Bode plot: magnitudes in one new pane, phases in another, both on log X.
    Bode,
    /// An eye diagram: every `period` from `offset` on drawn on top of the others, in a
    /// new pane.
    Eye { period: f64, offset: f64 },
}

/// Arrange the traces of a send for `layout`: panes, log X, folding.
pub fn lay_out(traces: Vec<Trace>, layout: Layout) -> Vec<Trace> {
    match layout {
        Layout::Plain => traces,
        Layout::Bode => {
            let (mut mags, mut phases): (Vec<Trace>, Vec<Trace>) = traces
                .into_iter()
                .partition(|t| !t.name.ends_with(" (phase)"));
            for (i, t) in mags.iter_mut().enumerate() {
                t.x_log = true;
                t.pane = if i == 0 {
                    PaneSel::New
                } else {
                    PaneSel::Default
                };
            }
            for (i, t) in phases.iter_mut().enumerate() {
                t.x_log = true;
                t.pane = if i == 0 {
                    PaneSel::New
                } else {
                    PaneSel::Default
                };
            }
            mags.extend(phases);
            mags
        }
        Layout::Eye { period, offset } => traces
            .into_iter()
            .enumerate()
            .map(|(i, mut t)| {
                t.fold = Some((period, offset));
                t.pane = if i == 0 {
                    PaneSel::New
                } else {
                    PaneSel::Default
                };
                // its own X axis (0..period), not linked to the time axis of other panes
                t.x_unit = Some(format!("{} (folded)", t.x_unit.as_deref().unwrap_or("s")));
                t
            })
            .collect(),
    }
}

/// What [`send`] did.
#[derive(Debug, Clone, PartialEq)]
pub struct Sent {
    pub outcomes: Vec<PointOutcome>,
    /// Traces the viewer accepted: name and number of points.
    pub pushed: Vec<(String, usize)>,
    /// A one-line description of the viewer used (started, or reused).
    pub viewer: Option<String>,
    /// Why the traces could not be sent (no viewer, an old one, a refusal), if so.
    pub problem: Option<String>,
    /// The viewer is part of this executable (the process stays to keep its window).
    pub embedded: bool,
    /// Things worth a line: a trace that was decimated, a signal that was not shown.
    pub notes: Vec<String>,
}

impl Sent {
    /// The scalar results as a text table, `point<TAB>value`, and the errors as
    /// `point<TAB>error: message`.
    pub fn table(&self) -> String {
        let mut lines = Vec::new();
        for o in &self.outcomes {
            match &o.result {
                Ok(Evaluated::Scalar(s)) => lines.push(format!("{}\t{s}", o.point)),
                Ok(Evaluated::Traces(_)) => {}
                Err(e) => lines.push(format!("{}\terror: {}", o.point, e.replace('\n', " | "))),
            }
        }
        lines.join("\n")
    }
}

/// Evaluate `expr` on `points` of a view and push the waveforms to the viewer of that
/// run: the running one is used as it is, else one is started on the points' raw files
/// (tags = point names). Never fails for a bad expression: the outcomes say it.
pub fn send(
    project: &Project,
    library: &str,
    cell: &str,
    view: &str,
    points: &[PointRaw],
    req: &Request,
) -> Sent {
    let (expr, name, plot) = (req.expr.as_str(), req.name.as_deref(), req.plot.as_deref());
    let outcomes = evaluate_points_with(points, expr, name, plot, &req.extra);
    let mut sent = Sent {
        outcomes,
        pushed: vec![],
        viewer: None,
        problem: None,
        embedded: false,
        notes: vec![],
    };
    let traces: Vec<&Trace> = sent
        .outcomes
        .iter()
        .filter_map(|o| match &o.result {
            Ok(Evaluated::Traces(t)) => Some(t),
            _ => None,
        })
        .flatten()
        .collect();
    // Markers go to the built-in viewer only, and start no viewer by themselves.
    let embedded = matches!(
        super::backend::choose(Some(project)),
        Ok(super::backend::Backend::Embedded)
    );
    let markers = if embedded && (!traces.is_empty() || super::embedded::running().is_some()) {
        markers_of(&sent.outcomes, expr, name, req.markers)
    } else {
        vec![]
    };
    if traces.is_empty() && markers.is_empty() {
        return sent;
    }
    let files: Vec<ViewFile> = points.iter().map(ViewFile::from).collect();
    let opened = match open_files(
        Some(project),
        &run_key(library, cell, view),
        &files,
        plot_analysis(points, plot),
        Mode::Connect,
        &ShowRequest::for_testbench(
            project,
            library,
            cell,
            view,
            req.show.clone().unwrap_or_default(),
        ),
    ) {
        Ok(o) => o,
        Err(e) => {
            sent.problem = Some(format!("{e:#}"));
            return sent;
        }
    };
    sent.viewer = Some(opened.text());
    sent.embedded = opened.embedded;
    sent.notes.extend(opened.notes.iter().cloned());
    let Some(mut client) = opened.client else {
        sent.problem = opened.old_viewer_note();
        return sent;
    };
    let limit = req.max_points.unwrap_or(project.wave.max_points);
    let mut pushed = Vec::new();
    let traces = lay_out(traces.into_iter().cloned().collect(), req.layout);
    for t in &traces {
        // Only a socket needs thinned traces: the embedded viewer takes the raw slices.
        let t = if client.wants_decimation() {
            decimated(t, limit, &mut sent.notes)
        } else {
            t.clone()
        };
        // A trace of that name from an earlier send is replaced.
        let _ = client.remove_trace(&t.name);
        match client.add_trace(&t) {
            Ok(()) => pushed.push((t.name.clone(), t.x.len())),
            Err(e) => {
                sent.problem = Some(format!("{e:#}"));
                break;
            }
        }
    }
    sent.pushed = pushed;
    if !markers.is_empty() {
        let _ = client.clear_markers(Some(MARKER_GROUP));
        for m in &markers {
            if let Err(e) = client.add_marker(m) {
                sent.notes.push(format!("markers: {e:#}"));
                break;
            }
        }
    }
    sent
}

/// The group of the calculator's markers in the viewer (replaced by each send).
pub const MARKER_GROUP: &str = "cdept-calc";

/// The markers of the scalar results: where each point's expression measured, labelled
/// with the name (or expression), the point when there are several, and the value.
pub fn markers_of(
    outcomes: &[PointOutcome],
    expr: &str,
    name: Option<&str>,
    wanted: bool,
) -> Vec<super::embedded::Marker> {
    if !wanted {
        return vec![];
    }
    let single = outcomes.len() == 1;
    let mut out = vec![];
    for o in outcomes {
        let Ok(Evaluated::Scalar(value)) = &o.result else {
            continue;
        };
        let label = name.unwrap_or(expr.trim());
        for (i, m) in o.marks.iter().enumerate().take(50) {
            let mut text = if single {
                format!("{label} {}", m.what)
            } else {
                format!("{label} {} @ {}", m.what, o.point)
            };
            if i + 1 == o.marks.len().min(50) {
                text.push_str(&format!(" = {value}"));
            }
            out.push(super::embedded::Marker {
                group: MARKER_GROUP.into(),
                label: text,
                x: m.x,
                y: m.y,
                pane: None,
                rgba: 0,
            });
        }
    }
    out
}

/// The trace as it goes to the viewer: itself, or its min/max envelope (name with
/// [`decimate::SUFFIX`]) when it has more than `limit` points; the note says so.
fn decimated(t: &Trace, limit: usize, notes: &mut Vec<String>) -> Trace {
    match decimate::min_max(&t.x, &t.y, limit) {
        Some((x, y)) => {
            let mut d = t.clone();
            notes.push(format!(
                "'{}' has {} points: sent as a min/max envelope of {} points (--no-decimate or [wave] max_points to change)",
                t.name,
                t.x.len(),
                x.len()
            ));
            d.name = format!("{}{}", t.name, decimate::SUFFIX);
            d.x = x;
            d.y = y;
            d
        }
        None => t.clone(),
    }
}

/// The analysis to preselect when the viewer is started for a calculator push: only
/// when asked (the expression may need a plot the transient default hides).
fn plot_analysis<'a>(points: &[PointRaw], plot: Option<&'a str>) -> Option<&'a str> {
    let first = points.first()?;
    let found = analyses(&first.raw).unwrap_or_default();
    plot.filter(|p| {
        found
            .iter()
            .any(|a| a.kind.eq_ignore_ascii_case(p) || a.name.eq_ignore_ascii_case(p))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calc::C;

    #[test]
    fn names() {
        assert_eq!(trace_name("v(out)", None, "tt_27", true), "v(out) @ tt_27");
        assert_eq!(trace_name("v(out)", Some("vo"), "tt_27", true), "vo");
        assert_eq!(
            trace_name("v(out)", Some("vo"), "tt_27", false),
            "vo @ tt_27"
        );
    }

    #[test]
    fn a_real_waveform_is_one_trace_and_a_complex_one_two() {
        let mut w = Waveform::real(vec![0.0, 1.0], vec![1.0, 2.0]);
        w.unit = "voltage".into();
        w.x_unit = "time".into();
        let Evaluated::Traces(t) = to_evaluated(&Value::Wave(w), "a") else {
            panic!()
        };
        assert_eq!(t.len(), 1);
        assert_eq!(
            (
                t[0].name.as_str(),
                t[0].unit.as_deref(),
                t[0].x_unit.as_deref()
            ),
            ("a", Some("V"), Some("s"))
        );

        let c = Waveform::complex(
            vec![1.0, 2.0, 3.0],
            vec![
                C { re: 10.0, im: 0.0 },
                C { re: 0.0, im: 1.0 },
                C {
                    re: -1.0,
                    im: -0.01,
                },
            ],
        );
        let Evaluated::Traces(t) = to_evaluated(&Value::Wave(c), "h") else {
            panic!()
        };
        assert_eq!(
            t.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
            ["h (mag dB)", "h (phase)"]
        );
        assert!((t[0].y[0] - 20.0).abs() < 1e-9 && t[0].y[1].abs() < 1e-9);
        // 0, 90, then -179.4 which unwrapped is 180.6
        assert!(
            (t[1].y[1] - 90.0).abs() < 1e-9 && (t[1].y[2] - 180.57).abs() < 0.01,
            "{:?}",
            t[1].y
        );
    }

    #[test]
    fn scalars_are_text() {
        assert_eq!(
            to_evaluated(&Value::Num(1.5), "x"),
            Evaluated::Scalar("1.500000e0".into())
        );
    }
}
