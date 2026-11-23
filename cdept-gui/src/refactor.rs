//! Renaming, copying and moving cells, views and libraries from the window, with what
//! refers to them following (`libcdept::refactor`).
//!
//! The dialogs show the plan before anything is done: which folders and files move,
//! which files are rewritten and how, which views cannot follow. Files open with
//! unsaved changes block the operation. After it, the tabs of files that moved are
//! opened again at their new place.
//!
//! Here: the dialogs that are not in `dialogs.rs` ([`Refactor`]), the plan preview
//! ([`preview`]), applying ([`apply`]), the entries of the library views' context
//! menus, and the script commands that test all this.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::{Id, RichText, Ui};
use eyre::{Result, eyre};
use libcdept::hierarchy::Index;
use libcdept::refactor::{self, ChangeKind, Operation, Options, Plan};
use libcdept::{Cell, Entry, Library, LibraryLayout, Project};

use crate::dialogs::{self, Dialog};
use crate::dock::{DocId, Tab};
use crate::state::{Action, AppCtx, Selection};
use crate::views::Views;

/// An open document, as the shell sees it each frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenFile {
    pub id: DocId,
    pub file: PathBuf,
    /// The tool has changes that are not saved.
    pub modified: bool,
}

/// What the Hierarchy panel is asked to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Show {
    pub library: String,
    pub cell: String,
    /// Its users, not what it is built from.
    pub used_by: bool,
}

/// What this module shares through [`AppCtx`].
#[derive(Debug, Default)]
pub struct Shared {
    /// The open documents, refreshed every frame by the shell.
    pub documents: Vec<OpenFile>,
    /// A request to the Hierarchy panel, taken by it.
    pub show: Option<Show>,
    /// Counts the operations applied: who keeps something read from the files reads
    /// it again.
    pub epoch: u64,
}

/// The open documents, for [`Shared::documents`].
pub fn open_files(views: &Views) -> Vec<OpenFile> {
    views
        .documents()
        .map(|(id, doc)| OpenFile {
            id,
            file: doc.file().to_path_buf(),
            modified: doc.is_modified(),
        })
        .collect()
}

/// The dialogs of this module (the cell rename and copy dialogs are in `dialogs.rs`).
#[derive(Debug, Clone, PartialEq)]
pub enum Refactor {
    MoveCell {
        library: String,
        cell: String,
        to: String,
        name: String,
    },
    /// Copy a cell with the cells of its library it is built from.
    CopyHierarchy {
        library: String,
        cell: String,
        to: String,
        name: String,
    },
    RenameView {
        library: String,
        cell: String,
        view: String,
        name: String,
    },
    CopyView {
        library: String,
        cell: String,
        view: String,
        to_library: String,
        to_cell: String,
        name: String,
    },
    RenameLibrary {
        library: String,
        name: String,
        folder: bool,
    },
}

impl Refactor {
    pub fn title(&self) -> &'static str {
        match self {
            Refactor::MoveCell { .. } => "Move Cell",
            Refactor::CopyHierarchy { .. } => "Copy Cell with Hierarchy",
            Refactor::RenameView { .. } => "Rename View",
            Refactor::CopyView { .. } => "Copy View",
            Refactor::RenameLibrary { .. } => "Rename Library",
        }
    }

    pub fn confirm_label(&self) -> &'static str {
        match self {
            Refactor::MoveCell { .. } => "Move",
            Refactor::CopyHierarchy { .. } | Refactor::CopyView { .. } => "Copy",
            Refactor::RenameView { .. } | Refactor::RenameLibrary { .. } => "Rename",
        }
    }

    pub fn operation(&self) -> Operation {
        match self.clone() {
            Refactor::MoveCell {
                library,
                cell,
                to,
                name,
            } => Operation::MoveCell {
                library,
                cell,
                to_library: to,
                new: name.trim().to_string(),
            },
            Refactor::CopyHierarchy {
                library,
                cell,
                to,
                name,
            } => Operation::CopyCell {
                library,
                cell,
                to_library: to,
                new: name.trim().to_string(),
                hierarchy: true,
            },
            Refactor::RenameView {
                library,
                cell,
                view,
                name,
            } => Operation::RenameView {
                library,
                cell,
                view,
                new: name.trim().to_string(),
            },
            Refactor::CopyView {
                library,
                cell,
                view,
                to_library,
                to_cell,
                name,
            } => Operation::CopyView {
                library,
                cell,
                view,
                to_library,
                to_cell: to_cell.trim().to_string(),
                new: name.trim().to_string(),
            },
            Refactor::RenameLibrary {
                library,
                name,
                folder,
            } => Operation::RenameLibrary {
                library,
                new: name.trim().to_string(),
                folder,
            },
        }
    }

    /// Set a field by name, for test scripts.
    fn set(&mut self, field: &str, value: &str) -> Result<(), String> {
        let text = value.to_string();
        match (self, field) {
            (Refactor::MoveCell { to, .. } | Refactor::CopyHierarchy { to, .. }, "to") => {
                *to = text
            }
            (Refactor::CopyView { to_library, .. }, "to" | "to_library") => *to_library = text,
            (Refactor::CopyView { to_cell, .. }, "to_cell") => *to_cell = text,
            (Refactor::RenameLibrary { folder, .. }, "folder") => {
                *folder = matches!(value, "yes" | "on" | "true")
            }
            (
                Refactor::MoveCell { name, .. }
                | Refactor::CopyHierarchy { name, .. }
                | Refactor::RenameView { name, .. }
                | Refactor::CopyView { name, .. }
                | Refactor::RenameLibrary { name, .. },
                "name",
            ) => *name = text,
            (_, other) => return Err(format!("this dialog has no field '{other}'")),
        }
        Ok(())
    }
}

