//! Test templates and suites in the Simulation tab (docs/TESTS.md §5).
//!
//! * A **template picker** above the setup form: pick a built-in, technology or project
//!   template, fill its parameters (documented, validated as you type); every change
//!   re-expands the template into the form. What the form adds to the template (other
//!   corners, a spec, a variable, ...) is kept as the setup's own fields; the file gets
//!   `template`, `[template_args]` and only those own fields ([`minimize`]).
//! * A **suite view**: a view with a `suite.toml` opens here as a table of its tests, a
//!   Run button (one shared job queue), and the combined results table.
//!
//! The Simulation tab's own code calls into this file at a few places only.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use egui::{Color32, ComboBox, Grid, RichText, ScrollArea, TextEdit, Ui};
use libcdept::Project;
use libcdept::jobs::{JobState, Queue};
use libcdept::sim::results::Overall;
use libcdept::sim::spec::Verdict;
use libcdept::sim::suite::{self, SuiteOptions, SuiteReport};
use libcdept::sim::template::TemplateSet;
use libcdept::sim::Setup;

use super::simulation::{Form, SimulationView, Target};
use crate::state::AppCtx;

// --- the project's templates for the form's validation ---------------------------------

thread_local! {
    static TEMPLATES: RefCell<Option<TemplateSet>> = const { RefCell::new(None) };
}

/// Remember the project's templates (the form is validated without a project at hand).
pub(super) fn use_project(project: &Project) {
    TEMPLATES.with(|t| *t.borrow_mut() = Some(TemplateSet::for_project(project)));
}

/// `Setup::parse` that knows the project's templates.
pub(super) fn parse(text: &str) -> eyre::Result<Setup> {
    TEMPLATES.with(|t| match &*t.borrow() {
        Some(set) => Setup::parse_with(text, set),
        None => Setup::parse(text),
    })
}

/// The templates the form validates with (tests).
#[cfg(test)]
pub(super) fn use_templates(set: TemplateSet) {
    TEMPLATES.with(|t| *t.borrow_mut() = Some(set));
}

fn set() -> TemplateSet {
    TEMPLATES.with(|t| t.borrow().clone()).unwrap_or_else(|| TemplateSet::builtin().clone())
}

// --- template text <-> form ------------------------------------------------------------

const OWN: [&str; 2] = ["template", "template_args"];

/// What a setup says on top of its template: the fields of `full` that differ from
/// `base` (the template alone), keyed arrays and maps entry by entry.
pub(crate) fn minimize(full: &mut toml::Table, base: &toml::Table) {
    use toml::Value;
    let keys: Vec<String> = full.keys().cloned().collect();
    for k in keys {
        if OWN.contains(&k.as_str()) {
            continue;
        }
        let Some(b) = base.get(&k) else { continue };
        let v = full.get(&k).cloned().unwrap_or(Value::Boolean(false));
        if &v == b {
            full.remove(&k);
            continue;
        }
        match (&v, b) {
            (Value::Array(va), Value::Array(ba)) if matches!(k.as_str(), "analysis" | "output" | "spec" | "sweep") => {
                let left: Vec<Value> = va.iter().filter(|e| !ba.contains(e)).cloned().collect();
                if left.is_empty() {
                    full.remove(&k);
                } else {
                    full.insert(k, Value::Array(left));
                }
            }
            // `[montecarlo]` of the setup replaces the template's as a whole: all or nothing.
            (Value::Table(vt), Value::Table(bt)) if k != "montecarlo" => {
                let left: toml::Table = vt.iter().filter(|(n, x)| bt.get(*n) != Some(*x)).map(|(n, x)| (n.clone(), x.clone())).collect();
                if left.is_empty() {
                    full.remove(&k);
                } else {
                    full.insert(k, Value::Table(left));
                }
            }
            _ => {}
        }
    }
}

type BaselineKey = (String, BTreeMap<String, String>);

thread_local! {
    static BASELINE: RefCell<Option<(BaselineKey, toml::Table)>> = const { RefCell::new(None) };
}

