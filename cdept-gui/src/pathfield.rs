//! Choosing folders and files without depending on the desktop's file dialog.
//!
//! The native dialog (`rfd`) needs a desktop portal or zenity. Containers and bare X
//! sessions have neither, and then it simply does not appear. So CDePT has its own:
//!
//! - [`PathField`]: a text field with completion (Tab, or a click in the list below
//!   it) and a "Browse…" button that opens the native dialog when there is one. When
//!   the native dialog cannot be shown, a line under the field says so.
//! - [`ask`]: ask for a path, for code that only wants the answer. Depending on the
//!   [`Request`] it shows CDePT's own prompt (a window with a [`PathField`]) or tries
//!   the native dialog first and falls back to the prompt when that does not open.
//!   The prompt appears on the window that asked. [`show`] draws the prompts; the
//!   shell calls it for every window, every frame.
//!
//! `CDEPT_NATIVE_DIALOG=off` in the environment treats the native dialog as not
//! there (for tests, and where it hangs).
//!
//! The short forms, for dialogs with a grid of labelled fields:
//!
//! ```ignore
//! // A row "Folder  [text field........] [Browse…]" in a two-column grid:
//! let changed = pathfield::folder_row(ui, "Folder", &mut text, Some(&project.root));
//! // The widget alone:
//! let changed = PathField::folder("my-id", &mut text).base(&root).show(ui).changed;
//! ```

use std::hash::Hash;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use egui::{Context, Id, Key, Modifiers, PopupCloseBehavior, RichText, Ui, ViewportId};

use crate::dock::WindowId;

/// A native dialog that answers "nothing" sooner than this did not open: nobody
/// cancels that fast.
const TOO_FAST: Duration = Duration::from_millis(400);
/// A prompt whose window has not been drawn for this long moves to the main window
/// (the window was closed).
const ORPHANED: Duration = Duration::from_secs(1);
/// The most candidates the completion list shows.
const MAX_SHOWN: usize = 12;

const NO_NATIVE: &str = "The system's file dialog is not available here (no desktop portal, no \
                         zenity): type the path. Tab completes it.";

/// Values for `$VAR` in a path: a variable's name → its value.
pub type Vars<'a> = dyn Fn(&str) -> Option<String> + 'a;

/// What is asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
    #[default]
    Folder,
    /// An existing file.
    File,
    /// A file to write, which need not exist.
    SaveFile,
}

/// Whether the native dialog may be tried at all.
pub fn native_allowed() -> bool {
    !std::env::var("CDEPT_NATIVE_DIALOG").is_ok_and(|v| matches!(v.as_str(), "off" | "0" | "no"))
}

/// What a native dialog said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    Waiting,
    Picked(PathBuf),
    Cancelled,
    /// The dialog did not open.
    Unavailable,
}

/// A native dialog running on its own thread.
pub struct Native {
    result: Receiver<Option<PathBuf>>,
    started: Instant,
}

impl Native {
    /// Open the native dialog for `request`. Without one ([`native_allowed`]) the
    /// first [`Native::poll`] says [`Answer::Unavailable`].
    pub fn open(ctx: &Context, request: &Request) -> Native {
        if !native_allowed() {
            return Native::unavailable();
        }
        let (sender, result) = mpsc::channel();
        let started = Instant::now();
        let (ctx, request) = (ctx.clone(), request.clone());
        std::thread::spawn(move || {
            let mut dialog = rfd::FileDialog::new();
            if !request.title.is_empty() {
                dialog = dialog.set_title(&request.title);
            }
            if let Some(dir) = request.directory.as_ref().filter(|d| d.is_dir()) {
                dialog = dialog.set_directory(dir);
            }
            if let Some(name) = &request.file_name {
                dialog = dialog.set_file_name(name);
            }
            for (name, extensions) in &request.filters {
                dialog = dialog.add_filter(name, extensions);
            }
            let chosen = match request.kind {
                Kind::Folder => dialog.pick_folder(),
                Kind::File => dialog.pick_file(),
                Kind::SaveFile => dialog.save_file(),
            };
            let _ = sender.send(chosen);
            ctx.request_repaint();
        });
        Native { result, started }
    }

    /// A dialog that did not open (what [`Native::open`] gives without a native one).
    fn unavailable() -> Native {
        let (sender, result) = mpsc::channel();
        let _ = sender.send(None);
        Native {
            result,
            started: Instant::now(),
        }
    }

