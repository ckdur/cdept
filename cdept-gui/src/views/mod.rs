//! Views drawn inside dock tabs.
//!
//! Every tab is backed by a [`View`]. A document view (center area) also provides the
//! contextual menus and toolbar buttons shown while it is the active document, so new
//! views bring their own menus without the shell changing.

mod console;
mod library;
mod placeholder;
mod start;
mod tools;

use egui::Ui;

use crate::dock::Tab;
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
    hierarchy: placeholder::Placeholder,
    inspector: placeholder::Placeholder,
    tools: tools::ToolsView,
    console: console::ConsoleView,
    jobs: placeholder::Placeholder,
    problems: placeholder::Placeholder,
}

impl Default for Views {
    fn default() -> Self {
        use placeholder::Placeholder;
        Self {
            start: Default::default(),
            library_manager: Default::default(),
            libraries: Default::default(),
            hierarchy: Placeholder::new(
                "Hierarchy",
                "Open a cell to see its hierarchy.",
                "Phase 1",
            ),
            inspector: Placeholder::new("Inspector", "Nothing selected.", "Phase 1"),
            tools: Default::default(),
            console: Default::default(),
            jobs: Placeholder::new("Jobs", "No simulation or verification jobs.", "Phase 4"),
            problems: Placeholder::new("Problems", "No DRC/LVS results.", "Phase 7"),
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
        }
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
