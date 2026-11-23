//! What a testbench run leaves behind, kept: a `result.json` in every run folder and
//! a project-wide SQLite index, `<project>/simulations/results.db`.
//!
//! The JSON ([`RunRecord`]) has everything about one run: the hash of the setup it
//! ran with, the point (corner, temperature, design variables, Monte Carlo seed), the
//! status, the measurements, the verdicts against the specs, durations and paths. The
//! database holds the same, one row per run in `runs` and one per measurement in
//! `measures`, to list, compare and aggregate runs of a testbench without reading
//! folders; [`Store::reindex`] rebuilds it from the `result.json` files. Design:
//! docs/SIMULATION.md section 5.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use eyre::{Result, WrapErr};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use super::deck::RunPoint;
use super::setup::{Setup, Spec};
use super::spec::{Row, Verdict};

/// The record of a run, in its folder.
pub const RESULT_FILE: &str = "result.json";
/// The index, in `<project>/simulations/`.
pub const DB_FILE: &str = "results.db";
const SCHEMA: u32 = 1;

/// The point of a run, as stored.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PointRecord {
    pub name: String,
    pub corner: String,
    pub temperature: Option<f64>,
    pub variables: BTreeMap<String, String>,
    pub seed: Option<u64>,
    pub run: Option<u32>,
}

impl From<&RunPoint> for PointRecord {
    fn from(p: &RunPoint) -> Self {
        PointRecord {
            name: p.name(),
            corner: p.corner.clone(),
            temperature: p.temperature,
            variables: p.variables.clone(),
            seed: p.seed,
            run: p.run,
        }
    }
}

/// A spec and what the run measured against it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpecRecord {
    pub name: String,
    pub value: Option<f64>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub verdict: Verdict,
}

/// How a run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// The simulator ran and left a raw file.
    Ok,
    /// It did not (the error is in the record).
    Error,
}

/// Everything about one run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunRecord {
    pub schema: u32,
    /// The row id in the database (not in the file).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    pub library: String,
    pub cell: String,
    pub view: String,
    /// Hash of the setup the run used ([`hash_text`] of its JSON).
    pub setup_hash: String,
    /// Hash of the deck (setup, models, netlist, point).
    pub deck_hash: String,
    pub simulator: String,
    pub point: PointRecord,
    pub status: Status,
    pub error: Option<String>,
    pub measures: BTreeMap<String, f64>,
    /// The measures that came from the calculator (`[[output]]`), not `.meas`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub calc: Vec<String>,
    /// Outputs that failed, with the message.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub calc_errors: BTreeMap<String, String>,
    pub specs: Vec<SpecRecord>,
    /// RFC 3339, local time.
    pub started: String,
    pub duration_ms: u64,
    pub dir: PathBuf,
    pub deck: Option<PathBuf>,
    pub raw: Option<PathBuf>,
    pub log: Option<PathBuf>,
}

/// What a run came to: an error, a failed or missing spec, or a pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overall {
    Pass,
    Fail,
    Error,
}

impl Overall {
    pub fn word(self) -> &'static str {
        match self {
            Overall::Pass => "pass",
            Overall::Fail => "fail",
            Overall::Error => "error",
        }
    }
}

impl RunRecord {
    pub fn overall(&self) -> Overall {
        if self.status == Status::Error {
            Overall::Error
        } else if self.specs.iter().any(|s| s.verdict != Verdict::Pass) {
            Overall::Fail
        } else {
            Overall::Pass
        }
    }

