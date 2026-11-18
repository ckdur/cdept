//! Console panel (Virtuoso: CIW). Shows the log and, later, a command line.

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, ScrollArea, TextStyle, Ui};
use tracing::Level;

use super::View;
use crate::log::LogLine;
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

        egui::Panel::bottom("console-command")
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
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
        // One display row per text line of a message. `show_rows` needs every row to
        // have the same height: a multi-line message (a simulator's output) drawn as one
        // row made the content taller than computed and the scroll position oscillate.
        let mut rows: Vec<(usize, usize)> = Vec::new();
        for (i, l) in lines.iter().enumerate() {
            if !level.shows(l.level) {
                continue;
            }
            if !filter.is_empty()
                && !l.message.to_lowercase().contains(&filter)
                && !l.target.to_lowercase().contains(&filter)
            {
                continue;
            }
            rows.extend((0..message_rows(&l.message)).map(|k| (i, k)));
        }

        let font = TextStyle::Monospace.resolve(ui.style());
        let row_height = ui.fonts_mut(|f| f.row_height(&font));
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                ScrollArea::both()
                    .auto_shrink(false)
                    .stick_to_bottom(true)
                    .show_rows(ui, row_height, rows.len(), |ui, range| {
                        for &(i, k) in &rows[range] {
                            let line = &lines[i];
                            let text = line.message.lines().nth(k).unwrap_or("");
                            ui.add(egui::Label::new(row_job(ui, &font, line, k, text)).extend());
                        }
                    });
            });
    }
}

/// Number of display rows of a message: its text lines, at least one.
fn message_rows(message: &str) -> usize {
    message.lines().count().max(1)
}

/// One row of the console as a single text layout, so its height is exactly the font's
/// row height. The first row of a message has time, level and target; the following
/// ones are indented to the message column.
fn row_job(ui: &Ui, font: &egui::FontId, line: &LogLine, k: usize, text: &str) -> LayoutJob {
    let v = ui.visuals();
    let fmt = |color: Color32| TextFormat {
        font_id: font.clone(),
        color,
        ..Default::default()
    };
    let text = text.replace('\t', "    ");
    let mut job = LayoutJob::default();
    let head = format!("{} {:5} {} ", line.time, line.level, line.target);
    if k == 0 {
        job.append(&line.time, 0.0, fmt(v.weak_text_color()));
        job.append(
            &format!(" {:5} ", line.level),
            0.0,
            fmt(level_color(ui, line.level)),
        );
        job.append(&format!("{} ", line.target), 0.0, fmt(v.weak_text_color()));
    } else {
        job.append(&" ".repeat(head.chars().count()), 0.0, fmt(v.text_color()));
    }
    job.append(&text, 0.0, fmt(v.text_color()));
    job
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

#[cfg(test)]
mod tests {
    use super::message_rows;

    #[test]
    fn a_message_takes_one_row_per_text_line() {
        assert_eq!(message_rows(""), 1);
        assert_eq!(message_rows("one"), 1);
        assert_eq!(message_rows("a\nb\nc\n"), 3);
    }
}