/// The operation a dialog stands for, when it is one that follows references.
pub fn operation_of(dialog: &Dialog) -> Option<Operation> {
    match dialog {
        Dialog::RenameCell {
            library,
            cell,
            name,
        } => Some(Operation::RenameCell {
            library: library.clone(),
            cell: cell.clone(),
            new: name.trim().to_string(),
        }),
        Dialog::CopyCell {
            library,
            cell,
            to,
            name,
        } => Some(Operation::CopyCell {
            library: library.clone(),
            cell: cell.clone(),
            to_library: to.clone(),
            new: name.trim().to_string(),
            hierarchy: false,
        }),
        Dialog::Refactor(refactor) => Some(refactor.operation()),
        _ => None,
    }
}

fn writable_libraries(project: Option<&Project>) -> Vec<String> {
    project
        .iter()
        .flat_map(|p| &p.libraries)
        .filter(|l| !l.read_only && l.error.is_none())
        .map(|l| l.name.clone())
        .collect()
}

fn library_combo(ui: &mut Ui, id: &str, chosen: &mut String, project: Option<&Project>) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(chosen.as_str())
        .show_ui(ui, |ui| {
            for name in writable_libraries(project) {
                ui.selectable_value(chosen, name.clone(), name);
            }
        });
}

/// The fields of a [`Refactor`] dialog and the plan below them. `focus`: the dialog
/// just opened, its first text field takes the keyboard.
pub fn body(ui: &mut Ui, refactor: &mut Refactor, app: &AppCtx, focus: bool) {
    let project = app.project.as_ref();
    let mut first = focus;
    let mut field = |ui: &mut Ui, label: &str, text: &mut String| {
        ui.label(label);
        let response = ui.add(egui::TextEdit::singleline(text).desired_width(280.0));
        if std::mem::take(&mut first) {
            response.request_focus();
        }
        ui.end_row();
    };
    let grid = egui::Grid::new("refactor-grid")
        .num_columns(2)
        .spacing([8.0, 6.0]);
    match refactor {
        Refactor::MoveCell {
            library,
            cell,
            to,
            name,
        }
        | Refactor::CopyHierarchy {
            library,
            cell,
            to,
            name,
        } => {
            grid.show(ui, |ui| {
                ui.label("Cell");
                ui.label(format!("{library} / {cell}"));
                ui.end_row();
                ui.label("To library");
                library_combo(ui, "refactor-to", to, project);
                ui.end_row();
                field(ui, "Name", name);
            });
        }
        Refactor::RenameView {
            library,
            cell,
            view,
            name,
        } => {
            grid.show(ui, |ui| {
                ui.label("View");
                ui.label(format!("{library} / {cell} / {view}"));
                ui.end_row();
                field(ui, "New name", name);
            });
        }
        Refactor::CopyView {
            library,
            cell,
            view,
            to_library,
            to_cell,
            name,
        } => {
            grid.show(ui, |ui| {
                ui.label("View");
                ui.label(format!("{library} / {cell} / {view}"));
                ui.end_row();
                ui.label("To library");
                library_combo(ui, "refactor-to", to_library, project);
                ui.end_row();
                field(ui, "To cell", to_cell);
                field(ui, "View name", name);
            });
        }
        Refactor::RenameLibrary {
            library,
            name,
            folder,
        } => {
            grid.show(ui, |ui| {
                ui.label("Library");
                ui.label(library.as_str());
                ui.end_row();
                field(ui, "New name", name);
            });
            ui.checkbox(folder, "Also rename the folder").on_hover_text(
                "xschem names a library by its folder: with the folder renamed, the \
                     schematics that use its cells are rewritten. Without, they keep \
                     naming the old folder, which keeps working.",
            );
        }
    }
    preview(ui, app, &refactor.operation());
}

