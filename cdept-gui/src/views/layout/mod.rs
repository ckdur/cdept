//! A layout open in a tab, drawn and edited by KLayout.
//!
//! KLayout runs in a helper process (`libcdept::klayout::Session`, docs/KLAYOUT.md). The
//! tab shows the helper's picture as a texture, one frame pixel per screen pixel, sends
//! it the mouse and the keyboard, and draws what a KLayout window would have around the
//! picture with egui: the tool bar, the layer list, the cell tree, the selection and the
//! status line.

mod checks;
mod inplace;
mod keys;
mod linking;
mod panels;
mod pcell;
mod place;
mod properties;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{Color32, Event, EventFilter, Modifiers, Pos2, Rect, Sense, Ui, Vec2};
use libcdept::Project;
use libcdept::klayout::Event as KEvent;
use libcdept::klayout::protocol::{Button, Input, NoticeLevel, method};
use libcdept::klayout::session::{Session, SessionOptions, Status};
use libcdept::layout::RefTable;
use libcdept::layout::refs::{self, SaveReport};
use serde_json::{Value, json};

pub use self::inplace::coordinate;
use self::keys::Shortcut;
pub use self::properties::Replacement;
use super::{CloseChoice, EditAction, EditCaps, Picture, View};
use crate::dock::{DocId, DocRef, Tab};
use crate::state::{Action, AppCtx, Selection};

/// How long a message from the editor stays over the picture.
const NOTICE_TIME: Duration = Duration::from_secs(6);
/// Notices kept for the test driver.
const KEPT_NOTICES: usize = 64;
/// Answers to calls kept for whoever asked (the test driver).
const KEPT_REPLIES: usize = 64;
/// Two presses this close in time and place are a double click.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);
const DOUBLE_CLICK_PIXELS: f32 = 6.0;
/// Levels at and above this are shown as "all".
const ALL_LEVELS: u32 = 16;
/// How long after a save the helper is asked for a sign of life.
/// The most often the helper is nudged.
const NUDGE_EVERY: Duration = Duration::from_millis(250);
const REFRESH_DELAY: Duration = Duration::from_millis(300);

/// The modes of the editor: KLayout's name and the label in the tool bar.
pub const MODES: [(&str, &str, &str); 9] = [
    ("select", "Select", "Select shapes and instances"),
    ("move", "Move", "Move shapes and instances"),
    ("box", "Box", "Draw boxes"),
    ("polygon", "Polygon", "Draw polygons (double click ends)"),
    ("path", "Path", "Draw paths (double click ends)"),
    ("text", "Text", "Place text labels"),
    (
        "instance",
        "Instance",
        "Place the cell chosen with Layout > Place Cell",
    ),
    ("ruler", "Ruler", "Measure distances"),
    ("partial", "Partial", "Edit vertices and edges"),
];

/// What to do with the answer of a call.
#[derive(Debug, Clone)]
enum Pending {
    /// Nothing, but an error is shown.
    Quiet,
    /// Remember the outcome for whoever asked ([`LayoutView::take_done`]).
    Save,
    Place,
    Export(PathBuf),
}

/// Closing a document with unsaved changes.
enum Closing {
    /// The user is being asked what to do with the changes.
    Asking,
    /// KLayout is saving; the id of the request.
    Saving(u64),
}

/// The cell tree as last reported.
#[derive(Debug, Clone, Default)]
struct Cells {
    top: String,
    /// The cell shown, as a path of cell names from the top.
    path: Vec<String>,
    cells: Vec<libcdept::klayout::protocol::CellNode>,
}

/// What is selected, as last reported.
#[derive(Debug, Clone, Default)]
struct Selected {
    count: usize,
    bbox: Option<[f64; 4]>,
    items: Vec<libcdept::klayout::protocol::SelItem>,
}

/// The form that adds a layer.
struct AddLayer {
    layer: i32,
    datatype: i32,
    name: String,
}

/// Which side panels are open.
struct Sections {
    layers: bool,
    cells: bool,
    selection: bool,
}

pub struct LayoutView {
    id: DocId,
    doc: DocRef,
    /// The file the document was opened for.
    file: PathBuf,
    editable: bool,
    session: Session,
    /// Which helper the state shown comes from.
    epoch: u64,
    /// The project, as it was when the view opened (for the sidecar after a save).
    project: Project,
    /// The file and everything it references, as the helper has it.
    refs: RefTable,
    picture: Picture,
    /// Frame size last asked for.
    requested: Option<(u32, u32)>,
    /// Mouse buttons held down that KLayout knows about.
    held: Vec<Button>,
    /// The pointer is over the drawing, as far as KLayout knows.
    inside: bool,
    /// Wheel movement not yet adding up to a notch.
    wheel: Vec2,
    pointer: Option<Pos2>,
    last_press: Option<(Instant, Pos2, Button)>,
    /// Where the drawing is on screen: its top-left corner in points, and the pixels
    /// per point. Frame pixel (x, y) is at `origin + (x, y) / scale`.
    placement: Option<(Pos2, f32)>,
    /// Answers nobody has claimed yet.
    replies: Vec<libcdept::klayout::Reply>,
    pending: HashMap<u64, Pending>,
    /// Outcomes of calls with a [`Pending`] that wants to be asked for.
    done: HashMap<u64, Result<String, String>>,
    closing: Option<Closing>,

