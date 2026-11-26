//! Clock and data timing statistics: periods, period jitter, cycle-to-cycle jitter, time
//! interval error (TIE), histograms and simple sample statistics; and the Welch power
//! spectral density.
//!
//! All the jitter figures start from the crossings of a level (default the middle of the
//! swing) on one edge (default rising), found by linear interpolation between samples as
//! [`super::measure::crossings`] does.

use eyre::{Result, bail};

use super::measure::{Edge, Range, crossings, extremes};
use super::spectral::{Window, fft_in_place};
use super::wave::{C, Waveform};

/// The crossing times of `w` (mid-swing unless `level`), at least `min` of them.
pub fn edges(
    w: &Waveform,
    level: Option<f64>,
    edge: Edge,
    range: Range,
    min: usize,
) -> Result<Vec<f64>> {
    let c = range.apply(w);
    let y = c.real_y("jitter")?;
    let level = match level {
        Some(l) => l,
        None => match extremes(y) {
            Some((a, b)) => 0.5 * (a + b),
            None => return Err(eyre::eyre!("the waveform has no finite values")),
        },
    };
    let t = crossings(&c.x, y, level, edge, false);
    if t.len() < min {
        bail!("{} crossings of {level} ({} needed)", t.len(), min);
    }
    Ok(t)
}

fn wave(x: Vec<f64>, y: Vec<f64>, name: &str, unit: &str) -> Waveform {
    let mut w = Waveform::real(x, y);
    w.name = name.into();
    w.unit = unit.into();
    w.x_name = "time".into();
    w.x_unit = "time".into();
    w
}

/// The length of every period: a waveform over the time each period starts.
pub fn periods(t: &[f64]) -> Waveform {
    let p: Vec<f64> = t.windows(2).map(|w| w[1] - w[0]).collect();
    wave(t[..t.len() - 1].to_vec(), p, "period", "time")
}

/// The difference of each period from the one before (cycle-to-cycle), over the time
/// the later period starts.
pub fn cycle_to_cycle(t: &[f64]) -> Waveform {
    let p: Vec<f64> = t.windows(2).map(|w| w[1] - w[0]).collect();
    let d: Vec<f64> = p.windows(2).map(|w| w[1] - w[0]).collect();
    wave(t[1..t.len() - 1].to_vec(), d, "cycle-to-cycle", "time")
}

/// Time interval error: each crossing against the ideal clock `t0 + k T`. `period` is
/// T when given (the ideal clock then starts at the first crossing plus the mean error),
/// else T and t0 are the least-squares fit of the crossings.
pub fn tie(t: &[f64], period: Option<f64>) -> Result<Waveform> {
    let n = t.len() as f64;
    let k: Vec<f64> = (0..t.len()).map(|i| i as f64).collect();
    let (slope, intercept) = match period {
        Some(p) => {
            if !(p.is_finite() && p > 0.0) {
                bail!("the period must be positive");
            }
            let mean_err = t.iter().zip(&k).map(|(t, k)| t - k * p).sum::<f64>() / n;
            (p, mean_err)
        }
        None => {
            let (mk, mt) = (k.iter().sum::<f64>() / n, t.iter().sum::<f64>() / n);
            let sxx: f64 = k.iter().map(|k| (k - mk).powi(2)).sum();
            let sxy: f64 = k.iter().zip(t).map(|(k, t)| (k - mk) * (t - mt)).sum();
            if sxx == 0.0 {
                bail!("too few crossings for a time interval error");
            }
            let s = sxy / sxx;
            (s, mt - s * mk)
        }
    };
    let e: Vec<f64> = t
        .iter()
        .zip(&k)
        .map(|(t, k)| t - (intercept + k * slope))
        .collect();
    Ok(wave(t.to_vec(), e, "TIE", "time"))
}