    /// Build a record from a run.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        (library, cell, view): (&str, &str, &str),
        setup: &Setup,
        deck: &str,
        point: &RunPoint,
        dir: &Path,
        outcome: &Result<super::Outcome>,
        rows: &[Row],
        started: String,
        duration_ms: u64,
    ) -> RunRecord {
        let specs = rows
            .iter()
            .map(|r| {
                let s: Option<&Spec> = setup.specs.iter().find(|s| s.name == r.name);
                SpecRecord {
                    name: r.name.clone(),
                    value: r.value,
                    min: s.and_then(|s| s.min),
                    max: s.and_then(|s| s.max),
                    verdict: r.verdict,
                }
            })
            .collect();
        let (status, error, measures, deck_f, raw, log) = match outcome {
            Ok(o) => (Status::Ok, None, o.measures.clone(), Some(o.deck.clone()), Some(o.raw.clone()), Some(o.log.clone())),
            Err(e) => (Status::Error, Some(format!("{e:#}")), BTreeMap::new(), None, None, None),
        };
        RunRecord {
            schema: SCHEMA,
            id: None,
            library: library.into(),
            cell: cell.into(),
            view: view.into(),
            setup_hash: hash_text(&serde_json::to_string(setup).unwrap_or_default()),
            deck_hash: hash_text(deck),
            simulator: if setup.simulator.is_empty() { "ngspice".into() } else { setup.simulator.clone() },
            point: point.into(),
            status,
            error,
            measures,
            calc: vec![],
            calc_errors: BTreeMap::new(),
            specs,
            started,
            duration_ms,
            dir: dir.to_path_buf(),
            deck: deck_f,
            raw,
            log,
        }
    }

    /// Write `result.json` into the run's folder.
    pub fn write(&self) -> Result<PathBuf> {
        let file = self.dir.join(RESULT_FILE);
        std::fs::write(&file, serde_json::to_string_pretty(self)?)
            .wrap_err_with(|| format!("cannot write {}", file.display()))?;
        Ok(file)
    }

    pub fn read(file: &Path) -> Result<RunRecord> {
        let text = std::fs::read_to_string(file).wrap_err_with(|| format!("cannot read {}", file.display()))?;
        serde_json::from_str(&text).wrap_err_with(|| format!("{} is not a run record", file.display()))
    }
}

/// A 64-bit FNV-1a hash as 16 hex digits: stable across machines and versions, to tell
/// whether two runs used the same setup, not for security.
pub fn hash_text(text: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// Statistics of one measurement over some runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stats {
    pub count: usize,
    pub min: f64,
    pub max: f64,
    pub mean: f64,
    /// Sample standard deviation (n - 1); 0 for a single value.
    pub std: f64,
}

impl Stats {
    pub fn of(values: &[f64]) -> Option<Stats> {
        if values.is_empty() {
            return None;
        }
        let n = values.len() as f64;
        let mean = values.iter().sum::<f64>() / n;
        let std = if values.len() > 1 {
            (values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1.0)).sqrt()
        } else {
            0.0
        };
        Some(Stats {
            count: values.len(),
            min: values.iter().copied().fold(f64::INFINITY, f64::min),
            max: values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            mean,
            std,
        })
    }
}

/// A set of runs summed up.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Aggregate {
    pub runs: usize,
    pub pass: usize,
    pub fail: usize,
    pub error: usize,
    pub measures: BTreeMap<String, Stats>,
}

/// Pass/fail counts and min/max/mean/std of every measurement over `records`
/// (runs that errored count as errors and have no measurements).
pub fn aggregate(records: &[RunRecord]) -> Aggregate {
    let mut agg = Aggregate { runs: records.len(), ..Default::default() };
    let mut values: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
    for r in records {
        match r.overall() {
            Overall::Pass => agg.pass += 1,
            Overall::Fail => agg.fail += 1,
            Overall::Error => agg.error += 1,
        }
        for (k, v) in &r.measures {
            values.entry(k).or_default().push(*v);
        }
    }
    agg.measures = values.into_iter().filter_map(|(k, v)| Some((k.to_string(), Stats::of(&v)?))).collect();
    agg
}

