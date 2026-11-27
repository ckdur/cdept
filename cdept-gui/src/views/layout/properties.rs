//! The properties of the selection (KLayout's dialog is Qt): boxes, polygons, paths,
//! texts and instances, with the parameters of PCell instances. Each "Apply" is one
//! step of KLayout's undo.

use egui::{Context, DragValue, Id, ScrollArea, Ui};
use libcdept::klayout::protocol::{InstArray, Object, Properties, Shape, method};
use serde_json::{Map, Value, json};

use super::LayoutView;
use super::pcell;

/// What an instance places instead ([`LayoutView::replace_cell`]).
#[derive(Debug, Clone, PartialEq)]
pub enum Replacement {
    Library {
        library: String,
        cell: String,
        values: Map<String, Value>,
    },
    View {
        cell: String,
        file: std::path::PathBuf,
    },
}

/// The dialog while it is open.
#[derive(Default)]
pub struct PropertiesDialog {
    /// The call being answered (`properties` or `set_properties`).
    asked: Option<u64>,
    /// Close when the change asked for is made (OK).
    close_after: bool,
    /// Close again when the answer shows nothing selected (opened by a double click).
    quiet_if_empty: bool,
    props: Properties,
    /// Which object is shown.
    current: usize,
    /// The form: a copy of the object, and its PCell values.
    edit: Option<Object>,
    pcell: Map<String, Value>,
    /// The selection the form was made for.
    seen: Option<(usize, Vec<String>)>,
    error: Option<String>,
    /// "Change…" was chosen for the instance shown.
    change: Option<ChangeCell>,
}

/// Where the new cell of an instance is chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeCell {
    /// A cell or PCell of KLayout's libraries ("Place Library Cell…").
    Library,
    /// A layout view of the project ("Place Cell…").
    View,
}

impl PropertiesDialog {
    fn original(&self) -> Option<&Object> {
        self.props.objects.get(self.current)
    }

    fn original_pcell(&self) -> Map<String, Value> {
        match self.original().map(|o| &o.shape) {
            Some(Shape::Instance(inst)) => inst
                .pcell
                .as_ref()
                .map(|f| f.values.clone())
                .unwrap_or_default(),
            _ => Map::new(),
        }
    }

    /// The form has changes not applied yet.
    fn is_edited(&self) -> bool {
        self.edit.as_ref() != self.original()
            || !pcell::changes(&self.original_pcell(), &self.pcell).is_empty()
    }

    fn show(&mut self, index: usize) {
        self.current = index.min(self.props.objects.len().saturating_sub(1));
        self.edit = self.original().cloned();
        self.pcell = self.original_pcell();
        self.error = None;
    }

    fn take(&mut self, props: Properties) {
        let at = self.current;
        self.props = props;
        self.show(at);
    }
}

impl LayoutView {
    /// Make instance `target` (`index`, `key` of the properties) place cell `cell` of
    /// KLayout library `library` (PCell parameters `values`), or the layout view
    /// `file` of the project. The properties show the outcome.
    pub fn replace_cell(&mut self, target: (usize, String), with: Replacement) -> u64 {
        let (index, key) = target;
        let params = match with {
            Replacement::Library {
                library,
                cell,
                values,
            } => json!({"index": index, "key": key, "library": library, "cell": cell,
                        "values": values}),
            Replacement::View { cell, file } => {
                let table = libcdept::layout::refs::resolve(&self.project, &file);
                self.refs.files.extend(table.files.clone());
                self.refs.pcells.extend(table.pcells.clone());
                self.session.set_refs(self.refs.clone());
                let mut sent = self.refs.clone();
                sent.root = file;
                json!({"index": index, "key": key, "refs": sent, "name": cell})
            }
        };
        let id = self.raw_call(method::REPLACE_CELL, params);
        let dialog = self
            .properties
            .get_or_insert_with(PropertiesDialog::default);
        dialog.asked = Some(id);
        dialog.close_after = false;
        id
    }

    /// The main file of view `library/cell/view` of the project.
    pub fn project_file(
        &self,
        library: &str,
        cell: &str,
        view: &str,
    ) -> Option<std::path::PathBuf> {
        self.project
            .library(library)?
            .cell(cell)?
            .entry(view)?
            .main_file
            .clone()
    }

