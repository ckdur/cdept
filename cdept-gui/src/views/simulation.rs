//! Simulation tab: edit the `setup.toml` of a testbench view, run it, watch the jobs
//! and read the results (docs/SIMULATION.md §5).
//!
//! The editor is a form over `libcdept::sim::Setup`: what it does not show (fields a
//! later version of the schema adds) is kept as it was loaded. Saving writes the file
//! back (comments before the first key stay) after `Setup::parse` accepted it. A run
//! uses `sim::run_testbench_with` on a thread, with the panel's own `jobs::Queue`; the
//! panel polls `Queue::snapshot`, and the queue's events wake the window up.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use egui::{Color32, ComboBox, Grid, RichText, ScrollArea, TextEdit, Ui};
use libcdept::jobs::{JobInfo, JobState, Queue};
use libcdept::sim::spec::{Row, Verdict};
use libcdept::sim::{self, RunOptions, Setup, Simulator};
use libcdept::{Project, ViewKind};

use super::{View, empty_state};
use crate::state::{Action, AppCtx, Selection};

/// The testbench a tab is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Target {
    pub library: String,
    pub cell: String,
    pub view: String,
}

impl Target {
    fn text(&self) -> String {
        format!("{}/{}/{}", self.library, self.cell, self.view)
    }
}

/// The first testbench view of a cell, or the view the selection names when it is one.
pub(crate) fn testbench_of(project: &Project, selection: &Selection) -> Option<Target> {
    let library = project.library(selection.library.as_deref()?)?;
    let cell = library.cell(selection.cell.as_deref()?)?;
    let is_tb = |name: &str| cell.entry(name).is_some_and(|e| e.kind == ViewKind::Testbench);
    let view = match &selection.view {
        Some(view) if is_tb(view) => view.clone(),
        Some(_) => return None,
        None => cell.entries.iter().find(|e| e.kind == ViewKind::Testbench)?.name.clone(),
    };
    Some(Target { library: library.name.clone(), cell: cell.name.clone(), view })
}

/// Is this view a testbench (has a `setup.toml`)?
pub(crate) fn is_testbench(entry: &libcdept::Entry) -> bool {
    entry.kind == ViewKind::Testbench
}

// --- the form ----------------------------------------------------------------------

const ANALYSIS_KINDS: [&str; 5] = ["op", "dc", "ac", "tran", "noise"];

