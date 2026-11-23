//! Views drawn inside dock tabs.
//!
//! Every tab is backed by a [`View`]. A document view (center area) also provides the
//! contextual menus and toolbar buttons shown while it is the active document, so new
//! views bring their own menus without the shell changing.

pub(crate) mod categories;
mod console;
mod contained;
mod document;
mod hierarchy;
mod inspector;
mod layout;
mod library;
mod placeholder;
mod schematic;
pub(crate) mod search;
pub(crate) mod simulation;
mod start;
pub(crate) mod technology;
mod testlib;
mod tools;
pub(crate) mod vcs;

use std::collections::HashMap;

use egui::Ui;

pub use self::document::Document;
pub use self::hierarchy::HierarchyView;
pub use self::layout::{LayoutView, Replacement, member_of as layout_member_of};
pub use self::schematic::{
    CloseChoice, NETLIST_FORMATS, Picture, SchematicView, Selected, pretend_clipboard,
};
use crate::dock::{DocId, Tab};
use crate::state::AppCtx;

/// Which Edit-menu actions the active document supports.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EditCaps {
    pub undo: bool,
    pub redo: bool,
    pub cut: bool,
    pub copy: bool,
    pub paste: bool,
    pub select_all: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditAction {
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
}

pub trait View {
    fn title(&self, ctx: &AppCtx) -> String;

    fn ui(&mut self, ui: &mut Ui, ctx: &mut AppCtx);

    /// Contextual menus, shown in the menu bar between View and Project.
    /// Each call to `ui.menu_button` adds one top-level menu.
    fn menus(&mut self, _ui: &mut Ui, _ctx: &mut AppCtx) {}

    /// Contextual toolbar buttons, shown after the global ones.
    fn toolbar(&mut self, _ui: &mut Ui, _ctx: &mut AppCtx) {}

    /// Entries added to the global File menu, after the project entries.
    fn file_menu(&mut self, _ui: &mut Ui, _ctx: &mut AppCtx) {}

    fn edit_caps(&self) -> EditCaps {
        EditCaps::default()
    }

    fn edit(&mut self, _action: EditAction, _ctx: &mut AppCtx) {}
}

/// State of all views of the main window.
pub struct Views {
    start: start::StartView,
    library_manager: library::LibraryManagerView,
    libraries: library::LibrariesView,
    hierarchy: HierarchyView,
    inspector: inspector::InspectorView,
    tools: tools::ToolsView,
    technology: technology::TechnologyView,
    simulation: simulation::SimulationView,
    console: console::ConsoleView,
    jobs: placeholder::Placeholder,
    problems: placeholder::Placeholder,
    search: search::SearchView,
    /// The open documents, by the id of their tab.
    documents: HashMap<DocId, Document>,
    /// Stands in for a document tab that has no view (any more).
    missing: placeholder::Placeholder,
    /// Layout files to generate from their schematic once their tab is ready
    /// (docs/LINKING.md §6).
    generate: Vec<std::path::PathBuf>,
}

impl Default for Views {
    fn default() -> Self {
        use placeholder::Placeholder;
        Self {
            start: Default::default(),
            library_manager: Default::default(),
            libraries: Default::default(),
            hierarchy: Default::default(),
            inspector: Default::default(),
            tools: Default::default(),
            technology: Default::default(),
            simulation: Default::default(),
            console: Default::default(),
            jobs: Placeholder::new("Jobs", "No simulation or verification jobs.", "Phase 4"),
            problems: Placeholder::new("Problems", "No DRC/LVS results.", "Phase 7"),
            search: Default::default(),
            generate: Vec::new(),
            documents: HashMap::new(),
            missing: Placeholder::new("Document", "This document is closed.", "a later step"),
        }
    }
}

