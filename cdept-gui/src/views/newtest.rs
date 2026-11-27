//! The "New Testbench" and "New Suite" dialogs: what `cdept sim new` does (and a
//! `suite.toml` written by hand) from the window. The new view opens in the
//! Simulation tab.

use std::collections::BTreeMap;

use egui::{Button, ComboBox, Grid, RichText, ScrollArea, TextEdit, Ui};
use eyre::{Result, bail, eyre};
use libcdept::sim::starter;
use libcdept::sim::suite::{self, Suite, TestRef};
use libcdept::sim::template::TemplateSet;
use libcdept::{Cell, Format, Library, LibraryLayout, Project, Template};

use crate::dialogs::Dialog;
use crate::guide;
use crate::state::{Action, AppCtx, Selection};

const TARGET: &str = "cdept::sim";

/// What the new testbench netlists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// A new schematic of the cell: the device under test with the template's sources.
    Dut,
    /// The schematic of another cell (`schematic =` in `setup.toml`).
    Other,
    /// A new, empty schematic of the cell, to draw the testbench in.
    Empty,
    /// Nothing yet.
    Nothing,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewTestbench {
    pub library: String,
    pub cell: String,
    pub view: String,
    /// The view name was typed: it no longer follows the template.
    pub view_typed: bool,
    pub template: String,
    pub args: BTreeMap<String, String>,
    pub source: Source,
    /// `library/cell` of the device under test.
    pub dut: String,
    /// `library/cell` whose schematic is netlisted.
    pub other: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewSuite {
    pub library: String,
    pub cell: String,
    pub view: String,
    pub description: String,
    /// The ticked testbenches, `library/cell/view`.
    pub tests: Vec<String>,
    /// Also offer the testbenches of the library's other cells.
    pub all_cells: bool,
    /// Corners and temperatures that replace every test's; empty: each test's own.
    pub corners: String,
    pub temperatures: String,
    /// The cell `tests` was filled for: another one ticks its own again.
    pub ticked_for: String,
}

fn writable(lib: &Library) -> bool {
    !lib.read_only && lib.error.is_none() && lib.layout == LibraryLayout::Cells
}

/// The library the dialog starts with: the selected one when views can be made in it.
fn home<'a>(project: &'a Project, selection: &Selection) -> Option<&'a Library> {
    selection
        .library
        .as_deref()
        .and_then(|name| project.library(name))
        .filter(|l| writable(l))
        .or_else(|| project.libraries.iter().find(|l| writable(l)))
}

fn has(cell: &Cell, format: Format) -> bool {
    cell.entries.iter().any(|e| e.format == Some(format))
}

/// `library/cell` of every cell with a view of `format`.
fn cells_with(project: &Project, format: Format) -> Vec<String> {
    project
        .libraries
        .iter()
        .flat_map(|l| {
            l.cells
                .iter()
                .filter(|c| has(c, format))
                .map(move |c| format!("{}/{}", l.name, c.name))
        })
        .collect()
}

fn cell_of<'a>(project: &'a Project, library: &str, cell: &str) -> Option<&'a Cell> {
    project.library(library)?.cell(cell.trim())
}

/// The first of `stem`, `stem2`, `stem3`, … that is not a view of the cell.
fn free_view(project: &Project, library: &str, cell: &str, stem: &str) -> String {
    let taken =
        |name: &str| cell_of(project, library, cell).is_some_and(|c| c.entry(name).is_some());
    (1..)
        .map(|n| match n {
            1 => stem.to_string(),
            n => format!("{stem}{n}"),
        })
        .find(|name| !taken(name))
        .expect("some name is free")
}

