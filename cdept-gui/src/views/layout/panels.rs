//! The panels beside the picture: layers, cells and selection.

use egui::{CollapsingHeader, Color32, RichText, ScrollArea, Sense, Stroke, Ui, Vec2};
use libcdept::klayout::protocol::{CellNode, Layer};

use super::LayoutView;

/// How deep the cell tree is followed (guards against cycles in the report).
const MAX_DEPTH: usize = 24;
/// The most the list of selected objects shows (the helper sends no more than 200).
const SHOWN_ITEMS: usize = 200;

/// `#rrggbb` as a colour; grey when it is not one.
pub fn color(text: &str) -> Color32 {
    let hex = text.trim().trim_start_matches('#');
    match (hex.len(), u32::from_str_radix(hex, 16)) {
        (6, Ok(v)) => Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8),
        _ => Color32::GRAY,
    }
}

/// Whether the layer list's filter keeps `layer`: a part of its name or of its
/// source (`8/0`), in either case of letters.
fn layer_matches(layer: &Layer, filter: &str) -> bool {
    let filter = filter.trim().to_lowercase();
    filter.is_empty()
        || layer.name.to_lowercase().contains(&filter)
        || layer.source.to_lowercase().contains(&filter)
}

/// What a click in the panels asks for.
enum Act {
    Visible(u32, bool),
    Current(u32),
    ShowCell(Vec<String>),
    OpenFile(std::path::PathBuf),
}

impl LayoutView {
    pub(super) fn side_panels(&mut self, ui: &mut Ui) {
        let mut acts = Vec::new();
        ScrollArea::vertical()
            .id_salt(("klayout-side-scroll", self.id))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                self.layers_section(ui, &mut acts);
                self.cells_section(ui, &mut acts);
                self.selection_section(ui);
            });
        for act in acts {
            match act {
                Act::Visible(id, visible) => self.set_layer_visible(id, visible),
                Act::Current(id) => self.set_current_layer(id),
                Act::ShowCell(path) => self.show_cell(&path),
                Act::OpenFile(file) => self.open_requests.push(file),
            }
        }
    }

    fn layers_section(&mut self, ui: &mut Ui, acts: &mut Vec<Act>) {
        let title = format!("Layers ({})", self.layers.len());
        let editable = self.editable && self.is_running();
        let open = CollapsingHeader::new(title)
            .id_salt(("klayout-layers", self.id))
            .default_open(true)
            .show(ui, |ui| {
                if self.layers.is_empty() {
                    ui.weak("No layers.");
                }
                // A PDK has hundreds of layers: find one by its name or numbers.
                let id = egui::Id::new(("klayout-layer-filter", self.id));
                let mut filter: String = ui.data(|d| d.get_temp(id)).unwrap_or_default();
                let find = ui.add(
                    egui::TextEdit::singleline(&mut filter)
                        .desired_width(180.0)
                        .hint_text("Find a layer: name or 8/0"),
                );
                // `layout.find:<layer>`: the box, while that layer is not the current one.
                for wanted in crate::guide::wanted(ui.ctx()) {
                    if let Some(name) = wanted.strip_prefix("layout.find:")
                        && !self.layers.iter().any(|l| l.current && l.name == name)
                    {
                        crate::guide::anchor(ui, &wanted, &find);
                    }
                }
                ui.data_mut(|d| d.insert_temp(id, filter.clone()));
                ScrollArea::vertical()
                    .id_salt(("klayout-layer-list", self.id))
                    .max_height(260.0)
                    .auto_shrink([true, true])
                    .show(ui, |ui| {
                        for layer in self.layers.iter().filter(|l| layer_matches(l, &filter)) {
                            layer_row(ui, layer, acts);
                        }
                    });
                ui.add_space(4.0);
                CollapsingHeader::new("Add layer")
                    .id_salt(("klayout-add-layer", self.id))
                    .show(ui, |ui| {
                        ui.add_enabled_ui(editable, |ui| {
                            let form = &mut self.add_layer;
                            ui.horizontal(|ui| {
                                ui.label("Layer");
                                ui.add(egui::DragValue::new(&mut form.layer).range(0..=65535));
                                ui.label("Datatype");
                                ui.add(egui::DragValue::new(&mut form.datatype).range(0..=65535));
                            });
                            ui.horizontal(|ui| {
                                ui.label("Name");
                                ui.add(
                                    egui::TextEdit::singleline(&mut form.name)
                                        .desired_width(110.0)
                                        .hint_text("optional"),
                                );
                            });
                            if ui.button("Add").clicked() {
                                let (layer, datatype, name) =
                                    (form.layer, form.datatype, form.name.clone());
                                self.add_layer(layer, datatype, &name);
                            }
                        });
                    });
            });
        self.sections.layers = open.fully_open();
    }

    fn cells_section(&mut self, ui: &mut Ui, acts: &mut Vec<Act>) {
        let open = CollapsingHeader::new("Cells")
            .id_salt(("klayout-cells", self.id))
            .default_open(true)
            .show(ui, |ui| {
                let Some(cells) = &self.cells else {
                    ui.weak("No cells.");
                    return;
                };
                let top = cells.top.clone();
                ui.horizontal_wrapped(|ui| {
                    ui.weak("Shown:");
                    ui.label(cells.path.join(" / "));
                });
                ui.add_space(2.0);
                let mut path = vec![top.clone()];
                cell_node(ui, &cells.cells, &cells.path, &top, &mut path, acts);
            });
        self.sections.cells = open.fully_open();
    }

    fn selection_section(&mut self, ui: &mut Ui) {
        let title = format!("Selection ({})", self.selection.count);
        let open = CollapsingHeader::new(title)
            .id_salt(("klayout-selection", self.id))
            .default_open(true)
            .show(ui, |ui| {
                if self.selection.count == 0 {
                    ui.weak("Nothing selected.");
                    return;
                }
                if let Some([l, b, r, t]) = self.selection.bbox {
                    ui.label(
                        RichText::new(format!("({l:.3}, {b:.3}) – ({r:.3}, {t:.3}) µm"))
                            .monospace(),
                    );
                }
                for item in self.selection.items.iter().take(SHOWN_ITEMS) {
                    let mut line = item.kind.clone();
                    if !item.layer.is_empty() {
                        line.push_str(&format!(" {}", item.layer));
                    }
                    line.push_str(&format!(" in {}", item.cell));
                    if !item.text.is_empty() {
                        line.push_str(&format!(": {}", item.text));
                    }
                    ui.add(egui::Label::new(line).truncate());
                }
                let more = self
                    .selection
                    .count
                    .saturating_sub(self.selection.items.len());
                if more > 0 {
                    ui.weak(format!("… and {more} more"));
                }
            });
        self.sections.selection = open.fully_open();
    }
}

