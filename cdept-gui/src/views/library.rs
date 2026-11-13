//! Library Manager in two forms: a compact tree for the side panel and the full
//! 3-column Library | Cell | View browser as a document. Both show the libraries of
//! the open project and share one selection.

use egui::{Button, CollapsingHeader, RichText, ScrollArea, Sense, Ui};
use libcdept::{Cell, Entry, Library, Project};

use super::{View, empty_state};
use crate::dialogs::Dialog;
use crate::dock::Tab;
use crate::state::{Action, AppCtx, PickFor, Selection};

const READ_ONLY: &str = "The library is read-only";
const OPEN_PHASE: &str = "opening views in their tool arrives in Phase 3";

/// What the user asked for while the project was borrowed for drawing.
enum Cmd {
    Select(Selection),
    Dialog(Dialog),
    Action(Action),
    CopyText(String),
    /// Double-click on a view.
    Open(Selection),
}

fn run(cmds: Vec<Cmd>, ui: &Ui, ctx: &mut AppCtx) {
    for cmd in cmds {
        match cmd {
            Cmd::Select(selection) => ctx.selection = selection,
            Cmd::Dialog(dialog) => ctx.open_dialog(dialog),
            Cmd::Action(action) => ctx.request(action),
            Cmd::CopyText(text) => ui.ctx().copy_text(text),
            Cmd::Open(selection) => {
                tracing::info!(
                    target: "cdept::library",
                    "{}/{}/{}: {OPEN_PHASE}",
                    selection.library.as_deref().unwrap_or_default(),
                    selection.cell.as_deref().unwrap_or_default(),
                    selection.view.as_deref().unwrap_or_default(),
                );
                ctx.selection = selection;
            }
        }
    }
}

fn writable(lib: &Library) -> bool {
    !lib.read_only && lib.error.is_none()
}

fn library_label(lib: &Library) -> String {
    if lib.error.is_some() {
        format!("⚠ {}", lib.name)
    } else if lib.read_only {
        format!("🔒 {}", lib.name)
    } else {
        lib.name.clone()
    }
}

fn library_hover(lib: &Library) -> String {
    let mut text = format!("{}\n{} layout", lib.path.display(), lib.layout.name());
    if lib.read_only {
        text.push_str(", read-only");
    }
    if let Some(description) = &lib.description {
        text.push_str(&format!("\n{description}"));
    }
    if let Some(error) = &lib.error {
        text.push_str(&format!("\n⚠ {error}"));
    }
    text
}

fn view_label(entry: &Entry) -> String {
    match entry.format {
        Some(format) => format!("{}  ({})", entry.name, format.label()),
        None => entry.name.clone(),
    }
}

fn copy_cell_dialog(project: &Project, lib: &Library, cell: &Cell) -> Dialog {
    // Copy into the same library when possible, else the first writable one.
    let to = if writable(lib) {
        lib.name.clone()
    } else {
        project
            .libraries
            .iter()
            .find(|l| writable(l))
            .map(|l| l.name.clone())
            .unwrap_or_default()
    };
    Dialog::CopyCell {
        library: lib.name.clone(),
        cell: cell.name.clone(),
        to,
        name: format!("{}_copy", cell.name),
    }
}

fn project_items(ui: &mut Ui, cmds: &mut Vec<Cmd>) {
    if ui.button("New Library…").clicked() {
        cmds.push(Cmd::Dialog(Dialog::new_library()));
    }
    if ui
        .button("Add Existing Library…")
        .on_hover_text("Define an existing folder (e.g. a PDK library) as a library.")
        .clicked()
    {
        cmds.push(Cmd::Action(Action::PickFolder(PickFor::AddLibrary)));
    }
}

fn library_items(ui: &mut Ui, lib: &Library, cmds: &mut Vec<Cmd>) {
    if ui
        .add_enabled(writable(lib), Button::new("New Cell…"))
        .on_disabled_hover_text(READ_ONLY)
        .clicked()
    {
        cmds.push(Cmd::Dialog(Dialog::new_cell(lib)));
    }
    if ui.button("Remove Library…").clicked() {
        cmds.push(Cmd::Dialog(Dialog::RemoveLibrary {
            library: lib.name.clone(),
        }));
    }
    if ui.button("Copy Folder Path").clicked() {
        cmds.push(Cmd::CopyText(lib.path.display().to_string()));
    }
}

