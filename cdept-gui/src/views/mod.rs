//! Views drawn inside dock tabs.
//!
//! Every tab is backed by a [`View`]. A document view (center area) also provides the
//! contextual menus and toolbar buttons shown while it is the active document, so new
//! views bring their own menus without the shell changing.

mod console;
mod hierarchy;
mod inspector;
mod library;
mod placeholder;
mod schematic;
mod start;
mod tools;

use std::collections::HashMap;

use egui::Ui;

pub use self::hierarchy::HierarchyView;
pub use self::schematic::{CloseChoice, NETLIST_FORMATS, Picture, SchematicView, Selected};
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
    console: console::ConsoleView,
    jobs: placeholder::Placeholder,
    problems: placeholder::Placeholder,
    /// The open documents, by the id of their tab.
    documents: HashMap<DocId, SchematicView>,
    /// Stands in for a document tab that has no view (any more).
    missing: placeholder::Placeholder,
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
            console: Default::default(),
            jobs: Placeholder::new("Jobs", "No simulation or verification jobs.", "Phase 4"),
            problems: Placeholder::new("Problems", "No DRC/LVS results.", "Phase 7"),
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
            Tab::Console => &mut self.console,
            Tab::Jobs => &mut self.jobs,
            Tab::Problems => &mut self.problems,
            Tab::Document(id) => match self.documents.get_mut(&id) {
                Some(view) => view,
                None => &mut self.missing,
            },
        }
    }

    pub fn hierarchy(&self) -> &HierarchyView {
        &self.hierarchy
    }

    /// The open documents.
    pub fn documents(&self) -> impl Iterator<Item = (DocId, &SchematicView)> {
        self.documents.iter().map(|(id, view)| (*id, view))
    }

    pub fn add_document(&mut self, id: DocId, view: SchematicView) {
        self.documents.insert(id, view);
    }

    pub fn document(&self, id: DocId) -> Option<&SchematicView> {
        self.documents.get(&id)
    }

    pub fn document_mut(&mut self, id: DocId) -> Option<&mut SchematicView> {
        self.documents.get_mut(&id)
    }

    /// Close the documents `keep` rejects; their xschem ends with them.
    pub fn retain_documents(&mut self, keep: impl Fn(DocId) -> bool) {
        self.documents.retain(|id, _| keep(*id));
    }
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
