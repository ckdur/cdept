//! Inspector panel: details of the library, cell or view selected in the library views.

use egui::{Grid, RichText, Ui};

use super::{View, empty_state};
use crate::state::AppCtx;

#[derive(Default)]
pub struct InspectorView;

fn row(ui: &mut Ui, label: &str, value: impl Into<String>) {
    ui.weak(label);
    ui.add(egui::Label::new(value.into()).wrap());
    ui.end_row();
}

impl View for InspectorView {
    fn title(&self, _ctx: &AppCtx) -> String {
        "Inspector".into()
    }

    fn ui(&mut self, ui: &mut Ui, ctx: &mut AppCtx) {
        let Some(project) = &ctx.project else {
            return empty_state(ui, "No project open.", None);
        };
        let Some(lib) = ctx.selected_library() else {
            ui.strong(format!("Project {}", project.name));
            Grid::new("inspect-project").num_columns(2).show(ui, |ui| {
                row(ui, "Folder", project.root.display().to_string());
                if let Some(description) = &project.description {
                    row(ui, "Description", description);
                }
                row(
                    ui,
                    "Technology",
                    project.technology.as_deref().unwrap_or("not set"),
                );
                row(ui, "Libraries", project.libraries.len().to_string());
            });
            for warning in &project.warnings {
                ui.colored_label(ui.visuals().warn_fg_color, format!("⚠ {warning}"));
            }
            return;
        };

        ui.strong(format!("Library {}", lib.name));
        Grid::new("inspect-library").num_columns(2).show(ui, |ui| {
            row(ui, "Folder", lib.path.display().to_string());
            if let Some(description) = &lib.description {
                row(ui, "Description", description);
            }
            row(ui, "Layout", lib.layout.name());
            row(
                ui,
                "Access",
                if lib.read_only {
                    "read-only"
                } else {
                    "read/write"
                },
            );
            row(ui, "Cells", lib.cells.len().to_string());
            if lib.defined_in != project.file {
                row(ui, "Defined in", lib.defined_in.display().to_string());
            }
        });
        if let Some(error) = &lib.error {
            ui.colored_label(ui.visuals().warn_fg_color, format!("⚠ {error}"));
        }

        let Some(cell) = ctx.selected_cell() else {
            return;
        };
        ui.separator();
        ui.strong(format!("Cell {}", cell.name));
        Grid::new("inspect-cell").num_columns(2).show(ui, |ui| {
            if let Some(path) = &cell.path {
                row(ui, "Folder", path.display().to_string());
            }
            let views: Vec<&str> = cell.entries.iter().map(|e| e.name.as_str()).collect();
            row(ui, "Views", views.join(", "));
        });

        let Some(entry) = ctx.selected_view() else {
            return;
        };
        ui.separator();
        ui.strong(format!("View {}", entry.name));
        Grid::new("inspect-view").num_columns(2).show(ui, |ui| {
            row(ui, "Kind", entry.kind.name());
            if let Some(format) = entry.format {
                row(ui, "Format", format.label());
                let tool = match format.tool() {
                    Some(name) => match ctx.tools.find(name) {
                        Some(t) if !t.found() => format!("{name} (not found)"),
                        _ => name.to_string(),
                    },
                    None => "text editor".into(),
                };
                row(ui, "Opens with", tool);
            }
            if let Some(file) = &entry.main_file {
                row(ui, "File", file.display().to_string());
            }
        });
        if entry.files.len() > 1 {
            ui.add_space(4.0);
            ui.weak("Files");
            for file in &entry.files {
                if let Some(name) = file.file_name() {
                    ui.label(RichText::new(name.to_string_lossy()).monospace());
                }
            }
        }
    }
}