/// The template alone, as the table of the setup it gives (cached).
fn baseline(template: &str, args: &BTreeMap<String, String>) -> Option<toml::Table> {
    let key = (template.to_string(), args.clone());
    if let Some(t) = BASELINE.with(|b| b.borrow().as_ref().filter(|(k, _)| *k == key).map(|(_, t)| t.clone())) {
        return Some(t);
    }
    let text = compose(template, args, &toml::Table::new());
    let mut table = toml::Table::try_from(parse(&text).ok()?).ok()?;
    super::simulation::prune(&mut table);
    BASELINE.with(|b| *b.borrow_mut() = Some((key, table.clone())));
    Some(table)
}

/// Called by the form when it makes the file's text: write only what the template does
/// not already say.
pub(super) fn minimize_for(table: &mut toml::Table, template: &str, args: &BTreeMap<String, String>) {
    if template.is_empty() {
        return;
    }
    if let Some(base) = baseline(template, args) {
        minimize(table, &base);
    }
}

/// The text of a setup that picks `template` with `args` and adds `own` fields.
pub(crate) fn compose(template: &str, args: &BTreeMap<String, String>, own: &toml::Table) -> String {
    let mut t = own.clone();
    t.insert("template".into(), toml::Value::String(template.into()));
    if args.is_empty() {
        t.remove("template_args");
    } else {
        t.insert(
            "template_args".into(),
            toml::Value::Table(args.iter().map(|(k, v)| (k.clone(), toml::Value::String(v.clone()))).collect()),
        );
    }
    toml::to_string_pretty(&t).unwrap_or_default()
}

/// The fields a template change leaves alone.
const KEEP: [&str; 9] =
    ["schematic", "schematic_view", "schematic_kind", "simulator", "models", "corners", "temperatures", "options", "variables"];

// --- the template picker ---------------------------------------------------------------

/// What the user changed in the picker.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Change {
    /// Empty: no template (the expanded fields stay, as plain fields).
    pub template: String,
    pub args: BTreeMap<String, String>,
    /// The template itself changed: the form's own additions are dropped.
    pub switched: bool,
}

#[derive(Default)]
pub(super) struct TestsUi {
    /// The working copy of the arguments (may be invalid while typing).
    args: BTreeMap<String, String>,
    /// (template, args) the picker was last in step with.
    synced: Option<(String, BTreeMap<String, String>)>,
    pub suite: Option<SuiteState>,
}

fn pass_color(ui: &Ui) -> Color32 {
    if ui.visuals().dark_mode { Color32::from_rgb(110, 200, 110) } else { Color32::from_rgb(20, 130, 40) }
}
fn fail_color(ui: &Ui) -> Color32 {
    if ui.visuals().dark_mode { Color32::from_rgb(240, 100, 100) } else { Color32::from_rgb(190, 30, 30) }
}
fn warn_color(ui: &Ui) -> Color32 {
    if ui.visuals().dark_mode { Color32::from_rgb(230, 170, 60) } else { Color32::from_rgb(170, 100, 0) }
}

