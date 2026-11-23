//! Library definitions in the GUI: the "Library Properties" dialog, the list of what
//! is wrong with the project file, hints about library folders, the count of files
//! CDePT does not recognise. Also the test-script commands for these and for the
//! path prompts (docs/GUI-TESTING.md, "Library definitions and path prompts").
//!
//! The dialogs themselves are variants of [`crate::dialogs::Dialog`]
//! (`LibraryProperties`, `Problems`); this file draws and applies them.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui::{Context, Id, RichText, Ui};
use eyre::{Result, eyre};
use libcdept::project_edit::{LibraryChange, LibraryDefinition, Origin};
use libcdept::{LibraryLayout, Project, formats};

use crate::dialogs::{self, Dialog};
use crate::dock::WindowId;
use crate::pathfield::{self, PathField};
use crate::state::{Action, AppCtx, PickFor};

/// How long a count of unrecognised files is good for before the folder is read
/// again.
const RECOUNT: Duration = Duration::from_secs(2);
/// The layouts the dialog offers, by name (what [`LibraryLayout::from_name`] knows).
const LAYOUTS: [&str; 3] = ["cells", "flat", "ref"];

/// The "Library Properties" dialog: a library definition being edited.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Properties {
    pub library: String,
    /// The definition as it is written now; `None` until it is read.
    was: Option<LibraryDefinition>,
    /// Why the definition cannot be edited (a library the PDK brings).
    fixed: Option<String>,
    pub path: String,
    pub read_only: bool,
    pub layout: Option<LibraryLayout>,
    pub description: String,
}

impl Properties {
    /// For library `name`; its definition is read when the dialog is first drawn.
    pub fn new(name: &str) -> Self {
        Self {
            library: name.to_string(),
            ..Self::default()
        }
    }

    /// For library `name`, with its definition read now.
    pub fn open(project: &Project, name: &str) -> Self {
        let mut properties = Self::new(name);
        properties.load(project);
        properties
    }

    fn loaded(&self) -> bool {
        self.was.is_some() || self.fixed.is_some()
    }

    fn load(&mut self, project: &Project) {
        if self.loaded() {
            return;
        }
        match project.library_definition(&self.library) {
            Ok(definition) => {
                self.path = definition.path.clone();
                self.read_only = definition.read_only;
                self.layout = definition.layout;
                self.description = definition.description.clone();
                self.was = Some(definition);
            }
            Err(error) => self.fixed = Some(format!("{error:#}")),
        }
    }

    /// What the dialog changes against what is written.
    fn change(&self) -> LibraryChange {
        let Some(was) = &self.was else {
            return LibraryChange::default();
        };
        LibraryChange {
            path: (self.path.trim() != was.path).then(|| self.path.trim().to_string()),
            read_only: (self.read_only != was.read_only).then_some(self.read_only),
            layout: (self.layout != was.layout).then_some(self.layout),
            description: (self.description.trim() != was.description)
                .then(|| self.description.trim().to_string()),
        }
    }

    /// Set a field by name, as the dialog's widgets do (for test scripts).
    fn set(&mut self, field: &str, value: &str) -> Result<(), String> {
        match field {
            "path" | "folder" => self.path = value.to_string(),
            "read_only" => self.read_only = yes_no(value)?,
            "layout" => self.layout = layout_named(value)?,
            "description" => self.description = value.to_string(),
            other => return Err(format!("the library properties have no field '{other}'")),
        }
        Ok(())
    }
}

fn yes_no(text: &str) -> Result<bool, String> {
    match text {
        "yes" | "true" | "on" => Ok(true),
        "no" | "false" | "off" => Ok(false),
        other => Err(format!("'{other}' is not yes or no")),
    }
}

fn layout_named(text: &str) -> Result<Option<LibraryLayout>, String> {
    if text == "detect" {
        return Ok(None);
    }
    LibraryLayout::from_name(text)
        .map(Some)
        .ok_or_else(|| format!("'{text}' is not a library layout"))
}

/// The folder a definition's relative path is relative to.
fn base_of(project: &Project, file: &Path) -> PathBuf {
    file.parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| project.root.clone())
}

