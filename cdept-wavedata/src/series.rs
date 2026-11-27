//! A waveform held in memory (a calculator result, a table column): the same queries as
//! a [`crate::Dataset`] signal, answered in Rust, so it works in the stub build too. The
//! arrays are handed over as they are: no file in between.
//!
//! The data may be hostile: x values that are not finite are dropped (with their y),
//! y values that are not finite are kept and break the line where they are. An x that
//! does not increase (a Lissajous figure, a sweep that goes back) is allowed: such a
//! series is returned in its order, thinned by a stride when it is long, and has no
//! value at a cursor.

use crate::{Mode, QueryInfo, Trace};

/// The most points a query of a series whose x does not increase returns.
pub const MAX_UNORDERED: usize = 1 << 20;

#[derive(Debug, Clone, PartialEq)]
pub struct Series {
    pub name: String,
    pub unit: String,
    pub x_unit: String,
    x: Vec<f64>,
    y: Vec<f64>,
    /// x never decreases.
    increasing: bool,
    /// Points left out because x was not finite (or y was missing).
    dropped: usize,
}

impl Series {
    /// A series of `x` and `y`; extra values of the longer one are dropped (counted).
    pub fn new(
        name: impl Into<String>,
        unit: impl Into<String>,
        x_unit: impl Into<String>,
        x: Vec<f64>,
        y: Vec<f64>,
    ) -> Series {
        let n = x.len().min(y.len());
        let mut dropped = x.len().max(y.len()) - n;
        let (mut xs, mut ys) = (x, y);
        xs.truncate(n);
        ys.truncate(n);
        if xs.iter().any(|v| !v.is_finite()) {
            let (mut kx, mut ky) = (Vec::with_capacity(n), Vec::with_capacity(n));
            for (a, b) in xs.iter().zip(&ys) {
                if a.is_finite() {
                    kx.push(*a);
                    ky.push(*b);
                } else {
                    dropped += 1;
                }
            }
            xs = kx;
            ys = ky;
        }
        let increasing = xs.windows(2).all(|w| w[1] >= w[0]);
        Series {
            name: name.into(),
            unit: unit.into(),
            x_unit: x_unit.into(),
            x: xs,
            y: ys,
            increasing,
            dropped,
        }
    }

    pub fn len(&self) -> usize {
        self.x.len()
    }

    pub fn is_empty(&self) -> bool {
        self.x.is_empty()
    }

    /// x never decreases (queries and cursor values work by x).
    pub fn increasing(&self) -> bool {
        self.increasing
    }

    /// Points dropped because their x was not finite.
    pub fn dropped(&self) -> usize {
        self.dropped
    }

    /// Finite y values.
    pub fn finite(&self) -> usize {
        self.y.iter().filter(|v| v.is_finite()).count()
    }

    /// The smallest and largest x (`None` when empty).
    pub fn x_range(&self) -> Option<(f64, f64)> {
        if self.x.is_empty() {
            return None;
        }
        if self.increasing {
            return Some((self.x[0], self.x[self.x.len() - 1]));
        }
        let lo = self.x.iter().copied().fold(f64::INFINITY, f64::min);
        let hi = self.x.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        Some((lo, hi))
    }

    /// The indices of the samples with `x_min <= x <= x_max`, plus one on each side.
    fn span(&self, x_min: f64, x_max: f64) -> Option<(usize, usize)> {
        if self.x.is_empty() || x_min.partial_cmp(&x_max).is_none_or(|o| o.is_gt()) {
            return None;
        }
        let first = self.x.partition_point(|v| *v < x_min).saturating_sub(1);
        let last = self
            .x
            .partition_point(|v| *v <= x_max)
            .min(self.x.len() - 1);
        (first <= last).then_some((first, last))
    }

