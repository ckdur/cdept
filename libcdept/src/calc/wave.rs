//! Waveforms and complex numbers.

use std::sync::Arc;

use eyre::{Result, bail};

/// A complex number (the simulators' AC vectors).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct C {
    pub re: f64,
    pub im: f64,
}

#[allow(clippy::should_implement_trait)]
impl C {
    pub const fn new(re: f64, im: f64) -> C {
        C { re, im }
    }
    pub fn abs(self) -> f64 {
        self.re.hypot(self.im)
    }
    /// Phase in radians, -pi..pi.
    pub fn arg(self) -> f64 {
        self.im.atan2(self.re)
    }
    pub fn add(self, o: C) -> C {
        C::new(self.re + o.re, self.im + o.im)
    }
    pub fn sub(self, o: C) -> C {
        C::new(self.re - o.re, self.im - o.im)
    }
    pub fn mul(self, o: C) -> C {
        C::new(
            self.re * o.re - self.im * o.im,
            self.re * o.im + self.im * o.re,
        )
    }
    pub fn div(self, o: C) -> C {
        let d = o.re * o.re + o.im * o.im;
        C::new(
            (self.re * o.re + self.im * o.im) / d,
            (self.im * o.re - self.re * o.im) / d,
        )
    }
    pub fn scale(self, k: f64) -> C {
        C::new(self.re * k, self.im * k)
    }
    pub fn exp(self) -> C {
        let e = self.re.exp();
        C::new(e * self.im.cos(), e * self.im.sin())
    }
    pub fn ln(self) -> C {
        C::new(self.abs().ln(), self.arg())
    }
    pub fn powc(self, o: C) -> C {
        if self.re == 0.0 && self.im == 0.0 {
            return if o.re == 0.0 && o.im == 0.0 {
                C::new(1.0, 0.0)
            } else {
                C::default()
            };
        }
        self.ln().mul(o).exp()
    }
    pub fn sqrt(self) -> C {
        let r = self.abs().sqrt();
        let a = self.arg() / 2.0;
        C::new(r * a.cos(), r * a.sin())
    }
}

/// The values of a waveform.
#[derive(Debug, Clone, PartialEq)]
pub enum Samples {
    Real(Vec<f64>),
    Complex(Vec<C>),
}

