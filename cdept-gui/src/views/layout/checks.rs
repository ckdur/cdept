//! DRC and LVS of the layout in a tab, and the panel with their results.
//!
//! Run DRC / Run LVS: the helper of the tab writes the layout as it is now (unsaved
//! changes too, every reference filled in), then a thread runs the PDK's deck in a
//! KLayout of its own (`libcdept::klayout::verify`) and, for LVS, netlists the
//! schematic with xschem first. Clicking a result highlights it over the picture
//! (the helper's `markers`) and zooms there.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Instant;

use egui::{CollapsingHeader, Color32, RichText, ScrollArea, Ui};
use libcdept::klayout::protocol::{NoticeLevel, method};
use libcdept::klayout::verify::{self, Check, DrcRun, LvsRun, Shape};
use serde_json::{Value, json};

use super::LayoutView;

/// Items listed per rule at most (the rest are counted).
const SHOWN_ITEMS: usize = 500;
/// Shapes sent for "Show all" of a rule at most.
const MARKED_SHAPES: usize = 5000;

/// What a finished check found.
pub(super) enum Outcome {
    Drc(DrcRun),
    Lvs(LvsRun),
}

enum Job {
    /// The helper writes the layout; the id of the call.
    Export {
        id: u64,
        check: Check,
        deck: PathBuf,
        dir: PathBuf,
        out: PathBuf,
        schematic: Option<PathBuf>,
    },
    /// The deck runs on a thread.
    Run {
        check: Check,
        result: Receiver<Result<Outcome, String>>,
        started: Instant,
    },
}

/// State of the checks of one tab.
#[derive(Default)]
pub(super) struct Checks {
    /// The results panel is shown.
    pub open: bool,
    job: Option<Job>,
    /// The last check and what came of it.
    last: Option<(Check, Result<Outcome, String>)>,
    /// What is highlighted: a label for the panel.
    marked: Option<String>,
}

impl Checks {
    pub fn running(&self) -> Option<Check> {
        match &self.job {
            Some(Job::Export { check, .. } | Job::Run { check, .. }) => Some(*check),
            None => None,
        }
    }
}

/// What a click in the panel asks for.
enum Act {
    Mark(String, Vec<Value>),
    Clear,
    Close,
    Rerun(Check),
}

impl LayoutView {
    /// Start DRC or LVS of the layout as it is in the helper now.
    pub fn run_check(&mut self, check: Check) {
        if self.checks.running().is_some() {
            return;
        }
        self.checks.open = true;
        match self.prepare_check(check) {
            Ok(job) => self.checks.job = Some(job),
            Err(error) => self.checks.last = Some((check, Err(error))),
        }
    }

    fn prepare_check(&mut self, check: Check) -> Result<Job, String> {
        let deck = verify::find_deck(&self.project, check).ok_or_else(|| {
            format!(
                "No {} deck: set [klayout] {} in cdept.toml (none was found in the PDK's \
                 libs.tech/klayout).",
                check.label(),
                check.name()
            )
        })?;
        let (library, cell) = self.check_names();
        let schematic = match check {
            Check::Drc => None,
            Check::Lvs => Some(
                verify::schematic_file(&self.project, &library, &cell, None)
                    .map_err(|e| format!("LVS needs the schematic: {e:#}"))?,
            ),
        };
        let dir = verify::work_dir(&self.project, &library, &cell, check)
            .map_err(|e| format!("{e:#}"))?;
        let out = dir.join(format!("{cell}.gds"));
        let id = self.raw_call(method::EXPORT, json!({ "out": out }));
        Ok(Job::Export {
            id,
            check,
            deck,
            dir,
            out,
            schematic,
        })
    }

    /// Library and cell of the document; the file name for a loose file.
    fn check_names(&self) -> (String, String) {
        if !self.doc.cell.is_empty() {
            return (self.doc.library.clone(), self.doc.cell.clone());
        }
        let stem = verify::fallback_top(&self.file);
        ("files".to_string(), stem)
    }

