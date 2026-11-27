//! What the guide draws: the card with the step list, and the outline and arrow on the
//! widget to use.

use egui::{
    Align, Align2, Color32, Context, CornerRadius, FontId, Id, LayerId, Order, Pos2, Rect,
    RichText, ScrollArea, Shape, Stroke, StrokeKind, TextureHandle, Ui, Vec2,
};

use super::circuit::Line;
use super::{Guide, Start};

const WIDTH: f32 = 380.0;
const ACCENT: Color32 = Color32::from_rgb(0xff, 0x9f, 0x1c);

/// The anchor the guide points at in this frame: the first of the step's list that is
/// on screen, with what to say there.
pub(crate) fn pointing(ctx: &Context, guide: &Guide) -> Option<(String, String)> {
    let step = guide.step()?;
    step.point
        .iter()
        // A mark named like a fact that Continue waits for is a place to get to:
        // once there, there is nothing to point at.
        .filter(|point| !(step.continue_needs.contains(&point.at) && guide.holds(&point.at)))
        .filter(|point| !point.unless.as_ref().is_some_and(|fact| guide.holds(fact)))
        .find(|point| super::find_anchor(ctx, &point.at).is_some())
        .map(|point| (point.at.clone(), point.say.clone()))
}

/// Outline the widget to use and put a labelled arrow next to it, in the window being
/// built. It is only paint: the widget keeps its clicks.
pub(crate) fn highlight(ctx: &Context, guide: &Guide) {
    if guide.closed {
        return;
    }
    let Some((name, say)) = pointing(ctx, guide) else {
        return;
    };
    let Some((viewport, rect)) = super::find_anchor(ctx, &name) else {
        return;
    };
    if viewport != ctx.viewport_id() {
        return;
    }
    let painter = ctx.layer_painter(LayerId::new(Order::Tooltip, Id::new("cdept-guide-point")));
    let time = ctx.input(|i| i.time);
    let pulse = (time * 5.0).sin() as f32 * 0.5 + 0.5;
    ctx.request_repaint_after(std::time::Duration::from_millis(50));

    let frame = rect.expand(3.0 + 3.0 * pulse);
    painter.rect_stroke(
        frame,
        CornerRadius::same(5),
        Stroke::new(2.5, ACCENT),
        StrokeKind::Outside,
    );

    let screen = ctx.content_rect();
    let galley = painter.layout(say, FontId::proportional(14.0), Color32::BLACK, 230.0);
    let size = galley.size() + Vec2::new(16.0, 10.0);
    let (label, from, to) = place(&name, frame, size, screen, 26.0 + 6.0 * pulse);
    painter.rect_filled(label, CornerRadius::same(5), ACCENT);
    painter.galley(label.min + Vec2::new(8.0, 5.0), galley, Color32::BLACK);
    let way = (to - from).normalized();
    let side = way.rot90() * 7.0;
    painter.line_segment([from, to - way * 8.0], Stroke::new(3.5, ACCENT));
    painter.add(Shape::convex_polygon(
        vec![to, to - way * 12.0 + side, to - way * 12.0 - side],
        ACCENT,
        Stroke::NONE,
    ));
}

