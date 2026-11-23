//! Measurements against specs: the pass/fail table.

use std::collections::BTreeMap;

use super::setup::Spec;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Pass,
    Fail,
    /// The measurement did not come out (the `.meas` failed or is not there).
    Missing,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub name: String,
    pub value: Option<f64>,
    pub verdict: Verdict,
}

/// One row per spec, in order. Names compare in lower case, as ngspice prints them.
pub fn check(specs: &[Spec], measures: &BTreeMap<String, f64>) -> Vec<Row> {
    specs
        .iter()
        .map(|s| {
            let value = measures.get(&s.name.to_lowercase()).copied();
            let verdict = match value {
                None => Verdict::Missing,
                Some(v) if s.min.is_some_and(|m| v < m) || s.max.is_some_and(|m| v > m) => Verdict::Fail,
                Some(_) => Verdict::Pass,
            };
            Row { name: s.name.clone(), value, verdict }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verdicts() {
        let specs = vec![
            Spec { name: "Gain".into(), min: Some(40.0), max: None },
            Spec { name: "delay".into(), min: None, max: Some(2e-10) },
            Spec { name: "pm".into(), min: Some(60.0), max: None },
            Spec { name: "ghost".into(), min: Some(0.0), max: None },
        ];
        let m = BTreeMap::from([("gain".to_string(), 45.0), ("delay".to_string(), 3e-10), ("pm".to_string(), 60.0)]);
        let v: Vec<Verdict> = check(&specs, &m).iter().map(|r| r.verdict).collect();
        assert_eq!(v, [Verdict::Pass, Verdict::Fail, Verdict::Pass, Verdict::Missing]);
    }
}