fn cell_items(ui: &mut Ui, project: &Project, lib: &Library, cell: &Cell, cmds: &mut Vec<Cmd>) {
    let rw = writable(lib);
    if ui
        .add_enabled(rw, Button::new("New View…"))
        .on_disabled_hover_text(READ_ONLY)
        .clicked()
    {
        cmds.push(Cmd::Dialog(Dialog::new_view(&lib.name, cell)));
    }
    if ui.button("Copy Cell…").clicked() {
        cmds.push(Cmd::Dialog(copy_cell_dialog(project, lib, cell)));
    }
    if ui
        .add_enabled(rw, Button::new("Rename Cell…"))
        .on_disabled_hover_text(READ_ONLY)
        .clicked()
    {
        cmds.push(Cmd::Dialog(Dialog::RenameCell {
            library: lib.name.clone(),
            cell: cell.name.clone(),
            name: cell.name.clone(),
        }));
    }
    if ui
        .add_enabled(rw, Button::new("Delete Cell…"))
        .on_disabled_hover_text(READ_ONLY)
        .clicked()
    {
        cmds.push(Cmd::Dialog(Dialog::DeleteCell {
            library: lib.name.clone(),
            cell: cell.name.clone(),
        }));
    }
}

fn view_items(ui: &mut Ui, lib: &Library, cell: &Cell, entry: &Entry, cmds: &mut Vec<Cmd>) {
    if ui
        .add_enabled(writable(lib), Button::new("Delete View…"))
        .on_disabled_hover_text(READ_ONLY)
        .clicked()
    {
        cmds.push(Cmd::Dialog(Dialog::DeleteView {
            library: lib.name.clone(),
            cell: cell.name.clone(),
            view: entry.name.clone(),
        }));
    }
    if ui.button("Copy File Path").clicked() {
        let path = entry.main_file.as_ref().unwrap_or(&entry.path);
        cmds.push(Cmd::CopyText(path.display().to_string()));
    }
}

fn matches(name: &str, filter: &str) -> bool {
    filter.is_empty() || name.to_lowercase().contains(filter)
}

/// Shown instead of the libraries while no project is open.
fn no_project(ui: &mut Ui, ctx: &mut AppCtx) {
    empty_state(ui, "No project open.", None);
    ui.add_space(8.0);
    ui.vertical_centered(|ui| {
        if ui.button("Open Project…").clicked() {
            ctx.request(Action::PickFolder(PickFor::OpenProject));
        }
        if ui.button("New Project…").clicked() {
            ctx.request(Action::PickFolder(PickFor::NewProject));
        }
    });
}

/// Side panel: one tree, lib ▸ cell ▸ view.
#[derive(Default)]
pub struct LibrariesView {
    filter: String,
}

impl View for LibrariesView {
    fn title(&self, _ctx: &AppCtx) -> String {
        "Libraries".into()
    }

    fn ui(&mut self, ui: &mut Ui, ctx: &mut AppCtx) {
        ui.horizontal(|ui| {
            if ui
                .button("⟳")
                .on_hover_text("Read cdept.toml and the library folders again")
                .clicked()
            {
                ctx.request(Action::ReloadProject);
            }
            if ui
                .button("☰")
                .on_hover_text("Open the Library Manager")
                .clicked()
            {
                ctx.request(Action::OpenTab(Tab::LibraryManager));
            }
            ui.add(
                egui::TextEdit::singleline(&mut self.filter)
                    .hint_text("🔍 Filter cells")
                    .desired_width(f32::INFINITY),
            );
        });
        ui.separator();
        let Some(project) = &ctx.project else {
            return no_project(ui, ctx);
        };

        let filter = self.filter.trim().to_lowercase();
        let selection = &ctx.selection;
        let mut cmds = Vec::new();
        if project.libraries.is_empty() {
            empty_state(ui, "No libraries.", Some("Right-click to add one."));
        }
        for lib in &project.libraries {
            let cells: Vec<&Cell> = lib
                .cells
                .iter()
                .filter(|c| matches(&c.name, &filter))
                .collect();
            if !filter.is_empty() && cells.is_empty() {
                continue;
            }
            let selected = selection.library.as_deref() == Some(&lib.name);
            let mut label = RichText::new(library_label(lib));
            if selected && selection.cell.is_none() {
                label = label.strong();
            }
            let header = CollapsingHeader::new(label)
                .id_salt(("lib", &lib.name))
                .open((!filter.is_empty()).then_some(true))
                .show(ui, |ui| {
                    if let Some(error) = &lib.error {
                        ui.colored_label(ui.visuals().warn_fg_color, error);
                    } else if cells.is_empty() {
                        ui.weak("No cells");
                    }
                    for cell in cells {
                        cell_node(ui, project, lib, cell, selected, selection, &mut cmds);
                    }
                })
                .header_response
                .on_hover_text(library_hover(lib));
            if header.clicked() {
                cmds.push(Cmd::Select(Selection::library(&lib.name)));
            }
            header.context_menu(|ui| {
                library_items(ui, lib, &mut cmds);
                ui.separator();
                project_items(ui, &mut cmds);
            });
        }
        // Right-click on the empty area below the tree.
        let rest = ui.allocate_response(ui.available_size(), Sense::click());
        rest.context_menu(|ui| project_items(ui, &mut cmds));
        run(cmds, ui, ctx);
    }
}