    layers: Vec<libcdept::klayout::protocol::Layer>,
    cells: Option<Cells>,
    selection: Selected,
    /// The pointer in µm, the mode and KLayout's hint line.
    at: (Option<f64>, Option<f64>),
    mode: String,
    hint: String,
    dirty: bool,
    can_undo: bool,
    can_redo: bool,
    notice: Option<(NoticeLevel, String, Instant)>,
    notices: Vec<(NoticeLevel, String)>,

    side: bool,
    sections: Sections,
    add_layer: AddLayer,
    /// Hierarchy levels shown, at most.
    levels: u32,
    place: Option<place::PlacePicker>,
    /// The instance (`index`, `key` of the properties) that the cell chosen in
    /// [`place`](Self::place) replaces, when it was opened by "Change…".
    replace_target: Option<(usize, String)>,
    /// The destination of an export is being asked for.
    export_dialog: Option<std::sync::mpsc::Receiver<Option<PathBuf>>>,
    /// Files to open in tabs of their own (external cells).
    open_requests: Vec<PathBuf>,
    /// The helper started and has not yet been asked to show the whole layout.
    fit_pending: bool,
    /// When to ask the helper for a sign of life, so that it reports what changed
    /// (see [`LayoutView::nudge`]).
    refresh_at: Option<Instant>,
    nudged: Option<Instant>,
    /// DRC and LVS runs and their results.
    checks: checks::Checks,
    /// Editing in place (see `inplace`): the external cell edited, the external files
    /// with unsaved changes, the locks last sent (with the helper's epoch), and the
    /// files saves wrote that other tabs have not heard of yet.
    in_place: Option<libcdept::klayout::protocol::InPlace>,
    edited: Vec<PathBuf>,
    locks_sent: Option<(u64, Vec<PathBuf>)>,
    written: Vec<PathBuf>,
    /// The properties dialog and "Place Library Cell…", while open.
    properties: Option<properties::PropertiesDialog>,
    library_picker: Option<pcell::LibraryPicker>,
    /// The link with the schematic (see `linking`).
    link: linking::LinkState,
}

impl LayoutView {
    /// Open `file` (the view `doc` of `project`) in a KLayout of its own. `editable` is
    /// false for the layouts of read-only libraries.
    pub fn open(
        id: DocId,
        doc: DocRef,
        file: PathBuf,
        editable: bool,
        project: &Project,
        ctx: &egui::Context,
    ) -> Self {
        let refs = refs::resolve(project, &file);
        let wake = ctx.clone();
        let session = Session::open(SessionOptions::for_project(
            project,
            refs.clone(),
            editable,
            Arc::new(move || wake.request_repaint()),
        ));
        let mut view = Self {
            id,
            doc,
            file,
            editable,
            session,
            epoch: 0,
            project: project.clone(),
            refs,
            picture: Picture::default(),
            requested: None,
            held: Vec::new(),
            inside: false,
            wheel: Vec2::ZERO,
            pointer: None,
            last_press: None,
            placement: None,
            replies: Vec::new(),
            pending: HashMap::new(),
            done: HashMap::new(),
            closing: None,
            layers: Vec::new(),
            cells: None,
            selection: Selected::default(),
            at: (None, None),
            mode: String::new(),
            hint: String::new(),
            dirty: false,
            can_undo: false,
            can_redo: false,
            notice: None,
            notices: Vec::new(),
            side: true,
            sections: Sections {
                layers: true,
                cells: true,
                selection: true,
            },
            add_layer: AddLayer {
                layer: 1,
                datatype: 0,
                name: String::new(),
            },
            levels: ALL_LEVELS,
            place: None,
            export_dialog: None,
            open_requests: Vec::new(),
            fit_pending: true,
            refresh_at: None,
            nudged: None,
            checks: checks::Checks::default(),
            in_place: None,
            edited: Vec::new(),
            locks_sent: None,
            written: Vec::new(),
            replace_target: None,
            properties: None,
            library_picker: None,
            link: linking::LinkState::new(),
        };
        view.find_link_source();
        view
    }

    /// The file the document was opened for.
    pub fn file(&self) -> &Path {
        &self.file
    }

    pub fn is_modified(&self) -> bool {
        self.dirty
    }

    pub fn is_running(&self) -> bool {
        self.session.status() == Status::Running
    }

    /// Why the helper stopped for good, if it did.
    pub fn failure(&self) -> Option<String> {
        match self.session.status() {
            Status::Failed(reason) => Some(reason),
            _ => None,
        }
    }

    pub fn mode(&self) -> &str {
        &self.mode
    }

    // ----- calls -----

    fn call(&mut self, method: &str, params: Value, pending: Pending) -> u64 {
        let id = self.session.call(method, params);
        self.pending.insert(id, pending);
        id
    }

    /// A call whose answer is only looked at for errors.
    fn quiet(&mut self, method: &str, params: Value) {
        self.call(method, params, Pending::Quiet);
    }

    /// A call whose answer is for [`take_reply`](Self::take_reply).
    pub fn raw_call(&mut self, method: &str, params: Value) -> u64 {
        self.session.call(method, params)
    }

    /// The answer to a [`raw_call`](Self::raw_call), as text.
    pub fn take_reply(&mut self, id: u64) -> Option<Result<String, String>> {
        self.pump();
        let at = self.replies.iter().position(|r| r.id == id)?;
        let reply = self.replies.remove(at);
        Some(reply.result.map(|value| match value {
            Value::String(text) => text,
            other => other.to_string(),
        }))
    }

