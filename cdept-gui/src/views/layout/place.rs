//! "Place Cell…": CDePT's library picker, limited to layout views.

use std::path::PathBuf;

use egui::{Context, Id, ScrollArea, Ui};
use libcdept::{Cell, Format, Project};

/// What the user has chosen so far.
#[derive(Default)]
pub struct PlacePicker {
    library: Option<String>,
    cell: Option<String>,
    file: Option<PathBuf>,
}

/// A cell the user chose to place: its name and the file of its layout view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placed {
    pub cell: String,
    pub file: PathBuf,
}

pub enum Picked {
    Nothing,
    Cancel,
    Cell(Placed),
}

/// The layout views of a cell: view name and file.
fn layouts(cell: &Cell) -> Vec<(&str, &PathBuf)> {
    cell.entries
        .iter()
        .filter(|e| matches!(e.format, Some(Format::Gds | Format::Oasis)))
        .filter_map(|e| Some((e.name.as_str(), e.main_file.as_ref()?)))
        .collect()
}

const COLUMN: egui::Vec2 = egui::Vec2::new(190.0, 240.0);

fn column(ui: &mut Ui, title: &str, content: impl FnOnce(&mut Ui)) {
    ui.allocate_ui(COLUMN + egui::Vec2::new(0.0, 24.0), |ui| {
        ui.vertical(|ui| {
            ui.strong(title);
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_min_size(COLUMN);
                ui.set_max_size(COLUMN);
                ScrollArea::vertical()
                    .id_salt(title)
                    .auto_shrink([false, false])
                    .show(ui, content);
            });
        });
    });
}

/// Show the picker and return what was decided in this frame.
pub fn show(ctx: &Context, id: Id, project: &Project, picker: &mut PlacePicker) -> Picked {
    let mut picked = Picked::Nothing;
    let mut open = true;
    egui::Window::new("Place Cell")
        .id(id)
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .show(ctx, |ui| {
            let library = picker.library.as_deref().and_then(|n| project.library(n));
            let cell = library.and_then(|l| l.cell(picker.cell.as_deref()?));
            ui.horizontal_top(|ui| {
                column(ui, "Library", |ui| {
                    for lib in project.libraries.iter().filter(|l| l.error.is_none()) {
                        let selected = library.is_some_and(|l| l.name == lib.name);
                        if ui.selectable_label(selected, &lib.name).clicked() {
                            picker.library = Some(lib.name.clone());
                            picker.cell = None;
                            picker.file = None;
                        }
                    }
                });
                column(ui, "Cell", |ui| {
                    let Some(lib) = library else {
                        ui.weak("Select a library");
                        return;
                    };
                    for c in lib.cells.iter().filter(|c| !layouts(c).is_empty()) {
                        let selected = cell.is_some_and(|s| s.name == c.name);
                        if ui.selectable_label(selected, &c.name).clicked() {
                            picker.cell = Some(c.name.clone());
                            picker.file = layouts(c).first().map(|(_, f)| (*f).clone());
                        }
                    }
                });
                column(ui, "View", |ui| {
                    let Some(cell) = cell else {
                        ui.weak("Select a cell");
                        return;
                    };
                    for (view, file) in layouts(cell) {
                        let selected = picker.file.as_ref() == Some(file);
                        let response = ui.selectable_label(selected, view);
                        if response.double_clicked() {
                            picked = Picked::Cell(Placed {
                                cell: cell.name.clone(),
                                file: file.clone(),
                            });
                        } else if response.clicked() {
                            picker.file = Some(file.clone());
                        }
                    }
                });
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let target = cell.zip(picker.file.clone());
                if ui
                    .add_enabled(target.is_some(), egui::Button::new("Place"))
                    .clicked()
                    && let Some((cell, file)) = target
                {
                    picked = Picked::Cell(Placed {
                        cell: cell.name.clone(),
                        file,
                    });
                }
                if ui.button("Cancel").clicked() {
                    picked = Picked::Cancel;
                }
            });
        });
    if !open {
        picked = Picked::Cancel;
    }
    picked
}