/// A line in the warning colour.
fn warn(ui: &mut Ui, text: impl Into<String>) {
    ui.label(
        RichText::new(text)
            .small()
            .color(ui.visuals().warn_fg_color),
    );
}

/// The body of the "Library Properties" dialog.
pub(crate) fn body(ui: &mut Ui, properties: &mut Properties, app: &AppCtx, focus: bool) {
    let Some(project) = app.project.as_ref() else {
        ui.label("No project is open.");
        return;
    };
    properties.load(project);
    let origin = project.library_origin(&properties.library);
    let where_defined = origin
        .as_ref()
        .map(|o| o.describe(project))
        .unwrap_or_default();
    if let Some(reason) = &properties.fixed {
        egui::Grid::new("library-fixed")
            .num_columns(2)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                ui.label("Library");
                ui.label(properties.library.as_str());
                ui.end_row();
                ui.label("Defined in");
                ui.label(RichText::new(where_defined.as_str()).small());
                ui.end_row();
                if let Some(library) = project.library(&properties.library) {
                    ui.label("Folder");
                    ui.label(RichText::new(library.path.display().to_string()).small());
                    ui.end_row();
                    ui.label("Layout");
                    ui.label(dialogs::layout_label(Some(library.layout)));
                    ui.end_row();
                }
            });
        ui.add_space(4.0);
        ui.label(reason.as_str());
        return;
    }
    let Some(was) = properties.was.clone() else {
        return;
    };
    let base = base_of(project, &was.file);
    let vars = |name: &str| project.var(name);
    egui::Grid::new("library-properties")
        .num_columns(2)
        .spacing([8.0, 6.0])
        .show(ui, |ui| {
            ui.label("Library");
            ui.label(properties.library.as_str());
            ui.end_row();

            ui.label("Defined in");
            ui.label(RichText::new(where_defined.as_str()).small());
            ui.end_row();

            ui.label("Folder");
            ui.vertical(|ui| {
                PathField::folder(
                    ("library-folder", &properties.library),
                    &mut properties.path,
                )
                .base(&base)
                .vars(&vars)
                .title("Library folder")
                .focus(focus)
                .show(ui);
            });
            ui.end_row();

            ui.label("Layout");
            egui::ComboBox::from_id_salt("library-layout")
                .selected_text(dialogs::layout_label(properties.layout))
                .show_ui(ui, |ui| {
                    let options = std::iter::once(None)
                        .chain(LAYOUTS.iter().map(|n| LibraryLayout::from_name(n)))
                        .collect::<Vec<_>>();
                    for (n, option) in options.into_iter().enumerate() {
                        if n > 0 && option.is_none() {
                            continue;
                        }
                        ui.selectable_value(
                            &mut properties.layout,
                            option,
                            dialogs::layout_label(option),
                        );
                    }
                });
            ui.end_row();

            ui.label("");
            ui.checkbox(&mut properties.read_only, "Read-only (reference library)");
            ui.end_row();

            ui.label("Description");
            ui.add_enabled(
                !properties.read_only,
                egui::TextEdit::singleline(&mut properties.description).desired_width(280.0),
            )
            .on_disabled_hover_text(
                "The description is kept in the lib.toml of the library folder, which \
                 CDePT does not change for a read-only library.",
            );
            ui.end_row();
        });

    // What the folder typed means, and what is in its way.
    match pathfield::resolve(&properties.path, Some(&base), Some(&vars)) {
        Some(folder) if folder.is_dir() => {
            if properties.path.trim() != was.path {
                ui.small(format!("Folder: {}", folder.display()));
            }
        }
        Some(folder) => warn(ui, format!("Folder not found: {}", folder.display())),
        None => warn(ui, "A $VARIABLE in the folder is not set."),
    }
    if was.included {
        ui.small(
            "This library is defined in an included file: the change is written there, \
             and the folder is relative to that file.",
        );
    } else if properties.path.trim() != was.path
        && let Some(clash) = project.folder_clash(&properties.library, &properties.path)
    {
        warn(ui, clash);
    }
    if properties.layout.is_none()
        && let Some(library) = project.library(&properties.library)
    {
        ui.small(format!(
            "Detected now: {}. Saying the layout keeps it from changing when files are \
             added to the folder.",
            library.layout.name()
        ));
    }
    if let Some(library) = project.library(&properties.library) {
        unrecognised_line(ui, library, None);
    }
}