impl NewTestbench {
    /// For the selection: a cell with a symbol is proposed as the device under test of
    /// a testbench cell `tb_<cell>`.
    pub fn new(project: &Project, selection: &Selection) -> Option<Self> {
        let lib = home(project, selection)?;
        let selected = (selection.library.as_deref() == Some(&lib.name))
            .then(|| selection.cell.as_deref().and_then(|c| lib.cell(c)))
            .flatten();
        let mut new = Self {
            library: lib.name.clone(),
            cell: selected.map(|c| c.name.clone()).unwrap_or_default(),
            view: String::new(),
            view_typed: false,
            template: "tran_delay".into(),
            args: BTreeMap::new(),
            source: Source::Empty,
            dut: String::new(),
            other: String::new(),
        };
        if let Some(cell) = selected
            && has(cell, Format::XschemSymbol)
            && !cell.name.starts_with("tb_")
        {
            new.dut = format!("{}/{}", lib.name, cell.name);
            new.cell = format!("tb_{}", cell.name);
            new.source = Source::Dut;
        }
        Some(new)
    }

    fn has_schematic(&self, project: &Project) -> bool {
        cell_of(project, &self.library, &self.cell).is_some_and(|c| has(c, Format::XschemSchematic))
    }
}

impl NewSuite {
    pub fn new(project: &Project, selection: &Selection) -> Option<Self> {
        let lib = home(project, selection)?;
        let cell = (selection.library.as_deref() == Some(&lib.name))
            .then(|| selection.cell.clone())
            .flatten()
            .unwrap_or_default();
        let mut new = Self {
            library: lib.name.clone(),
            view: free_view(project, &lib.name, &cell, "suite"),
            cell,
            description: String::new(),
            tests: Vec::new(),
            all_cells: false,
            corners: String::new(),
            temperatures: String::new(),
            ticked_for: String::new(),
        };
        new.tick_own(project);
        Some(new)
    }

    fn key(&self) -> String {
        format!("{}/{}", self.library, self.cell.trim())
    }

    /// Tick every testbench of the cell, when the cell is another one than before.
    fn tick_own(&mut self, project: &Project) {
        let key = self.key();
        if self.ticked_for == key {
            return;
        }
        self.tests = testbenches(project, &self.library)
            .into_iter()
            .filter(|(cell, _)| cell == self.cell.trim())
            .map(|(cell, view)| format!("{}/{cell}/{view}", self.library))
            .collect();
        self.ticked_for = key;
    }
}

/// `(cell, view)` of the testbench views (not suites) of a library.
fn testbenches(project: &Project, library: &str) -> Vec<(String, String)> {
    let Some(lib) = project.library(library) else {
        return Vec::new();
    };
    lib.cells
        .iter()
        .flat_map(|c| {
            c.entries
                .iter()
                .filter(|e| {
                    super::simulation::is_testbench(e) && !super::testlib::is_suite_entry(e)
                })
                .map(move |e| (c.name.clone(), e.name.clone()))
        })
        .collect()
}

// --- drawing ---------------------------------------------------------------------------

fn library_combo(ui: &mut Ui, project: &Project, library: &mut String, anchor: &str) {
    let combo = ComboBox::from_id_salt(anchor)
        .width(280.0)
        .selected_text(library.as_str())
        .show_ui(ui, |ui| {
            for lib in project.libraries.iter().filter(|l| writable(l)) {
                ui.selectable_value(library, lib.name.clone(), &lib.name);
            }
        });
    guide::anchor(ui, anchor, &combo.response);
}

fn text_field(
    ui: &mut Ui,
    text: &mut String,
    hint: &str,
    focus: bool,
    anchor: &str,
) -> egui::Response {
    let response = ui.add(
        TextEdit::singleline(text)
            .desired_width(280.0)
            .hint_text(hint),
    );
    if focus {
        response.request_focus();
    }
    guide::anchor(ui, anchor, &response);
    response
}

/// A drop-down of `library/cell` names; `anchor` also names its entries
/// (`<anchor>.<library/cell>`).
fn cell_combo(ui: &mut Ui, choices: &[String], chosen: &mut String, anchor: &str) {
    let combo = ComboBox::from_id_salt(anchor)
        .width(280.0)
        .selected_text(if chosen.is_empty() {
            "choose a cell…"
        } else {
            chosen.as_str()
        })
        .show_ui(ui, |ui| {
            if choices.is_empty() {
                ui.weak("no such cell in the project");
            }
            for name in choices {
                let item = ui.selectable_value(chosen, name.clone(), name);
                guide::anchor(ui, &format!("{anchor}.{name}"), &item);
            }
        });
    guide::anchor(ui, anchor, &combo.response);
}