/// Mean, sample standard deviation, peak to peak of the finite values.
pub fn stats(v: &[f64]) -> Result<(f64, f64, f64)> {
    let f: Vec<f64> = v.iter().copied().filter(|x| x.is_finite()).collect();
    if f.is_empty() {
        bail!("no values");
    }
    let n = f.len() as f64;
    let mean = f.iter().sum::<f64>() / n;
    let sd = if f.len() > 1 {
        (f.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0)).sqrt()
    } else {
        0.0
    };
    let (lo, hi) = extremes(&f).unwrap_or((mean, mean));
    Ok((mean, sd, hi - lo))
}

/// Which figure of a set of intervals: `rms` (standard deviation, the default), `pp`
/// (peak to peak), `mean`, `max` (largest magnitude).
pub fn figure(v: &[f64], stat: &str) -> Result<f64> {
    let (mean, sd, pp) = stats(v)?;
    Ok(match stat.to_lowercase().as_str() {
        "rms" | "std" | "sigma" | "" => sd,
        "pp" | "pkpk" | "peak" => pp,
        "mean" | "avg" => mean,
        "max" => v
            .iter()
            .filter(|x| x.is_finite())
            .fold(0.0f64, |m, x| m.max(x.abs())),
        other => {
            return Err(eyre::eyre!(
                "unknown statistic '{other}' (rms, pp, mean, max)"
            ));
        }
    })
}

/// The `p`-th percentile (0..100) of the finite values, linear interpolation.
pub fn percentile(v: &[f64], p: f64) -> Result<f64> {
    if !(0.0..=100.0).contains(&p) {
        bail!("the percentile must be between 0 and 100");
    }
    let mut f: Vec<f64> = v.iter().copied().filter(|x| x.is_finite()).collect();
    if f.is_empty() {
        bail!("no values");
    }
    f.sort_by(f64::total_cmp);
    let pos = p / 100.0 * (f.len() - 1) as f64;
    let (i, frac) = (pos.floor() as usize, pos.fract());
    Ok(if i + 1 < f.len() {
        f[i] + frac * (f[i + 1] - f[i])
    } else {
        f[i]
    })
}

/// A histogram of the y values: a waveform of counts over the bin centres. `weighted`:
/// each sample counts with the x span it stands for (the time a waveform spends in a
/// bin), else every sample counts 1.
pub fn histogram(
    w: &Waveform,
    bins: usize,
    lo: Option<f64>,
    hi: Option<f64>,
    weighted: bool,
) -> Result<Waveform> {
    if bins == 0 || bins > 1_000_000 {
        bail!("bins must be between 1 and 1000000");
    }
    let y = w.real_y("histogram")?;
    let (mn, mx) = match extremes(y) {
        Some(e) => e,
        None => return Err(eyre::eyre!("the waveform has no finite values")),
    };
    let (lo, hi) = (lo.unwrap_or(mn), hi.unwrap_or(mx));
    if !(lo.is_finite() && hi.is_finite()) || hi < lo {
        bail!("bad histogram range {lo}..{hi}");
    }
    let width = if hi > lo {
        (hi - lo) / bins as f64
    } else {
        1.0
    };
    let mut counts = vec![0.0; bins];
    for (i, v) in y.iter().enumerate() {
        if !v.is_finite() || *v < lo || *v > hi {
            continue;
        }
        let b = (((v - lo) / width) as usize).min(bins - 1);
        let weight = if weighted {
            let left = if i > 0 { w.x[i] - w.x[i - 1] } else { 0.0 };
            let right = if i + 1 < w.len() {
                w.x[i + 1] - w.x[i]
            } else {
                0.0
            };
            0.5 * (left + right)
        } else {
            1.0
        };
        counts[b] += weight;
    }
    let centres: Vec<f64> = (0..bins).map(|b| lo + (b as f64 + 0.5) * width).collect();
    let mut out = Waveform::real(centres, counts);
    out.x_name = w.name.clone();
    out.x_unit = w.unit.clone();
    out.name = format!("histogram({})", w.name);
    out.unit = if weighted {
        w.x_unit.clone()
    } else {
        "count".into()
    };
    Ok(out)
}

