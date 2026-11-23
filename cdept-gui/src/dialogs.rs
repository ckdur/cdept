//! Modal dialogs for project and library operations.
//!
//! A view opens one with [`AppCtx::open_dialog`]; the shell shows it over the window it
//! was opened from. Confirming runs the operation; a failure keeps the dialog open with
//! the error.

use std::path::Path;

use egui::{Context, Id, Key, RichText, Ui};
use eyre::{Result, eyre};
use libcdept::xschem::symbol::Unlinked;
use libcdept::{AddLibrary, Library, LibraryLayout, Project, Settings, Template};

use crate::state::{AppCtx, Selection, is_foreign};

#[derive(Debug, Clone, PartialEq)]
pub enum Dialog {
    Error(String),
    /// Create a library folder and define it.
    NewLibrary {
        name: String,
        path: String,
        /// The user typed a folder, so stop deriving it from the name.
        custom_path: bool,
    },
    /// Define an existing folder as a library.
    AddLibrary {
        name: String,
        path: String,
        read_only: bool,
        layout: Option<LibraryLayout>,
    },
    RemoveLibrary {
        library: String,
    },
    NewCell {
        library: String,
        name: String,
        template: Option<Template>,
    },
    NewView {
        library: String,
        cell: String,
        template: Template,
    },
    RenameCell {
        library: String,
        cell: String,
        name: String,
    },
    CopyCell {
        library: String,
        cell: String,
        to: String,
        name: String,
    },
    DeleteCell {
        library: String,
        cell: String,
    },
    DeleteView {
        library: String,
        cell: String,
        view: String,
    },
    /// Name, description, technology and PDK folder of the project.
    ProjectSettings(Settings),
    /// Write the expected `schematic=` into the broken symbols of a library.
    FixLinks {
        library: String,
        /// Also rewrite links that point to another cell's schematic.
        foreign: bool,
    },
    /// Change a library definition where it is defined (library_dialog.rs).
    LibraryProperties(crate::library_dialog::Properties),
    /// What is wrong with the project file and its libraries (library_dialog.rs).
    Problems,
    /// Move a cell, rename or copy a view, rename a library (refactor.rs).
    Refactor(crate::refactor::Refactor),
    /// Bring files from outside the project into a library (import_files.rs).
    ImportFiles(crate::import_files::ImportFiles),
    /// What an import did: one line each.
    ImportSummary(Vec<String>),
}

impl Dialog {
    pub fn project_settings(project: &Project) -> Self {
        Self::ProjectSettings(Settings {
            name: project.name.clone(),
            description: project.description.clone().unwrap_or_default(),
            technology: project.technology.clone().unwrap_or_default(),
            pdk_root: project.pdk_root.clone().unwrap_or_default(),
        })
    }

    pub fn new_library() -> Self {
        Self::NewLibrary {
            name: String::new(),
            path: "libs/".into(),
            custom_path: false,
        }
    }

    /// For an existing folder; the name defaults to the folder name.
    pub fn add_library(project: &Project, folder: &Path) -> Self {
        let name: String = folder
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        Self::AddLibrary {
            name,
            path: project.portable_path(folder),
            read_only: false,
            layout: None,
        }
    }

    pub fn new_cell(library: &Library) -> Self {
        Self::NewCell {
            library: library.name.clone(),
            name: String::new(),
            template: Some(Template::XschemSchematic),
        }
    }

    pub fn new_view(library: &str, cell: &libcdept::Cell) -> Self {
        // Offer the first kind of view the cell does not have yet.
        let template = Template::ALL
            .into_iter()
            .find(|t| cell.entry(t.view_name()).is_none())
            .unwrap_or(Template::XschemSchematic);
        Self::NewView {
            library: library.into(),
            cell: cell.name.clone(),
            template,
        }
    }