/// Where the label goes and the arrow from it to the outline. It should hide as little
/// as possible of what the user is reading: beside a menu entry (the other entries are
/// above and below it), left of a dialog's widgets (outside the dialog, when there is
/// room), else below the widget, or above it.
fn place(name: &str, frame: Rect, size: Vec2, screen: Rect, gap: f32) -> (Rect, Pos2, Pos2) {
    // The layout picture is what the user works in: the label stays in its corner,
    // not over the layer list beside it.
    if name == "layout.picture" {
        let label = Rect::from_min_size(frame.min + Vec2::new(12.0, 12.0), size);
        let from = Pos2::new(label.center().x, label.bottom());
        return (label, from, from + Vec2::new(0.0, gap));
    }
    let menu_entry = name.starts_with("menu.") && name.matches('.').count() > 1;
    let tall = frame.height() > screen.height() * 0.4;
    let right = frame.right() + gap + size.x < screen.right();
    let left = frame.left() - gap - size.x > screen.left();
    let at_left = (name.starts_with("wizard.") || name.starts_with("dialog.")) && left;
    if (menu_entry || tall) && right || at_left {
        let y = if tall {
            frame.top() + 40.0
        } else {
            frame.center().y
        };
        let (x, from, to) = if at_left {
            let x = frame.left() - gap - size.x;
            (x, x + size.x, frame.left() - 2.0)
        } else {
            let x = frame.right() + gap;
            (x, x, frame.right() + 2.0)
        };
        let label = Rect::from_min_size(Pos2::new(x, y - size.y / 2.0), size);
        return (label, Pos2::new(from, y), Pos2::new(to, y));
    }
    let x = (frame.center().x - size.x / 2.0)
        .min(screen.right() - size.x - 4.0)
        .max(screen.left() + 4.0);
    let below = frame.bottom() + gap + size.y < screen.bottom();
    let top = if below {
        frame.bottom() + gap
    } else {
        frame.top() - gap - size.y
    };
    let label = Rect::from_min_size(Pos2::new(x, top), size);
    let tip = frame
        .center()
        .x
        .clamp(label.left() + 12.0, label.right() - 12.0);
    let (from, to) = if below {
        (label.top(), frame.bottom() + 2.0)
    } else {
        (label.bottom(), frame.top() - 2.0)
    };
    (label, Pos2::new(tip, from), Pos2::new(tip, to))
}

/// A line about this machine.
fn status_line(ui: &mut Ui, what: &str) {
    let (label, found) = match what {
        "pdk_root" => (
            "PDK_ROOT",
            std::env::var("PDK_ROOT").ok().filter(|v| !v.is_empty()),
        ),
        "klayout" => (
            "KLayout",
            libcdept::klayout::setup::find_klayout().map(|p| p.display().to_string()),
        ),
        _ => (
            "xschem helper",
            libcdept::xschem::Engine::find_helper().map(|p| p.display().to_string()),
        ),
    };
    ui.horizontal_wrapped(|ui| match found {
        Some(value) => {
            ui.label(RichText::new("✔").color(Color32::from_rgb(0x4c, 0xaf, 0x50)));
            ui.label(format!("{label}:"));
            ui.add(egui::Label::new(RichText::new(value).monospace().small()).wrap());
        }
        None => {
            ui.label(RichText::new("✖").color(ui.visuals().warn_fg_color));
            ui.label(format!("{label}: not set / not found"));
        }
    });
}

/// The text of a step: paragraphs, and indented lines as commands that can be copied.
fn body(ui: &mut Ui, text: &str) {
    let mut paragraph = String::new();
    let flush = |ui: &mut Ui, paragraph: &mut String| {
        if !paragraph.is_empty() {
            ui.add(egui::Label::new(paragraph.as_str()).wrap());
            ui.add_space(4.0);
            paragraph.clear();
        }
    };
    for line in text.lines() {
        if let Some(command) = line.strip_prefix("    ") {
            flush(ui, &mut paragraph);
            ui.horizontal_wrapped(|ui| {
                if ui.small_button("📋").on_hover_text("Copy").clicked() {
                    ui.ctx().copy_text(command.to_string());
                }
                ui.add(egui::Label::new(RichText::new(command).code()).wrap());
            });
        } else if line.trim().is_empty() {
            flush(ui, &mut paragraph);
        } else if line.starts_with("- ") {
            flush(ui, &mut paragraph);
            paragraph.push_str(line);
        } else {
            if !paragraph.is_empty() {
                paragraph.push(' ');
            }
            paragraph.push_str(line.trim());
        }
    }
    flush(ui, &mut paragraph);
}