/// Save the "Library Properties" dialog.
pub(crate) fn apply(properties: &Properties, app: &mut AppCtx) -> Result<()> {
    let project = app
        .project
        .as_mut()
        .ok_or_else(|| eyre!("no project is open"))?;
    let mut properties = properties.clone();
    let untouched = !properties.loaded();
    properties.load(project);
    if let Some(reason) = &properties.fixed {
        return Err(eyre!("{reason}"));
    }
    // Opened without being drawn: there is nothing the user changed.
    let change = if untouched {
        LibraryChange::default()
    } else {
        properties.change()
    };
    if change.is_empty() {
        return Ok(());
    }
    let name = properties.library.as_str();
    if let Some(path) = &change.path
        && !properties.was.as_ref().is_some_and(|w| w.included)
        && let Some(clash) = project.folder_clash(name, path)
    {
        tracing::warn!(target: "cdept::library", "library '{name}': {clash}");
    }
    project.set_library(name, &change)?;
    let file = project
        .library(name)
        .map(|l| l.defined_in.display().to_string())
        .unwrap_or_default();
    tracing::info!(target: "cdept::library", "changed the definition of library '{name}' in {file}");
    for warning in &project.warnings {
        tracing::warn!(target: "cdept::project", "{warning}");
    }
    app.refresh_links();
    app.validate_selection();
    Ok(())
}

/// Everything that is wrong with the project as loaded: the warnings of the project
/// file and the libraries that cannot be read.
pub(crate) fn problems_of(project: &Project) -> Vec<String> {
    let mut problems = project.warnings.clone();
    for library in &project.libraries {
        if let Some(error) = &library.error {
            problems.push(format!("library '{}': {error}", library.name));
        }
    }
    problems
}

/// The body of the "Project Problems" dialog.
pub(crate) fn problems_body(ui: &mut Ui, app: &AppCtx) {
    let Some(project) = app.project.as_ref() else {
        ui.label("No project is open.");
        return;
    };
    let problems = problems_of(project);
    if problems.is_empty() {
        ui.label("Nothing is wrong with the project file and its libraries.");
        return;
    }
    ui.label(format!(
        "{} found while loading {}:",
        count(problems.len(), "problem"),
        project.file.display()
    ));
    ui.add_space(4.0);
    egui::ScrollArea::vertical()
        .max_height(320.0)
        .show(ui, |ui| {
            for problem in &problems {
                ui.horizontal_wrapped(|ui| {
                    ui.label("•");
                    ui.label(problem.as_str());
                });
            }
        });
    ui.add_space(4.0);
    if ui.button("Copy").clicked() {
        ui.ctx().copy_text(problems.join("\n"));
    }
}

fn count(n: usize, what: &str) -> String {
    if n == 1 {
        format!("1 {what}")
    } else {
        format!("{n} {what}s")
    }
}

/// A line over the library views when the project has problems (a library defined
/// twice, names that differ only by case, a library that cannot be read, …). A click
/// opens the list.
pub(crate) fn problems_banner(ui: &mut Ui, app: &mut AppCtx) {
    let Some(project) = app.project.as_ref() else {
        return;
    };
    let problems = problems_of(project);
    if problems.is_empty() {
        return;
    }
    let text = RichText::new(format!(
        "⚠ {} in the project",
        count(problems.len(), "problem")
    ))
    .color(ui.visuals().warn_fg_color);
    let hover = {
        let mut lines: Vec<&str> = problems.iter().take(6).map(String::as_str).collect();
        if problems.len() > 6 {
            lines.push("…");
        }
        lines.join("\n")
    };
    let response = ui
        .add(egui::Label::new(text).sense(egui::Sense::click()))
        .on_hover_text(hover);
    if response.clicked() {
        app.open_dialog(Dialog::Problems);
    }
}

