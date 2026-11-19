//! PCells: the parameter form (KLayout's is Qt), and "Place Library Cell…", which places
//! a cell or PCell of one of KLayout's libraries (a PDK's, registered on `KLAYOUT_PATH`,
//! or KLayout's own "Basic").

use egui::{Context, Id, ScrollArea, Ui};
use libcdept::klayout::protocol::{KLibrary, ParamType, PcellForm, PcellParam, method};
use serde_json::{Map, Value, json};

use super::LayoutView;

/// The parameters of `form` as fields; `values` is edited. True when a value changed.
pub fn form_ui(
    ui: &mut Ui,
    id: Id,
    form: &PcellForm,
    values: &mut Map<String, Value>,
    layers: &[String],
    enabled: bool,
) -> bool {
    let mut changed = false;
    let mut group = "";
    egui::Grid::new(id)
        .num_columns(2)
        .spacing([10.0, 4.0])
        .striped(true)
        .show(ui, |ui| {
            for param in form.params.iter().filter(|p| !p.hidden) {
                if param.group != group {
                    group = &param.group;
                    if !group.is_empty() {
                        ui.strong(group);
                        ui.end_row();
                    }
                }
                let label = if param.unit.is_empty() || param.unit == "micron" {
                    param.description.clone()
                } else {
                    format!("{} [{}]", param.description, param.unit)
                };
                ui.label(label).on_hover_text(&param.name);
                let value = values.entry(param.name.clone()).or_insert(Value::Null);
                ui.add_enabled_ui(enabled && !param.readonly, |ui| {
                    changed |= field(ui, id.with(&param.name), param, value, layers);
                });
                ui.end_row();
            }
        });
    changed
}

/// One parameter.
fn field(ui: &mut Ui, id: Id, param: &PcellParam, value: &mut Value, layers: &[String]) -> bool {
    let micron = param.unit == "micron";
    if !param.choices.is_empty() {
        let current = param
            .choices
            .iter()
            .find(|c| c.value == *value)
            .map_or_else(|| text_of(value), |c| c.label.clone());
        let mut changed = false;
        egui::ComboBox::from_id_salt(id)
            .selected_text(current)
            .show_ui(ui, |ui| {
                for choice in &param.choices {
                    if ui
                        .selectable_label(choice.value == *value, &choice.label)
                        .clicked()
                    {
                        *value = choice.value.clone();
                        changed = true;
                    }
                }
            });
        return changed;
    }
    match param.kind {
        ParamType::Int => {
            let mut n = value
                .as_i64()
                .or_else(|| value.as_f64().map(|f| f as i64))
                .unwrap_or(0);
            let changed = ui.add(egui::DragValue::new(&mut n)).changed();
            if changed || !value.is_i64() {
                *value = json!(n);
            }
            changed
        }
        ParamType::Double => {
            let mut f = value.as_f64().unwrap_or(0.0);
            let mut drag = egui::DragValue::new(&mut f).speed(0.01).max_decimals(6);
            if micron {
                drag = drag.suffix(" µm");
            }
            let changed = ui.add(drag).changed();
            if changed || !value.is_number() {
                *value = json!(f);
            }
            changed
        }
        ParamType::Bool => {
            let mut b = value.as_bool().unwrap_or(false);
            let changed = ui.checkbox(&mut b, "").changed();
            if changed || !value.is_boolean() {
                *value = json!(b);
            }
            changed
        }
        ParamType::String => {
            let mut text = value.as_str().unwrap_or_default().to_string();
            let changed = ui.text_edit_singleline(&mut text).changed();
            if changed || !value.is_string() {
                *value = json!(text);
            }
            changed
        }
        ParamType::List => {
            let mut text = match &*value {
                Value::Array(items) => items.iter().map(text_of).collect::<Vec<_>>().join(", "),
                other => text_of(other),
            };
            let changed = ui
                .text_edit_singleline(&mut text)
                .on_hover_text("Values separated by commas")
                .changed();
            if changed {
                *value = json!(
                    text.split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                );
            }
            changed
        }
        ParamType::Layer => layer_field(ui, id, value, layers),
        ParamType::Shape | ParamType::Callback | ParamType::None => {
            ui.weak(text_of(value));
            false
        }
    }
}

