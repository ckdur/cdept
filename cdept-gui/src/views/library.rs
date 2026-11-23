//! Library Manager in two forms: a compact tree for the side panel and the full
//! 3-column Library | Cell | View browser as a document. Both show the libraries of
//! the open project and share one selection.

use egui::{Button, CollapsingHeader, RichText, ScrollArea, Sense, Ui};
use libcdept::xschem::symbol::Unlinked;
use libcdept::{Cell, Entry, Library, Project};

use super::{View, empty_state};
use crate::dialogs::Dialog;
use crate::dock::Tab;
use crate::state::{Action, AppCtx, PickFor, Selection, cell_link, is_foreign, view_link};

const READ_ONLY: &str = "The library is read-only";

/// What the user asked for while the project was borrowed for drawing.
enum Cmd {
    Select(Selection),
    Dialog(Dialog),
    Action(Action),
    CopyText(String),
    /// Write the expected `schematic=` into a symbol.
    FixLink(Unlinked),
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
            Cmd::FixLink(unlinked) => {
                ctx.fix_links(&[unlinked]);
            }
            Cmd::Open(selection) => {
                ctx.selection = selection.clone();
                ctx.request(Action::OpenView(selection));
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

/// The mark in front of a symbol view whose `schematic=` link is broken.
pub(super) const LINK_MARK: &str = "⚠";

/// What is wrong with the link, and what it should be.
pub(super) fn link_hover(u: &Unlinked) -> String {
    let mut text = format!("Broken schematic link: {}", u.problem);
    text.push_str(&format!("\nExpected schematic={}", u.expected));
    if is_foreign(u) {
        text.push_str("\nDescending from this symbol opens another cell's schematic.");
    }
    text
}

/// The menu entry (and button) that repairs the link.
pub(super) fn fix_label(u: &Unlinked) -> &'static str {
    if is_foreign(u) {
        "Point to this cell's schematic"
    } else {
        "Fix schematic link"
    }
}

/// A view name, with the warning mark when its symbol link is broken.
fn view_text(ui: &Ui, text: String, link: Option<&Unlinked>) -> RichText {
    match link {
        Some(_) => RichText::new(format!("{LINK_MARK} {text}")).color(ui.visuals().warn_fg_color),
        None => RichText::new(text),
    }
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

fn library_items(ui: &mut Ui, lib: &Library, links: &[Unlinked], cmds: &mut Vec<Cmd>) {
    if ui
        .add_enabled(writable(lib), Button::new("New Cell…"))
        .on_disabled_hover_text(READ_ONLY)
        .clicked()
    {
        cmds.push(Cmd::Dialog(Dialog::new_cell(lib)));
    }
    cmds.extend(crate::import_files::menu(ui, lib).map(Cmd::Dialog));
    let broken = links.iter().filter(|u| u.library == lib.name).count();
    if broken > 0 {
        let fix = ui
            .add_enabled(writable(lib), Button::new("Fix all schematic links…"))
            .on_hover_text(format!(
                "{broken} symbol(s) not linked to their own schematic"
            ))
            .on_disabled_hover_text(READ_ONLY);
        if fix.clicked() {
            cmds.push(Cmd::Dialog(Dialog::FixLinks {
                library: lib.name.clone(),
                foreign: false,
            }));
        }
    }
    if ui
        .button("Library Properties…")
        .on_hover_text("Folder, layout, read-only and description, where the library is defined")
        .clicked()
    {
        cmds.push(Cmd::Dialog(Dialog::LibraryProperties(
            crate::library_dialog::Properties::new(&lib.name),
        )));
    }
    super::categories::library_menu(ui, lib);
    if ui.button("Remove Library…").clicked() {
        cmds.push(Cmd::Dialog(Dialog::RemoveLibrary {
            library: lib.name.clone(),
        }));
    }
    cmds.extend(crate::refactor::library_menu(ui, lib).map(Cmd::Dialog));
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
    super::categories::cell_menu(ui, lib, cell);
    if let Some(tb) = cell
        .entries
        .iter()
        .find(|e| super::simulation::is_testbench(e))
    {
        let selection = Selection::view(&lib.name, &cell.name, &tb.name);
        if ui
            .button("Run Simulation")
            .on_hover_text("Run the testbench view of this cell")
            .clicked()
        {
            cmds.push(Cmd::Action(Action::OpenSimulation {
                selection,
                run: true,
            }));
        }
    }
    cmds.extend(crate::cace::menu(ui, lib, cell).map(Cmd::Dialog));
    let netlist = ui.add_enabled_ui(rw && crate::app::schematic_file(cell).is_some(), |ui| {
        ui.menu_button("Netlist to View", |ui| {
            for (label, format) in super::NETLIST_FORMATS {
                if ui.button(label).clicked() {
                    cmds.push(Cmd::Action(Action::NetlistToView {
                        library: lib.name.clone(),
                        cell: cell.name.clone(),
                        format,
                    }));
                }
            }
        });
    });
    netlist.response.on_disabled_hover_text(if rw {
        "The cell has no xschem schematic"
    } else {
        READ_ONLY
    });
    let (dialog, action) = crate::refactor::cell_menu(ui, project, lib, cell);
    cmds.extend(dialog.map(Cmd::Dialog));
    cmds.extend(action.map(Cmd::Action));
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

fn view_items(
    ui: &mut Ui,
    lib: &Library,
    cell: &Cell,
    entry: &Entry,
    link: Option<&Unlinked>,
    cmds: &mut Vec<Cmd>,
) {
    if ui
        .add_enabled(crate::app::has_viewer(entry), Button::new("Open"))
        .on_disabled_hover_text("No embedded viewer for this kind of view yet")
        .clicked()
    {
        cmds.push(Cmd::Open(Selection::view(
            &lib.name,
            &cell.name,
            &entry.name,
        )));
    }
    if entry.format == Some(libcdept::entry::Format::QucsSchematic)
        && let Some(file) = &entry.main_file
        && ui
            .button("Open in Qucs-S")
            .on_hover_text("Start the Qucs-S window on this schematic")
            .clicked()
        && let Err(e) = libcdept::batch::qucs::launch(None, Some(file))
    {
        tracing::warn!("Open in Qucs-S: {e:#}");
    }
    if super::simulation::is_testbench(entry) {
        let selection = Selection::view(&lib.name, &cell.name, &entry.name);
        if ui
            .button("Simulation")
            .on_hover_text("Edit the setup and see the results")
            .clicked()
        {
            cmds.push(Cmd::Action(Action::OpenSimulation {
                selection: selection.clone(),
                run: false,
            }));
        }
        if ui
            .button("Run Simulation")
            .on_hover_text("Open the Simulation tab and run the testbench")
            .clicked()
        {
            cmds.push(Cmd::Action(Action::OpenSimulation {
                selection,
                run: true,
            }));
        }
    }
    if let Some(u) = link
        && ui
            .add_enabled(
                writable(lib),
                Button::new(format!("{LINK_MARK} {}", fix_label(u))),
            )
            .on_hover_text(link_hover(u))
            .on_disabled_hover_text(READ_ONLY)
            .clicked()
    {
        cmds.push(Cmd::FixLink(u.clone()));
    }
    ui.separator();
    cmds.extend(crate::refactor::view_menu(ui, lib, cell, entry).map(Cmd::Dialog));
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
    if crate::background::loading_ui(ui, ctx) {
        return;
    }
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
            super::categories::toggle_button(ui);
            ui.add(
                egui::TextEdit::singleline(&mut self.filter)
                    .hint_text("🔍 Filter cells")
                    .desired_width(f32::INFINITY),
            );
        });
        ui.separator();
        // Problems of the project file (a library defined twice, names, …).
        crate::library_dialog::problems_banner(ui, ctx);
        let Some(project) = &ctx.project else {
            return no_project(ui, ctx);
        };

        let filter = self.filter.trim().to_lowercase();
        let selection = &ctx.selection;
        let links = &ctx.links;
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
            let broken = links.iter().filter(|u| u.library == lib.name).count();
            let mut label = RichText::new(library_label(lib));
            if broken > 0 {
                label = RichText::new(format!("{} {LINK_MARK}", library_label(lib)));
            }
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
                    // One after the other, or under their categories.
                    super::categories::grouped(ui, lib, &cells, |ui, cell| {
                        let open = super::vcs::node_open(ui, ("cell", &lib.name, &cell.name));
                        if super::vcs::skip_hidden_row(ui, open) {
                            return;
                        }
                        cell_node(
                            ui, project, lib, cell, selected, selection, links, &mut cmds,
                        );
                    });
                })
                .header_response
                .on_hover_text(if broken > 0 {
                    format!(
                        "{}\n{LINK_MARK} {broken} symbol(s) with a broken schematic link",
                        library_hover(lib)
                    )
                } else {
                    library_hover(lib)
                });
            let header = super::vcs::library_badge(ui, header, lib);
            if header.clicked() {
                cmds.push(Cmd::Select(Selection::library(&lib.name)));
            }
            header.context_menu(|ui| {
                library_items(ui, lib, links, &mut cmds);
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

#[allow(clippy::too_many_arguments)]
fn cell_node(
    ui: &mut Ui,
    project: &Project,
    lib: &Library,
    cell: &Cell,
    lib_selected: bool,
    selection: &Selection,
    links: &[Unlinked],
    cmds: &mut Vec<Cmd>,
) {
    let selected = lib_selected && selection.cell.as_deref() == Some(&cell.name);
    let broken = cell_link(links, &lib.name, &cell.name);
    let mut label = RichText::new(&cell.name);
    if broken.is_some() {
        label = RichText::new(format!("{LINK_MARK} {}", cell.name))
            .color(ui.visuals().warn_fg_color.gamma_multiply(0.6));
    }
    if selected && selection.view.is_none() {
        label = label.strong();
    }
    let mut header = CollapsingHeader::new(label)
        .id_salt(("cell", &lib.name, &cell.name))
        .show(ui, |ui| {
            if cell.entries.is_empty() {
                ui.weak("No views");
            }
            for entry in &cell.entries {
                let active = selected && selection.view.as_deref() == Some(&entry.name);
                let target = Selection::view(&lib.name, &cell.name, &entry.name);
                let link = view_link(links, entry);
                let mut response =
                    ui.selectable_label(active, view_text(ui, entry.name.clone(), link));
                response = match link {
                    Some(u) => response.on_hover_text(format!(
                        "{}\n\n{}",
                        view_label(entry),
                        link_hover(u)
                    )),
                    None => response.on_hover_text(view_label(entry)),
                };
                response = super::vcs::view_badge(ui, response, entry);
                if response.double_clicked() {
                    cmds.push(Cmd::Open(target));
                } else if response.clicked() {
                    cmds.push(Cmd::Select(target));
                }
                response.context_menu(|ui| view_items(ui, lib, cell, entry, link, cmds));
            }
        })
        .header_response;
    if let Some(u) = broken {
        header = header.on_hover_text(format!(
            "A symbol of this cell has a broken schematic link:\n{}",
            link_hover(u)
        ));
    }
    header = super::vcs::cell_badge(ui, header, cell);
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
    fn columns(
        &self,
        ui: &mut Ui,
        project: &Project,
        selection: &Selection,
        links: &[Unlinked],
        cmds: &mut Vec<Cmd>,
    ) {
        let filter = self.filter.trim().to_lowercase();
        let library = selection
            .library
            .as_deref()
            .and_then(|name| project.library(name));
        let cell = library.and_then(|lib| lib.cell(selection.cell.as_deref()?));

        // With categories shown: Library | Category | Cell | View.
        let extra = usize::from(super::categories::shown(ui));
        ui.columns(3 + extra, |cols| {
            if extra == 1 {
                column(&mut cols[1], "Category", |ui| match library {
                    Some(lib) if lib.error.is_none() => super::categories::picker(ui, lib),
                    _ => empty_state(ui, "Select a library", None),
                });
            }
            column(&mut cols[0], "Library", |ui| {
                if project.libraries.is_empty() {
                    empty_state(ui, "No libraries", Some("Use Library ▸ New Library…"));
                }
                for lib in &project.libraries {
                    let selected = library.is_some_and(|l| l.name == lib.name);
                    let response = ui
                        .selectable_label(selected, library_label(lib))
                        .on_hover_text(library_hover(lib));
                    let response = super::vcs::library_badge(ui, response, lib);
                    if response.clicked() {
                        cmds.push(Cmd::Select(Selection::library(&lib.name)));
                    }
                    response.context_menu(|ui| library_items(ui, lib, links, cmds));
                }
                let rest = ui.allocate_response(ui.available_size(), Sense::click());
                rest.context_menu(|ui| project_items(ui, cmds));
            });

            let header = match library {
                Some(lib) => format!("Cell ({})", lib.cells.len()),
                None => "Cell".into(),
            };
            column(&mut cols[1 + extra], &header, |ui| {
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
                    .filter(|c| super::categories::shows(ui, lib, &c.name))
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
                // Go to Cell scrolls the list to the selected cell.
                let at = cell.and_then(|s| cells.iter().position(|c| c.name == s.name));
                let mut rows = ScrollArea::vertical().auto_shrink(false);
                if let Some(offset) = super::search::reveal_offset(ui, at, row_height) {
                    rows = rows.vertical_scroll_offset(offset);
                }
                rows.show_rows(ui, row_height, cells.len(), |ui, range| {
                    for c in &cells[range] {
                        let selected = cell.is_some_and(|s| s.name == c.name);
                        let broken = cell_link(links, &lib.name, &c.name);
                        let text = match broken {
                            Some(_) => RichText::new(format!("{LINK_MARK} {}", c.name))
                                .color(ui.visuals().warn_fg_color.gamma_multiply(0.6)),
                            None => RichText::new(&c.name),
                        };
                        let mut response = ui.selectable_label(selected, text);
                        if let Some(u) = broken {
                            response = response.on_hover_text(format!(
                                "A symbol of this cell has a broken schematic link:\n{}",
                                link_hover(u)
                            ));
                        }
                        response = super::vcs::cell_badge(ui, response, c);
                        if response.clicked() {
                            cmds.push(Cmd::Select(Selection::cell(&lib.name, &c.name)));
                        }
                        response.context_menu(|ui| cell_items(ui, project, lib, c, cmds));
                    }
                });
            });

            column(&mut cols[2 + extra], "View", |ui| {
                let (Some(lib), Some(cell)) = (library, cell) else {
                    return empty_state(ui, "Select a cell", None);
                };
                if cell.entries.is_empty() {
                    empty_state(ui, "No views", Some("Use Library ▸ New View…"));
                }
                for entry in &cell.entries {
                    let selected = selection.view.as_deref() == Some(&entry.name);
                    let target = Selection::view(&lib.name, &cell.name, &entry.name);
                    let link = view_link(links, entry);
                    let mut response =
                        ui.selectable_label(selected, view_text(ui, view_label(entry), link));
                    if let Some(u) = link {
                        response = response.on_hover_text(link_hover(u));
                    }
                    response = super::vcs::view_badge(ui, response, entry);
                    if response.double_clicked() {
                        cmds.push(Cmd::Open(target));
                    } else if response.clicked() {
                        cmds.push(Cmd::Select(target));
                    }
                    response.context_menu(|ui| view_items(ui, lib, cell, entry, link, cmds));
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
            super::categories::toggle(ui);
        });
        ui.separator();
        // Problems of the project file (a library defined twice, names, …).
        crate::library_dialog::problems_banner(ui, ctx);
        let Some(project) = &ctx.project else {
            return no_project(ui, ctx);
        };
        let mut cmds = Vec::new();
        self.columns(ui, project, &ctx.selection, &ctx.links, &mut cmds);
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
                Some(lib) => library_items(ui, lib, &ctx.links, &mut cmds),
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
                (Some(lib), Some(cell), Some(entry)) => {
                    let link = view_link(&ctx.links, entry);
                    view_items(ui, lib, cell, entry, link, &mut cmds)
                }
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
