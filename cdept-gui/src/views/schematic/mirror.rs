//! xschem's own widgets, mirrored: menu bar, tool bar, status bar, context menu and
//! dialogs, as the helper reports them (`UiUpdate`), drawn with egui.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};

use egui::load::SizedTexture;
use egui::{Button, Color32, Context, Id, Pos2, Rect, TextureHandle, TextureOptions, Ui};
use libcdept::xschem::protocol::{
    Image, MenuItem, MenuKind, StatusField, ToolItem, UiEvent, UiUpdate, Widget, Window,
};
use libcdept::xschem::session::PaneUpdate;

use libcdept::Project;

use super::picker::{self, Picked, Picker};
use super::widgets::{self, Draw, Edits};

/// Names of CDePT's own menus. xschem menus with the same name go into one
/// "Schematic" menu, so that the menu bar never shows a name twice.
const OWN_MENUS: [&str; 7] = ["File", "Edit", "View", "Project", "Tools", "Window", "Help"];

/// Decode standard base64, ignoring white space. `None` on anything else.
fn base64(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut bits, mut count) = (0u32, 0);
    for byte in text.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b' ' | b'\n' | b'\r' | b'\t' => continue,
            _ => return None,
        };
        bits = bits << 6 | u32::from(value);
        count += 6;
        if count >= 8 {
            count -= 8;
            out.push((bits >> count) as u8);
        }
    }
    Some(out)
}

enum Picture {
    Encoded(Image),
    Ready(TextureHandle),
    /// Could not be decoded; the widget falls back to its text.
    Broken,
}

/// The pictures the helper sent, decoded when first used.
#[derive(Default)]
pub struct Images {
    pictures: HashMap<String, Picture>,
}

impl Images {
    fn add(&mut self, image: Image) {
        self.pictures
            .insert(image.name.clone(), Picture::Encoded(image));
    }

    /// The texture of the picture named `name`.
    pub fn texture(&mut self, ctx: &Context, name: &str) -> Option<TextureHandle> {
        let picture = self.pictures.get_mut(name)?;
        if let Picture::Encoded(image) = picture {
            *picture = match decode(image) {
                Some(pixels) => Picture::Ready(ctx.load_texture(
                    format!("xschem-image-{name}"),
                    pixels,
                    TextureOptions::NEAREST,
                )),
                None => {
                    tracing::debug!(target: "xschem", "cannot decode the picture '{name}'");
                    Picture::Broken
                }
            };
        }
        match picture {
            Picture::Ready(texture) => Some(texture.clone()),
            _ => None,
        }
    }
}

fn decode(image: &Image) -> Option<egui::ColorImage> {
    let bytes = base64(&image.data)?;
    let pixels = image::load_from_memory(&bytes).ok()?.to_rgba8();
    let size = [pixels.width() as usize, pixels.height() as usize];
    Some(egui::ColorImage::from_rgba_unmultiplied(
        size,
        pixels.as_raw(),
    ))
}

/// A picture pane of a dialog: what xschem's C code drew into a frame of the dialog.
struct Pane {
    texture: TextureHandle,
    /// Size of the picture in pixels.
    size: [u32; 2],
    /// Size last asked for (`UiEvent::Configure`), in pixels.
    asked: Option<[u32; 2]>,
    /// How many different colours the picture has, counted up to 16 (for tests).
    colours: usize,
}

/// Changes of the size of a pane up to this many pixels are not passed on.
const PANE_SLACK: u32 = 40;

/// The picture panes of the dialogs, by Tk path of their widget.
#[derive(Default)]
pub struct Panes {
    panes: HashMap<String, Pane>,
}

fn count_colours(pixels: &[u8]) -> usize {
    let mut seen: Vec<&[u8; 4]> = Vec::new();
    for pixel in pixels.as_chunks::<4>().0 {
        if !seen.contains(&pixel) {
            seen.push(pixel);
            if seen.len() == 16 {
                break;
            }
        }
    }
    seen.len()
}

impl Panes {
    fn apply(&mut self, update: PaneUpdate, ctx: &Context) {
        match update {
            PaneUpdate::Changed {
                widget,
                width,
                height,
                pixels,
            } => {
                let size = [width, height];
                let colours = count_colours(&pixels);
                let image = egui::ColorImage::from_rgba_premultiplied(
                    [width as usize, height as usize],
                    &pixels,
                );
                match self.panes.get_mut(&widget) {
                    Some(pane) => {
                        pane.texture.set(image, TextureOptions::NEAREST);
                        pane.size = size;
                        pane.colours = colours;
                    }
                    None => {
                        let texture = ctx.load_texture(
                            format!("xschem-pane-{widget}"),
                            image,
                            TextureOptions::NEAREST,
                        );
                        let pane = Pane {
                            texture,
                            size,
                            asked: None,
                            colours,
                        };
                        self.panes.insert(widget, pane);
                    }
                }
            }
            PaneUpdate::Gone { widget } => {
                self.panes.remove(&widget);
            }
        }
    }

    /// Forget the panes of the window at `path`.
    fn forget(&mut self, path: &str) {
        let inside = format!("{path}.");
        self.panes
            .retain(|key, _| key != path && !key.starts_with(&inside));
    }

    /// Whether the widget at `path` is a picture pane.
    pub fn has(&self, path: &str) -> bool {
        self.panes.contains_key(path)
    }

