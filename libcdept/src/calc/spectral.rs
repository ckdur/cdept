//! Spectra and converter figures: FFT with windows, SNR / SNDR / SFDR / THD / ENOB,
//! INL and DNL.
//!
//! Transient waveforms from SPICE are not uniformly sampled, so a waveform that is not
//! on a uniform grid is resampled (linear interpolation) to `n` points over the
//! analysed range. For coherent sampling give a range with a whole number of signal
//! cycles; a uniform waveform whose length is a power of two is used as it is.

use std::f64::consts::PI;

use eyre::{Result, bail, eyre};

use super::measure::Range;
use super::wave::{C, Waveform};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Window {
    Rect,
    Hann,
    Blackman,
}

impl Window {
    pub fn parse(s: &str) -> Result<Window> {
        match s.to_lowercase().as_str() {
            "rect" | "rectangular" | "none" | "boxcar" => Ok(Window::Rect),
            "hann" | "hanning" => Ok(Window::Hann),
            "blackman" => Ok(Window::Blackman),
            other => {
                bail!("unknown window '{other}' (rect, hann or blackman)");
            }
        }
    }

    /// Periodic (DFT-even) coefficients, so a coherent signal stays on its bin.
    pub fn coefficients(self, n: usize) -> Vec<f64> {
        (0..n)
            .map(|k| {
                let p = 2.0 * PI * k as f64 / n as f64;
                match self {
                    Window::Rect => 1.0,
                    Window::Hann => 0.5 - 0.5 * p.cos(),
                    Window::Blackman => 0.42 - 0.5 * p.cos() + 0.08 * (2.0 * p).cos(),
                }
            })
            .collect()
    }

    /// Bins on each side of a tone that belong to it.
    fn half_width(self) -> usize {
        match self {
            Window::Rect => 1,
            Window::Hann => 3,
            Window::Blackman => 4,
        }
    }
}

/// In-place radix-2 FFT (`a.len()` a power of two).
pub fn fft_in_place(a: &mut [C]) {
    let n = a.len();
    debug_assert!(n.is_power_of_two());
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            a.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * PI / len as f64;
        let step = C::new(ang.cos(), ang.sin());
        for start in (0..n).step_by(len) {
            let mut w = C::new(1.0, 0.0);
            for k in 0..len / 2 {
                let (u, v) = (a[start + k], a[start + k + len / 2].mul(w));
                a[start + k] = u.add(v);
                a[start + k + len / 2] = u.sub(v);
                // Recompute now and then so the error does not build up in long stages.
                w = if k % 64 == 63 {
                    let t = ang * (k + 1) as f64;
                    C::new(t.cos(), t.sin())
                } else {
                    w.mul(step)
                };
            }
        }
        len <<= 1;
    }
}

/// Options of a spectral analysis.
#[derive(Debug, Clone)]
pub struct SpectrumOpts {
    pub window: Window,
    /// Number of points (power of two) to resample to; default from the data.
    pub n: Option<usize>,
    pub range: Range,
}

impl Default for SpectrumOpts {
    fn default() -> Self {
        SpectrumOpts {
            window: Window::Hann,
            n: None,
            range: Range::default(),
        }
    }
}

/// Uniform samples of `w` and their rate.
fn uniform(w: &Waveform, opts: &SpectrumOpts) -> Result<(Vec<f64>, f64)> {
    let c = opts.range.apply(w);
    let y = c.real_y("fft")?;
    if c.len() < 8 {
        bail!("fft: too few points ({})", c.len());
    }
    let dt = (c.x[c.len() - 1] - c.x[0]) / (c.len() - 1) as f64;
    let is_uniform =
        c.x.windows(2)
            .all(|p| ((p[1] - p[0]) - dt).abs() <= dt.abs() * 1e-6);
    if is_uniform
        && opts
            .n
            .is_none_or(|n| n == c.len().next_power_of_two() || n == c.len())
    {
        // Use the samples as they are, cut to the largest power of two.
        let mut n = c.len();
        if !n.is_power_of_two() {
            n = 1 << (usize::BITS - 1 - n.leading_zeros());
        }
        return Ok((y[..n].to_vec(), 1.0 / dt));
    }
    let n = opts
        .n
        .unwrap_or_else(|| c.len().next_power_of_two().clamp(256, 1 << 22));
    if !n.is_power_of_two() || n < 8 {
        bail!("fft: n must be a power of two (got {n})");
    }
    let (x0, x1) = (c.x[0], c.x[c.len() - 1]);
    let span = x1 - x0;
    if span <= 0.0 {
        bail!("fft: the range has no extent");
    }
    // N points over [x0, x1) so a whole number of cycles in the range is periodic.
    let step = span / n as f64;
    let out: Vec<f64> = (0..n)
        .map(|k| c.value_at(x0 + k as f64 * step).map_or(0.0, |v| v.re))
        .collect();
    Ok((out, 1.0 / step))
}