fn cell_node(
    ui: &mut Ui,
    project: &Project,
    lib: &Library,
    cell: &Cell,
    lib_selected: bool,
    selection: &Selection,
    cmds: &mut Vec<Cmd>,
) {
    let selected = lib_selected && selection.cell.as_deref() == Some(&cell.name);
    let mut label = RichText::new(&cell.name);
    if selected && selection.view.is_none() {
        label = label.strong();
    }
    let header = CollapsingHeader::new(label)
        .id_salt(("cell", &lib.name, &cell.name))
        .show(ui, |ui| {
            if cell.entries.is_empty() {
                ui.weak("No views");
            }
            for entry in &cell.entries {
                let active = selected && selection.view.as_deref() == Some(&entry.name);
                let target = Selection::view(&lib.name, &cell.name, &entry.name);
                let response = ui
                    .selectable_label(active, &entry.name)
                    .on_hover_text(view_label(entry));
                if response.double_clicked() {
                    cmds.push(Cmd::Open(target));
                } else if response.clicked() {
                    cmds.push(Cmd::Select(target));
                }
                response.context_menu(|ui| view_items(ui, lib, cell, entry, cmds));
            }
        })
        .header_response;
    if header.clicked() {
        cmds.push(Cmd::Select(Selection::cell(&lib.name, &cell.name)));
    }
    header.context_menu(|ui| cell_items(ui, project, lib, cell, cmds));
}

/// Document: the classic three-column browser.
#[derive(Default)]
pub struct LibraryManagerView {
    filter: String,
}

impl LibraryManagerView {
    fn columns(&self, ui: &mut Ui, project: &Project, selection: &Selection, cmds: &mut Vec<Cmd>) {
        let filter = self.filter.trim().to_lowercase();
        let library = selection
            .library
            .as_deref()
            .and_then(|name| project.library(name));
        let cell = library.and_then(|lib| lib.cell(selection.cell.as_deref()?));

        ui.columns(3, |cols| {
            column(&mut cols[0], "Library", |ui| {
                if project.libraries.is_empty() {
                    empty_state(ui, "No libraries", Some("Use Library ▸ New Library…"));
                }
                for lib in &project.libraries {
                    let selected = library.is_some_and(|l| l.name == lib.name);
                    let response = ui
                        .selectable_label(selected, library_label(lib))
                        .on_hover_text(library_hover(lib));
                    if response.clicked() {
                        cmds.push(Cmd::Select(Selection::library(&lib.name)));
                    }
                    response.context_menu(|ui| library_items(ui, lib, cmds));
                }
                let rest = ui.allocate_response(ui.available_size(), Sense::click());
                rest.context_menu(|ui| project_items(ui, cmds));
            });

            let header = match library {
                Some(lib) => format!("Cell ({})", lib.cells.len()),
                None => "Cell".into(),
            };
            column(&mut cols[1], &header, |ui| {
                let Some(lib) = library else {
                    return empty_state(ui, "Select a library", None);
                };
                if let Some(error) = &lib.error {
                    return empty_state(ui, "Library not available", Some(error));
                }
                let cells: Vec<&Cell> = lib
                    .cells
                    .iter()
                    .filter(|c| matches(&c.name, &filter))
                    .collect();
                if cells.is_empty() {
                    let text = if lib.cells.is_empty() {
                        "No cells"
                    } else {
                        "No cell matches the filter"
                    };
                    empty_state(ui, text, None);
                }
                // Reference libraries can hold thousands of cells: draw visible rows only.
                let row_height = ui.spacing().interact_size.y;
                ScrollArea::vertical().auto_shrink(false).show_rows(
                    ui,
                    row_height,
                    cells.len(),
                    |ui, range| {
                        for c in &cells[range] {
                            let selected = cell.is_some_and(|s| s.name == c.name);
                            let response = ui.selectable_label(selected, &c.name);
                            if response.clicked() {
                                cmds.push(Cmd::Select(Selection::cell(&lib.name, &c.name)));
                            }
                            response.context_menu(|ui| cell_items(ui, project, lib, c, cmds));
                        }
                    },
                );
            });

            column(&mut cols[2], "View", |ui| {
                let (Some(lib), Some(cell)) = (library, cell) else {
                    return empty_state(ui, "Select a cell", None);
                };
                if cell.entries.is_empty() {
                    empty_state(ui, "No views", Some("Use Library ▸ New View…"));
                }
                for entry in &cell.entries {
                    let selected = selection.view.as_deref() == Some(&entry.name);
                    let target = Selection::view(&lib.name, &cell.name, &entry.name);
                    let response = ui.selectable_label(selected, view_label(entry));
                    if response.double_clicked() {
                        cmds.push(Cmd::Open(target));
                    } else if response.clicked() {
                        cmds.push(Cmd::Select(target));
                    }
                    response.context_menu(|ui| view_items(ui, lib, cell, entry, cmds));
                }
            });
        });
    }
}