    /// The samples between `x_min` and `x_max` reduced to about `pixels` columns, as
    /// [`crate::Dataset::query`] does: each bucket gives its first, minimum, maximum and
    /// last sample in x order, so a one-sample spike is kept.
    pub fn query(&self, x_min: f64, x_max: f64, pixels: u32, out: &mut Trace) -> QueryInfo {
        let started = std::time::Instant::now();
        out.x.clear();
        out.y.clear();
        let pixels = pixels.clamp(1, 65536) as usize;
        let mut info = QueryInfo {
            level: -1,
            bucket: 1,
            ..Default::default()
        };
        if !self.increasing {
            // In its order, all of it (thinned when long): x ranges mean nothing here.
            let stride = self.x.len().div_ceil(MAX_UNORDERED).max(1);
            for i in (0..self.x.len()).step_by(stride) {
                out.x.push(self.x[i]);
                out.y.push(self.y[i]);
            }
            info.source_points = self.x.len() as u64;
            info.last_index = self.x.len().saturating_sub(1) as u64;
            info.bucket = stride as u64;
            info.mode = if stride > 1 {
                Mode::MinMax
            } else {
                Mode::Direct
            };
            info.ms = started.elapsed().as_secs_f64() * 1e3;
            return info;
        }
        let Some((first, last)) = self.span(x_min, x_max) else {
            return info;
        };
        let n = last - first + 1;
        info.first_index = first as u64;
        info.last_index = last as u64;
        info.source_points = n as u64;
        if n <= 4 * pixels {
            out.x.extend_from_slice(&self.x[first..=last]);
            out.y.extend_from_slice(&self.y[first..=last]);
            info.mode = Mode::Direct;
        } else {
            let bucket = n.div_ceil(pixels);
            info.bucket = bucket as u64;
            info.mode = Mode::MinMax;
            let mut i = first;
            while i <= last {
                let end = (i + bucket - 1).min(last);
                bucket_points(&self.x[i..=end], &self.y[i..=end], out);
                i = end + 1;
            }
        }
        info.ms = started.elapsed().as_secs_f64() * 1e3;
        info
    }

    /// The value at `x`, linear between the samples around it (the end values outside);
    /// `None` when the series is empty or its x does not increase.
    pub fn value_at(&self, x: f64) -> Option<f64> {
        if !self.increasing || self.x.is_empty() || !x.is_finite() {
            return None;
        }
        let i = self.x.partition_point(|v| *v <= x);
        if i == 0 {
            return Some(self.y[0]);
        }
        if i >= self.x.len() {
            return Some(self.y[self.x.len() - 1]);
        }
        let (x0, x1, y0, y1) = (self.x[i - 1], self.x[i], self.y[i - 1], self.y[i]);
        if x1 == x0 {
            return Some(y1);
        }
        Some(y0 + (y1 - y0) * (x - x0) / (x1 - x0))
    }

    /// The exact smallest and largest finite y with `x_min <= x <= x_max`, and how many
    /// samples were in the range.
    pub fn exact_minmax(&self, x_min: f64, x_max: f64) -> Option<(f64, f64, u64)> {
        let (mut lo, mut hi, mut count) = (f64::INFINITY, f64::NEG_INFINITY, 0u64);
        for (x, y) in self.x.iter().zip(&self.y) {
            if *x >= x_min && *x <= x_max {
                count += 1;
                if y.is_finite() {
                    lo = lo.min(*y);
                    hi = hi.max(*y);
                }
            }
        }
        (count > 0 && lo <= hi).then_some((lo, hi, count))
    }

    pub fn x(&self) -> &[f64] {
        &self.x
    }

    pub fn y(&self) -> &[f64] {
        &self.y
    }
}

