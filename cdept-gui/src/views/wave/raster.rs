//! Traces drawn into an image on the CPU, one image per pane.
//!
//! egui tessellates every line again every frame: 25 traces of ~5000 points cost 30 ms
//! of CPU per frame in a release build, and a family of hundreds of traces would cost
//! seconds. Here a pane's traces are drawn once per answer of the query thread (on that
//! thread), with anti-aliasing, widths and dash patterns, into an RGBA image that the
//! UI shows as one texture until the next answer. The cost depends on the pixels
//! touched, not on the number of points: the data is already reduced to about four
//! points per pixel column by the engine.
//!
//! Coordinates are pixels of the image, y down. A NaN in a polyline breaks it.

/// A dash pattern, in multiples of the line width (at least 1 px).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[allow(clippy::enum_variant_names)] // dash-dot is the line's name
pub enum Dash {
    #[default]
    Solid,
    Dashed,
    Dotted,
    DashDot,
}

impl Dash {
    pub const ALL: [Dash; 4] = [Dash::Solid, Dash::Dashed, Dash::Dotted, Dash::DashDot];

    pub fn name(self) -> &'static str {
        match self {
            Dash::Solid => "solid",
            Dash::Dashed => "dashed",
            Dash::Dotted => "dotted",
            Dash::DashDot => "dash-dot",
        }
    }

    pub fn parse(text: &str) -> Option<Dash> {
        Some(match text {
            "solid" => Dash::Solid,
            "dash" | "dashed" => Dash::Dashed,
            "dot" | "dotted" => Dash::Dotted,
            "dashdot" | "dash-dot" => Dash::DashDot,
            _ => return None,
        })
    }

    /// On and off lengths in pixels for a line of `width`.
    fn pattern(self, width: f32) -> &'static [f32] {
        let _ = width;
        match self {
            Dash::Solid => &[],
            Dash::Dashed => &[7.0, 4.0],
            Dash::Dotted => &[1.6, 2.6],
            Dash::DashDot => &[7.0, 3.0, 1.6, 3.0],
        }
    }
}

/// How a trace is drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stroke {
    /// Straight (not premultiplied) RGBA.
    pub rgba: [u8; 4],
    pub width: f32,
    pub dash: Dash,
}

/// An RGBA image (premultiplied) with a coverage buffer for the trace being drawn: a
/// pixel touched by two segments of one trace is covered once (the larger coverage), so
/// joints and dense columns do not get darker than the line.
pub struct Canvas {
    pub w: usize,
    pub h: usize,
    /// Column by column (x * h + y): the spans of decimated data are vertical, and
    /// walking down a column of a row-major image touches a cache line per pixel.
    px: Vec<[u8; 4]>,
    cov: Vec<u8>,
    /// Per column, the rows touched by the trace being drawn (min, max), or empty.
    dirty: Vec<(i32, i32)>,
}

impl Canvas {
    pub fn new(w: usize, h: usize) -> Canvas {
        let (w, h) = (w.clamp(1, 16384), h.clamp(1, 16384));
        Canvas {
            w,
            h,
            px: vec![[0; 4]; w * h],
            cov: vec![0; w * h],
            dirty: vec![(i32::MAX, i32::MIN); w],
        }
    }

    /// Put `top` (same size, drawn later) over this image.
    pub fn over(&mut self, top: &Canvas) {
        fn mul(x: u32, y: u32) -> u32 {
            let t = x * y + 128;
            (t + (t >> 8)) >> 8
        }
        for (d, s) in self.px.iter_mut().zip(&top.px) {
            match s[3] {
                0 => {}
                255 => *d = *s,
                a => {
                    let inv = 255 - a as u32;
                    for ch in 0..4 {
                        d[ch] = (s[ch] as u32 + mul(d[ch] as u32, inv)) as u8;
                    }
                }
            }
        }
    }

    /// The pixel at (x, y).
    #[cfg(test)]
    pub fn pixel(&self, x: usize, y: usize) -> [u8; 4] {
        self.px[x * self.h + y]
    }

    /// The image row by row, RGBA premultiplied.
    pub fn to_rgba(&self) -> Vec<u8> {
        let mut out = vec![0u8; self.w * self.h * 4];
        for x in 0..self.w {
            let col = &self.px[x * self.h..(x + 1) * self.h];
            for (y, p) in col.iter().enumerate() {
                if p[3] != 0 {
                    out[(y * self.w + x) * 4..(y * self.w + x) * 4 + 4].copy_from_slice(p);
                }
            }
        }
        out
    }