    /// Move a running check on; called every frame.
    pub(super) fn poll_checks(&mut self, ctx: &egui::Context) {
        match self.checks.job.take() {
            Some(Job::Export {
                id,
                check,
                deck,
                dir,
                out,
                schematic,
            }) => match self.take_reply(id) {
                None => {
                    self.checks.job = Some(Job::Export {
                        id,
                        check,
                        deck,
                        dir,
                        out,
                        schematic,
                    })
                }
                Some(Err(error)) => {
                    let text = format!(
                        "The layout could not be written for {}: {error}",
                        check.label()
                    );
                    self.checks.last = Some((check, Err(text)));
                }
                Some(Ok(answer)) => {
                    let value: Value = serde_json::from_str(&answer).unwrap_or(Value::Null);
                    let top = self
                        .cells
                        .as_ref()
                        .map(|c| c.top.clone())
                        .filter(|t| !t.is_empty())
                        .unwrap_or_else(|| verify::fallback_top(&self.file));
                    let stream = verify::stream_of(&out, &value, &top);
                    if !stream.missing.is_empty() {
                        self.notify(
                            NoticeLevel::Warning,
                            format!(
                                "Checked without the cells that cannot be found: {}",
                                stream.missing.join(", ")
                            ),
                        );
                    }
                    let project = self.project.clone();
                    let (sender, result) = mpsc::channel();
                    let wake = ctx.clone();
                    std::thread::Builder::new()
                        .name(format!("cdept-{}", check.name()))
                        .spawn(move || {
                            let outcome = run(&project, check, &deck, &stream, &dir, schematic);
                            let _ = sender.send(outcome.map_err(|e| format!("{e:#}")));
                            wake.request_repaint();
                        })
                        .expect("cannot start a thread");
                    self.checks.job = Some(Job::Run {
                        check,
                        result,
                        started: Instant::now(),
                    });
                }
            },
            Some(Job::Run {
                check,
                result,
                started,
            }) => match result.try_recv() {
                Ok(outcome) => {
                    let level = match &outcome {
                        Ok(Outcome::Drc(run)) if run.rdb.items.is_empty() => NoticeLevel::Info,
                        Ok(Outcome::Lvs(run)) if run.summary.matched => NoticeLevel::Info,
                        _ => NoticeLevel::Warning,
                    };
                    self.notify(level, summary(check, &outcome));
                    self.checks.last = Some((check, outcome));
                    self.checks.marked = None;
                }
                Err(TryRecvError::Empty) => {
                    self.checks.job = Some(Job::Run {
                        check,
                        result,
                        started,
                    })
                }
                Err(TryRecvError::Disconnected) => {
                    self.checks.last = Some((check, Err(format!("{} stopped", check.label()))))
                }
            },
            None => {}
        }
    }

    /// The menu entries (in the Layout menu).
    pub(super) fn checks_menu(&mut self, ui: &mut Ui, ready: bool) {
        let idle = ready && self.checks.running().is_none();
        for check in [Check::Drc, Check::Lvs] {
            let hover = match check {
                Check::Drc => "Run the PDK's design rule check on the layout as it is now",
                Check::Lvs => "Compare the layout with the cell's schematic (PDK's LVS deck)",
            };
            if ui
                .add_enabled(idle, egui::Button::new(format!("Run {}", check.label())))
                .on_hover_text(hover)
                .clicked()
            {
                self.run_check(check);
                ui.close();
            }
        }
        ui.checkbox(&mut self.checks.open, "DRC/LVS Results");
    }

    /// The tool bar buttons.
    pub(super) fn checks_toolbar(&mut self, ui: &mut Ui, ready: bool) {
        let idle = ready && self.checks.running().is_none();
        for check in [Check::Drc, Check::Lvs] {
            if ui
                .add_enabled(idle, egui::Button::new(check.label()))
                .on_hover_text(format!("Run {}", check.label()))
                .clicked()
            {
                self.run_check(check);
            }
        }
    }

    /// The results panel at the bottom of the tab, when it is open.
    pub(super) fn checks_panel(&mut self, ui: &mut Ui) {
        if !self.checks.open {
            return;
        }
        let mut acts = Vec::new();
        egui::Panel::bottom(egui::Id::new(("klayout-checks", self.id)))
            .resizable(true)
            .default_size(200.0)
            .size_range(80.0..=800.0)
            .show(ui, |ui| self.checks_contents(ui, &mut acts));
        for act in acts {
            match act {
                Act::Mark(label, shapes) => {
                    self.checks.marked = Some(label);
                    self.quiet(method::MARKERS, json!({"shapes": shapes, "zoom": true}));
                }
                Act::Clear => {
                    self.checks.marked = None;
                    self.quiet(method::MARKERS, json!({"shapes": []}));
                }
                Act::Close => self.checks.open = false,
                Act::Rerun(check) => self.run_check(check),
            }
        }
    }