    fn title(&self) -> &'static str {
        match self {
            Self::Error(_) => "Error",
            Self::NewLibrary { .. } => "New Library",
            Self::AddLibrary { .. } => "Add Existing Library",
            Self::RemoveLibrary { .. } => "Remove Library",
            Self::NewCell { .. } => "New Cell",
            Self::NewView { .. } => "New View",
            Self::RenameCell { .. } => "Rename Cell",
            Self::CopyCell { .. } => "Copy Cell",
            Self::DeleteCell { .. } => "Delete Cell",
            Self::DeleteView { .. } => "Delete View",
            Self::ProjectSettings(_) => "Project Settings",
            Self::FixLinks { .. } => "Fix Schematic Links",
            Self::LibraryProperties(_) => "Library Properties",
            Self::Problems => "Project Problems",
            Self::Refactor(refactor) => refactor.title(),
            Self::ImportFiles(_) => "Import Files",
            Self::ImportSummary(_) => "Import Finished",
        }
    }

    fn confirm_label(&self) -> &'static str {
        match self {
            Self::Error(_) | Self::Problems | Self::ImportSummary(_) => "Close",
            Self::ImportFiles(_) => "Import",
            Self::LibraryProperties(_) => "Save",
            Self::Refactor(refactor) => refactor.confirm_label(),
            Self::NewLibrary { .. } | Self::NewCell { .. } | Self::NewView { .. } => "Create",
            Self::AddLibrary { .. } => "Add",
            Self::RemoveLibrary { .. } => "Remove",
            Self::RenameCell { .. } => "Rename",
            Self::CopyCell { .. } => "Copy",
            Self::DeleteCell { .. } | Self::DeleteView { .. } => "Delete",
            Self::FixLinks { .. } => "Fix",
            Self::ProjectSettings(_) => "Save",
        }
    }
}

pub(crate) struct DialogState {
    dialog: Dialog,
    error: Option<String>,
    /// The first text field still has to take the keyboard focus.
    focus: bool,
    /// Delete dialogs: delete for good instead of moving to the trash.
    permanent: bool,
}

impl DialogState {
    pub fn new(dialog: Dialog) -> Self {
        Self {
            dialog,
            error: None,
            focus: true,
            permanent: false,
        }
    }
}

enum Outcome {
    Open,
    Cancel,
    Confirm,
}