    /// Draw a polyline (NaN or infinite points break it) with `stroke`.
    ///
    /// Points that fall into one pixel column (decimated data: about four per column)
    /// are drawn as one vertical span from their lowest to their highest, joined to
    /// the neighbouring columns by segments: the same pixels as the segments between
    /// them, at a fraction of the cost.
    pub fn polyline(&mut self, points: &[(f64, f64)], stroke: &Stroke) {
        let width = stroke.width.clamp(0.3, 12.0);
        let pattern: Vec<f32> = stroke
            .dash
            .pattern(width)
            .iter()
            .map(|l| l * width.max(1.0))
            .collect();
        let mut arc = 0.0f32;
        let mut prev: Option<(f64, f64)> = None;
        let mut prev_span: Option<(f64, f64)> = None;
        let mut i = 0;
        while i < points.len() {
            let (x, y) = points[i];
            if !(x.is_finite() && y.is_finite()) {
                prev = None;
                prev_span = None;
                i += 1;
                continue;
            }
            // a run of points in this column
            let col = x.floor();
            let mut j = i + 1;
            let (mut lo, mut hi) = (y, y);
            while j < points.len() {
                let (xj, yj) = points[j];
                if !(xj.is_finite() && yj.is_finite()) || xj.floor() != col {
                    break;
                }
                lo = lo.min(yj);
                hi = hi.max(yj);
                j += 1;
            }
            let entry = (x, y);
            let exit = points[j - 1];
            match prev {
                // neighbouring columns whose spans overlap the step between them are
                // joined already (decimated data: the common case)
                Some(p)
                    if pattern.is_empty()
                        && (entry.0 - p.0).abs() <= 1.5
                        && ((p.1 >= lo && p.1 <= hi)
                            || prev_span.is_some_and(|(a, b)| entry.1 >= a && entry.1 <= b)) =>
                {
                    arc += (entry.1 - p.1).abs() as f32;
                }
                Some(p) => arc = self.join(p, entry, width, &pattern, arc),
                None if j - i == 1 => {
                    // a lone point (its neighbours are breaks) shows as a dot
                    self.segment((x as f32, y as f32), (x as f32, y as f32), width, &[], 0.0);
                }
                None => {}
            }
            if j - i > 1 {
                if col >= -2.0 && col <= self.w as f64 + 2.0 {
                    self.span(x, lo, hi, width);
                }
                arc += (hi - lo) as f32;
            }
            prev = Some(exit);
            prev_span = (j - i > 1).then_some((lo, hi));
            i = j;
        }
        self.composite(stroke.rgba, width);
    }

    /// A segment between two points (clipped), dashes by arc length.
    fn join(&mut self, a: (f64, f64), b: (f64, f64), width: f32, pattern: &[f32], arc: f32) -> f32 {
        // less than a pixel wide and solid: a span (the step between two columns of
        // decimated data is mostly that)
        if pattern.is_empty() && (b.0 - a.0).abs() < 1.0 && (b.1 - a.1).abs() > 2.0 {
            let x = (a.0 + b.0) / 2.0;
            if x >= -2.0 && x <= self.w as f64 + 2.0 {
                self.span(x, a.1.min(b.1), a.1.max(b.1), width);
            }
            return arc + (b.1 - a.1).abs() as f32;
        }
        match self.clip(a.0, a.1, b.0, b.1, width as f64) {
            Some((p, q, skipped)) => self.segment(p, q, width, pattern, arc + skipped),
            None => arc + ((b.0 - a.0).hypot(b.1 - a.1)) as f32,
        }
    }

