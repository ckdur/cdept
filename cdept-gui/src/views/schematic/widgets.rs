//! Draws the widgets of a mirrored Tk window with egui widgets.
//!
//! xschem's dialogs are Tk widgets recorded by the helper (`protocol::Widget`). This
//! is a generic renderer for them: containers follow the `pack` and `grid` placement
//! roughly, the usual widget classes map to their egui counterparts, and unknown
//! classes show their children. What the user does comes back as `UiEvent`s.

use std::collections::{BTreeMap, HashMap, VecDeque};

use egui::load::SizedTexture;
use egui::{Key, RichText, ScrollArea, TextEdit, TextStyle, TextWrapMode, Ui};
use libcdept::xschem::protocol::{UiEvent, Widget};

use super::clipboard::{self, Selection};
use super::keys::tk_key_name;
use super::mirror::{Images, Panes};

/// Widest a text field grows to when Tk asks it to fill its parent, in points.
const MAX_FILL_WIDTH: f32 = 520.0;

/// Text being edited in a mirrored widget.
///
/// Every update from the helper replaces the whole window, usually echoing what was
/// just typed, a little late. The local text therefore wins until the helper reports
/// something that was not typed here.
#[derive(Default)]
struct Edit {
    text: String,
    /// The text the helper last reported.
    remote: String,
    /// Texts sent and not yet echoed, oldest first.
    pending: VecDeque<String>,
}

/// Local edit state of all mirrored text widgets, by Tk path.
#[derive(Default)]
pub struct Edits {
    texts: HashMap<String, Edit>,
}

fn holds_text(class: &str) -> bool {
    matches!(
        class,
        "Entry" | "TEntry" | "Text" | "Spinbox" | "TSpinbox" | "TCombobox" | "Scale" | "TScale"
    )
}

impl Edits {
    /// Take in a snapshot of `widget` and its children.
    pub fn sync(&mut self, widget: &Widget) {
        if holds_text(&widget.class) {
            self.sync_text(&widget.path, &widget.text);
        }
        for child in &widget.children {
            self.sync(child);
        }
    }

    /// Take in the text the helper reports for the field at `path`.
    pub fn sync_text(&mut self, path: &str, text: &str) {
        let edit = self.texts.entry(path.to_string()).or_insert_with(|| Edit {
            text: text.to_string(),
            remote: text.to_string(),
            pending: VecDeque::new(),
        });
        if text != edit.remote {
            edit.remote = text.to_string();
            match edit.pending.iter().position(|sent| sent == text) {
                // An echo of something typed here: later edits stay.
                Some(at) => {
                    edit.pending.drain(..=at);
                }
                // xschem's code changed the text.
                None => {
                    edit.text = text.to_string();
                    edit.pending.clear();
                }
            }
        }
    }

    /// The text in the field at `path`, for editing in place.
    pub fn text_mut(&mut self, path: &str) -> &mut String {
        &mut self.texts.entry(path.to_string()).or_default().text
    }

    /// The text of the field at `path` was edited: the event that reports it.
    pub fn changed(&mut self, path: &str) -> UiEvent {
        Self::sent(self.texts.entry(path.to_string()).or_default(), path)
    }

    /// Forget the widgets of the window at `path`.
    pub fn forget(&mut self, path: &str) {
        let inside = format!("{path}.");
        self.texts
            .retain(|key, _| key != path && !key.starts_with(&inside));
    }

    /// Replace the text of the widget at `path`, as if typed. Returns the event to
    /// send, or `None` when the text is the same.
    pub fn set(&mut self, path: &str, text: &str) -> Option<UiEvent> {
        let edit = self.texts.entry(path.to_string()).or_default();
        if edit.text == text {
            return None;
        }
        edit.text = text.to_string();
        Some(Self::sent(edit, path))
    }

    fn sent(edit: &mut Edit, path: &str) -> UiEvent {
        edit.pending.push_back(edit.text.clone());
        UiEvent::Text {
            path: path.to_string(),
            text: edit.text.clone(),
        }
    }

    #[cfg(test)]
    pub fn text(&self, path: &str) -> Option<&str> {
        self.texts.get(path).map(|edit| edit.text.as_str())
    }
}