/// The latest record of every point (by name), in the order of first appearance.
/// `records` are oldest first.
pub fn latest_per_point(records: &[RunRecord]) -> Vec<RunRecord> {
    let mut order: Vec<&str> = vec![];
    let mut last: BTreeMap<&str, &RunRecord> = BTreeMap::new();
    for r in records {
        if last.insert(&r.point.name, r).is_none() {
            order.push(&r.point.name);
        }
    }
    order.iter().map(|n| last[n].clone()).collect()
}

fn num(v: f64) -> String {
    format!("{v:.4e}")
}

/// A table of runs: point, verdict, then one column per measurement.
pub fn format_table(records: &[RunRecord]) -> String {
    let names: Vec<&String> = {
        let mut set = std::collections::BTreeSet::new();
        for r in records {
            set.extend(r.measures.keys());
        }
        set.into_iter().collect()
    };
    let width = records.iter().map(|r| r.point.name.len()).max().unwrap_or(5).max(5);
    let mut out = format!("{:<width$}  {:<6}", "point", "result");
    let is_calc = |n: &str| records.iter().any(|r| r.calc.iter().any(|c| c == n));
    for n in &names {
        let label = if is_calc(n) { format!("{n}*") } else { n.to_string() };
        out.push_str(&format!("  {label:>12}"));
    }
    out.push('\n');
    for r in records {
        out.push_str(&format!("{:<width$}  {:<6}", r.point.name, r.overall().word().to_uppercase()));
        for n in &names {
            match r.measures.get(*n) {
                Some(v) => out.push_str(&format!("  {:>12}", num(*v))),
                None => out.push_str(&format!("  {:>12}", "-")),
            }
        }
        for s in r.specs.iter().filter(|s| s.verdict != Verdict::Pass) {
            out.push_str(&format!("   [{} {}]", s.name, if s.verdict == Verdict::Fail { "FAIL" } else { "MISSING" }));
        }
        if let Some(e) = &r.error {
            out.push_str(&format!("   {}", e.lines().next().unwrap_or("")));
        }
        out.push('\n');
        for (k, e) in &r.calc_errors {
            out.push_str(&format!("  ! output {k}: {e}\n"));
        }
    }
    if names.iter().any(|n| is_calc(n)) {
        out.push_str("* calculator output\n");
    }
    out
}

/// The counts and the statistics of an [`Aggregate`].
pub fn format_aggregate(agg: &Aggregate) -> String {
    let mut out = format!("{} run(s): {} pass, {} fail, {} error\n", agg.runs, agg.pass, agg.fail, agg.error);
    if !agg.measures.is_empty() {
        out.push_str(&format!("{:<16} {:>5} {:>12} {:>12} {:>12} {:>12}\n", "measure", "n", "min", "max", "mean", "std"));
        for (k, s) in &agg.measures {
            out.push_str(&format!(
                "{k:<16} {:>5} {:>12} {:>12} {:>12} {:>12}\n",
                s.count,
                num(s.min),
                num(s.max),
                num(s.mean),
                num(s.std)
            ));
        }
    }
    out
}

/// The results database.
pub struct Store {
    conn: Connection,
}

impl Store {
    /// `<project root>/simulations/results.db`.
    pub fn path(project_root: &Path) -> PathBuf {
        project_root.join("simulations").join(DB_FILE)
    }

    /// Open (and create) the database of a project.
    pub fn open(project_root: &Path) -> Result<Store> {
        let file = Self::path(project_root);
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        Self::open_file(&file)
    }

    pub fn open_file(file: &Path) -> Result<Store> {
        let conn = Connection::open(file).wrap_err_with(|| format!("cannot open {}", file.display()))?;
        Self::init(conn)
    }

