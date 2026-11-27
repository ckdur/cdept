//! Numbers with SI prefixes for axes and readouts (`1.25 µs`, `-3.3 mV`).

/// The SI prefix for a magnitude: (exponent of 1000, prefix).
fn prefix_of(magnitude: f64) -> (i32, &'static str) {
    const PREFIXES: [(i32, &str); 11] = [
        (-6, "a"),
        (-5, "f"),
        (-4, "p"),
        (-3, "n"),
        (-2, "µ"),
        (-1, "m"),
        (0, ""),
        (1, "k"),
        (2, "M"),
        (3, "G"),
        (4, "T"),
    ];
    if !(magnitude.is_finite() && magnitude > 0.0) {
        return (0, "");
    }
    let e = (magnitude.log10() / 3.0).floor() as i32;
    let e = e.clamp(-6, 4);
    PREFIXES
        .iter()
        .find(|(x, _)| *x == e)
        .copied()
        .unwrap_or((0, ""))
}

/// `value` with an SI prefix and `digits` significant digits, then the unit.
pub fn si(value: f64, unit: &str, digits: usize) -> String {
    if value.is_nan() {
        return "NaN".into();
    }
    if value.is_infinite() {
        return if value > 0.0 {
            "inf".into()
        } else {
            "-inf".into()
        };
    }
    if value == 0.0 {
        return format!("0 {unit}").trim_end().to_string();
    }
    let (e, p) = prefix_of(value.abs());
    let scaled = value / 1000f64.powi(e);
    let int_digits = (scaled.abs().log10().floor() as i32 + 1).max(1) as usize;
    let decimals = digits.saturating_sub(int_digits);
    let text = format!("{scaled:.decimals$}");
    // 1.500 -> 1.5, 2.000 -> 2
    let text = if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        text
    };
    format!("{text} {p}{unit}").trim_end().to_string()
}

/// An axis tick: the prefix is chosen by the span of the axis, so all ticks share it
/// and show the digits the step needs.
pub fn tick(value: f64, span: f64, step: f64, unit: &str) -> String {
    let (e, p) = prefix_of(span.abs().max(value.abs()).max(f64::MIN_POSITIVE));
    let scale = 1000f64.powi(e);
    let step = (step / scale).abs();
    let decimals = if step > 0.0 && step.is_finite() {
        (-(step.log10().floor()) as i32).clamp(0, 9) as usize
    } else {
        3
    };
    let v = value / scale;
    let v = if v.abs() < step * 1e-6 { 0.0 } else { v };
    format!(
        "{v:.decimals$}{}",
        if p.is_empty() && unit.is_empty() {
            String::new()
        } else {
            format!(" {p}{unit}")
        }
    )
}

/// Grid marks on 1-2-5 steps, about `divisions` major ones across the range, with
/// minor ones between (egui_plot's own spacer only knows powers of ten, which leaves
/// a 35 µs wide view with one labelled line).
pub fn grid(bounds: (f64, f64), divisions: f64) -> Vec<egui_plot::GridMark> {
    let (lo, hi) = bounds;
    let span = hi - lo;
    if !(span.is_finite() && span > 0.0) {
        return Vec::new();
    }
    let raw = span / divisions.max(1.0);
    let mag = 10f64.powi(raw.log10().floor() as i32);
    let mantissa = raw / mag;
    let (major, ratio) = if mantissa <= 1.0 {
        (mag, 5.0)
    } else if mantissa <= 2.0 {
        (2.0 * mag, 4.0)
    } else if mantissa <= 5.0 {
        (5.0 * mag, 5.0)
    } else {
        (10.0 * mag, 5.0)
    };
    let minor = major / ratio;
    let first = (lo / minor - 1e-9).ceil();
    let last = (hi / minor + 1e-9).floor();
    if !(first.is_finite() && last.is_finite()) || last - first > 400.0 {
        return Vec::new();
    }
    let mut marks = Vec::new();
    let mut k = first;
    while k <= last {
        let value = k * minor;
        let on_major = (k / ratio - (k / ratio).round()).abs() < 1e-6;
        marks.push(egui_plot::GridMark {
            value: if value.abs() < minor * 1e-9 {
                0.0
            } else {
                value
            },
            step_size: if on_major { major } else { minor },
        });
        k += 1.0;
    }
    marks
}

