use egui::{RichText, Ui};

use super::View;
use crate::dock::Tab;
use crate::state::{Action, AppCtx, PickFor};

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
            if ui.button("New Project…").clicked() {
                ctx.request(Action::PickFolder(PickFor::NewProject));
            }
            if ui.button("Open Project…").clicked() {
                ctx.request(Action::PickFolder(PickFor::OpenProject));
            }
            if ui.button("Library Manager").clicked() {
                ctx.request(Action::OpenTab(Tab::LibraryManager));
            }
        });
        ui.add_space(16.0);

        if let Some(project) = &ctx.project {
            ui.strong(format!("Project {}", project.name));
            let cells: usize = project.libraries.iter().map(|l| l.cells.len()).sum();
            ui.label(format!(
                "{} libraries, {cells} cells in {}",
                project.libraries.len(),
                project.root.display()
            ));
            ui.add_space(16.0);
        }

        ui.strong("Recent projects");
        if ctx.settings.recent_projects.is_empty() {
            ui.weak("None yet. A project is a folder with a cdept.toml file.");
        }
        let mut open = None;
        for path in &ctx.settings.recent_projects {
            if ui.link(path.display().to_string()).clicked() {
                open = Some(path.clone());
            }
        }
        if let Some(path) = open {
            ctx.request(Action::LoadProject(path));
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
