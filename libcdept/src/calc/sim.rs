//! The calculator in a simulation run: a testbench's `[[output]]` expressions are
//! evaluated on the run's raw file and their numbers join the `.meas` measurements.

use std::collections::BTreeMap;
use std::path::Path;

use eyre::Result;

use super::{Env, Value};
use crate::raw::RawFile;
use crate::sim::deck::RunPoint;
use crate::sim::run::Outcome;
use crate::sim::setup::{Output, Setup};

/// The environment of a run point: `corner`, `temp`, `seed`, `run`, the setup's
/// `[variables]` (swept values win) and the `.meas` results so far.
pub fn point_env<'a>(
    raw: &'a RawFile,
    setup: &Setup,
    point: &RunPoint,
    measures: &BTreeMap<String, f64>,
) -> Env<'a> {
    let mut env = Env::new(raw);
    for (k, v) in &setup.variables {
        env.set_text(k, v);
    }
    for (k, v) in &point.variables {
        env.set_text(k, v);
    }
    for (k, v) in measures {
        env.set(k, Value::Num(*v));
    }
    env.set("corner", Value::Str(point.corner.clone()));
    if let Some(t) = point.temperature {
        env.set("temp", Value::Num(t));
        env.set("temperature", Value::Num(t));
    }
    if let Some(s) = point.seed {
        env.set("seed", Value::Num(s as f64));
    }
    if let Some(r) = point.run {
        env.set("run", Value::Num(r as f64));
    }
    env
}

/// Evaluate the outputs on a raw file. Returns the scalars (by lower-case name) and
/// the errors (output name, message with the position).
pub fn evaluate_outputs(
    outputs: &[Output],
    setup: &Setup,
    point: &RunPoint,
    raw_file: &Path,
    measures: &BTreeMap<String, f64>,
) -> (BTreeMap<String, f64>, BTreeMap<String, String>) {
    let (mut values, mut errors) = (BTreeMap::new(), BTreeMap::new());
    if outputs.is_empty() {
        return (values, errors);
    }
    let raw = match RawFile::open(raw_file) {
        Ok(r) => r,
        Err(e) => {
            for o in outputs {
                errors.insert(o.name.to_lowercase(), format!("{e:#}"));
            }
            return (values, errors);
        }
    };
    let mut known = measures.clone();
    for o in outputs {
        let mut env = point_env(&raw, setup, point, &known);
        if !o.plot.is_empty() {
            env = env.with_plot(o.plot.clone());
        }
        let key = o.name.to_lowercase();
        match super::evaluate(&o.expr, &env) {
            Ok(v) => match v.as_f64() {
                Some(x) if x.is_finite() => {
                    values.insert(key.clone(), x);
                    known.insert(key, x);
                }
                Some(x) => {
                    errors.insert(key, format!("the result is {x}"));
                }
                None => {
                    errors.insert(
                        key,
                        format!(
                            "the expression gives {}, an output needs a number",
                            v.kind()
                        ),
                    );
                }
            },
            Err(e) => {
                errors.insert(key, e.render(&o.expr).replace('\n', " | "));
            }
        }
    }
    (values, errors)
}

/// Add the outputs of `setup` to a finished run's measurements. A failed output is
/// left out and its error returned (a spec on it then reads as missing).
pub fn apply(
    setup: &Setup,
    point: &RunPoint,
    outcome: Result<Outcome>,
) -> (Result<Outcome>, BTreeMap<String, String>) {
    match outcome {
        Ok(mut o) if !setup.outputs.is_empty() => {
            let (values, errors) =
                evaluate_outputs(&setup.outputs, setup, point, &o.raw, &o.measures);
            o.measures.extend(values);
            (Ok(o), errors)
        }
        other => (other, BTreeMap::new()),
    }
}

/// Mark which measures of `rec` came from the calculator and keep the errors.
pub fn mark(
    rec: &mut crate::sim::results::RunRecord,
    setup: &Setup,
    errors: BTreeMap<String, String>,
) {
    rec.calc = setup
        .outputs
        .iter()
        .map(|o| o.name.to_lowercase())
        .filter(|n| rec.measures.contains_key(n))
        .collect();
    rec.calc_errors = errors;
}