    pub fn poll(&mut self) -> Answer {
        match self.result.try_recv() {
            Ok(Some(path)) => Answer::Picked(path),
            Ok(None) if self.started.elapsed() < TOO_FAST => Answer::Unavailable,
            Ok(None) | Err(TryRecvError::Disconnected) => Answer::Cancelled,
            Err(TryRecvError::Empty) => Answer::Waiting,
        }
    }
}

/// What the widget remembers between frames.
#[derive(Clone, Default)]
struct Memory {
    /// The native dialog opened by "Browse…".
    browse: Option<Arc<Mutex<Native>>>,
    /// Why "Browse…" did nothing.
    note: Option<String>,
    /// The text the candidates were worked out for.
    listed_for: Option<String>,
    candidates: Arc<Vec<Candidate>>,
    /// Where the completion list was drawn, to keep it open while the pointer is on it.
    list_rect: Option<egui::Rect>,
    list_open: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Candidate {
    name: String,
    is_dir: bool,
}

/// What [`PathField::show`] did.
pub struct PathFieldResponse {
    /// The text field.
    #[allow(dead_code)] // for callers that place something next to the field
    pub response: egui::Response,
    /// The text changed: typed, completed or browsed.
    pub changed: bool,
}

/// A text field for a folder or a file, with completion and a "Browse…" button.
pub struct PathField<'a> {
    id: Id,
    text: &'a mut String,
    kind: Kind,
    base: Option<PathBuf>,
    title: String,
    width: f32,
    browse: bool,
    focus: bool,
    vars: Option<&'a Vars<'a>>,
    filters: Vec<(String, Vec<String>)>,
}

impl<'a> PathField<'a> {
    pub fn new(id_salt: impl Hash + std::fmt::Debug, text: &'a mut String, kind: Kind) -> Self {
        Self {
            id: Id::new("cdept-pathfield").with(id_salt),
            text,
            kind,
            base: None,
            title: String::new(),
            width: 280.0,
            browse: true,
            focus: false,
            vars: None,
            filters: Vec::new(),
        }
    }

    pub fn folder(id_salt: impl Hash + std::fmt::Debug, text: &'a mut String) -> Self {
        Self::new(id_salt, text, Kind::Folder)
    }

    #[allow(dead_code)] // part of the widget's API; no dialog asks for a file yet
    pub fn file(id_salt: impl Hash + std::fmt::Debug, text: &'a mut String) -> Self {
        Self::new(id_salt, text, Kind::File)
    }

    #[allow(dead_code)] // as `file`
    pub fn save_file(id_salt: impl Hash + std::fmt::Debug, text: &'a mut String) -> Self {
        Self::new(id_salt, text, Kind::SaveFile)
    }

    /// The folder a relative text is relative to. A browsed path inside it is written
    /// relative too.
    pub fn base(mut self, base: &Path) -> Self {
        self.base = Some(base.to_path_buf());
        self
    }

    /// Title of the native dialog.
    pub fn title(mut self, title: &str) -> Self {
        self.title = title.to_string();
        self
    }

    /// Width of the text field (the button comes after it).
    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// Without the "Browse…" button.
    pub fn no_browse(mut self) -> Self {
        self.browse = false;
        self
    }

    /// Take the keyboard focus this frame.
    pub fn focus(mut self, focus: bool) -> Self {
        self.focus = focus;
        self
    }

    /// Values for `$VAR` in the text, e.g. the project's (`$PDK_ROOT`). The
    /// environment is used otherwise.
    pub fn vars(mut self, vars: &'a Vars<'a>) -> Self {
        self.vars = Some(vars);
        self
    }

    /// File types of the native dialog: a name and the extensions.
    pub fn filter(mut self, name: &str, extensions: &[&str]) -> Self {
        self.filters.push((
            name.to_string(),
            extensions.iter().map(|e| e.to_string()).collect(),
        ));
        self
    }

    /// The path the text means on this machine: `$VAR` and `~` expanded, relative to
    /// the base.
    fn resolved(&self, text: &str) -> Option<PathBuf> {
        resolve(text, self.base.as_deref(), self.vars)
    }