pub(crate) fn testbench_body(ui: &mut Ui, new: &mut NewTestbench, app: &AppCtx, focus: bool) {
    let Some(project) = app.project.as_ref() else {
        ui.label("No project is open.");
        return;
    };
    guide::fact(ui.ctx(), "dialog.new_testbench");
    let templates = TemplateSet::for_project(project);
    if !new.view_typed {
        new.view = free_view(project, &new.library, &new.cell, &new.template);
    }
    Grid::new("new-testbench")
        .num_columns(2)
        .spacing([8.0, 6.0])
        .show(ui, |ui| {
            ui.label("Library");
            library_combo(
                ui,
                project,
                &mut new.library,
                "dialog.new_testbench.library",
            );
            ui.end_row();
            ui.label("Cell")
                .on_hover_text("The testbench's cell: an existing one, or a new name");
            text_field(
                ui,
                &mut new.cell,
                "tb_mycell",
                focus,
                "dialog.new_testbench.cell",
            );
            ui.end_row();
            ui.label("Template");
            let before = new.template.clone();
            let combo = ComboBox::from_id_salt("dialog.new_testbench.template")
                .width(280.0)
                .selected_text(new.template.as_str())
                .show_ui(ui, |ui| {
                    for t in templates.list() {
                        let item = ui
                            .selectable_value(&mut new.template, t.name.clone(), &t.name)
                            .on_hover_text(&t.description);
                        guide::anchor(
                            ui,
                            &format!("dialog.new_testbench.template.{}", t.name),
                            &item,
                        );
                    }
                });
            guide::anchor(ui, "dialog.new_testbench.template", &combo.response);
            if before != new.template {
                new.args.clear();
            }
            ui.end_row();
            ui.label("View");
            if text_field(
                ui,
                &mut new.view,
                "view name",
                false,
                "dialog.new_testbench.view",
            )
            .changed()
            {
                new.view_typed = true;
            }
            ui.end_row();
        });
    guide::fact(
        ui.ctx(),
        &format!("dialog.new_testbench.template.{}", new.template),
    );

    ui.add_space(6.0);
    ui.strong("Schematic to simulate");
    if new.has_schematic(project) {
        ui.label(format!(
            "The cell '{}' has a schematic already: the testbench netlists it.",
            new.cell.trim()
        ));
    } else {
        for (source, label, anchor) in [
            (
                Source::Dut,
                "A starter schematic around a device under test",
                "dialog.new_testbench.source.dut",
            ),
            (
                Source::Other,
                "The schematic of another cell",
                "dialog.new_testbench.source.other",
            ),
            (
                Source::Empty,
                "A new empty schematic in this cell, to draw",
                "dialog.new_testbench.source.empty",
            ),
            (
                Source::Nothing,
                "None yet",
                "dialog.new_testbench.source.none",
            ),
        ] {
            let radio = ui.radio_value(&mut new.source, source, label);
            guide::anchor(ui, anchor, &radio);
        }
        match new.source {
            Source::Dut => {
                ui.horizontal(|ui| {
                    ui.label("Device under test")
                        .on_hover_text("A cell with an xschem symbol");
                    cell_combo(
                        ui,
                        &cells_with(project, Format::XschemSymbol),
                        &mut new.dut,
                        "dialog.new_testbench.dut",
                    );
                });
                if !new.dut.is_empty() {
                    guide::fact(ui.ctx(), &format!("dialog.new_testbench.dut.{}", new.dut));
                }
                ui.small(
                    "Written with the DUT, a supply, an input source and a load; pins are \
                     matched to the template's nets by name. Check it before you run.",
                );
            }
            Source::Other => {
                ui.horizontal(|ui| {
                    ui.label("Cell");
                    cell_combo(
                        ui,
                        &cells_with(project, Format::XschemSchematic),
                        &mut new.other,
                        "dialog.new_testbench.other",
                    );
                });
            }
            Source::Empty | Source::Nothing => {}
        }
    }

    ui.add_space(6.0);
    let Some(template) = templates.get(&new.template) else {
        ui.colored_label(
            ui.visuals().error_fg_color,
            format!("no template '{}'", new.template),
        );
        return;
    };
    ui.strong("Template parameters");
    ui.add(egui::Label::new(RichText::new(&template.description).weak()).wrap());
    let height = (ui.ctx().content_rect().height() - 560.0).clamp(120.0, 320.0);
    let scroll = ScrollArea::vertical()
        .id_salt("new-testbench-params")
        .max_height(height)
        .show(ui, |ui| {
            super::testlib::param_grid(ui, template, &mut new.args, "dialog.new_testbench.param");
        });
    let mut rect = scroll.inner_rect;
    rect.set_height(rect.height().min(scroll.content_size.y));
    guide::anchor_rect(ui, "dialog.new_testbench.params", rect);
    if let Err(e) = template.bind(&new.args) {
        ui.colored_label(
            ui.visuals().error_fg_color,
            format!("{e:#}").lines().next().unwrap_or("").to_string(),
        );
    }
    ui.small("They can be changed later, in the Simulation tab.");
}

