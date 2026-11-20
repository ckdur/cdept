//! A schematic or symbol open in a tab, drawn and edited by xschem.
//!
//! xschem runs in a helper process (`libcdept::xschem::Session`). The tab shows the
//! helper's drawing window as a texture, one frame pixel per screen pixel, sends it the
//! mouse and the keyboard the way X11 would, and draws xschem's own menus, tool bar,
//! status bar and dialogs with egui widgets ([`mirror`]).

mod clipboard;
pub mod linking;
pub use self::linking::LinkRequest;
mod keys;
mod mirror;
mod picker;
mod picture;
mod widgets;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::{Color32, Event, EventFilter, Modifiers, Pos2, Rect, Sense, Ui, Vec2};
use libcdept::Project;
use libcdept::xschem::input::Input;
use libcdept::xschem::protocol::UiEvent;
use libcdept::xschem::session::Reply;
use libcdept::xschem::{EngineOptions, NetlistFormat, Session, SessionOptions, Status};
use libcdept::xschem::{frame, tcl};

pub use self::clipboard::pretend as pretend_clipboard;
use self::mirror::Mirror;
pub use self::picture::Picture;
use super::{EditAction, EditCaps, View};
use crate::dock::{DocId, DocRef, Tab};
use crate::state::{Action, AppCtx, Selection};

/// Answers to Tcl requests kept for whoever asked (the test driver).
const KEPT_REPLIES: usize = 64;

/// The netlist formats offered in menus, with their names there.
pub const NETLIST_FORMATS: [(&str, NetlistFormat); 5] = [
    ("SPICE", NetlistFormat::Spice),
    ("Spectre", NetlistFormat::Spectre),
    ("Verilog", NetlistFormat::Verilog),
    ("VHDL", NetlistFormat::Vhdl),
    ("tEDAx", NetlistFormat::Tedax),
];

/// Closing a document with unsaved changes.
enum Closing {
    /// xschem is asking its own question (`cdept::close`); the id of the request. The
    /// answer only comes if the document stays open.
    Xschem(u64),
    /// The user is being asked what to do with the changes, by CDePT itself: for a
    /// helper that has no `cdept::close`.
    Asking,
    /// xschem is saving; the id of the request.
    Saving(u64),
}

/// What the helper of this document can do for CDePT (procedures of `cdept.tcl`).
#[derive(Default)]
struct Abilities {
    /// The question was sent to the helper; the id of the request.
    asked: Option<u64>,
    /// `cdept::close`: close the document through xschem's own "save?" question.
    close: bool,
    /// `cdept::selection`: describe what is selected.
    selection: bool,
}

/// One instance selected in xschem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedInstance {
    pub name: String,
    /// The symbol as the schematic refers to it.
    pub symbol: String,
    /// The attribute string, e.g. `name=M1 model=nmos w=1u l=0.35u`.
    pub attributes: String,
}

/// What is selected in a schematic, as `cdept::selection` reports it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Selected {
    /// Selected objects of any kind.
    pub count: u32,
    pub instances: Vec<SelectedInstance>,
    pub wires: u32,
    pub texts: u32,
}

impl Selected {
    /// Read the list of keys and values `cdept::selection` returns.
    pub fn parse(text: &str) -> Result<Selected, String> {
        let items = tcl::parse_list(text)?;
        let mut selected = Selected::default();
        let number = |value: &str| value.trim().parse::<u32>().map_err(|e| e.to_string());
        for pair in items.chunks(2) {
            let [key, value] = pair else {
                return Err("a key without a value".into());
            };
            match key.as_str() {
                "count" => selected.count = number(value)?,
                "wires" => selected.wires = number(value)?,
                "texts" => selected.texts = number(value)?,
                "instances" => {
                    for instance in tcl::parse_list(value)? {
                        let mut fields = tcl::parse_list(&instance)?.into_iter();
                        selected.instances.push(SelectedInstance {
                            name: fields.next().unwrap_or_default(),
                            symbol: fields.next().unwrap_or_default(),
                            attributes: fields.next().unwrap_or_default(),
                        });
                    }
                }
                _ => {}
            }
        }
        Ok(selected)
    }
}

/// The design xschem shows, from its window title (`xschem - logic/buf/schematic/buf.sch*`):
/// the file as xschem names it, and whether it has unsaved changes.
fn shown_design(title: &str) -> (&str, bool) {
    let title = title.trim();
    let (name, modified) = match title.strip_suffix('*') {
        Some(name) => (name.trim_end(), true),
        None => (title, false),
    };
    let name = name.strip_prefix("xschem -").map_or(name, str::trim_start);
    (name, modified)
}