    /// Replace the instance the properties show (for the test driver).
    pub fn replace_shown(&mut self, with: Replacement) -> Result<(), String> {
        let dialog = self
            .properties
            .as_ref()
            .ok_or("the properties are not open")?;
        if dialog.asked.is_some() {
            return Err("the properties are not there yet".into());
        }
        let object = dialog.edit.as_ref().ok_or("nothing is selected")?;
        if !matches!(object.shape, Shape::Instance(_)) {
            return Err("the object shown is not an instance".into());
        }
        let target = (object.index, object.key.clone());
        self.replace_cell(target, with);
        Ok(())
    }

    /// "Change… › Layout View…": the project's layout views; the choice replaces.
    fn choose_view_to_replace(&mut self, target: (usize, String)) {
        self.place = Some(super::place::PlacePicker::default());
        self.replace_target = Some(target);
    }

    /// Open the properties of the selection (Q, a double click, the tool bar).
    pub fn open_properties(&mut self) {
        let mut dialog = self.properties.take().unwrap_or_default();
        dialog.asked = Some(self.raw_call(method::PROPERTIES, json!({})));
        dialog.seen = Some(self.selection_signature());
        self.properties = Some(dialog);
    }

    /// The same, but nothing opens when nothing is selected.
    pub(super) fn open_properties_if_selected(&mut self) {
        let open = self.properties.is_some();
        self.open_properties();
        if !open && let Some(dialog) = &mut self.properties {
            dialog.quiet_if_empty = true;
        }
    }

    fn selection_signature(&self) -> (usize, Vec<String>) {
        let items = self
            .selection
            .items
            .iter()
            .map(|i| format!("{} {} {} {}", i.kind, i.cell, i.layer, i.text))
            .collect();
        (self.selection.count, items)
    }

    /// Apply the form: one undoable change. `close`: and close the dialog.
    pub fn apply_properties(&mut self, close: bool) -> Result<(), String> {
        let dialog = self
            .properties
            .as_ref()
            .ok_or("the properties are not open")?;
        let object = dialog.edit.clone().ok_or("nothing is selected")?;
        let pcell = pcell::changes(&dialog.original_pcell(), &dialog.pcell);
        let id = self.raw_call(
            method::SET_PROPERTIES,
            json!({"index": object.index, "key": object.key, "object": object, "pcell": pcell}),
        );
        if let Some(dialog) = &mut self.properties {
            dialog.asked = Some(id);
            dialog.close_after = close;
        }
        Ok(())
    }

    /// The form as JSON (for the test driver): the object's fields, and the PCell
    /// values under `pcell`.
    pub fn properties_form(&self) -> Option<Value> {
        let dialog = self.properties.as_ref()?;
        if dialog.asked.is_some() {
            return None;
        }
        let mut value = serde_json::to_value(dialog.edit.as_ref()?).ok()?;
        value["pcell"] = Value::Object(dialog.pcell.clone());
        Some(value)
    }

    /// Change one field of the form: `x`, `bbox.2`, `array.na`, `pcell.radius`, …
    /// `value` is JSON (a bare word is a string).
    pub fn set_properties_field(&mut self, field: &str, value: &str) -> Result<(), String> {
        let dialog = self
            .properties
            .as_mut()
            .ok_or("the properties are not open")?;
        if dialog.asked.is_some() {
            return Err("the properties are not there yet".into());
        }
        let value: Value = serde_json::from_str(value).unwrap_or_else(|_| json!(value));
        if let Some(name) = field.strip_prefix("pcell.") {
            if !dialog.pcell.contains_key(name) {
                return Err(format!("there is no PCell parameter '{name}'"));
            }
            dialog.pcell.insert(name.to_string(), value);
            return Ok(());
        }
        let edit = dialog.edit.as_ref().ok_or("nothing is selected")?;
        let mut json = serde_json::to_value(edit).map_err(|e| e.to_string())?;
        let mut at = &mut json;
        for part in field.split('.') {
            at = match part.parse::<usize>() {
                Ok(n) => at.get_mut(n),
                Err(_) => at.get_mut(part),
            }
            .ok_or(format!("the form has no field '{field}'"))?;
        }
        *at = value;
        dialog.edit = Some(serde_json::from_value(json).map_err(|e| format!("{field}: {e}"))?);
        Ok(())
    }

