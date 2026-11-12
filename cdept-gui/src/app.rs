//! The application shell: the main window and any detached native windows.
//!
//! Every native window has its own menu bar, toolbar and dock area. The menus are the
//! same global ones everywhere, plus the contextual menus of the active document *of
//! that window*. Only the main window has a status bar.

use std::collections::HashMap;

use egui::{
    Button, Context, Id, KeyboardShortcut, Modifiers, Ui, ViewportBuilder, ViewportCommand,
};
use egui_dock::{DockArea, Style};

use crate::dock::{Geometry, Layout, Tab, Viewer, WindowId};
use crate::log::LogBuffer;
use crate::state::{Action, AppCtx, Settings};
use crate::views::{EditAction, Views};
use crate::{APP_ID, GuiOptions};

// Bump the suffix when `Layout` or `Tab` change incompatibly, so stale layouts are dropped.
const LAYOUT_KEY: &str = "cdept.layout.v2";
const SETTINGS_KEY: &str = "cdept.settings";

const QUIT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, egui::Key::Q);
const PHASE3: &str = "Launching tools arrives in Phase 3";

/// Global toolbar launchers (tool names from `libcdept::tools::KNOWN_TOOLS`).
const LAUNCHERS: [&str; 4] = ["xschem", "KLayout", "Magic", "ngspice"];

const DETACHED_SIZE: [f32; 2] = [1000.0, 700.0];

pub struct CdeptApp {
    layout: Layout,
    views: Views,
    ctx: AppCtx,
    /// Last focused document tab of each window; it owns that window's contextual
    /// menus and toolbar.
    active_docs: HashMap<WindowId, Tab>,
    /// Display name of every window, refreshed each frame.
    window_names: Vec<(WindowId, String)>,
    /// Geometry each detached window was spawned with. The viewport builder must stay
    /// the same across frames, or egui would keep moving the window back.
    spawn_geometry: HashMap<u64, Option<Geometry>>,
    about: Option<WindowId>,
}

impl CdeptApp {
    pub fn new(cc: &eframe::CreationContext<'_>, options: GuiOptions, log: LogBuffer) -> Self {
        log.set_repaint_context(cc.egui_ctx.clone());

        let (layout, settings) = match cc.storage {
            Some(storage) => (
                eframe::get_value::<Layout>(storage, LAYOUT_KEY)
                    .map(Layout::restored)
                    .unwrap_or_default(),
                eframe::get_value::<Settings>(storage, SETTINGS_KEY).unwrap_or_default(),
            ),
            None => (Layout::default(), Settings::default()),
        };

        let mut ctx = AppCtx::new(settings, log);
        if let Some(path) = options.project {
            let path = path.canonicalize().unwrap_or(path);
            tracing::info!(target: "cdept", "project {}: loading arrives in Phase 1", path.display());
            ctx.settings.add_recent(path.clone());
            ctx.project = Some(path);
        }
        ctx.tools.start_detection(&cc.egui_ctx);
        tracing::info!(target: "cdept", "CDePT {} started", libcdept::version::long_version());

        Self {
            layout,
            views: Views::default(),
            ctx,
            active_docs: HashMap::new(),
            window_names: Vec::new(),
            spawn_geometry: HashMap::new(),
            about: None,
        }
    }

    fn active_doc(&self, win: WindowId) -> Option<Tab> {
        self.active_docs.get(&win).copied()
    }

    /// Track the focused document of `win`, dropping it once it left the window.
    fn update_active_doc(&mut self, win: WindowId) {
        let Some(dock) = self.layout.dock_mut(win) else {
            self.active_docs.remove(&win);
            return;
        };
        if let Some((_, tab)) = dock.find_active_focused()
            && tab.is_document()
        {
            self.active_docs.insert(win, *tab);
        }
        if let Some(tab) = self.active_docs.get(&win)
            && dock.find_tab(tab).is_none()
        {
            self.active_docs.remove(&win);
        }
        if let std::collections::hash_map::Entry::Vacant(entry) = self.active_docs.entry(win)
            // Fall back to any open document so its menus are reachable.
            && let Some(tab) = dock
                .iter_all_tabs()
                .map(|(_, t)| *t)
                .find(|t| t.is_document())
        {
            entry.insert(tab);
        }
    }

