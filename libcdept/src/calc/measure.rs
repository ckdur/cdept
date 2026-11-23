//! Measurements on waveforms: crossings, extremes, edges, timing, loop gain.
//!
//! All of these work on real waveforms (`mag`, `db20`, `real` … turn a complex one into
//! one) and interpolate linearly between samples. Functions that look at a frequency
//! response (`unity_gain_freq`, `bandwidth`, margins) interpolate on log x.

use eyre::{Result, bail, eyre};

use super::wave::{C, Samples, Waveform};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Rise,
    Fall,
    Either,
}

impl Edge {
    pub fn parse(s: &str) -> Result<Edge> {
        match s.to_lowercase().as_str() {
            "rise" | "rising" | "r" | "up" | "pos" => Ok(Edge::Rise),
            "fall" | "falling" | "f" | "down" | "neg" => Ok(Edge::Fall),
            "either" | "both" | "any" | "cross" | "" => Ok(Edge::Either),
            other => {
                bail!("unknown edge '{other}' (rise, fall or either)");
            }
        }
    }
}

/// An x range, either end open.
#[derive(Debug, Clone, Copy, Default)]
pub struct Range {
    pub from: Option<f64>,
    pub to: Option<f64>,
}

impl Range {
    pub fn apply(&self, w: &Waveform) -> Waveform {
        match (self.from, self.to) {
            (None, None) => w.clone(),
            (f, t) => {
                let lo = f.unwrap_or(f64::NEG_INFINITY);
                let hi = t.unwrap_or(f64::INFINITY);
                w.clip(lo, hi)
            }
        }
    }
}

fn interp_x(x0: f64, x1: f64, y0: f64, y1: f64, level: f64, log_x: bool) -> f64 {
    if y1 == y0 {
        return x1;
    }
    let t = (level - y0) / (y1 - y0);
    if log_x && x0 > 0.0 && x1 > 0.0 {
        (x0.ln() + t * (x1.ln() - x0.ln())).exp()
    } else {
        x0 + t * (x1 - x0)
    }
}

/// Every x where `y` crosses `level` in the given direction, in order.
pub fn crossings(x: &[f64], y: &[f64], level: f64, edge: Edge, log_x: bool) -> Vec<f64> {
    let mut out = vec![];
    for i in 1..x.len().min(y.len()) {
        let (a, b) = (y[i - 1] - level, y[i] - level);
        let rise = a < 0.0 && b >= 0.0;
        let fall = a > 0.0 && b <= 0.0;
        let hit = match edge {
            Edge::Rise => rise,
            Edge::Fall => fall,
            Edge::Either => rise || fall,
        };
        if hit {
            out.push(interp_x(x[i - 1], x[i], y[i - 1], y[i], level, log_x));
        }
    }
    out
}

/// The `n`-th crossing (1 is the first; -1 the last, -2 the one before). Within `range`.
pub fn cross(w: &Waveform, level: f64, edge: Edge, n: i64, range: Range) -> Result<f64> {
    let c = range.apply(w);
    let y = c.real_y("cross")?;
    let all = crossings(&c.x, y, level, edge, false);
    pick(&all, n).ok_or_else(|| eyre!("no crossing number {n} of {level} ({} found)", all.len()))
}

pub fn pick(all: &[f64], n: i64) -> Option<f64> {
    match n {
        0 => None,
        n if n > 0 => all.get(n as usize - 1).copied(),
        n => all.len().checked_sub((-n) as usize).map(|i| all[i]),
    }
}

/// `(min, max)` of the samples.
pub fn extremes(y: &[f64]) -> Option<(f64, f64)> {
    let mut it = y.iter().copied().filter(|v| !v.is_nan());
    let first = it.next()?;
    Some(it.fold((first, first), |(lo, hi), v| (lo.min(v), hi.max(v))))
}

fn nonempty(w: &Waveform, what: &str) -> Result<()> {
    if w.is_empty() {
        bail!("{what}: the waveform has no points in that range");
    }
    Ok(())
}