// ---------------------------------------------------------------------------------
// The plan
// ---------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Level {
    Step,
    Detail,
    Note,
    Warning,
    Blocker,
}

/// What the preview shows for an operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Shown {
    lines: Vec<(Level, String)>,
    /// Why there is no plan (a name that is taken, …).
    error: Option<String>,
}

/// The preview kept between frames: the index is read once per opening of a dialog,
/// the plan made again when a field changes.
#[derive(Clone)]
struct Cache {
    root: PathBuf,
    epoch: u64,
    seen: f64,
    index: Arc<Index>,
    operation: Operation,
    shown: Arc<Shown>,
}

fn options(app: &AppCtx) -> Options {
    let busy: BTreeSet<PathBuf> = app
        .refactor
        .documents
        .iter()
        .filter(|d| d.modified)
        .map(|d| d.file.clone())
        .collect();
    Options {
        busy,
        ..Options::default()
    }
}

pub(crate) fn shown(project: &Project, plan: Result<Plan>) -> Shown {
    let plan = match plan {
        Ok(plan) => plan,
        Err(e) => {
            return Shown {
                lines: Vec::new(),
                error: Some(format!("{e:#}")),
            };
        }
    };
    let show = |p: &Path| {
        p.strip_prefix(&project.root)
            .unwrap_or(p)
            .display()
            .to_string()
    };
    let mut lines = Vec::new();
    for change in &plan.changes {
        let mut text = format!("{} {}", change.kind.name(), show(&change.path));
        if let Some(to) = &change.to {
            text.push_str(&format!(" → {}", show(to)));
        }
        if !change.label.is_empty() {
            text = format!("{} {}", change.kind.name(), change.label);
        }
        let level = match change.kind {
            ChangeKind::NotFollowed => Level::Warning,
            _ => Level::Step,
        };
        lines.push((level, text));
        for detail in &change.details {
            lines.push((Level::Detail, detail.clone()));
        }
    }
    for note in &plan.notes {
        lines.push((Level::Note, note.clone()));
    }
    for blocker in &plan.blockers {
        lines.push((Level::Blocker, blocker.clone()));
    }
    Shown { lines, error: None }
}

/// The plan of an operation, from the cache when nothing changed.
fn planned(ui: &Ui, app: &AppCtx, operation: &Operation) -> Option<Arc<Shown>> {
    let project = app.project.as_ref()?;
    let id = Id::new("cdept-refactor-preview");
    let now = ui.input(|i| i.time);
    let cached: Option<Cache> = ui.data(|d| d.get_temp(id));
    // The index is kept while the dialog stays open (it is drawn every frame it gets).
    let cached = cached
        .filter(|c| c.root == project.root && c.epoch == app.refactor.epoch && now - c.seen < 1.5);
    let cache = match cached {
        Some(mut c) => {
            if c.operation != *operation {
                let plan = refactor::plan_with(project, &c.index, operation, &options(app));
                c.shown = Arc::new(shown(project, plan));
                c.operation = operation.clone();
            }
            c.seen = now;
            c
        }
        None => {
            let index = Arc::new(Index::build(project));
            let plan = refactor::plan_with(project, &index, operation, &options(app));
            Cache {
                root: project.root.clone(),
                epoch: app.refactor.epoch,
                seen: now,
                index,
                operation: operation.clone(),
                shown: Arc::new(shown(project, plan)),
            }
        }
    };
    let result = cache.shown.clone();
    ui.data_mut(|d| d.insert_temp(id, cache));
    Some(result)
}

/// Show what an operation will do, below the fields of its dialog.
pub fn preview(ui: &mut Ui, app: &AppCtx, operation: &Operation) {
    let Some(shown) = planned(ui, app, operation) else {
        return;
    };
    ui.add_space(6.0);
    draw(ui, &shown);
}

/// Draw a plan that was made by someone else (the import dialog).
pub(crate) fn draw(ui: &mut Ui, shown: &Shown) {
    if let Some(error) = &shown.error {
        ui.weak(error);
        return;
    }
    ui.label(RichText::new("What will be done:").small().strong());
    egui::ScrollArea::vertical()
        .id_salt("refactor-plan")
        .max_height(200.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for (level, text) in &shown.lines {
                let text = RichText::new(text).small();
                match level {
                    Level::Step => ui.label(text),
                    Level::Detail => {
                        ui.horizontal(|ui| {
                            ui.add_space(14.0);
                            ui.weak(text)
                        })
                        .inner
                    }
                    Level::Note => ui.weak(text),
                    Level::Warning => ui.colored_label(ui.visuals().warn_fg_color, text),
                    Level::Blocker => ui.colored_label(ui.visuals().error_fg_color, text),
                };
            }
        });
}

