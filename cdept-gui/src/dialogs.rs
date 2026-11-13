//! Modal dialogs for project and library operations.
//!
//! A view opens one with [`AppCtx::open_dialog`]; the shell shows it over the window it
//! was opened from. Confirming runs the operation; a failure keeps the dialog open with
//! the error.

use std::path::Path;

use egui::{Context, Id, Key, RichText, Ui};
use eyre::{Result, eyre};
use libcdept::{AddLibrary, Library, LibraryLayout, Project, Template};

use crate::state::{AppCtx, Selection};

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
}

impl Dialog {
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
        }
    }

    fn confirm_label(&self) -> &'static str {
        match self {
            Self::Error(_) => "Close",
            Self::NewLibrary { .. } | Self::NewCell { .. } | Self::NewView { .. } => "Create",
            Self::AddLibrary { .. } => "Add",
            Self::RemoveLibrary { .. } => "Remove",
            Self::RenameCell { .. } => "Rename",
            Self::CopyCell { .. } => "Copy",
            Self::DeleteCell { .. } | Self::DeleteView { .. } => "Delete",
        }
    }
}

pub(crate) struct DialogState {
    dialog: Dialog,
    error: Option<String>,
    /// The first text field still has to take the keyboard focus.
    focus: bool,
}

impl DialogState {
    pub fn new(dialog: Dialog) -> Self {
        Self {
            dialog,
            error: None,
            focus: true,
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
        let is_error = matches!(state.dialog, Dialog::Error(_));
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
            if let Err(e) = apply(&state.dialog, app) {
                state.error = Some(format!("{e:#}"));
                app.dialog = Some((win, state));
            }
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
        Dialog::Error(message) => {
            ui.label(message.as_str());
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
                if field(ui, "Folder", path).changed() {
                    *custom_path = true;
                }
            });
            ui.small("The folder is relative to the project and is created if missing.");
        }
        Dialog::AddLibrary {
            name,
            path,
            read_only,
            layout,
        } => {
            grid.show(ui, |ui| {
                field(ui, "Name", name);
                field(ui, "Folder", path);
                ui.label("Layout");
                egui::ComboBox::from_id_salt("dialog-layout")
                    .selected_text(layout_label(*layout))
                    .show_ui(ui, |ui| {
                        for option in [None, Some(LibraryLayout::Cells), Some(LibraryLayout::Flat)]
                        {
                            ui.selectable_value(layout, option, layout_label(option));
                        }
                    });
                ui.end_row();
                ui.label("");
                ui.checkbox(read_only, "Read-only (reference library)");
                ui.end_row();
            });
            ui.small("$VAR and ~ in the folder are kept as written, e.g. $PDK_ROOT/….");
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
            ui.small("Files named after the cell are renamed. Other cells that use it are not updated yet.");
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
        }
        Dialog::DeleteCell { library, cell } => {
            ui.label(format!("Delete cell '{cell}' from library '{library}'?"));
            ui.label(RichText::new("All its views are deleted from disk.").strong());
        }
        Dialog::DeleteView {
            library,
            cell,
            view,
        } => {
            ui.label(format!("Delete view '{view}' of '{library} / {cell}'?"));
            ui.label(RichText::new("Its files are deleted from disk.").strong());
        }
    }
}

fn layout_label(layout: Option<LibraryLayout>) -> &'static str {
    match layout {
        None => "Detect from the folder",
        Some(LibraryLayout::Cells) => "Cells: library/cell/view/files",
        Some(LibraryLayout::Flat) => "Flat: library/cell.ext",
    }
}

fn apply(dialog: &Dialog, app: &mut AppCtx) -> Result<()> {
    if matches!(dialog, Dialog::Error(_)) {
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
        Dialog::Error(_) => return Ok(()),
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
                    target.create_view(name, *template)?;
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
            let file = library(project, lib)?.create_view(cell, *template)?;
            tracing::info!(target: TARGET, "created {}", file.display());
            Selection::view(lib, cell, template.view_name())
        }
        Dialog::RenameCell {
            library: lib,
            cell,
            name,
        } => {
            let name = name.trim();
            library(project, lib)?.rename_cell(cell, name)?;
            tracing::info!(target: TARGET, "renamed {lib}/{cell} to {name}");
            Selection::cell(lib, name)
        }
        Dialog::CopyCell {
            library: lib,
            cell,
            to,
            name,
        } => {
            let name = name.trim();
            library(project, lib)?.copy_cell(cell, &library(project, to)?, name)?;
            tracing::info!(target: TARGET, "copied {lib}/{cell} to {to}/{name}");
            Selection::cell(to, name)
        }
        Dialog::DeleteCell { library: lib, cell } => {
            library(project, lib)?.delete_cell(cell)?;
            tracing::info!(target: TARGET, "deleted cell {lib}/{cell}");
            Selection::library(lib)
        }
        Dialog::DeleteView {
            library: lib,
            cell,
            view,
        } => {
            library(project, lib)?.delete_view(cell, view)?;
            tracing::info!(target: TARGET, "deleted view {lib}/{cell}/{view}");
            Selection::cell(lib, cell)
        }
    };
    project.rescan();
    app.selection = selection;
    app.validate_selection();
    Ok(())
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
        ];
        let dialogs = [
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