/// The title of a document tab: `own` (the document it was opened for), or, when xschem
/// has moved on to another design (after descending into an instance), that design's
/// name and where it was reached from: `buf (in tb_buf)`. A star marks unsaved changes.
fn tab_title(own: &str, opened: &Path, xschem_title: Option<&str>) -> String {
    let Some(title) = xschem_title else {
        return own.to_string();
    };
    let (shown, modified) = shown_design(title);
    let star = if modified { "*" } else { "" };
    let shown = Path::new(shown);
    // xschem names the file by the end of its path, as far as the library path goes.
    let same = shown.as_os_str().is_empty()
        || opened.ends_with(shown)
        || shown.file_name() == opened.file_name();
    if same {
        return format!("{own}{star}");
    }
    let stem = |path: &Path| {
        path.file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    format!("{} (in {}){star}", stem(shown), stem(opened))
}

/// The user's answer to "this document has unsaved changes".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseChoice {
    Save,
    Discard,
    Cancel,
}

pub struct SchematicView {
    id: DocId,
    doc: DocRef,
    /// The file the document was opened for.
    file: PathBuf,
    session: Session,
    /// Which helper the mirrored widgets come from.
    epoch: u64,
    mirror: Mirror,
    picture: Picture,
    /// Frame size last asked for.
    requested: Option<(u32, u32)>,
    /// Mouse buttons held down that xschem knows about, as a `state` mask.
    buttons: u32,
    /// The pointer is over the drawing, as far as xschem knows.
    inside: bool,
    /// Wheel movement not yet adding up to a notch.
    wheel: Vec2,
    pointer: Option<Pos2>,
    /// Where the drawing is on screen: its top-left corner in points, and the pixels
    /// per point. Frame pixel (x, y) is at `origin + (x, y) / scale`.
    placement: Option<(Pos2, f32)>,
    replies: Vec<Reply>,
    closing: Option<Closing>,
    abilities: Abilities,
    /// What is selected in xschem, as last reported.
    selection: Option<Selected>,
    /// Something happened that may have changed the selection.
    selection_stale: bool,
    /// The request for the selection that is under way.
    selection_asked: Option<u64>,
    /// A "save as" is under way: the request whose answer says xschem is done with it.
    saving_as: Option<u64>,
    /// What xschem knows of the desktop's clipboard.
    clipboard: clipboard::ClipboardSync,
    /// Cross-probing with layouts, Generate Layout (see `linking`).
    link: linking::SchematicLink,
}

impl SchematicView {
    /// Open `file` (the view `doc` of `project`) in an xschem of its own.
    pub fn open(
        id: DocId,
        doc: DocRef,
        file: PathBuf,
        project: &Project,
        ctx: &egui::Context,
    ) -> Self {
        let ctx = ctx.clone();
        let session = Session::open(SessionOptions {
            engine: EngineOptions::for_project(project),
            file: Some(file.clone()),
            wake: Arc::new(move || ctx.request_repaint()),
        });
        Self {
            id,
            doc,
            file,
            session,
            epoch: 0,
            mirror: Mirror::default(),
            picture: Picture::default(),
            requested: None,
            buttons: 0,
            inside: false,
            wheel: Vec2::ZERO,
            pointer: None,
            placement: None,
            replies: Vec::new(),
            closing: None,
            abilities: Abilities::default(),
            selection: None,
            selection_stale: false,
            selection_asked: None,
            saving_as: None,
            clipboard: clipboard::ClipboardSync::default(),
            link: linking::SchematicLink::default(),
        }
    }

    pub fn doc(&self) -> &DocRef {
        &self.doc
    }

    /// The file the document was opened for.
    pub fn file(&self) -> &Path {
        &self.file
    }

    /// What is selected in xschem, if anything is.
    pub fn selection(&self) -> Option<&Selected> {
        self.selection.as_ref().filter(|s| s.count > 0)
    }

    /// Ask xschem to save the design. The answer, fetched with
    /// [`take_reply`](Self::take_reply) and read with [`saved`](Self::saved), says
    /// whether it did: xschem may ask for a file name first, and the user may cancel.
    pub fn save(&self) -> u64 {
        self.session.tcl("xschem save; xschem get modified")
    }

    /// What the answer to [`save`](Self::save) means: saved, or why not.
    pub fn saved(answer: Result<String, String>) -> Result<(), String> {
        match answer {
            Ok(modified) if modified.trim() == "0" => Ok(()),
            Ok(_) => Err("the save was cancelled".into()),
            Err(error) => Err(error),
        }
    }

    /// xschem has changes that are not saved: it marks them with a star at the end of
    /// its window title.
    pub fn is_modified(&self) -> bool {
        self.mirror
            .title()
            .is_some_and(|t| t.trim_end().ends_with('*'))
    }

