//! The layout tab's pins (docs/LINKING.md §9): the Pins panel, Draw Pin, Pin from
//! Selection.
//!
//! A pin is a CDePT object kept in the layout's sidecar. The helper (`pins.py`) holds
//! the pins of the open layout as shapes carrying the pin's name (on the technology's
//! pin and label layers, or CDePT's own layer when there are none), so they move, undo
//! and save like everything else; the save's report carries them to the sidecar.

use std::collections::BTreeMap;

use egui::{Color32, Ui};
use libcdept::klayout::protocol::NoticeLevel;
use libcdept::layout::PinShape;
use serde_json::{Value, json};

use super::LayoutView;

/// A pin as the helper lists it.
#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
pub struct PinInfo {
    #[serde(default)]
    pub direction: String,
    #[serde(default)]
    pub net: Option<String>,
    #[serde(default)]
    pub shapes: Vec<PinShape>,
    #[serde(default)]
    pub label: Option<[f64; 2]>,
    /// Shapes carrying the name (a copied pin makes two).
    #[serde(default)]
    pub count: usize,
    /// Some shape is CDePT's only (no pin layer in this technology).
    #[serde(default)]
    pub overlay: bool,
    /// The label is on one of the shapes.
    #[serde(default)]
    pub label_on: bool,
}

#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
struct Listing {
    #[serde(default)]
    pins: BTreeMap<String, PinInfo>,
    /// The technology describes how pins are drawn.
    #[serde(default)]
    rules: bool,
    #[serde(default)]
    generate: Option<String>,
}

pub const DIRECTIONS: [&str; 3] = ["input", "output", "inout"];

/// The pins of a layout tab.
#[derive(Default)]
pub(super) struct PinsState {
    pub open: bool,
    listing: Listing,
    asked: Option<u64>,
    /// (helper epoch, revision) of the listing.
    seen: Option<(u64, u64)>,
    /// The form for a new pin.
    name: String,
    direction: String,
    renaming: Option<(String, String)>,
    /// The colour sent for CDePT's pins.
    sent_color: Option<(u64, String)>,
    /// A call whose answer is a listing (add, remove, …): its outcome is shown.
    acting: Option<u64>,
}

impl PinsState {
    pub fn forget_questions(&mut self) {
        self.asked = None;
        self.acting = None;
        self.seen = None;
        self.sent_color = None;
    }
}

enum Act {
    Show(String),
    Remove(String),
    Rename(String, String),
    Direction(String, String),
    LabelMiddle(String),
    Draw,
    FromSelection,
    Close,
}

impl LayoutView {
    /// Ask the helper for the pins after each change; keep the style; every frame.
    pub(super) fn poll_pins(&mut self) {
        if !self.ready() {
            return;
        }
        let epoch = self.epoch;
        // The schematic's pins, for the statuses.
        if self.pins.open && self.link_unread() {
            self.refresh_link(None);
        }
        if self.pins.seen != Some((epoch, self.revision)) && self.pins.asked.is_none() {
            self.pins.seen = Some((epoch, self.revision));
            self.pins.asked = Some(self.raw_call("pins_list", json!({})));
        }
        if let Some(id) = self.pins.asked
            && let Some(answer) = self.take_reply(id)
        {
            self.pins.asked = None;
            if let Ok(listing) = answer
                .ok()
                .and_then(|t| serde_json::from_str::<Value>(&t).ok())
                .map(serde_json::from_value::<Listing>)
                .transpose()
                .map(Option::unwrap_or_default)
            {
                self.pins.listing = listing;
            }
        }
        if let Some(id) = self.pins.acting
            && let Some(answer) = self.take_reply(id)
        {
            self.pins.acting = None;
            match answer {
                Ok(text) => {
                    if let Some(listing) = serde_json::from_str::<Value>(&text)
                        .ok()
                        .and_then(|v| serde_json::from_value::<Listing>(v).ok())
                    {
                        self.pins.listing = listing;
                    }
                    self.pins.seen = None;
                }
                Err(error) => self.notify(NoticeLevel::Error, format!("Pins: {error}")),
            }
        }
        let color = self.nets.style.pin.clone();
        if self.pins.sent_color.as_ref() != Some(&(epoch, color.clone())) {
            self.pins.sent_color = Some((epoch, color.clone()));
            self.quiet("pins_style", json!({"color": color}));
        }
    }

    /// The status of a pin, for people and for tests: `placed`, `not placed` (a pin of
    /// the schematic the layout does not have), `not in the schematic`, `twice`,
    /// `label off the pin`, `other direction`, or what the net check says.
    pub fn pin_status(&mut self, name: &str) -> String {
        if self.link_unread() {
            self.refresh_link(None);
        }
        let wanted = self.link_schematic_pins();
        let in_schematic = wanted.iter().find(|p| p.name == name);
        let Some(pin) = self.pins.listing.pins.get(name) else {
            return if in_schematic.is_some() {
                "not placed".into()
            } else {
                "absent".into()
            };
        };
        if pin.count > 1 {
            return "twice".into();
        }
        if !wanted.is_empty() && in_schematic.is_none() {
            return "not in the schematic".into();
        }
        if let Some(p) = in_schematic
            && libcdept::link::pins::direction(&p.direction)
                != libcdept::link::pins::direction(&pin.direction)
        {
            return "other direction".into();
        }
        if !pin.label_on {
            return "label off the pin".into();
        }
        match self.net_pin_status(name) {
            Some(status) if status != "ok" => status,
            _ => "placed".into(),
        }
    }