/// A layer: one of the layout's, or typed (`1/0`, `name (1/0)`).
fn layer_field(ui: &mut Ui, id: Id, value: &mut Value, layers: &[String]) -> bool {
    let mut text = value.as_str().unwrap_or_default().to_string();
    let mut changed = false;
    ui.horizontal(|ui| {
        changed |= ui
            .add(egui::TextEdit::singleline(&mut text).desired_width(90.0))
            .changed();
        egui::ComboBox::from_id_salt(id)
            .selected_text("")
            .width(20.0)
            .show_ui(ui, |ui| {
                for layer in layers {
                    if ui.selectable_label(*layer == text, layer).clicked() {
                        text = layer.clone();
                        changed = true;
                    }
                }
            });
    });
    if changed {
        *value = json!(text);
    }
    changed
}

fn text_of(value: &Value) -> String {
    match value {
        Value::Null => "(none)".to_string(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// The values of `now` that differ from `before`.
pub fn changes(before: &Map<String, Value>, now: &Map<String, Value>) -> Map<String, Value> {
    now.iter()
        .filter(|(k, v)| before.get(*k) != Some(*v))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// "Place Library Cell…": what was asked and chosen so far.
#[derive(Default)]
pub struct LibraryPicker {
    /// The `libraries` call, until it is answered.
    asked: Option<u64>,
    libraries: Vec<KLibrary>,
    library: Option<String>,
    cell: Option<String>,
    /// The `pcell_form` call for the chosen PCell.
    form_asked: Option<u64>,
    form: Option<PcellForm>,
    values: Map<String, Value>,
    error: Option<String>,
}

impl LibraryPicker {
    fn chosen_cell(&self) -> Option<(&KLibrary, &libcdept::klayout::protocol::KLibraryCell)> {
        let lib = self
            .libraries
            .iter()
            .find(|l| Some(&l.name) == self.library.as_ref())?;
        let cell = lib
            .cells
            .iter()
            .find(|c| Some(&c.name) == self.cell.as_ref())?;
        Some((lib, cell))
    }
}

impl LayoutView {
    /// Open "Place Library Cell…".
    pub fn open_library_picker(&mut self) {
        let asked = self.raw_call(method::LIBRARIES, json!({}));
        self.library_picker = Some(LibraryPicker {
            asked: Some(asked),
            ..LibraryPicker::default()
        });
    }

    /// Instance mode with a cell or PCell of a KLayout library; `values` are PCell
    /// parameters (the others keep their defaults).
    pub fn place_library(&mut self, library: &str, cell: &str, values: Map<String, Value>) -> u64 {
        self.mode = "instance".into();
        self.call(
            method::PLACE_LIBRARY,
            json!({"library": library, "cell": cell, "values": values}),
            super::Pending::Quiet,
        )
    }

    /// The answer to a [`raw_call`](Self::raw_call) as it came; errors are shown.
    pub(super) fn take_value(&mut self, id: u64) -> Option<Result<Value, String>> {
        let at = self.replies.iter().position(|r| r.id == id)?;
        let result = self.replies.remove(at).result;
        if let Err(error) = &result {
            self.notify(
                libcdept::klayout::protocol::NoticeLevel::Error,
                error.clone(),
            );
        }
        Some(result)
    }

    /// The window of "Place Library Cell…", while it is open.
    pub(super) fn library_picker_ui(&mut self, ctx: &Context) {
        let Some(mut picker) = self.library_picker.take() else {
            return;
        };
        if let Some(id) = picker.asked
            && let Some(answer) = self.take_value(id)
        {
            picker.asked = None;
            match answer.and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string())) {
                Ok(libraries) => picker.libraries = libraries,
                Err(e) => picker.error = Some(e),
            }
        }
        if let Some(id) = picker.form_asked
            && let Some(answer) = self.take_value(id)
        {
            picker.form_asked = None;
            match answer
                .and_then(|v| serde_json::from_value::<PcellForm>(v).map_err(|e| e.to_string()))
            {
                Ok(form) => {
                    picker.values = form.values.clone();
                    picker.form = Some(form);
                }
                Err(e) => picker.error = Some(e),
            }
        }
        let layers: Vec<String> = self
            .layers
            .iter()
            .filter(|l| l.valid)
            .map(|l| l.source.split('@').next().unwrap_or_default().to_string())
            .collect();
        let mut open = true;
        let mut keep = true;
        let mut ask_form = None;
        let mut place = None;
        let window_id = Id::new(("klayout-library-picker", self.id.0));
        egui::Window::new("Place Library Cell")
            .id(window_id)
            .open(&mut open)
            .collapsible(false)
            .default_pos(ctx.content_rect().center())
            .pivot(egui::Align2::CENTER_CENTER)
            .show(ctx, |ui| {
                if picker.asked.is_some() {
                    ui.spinner();
                    return;
                }
                if picker.libraries.is_empty() {
                    ui.weak("KLayout has no libraries");
                }
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.strong("Library");
                        ScrollArea::vertical()
                            .id_salt("libraries")
                            .max_height(260.0)
                            .show(ui, |ui| {
                                ui.set_min_width(140.0);
                                for lib in &picker.libraries {
                                    let on = picker.library.as_ref() == Some(&lib.name);
                                    let r = ui.selectable_label(on, &lib.name);
                                    let r = if lib.description.is_empty() {
                                        r
                                    } else {
                                        r.on_hover_text(&lib.description)
                                    };
                                    if r.clicked() && !on {
                                        picker.library = Some(lib.name.clone());
                                        picker.cell = None;
                                        picker.form = None;
                                    }
                                }
                            });
                    });
                    ui.separator();
                    ui.vertical(|ui| {
                        ui.strong("Cell");
                        ScrollArea::vertical()
                            .id_salt("cells")
                            .max_height(260.0)
                            .show(ui, |ui| {
                                ui.set_min_width(160.0);
                                let Some(lib) = picker
                                    .libraries
                                    .iter()
                                    .find(|l| picker.library.as_ref() == Some(&l.name))
                                else {
                                    ui.weak("Select a library");
                                    return;
                                };
                                for cell in &lib.cells {
                                    let on = picker.cell.as_ref() == Some(&cell.name);
                                    let label = if cell.pcell {
                                        format!("{} (PCell)", cell.name)
                                    } else {
                                        cell.name.clone()
                                    };
                                    if ui.selectable_label(on, label).clicked() && !on {
                                        picker.cell = Some(cell.name.clone());
                                        picker.form = None;
                                        if cell.pcell {
                                            ask_form = Some((lib.name.clone(), cell.name.clone()));
                                        }
                                    }
                                }
                            });
                    });
                    if let Some(form) = &picker.form {
                        ui.separator();
                        ui.vertical(|ui| {
                            ui.strong("Parameters");
                            ScrollArea::vertical()
                                .id_salt("parameters")
                                .max_height(300.0)
                                .show(ui, |ui| {
                                    form_ui(
                                        ui,
                                        window_id.with("form"),
                                        form,
                                        &mut picker.values,
                                        &layers,
                                        true,
                                    );
                                });
                        });
                    } else if picker.form_asked.is_some() {
                        ui.spinner();
                    }
                });
                if let Some(error) = &picker.error {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let chosen = picker.chosen_cell();
                    let ready = chosen.is_some_and(|(_, c)| !c.pcell || picker.form.is_some());
                    if ui.add_enabled(ready, egui::Button::new("Place")).clicked()
                        && let Some((lib, cell)) = chosen
                    {
                        let values = if cell.pcell {
                            picker.values.clone()
                        } else {
                            Map::new()
                        };
                        place = Some((lib.name.clone(), cell.name.clone(), values));
                    }
                    if ui.button("Cancel").clicked() {
                        keep = false;
                    }
                });
            });
        if let Some((library, cell)) = ask_form {
            picker.form_asked = Some(self.raw_call(
                method::PCELL_FORM,
                json!({"library": library, "cell": cell}),
            ));
        }
        if let Some((library, cell, values)) = place {
            self.place_library(&library, &cell, values);
            keep = false;
        }
        if open && keep {
            self.library_picker = Some(picker);
        }
    }
}
