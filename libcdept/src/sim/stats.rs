//! Statistics of a set of runs (a Monte Carlo or a sweep): percentiles, histograms and
//! yield against the specs. Works on stored [`RunRecord`]s (see [`super::results`]).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::results::{Overall, RunRecord, Stats};
use super::spec::Verdict;

/// Equal-width bins between the smallest and the largest value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Histogram {
    pub min: f64,
    pub max: f64,
    /// The number of values in each bin; a value equal to `max` is in the last one.
    pub counts: Vec<usize>,
}

impl Histogram {
    pub fn of(values: &[f64], bins: usize) -> Option<Histogram> {
        let bins = bins.max(1);
        if values.is_empty() {
            return None;
        }
        let min = values.iter().copied().fold(f64::INFINITY, f64::min);
        let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let mut counts = vec![0; bins];
        for v in values {
            let at = if max > min {
                (((v - min) / (max - min)) * bins as f64) as usize
            } else {
                0
            };
            counts[at.min(bins - 1)] += 1;
        }
        Some(Histogram { min, max, counts })
    }

    /// The lower and upper edge of bin `i`.
    pub fn edges(&self, i: usize) -> (f64, f64) {
        let w = (self.max - self.min) / self.counts.len() as f64;
        (self.min + w * i as f64, self.min + w * (i + 1) as f64)
    }
}

/// The value below which `p` percent (0..=100) of `sorted` lie, interpolating linearly
/// between neighbours. `sorted` is ascending and not empty.
pub fn percentile(sorted: &[f64], p: f64) -> f64 {
    let at = (p.clamp(0.0, 100.0) / 100.0) * (sorted.len() - 1) as f64;
    let (lo, hi) = (at.floor() as usize, at.ceil() as usize);
    sorted[lo] + (sorted[hi] - sorted[lo]) * (at - lo as f64)
}

/// The standard normal cumulative distribution.
pub fn normal_cdf(z: f64) -> f64 {
    // Abramowitz and Stegun 7.1.26 (erf, error below 1.5e-7).
    let x = z.abs() / std::f64::consts::SQRT_2;
    let t = 1.0 / (1.0 + 0.327_591_1 * x);
    let poly = t
        * (0.254_829_592
            + t * (-0.284_496_736
                + t * (1.421_413_741 + t * (-1.453_152_027 + t * 1.061_405_429))));
    let erf = 1.0 - poly * (-x * x).exp();
    0.5 * (1.0 + if z < 0.0 { -erf } else { erf })
}

/// One measurement over the runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Detail {
    pub stats: Stats,
    pub p5: f64,
    pub p50: f64,
    pub p95: f64,
    pub histogram: Histogram,
}

/// How often a spec held.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpecYield {
    pub name: String,
    pub min: Option<f64>,
    pub max: Option<f64>,
    /// Runs that met the spec, of all runs (a failed or missing one does not).
    pub pass: usize,
    pub total: usize,
    /// The yield if the measurement were normal with the sample's mean and sigma;
    /// None without at least two values.
    pub gaussian: Option<f64>,
    /// Whether mean - k sigma and mean + k sigma are both inside the spec.
    pub within_k_sigma: Option<bool>,
}

impl SpecYield {
    pub fn fraction(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.pass as f64 / self.total as f64
        }
    }
}

/// The statistics report of a set of runs.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub runs: usize,
    /// Runs that met every spec (none counts: no specs, then all that did not error).
    pub pass: usize,
    pub k: f64,
    pub measures: BTreeMap<String, Detail>,
    pub specs: Vec<SpecYield>,
}

impl Report {
    pub fn fraction(&self) -> f64 {
        if self.runs == 0 {
            0.0
        } else {
            self.pass as f64 / self.runs as f64
        }
    }
}

