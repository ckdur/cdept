//! The function library: a table of names, parameters and what they do.
//!
//! Arguments are positional or named (`cross(w, level=0.6, edge="rise")`); the
//! parameter list of each function says the order. `cdept calc --functions` prints this
//! table.

use eyre::{Result, bail, eyre};

use super::jitter;
use super::measure::{self, Edge, Range};
use super::spectral::{self, LinearityMethod, SpectrumOpts, Window};
use super::wave::{C, Samples, Waveform};
use super::{Env, Value};

/// One entry of the library.
pub struct Func {
    pub names: &'static [&'static str],
    /// Parameters in positional order; a name starting with `?` is optional.
    pub params: &'static [&'static str],
    pub doc: &'static str,
    pub run: fn(&Env, &Bound) -> Result<Value>,
}

/// The arguments of a call, bound to the parameter names.
pub struct Bound<'a> {
    pub func: &'a str,
    params: &'static [&'static str],
    vals: Vec<Option<Value>>,
}

fn bare(p: &str) -> &str {
    p.trim_start_matches('?')
}

impl<'a> Bound<'a> {
    pub fn new(
        func: &'a str,
        def: &Func,
        pos: Vec<Value>,
        named: Vec<(String, Value)>,
    ) -> Result<Bound<'a>> {
        let mut vals: Vec<Option<Value>> = vec![None; def.params.len()];
        if pos.len() > def.params.len() {
            bail!(
                "{func} takes at most {} arguments ({}), got {}",
                def.params.len(),
                param_list(def),
                pos.len()
            );
        }
        for (i, v) in pos.into_iter().enumerate() {
            vals[i] = Some(v);
        }
        for (name, v) in named {
            let Some(i) = def.params.iter().position(|p| bare(p) == name) else {
                bail!(
                    "{func} has no argument '{name}' (it takes {})",
                    param_list(def)
                );
            };
            if vals[i].is_some() {
                bail!("{func}: argument '{name}' is given twice");
            }
            vals[i] = Some(v);
        }
        for (i, p) in def.params.iter().enumerate() {
            if !p.starts_with('?') && vals[i].is_none() {
                bail!(
                    "{func} needs the argument '{p}' (it takes {})",
                    param_list(def)
                );
            }
        }
        Ok(Bound {
            func,
            params: def.params,
            vals,
        })
    }

    fn slot(&self, name: &str) -> Option<&Value> {
        let i = self
            .params
            .iter()
            .position(|p| bare(p) == name)
            .expect("parameter exists in the table");
        self.vals[i].as_ref()
    }

    pub fn has(&self, name: &str) -> bool {
        self.slot(name).is_some()
    }

    pub fn value(&self, name: &str) -> Result<&Value> {
        self.slot(name)
            .ok_or_else(|| eyre!("{} needs '{name}'", self.func))
    }

    pub fn wave(&self, name: &str) -> Result<&Waveform> {
        match self.value(name)? {
            Value::Wave(w) => Ok(w),
            other => {
                bail!(
                    "{}: '{name}' must be a waveform, got {}",
                    self.func,
                    other.kind()
                );
            }
        }
    }

    pub fn opt_num(&self, name: &str) -> Result<Option<f64>> {
        match self.slot(name) {
            None => Ok(None),
            Some(Value::Num(v)) => Ok(Some(*v)),
            Some(Value::Str(s)) => super::lang::parse_spice_number(s)
                .map(Some)
                .ok_or_else(|| eyre!("{}: '{name}' = \"{s}\" is not a number", self.func)),
            Some(other) => {
                bail!(
                    "{}: '{name}' must be a number, got {}",
                    self.func,
                    other.kind()
                );
            }
        }
    }

    pub fn num(&self, name: &str) -> Result<f64> {
        self.opt_num(name)?
            .ok_or_else(|| eyre!("{} needs '{name}'", self.func))
    }

    pub fn num_or(&self, name: &str, default: f64) -> Result<f64> {
        Ok(self.opt_num(name)?.unwrap_or(default))
    }

    pub fn int_or(&self, name: &str, default: i64) -> Result<i64> {
        Ok(self.opt_num(name)?.map_or(default, |v| v as i64))
    }

    pub fn str_or(&self, name: &str, default: &str) -> Result<String> {
        match self.slot(name) {
            None => Ok(default.to_string()),
            Some(Value::Str(s)) => Ok(s.clone()),
            Some(other) => {
                bail!(
                    "{}: '{name}' must be a string, got {}",
                    self.func,
                    other.kind()
                );
            }
        }
    }

    pub fn flag(&self, name: &str, default: bool) -> Result<bool> {
        match self.slot(name) {
            None => Ok(default),
            Some(Value::Num(v)) => Ok(*v != 0.0),
            Some(Value::Str(s)) => Ok(matches!(
                s.to_lowercase().as_str(),
                "true" | "yes" | "1" | "on"
            )),
            Some(other) => {
                bail!(
                    "{}: '{name}' must be true/false, got {}",
                    self.func,
                    other.kind()
                );
            }
        }
    }

    pub fn edge_or(&self, name: &str, default: &str) -> Result<Edge> {
        Edge::parse(&self.str_or(name, default)?)
    }

    pub fn range(&self) -> Result<Range> {
        Ok(Range {
            from: self.opt_num("from")?,
            to: self.opt_num("to")?,
        })
    }
}