    pub fn in_memory() -> Result<Store> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Store> {
        conn.busy_timeout(std::time::Duration::from_secs(10))?;
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS runs (
                 id INTEGER PRIMARY KEY,
                 library TEXT NOT NULL, cell TEXT NOT NULL, view TEXT NOT NULL,
                 name TEXT NOT NULL,
                 setup_hash TEXT NOT NULL, deck_hash TEXT NOT NULL, simulator TEXT NOT NULL,
                 corner TEXT NOT NULL, temperature REAL, variables TEXT NOT NULL,
                 seed INTEGER, run_index INTEGER,
                 status TEXT NOT NULL, verdict TEXT NOT NULL, error TEXT,
                 started TEXT NOT NULL, duration_ms INTEGER NOT NULL,
                 dir TEXT NOT NULL, record TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS runs_tb ON runs (library, cell, view, name);
             CREATE TABLE IF NOT EXISTS measures (
                 run_id INTEGER NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
                 name TEXT NOT NULL, value REAL,
                 spec_min REAL, spec_max REAL, verdict TEXT,
                 PRIMARY KEY (run_id, name)
             );
             CREATE INDEX IF NOT EXISTS measures_name ON measures (name);",
        )?;
        Ok(Store { conn })
    }

    /// Add a run (runs are kept: a re-run is a new row). Returns its id.
    pub fn insert(&mut self, r: &RunRecord) -> Result<i64> {
        let tx = self.conn.transaction()?;
        let mut stored = r.clone();
        stored.id = None;
        tx.execute(
            "INSERT INTO runs (library, cell, view, name, setup_hash, deck_hash, simulator, corner, temperature,
                 variables, seed, run_index, status, verdict, error, started, duration_ms, dir, record)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19)",
            params![
                r.library,
                r.cell,
                r.view,
                r.point.name,
                r.setup_hash,
                r.deck_hash,
                r.simulator,
                r.point.corner,
                r.point.temperature,
                serde_json::to_string(&r.point.variables)?,
                r.point.seed.map(|s| s as i64),
                r.point.run,
                serde_json::to_value(r.status)?.as_str().unwrap_or("ok"),
                r.overall().word(),
                r.error,
                r.started,
                r.duration_ms as i64,
                r.dir.display().to_string(),
                serde_json::to_string(&stored)?,
            ],
        )?;
        let id = tx.last_insert_rowid();
        for (name, value) in &r.measures {
            let spec = r.specs.iter().find(|s| s.name.eq_ignore_ascii_case(name));
            tx.execute(
                "INSERT INTO measures (run_id, name, value, spec_min, spec_max, verdict) VALUES (?1,?2,?3,?4,?5,?6)",
                params![
                    id,
                    name,
                    value,
                    spec.and_then(|s| s.min),
                    spec.and_then(|s| s.max),
                    spec.and_then(|s| serde_json::to_value(s.verdict).ok().and_then(|v| v.as_str().map(String::from)))
                ],
            )?;
        }
        // A spec whose measurement did not come out is a measure row without a value.
        for s in r.specs.iter().filter(|s| s.value.is_none()) {
            tx.execute(
                "INSERT OR IGNORE INTO measures (run_id, name, value, spec_min, spec_max, verdict) VALUES (?1,?2,NULL,?3,?4,'missing')",
                params![id, s.name.to_lowercase(), s.min, s.max],
            )?;
        }
        tx.commit()?;
        Ok(id)
    }

    fn query(&self, sql: &str, args: &[&dyn rusqlite::ToSql]) -> Result<Vec<RunRecord>> {
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map(args, |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))?;
        let mut out = vec![];
        for row in rows {
            let (id, json) = row?;
            let mut r: RunRecord = serde_json::from_str(&json)?;
            r.id = Some(id);
            out.push(r);
        }
        Ok(out)
    }

    /// Every run of a testbench, oldest first.
    pub fn runs(&self, library: &str, cell: &str, view: &str) -> Result<Vec<RunRecord>> {
        self.query(
            "SELECT id, record FROM runs WHERE library=?1 AND cell=?2 AND view=?3 ORDER BY id",
            &[&library, &cell, &view],
        )
    }

    /// The latest run of every point of a testbench, in point order of first run.
    pub fn latest(&self, library: &str, cell: &str, view: &str) -> Result<Vec<RunRecord>> {
        self.query(
            "SELECT r.id, r.record FROM runs r
             JOIN (SELECT name, MAX(id) AS id, MIN(id) AS first FROM runs
                   WHERE library=?1 AND cell=?2 AND view=?3 GROUP BY name) l ON r.id = l.id
             ORDER BY l.first",
            &[&library, &cell, &view],
        )
    }

    /// The runs of a testbench with this setup hash, latest per point.
    pub fn latest_of_setup(&self, library: &str, cell: &str, view: &str, setup_hash: &str) -> Result<Vec<RunRecord>> {
        let all = self.query(
            "SELECT id, record FROM runs WHERE library=?1 AND cell=?2 AND view=?3 AND setup_hash=?4 ORDER BY id",
            &[&library, &cell, &view, &setup_hash],
        )?;
        Ok(latest_per_point(&all))
    }

    /// Pass/fail counts and statistics over the latest run of every point.
    pub fn stats(&self, library: &str, cell: &str, view: &str) -> Result<Aggregate> {
        Ok(aggregate(&self.latest(library, cell, view)?))
    }

    /// One measurement over the latest runs of a testbench: `(point name, value)`.
    pub fn series(&self, library: &str, cell: &str, view: &str, measure: &str) -> Result<Vec<(String, f64)>> {
        Ok(self
            .latest(library, cell, view)?
            .into_iter()
            .filter_map(|r| r.measures.get(&measure.to_lowercase()).map(|v| (r.point.name, *v)))
            .collect())
    }

    /// Forget everything and read the `result.json` files under `<root>/simulations`
    /// again (oldest first). Returns how many runs it found.
    pub fn reindex(&mut self, project_root: &Path) -> Result<usize> {
        let mut found = vec![];
        collect_records(&project_root.join("simulations"), 0, &mut found);
        found.sort_by(|a, b| a.started.cmp(&b.started));
        self.conn.execute_batch("DELETE FROM measures; DELETE FROM runs;")?;
        for r in &found {
            self.insert(r)?;
        }
        Ok(found.len())
    }
}