/// What drawing a window needs besides the widgets.
pub struct Draw<'a> {
    pub edits: &'a mut Edits,
    pub events: &'a mut Vec<UiEvent>,
    pub images: &'a mut Images,
    /// Frames xschem draws pictures into (its file browser's preview).
    pub panes: &'a mut Panes,
    /// The widget to give the keyboard focus to in this frame.
    pub focus: Option<&'a str>,
}

fn option<'a>(widget: &'a Widget, name: &str) -> Option<&'a str> {
    widget
        .options
        .get(name)
        .map(String::as_str)
        .filter(|v| !v.is_empty())
}

fn number(widget: &Widget, name: &str) -> Option<f32> {
    option(widget, name)?.parse().ok()
}

fn placement<'a>(widget: &'a Widget, name: &str) -> &'a str {
    widget.layout.get(name).map_or("", String::as_str)
}

/// Whether Tk stretches the widget over the width of its parent: it is packed above or
/// below its siblings and asked to fill. (Beside its siblings it shares the width with
/// them; there it keeps the width it asked for.)
fn fills_width(widget: &Widget) -> bool {
    placement(widget, "manager") == "pack"
        && matches!(placement(widget, "side"), "" | "top" | "bottom")
        && matches!(placement(widget, "fill"), "x" | "both")
}

/// Width of `chars` characters of the body font, roughly.
fn chars_width(ui: &Ui, chars: f32) -> f32 {
    chars * ui.text_style_height(&TextStyle::Body) * 0.55
}

/// Width of a text field. Tk's `-width` is in characters; 0 means as wide as the text.
fn field_width(ui: &Ui, widget: &Widget, default_chars: f32) -> f32 {
    if fills_width(widget) {
        return ui.available_width().min(MAX_FILL_WIDTH);
    }
    let chars = match number(widget, "width") {
        Some(chars) if chars > 0.0 => chars,
        Some(_) => (widget.text.chars().count() as f32 + 1.0).max(4.0),
        None => default_chars,
    };
    chars_width(ui, chars).min(MAX_FILL_WIDTH)
}

/// Draw `widget` and everything inside it.
pub fn draw(ui: &mut Ui, widget: &Widget, cx: &mut Draw<'_>) {
    if cx.panes.has(&widget.path) {
        let size = pane_size(ui, widget);
        ui.push_id(&widget.path, |ui| {
            cx.panes.show(ui, &widget.path, size, cx.events)
        });
        return;
    }
    let drawn = ui.push_id(&widget.path, |ui| match widget.class.as_str() {
        "Label" | "TLabel" => label(ui, widget, cx, TextWrapMode::Extend),
        "Message" => label(ui, widget, cx, TextWrapMode::Wrap),
        "Button" | "TButton" => button(ui, widget, cx),
        "Checkbutton" | "TCheckbutton" => {
            let mut checked = widget.checked;
            let check = egui::Checkbox::new(&mut checked, widget.text.as_str());
            if ui.add_enabled(widget.enabled, check).changed() {
                invoke(widget, cx);
            }
        }
        "Radiobutton" | "TRadiobutton" => {
            let radio = egui::RadioButton::new(widget.checked, widget.text.as_str());
            let mut clicked = ui.add_enabled(widget.enabled, radio).clicked();
            // A radio button without text whose background is what it chooses: the
            // colour choosers of xschem's graph dialogs.
            if widget.text.is_empty()
                && let Some(colour) =
                    option(widget, "background").and_then(|c| egui::Color32::from_hex(c).ok())
            {
                let size = egui::vec2(14.0, 14.0);
                let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
                ui.painter().rect_filled(rect, 2.0, colour);
                if widget.checked {
                    let stroke = ui.visuals().selection.stroke;
                    ui.painter()
                        .rect_stroke(rect, 2.0, stroke, egui::StrokeKind::Outside);
                }
                clicked |= widget.enabled && response.clicked();
            }
            if clicked {
                invoke(widget, cx);
            }
        }
        "Entry" | "TEntry" | "Spinbox" | "TSpinbox" => entry(ui, widget, cx),
        "TCombobox" => combobox(ui, widget, cx),
        "Text" => text(ui, widget, cx),
        "Listbox" => listbox(ui, widget, cx),
        "Scale" | "TScale" => scale(ui, widget, cx),
        "Scrollbar" | "TScrollbar" | "Menu" => {}
        "Labelframe" | "TLabelframe" => {
            ui.group(|ui| {
                if !widget.text.is_empty() {
                    ui.label(RichText::new(&widget.text).strong());
                }
                children(ui, widget, cx);
            });
        }
        _ => {
            // A frame without content is a spacer or a rule.
            if widget.children.is_empty() {
                if number(widget, "height").is_some_and(|h| h <= 4.0) {
                    ui.separator();
                }
            } else {
                children(ui, widget, cx);
            }
        }
    });
    if let Some(tip) = option(widget, "tooltip") {
        drawn.response.on_hover_text(tip);
    }
}