/// Field names of an analysis kind, in the order of its form.
fn analysis_fields(kind: &str) -> &'static [&'static str] {
    match kind {
        "dc" => &["source", "start", "stop", "step"],
        "ac" => &["sweep", "points", "start", "stop"],
        "tran" => &["step", "stop", "start"],
        "noise" => &["output", "source", "sweep", "points", "start", "stop"],
        _ => &[],
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
struct AnalysisForm {
    kind: String,
    /// Values by field name.
    fields: Vec<(String, String)>,
}

impl AnalysisForm {
    fn get(&self, name: &str) -> &str {
        self.fields.iter().find(|(n, _)| n == name).map_or("", |(_, v)| v.as_str())
    }

    fn set(&mut self, name: &str, value: String) {
        match self.fields.iter_mut().find(|(n, _)| n == name) {
            Some((_, v)) => *v = value,
            None => self.fields.push((name.to_string(), value)),
        }
    }
}

/// The editable part of a `setup.toml`.
#[derive(Debug, Clone, PartialEq)]
struct Form {
    /// As loaded: what the form does not show goes back as it was.
    base: Setup,
    simulator: String,
    models: String,
    schematic: String,
    schematic_view: String,
    corners: Vec<String>,
    temperatures: String,
    variables: Vec<(String, String)>,
    options: Vec<(String, String)>,
    save: String,
    measures: String,
    analyses: Vec<AnalysisForm>,
    /// name, min, max
    specs: Vec<(String, String, String)>,
}

fn lines(text: &str) -> Vec<String> {
    text.lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect()
}

fn number(value: &str) -> String {
    let n: f64 = value.parse().unwrap_or(0.0);
    if n.fract() == 0.0 && n.abs() < 1e15 { format!("{}", n as i64) } else { format!("{n}") }
}

impl Form {
    fn of(setup: &Setup) -> Result<Form, String> {
        // Through the table: the analyses' fields are those of the file.
        let table = toml::Table::try_from(setup).map_err(|e| e.to_string())?;
        let text = |t: &toml::Table, k: &str| match t.get(k) {
            Some(toml::Value::String(s)) => s.clone(),
            Some(toml::Value::Integer(i)) => i.to_string(),
            Some(toml::Value::Float(f)) => f.to_string(),
            _ => String::new(),
        };
        let pairs = |k: &str| -> Vec<(String, String)> {
            match table.get(k) {
                Some(toml::Value::Table(t)) => t
                    .iter()
                    .map(|(n, v)| (n.clone(), v.as_str().map_or_else(|| v.to_string(), String::from)))
                    .collect(),
                _ => Vec::new(),
            }
        };
        let analyses = match table.get("analysis") {
            Some(toml::Value::Array(items)) => items
                .iter()
                .filter_map(toml::Value::as_table)
                .map(|t| AnalysisForm {
                    kind: text(t, "kind"),
                    fields: analysis_fields(&text(t, "kind"))
                        .iter()
                        .map(|f| (f.to_string(), text(t, f)))
                        .collect(),
                })
                .collect(),
            _ => Vec::new(),
        };
        Ok(Form {
            base: setup.clone(),
            simulator: if setup.simulator.is_empty() { "ngspice".into() } else { setup.simulator.clone() },
            models: setup.models.clone(),
            schematic: setup.schematic.clone(),
            schematic_view: setup.schematic_view.clone(),
            corners: setup.corners.clone(),
            temperatures: setup.temperatures.iter().map(|t| number(&t.to_string())).collect::<Vec<_>>().join(", "),
            variables: pairs("variables"),
            options: pairs("options"),
            save: setup.save.join("\n"),
            measures: setup.measures.join("\n"),
            analyses,
            specs: setup
                .specs
                .iter()
                .map(|s| {
                    (
                        s.name.clone(),
                        s.min.map(|v| v.to_string()).unwrap_or_default(),
                        s.max.map(|v| v.to_string()).unwrap_or_default(),
                    )
                })
                .collect(),
        })
    }

    /// The text of the file this form stands for (validated by `Setup::parse`).
    fn to_text(&self) -> Result<String, String> {
        use toml::Value;
        let mut table = toml::Table::try_from(&self.base).map_err(|e| e.to_string())?;
        let mut set = |key: &str, value: Value| {
            table.insert(key.to_string(), value);
        };
        let string = |s: &str| Value::String(s.trim().to_string());
        set("simulator", string(if self.simulator == "ngspice" { "" } else { &self.simulator }));
        set("models", string(&self.models));
        set("schematic", string(&self.schematic));
        set("schematic_view", string(&self.schematic_view));
        set("corners", Value::Array(self.corners.iter().map(|c| string(c)).collect()));
        let mut temps = Vec::new();
        for word in self.temperatures.split([',', ' ']).filter(|w| !w.is_empty()) {
            let t: f64 = word.parse().map_err(|_| format!("temperature '{word}' is not a number"))?;
            temps.push(Value::Float(t));
        }
        set("temperatures", Value::Array(temps));
        let map = |pairs: &[(String, String)]| {
            Value::Table(
                pairs
                    .iter()
                    .filter(|(n, _)| !n.trim().is_empty())
                    .map(|(n, v)| (n.trim().to_string(), string(v)))
                    .collect(),
            )
        };
        set("variables", map(&self.variables));
        set("options", map(&self.options));
        set("save", Value::Array(lines(&self.save).into_iter().map(Value::String).collect()));
        set("measures", Value::Array(lines(&self.measures).into_iter().map(Value::String).collect()));
        let mut analyses = Vec::new();
        for a in &self.analyses {
            let mut t = toml::Table::new();
            t.insert("kind".into(), Value::String(a.kind.clone()));
            for (name, value) in &a.fields {
                let value = value.trim();
                if value.is_empty() {
                    continue;
                }
                let v = if name == "points" {
                    Value::Integer(value.parse().map_err(|_| format!("points '{value}' is not a whole number"))?)
                } else {
                    Value::String(value.to_string())
                };
                t.insert(name.clone(), v);
            }
            analyses.push(Value::Table(t));
        }
        set("analysis", Value::Array(analyses));
        let mut specs = Vec::new();
        for (name, min, max) in &self.specs {
            let mut t = toml::Table::new();
            t.insert("name".into(), string(name));
            for (key, value) in [("min", min), ("max", max)] {
                let value = value.trim();
                if !value.is_empty() {
                    let v: f64 = value.parse().map_err(|_| format!("spec '{name}': '{value}' is not a number"))?;
                    t.insert(key.into(), Value::Float(v));
                }
            }
            specs.push(Value::Table(t));
        }
        set("spec", Value::Array(specs));
        prune(&mut table);
        let text = toml::to_string_pretty(&table).map_err(|e| e.to_string())?;
        Setup::parse(&text).map_err(|e| format!("{e:#}"))?;
        Ok(text)
    }

    fn setup(&self) -> Result<Setup, String> {
        Setup::parse(&self.to_text()?).map_err(|e| format!("{e:#}"))
    }
}

/// Leave out what is empty (a missing key is the default), at the top and in tables.
fn prune(table: &mut toml::Table) {
    use toml::Value;
    table.retain(|_, v| {
        match v {
            Value::Table(t) => prune(t),
            Value::Array(items) => items.iter_mut().for_each(|i| {
                if let Value::Table(t) = i {
                    prune(t)
                }
            }),
            _ => {}
        }
        !match v {
            Value::String(s) => s.is_empty(),
            Value::Array(a) => a.is_empty(),
            Value::Table(t) => t.is_empty(),
            _ => false,
        }
    });
}

/// The comment lines the file starts with.
fn header_comment(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        if line.trim_start().starts_with('#') {
            out.push_str(line);
            out.push('\n');
        } else if line.trim().is_empty() && out.is_empty() {
            continue;
        } else {
            break;
        }
    }
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

// --- runs ----------------------------------------------------------------------------

/// What a finished run point left.
#[derive(Debug, Clone)]
pub(crate) struct PointRow {
    pub name: String,
    pub error: Option<String>,
    pub measures: Vec<(String, f64)>,
    pub specs: Vec<Row>,
    pub dir: PathBuf,
}

type Slot = Arc<Mutex<Option<Result<Vec<PointRow>, String>>>>;

struct Run {
    /// Jobs of the queue from this index on belong to the run.
    first_job: usize,
    started: Instant,
    cancel: Arc<AtomicBool>,
    done: Slot,
    cancelled: bool,
    /// The outcome was taken from `done`.
    finished: bool,
}

/// Where a run is, for the status line and the test scripts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RunState {
    Idle,
    Running,
    Finished,
}

#[derive(Default)]
pub struct SimulationView {
    target: Option<Target>,
    form: Option<Form>,
    /// What the file says (canonical text), to know whether the form changed.
    saved: String,
    header: String,
    /// Why the form is not valid, or why the file could not be read or written.
    problem: Option<String>,
    queue: Option<Arc<Queue>>,
    run: Option<Run>,
    results: Vec<PointRow>,
    /// The run ended with an error before any point (netlist, tool not found).
    run_error: Option<String>,
    /// The corners to run, empty: all of the setup.
    only: Vec<String>,
    /// Run as soon as the testbench is loaded (the Library Manager's Run).
    run_when_loaded: bool,
    egui: Option<egui::Context>,
}

impl SimulationView {
    fn path(project: &Project, target: &Target) -> Option<PathBuf> {
        let entry = project.library(&target.library)?.cell(&target.cell)?.entry(&target.view)?;
        Some(entry.path.join(sim::setup::SETUP_FILE))
    }

    /// Show this testbench: read its `setup.toml` (changes of the form are dropped).
    pub(crate) fn open(&mut self, project: &Project, target: Target, run: bool) {
        self.results.clear();
        self.run_error = None;
        self.only.clear();
        if self.state() != RunState::Running {
            self.run = None;
        }
        self.target = Some(target.clone());
        self.form = None;
        self.problem = None;
        let loaded = Self::path(project, &target)
            .ok_or_else(|| format!("{} is not a view of the project", target.text()))
            .and_then(|file| {
                let text = std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
                let setup = Setup::parse(&text).map_err(|e| format!("{e:#}"))?;
                Ok((text, Form::of(&setup)?))
            });
        match loaded {
            Ok((text, form)) => {
                self.header = header_comment(&text);
                self.saved = form.to_text().unwrap_or_default();
                self.form = Some(form);
            }
            Err(e) => self.problem = Some(e),
        }
        self.run_when_loaded = run;
        tracing::info!(target: "cdept::sim", "simulation: {}", target.text());
    }

    pub(crate) fn target(&self) -> Option<&Target> {
        self.target.as_ref()
    }

    pub(crate) fn dirty(&self) -> bool {
        match &self.form {
            Some(form) => form.to_text().map_or(true, |t| t != self.saved),
            None => false,
        }
    }

    pub(crate) fn state(&self) -> RunState {
        match &self.run {
            _ if self.run_when_loaded => RunState::Running,
            Some(run) if !run.finished => RunState::Running,
            Some(_) => RunState::Finished,
            None => RunState::Idle,
        }
    }

    /// What is wrong: the form (as the tab shows it), or the last read, write or run.
    pub(crate) fn problem(&self) -> Option<String> {
        if let Some(p) = &self.problem {
            return Some(p.clone());
        }
        if let Some(form) = &self.form
            && let Err(e) = form.to_text()
        {
            return Some(e);
        }
        self.run_error.clone()
    }

    /// Validate the form and write the file.
    pub(crate) fn save(&mut self, project: &Project) -> Result<(), String> {
        let (target, form) = match (&self.target, &self.form) {
            (Some(t), Some(f)) => (t.clone(), f),
            _ => return Err("no testbench is open".into()),
        };
        let text = form.to_text().inspect_err(|e| self.problem = Some(e.clone()))?;
        let file = Self::path(project, &target).ok_or("the view is gone")?;
        std::fs::write(&file, format!("{}{}", self.header, text)).map_err(|e| format!("{}: {e}", file.display()))?;
        self.saved = text;
        self.problem = None;
        tracing::info!(target: "cdept::sim", "saved {}", file.display());
        Ok(())
    }

    /// Back to what the file says.
    pub(crate) fn revert(&mut self, project: &Project) {
        if let Some(target) = self.target.clone() {
            let (results, run_error) = (std::mem::take(&mut self.results), self.run_error.take());
            self.open(project, target, false);
            self.results = results;
            self.run_error = run_error;
        }
    }

    fn queue(&mut self) -> Arc<Queue> {
        let egui = self.egui.clone();
        self.queue
            .get_or_insert_with(|| {
                let queue = Queue::new(Queue::default_workers());
                queue.on_event(move |_| {
                    if let Some(ctx) = &egui {
                        ctx.request_repaint();
                    }
                });
                Arc::new(queue)
            })
            .clone()
    }

    /// Save, then run the testbench (corners `only`, or all of the setup) in the
    /// background.
    pub(crate) fn start(&mut self, project: &Project, only: Vec<String>) -> Result<(), String> {
        if self.state() == RunState::Running {
            return Err("a simulation is running".into());
        }
        if self.dirty() {
            self.save(project)?;
        }
        let target = self.target.clone().ok_or("no testbench is open")?;
        let setup = self.form.as_ref().ok_or("the setup is not valid")?.setup()?;
        let queue = self.queue();
        self.results.clear();
        self.run_error = None;
        self.only = only.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let done: Slot = Arc::default();
        self.run = Some(Run {
            first_job: queue.snapshot().len(),
            started: Instant::now(),
            cancel: cancel.clone(),
            done: done.clone(),
            cancelled: false,
            finished: false,
        });
        let project = project.clone();
        let egui = self.egui.clone();
        std::thread::spawn(move || {
            let result = run_target(&project, &target, &setup, &only, queue, cancel);
            *done.lock().unwrap() = Some(result);
            if let Some(ctx) = egui {
                ctx.request_repaint();
            }
        });
        Ok(())
    }

    pub(crate) fn cancel(&mut self) {
        if let (Some(run), Some(queue)) = (&mut self.run, &self.queue)
            && !run.finished
        {
            run.cancelled = true;
            run.cancel.store(true, Ordering::Relaxed);
            for job in queue.snapshot().iter().skip(run.first_job) {
                if !job.state.is_final() {
                    queue.cancel(job.id);
                }
            }
            tracing::info!(target: "cdept::sim", "simulation cancelled");
        }
    }

    /// The jobs of the present run.
    pub(crate) fn jobs(&self) -> Vec<JobInfo> {
        match (&self.run, &self.queue) {
            (Some(run), Some(queue)) => queue.snapshot().into_iter().skip(run.first_job).collect(),
            _ => Vec::new(),
        }
    }

    /// Take the outcome of a run that ended.
    pub(crate) fn poll(&mut self) {
        let Some(run) = &mut self.run else { return };
        if run.finished {
            return;
        }
        let outcome = run.done.lock().unwrap().take();
        let Some(outcome) = outcome else { return };
        run.finished = true;
        match outcome {
            Ok(rows) => {
                let (pass, bad) = tally(&rows);
                tracing::info!(target: "cdept::sim", "simulation done: {pass} spec(s) pass, {bad} do not");
                self.results = rows;
            }
            Err(e) => {
                tracing::error!(target: "cdept::sim", "simulation failed: {e}");
                self.run_error = Some(e);
            }
        }
    }

    /// Run when the load asked for it.
    fn after_load(&mut self, project: &Project) {
        if self.run_when_loaded && self.form.is_some() {
            self.run_when_loaded = false;
            if let Err(e) = self.start(project, Vec::new()) {
                self.run_error = Some(e);
            }
        }
    }
}

fn tally(rows: &[PointRow]) -> (usize, usize) {
    let mut pass = 0;
    let mut bad = 0;
    for r in rows {
        bad += r.error.is_some() as usize;
        for s in &r.specs {
            if s.verdict == Verdict::Pass { pass += 1 } else { bad += 1 }
        }
    }
    (pass, bad)
}

/// The thread of a run: netlist, queue the points, wait, collect.
fn run_target(
    project: &Project,
    target: &Target,
    setup: &Setup,
    only: &[String],
    queue: Arc<Queue>,
    cancel: Arc<AtomicBool>,
) -> Result<Vec<PointRow>, String> {
    let tb = sim::open(project, &target.library, &target.cell, &target.view).map_err(|e| format!("{e:#}"))?;
    // What the form says, which is what was just saved.
    let tb = sim::Testbench { setup: setup.clone(), ..tb };
    let simulator = Simulator::for_setup(&tb.setup).ok_or_else(|| {
        format!(
            "the simulator '{}' was not found: put it on PATH or set CDEPT_NGSPICE / CDEPT_XYCE",
            tb.setup.simulator
        )
    })?;
    let options = RunOptions { queue: Some(queue), cancel: Some(cancel), ..Default::default() };
    let results = sim::run_testbench_with(project, &tb, &simulator, only, &options).map_err(|e| format!("{e:#}"))?;
    let base = sim::runs_dir(project, &target.library, &target.cell, &target.view);
    Ok(results
        .into_iter()
        .map(|r| PointRow {
            name: r.point.name(),
            dir: base.join(r.point.name()),
            error: r.outcome.as_ref().err().map(|e| format!("{e:#}")),
            measures: r.outcome.as_ref().map(|o| o.measures.iter().map(|(k, v)| (k.clone(), *v)).collect()).unwrap_or_default(),
            specs: r.specs,
        })
        .collect())
}

// --- drawing -------------------------------------------------------------------------

fn verdict_color(ui: &Ui, v: Verdict) -> Color32 {
    let dark = ui.visuals().dark_mode;
    match (v, dark) {
        (Verdict::Pass, true) => Color32::from_rgb(110, 200, 110),
        (Verdict::Pass, false) => Color32::from_rgb(20, 130, 40),
        (Verdict::Fail, true) => Color32::from_rgb(240, 100, 100),
        (Verdict::Fail, false) => Color32::from_rgb(190, 30, 30),
        (Verdict::Missing, true) => Color32::from_rgb(230, 170, 60),
        (Verdict::Missing, false) => Color32::from_rgb(170, 100, 0),
    }
}

fn verdict_word(v: Verdict) -> &'static str {
    match v {
        Verdict::Pass => "PASS",
        Verdict::Fail => "FAIL",
        Verdict::Missing => "MISSING",
    }
}

