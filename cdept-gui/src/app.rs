//! The main window: menu bar, toolbar, dock area and status bar.

use egui::{Button, Context, Id, KeyboardShortcut, Modifiers, Ui};
use egui_dock::{DockArea, DockState, Style};

use crate::GuiOptions;
use crate::dock::{self, Tab, Viewer};
use crate::log::LogBuffer;
use crate::state::{Action, AppCtx, Settings};
use crate::views::{EditAction, Views};

// Bump the suffix when `Tab` changes incompatibly, so stale layouts are dropped.
const DOCK_KEY: &str = "cdept.dock.v1";
const SETTINGS_KEY: &str = "cdept.settings";

const QUIT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, egui::Key::Q);
const PHASE3: &str = "Launching tools arrives in Phase 3";

/// Global toolbar launchers (tool names from `libcdept::tools::KNOWN_TOOLS`).
const LAUNCHERS: [&str; 4] = ["xschem", "KLayout", "Magic", "ngspice"];

pub struct CdeptApp {
    dock: DockState<Tab>,
    views: Views,
    ctx: AppCtx,
    /// Last focused document tab; it owns the contextual menus and toolbar.
    active_doc: Option<Tab>,
    show_about: bool,
}

impl CdeptApp {
    pub fn new(cc: &eframe::CreationContext<'_>, options: GuiOptions, log: LogBuffer) -> Self {
        log.set_repaint_context(cc.egui_ctx.clone());

        let (dock, settings) = match cc.storage {
            Some(storage) => (
                eframe::get_value(storage, DOCK_KEY).unwrap_or_else(dock::default_layout),
                eframe::get_value::<Settings>(storage, SETTINGS_KEY).unwrap_or_default(),
            ),
            None => (dock::default_layout(), Settings::default()),
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
            dock,
            views: Views::default(),
            ctx,
            active_doc: None,
            show_about: false,
        }
    }

    fn menu_bar(&mut self, ui: &mut Ui) {
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
                if ui
                    .add(Button::new("Quit").shortcut_text(ui.ctx().format_shortcut(&QUIT)))
                    .clicked()
                {
                    self.ctx.request(Action::Quit);
                }
            });

            self.edit_menu(ui);

            ui.menu_button("View", |ui| {
                ui.menu_button("Theme", |ui| {
                    egui::widgets::global_theme_preference_buttons(ui)
                });
                ui.menu_button("Zoom", egui::gui_zoom::zoom_menu_buttons);
            });

            // Contextual menus of the active document.
            if let Some(tab) = self.active_doc {
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

            ui.menu_button("Window", |ui| {
                for tab in Tab::ALL {
                    let mut open = self.dock.find_tab(&tab).is_some();
                    let title = self.views.get(tab).title(&self.ctx);
                    if ui.checkbox(&mut open, title).changed() {
                        if open {
                            dock::open_tab(&mut self.dock, tab);
                        } else {
                            dock::close_tab(&mut self.dock, tab);
                        }
                    }
                }
                ui.separator();
                ui.weak("Right-click a tab to detach it.");
                if ui.button("Reset Layout").clicked() {
                    self.ctx.request(Action::ResetLayout);
                }
            });

            ui.menu_button("Help", |ui| {
                if ui.button("About CDePT").clicked() {
                    self.ctx.request(Action::ShowAbout);
                }
            });
        });
    }

    /// Edit actions are routed to the active document; unsupported ones are greyed out.
    fn edit_menu(&mut self, ui: &mut Ui) {
        let caps = self
            .active_doc
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
        if let (Some(action), Some(tab)) = (chosen, self.active_doc) {
            self.views.get(tab).edit(action, &mut self.ctx);
        }
    }

    fn toolbar(&mut self, ui: &mut Ui) {
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
            if let Some(tab) = self.active_doc {
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
            self.show_about = false;
        }
    }

    fn apply(&mut self, ctx: &Context, action: Action) {
        match action {
            Action::OpenTab(tab) => dock::open_tab(&mut self.dock, tab),
            Action::ResetLayout => {
                self.dock = dock::default_layout();
                self.active_doc = None;
            }
            Action::DetectTools => self.ctx.tools.start_detection(ctx),
            Action::ShowAbout => self.show_about = true,
            Action::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
        }
    }
}

impl eframe::App for CdeptApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        if ctx.input_mut(|i| i.consume_shortcut(&QUIT)) {
            self.ctx.request(Action::Quit);
        }
        // Drop the active document if its tab was closed.
        if let Some(tab) = self.active_doc
            && self.dock.find_tab(&tab).is_none()
        {
            self.active_doc = None;
        }

        egui::TopBottomPanel::top("menu-bar").show(ctx, |ui| self.menu_bar(ui));
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| self.toolbar(ui));
        egui::TopBottomPanel::bottom("status-bar").show(ctx, |ui| self.status_bar(ui));

        DockArea::new(&mut self.dock)
            .style(Style::from_egui(ctx.style().as_ref()))
            .show_leaf_collapse_buttons(false)
            .show(
                ctx,
                &mut Viewer {
                    views: &mut self.views,
                    ctx: &mut self.ctx,
                },
            );

        if let Some((_, tab)) = self.dock.find_active_focused()
            && tab.is_document()
        {
            self.active_doc = Some(*tab);
        }
        if self.active_doc.is_none() {
            // Fall back to any open document so its menus are reachable.
            self.active_doc = self
                .dock
                .iter_all_tabs()
                .map(|(_, t)| *t)
                .find(|t| t.is_document());
        }

        if self.show_about {
            self.about(ctx);
        }
        for action in self.ctx.take_actions() {
            self.apply(ctx, action);
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, DOCK_KEY, &self.dock);
        eframe::set_value(storage, SETTINGS_KEY, &self.ctx.settings);
    }
}