fn collect_records(dir: &Path, depth: usize, out: &mut Vec<RunRecord>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() && depth < 6 {
            collect_records(&p, depth + 1, out);
        } else if p.file_name().is_some_and(|n| n == RESULT_FILE)
            && let Ok(r) = RunRecord::read(&p)
        {
            out.push(r);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(name: &str, started: &str, tpd: Option<f64>, max: f64) -> RunRecord {
        let measures: BTreeMap<String, f64> = tpd.into_iter().map(|v| ("tpd".to_string(), v)).collect();
        let verdict = match tpd {
            None => Verdict::Missing,
            Some(v) if v > max => Verdict::Fail,
            _ => Verdict::Pass,
        };
        RunRecord {
            schema: SCHEMA,
            id: None,
            library: "cells".into(),
            cell: "tb".into(),
            view: "testbench".into(),
            setup_hash: "h".into(),
            deck_hash: "d".into(),
            simulator: "ngspice".into(),
            point: PointRecord {
                name: name.into(),
                corner: "tt".into(),
                variables: BTreeMap::from([("vdd".into(), "1.2".into())]),
                seed: Some(5),
                ..Default::default()
            },
            status: Status::Ok,
            error: None,
            measures,
            calc: vec![],
            calc_errors: BTreeMap::new(),
            specs: vec![SpecRecord { name: "tpd".into(), value: tpd, min: None, max: Some(max), verdict }],
            started: started.into(),
            duration_ms: 12,
            dir: "/tmp/x".into(),
            deck: None,
            raw: None,
            log: None,
        }
    }

    #[test]
    fn statistics() {
        let s = Stats::of(&[2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0]).unwrap();
        assert_eq!((s.count, s.min, s.max, s.mean), (8, 2.0, 9.0, 5.0));
        assert!((s.std - 2.138_089_935).abs() < 1e-6, "{}", s.std);
        assert_eq!(Stats::of(&[3.0]).unwrap().std, 0.0);
        assert!(Stats::of(&[]).is_none());
    }

    #[test]
    fn aggregates_count_verdicts() {
        let mut err = rec("c", "3", None, 1.0);
        err.status = Status::Error;
        err.error = Some("boom\nmore".into());
        let rs = vec![rec("a", "1", Some(1.0), 2.0), rec("b", "2", Some(3.0), 2.0), err, rec("d", "4", None, 2.0)];
        let agg = aggregate(&rs);
        assert_eq!((agg.runs, agg.pass, agg.fail, agg.error), (4, 1, 2, 1));
        assert_eq!(agg.measures["tpd"].count, 2);
        assert_eq!(agg.measures["tpd"].mean, 2.0);
        let table = format_table(&rs);
        assert!(table.contains("FAIL") && table.contains("ERROR") && table.contains("boom") && table.contains("[tpd MISSING]"), "{table}");
        assert!(format_aggregate(&agg).contains("1 pass, 2 fail, 1 error"));
    }

    #[test]
    fn hashes_are_stable() {
        assert_eq!(hash_text(""), "cbf29ce484222325");
        assert_eq!(hash_text("a"), "af63dc4c8601ec8c");
        assert_ne!(hash_text("a"), hash_text("b"));
    }

    #[test]
    fn store_lists_latest_and_aggregates() {
        let mut db = Store::in_memory().unwrap();
        db.insert(&rec("a", "1", Some(1.0), 2.0)).unwrap();
        db.insert(&rec("b", "2", Some(3.0), 2.0)).unwrap();
        let again = db.insert(&rec("a", "3", Some(1.5), 2.0)).unwrap();
        let mut other = rec("z", "4", Some(9.0), 2.0);
        other.view = "other".into();
        db.insert(&other).unwrap();
        let all = db.runs("cells", "tb", "testbench").unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].point.variables["vdd"], "1.2");
        assert_eq!(all[0].point.seed, Some(5));
        let latest = db.latest("cells", "tb", "testbench").unwrap();
        let names: Vec<&str> = latest.iter().map(|r| r.point.name.as_str()).collect();
        assert_eq!(names, ["a", "b"]);
        assert_eq!(latest[0].id, Some(again));
        assert_eq!(latest[0].measures["tpd"], 1.5);
        let agg = db.stats("cells", "tb", "testbench").unwrap();
        assert_eq!((agg.runs, agg.pass, agg.fail), (2, 1, 1));
        assert_eq!(agg.measures["tpd"].max, 3.0);
        assert_eq!(db.series("cells", "tb", "testbench", "TPD").unwrap(), [("a".to_string(), 1.5), ("b".to_string(), 3.0)]);
        assert_eq!(db.latest_of_setup("cells", "tb", "testbench", "nope").unwrap().len(), 0);
        let n: i64 = db.conn.query_row("SELECT COUNT(*) FROM measures WHERE verdict='fail'", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 2); // b, and z of the other view
    }

    #[test]
    fn json_roundtrip_and_reindex() {
        let dir = tempfile::tempdir().unwrap();
        let run_dir = dir.path().join("simulations/cells/tb/testbench/tt");
        std::fs::create_dir_all(&run_dir).unwrap();
        let mut r = rec("tt", "2026-01-01T00:00:00", Some(1.0), 2.0);
        r.dir = run_dir.clone();
        let file = r.write().unwrap();
        assert_eq!(RunRecord::read(&file).unwrap(), r);
        let mut db = Store::open(dir.path()).unwrap();
        assert_eq!(db.reindex(dir.path()).unwrap(), 1);
        assert_eq!(db.reindex(dir.path()).unwrap(), 1);
        assert_eq!(db.runs("cells", "tb", "testbench").unwrap().len(), 1);
        assert!(Store::path(dir.path()).is_file());
    }
}
