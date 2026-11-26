use egui::{Grid, RichText, ScrollArea, Ui};

use super::{View, empty_state};
use crate::state::{Action, AppCtx, HelperInfo};

/// External tools (binaries, Python packages, KLayout packages) and where they were found. Becomes the process supervisor view in Phase 3.
#[derive(Default)]
pub struct ToolsView;

/// The embedded xschem engine: part of CDePT, but a program of its own that has to be
/// built and found.
fn helper_row(ui: &mut Ui, helper: Option<HelperInfo>) {
    let helper = helper.unwrap_or_default();
    let (mark, color) = match helper.path {
        Some(_) => ("✔", ui.visuals().text_color()),
        None => ("✖", ui.visuals().warn_fg_color),
    };
    let name = ui.label(RichText::new(format!("{mark} cdept-xschem")).color(color));
    match &helper.path {
        Some(path) => name.on_hover_text(path.display().to_string()),
        None => name.on_hover_text(
            "Not found next to cdept or on PATH. Build it with\n\
             cargo build -p cdept-xschem\nor set $CDEPT_XSCHEM to its path.",
        ),
    };
    ui.weak("embedded schematic editor")
        .on_hover_text("xschem's engine as a helper program; schematics open in tabs through it");
    ui.weak("3");
    match (helper.path, helper.version) {
        (Some(_), Some(version)) => ui.label(version),
        (Some(_), None) => ui.weak("found, does not start"),
        (None, _) => ui.weak("not found"),
    };
    ui.end_row();
}

/// The tool windows CDePT started (`libcdept::launch::Launcher`): how they run, their
/// logs, stop and start again.
fn processes_ui(ui: &mut Ui, ctx: &mut AppCtx) {
    let procs = ctx.launcher.processes();
    if procs.is_empty() {
        return;
    }
    ui.strong("Tool windows started by CDePT");
    let mut todo: Option<(usize, bool)> = None;
    Grid::new("tool-windows").striped(true).num_columns(5).show(ui, |ui| {
        for p in &procs {
            ui.label(p.tool);
            ui.label(
                p.file
                    .as_ref()
                    .and_then(|f| f.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "(no file)".into()),
            );
            match &p.exit {
                None => ui.label(format!("running ({})", p.pid)),
                Some(exit) => ui.weak(exit),
            };
            if ui
                .small_button("log")
                .on_hover_text(p.log.display().to_string())
                .clicked()
            {
                ui.ctx().copy_text(p.log.display().to_string());
            }
            ui.horizontal(|ui| {
                if p.exit.is_none() && ui.small_button("Stop").clicked() {
                    todo = Some((p.index, false));
                }
                if ui
                    .small_button("Restart")
                    .on_hover_text("Start it again with the same command")
                    .clicked()
                {
                    todo = Some((p.index, true));
                }
            });
            ui.end_row();
        }
    });
    if let Some((index, restart)) = todo {
        let result = if restart {
            ctx.launcher.restart(index).map(|_| ())
        } else {
            ctx.launcher.stop(index)
        };
        if let Err(e) = result {
            ctx.error(format!("{e:#}"));
        }
    }
    ui.separator();
}

impl View for ToolsView {
    fn title(&self, _ctx: &AppCtx) -> String {
        "Tools".into()
    }

    fn ui(&mut self, ui: &mut Ui, ctx: &mut AppCtx) {
        ui.horizontal(|ui| {
            let running = ctx.tools.is_running();
            if ui
                .add_enabled(!running, egui::Button::new("⟳ Detect"))
                .clicked()
            {
                ctx.request(Action::DetectTools);
            }
            if running {
                ui.spinner();
            }
        });
        ui.separator();
        processes_ui(ui, ctx);
        let Some(tools) = ctx.tools.tools() else {
            empty_state(ui, "Detecting tools…", None);
            return;
        };
        ScrollArea::both().auto_shrink(false).show(ui, |ui| {
            Grid::new("tools-grid")
                .striped(true)
                .num_columns(4)
                .show(ui, |ui| {
                    ui.strong("Tool");
                    ui.strong("Role");
                    ui.strong("Phase");
                    ui.strong("Version");
                    ui.end_row();
                    helper_row(ui, ctx.tools.helper());
                    for t in &tools {
                        let (mark, color) = if t.found() {
                            ("✔", ui.visuals().text_color())
                        } else {
                            ("✖", ui.visuals().weak_text_color())
                        };
                        let name =
                            ui.label(RichText::new(format!("{mark} {}", t.spec.name)).color(color));
                        if let Some(path) = &t.path {
                            name.on_hover_text(path.display().to_string());
                        }
                        let role = ui.weak(t.spec.role);
                        if let Some(note) = t.spec.note {
                            role.on_hover_text(note);
                        }
                        match t.spec.phase {
                            Some(p) => ui.weak(p.to_string()),
                            None => ui
                                .weak("–")
                                .on_hover_text("Detected only; no integration planned yet"),
                        };
                        match (&t.path, &t.version) {
                            (Some(_), Some(v)) => ui.label(v),
                            (Some(_), None) => ui.weak("found"),
                            (None, _) => ui.weak("not found"),
                        };
                        ui.end_row();
                    }
                });
        });
    }
}