pub fn min(w: &Waveform) -> Result<f64> {
    nonempty(w, "min")?;
    Ok(extremes(w.real_y("min")?).map_or(f64::NAN, |e| e.0))
}

pub fn max(w: &Waveform) -> Result<f64> {
    nonempty(w, "max")?;
    Ok(extremes(w.real_y("max")?).map_or(f64::NAN, |e| e.1))
}

/// x of the first minimum / maximum.
pub fn argext(w: &Waveform, want_max: bool) -> Result<f64> {
    nonempty(w, "minx/maxx")?;
    let y = w.real_y("minx/maxx")?;
    let mut best = 0;
    for i in 1..y.len() {
        if (want_max && y[i] > y[best]) || (!want_max && y[i] < y[best]) {
            best = i;
        }
    }
    Ok(w.x[best])
}

/// Trapezoid integral over the whole waveform.
pub fn integrate(w: &Waveform) -> Result<f64> {
    let y = w.real_y("integrate")?;
    Ok((1..w.len())
        .map(|i| 0.5 * (y[i] + y[i - 1]) * (w.x[i] - w.x[i - 1]))
        .sum())
}

/// Time-weighted average (the integral over the x span divided by it).
pub fn avg(w: &Waveform) -> Result<f64> {
    nonempty(w, "avg")?;
    let y = w.real_y("avg")?;
    let span = w.x[w.len() - 1] - w.x[0];
    if w.len() == 1 || span == 0.0 {
        return Ok(y[0]);
    }
    Ok(integrate(w)? / span)
}

pub fn rms(w: &Waveform) -> Result<f64> {
    nonempty(w, "rms")?;
    let y = w.real_y("rms")?;
    let span = w.x[w.len() - 1] - w.x[0];
    if w.len() == 1 || span == 0.0 {
        return Ok(y[0].abs());
    }
    let sq: f64 = (1..w.len())
        .map(|i| {
            // Exact integral of a linear segment squared.
            let (a, b) = (y[i - 1], y[i]);
            (a * a + a * b + b * b) / 3.0 * (w.x[i] - w.x[i - 1])
        })
        .sum();
    Ok((sq / span).sqrt())
}

pub fn peak_to_peak(w: &Waveform) -> Result<f64> {
    Ok(max(w)? - min(w)?)
}

/// d/dx, central differences (one-sided at the ends).
pub fn derivative(w: &Waveform) -> Result<Waveform> {
    let y = w.real_y("derivative")?;
    let n = w.len();
    if n < 2 {
        bail!("derivative needs two points");
    }
    let d: Vec<f64> = (0..n)
        .map(|i| {
            let (a, b) = if i == 0 {
                (0, 1)
            } else if i == n - 1 {
                (n - 2, n - 1)
            } else {
                (i - 1, i + 1)
            };
            (y[b] - y[a]) / (w.x[b] - w.x[a])
        })
        .collect();
    Ok(w.with_y(d, ""))
}

/// Running trapezoid integral from the first point.
pub fn integral(w: &Waveform) -> Result<Waveform> {
    let y = w.real_y("integral")?;
    let mut acc = 0.0;
    let out: Vec<f64> = (0..w.len())
        .map(|i| {
            if i > 0 {
                acc += 0.5 * (y[i] + y[i - 1]) * (w.x[i] - w.x[i - 1]);
            }
            acc
        })
        .collect();
    Ok(w.with_y(out, ""))
}

/// Phase in degrees, unwrapped along x and started in (-180, 180].
pub fn phase_deg(w: &Waveform) -> Waveform {
    let mut out = Vec::with_capacity(w.len());
    let mut prev = 0.0;
    let mut offset = 0.0;
    for i in 0..w.len() {
        let mut p = w.y.at(i).arg().to_degrees();
        if i > 0 {
            let jump = p + offset - prev;
            if jump > 180.0 {
                offset -= 360.0 * ((jump + 180.0) / 360.0).floor();
            } else if jump < -180.0 {
                offset += 360.0 * ((-jump + 180.0) / 360.0).floor();
            }
        }
        p += offset;
        prev = p;
        out.push(p);
    }
    w.with_y(out, "deg")
}