    /// The outcome of a [`save`](Self::save), [`place`](Self::place) or
    /// [`export`](Self::export): the answer, once it came, after it has been dealt with.
    pub fn take_done(&mut self, id: u64) -> Option<Result<String, String>> {
        self.pump();
        self.done.remove(&id)
    }

    /// Take in the answers of the helper.
    fn pump(&mut self) {
        for reply in self.session.take_replies() {
            let Some(pending) = self.pending.remove(&reply.id) else {
                self.replies.push(reply);
                continue;
            };
            let id = reply.id;
            let result = match (reply.result, &pending) {
                (Err(error), _) => Err(error),
                (Ok(value), Pending::Save) => self.saved(value),
                (Ok(_), Pending::Place) => Ok("placed".to_string()),
                (Ok(_), Pending::Export(out)) => {
                    self.notify(NoticeLevel::Info, format!("Exported {}", out.display()));
                    Ok(out.display().to_string())
                }
                (Ok(_), Pending::Quiet) => Ok(String::new()),
            };
            match (&result, &pending) {
                (Err(error), _) => self.notify(NoticeLevel::Error, error.clone()),
                (Ok(_), Pending::Save) => {
                    self.notify(NoticeLevel::Info, "Saved".into());
                    // The helper reports the modified flag with the next thing it is
                    // asked, not on its own: ask something.
                    self.refresh_at = Some(Instant::now() + REFRESH_DELAY);
                }
                _ => {}
            }
            if !matches!(pending, Pending::Quiet) {
                self.done.insert(id, result);
            }
        }
        let excess = self.replies.len().saturating_sub(KEPT_REPLIES);
        self.replies.drain(..excess);
    }

    /// The answer to `save`: the references the file holds go to its sidecar.
    fn saved(&mut self, value: Value) -> Result<String, String> {
        let report: SaveReport =
            serde_json::from_value(value).map_err(|e| format!("unexpected answer to save: {e}"))?;
        self.written
            .extend(report.files().into_iter().map(Path::to_path_buf));
        refs::update_after_save(&self.project, &report)
            .map_err(|e| format!("saved, but the references were not recorded: {e:#}"))?;
        self.record_link()?;
        Ok(report.file.display().to_string())
    }

    /// Save the layout; the outcome is fetched with [`take_done`](Self::take_done).
    pub fn save(&mut self) -> u64 {
        self.call(method::SAVE, json!({}), Pending::Save)
    }

    /// Write the self-contained stream (every reference filled in) to `out`.
    pub fn export(&mut self, out: &Path) -> u64 {
        let params = json!({"refs": self.refs, "out": out});
        self.call(method::EXPORT, params, Pending::Export(out.to_path_buf()))
    }

    /// Place the cell of the layout file `file`, which the helper loads as an external
    /// cell, and switch to instance mode.
    pub fn place(&mut self, project: &Project, cell: &str, file: &Path) -> u64 {
        let table = refs::resolve(project, file);
        // What the helper knows already, and everything the new cell needs.
        self.refs.files.extend(table.files.clone());
        self.session.set_refs(self.refs.clone());
        let mut sent = self.refs.clone();
        sent.root = file.to_path_buf();
        self.mode = "instance".into();
        self.call(
            method::PLACE,
            json!({"name": cell, "refs": sent}),
            Pending::Place,
        )
    }

    /// Ask the helper something harmless: it reports changes of state (the modified flag
    /// after a save) when it handles a request, not at once.
    pub fn nudge(&mut self) {
        if self.nudged.is_none_or(|at| at.elapsed() > NUDGE_EVERY) {
            self.nudged = Some(Instant::now());
            self.quiet(method::INFO, json!({}));
        }
    }

    pub fn set_mode(&mut self, name: &str) {
        self.mode = name.to_string();
        self.quiet(method::MODE, json!({"name": name}));
    }

    pub fn zoom(&mut self, what: &str) {
        self.quiet(method::ZOOM, json!({"what": what}));
    }

    /// Undo, redo, delete and the like: methods without parameters.
    pub fn simple(&mut self, method: &str) {
        self.quiet(method, json!({}));
    }

    pub fn set_levels(&mut self, max: u32) {
        self.levels = max;
        let max = if max >= ALL_LEVELS { 1000 } else { max };
        self.quiet(method::LEVELS, json!({"min": 0, "max": max}));
    }

    pub fn show_cell(&mut self, path: &[String]) {
        self.quiet(method::SHOW_CELL, json!({"path": path}));
    }

    pub fn add_layer(&mut self, layer: i32, datatype: i32, name: &str) {
        let mut params = json!({"layer": layer, "datatype": datatype});
        if !name.trim().is_empty() {
            params["name"] = json!(name.trim());
        }
        self.quiet(method::ADD_LAYER, params);
    }

    /// The layer whose name or source is `text` (`1/0` also finds `1/0@1`).
    pub fn find_layer(&self, text: &str) -> Option<&libcdept::klayout::protocol::Layer> {
        self.layers.iter().find(|l| {
            l.name == text || l.source == text || l.source.split('@').next() == Some(text)
        })
    }