/// Under the fields of "New Library" and "Add Existing Library": what is in the way
/// of a library `name` in the folder `path` (relative to the project).
pub(crate) fn folder_hint(ui: &mut Ui, project: Option<&Project>, name: &str, path: &str) {
    let Some(project) = project else {
        return;
    };
    if path.trim().is_empty() {
        return;
    }
    if let Some(clash) = project.folder_clash(name.trim(), path) {
        warn(ui, clash);
    }
}

/// The "Folder" row of "Add Existing Library": a path field, and the name follows
/// the folder as long as it was not typed.
pub(crate) fn add_folder_row(
    ui: &mut Ui,
    project: Option<&Project>,
    name: &mut String,
    path: &mut String,
) {
    let id = Id::new("cdept-add-library-derived");
    ui.label("Folder");
    let vars = |var: &str| project.and_then(|p| p.var(var));
    let mut field = PathField::folder("add-library-folder", path)
        .vars(&vars)
        .title("Library folder");
    if let Some(project) = project {
        field = field.base(&project.root);
    }
    let changed = ui.vertical(|ui| field.show(ui).changed).inner;
    ui.end_row();
    if changed {
        let derived: Option<String> = ui.data(|d| d.get_temp(id));
        if name.is_empty() || derived.as_deref() == Some(name.as_str()) {
            let folder = path.trim().trim_end_matches('/');
            let new = name_for(folder.rsplit('/').next().unwrap_or(""));
            *name = new.clone();
            ui.data_mut(|d| d.insert_temp(id, new));
        }
    }
}

/// A library name out of a folder name.
pub(crate) fn name_for(folder: &str) -> String {
    folder
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// The files of a library (or of one cell) CDePT does not recognise, counted at most
/// every [`RECOUNT`].
fn unrecognised(ctx: &Context, library: &libcdept::Library, cell: Option<&str>) -> Vec<PathBuf> {
    let id = Id::new(("cdept-unrecognised", &library.path, cell));
    let cached: Option<(Instant, Vec<PathBuf>)> = ctx.data(|d| d.get_temp(id));
    if let Some((at, files)) = cached
        && at.elapsed() < RECOUNT
    {
        return files;
    }
    let files = match cell {
        Some(cell) => formats::unrecognised_in(library, cell),
        None => formats::unrecognised(library),
    };
    ctx.data_mut(|d| d.insert_temp(id, (Instant::now(), files.clone())));
    files
}

/// "3 files not recognised" for a library or one of its cells, with the names on
/// hover. Nothing when every file is recognised.
pub(crate) fn unrecognised_line(ui: &mut Ui, library: &libcdept::Library, cell: Option<&str>) {
    if library.error.is_some() {
        return;
    }
    let files = unrecognised(ui.ctx(), library, cell);
    if files.is_empty() {
        return;
    }
    let mut names: Vec<String> = files
        .iter()
        .take(12)
        .map(|f| {
            f.strip_prefix(&library.path)
                .unwrap_or(f)
                .display()
                .to_string()
        })
        .collect();
    if files.len() > 12 {
        names.push(format!("… and {} more", files.len() - 12));
    }
    ui.label(
        RichText::new(format!("{} not recognised", count(files.len(), "file")))
            .small()
            .color(ui.visuals().warn_fg_color),
    )
    .on_hover_text(format!(
        "CDePT does not know what these files are (their suffix), so they are part of \
         no view. A [suffixes] table in cdept.toml can say what a suffix means.\n\n{}",
        names.join("\n")
    ));
}

/// For the Inspector: the unrecognised files of the selected library or cell.
pub(crate) fn unrecognised_selected(ui: &mut Ui, app: &AppCtx) {
    let Some(library) = app.selected_library() else {
        return;
    };
    unrecognised_line(ui, library, app.selection.cell.as_deref());
}

/// Test-script commands for library definitions and path prompts.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ScriptCommand {
    /// `lib_properties <library>`: open the Library Properties dialog.
    Properties(String),
    /// `lib_set <field> <value>`: a field of the open library dialog.
    Set(String, String),
    /// `lib_save`: confirm the open dialog.
    Save,
    /// `lib_save_error <text>`: confirming must fail with an error with this text;
    /// the dialog stays open.
    SaveFails(String),
    /// `lib_remove <library>`: Remove Library, confirmed.
    Remove(String),
    /// `lib_cancel`: close the open dialog.
    Cancel,
    /// `expect_lib_error <text>`: the open dialog shows an error with this text.
    ExpectError(String),
    /// `expect_library <name> <key> <value>`.
    ExpectLibrary(String, String, String),
    /// `expect_problem <text|none>`.
    ExpectProblem(String),
    /// `pick_folder <open|new|library>`: what the menu entries do.
    Pick(PickFor),
    /// `expect_path_prompt <title|none>`.
    ExpectPrompt(String),
    /// `path_answer <text>` / `path_cancel`.
    PathAnswer(Option<String>),
    /// `path_wrong <text>`: answering with this text must be refused; the prompt
    /// stays open.
    PathWrong(String),
    /// `expect_path_note <text>`: the prompt's note or error has this text.
    ExpectPathNote(String),
}