/// The windowed transform: `(X, fs)` with X the whole N-point transform.
fn transform(w: &Waveform, opts: &SpectrumOpts) -> Result<(Vec<C>, f64, f64)> {
    let (samples, fs) = uniform(w, opts)?;
    let n = samples.len();
    let win = opts.window.coefficients(n);
    let gain = win.iter().sum::<f64>() / n as f64;
    let mut a: Vec<C> = samples
        .iter()
        .zip(&win)
        .map(|(s, w)| C::new(s * w, 0.0))
        .collect();
    fft_in_place(&mut a);
    Ok((a, fs, gain))
}

/// The one-sided spectrum as a complex waveform over frequency: amplitude (peak) of
/// each component, window gain corrected, so a sine of amplitude A shows A at its bin.
pub fn spectrum(w: &Waveform, opts: &SpectrumOpts) -> Result<Waveform> {
    let (a, fs, gain) = transform(w, opts)?;
    let n = a.len();
    let half = n / 2;
    let f: Vec<f64> = (0..=half).map(|k| k as f64 * fs / n as f64).collect();
    let y: Vec<C> = (0..=half)
        .map(|k| {
            let scale = if k == 0 || k == half { 1.0 } else { 2.0 } / (n as f64 * gain);
            a[k].scale(scale)
        })
        .collect();
    let mut out = Waveform::complex(f, y);
    out.x_name = "frequency".into();
    out.x_unit = "Hz".into();
    out.name = format!("fft({})", w.name);
    out.unit = w.unit.clone();
    Ok(out)
}

/// What a tone analysis found.
#[derive(Debug, Clone, PartialEq)]
pub struct ToneAnalysis {
    pub fundamental_hz: f64,
    pub fundamental_bin: usize,
    /// Signal, harmonics, everything else (noise and spurs), as powers in arbitrary
    /// common units.
    pub signal: f64,
    pub harmonics: f64,
    pub noise: f64,
    /// Power of the strongest bin outside the signal and DC bins.
    pub worst_spur: f64,
    pub peak: f64,
}

impl ToneAnalysis {
    pub fn snr_db(&self) -> f64 {
        10.0 * (self.signal / self.noise).log10()
    }
    pub fn sndr_db(&self) -> f64 {
        10.0 * (self.signal / (self.noise + self.harmonics)).log10()
    }
    /// Harmonics relative to the signal, in dB (negative for a clean tone).
    pub fn thd_db(&self) -> f64 {
        10.0 * (self.harmonics / self.signal).log10()
    }
    pub fn sfdr_db(&self) -> f64 {
        10.0 * (self.peak / self.worst_spur).log10()
    }
    pub fn enob(&self) -> f64 {
        (self.sndr_db() - 1.76) / 6.02
    }
}