/// Show the open dialog, if it belongs to the window being built.
pub(crate) fn show(ctx: &Context, app: &mut AppCtx) {
    if app
        .dialog
        .as_ref()
        .is_none_or(|(win, _)| *win != app.window)
    {
        return;
    }
    let Some((win, mut state)) = app.dialog.take() else {
        return;
    };
    let mut outcome = Outcome::Open;
    let response = egui::Modal::new(Id::new("cdept-dialog")).show(ctx, |ui| {
        ui.set_width(420.0);
        ui.heading(state.dialog.title());
        ui.add_space(6.0);
        body(ui, &mut state, app);
        if let Some(error) = &state.error {
            ui.add_space(6.0);
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        ui.add_space(10.0);
        let is_error = matches!(
            state.dialog,
            Dialog::Error(_) | Dialog::Problems | Dialog::ImportSummary(_)
        );
        ui.horizontal(|ui| {
            if ui.button(state.dialog.confirm_label()).clicked() {
                outcome = Outcome::Confirm;
            }
            if !is_error && ui.button("Cancel").clicked() {
                outcome = Outcome::Cancel;
            }
        });
        // Enter confirms, except where that would delete something.
        let destructive = matches!(
            state.dialog,
            Dialog::DeleteCell { .. } | Dialog::DeleteView { .. }
        );
        if !destructive && ui.input(|i| i.key_pressed(Key::Enter)) {
            outcome = Outcome::Confirm;
        }
    });
    if response.should_close() && matches!(outcome, Outcome::Open) {
        outcome = Outcome::Cancel;
    }
    match outcome {
        Outcome::Open => app.dialog = Some((win, state)),
        Outcome::Cancel => {}
        Outcome::Confirm => {
            app.background.removal = crate::background::removal(state.permanent);
            if let Err(e) = apply(&state.dialog, app) {
                state.error = Some(format!("{e:#}"));
                app.dialog = Some((win, state));
            }
        }
    }
}

/// Confirm the open dialog, as its button does (for the test driver). `permanent`: a
/// delete dialog deletes for good instead of moving to the trash.
pub(crate) fn confirm(app: &mut AppCtx, permanent: bool) -> Result<(), String> {
    let (_, state) = app.dialog.take().ok_or("no dialog is open")?;
    app.background.removal = crate::background::removal(permanent);
    apply(&state.dialog, app).map_err(|e| format!("{e:#}"))
}

/// The open dialog, for test scripts.
/// The open dialog.
pub(crate) fn open(app: &AppCtx) -> Option<&Dialog> {
    app.dialog.as_ref().map(|(_, state)| &state.dialog)
}

pub(crate) fn open_mut(app: &mut AppCtx) -> Option<&mut Dialog> {
    app.dialog.as_mut().map(|(_, state)| &mut state.dialog)
}

/// The error the open dialog shows, for test scripts.
pub(crate) fn open_error(app: &AppCtx) -> Option<String> {
    app.dialog.as_ref()?.1.error.clone()
}

/// Confirm the open dialog as its button does, for test scripts: a failure keeps it
/// open with the error.
pub(crate) fn confirm_open(app: &mut AppCtx) -> Result<()> {
    let (win, mut state) = app
        .dialog
        .take()
        .ok_or_else(|| eyre!("no dialog is open"))?;
    match apply(&state.dialog, app) {
        Ok(()) => Ok(()),
        Err(error) => {
            state.error = Some(format!("{error:#}"));
            app.dialog = Some((win, state));
            Err(error)
        }
    }
}

fn body(ui: &mut Ui, state: &mut DialogState, app: &AppCtx) {
    let focus = std::mem::take(&mut state.focus);
    let project = app.project.as_ref();
    // A labelled single-line text field; the first one takes the focus.
    let mut first = focus;
    let mut field = |ui: &mut Ui, label: &str, text: &mut String| {
        ui.label(label);
        let response = ui.add(egui::TextEdit::singleline(text).desired_width(280.0));
        if std::mem::take(&mut first) {
            response.request_focus();
        }
        ui.end_row();
        response
    };
    let grid = egui::Grid::new("dialog-grid")
        .num_columns(2)
        .spacing([8.0, 6.0]);

    match &mut state.dialog {
        Dialog::FixLinks { library, foreign } => {
            let items = fixable_links(&app.links, library, *foreign);
            let others = app
                .links
                .iter()
                .filter(|u| u.library == *library && !items.contains(u))
                .count();
            ui.label(format!(
                "Write the expected schematic= into {} symbol(s) of '{library}':",
                items.len()
            ));
            egui::ScrollArea::vertical()
                .max_height(220.0)
                .show(ui, |ui| {
                    for u in &items {
                        ui.label(RichText::new(format!("{} / {}", u.cell, u.problem)).small());
                        ui.label(
                            RichText::new(format!("    -> schematic={}", u.expected))
                                .monospace()
                                .small(),
                        );
                    }
                });
            if app
                .links
                .iter()
                .any(|u| u.library == *library && is_foreign(u))
            {
                ui.checkbox(
                    foreign,
                    "Also links to another cell's schematic (changes where descending goes)",
                );
            }
            if others > 0 {
                ui.small(format!(
                    "{others} link(s) to another cell are left as they are."
                ));
            }
        }
        Dialog::Error(message) => {
            ui.label(message.as_str());
        }
        Dialog::LibraryProperties(properties) => {
            crate::library_dialog::body(ui, properties, app, focus);
        }
        Dialog::Problems => crate::library_dialog::problems_body(ui, app),
        Dialog::Refactor(refactor) => crate::refactor::body(ui, refactor, app, focus),
        Dialog::ImportFiles(import) => crate::import_files::body(ui, import, app),
        Dialog::ImportSummary(lines) => {
            for line in lines {
                ui.label(egui::RichText::new(line.as_str()).small());
            }
        }
        Dialog::ProjectSettings(settings) => {
            grid.show(ui, |ui| {
                field(ui, "Name", &mut settings.name);
                field(ui, "Description", &mut settings.description);
                field(ui, "Technology", &mut settings.technology);
                field(ui, "PDK folder", &mut settings.pdk_root);
            });
            crate::newproject::settings_ui(ui, settings, app);
            ui.small(
                "Technology is the PDK name ($PDK), e.g. ihp-sg13g2. PDK folder is where PDKs \
                 are installed ($PDK_ROOT), relative to the project file; $VAR and ~ are \
                 expanded. Left empty, they come from the environment CDePT was started in.",
            );
            if let Some(project) = project {
                ui.small(format!(
                    "In use now: PDK {}, PDK_ROOT {}",
                    pdk_text(&project.pdk.name),
                    pdk_text(&project.pdk.root)
                ));
            }
        }
        Dialog::NewLibrary {
            name,
            path,
            custom_path,
        } => {
            grid.show(ui, |ui| {
                if field(ui, "Name", name).changed() && !*custom_path {
                    *path = format!("libs/{name}");
                }
                // A path field: completion and "Browse…".
                let root = project.map(|p| p.root.as_path());
                if crate::pathfield::folder_row(ui, "Folder", path, root) {
                    *custom_path = true;
                }
            });
            ui.small("The folder is relative to the project and is created if missing.");
            crate::library_dialog::folder_hint(ui, project, name, path);
        }
        Dialog::AddLibrary {
            name,
            path,
            read_only,
            layout,
        } => {
            grid.show(ui, |ui| {
                field(ui, "Name", name);
                // A path field: completion, "Browse…", and the name follows the folder.
                crate::library_dialog::add_folder_row(ui, project, name, path);
                ui.label("Layout");
                egui::ComboBox::from_id_salt("dialog-layout")
                    .selected_text(layout_label(*layout))
                    .show_ui(ui, |ui| {
                        for option in [
                            None,
                            Some(LibraryLayout::Cells),
                            Some(LibraryLayout::Flat),
                            Some(LibraryLayout::Ref),
                        ] {
                            ui.selectable_value(layout, option, layout_label(option));
                        }
                    });
                ui.end_row();
                ui.label("");
                ui.checkbox(read_only, "Read-only (reference library)");
                ui.end_row();
            });
            ui.small("$VAR and ~ in the folder are kept as written, e.g. $PDK_ROOT/….");
            crate::library_dialog::folder_hint(ui, project, name, path);
        }
        Dialog::RemoveLibrary { library } => {
            ui.label(format!("Remove library '{library}' from the project?"));
            ui.small("Only the definition in cdept.toml is removed. The folder stays on disk.");
        }
        Dialog::NewCell {
            library,
            name,
            template,
        } => {
            grid.show(ui, |ui| {
                ui.label("Library");
                ui.label(library.as_str());
                ui.end_row();
                field(ui, "Name", name);
                ui.label("First view");
                let none = "None (empty cell)";
                egui::ComboBox::from_id_salt("dialog-template")
                    .selected_text(template.map_or(none, Template::label))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(template, None, none);
                        for t in Template::ALL {
                            ui.selectable_value(template, Some(t), t.label());
                        }
                    });
                ui.end_row();
            });
        }
        Dialog::NewView {
            library,
            cell,
            template,
        } => {
            grid.show(ui, |ui| {
                ui.label("Cell");
                ui.label(format!("{library} / {cell}"));
                ui.end_row();
                ui.label("View");
                egui::ComboBox::from_id_salt("dialog-template")
                    .selected_text(template.label())
                    .show_ui(ui, |ui| {
                        for t in Template::ALL {
                            ui.selectable_value(template, t, t.label());
                        }
                    });
                ui.end_row();
            });
        }
        Dialog::RenameCell {
            library,
            cell,
            name,
        } => {
            grid.show(ui, |ui| {
                ui.label("Cell");
                ui.label(format!("{library} / {cell}"));
                ui.end_row();
                field(ui, "New name", name);
            });
            let operation = libcdept::refactor::Operation::RenameCell {
                library: library.clone(),
                cell: cell.clone(),
                new: name.trim().to_string(),
            };
            crate::refactor::preview(ui, app, &operation);
        }
        Dialog::CopyCell {
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
                egui::ComboBox::from_id_salt("dialog-to")
                    .selected_text(to.as_str())
                    .show_ui(ui, |ui| {
                        for lib in project.iter().flat_map(|p| &p.libraries) {
                            if !lib.read_only && lib.error.is_none() {
                                ui.selectable_value(to, lib.name.clone(), &lib.name);
                            }
                        }
                    });
                ui.end_row();
                field(ui, "Name", name);
            });
            let operation = libcdept::refactor::Operation::CopyCell {
                library: library.clone(),
                cell: cell.clone(),
                to_library: to.clone(),
                new: name.trim().to_string(),
                hierarchy: false,
            };
            crate::refactor::preview(ui, app, &operation);
        }
        Dialog::DeleteCell { library, cell } => {
            ui.label(format!("Delete cell '{cell}' from library '{library}'?"));
            crate::background::delete_note(ui, app, &mut state.permanent, library, cell, None);
        }
        Dialog::DeleteView {
            library,
            cell,
            view,
        } => {
            ui.label(format!("Delete view '{view}' of '{library} / {cell}'?"));
            crate::background::delete_note(
                ui,
                app,
                &mut state.permanent,
                library,
                cell,
                Some(view),
            );
        }
    }
}