/// A picture of the tutorial, at most `width` points wide.
fn picture_ui(ui: &mut Ui, name: &str, width: f32) -> egui::Response {
    let id = Id::new(("cdept-guide-picture", name));
    let texture = ui.data(|d| d.get_temp::<TextureHandle>(id)).or_else(|| {
        let bytes = super::tutorial::picture(name)?;
        let image = image::load_from_memory(bytes).ok()?.to_rgba8();
        let size = [image.width() as usize, image.height() as usize];
        let image = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
        let texture = ui
            .ctx()
            .load_texture(name, image, egui::TextureOptions::LINEAR);
        ui.data_mut(|d| d.insert_temp(id, texture.clone()));
        Some(texture)
    });
    let Some(texture) = texture else {
        return ui.weak(format!("(no picture '{name}')"));
    };
    let size = texture.size_vec2();
    let scale = (width / size.x).min(1.0);
    let sized = egui::load::SizedTexture::new(texture.id(), size * scale);
    ui.add(egui::Image::new(sized).sense(egui::Sense::click()))
}

/// The button that makes the starting project, and how that goes.
fn start_ui(ui: &mut Ui, start: &Start, id: &str) -> bool {
    let place = super::base_dir()
        .map(|base| base.join(id).display().to_string())
        .unwrap_or_default();
    match start {
        Start::Scanning(job) => {
            ui.label("Reading the PDK…");
            crate::pdkscan::progress_ui(ui, job);
            false
        }
        Start::Made(dir) => {
            ui.label(format!("✔ Made in {}", dir.display()));
            false
        }
        other => {
            if let Start::Failed(error) = other {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
            let button = ui
                .button("📦 Make the starting project")
                .on_hover_text(format!(
                    "A new folder, {place} (or with a number): nothing is written over"
                ));
            super::anchor(ui, "dialog.guide.start", &button);
            button.clicked()
        }
    }
}

/// What the circuits of the step still lack.
fn checklists(ui: &mut Ui, checks: &[(String, Option<Vec<Line>>)], tutorial: &super::Tutorial) {
    for (id, lines) in checks {
        let view = tutorial.circuit(id).map_or("", |c| c.view.as_str());
        ui.add_space(4.0);
        let Some(lines) = lines else {
            ui.weak(format!("To have it checked, open the schematic {view}."));
            continue;
        };
        let right = lines.iter().filter(|l| l.ok).count();
        ui.strong(format!(
            "Your schematic: {right} of {} checks pass",
            lines.len()
        ));
        // What is wrong first: that is what to work on.
        for line in lines.iter().filter(|l| !l.ok) {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("✖").color(ui.visuals().warn_fg_color));
                ui.add(egui::Label::new(&line.text).wrap());
            });
        }
        egui::CollapsingHeader::new(format!("What is right ({right})"))
            .id_salt(("cdept-guide-right", id))
            .show(ui, |ui| {
                for line in lines.iter().filter(|l| l.ok) {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new("✔").color(Color32::from_rgb(0x4c, 0xaf, 0x50)));
                        ui.add(egui::Label::new(&line.text).wrap());
                    });
                }
            });
    }
}

/// The card: the list of steps with the current one open. It floats above dialogs, so
/// that it can be read and used while the wizard is open.
pub(crate) fn show(ctx: &Context, guide: &mut Guide, project: Option<&str>) {
    if guide.closed {
        return;
    }
    let screen = ctx.content_rect();
    let mut open = true;
    let shown = egui::Window::new(format!("🎓 {}", guide.tutorial.title))
        .id(Id::new(("cdept-guide", &guide.tutorial.id)))
        .order(Order::Foreground)
        .open(&mut open)
        .default_width(WIDTH)
        .resizable(false)
        .pivot(Align2::RIGHT_BOTTOM)
        .default_pos(Pos2::new(screen.right() - 8.0, screen.bottom() - 32.0))
        .show(ctx, |ui| {
            ui.set_width(WIDTH);
            contents(ui, guide, project, screen.height() - 200.0);
        });
    // Above the modal dialogs, which take the top place when they open.
    if let Some(shown) = shown {
        ctx.move_to_top(shown.response.layer_id);
    }
    guide.closed = !open;
    if let Some(name) = guide.picture.clone() {
        let mut open = true;
        let large = egui::Window::new("Tutorial picture")
            .id(Id::new("cdept-guide-large"))
            .order(Order::Foreground)
            .collapsible(false)
            .open(&mut open)
            .pivot(Align2::CENTER_CENTER)
            .default_pos(screen.center())
            .show(ctx, |ui| {
                picture_ui(ui, &name, screen.width() * 0.6).clicked()
            });
        let clicked = large.as_ref().and_then(|l| l.inner).unwrap_or(false);
        if let Some(large) = large {
            ctx.move_to_top(large.response.layer_id);
        }
        if clicked || !open {
            guide.picture = None;
        }
    }
}