impl TestsUi {
    /// The picker and the parameter editor. `current` is what the form's setup has.
    pub(super) fn template_ui(
        &mut self,
        ui: &mut Ui,
        current: (&str, &BTreeMap<String, String>),
    ) -> Option<Change> {
        let (cur_name, cur_args) = current;
        let now = (cur_name.to_string(), cur_args.clone());
        if self.synced.as_ref() != Some(&now) {
            self.args = cur_args.clone();
            self.synced = Some(now);
        }
        let templates = set();
        let mut change = None;
        let mut chosen = cur_name.to_string();
        ui.horizontal(|ui| {
            ui.label("Template");
            ComboBox::from_id_salt("test-template")
                .selected_text(if chosen.is_empty() { "(none)".to_string() } else { chosen.clone() })
                .width(180.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut chosen, String::new(), "(none)");
                    for t in templates.list() {
                        ui.selectable_value(&mut chosen, t.name.clone(), &t.name).on_hover_text(&t.description);
                    }
                });
        });
        if chosen != cur_name {
            self.args.clear();
            return Some(Change { template: chosen, args: BTreeMap::new(), switched: true });
        }
        let Some(template) = templates.get(cur_name) else {
            if !cur_name.is_empty() {
                ui.colored_label(fail_color(ui), format!("no template '{cur_name}'"));
            }
            return None;
        };
        ui.label(RichText::new(&template.description).weak());
        let mut edited = false;
        Grid::new("test-template-params").num_columns(3).spacing([8.0, 4.0]).striped(true).show(ui, |ui| {
            for p in &template.params {
                let name = RichText::new(&p.name).strong();
                ui.label(name).on_hover_text(&p.doc);
                let mut value = self.args.get(&p.name).cloned().unwrap_or_default();
                let before = value.clone();
                let hint = p.default.clone().unwrap_or_else(|| if p.optional { "(unset)".into() } else { "required".into() });
                match p.kind.as_str() {
                    "choice" | "bool" => {
                        let options: Vec<String> =
                            if p.kind == "bool" { vec!["true".into(), "false".into()] } else { p.choices.clone() };
                        let shown = if value.is_empty() { format!("{hint} (default)") } else { value.clone() };
                        ComboBox::from_id_salt(("tp", &p.name)).selected_text(shown).show_ui(ui, |ui| {
                            ui.selectable_value(&mut value, String::new(), format!("{hint} (default)"));
                            for o in &options {
                                ui.selectable_value(&mut value, o.clone(), o);
                            }
                        });
                    }
                    _ => {
                        ui.add(TextEdit::singleline(&mut value).desired_width(150.0).hint_text(hint));
                    }
                }
                if value != before {
                    if value.trim().is_empty() {
                        self.args.remove(&p.name);
                    } else {
                        self.args.insert(p.name.clone(), value.trim().to_string());
                    }
                    edited = true;
                }
                match template.validate_param(&p.name, &self.args) {
                    Ok(()) => {
                        let unit = if p.unit.is_empty() { String::new() } else { format!("[{}]", p.unit) };
                        ui.label(RichText::new(unit).weak());
                    }
                    Err(e) => {
                        ui.colored_label(fail_color(ui), e.to_string());
                    }
                }
                ui.end_row();
            }
        });
        if edited && template.bind(&self.args).is_ok() {
            change = Some(Change { template: cur_name.to_string(), args: self.args.clone(), switched: false });
        } else if let Err(e) = template.bind(&self.args) {
            ui.colored_label(fail_color(ui), format!("{e:#}").lines().next().unwrap_or("").to_string());
        }
        if !template.analyses().is_empty() {
            ui.label(RichText::new(format!("analyses: {}", template.analyses().join(", "))).weak());
        }
        change
    }
}

impl SimulationView {
    /// Re-expand the template of the form after a picker change.
    pub(super) fn apply_template(&mut self, change: Change) -> Result<(), String> {
        let form = self.form.as_ref().ok_or("no testbench is open")?;
        let own_text = form.to_text()?;
        let mut own: toml::Table = toml::from_str(&own_text).map_err(|e| e.to_string())?;
        own.remove("template");
        own.remove("template_args");
        if change.template.is_empty() {
            // No template any more: what it gave stays, as plain fields.
            let setup = parse(&own_text).map_err(|e| format!("{e:#}"))?;
            let mut plain = setup;
            plain.template.clear();
            plain.template_args.clear();
            plain.requires_nets.clear();
            plain.requires_elements.clear();
            self.form = Some(Form::of(&plain)?);
            return Ok(());
        }
        // Own fields: those of the form (minimised already); a new template keeps only
        // the general ones.
        let mut keep = toml::Table::new();
        for (k, v) in own {
            if !change.switched || KEEP.contains(&k.as_str()) {
                keep.insert(k, v);
            }
        }
        let text = compose(&change.template, &change.args, &keep);
        let setup = parse(&text).map_err(|e| format!("{e:#}"))?;
        self.form = Some(Form::of(&setup)?);
        Ok(())
    }

    pub(super) fn egui_ctx(&self) -> Option<egui::Context> {
        self.egui.clone()
    }

    pub(super) fn open_target_for_suite(&mut self, project: &Project, target: Target) {
        self.open(project, target, false);
    }

    /// Open a suite in the tab.
    pub(super) fn open_suite(&mut self, project: &Project, target: &Target) {
        let mut state = SuiteState::new(target.clone());
        match suite::open(project, &target.library, &target.cell, &target.view) {
            Ok(s) => {
                state.description = s.description.clone();
                state.tests = s
                    .tests
                    .iter()
                    .map(|t| {
                        let (l, c, v) = t.resolve(&target.library, &target.cell);
                        format!("{l}/{c}/{v}")
                    })
                    .collect();
                state.report = suite::latest(project, &target.library, &target.cell, &target.view);
            }
            Err(e) => state.problem = Some(format!("{e:#}")),
        }
        self.tests.suite = Some(state);
    }
}