pub(crate) fn layout_label(layout: Option<LibraryLayout>) -> &'static str {
    match layout {
        None => "Detect from the folder",
        Some(LibraryLayout::Cells) => "Cells: library/cell/view/files",
        Some(LibraryLayout::Flat) => "Flat: library/cell.ext",
        Some(LibraryLayout::Ref) => "Reference: library/format/files (read only)",
    }
}

/// The broken links of `library` a "fix all" repairs: those that point to another
/// cell's schematic only when `foreign` is set.
fn fixable_links<'a>(links: &'a [Unlinked], library: &str, foreign: bool) -> Vec<&'a Unlinked> {
    links
        .iter()
        .filter(|u| u.library == library && (foreign || !is_foreign(u)))
        .collect()
}

/// A resolved PDK value with its source, for display.
pub(crate) fn pdk_text(value: &Option<libcdept::Resolved>) -> String {
    match value {
        Some(r) => format!("{} ({})", r.value, r.source.name()),
        None => "not set".into(),
    }
}

fn apply(dialog: &Dialog, app: &mut AppCtx) -> Result<()> {
    if matches!(dialog, Dialog::Error(_) | Dialog::ImportSummary(_)) {
        return Ok(());
    }
    if let Dialog::ImportFiles(import) = dialog {
        return crate::import_files::apply(app, import);
    }
    if let Dialog::ProjectSettings(settings) = dialog {
        return app.change_settings(settings);
    }
    if let Dialog::LibraryProperties(properties) = dialog {
        return crate::library_dialog::apply(properties, app);
    }
    // Renames, copies and moves: with what refers to the thing following.
    if let Some(operation) = crate::refactor::operation_of(dialog) {
        return crate::refactor::apply(app, &operation);
    }
    if let Dialog::FixLinks { library, foreign } = dialog {
        let items: Vec<Unlinked> = fixable_links(&app.links, library, *foreign)
            .into_iter()
            .cloned()
            .collect();
        let fixed = app.fix_links(&items);
        tracing::info!(target: "cdept::library", "fixed {fixed} schematic link(s) in '{library}'");
        if fixed < items.len() {
            return Err(eyre!(
                "{} link(s) could not be fixed; see the Console",
                items.len() - fixed
            ));
        }
        return Ok(());
    }
    let project = app
        .project
        .as_mut()
        .ok_or_else(|| eyre!("no project is open"))?;
    let library = |project: &Project, name: &str| -> Result<Library> {
        project
            .library(name)
            .cloned()
            .ok_or_else(|| eyre!("no library '{name}'"))
    };
    const TARGET: &str = "cdept::library";
    let selection = match dialog {
        Dialog::Error(_) | Dialog::FixLinks { .. } | Dialog::ProjectSettings(_) => return Ok(()),
        Dialog::LibraryProperties(_) | Dialog::Problems => return Ok(()),
        Dialog::ImportFiles(_) | Dialog::ImportSummary(_) => {
            unreachable!("handled by crate::import_files above")
        }
        Dialog::RenameCell { .. } | Dialog::CopyCell { .. } | Dialog::Refactor(_) => {
            unreachable!("handled by crate::refactor above")
        }
        Dialog::NewLibrary { name, path, .. } => {
            let options = AddLibrary {
                create: true,
                ..Default::default()
            };
            project.add_library(name.trim(), path.trim(), options)?;
            tracing::info!(target: TARGET, "created library '{}' in {}", name.trim(), path.trim());
            Selection::library(name.trim())
        }
        Dialog::AddLibrary {
            name,
            path,
            read_only,
            layout,
        } => {
            let options = AddLibrary {
                layout: *layout,
                read_only: *read_only,
                create: false,
            };
            project.add_library(name.trim(), path.trim(), options)?;
            tracing::info!(target: TARGET, "added library '{}' from {}", name.trim(), path.trim());
            Selection::library(name.trim())
        }
        Dialog::RemoveLibrary { library } => {
            project.remove_library(library)?;
            tracing::info!(target: TARGET, "removed library '{library}' from the project");
            Selection::default()
        }
        Dialog::NewCell {
            library: lib,
            name,
            template,
        } => {
            let name = name.trim();
            let target = library(project, lib)?;
            match template {
                Some(template) => {
                    if target.cell(name).is_some() {
                        return Err(eyre!("'{name}' already exists in library '{lib}'"));
                    }
                    target.create_view_with(name, *template, &project.template_setup())?;
                }
                None => {
                    target.create_cell(name)?;
                }
            }
            tracing::info!(target: TARGET, "created cell {lib}/{name}");
            Selection::cell(lib, name)
        }
        Dialog::NewView {
            library: lib,
            cell,
            template,
        } => {
            let file = library(project, lib)?.create_view_with(
                cell,
                *template,
                &project.template_setup(),
            )?;
            tracing::info!(target: TARGET, "created {}", file.display());
            Selection::view(lib, cell, template.view_name())
        }
        Dialog::DeleteCell { library: lib, cell } => {
            let orphans = libcdept::layout::follow::placed_by(project, lib, cell, None);
            crate::background::remove_cell(&mut app.background, &library(project, lib)?, cell)?;
            report_layouts(&orphans);
            tracing::info!(target: TARGET, "deleted cell {lib}/{cell}");
            Selection::library(lib)
        }
        Dialog::DeleteView {
            library: lib,
            cell,
            view,
        } => {
            let orphans = libcdept::layout::follow::placed_by(project, lib, cell, Some(view));
            crate::background::remove_view(
                &mut app.background,
                &library(project, lib)?,
                cell,
                view,
            )?;
            report_layouts(&orphans);
            tracing::info!(target: TARGET, "deleted view {lib}/{cell}/{view}");
            Selection::cell(lib, cell)
        }
    };
    crate::background::rescan_after(project, dialog);
    app.refresh_links();
    app.selection = selection;
    app.validate_selection();
    Ok(())
}