    /// Draw the pane of the widget at `path`, `size` points large (whole pixels are
    /// asked for: the picture is drawn one pixel per screen pixel). A size the picture
    /// does not have yet is asked for with `UiEvent::Configure`; a click on it is
    /// reported as `UiEvent::Invoke` (Tk runs the frame's button bindings).
    pub fn show(&mut self, ui: &mut Ui, path: &str, size: egui::Vec2, events: &mut Vec<UiEvent>) {
        let Some(pane) = self.panes.get_mut(path) else {
            return;
        };
        let scale = ui.ctx().pixels_per_point();
        let mut pixels = [
            (size.x * scale).floor().max(16.0) as u32,
            (size.y * scale).floor().max(16.0) as u32,
        ];
        // A window that fits its content grows a little with a pane that takes all the
        // width there is, frame after frame: small changes are ignored, so the window
        // settles; a resize by the user is larger than that.
        if let Some(asked) = pane.asked
            && pixels[0].abs_diff(asked[0]) <= PANE_SLACK
            && pixels[1].abs_diff(asked[1]) <= PANE_SLACK
        {
            pixels = asked;
        }
        if pane.asked != Some(pixels) {
            pane.asked = Some(pixels);
            if pane.size != pixels {
                events.push(UiEvent::Configure {
                    path: path.to_string(),
                    width: pixels[0],
                    height: pixels[1],
                });
            }
        }
        let area = egui::Vec2::new(pixels[0] as f32, pixels[1] as f32) / scale;
        let (rect, response) = ui.allocate_exact_size(area, egui::Sense::click());
        // Pixel for pixel, from a corner on a whole pixel; a picture of the old size
        // (a new one is on its way) is cut to the space.
        let origin = Pos2::new(
            (rect.left() * scale).ceil() / scale,
            (rect.top() * scale).ceil() / scale,
        );
        let shown = egui::Vec2::new(pane.size[0] as f32, pane.size[1] as f32) / scale;
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, Color32::BLACK);
        painter.image(
            pane.texture.id(),
            Rect::from_min_size(origin, shown),
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
        if response.clicked() {
            events.push(UiEvent::Invoke {
                path: path.to_string(),
            });
        }
    }

    /// Size of the picture of the pane at `path`, the size last asked for, and how
    /// many colours it has (up to 16).
    pub fn info(&self, path: &str) -> Option<([u32; 2], Option<[u32; 2]>, usize)> {
        let pane = self.panes.get(path)?;
        Some((pane.size, pane.asked, pane.colours))
    }
}

/// A native file chooser running on its own thread, for a `FileDialog` window.
struct Chooser {
    /// Tk path of the window that asked.
    path: String,
    result: Receiver<Option<PathBuf>>,
}

fn start_chooser(window: &Window, ctx: &Context) -> Chooser {
    let option = |name: &str| window.root.options.get(name).cloned().unwrap_or_default();
    let (mode, dir, file) = (option("mode"), option("initialdir"), option("initialfile"));
    let extensions: Vec<String> = option("extensions")
        .split_whitespace()
        .map(str::to_string)
        .collect();
    // The desktop's file dialog, or CDePT's own path prompt where there is none.
    let request = crate::pathfield::Request {
        kind: match mode.as_str() {
            "save" => crate::pathfield::Kind::SaveFile,
            "directory" => crate::pathfield::Kind::Folder,
            _ => crate::pathfield::Kind::File,
        },
        title: window.title.clone(),
        directory: (!dir.is_empty()).then(|| PathBuf::from(&dir)),
        file_name: (!file.is_empty()).then(|| file.clone()),
        filters: if extensions.is_empty() {
            Vec::new()
        } else {
            vec![("xschem".into(), extensions)]
        },
        ..Default::default()
    };
    let result = crate::pathfield::ask_here(ctx, request);
    Chooser {
        path: window.path.clone(),
        result,
    }
}

/// A context menu xschem asked for, shown where the pointer was.
struct Popup {
    items: Vec<MenuItem>,
    at: Option<Pos2>,
}

/// Everything mirrored from one xschem.
#[derive(Default)]
pub struct Mirror {
    menu: Vec<MenuItem>,
    toolbar: Vec<ToolItem>,
    images: Images,
    status: Vec<StatusField>,
    title: Option<String>,
    cursor: Option<String>,
    popup: Option<Popup>,
    /// Top-level windows, oldest first.
    windows: Vec<Window>,
    /// The widget each window last gave the focus to, so that it is done once.
    focused: HashMap<String, String>,
    edits: Edits,
    panes: Panes,
    /// A dialog opened since the last call of `take_opened_window`.
    window_opened: bool,
    choosers: Vec<Chooser>,
    /// Library pickers, by the Tk path of the window that asked.
    pickers: HashMap<String, Picker>,
    /// Files xschem wants opened as documents of their own.
    opened: Vec<PathBuf>,
    /// A "save as" was answered with a file: the libraries may have a new cell.
    saved_as: bool,
    /// xschem wrote a design file besides the document (a symbol made from it).
    written: bool,
}

impl Mirror {
    /// Take in one update from the helper.
    pub fn apply(&mut self, update: UiUpdate, ctx: &Context) {
        match update {
            UiUpdate::Menu { items } => self.menu = items,
            UiUpdate::Images { images } => images.into_iter().for_each(|i| self.images.add(i)),
            UiUpdate::Toolbar { items } => self.toolbar = items,
            UiUpdate::Status { fields } => {
                for field in fields.iter().filter(|f| f.editable) {
                    self.edits.sync_text(&field.name, &field.text);
                }
                self.status = fields;
            }
            UiUpdate::Open { path } => self.opened.push(PathBuf::from(path)),
            UiUpdate::Written { path } => {
                tracing::debug!(target: "xschem::ui", "written: {path}");
                self.written = true;
            }
            UiUpdate::Title { text } => self.title = Some(text),
            UiUpdate::Cursor { name } => self.cursor = Some(name),
            UiUpdate::Popup { items } => self.popup = Some(Popup { items, at: None }),
            UiUpdate::Log { text } => tracing::info!(target: "xschem", "{text}"),
            UiUpdate::Clipboard { text } => ctx.copy_text(text),
            UiUpdate::Window { window } => {
                tracing::trace!(target: "xschem::ui", "{window:?}");
                self.edits.sync(&window.root);
                let asks_for_file = window.root.class == "FileDialog";
                if asks_for_file && !self.choosers.iter().any(|c| c.path == window.path) {
                    self.choosers.push(start_chooser(&window, ctx));
                }
                match self.windows.iter_mut().find(|w| w.path == window.path) {
                    Some(open) => *open = window,
                    None => {
                        self.window_opened = true;
                        self.windows.push(window);
                    }
                }
            }
            UiUpdate::WindowClosed { path } => {
                self.windows.retain(|w| w.path != path);
                self.focused.remove(&path);
                self.pickers.remove(&path);
                self.edits.forget(&path);
                self.panes.forget(&path);
            }
        }
    }