pub fn db20(w: &Waveform) -> Waveform {
    w.map_c("dB", |c| 20.0 * c.abs().log10())
}

fn edge_levels(
    w: &Waveform,
    lo: f64,
    hi: f64,
    vlow: Option<f64>,
    vhigh: Option<f64>,
) -> Result<(f64, f64)> {
    let y = w.real_y("this measurement")?;
    let (mn, mx) = extremes(y).ok_or_else(|| eyre!("no points"))?;
    let (vl, vh) = (vlow.unwrap_or(mn), vhigh.unwrap_or(mx));
    if vh <= vl {
        bail!("the waveform does not swing ({vl} to {vh})");
    }
    Ok((vl + lo * (vh - vl), vl + hi * (vh - vl)))
}

/// Time of the `n`-th `lo`-to-`hi` (default 10 % to 90 %) edge and its slope in units/x.
/// Returns `(t_start, t_end, level_low, level_high)`.
pub struct EdgeTimes {
    pub t_low: f64,
    pub t_high: f64,
    pub low: f64,
    pub high: f64,
}

#[allow(clippy::too_many_arguments)]
pub fn edge(
    w: &Waveform,
    rising: bool,
    lo: f64,
    hi: f64,
    vlow: Option<f64>,
    vhigh: Option<f64>,
    n: i64,
    range: Range,
) -> Result<EdgeTimes> {
    let c = range.apply(w);
    let (l, h) = edge_levels(&c, lo, hi, vlow, vhigh)?;
    let y = c.real_y("edge time")?;
    let dir = if rising { Edge::Rise } else { Edge::Fall };
    // The first level an edge meets (low when rising, high when falling), then the other.
    let (first, second) = if rising { (l, h) } else { (h, l) };
    let starts = crossings(&c.x, y, first, dir, false);
    let t0 = pick(&starts, n).ok_or_else(|| {
        eyre!(
            "no {} edge number {n}",
            if rising { "rising" } else { "falling" }
        )
    })?;
    let t1 = crossings(&c.x, y, second, dir, false)
        .into_iter()
        .find(|&t| t >= t0)
        .ok_or_else(|| eyre!("the edge never reaches {second}"))?;
    Ok(EdgeTimes {
        t_low: t0,
        t_high: t1,
        low: l,
        high: h,
    })
}

pub fn rise_time(
    w: &Waveform,
    lo: f64,
    hi: f64,
    vlow: Option<f64>,
    vhigh: Option<f64>,
    n: i64,
    range: Range,
) -> Result<f64> {
    let e = edge(w, true, lo, hi, vlow, vhigh, n, range)?;
    Ok(e.t_high - e.t_low)
}

pub fn fall_time(
    w: &Waveform,
    lo: f64,
    hi: f64,
    vlow: Option<f64>,
    vhigh: Option<f64>,
    n: i64,
    range: Range,
) -> Result<f64> {
    let e = edge(w, false, lo, hi, vlow, vhigh, n, range)?;
    Ok(e.t_high - e.t_low)
}

/// Slope between the `lo` and `hi` points of an edge, units per x (always positive).
#[allow(clippy::too_many_arguments)]
pub fn slew(
    w: &Waveform,
    rising: bool,
    lo: f64,
    hi: f64,
    vlow: Option<f64>,
    vhigh: Option<f64>,
    n: i64,
    range: Range,
) -> Result<f64> {
    let e = edge(w, rising, lo, hi, vlow, vhigh, n, range)?;
    let dt = e.t_high - e.t_low;
    if dt <= 0.0 {
        bail!("the edge has no duration");
    }
    Ok((e.high - e.low) / dt)
}

