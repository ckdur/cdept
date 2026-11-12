//! Dock layout: which tabs exist, which native window each lives in, and how they are drawn.
//!
//! The main window and every detached native window each own a [`DockState`]. A tab is
//! open in at most one of them; [`Layout`] keeps that invariant.

use egui::{Id, Ui, ViewportId, WidgetText};
use egui_dock::{DockState, NodeIndex, NodePath, TabViewer};
use serde::{Deserialize, Serialize};

use crate::state::{Action, AppCtx};
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
    let mut state = labelled(DockState::new(vec![Tab::Start]));
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

/// Our wording for egui_dock's built-in tab menu ("Eject" floats the tab inside the
/// same native window; "Open in New Window" is ours). Translations are not persisted.
fn labelled(mut state: DockState<Tab>) -> DockState<Tab> {
    state.translations.tab_context_menu.eject_button = "Float in This Window".into();
    state
}

/// A native OS window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WindowId {
    Main,
    Detached(u64),
}

impl WindowId {
    pub fn viewport(self) -> ViewportId {
        match self {
            WindowId::Main => ViewportId::ROOT,
            WindowId::Detached(n) => ViewportId::from_hash_of(("cdept-window", n)),
        }
    }
}

/// Last known position and inner size of a detached window, in points.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Geometry {
    pub pos: [f32; 2],
    pub size: [f32; 2],
}

/// A detached native window with its own dock area.
#[derive(Serialize, Deserialize)]
pub struct Detached {
    pub id: u64,
    pub dock: DockState<Tab>,
    pub geometry: Option<Geometry>,
}

/// All native windows and their tabs.
#[derive(Serialize, Deserialize)]
pub struct Layout {
    pub main: DockState<Tab>,
    pub windows: Vec<Detached>,
    next_id: u64,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            main: default_layout(),
            windows: Vec::new(),
            next_id: 1,
        }
    }
}

impl Layout {
    /// Restore state that is not persisted, after deserializing.
    pub fn restored(mut self) -> Self {
        let main = std::mem::replace(&mut self.main, DockState::new(Vec::new()));
        self.main = labelled(main);
        for w in &mut self.windows {
            let dock = std::mem::replace(&mut w.dock, DockState::new(Vec::new()));
            w.dock = labelled(dock);
        }
        self.prune();
        self.next_id = self
            .windows
            .iter()
            .map(|w| w.id + 1)
            .max()
            .unwrap_or(1)
            .max(self.next_id);
        self
    }

    /// Back to the default main window with no detached windows. Window ids keep
    /// counting up so a closing native window is never confused with a new one.
    pub fn reset(&mut self) {
        let next_id = self.next_id;
        *self = Self {
            next_id,
            ..Self::default()
        };
    }

    /// Main window first, then detached windows in creation order.
    pub fn window_ids(&self) -> Vec<WindowId> {
        std::iter::once(WindowId::Main)
            .chain(self.windows.iter().map(|w| WindowId::Detached(w.id)))
            .collect()
    }

    pub fn dock(&self, win: WindowId) -> Option<&DockState<Tab>> {
        match win {
            WindowId::Main => Some(&self.main),
            WindowId::Detached(id) => self.windows.iter().find(|w| w.id == id).map(|w| &w.dock),
        }
    }

    pub fn dock_mut(&mut self, win: WindowId) -> Option<&mut DockState<Tab>> {
        match win {
            WindowId::Main => Some(&mut self.main),
            WindowId::Detached(id) => self
                .windows
                .iter_mut()
                .find(|w| w.id == id)
                .map(|w| &mut w.dock),
        }
    }

    pub fn detached_mut(&mut self, id: u64) -> Option<&mut Detached> {
        self.windows.iter_mut().find(|w| w.id == id)
    }

    /// The window `tab` is open in.
    pub fn find(&self, tab: Tab) -> Option<WindowId> {
        self.window_ids()
            .into_iter()
            .find(|&win| self.dock(win).is_some_and(|d| d.find_tab(&tab).is_some()))
    }