impl ScriptCommand {
    /// Checks are tried again for a while before they fail.
    pub(crate) fn patient(&self) -> bool {
        matches!(
            self,
            Self::ExpectError(_)
                | Self::ExpectLibrary(..)
                | Self::ExpectProblem(_)
                | Self::ExpectPrompt(_)
                | Self::ExpectPathNote(_)
        )
    }
}

/// Parse one of this file's script commands. `None`: not one of them.
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
    // The last argument may hold spaces: everything after the ones before it.
    let tail = |skip: usize| -> Option<String> {
        let mut rest = rest;
        for _ in 0..skip {
            rest = rest.trim_start();
            rest = &rest[rest.find(char::is_whitespace)?..];
        }
        let rest = rest.trim();
        (!rest.is_empty()).then(|| rest.to_string())
    };
    Some(match name {
        "lib_properties" => text().map(ScriptCommand::Properties),
        "lib_set" => match (args.first(), tail(1)) {
            (Some(field), Some(value)) => Ok(ScriptCommand::Set(field.to_string(), value)),
            // An empty value empties the field.
            (Some(field), None) => Ok(ScriptCommand::Set(field.to_string(), String::new())),
            _ => Err("usage: lib_set <field> <value>".into()),
        },
        "lib_save" => Ok(ScriptCommand::Save),
        "lib_save_error" => text().map(ScriptCommand::SaveFails),
        "lib_remove" => text().map(ScriptCommand::Remove),
        "lib_cancel" => Ok(ScriptCommand::Cancel),
        "expect_lib_error" => text().map(ScriptCommand::ExpectError),
        "expect_library" => match (args.first(), args.get(1), tail(2)) {
            (Some(library), Some(key), Some(value)) => Ok(ScriptCommand::ExpectLibrary(
                library.to_string(),
                key.to_string(),
                value,
            )),
            _ => Err("usage: expect_library <name> <key> <value>".into()),
        },
        "expect_problem" => text().map(ScriptCommand::ExpectProblem),
        "pick_folder" => match args.first().copied() {
            Some("open") => Ok(ScriptCommand::Pick(PickFor::OpenProject)),
            Some("new") => Ok(ScriptCommand::Pick(PickFor::NewProject)),
            Some("library") => Ok(ScriptCommand::Pick(PickFor::AddLibrary)),
            _ => Err("usage: pick_folder <open|new|library>".into()),
        },
        "expect_path_prompt" => text().map(ScriptCommand::ExpectPrompt),
        "path_answer" => text().map(|t| ScriptCommand::PathAnswer(Some(t))),
        "path_cancel" => Ok(ScriptCommand::PathAnswer(None)),
        "path_wrong" => text().map(ScriptCommand::PathWrong),
        "expect_path_note" => text().map(ScriptCommand::ExpectPathNote),
        _ => return None,
    })
}