/// First, minimum, maximum, last of a bucket, in x order; not-finite values are left out
/// of the extremes, and a bucket with nothing finite gives one NaN (a break).
fn bucket_points(x: &[f64], y: &[f64], out: &mut Trace) {
    let (mut lo, mut hi) = (usize::MAX, usize::MAX);
    for (i, v) in y.iter().enumerate() {
        if !v.is_finite() {
            continue;
        }
        if lo == usize::MAX || *v < y[lo] {
            lo = i;
        }
        if hi == usize::MAX || *v > y[hi] {
            hi = i;
        }
    }
    if lo == usize::MAX {
        out.x.push(x[0]);
        out.y.push(f64::NAN);
        return;
    }
    let mut idx = [0, lo, hi, y.len() - 1];
    idx.sort_unstable();
    let mut prev = usize::MAX;
    for i in idx {
        if i != prev {
            out.x.push(x[i]);
            out.y.push(y[i]);
            prev = i;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostile_arrays() {
        let s = Series::new("a", "V", "s", vec![], vec![]);
        assert!(s.is_empty() && s.x_range().is_none() && s.value_at(1.0).is_none());
        let mut t = Trace::default();
        assert_eq!(s.query(0.0, 1.0, 100, &mut t).source_points, 0);
        assert!(t.x.is_empty());

        let s = Series::new(
            "b",
            "V",
            "s",
            vec![0.0, f64::NAN, 2.0, f64::INFINITY, 4.0, 5.0],
            vec![1.0, 2.0, f64::NAN, 4.0, f64::INFINITY],
        );
        // 5 pairs, x NaN and Inf dropped, one y without x
        assert_eq!((s.len(), s.dropped()), (3, 3));
        assert_eq!(s.x(), &[0.0, 2.0, 4.0]);
        assert!(s.y()[1].is_nan() && s.y()[2].is_infinite());
        assert_eq!(s.exact_minmax(0.0, 10.0), Some((1.0, 1.0, 3)));
        assert_eq!(s.finite(), 1);
    }

    #[test]
    fn a_spike_is_kept_when_decimated() {
        let n = 1_000_000;
        let x: Vec<f64> = (0..n).map(|i| i as f64).collect();
        let mut y = vec![0.0; n];
        y[777_777] = 9.0;
        y[123] = -3.0;
        let s = Series::new("s", "V", "s", x, y);
        let mut t = Trace::default();
        let info = s.query(0.0, n as f64, 500, &mut t);
        assert_eq!(info.mode, Mode::MinMax);
        assert!(t.x.len() <= 4 * 500 + 8, "{}", t.x.len());
        let max = t.y.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let min = t.y.iter().copied().fold(f64::INFINITY, f64::min);
        assert_eq!((min, max), (-3.0, 9.0));
        assert!(t.x.windows(2).all(|w| w[1] >= w[0]));
        // zoomed in: every sample
        let info = s.query(777_770.0, 777_790.0, 500, &mut t);
        assert_eq!(info.mode, Mode::Direct);
        assert!(t.y.contains(&9.0));
        assert_eq!(s.value_at(777_777.5), Some(4.5));
        assert_eq!(s.value_at(-5.0), Some(0.0));
    }

    #[test]
    fn x_that_goes_back() {
        let s = Series::new(
            "l",
            "V",
            "V",
            vec![0.0, 1.0, 0.5, 2.0],
            vec![1.0, 2.0, 3.0, 4.0],
        );
        assert!(!s.increasing());
        assert_eq!(s.x_range(), Some((0.0, 2.0)));
        assert_eq!(s.value_at(0.7), None);
        let mut t = Trace::default();
        s.query(100.0, 200.0, 10, &mut t);
        assert_eq!(t.x, vec![0.0, 1.0, 0.5, 2.0]);
    }

    #[test]
    fn a_bucket_of_nan_breaks_the_line() {
        let x: Vec<f64> = (0..100).map(|i| i as f64).collect();
        let y: Vec<f64> = (0..100)
            .map(|i| if (40..60).contains(&i) { f64::NAN } else { 1.0 })
            .collect();
        let s = Series::new("n", "", "", x, y);
        let mut t = Trace::default();
        s.query(0.0, 99.0, 5, &mut t);
        assert!(t.y.iter().any(|v| v.is_nan()));
    }
}