    /// Show `tab`: focus it wherever it is open, otherwise add it to the main window
    /// next to tabs of the same area. Returns the window that now shows it.
    pub fn open_tab(&mut self, tab: Tab) -> WindowId {
        if let Some(win) = self.find(tab) {
            if let Some(dock) = self.dock_mut(win) {
                focus_tab(dock, tab);
            }
            return win;
        }
        place_in_main(&mut self.main, tab);
        WindowId::Main
    }

    pub fn close_tab(&mut self, tab: Tab) {
        self.take(tab);
        self.prune();
    }

    /// Move `tab` into a new native window and return its id.
    pub fn detach(&mut self, tab: Tab) -> Option<u64> {
        // Already alone in a detached window: nothing to do.
        if let Some(WindowId::Detached(id)) = self.find(tab)
            && self.dock(WindowId::Detached(id))?.iter_all_tabs().count() == 1
        {
            return Some(id);
        }
        self.take(tab)?;
        let id = self.next_id;
        self.next_id += 1;
        self.windows.push(Detached {
            id,
            dock: labelled(DockState::new(vec![tab])),
            geometry: None,
        });
        self.prune();
        Some(id)
    }

    /// Move `tab` into an existing window.
    pub fn move_tab(&mut self, tab: Tab, to: WindowId) {
        if self.find(tab) == Some(to) || self.dock(to).is_none() || self.take(tab).is_none() {
            return;
        }
        match to {
            WindowId::Main => place_in_main(&mut self.main, tab),
            WindowId::Detached(_) => {
                if let Some(dock) = self.dock_mut(to) {
                    dock.push_to_focused_leaf(tab);
                }
            }
        }
        self.prune();
    }

    /// Close a detached window, returning its tabs to the main window.
    pub fn close_window(&mut self, id: u64) {
        let Some(i) = self.windows.iter().position(|w| w.id == id) else {
            return;
        };
        let window = self.windows.remove(i);
        for (_, tab) in window.dock.iter_all_tabs() {
            place_in_main(&mut self.main, *tab);
        }
    }

    /// Drop detached windows whose last tab was closed or moved away.
    pub fn prune(&mut self) {
        self.windows
            .retain(|w| w.dock.iter_all_tabs().next().is_some());
    }

    fn take(&mut self, tab: Tab) -> Option<Tab> {
        let dock = self.dock_mut(self.find(tab)?)?;
        let path = dock.find_tab(&tab)?;
        dock.remove_tab(path)
    }
}

fn focus_tab(state: &mut DockState<Tab>, tab: Tab) {
    if let Some(path) = state.find_tab(&tab) {
        // The path was just found, so this cannot fail.
        let _ = state.set_active_tab(path);
        state.set_focused_node_and_surface(path.node_path());
    }
}

/// Add `tab` next to main-window tabs of the same area (or as a floating window if
/// that area was emptied).
fn place_in_main(state: &mut DockState<Tab>, tab: Tab) {
    let sibling = state
        .iter_all_tabs()
        .find(|(_, t)| t.area() == tab.area())
        .map(|(path, _)| path.node_path());
    match sibling {
        Some(node) => {
            state.set_focused_node_and_surface(node);
            state.push_to_focused_leaf(tab);
        }
        None => {
            state.add_window(vec![tab]);
        }
    }
}