/// The lines of the plan of the open dialog, for test scripts.
fn plan_lines(app: &AppCtx) -> Result<Vec<String>, String> {
    let project = app.project.as_ref().ok_or("no project is open")?;
    let dialog = dialogs::open(app).ok_or("no dialog is open")?;
    if let Dialog::ImportFiles(import) = dialog {
        return crate::import_files::plan_lines(app, import);
    }
    if let Dialog::ImportSummary(lines) = dialog {
        return Ok(lines.clone());
    }
    let operation = operation_of(dialog).ok_or("the open dialog has no plan")?;
    let plan = refactor::plan(project, &operation, &options(app)).map_err(|e| format!("{e:#}"))?;
    Ok(plan.lines(&project.root))
}

/// Where a file is after the moves of a plan.
fn after_moves(plan: &Plan, file: &Path) -> Option<PathBuf> {
    let mut current = file.to_path_buf();
    let mut moved = false;
    for (from, to) in plan.moved() {
        if let Ok(rest) = current.strip_prefix(from) {
            current = if rest.as_os_str().is_empty() {
                to.to_path_buf()
            } else {
                to.join(rest)
            };
            moved = true;
        }
    }
    moved.then_some(current)
}

/// Run an operation on the open project: plan it with what is known about the open
/// documents, apply it, read the libraries again, select the result.
pub fn apply(app: &mut AppCtx, operation: &Operation) -> Result<()> {
    const TARGET: &str = "cdept::library";
    let options = options(app);
    let project = app
        .project
        .as_mut()
        .ok_or_else(|| eyre!("no project is open"))?;
    let plan = refactor::plan(project, operation, &options)?;
    let outcome = refactor::apply(project, &plan)?;
    for line in &outcome.lines {
        tracing::info!(target: TARGET, "{line}");
    }
    for note in &plan.notes {
        tracing::info!(target: TARGET, "{note}");
    }
    for problem in &outcome.problems {
        tracing::warn!(target: TARGET, "{problem}");
    }
    if plan.reload {
        project.reload()?;
    } else {
        project.rescan();
    }
    // The tabs of files that moved: opened again where the files are now.
    let reopen: Vec<(DocId, PathBuf)> = app
        .refactor
        .documents
        .iter()
        .filter_map(|d| Some((d.id, after_moves(&plan, &d.file)?)))
        .collect();
    for (id, file) in reopen {
        app.request(Action::CloseTab(Tab::Document(id)));
        app.request(Action::OpenFile(file));
    }
    app.refactor.epoch += 1;
    app.refresh_links();
    let result = &plan.result;
    app.selection = match (result.cell.is_empty(), result.view.is_empty()) {
        (true, _) => Selection::library(&result.library),
        (false, true) => Selection::cell(&result.library, &result.cell),
        (false, false) => Selection::view(&result.library, &result.cell, &result.view),
    };
    app.validate_selection();
    Ok(())
}

// ---------------------------------------------------------------------------------
// Context menus of the library views
// ---------------------------------------------------------------------------------

const READ_ONLY: &str = "The library is read-only";

fn writable(lib: &Library) -> bool {
    !lib.read_only && lib.error.is_none()
}

/// Another writable library to offer first, else the same.
fn other_library(project: &Project, lib: &Library) -> String {
    project
        .libraries
        .iter()
        .find(|l| writable(l) && l.name != lib.name)
        .or_else(|| project.libraries.iter().find(|l| writable(l)))
        .map(|l| l.name.clone())
        .unwrap_or_default()
}

/// Entries of a cell's context menu: Used By, Move, Copy with Hierarchy.
pub fn cell_menu(
    ui: &mut Ui,
    project: &Project,
    lib: &Library,
    cell: &Cell,
) -> (Option<Dialog>, Option<Action>) {
    let mut dialog = None;
    let mut action = None;
    if ui
        .button("Used By…")
        .on_hover_text("Which cells place this one, in the Hierarchy panel")
        .clicked()
    {
        action = Some(Action::UsedBy(Selection::cell(&lib.name, &cell.name)));
    }
    if ui
        .add_enabled(writable(lib), egui::Button::new("Move Cell…"))
        .on_hover_text("Into another library; what uses it follows")
        .on_disabled_hover_text(READ_ONLY)
        .clicked()
    {
        dialog = Some(Dialog::Refactor(Refactor::MoveCell {
            library: lib.name.clone(),
            cell: cell.name.clone(),
            to: other_library(project, lib),
            name: cell.name.clone(),
        }));
    }
    if ui
        .button("Copy with Hierarchy…")
        .on_hover_text(
            "Into another library, with the cells of this library it is built from; the \
             copies use each other",
        )
        .clicked()
    {
        dialog = Some(Dialog::Refactor(Refactor::CopyHierarchy {
            library: lib.name.clone(),
            cell: cell.name.clone(),
            to: other_library(project, lib),
            name: cell.name.clone(),
        }));
    }
    (dialog, action)
}

