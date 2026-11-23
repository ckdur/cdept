//! The calculator feeding the viewer: an expression is evaluated on the raw file of a
//! run point (or of each point of a family) and the waveform results are pushed to
//! UberWave as traces (`add_trace`). Scalars are only reported.
//!
//! A complex waveform is sent as two traces, `<name> (mag dB)` (20 log10 |y|) and
//! `<name> (phase)` (degrees, unwrapped), since the viewer's plots are real.

use std::path::Path;

use eyre::{Result, eyre};

use super::ipc::Trace;
use super::session::{Mode, ViewFile, open_files, run_key};
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
}

/// Evaluate `expr` on a point's raw file. The environment has the point's `corner`,
/// `temp`, `seed`, `run` and swept variables when its `result.json` is next to the
/// raw file. `plot` limits the search to one analysis (as `cdept calc --plot`).
pub fn evaluate_raw(raw_file: &Path, expr: &str, plot: Option<&str>) -> Result<Value> {
    let raw = RawFile::open(raw_file)?;
    let mut env = Env::new(&raw);
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
    calc::evaluate(expr, &env).map_err(|e| eyre!("{}", e.render(expr)))
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
    let u = match kind.to_lowercase().as_str() {
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
    let single = points.len() == 1;
    points
        .iter()
        .map(|p| PointOutcome {
            point: p.point.clone(),
            result: evaluate_raw(&p.raw, expr, plot)
                .map(|v| to_evaluated(&v, &trace_name(expr, name, &p.point, single)))
                .map_err(|e| format!("{e:#}")),
        })
        .collect()
}

/// What to evaluate: the expression, a name for its traces, the analysis to look in.
#[derive(Debug, Clone, Default)]
pub struct Request {
    pub expr: String,
    pub name: Option<String>,
    pub plot: Option<String>,
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
    let outcomes = evaluate_points(points, expr, name, plot);
    let mut sent = Sent {
        outcomes,
        pushed: vec![],
        viewer: None,
        problem: None,
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
    if traces.is_empty() {
        return sent;
    }
    let files: Vec<ViewFile> = points.iter().map(ViewFile::from).collect();
    let opened = match open_files(
        Some(project),
        &run_key(library, cell, view),
        &files,
        plot_analysis(points, plot),
        Mode::Connect,
    ) {
        Ok(o) => o,
        Err(e) => {
            sent.problem = Some(format!("{e:#}"));
            return sent;
        }
    };
    sent.viewer = Some(opened.text());
    let Some(mut client) = opened.client else {
        sent.problem = opened.old_viewer_note();
        return sent;
    };
    let mut pushed = Vec::new();
    for t in traces {
        // A trace of that name from an earlier send is replaced.
        let _ = client.remove_trace(&t.name);
        match client.add_trace(t) {
            Ok(()) => pushed.push((t.name.clone(), t.x.len())),
            Err(e) => {
                sent.problem = Some(format!("{e:#}"));
                break;
            }
        }
    }
    sent.pushed = pushed;
    sent
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