pub(crate) fn verdict_of(word: &str) -> Option<Verdict> {
    [Verdict::Pass, Verdict::Fail, Verdict::Missing]
        .into_iter()
        .find(|v| verdict_word(*v).eq_ignore_ascii_case(word))
}

fn state_color(ui: &Ui, s: JobState) -> Color32 {
    match s {
        JobState::Done => verdict_color(ui, Verdict::Pass),
        JobState::Failed => verdict_color(ui, Verdict::Fail),
        JobState::Cancelled => verdict_color(ui, Verdict::Missing),
        _ => ui.visuals().text_color(),
    }
}

fn state_word(s: JobState) -> &'static str {
    match s {
        JobState::Queued => "queued",
        JobState::Running => "running",
        JobState::Done => "done",
        JobState::Failed => "failed",
        JobState::Cancelled => "cancelled",
    }
}

/// The last `n` lines of a file.
fn tail(file: &Path, n: usize) -> Vec<String> {
    let text = std::fs::read_to_string(file).unwrap_or_default();
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(n)..].iter().map(|l| l.to_string()).collect()
}

fn engineering(v: f64) -> String {
    format!("{v:.4e}")
}

fn bound(spec: &sim::Spec) -> String {
    match (spec.min, spec.max) {
        (Some(a), Some(b)) => format!("{} … {}", engineering(a), engineering(b)),
        (Some(a), None) => format!("≥ {}", engineering(a)),
        (None, Some(b)) => format!("≤ {}", engineering(b)),
        (None, None) => String::new(),
    }
}