// --- the suite view -------------------------------------------------------------------

type Slot = Arc<Mutex<Option<Result<SuiteReport, String>>>>;

pub(super) struct SuiteState {
    pub target: Target,
    pub description: String,
    pub tests: Vec<String>,
    pub report: Option<SuiteReport>,
    pub problem: Option<String>,
    all_columns: bool,
    queue: Option<Arc<Queue>>,
    running: Option<(Instant, Arc<AtomicBool>, Slot)>,
    first_job: usize,
}

impl SuiteState {
    fn new(target: Target) -> SuiteState {
        SuiteState {
            target,
            description: String::new(),
            tests: vec![],
            report: None,
            problem: None,
            all_columns: false,
            queue: None,
            running: None,
            first_job: 0,
        }
    }

    pub(super) fn is_running(&self) -> bool {
        self.running.is_some()
    }

    /// Start the suite on a thread with the tab's own queue.
    pub(super) fn start(&mut self, project: &Project, egui: Option<egui::Context>) -> Result<(), String> {
        if self.running.is_some() {
            return Err("the suite is running".into());
        }
        let queue = self
            .queue
            .get_or_insert_with(|| {
                let q = Queue::new(Queue::default_workers());
                let egui = egui.clone();
                q.on_event(move |_| {
                    if let Some(c) = &egui {
                        c.request_repaint();
                    }
                });
                Arc::new(q)
            })
            .clone();
        self.first_job = queue.snapshot().len();
        let cancel = Arc::new(AtomicBool::new(false));
        let slot: Slot = Arc::default();
        self.running = Some((Instant::now(), cancel.clone(), slot.clone()));
        self.problem = None;
        let (project, target) = (project.clone(), self.target.clone());
        std::thread::spawn(move || {
            let options = SuiteOptions { queue: Some(queue), cancel: Some(cancel), ..Default::default() };
            let result = suite::run(&project, &target.library, &target.cell, &target.view, &options).map_err(|e| format!("{e:#}"));
            *slot.lock().unwrap() = Some(result);
            if let Some(c) = egui {
                c.request_repaint();
            }
        });
        Ok(())
    }

    pub(super) fn cancel(&mut self) {
        if let Some((_, cancel, _)) = &self.running {
            cancel.store(true, Ordering::Relaxed);
            if let Some(q) = &self.queue {
                q.cancel_all();
            }
        }
    }

    pub(super) fn poll(&mut self) {
        let Some((_, _, slot)) = &self.running else { return };
        let done = slot.lock().unwrap().take();
        match done {
            Some(Ok(report)) => {
                self.report = Some(report);
                self.running = None;
            }
            Some(Err(e)) => {
                self.problem = Some(e);
                self.running = None;
            }
            None => {}
        }
    }

    fn verdict(&self) -> Option<bool> {
        self.report.as_ref().map(|r| r.summary().ok())
    }