    fn refresh_window_names(&mut self) {
        let mut names = Vec::new();
        for (n, win) in self.layout.window_ids().into_iter().enumerate() {
            let name = match win {
                WindowId::Main => "Main Window".to_string(),
                WindowId::Detached(_) => {
                    let tab = self.active_doc(win).or_else(|| {
                        self.layout
                            .dock(win)?
                            .iter_all_tabs()
                            .map(|(_, t)| *t)
                            .next()
                    });
                    match tab {
                        Some(tab) => {
                            format!("Window {n}: {}", self.views.get(tab).title(&self.ctx))
                        }
                        None => format!("Window {n}"),
                    }
                }
            };
            names.push((win, name));
        }
        self.window_names = names;
    }

    fn window_name(&self, win: WindowId) -> &str {
        self.window_names
            .iter()
            .find(|(w, _)| *w == win)
            .map_or("", |(_, name)| name.as_str())
    }

    /// Menu bar, toolbar and dock area of one native window.
    fn window_ui(&mut self, ui: &mut Ui, win: WindowId) {
        if ui.input_mut(|i| i.consume_shortcut(&QUIT)) {
            self.ctx.request(Action::Quit);
        }
        self.update_active_doc(win);

        egui::Panel::top(Id::new(("menu-bar", win))).show(ui, |ui| self.menu_bar(ui, win));
        egui::Panel::top(Id::new(("toolbar", win))).show(ui, |ui| self.toolbar(ui, win));
        if win == WindowId::Main {
            egui::Panel::bottom("status-bar").show(ui, |ui| self.status_bar(ui));
        }

        let style = Style::from_egui(ui.style().as_ref());
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                let Some(dock) = self.layout.dock_mut(win) else {
                    return;
                };
                DockArea::new(dock)
                    .id(Id::new(("dock-area", win)))
                    .style(style)
                    .show_leaf_collapse_buttons(false)
                    .show_inside(
                        ui,
                        &mut Viewer {
                            views: &mut self.views,
                            ctx: &mut self.ctx,
                            here: win,
                            windows: &self.window_names,
                        },
                    );
            });

        if self.about == Some(win) {
            self.about(ui.ctx());
        }
    }

    /// A detached window, as an immediate viewport (a real OS window where supported).
    fn show_detached(&mut self, ctx: &Context, id: u64) {
        let win = WindowId::Detached(id);
        let Some(window) = self.layout.detached_mut(id) else {
            return;
        };
        let geometry = *self.spawn_geometry.entry(id).or_insert(window.geometry);
        let mut builder = ViewportBuilder::default()
            .with_app_id(APP_ID)
            .with_title(format!("CDePT: {}", self.window_name(win)))
            .with_min_inner_size([400.0, 300.0]);
        builder = match geometry {
            Some(g) => builder.with_position(g.pos).with_inner_size(g.size),
            None => builder.with_inner_size(DETACHED_SIZE),
        };

        ctx.show_viewport_immediate(win.viewport(), builder, |ui, _class| {
            self.window_ui(ui, win);
            let (close, geometry) = ui.input(|i| {
                let vp = i.viewport();
                let geometry = vp
                    .outer_rect
                    .zip(vp.inner_rect)
                    .map(|(outer, inner)| Geometry {
                        pos: outer.min.into(),
                        size: inner.size().into(),
                    });
                (vp.close_requested(), geometry)
            });
            if let Some(g) = geometry
                && let Some(window) = self.layout.detached_mut(id)
            {
                window.geometry = Some(g);
            }
            if close {
                self.ctx.request(Action::CloseWindow(id));
            }
        });
    }

    fn menu_bar(&mut self, ui: &mut Ui, win: WindowId) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                ui.add_enabled(false, Button::new("New Project…"))
                    .on_disabled_hover_text("Arrives in Phase 1");
                ui.add_enabled(false, Button::new("Open Project…"))
                    .on_disabled_hover_text("Arrives in Phase 1");
                ui.menu_button("Recent Projects", |ui| {
                    if self.ctx.settings.recent_projects.is_empty() {
                        ui.weak("None");
                    }
                    for path in &self.ctx.settings.recent_projects {
                        ui.add_enabled(false, Button::new(path.display().to_string()));
                    }
                });
                ui.separator();
                if let WindowId::Detached(id) = win
                    && ui.button("Close Window").clicked()
                {
                    self.ctx.request(Action::CloseWindow(id));
                }
                if ui
                    .add(Button::new("Quit").shortcut_text(ui.ctx().format_shortcut(&QUIT)))
                    .clicked()
                {
                    self.ctx.request(Action::Quit);
                }
            });

            self.edit_menu(ui, win);

            ui.menu_button("View", |ui| {
                ui.menu_button("Theme", |ui| {
                    egui::widgets::global_theme_preference_buttons(ui)
                });
                ui.menu_button("Zoom", egui::gui_zoom::zoom_menu_buttons);
            });

            // Contextual menus of this window's active document.
            if let Some(tab) = self.active_doc(win) {
                self.views.get(tab).menus(ui, &mut self.ctx);
            }

            ui.menu_button("Project", |ui| {
                for item in ["Settings…", "Technology…", "Library Definitions…"] {
                    ui.add_enabled(false, Button::new(item))
                        .on_disabled_hover_text("Arrives in Phase 1–2");
                }
            });

            ui.menu_button("Tools", |ui| {
                if ui.button("Library Manager").clicked() {
                    self.ctx.request(Action::OpenTab(Tab::LibraryManager));
                }
                ui.separator();
                for name in LAUNCHERS {
                    ui.add_enabled(false, Button::new(format!("Launch {name}")))
                        .on_disabled_hover_text(PHASE3);
                }
                ui.separator();
                if ui.button("Detect Tools").clicked() {
                    self.ctx.request(Action::DetectTools);
                }
            });

            ui.menu_button("Window", |ui| self.window_menu(ui, win));

            ui.menu_button("Help", |ui| {
                if ui.button("About CDePT").clicked() {
                    self.ctx.request(Action::ShowAbout(win));
                }
            });
        });
    }

    fn window_menu(&mut self, ui: &mut Ui, win: WindowId) {
        for tab in Tab::ALL {
            let mut open = self.layout.find(tab).is_some();
            let title = self.views.get(tab).title(&self.ctx);
            if ui.checkbox(&mut open, title).changed() {
                if open {
                    self.ctx.request(Action::OpenTab(tab));
                } else {
                    self.layout.close_tab(tab);
                }
            }
        }
        ui.separator();
        let active = self.active_doc(win);
        if ui
            .add_enabled(active.is_some(), Button::new("Open Document in New Window"))
            .on_hover_text("Right-click any tab to move it to another window.")
            .clicked()
            && let Some(tab) = active
        {
            self.ctx.request(Action::DetachTab(tab));
        }
        ui.separator();
        for (other, name) in &self.window_names {
            if ui
                .add_enabled(*other != win, Button::new(name).selected(*other == win))
                .clicked()
            {
                self.ctx.request(Action::FocusWindow(*other));
            }
        }
        ui.separator();
        if let WindowId::Detached(id) = win
            && ui
                .button("Close Window")
                .on_hover_text("Its tabs return to the main window.")
                .clicked()
        {
            self.ctx.request(Action::CloseWindow(id));
        }
        if ui
            .button("Reset Layout")
            .on_hover_text("Close all detached windows and restore the default layout.")
            .clicked()
        {
            self.ctx.request(Action::ResetLayout);
        }
    }

    /// Edit actions are routed to the active document; unsupported ones are greyed out.
    fn edit_menu(&mut self, ui: &mut Ui, win: WindowId) {
        let active = self.active_doc(win);
        let caps = active
            .map(|t| self.views.get(t).edit_caps())
            .unwrap_or_default();
        let mut chosen = None;
        ui.menu_button("Edit", |ui| {
            let mut item = |ui: &mut Ui, enabled: bool, label: &str, shortcut: &str, action| {
                if ui
                    .add_enabled(enabled, Button::new(label).shortcut_text(shortcut))
                    .clicked()
                {
                    chosen = Some(action);
                }
            };
            item(ui, caps.undo, "Undo", "Ctrl+Z", EditAction::Undo);
            item(ui, caps.redo, "Redo", "Ctrl+Y", EditAction::Redo);
            ui.separator();
            item(ui, caps.cut, "Cut", "Ctrl+X", EditAction::Cut);
            item(ui, caps.copy, "Copy", "Ctrl+C", EditAction::Copy);
            item(ui, caps.paste, "Paste", "Ctrl+V", EditAction::Paste);
            ui.separator();
            item(
                ui,
                caps.select_all,
                "Select All",
                "Ctrl+A",
                EditAction::SelectAll,
            );
        });
        if let (Some(action), Some(tab)) = (chosen, active) {
            self.views.get(tab).edit(action, &mut self.ctx);
        }
    }

    fn toolbar(&mut self, ui: &mut Ui, win: WindowId) {
        ui.horizontal(|ui| {
            let project = self
                .ctx
                .project
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "No project".into());
            ui.label(format!("📁 {project}"));
            ui.separator();
            match self.ctx.pdk() {
                Some(pdk) => ui.label(format!("PDK: {pdk}")),
                None => ui
                    .weak("PDK: none")
                    .on_hover_text("Set $PDK, or choose a technology in Phase 2"),
            };
            ui.separator();
            for name in LAUNCHERS {
                let hover = match self.ctx.tools.find(name) {
                    Some(t) if t.found() => PHASE3.to_string(),
                    Some(_) => format!("{name} not found on PATH"),
                    None => "Detecting…".to_string(),
                };
                ui.add_enabled(false, Button::new(format!("▶ {name}")))
                    .on_disabled_hover_text(hover);
            }
            if let Some(tab) = self.active_doc(win) {
                ui.separator();
                self.views.get(tab).toolbar(ui, &mut self.ctx);
            }
        });
    }

    fn status_bar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            match &self.ctx.project {
                Some(p) => ui.label(p.display().to_string()),
                None => ui.weak("No project"),
            };
            ui.separator();
            ui.label(format!(
                "PDK: {}",
                self.ctx.pdk().unwrap_or_else(|| "none".into())
            ));
            ui.separator();
            ui.label("Jobs: 0");
            ui.separator();
            match self.ctx.tools.tools() {
                Some(tools) => {
                    let found = tools.iter().filter(|t| t.found()).count();
                    ui.label(format!("Tools: {found}/{}", tools.len()))
                }
                None => ui.weak("Tools: …"),
            };
            if !self.layout.windows.is_empty() {
                ui.separator();
                ui.label(format!("Windows: {}", self.layout.windows.len() + 1));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.weak(format!("CDePT {}", libcdept::version::long_version()));
            });
        });
    }

    fn about(&mut self, ctx: &Context) {
        let response = egui::Modal::new(Id::new("about")).show(ctx, |ui| {
            ui.set_width(360.0);
            ui.heading("CDePT");
            ui.label("Chip Design Project Tool");
            ui.add_space(4.0);
            ui.label(format!("Version {}", libcdept::version::long_version()));
            ui.label("An open-source orchestrator for xschem, KLayout, Magic and ngspice.");
            ui.weak("MIT License");
            ui.add_space(8.0);
            if ui.button("Close").clicked() {
                ui.close();
            }
        });
        if response.should_close() {
            self.about = None;
        }
    }

    fn focus(ctx: &Context, win: WindowId) {
        ctx.send_viewport_cmd_to(win.viewport(), ViewportCommand::Focus);
    }

    fn apply(&mut self, ctx: &Context, action: Action) {
        match action {
            Action::OpenTab(tab) => {
                let win = self.layout.open_tab(tab);
                Self::focus(ctx, win);
            }
            Action::DetachTab(tab) => {
                if let Some(id) = self.layout.detach(tab) {
                    Self::focus(ctx, WindowId::Detached(id));
                }
            }
            Action::MoveTab(tab, to) => {
                self.layout.move_tab(tab, to);
                Self::focus(ctx, to);
            }
            Action::FocusWindow(win) => Self::focus(ctx, win),
            Action::CloseWindow(id) => {
                self.layout.close_window(id);
                self.forget_window(id);
            }
            Action::ResetLayout => {
                self.layout.reset();
                self.active_docs.clear();
                self.spawn_geometry.clear();
                self.about = self.about.map(|_| WindowId::Main);
            }
            Action::DetectTools => self.ctx.tools.start_detection(ctx),
            Action::ShowAbout(win) => self.about = Some(win),
            Action::Quit => {
                ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, ViewportCommand::Close)
            }
        }
    }

    fn forget_window(&mut self, id: u64) {
        let win = WindowId::Detached(id);
        self.active_docs.remove(&win);
        self.spawn_geometry.remove(&id);
        if self.about == Some(win) {
            self.about = Some(WindowId::Main);
        }
    }
}

impl eframe::App for CdeptApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = &ui.ctx().clone();
        self.refresh_window_names();

        self.window_ui(ui, WindowId::Main);
        let detached: Vec<u64> = self.layout.windows.iter().map(|w| w.id).collect();
        for id in detached {
            self.show_detached(ctx, id);
        }

        for action in self.ctx.take_actions() {
            self.apply(ctx, action);
        }
        // A detached window whose last tab was closed or dragged away disappears.
        let before: Vec<u64> = self.layout.windows.iter().map(|w| w.id).collect();
        self.layout.prune();
        for id in before {
            if self.layout.detached_mut(id).is_none() {
                self.forget_window(id);
            }
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, LAYOUT_KEY, &self.layout);
        eframe::set_value(storage, SETTINGS_KEY, &self.ctx.settings);
    }
}