/// Run one of this file's script commands.
pub(crate) fn script_run(
    command: ScriptCommand,
    app: &mut AppCtx,
    ctx: &Context,
) -> Result<(), String> {
    let main = WindowId::Main;
    match command {
        ScriptCommand::Properties(name) => {
            let project = app.project.as_ref().ok_or("no project is open")?;
            if project.library(&name).is_none() {
                return Err(format!("no library '{name}'"));
            }
            let dialog = Dialog::LibraryProperties(Properties::open(project, &name));
            app.window = main;
            app.open_dialog(dialog);
        }
        ScriptCommand::Set(field, value) => {
            match dialogs::open_mut(app).ok_or("no dialog is open")? {
                Dialog::LibraryProperties(properties) => properties.set(&field, &value)?,
                Dialog::AddLibrary {
                    name,
                    path,
                    read_only,
                    layout,
                } => match field.as_str() {
                    "name" => *name = value,
                    "path" | "folder" => *path = value,
                    "read_only" => *read_only = yes_no(&value)?,
                    "layout" => *layout = layout_named(&value)?,
                    other => return Err(format!("Add Existing Library has no field '{other}'")),
                },
                Dialog::NewLibrary {
                    name,
                    path,
                    custom_path,
                } => match field.as_str() {
                    "name" => *name = value,
                    "path" | "folder" => {
                        *path = value;
                        *custom_path = true;
                    }
                    other => return Err(format!("New Library has no field '{other}'")),
                },
                _ => return Err("the open dialog is not a library dialog".into()),
            }
        }
        ScriptCommand::Save => {
            dialogs::confirm_open(app).map_err(|e| format!("{e:#}"))?;
        }
        ScriptCommand::SaveFails(text) => match dialogs::confirm_open(app) {
            Ok(()) => return Err(format!("the dialog was accepted, expected '{text}'")),
            Err(error) => {
                let error = format!("{error:#}");
                if !error.contains(&text) {
                    return Err(format!("the dialog failed with '{error}', not '{text}'"));
                }
            }
        },
        ScriptCommand::Remove(name) => {
            app.window = main;
            app.open_dialog(Dialog::RemoveLibrary { library: name });
            dialogs::confirm_open(app).map_err(|e| format!("{e:#}"))?;
        }
        ScriptCommand::Cancel => {
            if app.dialog.take().is_none() {
                return Err("no dialog is open".into());
            }
        }
        ScriptCommand::ExpectError(text) => {
            let error = dialogs::open_error(app).unwrap_or_default();
            if !error.contains(&text) {
                return Err(format!("the dialog's error is '{error}', not '{text}'"));
            }
        }
        ScriptCommand::ExpectLibrary(name, key, value) => {
            let project = app.project.as_ref().ok_or("no project is open")?;
            let library = project.library(&name);
            if key == "present" {
                return match (library.is_some(), yes_no(&value)?) {
                    (a, b) if a == b => Ok(()),
                    (true, _) => Err(format!("library '{name}' is there")),
                    (false, _) => Err(format!("no library '{name}'")),
                };
            }
            let library = library.ok_or(format!("no library '{name}'"))?;
            let definition = project.library_definition(&name);
            let found = match key.as_str() {
                "path" => definition.map(|d| d.path).map_err(|e| format!("{e:#}"))?,
                "folder" => library.path.display().to_string(),
                "layout" => library.layout.name().to_string(),
                "layout_written" => definition
                    .map(|d| d.layout.map_or("detect", LibraryLayout::name).to_string())
                    .map_err(|e| format!("{e:#}"))?,
                "read_only" => if library.read_only { "yes" } else { "no" }.to_string(),
                "description" => library.description.clone().unwrap_or_default(),
                "cells" => library.cells.len().to_string(),
                "origin" => match project.library_origin(&name) {
                    Some(Origin::Project) => "project",
                    Some(Origin::Included(_)) => "included",
                    Some(Origin::Snapshot(_)) => "pdk",
                    None => "none",
                }
                .to_string(),
                "unrecognised" => formats::unrecognised(library).len().to_string(),
                other => return Err(format!("expect_library knows no '{other}'")),
            };
            if found != value {
                return Err(format!(
                    "library '{name}': {key} is '{found}', expected '{value}'"
                ));
            }
        }
        ScriptCommand::ExpectProblem(text) => {
            let project = app.project.as_ref().ok_or("no project is open")?;
            let problems = problems_of(project);
            let ok = if text == "none" {
                problems.is_empty()
            } else {
                problems.iter().any(|p| p.contains(&text))
            };
            if !ok {
                return Err(format!("the project's problems are {problems:?}"));
            }
        }
        ScriptCommand::Pick(purpose) => {
            app.window = main;
            app.request(Action::PickFolder(purpose));
        }
        ScriptCommand::ExpectPrompt(title) => {
            let open = pathfield::open_prompt(ctx, main);
            let ok = match (&open, title.as_str()) {
                (None, "none") => true,
                (Some(open), wanted) => open.contains(wanted),
                (None, _) => false,
            };
            if !ok {
                return Err(format!(
                    "the open path prompt is {open:?}, expected '{title}'"
                ));
            }
        }
        ScriptCommand::PathAnswer(text) => {
            pathfield::answer_prompt(ctx, main, text.as_deref())?;
        }
        ScriptCommand::PathWrong(text) => {
            if pathfield::open_prompt(ctx, main).is_none() {
                return Err("no path prompt is open".into());
            }
            if pathfield::answer_prompt(ctx, main, Some(&text)).is_ok() {
                return Err(format!("the path prompt took '{text}'"));
            }
        }
        ScriptCommand::ExpectPathNote(text) => {
            let (note, error) =
                pathfield::prompt_messages(ctx, main).ok_or("no path prompt is open")?;
            let both = format!("{} {}", note.unwrap_or_default(), error.unwrap_or_default());
            if !both.contains(&text) {
                return Err(format!(
                    "the path prompt says '{}', not '{text}'",
                    both.trim()
                ));
            }
        }
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

    /// A project with a library in the project file and one in an included file.
    fn app(root: &Path) -> AppCtx {
        fs::create_dir_all(root.join("libs/analog/ota/schematic")).unwrap();
        fs::write(root.join("libs/analog/ota/schematic/ota.sch"), "").unwrap();
        fs::write(root.join("libs/analog/ota/notes.md"), "").unwrap();
        fs::create_dir_all(root.join("shared/prims")).unwrap();
        fs::write(root.join("shared/prims/nmos.sym"), "").unwrap();
        fs::write(
            root.join("shared/libraries.toml"),
            "[libraries]\nprims = \"prims\"\n",
        )
        .unwrap();
        fs::write(
            root.join(PROJECT_FILE),
            "name = \"demo\"\ninclude = [\"shared/libraries.toml\"]\n\
             [libraries]\nanalog = \"libs/analog\"\ngone = \"nowhere\"\n",
        )
        .unwrap();
        let mut app = AppCtx::new(Settings::default(), LogBuffer::new());
        app.load_project(root);
        assert!(app.dialog.is_none(), "the project loads");
        app
    }

    #[test]
    fn properties_change_a_definition_where_it_is_defined() {
        let _env = libcdept::testutil::hermetic_env();
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app(tmp.path());

        // Not drawn, not changed: saving does nothing.
        apply(&Properties::new("analog"), &mut app).unwrap();

        let project_file = app.project.as_ref().unwrap().file.clone();
        let mut properties = Properties::open(app.project.as_ref().unwrap(), "analog");
        assert_eq!(properties.path, "libs/analog");
        assert!(properties.change().is_empty());
        properties.set("read_only", "yes").unwrap();
        properties.set("layout", "cells").unwrap();
        assert!(properties.set("layout", "heap").is_err());
        assert!(properties.set("colour", "red").is_err());
        apply(&properties, &mut app).unwrap();
        let text = fs::read_to_string(&project_file).unwrap();
        assert!(
            text.contains(
                "analog = { path = \"libs/analog\", layout = \"cells\", read_only = true }"
            ),
            "{text}"
        );
        assert!(
            app.project
                .as_ref()
                .unwrap()
                .library("analog")
                .unwrap()
                .read_only
        );

        // The included library: written to the included file.
        let mut properties = Properties::open(app.project.as_ref().unwrap(), "prims");
        assert!(properties.was.as_ref().unwrap().included);
        properties.set("description", "the primitives").unwrap();
        apply(&properties, &mut app).unwrap();
        let library = app.project.as_ref().unwrap().library("prims").unwrap();
        assert_eq!(library.description.as_deref(), Some("the primitives"));
        // The lib.toml that now exists must not turn the flat folder into `cells`.
        assert_eq!(library.layout, LibraryLayout::Flat);
        assert!(
            fs::read_to_string(tmp.path().join("shared/libraries.toml"))
                .unwrap()
                .contains("layout = \"flat\"")
        );
        assert_eq!(fs::read_to_string(&project_file).unwrap(), text);

        // A folder that is not there keeps the dialog's error.
        let mut properties = Properties::open(app.project.as_ref().unwrap(), "prims");
        properties.set("path", "nowhere").unwrap();
        assert!(apply(&properties, &mut app).is_err());
    }

    #[test]
    fn problems_are_listed() {
        let _env = libcdept::testutil::hermetic_env();
        let tmp = tempfile::tempdir().unwrap();
        let app = app(tmp.path());
        let problems = problems_of(app.project.as_ref().unwrap());
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("library 'gone'"));
        assert_eq!(count(1, "problem"), "1 problem");
        assert_eq!(count(2, "file"), "2 files");
        assert_eq!(name_for("my lib.v2"), "my_lib_v2");
    }

    #[test]
    fn script_commands_parse() {
        let parse = |line: &str| {
            let (name, rest) = line.split_once(' ').unwrap_or((line, ""));
            let args: Vec<&str> = rest.split_whitespace().collect();
            script_parse(name, &args, rest)
        };
        assert_eq!(
            parse("lib_set description the  primitives"),
            Some(Ok(ScriptCommand::Set(
                "description".into(),
                "the  primitives".into()
            )))
        );
        assert_eq!(
            parse("lib_set description"),
            Some(Ok(ScriptCommand::Set("description".into(), String::new())))
        );
        assert_eq!(
            parse("expect_library analog path libs/my folder"),
            Some(Ok(ScriptCommand::ExpectLibrary(
                "analog".into(),
                "path".into(),
                "libs/my folder".into()
            )))
        );
        assert_eq!(
            parse("pick_folder open"),
            Some(Ok(ScriptCommand::Pick(PickFor::OpenProject)))
        );
        assert!(matches!(parse("pick_folder"), Some(Err(_))));
        assert_eq!(
            parse("path_cancel"),
            Some(Ok(ScriptCommand::PathAnswer(None)))
        );
        assert!(parse("open a b c").is_none());
        assert!(ScriptCommand::ExpectProblem("x".into()).patient());
        assert!(!ScriptCommand::Save.patient());
    }

    #[test]
    fn the_dialogs_draw() {
        let _env = libcdept::testutil::hermetic_env();
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app(tmp.path());
        let ctx = Context::default();
        let dialogs = [
            Dialog::LibraryProperties(Properties::new("analog")),
            Dialog::LibraryProperties(Properties::new("prims")),
            Dialog::LibraryProperties(Properties::new("gone")),
            Dialog::Problems,
        ];
        for dialog in dialogs {
            app.open_dialog(dialog);
            for _ in 0..2 {
                let output = ctx.run_ui(egui::RawInput::default(), |ui| {
                    problems_banner(ui, &mut app);
                    unrecognised_selected(ui, &app);
                    crate::dialogs::show(ui.ctx(), &mut app);
                    pathfield::show(ui.ctx(), WindowId::Main);
                });
                output.drop_without_applying_deltas();
            }
            assert!(app.dialog.is_some(), "the dialog stays open");
        }
        // The properties were read when first drawn.
        app.open_dialog(Dialog::LibraryProperties(Properties::new("analog")));
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            crate::dialogs::show(ui.ctx(), &mut app);
        });
        output.drop_without_applying_deltas();
        match dialogs::open_mut(&mut app) {
            Some(Dialog::LibraryProperties(properties)) => {
                assert_eq!(properties.path, "libs/analog");
                properties.set("read_only", "yes").unwrap();
            }
            other => panic!("{other:?}"),
        }
        dialogs::confirm_open(&mut app).unwrap();
        assert!(app.dialog.is_none());
        assert!(
            app.project
                .as_ref()
                .unwrap()
                .library("analog")
                .unwrap()
                .read_only
        );
        // The unrecognised file of the cell is counted.
        let library = app.project.as_ref().unwrap().library("analog").unwrap();
        assert_eq!(unrecognised(&ctx, library, Some("ota")).len(), 1);
    }
}