/// Draw the children of a container, placed as their geometry manager says.
pub fn children(ui: &mut Ui, parent: &Widget, cx: &mut Draw<'_>) {
    // Not placed means not shown.
    let placed = parent.children.iter().filter(|c| !c.layout.is_empty());
    let (gridded, packed): (Vec<&Widget>, Vec<&Widget>) =
        placed.partition(|c| placement(c, "manager") == "grid");
    ui.vertical(|ui| {
        if !gridded.is_empty() {
            grid(ui, parent, &gridded, cx);
        }
        pack(ui, &packed, cx);
    });
}

fn grid(ui: &mut Ui, parent: &Widget, cells: &[&Widget], cx: &mut Draw<'_>) {
    let position =
        |cell: &Widget, name: &str| -> u32 { placement(cell, name).parse().unwrap_or(0) };
    let mut rows: BTreeMap<u32, Vec<(u32, &Widget)>> = BTreeMap::new();
    for cell in cells {
        rows.entry(position(cell, "row"))
            .or_default()
            .push((position(cell, "column"), cell));
    }
    // A row holding one widget that spans columns (a list or text with its scroll
    // bars) is drawn on its own: in the grid it would widen its first column and push
    // the other rows apart. The rows between such rows make one grid each.
    let wide = |cells: &Vec<(u32, &Widget)>| {
        let mut shown = cells
            .iter()
            .filter(|(_, c)| !matches!(c.class.as_str(), "Scrollbar" | "TScrollbar"));
        matches!((shown.next(), shown.next()), (Some((_, c)), None)
            if placement(c, "columnspan").parse().unwrap_or(1) > 1)
    };
    let mut rows: Vec<Vec<(u32, &Widget)>> = rows.into_values().collect();
    for cells in &mut rows {
        cells.sort_by_key(|(column, _)| *column);
    }
    for (segment, run) in rows.chunk_by(|a, b| wide(a) == wide(b)).enumerate() {
        if wide(&run[0]) {
            for cells in run {
                for (_, cell) in cells {
                    draw(ui, cell, cx);
                }
            }
        } else {
            egui::Grid::new(("grid", &parent.path, segment)).show(ui, |ui| {
                for cells in run {
                    let mut at = 0;
                    for (column, cell) in cells {
                        // A widget spanning columns sits in its first one.
                        while at < *column {
                            ui.label("");
                            at += 1;
                        }
                        draw(ui, cell, cx);
                        at += 1;
                    }
                    ui.end_row();
                }
            });
        }
    }
}

/// Tk's packer: each widget takes one side of what the ones before it left over.
fn pack(ui: &mut Ui, widgets: &[&Widget], cx: &mut Draw<'_>) {
    let Some(first) = widgets.first() else {
        return;
    };
    let side = |w: &Widget| {
        match placement(w, "side") {
            "" => "top",
            other => other,
        }
        .to_string()
    };
    let first_side = side(first);
    let run = widgets.iter().take_while(|w| side(w) == first_side).count();
    let (now, rest) = widgets.split_at(run);
    match first_side.as_str() {
        "left" => {
            ui.horizontal_top(|ui| {
                for widget in now {
                    draw(ui, widget, cx);
                }
                ui.vertical(|ui| pack(ui, rest, cx));
            });
        }
        "right" => {
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| pack(ui, rest, cx));
                for widget in now.iter().rev() {
                    draw(ui, widget, cx);
                }
            });
        }
        "bottom" => {
            pack(ui, rest, cx);
            for widget in now.iter().rev() {
                draw(ui, widget, cx);
            }
        }
        _ => {
            for widget in now {
                draw(ui, widget, cx);
            }
            pack(ui, rest, cx);
        }
    }
}

fn invoke(widget: &Widget, cx: &mut Draw<'_>) {
    cx.events.push(UiEvent::Invoke {
        path: widget.path.clone(),
    });
}