fn param_list(def: &Func) -> String {
    def.params
        .iter()
        .map(|p| {
            if let Some(o) = p.strip_prefix('?') {
                format!("[{o}]")
            } else {
                p.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Find a function by name (case-insensitive).
pub fn lookup(name: &str) -> Option<&'static Func> {
    FUNCS
        .iter()
        .find(|f| f.names.iter().any(|n| n.eq_ignore_ascii_case(name)))
}

/// All functions, for help output.
pub fn all() -> &'static [Func] {
    FUNCS
}

fn num(v: f64) -> Result<Value> {
    Ok(Value::Num(v))
}

fn wave(w: Waveform) -> Result<Value> {
    Ok(Value::Wave(w))
}

// Elementwise helpers ------------------------------------------------------------

/// A function of one real or complex value that gives a real number.
fn to_real(
    v: &Value,
    name: &str,
    unit: &str,
    rf: fn(f64) -> f64,
    cf: Option<fn(C) -> f64>,
) -> Result<Value> {
    match v {
        Value::Num(x) => num(rf(*x)),
        Value::Complex(c) => match cf {
            Some(f) => num(f(*c)),
            None => {
                bail!("{name} does not take a complex value");
            }
        },
        Value::Wave(w) => match (&w.y, cf) {
            (Samples::Real(y), _) => wave(w.with_y(y.iter().map(|&v| rf(v)).collect(), unit)),
            (Samples::Complex(_), Some(f)) => wave(w.map_c(unit, f)),
            _ => {
                bail!("{name} does not take a complex waveform (try mag, real, imag, phase first)");
            }
        },
        Value::Str(_) => {
            bail!("{name} does not take a string");
        }
    }
}

/// A function of one value that gives a complex number for a complex input.
fn to_cplx(v: &Value, name: &str, rf: fn(f64) -> f64, cf: fn(C) -> C) -> Result<Value> {
    match v {
        Value::Num(x) => num(rf(*x)),
        Value::Complex(c) => Ok(Value::Complex(cf(*c))),
        Value::Wave(w) => match &w.y {
            Samples::Real(y) => wave(w.with_y(y.iter().map(|&v| rf(v)).collect(), "")),
            Samples::Complex(y) => wave(w.with_complex(y.iter().map(|&c| cf(c)).collect())),
        },
        Value::Str(_) => {
            bail!("{name} does not take a string");
        }
    }
}

macro_rules! real_fn {
    ($names:expr, $doc:expr, $unit:expr, $rf:expr) => {
        Func {
            names: $names,
            params: &["x"],
            doc: $doc,
            run: |_, b| to_real(b.value("x")?, b.func, $unit, $rf, None),
        }
    };
    ($names:expr, $doc:expr, $unit:expr, $rf:expr, $cf:expr) => {
        Func {
            names: $names,
            params: &["x"],
            doc: $doc,
            run: |_, b| to_real(b.value("x")?, b.func, $unit, $rf, Some($cf)),
        }
    };
}

/// The most points a function makes up (resample, the FFT length): a typo such as
/// `n=1e12` is an error, not an allocation of terabytes.
const MAX_POINTS: f64 = 16_777_216.0;

fn spectrum_opts(b: &Bound) -> Result<SpectrumOpts> {
    let n = b.opt_num("n")?;
    if let Some(n) = n
        && !(8.0..=MAX_POINTS).contains(&n)
    {
        bail!("n must be between 8 and {MAX_POINTS} (got {n})");
    }
    Ok(SpectrumOpts {
        window: Window::parse(&b.str_or("window", "hann")?)?,
        n: n.map(|n| n as usize),
        range: b.range()?,
    })
}

fn tone(b: &Bound) -> Result<spectral::ToneAnalysis> {
    spectral::analyze(
        b.wave("w")?,
        &spectrum_opts(b)?,
        b.opt_num("fund")?,
        b.int_or("harmonics", 5)?.max(1) as usize,
    )
}

fn lookup_vector(env: &Env, b: &Bound, candidates: &[String]) -> Result<Value> {
    let plot = b.str_or("plot", "")?;
    let analysis = b.str_or("analysis", "")?;
    wave(env.vector(
        candidates,
        if plot.is_empty() { None } else { Some(&plot) },
        if analysis.is_empty() {
            None
        } else {
            Some(&analysis)
        },
    )?)
}

/// The name VACASK gives a vector that ngspice names `a.b.c` (a node inside subcircuits,
/// `a:b:c`) or `@n.a.b.c[var]` (a device's value, `a:b:c.var`).
fn vacask_alias(name: &str) -> Option<String> {
    if let Some(device) = name.strip_prefix('@') {
        let (path, var) = device.strip_suffix(']')?.split_once('[')?;
        let mut parts: Vec<&str> = path.split('.').collect();
        if parts.len() > 1 && parts[0].len() == 1 {
            parts.remove(0);
        }
        return Some(format!("{}.{}", parts.join(":"), var));
    }
    name.contains('.').then(|| name.replace('.', ":"))
}

fn clean_name(b: &Bound) -> Result<String> {
    Ok(b.str_or("name", "")?
        .trim()
        .trim_start_matches('/')
        .to_string())
}

const TOLERANCE: &str = "tol";

static FUNCS: &[Func] = &[
    // Lookup
    Func {
        names: &["v"],
        params: &["name", "?plot", "?analysis"],
        doc: "Node voltage from the raw file: v(\"out\") finds `v(out)` or `out`. `analysis` (tran, ac, dc, noise) or `plot` picks the plot when several have it.",
        run: |env, b| {
            let n = clean_name(b)?;
            let mut names = vec![format!("v({n})"), n.clone()];
            names.extend(vacask_alias(&n));
            lookup_vector(env, b, &names)
        },
    },
    Func {
        names: &["i"],
        params: &["name", "?plot", "?analysis"],
        doc: "Branch current: i(\"vdd\") finds `i(vdd)`, `vdd#branch` or (VACASK) `vdd:flow(br)`. Same plot selection as v().",
        run: |env, b| {
            let n = clean_name(b)?;
            lookup_vector(
                env,
                b,
                &[
                    format!("i({n})"),
                    format!("{n}#branch"),
                    format!("{n}:flow(br)"),
                    n.clone(),
                ],
            )
        },
    },
    Func {
        names: &["sig", "vector"],
        params: &["name", "?plot", "?analysis"],
        doc: "Any vector of the raw file by its exact name (case-insensitive): sig(\"onoise_spectrum\").",
        run: |env, b| {
            let n = clean_name(b)?;
            lookup_vector(env, b, &[n])
        },
    },
    Func {
        names: &["xaxis", "x"],
        params: &["w"],
        doc: "The x values of a waveform as a waveform (time against time, frequency against frequency).",
        run: |_, b| {
            let w = b.wave("w")?;
            wave(w.with_y(w.x.to_vec(), &w.x_unit))
        },
    },
    // Elementwise
    real_fn!(
        &["abs", "mag"],
        "Absolute value; magnitude of a complex number or waveform.",
        "",
        f64::abs,
        C::abs
    ),
    real_fn!(
        &["db20"],
        "20 log10 |x|: voltage gain in dB.",
        "dB",
        |v| 20.0 * v.abs().log10(),
        |c| 20.0 * c.abs().log10()
    ),
    real_fn!(
        &["db10"],
        "10 log10 |x|: power gain in dB.",
        "dB",
        |v| 10.0 * v.abs().log10(),
        |c| 10.0 * c.abs().log10()
    ),
    Func {
        names: &["phase", "phase_deg"],
        params: &["x"],
        doc: "Phase in degrees; of a waveform it is unwrapped along x.",
        run: |_, b| match b.value("x")? {
            Value::Num(x) => num(if *x < 0.0 { 180.0 } else { 0.0 }),
            Value::Complex(c) => num(c.arg().to_degrees()),
            Value::Wave(w) => wave(measure::phase_deg(w)),
            Value::Str(_) => {
                bail!("phase does not take a string");
            }
        },
    },
    real_fn!(
        &["phase_rad"],
        "Phase in radians (not unwrapped).",
        "rad",
        |v| if v < 0.0 { std::f64::consts::PI } else { 0.0 },
        C::arg
    ),
    real_fn!(&["real", "re"], "Real part.", "", |v| v, |c| c.re),
    real_fn!(&["imag", "im"], "Imaginary part.", "", |_| 0.0, |c| c.im),
    Func {
        names: &["conj"],
        params: &["x"],
        doc: "Complex conjugate.",
        run: |_, b| to_cplx(b.value("x")?, "conj", |v| v, |c| C::new(c.re, -c.im)),
    },
    Func {
        names: &["sqrt"],
        params: &["x"],
        doc: "Square root.",
        run: |_, b| to_cplx(b.value("x")?, "sqrt", f64::sqrt, C::sqrt),
    },
    Func {
        names: &["exp"],
        params: &["x"],
        doc: "e to the power x.",
        run: |_, b| to_cplx(b.value("x")?, "exp", f64::exp, C::exp),
    },
    Func {
        names: &["ln", "log"],
        params: &["x"],
        doc: "Natural logarithm.",
        run: |_, b| to_cplx(b.value("x")?, "ln", f64::ln, C::ln),
    },
    real_fn!(&["log10"], "Logarithm to base 10.", "", f64::log10),
    real_fn!(&["sin"], "Sine (radians).", "", f64::sin),
    real_fn!(&["cos"], "Cosine (radians).", "", f64::cos),
    real_fn!(&["tan"], "Tangent (radians).", "", f64::tan),
    real_fn!(&["atan"], "Arc tangent (radians).", "", f64::atan),
    real_fn!(&["floor"], "Round down.", "", f64::floor),
    real_fn!(&["ceil"], "Round up.", "", f64::ceil),
    real_fn!(&["round"], "Round to the nearest integer.", "", f64::round),
    Func {
        names: &["pow"],
        params: &["x", "y"],
        doc: "x to the power y (same as x ^ y).",
        run: |_, b| {
            super::binary(
                super::lang::BinOp::Pow,
                b.value("x")?.clone(),
                b.value("y")?.clone(),
            )
        },
    },
    // Reductions
    Func {
        names: &["min"],
        params: &["w", "?b", "?from", "?to"],
        doc: "Smallest value of a waveform (within from..to), or min(a, b) of two numbers.",
        run: |env, b| {
            if let (Value::Num(a), Some(Value::Num(c))) =
                (b.value("w")?, b.has("b").then(|| b.value("b").unwrap()))
            {
                return num(a.min(*c));
            }
            let w = b.range()?.apply(b.wave("w")?);
            let v = measure::min(&w)?;
            if let Ok(x) = measure::argext(&w, false) {
                env.mark(x, Some(v), "min", &w.x_unit);
            }
            num(v)
        },
    },
    Func {
        names: &["max"],
        params: &["w", "?b", "?from", "?to"],
        doc: "Largest value of a waveform (within from..to), or max(a, b) of two numbers.",
        run: |env, b| {
            if let (Value::Num(a), Some(Value::Num(c))) =
                (b.value("w")?, b.has("b").then(|| b.value("b").unwrap()))
            {
                return num(a.max(*c));
            }
            let w = b.range()?.apply(b.wave("w")?);
            let v = measure::max(&w)?;
            if let Ok(x) = measure::argext(&w, true) {
                env.mark(x, Some(v), "max", &w.x_unit);
            }
            num(v)
        },
    },
    Func {
        names: &["minx"],
        params: &["w", "?from", "?to"],
        doc: "x of the smallest value.",
        run: |_, b| num(measure::argext(&b.range()?.apply(b.wave("w")?), false)?),
    },
    Func {
        names: &["maxx"],
        params: &["w", "?from", "?to"],
        doc: "x of the largest value.",
        run: |_, b| num(measure::argext(&b.range()?.apply(b.wave("w")?), true)?),
    },
    Func {
        names: &["avg", "mean", "average"],
        params: &["w", "?from", "?to"],
        doc: "Time-weighted average (trapezoid integral over the x span).",
        run: |_, b| num(measure::avg(&b.range()?.apply(b.wave("w")?))?),
    },
    Func {
        names: &["rms"],
        params: &["w", "?from", "?to"],
        doc: "Root mean square, time-weighted.",
        run: |_, b| num(measure::rms(&b.range()?.apply(b.wave("w")?))?),
    },
    Func {
        names: &["pkpk", "peak_to_peak"],
        params: &["w", "?from", "?to"],
        doc: "Maximum minus minimum.",
        run: |_, b| num(measure::peak_to_peak(&b.range()?.apply(b.wave("w")?))?),
    },
    Func {
        names: &["integrate", "area"],
        params: &["w", "?from", "?to"],
        doc: "Integral over x (a number), trapezoid rule.",
        run: |_, b| num(measure::integrate(&b.range()?.apply(b.wave("w")?))?),
    },
    Func {
        names: &["len", "count"],
        params: &["w"],
        doc: "Number of points.",
        run: |_, b| num(b.wave("w")?.len() as f64),
    },
    Func {
        names: &["xmin", "xmax"],
        params: &["w"],
        doc: "First or last x of the waveform.",
        run: |_, b| {
            let w = b.wave("w")?;
            let v = if b.func.eq_ignore_ascii_case("xmin") {
                w.x.first()
            } else {
                w.x.last()
            };
            num(*v.ok_or_else(|| eyre!("empty waveform"))?)
        },
    },
    // Waveform to waveform
    Func {
        names: &["derivative", "deriv", "ddx"],
        params: &["w"],
        doc: "d/dx (central differences).",
        run: |_, b| wave(measure::derivative(b.wave("w")?)?),
    },
    Func {
        names: &["integral", "integ"],
        params: &["w"],
        doc: "Running integral from the first point.",
        run: |_, b| wave(measure::integral(b.wave("w")?)?),
    },
    Func {
        names: &["clip", "xrange"],
        params: &["w", "from", "to"],
        doc: "The part of the waveform between two x values (ends interpolated).",
        run: |_, b| wave(b.wave("w")?.clip(b.num("from")?, b.num("to")?)),
    },
    Func {
        names: &["value_at", "at", "interp"],
        params: &["w", "x"],
        doc: "Value at x, linear interpolation; a complex number for a complex waveform.",
        run: |env, b| {
            let c = measure::value_at(b.wave("w")?, b.num("x")?)?;
            if !b.wave("w")?.y.is_complex() {
                env.mark(b.num("x")?, Some(c.re), "at", &b.wave("w")?.x_unit);
            }
            Ok(if b.wave("w")?.y.is_complex() {
                Value::Complex(c)
            } else {
                Value::Num(c.re)
            })
        },
    },
    Func {
        names: &["coherent"],
        params: &["fin", "fs", "n"],
        doc: "Coherent-sampling tone: the frequency nearest `fin` that fits a whole, odd number of cycles (coprime with n) in n samples at fs, i.e. m*fs/n.",
        run: |_, b| {
            num(measure::coherent_frequency(
                b.num("fin")?,
                b.num("fs")?,
                b.num("n")?,
            )?)
        },
    },
    Func {
        names: &["resample"],
        params: &["w", "?n", "?step", "?from", "?to"],
        doc: "Resample on a uniform grid of n points or every `step`, over from..to (default the whole x range).",
        run: |_, b| {
            let w = b.wave("w")?;
            let (f, t) = (
                b.opt_num("from")?.unwrap_or(w.x[0]),
                b.opt_num("to")?.unwrap_or(w.x[w.len() - 1]),
            );
            let count = match (b.opt_num("n")?, b.opt_num("step")?) {
                (Some(n), None) => n,
                (None, Some(s)) if s > 0.0 => (t - f) / s + 1.0,
                _ => 0.0,
            };
            if count.is_nan() || count > MAX_POINTS {
                bail!("resample would make {count:.0} points (at most {MAX_POINTS})");
            }
            let xs: Vec<f64> = match (b.opt_num("n")?, b.opt_num("step")?) {
                (Some(n), None) if n >= 2.0 => (0..n as usize)
                    .map(|k| f + (t - f) * k as f64 / (n - 1.0))
                    .collect(),
                (None, Some(s)) if s > 0.0 => (0..=((t - f) / s + 1e-9).floor() as usize)
                    .map(|k| f + s * k as f64)
                    .collect(),
                _ => {
                    bail!("resample needs n (>= 2) or step (> 0), not both");
                }
            };
            wave(w.sample_on(&xs))
        },
    },
    // Timing
    Func {
        names: &["cross"],
        params: &["w", "?level", "?edge", "?n", "?from", "?to"],
        doc: "x of the n-th crossing (1 = first, -1 = last) of `level` (default 0); edge rise, fall or either (default).",
        run: |env, b| {
            let level = b.num_or("level", 0.0)?;
            let x = measure::cross(
                b.wave("w")?,
                level,
                b.edge_or("edge", "either")?,
                b.int_or("n", 1)?,
                b.range()?,
            )?;
            env.mark(x, Some(level), "cross", &b.wave("w")?.x_unit);
            num(x)
        },
    },
    Func {
        names: &["delay"],
        params: &[
            "trig",
            "targ",
            "?level",
            "?trig_edge",
            "?targ_edge",
            "?trig_level",
            "?targ_level",
            "?trig_n",
            "?targ_n",
            "?from",
            "?to",
        ],
        doc: "Time from the trig_n-th trig_edge crossing of `trig` to the targ_n-th targ_edge crossing of `targ` after it. `level` is the threshold of both unless trig_level/targ_level say otherwise; edges default to rise.",
        run: |env, b| {
            let level = b.opt_num("level")?;
            let pick = |name: &str| -> Result<f64> {
                b.opt_num(name)?
                    .or(level)
                    .ok_or_else(|| eyre!("delay needs `level` (or trig_level and targ_level)"))
            };
            let (lt, lg) = (pick("trig_level")?, pick("targ_level")?);
            let (t0, t1) = measure::delay_times(
                b.wave("trig")?,
                b.wave("targ")?,
                lt,
                lg,
                b.edge_or("trig_edge", "rise")?,
                b.edge_or("targ_edge", "rise")?,
                b.int_or("trig_n", 1)?,
                b.int_or("targ_n", 1)?,
                b.range()?,
            )?;
            env.mark(t0, Some(lt), "trigger", &b.wave("trig")?.x_unit);
            env.mark(t1, Some(lg), "target", &b.wave("targ")?.x_unit);
            num(t1 - t0)
        },
    },
    Func {
        names: &["rise_time", "trise"],
        params: &["w", "?lo", "?hi", "?vlow", "?vhigh", "?n", "?from", "?to"],
        doc: "Time for the n-th rising edge to go from lo to hi (fractions of the swing, default 0.1 and 0.9). The swing is vlow..vhigh, default the waveform's min and max.",
        run: |env, b| {
            let e = measure::edge(
                b.wave("w")?,
                true,
                b.num_or("lo", 0.1)?,
                b.num_or("hi", 0.9)?,
                b.opt_num("vlow")?,
                b.opt_num("vhigh")?,
                b.int_or("n", 1)?,
                b.range()?,
            )?;
            let (first, second) = if true {
                (e.low, e.high)
            } else {
                (e.high, e.low)
            };
            env.mark(e.t_low, Some(first), "start", &b.wave("w")?.x_unit);
            env.mark(e.t_high, Some(second), "end", &b.wave("w")?.x_unit);
            num(e.t_high - e.t_low)
        },
    },
    Func {
        names: &["fall_time", "tfall"],
        params: &["w", "?lo", "?hi", "?vlow", "?vhigh", "?n", "?from", "?to"],
        doc: "Time for the n-th falling edge to go from hi to lo; arguments as rise_time.",
        run: |env, b| {
            let e = measure::edge(
                b.wave("w")?,
                false,
                b.num_or("lo", 0.1)?,
                b.num_or("hi", 0.9)?,
                b.opt_num("vlow")?,
                b.opt_num("vhigh")?,
                b.int_or("n", 1)?,
                b.range()?,
            )?;
            let (first, second) = if false {
                (e.low, e.high)
            } else {
                (e.high, e.low)
            };
            env.mark(e.t_low, Some(first), "start", &b.wave("w")?.x_unit);
            env.mark(e.t_high, Some(second), "end", &b.wave("w")?.x_unit);
            num(e.t_high - e.t_low)
        },
    },
    Func {
        names: &["slew", "slew_rate"],
        params: &[
            "w", "?edge", "?lo", "?hi", "?vlow", "?vhigh", "?n", "?from", "?to",
        ],
        doc: "Slope of the n-th edge between lo and hi of the swing, in units per x (V/s), always positive; edge rise (default) or fall.",
        run: |_, b| {
            let rising = !matches!(b.edge_or("edge", "rise")?, Edge::Fall);
            num(measure::slew(
                b.wave("w")?,
                rising,
                b.num_or("lo", 0.1)?,
                b.num_or("hi", 0.9)?,
                b.opt_num("vlow")?,
                b.opt_num("vhigh")?,
                b.int_or("n", 1)?,
                b.range()?,
            )?)
        },
    },
    Func {
        names: &["overshoot"],
        params: &["w", "?initial", "?final"],
        doc: "Percent the step response goes past its final value (default: its first value and the mean of its last 5 %).",
        run: |_, b| {
            num(measure::overshoot(
                b.wave("w")?,
                b.opt_num("initial")?,
                b.opt_num("final")?,
                true,
            )?)
        },
    },
    Func {
        names: &["undershoot"],
        params: &["w", "?initial", "?final"],
        doc: "Percent the step response goes past its initial value the other way.",
        run: |_, b| {
            num(measure::overshoot(
                b.wave("w")?,
                b.opt_num("initial")?,
                b.opt_num("final")?,
                false,
            )?)
        },
    },
    Func {
        names: &["settling_time", "settle"],
        params: &["w", "?tol", "?initial", "?final", "?from"],
        doc: "Time after `from` (default the start) until the response stays within tol (fraction of the step, default 0.02) of its final value.",
        run: |env, b| {
            let w = b.wave("w")?;
            let t = measure::settling_time(
                w,
                b.num_or(TOLERANCE, 0.02)?,
                b.opt_num("initial")?,
                b.opt_num("final")?,
                b.opt_num("from")?,
            )?;
            let t0 = b.opt_num("from")?.unwrap_or(w.x[0]);
            env.mark(t0 + t, None, "settled", &w.x_unit);
            num(t)
        },
    },
    Func {
        names: &["frequency", "freq"],
        params: &["w", "?level", "?edge", "?from", "?to"],
        doc: "Mean frequency of the crossings of `level` (default mid-swing), rising edges unless edge=\"fall\".",
        run: |_, b| {
            num(measure::frequency(
                b.wave("w")?,
                b.opt_num("level")?,
                b.edge_or("edge", "rise")?,
                b.range()?,
            )?)
        },
    },
    Func {
        names: &["period"],
        params: &["w", "?level", "?edge", "?from", "?to"],
        doc: "Mean period: 1 / frequency.",
        run: |_, b| {
            num(1.0
                / measure::frequency(
                    b.wave("w")?,
                    b.opt_num("level")?,
                    b.edge_or("edge", "rise")?,
                    b.range()?,
                )?)
        },
    },
    Func {
        names: &["duty", "duty_cycle"],
        params: &["w", "?level", "?from", "?to"],
        doc: "Share of the period above `level` (default mid-swing), in percent, averaged over the full periods.",
        run: |_, b| {
            num(measure::duty(
                b.wave("w")?,
                b.opt_num("level")?,
                b.range()?,
            )?)
        },
    },
    // Frequency response
    Func {
        names: &["bandwidth", "bw"],
        params: &["w", "?drop", "?ref"],
        doc: "Frequency where |w| first falls `drop` dB (default 3) below its first value (or below `ref` dB). Takes the complex response or its magnitude.",
        run: |_, b| {
            num(measure::bandwidth(
                b.wave("w")?,
                b.num_or("drop", 3.0)?,
                b.opt_num("ref")?,
            )?)
        },
    },
    Func {
        names: &["unity_gain_freq", "ugbw", "ugf", "ugb"],
        params: &["w"],
        doc: "Frequency where |w| first falls through 0 dB. Takes the complex gain, or a magnitude (not dB: use bandwidth with ref for dB curves).",
        run: |_, b| num(measure::unity_gain_freq(b.wave("w")?)?),
    },
    Func {
        names: &["phase_margin", "pm"],
        params: &["w"],
        doc: "180 + phase of the complex loop gain (negative feedback included, so the phase starts near 0) at its unity-gain frequency, in degrees.",
        run: |_, b| num(measure::phase_margin(b.wave("w")?)?),
    },
    Func {
        names: &["gain_margin", "gm"],
        params: &["w"],
        doc: "Minus the gain in dB where the phase of the complex loop gain first reaches -180 degrees.",
        run: |_, b| num(measure::gain_margin(b.wave("w")?)?),
    },
    // Spectra
    Func {
        names: &["fft"],
        params: &["w", "?window", "?n", "?from", "?to"],
        doc: "One-sided spectrum of a real waveform, a complex waveform over frequency whose magnitude is the peak amplitude of each component (a sine of amplitude A shows A). window rect, hann (default) or blackman; n points (power of two) when the waveform is not on a uniform power-of-two grid.",
        run: |_, b| wave(spectral::spectrum(b.wave("w")?, &spectrum_opts(b)?)?),
    },
    Func {
        names: &["sndr", "sinad"],
        params: &["w", "?fund", "?harmonics", "?window", "?n", "?from", "?to"],
        doc: "Signal to noise-and-distortion ratio in dB of the tone in `w` (fund Hz, default the largest bin; harmonics 2..=5 by default count as distortion).",
        run: |_, b| num(tone(b)?.sndr_db()),
    },
    Func {
        names: &["snr"],
        params: &["w", "?fund", "?harmonics", "?window", "?n", "?from", "?to"],
        doc: "Signal to noise ratio in dB, harmonics left out.",
        run: |_, b| num(tone(b)?.snr_db()),
    },
    Func {
        names: &["thd"],
        params: &["w", "?fund", "?harmonics", "?window", "?n", "?from", "?to"],
        doc: "Total harmonic distortion in dB relative to the signal (negative: -60 is 0.1 %).",
        run: |_, b| num(tone(b)?.thd_db()),
    },
    Func {
        names: &["thd_percent"],
        params: &["w", "?fund", "?harmonics", "?window", "?n", "?from", "?to"],
        doc: "Total harmonic distortion as a percentage of the signal amplitude.",
        run: |_, b| num(100.0 * 10f64.powf(tone(b)?.thd_db() / 20.0)),
    },
    Func {
        names: &["sfdr"],
        params: &["w", "?fund", "?harmonics", "?window", "?n", "?from", "?to"],
        doc: "Spurious-free dynamic range in dB: the tone over the strongest other bin.",
        run: |_, b| num(tone(b)?.sfdr_db()),
    },
    Func {
        names: &["enob"],
        params: &["w", "?fund", "?harmonics", "?window", "?n", "?from", "?to"],
        doc: "Effective number of bits: (SNDR - 1.76) / 6.02.",
        run: |_, b| num(tone(b)?.enob()),
    },
    Func {
        names: &["fundamental"],
        params: &["w", "?fund", "?harmonics", "?window", "?n", "?from", "?to"],
        doc: "Frequency of the tone, Hz.",
        run: |_, b| num(tone(b)?.fundamental_hz),
    },
    // Noise
    Func {
        names: &["integrate_noise", "noise_total"],
        params: &["w", "?from", "?to", "?squared"],
        doc: "RMS noise: the density (V/sqrt(Hz), or V^2/Hz with squared=1) integrated over from..to, in V (or A).",
        run: |_, b| {
            num(measure::integrate_noise(
                &b.range()?.apply(b.wave("w")?),
                b.flag("squared", false)?,
            )?)
        },
    },
    Func {
        names: &["spot_noise"],
        params: &["w", "f"],
        doc: "The noise density at one frequency (interpolated).",
        run: |_, b| num(measure::value_at(b.wave("w")?, b.num("f")?)?.abs()),
    },
    // Converters
    Func {
        names: &["dnl"],
        params: &["w", "?method", "?lsb"],
        doc: "DNL in LSB as a waveform over the code. `w` is the output code: against a swept input (method=\"edge\", default; lsb overrides the end-point step) or sampled over a uniform ramp (method=\"hist\").",
        run: |_, b| {
            let (d, _) =
                spectral::linearity(b.wave("w")?, linearity_method(b)?, b.opt_num("lsb")?)?;
            wave(d)
        },
    },
    Func {
        names: &["inl"],
        params: &["w", "?method", "?lsb"],
        doc: "INL in LSB as a waveform over the code (end-point fit); arguments as dnl.",
        run: |_, b| {
            let (_, i) =
                spectral::linearity(b.wave("w")?, linearity_method(b)?, b.opt_num("lsb")?)?;
            wave(i)
        },
    },
    Func {
        names: &["dnl_max"],
        params: &["w", "?method", "?lsb"],
        doc: "Largest |DNL| in LSB.",
        run: |_, b| {
            num(measure::max(
                &spectral::linearity(b.wave("w")?, linearity_method(b)?, b.opt_num("lsb")?)?
                    .0
                    .map(" ", f64::abs)?,
            )?)
        },
    },
    Func {
        names: &["inl_max"],
        params: &["w", "?method", "?lsb"],
        doc: "Largest |INL| in LSB.",
        run: |_, b| {
            num(measure::max(
                &spectral::linearity(b.wave("w")?, linearity_method(b)?, b.opt_num("lsb")?)?
                    .1
                    .map(" ", f64::abs)?,
            )?)
        },
    },
    // Jitter, statistics, sampling, power spectral density
    Func {
        names: &["periods"],
        params: &["w", "?level", "?edge", "?from", "?to"],
        doc: "The length of every period between `edge` crossings (rise by default) of `level` (default mid-swing): a waveform over the time each period starts.",
        run: |_, b| wave(jitter::periods(&jitter_edges(b, 3)?)),
    },
    Func {
        names: &["period_jitter", "jitter_period"],
        params: &["w", "?stat", "?level", "?edge", "?from", "?to"],
        doc: "Period jitter: stat = rms (standard deviation of the periods, default), pp (peak to peak), mean or max.",
        run: |_, b| {
            let p = jitter::periods(&jitter_edges(b, 3)?);
            num(jitter::figure(&p.y.re(), &b.str_or("stat", "rms")?)?)
        },
    },
    Func {
        names: &["cc_periods"],
        params: &["w", "?level", "?edge", "?from", "?to"],
        doc: "Cycle-to-cycle differences of consecutive periods: a waveform over time.",
        run: |_, b| wave(jitter::cycle_to_cycle(&jitter_edges(b, 3)?)),
    },
    Func {
        names: &["cc_jitter", "jitter_cc", "c2c_jitter"],
        params: &["w", "?stat", "?level", "?edge", "?from", "?to"],
        doc: "Cycle-to-cycle jitter: rms (default), pp, mean or max (the largest |difference|, the usual JEDEC figure) of the differences of consecutive periods.",
        run: |_, b| {
            let c = jitter::cycle_to_cycle(&jitter_edges(b, 3)?);
            num(jitter::figure(&c.y.re(), &b.str_or("stat", "rms")?)?)
        },
    },
    Func {
        names: &["tie"],
        params: &["w", "?period", "?level", "?edge", "?from", "?to"],
        doc: "Time interval error of every crossing against the ideal clock: the least-squares fit of the crossings, or `period` with its phase at the mean error. A waveform over time.",
        run: |_, b| wave(jitter::tie(&jitter_edges(b, 2)?, b.opt_num("period")?)?),
    },
    Func {
        names: &["tie_jitter", "jitter_tie"],
        params: &["w", "?stat", "?period", "?level", "?edge", "?from", "?to"],
        doc: "Time interval error jitter: rms (default), pp, mean or max of tie().",
        run: |_, b| {
            let t = jitter::tie(&jitter_edges(b, 2)?, b.opt_num("period")?)?;
            num(jitter::figure(&t.y.re(), &b.str_or("stat", "rms")?)?)
        },
    },
    Func {
        names: &["crossings", "edges"],
        params: &["w", "?level", "?edge", "?from", "?to"],
        doc: "The crossing times as a waveform over the crossing number (1, 2, ...): edge rise (default), fall or either.",
        run: |_, b| {
            let t = jitter_edges(b, 1)?;
            let mut w = Waveform::real((1..=t.len()).map(|k| k as f64).collect(), t);
            w.x_name = "crossing".into();
            w.unit = "time".into();
            wave(w)
        },
    },
    Func {
        names: &["histogram", "hist"],
        params: &["w", "?bins", "?min", "?max", "?weight"],
        doc: "Histogram of the y values: counts over the bin centres (bins default 50, between min and max of the data unless given). weight=\"time\" counts the x span of each sample (time spent in a bin) instead of 1 per sample.",
        run: |_, b| {
            let weighted = match b.str_or("weight", "samples")?.to_lowercase().as_str() {
                "time" | "x" => true,
                "samples" | "count" | "" => false,
                other => return Err(eyre!("weight must be samples or time, not '{other}'")),
            };
            let bins = b.num_or("bins", 50.0)?;
            if !(1.0..=1e6).contains(&bins) {
                bail!("bins must be between 1 and 1000000");
            }
            wave(jitter::histogram(
                b.wave("w")?,
                bins as usize,
                b.opt_num("min")?,
                b.opt_num("max")?,
                weighted,
            )?)
        },
    },
    Func {
        names: &["stddev", "std", "sigma"],
        params: &["w", "?from", "?to"],
        doc: "Sample standard deviation of the y values (every sample counts once; for a time-weighted spread use rms(w - avg(w))).",
        run: |_, b| num(jitter::stats(b.range()?.apply(b.wave("w")?).real_y("stddev")?)?.1),
    },
    Func {
        names: &["percentile"],
        params: &["w", "p", "?from", "?to"],
        doc: "The p-th percentile (0..100) of the y values, linear interpolation between samples.",
        run: |_, b| {
            num(jitter::percentile(
                b.range()?.apply(b.wave("w")?).real_y("percentile")?,
                b.num("p")?,
            )?)
        },
    },
    Func {
        names: &["median"],
        params: &["w", "?from", "?to"],
        doc: "The median of the y values (percentile 50).",
        run: |_, b| {
            num(jitter::percentile(
                b.range()?.apply(b.wave("w")?).real_y("median")?,
                50.0,
            )?)
        },
    },
    Func {
        names: &["sample", "sample_hold"],
        params: &["w", "period", "?from", "?n"],
        doc: "Zero-order-hold samples every `period` from `from` (default the start): the value at or just before each instant, as an ADC output read at its clock. At most n samples.",
        run: |_, b| {
            let n = b.opt_num("n")?;
            if let Some(n) = n
                && (n.is_nan() || n < 1.0)
            {
                bail!("n must be at least 1");
            }
            wave(jitter::sample(
                b.wave("w")?,
                b.num("period")?,
                b.opt_num("from")?,
                n.map(|n| n.min(1e9) as usize),
            )?)
        },
    },
    Func {
        names: &["psd"],
        params: &["w", "?window", "?n", "?segments", "?from", "?to"],
        doc: "Power spectral density (Welch: `segments` 50 % overlapping windowed segments of n points, default 8 segments, hann window), one-sided, in unit^2/Hz; its integral is the variance. Use db10(psd(...)) for dB/Hz, sqrt() for unit/sqrt(Hz).",
        run: |_, b| {
            let seg = b.num_or("segments", 8.0)?;
            if !(1.0..=1024.0).contains(&seg) {
                bail!("segments must be between 1 and 1024");
            }
            wave(jitter::psd(
                b.wave("w")?,
                Window::parse(&b.str_or("window", "hann")?)?,
                b.opt_num("n")?.map(|n| n.clamp(0.0, 1e9) as usize),
                seg as usize,
                b.range()?,
            )?)
        },
    },
];

/// The crossings a jitter function measures: level, edge, range of its arguments.
fn jitter_edges(b: &Bound, min: usize) -> Result<Vec<f64>> {
    jitter::edges(
        b.wave("w")?,
        b.opt_num("level")?,
        b.edge_or("edge", "rise")?,
        b.range()?,
        min,
    )
}

fn linearity_method(b: &Bound) -> Result<LinearityMethod> {
    match b.str_or("method", "edge")?.to_lowercase().as_str() {
        "edge" | "code" | "code-edge" => Ok(LinearityMethod::Edge),
        "hist" | "histogram" => Ok(LinearityMethod::Histogram),
        other => {
            bail!("unknown method '{other}' (edge or hist)");
        }
    }
}