/// Find the tone (the largest bin, or the bin of `fund` Hz) and sum the power of the
/// signal, its first `harmonics` harmonics (2..=harmonics, aliased back below fs/2) and
/// the rest.
pub fn analyze(
    w: &Waveform,
    opts: &SpectrumOpts,
    fund: Option<f64>,
    harmonics: usize,
) -> Result<ToneAnalysis> {
    let (a, fs, _) = transform(w, opts)?;
    let n = a.len();
    let half = n / 2;
    let hw = opts.window.half_width();
    let p: Vec<f64> = (0..=half)
        .map(|k| a[k].re * a[k].re + a[k].im * a[k].im)
        .collect();
    // Bins taken by DC and the window's lobe around it.
    let dc_end = hw.min(half);
    let kf = match fund {
        Some(f) => {
            let center = (f * n as f64 / fs).round() as usize;
            let lo = center.saturating_sub(hw).max(dc_end + 1);
            let hi = (center + hw).min(half);
            (lo..=hi)
                .max_by(|&i, &j| p[i].total_cmp(&p[j]))
                .ok_or_else(|| eyre!("the fundamental {f} Hz is outside the spectrum"))?
        }
        None => (dc_end + 1..=half)
            .max_by(|&i, &j| p[i].total_cmp(&p[j]))
            .ok_or_else(|| eyre!("no spectrum"))?,
    };
    // 0 noise, 1 dc, 2 signal, 3 harmonic.
    let mut role = vec![0u8; half + 1];
    for r in role.iter_mut().take(dc_end + 1) {
        *r = 1;
    }
    let mark = |role: &mut Vec<u8>, center: usize, value: u8| {
        let (lo, hi) = (center.saturating_sub(hw), (center + hw).min(half));
        for r in &mut role[lo..=hi] {
            if *r == 0 {
                *r = value;
            }
        }
    };
    mark(&mut role, kf, 2);
    for h in 2..=harmonics.max(1) {
        let m = (h * kf) % n;
        let bin = if m > half { n - m } else { m };
        if bin > dc_end {
            mark(&mut role, bin, 3);
        }
    }
    let sum = |r: u8| {
        (0..=half)
            .filter(|&k| role[k] == r)
            .map(|k| p[k])
            .sum::<f64>()
    };
    let worst_spur = (0..=half)
        .filter(|&k| role[k] == 0 || role[k] == 3)
        .map(|k| p[k])
        .fold(0.0, f64::max);
    let (signal, harm, noise) = (sum(2), sum(3), sum(0));
    if signal <= 0.0 {
        bail!("no signal found");
    }
    Ok(ToneAnalysis {
        fundamental_hz: kf as f64 * fs / n as f64,
        fundamental_bin: kf,
        signal,
        harmonics: harm,
        noise: noise.max(f64::MIN_POSITIVE),
        worst_spur: worst_spur.max(f64::MIN_POSITIVE),
        peak: p[kf],
    })
}

/// DNL (and INL) of a converter, in LSB, as a waveform over the code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinearityMethod {
    /// `w` is the output code against a swept input (a ramp): transitions are the
    /// inputs where the code steps up.
    Edge,
    /// `w` is the output code sampled while a slow ramp covers the input range
    /// uniformly: code counts give the widths.
    Histogram,
}