fn label(ui: &mut Ui, widget: &Widget, cx: &mut Draw<'_>, wrap: TextWrapMode) {
    if widget.text.is_empty()
        && let Some(texture) =
            option(widget, "image").and_then(|name| cx.images.texture(ui.ctx(), name))
    {
        ui.image(SizedTexture::from_handle(&texture));
        return;
    }
    ui.add(egui::Label::new(widget.text.as_str()).wrap_mode(wrap));
}

fn button(ui: &mut Ui, widget: &Widget, cx: &mut Draw<'_>) {
    let picture = option(widget, "image").and_then(|name| cx.images.texture(ui.ctx(), name));
    let sized = picture.as_ref().map(SizedTexture::from_handle);
    let button = match (sized, widget.text.is_empty()) {
        (Some(picture), true) => egui::Button::image(picture),
        (Some(picture), false) => egui::Button::image_and_text(picture, widget.text.as_str()),
        (None, _) => egui::Button::new(widget.text.as_str()),
    };
    let response = ui.add_enabled(widget.enabled, button);
    if cx.focus == Some(widget.path.as_str()) {
        response.request_focus();
    }
    if response.clicked() {
        invoke(widget, cx);
    }
}

/// Report the keys with a meaning beyond editing that were pressed in a text field,
/// as Tk names them: `Return`, `Shift-Return`, `Control-Return`, `Escape`, …
fn field_keys(
    ui: &Ui,
    response: &egui::Response,
    widget: &Widget,
    multiline: bool,
    cx: &mut Draw<'_>,
) {
    if response.gained_focus() {
        cx.events.push(UiEvent::Focus {
            path: widget.path.clone(),
        });
    }
    // Escape and Tab take the focus away from an egui text field, and so does Return
    // from a one-line field; in a text of several lines Return starts a new line.
    let mut keys = Vec::new();
    if response.lost_focus() {
        keys.extend([Key::Escape, Key::Tab]);
        if !multiline {
            keys.push(Key::Enter);
        }
    } else if response.has_focus() {
        if multiline {
            keys.push(Key::Enter);
        } else {
            keys.extend([Key::ArrowUp, Key::ArrowDown]);
        }
    }
    let modifiers = ui.input(|i| i.modifiers);
    for key in keys {
        if ui.input(|i| i.key_pressed(key))
            && let Some(name) = tk_key_name(key)
        {
            let mut key = String::new();
            if modifiers.ctrl || modifiers.command {
                key.push_str("Control-");
            }
            if modifiers.shift {
                key.push_str("Shift-");
            }
            key.push_str(name);
            cx.events.push(UiEvent::Key {
                path: widget.path.clone(),
                key,
            });
        }
    }
}

/// Size to show a picture pane at, in points: as wide as there is room for (it grows
/// and shrinks with the dialog), as high as Tk's `-height` asks (else in proportion).
fn pane_size(ui: &Ui, widget: &Widget) -> egui::Vec2 {
    let width = ui.available_width().clamp(200.0, 900.0);
    let asked = option(widget, "height").map_or(0.0, |h| {
        let (number, unit) = h.split_at(h.trim_end_matches(['c', 'm', 'i', 'p']).len());
        let n: f32 = number.parse().unwrap_or(0.0);
        match unit {
            "c" => n * 37.8,
            "m" => n * 3.78,
            "i" => n * 96.0,
            "p" => n * 96.0 / 72.0,
            _ => n,
        }
    });
    let height = if asked > 0.0 { asked } else { width * 0.6 };
    egui::Vec2::new(width, height.clamp(100.0, 600.0))
}

/// Tk pastes the primary selection (the text last selected anywhere) into a text field
/// with the middle button; egui does not know that selection. Inserted at the cursor.
fn paste_primary(ui: &Ui, response: &mut egui::Response, text: &mut String, multiline: bool) {
    if !response.middle_clicked() || !response.sense.senses_click() {
        return;
    }
    let Some(mut pasted) = clipboard::read(Selection::Primary) else {
        return;
    };
    if !multiline {
        pasted = pasted.replace(['\r', '\n'], " ");
    }
    let mut state = TextEdit::load_state(ui.ctx(), response.id).unwrap_or_default();
    let chars = text.chars().count();
    let at = state
        .cursor
        .char_range()
        .map_or(chars, |range| range.primary.index.0.min(chars));
    let byte = text.char_indices().nth(at).map_or(text.len(), |(i, _)| i);
    text.insert_str(byte, &pasted);
    let after = egui::text::CCursor::new(at + pasted.chars().count());
    state
        .cursor
        .set_char_range(Some(egui::text::CCursorRange::one(after)));
    state.store(ui.ctx(), response.id);
    response.request_focus();
    response.mark_changed();
    // the field was drawn before the text changed
    ui.ctx().request_repaint();
}