/// Delay from the `trig_n`-th `trig_edge` crossing of `trig_level` in `trig` to the
/// first `targ_n`-th `targ_edge` crossing of `targ_level` in `targ` at or after it.
#[allow(clippy::too_many_arguments)]
pub fn delay(
    trig: &Waveform,
    targ: &Waveform,
    trig_level: f64,
    targ_level: f64,
    trig_edge: Edge,
    targ_edge: Edge,
    trig_n: i64,
    targ_n: i64,
    range: Range,
) -> Result<f64> {
    let tw = range.apply(trig);
    let gw = range.apply(targ);
    let t0 = pick(
        &crossings(&tw.x, tw.real_y("delay")?, trig_level, trig_edge, false),
        trig_n,
    )
    .ok_or_else(|| eyre!("the trigger waveform has no crossing number {trig_n} of {trig_level}"))?;
    let after: Vec<f64> = crossings(&gw.x, gw.real_y("delay")?, targ_level, targ_edge, false)
        .into_iter()
        .filter(|&t| t >= t0)
        .collect();
    let t1 = pick(&after, targ_n).ok_or_else(|| {
        eyre!(
            "the target waveform has no crossing number {targ_n} of {targ_level} after the trigger"
        )
    })?;
    Ok(t1 - t0)
}

/// The value the waveform ends at: the mean of its last `tail` fraction of x.
pub fn final_value(w: &Waveform, tail: f64) -> Result<f64> {
    nonempty(w, "final value")?;
    let last = w.x[w.len() - 1];
    let first = w.x[0];
    let c = w.clip(last - (last - first) * tail, last);
    avg(&c)
}

/// Percent overshoot of a step response: how far past the final value, relative to
/// the step (initial to final). Zero when there is none.
pub fn overshoot(w: &Waveform, initial: Option<f64>, fin: Option<f64>, upper: bool) -> Result<f64> {
    let y = w.real_y("overshoot")?;
    let init = initial.unwrap_or(y[0]);
    let fin = match fin {
        Some(f) => f,
        None => final_value(w, 0.05)?,
    };
    let step = fin - init;
    if step == 0.0 {
        bail!("overshoot: the response has no step (initial equals final)");
    }
    let (mn, mx) = extremes(y).unwrap();
    // `upper`: overshoot past the final value in the direction of the step; else the
    // undershoot past the initial value, the other way.
    let dev = match (step > 0.0, upper) {
        (true, true) => mx - fin,
        (false, true) => fin - mn,
        (true, false) => init - mn,
        (false, false) => mx - init,
    }
    .max(0.0);
    Ok(100.0 * dev / step.abs())
}

/// Time from `from` (default the first x) until the waveform stays within `tol` (a
/// fraction of the step) of its final value.
pub fn settling_time(
    w: &Waveform,
    tol: f64,
    initial: Option<f64>,
    fin: Option<f64>,
    from: Option<f64>,
) -> Result<f64> {
    let y = w.real_y("settling_time")?;
    nonempty(w, "settling_time")?;
    let init = initial.unwrap_or(y[0]);
    let fin = match fin {
        Some(f) => f,
        None => final_value(w, 0.05)?,
    };
    let band = tol * (fin - init).abs();
    if band == 0.0 {
        bail!("settling_time: the response has no step");
    }
    let t0 = from.unwrap_or(w.x[0]);
    let d: Vec<f64> = y.iter().map(|v| (v - fin).abs() - band).collect();
    let n = w.len();
    if d[n - 1] > 0.0 {
        bail!(
            "the waveform has not settled within {}% by its end",
            tol * 100.0
        );
    }
    let Some(i) = (0..n).rev().find(|&i| d[i] > 0.0) else {
        return Ok(0.0);
    };
    // d[i] > 0 outside the band, d[i+1] <= 0 inside: where the band is entered.
    let t = interp_x(w.x[i], w.x[i + 1], d[i], d[i + 1], 0.0, false);
    Ok((t - t0).max(0.0))
}