/// Entries of a view's context menu: Rename, Copy.
pub fn view_menu(ui: &mut Ui, lib: &Library, cell: &Cell, entry: &Entry) -> Option<Dialog> {
    let mut dialog = None;
    let in_folder = lib.layout == LibraryLayout::Cells && entry.path.is_dir();
    if ui
        .add_enabled(
            writable(lib) && in_folder,
            egui::Button::new("Rename View…"),
        )
        .on_disabled_hover_text(if writable(lib) {
            "A view that is a single file is named after its kind of file"
        } else {
            READ_ONLY
        })
        .clicked()
    {
        dialog = Some(Dialog::Refactor(Refactor::RenameView {
            library: lib.name.clone(),
            cell: cell.name.clone(),
            view: entry.name.clone(),
            name: entry.name.clone(),
        }));
    }
    if ui.button("Copy View…").clicked() {
        dialog = Some(Dialog::Refactor(Refactor::CopyView {
            library: lib.name.clone(),
            cell: cell.name.clone(),
            view: entry.name.clone(),
            to_library: lib.name.clone(),
            to_cell: cell.name.clone(),
            name: format!("{}_copy", entry.name),
        }));
    }
    dialog
}

/// Entry of a library's context menu: Rename.
pub fn library_menu(ui: &mut Ui, lib: &Library) -> Option<Dialog> {
    let clicked = ui
        .add_enabled(lib.error.is_none(), egui::Button::new("Rename Library…"))
        .on_hover_text("In the project file, and optionally its folder; what names it follows")
        .clicked();
    clicked.then(|| rename_library_dialog(lib))
}

fn rename_library_dialog(lib: &Library) -> Dialog {
    let folder_named_alike = lib
        .path
        .file_name()
        .is_some_and(|name| name.to_string_lossy() == lib.name);
    Dialog::Refactor(Refactor::RenameLibrary {
        library: lib.name.clone(),
        name: lib.name.clone(),
        // Keep the folder and the library named alike when they are, unless the
        // folder may not be touched.
        folder: folder_named_alike && !lib.read_only,
    })
}

// ---------------------------------------------------------------------------------
// Test scripts
// ---------------------------------------------------------------------------------

/// Script commands of this module (docs/GUI-TESTING.md).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ScriptCommand {
    /// `refactor <what> <names…>`: open the dialog an entry of the context menu opens.
    Open(Dialog),
    /// `refactor_set <field> <value>`: a field of the open dialog.
    Set(String, String),
    /// `expect_plan <text>`: a line of the open dialog's plan has this text.
    ExpectPlan(String),
    /// `refactor_ok`: confirm the open dialog; it must go through.
    Confirm,
    /// `refactor_fails <text>`: confirm the open dialog; it must fail with this text.
    Fails(String),
    /// `refactor_cancel`.
    Cancel,
    /// `used_by <library> <cell>`: what Used By… does.
    UsedBy(String, String),
    /// `hierarchy_mode <schematic|layout|netlist|hdl|used-by> [files|instances]`.
    Mode(String, Option<String>),
    /// `hierarchy_of <library> <cell>` / `hierarchy_of selection`.
    Of(Option<(String, String)>),
}

const USAGE: &str = "usage: refactor rename_cell <library> <cell> <name> | copy_cell <library> \
                     <cell> <to library> <name> | move_cell <library> <cell> <to library> \
                     [name] | copy_hierarchy <library> <cell> <to library> [name] | \
                     rename_view <library> <cell> <view> <name> | copy_view <library> <cell> \
                     <view> <to library> <to cell> <name> | rename_library <library> <name> \
                     [folder] | import_files <library>";