/// Zero-order-hold samples of `w` every `period` from `offset` (default the first x):
/// the value of the last point at or before each instant (an ADC's output code read at
/// the sampling clock). At most `n` samples when given.
pub fn sample(
    w: &Waveform,
    period: f64,
    offset: Option<f64>,
    n: Option<usize>,
) -> Result<Waveform> {
    if !(period.is_finite() && period > 0.0) {
        bail!("the sampling period must be positive");
    }
    let y = w.real_y("sample")?;
    if w.is_empty() {
        bail!("the waveform is empty");
    }
    let (first, last) = (w.x[0], w.x[w.len() - 1]);
    let start = offset.unwrap_or(first);
    let count = ((last - start) / period + 1e-9).floor();
    if !(count >= 0.0) {
        bail!("the first sample ({start}) is after the end of the waveform ({last})");
    }
    if count >= 50_000_000.0 && n.is_none_or(|n| n >= 50_000_000) {
        bail!("{count:.0} samples: use a longer period or n=");
    }
    let mut count = count.min(5e7) as usize + 1;
    if let Some(n) = n {
        count = count.min(n);
    }
    let mut xs = Vec::with_capacity(count);
    let mut ys = Vec::with_capacity(count);
    let mut j = 0usize;
    for k in 0..count {
        let t = start + period * k as f64;
        while j + 1 < w.len() && w.x[j + 1] <= t {
            j += 1;
        }
        xs.push(t);
        ys.push(if t < first { f64::NAN } else { y[j] });
    }
    let mut out = Waveform::real(xs, ys);
    out.x_name = w.x_name.clone();
    out.x_unit = w.x_unit.clone();
    out.name = format!("sample({})", w.name);
    out.unit = w.unit.clone();
    Ok(out)
}