/// Mean frequency from the crossings of `level` (default the middle of the swing).
pub fn frequency(w: &Waveform, level: Option<f64>, edge: Edge, range: Range) -> Result<f64> {
    let c = range.apply(w);
    let y = c.real_y("frequency")?;
    let level = match level {
        Some(l) => l,
        None => extremes(y)
            .map(|(a, b)| 0.5 * (a + b))
            .ok_or_else(|| eyre!("no points"))?,
    };
    let t = crossings(
        &c.x,
        y,
        level,
        if edge == Edge::Either {
            Edge::Rise
        } else {
            edge
        },
        false,
    );
    if t.len() < 2 {
        bail!(
            "frequency: fewer than two {} crossings of {level}",
            if edge == Edge::Fall {
                "falling"
            } else {
                "rising"
            }
        );
    }
    Ok((t.len() - 1) as f64 / (t[t.len() - 1] - t[0]))
}

/// Duty cycle in percent: the share of a period the waveform is above `level`.
pub fn duty(w: &Waveform, level: Option<f64>, range: Range) -> Result<f64> {
    let c = range.apply(w);
    let y = c.real_y("duty")?;
    let level = match level {
        Some(l) => l,
        None => extremes(y)
            .map(|(a, b)| 0.5 * (a + b))
            .ok_or_else(|| eyre!("no points"))?,
    };
    let r = crossings(&c.x, y, level, Edge::Rise, false);
    let f = crossings(&c.x, y, level, Edge::Fall, false);
    let mut shares = vec![];
    for k in 0..r.len().saturating_sub(1) {
        if let Some(&fall) = f.iter().find(|&&t| t > r[k] && t < r[k + 1]) {
            shares.push((fall - r[k]) / (r[k + 1] - r[k]));
        }
    }
    if shares.is_empty() {
        bail!("duty: no full period");
    }
    Ok(100.0 * shares.iter().sum::<f64>() / shares.len() as f64)
}

fn log_ok(w: &Waveform) -> bool {
    w.x.iter().all(|&x| x > 0.0)
}

/// The frequency where the gain, `|w|` in dB, first falls through 0 dB.
pub fn unity_gain_freq(w: &Waveform) -> Result<f64> {
    let db = db20(w);
    let y = db.real_y("unity_gain_freq")?;
    let c = crossings(&db.x, y, 0.0, Edge::Fall, log_ok(w));
    c.first()
        .copied()
        .ok_or_else(|| eyre!("the gain never falls through 0 dB"))
}

/// The frequency where the gain first falls `drop` dB (default 3) below its first value
/// (or below `reference`, in dB).
pub fn bandwidth(w: &Waveform, drop: f64, reference: Option<f64>) -> Result<f64> {
    nonempty(w, "bandwidth")?;
    let db = db20(w);
    let y = db.real_y("bandwidth")?;
    let level = reference.unwrap_or(y[0]) - drop;
    let c = crossings(&db.x, y, level, Edge::Fall, log_ok(w));
    c.first().copied().ok_or_else(|| {
        eyre!(
            "the gain never falls {drop} dB below {:.2} dB",
            level + drop
        )
    })
}

fn complex_loop(w: &Waveform, what: &str) -> Result<()> {
    if !matches!(w.y, Samples::Complex(_)) {
        bail!("{what} needs the complex loop gain (an AC result, not a magnitude)");
    }
    Ok(())
}

/// Phase margin in degrees: 180 plus the phase of the loop gain at its unity-gain
/// frequency. `w` is the loop gain with the sign of the feedback included, so its
/// phase starts near 0.
pub fn phase_margin(w: &Waveform) -> Result<f64> {
    complex_loop(w, "phase_margin")?;
    let f = unity_gain_freq(w)?;
    let ph = phase_deg(w);
    let at = at_log(&ph, f).ok_or_else(|| eyre!("no phase at {f}"))?;
    Ok(180.0 + at)
}