    /// A vertical span of a column, from `lo` to `hi` (pixels, clipped), `width` wide.
    fn span(&mut self, x: f64, lo: f64, hi: f64, width: f32) {
        let hw = width.max(1.0) / 2.0;
        let gain = width.min(1.0);
        let cx = x as f32;
        let lo = (lo as f32).max(-hw - 2.0);
        let hi = (hi as f32).min(self.h as f32 + hw + 2.0);
        if hi < lo {
            return;
        }
        let (x0, x1) = (
            ((cx - hw - 1.0).floor() as i32).max(0),
            ((cx + hw + 1.0).ceil() as i32).min(self.w as i32 - 1),
        );
        let (y0, y1) = (
            ((lo - hw - 1.0).floor() as i32).max(0),
            ((hi + hw + 1.0).ceil() as i32).min(self.h as i32 - 1),
        );
        for px in x0..=x1 {
            let dx = (px as f32 + 0.5 - cx).abs();
            let side = (hw + 0.5 - dx).clamp(0.0, 1.0) * gain;
            if side <= 0.0 {
                continue;
            }
            let base = px as usize * self.h;
            // the ends (round caps) one by one, the middle as a run
            let (m0, m1) = (
                (lo.ceil() as i32).clamp(y0, y1 + 1),
                ((hi - 1.0).floor() as i32).clamp(y0 - 1, y1),
            );
            let mut touched = false;
            for py in (y0..m0).chain((m1 + 1).max(m0)..=y1) {
                let c = py as f32 + 0.5;
                let dy = if c < lo {
                    lo - c
                } else if c > hi {
                    c - hi
                } else {
                    0.0
                };
                let d = (dx * dx + dy * dy).sqrt();
                let cover = (hw + 0.5 - d).clamp(0.0, 1.0) * gain;
                if cover <= 0.0 {
                    continue;
                }
                let k = base + py as usize;
                let v = (cover * 255.0 + 0.5) as u8;
                if v > self.cov[k] {
                    self.cov[k] = v;
                }
                touched = true;
            }
            if m1 >= m0 {
                let v = (side * 255.0 + 0.5) as u8;
                for c in &mut self.cov[base + m0 as usize..=base + m1 as usize] {
                    *c = (*c).max(v);
                }
                touched = true;
            }
            if touched {
                let dr = &mut self.dirty[px as usize];
                dr.0 = dr.0.min(y0);
                dr.1 = dr.1.max(y1);
            }
        }
    }

    /// The part of a segment inside the canvas (with a margin), in f32, and the length
    /// cut off at its start (for the dash phase). `None` when it misses the canvas.
    #[allow(clippy::type_complexity)]
    fn clip(
        &self,
        x0: f64,
        y0: f64,
        x1: f64,
        y1: f64,
        width: f64,
    ) -> Option<((f32, f32), (f32, f32), f32)> {
        let m = width + 2.0;
        let (xmin, xmax, ymin, ymax) = (-m, self.w as f64 + m, -m, self.h as f64 + m);
        let (dx, dy) = (x1 - x0, y1 - y0);
        let (mut t0, mut t1) = (0.0f64, 1.0f64);
        for (p, q) in [
            (-dx, x0 - xmin),
            (dx, xmax - x0),
            (-dy, y0 - ymin),
            (dy, ymax - y0),
        ] {
            if p == 0.0 {
                if q < 0.0 {
                    return None;
                }
            } else {
                let r = q / p;
                if p < 0.0 {
                    t0 = t0.max(r);
                } else {
                    t1 = t1.min(r);
                }
            }
        }
        if t0 > t1 {
            return None;
        }
        let len = dx.hypot(dy);
        let a = ((x0 + t0 * dx) as f32, (y0 + t0 * dy) as f32);
        let b = ((x0 + t1 * dx) as f32, (y0 + t1 * dy) as f32);
        Some((a, b, (t0 * len) as f32))
    }