    fn checks_contents(&self, ui: &mut Ui, acts: &mut Vec<Act>) {
        ui.horizontal(|ui| {
            ui.strong("DRC/LVS");
            if let Some(check) = self.checks.running() {
                ui.spinner();
                let since = match &self.checks.job {
                    Some(Job::Run { started, .. }) => {
                        format!(" ({} s)", started.elapsed().as_secs())
                    }
                    _ => String::new(),
                };
                ui.label(format!("Running {}…{since}", check.label()));
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(500));
            } else if let Some((check, _)) = &self.checks.last
                && ui.button("Run again").clicked()
            {
                acts.push(Act::Rerun(*check));
            }
            if let Some(marked) = &self.checks.marked {
                ui.separator();
                ui.label(format!("Shown: {marked}"));
                if ui
                    .button("Clear")
                    .on_hover_text("Remove the highlights")
                    .clicked()
                {
                    acts.push(Act::Clear);
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button("Close")
                    .on_hover_text("Close the results")
                    .clicked()
                {
                    acts.push(Act::Close);
                }
            });
        });
        ui.separator();
        let Some((check, outcome)) = &self.checks.last else {
            if self.checks.running().is_none() {
                ui.weak("Layout > Run DRC or Run LVS checks the layout with the PDK's decks.");
            }
            return;
        };
        ScrollArea::vertical()
            .id_salt(("klayout-checks-scroll", self.id))
            .auto_shrink([false, false])
            .show(ui, |ui| match outcome {
                Err(error) => {
                    ui.colored_label(Color32::LIGHT_RED, format!("{} failed", check.label()));
                    ui.label(error);
                }
                Ok(Outcome::Drc(run)) => drc_results(ui, run, self.id, acts),
                Ok(Outcome::Lvs(run)) => lvs_results(ui, run, self.id, acts),
            });
    }

    /// Highlight DRC result `n` (from 1, in the order of the report), as a click on it
    /// in the panel does.
    pub fn show_check_item(&mut self, n: usize) -> Result<(), String> {
        let Some((_, Ok(Outcome::Drc(run)))) = &self.checks.last else {
            return Err("there are no DRC results".into());
        };
        let item = n
            .checked_sub(1)
            .and_then(|i| run.rdb.items.get(i))
            .ok_or(format!("there are {} DRC results", run.rdb.items.len()))?;
        let label = format!("{} #{n}", item.category.join("."));
        let shapes: Vec<Value> = item.shapes.iter().map(Shape::to_marker).collect();
        self.checks.marked = Some(label);
        self.quiet(method::MARKERS, json!({"shapes": shapes, "zoom": true}));
        Ok(())
    }

    /// For the test driver: the state of the checks as text.
    pub fn check_status(&self) -> String {
        if let Some(check) = self.checks.running() {
            return format!("running {}", check.name());
        }
        match &self.checks.last {
            None => "none".into(),
            Some((check, outcome)) => summary(*check, outcome),
        }
    }
}

fn run(
    project: &libcdept::Project,
    check: Check,
    deck: &Path,
    stream: &verify::Stream,
    dir: &Path,
    schematic: Option<PathBuf>,
) -> eyre::Result<Outcome> {
    match (check, schematic) {
        (Check::Lvs, Some(schematic)) => {
            let cell = &stream.top;
            let netlist =
                verify::schematic_netlist(project, &schematic, &dir.join(format!("{cell}.spice")))?;
            let run = verify::run_lvs(project, deck, stream, &netlist, dir, &[])?;
            Ok(Outcome::Lvs(run))
        }
        (Check::Lvs, None) => Err(eyre::eyre!("LVS needs a schematic")),
        (Check::Drc, _) => Ok(Outcome::Drc(verify::run_drc(
            project,
            deck,
            stream,
            dir,
            &[],
        )?)),
    }
}

/// One line about an outcome.
fn summary(check: Check, outcome: &Result<Outcome, String>) -> String {
    match outcome {
        Err(error) => format!("{} failed: {error}", check.label()),
        Ok(Outcome::Drc(run)) => {
            let counts = run.rdb.counts();
            if counts.is_empty() {
                "DRC clean".into()
            } else {
                format!(
                    "DRC: {} violation(s) of {} rule(s)",
                    run.rdb.items.len(),
                    counts.len()
                )
            }
        }
        Ok(Outcome::Lvs(run)) if run.summary.matched => "LVS: the netlists match".into(),
        Ok(Outcome::Lvs(run)) => {
            let n: usize = run.summary.circuits.iter().map(|c| c.pairs().count()).sum();
            if n == 0 {
                "LVS: the netlists do not match".into()
            } else {
                format!("LVS: the netlists do not match ({n} difference(s))")
            }
        }
    }
}

fn files_line(ui: &mut Ui, what: &str, file: &Path) {
    ui.horizontal(|ui| {
        ui.weak(what);
        ui.label(
            RichText::new(file.display().to_string())
                .monospace()
                .small(),
        )
        .on_hover_text("Open it in KLayout for the full report");
    });
}