    pub fn set_layer_visible(&mut self, id: u32, visible: bool) {
        if let Some(layer) = self.layers.iter_mut().find(|l| l.id == id) {
            layer.visible = visible;
        }
        self.quiet(method::LAYER_VISIBLE, json!({"id": id, "visible": visible}));
    }

    pub fn set_current_layer(&mut self, id: u32) {
        for layer in &mut self.layers {
            layer.current = layer.id == id;
        }
        self.quiet(method::CURRENT_LAYER, json!({"id": id}));
    }

    /// What a cell of the tree is: `local`, `external` or `missing`, if it is there.
    pub fn cell_kind(&self, name: &str) -> Option<&'static str> {
        let node = self.cells.as_ref()?.cells.iter().find(|c| c.name == name)?;
        Some(if node.missing {
            "missing"
        } else if node.external.is_some() {
            "external"
        } else {
            "local"
        })
    }

    /// What a double click on a cell of the tree does: a local cell is shown, an external
    /// one opens its view in a tab of its own.
    pub fn open_cell(&mut self, name: &str) -> Result<(), String> {
        let cells = self.cells.as_ref().ok_or("there is no cell tree yet")?;
        let node = cells
            .cells
            .iter()
            .find(|c| c.name == name)
            .ok_or(format!("there is no cell '{name}'"))?;
        if node.missing {
            return Err(format!("cell '{name}' cannot be found"));
        }
        if node.external.is_some() {
            let file = node.file.clone().ok_or("the cell has no file")?;
            self.open_requests.push(file.into());
            return Ok(());
        }
        let path = panels::cell_path(&cells.cells, &cells.top, name)
            .ok_or(format!("cell '{name}' is not below the top cell"))?;
        self.show_cell(&path);
        Ok(())
    }

    pub fn selection_count(&self) -> usize {
        self.selection.count
    }

    /// Whether a notice with this text came (all of them are kept for a while).
    pub fn has_notice(&self, text: &str) -> bool {
        self.notices.iter().any(|(_, t)| t.contains(text))
    }

    // ----- what the helper reports -----

    fn notify(&mut self, level: NoticeLevel, text: String) {
        match level {
            NoticeLevel::Info => {
                tracing::info!(target: "cdept::layout", "{}: {text}", self.doc.title())
            }
            NoticeLevel::Warning => {
                tracing::warn!(target: "cdept::layout", "{}: {text}", self.doc.title())
            }
            NoticeLevel::Error => {
                tracing::error!(target: "cdept::layout", "{}: {text}", self.doc.title())
            }
        }
        self.notice = Some((level, text.clone(), Instant::now()));
        self.notices.push((level, text));
        let excess = self.notices.len().saturating_sub(KEPT_NOTICES);
        self.notices.drain(..excess);
    }

    fn apply(&mut self, event: KEvent) {
        match event {
            KEvent::Layers { layers } => self.layers = layers,
            KEvent::Cells {
                top,
                path,
                cells,
                context,
            } => {
                self.cells = Some(Cells { top, path, cells });
                self.in_place = context;
                // The first sight of a layout is all of it.
                if std::mem::take(&mut self.fit_pending) {
                    self.zoom("fit");
                }
            }
            KEvent::Selection { count, bbox, items } => {
                self.selection = Selected { count, bbox, items };
            }
            KEvent::Status {
                x,
                y,
                mode,
                message,
            } => {
                self.at = (x, y);
                if !mode.is_empty() {
                    self.mode = mode;
                }
                self.hint = message;
            }
            KEvent::Modified {
                dirty,
                can_undo,
                can_redo,
                edited,
            } => {
                self.dirty = dirty;
                self.edited = edited;
                self.can_undo = can_undo;
                self.can_redo = can_redo;
            }
            KEvent::Notice { level, text } => self.notify(level, text),
        }
    }

    /// Take in what the helper reported since the last frame.
    fn sync(&mut self) {
        let epoch = self.session.epoch();
        if epoch != self.epoch {
            // A new helper: it sends everything again.
            self.epoch = epoch;
            self.held.clear();
            self.inside = false;
            self.layers.clear();
            self.cells = None;
            self.selection = Selected::default();
            self.at = (None, None);
            self.mode.clear();
            self.dirty = false;
            self.edited.clear();
            self.in_place = None;
            self.can_undo = false;
            self.can_redo = false;
            self.requested = None;
            self.fit_pending = true;
            self.pending.clear();
            // Their questions went to the helper that is gone.
            self.properties = None;
            self.library_picker = None;
            self.link.forget_questions();
            if matches!(self.closing, Some(Closing::Saving(_))) {
                self.closing = None;
            }
        }
        for event in self.session.take_events() {
            self.apply(event);
        }
        self.pump();
    }

    // ----- the picture -----

    /// The screen position (in points) of the middle of frame pixel (`x`, `y`).
    pub fn frame_to_screen(&self, x: f32, y: f32) -> Option<Pos2> {
        let (origin, scale) = self.placement?;
        Some(origin + Vec2::new(x + 0.5, y + 0.5) / scale)
    }

    /// Whether a frame of the running helper is on screen.
    pub fn has_frame(&self) -> bool {
        self.is_running() && self.picture.texture().is_some() && self.placement.is_some()
    }

    pub fn frame_size(&self) -> Option<[u32; 2]> {
        self.session.frame(|frame| [frame.width(), frame.height()])
    }

    /// The colour of a frame pixel, as R G B A.
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        let mut out = Vec::new();
        let copied = self
            .session
            .frame(|frame| frame.copy(libcdept::xschem::frame::Rect::new(x, y, 1, 1), &mut out))?;
        (!copied.is_empty()).then(|| [out[0], out[1], out[2], out[3]])
    }

    /// Check that the drawing on screen (in a screenshot of the window) is the frame,
    /// pixel for pixel: nothing scaled, nothing shifted.
    pub fn compare_with_screen(&self, screen: &egui::ColorImage) -> Result<(), String> {
        let (origin, scale) = self.placement.ok_or("the layout is not on screen")?;
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

    // ----- input -----

    /// A shortcut of CDePT's, or an edit command.
    fn command(&mut self, shortcut: Shortcut) {
        match shortcut {
            Shortcut::Undo => self.simple(method::UNDO),
            Shortcut::Redo => self.simple(method::REDO),
            Shortcut::Save => {
                if self.editable {
                    self.save();
                }
            }
            Shortcut::ZoomFit => self.zoom("fit"),
            Shortcut::ZoomIn => self.zoom("in"),
            Shortcut::ZoomOut => self.zoom("out"),
            Shortcut::SelectAll => self.simple(method::SELECT_ALL),
            Shortcut::Delete => {
                if self.editable {
                    self.simple(method::DELETE);
                }
            }
            Shortcut::Descend => {
                if self.editable {
                    self.descend();
                }
            }
            Shortcut::Ascend => self.ascend(false),
            Shortcut::Properties => self.open_properties(),
            Shortcut::AscendAll => self.ascend(true),
        }
    }

    /// Pass the mouse and the keyboard on to KLayout. `over`: the pointer is on the
    /// drawing and nothing covers it. `keys`: the drawing has the keyboard.
    fn input(&mut self, events: &[Event], modifiers: Modifiers, over: bool, keys: bool) {
        let Some((origin, scale)) = self.placement else {
            return;
        };
        let to_frame = |pos: Pos2| {
            let p = (pos - origin) * scale;
            (f64::from(p.x.floor()), f64::from(p.y.floor()))
        };
        let mods = keys::modifiers(modifiers);
        for event in events {
            match event {
                Event::PointerMoved(pos) => {
                    self.pointer = Some(*pos);
                    let (x, y) = to_frame(*pos);
                    if over && !self.inside {
                        self.inside = true;
                        self.session.input(Input::Enter);
                    }
                    // A drag goes on outside the drawing, as with a grabbed pointer.
                    if over || !self.held.is_empty() {
                        self.session.input(Input::Move {
                            x,
                            y,
                            buttons: self.held.clone(),
                            modifiers: mods,
                        });
                    } else if self.inside {
                        self.inside = false;
                        self.session.input(Input::Leave);
                    }
                }
                Event::PointerGone => {
                    if self.inside && self.held.is_empty() {
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
                    let Some(button) = keys::button(*button) else {
                        continue;
                    };
                    let (x, y) = to_frame(*pos);
                    let modifiers = keys::modifiers(*modifiers);
                    if *pressed && over {
                        let again = self.last_press.is_some_and(|(when, at, was)| {
                            was == button
                                && when.elapsed() < DOUBLE_CLICK
                                && (at - *pos).length() * scale < DOUBLE_CLICK_PIXELS
                        });
                        if again {
                            self.last_press = None;
                            self.session.input(Input::DoubleClick {
                                x,
                                y,
                                button,
                                modifiers,
                            });
                            // A double click on an object shows its properties.
                            if self.mode == "select" && button == Button::Left {
                                self.open_properties_if_selected();
                            }
                        } else {
                            self.last_press = Some((Instant::now(), *pos, button));
                            self.session.input(Input::Press {
                                x,
                                y,
                                button,
                                modifiers,
                            });
                        }
                        self.held.push(button);
                    } else if !*pressed && self.held.contains(&button) {
                        self.session.input(Input::Release {
                            x,
                            y,
                            button,
                            modifiers,
                        });
                        self.held.retain(|b| *b != button);
                    }
                }
                Event::MouseWheel { unit, delta, .. } if over => {
                    self.wheel += keys::wheel_notches(*unit, *delta);
                    let (x, y) = self.pointer.map_or((0.0, 0.0), to_frame);
                    for (delta, horizontal) in keys::take_wheel(&mut self.wheel) {
                        self.session.input(Input::Wheel {
                            x,
                            y,
                            delta,
                            horizontal,
                            modifiers: mods,
                        });
                    }
                }
                Event::Text(text) if keys && !(modifiers.command || modifiers.ctrl) => {
                    for c in text.chars().filter(|c| !c.is_control()) {
                        self.session.input(Input::Key {
                            key: String::new(),
                            text: c.to_string(),
                            modifiers: mods,
                        });
                    }
                }
                Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } if keys => {
                    if let Some(shortcut) = keys::shortcut(*key, *modifiers) {
                        self.command(shortcut);
                    } else if let Some(name) = keys::key_name(*key) {
                        self.session.input(Input::Key {
                            key: name.to_string(),
                            text: String::new(),
                            modifiers: keys::modifiers(*modifiers),
                        });
                    }
                }
                // egui turns Ctrl+C, Ctrl+X and Ctrl+V into these.
                Event::Copy if keys => self.simple(method::COPY),
                Event::Cut if keys && self.editable => self.simple(method::CUT),
                Event::Paste(_) if keys && self.editable => self.simple(method::PASTE),
                _ => {}
            }
        }
    }

    // ----- drawing the tab -----

    /// The picture, with its input, and what covers it.
    fn drawing(&mut self, ui: &mut Ui) {
        let area = ui.available_rect_before_wrap();
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

        let name = format!("klayout-frame-{}", self.id.0);
        let picture = &mut self.picture;
        self.session.frame(|frame| {
            let size = [frame.width(), frame.height()];
            picture.update_from(
                ui.ctx(),
                &name,
                (frame.generation, size, frame.damage),
                |rect, out| frame.copy(rect, out),
            );
        });
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

        let modal = self.place.is_some() || matches!(self.closing, Some(Closing::Asking));
        let running = self.is_running();
        let over = response.contains_pointer() && running && !modal;
        if over {
            if self.mode != "select" && !self.mode.is_empty() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            }
            // The keyboard follows the pointer, unless something else (a text field)
            // is using it.
            if ui.memory(|m| m.focused().is_none()) || response.clicked() || response.drag_started()
            {
                response.request_focus();
            }
        }
        let keys = over && response.has_focus();
        if response.has_focus() {
            // Keep egui from moving the focus with keys that belong to KLayout.
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
        } else if running && !self.held.is_empty() {
            // A button released while something covers the drawing still has to reach
            // KLayout, or it keeps the button down.
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
        self.overlay(ui, area);
    }

    /// What the helper is doing when it is not simply running, and the last message.
    fn overlay(&mut self, ui: &mut Ui, rect: Rect) {
        let status = self.session.status();
        let (text, failed) = match &status {
            Status::Running | Status::Closed => (None, false),
            Status::Starting => (Some("Starting KLayout…".to_string()), false),
            Status::Restarting(reason) => (Some(format!("{reason}. Starting it again…")), false),
            Status::Failed(reason) => (Some(reason.clone()), true),
        };
        if let Some(text) = text.filter(|_| !failed) {
            self.label_on_picture(ui, rect, text, Color32::from_gray(220), false);
        }
        if failed && let Status::Failed(reason) = &status {
            let panel =
                Rect::from_center_size(rect.center(), rect.size().min(Vec2::new(520.0, 160.0)));
            ui.scope_builder(egui::UiBuilder::new().max_rect(panel), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.strong("The KLayout engine stopped");
                        ui.label(reason);
                        ui.add_space(4.0);
                        if ui.button("Restart").clicked() {
                            self.session.restart();
                        }
                    });
                });
            });
        }
        // The last notice, for a few seconds.
        if let Some((level, text, since)) = self.notice.clone() {
            let age = since.elapsed();
            if age >= NOTICE_TIME {
                self.notice = None;
            } else {
                let color = match level {
                    NoticeLevel::Info => Color32::from_gray(230),
                    NoticeLevel::Warning => Color32::from_rgb(255, 210, 90),
                    NoticeLevel::Error => Color32::from_rgb(255, 130, 130),
                };
                self.label_on_picture(ui, rect, text, color, true);
                ui.ctx().request_repaint_after(NOTICE_TIME - age);
            }
        }
    }

    /// A line of text on a dark patch, at the top left of the picture or at the bottom.
    fn label_on_picture(&self, ui: &Ui, rect: Rect, text: String, color: Color32, bottom: bool) {
        let galley = ui.painter().layout(
            text,
            egui::TextStyle::Body.resolve(ui.style()),
            color,
            (rect.width() - 24.0).max(40.0),
        );
        let at = if bottom {
            Pos2::new(rect.left() + 8.0, rect.bottom() - 8.0 - galley.size().y)
        } else {
            rect.left_top() + Vec2::splat(8.0)
        };
        let back = Rect::from_min_size(at, galley.size()).expand(4.0);
        let painter = ui.painter_at(rect);
        painter.rect_filled(back, 4.0, Color32::from_black_alpha(170));
        painter.galley(at, galley, color);
    }

    /// The status line: pointer in µm, mode, hint.
    fn status_line(&self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            let number = |v: Option<f64>| v.map_or("–".to_string(), |v| format!("{v:.3}"));
            ui.monospace(format!(
                "x: {}  y: {} µm",
                number(self.at.0),
                number(self.at.1)
            ));
            ui.separator();
            let mode = MODES
                .iter()
                .find(|(name, ..)| *name == self.mode)
                .map_or(self.mode.as_str(), |(_, label, _)| label);
            ui.label(format!(
                "Mode: {}",
                if mode.is_empty() { "…" } else { mode }
            ));
            if !self.hint.is_empty() {
                ui.separator();
                ui.label(&self.hint);
            }
            if !self.editable {
                ui.separator();
                ui.weak("read-only");
            }
            self.in_place_status(ui);
            if let Some(version) = self.session.klayout_version() {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.weak(version);
                });
            }
        });
    }

    /// The question about unsaved changes, and the wait for the save.
    fn closing(&mut self, ui: &Ui, ctx: &mut AppCtx) {
        match self.closing {
            None => {}
            Some(Closing::Asking) => {
                let mut choice = None;
                let modal = egui::Modal::new(egui::Id::new(("klayout-close", self.id)));
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
                            if ui
                                .add_enabled(
                                    answer != CloseChoice::Save || self.editable,
                                    egui::Button::new(label),
                                )
                                .clicked()
                            {
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
            Some(Closing::Saving(id)) => match self.take_done(id) {
                None if self.is_running() => {}
                None => self.closing = None,
                Some(Ok(_)) => {
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

    /// The tab is about to be closed. Returns whether it may go right away; if not, the
    /// user is asked about the unsaved changes first.
    pub fn may_close(&mut self) -> bool {
        if !self.dirty || !self.is_running() {
            return true;
        }
        self.closing.get_or_insert(Closing::Asking);
        false
    }

    pub fn asks_to_close(&self) -> bool {
        matches!(self.closing, Some(Closing::Asking))
    }

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

    /// Start the question for where to export to, with the desktop's file dialog.
    fn ask_export(&mut self) {
        if self.export_dialog.is_some() {
            return;
        }
        let (send, receive) = std::sync::mpsc::channel();
        let name = format!("{}.gds", self.doc.cell);
        let folder = self.file.parent().map(Path::to_path_buf);
        std::thread::spawn(move || {
            let mut dialog = rfd::FileDialog::new()
                .set_title("Export self-contained GDS")
                .set_file_name(name)
                .add_filter("Layout", &["gds", "oas"]);
            if let Some(folder) = folder {
                dialog = dialog.set_directory(folder);
            }
            let _ = send.send(dialog.save_file());
        });
        self.export_dialog = Some(receive);
    }

    fn ready(&self) -> bool {
        self.is_running() && self.place.is_none()
    }
}

impl View for LayoutView {
    fn title(&self, _ctx: &AppCtx) -> String {
        let star = if self.dirty { "*" } else { "" };
        format!("{}{star}", self.doc.title())
    }

    fn ui(&mut self, ui: &mut Ui, ctx: &mut AppCtx) {
        self.sync();
        if let Some(at) = self.refresh_at {
            if Instant::now() >= at {
                self.refresh_at = None;
                self.nudge();
            } else {
                ui.ctx().request_repaint_after(at - Instant::now());
            }
        }
        if self.session.status() == Status::Closed {
            ctx.request(Action::CloseTab(Tab::Document(self.id)));
        }
        let id = self.id;
        egui::Panel::bottom(egui::Id::new(("klayout-status", id)))
            .show(ui, |ui| self.status_line(ui));
        if self.side {
            egui::Panel::right(egui::Id::new(("klayout-side", id)))
                .resizable(true)
                .default_size(250.0)
                .size_range(160.0..=600.0)
                .show(ui, |ui| self.side_panels(ui));
        }
        self.poll_checks(ui.ctx());
        self.checks_panel(ui);
        self.poll_link();
        self.link_panel(ui, ctx);
        self.drawing(ui);

        if let Some(picker) = &mut self.place
            && let Some(project) = &ctx.project
        {
            match place::show(
                ui.ctx(),
                egui::Id::new(("klayout-place", id)),
                project,
                picker,
            ) {
                place::Picked::Nothing => {}
                place::Picked::Cancel => {
                    self.place = None;
                    self.replace_target = None;
                }
                place::Picked::Cell(placed) => {
                    self.place = None;
                    if let Some(target) = self.replace_target.take() {
                        self.replace_cell(
                            target,
                            properties::Replacement::View {
                                cell: placed.cell,
                                file: placed.file,
                            },
                        );
                    } else {
                        self.place(project, &placed.cell, &placed.file);
                    }
                }
            }
        }
        if let Some(dialog) = &self.export_dialog
            && let Ok(answer) = dialog.try_recv()
        {
            self.export_dialog = None;
            if let Some(out) = answer {
                self.export(&out);
            }
        }
        self.properties_ui(ui.ctx());
        self.library_picker_ui(ui.ctx());
        for file in std::mem::take(&mut self.open_requests) {
            ctx.request(Action::OpenFile(file));
        }
        self.closing(ui, ctx);
    }

    fn menus(&mut self, ui: &mut Ui, ctx: &mut AppCtx) {
        let ready = self.ready();
        let editable = ready && self.editable;
        ui.menu_button("Layout", |ui| {
            ui.add_enabled_ui(ready, |ui| {
                if ui
                    .add_enabled(
                        editable && ctx.project.is_some(),
                        egui::Button::new("Place Cell…"),
                    )
                    .on_hover_text("Place a cell of another layout view of the project")
                    .clicked()
                {
                    self.place = Some(place::PlacePicker::default());
                    ui.close();
                }
                if ui
                    .add_enabled(editable, egui::Button::new("Place Library Cell…"))
                    .on_hover_text("Place a cell or PCell of KLayout's libraries (PDK, Basic)")
                    .clicked()
                {
                    self.open_library_picker();
                    ui.close();
                }
                if ui
                    .add(egui::Button::new("Properties…").shortcut_text("Q"))
                    .on_hover_text("Properties of the selection")
                    .clicked()
                {
                    self.open_properties();
                    ui.close();
                }
                ui.separator();
                if ui
                    .add_enabled(editable, egui::Button::new("Delete").shortcut_text("Del"))
                    .clicked()
                {
                    self.simple(method::DELETE);
                    ui.close();
                }
                if ui.button("Clear Selection").clicked() {
                    self.simple(method::CLEAR_SELECTION);
                    ui.close();
                }
                self.in_place_menu(ui, editable);
                ui.separator();
                ui.menu_button("Mode", |ui| {
                    for (name, label, hover) in MODES {
                        let on = self.mode == name;
                        if ui
                            .add_enabled(
                                editable || name == "select" || name == "ruler",
                                egui::Button::new(label).selected(on),
                            )
                            .on_hover_text(hover)
                            .clicked()
                        {
                            self.set_mode(name);
                            ui.close();
                        }
                    }
                });
                ui.menu_button("Zoom", |ui| {
                    for (what, label, shortcut) in ZOOMS {
                        if ui
                            .add(egui::Button::new(label).shortcut_text(shortcut))
                            .clicked()
                        {
                            self.zoom(what);
                            ui.close();
                        }
                    }
                });
            });
            ui.separator();
            self.link_menu(ui, ctx, editable);
            ui.separator();
            self.checks_menu(ui, ready);
            ui.separator();
            ui.checkbox(&mut self.side, "Layers, Cells and Selection");
            if ui.button("Select in Libraries").clicked() {
                ctx.selection = Selection::view(&self.doc.library, &self.doc.cell, &self.doc.view);
                ctx.request(Action::OpenTab(Tab::Libraries));
                ui.close();
            }
        });
    }

    fn toolbar(&mut self, ui: &mut Ui, _ctx: &mut AppCtx) {
        let ready = self.ready();
        let editable = ready && self.editable;
        ui.add_enabled_ui(ready, |ui| {
            for (name, label, hover) in MODES {
                let usable = editable || name == "select" || name == "ruler";
                let on = self.mode == name;
                if ui
                    .add_enabled(usable, egui::Button::new(label).selected(on))
                    .on_hover_text(hover)
                    .clicked()
                {
                    self.set_mode(name);
                }
            }
            ui.separator();
            for (what, label, shortcut) in ZOOMS {
                if ui
                    .button(label)
                    .on_hover_text(format!("Zoom {what} ({shortcut})"))
                    .clicked()
                {
                    self.zoom(what);
                }
            }
            ui.separator();
            if ui
                .add_enabled(editable && self.can_undo, egui::Button::new("Undo"))
                .on_hover_text("Undo (Ctrl+Z)")
                .clicked()
            {
                self.simple(method::UNDO);
            }
            if ui
                .add_enabled(editable && self.can_redo, egui::Button::new("Redo"))
                .on_hover_text("Redo (Ctrl+Y)")
                .clicked()
            {
                self.simple(method::REDO);
            }
            if ui
                .add_enabled(self.selection.count > 0, egui::Button::new("Properties"))
                .on_hover_text("Properties of the selection (Q, or a double click)")
                .clicked()
            {
                self.open_properties();
            }
            if ui
                .add_enabled(editable && self.dirty, egui::Button::new("Save"))
                .on_hover_text("Save (Ctrl+S)")
                .clicked()
            {
                self.save();
            }
            ui.separator();
            ui.label("Levels");
            let mut levels = self.levels;
            let changed = ui
                .add(
                    egui::DragValue::new(&mut levels)
                        .range(0..=ALL_LEVELS)
                        .custom_formatter(|n, _| {
                            if n as u32 >= ALL_LEVELS {
                                "all".to_string()
                            } else {
                                format!("{n}")
                            }
                        }),
                )
                .on_hover_text("Hierarchy levels shown")
                .changed();
            if changed {
                self.set_levels(levels);
            }
            ui.separator();
            self.checks_toolbar(ui, ready);
        });
    }

    fn file_menu(&mut self, ui: &mut Ui, _ctx: &mut AppCtx) {
        let ready = self.ready();
        if ui
            .add_enabled(
                ready && self.editable,
                egui::Button::new("Save Layout").shortcut_text("Ctrl+S"),
            )
            .clicked()
        {
            self.save();
            ui.close();
        }
        if ui
            .add_enabled(ready, egui::Button::new("Export Self-contained GDS…"))
            .on_hover_text("Write a stream with every referenced cell filled in")
            .clicked()
        {
            self.ask_export();
            ui.close();
        }
        ui.separator();
    }

    fn edit_caps(&self) -> EditCaps {
        let ready = self.ready();
        EditCaps {
            undo: ready && self.editable && self.can_undo,
            redo: ready && self.editable && self.can_redo,
            cut: ready && self.editable,
            copy: ready,
            paste: ready && self.editable,
            select_all: ready,
        }
    }

    fn edit(&mut self, action: EditAction, _ctx: &mut AppCtx) {
        let method = match action {
            EditAction::Undo => method::UNDO,
            EditAction::Redo => method::REDO,
            EditAction::Cut => method::CUT,
            EditAction::Copy => method::COPY,
            EditAction::Paste => method::PASTE,
            EditAction::SelectAll => method::SELECT_ALL,
        };
        self.simple(method);
    }
}

/// The zoom commands: the name for the helper, the label and KLayout's shortcut.
const ZOOMS: [(&str, &str, &str); 3] = [
    ("fit", "Fit", "F2"),
    ("in", "+", "Ctrl++"),
    ("out", "-", "Ctrl+-"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_are_the_ones_the_helper_knows() {
        let names: Vec<&str> = MODES.iter().map(|(name, ..)| *name).collect();
        for wanted in [
            "select", "move", "box", "polygon", "path", "text", "instance", "ruler", "partial",
        ] {
            assert!(names.contains(&wanted), "{wanted}");
        }
    }
}
