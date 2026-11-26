//! The measurement calculator: waveforms from a SPICE raw file, a function library
//! (crossings, delay, rise time, bandwidth, margins, FFT figures, noise, INL/DNL, …)
//! and a small expression language to combine them.
//!
//! ```text
//! ugbw(v("out") / v("in"))
//! delay(v("in"), v("out"), 0.6, trig_edge="rise", targ_edge="fall")
//! enob(clip(v("dout"), 1u, 9u), window="rect")
//! ```
//!
//! [`evaluate`] parses and runs an expression against an [`Env`]: the raw file, optionally
//! one plot to look in, and named values (the corner, the temperature, the setup's
//! variables). The result is a [`Value`]: a number, a complex number, a waveform or a
//! string. Errors carry the position in the source ([`CalcError::render`]). The language
//! and the functions: docs/CALC.md.

pub mod funcs;
pub mod jitter;
pub mod lang;
pub mod measure;
pub mod sim;
pub mod spectral;
pub mod wave;

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};

use eyre::eyre;

use crate::raw::RawFile;
pub use lang::{CalcError, Expr, Span, parse, parse_spice_number};
pub use wave::{C, Samples, Waveform};

use lang::{BinOp, ExprKind};

/// What an expression gives.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Num(f64),
    Complex(C),
    Wave(Waveform),
    Str(String),
}

impl Value {
    /// `number`, `complex`, `waveform` or `string`.
    pub fn kind(&self) -> &'static str {
        match self {
            Value::Num(_) => "a number",
            Value::Complex(_) => "a complex number",
            Value::Wave(_) => "a waveform",
            Value::Str(_) => "a string",
        }
    }

    /// The number, for a scalar result (the value a spec can be checked against).
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Num(v) => Some(*v),
            Value::Complex(c) if c.im == 0.0 => Some(c.re),
            _ => None,
        }
    }

    /// A short text for tables: numbers in engineering style, waveforms as their size.
    pub fn display(&self) -> String {
        match self {
            Value::Num(v) => format!("{v:.6e}"),
            Value::Complex(c) => format!("{:.6e}{:+.6e}j", c.re, c.im),
            Value::Wave(w) => format!(
                "waveform {} ({} points, {} {} to {})",
                w.name,
                w.len(),
                w.x_name,
                w.x.first().copied().unwrap_or(f64::NAN),
                w.x.last().copied().unwrap_or(f64::NAN)
            ),
            Value::Str(s) => s.clone(),
        }
    }
}

/// What expressions are evaluated against.
pub struct Env<'a> {
    raw: Option<&'a RawFile>,
    /// Only look in this plot (an index, a kind like `tran`, or part of its name).
    plot: Option<String>,
    vars: BTreeMap<String, Value>,
    cache: RefCell<HashMap<(usize, String), Waveform>>,
}