    fn ui(&mut self, ui: &mut Ui, project: &Project, egui: &mut Option<egui::Context>) {
        self.poll();
        let running = self.is_running();
        if running {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
        }
        ui.heading(format!("Suite {}/{}/{}", self.target.library, self.target.cell, self.target.view));
        if !self.description.is_empty() {
            ui.label(RichText::new(&self.description).weak());
        }
        ui.horizontal(|ui| {
            if ui.add_enabled(!running, egui::Button::new("▶ Run suite")).on_hover_text("Runs every test of the suite on one job queue").clicked() {
                let ctx = egui.clone().or_else(|| Some(ui.ctx().clone()));
                if let Err(e) = self.start(project, ctx) {
                    self.problem = Some(e);
                }
            }
            if ui.add_enabled(running, egui::Button::new("■ Cancel")).clicked() {
                self.cancel();
            }
            ui.checkbox(&mut self.all_columns, "all measurements");
            if let Some((t, _, _)) = &self.running {
                ui.spinner();
                ui.label(format!("{:.0} s", t.elapsed().as_secs_f32()));
            }
        });
        if let Some(p) = &self.problem {
            ui.colored_label(fail_color(ui), p.lines().next().unwrap_or("").to_string());
        }
        if running && let Some(q) = &self.queue {
            let jobs: Vec<_> = q.snapshot().into_iter().skip(self.first_job).collect();
            let done = jobs.iter().filter(|j| j.state.is_final()).count();
            let failed = jobs.iter().filter(|j| j.state == JobState::Failed).count();
            ui.label(format!("{done} of {} runs finished, {failed} failed", jobs.len()));
        }
        ui.separator();
        ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
            egui::CollapsingHeader::new(format!("Tests ({})", self.tests.len())).default_open(self.report.is_none()).show(ui, |ui| {
                for t in &self.tests {
                    ui.label(t);
                }
            });
            if let Some(report) = &self.report {
                ui.add_space(6.0);
                let s = report.summary();
                let (word, color) = if s.ok() { ("SUITE PASS", pass_color(ui)) } else { ("SUITE FAIL", fail_color(ui)) };
                ui.horizontal(|ui| {
                    ui.label(RichText::new(word).strong().color(color));
                    ui.label(format!(
                        "{} test(s), {} run(s): {} pass, {} fail, {} error; specs: {} failing, {} missing{}",
                        s.tests,
                        s.runs,
                        s.pass,
                        s.fail,
                        s.error,
                        s.spec_fail,
                        s.spec_missing,
                        if s.test_errors > 0 { format!("; {} test(s) did not run", s.test_errors) } else { String::new() }
                    ));
                });
                let cols = suite::columns(report, self.all_columns);
                Grid::new("suite-table").num_columns(3 + cols.len()).striped(true).spacing([10.0, 3.0]).show(ui, |ui| {
                    ui.label(RichText::new("test").strong());
                    ui.label(RichText::new("point").strong());
                    ui.label(RichText::new("result").strong());
                    for c in &cols {
                        ui.label(RichText::new(c).strong());
                    }
                    ui.end_row();
                    for t in &report.tests {
                        if let Some(e) = &t.error {
                            ui.label(&t.label);
                            ui.label("-");
                            ui.colored_label(fail_color(ui), "ERROR").on_hover_text(e);
                            ui.label(RichText::new(e.lines().next().unwrap_or("")).weak());
                            ui.end_row();
                            continue;
                        }
                        for r in &t.records {
                            ui.label(&t.label);
                            ui.label(&r.point.name);
                            let (word, color) = match r.overall() {
                                Overall::Pass => ("PASS", pass_color(ui)),
                                Overall::Fail => ("FAIL", fail_color(ui)),
                                Overall::Error => ("ERROR", warn_color(ui)),
                            };
                            ui.colored_label(color, word);
                            for c in &cols {
                                match r.measures.get(c) {
                                    Some(v) => {
                                        let bad = r.specs.iter().any(|s| s.name.eq_ignore_ascii_case(c) && s.verdict != Verdict::Pass);
                                        let text = RichText::new(format!("{v:.4e}"));
                                        ui.label(if bad { text.color(fail_color(ui)) } else { text });
                                    }
                                    None => {
                                        let missing = r.specs.iter().any(|s| s.name.eq_ignore_ascii_case(c));
                                        ui.label(RichText::new("-").color(if missing { warn_color(ui) } else { ui.visuals().weak_text_color() }));
                                    }
                                }
                            }
                            ui.end_row();
                        }
                    }
                });
                // what did not pass
                let mut bad = vec![];
                for t in &report.tests {
                    for r in &t.records {
                        for sp in r.specs.iter().filter(|s| s.verdict != Verdict::Pass) {
                            bad.push(format!(
                                "{} / {}: {} {} = {}",
                                t.label,
                                r.point.name,
                                sp.name,
                                if sp.verdict == Verdict::Fail { "FAIL" } else { "MISSING" },
                                sp.value.map_or("-".into(), |v| format!("{v:.4e}"))
                            ));
                        }
                    }
                }
                if !bad.is_empty() {
                    ui.add_space(6.0);
                    ui.label(RichText::new("Not passing").strong());
                    for b in bad {
                        ui.colored_label(fail_color(ui), b);
                    }
                }
            } else if !running {
                ui.label(RichText::new("No result yet: Run suite.").weak());
            }
        });
    }
}