/// One column of the browser: a header and a framed, scrollable list.
fn column(ui: &mut Ui, header: &str, content: impl FnOnce(&mut Ui)) {
    ui.strong(header);
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_min_size(ui.available_size());
        ScrollArea::vertical()
            .id_salt(("column", header.split(' ').next()))
            .auto_shrink(false)
            .show(ui, content);
    });
}

impl View for LibraryManagerView {
    fn title(&self, _ctx: &AppCtx) -> String {
        "Library Manager".into()
    }

    fn ui(&mut self, ui: &mut Ui, ctx: &mut AppCtx) {
        ui.horizontal(|ui| {
            ui.label("Filter cells:");
            ui.text_edit_singleline(&mut self.filter);
            if !self.filter.is_empty() && ui.button("✖").clicked() {
                self.filter.clear();
            }
        });
        ui.separator();
        let Some(project) = &ctx.project else {
            return no_project(ui, ctx);
        };
        let mut cmds = Vec::new();
        self.columns(ui, project, &ctx.selection, &mut cmds);
        run(cmds, ui, ctx);
    }

    fn menus(&mut self, ui: &mut Ui, ctx: &mut AppCtx) {
        let mut cmds = Vec::new();
        ui.menu_button("Library", |ui| {
            let Some(project) = &ctx.project else {
                ui.weak("No project open");
                return;
            };
            project_items(ui, &mut cmds);
            let library = ctx.selected_library();
            let cell = ctx.selected_cell();
            ui.separator();
            match library {
                Some(lib) => library_items(ui, lib, &mut cmds),
                None => {
                    ui.weak("No library selected");
                }
            }
            ui.separator();
            match (library, cell) {
                (Some(lib), Some(cell)) => cell_items(ui, project, lib, cell, &mut cmds),
                _ => {
                    ui.weak("No cell selected");
                }
            }
            ui.separator();
            match (library, cell, ctx.selected_view()) {
                (Some(lib), Some(cell), Some(entry)) => view_items(ui, lib, cell, entry, &mut cmds),
                _ => {
                    ui.weak("No view selected");
                }
            }
            ui.separator();
            if ui.button("Reload").clicked() {
                cmds.push(Cmd::Action(Action::ReloadProject));
            }
        });
        run(cmds, ui, ctx);
    }

    fn toolbar(&mut self, ui: &mut Ui, ctx: &mut AppCtx) {
        let mut cmds = Vec::new();
        let has_project = ctx.project.is_some();
        let library = ctx.selected_library().filter(|l| writable(l));
        let cell = ctx.selected_cell();
        if ui
            .add_enabled(has_project, Button::new("+ Library"))
            .clicked()
        {
            cmds.push(Cmd::Dialog(Dialog::new_library()));
        }
        if ui
            .add_enabled(library.is_some(), Button::new("+ Cell"))
            .on_disabled_hover_text("Select a writable library")
            .clicked()
            && let Some(lib) = library
        {
            cmds.push(Cmd::Dialog(Dialog::new_cell(lib)));
        }
        if ui
            .add_enabled(library.is_some() && cell.is_some(), Button::new("+ View"))
            .on_disabled_hover_text("Select a cell in a writable library")
            .clicked()
            && let (Some(lib), Some(cell)) = (library, cell)
        {
            cmds.push(Cmd::Dialog(Dialog::new_view(&lib.name, cell)));
        }
        if ui
            .add_enabled(has_project, Button::new("⟳"))
            .on_hover_text("Read cdept.toml and the library folders again")
            .clicked()
        {
            cmds.push(Cmd::Action(Action::ReloadProject));
        }
        run(cmds, ui, ctx);
    }
}