impl<'a> Env<'a> {
    /// An environment without a raw file (numbers and variables only).
    pub fn empty() -> Env<'static> {
        Env {
            raw: None,
            plot: None,
            vars: Self::builtins(),
            cache: RefCell::default(),
        }
    }

    pub fn new(raw: &'a RawFile) -> Env<'a> {
        Env {
            raw: Some(raw),
            plot: None,
            vars: Self::builtins(),
            cache: RefCell::default(),
        }
    }

    fn builtins() -> BTreeMap<String, Value> {
        BTreeMap::from([("pi".to_string(), Value::Num(std::f64::consts::PI))])
    }

    /// Look only in the plot `selector` names (see [`crate::raw::Plot::matches`]).
    pub fn with_plot(mut self, selector: impl Into<String>) -> Self {
        self.plot = Some(selector.into());
        self
    }

    /// A named value (case-insensitive): a number, or a string such as the corner.
    pub fn set(&mut self, name: &str, value: Value) {
        self.vars.insert(name.to_lowercase(), value);
    }

    /// A variable given as text: a number when it reads as one (SPICE suffixes), else a string.
    pub fn set_text(&mut self, name: &str, text: &str) {
        let v = parse_spice_number(text).map_or_else(|| Value::Str(text.to_string()), Value::Num);
        self.set(name, v);
    }

    pub fn variables(&self) -> &BTreeMap<String, Value> {
        &self.vars
    }

    /// Find a vector under one of the names in `candidates`.
    ///
    /// Plots are searched in file order; with `plot` or `analysis` (or the environment's
    /// plot) only the matching ones. A name found in several plots is an error unless
    /// all but one are single-point plots (an operating point).
    pub fn vector(
        &self,
        candidates: &[String],
        plot: Option<&str>,
        analysis: Option<&str>,
    ) -> eyre::Result<Waveform> {
        let shown = candidates.last().map_or("", String::as_str);
        let raw = self
            .raw
            .ok_or_else(|| eyre!("there is no raw file to read \"{}\" from", shown))?;
        let selector = plot.or(analysis).or(self.plot.as_deref());
        let mut found: Vec<(usize, String)> = vec![];
        for (pi, p) in raw.plots().iter().enumerate() {
            if let Some(s) = selector
                && !p.matches(pi, s)
            {
                continue;
            }
            if let Some(name) = candidates
                .iter()
                .find_map(|c| p.var_index(c).map(|i| p.vars[i].name.clone()))
            {
                found.push((pi, name));
            }
        }
        if found.len() > 1 {
            let multi: Vec<_> = found
                .iter()
                .filter(|(pi, _)| raw.plots()[*pi].points > 1)
                .cloned()
                .collect();
            if multi.len() == 1 {
                found = multi;
            } else if selector.is_none() {
                let names: Vec<String> = found
                    .iter()
                    .map(|(pi, _)| {
                        format!("{} ({})", raw.plots()[*pi].name, raw.plots()[*pi].kind())
                    })
                    .collect();
                return Err(eyre!(
                    "\"{}\" is in several plots: {}; say which with analysis=\"tran\" (or plot=)",
                    shown,
                    names.join(", ")
                ));
            }
        }
        let Some((pi, name)) = found.into_iter().next() else {
            let where_ = match selector {
                Some(s) => format!("in the plot '{s}'"),
                None => "in the raw file".to_string(),
            };
            let have: Vec<&str> = raw
                .plots()
                .iter()
                .flat_map(|p| p.vars.iter().map(|v| v.name.as_str()))
                .take(12)
                .collect();
            return Err(eyre!(
                "no vector \"{}\" {where_} (there are: {}{})",
                shown,
                have.join(", "),
                if have.len() == 12 { ", …" } else { "" }
            ));
        };
        let key = (pi, name.to_lowercase());
        if let Some(w) = self.cache.borrow().get(&key) {
            return Ok(w.clone());
        }
        let w = raw.vector(pi, &name)?;
        self.cache.borrow_mut().insert(key, w.clone());
        Ok(w)
    }
}

/// Parse and evaluate `src`.
pub fn evaluate(src: &str, env: &Env) -> Result<Value, CalcError> {
    let expr = parse(src)?;
    eval(&expr, env)
}

/// Evaluate a parsed expression.
pub fn eval(expr: &Expr, env: &Env) -> Result<Value, CalcError> {
    match &expr.kind {
        ExprKind::Num(v) => Ok(Value::Num(*v)),
        ExprKind::Str(s) => Ok(Value::Str(s.clone())),
        ExprKind::Var(name) => env.vars.get(name).cloned().ok_or_else(|| {
            let known: Vec<&str> = env.vars.keys().map(String::as_str).collect();
            CalcError::new(
                if funcs::lookup(name).is_some() {
                    format!("'{name}' is a function: call it as {name}(…)")
                } else {
                    format!("unknown name '{name}' (known: {})", known.join(", "))
                },
                expr.span,
            )
        }),
        ExprKind::Neg(inner) => {
            let v = eval(inner, env)?;
            binary(BinOp::Mul, Value::Num(-1.0), v)
                .map_err(|e| CalcError::new(format!("{e:#}"), expr.span))
        }
        ExprKind::Bin(op, a, b) => {
            // `a + b + c + ...` is a long left spine: walk it in a loop, not by recursion,
            // so a long sum cannot overflow the stack.
            let mut spine = vec![(*op, b.as_ref(), expr.span)];
            let mut first = a.as_ref();
            while let ExprKind::Bin(op, a, b) = &first.kind {
                spine.push((*op, b.as_ref(), first.span));
                first = a.as_ref();
            }
            let mut acc = eval(first, env)?;
            for (op, right, span) in spine.into_iter().rev() {
                let r = eval(right, env)?;
                acc = binary(op, acc, r).map_err(|e| CalcError::new(format!("{e:#}"), span))?;
            }
            Ok(acc)
        }
        ExprKind::Call { name, args } => {
            let def = funcs::lookup(name).ok_or_else(|| {
                let near: Vec<&str> = funcs::all()
                    .iter()
                    .flat_map(|f| f.names.iter().copied())
                    .filter(|n| {
                        name.chars()
                            .next()
                            .is_some_and(|c| n.starts_with(c.to_ascii_lowercase()))
                    })
                    .take(6)
                    .collect();
                CalcError::new(
                    format!(
                        "unknown function '{name}'{}",
                        if near.is_empty() {
                            String::new()
                        } else {
                            format!(" (did you mean {}?)", near.join(", "))
                        }
                    ),
                    expr.span,
                )
            })?;
            let (mut pos, mut named) = (vec![], vec![]);
            for a in args {
                let v = eval(&a.value, env)?;
                match &a.name {
                    Some(n) => named.push((n.clone(), v)),
                    None => pos.push(v),
                }
            }
            let bound = funcs::Bound::new(name, def, pos, named)
                .map_err(|e| CalcError::new(format!("{e:#}"), expr.span))?;
            (def.run)(env, &bound).map_err(|e| {
                CalcError::new(
                    format!("{name}: {e:#}").replacen(&format!("{name}: {name}"), name, 1),
                    expr.span,
                )
            })
        }
    }
}