impl Views {
    pub fn get(&mut self, tab: Tab) -> &mut dyn View {
        match tab {
            Tab::Start => &mut self.start,
            Tab::LibraryManager => &mut self.library_manager,
            Tab::Libraries => &mut self.libraries,
            Tab::Hierarchy => &mut self.hierarchy,
            Tab::Inspector => &mut self.inspector,
            Tab::Tools => &mut self.tools,
            Tab::Technology => &mut self.technology,
            Tab::Simulation => &mut self.simulation,
            Tab::Console => &mut self.console,
            Tab::Jobs => &mut self.jobs,
            Tab::Problems => &mut self.problems,
            Tab::Search => &mut self.search,
            Tab::Document(id) => match self.documents.get_mut(&id) {
                Some(view) => view,
                None => &mut self.missing,
            },
        }
    }

    /// The Simulation tab.
    pub(crate) fn simulation_mut(&mut self) -> &mut simulation::SimulationView {
        &mut self.simulation
    }

    /// The Search panel.
    pub(crate) fn search_mut(&mut self) -> &mut search::SearchView {
        &mut self.search
    }

    pub fn hierarchy(&self) -> &HierarchyView {
        &self.hierarchy
    }

    pub fn hierarchy_mut(&mut self) -> &mut HierarchyView {
        &mut self.hierarchy
    }

    /// The open documents.
    pub fn documents(&self) -> impl Iterator<Item = (DocId, &Document)> {
        self.documents.iter().map(|(id, view)| (*id, view))
    }

    pub fn add_document(&mut self, id: DocId, view: impl Into<Document>) {
        self.documents.insert(id, view.into());
    }

    pub fn document(&self, id: DocId) -> Option<&Document> {
        self.documents.get(&id)
    }

    pub fn document_mut(&mut self, id: DocId) -> Option<&mut Document> {
        self.documents.get_mut(&id)
    }

    /// Keep the layout tabs in step (editing in place, docs/KLAYOUT.md §3.4).
    pub fn coordinate_layouts(&mut self) {
        layout::coordinate(
            self.documents
                .values_mut()
                .filter_map(Document::as_layout_mut)
                .collect(),
        );
    }

