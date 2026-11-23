//! Inspector panel: details of what is selected. That is what is selected in the
//! schematic in front, if anything is; otherwise the library, cell or view selected in
//! the library views, with a picture of a schematic or symbol view.

use egui::{Grid, RichText, Ui};
use libcdept::Format;

use super::library::{LINK_MARK, fix_label, link_hover};
use super::{Selected, View, empty_state};
use crate::state::{AppCtx, view_link};

#[derive(Default)]
pub struct InspectorView;

fn row(ui: &mut Ui, label: &str, value: impl Into<String>) {
    ui.weak(label);
    ui.add(egui::Label::new(value.into()).wrap());
    ui.end_row();
}

/// The attributes of an instance, one `name=value` per line. A value in quotes may
/// hold spaces and line ends.
fn attribute_lines(attributes: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut escaped = false;
    for c in attributes.chars() {
        if c.is_whitespace() && !quoted {
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
            }
            continue;
        }
        if c == '"' && !escaped {
            quoted = !quoted;
        }
        escaped = c == '\\' && !escaped;
        current.push(c);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

/// What is selected in the schematic `title`.
fn schematic_selection(ui: &mut Ui, title: &str, selected: &Selected) {
    ui.strong(format!("Selected in {}", title.trim_end_matches('*')));
    let others = selected
        .count
        .saturating_sub(selected.instances.len() as u32 + selected.wires + selected.texts);
    let mut counts = Vec::new();
    for (count, one, many) in [
        (selected.instances.len() as u32, "instance", "instances"),
        (selected.wires, "wire", "wires"),
        (selected.texts, "text", "texts"),
        (others, "other object", "other objects"),
    ] {
        if count > 0 {
            counts.push(format!("{count} {}", if count == 1 { one } else { many }));
        }
    }
    ui.weak(counts.join(", "));
    for instance in &selected.instances {
        ui.separator();
        ui.strong(format!("Instance {}", instance.name));
        Grid::new(("inspect-instance", &instance.name))
            .num_columns(2)
            .show(ui, |ui| {
                row(ui, "Symbol", instance.symbol.as_str());
                ui.weak("Attributes");
                ui.vertical(|ui| {
                    for line in attribute_lines(&instance.attributes) {
                        ui.add(egui::Label::new(RichText::new(line).monospace()).wrap());
                    }
                });
                ui.end_row();
            });
    }
}

impl View for InspectorView {
    fn title(&self, _ctx: &AppCtx) -> String {
        "Inspector".into()
    }

    fn ui(&mut self, ui: &mut Ui, ctx: &mut AppCtx) {
        if let Some(active) = &ctx.schematic
            && let Some(selected) = &active.selection
        {
            return schematic_selection(ui, &active.title, selected);
        }
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
        // Files of the library (or the selected cell) that are part of no view.
        crate::library_dialog::unrecognised_selected(ui, ctx);

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
                let embedded = matches!(format, Format::XschemSchematic | Format::XschemSymbol);
                let layout = matches!(format, Format::Gds | Format::Oasis);
                let tool = match format.tool() {
                    // KLayout runs in a helper of its own and is shown in a tab.
                    Some(_) if layout => "KLayout, in a tab".into(),
                    // A LEF abstract is drawn by KLayout's LEF reader, read-only.
                    None if format == Format::Lef => "KLayout, in a tab (read-only)".into(),
                    // xschem is built in: it needs its engine, not the program.
                    Some(_) if embedded => match ctx.tools.helper() {
                        Some(helper) if helper.path.is_none() => {
                            "xschem, in a tab (its engine was not found)".into()
                        }
                        _ => "xschem, in a tab".into(),
                    },
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
            // What the file says about the cell (LEF, Liberty, netlists, …).
            for (name, value) in super::contained::facts(ui, entry, &cell.name).iter() {
                row(ui, name, value);
            }
        });
        let mut fix = None;
        if let Some(u) = view_link(&ctx.links, entry) {
            ui.add_space(4.0);
            ui.colored_label(
                ui.visuals().warn_fg_color,
                format!("{LINK_MARK} Schematic link: {}", u.problem),
            );
            ui.add(
                egui::Label::new(RichText::new(format!("schematic={}", u.expected)).monospace())
                    .wrap(),
            );
            let writable = ctx
                .selected_library()
                .is_some_and(|l| !l.read_only && l.error.is_none());
            if ui
                .add_enabled(writable, egui::Button::new(fix_label(u)))
                .on_hover_text(link_hover(u))
                .on_disabled_hover_text("The library is read-only")
                .clicked()
            {
                fix = Some(u.clone());
            }
        }
        if let Some(u) = fix {
            ctx.fix_links(&[u]);
            return;
        }
        if entry.files.len() > 1 {
            ui.add_space(4.0);
            ui.weak("Files");
            for file in &entry.files {
                if let Some(name) = file.file_name() {
                    ui.label(RichText::new(name.to_string_lossy()).monospace());
                }
            }
        }

        // A picture of what xschem can draw.
        let layout = matches!(
            entry.format,
            Some(Format::Gds | Format::Oasis | Format::Lef)
        );
        let drawable = matches!(
            entry.format,
            Some(Format::XschemSchematic | Format::XschemSymbol)
        );
        let main_file = entry.main_file.clone();
        // Of a file that holds several cells, the one this view is.
        let member = entry.member.clone();
        if let (true, Some(file)) = (drawable, main_file.clone()) {
            ui.add_space(6.0);
            ctx.engines.preview(ui, project, &file);
        }
        // A picture of a layout, drawn by a KLayout of its own.
        if let (true, Some(file)) = (layout, main_file) {
            ui.add_space(6.0);
            ctx.engines
                .layout_member_preview(ui, project, &file, member.as_deref());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attributes_are_listed_one_per_line() {
        assert_eq!(
            attribute_lines("name=M1 model=nmos  w=1u\nl=0.35u"),
            ["name=M1", "model=nmos", "w=1u", "l=0.35u"]
        );
        // A quoted value stays whole, with its spaces and escaped quotes.
        assert_eq!(
            attribute_lines(r#"name=Vin value="pulse(0 1.8 1n)" note="say \"hi\" twice""#),
            [
                "name=Vin",
                r#"value="pulse(0 1.8 1n)""#,
                r#"note="say \"hi\" twice""#
            ]
        );
        assert!(attribute_lines("  ").is_empty());
    }
}