fn op_c(op: BinOp) -> fn(C, C) -> C {
    match op {
        BinOp::Add => C::add,
        BinOp::Sub => C::sub,
        BinOp::Mul => C::mul,
        BinOp::Div => C::div,
        BinOp::Pow => C::powc,
    }
}

fn op_f(op: BinOp, a: f64, b: f64) -> f64 {
    match op {
        BinOp::Add => a + b,
        BinOp::Sub => a - b,
        BinOp::Mul => a * b,
        BinOp::Div => a / b,
        BinOp::Pow => a.powf(b),
    }
}

/// `a op b` on numbers, complex numbers and waveforms (a number is broadcast over a
/// waveform; two waveforms are combined on their common grid).
pub fn binary(op: BinOp, a: Value, b: Value) -> eyre::Result<Value> {
    use Value::*;
    Ok(match (a, b) {
        (Num(a), Num(b)) => Num(op_f(op, a, b)),
        (Num(a), Complex(b)) => Complex(op_c(op)(C::new(a, 0.0), b)),
        (Complex(a), Num(b)) => Complex(op_c(op)(a, C::new(b, 0.0))),
        (Complex(a), Complex(b)) => Complex(op_c(op)(a, b)),
        (Wave(w), Num(n)) => scalar_wave(op, &w, C::new(n, 0.0), false, true),
        (Num(n), Wave(w)) => scalar_wave(op, &w, C::new(n, 0.0), true, true),
        (Wave(w), Complex(c)) => scalar_wave(op, &w, c, false, false),
        (Complex(c), Wave(w)) => scalar_wave(op, &w, c, true, false),
        (Wave(a), Wave(b)) => {
            let real = op != BinOp::Pow || (!a.y.is_complex() && !b.y.is_complex());
            Wave(wave::zip(&a, &b, op_c(op), real)?)
        }
        (a, b) => return Err(eyre!("cannot combine {} and {}", a.kind(), b.kind())),
    })
}