    pub fn show(self, ui: &mut Ui) -> PathFieldResponse {
        let id = self.id;
        let edit_id = id.with("edit");
        let mut memory: Memory = ui.data(|d| d.get_temp(id)).unwrap_or_default();
        let mut changed = false;

        // "Browse…" answered?
        if let Some(native) = memory.browse.clone() {
            let answer = native.lock().map(|mut n| n.poll());
            match answer {
                Ok(Answer::Waiting) => {}
                Ok(Answer::Picked(path)) => {
                    *self.text = match &self.base {
                        Some(base) => libcdept::project_edit::portable(base, &path),
                        None => path.to_string_lossy().into_owned(),
                    };
                    changed = true;
                    memory.browse = None;
                    memory.note = None;
                }
                Ok(Answer::Unavailable) => {
                    memory.browse = None;
                    memory.note = Some(NO_NATIVE.into());
                }
                Ok(Answer::Cancelled) | Err(_) => memory.browse = None,
            }
        }

        // Tab completes instead of moving on to the next field.
        let focused = ui.memory(|m| m.has_focus(edit_id));
        let tab = focused && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Tab));

        let (response, browse_clicked) = ui
            .horizontal(|ui| {
                let response = ui.add(
                    egui::TextEdit::singleline(self.text)
                        .id(edit_id)
                        .desired_width(self.width),
                );
                let clicked = self.browse
                    && native_allowed()
                    && ui
                        .add_enabled(memory.browse.is_none(), egui::Button::new("Browse…"))
                        .on_hover_text("Choose with the system's file dialog")
                        .clicked();
                (response, clicked)
            })
            .inner;
        if self.focus {
            response.request_focus();
        }
        changed |= response.changed();

        if browse_clicked {
            let directory = self
                .resolved(self.text)
                .and_then(|p| p.ancestors().find(|a| a.is_dir()).map(Path::to_path_buf))
                .or_else(|| self.base.clone());
            let request = Request {
                kind: self.kind,
                title: self.title.clone(),
                directory,
                filters: self.filters.clone(),
                ..Request::default()
            };
            memory.browse = Some(Arc::new(Mutex::new(Native::open(ui.ctx(), &request))));
        }

        // Completion: the entries of the folder typed so far.
        if memory.listed_for.as_deref() != Some(self.text.as_str()) || response.gained_focus() {
            memory.candidates = Arc::new(candidates(
                self.text,
                self.base.as_deref(),
                self.vars,
                self.kind,
            ));
            memory.listed_for = Some(self.text.clone());
        }
        let mut picked: Option<String> = None;
        if tab && let Some(completed) = complete(self.text, &memory.candidates) {
            picked = Some(completed);
        }
        let pointer_on_list = memory.list_rect.is_some_and(|rect| {
            ui.ctx()
                .pointer_latest_pos()
                .is_some_and(|pos| rect.contains(pos))
        });
        let open = !memory.candidates.is_empty()
            && (response.has_focus() || (memory.list_open && pointer_on_list));
        memory.list_open = open;
        memory.list_rect = None;
        if open {
            let dir = dir_part(self.text).to_string();
            let shown = egui::Popup::from_response(&response)
                .id(id.with("list"))
                .open(true)
                .close_behavior(PopupCloseBehavior::IgnoreClicks)
                .show(|ui| {
                    ui.set_min_width(response.rect.width());
                    for candidate in memory.candidates.iter().take(MAX_SHOWN) {
                        let slash = if candidate.is_dir { "/" } else { "" };
                        let label = format!("{}{slash}", candidate.name);
                        if ui.selectable_label(false, label).clicked() {
                            picked = Some(format!("{dir}{}{slash}", candidate.name));
                        }
                    }
                    if memory.candidates.len() > MAX_SHOWN {
                        ui.weak(format!("… {} more", memory.candidates.len() - MAX_SHOWN));
                    }
                });
            memory.list_rect = shown.map(|inner| inner.response.rect);
        }
        if let Some(text) = picked
            && text != *self.text
        {
            *self.text = text;
            changed = true;
            // Back to typing, with the cursor at the end.
            response.request_focus();
            if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), edit_id) {
                let end = egui::text::CCursor::new(self.text.chars().count());
                state
                    .cursor
                    .set_char_range(Some(egui::text::CCursorRange::one(end)));
                state.store(ui.ctx(), edit_id);
            }
        }

        if memory.browse.is_some() {
            ui.weak("Waiting for the system's file dialog…");
        }
        if let Some(note) = &memory.note {
            ui.label(
                RichText::new(note)
                    .small()
                    .color(ui.visuals().warn_fg_color),
            );
        }
        ui.data_mut(|d| d.insert_temp(id, memory));
        PathFieldResponse { response, changed }
    }
}