impl SimulationView {
    /// Is the tab showing a suite?
    pub(super) fn suite_active(&self) -> bool {
        self.tests.suite.is_some() && self.target().is_some_and(|t| self.tests.suite.as_ref().is_some_and(|s| &s.target == t))
    }

    /// The suite panel, in place of the testbench editor.
    pub(super) fn suite_ui(&mut self, ui: &mut Ui, project: &Project) {
        let mut egui = self.egui_ctx();
        if let Some(s) = &mut self.tests.suite {
            s.ui(ui, project, &mut egui);
        }
    }
}

/// Is `target` a suite view (a `suite.toml` in it)?
pub(crate) fn is_suite(project: &Project, library: &str, cell: &str, view: &str) -> bool {
    project
        .library(library)
        .and_then(|l| l.cell(cell))
        .and_then(|c| c.entry(view))
        .is_some_and(is_suite_entry)
}

pub(crate) fn is_suite_entry(entry: &libcdept::Entry) -> bool {
    entry.path.join(suite::SUITE_FILE).is_file()
}

// --- test scripts (docs/GUI-TESTING.md) ----------------------------------------------

/// The `sim` commands of this file: `suite_*` and `template_*`.
pub(crate) fn is_command(word: &str) -> bool {
    word.starts_with("suite_") || word.starts_with("template_")
}

pub(super) fn script(view: &mut SimulationView, app: &mut AppCtx, args: &[String]) -> Result<(), String> {
    let project = app.project.clone().ok_or("no project is open")?;
    let arg = |i: usize| args.get(i).map(String::as_str).ok_or_else(|| "missing argument".to_string());
    match arg(0)? {
        // suite_open <library> <cell> <view>
        "suite_open" => {
            let target = Target { library: arg(1)?.into(), cell: arg(2)?.into(), view: arg(3)?.into() };
            if !is_suite(&project, &target.library, &target.cell, &target.view) {
                return Err("not a suite view".into());
            }
            view.open_target_for_suite(&project, target);
            app.request(crate::state::Action::OpenTab(crate::dock::Tab::Simulation));
            Ok(())
        }
        "suite_run" => {
            let ctx = view.egui_ctx();
            view.tests.suite.as_mut().ok_or("no suite is open")?.start(&project, ctx)
        }
        "suite_wait" => {
            let s = view.tests.suite.as_mut().ok_or("no suite is open")?;
            s.poll();
            if s.is_running() { Err("still running".into()) } else { Ok(()) }
        }
        // suite_expect pass|fail
        "suite_expect" => {
            let s = view.tests.suite.as_mut().ok_or("no suite is open")?;
            s.poll();
            let want = match arg(1)? {
                "pass" => true,
                "fail" => false,
                other => return Err(format!("expected pass or fail, not '{other}'")),
            };
            match s.verdict() {
                Some(v) if v == want => Ok(()),
                other => Err(format!("the suite verdict is {other:?} (problem: {:?})", s.problem)),
            }
        }
        // suite_expect_rows <n>: runs in the combined table
        "suite_expect_rows" => {
            let s = view.tests.suite.as_ref().ok_or("no suite is open")?;
            let n = s.report.as_ref().map_or(0, |r| r.summary().runs);
            if n.to_string() == arg(1)? { Ok(()) } else { Err(format!("{n} run(s) in the table")) }
        }
        // template_set <name|none> [k=v …]
        "template_set" => {
            let name = if arg(1)? == "none" { String::new() } else { arg(1)?.to_string() };
            let mut args_map = BTreeMap::new();
            for kv in &args[2..] {
                let (k, v) = kv.split_once('=').ok_or("arguments are k=v")?;
                args_map.insert(k.to_string(), v.to_string());
            }
            let templates = set();
            if !name.is_empty() {
                templates.find(&name).map_err(|e| format!("{e:#}"))?;
            }
            view.apply_template(Change { template: name, args: args_map, switched: true })
        }
        // template_arg <k> <v>: one parameter of the form's template
        "template_arg" => {
            let (name, mut args_map) = view.template_state().ok_or("the setup has no template")?;
            if arg(2)? == "-" {
                args_map.remove(arg(1)?);
            } else {
                args_map.insert(arg(1)?.to_string(), arg(2)?.to_string());
            }
            let templates = set();
            let t = templates.get(&name).ok_or("unknown template")?;
            if let Some(p) = t.param(arg(1)?) {
                // A bad value is the tab's problem (as in the picker), not a script failure.
                if let Err(e) = t.validate_param(&p.name, &args_map) {
                    view.problem = Some(format!("{}: {e:#}", p.name));
                    return Ok(());
                }
            } else {
                view.problem = Some(format!("template '{name}' has no parameter '{}'", arg(1)?));
                return Ok(());
            }
            view.apply_template(Change { template: name, args: args_map, switched: false })
        }
        // template_expect <name|none> / template_expect_output <name> / template_expect_arg k v
        "template_expect" => {
            let (name, _) = view.template_state().unwrap_or_default();
            let want = if arg(1)? == "none" { "" } else { arg(1)? };
            if name == want { Ok(()) } else { Err(format!("the template is '{name}'")) }
        }
        "template_expect_output" => {
            let f = view.form.as_ref().ok_or("no testbench is open")?;
            if f.base.outputs.iter().any(|o| o.name == arg(1).unwrap_or("")) { Ok(()) } else { Err("no such output".into()) }
        }
        "template_expect_spec" => {
            let f = view.form.as_ref().ok_or("no testbench is open")?;
            if f.base.specs.iter().any(|o| o.name == arg(1).unwrap_or("")) { Ok(()) } else { Err("no such spec".into()) }
        }
        // template_expect_file <substring>: the text that would be saved
        "template_expect_file" => {
            let text = view.form.as_ref().ok_or("no testbench is open")?.to_text()?;
            let want = args[1..].join(" ");
            if text.contains(&want) { Ok(()) } else { Err(format!("the file would be:\n{text}")) }
        }
        "template_expect_not_file" => {
            let text = view.form.as_ref().ok_or("no testbench is open")?.to_text()?;
            let want = args[1..].join(" ");
            if !text.contains(&want) { Ok(()) } else { Err(format!("the file would be:\n{text}")) }
        }
        other => Err(format!("unknown command '{other}'")),
    }
}