impl Samples {
    pub fn len(&self) -> usize {
        match self {
            Samples::Real(v) => v.len(),
            Samples::Complex(v) => v.len(),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// The real parts.
    pub fn re(&self) -> Vec<f64> {
        match self {
            Samples::Real(v) => v.clone(),
            Samples::Complex(v) => v.iter().map(|c| c.re).collect(),
        }
    }
    pub fn at(&self, i: usize) -> C {
        match self {
            Samples::Real(v) => C::new(v[i], 0.0),
            Samples::Complex(v) => v[i],
        }
    }
    pub fn is_complex(&self) -> bool {
        matches!(self, Samples::Complex(_))
    }
}

/// `y` against `x`. `x` is shared between waveforms that came from one plot.
#[derive(Debug, Clone, PartialEq)]
pub struct Waveform {
    pub x: Arc<Vec<f64>>,
    pub y: Samples,
    pub x_name: String,
    pub x_unit: String,
    pub name: String,
    pub unit: String,
}

impl Waveform {
    pub fn real(x: Vec<f64>, y: Vec<f64>) -> Waveform {
        Waveform {
            x: Arc::new(x),
            y: Samples::Real(y),
            x_name: String::new(),
            x_unit: String::new(),
            name: String::new(),
            unit: String::new(),
        }
    }

    pub fn complex(x: Vec<f64>, y: Vec<C>) -> Waveform {
        Waveform {
            x: Arc::new(x),
            y: Samples::Complex(y),
            x_name: String::new(),
            x_unit: String::new(),
            name: String::new(),
            unit: String::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.x.len()
    }

    pub fn is_empty(&self) -> bool {
        self.x.is_empty()
    }

    /// The real samples, or an error for a complex waveform (`what` names the caller).
    pub fn real_y(&self, what: &str) -> Result<&[f64]> {
        match &self.y {
            Samples::Real(v) => Ok(v),
            Samples::Complex(_) => {
                bail!(
                    "{what} needs a real waveform; take mag(), db20(), real() or imag() of a complex one"
                );
            }
        }
    }

    /// The same x, other y (real), with a new name.
    pub fn with_y(&self, y: Vec<f64>, unit: &str) -> Waveform {
        Waveform {
            x: self.x.clone(),
            y: Samples::Real(y),
            x_name: self.x_name.clone(),
            x_unit: self.x_unit.clone(),
            name: self.name.clone(),
            unit: unit.to_string(),
        }
    }

    pub fn with_complex(&self, y: Vec<C>) -> Waveform {
        Waveform {
            x: self.x.clone(),
            y: Samples::Complex(y),
            x_name: self.x_name.clone(),
            x_unit: self.x_unit.clone(),
            name: self.name.clone(),
            unit: self.unit.clone(),
        }
    }

    /// Value at `x` by linear interpolation (the ends hold; `None` outside the range).
    pub fn value_at(&self, at: f64) -> Option<C> {
        let n = self.x.len();
        if n == 0 || at.is_nan() {
            return None;
        }
        let (first, last) = (self.x[0], self.x[n - 1]);
        let eps = (last - first).abs() * 1e-12;
        if at < first.min(last) - eps || at > first.max(last) + eps {
            return None;
        }
        if n == 1 {
            return Some(self.y.at(0));
        }
        // x is ascending in practice; a descending sweep is handled by the search order.
        let ascending = last >= first;
        let idx = if ascending {
            self.x.partition_point(|&v| v < at)
        } else {
            self.x.partition_point(|&v| v > at)
        };
        if idx == 0 {
            return Some(self.y.at(0));
        }
        if idx >= n {
            return Some(self.y.at(n - 1));
        }
        let (x0, x1) = (self.x[idx - 1], self.x[idx]);
        let (y0, y1) = (self.y.at(idx - 1), self.y.at(idx));
        if x1 == x0 {
            return Some(y1);
        }
        let t = (at - x0) / (x1 - x0);
        Some(y0.add(y1.sub(y0).scale(t)))
    }

    /// Resample on the grid `xs` (points outside the range are dropped).
    pub fn sample_on(&self, xs: &[f64]) -> Waveform {
        let pts: Vec<(f64, C)> = xs
            .iter()
            .filter_map(|&x| self.value_at(x).map(|v| (x, v)))
            .collect();
        let x: Vec<f64> = pts.iter().map(|p| p.0).collect();
        let y = match &self.y {
            Samples::Real(_) => Samples::Real(pts.iter().map(|p| p.1.re).collect()),
            Samples::Complex(_) => Samples::Complex(pts.iter().map(|p| p.1).collect()),
        };
        Waveform {
            x: Arc::new(x),
            y,
            x_name: self.x_name.clone(),
            x_unit: self.x_unit.clone(),
            name: self.name.clone(),
            unit: self.unit.clone(),
        }
    }

    /// The part of the waveform with `from <= x <= to`, the ends interpolated so the
    /// range is exact.
    pub fn clip(&self, from: f64, to: f64) -> Waveform {
        let (lo, hi) = if from <= to { (from, to) } else { (to, from) };
        let mut x = vec![];
        let mut y = vec![];
        let mut push = |xv: f64, v: C| {
            x.push(xv);
            y.push(v);
        };
        let first = self.x.first().copied().unwrap_or(0.0);
        let last = self.x.last().copied().unwrap_or(0.0);
        if lo > last.max(first) || hi < first.min(last) || self.x.is_empty() {
            return Waveform {
                x: Arc::new(vec![]),
                y: if self.y.is_complex() {
                    Samples::Complex(vec![])
                } else {
                    Samples::Real(vec![])
                },
                ..self.clone()
            };
        }
        let lo = lo.max(first.min(last));
        let hi = hi.min(first.max(last));
        if let Some(v) = self.value_at(lo)
            && self.x.iter().all(|&v| v != lo)
        {
            push(lo, v);
        }
        for i in 0..self.x.len() {
            if self.x[i] >= lo && self.x[i] <= hi {
                push(self.x[i], self.y.at(i));
            }
        }
        if let Some(v) = self.value_at(hi)
            && self.x.iter().all(|&v| v != hi)
        {
            push(hi, v);
        }
        let y = match &self.y {
            Samples::Real(_) => Samples::Real(y.iter().map(|c| c.re).collect()),
            Samples::Complex(_) => Samples::Complex(y),
        };
        Waveform {
            x: Arc::new(x),
            y,
            ..self.clone()
        }
    }

    /// Apply `f` to every real sample.
    pub fn map(&self, unit: &str, f: impl Fn(f64) -> f64) -> Result<Waveform> {
        let y = self.real_y("this function")?;
        Ok(self.with_y(y.iter().map(|&v| f(v)).collect(), unit))
    }

    /// Apply `f` to every sample as a complex number; the result is real.
    pub fn map_c(&self, unit: &str, f: impl Fn(C) -> f64) -> Waveform {
        let y: Vec<f64> = (0..self.len()).map(|i| f(self.y.at(i))).collect();
        self.with_y(y, unit)
    }

    /// The values as reals: real waveforms as they are, complex ones as magnitudes.
    pub fn magnitude(&self) -> Vec<f64> {
        match &self.y {
            Samples::Real(v) => v.clone(),
            Samples::Complex(v) => v.iter().map(|c| c.abs()).collect(),
        }
    }

    /// Does the grid of `self` equal that of `other`?
    pub fn same_grid(&self, other: &Waveform) -> bool {
        Arc::ptr_eq(&self.x, &other.x) || *self.x == *other.x
    }
}

/// A binary operation on two waveforms: on the shared grid, or on the union of the two
/// grids (inside the range both cover) when they differ.
pub fn zip(a: &Waveform, b: &Waveform, f: impl Fn(C, C) -> C, real: bool) -> Result<Waveform> {
    let (a2, b2);
    let (a, b) = if a.same_grid(b) {
        (a, b)
    } else {
        let mut xs: Vec<f64> = a.x.iter().chain(b.x.iter()).copied().collect();
        xs.sort_by(|p, q| p.total_cmp(q));
        xs.dedup();
        xs.retain(|&x| a.value_at(x).is_some() && b.value_at(x).is_some());
        if xs.is_empty() {
            bail!("the two waveforms share no x range");
        }
        a2 = a.sample_on(&xs);
        b2 = b.sample_on(&xs);
        (&a2, &b2)
    };
    let y: Vec<C> = (0..a.len()).map(|i| f(a.y.at(i), b.y.at(i))).collect();
    let mut out = a.clone();
    out.name.clear();
    out.y = if real && !a.y.is_complex() && !b.y.is_complex() {
        Samples::Real(y.iter().map(|c| c.re).collect())
    } else {
        Samples::Complex(y)
    };
    if let Samples::Complex(v) = &out.y
        && v.iter().all(|c| c.im == 0.0)
        && !a.y.is_complex()
        && !b.y.is_complex()
    {
        out.y = Samples::Real(v.iter().map(|c| c.re).collect());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpolates_and_clips() {
        let w = Waveform::real(vec![0.0, 1.0, 2.0, 4.0], vec![0.0, 2.0, 2.0, 6.0]);
        assert_eq!(w.value_at(0.5).unwrap().re, 1.0);
        assert_eq!(w.value_at(3.0).unwrap().re, 4.0);
        assert!(w.value_at(5.0).is_none());
        let c = w.clip(0.5, 3.0);
        assert_eq!(*c.x, vec![0.5, 1.0, 2.0, 3.0]);
        assert_eq!(c.y.re(), vec![1.0, 2.0, 2.0, 4.0]);
    }

    #[test]
    fn arithmetic_on_different_grids() {
        let a = Waveform::real(vec![0.0, 2.0], vec![0.0, 2.0]);
        let b = Waveform::real(vec![0.0, 1.0, 2.0], vec![1.0, 1.0, 1.0]);
        let s = zip(&a, &b, |p, q| p.add(q), true).unwrap();
        assert_eq!(*s.x, vec![0.0, 1.0, 2.0]);
        assert_eq!(s.y.re(), vec![1.0, 2.0, 3.0]);
    }
}
