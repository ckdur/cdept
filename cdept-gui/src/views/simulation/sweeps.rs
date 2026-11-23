//! Sweeps and Monte Carlo in the Simulation tab's form (SYMBI-77).
//!
//! The model (what the form holds, its validation, the count of run points, what a
//! template brings) is plain code with unit tests; the drawing and the `sim …` script
//! verbs at the end only call it.
//!
//! * A sweep row is a variable and one text of values: `1.08, 1.2, 1.32` (SPICE numbers,
//!   commas or spaces) or a range `start:stop:step` (the schema's `start`/`stop`/`step`).
//! * A template can declare `[[sweep]]` and `[montecarlo]` (docs/TESTS.md). The form
//!   shows them as inherited and read-only; its own rows are the setup's own tables,
//!   which win (a sweep of the same variable replaces the template's, `[montecarlo]`
//!   replaces it as a whole). Only the own ones are written to the file.

use std::collections::BTreeMap;

use egui::{Color32, ComboBox, RichText, TextEdit, Ui};
use libcdept::Project;
use libcdept::calc::lang::parse_spice_number;
use libcdept::sim::setup::{McMode, MonteCarlo, Sweep, fmt_num};
use libcdept::sim::{self, Setup};

use super::{Form, SimulationView};

/// More run points than this get a warning.
pub(super) const MANY_POINTS: usize = 500;

// --- sweep rows ----------------------------------------------------------------------

/// One `[[sweep]]` of the form.
#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct SweepForm {
    pub variable: String,
    /// `1, 1.2, 1.4` or `start:stop:step`.
    pub values: String,
}

fn words(text: &str) -> Vec<&str> {
    text.split([',', ' ', '\t']).filter(|w| !w.is_empty()).collect()
}

impl SweepForm {
    pub(super) fn new(variable: &str, values: &str) -> SweepForm {
        SweepForm { variable: variable.into(), values: values.into() }
    }

    pub(super) fn of(s: &Sweep) -> SweepForm {
        let values = if !s.values.is_empty() {
            s.values.join(", ")
        } else if let (Some(a), Some(b), Some(c)) = (s.start, s.stop, s.step) {
            format!("{}:{}:{}", fmt_num(a), fmt_num(b), fmt_num(c))
        } else {
            String::new()
        };
        SweepForm { variable: s.variable.clone(), values }
    }

    /// The schema's sweep this row stands for, or what is wrong with it (the messages of
    /// `Setup::parse`, and "is not a number" for each value and range bound).
    pub(super) fn to_sweep(&self) -> Result<Sweep, String> {
        let variable = self.variable.trim().to_string();
        let text = self.values.trim();
        let mut s = Sweep { variable: variable.clone(), ..Default::default() };
        if text.contains(':') {
            let parts: Vec<&str> = text.split(':').map(str::trim).collect();
            if parts.len() != 3 {
                return Err(format!("sweep '{variable}': a range is start:stop:step"));
            }
            let mut bounds = [0.0; 3];
            for (i, (name, part)) in ["start", "stop", "step"].iter().zip(&parts).enumerate() {
                bounds[i] = part
                    .parse::<f64>()
                    .map_err(|_| format!("sweep '{variable}': {name} '{part}' is not a number"))?;
            }
            (s.start, s.stop, s.step) = (Some(bounds[0]), Some(bounds[1]), Some(bounds[2]));
        } else {
            for w in words(text) {
                if parse_spice_number(w).is_none() {
                    return Err(format!("sweep '{variable}': value '{w}' is not a number"));
                }
                s.values.push(w.to_string());
            }
        }
        // The schema's own checks: values or a range, a usable variable name.
        s.expand().map_err(|e| format!("{e:#}"))?;
        if variable.is_empty() || variable.contains(char::is_whitespace) {
            return Err(format!("'{variable}' is not a valid sweep variable"));
        }
        Ok(s)
    }

    /// How many values the row has (0 when it is not valid).
    pub(super) fn count(&self) -> usize {
        self.to_sweep().and_then(|s| s.expand().map_err(|e| e.to_string())).map_or(0, |v| v.len())
    }
}

/// The own sweeps of a form, validated: each row and no variable twice.
pub(super) fn own_sweeps(rows: &[SweepForm]) -> Result<Vec<Sweep>, String> {
    let mut out: Vec<Sweep> = Vec::new();
    for row in rows {
        let s = row.to_sweep()?;
        if out.iter().any(|o| o.variable == s.variable) {
            return Err(format!("variable '{}' is swept twice", s.variable));
        }
        out.push(s);
    }
    Ok(out)
}