/// Bridges egui_dock to our [`Views`].
pub struct Viewer<'a> {
    pub views: &'a mut Views,
    pub ctx: &'a mut AppCtx,
    /// The window this dock area is drawn in.
    pub here: WindowId,
    /// Every window with its display name, for the "Move to" menu.
    pub windows: &'a [(WindowId, String)],
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

    fn context_menu(&mut self, ui: &mut Ui, tab: &mut Tab, _path: NodePath) {
        if ui.button("Open in New Window").clicked() {
            self.ctx.request(Action::DetachTab(*tab));
        }
        let others: Vec<_> = self
            .windows
            .iter()
            .filter(|(w, _)| *w != self.here)
            .collect();
        if !others.is_empty() {
            ui.menu_button("Move to", |ui| {
                for (win, name) in others {
                    if ui.button(name).clicked() {
                        self.ctx.request(Action::MoveTab(*tab, *win));
                    }
                }
            });
        }
        ui.separator();
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

    /// Every tab is open at most once across all windows.
    fn assert_unique(layout: &Layout) {
        for tab in Tab::ALL {
            let n: usize = layout
                .window_ids()
                .into_iter()
                .map(|w| {
                    layout
                        .dock(w)
                        .unwrap()
                        .iter_all_tabs()
                        .filter(|(_, t)| **t == tab)
                        .count()
                })
                .sum();
            assert!(n <= 1, "{tab:?} open {n} times");
        }
    }

    #[test]
    fn layout_roundtrips_through_ron() {
        // eframe persists app state as RON.
        let mut layout = Layout::default();
        layout.detach(Tab::Console).unwrap();
        let text = ron::to_string(&layout).unwrap();
        let back = ron::from_str::<Layout>(&text).unwrap().restored();
        assert_eq!(back.windows.len(), 1);
        assert_eq!(back.find(Tab::Console), Some(WindowId::Detached(1)));
        assert_eq!(
            back.main.iter_all_tabs().count(),
            layout.main.iter_all_tabs().count()
        );
        assert_eq!(
            back.main.translations.tab_context_menu.eject_button,
            "Float in This Window"
        );
    }

    #[test]
    fn reopening_goes_to_its_area() {
        let mut layout = Layout::default();
        layout.close_tab(Tab::Jobs);
        assert_eq!(layout.find(Tab::Jobs), None);
        assert_eq!(layout.open_tab(Tab::Jobs), WindowId::Main);
        let jobs = layout.main.find_tab(&Tab::Jobs).unwrap();
        let console = layout.main.find_tab(&Tab::Console).unwrap();
        assert_eq!(jobs.node_path(), console.node_path());
    }

    #[test]
    fn documents_open_next_to_start() {
        let mut layout = Layout::default();
        layout.open_tab(Tab::LibraryManager);
        let a = layout.main.find_tab(&Tab::LibraryManager).unwrap();
        let b = layout.main.find_tab(&Tab::Start).unwrap();
        assert_eq!(a.node_path(), b.node_path());
    }

    #[test]
    fn detach_moves_tab_to_new_window() {
        let mut layout = Layout::default();
        let id = layout.detach(Tab::Start).unwrap();
        assert_eq!(layout.find(Tab::Start), Some(WindowId::Detached(id)));
        assert!(layout.main.find_tab(&Tab::Start).is_none());
        // Detaching the only tab of a detached window keeps that window.
        assert_eq!(layout.detach(Tab::Start), Some(id));
        assert_eq!(layout.windows.len(), 1);
        // Opening a detached tab focuses it there instead of duplicating it.
        assert_eq!(layout.open_tab(Tab::Start), WindowId::Detached(id));
        assert_unique(&layout);
    }

    #[test]
    fn move_between_windows_and_prune() {
        let mut layout = Layout::default();
        let a = layout.detach(Tab::Console).unwrap();
        let b = layout.detach(Tab::Tools).unwrap();
        layout.move_tab(Tab::Tools, WindowId::Detached(a));
        // b lost its last tab and is gone.
        assert_eq!(
            layout.window_ids(),
            vec![WindowId::Main, WindowId::Detached(a)]
        );
        assert_eq!(layout.find(Tab::Tools), Some(WindowId::Detached(a)));
        layout.move_tab(Tab::Console, WindowId::Main);
        assert_eq!(layout.find(Tab::Console), Some(WindowId::Main));
        // Ids are not reused.
        assert!(layout.detach(Tab::Jobs).unwrap() > b);
        assert_unique(&layout);
    }

    #[test]
    fn closing_window_returns_tabs_to_main() {
        let mut layout = Layout::default();
        let id = layout.detach(Tab::Console).unwrap();
        layout.move_tab(Tab::Inspector, WindowId::Detached(id));
        layout.close_window(id);
        assert!(layout.windows.is_empty());
        assert_eq!(layout.find(Tab::Console), Some(WindowId::Main));
        assert_eq!(layout.find(Tab::Inspector), Some(WindowId::Main));
        // Back in their default areas.
        let console = layout.main.find_tab(&Tab::Console).unwrap();
        let jobs = layout.main.find_tab(&Tab::Jobs).unwrap();
        assert_eq!(console.node_path(), jobs.node_path());
        assert_unique(&layout);
    }
}