    /// Coverage of one segment into `cov` (max), dashes by arc length. Returns the arc
    /// length at its end.
    fn segment(
        &mut self,
        a: (f32, f32),
        b: (f32, f32),
        width: f32,
        pattern: &[f32],
        arc: f32,
    ) -> f32 {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len2 = dx * dx + dy * dy;
        let len = len2.sqrt();
        let hw = width.max(1.0) / 2.0;
        // thinner than a pixel: lighter instead
        let gain = width.min(1.0);
        let period: f32 = pattern.iter().sum();
        let reach = hw + 1.0;
        let x_major = dx.abs() >= dy.abs();
        let (lo, hi) = if x_major {
            (a.0.min(b.0) - reach, a.0.max(b.0) + reach)
        } else {
            (a.1.min(b.1) - reach, a.1.max(b.1) + reach)
        };
        let limit = if x_major { self.w } else { self.h } as i32;
        let (i0, i1) = (
            (lo.floor() as i32).max(0),
            (hi.ceil() as i32).min(limit - 1),
        );
        // the band around the line along the minor axis
        let slope = if x_major {
            if dx != 0.0 { dy / dx } else { 0.0 }
        } else if dy != 0.0 {
            dx / dy
        } else {
            0.0
        };
        let ext = reach * (1.0 + slope * slope).sqrt() + 0.5;
        for i in i0..=i1 {
            let c = i as f32 + 0.5;
            // where the line crosses this column (row), clamped to the segment
            let m = if x_major {
                if dx != 0.0 {
                    a.1 + (c.clamp(a.0.min(b.0), a.0.max(b.0)) - a.0) * slope
                } else {
                    a.1
                }
            } else if dy != 0.0 {
                a.0 + (c.clamp(a.1.min(b.1), a.1.max(b.1)) - a.1) * slope
            } else {
                a.0
            };
            let (mlo, mhi) = if (x_major && dx == 0.0) || (!x_major && dy == 0.0) {
                // a dot or a segment along the minor axis only
                let (s, e) = if x_major { (a.1, b.1) } else { (a.0, b.0) };
                (s.min(e) - reach, s.max(e) + reach)
            } else {
                (m - ext, m + ext)
            };
            let other = if x_major { self.h } else { self.w } as i32;
            let (j0, j1) = (
                (mlo.floor() as i32).max(0),
                (mhi.ceil() as i32).min(other - 1),
            );
            for j in j0..=j1 {
                let (px, py) = if x_major {
                    (c, j as f32 + 0.5)
                } else {
                    (j as f32 + 0.5, c)
                };
                // distance to the segment (a capsule)
                let t = if len2 > 0.0 {
                    (((px - a.0) * dx + (py - a.1) * dy) / len2).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                if period > 0.0 && !dash_on(pattern, period, arc + t * len) {
                    continue;
                }
                let (qx, qy) = (a.0 + t * dx - px, a.1 + t * dy - py);
                let d = (qx * qx + qy * qy).sqrt();
                let cover = (hw + 0.5 - d).clamp(0.0, 1.0) * gain;
                if cover <= 0.0 {
                    continue;
                }
                let (x, y) = if x_major { (i, j) } else { (j, i) };
                let k = x as usize * self.h + y as usize;
                let v = (cover * 255.0 + 0.5) as u8;
                if v > self.cov[k] {
                    self.cov[k] = v;
                }
                let dr = &mut self.dirty[x as usize];
                dr.0 = dr.0.min(y);
                dr.1 = dr.1.max(y);
            }
        }
        arc + len
    }

    /// The trace's coverage onto the image in its colour; the coverage is cleared.
    fn composite(&mut self, rgba: [u8; 4], _width: f32) {
        // x * y / 255, rounded, without a division
        fn mul(x: u32, y: u32) -> u32 {
            let t = x * y + 128;
            (t + (t >> 8)) >> 8
        }
        let alpha = rgba[3] as u32;
        let solid = [rgba[0], rgba[1], rgba[2], 255];
        let h = self.h;
        for x in 0..self.w {
            let (y0, y1) = self.dirty[x];
            if y0 > y1 {
                continue;
            }
            self.dirty[x] = (i32::MAX, i32::MIN);
            let (k0, k1) = (x * h + y0 as usize, x * h + y1 as usize);
            let cov = &mut self.cov[k0..=k1];
            let px = &mut self.px[k0..=k1];
            for (c, d) in cov.iter_mut().zip(px.iter_mut()) {
                let cv = *c as u32;
                if cv == 0 {
                    continue;
                }
                *c = 0;
                if cv == 255 && alpha == 255 {
                    *d = solid;
                    continue;
                }
                let a = mul(cv, alpha);
                let inv = 255 - a;
                for ch in 0..3 {
                    d[ch] = (mul(rgba[ch] as u32, a) + mul(d[ch] as u32, inv)) as u8;
                }
                d[3] = (a + mul(d[3] as u32, inv)) as u8;
            }
        }
    }
}

fn dash_on(pattern: &[f32], period: f32, s: f32) -> bool {
    let mut r = s.rem_euclid(period);
    for (i, l) in pattern.iter().enumerate() {
        if r < *l {
            return i % 2 == 0;
        }
        r -= l;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stroke(dash: Dash) -> Stroke {
        Stroke {
            rgba: [255, 0, 0, 255],
            width: 1.5,
            dash,
        }
    }

    fn lit(c: &Canvas) -> usize {
        (0..c.w)
            .flat_map(|x| (0..c.h).map(move |y| (x, y)))
            .filter(|&(x, y)| c.pixel(x, y)[3] > 0)
            .count()
    }

    #[test]
    fn a_line_is_drawn_and_a_dashed_one_has_gaps() {
        let mut c = Canvas::new(200, 50);
        c.polyline(&[(0.0, 25.0), (200.0, 25.0)], &stroke(Dash::Solid));
        let solid = lit(&c);
        // every column has the line
        assert!((0..200).all(|x| (20..30).any(|y| c.pixel(x, y)[3] > 100)));
        let mut d = Canvas::new(200, 50);
        d.polyline(&[(0.0, 25.0), (200.0, 25.0)], &stroke(Dash::Dashed));
        let dashed = lit(&d);
        assert!(
            dashed < solid * 3 / 4 && dashed > solid / 3,
            "{dashed} {solid}"
        );
        let mut e = Canvas::new(200, 50);
        e.polyline(&[(0.0, 25.0), (200.0, 25.0)], &stroke(Dash::Dotted));
        assert!(lit(&e) < dashed);
    }

    #[test]
    fn parts_put_over_each_other_are_the_lines_drawn_in_order() {
        let a = [(0.0, 10.0), (100.0, 40.0)];
        let b = [(0.0, 40.0), (100.0, 10.0)];
        let half = |c: [u8; 4]| Stroke {
            rgba: c,
            width: 2.0,
            dash: Dash::Solid,
        };
        let mut one = Canvas::new(100, 50);
        one.polyline(&a, &half([255, 0, 0, 255]));
        one.polyline(&b, &half([0, 0, 255, 160]));
        let mut first = Canvas::new(100, 50);
        first.polyline(&a, &half([255, 0, 0, 255]));
        let mut second = Canvas::new(100, 50);
        second.polyline(&b, &half([0, 0, 255, 160]));
        first.over(&second);
        let (x, y) = (one.to_rgba(), first.to_rgba());
        assert!(x.iter().zip(&y).all(|(p, q)| p.abs_diff(*q) <= 2));
    }

    #[test]
    fn a_one_sample_spike_is_drawn() {
        let mut c = Canvas::new(100, 100);
        c.polyline(
            &[
                (10.0, 90.0),
                (50.0, 90.0),
                (50.0, 5.0),
                (50.0, 90.0),
                (90.0, 90.0),
            ],
            &stroke(Dash::Solid),
        );
        assert!(c.pixel(50, 6)[3] > 100);
    }

    #[test]
    fn far_away_and_broken_points_are_safe() {
        let mut c = Canvas::new(64, 64);
        c.polyline(
            &[
                (-1e300, 1e300),
                (1e300, -1e300),
                (f64::NAN, 3.0),
                (5.0, f64::INFINITY),
                (1e15, 32.0),
                (-1e15, 32.0),
            ],
            &stroke(Dash::DashDot),
        );
        // the long horizontal line crosses
        assert!(c.pixel(10, 32)[3] > 0);
        let mut e = Canvas::new(0, 0);
        e.polyline(&[], &stroke(Dash::Solid));
        e.polyline(&[(0.5, 0.5)], &stroke(Dash::Solid));
    }

    #[test]
    fn a_dense_column_is_not_darker_than_the_line() {
        let mut c = Canvas::new(10, 100);
        let zig: Vec<(f64, f64)> = (0..400)
            .map(|i| (5.0, if i % 2 == 0 { 10.0 } else { 90.0 }))
            .collect();
        c.polyline(
            &zig,
            &Stroke {
                rgba: [0, 0, 255, 128],
                width: 1.0,
                dash: Dash::Solid,
            },
        );
        let max = c.to_rgba().chunks(4).map(|p| p[3]).max().unwrap();
        assert!(max <= 129, "{max}");
    }
}

#[cfg(test)]
mod bench {
    use super::*;

    /// `cargo test --release -p cdept-gui --lib raster_bench -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn raster_bench() {
        let (w, h) = (2000usize, 1000usize);
        let pts: Vec<(f64, f64)> = (0..w * 4)
            .map(|i| {
                (
                    (i / 4) as f64 + 0.25 * (i % 4) as f64,
                    if (i / 2) % 2 == 0 { 50.0 } else { 950.0 },
                )
            })
            .collect();
        let t = std::time::Instant::now();
        let mut c = Canvas::new(w, h);
        let alloc = t.elapsed();
        for _ in 0..26 {
            c.polyline(
                &pts,
                &Stroke {
                    rgba: [255, 0, 0, 255],
                    width: 1.5,
                    dash: Dash::Solid,
                },
            );
        }
        let lines = t.elapsed();
        let img = c.to_rgba();
        eprintln!(
            "alloc {alloc:?} lines {:?} to_rgba {:?} {}",
            lines - alloc,
            t.elapsed() - lines,
            img.len()
        );
    }
}