/// The names from the top cell down to the first cell called `name`.
pub fn cell_path(cells: &[CellNode], top: &str, name: &str) -> Option<Vec<String>> {
    fn walk(cells: &[CellNode], at: &str, name: &str, path: &mut Vec<String>) -> bool {
        path.push(at.to_string());
        if at == name {
            return true;
        }
        let children = cells.iter().find(|c| c.name == at).map(|c| &c.children);
        if path.len() < MAX_DEPTH
            && children.is_some_and(|c| c.iter().any(|child| walk(cells, child, name, path)))
        {
            return true;
        }
        path.pop();
        false
    }
    let mut path = Vec::new();
    walk(cells, top, name, &mut path).then_some(path)
}

/// One line of the layer list: visibility, colour swatch, name.
fn layer_row(ui: &mut Ui, layer: &Layer, acts: &mut Vec<Act>) {
    ui.horizontal(|ui| {
        ui.add_space(8.0 * layer.depth as f32);
        let mut visible = layer.visible;
        if ui
            .checkbox(&mut visible, "")
            .on_hover_text("Visible")
            .changed()
        {
            acts.push(Act::Visible(layer.id, visible));
        }
        let (swatch, _) = ui.allocate_exact_size(Vec2::new(16.0, 12.0), Sense::hover());
        let fill = color(&layer.color);
        ui.painter().rect_filled(swatch, 2.0, fill);
        ui.painter().rect_stroke(
            swatch,
            2.0,
            Stroke::new(1.5, color(&layer.frame_color)),
            egui::StrokeKind::Inside,
        );
        let name = if layer.name.is_empty() {
            layer.source.as_str()
        } else {
            layer.name.as_str()
        };
        let mut text = RichText::new(name);
        if !layer.valid {
            text = text.weak();
        }
        let response = ui.selectable_label(layer.current, text);
        if !layer.current {
            crate::guide::anchor(ui, &format!("layout.layer:{}", layer.name), &response);
        }
        let response = if layer.name.is_empty() || layer.name == layer.source {
            response
        } else {
            response.on_hover_text(&layer.source)
        };
        if response.clicked() {
            acts.push(Act::Current(layer.id));
        }
        if !layer.name.is_empty() && layer.name != layer.source {
            ui.weak(&layer.source);
        }
    });
}