/// What is wrong with row `i` (for the red text under it).
pub(super) fn row_problem(rows: &[SweepForm], i: usize) -> Option<String> {
    if let Err(e) = rows[i].to_sweep() {
        return Some(e);
    }
    let v = rows[i].variable.trim();
    rows[..i].iter().any(|o| o.variable.trim() == v).then(|| format!("variable '{v}' is swept twice"))
}

/// The sweeps a setup runs: the template's with the own ones laid over (a sweep of the
/// same variable replaces, the others are added), as the template overlay does.
pub(super) fn overlay(inherited: &[Sweep], own: &[Sweep]) -> Vec<Sweep> {
    let mut out = inherited.to_vec();
    for s in own {
        match out.iter().position(|o| o.variable.eq_ignore_ascii_case(&s.variable)) {
            Some(i) => out[i] = s.clone(),
            None => out.push(s.clone()),
        }
    }
    out
}

// --- what the template brings ----------------------------------------------------------

/// The sweeps and Monte Carlo the template alone declares (with its parameters).
#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct Inherited {
    pub template: String,
    pub sweeps: Vec<Sweep>,
    pub mc: Option<MonteCarlo>,
}

pub(super) fn inherited(template: &str, args: &BTreeMap<String, String>) -> Inherited {
    if template.is_empty() {
        return Inherited::default();
    }
    let text = crate::views::testlib::compose(template, args, &toml::Table::new());
    match crate::views::testlib::parse(&text) {
        Ok(s) => Inherited { template: template.to_string(), sweeps: s.sweeps, mc: s.montecarlo },
        Err(_) => Inherited { template: template.to_string(), ..Default::default() },
    }
}

impl Inherited {
    /// Is the template's sweep of this variable replaced by an own one?
    pub(super) fn overridden(&self, own: &[SweepForm], variable: &str) -> bool {
        own.iter().any(|o| o.variable.trim().eq_ignore_ascii_case(variable))
    }
}

/// The rows of `sweeps` that the template does not already declare.
pub(super) fn own_rows(sweeps: &[Sweep], inherited: &Inherited) -> Vec<SweepForm> {
    sweeps.iter().filter(|s| !inherited.sweeps.contains(s)).map(SweepForm::of).collect()
}

/// A sweep as the file writes it.
pub(super) fn sweep_value(s: &Sweep) -> toml::Value {
    let mut t = toml::Table::try_from(s).unwrap_or_default();
    super::prune(&mut t);
    toml::Value::Table(t)
}

// --- how many run points ---------------------------------------------------------------

/// The runs a setup makes: corners x temperatures x sweep points, or with Monte Carlo
/// temperatures x sweep points x runs (the corner is the statistical one).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RunPlan {
    pub corners: usize,
    pub temperatures: usize,
    pub sweep_points: usize,
    pub mc_runs: Option<usize>,
    pub total: usize,
}

impl RunPlan {
    pub(super) fn of(setup: &Setup) -> RunPlan {
        let runs = setup.montecarlo.as_ref().map(|m| m.runs as usize);
        Self::with(setup, runs)
    }

    /// The grid without Monte Carlo (the Grid button).
    pub(super) fn grid(setup: &Setup) -> RunPlan {
        Self::with(setup, None)
    }

    /// Monte Carlo with `runs` runs whatever the setup says (the Run N MC button).
    pub(super) fn with_runs(setup: &Setup, runs: usize) -> RunPlan {
        Self::with(setup, Some(runs))
    }

    fn with(setup: &Setup, mc_runs: Option<usize>) -> RunPlan {
        let corners = setup.corners.len().max(1);
        let temperatures = setup.temperatures.len().max(1);
        let sweep_points = setup
            .sweeps
            .iter()
            .map(|s| s.expand().map_or(0, |v| v.len()))
            .fold(1usize, usize::saturating_mul);
        let total = match mc_runs {
            Some(r) => temperatures.saturating_mul(sweep_points).saturating_mul(r),
            None => corners.saturating_mul(temperatures).saturating_mul(sweep_points),
        };
        RunPlan { corners, temperatures, sweep_points, mc_runs, total }
    }