pub(crate) fn script_parse(
    name: &str,
    args: &[&str],
    rest: &str,
) -> Option<Result<ScriptCommand, String>> {
    let text = || {
        if rest.is_empty() {
            Err(format!("'{name}' needs an argument"))
        } else {
            Ok(rest.to_string())
        }
    };
    let s = |n: usize| args[n].to_string();
    let command = match name {
        "refactor" => {
            let dialog = match args {
                ["rename_cell", _, _, _] => Dialog::RenameCell {
                    library: s(1),
                    cell: s(2),
                    name: s(3),
                },
                ["copy_cell", _, _, _, _] => Dialog::CopyCell {
                    library: s(1),
                    cell: s(2),
                    to: s(3),
                    name: s(4),
                },
                ["move_cell", _, _, _] | ["move_cell", _, _, _, _] => {
                    Dialog::Refactor(Refactor::MoveCell {
                        library: s(1),
                        cell: s(2),
                        to: s(3),
                        name: args.get(4).unwrap_or(&args[2]).to_string(),
                    })
                }
                ["copy_hierarchy", _, _, _] | ["copy_hierarchy", _, _, _, _] => {
                    Dialog::Refactor(Refactor::CopyHierarchy {
                        library: s(1),
                        cell: s(2),
                        to: s(3),
                        name: args.get(4).unwrap_or(&args[2]).to_string(),
                    })
                }
                ["rename_view", _, _, _, _] => Dialog::Refactor(Refactor::RenameView {
                    library: s(1),
                    cell: s(2),
                    view: s(3),
                    name: s(4),
                }),
                ["copy_view", _, _, _, _, _, _] => Dialog::Refactor(Refactor::CopyView {
                    library: s(1),
                    cell: s(2),
                    view: s(3),
                    to_library: s(4),
                    to_cell: s(5),
                    name: s(6),
                }),
                ["rename_library", _, _] | ["rename_library", _, _, "folder"] => {
                    Dialog::Refactor(Refactor::RenameLibrary {
                        library: s(1),
                        name: s(2),
                        folder: args.len() == 4,
                    })
                }
                ["import_files", _] => Dialog::ImportFiles(crate::import_files::ImportFiles::new(args[1])),
                _ => return Some(Err(USAGE.into())),
            };
            Ok(ScriptCommand::Open(dialog))
        }
        "refactor_set" => match args {
            [field, _, ..] => Ok(ScriptCommand::Set(
                field.to_string(),
                rest[field.len()..].trim().to_string(),
            )),
            _ => Err("usage: refactor_set <field> <value>".into()),
        },
        "expect_plan" => text().map(ScriptCommand::ExpectPlan),
        "refactor_ok" => Ok(ScriptCommand::Confirm),
        "refactor_fails" => text().map(ScriptCommand::Fails),
        "refactor_cancel" => Ok(ScriptCommand::Cancel),
        "used_by" => match args {
            [library, cell] => Ok(ScriptCommand::UsedBy(library.to_string(), cell.to_string())),
            _ => Err("usage: used_by <library> <cell>".into()),
        },
        "hierarchy_mode" => match args {
            [mode] => Ok(ScriptCommand::Mode(mode.to_string(), None)),
            [mode, source] => Ok(ScriptCommand::Mode(
                mode.to_string(),
                Some(source.to_string()),
            )),
            _ => Err(
                "usage: hierarchy_mode <schematic|layout|netlist|hdl|used-by> [files|instances]"
                    .into(),
            ),
        },
        "hierarchy_of" => match args {
            ["selection"] => Ok(ScriptCommand::Of(None)),
            [library, cell] => Ok(ScriptCommand::Of(Some((
                library.to_string(),
                cell.to_string(),
            )))),
            _ => Err("usage: hierarchy_of <library> <cell> | selection".into()),
        },
        _ => return None,
    };
    Some(command)
}