impl SimulationView {
    /// The template of the form and its arguments.
    pub(super) fn template_state(&self) -> Option<(String, BTreeMap<String, String>)> {
        let f = self.form.as_ref()?;
        (!f.base.template.is_empty()).then(|| (f.base.template.clone(), f.base.template_args.clone()))
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn table(text: &str) -> toml::Table {
        toml::from_str(text).unwrap()
    }

    #[test]
    fn minimize_keeps_only_what_the_template_does_not_say() {
        let base = table("temperatures=[27]\n[variables]\nvdd=\"1.2\"\n[[output]]\nname=\"a\"\nexpr=\"1\"\n[[spec]]\nname=\"a\"\nmax=1.0");
        let mut full = table(
            "template=\"t\"\ntemperatures=[27]\ncorners=[\"tt\",\"ss\"]\n[variables]\nvdd=\"1.2\"\nextra=\"3\"\n\
             [[output]]\nname=\"a\"\nexpr=\"1\"\n[[output]]\nname=\"b\"\nexpr=\"2\"\n[[spec]]\nname=\"a\"\nmax=1.0",
        );
        minimize(&mut full, &base);
        assert!(!full.contains_key("temperatures") && !full.contains_key("spec"));
        assert_eq!(full["corners"].as_array().unwrap().len(), 2);
        assert_eq!(full["variables"].as_table().unwrap().len(), 1);
        assert_eq!(full["output"].as_array().unwrap().len(), 1);
        assert!(full.contains_key("template"));
    }

    #[test]
    fn compose_writes_the_template_and_its_arguments() {
        let args = BTreeMap::from([("out".to_string(), "y".to_string())]);
        let text = compose("tran_delay", &args, &table("corners=[\"tt\"]"));
        let setup = Setup::parse(&text).unwrap();
        assert_eq!((setup.template.as_str(), setup.corners.as_slice()), ("tran_delay", ["tt".to_string()].as_slice()));
        assert!(setup.outputs[0].expr.contains("v(\"y\")"), "{:?}", setup.outputs[0].expr);
    }
}