    /// Add pin `name` from the selected shapes (each on a conductor or a pin layer).
    pub fn pin_from_selection(&mut self, name: &str, direction: &str) {
        let net = name.to_string();
        self.pins.acting = Some(self.raw_call(
            "pins_add",
            json!({"name": name, "direction": direction, "net": net, "from_selection": true}),
        ));
    }

    /// Add pin `name` with a box on conductor `layer` (µm), as the form does.
    pub fn pin_add(&mut self, name: &str, direction: &str, layer: &str, b: [f64; 4]) {
        self.pins.acting = Some(self.raw_call(
            "pins_add",
            json!({"name": name, "direction": direction, "net": name, "layer": layer, "box": b}),
        ));
    }

    /// Draw Pin: the next box drawn on the current layer becomes pin `name`.
    pub fn pin_draw(&mut self, name: &str, direction: &str) {
        self.pins.acting = Some(self.raw_call(
            "pins_tool",
            json!({"name": name, "direction": direction, "net": name}),
        ));
    }

    pub fn pin_remove(&mut self, name: &str) {
        self.pins.acting = Some(self.raw_call("pins_remove", json!({"name": name})));
    }

    pub fn pin_rename(&mut self, name: &str, new: &str) {
        self.pins.acting = Some(self.raw_call("pins_rename", json!({"name": name, "new": new})));
    }

    /// Select and show the pin's shapes.
    pub fn pin_show(&mut self, name: &str) {
        self.quiet("pins_select", json!({"names": [name], "zoom": true}));
    }

    /// Show or hide the Pins panel.
    pub fn show_pins_panel(&mut self, on: bool) {
        self.pins.open = on;
    }

    /// A pin question or change is under way (for the test driver).
    pub fn pins_busy(&self) -> bool {
        self.pins.asked.is_some()
            || self.pins.acting.is_some()
            || self.pins.seen != Some((self.epoch, self.revision))
    }

    /// The Layout menu entry.
    pub(super) fn pins_menu(&mut self, ui: &mut Ui) {
        if ui
            .checkbox(&mut self.pins.open, "Pins Panel")
            .on_hover_text("The layout's pins: place, rename, delete, check")
            .changed()
        {
            ui.close();
        }
    }

    pub(super) fn pins_panel(&mut self, ui: &mut Ui, editable: bool) {
        if !self.pins.open {
            return;
        }
        let mut acts = Vec::new();
        egui::Panel::bottom(egui::Id::new(("klayout-pins", self.id)))
            .resizable(true)
            .default_size(180.0)
            .size_range(80.0..=600.0)
            .show(ui, |ui| self.pins_contents(ui, editable, &mut acts));
        for act in acts {
            match act {
                Act::Show(name) => self.pin_show(&name),
                Act::Remove(name) => self.pin_remove(&name),
                Act::Rename(name, new) => self.pin_rename(&name, &new),
                Act::Direction(name, direction) => {
                    self.pins.acting = Some(
                        self.raw_call("pins_update", json!({"name": name, "direction": direction})),
                    );
                }
                Act::LabelMiddle(name) => {
                    self.pins.acting = Some(
                        self.raw_call("pins_update", json!({"name": name, "label": "middle"})),
                    );
                }
                Act::Draw => {
                    let (name, direction) = self.pin_form();
                    self.pin_draw(&name, &direction);
                }
                Act::FromSelection => {
                    let (name, direction) = self.pin_form();
                    self.pin_from_selection(&name, &direction);
                }
                Act::Close => self.pins.open = false,
            }
        }
    }

    fn pin_form(&self) -> (String, String) {
        let direction = if self.pins.direction.is_empty() {
            "inout".to_string()
        } else {
            self.pins.direction.clone()
        };
        (self.pins.name.trim().to_string(), direction)
    }