    /// Take in a change of a picture pane.
    pub fn apply_pane(&mut self, update: PaneUpdate, ctx: &Context) {
        self.panes.apply(update, ctx);
    }

    /// Whether a dialog opened since the last call.
    pub fn take_opened_window(&mut self) -> bool {
        std::mem::take(&mut self.window_opened)
    }

    /// The picture pane of the widget at `path`: see [`Panes::info`].
    pub fn pane_info(&self, path: &str) -> Option<([u32; 2], Option<[u32; 2]>, usize)> {
        self.panes.info(path)
    }

    /// Whether a "save as" got a file name since the last call.
    pub fn take_saved_as(&mut self) -> bool {
        std::mem::take(&mut self.saved_as)
    }

    /// Whether xschem wrote another design file since the last call: the libraries
    /// may hold a new view.
    pub fn take_written(&mut self) -> bool {
        std::mem::take(&mut self.written)
    }

    /// The files xschem asked to open in tabs of their own since the last call.
    pub fn take_opened(&mut self) -> Vec<PathBuf> {
        std::mem::take(&mut self.opened)
    }

    /// Title of the main window, as xschem set it.
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    /// Pointer shape over the drawing, from the Tk cursor name xschem set.
    pub fn cursor(&self) -> egui::CursorIcon {
        use egui::CursorIcon;
        match self.cursor.as_deref().unwrap_or_default() {
            "crosshair" | "tcross" | "cross" | "plus" => CursorIcon::Crosshair,
            "watch" | "clock" => CursorIcon::Wait,
            "hand1" | "hand2" => CursorIcon::PointingHand,
            "fleur" => CursorIcon::Move,
            "xterm" => CursorIcon::Text,
            "question_arrow" => CursorIcon::Help,
            "sb_h_double_arrow" => CursorIcon::ResizeHorizontal,
            "sb_v_double_arrow" => CursorIcon::ResizeVertical,
            _ => CursorIcon::Default,
        }
    }

    /// A dialog is open that xschem waits for: nothing else reacts meanwhile.
    pub fn has_modal(&self) -> bool {
        self.windows.iter().any(|w| w.modal) || !self.choosers.is_empty()
    }

    /// The menu bar, as top-level menus of the application's menu bar.
    pub fn menus(&mut self, ui: &mut Ui) -> Vec<UiEvent> {
        let mut events = Vec::new();
        let enabled = !self.has_modal();
        let images = &mut self.images;
        ui.add_enabled_ui(enabled, |ui| {
            // xschem pads its menu bar with an entry labelled "-": not a menu.
            let (shared, own): (Vec<&MenuItem>, Vec<&MenuItem>) = self
                .menu
                .iter()
                .filter(|item| item.label.trim() != "-")
                .partition(|item| OWN_MENUS.contains(&item.label.as_str()));
            if !shared.is_empty() {
                ui.menu_button("Schematic", |ui| {
                    for item in shared {
                        menu_item(ui, item, images, &mut events);
                    }
                });
            }
            for item in own {
                menu_item(ui, item, images, &mut events);
            }
        });
        events
    }

    /// The tool bar buttons.
    pub fn toolbar(&mut self, ui: &mut Ui) -> Vec<UiEvent> {
        let mut events = Vec::new();
        let enabled = !self.has_modal();
        for item in &self.toolbar {
            if item.separator {
                ui.separator();
                continue;
            }
            let picture = self.images.texture(ui.ctx(), &item.image);
            let label = if item.label.is_empty() {
                item.tooltip.as_str()
            } else {
                item.label.as_str()
            };
            let button = match &picture {
                Some(texture) => Button::image(SizedTexture::from_handle(texture)),
                None => Button::new(label),
            };
            let mut response =
                ui.add_enabled(enabled && item.enabled, button.selected(item.checked));
            if !item.tooltip.is_empty() {
                response = response.on_hover_text(&item.tooltip);
            }
            if response.clicked() {
                events.push(UiEvent::Invoke {
                    path: item.path.clone(),
                });
            }
        }
        events
    }