fn scalar_wave(op: BinOp, w: &Waveform, k: C, scalar_first: bool, real_scalar: bool) -> Value {
    let f = op_c(op);
    let apply = |c: C| if scalar_first { f(k, c) } else { f(c, k) };
    let all_real = !w.y.is_complex() && real_scalar;
    if all_real {
        let y = w.y.re();
        // Real arithmetic, so a negative base with a fractional power gives NaN as f64 does.
        let out: Vec<f64> = y
            .iter()
            .map(|&v| {
                if scalar_first {
                    op_f(op, k.re, v)
                } else {
                    op_f(op, v, k.re)
                }
            })
            .collect();
        return Value::Wave(w.with_y(out, &w.unit));
    }
    Value::Wave(w.with_complex((0..w.len()).map(|i| apply(w.y.at(i))).collect()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raw::RawFile;

    fn raw_with(plots: Vec<Vec<u8>>) -> RawFile {
        RawFile::from_bytes(plots.concat()).unwrap()
    }

    fn rc_raw() -> RawFile {
        let n = 2001;
        let tau = 1e-9;
        let t: Vec<f64> = (0..n).map(|i| 10e-9 * i as f64 / (n - 1) as f64).collect();
        let vin: Vec<f64> = t
            .iter()
            .map(|&t| if t >= 1e-9 { 1.0 } else { 0.0 })
            .collect();
        let vout: Vec<f64> = t
            .iter()
            .map(|&t| {
                if t >= 1e-9 {
                    1.0 - (-(t - 1e-9) / tau).exp()
                } else {
                    0.0
                }
            })
            .collect();
        raw_with(vec![crate::raw::tests::binary_plot(
            "Transient Analysis",
            &[
                ("time", "time"),
                ("v(in)", "voltage"),
                ("v(out)", "voltage"),
            ],
            &[t, vin, vout],
            false,
        )])
    }

    fn num(src: &str, env: &Env) -> f64 {
        match evaluate(src, env) {
            Ok(v) => v.as_f64().unwrap_or_else(|| panic!("{src}: {v:?}")),
            Err(e) => panic!("{}", e.render(src)),
        }
    }

    #[test]
    fn evaluates_expressions_on_a_raw_file() {
        let raw = rc_raw();
        let mut env = Env::new(&raw);
        env.set_text("vdd", "1.0");
        env.set("corner", Value::Str("tt".into()));
        assert!((num("max(v(\"out\"))", &env) - 1.0).abs() < 1e-3);
        // 50 % crossing of an RC step is tau ln 2 after the input edge.
        let d = num("delay(v(\"in\"), v(\"out\"), 0.5)", &env);
        assert!((d - 1e-9 * 2f64.ln()).abs() < 5e-12, "{d}");
        let d2 = num(
            "delay(v(\"in\"), v(\"out\"), level=vdd/2, trig_edge=\"rise\", targ_edge=\"rise\")",
            &env,
        );
        assert_eq!(d, d2);
        assert!(
            (num("rise_time(v(\"OUT\"), vlow=0, vhigh=1)", &env) - 1e-9 * 9f64.ln()).abs() < 1e-11
        );
        assert!((num("value_at(v(\"out\"), 2n)", &env) - (1.0 - (-1f64).exp())).abs() < 1e-5);
        assert!((num("avg(clip(v(\"in\"), 2n, 10n))", &env) - 1.0).abs() < 1e-9);
        assert!((num("(max(v(\"out\")) - min(v(\"out\"))) * 1000", &env) - 1000.0).abs() < 2.0);
        assert!((num("-2 ^ 2 + 5p * 2e12", &env) - 6.0).abs() < 1e-9);
        assert!(
            (num("integrate(v(\"in\") - v(\"out\"), from=1n, to=10n)", &env) - 1e-9).abs() < 2e-11
        );
        assert!((num("max(2, 3) + pi", &env) - (3.0 + std::f64::consts::PI)).abs() < 1e-12);
        assert!(matches!(evaluate("corner", &env).unwrap(), Value::Str(s) if s == "tt"));
        assert!(matches!(
            evaluate("v(\"out\") * 2", &env).unwrap(),
            Value::Wave(_)
        ));
    }

    #[test]
    fn errors_point_into_the_source() {
        let raw = rc_raw();
        let env = Env::new(&raw);
        for (src, needle, at) in [
            ("max(v(\"nothing\"))", "no vector \"nothing\"", 4),
            ("avg(v(\"out\"), bogus=1)", "no argument 'bogus'", 0),
            ("1 + wat", "unknown name 'wat'", 4),
            ("fnord(1)", "unknown function", 0),
            ("cross(v(\"out\"), 7)", "no crossing", 0),
            ("delay(v(\"in\"), v(\"out\"))", "needs `level`", 0),
            ("\"a\" + 1", "cannot combine", 0),
            ("max", "is a function", 0),
        ] {
            let e = evaluate(src, &env).unwrap_err();
            assert!(e.msg.contains(needle), "{src}: {}", e.msg);
            assert_eq!(e.span.unwrap().start, at, "{src}");
        }
    }

    /// Hostile expressions: errors, never a panic, a hang or a stack overflow, also on a
    /// thread with a small stack (the GUI's and the job queue's threads).
    #[test]
    fn hostile_expressions() {
        let run = || {
            let raw = rc_raw();
            let env = Env::new(&raw);
            let bad = |src: &str| {
                let r = evaluate(src, &env);
                assert!(
                    r.is_err()
                        || r.as_ref().unwrap().as_f64().is_none_or(|v| !v.is_finite())
                        || src.contains("ok"),
                    "{src}: {r:?}"
                );
            };
            // nesting and length
            bad(&format!("{}1{}", "(".repeat(100_000), ")".repeat(100_000)));
            bad(&format!("{}1", "-".repeat(100_000)));
            bad(&format!("1{}", "^1".repeat(100_000)));
            bad(&format!("{}1{}", "abs(".repeat(10_000), ")".repeat(10_000)));
            bad(&"1+".repeat(100_000));
            // the longest chain that is accepted evaluates
            let chain = format!("1{}", "+1".repeat(lang::MAX_TOKENS / 2 - 1));
            assert_eq!(
                evaluate(&chain, &env).unwrap().as_f64(),
                Some((lang::MAX_TOKENS / 2) as f64)
            );
            let nest = format!(
                "{}1{}",
                "(".repeat(lang::MAX_NESTING / 2 - 2),
                ")".repeat(lang::MAX_NESTING / 2 - 2)
            );
            assert_eq!(evaluate(&nest, &env).unwrap().as_f64(), Some(1.0));
            // arithmetic that has no finite answer is an error or a non-finite number
            for src in [
                "1/0",
                "0/0",
                "ln(0)",
                "sqrt(-1)",
                "10^1000",
                "-(10^400)",
                "1e308*10",
            ] {
                let _ = evaluate(src, &env);
            }
            // type errors and nonsense
            for src in [
                "v(\"out\") + \"text\"",
                "\"a\" * 2",
                "abs(\"x\")",
                "fft(1)",
                "enob(3)",
                "cross(v(\"out\"), level=\"high\")",
                "delay(1, 2, 3)",
                "v()",
                "v(1)",
                "v(\"\")",
                "unknownfn(1)",
                "min()",
                "clip(v(\"out\"), 1, 0)",
                "resample(v(\"out\"), n=1e12)",
                "sample(v(\"out\"), 1e-300)",
                "histogram(v(\"out\"), bins=1e300)",
                "psd(v(\"out\"), n=3)",
                "psd(v(\"out\"), segments=-1)",
                "tie(v(\"out\"), period=-1)",
                "percentile(v(\"out\"), 200)",
                "1 +",
                ")",
                "\"unterminated",
                "v(\"out\", analysis=\"nope\")",
                "x(1)",
                "pow(2)",
                "fft(v(\"out\"), n=1e15)",
                "value_at(v(\"out\"), 1e300)",
                "integrate(v(\"out\"), from=5, to=1)",
                "settling_time(v(\"out\"), tol=-1)",
                "\u{0}",
                "αβγ(1)",
                "v(\"ö\")",
            ] {
                let _ = evaluate(src, &env);
            }
        };
        std::thread::Builder::new()
            .stack_size(2 << 20)
            .spawn(run)
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn several_plots_need_a_choice() {
        let tran = crate::raw::tests::binary_plot(
            "Transient Analysis",
            &[("time", "time"), ("v(out)", "voltage")],
            &[vec![0.0, 1.0], vec![1.0, 2.0]],
            false,
        );
        let ac = crate::raw::tests::binary_plot(
            "AC Analysis",
            &[("frequency", "frequency"), ("v(out)", "voltage")],
            &[vec![1.0, 10.0], vec![1.0, 0.5]],
            true,
        );
        let op = crate::raw::tests::binary_plot(
            "Operating Point",
            &[("v(out)", "voltage")],
            &[vec![0.7]],
            false,
        );
        let raw = raw_with(vec![op, tran, ac]);
        let env = Env::new(&raw);
        let e = evaluate("max(v(\"out\"))", &env).unwrap_err();
        assert!(e.msg.contains("several plots"), "{}", e.msg);
        assert_eq!(num("max(v(\"out\", analysis=\"tran\"))", &env), 2.0);
        assert_eq!(num("max(mag(v(\"out\", analysis=\"ac\")))", &env), 1.0);
        assert_eq!(num("max(v(\"out\", plot=\"Transient\"))", &env), 2.0);
        let env = Env::new(&raw).with_plot("ac");
        assert_eq!(num("min(mag(v(\"out\")))", &env), 0.5);
        // Complex values come out as such.
        assert!(matches!(
            evaluate("value_at(v(\"out\"), 1)", &env).unwrap(),
            Value::Complex(_)
        ));
    }
}