/// Gain margin in dB: minus the gain where the phase first reaches -180 degrees.
pub fn gain_margin(w: &Waveform) -> Result<f64> {
    complex_loop(w, "gain_margin")?;
    let ph = phase_deg(w);
    let y = ph.real_y("gain_margin")?;
    let c = crossings(&ph.x, y, -180.0, Edge::Fall, log_ok(w));
    let f = c
        .first()
        .copied()
        .ok_or_else(|| eyre!("the phase never reaches -180 degrees"))?;
    let gain = at_log(&db20(w), f).ok_or_else(|| eyre!("no gain at {f}"))?;
    Ok(-gain)
}

/// Value at `x`, interpolating on log x where the grid allows.
pub fn at_log(w: &Waveform, at: f64) -> Option<f64> {
    if at <= 0.0 || !log_ok(w) {
        return w.value_at(at).map(|c| c.re);
    }
    let n = w.len();
    let y = w.real_y("at").ok()?;
    let idx = w.x.partition_point(|&v| v < at);
    if idx == 0 || idx >= n {
        return w.value_at(at).map(|c| c.re);
    }
    let (x0, x1) = (w.x[idx - 1].ln(), w.x[idx].ln());
    let t = (at.ln() - x0) / (x1 - x0);
    Some(y[idx - 1] + t * (y[idx] - y[idx - 1]))
}

/// Complex value at `x`.
pub fn value_at(w: &Waveform, at: f64) -> Result<C> {
    w.value_at(at).ok_or_else(|| {
        eyre!(
            "{at} is outside the waveform's range ({} to {})",
            w.x.first().copied().unwrap_or(f64::NAN),
            w.x.last().copied().unwrap_or(f64::NAN)
        )
    })
}

/// The rms of a spectral density (V/sqrt(Hz), or V²/Hz with `squared`) integrated over
/// the range, in V.
pub fn integrate_noise(w: &Waveform, squared: bool) -> Result<f64> {
    let y = w.magnitude();
    let p: Vec<f64> = y.iter().map(|&v| if squared { v } else { v * v }).collect();
    let total: f64 = (1..w.len())
        .map(|i| 0.5 * (p[i] + p[i - 1]) * (w.x[i] - w.x[i - 1]))
        .sum();
    Ok(total.sqrt())
}