    pub(super) fn many(&self) -> bool {
        self.total > MANY_POINTS
    }

    pub(super) fn describe(&self) -> String {
        let plural = |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
        let t = plural(self.temperatures, "temperature", "temperatures");
        let s = plural(self.sweep_points, "sweep point", "sweep points");
        match self.mc_runs {
            Some(r) => format!(
                "{} run points = {t} × {s} × {} (Monte Carlo: the corner is the statistical one)",
                self.total,
                plural(r, "run", "runs")
            ),
            None => format!("{} run points = {} × {t} × {s}", self.total, plural(self.corners, "corner", "corners")),
        }
    }
}

// --- Monte Carlo -----------------------------------------------------------------------

/// The reason Monte Carlo cannot run as the form has it, or None. For VACASK it is the
/// schema's own message; otherwise the mode's corner sets are looked up in the model
/// set (the text of `cdept sim run`: `needs the corner set 'tt_mm' in the models …`).
pub(super) fn mc_note(form: &Form, project: &Project) -> Option<String> {
    if form.simulator == "vacask" && form.has_mc() {
        return Some("Monte Carlo is run with ngspice or Xyce only (the statistical models are not converted for VACASK)".into());
    }
    let setup = form.setup().ok()?;
    let mc = setup.montecarlo.as_ref()?;
    let tech = project.tech.as_ref()?;
    let set = if setup.models.is_empty() { tech.models.default.clone()? } else { setup.models.clone() };
    sim::monte_carlo_plan(project, &tech.models, &set, &setup.simulator, mc).err().map(|e| format!("{e:#}"))
}

/// Why a mode cannot be picked with Xyce until its corner set is named.
pub(super) fn xyce_mode_reason(mode: McMode) -> &'static str {
    match mode {
        McMode::Process => "",
        McMode::Mismatch => {
            "Xyce: the PDK's models have no transistor mismatch sections; name a corner set for mismatch first"
        }
        McMode::Both => "Xyce: process + mismatch needs a corner set that has both; name it first",
    }
}

impl Form {
    /// Is the corner set of this mode named (Xyce needs it for mismatch and both)?
    fn mode_has_corner(&self, mode: McMode) -> bool {
        match mode {
            McMode::Process => true,
            McMode::Mismatch => !self.mc_mismatch_corner.trim().is_empty(),
            McMode::Both => !self.mc_both_corner.trim().is_empty(),
        }
    }

    /// Does the run have Monte Carlo (own or the template's)?
    pub(super) fn has_mc(&self) -> bool {
        self.mc || self.inherited.mc.is_some()
    }

    /// Own Monte Carlo from the template's (the user overrides it).
    pub(super) fn override_mc(&mut self) {
        if let Some(m) = self.inherited.mc.clone() {
            self.mc = true;
            self.mc_runs = m.runs.to_string();
            self.mc_seed = m.seed.to_string();
            self.mc_mode = m.mode;
            self.mc_corner = m.corner;
            self.mc_mismatch_corner = m.mismatch_corner;
            self.mc_both_corner = m.both_corner;
        }
    }

    /// Own sweep rows from the template's sweep of this variable.
    pub(super) fn override_sweep(&mut self, variable: &str) -> Result<(), String> {
        let s = self
            .inherited
            .sweeps
            .iter()
            .find(|s| s.variable.eq_ignore_ascii_case(variable))
            .ok_or_else(|| format!("the template has no sweep of '{variable}'"))?;
        self.sweeps.push(SweepForm::of(s));
        Ok(())
    }

    /// The table `[montecarlo]` of the file for this form (own fields only), or None.
    pub(super) fn mc_table(&self) -> Result<Option<toml::Table>, String> {
        use toml::Value;
        if !self.mc {
            return Ok(self.inherited.mc.as_ref().and_then(|m| toml::Table::try_from(m).ok()));
        }
        let whole = |what: &str, v: &str| -> Result<Value, String> {
            v.trim().parse::<i64>().map(Value::Integer).map_err(|_| format!("Monte Carlo {what} '{v}' is not a whole number"))
        };
        let mut t = toml::Table::new();
        t.insert("runs".into(), whole("runs", &self.mc_runs)?);
        t.insert("seed".into(), whole("seed", &self.mc_seed)?);
        if self.mc_mode != McMode::Process {
            t.insert("mode".into(), Value::String(self.mc_mode.name().into()));
        }
        for (key, value) in [
            ("corner", &self.mc_corner),
            ("mismatch_corner", &self.mc_mismatch_corner),
            ("both_corner", &self.mc_both_corner),
        ] {
            if !value.trim().is_empty() {
                t.insert(key.into(), Value::String(value.trim().to_string()));
            }
        }
        Ok(Some(t))
    }
}

