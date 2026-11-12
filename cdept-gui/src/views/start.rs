use egui::{RichText, Ui};

use super::View;
use crate::dock::Tab;
use crate::state::{Action, AppCtx};

#[derive(Default)]
pub struct StartView;

impl View for StartView {
    fn title(&self, _ctx: &AppCtx) -> String {
        "Start".into()
    }

    fn ui(&mut self, ui: &mut Ui, ctx: &mut AppCtx) {
        ui.add_space(16.0);
        ui.heading(RichText::new("CDePT").size(28.0).strong());
        ui.label("Chip Design Project Tool");
        ui.weak(format!("Version {}", libcdept::version::long_version()));
        ui.add_space(16.0);

        ui.horizontal(|ui| {
            ui.add_enabled(false, egui::Button::new("New Project…"))
                .on_disabled_hover_text("Project creation arrives in Phase 1");
            ui.add_enabled(false, egui::Button::new("Open Project…"))
                .on_disabled_hover_text("Project loading arrives in Phase 1");
            if ui.button("Library Manager").clicked() {
                ctx.request(Action::OpenTab(Tab::LibraryManager));
            }
        });
        ui.add_space(16.0);

        ui.strong("Recent projects");
        if ctx.settings.recent_projects.is_empty() {
            ui.weak("No recent projects. Start one with `cdept --project <dir>`.");
        } else {
            for path in &ctx.settings.recent_projects {
                ui.label(path.display().to_string());
            }
        }
        ui.add_space(16.0);

        ui.strong("External tools");
        match ctx.tools.tools() {
            None => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.weak("Detecting…");
                });
            }
            Some(tools) => {
                let found = tools.iter().filter(|t| t.found()).count();
                ui.horizontal(|ui| {
                    ui.label(format!("{found} of {} tools found on PATH.", tools.len()));
                    if ui.link("Show details").clicked() {
                        ctx.request(Action::OpenTab(Tab::Tools));
                    }
                });
            }
        }
    }
}