/// A text field bound to the local edit state. Sends the text when it changed.
fn edit_field(
    ui: &mut Ui,
    widget: &Widget,
    cx: &mut Draw<'_>,
    field: impl FnOnce(&mut Ui, &mut String) -> egui::Response,
) -> egui::Response {
    let edit = cx.edits.texts.entry(widget.path.clone()).or_default();
    let mut response = field(ui, &mut edit.text);
    paste_primary(ui, &mut response, &mut edit.text, widget.class == "Text");
    if response.changed() {
        let event = Edits::sent(edit, &widget.path);
        cx.events.push(event);
    }
    if cx.focus == Some(widget.path.as_str()) {
        response.request_focus();
    }
    response
}

fn entry(ui: &mut Ui, widget: &Widget, cx: &mut Draw<'_>) {
    let width = field_width(ui, widget, 20.0);
    let password = option(widget, "show").is_some();
    let enabled = widget.enabled;
    // Read-only text can still be selected and copied.
    let editable = option(widget, "state") != Some("readonly");
    let response = edit_field(ui, widget, cx, |ui, text| {
        // egui keeps a field within the space it is given, which in a grid cell is
        // the column as wide as it was: let the column grow to the width asked for.
        ui.set_min_width(width);
        let field = TextEdit::singleline(text)
            .desired_width(width)
            .password(password)
            .interactive(editable);
        ui.add_enabled(enabled, field)
    });
    field_keys(ui, &response, widget, false, cx);
}

fn combobox(ui: &mut Ui, widget: &Widget, cx: &mut Draw<'_>) {
    ui.horizontal(|ui| {
        let width = field_width(ui, widget, 20.0);
        let enabled = widget.enabled && option(widget, "state") != Some("readonly");
        let response = edit_field(ui, widget, cx, |ui, text| {
            ui.add_enabled(enabled, TextEdit::singleline(text).desired_width(width))
        });
        field_keys(ui, &response, widget, false, cx);
        let mut chosen = None;
        ui.add_enabled_ui(widget.enabled && !widget.items.is_empty(), |ui| {
            ui.menu_button("⏷", |ui| {
                for item in &widget.items {
                    if ui.button(item).clicked() {
                        chosen = Some(item.clone());
                        ui.close();
                    }
                }
            });
        });
        if let Some(item) = chosen
            && let Some(event) = cx.edits.set(&widget.path, &item)
        {
            cx.events.push(event);
        }
    });
}

fn text(ui: &mut Ui, widget: &Widget, cx: &mut Draw<'_>) {
    let rows = number(widget, "height").unwrap_or(10.0).clamp(1.0, 30.0);
    let width = field_width(ui, widget, 60.0).max(chars_width(ui, 40.0));
    let row_height = ui.text_style_height(&TextStyle::Monospace);
    let enabled = widget.enabled;
    // as in `entry`: in a grid cell, room for the width asked for
    ui.set_min_width(width);
    ScrollArea::vertical()
        .id_salt("text")
        .max_height(rows * row_height + 8.0)
        .show(ui, |ui| {
            let response = edit_field(ui, widget, cx, |ui, text| {
                let field = TextEdit::multiline(text)
                    .font(TextStyle::Monospace)
                    .desired_rows(rows as usize)
                    .desired_width(width);
                ui.add_enabled(enabled, field)
            });
            field_keys(ui, &response, widget, true, cx);
        });
}