    /// Linking schematics with layouts (docs/LINKING.md §6): Generate requests of
    /// schematic tabs, and cross-probing (a selection of instances in one tab selects
    /// the same instances in the tabs linked with it).
    pub fn coordinate_links(&mut self, ctx: &mut AppCtx) {
        let mut schematics = Vec::new();
        let mut layouts = Vec::new();
        for doc in self.documents.values_mut() {
            match doc {
                Document::Schematic(view) => schematics.push(view),
                Document::Layout(view) => layouts.push(view),
            }
        }
        for view in &mut schematics {
            if let Some(schematic::LinkRequest::Generate(file)) = view.take_link_request() {
                self.generate.push(file);
            }
        }
        self.generate.retain(|file| {
            let Some(view) = layouts.iter_mut().find(|l| l.file() == file.as_path()) else {
                return true; // its tab is not open yet
            };
            if !view.link_ready() {
                return true;
            }
            view.request_generate(false);
            false
        });
        // Generate compares with the schematic as saved: an open schematic with
        // unsaved changes is saved first (as for Update Connectivity, below).
        for layout in &mut layouts {
            if layout.generate_wanted().is_none() {
                continue;
            }
            let Some(file) = layout.link_schematic().map(std::path::Path::to_path_buf) else {
                layout.start_generate(ctx.project.as_ref());
                continue;
            };
            if let Some(request) = layout.generate_save_request() {
                let Some(view) = schematics.iter_mut().find(|s| s.file() == file) else {
                    layout.start_generate(ctx.project.as_ref());
                    continue;
                };
                if let Some(answer) = view.take_reply(request) {
                    match schematic::SchematicView::saved(answer) {
                        Ok(()) => layout.start_generate(ctx.project.as_ref()),
                        Err(error) => layout.generate_not_started(&error),
                    }
                }
                continue;
            }
            match schematics
                .iter_mut()
                .find(|s| s.file() == file && s.is_modified())
            {
                Some(view) => {
                    let request = view.save();
                    layout.generate_saving(file, request);
                }
                None => layout.start_generate(ctx.project.as_ref()),
            }
        }
        // Cross-probing (docs/LINKING.md §6, §9). Names are instance (or net) paths:
        // a layout tab's from its top cell, a schematic tab's from its own schematic.
        // A layout linked with the schematic of the tab maps one to one; one linked
        // with the schematic xschem shows after descending (into x1) maps below it.
        for layout in &mut layouts {
            let out = (layout.take_probe_out(), layout.take_net_probe_out());
            if out == (None, None) {
                continue;
            }
            let Some(file) = layout.link_schematic().map(std::path::Path::to_path_buf) else {
                continue;
            };
            for view in &mut schematics {
                let Some(prefix) = link_prefix(view, &file) else {
                    continue;
                };
                let lift = |names: &[String]| -> Vec<String> {
                    names.iter().map(|n| format!("{prefix}{n}")).collect()
                };
                if let Some(names) = &out.0 {
                    view.probe_in(&lift(names));
                }
                if let Some(names) = &out.1 {
                    view.probe_nets_in(&lift(names));
                }
            }
        }
        for view in &mut schematics {
            let out = (view.take_probe_out(), view.take_net_probe_out());
            if out == (None, None) {
                continue;
            }
            for layout in &mut layouts {
                let Some(file) = layout.link_schematic().map(std::path::Path::to_path_buf) else {
                    continue;
                };
                let Some(prefix) = link_prefix(view, &file) else {
                    continue;
                };
                let lower = |names: &[String]| -> Vec<String> {
                    names
                        .iter()
                        .filter_map(|n| n.strip_prefix(prefix.as_str()))
                        .map(str::to_string)
                        .collect()
                };
                if let Some(names) = &out.0 {
                    layout.probe_in(&lower(names));
                }
                if let Some(names) = &out.1 {
                    let names = lower(names);
                    if !names.is_empty() {
                        layout.probe_nets_in(&names);
                    }
                }
            }
        }
        // Update Connectivity compares with the schematic as saved: an open schematic
        // with unsaved changes is saved first.
        for layout in &mut layouts {
            if !layout.connectivity_requested() {
                continue;
            }
            let Some(file) = layout.link_schematic().map(std::path::Path::to_path_buf) else {
                layout.start_connectivity();
                continue;
            };
            if let Some(request) = layout.connectivity_save_request() {
                let Some(view) = schematics.iter_mut().find(|s| s.file() == file) else {
                    layout.start_connectivity(); // closed meanwhile (xschem asked)
                    continue;
                };
                if let Some(answer) = view.take_reply(request) {
                    match schematic::SchematicView::saved(answer) {
                        Ok(()) => layout.start_connectivity(),
                        Err(error) => layout.connectivity_not_started(&error),
                    }
                }
                continue;
            }
            match schematics
                .iter_mut()
                .find(|s| s.file() == file && s.is_modified())
            {
                Some(view) => {
                    let request = view.save();
                    layout.connectivity_saving(file, request);
                }
                None => layout.start_connectivity(),
            }
        }
    }

    /// Close the documents `keep` rejects; their xschem ends with them.
    pub fn retain_documents(&mut self, keep: impl Fn(DocId) -> bool) {
        self.documents.retain(|id, _| keep(*id));
    }
}

/// How a layout linked with the schematic `file` sees the names of a schematic tab:
/// `Some("")` when the tab is that schematic, `Some("x1/")` when xschem shows it inside
/// instance x1 of the tab's schematic, `None` when the tab has nothing to do with it.
fn link_prefix(view: &schematic::SchematicView, file: &std::path::Path) -> Option<String> {
    if view.file() == file {
        return Some(String::new());
    }
    let prefix = view.hierarchy_prefix();
    let shown = view.shown_schematic()?;
    let same = |a: &std::path::Path, b: &std::path::Path| {
        a == b
            || a.canonicalize()
                .ok()
                .is_some_and(|a| b.canonicalize().ok() == Some(a))
    };
    (!prefix.is_empty() && same(shown, file)).then_some(prefix)
}

/// Grey, centered text for empty states.
pub(crate) fn empty_state(ui: &mut Ui, text: &str, detail: Option<&str>) {
    ui.add_space(12.0);
    ui.vertical_centered(|ui| {
        ui.weak(text);
        if let Some(detail) = detail {
            ui.small(detail);
        }
    });
}
