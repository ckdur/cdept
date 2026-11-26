//! Suites: a family of testbench views run together.
//!
//! A suite is a view of a cell with a `suite.toml` in it:
//!
//! ```toml
//! description = "inverter characterisation"
//! corners = ["tt", "ss", "ff"]      # optional: replaces the corners of every test
//! temperatures = [-40, 27, 125]     # optional: replaces their temperatures
//! [[test]]
//! view = "cells/tb_inv/tran_delay"  # library/cell/view; `cell/view` in the suite's own library
//! [[test]]
//! view = "tb_inv/power"
//! ```
//!
//! A `[[combine]]` computes a figure from the results of several tests at the same run
//! point (corner and temperature), after they all ran: `<test>.<measure>` names a
//! measure of a test (its view name, or its `name`), as in
//!
//! ```toml
//! [[combine]]
//! test = "psrr"                      # whose run points get the result (and its spec)
//! name = "psrr_rejection_db"
//! expr = "ac_open.gain_db - psrr.xfer_db"
//! min = 60
//! ```
//!
//! [`run`] runs every test's points on one shared job queue, each test's runs are stored
//! as usual (`result.json`, `results.db`), and the whole thing is one [`SuiteReport`]:
//! a combined table (rows per test and point, a column per key output), a verdict
//! summary (failing and missing specs, errors) and a record of its own in
//! `<project>/simulations/<lib>/<cell>/<suite>/suite.json` and the `suite_runs` table
//! of `results.db`. docs/TESTS.md.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use eyre::{Result, WrapErr, bail, eyre};
use serde::{Deserialize, Serialize};

use super::results::{Overall, RunRecord};
use super::spec::Verdict;
use crate::Project;

/// The file that makes a view a suite.
pub const SUITE_FILE: &str = "suite.toml";

/// `suite.toml`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Suite {
    pub description: String,
    /// Replaces the corners of every test when not empty.
    pub corners: Vec<String>,
    /// Replaces the temperatures of every test when not empty.
    pub temperatures: Vec<f64>,
    /// Runs every test with this simulator (`ngspice`, `xyce`, `vacask`) when not empty.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub simulator: String,
    #[serde(rename = "test")]
    pub tests: Vec<TestRef>,
    /// Figures computed from several tests' results at the same run point.
    #[serde(rename = "combine", skip_serializing_if = "Vec::is_empty")]
    pub combines: Vec<Combine>,
    /// The CACE datasheet the suite came from (`cdept cace import`): its header and the
    /// parameters that are not testbenches. Not used to run (docs/reference/files/cace.md).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cace: Option<toml::Table>,
}

/// A figure from several tests' results at the same run point (`[[combine]]`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Combine {
    /// The test whose run points get the figure (its view name or its `name`).
    pub test: String,
    pub name: String,
    /// A calculator expression (docs/CALC.md) over `<test>.<measure>` names.
    pub expr: String,
    pub min: Option<f64>,
    pub max: Option<f64>,
}

/// One test of the family.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TestRef {
    /// The testbench view: `library/cell/view`, `cell/view` (this library) or `view`
    /// (this cell).
    pub view: String,
    /// A label for the table; default the cell and view.
    pub name: String,
}