pub(crate) fn script_run(
    command: ScriptCommand,
    app: &mut AppCtx,
    views: &mut Views,
) -> Result<(), String> {
    match command {
        ScriptCommand::Open(dialog) => {
            app.window = crate::dock::WindowId::Main;
            app.open_dialog(dialog);
        }
        ScriptCommand::Set(field, value) => {
            match dialogs::open_mut(app).ok_or("no dialog is open")? {
                Dialog::Refactor(refactor) => refactor.set(&field, &value)?,
                Dialog::ImportFiles(import) => import.set(&field, &value)?,
                Dialog::RenameCell { name, .. } if field == "name" => *name = value,
                Dialog::CopyCell { name, .. } if field == "name" => *name = value,
                Dialog::CopyCell { to, .. } if field == "to" => *to = value,
                _ => return Err(format!("the open dialog has no field '{field}'")),
            }
        }
        ScriptCommand::ExpectPlan(text) => {
            let lines = plan_lines(app)?;
            if !lines.iter().any(|line| line.contains(&text)) {
                return Err(format!("no '{text}' in the plan: {lines:#?}"));
            }
        }
        ScriptCommand::Confirm => {
            dialogs::confirm_open(app).map_err(|e| format!("{e:#}"))?;
        }
        ScriptCommand::Fails(text) => match dialogs::confirm_open(app) {
            Ok(()) => return Err("the operation went through".into()),
            Err(e) => {
                let error = format!("{e:#}");
                if !error.contains(&text) {
                    return Err(format!("it failed with '{error}', not '{text}'"));
                }
            }
        },
        ScriptCommand::Cancel => {
            if app.dialog.take().is_none() {
                return Err("no dialog is open".into());
            }
        }
        ScriptCommand::UsedBy(library, cell) => {
            app.request(Action::UsedBy(Selection::cell(&library, &cell)));
        }
        ScriptCommand::Mode(mode, source) => {
            views
                .hierarchy_mut()
                .script_mode(&mode, source.as_deref())?;
        }
        ScriptCommand::Of(cell) => views.hierarchy_mut().pin(cell),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use libcdept::PROJECT_FILE;

    use super::*;
    use crate::log::LogBuffer;
    use crate::state::Settings;

    const SYMBOL: &str = "v {xschem version=3.4.5 file_version=1.2\n}\nG {}\nK {type=subcircuit\n\
                          schematic=analog/@symname/schematic/@symname.sch\n}\n";

    /// A project with `analog` (cells `inv` and `buf`, buf placing inv twice) and an
    /// empty library `digital`.
    fn app(root: &Path) -> AppCtx {
        let cell = |name: &str, schematic: &str| {
            let dir = root.join("libs/analog").join(name);
            fs::create_dir_all(dir.join("symbol")).unwrap();
            fs::create_dir_all(dir.join("schematic")).unwrap();
            fs::write(dir.join(format!("schematic/{name}.sch")), schematic).unwrap();
            fs::write(dir.join(format!("symbol/{name}.sym")), SYMBOL).unwrap();
        };
        cell("inv", "v {}\n");
        cell(
            "buf",
            "v {}\nC {analog/inv/symbol/inv.sym} 0 0 0 0 {name=x1}\n\
             C {analog/inv/symbol/inv.sym} 9 0 0 0 {name=x2}\n",
        );
        fs::create_dir_all(root.join("libs/digital")).unwrap();
        fs::write(root.join("libs/digital/lib.toml"), "").unwrap();
        fs::write(
            root.join(PROJECT_FILE),
            "name = \"demo\"\n[libraries]\nanalog = \"libs/analog\"\ndigital = \"libs/digital\"\n",
        )
        .unwrap();
        let mut app = AppCtx::new(Settings::default(), LogBuffer::new());
        app.load_project(root);
        assert!(app.dialog.is_none(), "the project loads");
        app
    }

    fn buf(root: &Path) -> String {
        fs::read_to_string(root.join("libs/analog/buf/schematic/buf.sch")).unwrap()
    }

    #[test]
    fn the_rename_dialog_takes_the_users_along() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let mut app = app(&root);
        app.open_dialog(Dialog::RenameCell {
            library: "analog".into(),
            cell: "inv".into(),
            name: "inv_x1".into(),
        });
        let lines = plan_lines(&app).unwrap();
        assert!(
            lines
                .iter()
                .any(|l| l.contains("2 instance(s) of analog/inv/symbol/inv.sym")),
            "{lines:#?}"
        );
        dialogs::confirm_open(&mut app).unwrap();
        assert_eq!(
            buf(&root)
                .matches("analog/inv_x1/symbol/inv_x1.sym")
                .count(),
            2
        );
        assert_eq!(app.selection, Selection::cell("analog", "inv_x1"));
        assert_eq!(app.refactor.epoch, 1);
        assert!(app.links.is_empty(), "{:?}", app.links);
    }

    #[test]
    fn unsaved_documents_block_and_moved_ones_are_reopened() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let mut app = app(&root);
        let user = root.join("libs/analog/buf/schematic/buf.sch");
        let own = root.join("libs/analog/inv/schematic/inv.sch");
        app.refactor.documents = vec![
            OpenFile {
                id: DocId(1),
                file: user.clone(),
                modified: true,
            },
            OpenFile {
                id: DocId(2),
                file: own,
                modified: false,
            },
        ];
        let operation = Operation::MoveCell {
            library: "analog".into(),
            cell: "inv".into(),
            to_library: "digital".into(),
            new: "inv".into(),
        };
        let error = apply(&mut app, &operation).unwrap_err();
        assert!(
            format!("{error:#}").contains("unsaved changes"),
            "{error:#}"
        );
        assert!(root.join("libs/analog/inv").is_dir());

        app.refactor.documents[0].modified = false;
        apply(&mut app, &operation).unwrap();
        assert_eq!(buf(&root).matches("digital/inv/symbol/inv.sym").count(), 2);
        // The tab of the moved schematic: closed and opened at its new place.
        let actions = app.take_actions();
        assert_eq!(
            actions,
            [
                Action::CloseTab(Tab::Document(DocId(2))),
                Action::OpenFile(root.join("libs/digital/inv/schematic/inv.sch")),
            ]
        );
    }

    #[test]
    fn the_dialogs_and_the_hierarchy_panel_draw() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let mut app = app(&root);
        let mut views = Views::default();
        let project = app.project.clone().unwrap();
        let analog = project.library("analog").unwrap();
        let buf_cell = analog.cell("buf").unwrap();
        let ctx = egui::Context::default();
        let frame = |app: &mut AppCtx, views: &mut Views| {
            let output = ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.menu_button("cell", |ui| {
                    let _ = cell_menu(ui, &project, analog, buf_cell);
                    let _ = view_menu(ui, analog, buf_cell, &buf_cell.entries[0]);
                    let _ = library_menu(ui, analog);
                });
                views.get(Tab::Hierarchy).ui(ui, app);
                dialogs::show(ui.ctx(), app);
            });
            output.drop_without_applying_deltas();
        };
        let text = |s: &str| s.to_string();
        let all = [
            Refactor::MoveCell {
                library: text("analog"),
                cell: text("inv"),
                to: text("digital"),
                name: text("inv"),
            },
            Refactor::CopyHierarchy {
                library: text("analog"),
                cell: text("buf"),
                to: text("digital"),
                name: text("buf"),
            },
            Refactor::RenameView {
                library: text("analog"),
                cell: text("inv"),
                view: text("schematic"),
                name: text("sch2"),
            },
            Refactor::CopyView {
                library: text("analog"),
                cell: text("inv"),
                view: text("symbol"),
                to_library: text("digital"),
                to_cell: text("inv"),
                name: text("symbol"),
            },
            Refactor::RenameLibrary {
                library: text("analog"),
                name: text("core"),
                folder: true,
            },
            // A name that is taken: the dialog says so instead of a plan.
            Refactor::RenameView {
                library: text("analog"),
                cell: text("inv"),
                view: text("schematic"),
                name: text("symbol"),
            },
        ];
        for refactor in all {
            assert!(!refactor.title().is_empty() && !refactor.confirm_label().is_empty());
            app.open_dialog(Dialog::Refactor(refactor));
            frame(&mut app, &mut views);
            frame(&mut app, &mut views);
            assert!(app.dialog.take().is_some());
        }

        // The Hierarchy panel, from the files: what buf is built from, who uses inv.
        app.selection = Selection::cell("analog", "buf");
        views
            .hierarchy_mut()
            .script_mode("schematic", Some("files"))
            .unwrap();
        let wait = |app: &mut AppCtx, views: &mut Views, wanted: &str| {
            for _ in 0..200 {
                let output = ctx.run_ui(egui::RawInput::default(), |ui| {
                    views.get(Tab::Hierarchy).ui(ui, app)
                });
                output.drop_without_applying_deltas();
                if views.hierarchy().lines().iter().any(|l| l.trim() == wanted) {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            panic!("no '{wanted}' in {:?}", views.hierarchy().lines());
        };
        wait(&mut app, &mut views, "inv ×2 (x1, x2)");
        assert!(views.hierarchy().schematic_of("inv").is_some());
        app.refactor.show = Some(Show {
            library: text("analog"),
            cell: text("inv"),
            used_by: true,
        });
        wait(
            &mut app,
            &mut views,
            "analog/buf/schematic  instance ×2 (x1, x2)",
        );
        views.hierarchy_mut().pin(None);
        views.hierarchy_mut().script_mode("layout", None).unwrap();
        frame(&mut app, &mut views);
        assert!(views.hierarchy().lines().is_empty());
        assert!(views.hierarchy_mut().script_mode("sideways", None).is_err());
    }

    #[test]
    fn a_library_is_renamed_with_its_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let mut app = app(&root);
        let lib = app.project.as_ref().unwrap().library("analog").unwrap();
        let Dialog::Refactor(mut dialog) = rename_library_dialog(lib) else {
            unreachable!()
        };
        assert!(matches!(
            dialog,
            Refactor::RenameLibrary { folder: true, .. }
        ));
        dialog.set("name", "core").unwrap();
        apply(&mut app, &dialog.operation()).unwrap();
        let project = app.project.as_ref().unwrap();
        assert!(project.library("core").is_some() && project.library("analog").is_none());
        let sch = fs::read_to_string(root.join("libs/core/buf/schematic/buf.sch")).unwrap();
        assert_eq!(sch.matches("core/inv/symbol/inv.sym").count(), 2);
        assert_eq!(app.selection, Selection::library("core"));
    }
}