/// A folder field with its label as a row of a two-column grid. Returns whether the
/// text changed.
pub fn folder_row(ui: &mut Ui, label: &str, text: &mut String, base: Option<&Path>) -> bool {
    ui.label(label);
    let mut field = PathField::folder(label, text);
    if let Some(base) = base {
        field = field.base(base);
    }
    let changed = ui.vertical(|ui| field.show(ui).changed).inner;
    ui.end_row();
    changed
}

/// `$VAR` and `~` expanded, relative to `base`. `None` when a variable is not set.
pub fn resolve(text: &str, base: Option<&Path>, vars: Option<&Vars<'_>>) -> Option<PathBuf> {
    let var = |name: &str| match vars.and_then(|vars| vars(name)) {
        Some(value) => Some(value),
        None => std::env::var(name).ok(),
    };
    let expanded = libcdept::project::expand(text.trim(), var).ok()?;
    let path = PathBuf::from(expanded);
    Some(match base {
        Some(base) => base.join(path),
        None => path,
    })
}

/// The text up to and including the last `/`.
fn dir_part(text: &str) -> &str {
    match text.rfind('/') {
        Some(pos) => &text[..=pos],
        None => "",
    }
}

/// The entries of the folder typed so far whose name starts with what follows the
/// last `/`. Folders only when a folder is asked for; hidden entries only when the
/// name typed starts with a dot.
fn candidates(
    text: &str,
    base: Option<&Path>,
    vars: Option<&Vars<'_>>,
    kind: Kind,
) -> Vec<Candidate> {
    let dir_text = dir_part(text);
    let partial = &text[dir_text.len()..];
    // `~` and `$VAR` are completed once they are followed by a `/`.
    if dir_text.is_empty() && (partial.starts_with('~') || partial.starts_with('$')) {
        return Vec::new();
    }
    let dir = if dir_text.is_empty() {
        match base {
            Some(base) => base.to_path_buf(),
            None => PathBuf::from("."),
        }
    } else {
        match resolve(dir_text, base, vars) {
            Some(dir) => dir,
            None => return Vec::new(),
        }
    };
    let Ok(items) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut found: Vec<Candidate> = items
        .flatten()
        .filter_map(|item| {
            let name = item.file_name().into_string().ok()?;
            if !name.starts_with(partial) || (name.starts_with('.') && !partial.starts_with('.')) {
                return None;
            }
            let is_dir = item.path().is_dir();
            (is_dir || kind != Kind::Folder).then_some(Candidate { name, is_dir })
        })
        .collect();
    found.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));
    // Nothing to offer when the one match is what is typed already.
    if found.len() == 1 && found[0].name == partial && !found[0].is_dir {
        found.clear();
    }
    found
}

/// What Tab makes of the text: the longest start all candidates share; a single
/// folder gets its `/`.
fn complete(text: &str, candidates: &[Candidate]) -> Option<String> {
    let first = candidates.first()?;
    let mut common: &str = &first.name;
    for candidate in &candidates[1..] {
        let shared = common
            .char_indices()
            .zip(candidate.name.chars())
            .take_while(|((_, a), b)| a == b)
            .last()
            .map_or(0, |((i, a), _)| i + a.len_utf8());
        common = &common[..shared];
    }
    let slash = if candidates.len() == 1 && first.is_dir {
        "/"
    } else {
        ""
    };
    Some(format!("{}{common}{slash}", dir_part(text)))
}

/// A question for a path, for [`ask`].
#[derive(Debug, Clone, Default)]
pub struct Request {
    pub kind: Kind,
    /// Title of the prompt and of the native dialog.
    pub title: String,
    /// Where to start, and what a relative answer is relative to.
    pub directory: Option<PathBuf>,
    /// A file name to start with ([`Kind::SaveFile`]).
    pub file_name: Option<String>,
    /// File types of the native dialog: a name and the extensions.
    pub filters: Vec<(String, Vec<String>)>,
    /// The answer has to exist ([`Kind::SaveFile`] never has to).
    pub must_exist: bool,
    /// Show CDePT's own prompt at once (its "Browse…" opens the native dialog).
    /// Otherwise the native dialog comes first, and the prompt only when that does
    /// not open.
    pub prompt_first: bool,
    /// A line of explanation in the prompt.
    pub hint: Option<String>,
}