fn contents(ui: &mut Ui, guide: &mut Guide, project: Option<&str>, height: f32) {
    let count = guide.tutorial.steps.len();
    let finished = (0..count).filter(|n| guide.is_done(*n)).count();
    ui.add(
        egui::ProgressBar::new(finished as f32 / count as f32)
            .text(format!("{finished} of {count} steps done")),
    );
    let mut jump = None;
    let mut advance = false;
    let mut restart = false;
    let mut enlarge = None;
    let mut asked_start = false;
    ScrollArea::vertical()
        .max_height(height.max(200.0))
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for (n, step) in guide.tutorial.steps.iter().enumerate() {
                let current = n == guide.at();
                let mark = match (current, guide.is_done(n)) {
                    (true, _) => "▶",
                    (false, true) => "✔",
                    (false, false) => "○",
                };
                let mut title = RichText::new(format!("{mark} {}. {}", n + 1, step.title));
                if current {
                    title = title.strong();
                }
                let row = ui.selectable_label(current, title);
                if row.clicked() {
                    jump = Some(n);
                }
                if !current {
                    continue;
                }
                ui.indent(("step", n), |ui| {
                    if let Some(notice) = &guide.notice {
                        ui.colored_label(ui.visuals().warn_fg_color, notice);
                        ui.add_space(4.0);
                    }
                    let text = step
                        .text
                        .replace("{project}", project.unwrap_or("<your project folder>"));
                    body(ui, &text);
                    for what in &step.status {
                        status_line(ui, what);
                    }
                    if let Some(name) = &step.picture
                        && picture_ui(ui, name, WIDTH - 40.0)
                            .on_hover_text("Click to see it large")
                            .clicked()
                    {
                        enlarge = Some(name.clone());
                    }
                    if step.start_button {
                        asked_start |= start_ui(ui, &guide.start, &guide.tutorial.id);
                    }
                    checklists(ui, &guide.checks, &guide.tutorial);
                });
                // Bring a new step into view once.
                let id = Id::new("cdept-guide-shown");
                if ui.data(|d| d.get_temp::<usize>(id)) != Some(n) {
                    ui.data_mut(|d| d.insert_temp(id, n));
                    row.scroll_to_me(Some(Align::TOP));
                }
            }
            if guide.finished() {
                ui.add_space(6.0);
                ui.strong("Done: you finished the tutorial.");
                if ui.button("Start again").clicked() {
                    restart = true;
                }
            }
        });
    // Below the list, so that they are in sight however long a step's text is.
    if guide.step().is_some() {
        let at = guide.at();
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            if at > 0 && ui.button("‹ Back").clicked() {
                jump = Some(at - 1);
            }
            if guide.shows_continue() {
                let label = if at + 1 == count {
                    "Finish"
                } else {
                    "Continue ›"
                };
                let hint = guide.step().map_or("", |s| s.needs_hint.as_str());
                let button = ui
                    .add_enabled(guide.can_continue(), egui::Button::new(label))
                    .on_disabled_hover_text(hint);
                super::anchor(ui, "guide.continue", &button);
                if button.clicked() {
                    advance = true;
                }
                if !guide.can_continue() {
                    ui.weak(hint);
                }
            } else {
                ui.weak("Moves on when you have done it.");
            }
        });
    }
    if let Some(n) = jump {
        guide.jump(n);
    }
    if advance {
        guide.advance();
    }
    if enlarge.is_some() {
        guide.picture = enlarge;
    }
    if asked_start {
        guide.start = Start::Asked;
    }
    if restart {
        let project = project.map(Into::into);
        *guide = Guide::new(guide.tutorial.clone(), project);
    }
}
