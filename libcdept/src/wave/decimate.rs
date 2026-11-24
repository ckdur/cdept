//! Min/max decimation of big traces before they go to the viewer: the viewer parses a
//! request on its UI thread (about 1 s per million points), and a plot cannot show more
//! than a few points per pixel column anyway.

/// A trace with more points than this is decimated (`[wave] max_points` of cdept.toml
/// changes it; zero never decimates).
pub const DEFAULT_MAX_POINTS: usize = 200_000;

/// Suffix of the name of a decimated trace.
pub const SUFFIX: &str = " (decimated)";

/// How many points a decimated trace of a `max_points` limit has at most: half of it.
pub fn target(max_points: usize) -> usize {
    (max_points / 2).max(4)
}

/// The min/max envelope of `(x, y)` in at most `target(max_points)` points, or `None`
/// when there are `max_points` points or fewer, or `max_points` is zero (nothing to do).
/// Points that are not finite are dropped first. The first and last point are kept; the
/// points in between are cut into buckets of equal size and each bucket keeps the point
/// of its smallest and of its largest `y` (with their own `x`), in `x` order, so
/// extrema survive and the x values stay as they were.
pub fn min_max(x: &[f64], y: &[f64], max_points: usize) -> Option<(Vec<f64>, Vec<f64>)> {
    if max_points == 0 || x.len() <= max_points || x.len() != y.len() {
        return None;
    }
    let (x, y): (Vec<f64>, Vec<f64>) = x
        .iter()
        .zip(y)
        .filter(|(a, b)| a.is_finite() && b.is_finite())
        .map(|(a, b)| (*a, *b))
        .unzip();
    let n = x.len();
    let goal = target(max_points);
    if n <= goal {
        return Some((x, y));
    }
    let buckets = ((goal - 2) / 2).max(1);
    let interior = n - 2;
    let mut keep: Vec<usize> = Vec::with_capacity(2 * buckets + 2);
    keep.push(0);
    for b in 0..buckets {
        let lo = 1 + interior * b / buckets;
        let hi = 1 + interior * (b + 1) / buckets;
        if lo >= hi {
            continue;
        }
        let (mut imin, mut imax) = (lo, lo);
        for i in lo..hi {
            if y[i] < y[imin] {
                imin = i;
            }
            if y[i] > y[imax] {
                imax = i;
            }
        }
        if imin == imax {
            keep.push(imin);
        } else {
            keep.push(imin.min(imax));
            keep.push(imin.max(imax));
        }
    }
    keep.push(n - 1);
    Some((
        keep.iter().map(|&i| x[i]).collect(),
        keep.iter().map(|&i| y[i]).collect(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wave(n: usize) -> (Vec<f64>, Vec<f64>) {
        let x: Vec<f64> = (0..n).map(|i| i as f64).collect();
        let y: Vec<f64> = x.iter().map(|t| (t * 0.01).sin()).collect();
        (x, y)
    }

    #[test]
    fn small_inputs_and_zero_are_left_alone() {
        let (x, y) = wave(1000);
        assert!(min_max(&x, &y, 1000).is_none());
        assert!(min_max(&x, &y, 2000).is_none());
        assert!(min_max(&x, &y, 0).is_none());
        // 200 001 points with the default limit are cut, 200 000 are not.
        let (x, y) = wave(DEFAULT_MAX_POINTS);
        assert!(min_max(&x, &y, DEFAULT_MAX_POINTS).is_none());
        let (x, y) = wave(DEFAULT_MAX_POINTS + 1);
        assert!(min_max(&x, &y, DEFAULT_MAX_POINTS).is_some());
    }

    #[test]
    fn at_most_half_the_limit_ends_and_extrema_kept_in_order() {
        let n = 1_000_000;
        let (x, mut y) = wave(n);
        // Spikes that a plain subsampling would miss.
        y[123_457] = 50.0;
        y[765_431] = -70.0;
        let (dx, dy) = min_max(&x, &y, DEFAULT_MAX_POINTS).unwrap();
        assert_eq!(dx.len(), dy.len());
        assert!(dx.len() <= 100_000, "{}", dx.len());
        assert!(dx.len() > 90_000, "{}", dx.len());
        assert_eq!((dx[0], dy[0]), (x[0], y[0]));
        assert_eq!((dx[dx.len() - 1], dy[dy.len() - 1]), (x[n - 1], y[n - 1]));
        assert!(dx.windows(2).all(|w| w[0] < w[1]), "x strictly increasing");
        let max = dy.iter().cloned().fold(f64::MIN, f64::max);
        let min = dy.iter().cloned().fold(f64::MAX, f64::min);
        assert_eq!((max, min), (50.0, -70.0));
        // Every kept point is an original one.
        assert!(dx.iter().zip(&dy).all(|(a, b)| y[*a as usize] == *b));
    }

    #[test]
    fn a_custom_limit_and_non_finite_points() {
        let (x, mut y) = wave(10_000);
        y[5] = f64::NAN;
        let (dx, dy) = min_max(&x, &y, 1000).unwrap();
        assert!(dx.len() <= 500 && dx.len() > 400, "{}", dx.len());
        assert!(dy.iter().all(|v| v.is_finite()));
        assert_eq!(dx[0], 0.0);
        assert_eq!(*dx.last().unwrap(), 9999.0);
    }
}