fn drc_results(ui: &mut Ui, run: &DrcRun, id: crate::dock::DocId, acts: &mut Vec<Act>) {
    let counts = run.rdb.counts();
    if counts.is_empty() {
        ui.colored_label(Color32::LIGHT_GREEN, "No violations.");
    } else {
        ui.label(format!(
            "{} violation(s) of {} rule(s). Click one to show it.",
            run.rdb.items.len(),
            counts.len()
        ));
    }
    for (category, count) in counts {
        let name = category.name();
        let header = CollapsingHeader::new(format!("{name}  ({count})"))
            .id_salt(("klayout-drc-rule", id, name.clone()))
            .show(ui, |ui| {
                if !category.description.is_empty() {
                    ui.weak(&category.description);
                }
                if ui.small_button("Show all").clicked() {
                    let shapes: Vec<Value> = run
                        .rdb
                        .items_of(&category.path)
                        .flat_map(|i| i.shapes.iter())
                        .take(MARKED_SHAPES)
                        .map(Shape::to_marker)
                        .collect();
                    acts.push(Act::Mark(format!("{name} (all)"), shapes));
                }
                for (n, item) in run.rdb.items_of(&category.path).enumerate() {
                    if n == SHOWN_ITEMS {
                        ui.weak(format!("… and {} more", count - SHOWN_ITEMS));
                        break;
                    }
                    let place = match item.bbox() {
                        Some([l, b, r, t]) => format!("({l:.3}, {b:.3}) – ({r:.3}, {t:.3}) µm"),
                        None => "no place".into(),
                    };
                    let mut text = format!("#{}  {place}", n + 1);
                    if item.cell != run.rdb.top_cell && !item.cell.is_empty() {
                        text.push_str(&format!("  in {}", item.cell));
                    }
                    if let Some(note) = item.notes.first() {
                        text.push_str(&format!("  {note}"));
                    }
                    let enabled = !item.shapes.is_empty();
                    if ui
                        .add_enabled(enabled, egui::Button::new(text).frame(false))
                        .clicked()
                    {
                        let shapes = item.shapes.iter().map(Shape::to_marker).collect();
                        acts.push(Act::Mark(format!("{name} #{}", n + 1), shapes));
                    }
                }
            });
        header.header_response.on_hover_text(&category.description);
    }
    ui.separator();
    files_line(ui, "Report", &run.report);
    files_line(ui, "Output", &run.log);
    files_line(ui, "Deck", &run.deck);
}

fn lvs_results(ui: &mut Ui, run: &LvsRun, id: crate::dock::DocId, acts: &mut Vec<Act>) {
    if run.summary.matched {
        ui.colored_label(Color32::LIGHT_GREEN, "The layout matches the schematic.");
    } else {
        ui.colored_label(
            Color32::LIGHT_RED,
            "The layout does not match the schematic.",
        );
    }
    for circuit in &run.summary.circuits {
        let name = |n: &str| {
            if n.is_empty() {
                "—".to_string()
            } else {
                n.to_string()
            }
        };
        let title = format!(
            "{}  layout {} / schematic {}",
            circuit.status,
            name(&circuit.layout),
            name(&circuit.schematic)
        );
        let differences = circuit.pairs().count();
        CollapsingHeader::new(title)
            .id_salt((
                "klayout-lvs-circuit",
                id,
                circuit.layout.clone(),
                circuit.schematic.clone(),
            ))
            .default_open(differences > 0)
            .show(ui, |ui| {
                if differences == 0 {
                    ui.weak("No differences.");
                }
                for (kind, pair) in circuit.pairs() {
                    let text = format!(
                        "{:<16} {kind:<10} {} / {}",
                        pair.status,
                        name(&pair.layout),
                        name(&pair.schematic)
                    );
                    match pair.bbox {
                        Some(b) => {
                            if ui
                                .add(
                                    egui::Button::new(RichText::new(text).monospace()).frame(false),
                                )
                                .on_hover_text("Show the shapes of this net")
                                .clicked()
                            {
                                let shape = Shape::Box(b).to_marker();
                                acts.push(Act::Mark(format!("net {}", pair.layout), vec![shape]));
                            }
                        }
                        None => {
                            ui.label(RichText::new(text).monospace());
                        }
                    }
                }
            });
    }
    ui.separator();
    files_line(ui, "Database", &run.report);
    files_line(ui, "Extracted", &run.extracted);
    files_line(ui, "Schematic", &run.schematic);
    files_line(ui, "Output", &run.log);
    files_line(ui, "Deck", &run.deck);
}