/// Grid marks of a logarithmic axis whose plot coordinate is log10(x): a labelled
/// mark per decade, minor ones at 2..9 (every few decades when the range is wide).
/// `per_label` is the plot width a label needs (egui_plot hides labels closer than
/// about 60 points on x): decades are skipped so that each mark keeps its label.
pub fn log_grid(bounds: (f64, f64), per_label: f64) -> Vec<egui_plot::GridMark> {
    let (lo, hi) = bounds;
    if !(lo.is_finite() && hi.is_finite() && hi > lo) {
        return Vec::new();
    }
    let decades = hi - lo;
    if decades < 1.0 {
        // less than a decade: a linear grid in the plot coordinate
        return grid(bounds, 5.0);
    }
    let fit = if per_label.is_finite() && per_label > 0.0 {
        per_label.ceil()
    } else {
        1.0
    };
    let step = (decades / 10.0).ceil().max(fit).max(1.0);
    let mut marks = Vec::new();
    let mut d = (lo / step).floor() * step;
    while d <= hi + 1e-9 && marks.len() < 400 {
        if d >= lo - 1e-9 {
            marks.push(egui_plot::GridMark {
                value: d,
                step_size: step,
            });
        }
        if step == 1.0 && decades < 6.0 {
            for k in 2..10 {
                let v = d + (k as f64).log10();
                if v >= lo && v <= hi {
                    marks.push(egui_plot::GridMark {
                        value: v,
                        step_size: 0.1,
                    });
                }
            }
        }
        d += step;
    }
    marks
}

/// The label of the x axis: `Time (s)`, `Frequency (Hz)` or `name (unit)` (as
/// UberWave's viewer words it).
pub fn x_label(name: &str, unit: &str) -> String {
    match (name, unit) {
        ("time", "s" | "") => "Time (s)".into(),
        ("freq" | "frequency", _) => "Frequency (Hz)".into(),
        (n, "") => n.into(),
        (n, u) => format!("{n} ({u})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn si_prefixes() {
        assert_eq!(si(1.25e-6, "s", 4), "1.25 µs");
        assert_eq!(si(-3.3e-3, "V", 3), "-3.3 mV");
        assert_eq!(si(0.0, "V", 3), "0 V");
        assert_eq!(si(1500.0, "Hz", 4), "1.5 kHz");
        assert_eq!(si(2.0, "", 4), "2");
        assert_eq!(si(f64::NAN, "V", 3), "NaN");
        assert_eq!(si(123.456, "A", 4), "123.5 A");
    }

    #[test]
    fn grid_on_125_steps() {
        let marks = grid((25e-6, 62e-6), 8.0);
        let major: Vec<f64> = marks
            .iter()
            .filter(|m| (m.step_size / 5e-6 - 1.0).abs() < 1e-9)
            .map(|m| (m.value * 1e7).round() / 10.0)
            .collect();
        assert_eq!(major, [25.0, 30.0, 35.0, 40.0, 45.0, 50.0, 55.0, 60.0]);
        assert!(
            marks
                .iter()
                .any(|m| (m.step_size / 1e-6 - 1.0).abs() < 1e-9)
        );
        assert!(grid((1.0, 1.0), 8.0).is_empty());
        assert!(grid((0.0, f64::INFINITY), 8.0).is_empty());
    }

    #[test]
    fn log_grid_keeps_its_labels() {
        // ten decades: one mark a decade when there is room, every second when not
        let decades = |m: &[egui_plot::GridMark]| {
            m.iter()
                .filter(|g| (g.value - g.value.round()).abs() < 1e-9)
                .count()
        };
        assert_eq!(decades(&log_grid((0.0, 10.0), 0.5)), 11);
        let tight = log_grid((0.0, 10.0), 1.6);
        assert_eq!(decades(&tight), 6);
        assert!(tight.iter().all(|g| g.step_size == 2.0));
        // three decades: the minors 2..9 too
        assert!(log_grid((0.0, 3.0), 0.2).len() > 20);
        assert!(log_grid((f64::NAN, 1.0), 1.0).is_empty());
    }

    #[test]
    fn ticks_share_a_prefix() {
        assert_eq!(tick(2e-6, 10e-6, 1e-6, "s"), "2 µs");
        assert_eq!(tick(2.5e-6, 10e-6, 0.5e-6, "s"), "2.5 µs");
        assert_eq!(tick(0.0, 1.0, 0.2, "V"), "0.0 V");
        assert_eq!(x_label("time", "s"), "Time (s)");
        assert_eq!(x_label("v-sweep", "V"), "v-sweep (V)");
    }
}