fn list_editor(ui: &mut Ui, id: &str, rows: &mut Vec<(String, String)>, names: (&str, &str)) {
    let mut remove = None;
    Grid::new(id).num_columns(3).show(ui, |ui| {
        for (i, (n, v)) in rows.iter_mut().enumerate() {
            ui.add(TextEdit::singleline(n).desired_width(120.0).hint_text(names.0));
            ui.add(TextEdit::singleline(v).desired_width(160.0).hint_text(names.1));
            if ui.small_button("✖").on_hover_text("Remove").clicked() {
                remove = Some(i);
            }
            ui.end_row();
        }
    });
    if let Some(i) = remove {
        rows.remove(i);
    }
    if ui.small_button("＋ Add").clicked() {
        rows.push((String::new(), String::new()));
    }
}

impl SimulationView {
    fn editor(&mut self, ui: &mut Ui, project: &Project) {
        let Some(form) = &mut self.form else { return };
        let tech = project.tech.as_ref().map(|t| &t.models);
        let set_name = if form.models.is_empty() {
            tech.and_then(|m| m.default.clone()).unwrap_or_default()
        } else {
            form.models.clone()
        };
        let available: Vec<String> = tech
            .and_then(|m| m.sets.get(&set_name))
            .map(|s| s.corners.keys().cloned().collect())
            .unwrap_or_default();
        let sets: Vec<String> = tech.map(|m| m.sets.keys().cloned().collect()).unwrap_or_default();

        Grid::new("sim-general").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
            ui.label("Simulator");
            ComboBox::from_id_salt("sim-simulator").selected_text(&form.simulator).show_ui(ui, |ui| {
                for name in ["ngspice", "xyce"] {
                    ui.selectable_value(&mut form.simulator, name.to_string(), name);
                }
            });
            ui.end_row();
            ui.label("Model set");
            ComboBox::from_id_salt("sim-models")
                .selected_text(if form.models.is_empty() { format!("{set_name} (default)") } else { form.models.clone() })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut form.models, String::new(), "technology default");
                    for name in &sets {
                        ui.selectable_value(&mut form.models, name.clone(), name);
                    }
                });
            ui.end_row();
            ui.label("Schematic");
            ui.horizontal(|ui| {
                ui.add(TextEdit::singleline(&mut form.schematic).desired_width(160.0).hint_text("this cell"));
                ui.add(TextEdit::singleline(&mut form.schematic_view).desired_width(100.0).hint_text("view"));
            });
            ui.end_row();
            ui.label("Corners").on_hover_text("From the technology snapshot (tech.toml)");
            ui.horizontal_wrapped(|ui| {
                if available.is_empty() {
                    ui.weak("the technology snapshot has no corners for this model set");
                }
                for corner in &available {
                    let mut on = form.corners.contains(corner);
                    if ui.checkbox(&mut on, corner).changed() {
                        if on {
                            form.corners.push(corner.clone());
                            form.corners.sort_by_key(|c| available.iter().position(|a| a == c).unwrap_or(usize::MAX));
                        } else {
                            form.corners.retain(|c| c != corner);
                        }
                    }
                }
                // A corner the technology does not have: shown so it can be removed.
                let unknown: Vec<String> = form.corners.iter().filter(|c| !available.contains(c)).cloned().collect();
                for corner in unknown {
                    let mut on = true;
                    let text = RichText::new(format!("{corner} ?")).color(ui.visuals().warn_fg_color);
                    if ui.checkbox(&mut on, text).on_hover_text("Not a corner of this model set").changed() {
                        form.corners.retain(|c| c != &corner);
                    }
                }
            });
            ui.end_row();
            ui.label("Temperatures").on_hover_text("°C, one run each; empty: the simulator's own");
            ui.add(TextEdit::singleline(&mut form.temperatures).desired_width(200.0).hint_text("-40, 27, 125"));
            ui.end_row();
        });

        ui.add_space(4.0);
        ui.strong("Variables  (.param)");
        list_editor(ui, "sim-vars", &mut form.variables, ("name", "value"));
        ui.strong("Options  (.options)");
        list_editor(ui, "sim-opts", &mut form.options, ("name", "value"));

        ui.add_space(4.0);
        ui.strong("Analyses");
        let mut remove = None;
        for (i, a) in form.analyses.iter_mut().enumerate() {
            ui.horizontal_wrapped(|ui| {
                let before = a.kind.clone();
                ComboBox::from_id_salt(("sim-an", i)).width(70.0).selected_text(&a.kind).show_ui(ui, |ui| {
                    for kind in ANALYSIS_KINDS {
                        ui.selectable_value(&mut a.kind, kind.to_string(), kind);
                    }
                });
                if a.kind != before {
                    a.fields = analysis_fields(&a.kind).iter().map(|f| (f.to_string(), String::new())).collect();
                }
                for field in analysis_fields(&a.kind) {
                    let mut value = a.get(field).to_string();
                    ui.label(*field);
                    if ui.add(TextEdit::singleline(&mut value).desired_width(70.0)).changed() {
                        a.set(field, value);
                    }
                }
                if ui.small_button("✖").on_hover_text("Remove").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            form.analyses.remove(i);
        }
        if ui.small_button("＋ Add analysis").clicked() {
            form.analyses.push(AnalysisForm {
                kind: "tran".into(),
                fields: analysis_fields("tran").iter().map(|f| (f.to_string(), String::new())).collect(),
            });
        }

        ui.add_space(4.0);
        ui.strong("Measures  (.meas, one per line, without the .meas)");
        ui.add(TextEdit::multiline(&mut form.measures).desired_rows(2).desired_width(f32::INFINITY).code_editor());
        ui.strong("Save  (.save, one per line; empty: everything)");
        ui.add(TextEdit::multiline(&mut form.save).desired_rows(1).desired_width(f32::INFINITY).code_editor());

        ui.add_space(4.0);
        ui.strong("Specs");
        let mut remove = None;
        Grid::new("sim-specs").num_columns(4).show(ui, |ui| {
            for (i, (n, min, max)) in form.specs.iter_mut().enumerate() {
                ui.add(TextEdit::singleline(n).desired_width(120.0).hint_text("measure"));
                ui.add(TextEdit::singleline(min).desired_width(90.0).hint_text("min"));
                ui.add(TextEdit::singleline(max).desired_width(90.0).hint_text("max"));
                if ui.small_button("✖").on_hover_text("Remove").clicked() {
                    remove = Some(i);
                }
                ui.end_row();
            }
        });
        if let Some(i) = remove {
            form.specs.remove(i);
        }
        if ui.small_button("＋ Add spec").clicked() {
            form.specs.push((String::new(), String::new(), String::new()));
        }
    }

    fn jobs_ui(&mut self, ui: &mut Ui) {
        let jobs = self.jobs();
        if jobs.is_empty() {
            return;
        }
        ui.strong("Jobs");
        Grid::new("sim-jobs").striped(true).num_columns(5).show(ui, |ui| {
            for job in &jobs {
                ui.label(&job.name);
                ui.colored_label(state_color(ui, job.state), state_word(job.state));
                ui.add(egui::ProgressBar::new(if job.state.is_final() { 1.0 } else { job.progress }).desired_width(90.0));
                ui.label(format!("{:.1} s", job.elapsed.as_secs_f32()));
                let last = job.log.as_deref().map(|f| tail(f, 1)).and_then(|l| l.into_iter().next()).unwrap_or_default();
                let label = ui.label(RichText::new(last).weak().small());
                if let Some(log) = &job.log {
                    label.on_hover_text(tail(log, 12).join("\n"));
                }
                ui.end_row();
                if let Some(error) = &job.error {
                    ui.label("");
                    ui.colored_label(verdict_color(ui, Verdict::Fail), error.lines().next().unwrap_or(""));
                    ui.end_row();
                }
            }
        });
    }

    fn results_ui(&mut self, ui: &mut Ui, project: &Project) {
        if let Some(e) = &self.run_error {
            ui.colored_label(verdict_color(ui, Verdict::Fail), e.lines().next().unwrap_or(""));
        }
        if self.results.is_empty() {
            return;
        }
        let (pass, bad) = tally(&self.results);
        ui.strong("Results");
        ui.label(format!("{pass} spec(s) pass, {bad} fail or are missing"));
        let specs = self
            .form
            .as_ref()
            .and_then(|f| f.setup().ok())
            .map(|s| s.specs)
            .unwrap_or_default();
        let _ = project;
        for row in &self.results {
            ui.horizontal(|ui| {
                ui.strong(&row.name);
                if ui.small_button("Open folder").clicked() {
                    super::technology::open_outside(&row.dir);
                }
                if ui.small_button("Open log").clicked() {
                    let log = ["out.log", "job.log"].iter().map(|f| row.dir.join(f)).find(|f| f.is_file());
                    match log {
                        Some(log) => super::technology::open_outside(&log),
                        None => tracing::warn!(target: "cdept::sim", "{} has no log", row.name),
                    }
                }
            });
            if let Some(e) = &row.error {
                ui.colored_label(verdict_color(ui, Verdict::Fail), e.lines().take(6).collect::<Vec<_>>().join("\n"));
                continue;
            }
            Grid::new(("sim-res", &row.name)).striped(true).num_columns(4).show(ui, |ui| {
                for s in &row.specs {
                    ui.label(&s.name);
                    ui.label(s.value.map(engineering).unwrap_or_else(|| "—".into()));
                    ui.label(specs.iter().find(|x| x.name == s.name).map(bound).unwrap_or_default());
                    ui.label(RichText::new(verdict_word(s.verdict)).color(verdict_color(ui, s.verdict)).strong());
                    ui.end_row();
                }
                for (name, value) in &row.measures {
                    if !row.specs.iter().any(|s| s.name.eq_ignore_ascii_case(name)) {
                        ui.weak(name);
                        ui.weak(engineering(*value));
                        ui.weak("");
                        ui.weak("");
                        ui.end_row();
                    }
                }
            });
        }
    }
}