enum Stage {
    Native(Native),
    Prompt,
}

struct Pending {
    serial: u64,
    /// The window that asked, and since when the prompt waits to be drawn there.
    viewport: ViewportId,
    waiting_since: Instant,
    request: Request,
    stage: Stage,
    text: String,
    note: Option<String>,
    error: Option<String>,
    focus: bool,
    sender: Sender<Option<PathBuf>>,
}

/// The questions that are open, kept in the egui context.
#[derive(Clone, Default)]
struct Prompts(Arc<Mutex<PromptList>>);

#[derive(Default)]
struct PromptList {
    next: u64,
    open: Vec<Pending>,
}

fn prompts(ctx: &Context) -> Prompts {
    ctx.data_mut(|d| {
        d.get_temp_mut_or_default::<Prompts>(Id::new("cdept-path-prompts"))
            .clone()
    })
}

/// Ask for a path on behalf of `window`. The answer arrives on the channel: the path
/// (absolute), or `None` when the user gave up.
pub fn ask(ctx: &Context, window: WindowId, request: Request) -> Receiver<Option<PathBuf>> {
    let native = (!request.prompt_first).then(|| Native::open(ctx, &request));
    ask_with(ctx, window.viewport(), request, native)
}

/// [`ask`] on behalf of the window whose UI is being built.
pub fn ask_here(ctx: &Context, request: Request) -> Receiver<Option<PathBuf>> {
    let native = (!request.prompt_first).then(|| Native::open(ctx, &request));
    ask_with(ctx, ctx.viewport_id(), request, native)
}

/// [`ask`] with the native dialog already opened (`None`: the prompt at once).
fn ask_with(
    ctx: &Context,
    viewport: ViewportId,
    request: Request,
    native: Option<Native>,
) -> Receiver<Option<PathBuf>> {
    let (sender, receiver) = mpsc::channel();
    let stage = match native {
        Some(native) => Stage::Native(native),
        None => Stage::Prompt,
    };
    let text = match (&request.directory, &request.file_name) {
        (Some(dir), Some(name)) => dir.join(name).to_string_lossy().into_owned(),
        (Some(dir), None) => format!("{}/", dir.to_string_lossy().trim_end_matches('/')),
        (None, Some(name)) => name.clone(),
        (None, None) => String::new(),
    };
    let prompts = prompts(ctx);
    let mut list = prompts.0.lock().unwrap_or_else(|e| e.into_inner());
    list.next += 1;
    let serial = list.next;
    list.open.push(Pending {
        serial,
        viewport,
        waiting_since: Instant::now(),
        request,
        stage,
        text,
        note: None,
        error: None,
        focus: true,
        sender,
    });
    ctx.request_repaint();
    receiver
}

/// The prompt that is open on `window`, if any: its title.
pub fn open_prompt(ctx: &Context, window: WindowId) -> Option<String> {
    let prompts = prompts(ctx);
    let list = prompts.0.lock().unwrap_or_else(|e| e.into_inner());
    list.open
        .iter()
        .find(|p| p.viewport == window.viewport() && matches!(p.stage, Stage::Prompt))
        .map(|p| p.request.title.clone())
}

/// The note under the field of the prompt open on `window` (why there is no native
/// dialog) and its error, for tests.
pub fn prompt_messages(
    ctx: &Context,
    window: WindowId,
) -> Option<(Option<String>, Option<String>)> {
    let prompts = prompts(ctx);
    let list = prompts.0.lock().unwrap_or_else(|e| e.into_inner());
    list.open
        .iter()
        .find(|p| p.viewport == window.viewport() && matches!(p.stage, Stage::Prompt))
        .map(|p| (p.note.clone(), p.error.clone()))
}

/// Answer the prompt open on `window` as typing `text` and pressing OK does (`None`:
/// Cancel). `Err` when no prompt is open, or the answer is refused (the prompt then
/// stays open with that error).
pub fn answer_prompt(ctx: &Context, window: WindowId, text: Option<&str>) -> Result<(), String> {
    let prompts = prompts(ctx);
    let mut list = prompts.0.lock().unwrap_or_else(|e| e.into_inner());
    let index = list
        .open
        .iter()
        .position(|p| p.viewport == window.viewport() && matches!(p.stage, Stage::Prompt))
        .ok_or("no path prompt is open")?;
    let Some(text) = text else {
        let pending = list.open.remove(index);
        let _ = pending.sender.send(None);
        return Ok(());
    };
    let pending = &mut list.open[index];
    pending.text = text.to_string();
    match accept(&pending.request, &pending.text) {
        Ok(path) => {
            let pending = list.open.remove(index);
            let _ = pending.sender.send(Some(path));
            Ok(())
        }
        Err(error) => {
            pending.error = Some(error.clone());
            Err(error)
        }
    }
}