/// Analyse `records` (one per point; use [`super::results::latest_per_point`] for a
/// store): `bins` histogram bins, `k` sigmas for the yield estimate.
pub fn analyze(records: &[RunRecord], bins: usize, k: f64) -> Report {
    let mut report = Report {
        runs: records.len(),
        k,
        ..Default::default()
    };
    report.pass = records
        .iter()
        .filter(|r| r.overall() == Overall::Pass)
        .count();
    let mut values: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
    for r in records {
        for (name, v) in &r.measures {
            values.entry(name).or_default().push(*v);
        }
    }
    for (name, mut v) in values {
        let Some(stats) = Stats::of(&v) else { continue };
        let Some(histogram) = Histogram::of(&v, bins) else {
            continue;
        };
        v.sort_by(|a, b| a.total_cmp(b));
        report.measures.insert(
            name.to_string(),
            Detail {
                p5: percentile(&v, 5.0),
                p50: percentile(&v, 50.0),
                p95: percentile(&v, 95.0),
                stats,
                histogram,
            },
        );
    }
    // Specs in the order they appear in the records.
    let mut order: Vec<(String, Option<f64>, Option<f64>)> = vec![];
    for r in records {
        for s in &r.specs {
            if !order.iter().any(|(n, ..)| *n == s.name) {
                order.push((s.name.clone(), s.min, s.max));
            }
        }
    }
    for (name, min, max) in order {
        let pass = records
            .iter()
            .filter(|r| {
                r.specs
                    .iter()
                    .any(|s| s.name == name && s.verdict == Verdict::Pass)
            })
            .count();
        let detail = report.measures.get(&name).filter(|d| d.stats.count > 1);
        let gaussian = detail.map(|d| {
            let (mean, std) = (d.stats.mean, d.stats.std);
            if std <= 0.0 {
                let inside = min.is_none_or(|m| mean >= m) && max.is_none_or(|m| mean <= m);
                return if inside { 1.0 } else { 0.0 };
            }
            let upper = max.map_or(1.0, |m| normal_cdf((m - mean) / std));
            let lower = min.map_or(0.0, |m| normal_cdf((m - mean) / std));
            (upper - lower).clamp(0.0, 1.0)
        });
        let within = detail.map(|d| {
            min.is_none_or(|m| d.stats.mean - k * d.stats.std >= m)
                && max.is_none_or(|m| d.stats.mean + k * d.stats.std <= m)
        });
        report.specs.push(SpecYield {
            name,
            min,
            max,
            pass,
            total: records.len(),
            gaussian,
            within_k_sigma: within,
        });
    }
    report
}

fn num(v: f64) -> String {
    format!("{v:.4e}")
}

fn bound(min: Option<f64>, max: Option<f64>) -> String {
    match (min, max) {
        (Some(a), Some(b)) => format!("{} .. {}", num(a), num(b)),
        (Some(a), None) => format!(">= {}", num(a)),
        (None, Some(b)) => format!("<= {}", num(b)),
        (None, None) => "-".into(),
    }
}

/// A text bar of `n` in a scale where `full` fills `width` cells.
pub fn bar(n: usize, full: usize, width: usize) -> String {
    if full == 0 {
        String::new()
    } else {
        "#".repeat(
            (n * width)
                .div_ceil(full)
                .min(width)
                .max(usize::from(n > 0)),
        )
    }
}

/// The percentile table, a text histogram per measurement and the yield table.
pub fn format_report(report: &Report) -> String {
    let mut out = String::new();
    if !report.measures.is_empty() {
        out.push_str(&format!(
            "{:<16} {:>5} {:>12} {:>12} {:>12} {:>12} {:>12} {:>12} {:>12}\n",
            "measure", "n", "min", "p5", "p50", "p95", "max", "mean", "std"
        ));
        for (k, d) in &report.measures {
            out.push_str(&format!(
                "{k:<16} {:>5} {:>12} {:>12} {:>12} {:>12} {:>12} {:>12} {:>12}\n",
                d.stats.count,
                num(d.stats.min),
                num(d.p5),
                num(d.p50),
                num(d.p95),
                num(d.stats.max),
                num(d.stats.mean),
                num(d.stats.std)
            ));
        }
        for (k, d) in &report.measures {
            out.push_str(&format!("\n{k}\n"));
            let full = d.histogram.counts.iter().copied().max().unwrap_or(0);
            for (i, n) in d.histogram.counts.iter().enumerate() {
                let (lo, hi) = d.histogram.edges(i);
                out.push_str(&format!(
                    "  {:>12} .. {:>12} {:>5}  {}\n",
                    num(lo),
                    num(hi),
                    n,
                    bar(*n, full, 40)
                ));
            }
        }
    }
    if !report.specs.is_empty() || report.runs > 0 {
        out.push_str(&format!(
            "\n{:<16} {:<28} {:>9} {:>8} {:>10} {:>12}\n",
            "yield",
            "spec",
            "pass",
            "yield",
            "gaussian",
            format!("mean+-{}s", fmt_k(report.k))
        ));
        for s in &report.specs {
            out.push_str(&format!(
                "{:<16} {:<28} {:>9} {:>7.1}% {:>10} {:>12}\n",
                s.name,
                bound(s.min, s.max),
                format!("{}/{}", s.pass, s.total),
                s.fraction() * 100.0,
                s.gaussian
                    .map_or("-".into(), |g| format!("{:.2}%", g * 100.0)),
                match s.within_k_sigma {
                    Some(true) => "inside",
                    Some(false) => "outside",
                    None => "-",
                }
            ));
        }
        out.push_str(&format!(
            "{:<16} {:<28} {:>9} {:>7.1}%\n",
            "all specs",
            "",
            format!("{}/{}", report.pass, report.runs),
            report.fraction() * 100.0
        ));
    }
    out
}

