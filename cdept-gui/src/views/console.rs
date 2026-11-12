//! Console panel (Virtuoso: CIW). Shows the log and, later, a command line.

use egui::{Color32, RichText, ScrollArea, TextStyle, Ui};
use tracing::Level;

use super::View;
use crate::state::{AppCtx, ConsoleLevel};

#[derive(Default)]
pub struct ConsoleView {
    filter: String,
    command: String,
}

impl View for ConsoleView {
    fn title(&self, _ctx: &AppCtx) -> String {
        "Console".into()
    }

    fn ui(&mut self, ui: &mut Ui, ctx: &mut AppCtx) {
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("console-level")
                .selected_text(ctx.settings.console_level.label())
                .width(70.0)
                .show_ui(ui, |ui| {
                    for level in ConsoleLevel::ALL {
                        ui.selectable_value(&mut ctx.settings.console_level, level, level.label());
                    }
                });
            ui.add(
                egui::TextEdit::singleline(&mut self.filter)
                    .hint_text("Filter")
                    .desired_width(160.0),
            );
            if ui.button("Clear").clicked() {
                ctx.log.clear();
            }
        });

        egui::TopBottomPanel::bottom("console-command")
            .frame(egui::Frame::NONE)
            .show_inside(ui, |ui| {
                ui.add_space(2.0);
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.command)
                        .font(TextStyle::Monospace)
                        .hint_text("> command line (not available yet)")
                        .desired_width(f32::INFINITY),
                );
                if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    let cmd = std::mem::take(&mut self.command);
                    if !cmd.trim().is_empty() {
                        tracing::info!(target: "cdept::console", "> {cmd}");
                        tracing::warn!(target: "cdept::console", "the command line is not implemented yet");
                    }
                    response.request_focus();
                }
            });

        let level = ctx.settings.console_level;
        let filter = self.filter.to_lowercase();
        let lines = ctx.log.lines();
        let shown: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, l)| level.shows(l.level))
            .filter(|(_, l)| {
                filter.is_empty()
                    || l.message.to_lowercase().contains(&filter)
                    || l.target.to_lowercase().contains(&filter)
            })
            .map(|(i, _)| i)
            .collect();

        let row_height = ui.text_style_height(&TextStyle::Monospace);
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show_inside(ui, |ui| {
                ScrollArea::both()
                    .auto_shrink(false)
                    .stick_to_bottom(true)
                    .show_rows(ui, row_height, shown.len(), |ui, range| {
                        for &i in &shown[range] {
                            let line = &lines[i];
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 8.0;
                                ui.label(RichText::new(&line.time).monospace().weak());
                                ui.label(
                                    RichText::new(format!("{:5}", line.level))
                                        .monospace()
                                        .color(level_color(ui, line.level)),
                                );
                                ui.label(RichText::new(&line.target).monospace().weak());
                                ui.label(RichText::new(&line.message).monospace());
                            });
                        }
                    });
            });
    }
}

fn level_color(ui: &Ui, level: Level) -> Color32 {
    let v = ui.visuals();
    match level {
        Level::ERROR => v.error_fg_color,
        Level::WARN => v.warn_fg_color,
        Level::INFO => v.text_color(),
        _ => v.weak_text_color(),
    }
}