/// The path a prompt's text means, or why it is no answer.
fn accept(request: &Request, text: &str) -> Result<PathBuf, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("type a path".into());
    }
    let path = resolve(text, request.directory.as_deref(), None)
        .ok_or("a $VARIABLE in the path is not set")?;
    let path = std::path::absolute(&path).unwrap_or(path);
    match request.kind {
        Kind::Folder if path.is_file() => Err(format!("{} is a file", path.display())),
        Kind::Folder if request.must_exist && !path.is_dir() => {
            Err(format!("folder not found: {}", path.display()))
        }
        Kind::File if !path.is_file() => Err(format!("file not found: {}", path.display())),
        Kind::SaveFile if path.is_dir() => Err(format!("{} is a folder", path.display())),
        _ => Ok(path),
    }
}

/// Draw the path prompts of `window` and look after the native dialogs that are
/// open. Call it once per window and frame.
pub fn show(ctx: &Context, window: WindowId) {
    let prompts = prompts(ctx);
    // Taken out while drawing, so that the lock is not held over the UI.
    let mut open = {
        let mut list = prompts.0.lock().unwrap_or_else(|e| e.into_inner());
        if list.open.is_empty() {
            return;
        }
        std::mem::take(&mut list.open)
    };
    let mut shown = false;
    open.retain_mut(|pending| {
        if let Stage::Native(native) = &mut pending.stage {
            match native.poll() {
                Answer::Waiting => return true,
                Answer::Picked(path) => {
                    let _ = pending.sender.send(Some(path));
                    return false;
                }
                Answer::Cancelled => {
                    let _ = pending.sender.send(None);
                    return false;
                }
                Answer::Unavailable => {
                    tracing::warn!(
                        target: "cdept::gui",
                        "the system's file dialog did not open (no desktop portal, no zenity): \
                         asking for the path in CDePT"
                    );
                    pending.stage = Stage::Prompt;
                    pending.note = Some(NO_NATIVE.into());
                    pending.waiting_since = Instant::now();
                    ctx.request_repaint();
                }
            }
        }
        if pending.viewport != window.viewport() {
            // Its window is gone: the main window takes the question over.
            if window == WindowId::Main && pending.waiting_since.elapsed() > ORPHANED {
                pending.viewport = ViewportId::ROOT;
                ctx.request_repaint();
            }
            return true;
        }
        pending.waiting_since = Instant::now();
        if shown {
            return true;
        }
        shown = true;
        prompt(ctx, pending)
    });
    let mut list = prompts.0.lock().unwrap_or_else(|e| e.into_inner());
    // Questions asked while drawing come after the ones that were open.
    open.append(&mut list.open);
    open.sort_by_key(|p| p.serial);
    list.open = open;
}