fn fmt_k(k: f64) -> String {
    if k.fract() == 0.0 {
        format!("{}", k as i64)
    } else {
        format!("{k}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn record(i: usize, tpd: f64, max: f64) -> RunRecord {
        let verdict = if tpd <= max { "pass" } else { "fail" };
        serde_json::from_value(serde_json::json!({
            "schema": 1, "library": "l", "cell": "c", "view": "v", "setup_hash": "", "deck_hash": "",
            "simulator": "ngspice",
            "point": { "name": format!("mc_r{i:03}"), "corner": "mc", "temperature": null, "variables": {}, "seed": i, "run": i },
            "status": "ok", "error": null, "measures": { "tpd": tpd },
            "specs": [{ "name": "tpd", "value": tpd, "min": null, "max": max, "verdict": verdict }],
            "started": "", "duration_ms": 0, "dir": "", "deck": null, "raw": null, "log": null
        }))
        .unwrap()
    }

    #[test]
    fn percentiles_interpolate() {
        let v = [1.0, 2.0, 3.0, 4.0, 5.0];
        assert_eq!(percentile(&v, 0.0), 1.0);
        assert_eq!(percentile(&v, 50.0), 3.0);
        assert_eq!(percentile(&v, 100.0), 5.0);
        assert!((percentile(&v, 25.0) - 2.0).abs() < 1e-12);
        assert!((percentile(&v, 5.0) - 1.2).abs() < 1e-12);
        assert_eq!(percentile(&[7.0], 95.0), 7.0);
    }

    #[test]
    fn histogram_counts_every_value_once() {
        let h = Histogram::of(&[0.0, 1.0, 2.0, 3.0, 4.0, 10.0], 5).unwrap();
        assert_eq!(h.counts.iter().sum::<usize>(), 6);
        assert_eq!(h.counts, [2, 2, 1, 0, 1]);
        assert_eq!(h.edges(4), (8.0, 10.0));
        assert_eq!(Histogram::of(&[2.0, 2.0], 3).unwrap().counts, [2, 0, 0]);
        assert!(Histogram::of(&[], 3).is_none());
        assert_eq!(bar(5, 10, 20), "##########");
        assert_eq!(bar(1, 1000, 20), "#");
        assert_eq!(bar(0, 10, 20), "");
    }

    #[test]
    fn the_normal_cdf_is_right() {
        assert!((normal_cdf(0.0) - 0.5).abs() < 1e-7);
        assert!((normal_cdf(1.0) - 0.841_344_7).abs() < 1e-6);
        assert!((normal_cdf(-2.0) - 0.022_750_1).abs() < 1e-6);
        assert!((normal_cdf(3.0) - 0.998_650_1).abs() < 1e-6);
    }

    #[test]
    fn yield_per_spec_and_overall() {
        // tpd 1..=10, spec <= 8: 8 of 10 pass; mean 5.5, sigma 3.03.
        let records: Vec<RunRecord> = (1..=10).map(|i| record(i, i as f64, 8.0)).collect();
        let r = analyze(&records, 5, 3.0);
        assert_eq!((r.runs, r.pass), (10, 8));
        assert_eq!(r.specs.len(), 1);
        let s = &r.specs[0];
        assert_eq!((s.pass, s.total), (8, 10));
        assert!((s.fraction() - 0.8).abs() < 1e-12);
        // z = (8 - 5.5) / 3.0277 = 0.826: about 79.7 %.
        assert!(
            (s.gaussian.unwrap() - 0.7956).abs() < 0.005,
            "{:?}",
            s.gaussian
        );
        assert_eq!(s.within_k_sigma, Some(false));
        let d = &r.measures["tpd"];
        assert_eq!((d.p50, d.stats.count), (5.5, 10));
        let text = format_report(&r);
        assert!(
            text.contains("p95")
                && text.contains("8/10")
                && text.contains("80.0%")
                && text.contains('#'),
            "{text}"
        );
        // Wide margins: inside k sigma.
        let wide = analyze(&records, 5, 0.5);
        assert!(wide.specs[0].within_k_sigma == Some(true));
        // A spec nobody meets, a single run, no records.
        assert_eq!(
            analyze(&[record(0, 9.0, 8.0)], 4, 3.0).specs[0].gaussian,
            None
        );
        assert_eq!(analyze(&[], 4, 3.0).runs, 0);
    }
}