    /// The tab is about to be closed. Returns whether it may go right away; if not,
    /// the user is asked about the unsaved changes first: by xschem itself (its "save?"
    /// dialog, after which xschem ends and the tab closes), or by CDePT when the helper
    /// cannot do that.
    pub fn may_close(&mut self) -> bool {
        if !self.is_modified() || self.session.status() != Status::Running {
            return true;
        }
        if self.closing.is_none() {
            self.closing = Some(if self.abilities.close {
                Closing::Xschem(self.session.tcl("cdept::close"))
            } else {
                Closing::Asking
            });
        }
        false
    }

    /// Whether the user is being asked about unsaved changes by CDePT's own dialog.
    pub fn asks_to_close(&self) -> bool {
        matches!(self.closing, Some(Closing::Asking))
    }

    /// Whether closing is under way in any form.
    pub fn is_closing(&self) -> bool {
        self.closing.is_some()
    }

    /// Act on the answer to the question about unsaved changes.
    pub fn close_with(&mut self, choice: CloseChoice, ctx: &mut AppCtx) {
        match choice {
            CloseChoice::Save => self.closing = Some(Closing::Saving(self.save())),
            CloseChoice::Discard => {
                self.closing = None;
                ctx.request(Action::CloseTab(Tab::Document(self.id)));
            }
            CloseChoice::Cancel => self.closing = None,
        }
    }