/// Welch power spectral density: `segments` overlapping (50 %) windowed segments of `n`
/// points (a power of two) from a uniform resampling, averaged. A real waveform over
/// frequency in units²/Hz, one-sided, so its integral over frequency is the variance
/// (the mean square without the DC bin).
pub fn psd(
    w: &Waveform,
    window: Window,
    n: Option<usize>,
    segments: usize,
    range: Range,
) -> Result<Waveform> {
    let c = range.apply(w);
    let y = c.real_y("psd")?;
    if c.len() < 16 {
        bail!("psd: too few points ({})", c.len());
    }
    let span = c.x[c.len() - 1] - c.x[0];
    if !(span > 0.0) {
        bail!("psd: the range has no extent");
    }
    let segments = segments.clamp(1, 1024);
    // Uniform samples are used as they are; anything else (SPICE's adaptive steps) is
    // resampled by linear interpolation onto a power-of-two grid over the range.
    let step = span / (c.len() - 1) as f64;
    let uniform =
        c.x.windows(2)
            .all(|p| ((p[1] - p[0]) - step).abs() <= step * 1e-6);
    let (grid, dt) = if uniform {
        (y.to_vec(), step)
    } else {
        let total = c.len().next_power_of_two().clamp(64, 1 << 22);
        let dt = span / total as f64;
        let s: Vec<f64> = (0..total)
            .map(|k| {
                c.value_at(c.x[0] + k as f64 * dt)
                    .map_or(f64::NAN, |v| v.re)
            })
            .collect();
        (s, dt)
    };
    // the largest power-of-two segment that gives `segments` half-overlapping ones
    let seg = match n {
        Some(n) => n,
        None => {
            let fit = 2 * grid.len() / (segments + 1);
            if fit < 16 {
                bail!("psd: too few points for {segments} segments");
            }
            1usize << (usize::BITS - 1 - fit.leading_zeros())
        }
    };
    if !seg.is_power_of_two() || seg < 16 || seg > 1 << 22 {
        bail!("psd: n must be a power of two between 16 and 4194304 (got {seg})");
    }
    let total = seg / 2 * (segments + 1);
    if total > grid.len() {
        bail!(
            "psd: {segments} segments of {seg} points need {total} samples, the range has {}",
            grid.len()
        );
    }
    let fs = 1.0 / dt;
    let samples = &grid[..total];
    if samples.iter().any(|v| !v.is_finite()) {
        bail!("psd: the waveform has gaps (NaN) in the range");
    }
    let win = window.coefficients(seg);
    let power: f64 = win.iter().map(|v| v * v).sum();
    let mut acc = vec![0.0; seg / 2 + 1];
    for s in 0..segments {
        let part = &samples[s * seg / 2..s * seg / 2 + seg];
        let mean = part.iter().sum::<f64>() / seg as f64;
        let mut a: Vec<C> = part
            .iter()
            .zip(&win)
            .map(|(v, w)| C::new((v - mean) * w, 0.0))
            .collect();
        fft_in_place(&mut a);
        for (k, out) in acc.iter_mut().enumerate() {
            let scale = if k == 0 || k == seg / 2 { 1.0 } else { 2.0 };
            *out += scale * (a[k].re * a[k].re + a[k].im * a[k].im) / (fs * power);
        }
    }
    let f: Vec<f64> = (0..=seg / 2).map(|k| k as f64 * fs / seg as f64).collect();
    let p: Vec<f64> = acc.iter().map(|v| v / segments as f64).collect();
    let mut out = Waveform::real(f, p);
    out.x_name = "frequency".into();
    out.x_unit = "Hz".into();
    out.name = format!("psd({})", w.name);
    out.unit = format!("{}^2/Hz", if w.unit.is_empty() { "unit" } else { &w.unit });
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A clock with a known jitter: period 1 ns, every edge moved by a fixed pattern.
    fn jittery_clock(moves: &[f64]) -> Waveform {
        let mut x = vec![];
        let mut y = vec![];
        for (k, m) in moves.iter().enumerate() {
            let t = k as f64 * 1e-9 + m;
            // a 10 ps ramp from 0 to 1 centred on t, then back down half a period later
            x.extend([t - 5e-12, t + 5e-12, t + 0.5e-9 - 5e-12, t + 0.5e-9 + 5e-12]);
            y.extend([0.0, 1.0, 1.0, 0.0]);
        }
        Waveform::real(x, y)
    }

    #[test]
    fn periods_cycle_to_cycle_and_tie() {
        let moves = [0.0, 2e-12, -2e-12, 0.0, 2e-12, -2e-12, 0.0, 2e-12];
        let w = jittery_clock(&moves);
        let t = edges(&w, Some(0.5), Edge::Rise, Range::default(), 3).unwrap();
        assert_eq!(t.len(), moves.len());
        let p = periods(&t);
        let pv = p.y.re();
        assert!((pv[0] - 1.002e-9).abs() < 1e-15 && (pv[1] - 0.996e-9).abs() < 1e-15);
        // period jitter peak to peak: 1.002 - 0.996 = 6 ps
        assert!((figure(&pv, "pp").unwrap() - 6e-12).abs() < 1e-16);
        let cc = cycle_to_cycle(&t).y.re();
        assert!((cc[0] + 6e-12).abs() < 1e-16, "{cc:?}");
        // TIE against the ideal 1 ns clock is the moves themselves (their mean is
        // removed: the ideal clock is placed at the mean)
        let e = tie(&t, Some(1e-9)).unwrap().y.re();
        let mean = moves.iter().sum::<f64>() / moves.len() as f64;
        for (e, m) in e.iter().zip(&moves) {
            assert!((e - (m - mean)).abs() < 1e-16);
        }
        // the least-squares fit of a perfect clock gives zero error
        let perfect = jittery_clock(&[0.0; 6]);
        let t = edges(&perfect, None, Edge::Rise, Range::default(), 3).unwrap();
        assert!(
            tie(&t, None)
                .unwrap()
                .y
                .re()
                .iter()
                .all(|e| e.abs() < 1e-18)
        );
        assert!(
            edges(
                &Waveform::real(vec![0.0, 1.0], vec![0.0, 0.0]),
                None,
                Edge::Rise,
                Range::default(),
                2
            )
            .is_err()
        );
        assert!(figure(&[1.0], "nonsense").is_err());
    }

    #[test]
    fn histograms_percentiles_and_samples() {
        let w = Waveform::real(
            vec![0.0, 1.0, 2.0, 3.0, 4.0],
            vec![0.0, 1.0, 1.0, 2.0, f64::NAN],
        );
        let h = histogram(&w, 2, None, None, false).unwrap();
        assert_eq!(h.y.re(), vec![1.0, 3.0]);
        let ht = histogram(&w, 2, None, None, true).unwrap();
        assert_eq!(ht.y.re(), vec![0.5, 3.0]);
        assert!(histogram(&w, 0, None, None, false).is_err());
        assert!(histogram(&w, 3, Some(2.0), Some(1.0), false).is_err());
        assert_eq!(percentile(&[3.0, 1.0, 2.0], 50.0).unwrap(), 2.0);
        assert_eq!(percentile(&[1.0, 2.0], 25.0).unwrap(), 1.25);
        assert!(percentile(&[f64::NAN], 50.0).is_err());
        // zero-order hold: the value at or before each instant
        let s = sample(&w, 1.5, None, None).unwrap();
        assert_eq!(*s.x, vec![0.0, 1.5, 3.0]);
        assert_eq!(s.y.re()[..2], [0.0, 1.0]);
        assert!(sample(&w, 0.0, None, None).is_err());
        assert!(sample(&w, 1.0, Some(10.0), None).is_err());
        assert!(sample(&w, 1e-30, None, None).is_err());
        assert_eq!(sample(&w, 1.0, None, Some(2)).unwrap().len(), 2);
    }

    #[test]
    fn psd_of_white_noise_and_a_tone() {
        // a tone of amplitude 1: its power 0.5 is the area of the PSD
        let n = 8192;
        let x: Vec<f64> = (0..n).map(|i| i as f64 * 1e-6).collect();
        let y: Vec<f64> = x
            .iter()
            .map(|t| (2.0 * std::f64::consts::PI * 12_345.0 * t).sin())
            .collect();
        let w = Waveform::real(x.clone(), y);
        let p = psd(&w, Window::Hann, None, 8, Range::default()).unwrap();
        let df = p.x[1] - p.x[0];
        let area: f64 = p.y.re().iter().sum::<f64>() * df;
        assert!((area - 0.5).abs() < 0.03, "{area}");
        // the peak is at the tone
        let ys = p.y.re();
        let k = (0..ys.len())
            .max_by(|&a, &b| ys[a].total_cmp(&ys[b]))
            .unwrap();
        assert!((p.x[k] - 12_345.0).abs() < 2.0 * df);
        // pseudo-random noise of variance 1/12: the area is the variance
        let mut s = 1u64;
        let noise: Vec<f64> = (0..n)
            .map(|_| {
                s = s
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                (s >> 11) as f64 / (1u64 << 53) as f64 - 0.5
            })
            .collect();
        let p = psd(
            &Waveform::real(x, noise),
            Window::Hann,
            None,
            16,
            Range::default(),
        )
        .unwrap();
        let df = p.x[1] - p.x[0];
        let area: f64 = p.y.re().iter().sum::<f64>() * df;
        assert!((area - 1.0 / 12.0).abs() < 0.01, "{area}");
        assert!(
            psd(
                &Waveform::real(vec![0.0, 1.0], vec![0.0, 1.0]),
                Window::Hann,
                None,
                4,
                Range::default()
            )
            .is_err()
        );
    }
}