fn listbox(ui: &mut Ui, widget: &Widget, cx: &mut Draw<'_>) {
    let rows = number(widget, "height").unwrap_or(10.0).clamp(2.0, 30.0);
    let width = field_width(ui, widget, 20.0);
    let many = matches!(option(widget, "selectmode"), Some("extended" | "multiple"));
    let row_height = ui.spacing().interact_size.y;
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ScrollArea::vertical()
            .id_salt("list")
            .max_height(rows * row_height)
            .auto_shrink([false, false])
            .max_width(width)
            .show(ui, |ui| {
                ui.set_min_width(width);
                // one item under the other, also inside a grid row
                ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                    for (index, item) in widget.items.iter().enumerate() {
                        let index = index as u32;
                        let selected = widget.selection.contains(&index);
                        let response = ui.add_enabled(
                            widget.enabled,
                            egui::Button::selectable(selected, item.as_str()),
                        );
                        if response.double_clicked() {
                            // The second click of a double click is a click too.
                            cx.events.push(UiEvent::Select {
                                path: widget.path.clone(),
                                selection: vec![index],
                            });
                            cx.events.push(UiEvent::Activate {
                                path: widget.path.clone(),
                                index,
                            });
                        } else if response.clicked() {
                            let extend =
                                many && ui.input(|i| i.modifiers.command || i.modifiers.shift);
                            let mut selection = if extend {
                                widget.selection.clone()
                            } else {
                                Vec::new()
                            };
                            match selection.iter().position(|i| *i == index) {
                                Some(at) if extend => {
                                    selection.remove(at);
                                }
                                _ => selection.push(index),
                            }
                            selection.sort_unstable();
                            cx.events.push(UiEvent::Select {
                                path: widget.path.clone(),
                                selection,
                            });
                        }
                    }
                });
            });
    });
}

fn scale(ui: &mut Ui, widget: &Widget, cx: &mut Draw<'_>) {
    let from = number(widget, "from").unwrap_or(0.0);
    let to = number(widget, "to").unwrap_or(100.0);
    let step = number(widget, "resolution").unwrap_or(1.0);
    let edit = cx.edits.texts.entry(widget.path.clone()).or_default();
    let mut value: f32 = edit.text.trim().parse().unwrap_or(from);
    let slider =
        egui::Slider::new(&mut value, from.min(to)..=from.max(to)).step_by(f64::from(step));
    if ui.add_enabled(widget.enabled, slider).changed() {
        let text = if step.fract() == 0.0 {
            format!("{value:.0}")
        } else {
            value.to_string()
        };
        if text != edit.text {
            edit.text = text;
            let event = Edits::sent(edit, &widget.path);
            cx.events.push(event);
        }
    }
}

/// The first widget of `window` (depth first) for which `found` holds.
pub fn find<'a>(widget: &'a Widget, found: &dyn Fn(&Widget) -> bool) -> Option<&'a Widget> {
    if found(widget) {
        return Some(widget);
    }
    widget.children.iter().find_map(|child| find(child, found))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry_widget(text: &str) -> Widget {
        Widget {
            path: ".d.e".into(),
            class: "Entry".into(),
            text: text.into(),
            enabled: true,
            ..Default::default()
        }
    }

    #[test]
    fn typing_survives_late_echoes() {
        let mut edits = Edits::default();
        edits.sync(&entry_widget("R1"));
        assert_eq!(edits.text(".d.e"), Some("R1"));

        // Typed "R10", then "R100", before any echo arrived.
        assert!(edits.set(".d.e", "R10").is_some());
        assert!(edits.set(".d.e", "R100").is_some());
        // The echo of the first keystroke must not undo the second.
        edits.sync(&entry_widget("R10"));
        assert_eq!(edits.text(".d.e"), Some("R100"));
        edits.sync(&entry_widget("R100"));
        assert_eq!(edits.text(".d.e"), Some("R100"));
        // Nothing new: no event.
        assert!(edits.set(".d.e", "R100").is_none());

        // xschem's code replaces the text (a "default" button, say).
        edits.sync(&entry_widget("R1"));
        assert_eq!(edits.text(".d.e"), Some("R1"));

        // A snapshot that repeats the old text while an edit is on its way is not an
        // echo of it and not news either.
        assert!(edits.set(".d.e", "R2").is_some());
        edits.sync(&entry_widget("R1"));
        assert_eq!(edits.text(".d.e"), Some("R2"));

        edits.forget(".d");
        assert_eq!(edits.text(".d.e"), None);
    }

    #[test]
    fn forgetting_a_window_keeps_its_namesakes() {
        let mut edits = Edits::default();
        for path in [".d.e", ".dialog.e", ".d"] {
            edits.texts.insert(path.into(), Edit::default());
        }
        edits.forget(".d");
        assert_eq!(edits.texts.len(), 1);
        assert!(edits.texts.contains_key(".dialog.e"));
    }
}