    /// The question about unsaved changes, and the wait for xschem to save.
    fn closing(&mut self, ui: &Ui, ctx: &mut AppCtx) {
        match self.closing {
            None => {}
            // xschem answers only when the document stays open (the user cancelled, or
            // the save did not happen); otherwise it ends and the tab closes.
            Some(Closing::Xschem(id)) => match self.take_reply(id) {
                None if self.session.status() == Status::Running => {}
                None | Some(Ok(_)) => self.closing = None,
                Some(Err(error)) => {
                    tracing::warn!(target: "xschem", "cdept::close: {error}");
                    self.closing = Some(Closing::Asking);
                }
            },
            Some(Closing::Asking) => {
                let mut choice = None;
                let modal = egui::Modal::new(egui::Id::new(("xschem-close", self.id)));
                let response = modal.show(ui.ctx(), |ui| {
                    ui.set_max_width(420.0);
                    ui.heading("Unsaved changes");
                    ui.label(format!(
                        "{} has changes that are not saved.",
                        self.doc.title()
                    ));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        for (label, answer) in [
                            ("Save", CloseChoice::Save),
                            ("Discard", CloseChoice::Discard),
                            ("Cancel", CloseChoice::Cancel),
                        ] {
                            if ui.button(label).clicked() {
                                choice = Some(answer);
                            }
                        }
                    });
                });
                if response.should_close() {
                    choice.get_or_insert(CloseChoice::Cancel);
                }
                if let Some(choice) = choice {
                    self.close_with(choice, ctx);
                }
            }
            Some(Closing::Saving(id)) => match self.take_reply(id).map(Self::saved) {
                None if self.session.status() == Status::Running => {}
                None => self.closing = None,
                Some(Ok(())) => {
                    self.closing = None;
                    ctx.request(Action::CloseTab(Tab::Document(self.id)));
                }
                Some(Err(error)) => {
                    self.closing = None;
                    ctx.error(format!("{} was not saved: {error}", self.doc.title()));
                }
            },
        }
    }

    /// The screen position (in points) of the middle of frame pixel (`x`, `y`).
    pub fn frame_to_screen(&self, x: f32, y: f32) -> Option<Pos2> {
        let (origin, scale) = self.placement?;
        Some(origin + Vec2::new(x + 0.5, y + 0.5) / scale)
    }

    /// Whether a frame of the running helper is on screen.
    pub fn has_frame(&self) -> bool {
        self.session.status() == Status::Running
            && self.picture.texture().is_some()
            && self.placement.is_some()
    }

    /// Size of the frame the helper last sent, in pixels.
    pub fn frame_size(&self) -> Option<[u32; 2]> {
        self.session.frame(|frame| [frame.width(), frame.height()])
    }

    /// The colour of a frame pixel, as R G B A.
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        let mut out = Vec::new();
        let copied = self
            .session
            .frame(|frame| frame.copy(frame::Rect::new(x, y, 1, 1), &mut out))?;
        (!copied.is_empty()).then(|| [out[0], out[1], out[2], out[3]])
    }

    /// Check that the drawing on screen (in a screenshot of the window) is the frame,
    /// pixel for pixel: nothing scaled, nothing shifted.
    pub fn compare_with_screen(&self, screen: &egui::ColorImage) -> Result<(), String> {
        let (origin, scale) = self.placement.ok_or("the schematic is not on screen")?;
        let (left, top) = (
            (origin.x * scale).round() as usize,
            (origin.y * scale).round() as usize,
        );
        let mut pixels = Vec::new();
        let size = self
            .session
            .frame(|frame| {
                frame.copy(frame.full(), &mut pixels);
                [frame.width() as usize, frame.height() as usize]
            })
            .ok_or("there is no frame yet")?;
        if left + size[0] > screen.size[0] || top + size[1] > screen.size[1] {
            return Err(format!(
                "the frame ({}x{} at {left},{top}) does not fit the window ({}x{})",
                size[0], size[1], screen.size[0], screen.size[1]
            ));
        }
        let mut different = 0;
        let mut first = None;
        for y in 0..size[1] {
            for x in 0..size[0] {
                let frame = &pixels[(y * size[0] + x) * 4..][..3];
                let shown = screen.pixels[(top + y) * screen.size[0] + left + x].to_array();
                // The window system may dither by one step.
                if frame.iter().zip(shown).any(|(a, b)| a.abs_diff(b) > 2) {
                    different += 1;
                    first.get_or_insert((x, y));
                }
            }
        }
        match first {
            Some((x, y)) => Err(format!(
                "{different} of {} pixels on screen differ from the frame, the first at {x},{y}",
                size[0] * size[1]
            )),
            None => Ok(()),
        }
    }

    /// Evaluate Tcl in xschem; the answer is fetched with [`take_reply`](Self::take_reply).
    pub fn tcl(&self, script: &str) -> u64 {
        self.session.tcl(script)
    }

    pub fn take_reply(&mut self, id: u64) -> Option<Result<String, String>> {
        self.replies.extend(self.session.take_replies());
        let at = self.replies.iter().position(|r| r.id == id)?;
        Some(self.replies.remove(at).result)
    }

    pub fn status(&self) -> Status {
        self.session.status()
    }

    /// Report what the user did in a mirrored widget.
    pub fn send(&mut self, event: UiEvent) {
        self.session.ui(event);
        self.selection_stale = true;
    }

    /// Choose an entry of xschem's menu bar by its labels, e.g. `["File", "Save"]`.
    pub fn choose_menu(&mut self, labels: &[&str]) -> bool {
        let event = self.mirror.menu_entry(labels);
        let found = event.is_some();
        event.into_iter().for_each(|e| self.send(e));
        found
    }

    /// Every entry of xschem's menu bar, as `Menu / Entry`.
    pub fn menu_paths(&self) -> Vec<String> {
        self.mirror.menu_paths()
    }

    /// Press the button with this text in the topmost dialog.
    pub fn press_dialog_button(&mut self, text: &str) -> bool {
        let event = self.mirror.dialog_button(text);
        let found = event.is_some();
        event.into_iter().for_each(|e| self.send(e));
        found
    }

    /// Type into a dialog's text field (see [`Mirror::dialog_type`]).
    pub fn type_in_dialog(&mut self, target: &str, text: &str) -> Result<(), String> {
        let event = self.mirror.dialog_type(target, text)?;
        event.into_iter().for_each(|e| self.send(e));
        Ok(())
    }

    /// The mode of the library picker that waits for an answer, if one does.
    pub fn picker_mode(&self) -> Option<String> {
        self.mirror.picker_mode().map(str::to_string)
    }

    /// Answer the library picker that waits (`None` cancels). Returns whether one did.
    pub fn answer_picker(&mut self, file: Option<&Path>) -> bool {
        let event = self.mirror.answer_picker(file);
        let found = event.is_some();
        event.into_iter().for_each(|e| self.send(e));
        found
    }

    /// The picture pane of the widget at `path` in a dialog: its size in pixels, the
    /// size asked for, and how many colours it has (counted up to 16).
    pub fn dialog_pane(&self, path: &str) -> Option<([u32; 2], Option<[u32; 2]>, usize)> {
        self.mirror.pane_info(path)
    }

    /// Titles of the open dialogs.
    pub fn dialog_titles(&self) -> Vec<String> {
        self.mirror
            .window_titles()
            .into_iter()
            .map(str::to_string)
            .collect()
    }

    /// Take in what the helper reported since the last frame.
    fn sync(&mut self, ctx: &egui::Context) {
        let epoch = self.session.epoch();
        if epoch != self.epoch {
            // A new helper: the widgets of the old one are gone with it.
            self.epoch = epoch;
            self.mirror = Mirror::default();
            self.buttons = 0;
            self.inside = false;
            self.selection = None;
            self.selection_asked = None;
            self.closing = None;
            self.clipboard.forget();
            // Which of CDePT's procedures this helper has.
            let ask = "list [llength [info commands ::cdept::close]] \
                       [llength [info commands ::cdept::selection]]";
            self.abilities = Abilities {
                asked: Some(self.session.tcl(ask)),
                ..Default::default()
            };
        }
        for update in self.session.take_updates() {
            self.mirror.apply(update, ctx);
        }
        for update in self.session.take_panes() {
            self.mirror.apply_pane(update, ctx);
        }
        // xschem's Tcl may ask for the desktop's clipboard (`clipboard get`) and its
        // dialogs paste it: tell it what the clipboard holds whenever that may have
        // changed: a dialog opened, the window got the keyboard back.
        let focus_back = ctx.input(|i| {
            i.events
                .iter()
                .any(|e| matches!(e, Event::WindowFocused(true)))
        });
        if self.mirror.take_opened_window() || focus_back {
            self.clipboard.refresh(ctx);
        }
        let pasted: Vec<String> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    Event::Paste(text) if !text.is_empty() => Some(text.clone()),
                    _ => None,
                })
                .collect()
        });
        let mut news: Vec<UiEvent> = pasted
            .iter()
            .filter_map(|text| self.clipboard.pasted(text))
            .collect();
        news.extend(self.clipboard.poll(ctx));
        for event in news {
            self.send(event);
        }
        self.replies.extend(self.session.take_replies());

        if let Some(id) = self.abilities.asked
            && let Some(answer) = self.take_reply(id)
        {
            let has: Vec<bool> = answer
                .unwrap_or_default()
                .split_whitespace()
                .map(|n| n != "0")
                .collect();
            self.abilities = Abilities {
                asked: None,
                close: has.first() == Some(&true),
                selection: has.get(1) == Some(&true),
            };
        }
        if let Some(id) = self.selection_asked
            && let Some(answer) = self.take_reply(id)
        {
            self.selection_asked = None;
            match answer.and_then(|text| Selected::parse(&text)) {
                Ok(selected) => {
                    self.selection = Some(selected);
                    self.selection_changed();
                }
                Err(error) => tracing::debug!(target: "xschem", "cdept::selection: {error}"),
            }
        }
        // One question at a time; what happened meanwhile is asked about afterwards.
        if self.selection_stale
            && self.selection_asked.is_none()
            && self.abilities.selection
            && self.session.status() == Status::Running
            && !self.mirror.has_modal()
        {
            self.selection_stale = false;
            self.selection_asked = Some(self.session.tcl("cdept::selection"));
        }

        let excess = self.replies.len().saturating_sub(KEPT_REPLIES);
        self.replies.drain(..excess);
    }

    /// Pass the mouse and the keyboard on to xschem. `over`: the pointer is on the
    /// drawing and nothing covers it. `keys`: the drawing has the keyboard.
    fn input(&mut self, events: &[Event], modifiers: Modifiers, over: bool, keys: bool) {
        let Some((origin, scale)) = self.placement else {
            return;
        };
        let to_frame = |pos: Pos2| {
            let p = (pos - origin) * scale;
            (p.x.floor() as i32, p.y.floor() as i32)
        };
        for event in events {
            match event {
                Event::PointerMoved(pos) => {
                    self.pointer = Some(*pos);
                    let (x, y) = to_frame(*pos);
                    let state = keys::state_mask(modifiers, self.buttons);
                    if over && !self.inside {
                        self.inside = true;
                        self.session.input(Input::Enter { x, y, state });
                    }
                    // A drag goes on outside the drawing, as with a grabbed pointer.
                    if over || self.buttons != 0 {
                        self.session.input(Input::Motion { x, y, state });
                    } else if self.inside {
                        self.inside = false;
                        self.session.input(Input::Leave);
                    }
                }
                Event::PointerGone => {
                    if self.inside && self.buttons == 0 {
                        self.inside = false;
                        self.session.input(Input::Leave);
                    }
                }
                Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    modifiers,
                } => {
                    let Some(number) = keys::button_number(*button) else {
                        continue;
                    };
                    let (x, y) = to_frame(*pos);
                    let state = keys::state_mask(*modifiers, self.buttons);
                    let mask = keys::button_mask(number);
                    if *pressed && over {
                        self.session.input(Input::ButtonPress {
                            button: number,
                            x,
                            y,
                            state,
                        });
                        self.buttons |= mask;
                    } else if !*pressed && self.buttons & mask != 0 {
                        self.session.input(Input::ButtonRelease {
                            button: number,
                            x,
                            y,
                            state,
                        });
                        self.buttons &= !mask;
                        self.selection_stale = true;
                    }
                }
                Event::MouseWheel {
                    unit,
                    delta,
                    modifiers,
                    ..
                } if over => {
                    self.wheel += keys::wheel_notches(*unit, *delta);
                    let (x, y) = self.pointer.map_or((0, 0), to_frame);
                    let state = keys::state_mask(*modifiers, self.buttons);
                    for button in keys::take_wheel_buttons(&mut self.wheel) {
                        self.session.input(Input::ButtonPress {
                            button,
                            x,
                            y,
                            state,
                        });
                        self.session.input(Input::ButtonRelease {
                            button,
                            x,
                            y,
                            state,
                        });
                    }
                }
                Event::Text(text) if keys => {
                    for c in text.chars() {
                        self.key(keys::char_symbol(c), modifiers, true);
                        self.key(keys::char_symbol(c), modifiers, false);
                    }
                }
                Event::Key {
                    key,
                    pressed,
                    modifiers,
                    ..
                } if keys => {
                    // Ctrl+Z and the like are CDePT's: xschem would zoom out.
                    if let Some(action) = keys::edit_shortcut(*key, *modifiers) {
                        if *pressed {
                            self.undo_redo(action);
                        }
                        continue;
                    }
                    // Keys that type a character arrive as text, with the right case.
                    if keys::types_text(*key, *modifiers) {
                        continue;
                    }
                    if let Some(symbol) = keys::key_symbol(*key, *modifiers) {
                        self.key(symbol, *modifiers, *pressed);
                    }
                }
                // egui turns Ctrl+C, Ctrl+X and Ctrl+V into these.
                Event::Copy | Event::Cut | Event::Paste(_) if keys => {
                    let letter = match event {
                        Event::Copy => 'c',
                        Event::Cut => 'x',
                        _ => 'v',
                    };
                    let letter = if modifiers.shift {
                        letter.to_ascii_uppercase()
                    } else {
                        letter
                    };
                    let modifiers = modifiers | Modifiers::CTRL;
                    self.key(keys::char_symbol(letter), modifiers, true);
                    self.key(keys::char_symbol(letter), modifiers, false);
                }
                _ => {}
            }
        }
    }

    fn key(&mut self, keysym: u32, modifiers: Modifiers, pressed: bool) {
        self.selection_stale |= !pressed;
        let (origin, scale) = self.placement.unwrap_or((Pos2::ZERO, 1.0));
        let p = self
            .pointer
            .map_or(Vec2::ZERO, |pos| (pos - origin) * scale);
        let (x, y) = (p.x.floor() as i32, p.y.floor() as i32);
        let state = keys::state_mask(modifiers, self.buttons);
        self.session.input(if pressed {
            Input::KeyPress {
                keysym,
                x,
                y,
                state,
            }
        } else {
            Input::KeyRelease {
                keysym,
                x,
                y,
                state,
            }
        });
    }

    /// What the engine is doing, when it is not simply running.
    fn status_overlay(&self, ui: &mut Ui, rect: Rect) {
        let status = self.session.status();
        let (text, failed) = match &status {
            Status::Running | Status::Closed => return,
            Status::Starting => ("Starting xschem…".to_string(), false),
            Status::Restarting(reason) => (format!("{reason}. Starting it again…"), false),
            Status::Failed(reason) => (reason.clone(), true),
        };
        if !failed {
            let at = rect.left_top() + Vec2::splat(8.0);
            let galley = ui.painter().layout_no_wrap(
                text,
                egui::TextStyle::Body.resolve(ui.style()),
                Color32::from_gray(220),
            );
            let back = Rect::from_min_size(at, galley.size()).expand(4.0);
            let painter = ui.painter_at(rect);
            painter.rect_filled(back, 4.0, Color32::from_black_alpha(170));
            painter.galley(at, galley, Color32::from_gray(220));
            return;
        }
        let panel = Rect::from_center_size(rect.center(), rect.size().min(Vec2::new(520.0, 160.0)));
        ui.scope_builder(egui::UiBuilder::new().max_rect(panel), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.strong("The xschem engine stopped");
                    ui.label(text);
                    ui.add_space(4.0);
                    if ui.button("Restart").clicked() {
                        self.session.restart();
                    }
                });
            });
        });
    }
}