impl View for SimulationView {
    fn title(&self, _ctx: &AppCtx) -> String {
        match &self.target {
            Some(t) => format!("Simulation: {}/{}", t.cell, t.view),
            None => "Simulation".into(),
        }
    }

    fn ui(&mut self, ui: &mut Ui, ctx: &mut AppCtx) {
        self.egui.get_or_insert_with(|| ui.ctx().clone());
        let Some(project) = ctx.project.clone() else {
            empty_state(ui, "No project is open.", None);
            return;
        };
        self.poll();
        self.after_load(&project);
        let Some(target) = self.target.clone() else {
            empty_state(
                ui,
                "No testbench is open.",
                Some("Double-click a testbench view (one with a setup.toml) in the Library Manager."),
            );
            return;
        };
        let running = self.state() == RunState::Running;
        if running {
            ui.ctx().request_repaint_after(Duration::from_millis(250));
        }
        let dirty = self.dirty();
        let mut ask: Option<&str> = None;
        ui.horizontal(|ui| {
            ui.heading(target.text());
            if dirty {
                ui.weak("(changed)");
            }
        });
        ui.horizontal(|ui| {
            if ui.add_enabled(self.form.is_some() && dirty, egui::Button::new("Save")).clicked() {
                ask = Some("save");
            }
            if ui.add_enabled(dirty, egui::Button::new("Revert")).clicked() {
                ask = Some("revert");
            }
            ui.separator();
            if ui
                .add_enabled(self.form.is_some() && !running, egui::Button::new("▶ Run"))
                .on_hover_text("Saves the setup, netlists the schematic and runs every corner and temperature")
                .clicked()
            {
                ask = Some("run");
            }
            if ui.add_enabled(running, egui::Button::new("■ Cancel")).clicked() {
                ask = Some("cancel");
            }
            if running && let Some(run) = &self.run {
                ui.spinner();
                ui.label(format!("{:.0} s", run.started.elapsed().as_secs_f32()));
            } else if self.run.as_ref().is_some_and(|r| r.cancelled) {
                ui.weak("cancelled");
            }
        });
        match ask {
            Some("save") => {
                if let Err(e) = self.save(&project) {
                    self.problem = Some(e);
                }
            }
            Some("revert") => self.revert(&project),
            Some("run") => {
                if let Err(e) = self.start(&project, Vec::new()) {
                    self.problem = Some(e);
                }
            }
            Some("cancel") => self.cancel(),
            _ => {}
        }
        if let Some(problem) = &self.problem {
            ui.colored_label(verdict_color(ui, Verdict::Fail), problem.lines().next().unwrap_or(""));
        } else if let Some(form) = &self.form
            && let Err(e) = form.to_text()
        {
            ui.colored_label(verdict_color(ui, Verdict::Fail), e.lines().next().unwrap_or(""));
        }
        ui.separator();
        ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            if self.form.is_some() {
                egui::CollapsingHeader::new("Setup").default_open(true).show(ui, |ui| self.editor(ui, &project));
            }
            ui.add_space(6.0);
            self.jobs_ui(ui);
            ui.add_space(6.0);
            self.results_ui(ui, &project);
        });
    }
}

