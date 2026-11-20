//! Views drawn inside dock tabs.
//!
//! Every tab is backed by a [`View`]. A document view (center area) also provides the
//! contextual menus and toolbar buttons shown while it is the active document, so new
//! views bring their own menus without the shell changing.

mod console;
mod document;
mod hierarchy;
mod inspector;
mod layout;
mod library;
mod placeholder;
mod schematic;
mod start;
mod tools;

use std::collections::HashMap;

use egui::Ui;

pub use self::document::Document;
pub use self::hierarchy::HierarchyView;
pub use self::layout::{LayoutView, Replacement};
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
    console: console::ConsoleView,
    jobs: placeholder::Placeholder,
    problems: placeholder::Placeholder,
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
            console: Default::default(),
            jobs: Placeholder::new("Jobs", "No simulation or verification jobs.", "Phase 4"),
            problems: Placeholder::new("Problems", "No DRC/LVS results.", "Phase 7"),
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
            view.generate_from_schematic(ctx.project.as_ref(), false);
            false
        });
        for layout in &mut layouts {
            let Some(names) = layout.take_probe_out() else {
                continue;
            };
            let Some(file) = layout.link_schematic().map(std::path::Path::to_path_buf) else {
                continue;
            };
            for view in schematics.iter_mut().filter(|s| s.file() == file) {
                view.probe_in(&names);
            }
        }
        for view in &mut schematics {
            let Some(names) = view.take_probe_out() else {
                continue;
            };
            for layout in &mut layouts {
                if layout.link_schematic() == Some(view.file()) {
                    layout.probe_in(&names);
                }
            }
        }
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