/// DNL and INL in LSB. `lsb` overrides the ideal step (edge method; the end-point fit
/// by default).
pub fn linearity(
    w: &Waveform,
    method: LinearityMethod,
    lsb: Option<f64>,
) -> Result<(Waveform, Waveform)> {
    let y = w.real_y("inl/dnl")?;
    let (lo, hi) = super::measure::extremes(y).ok_or_else(|| eyre!("no points"))?;
    let (lo, hi) = (lo.round() as i64, hi.round() as i64);
    if hi - lo < 3 {
        bail!("inl/dnl: the code spans {} values", hi - lo + 1);
    }
    match method {
        LinearityMethod::Edge => {
            // transition[k] = x where the code first reaches k (k = lo+1 ..= hi).
            let mut edges = vec![];
            for k in lo + 1..=hi {
                let t = super::measure::crossings(
                    &w.x,
                    y,
                    k as f64 - 0.5,
                    super::measure::Edge::Rise,
                    false,
                )
                .first()
                .copied()
                .ok_or_else(|| eyre!("code {k} never appears (missing code)"))?;
                edges.push(t);
            }
            let n = edges.len();
            let step = lsb.unwrap_or((edges[n - 1] - edges[0]) / (n - 1) as f64);
            let codes: Vec<f64> = (lo + 1..=hi).map(|k| k as f64).collect();
            let inl: Vec<f64> = (0..n)
                .map(|i| (edges[i] - edges[0]) / step - i as f64)
                .collect();
            let dnl: Vec<f64> = (0..n - 1)
                .map(|i| (edges[i + 1] - edges[i]) / step - 1.0)
                .collect();
            let dnl_codes = codes[..n - 1].to_vec();
            let mut d = Waveform::real(dnl_codes, dnl);
            d.name = "dnl".into();
            d.unit = "LSB".into();
            let mut i = Waveform::real(codes, inl);
            i.name = "inl".into();
            i.unit = "LSB".into();
            Ok((d, i))
        }
        LinearityMethod::Histogram => {
            let mut counts = vec![0f64; (hi - lo + 1) as usize];
            for v in y {
                counts[(v.round() as i64 - lo) as usize] += 1.0;
            }
            // The end codes are clipped by the ramp's overrange; leave them out.
            let inner = &counts[1..counts.len() - 1];
            let mean = inner.iter().sum::<f64>() / inner.len() as f64;
            let dnl: Vec<f64> = inner.iter().map(|c| c / mean - 1.0).collect();
            let mut acc = 0.0;
            let mut inl: Vec<f64> = dnl
                .iter()
                .map(|d| {
                    acc += d;
                    acc
                })
                .collect();
            // End-point fit.
            let n = inl.len();
            let (first, last) = (inl[0], inl[n - 1]);
            for (i, v) in inl.iter_mut().enumerate() {
                *v -= first + (last - first) * i as f64 / (n - 1).max(1) as f64;
            }
            let codes: Vec<f64> = (lo + 1..hi).map(|k| k as f64).collect();
            let mut d = Waveform::real(codes.clone(), dnl);
            d.name = "dnl".into();
            d.unit = "LSB".into();
            let mut i = Waveform::real(codes, inl);
            i.name = "inl".into();
            i.unit = "LSB".into();
            Ok((d, i))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(n: usize, cycles: f64, amp: f64, extra: impl Fn(f64) -> f64) -> Waveform {
        let dt = 1e-9;
        let x: Vec<f64> = (0..n).map(|k| k as f64 * dt).collect();
        let y: Vec<f64> = (0..n)
            .map(|k| {
                let ph = 2.0 * PI * cycles * k as f64 / n as f64;
                amp * ph.sin() + extra(ph)
            })
            .collect();
        Waveform::real(x, y)
    }

    #[test]
    fn fft_of_a_sine_is_one_bin() {
        let w = sine(1024, 37.0, 0.5, |_| 0.0);
        for win in [Window::Rect, Window::Hann, Window::Blackman] {
            let s = spectrum(
                &w,
                &SpectrumOpts {
                    window: win,
                    ..Default::default()
                },
            )
            .unwrap();
            let m = s.magnitude();
            let (k, peak) = m
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(b.1))
                .unwrap();
            assert_eq!(k, 37);
            assert!((peak - 0.5).abs() < 1e-9, "{win:?}: {peak}");
            assert!((s.x[37] - 37.0 / (1024.0 * 1e-9)).abs() < 1.0);
        }
    }

    #[test]
    fn thd_of_a_distorted_sine() {
        // A third harmonic at 1 % of the fundamental: -40 dB.
        let w = sine(4096, 127.0, 1.0, |ph| 0.01 * (3.0 * ph).sin());
        for win in [Window::Rect, Window::Hann, Window::Blackman] {
            let t = analyze(
                &w,
                &SpectrumOpts {
                    window: win,
                    ..Default::default()
                },
                None,
                5,
            )
            .unwrap();
            assert_eq!(t.fundamental_bin, 127);
            assert!((t.thd_db() + 40.0).abs() < 0.05, "{win:?} {}", t.thd_db());
            assert!((t.sfdr_db() - 40.0).abs() < 0.05, "{win:?} {}", t.sfdr_db());
        }
        // A second and a fifth harmonic fold in as they should (127 * 5 aliases past fs/2).
        let f = analyze(
            &w,
            &SpectrumOpts::default(),
            Some(127.0 / (4096.0 * 1e-9)),
            3,
        )
        .unwrap();
        assert_eq!(f.fundamental_bin, 127);
    }

    #[test]
    fn quantised_sine_gives_enob() {
        for bits in [6u32, 8, 10] {
            let n = 16384;
            let levels = (1u64 << bits) as f64;
            // Full scale -1..1, step 2/levels; an odd number of cycles keeps it coherent.
            let w0 = sine(n, 1021.0, 1.0 - 1.0 / levels, |_| 0.0);
            let lsb = 2.0 / levels;
            let q: Vec<f64> =
                w0.y.re()
                    .iter()
                    .map(|v| ((v / lsb).round() + 0.0) * lsb)
                    .collect();
            let w = Waveform::real((*w0.x).clone(), q);
            let t = analyze(
                &w,
                &SpectrumOpts {
                    window: Window::Rect,
                    ..Default::default()
                },
                None,
                5,
            )
            .unwrap();
            let ideal = 6.02 * bits as f64 + 1.76;
            assert!(
                (t.sndr_db() - ideal).abs() < 1.5,
                "{bits} bits: {} vs {ideal}",
                t.sndr_db()
            );
            assert!((t.enob() - bits as f64).abs() < 0.3, "{bits}: {}", t.enob());
        }
    }

    #[test]
    fn snr_of_a_noisy_sine() {
        // Deterministic pseudo-noise of rms 0.01 on a unit sine: signal 0.5, noise 1e-4: 37 dB.
        let mut seed = 12345u64;
        let mut noise = || {
            let mut s = 0.0;
            for _ in 0..12 {
                seed = seed
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                s += (seed >> 11) as f64 / (1u64 << 53) as f64;
            }
            s - 6.0
        };
        let n = 8192;
        let x: Vec<f64> = (0..n).map(|k| k as f64 * 1e-9).collect();
        let y: Vec<f64> = (0..n)
            .map(|k| (2.0 * PI * 501.0 * k as f64 / n as f64).sin() + 0.01 * noise())
            .collect();
        let w = Waveform::real(x, y);
        let t = analyze(
            &w,
            &SpectrumOpts {
                window: Window::Rect,
                ..Default::default()
            },
            None,
            5,
        )
        .unwrap();
        assert!((t.snr_db() - 37.0).abs() < 0.7, "{}", t.snr_db());
    }

    #[test]
    fn resamples_an_irregular_grid() {
        // 40 cycles over 1 us on an irregular time grid: still one bin at 40 MHz.
        let n = 3000;
        let x: Vec<f64> = (0..n)
            .map(|k| {
                1e-6 * ((k as f64 / (n - 1) as f64).powf(1.0) + 0.0001 * (k % 7) as f64 / n as f64)
                    .min(1.0)
            })
            .collect();
        let mut x = x;
        x.sort_by(|a, b| a.total_cmp(b));
        x.dedup();
        let y: Vec<f64> = x.iter().map(|t| (2.0 * PI * 40e6 * t).sin()).collect();
        let w = Waveform::real(x, y);
        let t = analyze(
            &w,
            &SpectrumOpts {
                n: Some(4096),
                ..Default::default()
            },
            None,
            3,
        )
        .unwrap();
        assert!(
            (t.fundamental_hz - 40e6).abs() < 1e5,
            "{}",
            t.fundamental_hz
        );
        assert!(t.sfdr_db() > 40.0, "{}", t.sfdr_db());
    }

    #[test]
    fn dnl_inl_from_edges_and_histogram() {
        // A 3-bit converter on a ramp 0..8 V; one code (3) is 0.5 LSB wide and the next 1.5.
        let edges = [1.0, 2.0, 3.0, 3.5, 5.0, 6.0, 7.0];
        let x: Vec<f64> = (0..8000).map(|i| i as f64 * 0.001).collect();
        let code = |v: f64| edges.iter().filter(|&&e| v >= e).count() as f64;
        let w = Waveform::real(x.clone(), x.iter().map(|&v| code(v)).collect());
        let (dnl, inl) = linearity(&w, LinearityMethod::Edge, Some(1.0)).unwrap();
        // Code 3 (between edges 3.0 and 3.5) is 0.5 wide: DNL -0.5; code 4: +0.5.
        let d = dnl.y.re();
        assert!(
            (d[2] + 0.5).abs() < 2e-3 && (d[3] - 0.5).abs() < 2e-3,
            "{d:?}"
        );
        assert!(d[0].abs() < 2e-3);
        let i = inl.y.re();
        assert!(
            (i[3] + 0.5).abs() < 2e-3 || (i[3] - 0.0).abs() < 2e-3,
            "{i:?}"
        );
        let (hd, hi) = linearity(&w, LinearityMethod::Histogram, None).unwrap();
        let hd = hd.y.re();
        // Interior codes 1..6: widths 1,1,0.5,1.5,1,1 -> DNL against mean width 1.
        assert!(
            (hd[2] + 0.5).abs() < 0.02 && (hd[3] - 0.5).abs() < 0.02,
            "{hd:?}"
        );
        assert_eq!(hi.len(), 6);
        assert!(
            linearity(
                &Waveform::real(vec![0.0, 1.0], vec![0.0, 1.0]),
                LinearityMethod::Edge,
                None
            )
            .is_err()
        );
    }
}