/// `sim …` of a test script (docs/GUI-TESTING.md).
pub(crate) fn script(
    view: &mut SimulationView,
    app: &mut AppCtx,
    args: &[String],
) -> Result<(), String> {
    let project = app.project.clone().ok_or("no project is open")?;
    view.poll();
    let arg = |i: usize| args.get(i).map(String::as_str).ok_or_else(|| "missing argument".to_string());
    let yes = |s: &str| matches!(s, "yes" | "on" | "true");
    fn form(view: &mut SimulationView) -> Result<&mut Form, String> {
        view.form.as_mut().ok_or_else(|| "no testbench is open".to_string())
    }
    match arg(0)? {
        "open" => {
            let selection = Selection::view(arg(1)?, arg(2)?, arg(3)?);
            let target = testbench_of(&project, &selection).ok_or("not a testbench view")?;
            view.open(&project, target, false);
            app.request(Action::OpenTab(crate::dock::Tab::Simulation));
            Ok(())
        }
        // The Library Manager's context menu: Simulation / Run Simulation.
        "menu_open" | "menu_run" => {
            let selection = match args.len() {
                3 => Selection::cell(arg(1)?, arg(2)?),
                _ => Selection::view(arg(1)?, arg(2)?, arg(3)?),
            };
            app.request(Action::OpenSimulation { selection, run: args[0] == "menu_run" });
            Ok(())
        }
        "save" => view.save(&project),
        "revert" => {
            view.revert(&project);
            Ok(())
        }
        "run" => view.start(&project, args[1..].to_vec()),
        "cancel" => {
            view.cancel();
            Ok(())
        }
        "wait" => match view.state() {
            RunState::Running => Err("still running".into()),
            _ => Ok(()),
        },
        "expect_state" => {
            let want = match arg(1)? {
                "idle" => RunState::Idle,
                "running" => RunState::Running,
                "finished" => RunState::Finished,
                other => return Err(format!("unknown state '{other}'")),
            };
            if view.state() == want { Ok(()) } else { Err(format!("the simulation is {:?}", view.state())) }
        }
        "expect_dirty" => {
            if view.dirty() == yes(arg(1)?) { Ok(()) } else { Err(format!("changed is {}", view.dirty())) }
        }
        "expect_problem" => {
            let wanted = args[1..].join(" ");
            match view.problem() {
                Some(p) if p.contains(&wanted) => Ok(()),
                other => Err(format!("problem is {other:?}")),
            }
        }
        "expect_no_problem" => match view.problem() {
            None => Ok(()),
            Some(p) => Err(format!("problem: {p}")),
        },
        "expect_target" => {
            let text = view.target().map(Target::text).unwrap_or_default();
            if text == arg(1)? { Ok(()) } else { Err(format!("the tab shows '{text}'")) }
        }
        // Fields of the form: simulator, models, schematic, corners (words),
        // temperatures (words).
        "set" => {
            let value = args[2.min(args.len())..].join(" ");
            let f = form(view)?;
            match arg(1)? {
                "simulator" => f.simulator = value,
                "models" => f.models = value,
                "schematic" => f.schematic = value,
                "corners" => f.corners = value.split_whitespace().map(String::from).collect(),
                "temperatures" => f.temperatures = value,
                "measures" => f.measures = value.replace(" ; ", "\n"),
                other => return Err(format!("unknown field '{other}'")),
            }
            Ok(())
        }
        "add_variable" => {
            form(view)?.variables.push((arg(1)?.into(), arg(2)?.into()));
            Ok(())
        }
        "add_spec" => {
            let f = form(view)?;
            f.specs.push((arg(1)?.into(), arg(2).unwrap_or("").replace('-', ""), arg(3).unwrap_or("").replace('-', "")));
            Ok(())
        }
        "set_spec" => {
            // set_spec <name> <min|max> <value>
            let f = form(view)?;
            let row = f.specs.iter_mut().find(|s| s.0 == arg(1).unwrap_or("")).ok_or("no such spec")?;
            match arg(2)? {
                "min" => row.1 = arg(3)?.into(),
                "max" => row.2 = arg(3)?.into(),
                other => return Err(format!("min or max, not '{other}'")),
            }
            Ok(())
        }
        // The corners the form offers (from tech.toml).
        "expect_corners" => {
            let tech = project.tech.as_ref().ok_or("no technology snapshot")?;
            let f = view.form.as_ref().ok_or("no testbench is open")?;
            let set = if f.models.is_empty() { tech.models.default.clone().unwrap_or_default() } else { f.models.clone() };
            let have: Vec<String> =
                tech.models.sets.get(&set).map(|s| s.corners.keys().cloned().collect()).unwrap_or_default();
            let want: Vec<&str> = args[1..].iter().map(String::as_str).collect();
            if want.iter().all(|w| have.iter().any(|h| h == w)) { Ok(()) } else { Err(format!("offered: {have:?}")) }
        }
        // expect_rows <n> <PASS|FAIL|MISSING>: spec rows of the results with a verdict.
        "expect_rows" => {
            let n: usize = arg(1)?.parse().map_err(|_| "not a number")?;
            let v = verdict_of(arg(2)?).ok_or("PASS, FAIL or MISSING")?;
            let have = view.results.iter().flat_map(|r| &r.specs).filter(|s| s.verdict == v).count();
            if have == n { Ok(()) } else { Err(format!("{have} {} rows, expected {n}", arg(2).unwrap_or(""))) }
        }
        // expect_points <n>: run points with results.
        "expect_points" => {
            let n: usize = arg(1)?.parse().map_err(|_| "not a number")?;
            if view.results.len() == n { Ok(()) } else { Err(format!("{} points", view.results.len())) }
        }
        // expect_jobs <n> [state]
        "expect_jobs" => {
            let n: usize = arg(1)?.parse().map_err(|_| "not a number")?;
            let jobs = view.jobs();
            let have = match args.get(2) {
                Some(s) => jobs.iter().filter(|j| state_word(j.state) == s).count(),
                None => jobs.len(),
            };
            if have == n { Ok(()) } else { Err(format!("{have} jobs ({jobs:?})")) }
        }
        // expect_file <point> <name>: a file of a run point's folder exists.
        "expect_file" => {
            let row = view.results.iter().find(|r| r.name == arg(1).unwrap_or("")).ok_or("no such point")?;
            let file = row.dir.join(arg(2)?);
            if file.is_file() { Ok(()) } else { Err(format!("{} is not there", file.display())) }
        }
        other => Err(format!("unknown sim command '{other}'")),
    }
}