pub(crate) fn suite_body(ui: &mut Ui, new: &mut NewSuite, app: &AppCtx, focus: bool) {
    let Some(project) = app.project.as_ref() else {
        ui.label("No project is open.");
        return;
    };
    guide::fact(ui.ctx(), "dialog.new_suite");
    Grid::new("new-suite")
        .num_columns(2)
        .spacing([8.0, 6.0])
        .show(ui, |ui| {
            ui.label("Library");
            library_combo(ui, project, &mut new.library, "dialog.new_suite.library");
            ui.end_row();
            ui.label("Cell");
            text_field(
                ui,
                &mut new.cell,
                "cell of the suite",
                focus,
                "dialog.new_suite.cell",
            );
            ui.end_row();
            ui.label("View");
            text_field(ui, &mut new.view, "suite", false, "dialog.new_suite.view");
            ui.end_row();
            ui.label("Description");
            text_field(
                ui,
                &mut new.description,
                "optional",
                false,
                "dialog.new_suite.description",
            );
            ui.end_row();
        });
    new.tick_own(project);

    ui.add_space(6.0);
    ui.strong("Testbenches to run");
    let all = testbenches(project, &new.library);
    let own = new.cell.trim().to_string();
    let none_own = !all.iter().any(|(cell, _)| *cell == own);
    if !none_own {
        ui.checkbox(
            &mut new.all_cells,
            "also those of the library's other cells",
        );
    }
    let everything = new.all_cells || none_own;
    let scroll = ScrollArea::vertical()
        .id_salt("new-suite-tests")
        .max_height(220.0)
        .show(ui, |ui| {
            if all.is_empty() {
                ui.weak("The library has no testbench view yet: make them first (New Testbench…).");
            }
            for (cell, view) in all.iter().filter(|(c, _)| everything || *c == own) {
                let id = format!("{}/{cell}/{view}", new.library);
                let mut on = new.tests.contains(&id);
                let label = if *cell == own {
                    view.clone()
                } else {
                    format!("{cell}/{view}")
                };
                let check = ui.checkbox(&mut on, label);
                guide::anchor(ui, &format!("dialog.new_suite.test.{view}"), &check);
                if check.changed() {
                    if on {
                        new.tests.push(id);
                    } else {
                        new.tests.retain(|t| *t != id);
                    }
                }
            }
        });
    let mut rect = scroll.inner_rect;
    rect.set_height(rect.height().min(scroll.content_size.y));
    guide::anchor_rect(ui, "dialog.new_suite.tests", rect);
    for test in &new.tests {
        if let Some(view) = test.rsplit('/').next() {
            guide::fact(ui.ctx(), &format!("dialog.new_suite.ticked.{view}"));
        }
    }
    ui.small(format!("{} chosen", new.tests.len()));

    ui.add_space(6.0);
    ui.strong("For every test (optional)");
    Grid::new("new-suite-grid")
        .num_columns(2)
        .spacing([8.0, 6.0])
        .show(ui, |ui| {
            ui.label("Corners");
            text_field(
                ui,
                &mut new.corners,
                "each test's own, e.g. tt, ff, ss",
                false,
                "dialog.new_suite.corners",
            );
            ui.end_row();
            ui.label("Temperatures");
            text_field(
                ui,
                &mut new.temperatures,
                "each test's own, e.g. -40, 27, 125",
                false,
                "dialog.new_suite.temperatures",
            );
            ui.end_row();
        });
    let known = project.model_corners("ngspice");
    if !known.is_empty() {
        ui.add(
            egui::Label::new(
                RichText::new(format!(
                    "Corners of the technology (ngspice): {}",
                    known.join(", ")
                ))
                .small()
                .weak(),
            )
            .wrap(),
        );
    }
}