// --- drawing ---------------------------------------------------------------------------

fn bad(ui: &Ui) -> Color32 {
    ui.visuals().error_fg_color
}

fn warn(ui: &Ui) -> Color32 {
    ui.visuals().warn_fg_color
}

/// The Sweeps section.
pub(super) fn sweeps_ui(ui: &mut Ui, form: &mut Form) {
    ui.strong("Sweeps  ([[sweep]]: a design variable, crossed with the corners and temperatures)");
    // From the template: read-only, replaced by an own sweep of the same variable.
    let mut override_var = None;
    let name = form.inherited.template.clone();
    for s in &form.inherited.sweeps {
        let overridden = form.inherited.overridden(&form.sweeps, &s.variable);
        let values = SweepForm::of(s).values;
        ui.horizontal(|ui| {
            let text = format!("{} = {}", s.variable, values);
            if overridden {
                ui.weak(RichText::new(text).strikethrough());
                ui.weak(format!("(from template {name}, replaced by your own below)"));
            } else {
                ui.label(RichText::new(text).monospace().weak());
                ui.weak(format!("from template {name}"));
                if ui.small_button("Override").on_hover_text("Copy it below and edit it").clicked() {
                    override_var = Some(s.variable.clone());
                }
            }
        });
    }
    if let Some(v) = override_var {
        let _ = form.override_sweep(&v);
    }
    let mut remove = None;
    let names: Vec<String> = form.variables.iter().map(|(n, _)| n.trim().to_string()).filter(|n| !n.is_empty()).collect();
    for i in 0..form.sweeps.len() {
        let problem = row_problem(&form.sweeps, i);
        let count = form.sweeps[i].count();
        ui.horizontal(|ui| {
            let row = &mut form.sweeps[i];
            ui.add(TextEdit::singleline(&mut row.variable).desired_width(90.0).hint_text("variable"));
            if !names.is_empty() {
                ui.menu_button("…", |ui| {
                    ui.weak("the [variables] of this setup");
                    for n in &names {
                        if ui.button(n).clicked() {
                            row.variable = n.clone();
                            ui.close();
                        }
                    }
                })
                .response
                .on_hover_text("Pick one of the setup's variables");
            }
            ui.add(TextEdit::singleline(&mut row.values).desired_width(220.0).hint_text("1.08, 1.2, 1.32  or  0.9:1.3:0.1"))
                .on_hover_text("SPICE numbers separated by commas or spaces, or a range start:stop:step");
            if problem.is_none() {
                ui.weak(format!("{count} value{}", if count == 1 { "" } else { "s" }));
            }
            if ui.small_button("✖").on_hover_text("Remove").clicked() {
                remove = Some(i);
            }
        });
        if let Some(p) = problem {
            ui.colored_label(bad(ui), p);
        }
    }
    if let Some(i) = remove {
        form.sweeps.remove(i);
    }
    if ui.small_button("＋ Add sweep").clicked() {
        form.sweeps.push(SweepForm::default());
    }
}

