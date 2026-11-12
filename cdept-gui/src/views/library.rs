//! Library Manager in two forms: a compact tree for the side panel and the full
//! 3-column Library | Cell | View browser as a document. Filled in during Phase 1.

use egui::{Button, Ui};

use super::{View, empty_state};
use crate::dock::Tab;
use crate::state::{Action, AppCtx};

const PHASE: &str = "Libraries arrive in Phase 1";

/// Side panel: one tree, lib ▸ cell ▸ view.
#[derive(Default)]
pub struct LibrariesView {
    filter: String,
}

impl View for LibrariesView {
    fn title(&self, _ctx: &AppCtx) -> String {
        "Libraries".into()
    }

    fn ui(&mut self, ui: &mut Ui, ctx: &mut AppCtx) {
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.filter)
                    .hint_text("🔍 Filter")
                    .desired_width(f32::INFINITY),
            );
        });
        ui.separator();
        if ctx.project.is_none() {
            empty_state(ui, "No project open.", None);
        } else {
            empty_state(ui, "No libraries.", Some(PHASE));
        }
        ui.add_space(8.0);
        ui.vertical_centered(|ui| {
            if ui.button("Open Library Manager").clicked() {
                ctx.request(Action::OpenTab(Tab::LibraryManager));
            }
        });
    }
}

/// Document: the classic three-column browser.
#[derive(Default)]
pub struct LibraryManagerView {
    filter: String,
}

impl View for LibraryManagerView {
    fn title(&self, _ctx: &AppCtx) -> String {
        "Library Manager".into()
    }

    fn ui(&mut self, ui: &mut Ui, _ctx: &mut AppCtx) {
        ui.horizontal(|ui| {
            ui.label("Filter:");
            ui.text_edit_singleline(&mut self.filter);
        });
        ui.separator();
        ui.columns(3, |cols| {
            for (col, (header, empty)) in cols.iter_mut().zip([
                ("Library", "No libraries"),
                ("Cell", "Select a library"),
                ("View", "Select a cell"),
            ]) {
                col.strong(header);
                egui::Frame::group(col.style()).show(col, |ui| {
                    ui.set_min_size(ui.available_size());
                    egui::ScrollArea::vertical().id_salt(header).show(ui, |ui| {
                        empty_state(ui, empty, None);
                    });
                });
            }
        });
    }

    fn menus(&mut self, ui: &mut Ui, _ctx: &mut AppCtx) {
        ui.menu_button("Library", |ui| {
            for item in ["New Library…", "New Cell…", "New View…"] {
                ui.add_enabled(false, Button::new(item))
                    .on_disabled_hover_text(PHASE);
            }
            ui.separator();
            for item in ["Copy…", "Rename…", "Delete"] {
                ui.add_enabled(false, Button::new(item))
                    .on_disabled_hover_text(PHASE);
            }
            ui.separator();
            ui.add_enabled(false, Button::new("Categories…"))
                .on_disabled_hover_text(PHASE);
        });
    }

    fn toolbar(&mut self, ui: &mut Ui, _ctx: &mut AppCtx) {
        for item in ["+ Library", "+ Cell", "+ View"] {
            ui.add_enabled(false, Button::new(item))
                .on_disabled_hover_text(PHASE);
        }
    }
}