    /// The window, while it is open.
    pub(super) fn properties_ui(&mut self, ctx: &Context) {
        let Some(mut dialog) = self.properties.take() else {
            return;
        };
        if let Some(id) = dialog.asked
            && let Some(answer) = self.take_value(id)
        {
            dialog.asked = None;
            match answer.and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string())) {
                Ok(props) => {
                    dialog.take(props);
                    if std::mem::take(&mut dialog.quiet_if_empty) && dialog.props.objects.is_empty()
                    {
                        return;
                    }
                    if std::mem::take(&mut dialog.close_after) {
                        return;
                    }
                }
                Err(error) => {
                    dialog.close_after = false;
                    dialog.error = Some(error);
                }
            }
            dialog.seen = Some(self.selection_signature());
        }
        // Another selection: show it, unless the form holds changes.
        let seen = self.selection_signature();
        if dialog.asked.is_none() && dialog.seen.as_ref() != Some(&seen) && !dialog.is_edited() {
            dialog.seen = Some(seen);
            dialog.asked = Some(self.raw_call(method::PROPERTIES, json!({})));
        }

        let mut open = true;
        let mut apply = None;
        let window_id = Id::new(("klayout-properties", self.id.0));
        if let Some(change) = dialog.change.take()
            && let Some(object) = dialog.edit.as_ref()
        {
            let target = (object.index, object.key.clone());
            match change {
                ChangeCell::Library => self.open_library_picker_to_replace(target),
                ChangeCell::View => self.choose_view_to_replace(target),
            }
        }
        egui::Window::new("Properties")
            .id(window_id)
            .open(&mut open)
            .collapsible(true)
            .resizable(true)
            .default_width(320.0)
            .show(ctx, |ui| {
                apply = dialog_ui(ui, window_id, &mut dialog);
            });
        if let Some(close) = apply {
            self.properties = Some(dialog);
            if let Err(error) = self.apply_properties(close)
                && let Some(dialog) = &mut self.properties
            {
                dialog.error = Some(error);
            }
            return;
        }
        if open {
            self.properties = Some(dialog);
        }
    }
}

/// The content of the window; `Some(close)` when the form is to be applied.
fn dialog_ui(ui: &mut Ui, id: Id, dialog: &mut PropertiesDialog) -> Option<bool> {
    let count = dialog.props.objects.len();
    if count == 0 {
        if dialog.asked.is_some() {
            ui.spinner();
        } else {
            ui.weak("Nothing is selected");
        }
        return None;
    }
    let mut apply = None;
    if count > 1 || dialog.props.count > count {
        ui.horizontal(|ui| {
            if ui
                .add_enabled(dialog.current > 0, egui::Button::new("◀"))
                .clicked()
            {
                dialog.show(dialog.current - 1);
            }
            let more = if dialog.props.count > count {
                format!(" (of {} selected)", dialog.props.count)
            } else {
                String::new()
            };
            ui.label(format!("{} of {count}{more}", dialog.current + 1));
            if ui
                .add_enabled(dialog.current + 1 < count, egui::Button::new("▶"))
                .clicked()
            {
                dialog.show(dialog.current + 1);
            }
        });
        ui.separator();
    }
    let layers = dialog.props.layers.clone();
    let Some(edit) = &mut dialog.edit else {
        return None;
    };
    let editable = edit.editable;
    ui.horizontal(|ui| {
        ui.strong(kind_label(&edit.shape));
        ui.weak(format!("in {}", edit.parent));
    });
    if !edit.reason.is_empty() {
        ui.colored_label(ui.visuals().warn_fg_color, &edit.reason);
    }
    ScrollArea::vertical()
        .max_height(420.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.add_enabled_ui(editable, |ui| {
                object_ui(ui, id, edit, &mut dialog.pcell, &layers, &mut dialog.change);
            });
        });
    if let Some(error) = &dialog.error {
        ui.colored_label(ui.visuals().error_fg_color, error);
    }
    ui.separator();
    let edited = dialog.is_edited();
    let busy = dialog.asked.is_some();
    ui.horizontal(|ui| {
        let can = editable && edited && !busy;
        let ok = ui.add_enabled(can, egui::Button::new("OK"));
        if can {
            crate::guide::anchor(ui, "layout.properties.ok", &ok);
        }
        if ok.clicked() {
            apply = Some(true);
        }
        if ui.add_enabled(can, egui::Button::new("Apply")).clicked() {
            apply = Some(false);
        }
        if ui
            .add_enabled(edited && !busy, egui::Button::new("Reset"))
            .clicked()
        {
            dialog.show(dialog.current);
        }
        if busy {
            ui.spinner();
        }
    });
    apply
}