impl Suite {
    pub fn parse(text: &str) -> Result<Suite> {
        let s: Suite = toml::from_str(text).wrap_err("not a valid suite")?;
        if s.tests.is_empty() {
            bail!("a suite lists at least one [[test]]");
        }
        for t in &s.tests {
            if t.view.trim().is_empty() || t.view.split('/').count() > 3 {
                bail!("test view '{}' is not library/cell/view", t.view);
            }
        }
        for c in &s.combines {
            if c.name.trim().is_empty() || c.name.contains(char::is_whitespace) {
                bail!("combine '{}': the name must be one word", c.name);
            }
            if !s
                .tests
                .iter()
                .any(|t| t.key() == c.test || t.name == c.test)
            {
                bail!(
                    "combine '{}': no test '{}' in the suite (tests: {})",
                    c.name,
                    c.test,
                    s.tests
                        .iter()
                        .map(TestRef::key)
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            crate::calc::parse(&c.expr)
                .map_err(|e| eyre!("combine '{}': {}", c.name, e.render(&c.expr)))?;
        }
        Ok(s)
    }

    pub fn load(file: &Path) -> Result<Suite> {
        let text = std::fs::read_to_string(file)
            .wrap_err_with(|| format!("cannot read {}", file.display()))?;
        Self::parse(&text).wrap_err_with(|| file.display().to_string())
    }
}

impl TestRef {
    /// The name `[[combine]]` knows the test by: its view (the last part of `view`).
    pub fn key(&self) -> String {
        self.view
            .trim()
            .rsplit('/')
            .next()
            .unwrap_or("")
            .to_string()
    }

    /// The `(library, cell, view)` of the test, relative to the suite's own.
    pub fn resolve(&self, library: &str, cell: &str) -> (String, String, String) {
        let parts: Vec<&str> = self.view.trim().split('/').collect();
        match parts.as_slice() {
            [l, c, v] => (l.to_string(), c.to_string(), v.to_string()),
            [c, v] => (library.to_string(), c.to_string(), v.to_string()),
            [v] => (library.to_string(), cell.to_string(), v.to_string()),
            _ => (library.into(), cell.into(), self.view.clone()),
        }
    }
}

/// The suite view `library/cell/view`.
pub fn open(project: &Project, library: &str, cell: &str, view: &str) -> Result<Suite> {
    let entry = project
        .library(library)
        .and_then(|l| l.cell(cell))
        .and_then(|c| c.entry(view))
        .ok_or_else(|| eyre!("no view '{view}' of {library}/{cell}"))?;
    let file = entry.path.join(SUITE_FILE);
    if !file.is_file() {
        bail!("{library}/{cell}/{view} is not a suite (no {SUITE_FILE})");
    }
    Suite::load(&file)
}

/// The suite views of a cell.
pub fn suites_of(project: &Project, library: &str, cell: &str) -> Vec<String> {
    project
        .library(library)
        .and_then(|l| l.cell(cell))
        .map(|c| {
            c.entries
                .iter()
                .filter(|e| e.path.join(SUITE_FILE).is_file())
                .map(|e| e.name.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// What happened to one test.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TestReport {
    /// `library/cell/view`.
    pub id: String,
    /// The table label.
    pub label: String,
    /// The template the testbench uses, if any.
    pub template: String,
    /// The test could not run at all (not a testbench, template or net error, no simulator).
    pub error: Option<String>,
    /// The latest runs of its points.
    pub records: Vec<RunRecord>,
}

/// The report of a suite run.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SuiteReport {
    pub library: String,
    pub cell: String,
    pub view: String,
    pub description: String,
    pub started: String,
    pub duration_ms: u64,
    pub tests: Vec<TestReport>,
}

/// The verdict counts of a report.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    pub tests: usize,
    /// Tests that did not run at all.
    pub test_errors: usize,
    pub runs: usize,
    pub pass: usize,
    pub fail: usize,
    pub error: usize,
    /// Specs that failed / did not come out, over all runs.
    pub spec_fail: usize,
    pub spec_missing: usize,
}

impl Summary {
    pub fn ok(&self) -> bool {
        self.test_errors == 0 && self.fail == 0 && self.error == 0
    }
}

impl TestReport {
    pub fn overall(&self) -> Overall {
        if self.error.is_some() || self.records.is_empty() {
            return Overall::Error;
        }
        let all: Vec<Overall> = self.records.iter().map(|r| r.overall()).collect();
        if all.contains(&Overall::Error) {
            Overall::Error
        } else if all.contains(&Overall::Fail) {
            Overall::Fail
        } else {
            Overall::Pass
        }
    }
}

impl SuiteReport {
    pub fn summary(&self) -> Summary {
        let mut s = Summary {
            tests: self.tests.len(),
            ..Default::default()
        };
        for t in &self.tests {
            if t.error.is_some() {
                s.test_errors += 1;
            }
            for r in &t.records {
                s.runs += 1;
                match r.overall() {
                    Overall::Pass => s.pass += 1,
                    Overall::Fail => s.fail += 1,
                    Overall::Error => s.error += 1,
                }
                for sp in &r.specs {
                    match sp.verdict {
                        Verdict::Fail => s.spec_fail += 1,
                        Verdict::Missing => s.spec_missing += 1,
                        Verdict::Pass => {}
                    }
                }
            }
        }
        s
    }
}

/// How [`run`] runs.
#[derive(Clone, Default)]
pub struct SuiteOptions {
    /// Simulators running at once (default: the cores).
    pub jobs: usize,
    /// Called on every job event, from worker threads.
    pub on_event: Option<crate::jobs::Listener>,
    pub no_store: bool,
    /// Use this queue (a UI's) instead of a private one.
    pub queue: Option<Arc<crate::jobs::Queue>>,
    pub cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    /// Run every test with this simulator (`--simulator`); wins over the suite's.
    pub simulator: Option<String>,
    /// Run every test's points on this remote (`--remote`, [`crate::jobs::remote`]).
    pub remote: Option<String>,
    /// The time limit of each run point (`--timeout`; zero: none), as
    /// [`super::RunOptions::timeout`].
    pub timeout: Option<std::time::Duration>,
}

/// Where the report goes: `<project>/simulations/<lib>/<cell>/<suite view>/suite.json`.
pub fn report_file(project: &Project, library: &str, cell: &str, view: &str) -> PathBuf {
    super::runs_dir(project, library, cell, view).join("suite.json")
}

/// Run every test of the suite `library/cell/view`.
pub fn run(
    project: &Project,
    library: &str,
    cell: &str,
    view: &str,
    options: &SuiteOptions,
) -> Result<SuiteReport> {
    let suite = open(project, library, cell, view)?;
    // Every test is forced to VACASK: refuse at once when it cannot run here.
    let forced = options
        .simulator
        .as_deref()
        .or(Some(suite.simulator.as_str()).filter(|s| !s.is_empty()));
    if forced == Some("vacask") {
        crate::vacask::availability(project).check()?;
    }
    let started = chrono::Local::now().to_rfc3339();
    let clock = std::time::Instant::now();
    let queue = match &options.queue {
        Some(q) => q.clone(),
        None => {
            let workers = if options.jobs > 0 {
                options.jobs
            } else {
                crate::jobs::Queue::default_workers()
            };
            let q = Arc::new(crate::jobs::Queue::new(workers));
            if let Some(f) = &options.on_event {
                let f = f.clone();
                q.on_event(move |e| f(e));
            }
            q
        }
    };
    let run_options = super::RunOptions {
        no_store: options.no_store,
        queue: Some(queue),
        cancel: options.cancel.clone(),
        remote: options.remote.clone(),
        timeout: options.timeout,
        ..Default::default()
    };
    // Tests are netlisted and queued from their own threads; the simulations of all of
    // them share the queue.
    let tests: Vec<TestReport> = std::thread::scope(|scope| {
        let handles: Vec<_> = suite
            .tests
            .iter()
            .map(|t| {
                let (l, c, v) = t.resolve(library, cell);
                let label = if t.name.is_empty() {
                    format!("{c}/{v}")
                } else {
                    t.name.clone()
                };
                let run_options = run_options.clone();
                let suite = &suite;
                let sim = options.simulator.as_deref();
                scope
                    .spawn(move || run_test(project, suite, (&l, &c, &v), label, &run_options, sim))
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("a suite test panicked"))
            .collect()
    });
    let mut tests = tests;
    combine(&suite, &mut tests, !options.no_store);
    let report = SuiteReport {
        library: library.into(),
        cell: cell.into(),
        view: view.into(),
        description: suite.description.clone(),
        started,
        duration_ms: clock.elapsed().as_millis() as u64,
        tests,
    };
    if !options.no_store
        && let Err(e) = store(project, &report)
    {
        tracing::warn!("suite record: {e:#}");
    }
    Ok(report)
}

/// Evaluate the suite's `[[combine]]`s on every run point of their test: the measures of
/// all tests at the point of the same name are the expression's `<test>.<measure>`
/// names. The figure joins the point's measures (a calculator measure), its spec is
/// checked, and the point's `result.json` is written again (`write`).
pub fn combine(suite: &Suite, tests: &mut [TestReport], write: bool) {
    if suite.combines.is_empty() {
        return;
    }
    // test key -> point -> measures
    let mut by_test: Vec<(
        Vec<String>,
        std::collections::BTreeMap<String, std::collections::BTreeMap<String, f64>>,
    )> = vec![];
    for (t, report) in suite.tests.iter().zip(tests.iter()) {
        let mut names = vec![t.key()];
        if !t.name.is_empty() {
            names.push(t.name.clone());
        }
        let points = report
            .records
            .iter()
            .map(|r| (r.point.name.clone(), r.measures.clone()))
            .collect();
        by_test.push((names, points));
    }
    for c in &suite.combines {
        let Some(target) = suite
            .tests
            .iter()
            .position(|t| t.key() == c.test || t.name == c.test)
        else {
            continue;
        };
        let Ok(expr) = crate::calc::parse(&c.expr) else {
            continue;
        };
        for record in &mut tests[target].records {
            let mut env = crate::calc::Env::empty();
            for (names, points) in &by_test {
                if let Some(m) = points.get(&record.point.name) {
                    for (k, v) in m {
                        for n in names {
                            env.set(&format!("{n}.{k}"), crate::calc::Value::Num(*v));
                        }
                    }
                }
            }
            let key = c.name.to_lowercase();
            let value = match crate::calc::eval(&expr, &env) {
                Ok(v) => match v.as_f64().filter(|v| v.is_finite()) {
                    Some(v) => Some(v),
                    None => {
                        record.calc_errors.insert(
                            key.clone(),
                            format!("{} is not a finite number", v.display()),
                        );
                        None
                    }
                },
                Err(e) => {
                    record.calc_errors.insert(key.clone(), e.render(&c.expr));
                    None
                }
            };
            if let Some(v) = value {
                record.measures.insert(key.clone(), v);
                record.calc_errors.remove(&key);
            }
            if !record.calc.contains(&key) {
                record.calc.push(key.clone());
            }
            if c.min.is_some() || c.max.is_some() {
                let verdict = match value {
                    None => Verdict::Missing,
                    Some(v) if c.min.is_some_and(|m| v < m) || c.max.is_some_and(|m| v > m) => {
                        Verdict::Fail
                    }
                    Some(_) => Verdict::Pass,
                };
                record.specs.retain(|s| s.name.to_lowercase() != key);
                record.specs.push(super::results::SpecRecord {
                    name: key.clone(),
                    value,
                    min: c.min,
                    max: c.max,
                    verdict,
                });
            }
            if write
                && record.dir.is_dir()
                && let Err(e) = record.write()
            {
                tracing::warn!("combine {}: {e:#}", c.name);
            }
        }
    }
}

fn run_test(
    project: &Project,
    suite: &Suite,
    (library, cell, view): (&str, &str, &str),
    label: String,
    options: &super::RunOptions,
    simulator: Option<&str>,
) -> TestReport {
    let mut report = TestReport {
        id: format!("{library}/{cell}/{view}"),
        label,
        ..Default::default()
    };
    let outcome = (|| -> Result<Vec<RunRecord>> {
        let mut tb = super::open(project, library, cell, view)?;
        report.template = tb.setup.template.clone();
        if !suite.corners.is_empty() {
            tb.setup.corners = suite.corners.clone();
        }
        if !suite.temperatures.is_empty() {
            tb.setup.temperatures = suite.temperatures.clone();
        }
        let chosen = simulator
            .map(str::to_string)
            .or_else(|| Some(suite.simulator.clone()).filter(|s| !s.is_empty()));
        if let Some(name) = chosen {
            tb.setup.set_simulator(&name)?;
        }
        let remote = project
            .jobs
            .pick(options.remote.as_deref(), &tb.setup.remote)?
            .is_some();
        let simulator = (if remote {
            super::Simulator::for_remote(&tb.setup)
        } else {
            super::Simulator::for_setup(&tb.setup)
        })
        .ok_or_else(|| eyre!("the simulator '{}' was not found", tb.setup.simulator))?;
        let results = super::run_testbench_with(project, &tb, &simulator, &[], options)?;
        Ok(results.into_iter().map(|r| r.run).collect())
    })();
    match outcome {
        Ok(records) => report.records = records,
        Err(e) => report.error = Some(format!("{e:#}")),
    }
    report
}

/// Write `suite.json` and add a row to `suite_runs` in `results.db`.
fn store(project: &Project, report: &SuiteReport) -> Result<()> {
    let file = report_file(project, &report.library, &report.cell, &report.view);
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&file, serde_json::to_string_pretty(report)?)?;
    // The database is opened (and created) by the results store; the suite table is ours.
    drop(super::results::Store::open(&project.root)?);
    let conn = rusqlite::Connection::open(super::results::Store::path(&project.root))?;
    conn.busy_timeout(std::time::Duration::from_secs(10))?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS suite_runs (
             id INTEGER PRIMARY KEY,
             library TEXT NOT NULL, cell TEXT NOT NULL, view TEXT NOT NULL,
             started TEXT NOT NULL, duration_ms INTEGER NOT NULL,
             tests INTEGER NOT NULL, runs INTEGER NOT NULL,
             pass INTEGER NOT NULL, fail INTEGER NOT NULL, error INTEGER NOT NULL,
             spec_fail INTEGER NOT NULL, spec_missing INTEGER NOT NULL,
             record TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS suite_runs_view ON suite_runs (library, cell, view);",
    )?;
    let s = report.summary();
    conn.execute(
        "INSERT INTO suite_runs (library, cell, view, started, duration_ms, tests, runs, pass, fail, error, spec_fail, spec_missing, record)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        rusqlite::params![
            report.library,
            report.cell,
            report.view,
            report.started,
            report.duration_ms as i64,
            s.tests as i64,
            s.runs as i64,
            s.pass as i64,
            s.fail as i64,
            s.error as i64,
            s.spec_fail as i64,
            s.spec_missing as i64,
            serde_json::to_string(report)?,
        ],
    )?;
    Ok(())
}

/// The latest stored report of a suite, from `suite.json`.
pub fn latest(project: &Project, library: &str, cell: &str, view: &str) -> Option<SuiteReport> {
    let text = std::fs::read_to_string(report_file(project, library, cell, view)).ok()?;
    serde_json::from_str(&text).ok()
}

fn num(v: f64) -> String {
    format!("{v:.4e}")
}

/// The columns of the combined table: the measures a spec names in some test, or every
/// measure when `all` (or when no test has a spec).
pub fn columns(report: &SuiteReport, all: bool) -> Vec<String> {
    let mut specced = std::collections::BTreeSet::new();
    let mut every = std::collections::BTreeSet::new();
    for r in report.tests.iter().flat_map(|t| &t.records) {
        every.extend(r.measures.keys().cloned());
        specced.extend(r.specs.iter().map(|s| s.name.to_lowercase()));
    }
    if all || specced.is_empty() {
        return every.into_iter().collect();
    }
    // Keep the order of first appearance within each test, so a test's outputs stay together.
    let mut out: Vec<String> = vec![];
    for r in report.tests.iter().flat_map(|t| &t.records) {
        for s in &r.specs {
            let n = s.name.to_lowercase();
            if !out.contains(&n) {
                out.push(n);
            }
        }
    }
    out
}

/// The combined table: a row per test and point, a column per output, the verdict of
/// each row, then the verdict summary.
pub fn format_report(report: &SuiteReport, all_columns: bool) -> String {
    let cols = columns(report, all_columns);
    let tw = report
        .tests
        .iter()
        .map(|t| t.label.len())
        .max()
        .unwrap_or(4)
        .max(4);
    let pw = report
        .tests
        .iter()
        .flat_map(|t| &t.records)
        .map(|r| r.point.name.len())
        .max()
        .unwrap_or(5)
        .max(5);
    let mut out = format!("{:<tw$}  {:<pw$}  {:<6}", "test", "point", "result");
    for c in &cols {
        out.push_str(&format!("  {c:>12}"));
    }
    out.push('\n');
    for t in &report.tests {
        if let Some(e) = &t.error {
            out.push_str(&format!(
                "{:<tw$}  {:<pw$}  {:<6}   {}\n",
                t.label,
                "-",
                "ERROR",
                e.lines().next().unwrap_or("")
            ));
            continue;
        }
        for r in &t.records {
            out.push_str(&format!(
                "{:<tw$}  {:<pw$}  {:<6}",
                t.label,
                r.point.name,
                r.overall().word().to_uppercase()
            ));
            for c in &cols {
                match r.measures.get(c) {
                    Some(v) => out.push_str(&format!("  {:>12}", num(*v))),
                    None => out.push_str(&format!("  {:>12}", "-")),
                }
            }
            if let Some(e) = &r.error {
                out.push_str(&format!("   {}", e.lines().next().unwrap_or("")));
            }
            out.push('\n');
        }
    }
    out.push('\n');
    // What did not pass.
    let mut bad = String::new();
    for t in &report.tests {
        for r in &t.records {
            for s in r.specs.iter().filter(|s| s.verdict != Verdict::Pass) {
                let bound = match (s.min, s.max) {
                    (Some(a), Some(b)) => format!("[{}, {}]", num(a), num(b)),
                    (Some(a), None) => format!(">= {}", num(a)),
                    (None, Some(b)) => format!("<= {}", num(b)),
                    (None, None) => String::new(),
                };
                bad.push_str(&format!(
                    "  {} / {}: {} {} = {} (needs {bound})\n",
                    t.label,
                    r.point.name,
                    s.name,
                    if s.verdict == Verdict::Fail {
                        "FAIL"
                    } else {
                        "MISSING"
                    },
                    s.value.map_or("-".into(), num)
                ));
            }
            for (k, e) in &r.calc_errors {
                bad.push_str(&format!(
                    "  {} / {}: output {k}: {e}\n",
                    t.label, r.point.name
                ));
            }
        }
    }
    if !bad.is_empty() {
        out.push_str("not passing:\n");
        out.push_str(&bad);
        out.push('\n');
    }
    let s = report.summary();
    out.push_str(&format!(
        "suite {}/{}/{}: {} test(s), {} run(s): {} pass, {} fail, {} error; specs: {} failing, {} missing{}\n",
        report.library,
        report.cell,
        report.view,
        s.tests,
        s.runs,
        s.pass,
        s.fail,
        s.error,
        s.spec_fail,
        s.spec_missing,
        if s.test_errors > 0 { format!("; {} test(s) did not run", s.test_errors) } else { String::new() }
    ));
    out.push_str(if s.ok() {
        "SUITE PASS\n"
    } else {
        "SUITE FAIL\n"
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::results::{PointRecord, SpecRecord, Status};
    use std::collections::BTreeMap;

    fn record(
        test: &str,
        point: &str,
        measures: &[(&str, f64)],
        specs: Vec<SpecRecord>,
    ) -> RunRecord {
        RunRecord {
            schema: 1,
            id: None,
            library: "l".into(),
            cell: "c".into(),
            view: test.into(),
            setup_hash: String::new(),
            deck_hash: String::new(),
            simulator: "ngspice".into(),
            point: PointRecord {
                name: point.into(),
                ..Default::default()
            },
            status: Status::Ok,
            error: None,
            measures: measures
                .iter()
                .map(|(k, v)| (k.to_string(), *v))
                .collect::<BTreeMap<_, _>>(),
            calc: vec![],
            calc_errors: BTreeMap::new(),
            specs,
            started: String::new(),
            duration_ms: 0,
            dir: PathBuf::new(),
            deck: None,
            raw: None,
            log: None,
        }
    }

    fn spec(name: &str, value: Option<f64>, max: f64, verdict: Verdict) -> SpecRecord {
        SpecRecord {
            name: name.into(),
            value,
            min: None,
            max: Some(max),
            verdict,
        }
    }

    #[test]
    fn combines_use_other_tests_at_the_same_point() {
        let suite = Suite::parse(
            "[[test]]\nview='ac_open'\n[[test]]\nview='psrr'\n\
             [[combine]]\ntest='psrr'\nname='rejection_db'\nexpr='ac_open.gain_db - psrr.xfer_db'\nmin=40\n",
        )
        .unwrap();
        let mut tests = vec![
            TestReport {
                records: vec![
                    record("ac_open", "tt_27", &[("gain_db", 36.0)], vec![]),
                    record("ac_open", "ss_27", &[("gain_db", 33.0)], vec![]),
                ],
                ..Default::default()
            },
            TestReport {
                records: vec![
                    record("psrr", "tt_27", &[("xfer_db", -10.0)], vec![]),
                    record("psrr", "ss_27", &[("xfer_db", -5.0)], vec![]),
                    record("psrr", "ff_27", &[("xfer_db", -5.0)], vec![]),
                ],
                ..Default::default()
            },
        ];
        combine(&suite, &mut tests, false);
        let r = &tests[1].records;
        assert_eq!(r[0].measures["rejection_db"], 46.0);
        assert_eq!(r[0].specs[0].verdict, Verdict::Pass);
        assert_eq!(r[1].measures["rejection_db"], 38.0);
        assert_eq!(r[1].specs[0].verdict, Verdict::Fail);
        // ac_open has no ff_27: missing, with the reason
        assert!(!r[2].measures.contains_key("rejection_db"));
        assert_eq!(r[2].specs[0].verdict, Verdict::Missing);
        assert!(r[2].calc_errors["rejection_db"].contains("ac_open.gain_db"));
        // bad combines are refused when the suite is read
        assert!(
            Suite::parse("[[test]]\nview='a'\n[[combine]]\ntest='b'\nname='x'\nexpr='1'").is_err()
        );
        assert!(
            Suite::parse("[[test]]\nview='a'\n[[combine]]\ntest='a'\nname='x'\nexpr='1 +'")
                .is_err()
        );
        assert!(
            Suite::parse("[[test]]\nview='a'\n[[combine]]\ntest='a'\nname='x y'\nexpr='1'")
                .is_err()
        );
    }

    #[test]
    fn parses_and_resolves() {
        let s = Suite::parse(
            "description='d'\ncorners=['tt']\n[[test]]\nview='a/b/c'\n[[test]]\nview='b/c'\nname='x'\n[[test]]\nview='c'",
        )
        .unwrap();
        assert_eq!(
            s.tests[0].resolve("L", "C"),
            ("a".into(), "b".into(), "c".into())
        );
        assert_eq!(
            s.tests[1].resolve("L", "C"),
            ("L".into(), "b".into(), "c".into())
        );
        assert_eq!(
            s.tests[2].resolve("L", "C"),
            ("L".into(), "C".into(), "c".into())
        );
        assert!(Suite::parse("description='x'").is_err());
        assert!(Suite::parse("[[test]]\nview='a/b/c/d'").is_err());
        assert!(Suite::parse("[[test]]\nview='a'\nbogus=1").is_err());
    }

    #[test]
    fn summary_and_table() {
        let ok = record(
            "t1",
            "tt_27",
            &[("tpd", 1e-10), ("t_rise", 2e-11)],
            vec![spec("tpd", Some(1e-10), 3e-10, Verdict::Pass)],
        );
        let bad = record(
            "t2",
            "tt_27",
            &[("p_avg", 5e-6)],
            vec![
                spec("p_avg", Some(5e-6), 1e-6, Verdict::Fail),
                spec("e_cycle", None, 1e-15, Verdict::Missing),
            ],
        );
        let report = SuiteReport {
            library: "l".into(),
            cell: "c".into(),
            view: "suite".into(),
            tests: vec![
                TestReport {
                    id: "l/c/t1".into(),
                    label: "t1".into(),
                    records: vec![ok],
                    ..Default::default()
                },
                TestReport {
                    id: "l/c/t2".into(),
                    label: "t2".into(),
                    records: vec![bad],
                    ..Default::default()
                },
                TestReport {
                    id: "l/c/t3".into(),
                    label: "t3".into(),
                    error: Some("net 'x' missing".into()),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let s = report.summary();
        assert_eq!((s.tests, s.runs, s.pass, s.fail, s.error), (3, 2, 1, 1, 0));
        assert_eq!((s.spec_fail, s.spec_missing, s.test_errors), (1, 1, 1));
        assert!(!s.ok());
        assert_eq!(report.tests[2].overall(), Overall::Error);
        // key columns are those with a spec
        assert_eq!(columns(&report, false), ["tpd", "p_avg", "e_cycle"]);
        assert!(columns(&report, true).contains(&"t_rise".to_string()));
        let text = format_report(&report, false);
        assert!(
            text.contains("t1") && text.contains("FAIL") && text.contains("ERROR"),
            "{text}"
        );
        assert!(
            text.contains("e_cycle MISSING") && text.contains("p_avg FAIL"),
            "{text}"
        );
        assert!(
            text.contains("SUITE FAIL") && text.contains("1 missing"),
            "{text}"
        );
    }
}