/// Log what a rename or deletion did to the layout views placing the cell: a line per
/// view, warnings for the ones now wrong.
fn report_layouts(followed: &[libcdept::layout::follow::Followed]) {
    for f in followed {
        if f.is_problem() {
            tracing::warn!(target: "cdept::library", "{f}");
        } else {
            tracing::info!(target: "cdept::library", "{f}");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use libcdept::PROJECT_FILE;

    use super::*;
    use crate::dock::Tab;
    use crate::log::LogBuffer;
    use crate::state::Settings;
    use crate::views::Views;

    /// A project with a writable library, a flat read-only one and a missing one.
    fn app(root: &Path) -> AppCtx {
        fs::create_dir_all(root.join("libs/analog/ota/schematic")).unwrap();
        fs::write(root.join("libs/analog/ota/schematic/ota.sch"), "").unwrap();
        fs::create_dir_all(root.join("prims")).unwrap();
        fs::write(root.join("prims/nmos.sym"), "").unwrap();
        fs::write(
            root.join(PROJECT_FILE),
            "name = \"demo\"\n[libraries]\nanalog = \"libs/analog\"\n\
             prims = { path = \"prims\", read_only = true }\ngone = \"nowhere\"\n",
        )
        .unwrap();
        let mut app = AppCtx::new(Settings::default(), LogBuffer::new());
        app.load_project(root);
        assert!(app.dialog.is_none(), "the project loads");
        app
    }

    fn confirm(app: &mut AppCtx, dialog: Dialog) -> Result<()> {
        apply(&dialog, app)
    }

    #[test]
    fn fix_all_links_leaves_links_to_another_cell_unless_asked() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app(tmp.path());
        let root = tmp.path();
        let cell = |name: &str, symbol: &str| {
            let dir = root.join("libs/analog").join(name);
            fs::create_dir_all(dir.join("symbol")).unwrap();
            fs::create_dir_all(dir.join("schematic")).unwrap();
            fs::write(dir.join(format!("schematic/{name}.sch")), "v {}\n").unwrap();
            fs::write(dir.join(format!("symbol/{name}.sym")), symbol).unwrap();
        };
        cell("a", "G {}\nK {type=subcircuit\n}\n");
        cell(
            "b",
            "G {}\nK {type=subcircuit\nschematic=analog/ota/schematic/ota.sch\n}\n",
        );
        app.reload_project();
        assert_eq!(app.links.len(), 2);
        let fix = |foreign| Dialog::FixLinks {
            library: "analog".into(),
            foreign,
        };
        confirm(&mut app, fix(false)).unwrap();
        assert_eq!(app.links.len(), 1);
        assert!(is_foreign(&app.links[0]));
        confirm(&mut app, fix(true)).unwrap();
        assert!(app.links.is_empty());
    }

    #[test]
    fn project_settings_set_the_pdk() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app(tmp.path());
        let Dialog::ProjectSettings(mut settings) =
            Dialog::project_settings(app.project.as_ref().unwrap())
        else {
            unreachable!()
        };
        assert_eq!(settings.name, "demo");
        settings.technology = "ihp-sg13g2".into();
        settings.pdk_root = "/opt/ext/OpenPDKs".into();
        confirm(&mut app, Dialog::ProjectSettings(settings)).unwrap();
        let project = app.project.as_ref().unwrap();
        assert_eq!(app.pdk().as_deref(), Some("ihp-sg13g2"));
        assert_eq!(app.pdk_root().as_deref(), Some("/opt/ext/OpenPDKs"));
        // The libraries survive the edit.
        assert_eq!(project.libraries.len(), 3);
        assert!(
            fs::read_to_string(&project.file)
                .unwrap()
                .contains("analog = \"libs/analog\"")
        );
    }

    #[test]
    fn dialogs_run_library_operations() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app(tmp.path());
        let root = app.project.as_ref().unwrap().root.clone();
        let text = |s: &str| s.to_string();

        confirm(
            &mut app,
            Dialog::NewLibrary {
                name: text("digital"),
                path: text("libs/digital"),
                custom_path: false,
            },
        )
        .unwrap();
        assert_eq!(app.selection, Selection::library("digital"));
        assert!(root.join("libs/digital/lib.toml").is_file());

        let new_cell = Dialog::NewCell {
            library: text("digital"),
            name: text("inv"),
            template: Some(Template::XschemSchematic),
        };
        confirm(&mut app, new_cell.clone()).unwrap();
        assert_eq!(app.selection, Selection::cell("digital", "inv"));
        // The same name again fails and leaves the selection alone.
        assert!(confirm(&mut app, new_cell).is_err());

        confirm(
            &mut app,
            Dialog::NewView {
                library: text("digital"),
                cell: text("inv"),
                template: Template::GdsLayout,
            },
        )
        .unwrap();
        assert_eq!(app.selection, Selection::view("digital", "inv", "layout"));

        confirm(
            &mut app,
            Dialog::CopyCell {
                library: text("prims"),
                cell: text("nmos"),
                to: text("digital"),
                name: text("my_nmos"),
            },
        )
        .unwrap();
        assert!(
            root.join("libs/digital/my_nmos/symbol/my_nmos.sym")
                .is_file()
        );

        confirm(
            &mut app,
            Dialog::RenameCell {
                library: text("digital"),
                cell: text("inv"),
                name: text("inv_x1"),
            },
        )
        .unwrap();
        assert!(root.join("libs/digital/inv_x1/layout/inv_x1.gds").is_file());

        // Read-only libraries refuse changes.
        let delete = |library: &str, cell: &str| Dialog::DeleteCell {
            library: text(library),
            cell: text(cell),
        };
        assert!(confirm(&mut app, delete("prims", "nmos")).is_err());
        assert!(root.join("prims/nmos.sym").is_file());

        confirm(
            &mut app,
            Dialog::DeleteView {
                library: text("digital"),
                cell: text("inv_x1"),
                view: text("layout"),
            },
        )
        .unwrap();
        confirm(&mut app, delete("digital", "inv_x1")).unwrap();
        assert_eq!(app.selection, Selection::library("digital"));
        assert!(!root.join("libs/digital/inv_x1").exists());

        confirm(
            &mut app,
            Dialog::RemoveLibrary {
                library: text("digital"),
            },
        )
        .unwrap();
        assert!(app.project.as_ref().unwrap().library("digital").is_none());
        assert!(root.join("libs/digital").is_dir());
    }

    /// Draw every library-related view and dialog headlessly, for several selections.
    #[test]
    fn views_and_dialogs_draw() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app(tmp.path());
        // A symbol without its schematic link: the views draw the warning.
        fs::create_dir_all(tmp.path().join("libs/analog/ota/symbol")).unwrap();
        fs::write(
            tmp.path().join("libs/analog/ota/symbol/ota.sym"),
            "G {}\nK {type=subcircuit\n}\n",
        )
        .unwrap();
        app.reload_project();
        assert_eq!(app.links.len(), 1);
        let mut views = Views::default();
        let project = app.project.clone().unwrap();
        let analog = project.library("analog").unwrap();
        let ota = analog.cell("ota").unwrap();
        let text = |s: &str| s.to_string();

        let selections = [
            Selection::default(),
            Selection::library("gone"),
            Selection::library("prims"),
            Selection::cell("prims", "nmos"),
            Selection::view("analog", "ota", "schematic"),
            Selection::view("analog", "ota", "symbol"),
        ];
        let dialogs = [
            Dialog::FixLinks {
                library: text("analog"),
                foreign: false,
            },
            Dialog::Error(text("boom")),
            Dialog::new_library(),
            Dialog::add_library(&project, &project.root.join("prims")),
            Dialog::RemoveLibrary {
                library: text("analog"),
            },
            Dialog::new_cell(analog),
            Dialog::new_view("analog", ota),
            Dialog::RenameCell {
                library: text("analog"),
                cell: text("ota"),
                name: text("ota"),
            },
            Dialog::CopyCell {
                library: text("analog"),
                cell: text("ota"),
                to: text("analog"),
                name: text("ota_copy"),
            },
            Dialog::DeleteCell {
                library: text("analog"),
                cell: text("ota"),
            },
            Dialog::DeleteView {
                library: text("analog"),
                cell: text("ota"),
                view: text("schematic"),
            },
        ];

        let ctx = Context::default();
        let frame = |app: &mut AppCtx, views: &mut Views| {
            let output = ctx.run_ui(egui::RawInput::default(), |ui| {
                egui::MenuBar::new().ui(ui, |ui| {
                    views.get(Tab::LibraryManager).menus(ui, app);
                    views.get(Tab::LibraryManager).toolbar(ui, app);
                });
                for tab in [
                    Tab::LibraryManager,
                    Tab::Libraries,
                    Tab::Inspector,
                    Tab::Start,
                ] {
                    ui.push_id(tab, |ui| views.get(tab).ui(ui, app));
                }
                show(ui.ctx(), app);
            });
            output.drop_without_applying_deltas();
        };
        for selection in selections {
            app.selection = selection;
            frame(&mut app, &mut views);
            frame(&mut app, &mut views);
        }
        for dialog in dialogs {
            app.open_dialog(dialog.clone());
            frame(&mut app, &mut views);
            frame(&mut app, &mut views);
            let (_, state) = app.dialog.as_ref().expect("the dialog stays open");
            assert_eq!(state.dialog.title(), dialog.title());
        }
        // Without a project the views fall back to their empty states.
        app.dialog = None;
        app.close_project();
        frame(&mut app, &mut views);
    }
}