fn kind_label(shape: &Shape) -> &'static str {
    match shape {
        Shape::Box { .. } => "Box",
        Shape::Polygon { .. } => "Polygon",
        Shape::Path { .. } => "Path",
        Shape::Text { .. } => "Text",
        Shape::Instance(_) => "Instance",
        Shape::Other { .. } => "Shape",
    }
}

fn micron(ui: &mut Ui, value: &mut f64) {
    ui.add(
        DragValue::new(value)
            .speed(0.01)
            .max_decimals(6)
            .suffix(" µm"),
    );
}

fn point(ui: &mut Ui, p: &mut [f64; 2]) {
    ui.horizontal(|ui| {
        micron(ui, &mut p[0]);
        micron(ui, &mut p[1]);
    });
}

/// The fields of one object.
fn object_ui(
    ui: &mut Ui,
    id: Id,
    object: &mut Object,
    pcell: &mut Map<String, Value>,
    layers: &[String],
    change: &mut Option<ChangeCell>,
) {
    let grid = egui::Grid::new(id.with("grid"))
        .num_columns(2)
        .spacing([10.0, 4.0]);
    let mut pcell_form = None;
    grid.show(ui, |ui| {
        if let Some(layer) = &mut object.layer {
            ui.label("Layer");
            egui::ComboBox::from_id_salt(id.with("layer"))
                .selected_text(layer.as_str())
                .show_ui(ui, |ui| {
                    for l in layers {
                        ui.selectable_value(layer, l.clone(), l);
                    }
                });
            ui.end_row();
        }
        match &mut object.shape {
            Shape::Box { bbox } => {
                for (label, at) in [("Left", 0), ("Bottom", 1), ("Right", 2), ("Top", 3)] {
                    ui.label(label);
                    micron(ui, &mut bbox[at]);
                    ui.end_row();
                }
                ui.label("Size");
                ui.weak(format!(
                    "{:.4} × {:.4} µm",
                    bbox[2] - bbox[0],
                    bbox[3] - bbox[1]
                ));
                ui.end_row();
            }
            Shape::Polygon {
                points,
                holes,
                offset,
            } => {
                ui.label("Points");
                ui.label(format!(
                    "{}{}",
                    points.len(),
                    if *holes > 0 {
                        format!(", {holes} holes")
                    } else {
                        String::new()
                    }
                ));
                ui.end_row();
                ui.label("Move by");
                point(ui, offset);
                ui.end_row();
            }
            Shape::Path {
                width,
                points,
                begin_ext,
                end_ext,
                round,
            } => {
                ui.label("Width");
                micron(ui, width);
                ui.end_row();
                ui.label("Begin extension");
                micron(ui, begin_ext);
                ui.end_row();
                ui.label("End extension");
                micron(ui, end_ext);
                ui.end_row();
                ui.label("Ends");
                ui.horizontal(|ui| {
                    ui.checkbox(round, "Round");
                    if ui.small_button("Flush").clicked() {
                        (*begin_ext, *end_ext) = (0.0, 0.0);
                    }
                    if ui.small_button("Square").clicked() {
                        (*begin_ext, *end_ext) = (*width / 2.0, *width / 2.0);
                    }
                });
                ui.end_row();
                let mut remove = None;
                for (n, p) in points.iter_mut().enumerate() {
                    ui.label(format!("Point {}", n + 1));
                    ui.horizontal(|ui| {
                        point(ui, p);
                        if ui.small_button("−").on_hover_text("Remove").clicked() {
                            remove = Some(n);
                        }
                    });
                    ui.end_row();
                }
                if let Some(n) = remove.filter(|_| points.len() > 1) {
                    points.remove(n);
                }
                ui.label("");
                if ui.small_button("Add point").clicked()
                    && let Some(last) = points.last().copied()
                {
                    points.push([last[0] + 1.0, last[1]]);
                }
                ui.end_row();
            }
            Shape::Text {
                string,
                size,
                x,
                y,
                rotation,
                mirror,
                halign,
                valign,
            } => {
                ui.label("Text");
                ui.text_edit_singleline(string);
                ui.end_row();
                ui.label("Size");
                micron(ui, size);
                ui.end_row();
                ui.label("Position");
                ui.horizontal(|ui| {
                    micron(ui, x);
                    micron(ui, y);
                });
                ui.end_row();
                ui.label("Orientation");
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt(id.with("rot"))
                        .selected_text(format!("{}°", *rotation * 90))
                        .show_ui(ui, |ui| {
                            for r in 0..4 {
                                ui.selectable_value(rotation, r, format!("{}°", r * 90));
                            }
                        });
                    ui.checkbox(mirror, "Mirror");
                });
                ui.end_row();
                for (label, value, names) in [
                    ("Horizontal", halign, ["left", "centre", "right"]),
                    ("Vertical", valign, ["bottom", "centre", "top"]),
                ] {
                    ui.label(label);
                    let text = usize::try_from(*value)
                        .ok()
                        .and_then(|v| names.get(v))
                        .copied()
                        .unwrap_or("default");
                    egui::ComboBox::from_id_salt(id.with(label))
                        .selected_text(text)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(value, -1, "default");
                            for (n, name) in names.iter().enumerate() {
                                ui.selectable_value(value, n as i32, *name);
                            }
                        });
                    ui.end_row();
                }
            }
            Shape::Instance(inst) => {
                ui.label("Cell");
                ui.horizontal(|ui| {
                    ui.strong(&inst.cell);
                    if let Some(lib) = &inst.library {
                        ui.weak(format!("library {lib}"));
                    }
                    ui.menu_button("Change…", |ui| {
                        if ui
                            .button("Library Cell…")
                            .on_hover_text("A cell or PCell of KLayout's libraries")
                            .clicked()
                        {
                            *change = Some(ChangeCell::Library);
                            ui.close();
                        }
                        if ui
                            .button("Layout View…")
                            .on_hover_text("A layout view of the project")
                            .clicked()
                        {
                            *change = Some(ChangeCell::View);
                            ui.close();
                        }
                    })
                    .response
                    .on_hover_text("Place another cell here, keeping position and rotation");
                });
                ui.end_row();
                if let Some(label) = &inst.external {
                    ui.label("View");
                    ui.weak(label).on_hover_text(
                        "A cell of another view: its place can change here, its content \
                         only in its own view",
                    );
                    ui.end_row();
                } else if inst.missing {
                    ui.label("View");
                    ui.colored_label(ui.visuals().error_fg_color, "cannot be found");
                    ui.end_row();
                }
                ui.label("Position");
                ui.horizontal(|ui| {
                    micron(ui, &mut inst.x);
                    micron(ui, &mut inst.y);
                });
                ui.end_row();
                ui.label("Rotation");
                ui.horizontal(|ui| {
                    ui.add(
                        DragValue::new(&mut inst.angle)
                            .speed(1.0)
                            .range(-360.0..=360.0)
                            .suffix("°"),
                    );
                    ui.checkbox(&mut inst.mirror, "Mirror");
                });
                ui.end_row();
                ui.label("Magnification");
                ui.add(
                    DragValue::new(&mut inst.mag)
                        .speed(0.01)
                        .range(1e-6..=1e6)
                        .max_decimals(6),
                );
                ui.end_row();
                ui.label("Array");
                let mut is_array = inst.array.is_some();
                if ui.checkbox(&mut is_array, "").changed() {
                    inst.array = is_array.then_some(InstArray {
                        a: [1.0, 0.0],
                        b: [0.0, 1.0],
                        na: 1,
                        nb: 1,
                    });
                }
                ui.end_row();
                if let Some(array) = &mut inst.array {
                    ui.label("Columns, step");
                    ui.horizontal(|ui| {
                        ui.add(DragValue::new(&mut array.na).range(1..=100_000));
                        point(ui, &mut array.a);
                    });
                    ui.end_row();
                    ui.label("Rows, step");
                    ui.horizontal(|ui| {
                        ui.add(DragValue::new(&mut array.nb).range(1..=100_000));
                        point(ui, &mut array.b);
                    });
                    ui.end_row();
                }
                pcell_form = inst.pcell.clone();
            }
            Shape::Other { text } => {
                ui.label("Shape");
                ui.weak(text.as_str());
                ui.end_row();
            }
        }
    });
    if let Some(form) = pcell_form {
        ui.separator();
        ui.strong(format!("PCell {} ({})", form.name, form.library));
        pcell::form_ui(ui, id.with("pcell"), &form, pcell, layers, true);
    }
}