impl View for SchematicView {
    fn title(&self, _ctx: &AppCtx) -> String {
        tab_title(&self.doc.title(), &self.file, self.mirror.title())
    }

    fn ui(&mut self, ui: &mut Ui, ctx: &mut AppCtx) {
        self.sync(ui.ctx());
        if self.session.status() == Status::Closed {
            ctx.request(Action::CloseTab(Tab::Document(self.id)));
        }

        // The status bar takes one line at the bottom; the drawing gets the rest.
        let mut area = ui.available_rect_before_wrap();
        if self.mirror.has_status() {
            let height = ui.spacing().interact_size.y + 2.0;
            let line = Rect::from_min_max(
                Pos2::new(area.left(), area.bottom() - height),
                area.right_bottom(),
            );
            area.max.y = line.top();
            let line = egui::UiBuilder::new().max_rect(line.shrink2(Vec2::new(6.0, 0.0)));
            let events = ui
                .scope_builder(line, |ui| self.mirror.status_line(ui))
                .inner;
            for event in events {
                self.send(event);
            }
        }
        let response = ui.allocate_rect(area, Sense::click_and_drag());

        // Whole pixels inside the area: one frame pixel is one screen pixel.
        let scale = ui.ctx().pixels_per_point();
        let left = (area.left() * scale).ceil();
        let top = (area.top() * scale).ceil();
        let width = ((area.right() * scale).floor() - left).max(1.0) as u32;
        let height = ((area.bottom() * scale).floor() - top).max(1.0) as u32;
        let origin = Pos2::new(left, top) / scale;
        self.placement = Some((origin, scale));
        if self.requested != Some((width, height)) {
            self.requested = Some((width, height));
            self.session.resize(width, height);
        }

        let name = format!("xschem-frame-{}", self.id.0);
        self.picture.update(&self.session, ui.ctx(), &name);
        let painter = ui.painter_at(area);
        painter.rect_filled(area, 0.0, Color32::BLACK);
        if let Some((texture, pixels)) = self.picture.texture() {
            let size = Vec2::new(pixels[0] as f32, pixels[1] as f32) / scale;
            painter.image(
                texture.id(),
                Rect::from_min_size(origin, size),
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }

        let modal = self.mirror.has_modal();
        let running = self.session.status() == Status::Running;
        let over = response.contains_pointer() && running && !modal;
        if over {
            ui.ctx().set_cursor_icon(self.mirror.cursor());
            // The keyboard follows the pointer, as in xschem, unless something else
            // (a text field) is using it.
            if ui.memory(|m| m.focused().is_none()) || response.clicked() || response.drag_started()
            {
                response.request_focus();
            }
        }
        let keys = over && response.has_focus();
        if response.has_focus() {
            // Keep egui from moving the focus with keys that belong to xschem.
            let filter = EventFilter {
                tab: true,
                horizontal_arrows: true,
                vertical_arrows: true,
                escape: true,
            };
            ui.memory_mut(|m| m.set_focus_lock_filter(response.id, filter));
        }
        if running && !modal {
            let (events, modifiers) = ui.input(|i| (i.events.clone(), i.modifiers));
            self.input(&events, modifiers, over, keys);
        } else if running && self.buttons != 0 {
            // A press can open a dialog (a double click opens the attribute dialog)
            // while the button is still down. Its release must still reach xschem, as
            // it would with X: otherwise the button stays down for xschem and the
            // first movement after the dialog starts selecting by area.
            let (events, modifiers) = ui.input(|i| (i.events.clone(), i.modifiers));
            let releases: Vec<Event> = events
                .into_iter()
                .filter(|e| matches!(e, Event::PointerButton { pressed: false, .. }))
                .collect();
            self.input(&releases, modifiers, false, false);
        }

        if modal {
            painter.rect_filled(area, 0.0, Color32::from_black_alpha(90));
        }
        self.status_overlay(ui, area);

        let salt = egui::Id::new(("xschem", self.id));
        let mut events = self.mirror.popup(ui, salt, self.pointer);
        // The library picker shows the view chosen, as the Inspector does.
        let (project, engines) = (ctx.project.as_ref(), &mut ctx.engines);
        let mut picture = |ui: &mut Ui, file: &Path| {
            if let Some(project) = project {
                engines.picker_preview(ui, project, file);
            }
        };
        events.extend(
            self.mirror
                .windows(ui.ctx(), salt, area, project, &mut picture),
        );
        for event in events {
            self.send(event);
        }
        for file in self.mirror.take_opened() {
            ctx.request(Action::OpenFile(file));
        }
        if self.mirror.take_written() {
            ctx.request(Action::ReloadProject);
        }
        // After "save as" the libraries may hold a new cell: look again, once xschem
        // has written the file.
        if self.mirror.take_saved_as() {
            self.saving_as = Some(self.session.tcl("expr 1"));
        }
        if let Some(id) = self.saving_as
            && self.take_reply(id).is_some()
        {
            self.saving_as = None;
            ctx.request(Action::ReloadProject);
        }
        self.closing(ui, ctx);
    }

    fn menus(&mut self, ui: &mut Ui, ctx: &mut AppCtx) {
        for event in self.mirror.menus(ui) {
            self.send(event);
        }
        self.cell_menu(ui, ctx);
    }

    fn toolbar(&mut self, ui: &mut Ui, _ctx: &mut AppCtx) {
        for event in self.mirror.toolbar(ui) {
            self.send(event);
        }
    }

    fn edit_caps(&self) -> EditCaps {
        let ready = self.session.status() == Status::Running && !self.mirror.has_modal();
        EditCaps {
            undo: ready,
            redo: ready,
            ..EditCaps::default()
        }
    }

    fn edit(&mut self, action: EditAction, _ctx: &mut AppCtx) {
        self.undo_redo(action);
    }

    fn file_menu(&mut self, ui: &mut Ui, _ctx: &mut AppCtx) {
        let ready = self.session.status() == Status::Running && !self.mirror.has_modal();
        ui.add_enabled_ui(ready, |ui| {
            ui.menu_button("Export", |ui| {
                // xschem's own commands; it asks for the destination file.
                for (label, command) in EXPORTS {
                    if ui.button(label).clicked() {
                        self.session.tcl(command);
                        self.selection_stale = true;
                        ui.close();
                    }
                }
            })
            .response
            .on_hover_text(
                "Write the drawing as an image (same as Schematic > File > Image export)",
            );
        });
        ui.separator();
    }
}

/// File > Export: xschem's "Image export" commands.
const EXPORTS: [(&str, &str); 4] = [
    ("PNG Image…", "xschem print png"),
    ("SVG Image…", "xschem print svg"),
    ("PDF/PS Document…", "xschem print pdf"),
    ("PDF/PS Document, full drawing…", "xschem print pdf_full"),
];

impl SchematicView {
    /// Undo or redo with xschem's own commands, as its Edit menu does.
    fn undo_redo(&mut self, action: EditAction) {
        let command = match action {
            EditAction::Undo => "xschem undo; xschem redraw",
            EditAction::Redo => "xschem redo; xschem redraw",
            _ => return,
        };
        if self.session.status() == Status::Running && !self.mirror.has_modal() {
            self.session.tcl(command);
            self.selection_stale = true;
        }
    }