/// Draw one prompt. `false` once it is answered.
fn prompt(ctx: &Context, pending: &mut Pending) -> bool {
    enum Outcome {
        Open,
        Ok,
        Cancel,
    }
    let mut outcome = Outcome::Open;
    let native_known_missing = pending.note.is_some() || !native_allowed();
    let response =
        egui::Modal::new(Id::new("cdept-path-prompt").with(pending.serial)).show(ctx, |ui| {
            ui.set_width(460.0);
            let title = if pending.request.title.is_empty() {
                match pending.request.kind {
                    Kind::Folder => "Choose a folder",
                    Kind::File => "Choose a file",
                    Kind::SaveFile => "Save as",
                }
            } else {
                pending.request.title.as_str()
            };
            ui.heading(title);
            ui.add_space(6.0);
            if let Some(hint) = &pending.request.hint {
                ui.label(hint);
                ui.add_space(4.0);
            }
            let mut field = PathField::new(pending.serial, &mut pending.text, pending.request.kind)
                .title(title)
                .width(360.0)
                .focus(std::mem::take(&mut pending.focus));
            if let Some(dir) = &pending.request.directory {
                field = field.base(dir);
            }
            for (name, extensions) in &pending.request.filters {
                let extensions: Vec<&str> = extensions.iter().map(String::as_str).collect();
                field = field.filter(name, &extensions);
            }
            if native_known_missing {
                field = field.no_browse();
            }
            if field.show(ui).changed {
                pending.error = None;
            }
            if let Some(note) = &pending.note {
                ui.label(
                    RichText::new(note)
                        .small()
                        .color(ui.visuals().warn_fg_color),
                );
            }
            if let Some(error) = &pending.error {
                ui.add_space(4.0);
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("OK").clicked() {
                    outcome = Outcome::Ok;
                }
                if ui.button("Cancel").clicked() {
                    outcome = Outcome::Cancel;
                }
            });
            if ui.input(|i| i.key_pressed(Key::Enter)) {
                outcome = Outcome::Ok;
            }
        });
    if response.should_close() && matches!(outcome, Outcome::Open) {
        outcome = Outcome::Cancel;
    }
    match outcome {
        Outcome::Open => true,
        Outcome::Cancel => {
            let _ = pending.sender.send(None);
            false
        }
        Outcome::Ok => match accept(&pending.request, &pending.text) {
            Ok(path) => {
                let _ = pending.sender.send(Some(path));
                false
            }
            Err(error) => {
                pending.error = Some(error);
                true
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn names(found: &[Candidate]) -> Vec<String> {
        found
            .iter()
            .map(|c| format!("{}{}", c.name, if c.is_dir { "/" } else { "" }))
            .collect()
    }

    fn tree() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        for dir in [
            "libs/analog",
            "libs/analog2",
            "libs/digital",
            ".git",
            "layout",
        ] {
            fs::create_dir_all(tmp.path().join(dir)).unwrap();
        }
        fs::write(tmp.path().join("libs/notes.txt"), "").unwrap();
        fs::write(tmp.path().join("cdept.toml"), "").unwrap();
        tmp
    }

    #[test]
    fn candidates_are_the_entries_that_start_as_typed() {
        let tmp = tree();
        let base = Some(tmp.path());
        let list = |text: &str, kind| names(&candidates(text, base, None, kind));
        assert_eq!(list("", Kind::Folder), ["layout/", "libs/"]);
        assert_eq!(list("l", Kind::Folder), ["layout/", "libs/"]);
        assert_eq!(list(".", Kind::Folder), [".git/"]);
        assert_eq!(
            list("libs/", Kind::Folder),
            ["analog/", "analog2/", "digital/"]
        );
        assert_eq!(list("libs/a", Kind::Folder), ["analog/", "analog2/"]);
        // Files only when a file is asked for; folders first.
        assert_eq!(
            list("libs/", Kind::File),
            ["analog/", "analog2/", "digital/", "notes.txt"]
        );
        assert_eq!(list("", Kind::SaveFile), ["layout/", "libs/", "cdept.toml"]);
        // A file typed in full has nothing more to offer; no such folder: nothing.
        assert!(list("cdept.toml", Kind::File).is_empty());
        assert!(list("nowhere/", Kind::Folder).is_empty());
        assert!(list("$CDEPT_SURELY_UNSET/", Kind::Folder).is_empty());
        // An absolute path does not need the base; a variable is expanded.
        let absolute = format!("{}/li", tmp.path().display());
        assert_eq!(
            names(&candidates(&absolute, None, None, Kind::Folder)),
            ["libs/"]
        );
        let root = tmp.path().to_string_lossy().into_owned();
        let vars = move |name: &str| (name == "ROOT").then(|| root.clone());
        assert_eq!(
            names(&candidates("$ROOT/libs/d", None, Some(&vars), Kind::Folder)),
            ["digital/"]
        );
    }

    #[test]
    fn tab_completes_what_the_candidates_share() {
        let tmp = tree();
        let base = Some(tmp.path());
        let tab = |text: &str| complete(text, &candidates(text, base, None, Kind::Folder));
        assert_eq!(tab("libs/a").as_deref(), Some("libs/analog"));
        assert_eq!(tab("libs/d").as_deref(), Some("libs/digital/"));
        assert_eq!(tab("l").as_deref(), Some("l"));
        assert_eq!(tab("la").as_deref(), Some("layout/"));
        assert_eq!(tab("zzz"), None);
    }

    #[test]
    fn answers_are_checked() {
        let tmp = tree();
        let request = |kind, must_exist| Request {
            kind,
            directory: Some(tmp.path().to_path_buf()),
            must_exist,
            ..Request::default()
        };
        let folder = request(Kind::Folder, true);
        assert_eq!(
            accept(&folder, "libs/analog").unwrap(),
            tmp.path().join("libs/analog")
        );
        assert!(
            accept(&folder, "libs/none")
                .unwrap_err()
                .contains("not found")
        );
        assert!(
            accept(&folder, "cdept.toml")
                .unwrap_err()
                .contains("is a file")
        );
        assert!(accept(&folder, "  ").is_err());
        assert!(accept(&folder, "$CDEPT_SURELY_UNSET/x").is_err());
        // A new folder is fine when it need not exist.
        assert!(accept(&request(Kind::Folder, false), "libs/none").is_ok());
        assert!(accept(&request(Kind::File, true), "cdept.toml").is_ok());
        assert!(accept(&request(Kind::File, true), "libs").is_err());
        assert!(accept(&request(Kind::SaveFile, false), "out.gds").is_ok());
        assert!(accept(&request(Kind::SaveFile, false), "libs").is_err());
    }

    /// Run frames of a context with the prompts of the main window.
    fn frames(ctx: &Context, n: usize) {
        for _ in 0..n {
            let output = ctx.run_ui(egui::RawInput::default(), |ui| {
                show(ui.ctx(), WindowId::Main);
            });
            output.drop_without_applying_deltas();
        }
    }

    #[test]
    fn a_prompt_is_drawn_and_answered() {
        let tmp = tree();
        let ctx = Context::default();
        let request = Request {
            title: "Open project folder".into(),
            directory: Some(tmp.path().to_path_buf()),
            must_exist: true,
            prompt_first: true,
            ..Request::default()
        };
        let answer = ask(&ctx, WindowId::Main, request.clone());
        frames(&ctx, 2);
        assert_eq!(
            open_prompt(&ctx, WindowId::Main).as_deref(),
            Some("Open project folder")
        );
        // It belongs to the main window only.
        assert!(open_prompt(&ctx, WindowId::Detached(1)).is_none());
        assert!(answer.try_recv().is_err());
        // A wrong answer keeps it open, with the reason.
        assert!(answer_prompt(&ctx, WindowId::Main, Some("libs/none")).is_err());
        frames(&ctx, 1);
        let (_, error) = prompt_messages(&ctx, WindowId::Main).unwrap();
        assert!(error.unwrap().contains("not found"));
        answer_prompt(&ctx, WindowId::Main, Some("libs/analog")).unwrap();
        assert_eq!(
            answer.try_recv().unwrap(),
            Some(tmp.path().join("libs/analog"))
        );
        assert!(open_prompt(&ctx, WindowId::Main).is_none());

        // Cancel answers with nothing.
        let answer = ask(&ctx, WindowId::Main, request);
        frames(&ctx, 1);
        answer_prompt(&ctx, WindowId::Main, None).unwrap();
        assert_eq!(answer.try_recv().unwrap(), None);
        assert!(answer_prompt(&ctx, WindowId::Main, None).is_err());
    }

    #[test]
    fn without_a_native_dialog_the_prompt_takes_over() {
        let tmp = tree();
        let ctx = Context::default();
        let request = Request {
            kind: Kind::SaveFile,
            title: "Export".into(),
            directory: Some(tmp.path().to_path_buf()),
            file_name: Some("out.gds".into()),
            ..Request::default()
        };
        let answer = ask_with(
            &ctx,
            WindowId::Detached(3).viewport(),
            request,
            Some(Native::unavailable()),
        );
        // Whichever window draws next notices; the prompt is for the one that asked.
        frames(&ctx, 1);
        assert!(open_prompt(&ctx, WindowId::Main).is_none());
        assert_eq!(
            open_prompt(&ctx, WindowId::Detached(3)).as_deref(),
            Some("Export")
        );
        let (note, _) = prompt_messages(&ctx, WindowId::Detached(3)).unwrap();
        assert!(note.unwrap().contains("not available"));
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            show(ui.ctx(), WindowId::Detached(3));
        });
        output.drop_without_applying_deltas();
        // The file name asked for is in the field already.
        answer_prompt(
            &ctx,
            WindowId::Detached(3),
            Some(&tmp.path().join("out.gds").to_string_lossy()),
        )
        .unwrap();
        assert_eq!(answer.try_recv().unwrap(), Some(tmp.path().join("out.gds")));
    }
}
