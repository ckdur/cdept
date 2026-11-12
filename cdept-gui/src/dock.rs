//! Dock layout: which tabs exist, where they live by default, and how they are drawn.

use egui::{Id, Ui, WidgetText};
use egui_dock::{DockState, NodeIndex, TabViewer};
use serde::{Deserialize, Serialize};

use crate::state::AppCtx;
use crate::views::Views;

/// Every tab the main window can show.
///
/// Phase 1 adds `Document(DocId)` for opened cellviews (testbench, waveform, report...).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Tab {
    Start,
    LibraryManager,
    Libraries,
    Hierarchy,
    Inspector,
    Tools,
    Console,
    Jobs,
    Problems,
}

/// Default region of a tab, used when (re)opening it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Area {
    Left,
    Center,
    Right,
    Bottom,
}

impl Tab {
    /// All tabs, in the order the Window menu lists them.
    pub const ALL: [Tab; 9] = [
        Tab::Start,
        Tab::LibraryManager,
        Tab::Libraries,
        Tab::Hierarchy,
        Tab::Inspector,
        Tab::Tools,
        Tab::Console,
        Tab::Jobs,
        Tab::Problems,
    ];

    pub fn area(self) -> Area {
        match self {
            Tab::Start | Tab::LibraryManager => Area::Center,
            Tab::Libraries | Tab::Hierarchy => Area::Left,
            Tab::Inspector | Tab::Tools => Area::Right,
            Tab::Console | Tab::Jobs | Tab::Problems => Area::Bottom,
        }
    }

    /// Documents live in the center and own the contextual menus and toolbar.
    /// Side panels (console, inspector...) never take over the menu bar.
    pub fn is_document(self) -> bool {
        self.area() == Area::Center
    }
}

/// Main window default: libraries left, documents center, inspector/tools right,
/// console/jobs/problems along the bottom.
pub fn default_layout() -> DockState<Tab> {
    let mut state = DockState::new(vec![Tab::Start]);
    let tree = state.main_surface_mut();
    let [top, _bottom] = tree.split_below(
        NodeIndex::root(),
        0.75,
        vec![Tab::Console, Tab::Jobs, Tab::Problems],
    );
    let [center, _left] = tree.split_left(top, 0.2, vec![Tab::Libraries, Tab::Hierarchy]);
    let [_center, _right] = tree.split_right(center, 0.75, vec![Tab::Inspector, Tab::Tools]);
    state
}

/// Show `tab`: focus it if open, otherwise add it next to tabs of the same area
/// (or as a floating window if that area was emptied).
pub fn open_tab(state: &mut DockState<Tab>, tab: Tab) {
    if let Some((surface, node, index)) = state.find_tab(&tab) {
        state.set_active_tab((surface, node, index));
        state.set_focused_node_and_surface((surface, node));
        return;
    }
    let sibling = state
        .iter_all_tabs()
        .find(|(_, t)| t.area() == tab.area())
        .map(|(loc, _)| loc);
    match sibling {
        Some((surface, node)) => {
            state.set_focused_node_and_surface((surface, node));
            state.push_to_focused_leaf(tab);
        }
        None => {
            state.add_window(vec![tab]);
        }
    }
}

pub fn close_tab(state: &mut DockState<Tab>, tab: Tab) {
    if let Some(loc) = state.find_tab(&tab) {
        state.remove_tab(loc);
    }
}

/// Bridges egui_dock to our [`Views`].
pub struct Viewer<'a> {
    pub views: &'a mut Views,
    pub ctx: &'a mut AppCtx,
}

impl TabViewer for Viewer<'_> {
    type Tab = Tab;

    fn title(&mut self, tab: &mut Tab) -> WidgetText {
        self.views.get(*tab).title(self.ctx).into()
    }

    fn id(&mut self, tab: &mut Tab) -> Id {
        Id::new(("cdept-tab", *tab))
    }

    fn ui(&mut self, ui: &mut Ui, tab: &mut Tab) {
        self.views.get(*tab).ui(ui, self.ctx);
    }

    fn scroll_bars(&self, tab: &Tab) -> [bool; 2] {
        // Views with their own scroll areas (tables, console) handle scrolling themselves.
        match tab {
            Tab::Console | Tab::Tools | Tab::LibraryManager => [false, false],
            _ => [true, true],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_layout_has_every_tab_once() {
        let state = default_layout();
        for tab in Tab::ALL {
            let n = state.iter_all_tabs().filter(|(_, t)| **t == tab).count();
            let expected = usize::from(tab != Tab::LibraryManager);
            assert_eq!(n, expected, "{tab:?}");
        }
    }

    #[test]
    fn layout_roundtrips_through_ron() {
        // eframe persists app state as RON.
        let state = default_layout();
        let text = ron::to_string(&state).unwrap();
        let back: DockState<Tab> = ron::from_str(&text).unwrap();
        assert_eq!(back.iter_all_tabs().count(), state.iter_all_tabs().count());
    }

    #[test]
    fn reopening_goes_to_its_area() {
        let mut state = default_layout();
        close_tab(&mut state, Tab::Jobs);
        assert!(state.find_tab(&Tab::Jobs).is_none());
        open_tab(&mut state, Tab::Jobs);
        let (s, n, _) = state.find_tab(&Tab::Jobs).unwrap();
        let (s2, n2, _) = state.find_tab(&Tab::Console).unwrap();
        assert_eq!((s, n), (s2, n2));
    }

    #[test]
    fn documents_open_next_to_start() {
        let mut state = default_layout();
        open_tab(&mut state, Tab::LibraryManager);
        let a = state.find_tab(&Tab::LibraryManager).unwrap();
        let b = state.find_tab(&Tab::Start).unwrap();
        assert_eq!((a.0, a.1), (b.0, b.1));
    }
}