/// The Monte Carlo section. `corners` are the corner sets of the model set.
pub(super) fn mc_ui(ui: &mut Ui, form: &mut Form, corners: &[String], note: Option<&str>) {
    let template = form.inherited.template.clone();
    match (form.inherited.mc.clone(), form.mc) {
        (Some(m), false) => {
            ui.horizontal(|ui| {
                ui.label("Monte Carlo");
                ui.weak(format!(
                    "from template {template}: {} runs, seed {}, {}",
                    m.runs,
                    m.seed,
                    m.mode.name()
                ));
                if ui.small_button("Override").on_hover_text("Edit your own [montecarlo]: it replaces the template's").clicked() {
                    form.override_mc();
                }
            });
        }
        (inherited, _) => {
            ui.horizontal(|ui| {
                ui.checkbox(&mut form.mc, "Monte Carlo  ([montecarlo]: seeded runs instead of the corner grid)");
                if inherited.is_some() && form.mc && ui.small_button("Use the template's").clicked() {
                    form.mc = false;
                }
            });
        }
    }
    if form.mc {
        let xyce = form.simulator == "xyce";
        egui::Grid::new("sim-mc").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
            ui.label("Runs / seed");
            ui.horizontal(|ui| {
                ui.add(TextEdit::singleline(&mut form.mc_runs).desired_width(60.0));
                ui.add(TextEdit::singleline(&mut form.mc_seed).desired_width(90.0));
            });
            ui.end_row();
            ui.label("Draws").on_hover_text(
                "process: global variation (the statistical corner), mismatch: per device, both: together",
            );
            ComboBox::from_id_salt("sim-mc-mode").selected_text(form.mc_mode.name()).show_ui(ui, |ui| {
                for mode in [McMode::Process, McMode::Mismatch, McMode::Both] {
                    let allowed = !xyce || form.mode_has_corner(mode);
                    let r = ui.add_enabled_ui(allowed, |ui| ui.selectable_value(&mut form.mc_mode, mode, mode.name())).inner;
                    if !allowed {
                        r.on_hover_text(xyce_mode_reason(mode));
                    }
                }
            });
            ui.end_row();
            let mode = form.mc_mode;
            for (label, hint, value, used) in [
                ("Process corner", "mc", &mut form.mc_corner, true),
                ("Mismatch corner", "tt_mm", &mut form.mc_mismatch_corner, mode == McMode::Mismatch || xyce),
                ("Both corner", "mc_mm", &mut form.mc_both_corner, mode == McMode::Both || xyce),
            ] {
                ui.label(label).on_hover_text("The corner set of this kind of draw; empty: the default");
                ui.horizontal(|ui| {
                    ui.add_enabled_ui(used, |ui| corner_combo(ui, label, value, corners, hint));
                });
                ui.end_row();
            }
        });
        if xyce && form.mc_mode == McMode::Process {
            ui.weak("Xyce: process variation only, unless a corner set with mismatch is named above");
        }
    }
    if let Some(note) = note {
        ui.colored_label(warn(ui), note.lines().next().unwrap_or(""));
    }
}

fn corner_combo(ui: &mut Ui, id: &str, value: &mut String, corners: &[String], default: &str) {
    let text = if value.is_empty() { format!("{default} (default)") } else { value.clone() };
    ComboBox::from_id_salt(("sim-mc-corner", id)).selected_text(text).show_ui(ui, |ui| {
        ui.selectable_value(value, String::new(), format!("{default} (default)"));
        for c in corners {
            ui.selectable_value(value, c.clone(), c);
        }
    });
}

/// The line next to the Run buttons: how many points a run makes.
pub(super) fn plan_ui(ui: &mut Ui, view: &SimulationView) {
    let Some(form) = &view.form else { return };
    let Ok(setup) = form.setup() else { return };
    let plan = RunPlan::of(&setup);
    ui.horizontal_wrapped(|ui| {
        ui.label(plan.describe());
        if plan.many() {
            ui.colored_label(warn(ui), format!("more than {MANY_POINTS} points: this will take a while"));
        }
        if let Ok(n) = view.now_runs_value()
            && setup.montecarlo.is_some()
        {
            ui.weak(format!("Run N MC: {} points", RunPlan::with_runs(&setup, n).total));
        }
        if setup.montecarlo.is_some() {
            ui.weak(format!("Grid: {} points", RunPlan::grid(&setup).total));
        }
    });
}

// --- script verbs ----------------------------------------------------------------------

fn same_values(a: &str, b: &str) -> bool {
    let norm = |t: &str| if t.contains(':') { t.replace(' ', "") } else { words(t).join(",") };
    norm(a) == norm(b)
}

/// The `sim …` verbs of this file: add_sweep, set_sweep, remove_sweep, override_sweep,
/// set_mc, expect_sweep, expect_inherited, expect_plan, expect_mc_note. `None` when the
/// verb is not one of them.
pub(super) fn script(view: &mut SimulationView, project: &Project, args: &[String]) -> Option<Result<(), String>> {
    let verb = args.first()?.as_str();
    if !matches!(
        verb,
        "add_sweep" | "set_sweep" | "remove_sweep" | "override_sweep" | "set_mc" | "expect_sweep" | "expect_inherited" | "expect_plan" | "expect_mc_note" | "expect_mc"
    ) {
        return None;
    }
    Some(run_verb(view, project, verb, &args[1..]))
}