    fn pins_contents(&mut self, ui: &mut Ui, editable: bool, acts: &mut Vec<Act>) {
        let wanted = self.link_schematic_pins();
        let unplaced: Vec<(String, String)> = wanted
            .iter()
            .filter(|p| !self.pins.listing.pins.contains_key(&p.name))
            .map(|p| {
                (
                    p.name.clone(),
                    libcdept::link::pins::direction(&p.direction).to_string(),
                )
            })
            .collect();
        ui.horizontal(|ui| {
            ui.strong("Pins");
            ui.weak(format!(
                "{} placed, {} of the schematic not placed",
                self.pins.listing.pins.len(),
                unplaced.len()
            ));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("✕").on_hover_text("Close").clicked() {
                    acts.push(Act::Close);
                }
            });
        });
        if !self.pins.listing.rules {
            ui.weak(
                "This technology describes no pin layers: pins are CDePT's only (kept in the \
                 layout's sidecar, drawn over the layout).",
            );
        }
        ui.add_enabled_ui(editable, |ui| {
            ui.horizontal(|ui| {
                ui.label("New pin");
                egui::ComboBox::from_id_salt(("klayout-pin-name", self.id))
                    .selected_text(if self.pins.name.is_empty() {
                        "name…".to_string()
                    } else {
                        self.pins.name.clone()
                    })
                    .show_ui(ui, |ui| {
                        for (name, direction) in &unplaced {
                            if ui.selectable_label(self.pins.name == *name, name).clicked() {
                                self.pins.name = name.clone();
                                self.pins.direction = direction.clone();
                            }
                        }
                    });
                ui.add(egui::TextEdit::singleline(&mut self.pins.name).desired_width(80.0));
                egui::ComboBox::from_id_salt(("klayout-pin-direction", self.id))
                    .selected_text(if self.pins.direction.is_empty() {
                        "inout"
                    } else {
                        self.pins.direction.as_str()
                    })
                    .show_ui(ui, |ui| {
                        for d in DIRECTIONS {
                            ui.selectable_value(&mut self.pins.direction, d.to_string(), d);
                        }
                    });
                let named = !self.pins.name.trim().is_empty();
                if ui
                    .add_enabled(named, egui::Button::new("Draw"))
                    .on_hover_text("Draw the pin's box on the current layer (a conductor)")
                    .clicked()
                {
                    acts.push(Act::Draw);
                }
                if ui
                    .add_enabled(named, egui::Button::new("From Selection"))
                    .on_hover_text("The selected shapes become the pin's")
                    .clicked()
                {
                    acts.push(Act::FromSelection);
                }
            });
        });
        ui.separator();
        let names: Vec<String> = self.pins.listing.pins.keys().cloned().collect();
        let statuses: Vec<String> = names.iter().map(|n| self.pin_status(n)).collect();
        egui::ScrollArea::vertical()
            .auto_shrink([false, true])
            .show(ui, |ui| {
                egui::Grid::new(("klayout-pins-grid", self.id))
                    .striped(true)
                    .num_columns(5)
                    .show(ui, |ui| {
                        for (name, status) in names.iter().zip(&statuses) {
                            let pin = &self.pins.listing.pins[name];
                            if ui.link(name).on_hover_text("Select and show").clicked() {
                                acts.push(Act::Show(name.clone()));
                            }
                            let mut direction = pin.direction.clone();
                            ui.add_enabled_ui(editable, |ui| {
                                egui::ComboBox::from_id_salt(("klayout-pin-dir", self.id, name))
                                    .selected_text(direction.clone())
                                    .show_ui(ui, |ui| {
                                        for d in DIRECTIONS {
                                            ui.selectable_value(&mut direction, d.to_string(), d);
                                        }
                                    });
                            });
                            if direction != pin.direction {
                                acts.push(Act::Direction(name.clone(), direction));
                            }
                            let layers: Vec<&str> =
                                pin.shapes.iter().map(|s| s.layer.as_str()).collect();
                            let mut where_ = layers.join(", ");
                            if pin.overlay {
                                where_.push_str(" (CDePT)");
                            }
                            ui.label(where_);
                            let color = if status == "placed" {
                                Color32::from_rgb(60, 160, 60)
                            } else {
                                Color32::from_rgb(220, 140, 0)
                            };
                            ui.colored_label(color, status);
                            ui.horizontal(|ui| {
                                ui.add_enabled_ui(editable, |ui| {
                                    if status == "label off the pin"
                                        && ui.small_button("Label to Pin").clicked()
                                    {
                                        acts.push(Act::LabelMiddle(name.clone()));
                                    }
                                    match &mut self.pins.renaming {
                                        Some((old, new)) if old == name => {
                                            let edit = ui.add(
                                                egui::TextEdit::singleline(new).desired_width(70.0),
                                            );
                                            if edit.lost_focus()
                                                && ui.input(|i| i.key_pressed(egui::Key::Enter))
                                            {
                                                acts.push(Act::Rename(old.clone(), new.clone()));
                                                self.pins.renaming = None;
                                            } else if ui.small_button("Cancel").clicked() {
                                                self.pins.renaming = None;
                                            }
                                        }
                                        _ => {
                                            if ui.small_button("Rename").clicked() {
                                                self.pins.renaming =
                                                    Some((name.clone(), name.clone()));
                                            }
                                        }
                                    }
                                    if ui.small_button("Delete").clicked() {
                                        acts.push(Act::Remove(name.clone()));
                                    }
                                });
                            });
                            ui.end_row();
                        }
                        for (name, direction) in &unplaced {
                            ui.weak(name);
                            ui.weak(direction);
                            ui.label("");
                            ui.colored_label(Color32::from_rgb(220, 140, 0), "not placed");
                            ui.add_enabled_ui(editable, |ui| {
                                if ui
                                    .small_button("Place…")
                                    .on_hover_text("Fill in the form above with this pin")
                                    .clicked()
                                {
                                    self.pins.name = name.clone();
                                    self.pins.direction = direction.clone();
                                }
                            });
                            ui.end_row();
                        }
                    });
            });
    }
}