    /// The status bar, as one line. Some fields can be edited (xschem's snap and grid).
    pub fn status_line(&mut self, ui: &mut Ui) -> Vec<UiEvent> {
        let mut events = Vec::new();
        ui.horizontal(|ui| {
            let shown = self
                .status
                .iter()
                .filter(|f| f.editable || !f.text.is_empty());
            for (n, field) in shown.enumerate() {
                if n > 0 {
                    ui.separator();
                }
                if !field.editable {
                    ui.label(&field.text);
                    continue;
                }
                let edit = egui::TextEdit::singleline(self.edits.text_mut(&field.name))
                    .desired_width(48.0)
                    .id_salt(&field.name);
                let response = ui.add(edit);
                if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    events.push(self.edits.changed(&field.name));
                    events.push(UiEvent::Key {
                        path: field.name.clone(),
                        key: "Return".into(),
                    });
                } else if response.lost_focus() {
                    // Left without Return: back to what xschem has.
                    *self.edits.text_mut(&field.name) = field.text.clone();
                }
            }
        });
        events
    }

    pub fn has_status(&self) -> bool {
        self.status.iter().any(|f| f.editable || !f.text.is_empty())
    }

    /// The context menu xschem asked for, if any, at the pointer.
    pub fn popup(&mut self, ui: &Ui, salt: Id, pointer: Option<Pos2>) -> Vec<UiEvent> {
        let mut events = Vec::new();
        let Some(popup) = &mut self.popup else {
            return events;
        };
        let at = *popup
            .at
            .get_or_insert(pointer.unwrap_or_else(|| ui.max_rect().center()));
        let mut open = true;
        let images = &mut self.images;
        egui::Popup::new(salt.with("popup"), ui.ctx().clone(), at, ui.layer_id())
            .kind(egui::PopupKind::Menu)
            .layout(egui::Layout::top_down_justified(egui::Align::Min))
            .style(egui::containers::menu::menu_style)
            .open_bool(&mut open)
            .show(|ui| {
                for item in &popup.items {
                    menu_item(ui, item, images, &mut events);
                }
            });
        if !events.is_empty() {
            self.popup = None;
        } else if !open {
            self.popup = None;
            events.push(UiEvent::PopupClosed);
        }
        events
    }

    /// The dialogs, as windows over `area` (the tab they belong to).
    pub fn windows(
        &mut self,
        ctx: &Context,
        salt: Id,
        area: Rect,
        project: Option<&Project>,
        picture: &mut picker::ShowPicture<'_>,
    ) -> Vec<UiEvent> {
        let mut events = Vec::new();
        self.choosers
            .retain(|chooser| match chooser.result.try_recv() {
                Ok(chosen) => {
                    let text = chosen.map(|p| p.to_string_lossy().into_owned());
                    events.push(UiEvent::Text {
                        path: chooser.path.clone(),
                        text: text.unwrap_or_default(),
                    });
                    false
                }
                Err(mpsc::TryRecvError::Empty) => true,
                Err(mpsc::TryRecvError::Disconnected) => {
                    events.push(UiEvent::Text {
                        path: chooser.path.clone(),
                        text: String::new(),
                    });
                    false
                }
            });

        let special = |w: &Window| matches!(w.root.class.as_str(), "FileDialog" | "LibraryPicker");
        let top = self.windows.iter().rposition(|w| !special(w));
        let mut elsewhere = Vec::new();
        for (n, window) in self.windows.iter().enumerate() {
            let id = salt.with(("window", &window.path));
            if window.root.class == "LibraryPicker" {
                let waiting = self.choosers.iter().any(|c| c.path == window.path);
                let picker = self.pickers.entry(window.path.clone()).or_default();
                if waiting || picker.answered {
                    continue;
                }
                let answer = match picker::show(ctx, id, area, window, project, picker, picture) {
                    Picked::Nothing => continue,
                    Picked::Elsewhere => {
                        elsewhere.push(start_chooser(window, ctx));
                        continue;
                    }
                    Picked::Cancel => String::new(),
                    Picked::File(file) => file.to_string_lossy().into_owned(),
                };
                let saving = window.root.options.get("mode").is_some_and(|m| m == "save");
                self.saved_as |= saving && !answer.is_empty();
                picker.answered = true;
                events.push(UiEvent::Text {
                    path: window.path.clone(),
                    text: answer,
                });
                continue;
            }
            let mut cx = Draw {
                edits: &mut self.edits,
                events: &mut events,
                images: &mut self.images,
                panes: &mut self.panes,
                focus: None,
            };
            // Give the focus where xschem put it, once per change.
            let wanted = widgets::find(&window.root, &|w| w.focus).map(|w| w.path.clone());
            if let Some(path) = &wanted
                && self.focused.get(&window.path) != Some(path)
            {
                self.focused.insert(window.path.clone(), path.clone());
                cx.focus = wanted.as_deref();
            }
            match window.root.class.as_str() {
                "FileDialog" => {}
                "MessageBox" => message_box(ctx, id, window, &mut cx),
                _ => dialog(ctx, id, area, window, Some(n) == top, &mut cx),
            }
        }
        self.choosers.extend(elsewhere);
        events
    }

    /// The button of the topmost dialog whose text is `text`.
    pub fn dialog_button(&self, text: &str) -> Option<UiEvent> {
        let found = self.windows.iter().rev().find_map(|window| {
            widgets::find(&window.root, &|w| {
                w.class.ends_with("utton") && w.text.trim() == text.trim()
            })
        })?;
        Some(UiEvent::Invoke {
            path: found.path.clone(),
        })
    }

    /// Type `text` into a dialog's text field, named by its Tk path or by the text of
    /// the label in front of it.
    pub fn dialog_type(&mut self, target: &str, text: &str) -> Result<Option<UiEvent>, String> {
        let is_field = |w: &Widget| {
            matches!(
                w.class.as_str(),
                "Entry" | "TEntry" | "Text" | "Spinbox" | "TSpinbox" | "TCombobox"
            )
        };
        let mut fields: Vec<(String, String)> = Vec::new();
        for window in self.windows.iter().rev() {
            let mut label = String::new();
            collect_fields(&window.root, &is_field, &mut label, &mut fields);
        }
        let path = fields
            .iter()
            .find(|(path, label)| {
                path == target || label.trim().trim_end_matches(':') == target.trim_end_matches(':')
            })
            .map(|(path, _)| path.clone())
            .ok_or_else(|| format!("no text field '{target}' in an open dialog"))?;
        Ok(self.edits.set(&path, text))
    }

    /// The entry of the mirrored menu bar found by following `labels`.
    pub fn menu_entry(&self, labels: &[&str]) -> Option<UiEvent> {
        let mut items = &self.menu;
        let mut found = None;
        for label in labels {
            let item = items.iter().find(|i| plain(&i.label) == plain(label))?;
            items = &item.items;
            found = Some(item);
        }
        let item = found.filter(|i| !matches!(i.kind, MenuKind::Cascade | MenuKind::Separator))?;
        Some(UiEvent::Menu {
            menu: item.menu.clone(),
            index: item.index,
        })
    }

    /// Every entry of the mirrored menu bar, as `Menu / Sub-menu / Entry`.
    pub fn menu_paths(&self) -> Vec<String> {
        fn walk(items: &[MenuItem], prefix: &str, out: &mut Vec<String>) {
            for item in items {
                let path = format!("{prefix}{}", plain(&item.label));
                match item.kind {
                    MenuKind::Separator => {}
                    MenuKind::Cascade => walk(&item.items, &format!("{path} / "), out),
                    MenuKind::Check | MenuKind::Radio if item.checked => {
                        out.push(format!("{path} ✔"))
                    }
                    _ => out.push(path),
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.menu, "", &mut out);
        out
    }

    /// The mode (`open`, `save`, `symbol`, `merge`) of the library picker that waits
    /// for an answer.
    pub fn picker_mode(&self) -> Option<&str> {
        let window = self.waiting_picker()?;
        Some(window.root.options.get("mode").map_or("", String::as_str))
    }

    fn waiting_picker(&self) -> Option<&Window> {
        self.windows.iter().rev().find(|w| {
            w.root.class == "LibraryPicker"
                && !self.pickers.get(&w.path).is_some_and(|p| p.answered)
        })
    }

    /// Answer the library picker that waits: with a file, or with nothing (cancel).
    pub fn answer_picker(&mut self, file: Option<&std::path::Path>) -> Option<UiEvent> {
        self.saved_as |= file.is_some() && self.picker_mode() == Some("save");
        let path = self.waiting_picker()?.path.clone();
        self.pickers.entry(path.clone()).or_default().answered = true;
        Some(UiEvent::Text {
            path,
            text: file.map_or_else(String::new, |f| f.to_string_lossy().into_owned()),
        })
    }

    /// Titles of the open dialogs, oldest first.
    pub fn window_titles(&self) -> Vec<&str> {
        self.windows.iter().map(|w| w.title.as_str()).collect()
    }
}

/// A label for comparing: without the dots of "Open…" and surrounding space.
fn plain(label: &str) -> &str {
    label.trim().trim_end_matches(['.', '…']).trim()
}

/// Collect `(path, label in front)` of the text fields below `widget`.
fn collect_fields(
    widget: &Widget,
    is_field: &dyn Fn(&Widget) -> bool,
    label: &mut String,
    out: &mut Vec<(String, String)>,
) {
    if matches!(widget.class.as_str(), "Label" | "TLabel") {
        *label = widget.text.clone();
    } else if is_field(widget) {
        out.push((widget.path.clone(), std::mem::take(label)));
    }
    for child in &widget.children {
        collect_fields(child, is_field, label, out);
    }
}

/// A colour as Tk writes it: `#rgb`, `#rrggbb`, `#rrrrggggbbbb`, or one of the names
/// xschem uses.
fn tk_color(text: &str) -> Option<Color32> {
    if let Some(hex) = text.strip_prefix('#') {
        let digits = hex.len() / 3;
        if !matches!(digits, 1 | 2 | 4) || hex.len() != digits * 3 {
            return None;
        }
        // The first two digits of each part carry the eight bits egui keeps.
        let part = |n: usize| -> Option<u8> {
            let part = &hex[n * digits..(n + 1) * digits];
            let value = u32::from_str_radix(part, 16).ok()?;
            Some(match digits {
                1 => (value * 17) as u8,
                2 => value as u8,
                _ => (value >> 8) as u8,
            })
        };
        return Some(Color32::from_rgb(part(0)?, part(1)?, part(2)?));
    }
    let rgb = match text.to_ascii_lowercase().as_str() {
        "red" => [255, 0, 0],
        "green" => [0, 200, 0],
        "blue" => [80, 120, 255],
        "yellow" => [255, 255, 0],
        "orange" => [255, 165, 0],
        "cyan" => [0, 255, 255],
        "magenta" => [255, 0, 255],
        "white" => [255, 255, 255],
        "black" => [0, 0, 0],
        "gray" | "grey" => [190, 190, 190],
        "palegreen" => [152, 251, 152],
        "darkgreen" => [0, 140, 0],
        "lightblue" => [173, 216, 230],
        "pink" => [255, 192, 203],
        _ => return None,
    };
    Some(Color32::from_rgb(rgb[0], rgb[1], rgb[2]))
}

fn menu_item(ui: &mut Ui, item: &MenuItem, images: &mut Images, events: &mut Vec<UiEvent>) {
    let mut label = egui::RichText::new(&item.label);
    if let Some(color) = tk_color(&item.color) {
        label = label.color(color);
    }
    let picture = images.texture(ui.ctx(), &item.image);
    let chosen = match item.kind {
        MenuKind::Separator => {
            ui.separator();
            false
        }
        MenuKind::Cascade => {
            ui.add_enabled_ui(item.enabled, |ui| {
                ui.menu_button(label, |ui| {
                    for child in &item.items {
                        menu_item(ui, child, images, events);
                    }
                });
            });
            false
        }
        MenuKind::Command => {
            let button = match &picture {
                Some(texture) => Button::image_and_text(SizedTexture::from_handle(texture), label),
                None => Button::new(label),
            };
            let button = button.shortcut_text(&item.accelerator);
            ui.add_enabled(item.enabled, button).clicked()
        }
        MenuKind::Check => {
            let mut checked = item.checked;
            let check = egui::Checkbox::new(&mut checked, label);
            ui.add_enabled(item.enabled, check).changed()
        }
        MenuKind::Radio => {
            let radio = egui::RadioButton::new(item.checked, label);
            ui.add_enabled(item.enabled, radio).clicked()
        }
    };
    if chosen {
        events.push(UiEvent::Menu {
            menu: item.menu.clone(),
            index: item.index,
        });
        ui.close();
    }
}

/// An ordinary dialog: a window over the tab, with the widgets inside.
fn dialog(ctx: &Context, id: Id, area: Rect, window: &Window, top: bool, cx: &mut Draw<'_>) {
    let mut open = true;
    let title = if window.title.is_empty() {
        "xschem"
    } else {
        window.title.as_str()
    };
    let shown = egui::Window::new(title)
        .id(id)
        .open(&mut open)
        .collapsible(false)
        // The height follows the content, up to what the tab has room for (the scroll
        // area below): egui's default of 420 points hid the buttons at the bottom of
        // tall dialogs (simulator configuration, graph dialog). The width stays the
        // user's to change.
        .resizable([true, false])
        .default_height(area.height().max(200.0) - 40.0)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(area.center())
        .show(ctx, |ui| {
            let before = cx.events.len();
            if cx.panes.has(&window.root.path) {
                // The window is a picture (xschem's key cheat sheet): as large as the
                // tab allows.
                let size =
                    (area.size() - egui::Vec2::new(40.0, 90.0)).max(egui::Vec2::splat(200.0));
                cx.panes.show(ui, &window.root.path, size, cx.events);
            } else {
                dialog_body(ui, id, area, &window.root, cx);
            }
            cx.events.len() > before
        });
    // Return and Escape with no widget focused go to the window, where Tk dialogs
    // usually bind them.
    let used = shown.and_then(|s| s.inner).unwrap_or(false);
    if top && !used && ctx.memory(|m| m.focused().is_none()) {
        for (key, name) in [(egui::Key::Enter, "Return"), (egui::Key::Escape, "Escape")] {
            if ctx.input(|i| i.key_pressed(key)) {
                cx.events.push(UiEvent::Key {
                    path: window.path.clone(),
                    key: name.to_string(),
                });
            }
        }
    }
    if !open {
        cx.events.push(UiEvent::Close {
            path: window.path.clone(),
        });
    }
}

/// The widgets of a dialog, scrolled when the tab is too low for them. As with Tk, the
/// part that is packed to expand is what gives way: what is packed before and after it
/// (a title, the row of buttons at the bottom) stays in view.
fn dialog_body(ui: &mut Ui, id: Id, area: Rect, root: &Widget, cx: &mut Draw<'_>) {
    let room = area.height().max(200.0) - 60.0;
    let placed: Vec<&Widget> = root
        .children
        .iter()
        .filter(|c| !c.layout.is_empty())
        .collect();
    let packed = placed
        .iter()
        .all(|c| c.layout.get("manager").map(String::as_str) == Some("pack"));
    let expands = |c: &Widget| {
        matches!(
            c.layout.get("expand").map(String::as_str),
            Some("1" | "yes" | "true")
        ) && matches!(
            c.layout.get("side").map(String::as_str),
            None | Some("top" | "bottom")
        )
    };
    let at = placed.iter().position(|c| expands(c));
    let (Some(at), true) = (at, packed) else {
        egui::ScrollArea::vertical()
            .max_height(room)
            .show(ui, |ui| widgets::children(ui, root, cx));
        return;
    };
    let part = |widgets: &[&Widget]| Widget {
        children: widgets.iter().map(|w| (*w).clone()).collect(),
        ..root.clone()
    };
    let (before, rest) = placed.split_at(at);
    let (middle, after) = rest.split_at(1);
    // The height of the parts around the scrolled one, from the previous frame.
    let fixed_id = id.with("fixed height");
    let fixed: f32 = ui.data(|d| d.get_temp(fixed_id)).unwrap_or(0.0);
    let top = ui.cursor().top();
    let mut fixed_now = 0.0;
    if !before.is_empty() {
        widgets::children(ui, &part(before), cx);
    }
    fixed_now += ui.cursor().top() - top;
    egui::ScrollArea::vertical()
        .max_height((room - fixed).max(100.0))
        .show(ui, |ui| widgets::children(ui, &part(middle), cx));
    if !after.is_empty() {
        let start = ui.cursor().top();
        widgets::children(ui, &part(after), cx);
        fixed_now += ui.cursor().top() - start;
    }
    if (fixed_now - fixed).abs() > 0.5 {
        ui.data_mut(|d| d.insert_temp(fixed_id, fixed_now));
        ui.ctx().request_repaint();
    }
}

/// `tk_messageBox`: a message and a row of buttons, over everything.
fn message_box(ctx: &Context, id: Id, window: &Window, cx: &mut Draw<'_>) {
    let option = |name: &str| window.root.options.get(name).map_or("", String::as_str);
    let icon = match option("icon") {
        "error" => "⛔",
        "warning" => "⚠",
        "question" => "❓",
        _ => "ℹ",
    };
    let response = egui::Modal::new(id).show(ctx, |ui| {
        ui.set_max_width(480.0);
        if !window.title.is_empty() {
            ui.heading(&window.title);
        }
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(icon).size(28.0));
            ui.vertical(|ui| {
                ui.label(option("message"));
                if !option("detail").is_empty() {
                    ui.weak(option("detail"));
                }
            });
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            for button in &window.root.children {
                widgets::draw(ui, button, cx);
            }
        });
    });
    if response.should_close() {
        cx.events.push(UiEvent::Close {
            path: window.path.clone(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tk_colors() {
        assert_eq!(tk_color("#ff8000"), Some(Color32::from_rgb(255, 128, 0)));
        assert_eq!(tk_color("#f80"), Some(Color32::from_rgb(255, 136, 0)));
        assert_eq!(
            tk_color("#ffff80800000"),
            Some(Color32::from_rgb(255, 128, 0))
        );
        assert_eq!(tk_color("Red"), Some(Color32::from_rgb(255, 0, 0)));
        assert_eq!(tk_color(""), None);
        assert_eq!(tk_color("#12345"), None);
        assert_eq!(tk_color("no such colour"), None);
    }

    #[test]
    fn base64_decodes() {
        assert_eq!(base64("aGVsbG8=").as_deref(), Some(&b"hello"[..]));
        assert_eq!(base64("aGVs\n bG8h").as_deref(), Some(&b"hello!"[..]));
        assert_eq!(base64("").as_deref(), Some(&b""[..]));
        assert_eq!(base64("a$b"), None);
    }

    /// A 2×1 GIF: one red and one blue pixel.
    const GIF: &str = "R0lGODlhAgABAPAAAP8AAAAA/ywAAAAAAgABAAACAkQKADs=";

    #[test]
    fn pictures_decode_once_and_broken_ones_fall_back() {
        let ctx = Context::default();
        let mut images = Images::default();
        images.add(Image {
            name: "ok".into(),
            format: "gif".into(),
            data: GIF.into(),
        });
        images.add(Image {
            name: "bad".into(),
            format: "gif".into(),
            data: "AAAA".into(),
        });
        let texture = images.texture(&ctx, "ok").expect("the GIF decodes");
        assert_eq!(texture.size(), [2, 1]);
        assert!(images.texture(&ctx, "ok").is_some());
        assert!(images.texture(&ctx, "bad").is_none());
        assert!(images.texture(&ctx, "unknown").is_none());
    }

    fn widget(path: &str, class: &str, text: &str, layout: &[(&str, &str)]) -> Widget {
        Widget {
            path: path.into(),
            class: class.into(),
            text: text.into(),
            enabled: true,
            layout: layout
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            ..Default::default()
        }
    }

    /// A dialog with one of everything, placed with both `pack` and `grid`.
    fn sample_dialog() -> Window {
        let top = [("manager", "pack"), ("side", "top"), ("fill", "x")];
        let left = [("manager", "pack"), ("side", "left")];
        let cell = |row: &'static str, column: &'static str| {
            [("manager", "grid"), ("row", row), ("column", column)]
        };
        let mut list = widget(".d.list", "Listbox", "", &top);
        list.items = vec!["one".into(), "two".into(), "three".into()];
        list.selection = vec![1];
        list.options.insert("selectmode".into(), "extended".into());
        let mut combo = widget(".d.g.combo", "TCombobox", "b", &cell("1", "1"));
        combo.items = vec!["a".into(), "b".into()];
        let mut scale = widget(".d.scale", "Scale", "3", &top);
        scale.options.insert("from".into(), "0".into());
        scale.options.insert("to".into(), "10".into());
        let mut check = widget(".d.row.check", "Checkbutton", "Keep", &left);
        check.checked = true;
        let mut name = widget(".d.g.name", "Entry", "R1", &cell("0", "1"));
        name.focus = true;
        let mut grid = widget(".d.g", "Frame", "", &top);
        grid.children = vec![
            widget(".d.g.l0", "Label", "Name:", &cell("0", "0")),
            name,
            widget(".d.g.l1", "Label", "Kind", &cell("1", "0")),
            combo,
            widget(".d.g.spin", "Spinbox", "4", &cell("2", "1")),
        ];
        let mut row = widget(
            ".d.row",
            "Frame",
            "",
            &[("manager", "pack"), ("side", "bottom")],
        );
        row.children = vec![
            widget(".d.row.ok", "Button", "OK", &left),
            widget(".d.row.cancel", "Button", "Cancel", &left),
            check,
            widget(
                ".d.row.radio",
                "Radiobutton",
                "Fast",
                &[("manager", "pack"), ("side", "right")],
            ),
        ];
        let mut frame = widget(".d.lf", "Labelframe", "Options", &top);
        frame.children = vec![
            widget(".d.lf.msg", "Message", "A longer explanation.", &top),
            widget(".d.lf.hidden", "Label", "not placed", &[]),
            widget(".d.lf.what", "Mystery", "", &top),
        ];
        let mut root = widget(".d", "Toplevel", "", &[]);
        root.children = vec![
            grid,
            row,
            frame,
            list,
            scale,
            widget(".d.text", "Text", "line 1\nline 2", &top),
            widget(".d.rule", "Frame", "", &top),
            widget(
                ".d.sb",
                "Scrollbar",
                "",
                &[("manager", "pack"), ("side", "right")],
            ),
        ];
        Window {
            path: ".d".into(),
            title: "Sample".into(),
            modal: true,
            root,
        }
    }

    fn message_box() -> Window {
        let mut root = widget(".msg", "MessageBox", "", &[]);
        for (key, value) in [
            ("icon", "warning"),
            ("message", "Save changes?"),
            ("detail", "x.sch"),
        ] {
            root.options.insert(key.into(), value.into());
        }
        root.children = vec![
            widget(".msg.yes", "Button", "Yes", &[]),
            widget(".msg.no", "Button", "No", &[]),
        ];
        Window {
            path: ".msg".into(),
            title: "xschem".into(),
            modal: true,
            root,
        }
    }

    /// A picture pane: shown where its widget is, asked for at the size it is shown
    /// at (once), gone with its window.
    #[test]
    fn picture_panes() {
        let ctx = Context::default();
        let mut mirror = Mirror::default();
        let mut window = sample_dialog();
        window.root.children.push(widget(
            ".d.pane",
            "Frame",
            "",
            &[("manager", "pack"), ("fill", "both")],
        ));
        let path = window.path.clone();
        mirror.apply(UiUpdate::Window { window }, &ctx);
        mirror.apply_pane(
            PaneUpdate::Changed {
                widget: ".d.pane".into(),
                width: 2,
                height: 2,
                pixels: vec![255, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255],
            },
            &ctx,
        );
        assert_eq!(mirror.pane_info(".d.pane"), Some(([2, 2], None, 2)));
        let mut configures = Vec::new();
        for _ in 0..3 {
            let output = ctx.run_ui(egui::RawInput::default(), |ui| {
                let area = ui.max_rect();
                let events = mirror.windows(ui.ctx(), Id::new("t"), area, None, &mut |_, _| {});
                configures.extend(
                    events
                        .into_iter()
                        .filter(|e| matches!(e, UiEvent::Configure { .. })),
                );
            });
            output.drop_without_applying_deltas();
        }
        let [
            UiEvent::Configure {
                path: pane,
                width,
                height,
            },
        ] = &configures[..]
        else {
            panic!("one size asked for: {configures:?}");
        };
        assert_eq!(pane, ".d.pane");
        assert!(*width >= 200 && *height >= 100, "{width} x {height}");
        assert_eq!(
            mirror.pane_info(".d.pane").unwrap().1,
            Some([*width, *height])
        );
        mirror.apply(UiUpdate::WindowClosed { path }, &ctx);
        assert_eq!(mirror.pane_info(".d.pane"), None);
    }

    /// Draw everything a mirror can hold, headless, a few frames.
    #[test]
    fn everything_draws() {
        let ctx = Context::default();
        let mut mirror = Mirror::default();
        let file = item(
            MenuKind::Cascade,
            "File",
            0,
            vec![
                item(MenuKind::Command, "Open...", 1, vec![]),
                item(MenuKind::Separator, "", 2, vec![]),
                item(MenuKind::Check, "Show grid", 3, vec![]),
                item(MenuKind::Radio, "Spice", 4, vec![]),
                item(
                    MenuKind::Cascade,
                    "Export",
                    5,
                    vec![item(MenuKind::Command, "PNG", 0, vec![])],
                ),
            ],
        );
        let layers = item(
            MenuKind::Cascade,
            "Layers",
            1,
            vec![item(MenuKind::Command, "4", 0, vec![])],
        );
        let tool = |path: &str, image: &str| ToolItem {
            separator: false,
            path: path.into(),
            label: "Wire".into(),
            tooltip: "Insert wire".into(),
            image: image.into(),
            enabled: true,
            checked: false,
        };
        let gap = ToolItem {
            separator: true,
            ..tool("", "")
        };
        let updates = vec![
            UiUpdate::Menu {
                items: vec![file.clone(), layers],
            },
            UiUpdate::Images {
                images: vec![Image {
                    name: "icon".into(),
                    format: "gif".into(),
                    data: GIF.into(),
                }],
            },
            UiUpdate::Toolbar {
                items: vec![tool(".tb.wire", "icon"), gap, tool(".tb.text", "missing")],
            },
            UiUpdate::Status {
                fields: vec![
                    StatusField {
                        name: ".statusbar.1".into(),
                        text: "SNAP:".into(),
                        editable: false,
                    },
                    StatusField {
                        name: ".statusbar.2".into(),
                        text: "10".into(),
                        editable: true,
                    },
                    StatusField {
                        name: ".statusbar.3".into(),
                        text: String::new(),
                        editable: false,
                    },
                ],
            },
            UiUpdate::Title {
                text: "xschem - x.sch*".into(),
            },
            UiUpdate::Cursor {
                name: "crosshair".into(),
            },
            UiUpdate::Popup { items: file.items },
            UiUpdate::Log {
                text: "hello".into(),
            },
            UiUpdate::Window {
                window: sample_dialog(),
            },
            UiUpdate::Window {
                window: message_box(),
            },
        ];
        for update in updates {
            mirror.apply(update, &ctx);
        }
        assert!(mirror.has_modal());
        assert!(mirror.has_status());
        assert_eq!(mirror.title(), Some("xschem - x.sch*"));
        assert_eq!(mirror.cursor(), egui::CursorIcon::Crosshair);
        assert_eq!(mirror.window_titles(), ["Sample", "xschem"]);

        for _ in 0..3 {
            let output = ctx.run_ui(egui::RawInput::default(), |ui| {
                egui::MenuBar::new().ui(ui, |ui| {
                    assert!(mirror.menus(ui).is_empty());
                    assert!(mirror.toolbar(ui).is_empty());
                });
                assert!(mirror.status_line(ui).is_empty());
                let area = ui.max_rect();
                let salt = Id::new("test");
                let events = mirror.popup(ui, salt, None);
                assert!(events.is_empty(), "{events:?}");
                // The text field xschem focused reports the focus once.
                let events = mirror.windows(ui.ctx(), salt, area, None, &mut |_, _| {});
                for event in &events {
                    assert!(matches!(event, UiEvent::Focus { .. }), "{event:?}");
                }
            });
            output.drop_without_applying_deltas();
        }

        // The driver's ways into a dialog.
        assert_eq!(
            mirror.dialog_button("No"),
            Some(UiEvent::Invoke {
                path: ".msg.no".into()
            })
        );
        assert_eq!(
            mirror.dialog_button("OK"),
            Some(UiEvent::Invoke {
                path: ".d.row.ok".into()
            })
        );
        assert_eq!(mirror.dialog_button("Apply"), None);
        // By path, and by the label in front of the field.
        let typed = |path: &str, text: &str| {
            Some(UiEvent::Text {
                path: path.into(),
                text: text.into(),
            })
        };
        assert_eq!(
            mirror.dialog_type(".d.text", "new"),
            Ok(typed(".d.text", "new"))
        );
        assert_eq!(
            mirror.dialog_type("Name", "R2"),
            Ok(typed(".d.g.name", "R2"))
        );
        assert_eq!(
            mirror.dialog_type("Name", "R2"),
            Ok(None),
            "nothing changed"
        );
        assert!(mirror.dialog_type("Nothing", "x").is_err());
        assert!(
            mirror
                .menu_paths()
                .contains(&"File / Export / PNG".to_string())
        );

        mirror.apply(UiUpdate::WindowClosed { path: ".d".into() }, &ctx);
        mirror.apply(
            UiUpdate::WindowClosed {
                path: ".msg".into(),
            },
            &ctx,
        );
        assert!(!mirror.has_modal());
    }

    fn item(kind: MenuKind, label: &str, index: u32, items: Vec<MenuItem>) -> MenuItem {
        MenuItem {
            kind,
            label: label.into(),
            accelerator: String::new(),
            menu: ".menubar.file".into(),
            index,
            enabled: true,
            checked: false,
            image: String::new(),
            color: String::new(),
            items,
        }
    }

    #[test]
    fn menu_entries_are_found_by_their_labels() {
        let mut mirror = Mirror::default();
        let ctx = Context::default();
        let file = item(
            MenuKind::Cascade,
            "File",
            0,
            vec![
                item(MenuKind::Command, "Open...", 1, vec![]),
                item(MenuKind::Separator, "", 2, vec![]),
                item(MenuKind::Check, "Show grid", 3, vec![]),
            ],
        );
        mirror.apply(UiUpdate::Menu { items: vec![file] }, &ctx);
        assert_eq!(
            mirror.menu_entry(&["File", "Open"]),
            Some(UiEvent::Menu {
                menu: ".menubar.file".into(),
                index: 1
            })
        );
        assert!(mirror.menu_entry(&["File", "Show grid"]).is_some());
        assert_eq!(mirror.menu_entry(&["File"]), None, "a sub-menu is no entry");
        assert_eq!(mirror.menu_entry(&["File", "Close"]), None);
    }
}