fn run_verb(view: &mut SimulationView, project: &Project, verb: &str, args: &[String]) -> Result<(), String> {
    let arg = |i: usize| args.get(i).map(String::as_str).ok_or_else(|| "missing argument".to_string());
    let index = |form: &Form, i: usize| -> Result<usize, String> {
        let n: usize = arg(i)?.parse().map_err(|_| "the index is a number".to_string())?;
        if n < form.sweeps.len() { Ok(n) } else { Err(format!("there are {} own sweeps", form.sweeps.len())) }
    };
    let form = view.form.as_mut().ok_or("no testbench is open")?;
    match verb {
        // add_sweep <variable> <value>…   (or <start>:<stop>:<step>, or start <a> stop <b> step <c>)
        "add_sweep" => {
            let words = &args[1.min(args.len())..];
            let values = if words.len() == 6 && words[0] == "start" {
                format!("{}:{}:{}", words[1], words[3], words[5])
            } else {
                words.join(", ")
            };
            form.sweeps.push(SweepForm::new(arg(0)?, &values));
        }
        // set_sweep <index> variable|values <text…>
        "set_sweep" => {
            let i = index(form, 0)?;
            let text = args[2.min(args.len())..].join(" ");
            match arg(1)? {
                "variable" => form.sweeps[i].variable = text,
                "values" => form.sweeps[i].values = text.replace(' ', ", ").replace(",,", ","),
                other => return Err(format!("variable or values, not '{other}'")),
            }
        }
        "remove_sweep" => {
            let i = index(form, 0)?;
            form.sweeps.remove(i);
        }
        // override_sweep <variable>: an own copy of the template's sweep
        "override_sweep" => form.override_sweep(arg(0)?)?,
        // set_mc override | runs|seed|mode <v> | corner|mismatch_corner|both_corner <v> | corners <name>
        "set_mc" => {
            let value = args[1.min(args.len())..].join(" ");
            match arg(0)? {
                "override" => form.override_mc(),
                "runs" => form.mc_runs = value,
                "seed" => form.mc_seed = value,
                "mode" => form.mc_mode = value.parse()?,
                "corner" => form.mc_corner = value,
                "mismatch_corner" => form.mc_mismatch_corner = value,
                "both_corner" => form.mc_both_corner = value,
                // The corner set of the mode that is picked.
                "corners" => match form.mc_mode {
                    McMode::Process => form.mc_corner = value,
                    McMode::Mismatch => form.mc_mismatch_corner = value,
                    McMode::Both => form.mc_both_corner = value,
                },
                other => return Err(format!("unknown Monte Carlo field '{other}'")),
            }
        }
        // expect_sweep <index> <variable> <values…>
        "expect_sweep" => {
            let i = index(form, 0)?;
            let row = &form.sweeps[i];
            let want = args[2.min(args.len())..].join(" ");
            if row.variable.trim() != arg(1)? || !same_values(&row.values, &want) {
                return Err(format!("sweep {i} is '{}' = '{}'", row.variable, row.values));
            }
        }
        // expect_inherited <sweeps> <mc|nomc>: what the template brings
        "expect_inherited" => {
            let n: usize = arg(0)?.parse().map_err(|_| "a number of sweeps")?;
            let mc = match arg(1)? {
                "mc" => true,
                "nomc" => false,
                other => return Err(format!("mc or nomc, not '{other}'")),
            };
            if form.inherited.sweeps.len() != n || form.inherited.mc.is_some() != mc {
                return Err(format!(
                    "the template brings {} sweeps and {} Monte Carlo",
                    form.inherited.sweeps.len(),
                    if form.inherited.mc.is_some() { "a" } else { "no" }
                ));
            }
        }
        // expect_plan <n>: run points of a run now (corners x temperatures x sweeps, or x MC runs)
        "expect_plan" => {
            let n: usize = arg(0)?.parse().map_err(|_| "a number of points")?;
            let have = RunPlan::of(&form.setup()?).total;
            if have != n {
                return Err(format!("the plan is {have} points"));
            }
        }
        // expect_mc_note <text…>: the inline reason Monte Carlo cannot run (empty: none)
        "expect_mc_note" => {
            let want = args.join(" ");
            let have = mc_note(form, project);
            match (&have, want.is_empty()) {
                (None, true) => {}
                (Some(h), false) if h.contains(&want) => {}
                _ => return Err(format!("the note is {have:?}")),
            }
        }
        // expect_mc <on|off>: the run has Monte Carlo (own or the template's)
        "expect_mc" => {
            if form.has_mc() != matches!(arg(0)?, "on" | "yes") {
                return Err(format!("Monte Carlo is {}", form.has_mc()));
            }
        }
        _ => unreachable!(),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::views::testlib;
    use libcdept::sim::template::TemplateSet;

    const SWEEPY: &str = r#"
name = "sweepy"
description = "a template with sweeps"
[[param]]
name = "mc"
type = "bool"
default = "false"
[[param]]
name = "runs"
type = "int"
optional = true
[[sweep]]
variable = "cl"
values = ["10f", "100f"]
[[sweep]]
variable = "vdd"
start = 1.0
stop = 1.4
step = 0.2
[montecarlo]
when = "mc"
runs = "${runs}"
seed = 7
mode = "mismatch"
[setup]
temperatures = [27]
"#;

    fn form(text: &str) -> Form {
        Form::of(&testlib::parse(text).unwrap()).unwrap()
    }

    #[test]
    fn rows_are_validated_like_the_schema() {
        let r = |v: &str, x: &str| SweepForm::new(v, x).to_sweep();
        assert_eq!(r("vdd", "1.08, 1.2 1.32").unwrap().values, ["1.08", "1.2", "1.32"]);
        assert_eq!(r("c", "10f,1p").unwrap().values, ["10f", "1p"]);
        let range = r("v", "0.9:1.3:0.1").unwrap();
        assert_eq!(range.expand().unwrap(), ["0.9", "1", "1.1", "1.2", "1.3"]);
        assert!(r("v", "1,x").unwrap_err().contains("value 'x' is not a number"));
        assert!(r("v", "a:2:1").unwrap_err().contains("start 'a' is not a number"));
        assert!(r("v", "1:2").unwrap_err().contains("start:stop:step"));
        assert!(r("v", "").unwrap_err().contains("needs `values` or start, stop and step"));
        assert!(r("v", "3:1:1").unwrap_err().contains("does not lead from"));
        assert!(r("", "1").unwrap_err().contains("is not a valid sweep variable"));
        assert!(r("a b", "1").unwrap_err().contains("is not a valid sweep variable"));
        let rows = vec![SweepForm::new("a", "1"), SweepForm::new("a", "2")];
        assert_eq!(own_sweeps(&rows).unwrap_err(), "variable 'a' is swept twice");
        assert_eq!(row_problem(&rows, 0), None);
        assert!(row_problem(&rows, 1).unwrap().contains("swept twice"));
        // a row shows what the file had, in both forms
        assert_eq!(SweepForm::of(&range).values, "0.9:1.3:0.1");
        assert_eq!(SweepForm::of(&r("v", "1,2").unwrap()).values, "1, 2");
    }

    #[test]
    fn points_are_counted() {
        let s = |t: &str| testlib::parse(t).unwrap();
        let base = "corners = [\"tt\", \"ff\"]\ntemperatures = [-40, 27, 125]\n\
                    [[sweep]]\nvariable = \"vdd\"\nvalues = [\"1.1\", \"1.2\"]\n\
                    [[sweep]]\nvariable = \"c\"\nvalues = [\"1p\", \"2p\", \"3p\"]\n";
        let grid = RunPlan::of(&s(base));
        assert_eq!((grid.corners, grid.temperatures, grid.sweep_points, grid.total), (2, 3, 6, 36));
        assert!(grid.describe().contains("36 run points = 2 corners × 3 temperatures × 6 sweep points"));
        let mc = s(&format!("{base}[montecarlo]\nruns = 5\n"));
        let p = RunPlan::of(&mc);
        assert_eq!((p.mc_runs, p.total), (Some(5), 3 * 6 * 5));
        assert_eq!(RunPlan::grid(&mc).total, 36);
        assert_eq!(RunPlan::with_runs(&mc, 2).total, 36);
        let none = RunPlan::of(&s("schematic = \"x\""));
        assert_eq!(none.total, 1);
        assert!(none.describe().contains("1 run points = 1 corner × 1 temperature"));
        // the warning threshold
        let big = s("[[sweep]]\nvariable = \"a\"\nstart = 1\nstop = 600\nstep = 1\n");
        assert!(RunPlan::of(&big).many());
        assert!(!grid.many());
    }

    #[test]
    fn the_form_writes_its_sweeps_and_monte_carlo() {
        let mut f = form("corners = [\"tt\"]\n");
        assert!(f.sweeps.is_empty() && !f.mc);
        f.sweeps.push(SweepForm::new("vdd", "1.08, 1.2, 1.32"));
        f.mc = true;
        f.mc_runs = "5".into();
        let text = f.to_text().unwrap();
        let back = testlib::parse(&text).unwrap();
        assert_eq!(back.sweeps.len(), 1);
        assert_eq!(back.montecarlo.as_ref().map(|m| (m.runs, m.mode)), Some((5, McMode::Process)));
        assert!(!text.contains("mode"), "the default mode is not written: {text}");
        f.mc_mode = McMode::Both;
        f.mc_both_corner = "mc_mm".into();
        assert!(f.to_text().unwrap().contains("both_corner = \"mc_mm\""));
        // errors of the form are the schema's
        f.sweeps.push(SweepForm::new("vdd", "1"));
        assert!(f.to_text().unwrap_err().contains("swept twice"));
        f.sweeps.pop();
        f.mc_runs = "0".into();
        assert!(f.to_text().unwrap_err().contains("runs must be 1..100000"));
        f.mc_runs = "5".into();
        f.simulator = "vacask".into();
        assert!(f.to_text().unwrap_err().contains("VACASK"));
        f.simulator = "xyce".into();
        assert!(f.to_text().is_ok());
        // a form that has them round-trips
        let g = form(&f.to_text().unwrap());
        assert_eq!(g.sweeps, f.sweeps);
        assert!(g.mc && g.mc_both_corner == "mc_mm");
    }

    #[test]
    fn what_a_template_brings_is_inherited_and_not_written() {
        testlib::use_templates(TemplateSet::from_texts(&[SWEEPY]).unwrap());
        let f = form("template = \"sweepy\"\n");
        // inherited, read-only: not among the own rows, and not in the file
        assert_eq!(f.inherited.sweeps.iter().map(|s| s.variable.as_str()).collect::<Vec<_>>(), ["cl", "vdd"]);
        assert!(f.sweeps.is_empty() && !f.mc && f.inherited.mc.is_none());
        let text = f.to_text().unwrap();
        assert!(!text.contains("[[sweep]]") && !text.contains("montecarlo"), "{text}");
        assert_eq!(RunPlan::of(&f.setup().unwrap()).sweep_points, 2 * 3);

        // own sweep of another variable: added; of the same one: replaces
        let mut f = f;
        f.sweeps.push(SweepForm::new("cl", "1f"));
        f.sweeps.push(SweepForm::new("temp", "1, 2"));
        let text = f.to_text().unwrap();
        assert!(text.contains("temp") && text.contains("\"1f\""), "{text}");
        assert!(!text.contains("vdd") && !text.contains("100f"), "{text}");
        let s = f.setup().unwrap();
        assert_eq!(s.sweeps.iter().map(|w| w.variable.as_str()).collect::<Vec<_>>(), ["cl", "vdd", "temp"]);
        assert_eq!(RunPlan::of(&s).sweep_points, 3 * 2);
        // loading that file gives the same own rows
        let g = form(&text);
        assert_eq!(g.sweeps, f.sweeps);

        // Monte Carlo from a template parameter: inherited; overriding writes it whole
        let mut m = form("template = \"sweepy\"\n[template_args]\nmc = true\nruns = 25\n");
        assert_eq!(m.inherited.mc.as_ref().map(|c| (c.runs, c.seed)), Some((25, 7)));
        assert!(!m.mc && m.has_mc());
        assert!(!m.to_text().unwrap().contains("montecarlo"));
        m.override_mc();
        assert!(m.mc && m.mc_runs == "25" && m.mc_mode == McMode::Mismatch);
        assert!(!m.to_text().unwrap().contains("montecarlo"), "unchanged is not written");
        m.mc_runs = "3".into();
        let text = m.to_text().unwrap();
        assert!(text.contains("[montecarlo]") && text.contains("runs = 3"), "{text}");
        // the own table replaces the template's as a whole: the seed is written too
        assert!(text.contains("seed = 7") && text.contains("mismatch"), "{text}");
        assert_eq!(m.setup().unwrap().montecarlo.unwrap().runs, 3);
        let back = form(&text);
        assert!(back.mc && back.mc_runs == "3");
        testlib::use_templates(TemplateSet::builtin().clone());
    }
}