    /// CDePT's own menu for the cell of the document, next to xschem's.
    fn cell_menu(&mut self, ui: &mut Ui, ctx: &mut AppCtx) {
        if self.doc.file.is_some() {
            return; // not a cell of a library
        }
        let (library, cell) = (&self.doc.library.clone(), &self.doc.cell.clone());
        ui.menu_button("Cell", |ui| {
            let has_schematic = ctx
                .project
                .as_ref()
                .and_then(|p| p.library(library)?.cell(cell))
                .is_some_and(|c| crate::app::schematic_file(c).is_some());
            ui.add_enabled_ui(has_schematic, |ui| {
                ui.menu_button("Netlist to View", |ui| {
                    for (label, format) in NETLIST_FORMATS {
                        if ui.button(label).clicked() {
                            ctx.request(Action::NetlistToView {
                                library: library.clone(),
                                cell: cell.clone(),
                                format,
                            });
                        }
                    }
                })
                .response
                .on_hover_text(
                    "Write the netlist of the cell's schematic, as saved, into its netlist view",
                );
            });
            if ui.button("Select in Libraries").clicked() {
                ctx.selection = Selection::view(library, cell, &self.doc.view);
                ctx.request(Action::OpenTab(Tab::Libraries));
            }
            self.link_menu(ui, ctx);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_titles_follow_the_design_shown() {
        let opened = Path::new("/p/libs/bench/tb_buf/schematic/tb_buf.sch");
        let own = "tb_buf (schematic)";
        // Before xschem has said anything.
        assert_eq!(tab_title(own, opened, None), own);
        // The file the tab was opened for, as xschem names it, clean and changed.
        let title = "xschem - bench/tb_buf/schematic/tb_buf.sch";
        assert_eq!(tab_title(own, opened, Some(title)), own);
        assert_eq!(
            tab_title(own, opened, Some(&format!("{title}*"))),
            "tb_buf (schematic)*"
        );
        // After descending into an instance.
        let inside = "xschem - logic/buf/schematic/buf.sch";
        assert_eq!(tab_title(own, opened, Some(inside)), "buf (in tb_buf)");
        assert_eq!(
            tab_title(own, opened, Some(&format!("{inside}*"))),
            "buf (in tb_buf)*"
        );
        // A title that names no file leaves the tab alone.
        assert_eq!(tab_title(own, opened, Some("xschem - ")), own);
        // A file opened by path, shown under its absolute name.
        let loose = Path::new("/tmp/scratch.sch");
        assert_eq!(
            tab_title("scratch.sch", loose, Some("xschem - /tmp/scratch.sch*")),
            "scratch.sch*"
        );
    }

    #[test]
    fn the_selection_report_is_read() {
        let text = "count 3 instances {{x1 logic/buf/symbol/buf.sym name=x1} \
                    {M1 nmos4.sym {name=M1 model=nmos w=1u l=0.35u}}} wires 1 texts 0";
        let selected = Selected::parse(text).unwrap();
        assert_eq!(selected.count, 3);
        assert_eq!(selected.wires, 1);
        assert_eq!(selected.texts, 0);
        assert_eq!(selected.instances.len(), 2);
        assert_eq!(selected.instances[0].name, "x1");
        assert_eq!(selected.instances[0].symbol, "logic/buf/symbol/buf.sym");
        assert_eq!(
            selected.instances[1].attributes,
            "name=M1 model=nmos w=1u l=0.35u"
        );

        let nothing = Selected::parse("count 0 instances {} wires 0 texts 0").unwrap();
        assert_eq!(nothing, Selected::default());
        assert!(Selected::parse("count many").is_err());
        assert!(Selected::parse("count").is_err());
    }
}