/// The tree below cell `name`; `path` is the way from the top to it.
fn cell_node(
    ui: &mut Ui,
    cells: &[CellNode],
    shown: &[String],
    name: &str,
    path: &mut Vec<String>,
    acts: &mut Vec<Act>,
) {
    let Some(node) = cells.iter().find(|c| c.name == name) else {
        ui.label(name);
        return;
    };
    let current = shown == path.as_slice();
    let text = |node: &CellNode| {
        let mut text = RichText::new(&node.name);
        if node.missing {
            text = text.color(Color32::from_rgb(230, 80, 80));
        } else if current {
            text = text.strong();
        }
        text
    };
    let tags = |ui: &mut Ui, node: &CellNode| {
        if let Some(label) = &node.external {
            ui.weak(format!("[{label}]"));
        }
    };
    let react =
        |node: &CellNode, response: &egui::Response, path: &[String], acts: &mut Vec<Act>| {
            let response = if node.missing {
                response.clone().on_hover_text("cannot be found")
            } else if let Some(label) = &node.external {
                response
                    .clone()
                    .on_hover_text(format!("{label}: double click to open that view"))
            } else {
                response.clone()
            };
            if response.double_clicked() {
                match (&node.file, node.external.is_some(), node.missing) {
                    (_, _, true) => {}
                    (Some(file), true, _) => acts.push(Act::OpenFile(file.into())),
                    (_, true, _) => {}
                    _ => acts.push(Act::ShowCell(path.to_vec())),
                }
            }
        };
    if node.children.is_empty() || path.len() >= MAX_DEPTH {
        ui.horizontal(|ui| {
            ui.add_space(ui.spacing().indent);
            let response = ui.selectable_label(current, text(node));
            tags(ui, node);
            react(node, &response, path, acts);
        });
        return;
    }
    let id = ui.make_persistent_id(("klayout-cell", path.join("/")));
    let header = egui::collapsing_header::CollapsingState::load_with_default_open(
        ui.ctx(),
        id,
        path.len() < 2,
    );
    header
        .show_header(ui, |ui| {
            let response = ui.selectable_label(current, text(node));
            tags(ui, node);
            react(node, &response, path, acts);
        })
        .body(|ui| {
            for child in &node.children {
                path.push(child.clone());
                cell_node(ui, cells, shown, child, path, acts);
                path.pop();
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(name: &str, children: &[&str]) -> CellNode {
        CellNode {
            name: name.into(),
            children: children.iter().map(|c| c.to_string()).collect(),
            external: None,
            missing: false,
            file: None,
        }
    }

    #[test]
    fn cell_paths_start_at_the_top() {
        let cells = [
            node("top", &["a", "b"]),
            node("a", &["c"]),
            node("b", &[]),
            node("c", &[]),
        ];
        assert_eq!(cell_path(&cells, "top", "top").unwrap(), ["top"]);
        assert_eq!(cell_path(&cells, "top", "c").unwrap(), ["top", "a", "c"]);
        assert_eq!(cell_path(&cells, "top", "b").unwrap(), ["top", "b"]);
        assert_eq!(cell_path(&cells, "top", "x"), None);
    }

    #[test]
    fn layer_colours_are_read() {
        assert_eq!(color("#ff8000"), Color32::from_rgb(255, 128, 0));
        assert_eq!(color("00ff00"), Color32::from_rgb(0, 255, 0));
        assert_eq!(color("nonsense"), Color32::GRAY);
        assert_eq!(color(""), Color32::GRAY);
    }
}