/// Commands that are tried again for a while.
pub(crate) fn patient(args: &[String]) -> bool {
    matches!(
        args.first().map(String::as_str),
        Some("wait" | "expect_state" | "expect_rows" | "expect_points" | "expect_jobs" | "expect_file")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const SETUP: &str = r#"# a comment
corners = ["tt", "ff"]
temperatures = [27.0, -40.0]
save = ["v(out)"]
measures = ["tran tpd trig v(in) val=0.6 rise=1 targ v(out) val=0.6 fall=1"]

[variables]
vdd = "1.2"

[[analysis]]
kind = "tran"
step = "5p"
stop = "1.2n"

[[spec]]
name = "tpd"
max = 3e-10
"#;

    #[test]
    fn the_form_round_trips_and_validates() {
        let setup = Setup::parse(SETUP).unwrap();
        let mut form = Form::of(&setup).unwrap();
        assert_eq!(form.setup().unwrap(), setup);
        assert_eq!(form.temperatures, "27, -40");
        form.specs[0].2 = "x".into();
        assert!(form.to_text().unwrap_err().contains("not a number"));
        form.specs[0].2 = "1e-9".into();
        form.specs.push(("gain".into(), "40".into(), "30".into()));
        assert!(form.to_text().unwrap_err().contains("above max"), "min above max is the schema's");
        form.specs.pop();
        form.analyses.push(AnalysisForm { kind: "ac".into(), fields: vec![("points".into(), "10".into())] });
        form.analyses.pop();
        form.corners = vec!["ss".into()];
        form.simulator = "ngspice".into();
        let text = form.to_text().unwrap();
        let back = Setup::parse(&text).unwrap();
        assert_eq!(back.corners, ["ss"]);
        assert_eq!(back.specs[0].max, Some(1e-9));
        assert!(!text.contains("simulator"), "{text}");
        assert_eq!(header_comment(SETUP), "# a comment\n\n");
    }
}