/// The words of a list typed with commas or spaces.
fn words(text: &str) -> Vec<String> {
    text.split(|c: char| c == ',' || c.is_whitespace())
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

// --- doing it --------------------------------------------------------------------------

/// The project again, with the new view selected and open in the Simulation tab.
fn show(app: &mut AppCtx, library: &str, cell: &str, view: &str) {
    if let Some(project) = &mut app.project {
        project.rescan_library(library);
    }
    app.refresh_links();
    app.selection = Selection::view(library, cell, view);
    app.validate_selection();
    app.request(Action::OpenSimulation {
        selection: Selection::view(library, cell, view),
        run: false,
    });
}

/// The `library/cell` a testbench of `library/cell` netlists: its `schematic =`
/// (`library/cell` or `library/cell/view`), else its own cell.
fn netlisted_cell(schematic: &str, library: &str, cell: &str) -> String {
    let mut parts = schematic.split('/');
    match (parts.next().filter(|p| !p.is_empty()), parts.next()) {
        (Some(other_library), Some(other_cell)) => format!("{other_library}/{other_cell}"),
        _ => format!("{library}/{cell}"),
    }
}

fn same_file(a: &std::path::Path, b: &std::path::Path) -> bool {
    a == b || matches!((a.canonicalize(), b.canonicalize()), (Ok(a), Ok(b)) if a == b)
}

/// A schematic's sources use design variables (`{vdd}`), and a new testbench of it
/// needs their values: those of another testbench of the library that simulates the
/// same schematic (one of the same cell first, then the one with the most). What the
/// new setup sets already stays. Returns where from and which, if any were added.
fn copy_variables(
    project: &Project,
    templates: &TemplateSet,
    setup: &std::path::Path,
    library: &str,
    cell: &str,
    netlisted: &str,
) -> Result<Option<(String, Vec<String>)>> {
    let read = |file: &std::path::Path| -> Result<libcdept::sim::Setup> {
        libcdept::sim::Setup::parse_with(&std::fs::read_to_string(file)?, templates)
    };
    // The project does not know the new view yet: read its libraries again.
    let mut project = project.clone();
    project.rescan_library(library);
    let lib = project
        .library(library)
        .ok_or_else(|| eyre!("no library '{library}'"))?;
    let mut best: Option<((bool, usize), String, libcdept::sim::Setup)> = None;
    for (other_cell, view) in testbenches(&project, library) {
        let file = lib
            .cell(&other_cell)
            .and_then(|c| c.entry(&view))
            // A testbench view is a folder; its set-up is in it.
            .map(|e| e.path.join("setup.toml"));
        let Some(file) = file.filter(|f| !same_file(f, setup)) else {
            continue;
        };
        // A testbench that cannot be read is no source; it is not this one's trouble.
        let Ok(other) = read(&file) else {
            continue;
        };
        if netlisted_cell(&other.schematic, library, &other_cell) != netlisted
            || other.variables.is_empty()
        {
            continue;
        }
        let rank = (other_cell == cell, other.variables.len());
        if best.as_ref().is_none_or(|(have, ..)| rank > *have) {
            best = Some((rank, format!("{library}/{other_cell}/{view}"), other));
        }
    }
    let Some((_, from, source)) = best else {
        return Ok(None);
    };
    let have = read(setup)?.variables;
    let mut document: toml_edit::DocumentMut = std::fs::read_to_string(setup)?.parse()?;
    let table = document
        .entry("variables")
        .or_insert(toml_edit::Item::Table(toml_edit::Table::new()))
        .as_table_like_mut()
        .ok_or_else(|| eyre!("`variables` of the new setup is not a table"))?;
    let mut added = Vec::new();
    for (name, value) in &source.variables {
        if !have.contains_key(name) {
            table.insert(name, toml_edit::value(value.as_str()));
            added.push(name.clone());
        }
    }
    if added.is_empty() {
        return Ok(None);
    }
    std::fs::write(setup, document.to_string())?;
    Ok(Some((from, added)))
}

pub(crate) fn create_testbench(app: &mut AppCtx, new: &NewTestbench) -> Result<()> {
    let project = app
        .project
        .as_ref()
        .ok_or_else(|| eyre!("no project is open"))?;
    let (library, cell, view) = (new.library.as_str(), new.cell.trim(), new.view.trim());
    if cell.is_empty() {
        bail!("name the testbench's cell");
    }
    let templates = TemplateSet::for_project(project);
    let template = templates.find(&new.template)?;
    let has_schematic = new.has_schematic(project);
    let source = if has_schematic {
        Source::Nothing
    } else {
        new.source
    };
    if source == Source::Dut && new.dut.is_empty() {
        bail!("choose the device under test (a cell with a symbol)");
    }
    if source == Source::Other && new.other.is_empty() {
        bail!("choose the cell whose schematic is simulated");
    }
    let created = starter::create_testbench_using(
        project,
        library,
        cell,
        view,
        template,
        &new.args,
        (source == Source::Dut).then_some(new.dut.as_str()),
        (source == Source::Other).then_some(new.other.as_str()),
    )?;
    tracing::info!(target: TARGET, "created testbench {library}/{cell}/{view}: {}", created.setup.display());
    if let Some(file) = &created.schematic {
        tracing::info!(target: TARGET, "created the starter schematic {}", file.display());
    }
    for note in &created.notes {
        // The notes about a missing schematic say what `cdept sim new` would take.
        if source != Source::Empty {
            tracing::info!(target: TARGET, "{note}");
        }
    }
    if source == Source::Empty {
        let lib = project
            .library(library)
            .ok_or_else(|| eyre!("no library '{library}'"))?;
        let file =
            lib.create_view_with(cell, Template::XschemSchematic, &project.template_setup())?;
        tracing::info!(target: TARGET, "created the empty schematic {}", file.display());
    }
    if source != Source::Dut {
        let netlisted = match source {
            Source::Other => new.other.clone(),
            _ => format!("{library}/{cell}"),
        };
        match copy_variables(
            project,
            &templates,
            &created.setup,
            library,
            cell,
            &netlisted,
        ) {
            Ok(Some((from, names))) => tracing::info!(
                target: TARGET,
                "design variables copied from {from}, which simulates the same schematic: {}",
                names.join(", ")
            ),
            Ok(None) => {}
            Err(e) => tracing::warn!(target: TARGET, "no design variables copied: {e:#}"),
        }
    }
    show(app, library, cell, view);
    Ok(())
}

pub(crate) fn create_suite(app: &mut AppCtx, new: &NewSuite) -> Result<()> {
    let project = app
        .project
        .as_ref()
        .ok_or_else(|| eyre!("no project is open"))?;
    let (library, cell, view) = (new.library.as_str(), new.cell.trim(), new.view.trim());
    if cell.is_empty() {
        bail!("name the suite's cell");
    }
    let mut temperatures = Vec::new();
    for word in words(&new.temperatures) {
        temperatures.push(
            word.parse::<f64>()
                .map_err(|_| eyre!("'{word}' is not a temperature (a number, °C)"))?,
        );
    }
    let corners = words(&new.corners);
    let known = project.model_corners("ngspice");
    if let Some(odd) = corners.iter().find(|c| !known.contains(c))
        && !known.is_empty()
    {
        bail!(
            "'{odd}' is not a corner of the technology: {}",
            known.join(", ")
        );
    }
    // In the order they are offered, and as short as the suite's place allows.
    let tests = testbenches(project, library)
        .into_iter()
        .filter(|(c, v)| new.tests.contains(&format!("{library}/{c}/{v}")))
        .map(|(c, v)| TestRef {
            view: if c == cell { v } else { format!("{c}/{v}") },
            name: String::new(),
        })
        .collect();
    let suite = Suite {
        description: new.description.trim().to_string(),
        corners,
        temperatures,
        tests,
        ..Default::default()
    };
    let file = suite::create(project, library, cell, view, &suite)?;
    tracing::info!(target: TARGET, "created suite {library}/{cell}/{view}: {}", file.display());
    show(app, library, cell, view);
    Ok(())
}

// --- ways in ---------------------------------------------------------------------------

/// The dialog for a new testbench, when the project has a library to put it in.
pub(crate) fn testbench_dialog(project: &Project, selection: &Selection) -> Option<Dialog> {
    NewTestbench::new(project, selection).map(Dialog::NewTestbench)
}

pub(crate) fn suite_dialog(project: &Project, selection: &Selection) -> Option<Dialog> {
    NewSuite::new(project, selection).map(Dialog::NewSuite)
}

const NO_LIBRARY: &str = "Needs a writable library with the cells layout";

/// "New Testbench…" and "New Suite…" as menu entries; the dialog asked for, if any.
/// `prefix` names them for the guide.
pub(crate) fn menu_items(
    ui: &mut Ui,
    project: Option<&Project>,
    selection: &Selection,
    prefix: &str,
) -> Option<Dialog> {
    let possible = project.is_some_and(|p| home(p, selection).is_some());
    let testbench = ui
        .add_enabled(possible, Button::new("New Testbench…"))
        .on_hover_text("A simulation set-up from a test template")
        .on_disabled_hover_text(NO_LIBRARY);
    guide::anchor(ui, &format!("{prefix}.new_testbench"), &testbench);
    let suite = ui
        .add_enabled(possible, Button::new("New Suite…"))
        .on_hover_text("Several testbenches run together, one table")
        .on_disabled_hover_text(NO_LIBRARY);
    guide::anchor(ui, &format!("{prefix}.new_suite"), &suite);
    let project = project?;
    if testbench.clicked() {
        testbench_dialog(project, selection)
    } else if suite.clicked() {
        suite_dialog(project, selection)
    } else {
        None
    }
}

/// [`menu_items`] where the dialog is opened at once.
pub(crate) fn menu(ui: &mut Ui, app: &mut AppCtx, prefix: &str) {
    if let Some(dialog) = menu_items(ui, app.project.as_ref(), &app.selection, prefix) {
        app.open_dialog(dialog);
        ui.close();
    }
}

// --- test scripts (docs/GUI-TESTING.md) ------------------------------------------------

/// The `sim new_…` commands.
pub(crate) fn script(app: &mut AppCtx, args: &[String]) -> Result<(), String> {
    let verb = args.first().map(String::as_str).unwrap_or("");
    let rest = args.get(1..).unwrap_or_default().join(" ");
    match verb {
        "new_testbench" | "new_suite" => {
            let project = app.project.as_ref().ok_or("no project is open")?;
            let dialog = match verb {
                "new_testbench" => testbench_dialog(project, &app.selection),
                _ => suite_dialog(project, &app.selection),
            }
            .ok_or(NO_LIBRARY)?;
            app.open_dialog(dialog);
            Ok(())
        }
        // new_set <field> <value>: a field of the open dialog, as a user fills it.
        "new_set" => {
            let field = args.get(1).ok_or("usage: sim new_set <field> <value>")?;
            let value = args.get(2..).unwrap_or_default().join(" ");
            match crate::dialogs::open_mut(app) {
                Some(Dialog::NewTestbench(new)) => match field.as_str() {
                    "library" => new.library = value,
                    "cell" => new.cell = value,
                    "view" => {
                        new.view = value;
                        new.view_typed = true;
                    }
                    "template" => {
                        new.template = value;
                        new.args.clear();
                    }
                    "source" => {
                        new.source = match value.as_str() {
                            "dut" => Source::Dut,
                            "other" => Source::Other,
                            "empty" => Source::Empty,
                            "none" => Source::Nothing,
                            other => return Err(format!("unknown source '{other}'")),
                        }
                    }
                    "dut" => new.dut = value,
                    "other" => new.other = value,
                    name => match name.strip_prefix("param.") {
                        Some(name) => {
                            new.args.insert(name.to_string(), value);
                        }
                        None => return Err(format!("no field '{name}'")),
                    },
                },
                Some(Dialog::NewSuite(new)) => match field.as_str() {
                    "cell" => new.cell = value,
                    "view" => new.view = value,
                    "description" => new.description = value,
                    "corners" => new.corners = value,
                    "temperatures" => new.temperatures = value,
                    // `new_set test <cell/view> on|off`
                    "test" => {
                        let (test, state) = value
                            .split_once(' ')
                            .ok_or("usage: sim new_set test <cell/view> on|off")?;
                        let id = format!("{}/{test}", new.library);
                        new.tests.retain(|t| *t != id);
                        if state == "on" {
                            new.tests.push(id);
                        }
                    }
                    name => return Err(format!("no field '{name}'")),
                },
                _ => return Err("no New Testbench or New Suite dialog is open".into()),
            }
            Ok(())
        }
        // new_expect <field> <value>
        "new_expect" => {
            let field = args.get(1).ok_or("usage: sim new_expect <field> <value>")?;
            let want = args.get(2..).unwrap_or_default().join(" ");
            let have = match crate::dialogs::open(app) {
                Some(Dialog::NewTestbench(new)) => match field.as_str() {
                    "cell" => new.cell.clone(),
                    "view" => new.view.clone(),
                    "template" => new.template.clone(),
                    "dut" => new.dut.clone(),
                    name => return Err(format!("no field '{name}'")),
                },
                Some(Dialog::NewSuite(new)) => match field.as_str() {
                    "cell" => new.cell.clone(),
                    "view" => new.view.clone(),
                    "tests" => new.tests.len().to_string(),
                    name => return Err(format!("no field '{name}'")),
                },
                _ => return Err("no New Testbench or New Suite dialog is open".into()),
            };
            if have == want {
                Ok(())
            } else {
                Err(format!("{field} is '{have}', not '{want}'"))
            }
        }
        // new_expect_error <text>: the dialog stayed open with an error that has it.
        "new_expect_error" => match crate::dialogs::open_error(app) {
            Some(error) if error.contains(&rest) => Ok(()),
            other => Err(format!("the dialog's error is {other:?}")),
        },
        other => Err(format!("unknown sim command '{other}'")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_lists() {
        assert_eq!(words(" tt, ff  ss,"), ["tt", "ff", "ss"]);
        assert!(words("  ").is_empty());
    }
}

#[cfg(test)]
mod copy_tests {
    use super::netlisted_cell;

    #[test]
    fn what_a_testbench_netlists() {
        assert_eq!(netlisted_cell("", "work", "tb"), "work/tb");
        assert_eq!(netlisted_cell("lib/other", "work", "tb"), "lib/other");
        assert_eq!(
            netlisted_cell("lib/other/schematic", "work", "tb"),
            "lib/other"
        );
    }
}