/// The tone frequency for coherent sampling: `m * fs / n` with the odd integer `m`
/// nearest to `fin * n / fs` (and coprime with `n`), so `n` samples hold exactly `m`
/// cycles and no window is needed.
pub fn coherent_frequency(fin: f64, fs: f64, n: f64) -> Result<f64> {
    if !(fin > 0.0 && fs > 0.0 && n >= 4.0) || !(fin.is_finite() && fs.is_finite() && n.is_finite()) {
        eyre::bail!("coherent needs fin > 0, fs > 0 and n >= 4 (got {fin}, {fs}, {n})");
    }
    if fin >= fs / 2.0 {
        eyre::bail!("coherent: the tone {fin} Hz is not below fs/2 = {} Hz", fs / 2.0);
    }
    let n = n.round() as u64;
    let gcd = |mut a: u64, mut b: u64| {
        while b != 0 {
            (a, b) = (b, a % b);
        }
        a
    };
    let target = (fin * n as f64 / fs).round().max(1.0) as u64;
    let ok = |m: u64| m % 2 == 1 && gcd(m, n) == 1 && (m as f64) < n as f64 / 2.0;
    let m = (0..n)
        .flat_map(|d| [target.checked_sub(d), Some(target + d)])
        .flatten()
        .find(|m| *m >= 1 && ok(*m))
        .ok_or_else(|| eyre::eyre!("coherent: no coherent tone below fs/2 for n = {n}"))?;
    Ok(m as f64 * fs / n as f64)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn rc_step(tau: f64, n: usize, t_end: f64) -> Waveform {
        let x: Vec<f64> = (0..n).map(|i| t_end * i as f64 / (n - 1) as f64).collect();
        let y = x.iter().map(|t| 1.0 - (-t / tau).exp()).collect();
        Waveform::real(x, y)
    }

    #[test]
    fn crossings_and_extremes() {
        let w = Waveform::real(vec![0.0, 1.0, 2.0, 3.0, 4.0], vec![0.0, 2.0, 0.0, 2.0, 0.0]);
        let r = crossings(&w.x, w.real_y("t").unwrap(), 1.0, Edge::Rise, false);
        assert_eq!(r, [0.5, 2.5]);
        assert_eq!(
            cross(&w, 1.0, Edge::Fall, 2, Range::default()).unwrap(),
            3.5
        );
        assert_eq!(
            cross(&w, 1.0, Edge::Either, -1, Range::default()).unwrap(),
            3.5
        );
        assert!(cross(&w, 5.0, Edge::Rise, 1, Range::default()).is_err());
        assert_eq!(
            (
                min(&w).unwrap(),
                max(&w).unwrap(),
                peak_to_peak(&w).unwrap()
            ),
            (0.0, 2.0, 2.0)
        );
        assert!((avg(&w).unwrap() - 1.0).abs() < 1e-12);
        assert!((rms(&w).unwrap() - (4.0f64 / 3.0).sqrt()).abs() < 1e-12);
    }

    #[test]
    fn rc_step_times() {
        let tau = 1e-9;
        let w = rc_step(tau, 20001, 10e-9);
        // 10-90 % of an RC step is tau * ln 9.
        let tr = rise_time(&w, 0.1, 0.9, Some(0.0), Some(1.0), 1, Range::default()).unwrap();
        assert!((tr - tau * 9f64.ln()).abs() < 1e-12, "{tr}");
        let sr = slew(
            &w,
            true,
            0.1,
            0.9,
            Some(0.0),
            Some(1.0),
            1,
            Range::default(),
        )
        .unwrap();
        assert!((sr - 0.8 / tr).abs() < 1.0);
        let ts = settling_time(&w, 0.02, None, Some(1.0), None).unwrap();
        assert!((ts - tau * (50f64).ln()).abs() < 1e-11, "{ts}");
        // No overshoot on a first-order response.
        assert!(overshoot(&w, None, Some(1.0), true).unwrap().abs() < 1e-9);
        let fall = Waveform::real((*w.x).clone(), w.y.re().iter().map(|v| 1.0 - v).collect());
        let tf = fall_time(&fall, 0.1, 0.9, Some(0.0), Some(1.0), 1, Range::default()).unwrap();
        assert!((tf - tr).abs() < 1e-12);
    }

    #[test]
    fn delay_frequency_duty_overshoot() {
        let n = 40001;
        let x: Vec<f64> = (0..n).map(|i| 1e-6 * i as f64 / (n - 1) as f64).collect();
        // A 5 MHz square-ish sine, 30 % duty from a threshold on a sine.
        let a: Vec<f64> = x
            .iter()
            .map(|t| (2.0 * std::f64::consts::PI * 5e6 * t).sin())
            .collect();
        let b: Vec<f64> = x
            .iter()
            .map(|t| (2.0 * std::f64::consts::PI * 5e6 * (t - 20e-9)).sin())
            .collect();
        let (wa, wb) = (Waveform::real(x.clone(), a), Waveform::real(x.clone(), b));
        let d = delay(
            &wa,
            &wb,
            0.0,
            0.0,
            Edge::Rise,
            Edge::Rise,
            2,
            1,
            Range::default(),
        )
        .unwrap();
        assert!((d - 20e-9).abs() < 1e-11, "{d}");
        let f = frequency(&wa, Some(0.0), Edge::Rise, Range::default()).unwrap();
        assert!((f - 5e6).abs() < 1e3, "{f}");
        let dc = duty(&wa, Some(0.5), Range::default()).unwrap();
        // sin > 0.5 for 1/3 of the period.
        assert!((dc - 100.0 / 3.0).abs() < 0.1, "{dc}");
        // Second-order step, zeta = 0.5: overshoot exp(-pi zeta / sqrt(1-zeta^2)) = 16.3 %.
        let (zeta, wn) = (0.5f64, 1e9f64);
        let wd = wn * (1.0 - zeta * zeta).sqrt();
        let t: Vec<f64> = (0..20001).map(|i| 20e-9 * i as f64 / 20000.0).collect();
        let y: Vec<f64> = t
            .iter()
            .map(|&t| {
                1.0 - (-zeta * wn * t).exp() * ((wd * t).cos() + zeta * wn / wd * (wd * t).sin())
            })
            .collect();
        let step = Waveform::real(t, y);
        let os = overshoot(&step, Some(0.0), Some(1.0), true).unwrap();
        assert!((os - 16.3).abs() < 0.1, "{os}");
        let ts = settling_time(&step, 0.02, Some(0.0), Some(1.0), None).unwrap();
        assert!(ts > 5e-9 && ts < 10e-9, "{ts}");
    }

    /// Two-pole amplifier: A0 = 1000 (60 dB), poles at 1 kHz and 1 MHz.
    pub fn two_pole() -> Waveform {
        let (a0, p1, p2) = (1000.0, 1e3, 1e6);
        let f: Vec<f64> = (0..=800).map(|i| 10f64.powf(i as f64 / 100.0)).collect();
        let y: Vec<C> = f
            .iter()
            .map(|&f| {
                let den = C::new(1.0, f / p1).mul(C::new(1.0, f / p2));
                C::new(a0, 0.0).div(den)
            })
            .collect();
        Waveform::complex(f, y)
    }

    #[test]
    fn two_pole_amplifier() {
        let h = two_pole();
        // |H| = 1: solve numerically for the reference.
        let ugf = unity_gain_freq(&h).unwrap();
        let mag = |f: f64| 1000.0 / ((1.0 + (f / 1e3).powi(2)) * (1.0 + (f / 1e6).powi(2))).sqrt();
        assert!((mag(ugf) - 1.0).abs() < 2e-3, "ugf {ugf} -> {}", mag(ugf));
        let pm = phase_margin(&h).unwrap();
        let expect = 180.0 - (ugf / 1e3).atan().to_degrees() - (ugf / 1e6).atan().to_degrees();
        assert!((pm - expect).abs() < 0.2, "{pm} vs {expect}");
        let bw = bandwidth(&h, 3.0, None).unwrap();
        assert!((bw - 1e3).abs() < 15.0, "{bw}");
        // The phase never gets to -180 degrees with two poles.
        assert!(gain_margin(&h).is_err());
        // A third pole at 3 MHz: the margin exists and is 20 log |T| at -180.
        let t = Waveform::complex(
            (*h.x).clone(),
            (0..h.len())
                .map(|i| h.y.at(i).div(C::new(1.0, h.x[i] / 3e6)))
                .collect(),
        );
        let gm = gain_margin(&t).unwrap();
        assert!(gm.is_finite());
        assert!(phase_margin(&t).unwrap() < pm);
        let db = db20(&h);
        assert!((db.y.re()[0] - 60.0).abs() < 0.01);
        assert!(unity_gain_freq(&Waveform::real(vec![1.0, 2.0], vec![1.0, 2.0])).is_err());
    }

    #[test]
    fn noise_integration_and_unwrap() {
        // White 1 nV/sqrt(Hz) over 1 MHz is 1 uV rms.
        let w = Waveform::real(vec![1.0, 1e6], vec![1e-9, 1e-9]);
        assert!((integrate_noise(&w, false).unwrap() - 1e-6).abs() < 1e-12);
        let p = Waveform::complex(
            vec![0.0, 1.0, 2.0, 3.0],
            (0..4)
                .map(|k| C::new(1.0, 0.0).mul(C::new(0.0, -(k as f64) * 2.0).exp()))
                .collect(),
        );
        let ph = phase_deg(&p);
        assert!((ph.y.re()[3] + 343.775).abs() < 0.01, "{:?}", ph.y.re());
    }
}
