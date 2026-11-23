//! Measurements on VACASK results. VACASK has no `.meas`, so the `measures` of a
//! testbench setup (ngspice's `.meas` syntax without the `.meas`) are evaluated here
//! on the raw file. A small subset, until the calculator takes over:
//!
//! ```text
//! tran tpd trig v(in) val=0.6 rise=1 targ v(out) val=0.6 fall=1 [td=1n]
//! tran vmax max v(out) [from=0 to=1u]      # also min, pp, avg, rms, integ
//! tran v1 find v(out) at=5n
//! ```
//! `rise=N` / `fall=N` / `cross=N` pick the N-th crossing; `last` is the last one.
//! A measurement that finds nothing (no such crossing) gives `None`, like ngspice's
//! `failed`. Anything outside the subset is an error.

use eyre::{Result, bail, eyre};

use super::raw::Plot;
use super::translate::expr;

fn num(s: &str) -> Result<f64> {
    let e = expr(s).map_err(|m| eyre!(m))?;
    e.parse::<f64>().map_err(|_| eyre!("'{s}' is not a number"))
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Edge {
    Rise,
    Fall,
    Cross,
}

#[derive(Debug, Clone)]
struct Event {
    vec: String,
    val: f64,
    edge: Edge,
    n: usize,
    last: bool,
    td: f64,
}

fn parse_event(w: &[&str]) -> Result<Event> {
    let mut e = Event { vec: w[0].to_string(), val: f64::NAN, edge: Edge::Cross, n: 1, last: false, td: 0.0 };
    for t in &w[1..] {
        let lower = t.to_lowercase();
        if lower == "last" {
            e.last = true;
            continue;
        }
        let Some((k, v)) = lower.split_once('=') else { return Err(eyre!("cannot read '{t}' in a measurement")) };
        match k {
            "val" => e.val = num(v)?,
            "td" => e.td = num(v)?,
            "rise" | "fall" | "cross" => {
                e.edge = match k {
                    "rise" => Edge::Rise,
                    "fall" => Edge::Fall,
                    _ => Edge::Cross,
                };
                if v == "last" {
                    e.last = true;
                } else {
                    e.n = v.parse().map_err(|_| eyre!("'{t}': the number of the crossing must be a whole number"))?;
                }
            }
            other => return Err(eyre!("'{other}' is not supported in a VACASK measurement")),
        }
    }
    if e.val.is_nan() {
        bail!("a trigger or target needs val=");
    }
    Ok(e)
}

fn crossing(plot: &Plot, e: &Event) -> Result<Option<f64>> {
    let y = plot.get(&e.vec).ok_or_else(|| eyre!("no vector '{}' in the {} results", e.vec, plot.plotname))?;
    let x = plot.x();
    let mut count = 0;
    let mut found = None;
    for i in 1..y.len() {
        let (a, b) = (y[i - 1], y[i]);
        let rising = a < e.val && b >= e.val;
        let falling = a > e.val && b <= e.val;
        let hit = match e.edge {
            Edge::Rise => rising,
            Edge::Fall => falling,
            Edge::Cross => rising || falling,
        };
        if !hit {
            continue;
        }
        let t = x[i - 1] + (e.val - a) / (b - a) * (x[i] - x[i - 1]);
        if t < e.td {
            continue;
        }
        count += 1;
        if e.last {
            found = Some(t);
        } else if count == e.n {
            return Ok(Some(t));
        }
    }
    Ok(found)
}

fn window(plot: &Plot, w: &[&str]) -> Result<(f64, f64)> {
    let x = plot.x();
    let (mut from, mut to) = (x.first().copied().unwrap_or(0.0), x.last().copied().unwrap_or(0.0));
    for t in w {
        let lower = t.to_lowercase();
        match lower.split_once('=') {
            Some(("from", v)) => from = num(v)?,
            Some(("to", v)) => to = num(v)?,
            _ => return Err(eyre!("cannot read '{t}' in a measurement")),
        }
    }
    Ok((from, to))
}

/// The `kind` word of a measurement (`tran`, `ac`, `dc`, `noise`, `op`).
pub fn kind_of(measure: &str) -> Option<&str> {
    measure.split_whitespace().next()
}

/// The name of a measurement (second word).
pub fn name_of(measure: &str) -> Option<String> {
    measure.split_whitespace().nth(1).map(str::to_lowercase)
}

/// Evaluate `measure` on `plot`. `Ok(None)`: it found nothing.
pub fn evaluate(measure: &str, plot: &Plot) -> Result<Option<f64>> {
    let w: Vec<&str> = measure.split_whitespace().collect();
    if w.len() < 4 {
        bail!("measurement '{measure}' is too short");
    }
    let lower: Vec<String> = w.iter().map(|s| s.to_lowercase()).collect();
    if lower[2] == "trig" {
        let targ = lower.iter().position(|t| t == "targ").ok_or_else(|| eyre!("'{measure}': trig without targ"))?;
        let trig = parse_event(&w[3..targ])?;
        let targ = parse_event(&w[targ + 1..])?;
        let Some(t0) = crossing(plot, &trig)? else { return Ok(None) };
        let Some(t1) = crossing(plot, &targ)? else { return Ok(None) };
        return Ok(Some(t1 - t0));
    }
    let what = lower[2].as_str();
    let vec = w[3];
    let y = plot.get(vec).ok_or_else(|| eyre!("no vector '{vec}' in the {} results", plot.plotname))?;
    let x = plot.x();
    if what == "find" {
        let at = w[4..]
            .iter()
            .find_map(|t| t.to_lowercase().strip_prefix("at=").map(str::to_string))
            .ok_or_else(|| eyre!("'{measure}': only 'find <vec> at=<x>' is supported"))?;
        let at = num(&at)?;
        for i in 1..x.len() {
            if (x[i - 1] <= at && at <= x[i]) || (x[i - 1] >= at && at >= x[i]) {
                let f = if x[i] == x[i - 1] { 0.0 } else { (at - x[i - 1]) / (x[i] - x[i - 1]) };
                return Ok(Some(y[i - 1] + f * (y[i] - y[i - 1])));
            }
        }
        return Ok(None);
    }
    if !matches!(what, "max" | "min" | "pp" | "avg" | "rms" | "integ") {
        bail!("'{what}' is not supported in a VACASK measurement (trig/targ, max, min, pp, avg, rms, integ, find at)");
    }
    let (from, to) = window(plot, &w[4..])?;
    // A plot of one point (an operating point) has no axis to cut a window from.
    let idx: Vec<usize> =
        if x.len() == 1 { vec![0] } else { (0..x.len()).filter(|&i| x[i] >= from && x[i] <= to).collect() };
    if idx.is_empty() {
        return Ok(None);
    }
    let vals = idx.iter().map(|&i| y[i]);
    Ok(Some(match what {
        "max" => vals.fold(f64::NEG_INFINITY, f64::max),
        "min" => vals.fold(f64::INFINITY, f64::min),
        "pp" => vals.clone().fold(f64::NEG_INFINITY, f64::max) - vals.fold(f64::INFINITY, f64::min),
        _ => {
            // Trapezoid integral over the window.
            let mut int = 0.0;
            let mut sq = 0.0;
            for p in idx.windows(2) {
                let dx = x[p[1]] - x[p[0]];
                int += 0.5 * (y[p[0]] + y[p[1]]) * dx;
                sq += 0.5 * (y[p[0]].powi(2) + y[p[1]].powi(2)) * dx;
            }
            let span = x[*idx.last().unwrap()] - x[idx[0]];
            match what {
                "integ" => int,
                "avg" if span > 0.0 => int / span,
                "rms" if span > 0.0 => (sq / span).sqrt(),
                _ => return Ok(None),
            }
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::super::raw;
    use super::*;

    fn plot() -> Plot {
        let t: Vec<f64> = (0..=10).map(|i| i as f64).collect();
        let vin: Vec<f64> = t.iter().map(|t| if *t < 2.0 { 0.0 } else { 1.0 }).collect();
        let vout: Vec<f64> = t.iter().map(|t| (t / 10.0).min(1.0)).collect();
        raw::parse(&raw::tests::binary(&["time", "in", "out"], &[t, vin, vout])).unwrap().remove(0)
    }

    #[test]
    fn delays_between_crossings() {
        let p = plot();
        // in rises through 0.5 at t=1.5, out through 0.5 at t=5.
        let d = evaluate("tran tpd trig v(in) val=0.5 rise=1 targ v(out) val=0.5 rise=1", &p).unwrap().unwrap();
        assert!((d - 3.5).abs() < 1e-12, "{d}");
        assert!(evaluate("tran x trig v(in) val=0.5 fall=1 targ v(out) val=0.5 rise=1", &p).unwrap().is_none());
        assert!(evaluate("tran x trig v(in) val=0.5 rise=1 targ v(out) val=0.5 rise=1 td=9", &p).unwrap().is_none());
    }

    #[test]
    fn extremes_averages_and_points() {
        let p = plot();
        assert_eq!(evaluate("tran m max v(out)", &p).unwrap(), Some(1.0));
        assert_eq!(evaluate("tran m min v(out) from=2 to=4", &p).unwrap(), Some(0.2));
        assert!((evaluate("tran m avg v(out)", &p).unwrap().unwrap() - 0.5).abs() < 1e-12);
        assert_eq!(evaluate("tran m pp v(in)", &p).unwrap(), Some(1.0));
        assert!((evaluate("tran m find v(out) at=2.5", &p).unwrap().unwrap() - 0.25).abs() < 1e-12);
        assert!((evaluate("tran m integ v(out)", &p).unwrap().unwrap() - 5.0).abs() < 1e-12);
    }

    #[test]
    fn unsupported_things_are_errors() {
        let p = plot();
        assert!(evaluate("tran m deriv v(out)", &p).is_err());
        assert!(evaluate("tran m max v(nothing)", &p).is_err());
        assert!(evaluate("tran m", &p).is_err());
        assert!(evaluate("tran x trig v(in) val=0.5 rise=1", &p).is_err());
    }
}
