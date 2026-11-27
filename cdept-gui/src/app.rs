//! The application shell: the main window and any detached native windows.
//!
//! Every native window has its own menu bar, toolbar and dock area. The menus are the
//! same global ones everywhere, plus the contextual menus of the active document *of
//! that window*. Only the main window has a status bar.

use std::collections::HashMap;

use egui::{
    Button, Context, Id, KeyboardShortcut, Modifiers, Ui, ViewportBuilder, ViewportCommand,
};
use egui_dock::{DockArea, Style};

use libcdept::xschem::NetlistFormat;
use libcdept::{Cell, Entry, Format, Project};

use crate::dialogs::{self, Dialog};
use crate::dock::{DocId, DocRef, Geometry, Layout, Tab, Viewer, WindowId};
use crate::guide;
use crate::log::LogBuffer;
use crate::script::Script;
use crate::state::{Action, ActiveSchematic, AppCtx, PickFor, Selection, Settings};
use crate::views::{CloseChoice, Document, EditAction, LayoutView, SchematicView, View, Views};
use crate::{APP_ID, GuiOptions};

// Bump the suffix when `Layout` or `Tab` change incompatibly, so stale layouts are dropped.
const LAYOUT_KEY: &str = "cdept.layout.v3";
const SETTINGS_KEY: &str = "cdept.settings";

const QUIT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, egui::Key::Q);
/// The window a launcher button starts (`libcdept::launch::Choice::id`); `None`: the
/// tool runs inside CDePT only (ngspice runs in testbenches).
fn launcher_choice(name: &str) -> Option<&'static str> {
    match name {
        "xschem" => Some("xschem-window"),
        "KLayout" => Some("klayout-window"),
        "Magic" => Some("magic-window"),
        _ => None,
    }
}

/// Global toolbar launchers (tool names from `libcdept::tools::KNOWN_TOOLS`).
const LAUNCHERS: [&str; 4] = ["xschem", "KLayout", "Magic", "ngspice"];

const DETACHED_SIZE: [f32; 2] = [1000.0, 700.0];

/// The formats that open in a document tab: xschem schematics and symbols, layouts
/// (KLayout), LEF abstracts (KLayout, read-only) and Magic layouts (Magic).
fn embedded_format(format: Option<Format>) -> bool {
    matches!(
        format,
        Some(
            Format::XschemSchematic
                | Format::XschemSymbol
                | Format::Gds
                | Format::Oasis
                | Format::Lef
                | Format::Magic
        )
    )
}

/// Whether a view opens in a document tab: schematics, symbols and layouts do.
fn opens_in_tab(entry: &Entry) -> bool {
    crate::views::simulation::is_testbench(entry)
        || embedded_format(entry.format) && entry.main_file.is_some()
}

/// Whether a view opens with a double click: in a tab, or in a tool's window.
pub(crate) fn has_viewer(entry: &Entry) -> bool {
    crate::views::simulation::is_testbench(entry)
        || embedded_format(entry.format) && entry.main_file.is_some()
        || !libcdept::launch::choices_for(entry).is_empty()
}

/// The xschem schematic of a cell: the view named `schematic` if it is one, else the
/// first.
pub(crate) fn schematic_file(cell: &Cell) -> Option<&std::path::PathBuf> {
    let mut views = cell
        .entries
        .iter()
        .filter(|e| e.format == Some(Format::XschemSchematic) && e.main_file.is_some());
    let view = views
        .clone()
        .find(|e| e.name == "schematic")
        .or_else(|| views.next())?;
    view.main_file.as_ref()
}

/// Something that closes documents, held back while the user decides what to do with
/// their unsaved changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Leaving {
    CloseProject,
    LoadProject(std::path::PathBuf),
    NewProject(std::path::PathBuf),
    Quit,
}

/// The answer to "these documents have unsaved changes".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaveChoice {
    SaveAll,
    DiscardAll,
    Cancel,
}

/// The question about unsaved changes before a [`Leaving`].
struct Guard {
    leaving: Leaving,
    /// The documents with unsaved changes.
    documents: Vec<DocId>,
    /// "Save All" was chosen: the save requests still unanswered.
    saving: Option<Vec<(DocId, u64)>>,
}

pub struct CdeptApp {
    layout: Layout,
    views: Views,
    ctx: AppCtx,
    /// Last focused document tab of each window; it owns that window's contextual
    /// menus and toolbar.
    active_docs: HashMap<WindowId, Tab>,
    /// Display name of every window, refreshed each frame.
    window_names: Vec<(WindowId, String)>,
    /// Geometry each detached window was spawned with. The viewport builder must stay
    /// the same across frames, or egui would keep moving the window back.
    spawn_geometry: HashMap<u64, Option<Geometry>>,
    about: Option<WindowId>,
    /// The question about unsaved changes, while closing the project or quitting.
    guard: Option<Guard>,
    /// The user agreed to quit (or nothing stood in the way): let the window close.
    quit_confirmed: bool,
    /// The test script being run, if any.
    script: Option<Script>,
    /// The tutorial being followed, with its card and highlight.
    guide: Option<crate::guide::Guide>,
    /// A tool chosen for a document about to start ("Open with": a GDS in Magic).
    forced_tool: HashMap<DocId, &'static str>,
    /// The waveform tab `wv ...` script commands act on (`None`: the newest).
    script_wave: Option<u64>,
    /// File › Open Waveform File…: the path being asked for.
    wave_pick: Option<std::sync::mpsc::Receiver<Option<std::path::PathBuf>>>,
    /// File › Open in UberWave…: the path being asked for, then the opens running.
    uberwave_pick: Option<std::sync::mpsc::Receiver<Option<std::path::PathBuf>>>,
    uberwave_opens: Vec<std::sync::mpsc::Receiver<(std::path::PathBuf, Result<String, String>)>>,
    /// Where the time of the frames goes (slow frames are logged, `cdept::frame`).
    frame_clock: FrameClock,
}

/// The time of a frame by part, to find what makes one slow: the main window, each
/// detached window (inside its viewport's closure, and the whole call, which includes
/// creating the window), the rest; and the interval since the frame before (painting
/// and swapping happen between our calls).
#[derive(Default)]
struct FrameClock {
    last: Option<std::time::Instant>,
    /// (window, inside the closure ms, whole call ms)
    detached: Vec<(u64, f64, f64)>,
    inside: f64,
}

impl FrameClock {
    /// A frame slower than this is logged (`CDEPT_SLOW_FRAME_MS` changes it).
    fn slow_ms() -> f64 {
        static SLOW: std::sync::OnceLock<f64> = std::sync::OnceLock::new();
        *SLOW.get_or_init(|| {
            std::env::var("CDEPT_SLOW_FRAME_MS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(250.0)
        })
    }
}

impl CdeptApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        options: GuiOptions,
        log: LogBuffer,
        script: Option<Script>,
    ) -> Self {
        log.set_repaint_context(cc.egui_ctx.clone());

        let (layout, settings) = match cc.storage {
            Some(storage) => (
                eframe::get_value::<Layout>(storage, LAYOUT_KEY)
                    .map(Layout::restored)
                    .unwrap_or_default(),
                eframe::get_value::<Settings>(storage, SETTINGS_KEY).unwrap_or_default(),
            ),
            None => (Layout::default(), Settings::default()),
        };
        Self::start(&cc.egui_ctx, layout, settings, options, log, script)
    }

    /// Start with a layout and settings, as restored from the last run or new.
    fn start(
        egui_ctx: &Context,
        layout: Layout,
        settings: Settings,
        options: GuiOptions,
        log: LogBuffer,
        script: Option<Script>,
    ) -> Self {
        let mut ctx = AppCtx::new(settings, log);
        tracing::info!(target: "cdept", "CDePT {} started", libcdept::version::long_version());
        if let Some(path) = options.project {
            crate::background::load_at_start(&mut ctx, egui_ctx, &path);
        }
        ctx.tools.start_detection(egui_ctx);
        let tutorial = options.tutorial;

        let mut app = Self {
            layout,
            views: Views::default(),
            ctx,
            active_docs: HashMap::new(),
            window_names: Vec::new(),
            spawn_geometry: HashMap::new(),
            about: None,
            guard: None,
            quit_confirmed: false,
            script,
            guide: None,
            forced_tool: HashMap::new(),
            script_wave: None,
            wave_pick: None,
            uberwave_pick: None,
            uberwave_opens: Vec::new(),
            frame_clock: FrameClock::default(),
        };
        if let Some(tutorial) = tutorial {
            app.start_tutorial(tutorial);
        }
        // With the project still being read, the documents come back once it is there.
        if !app.ctx.background.restoring() {
            app.restore_documents(egui_ctx);
        }
        app
    }

    /// The file `doc` stands for, if it is one that opens in a document tab.
    fn document_file(&self, doc: &DocRef) -> Option<std::path::PathBuf> {
        if let Some(file) = &doc.file {
            let known = embedded_format(Format::of_file(file));
            return (known && file.is_file()).then(|| file.clone());
        }
        let entry = self
            .ctx
            .project
            .as_ref()?
            .library(&doc.library)?
            .cell(&doc.cell)?
            .entry(&doc.view)?;
        opens_in_tab(entry)
            .then(|| entry.main_file.clone())
            .flatten()
    }

    fn start_document(&mut self, ctx: &Context, id: DocId, doc: DocRef) -> bool {
        let (Some(file), Some(project)) = (self.document_file(&doc), &self.ctx.project) else {
            return false;
        };
        let forced = self.forced_tool.remove(&id);
        match Format::of_file(&file) {
            // A GDS shown by Magic: read into Magic, looked at (it stays a GDS view).
            Some(Format::Gds) if forced == Some("magic") => {
                let view = crate::views::MagicView::open(id, doc, file, false, project, ctx);
                self.views.add_document(id, view);
            }
            Some(Format::Magic) => {
                let editable = project
                    .library(&doc.library)
                    .is_none_or(|library| !library.read_only);
                let view = crate::views::MagicView::open(id, doc, file, editable, project, ctx);
                self.views.add_document(id, view);
            }
            Some(Format::Gds | Format::Oasis | Format::Lef) => {
                // Layouts of read-only libraries open for looking only.
                // One cell of a file that holds several (a standard-cell GDS) too:
                // the file belongs to all of them. A LEF abstract never opens for
                // editing.
                let member = crate::views::layout_member_of(project, &doc);
                let editable = member.is_none()
                    && Format::of_file(&file) != Some(Format::Lef)
                    && project
                        .library(&doc.library)
                        .is_none_or(|library| !library.read_only);
                let mut view = LayoutView::open(id, doc, file, editable, project, ctx);
                view.set_member(member);
                self.views.add_document(id, view);
            }
            _ => {
                let view = SchematicView::open(id, doc, file, project, ctx);
                self.views.add_document(id, view);
            }
        }
        true
    }

    /// After a start: reopen the documents that were open in this project.
    fn restore_documents(&mut self, ctx: &Context) {
        let root = self.ctx.project.as_ref().map(|p| p.root.clone());
        if self.layout.documents.project != root {
            self.layout.retain_documents(|_| false);
            self.layout.documents.project = root;
        }
        let open: Vec<(DocId, DocRef)> = self
            .layout
            .documents
            .iter()
            .map(|(id, doc)| (id, doc.clone()))
            .collect();
        for (id, doc) in open {
            if !self.start_document(ctx, id, doc) {
                self.layout.close_tab(Tab::Document(id));
            }
        }
        self.layout.sync_documents();
    }

    /// Start the open documents again in their tabs, so they run with the project's
    /// present technology. Documents with unsaved changes are left as they are.
    fn restart_documents(&mut self, ctx: &Context) {
        let modified = self.modified_documents();
        let open: Vec<(DocId, DocRef)> = self
            .layout
            .documents
            .iter()
            .filter(|(id, _)| !modified.contains(id))
            .map(|(id, doc)| (id, doc.clone()))
            .collect();
        let mut restarted = 0;
        for (id, doc) in open {
            if self.start_document(ctx, id, doc) {
                restarted += 1;
            }
        }
        tracing::info!(
            target: "cdept::tech",
            "{restarted} tab(s) reopened with the new technology{}",
            match modified.len() {
                0 => String::new(),
                n => format!("; {n} with unsaved changes left as they are"),
            }
        );
    }

    /// Open a view of a cell in a document tab, or show the tab it is already in.
    fn open_view(&mut self, ctx: &Context, selection: Selection) {
        // A testbench opens in the Simulation tab.
        let is_testbench = self.ctx.project.as_ref().is_some_and(|p| {
            selection.view.is_some()
                && crate::views::simulation::testbench_of(p, &selection).is_some()
        });
        if is_testbench {
            return self.apply(
                ctx,
                Action::OpenSimulation {
                    selection,
                    run: false,
                },
            );
        }
        // Several tabs can show it (a GDS in KLayout or Magic), or only tool windows
        // can (a Qucs-S schematic, a netlist): the user's choice for the format, else
        // ask. The windows of the tools that have a tab are in "Open With".
        let choices = self
            .view_entry(&selection)
            .map(libcdept::launch::choices_for);
        let tabs = choices
            .as_ref()
            .map_or(0, |c| c.iter().filter(|c| c.embedded).count());
        if let Some([only]) = choices.as_deref()
            && !only.embedded
        {
            let tool = only.id;
            return self.open_with(ctx, selection, tool);
        }
        if let (Some(choices), Some(format)) = (choices, self.view_format(&selection))
            && (tabs > 1 || (tabs == 0 && choices.len() > 1))
        {
            let key = format.label().to_string();
            match self.ctx.settings.open_with.get(&key).cloned() {
                Some(tool) if choices.iter().any(|c| c.id == tool) => {
                    return self.open_with(ctx, selection, &tool);
                }
                _ => {
                    self.ctx.open_choice = Some((selection, choices, false));
                    return;
                }
            }
        }
        if let (Some(library), Some(cell), Some(view)) =
            (selection.library, selection.cell, selection.view)
        {
            self.open_document(ctx, DocRef::view(&library, &cell, &view));
        }
    }

    fn view_entry(&self, selection: &Selection) -> Option<&Entry> {
        let (Some(library), Some(cell), Some(view)) =
            (&selection.library, &selection.cell, &selection.view)
        else {
            return None;
        };
        self.ctx
            .project
            .as_ref()?
            .library(library)?
            .cell(cell)?
            .entry(view)
    }

    fn view_format(&self, selection: &Selection) -> Option<Format> {
        self.view_entry(selection)?.format
    }

    /// Open a view with one tool: in a tab (embedded), or in the tool's own window.
    fn open_with(&mut self, ctx: &Context, selection: Selection, tool: &str) {
        let Some(choice) = libcdept::launch::choice(tool) else {
            return;
        };
        let (Some(library), Some(cell), Some(view)) = (
            selection.library.clone(),
            selection.cell.clone(),
            selection.view.clone(),
        ) else {
            return;
        };
        if choice.embedded {
            let doc = DocRef::view(&library, &cell, &view);
            if self.document_file(&doc).is_none() {
                return;
            }
            let (id, win, new) = self.layout.open_document(doc.clone());
            if new {
                self.forced_tool.insert(id, choice.id);
                self.start_document(ctx, id, doc);
            }
            Self::focus(ctx, win);
            return;
        }
        let file = self
            .view_entry(&selection)
            .and_then(|e| e.main_file.clone());
        let launched = self
            .ctx
            .launcher
            .open(self.ctx.project.as_ref(), choice, file.as_deref());
        match launched {
            Ok(libcdept::launch::Launched::Started { log, .. }) => tracing::info!(
                target: "cdept::tools",
                "{library}/{cell}/{view} opened in {} (log {})",
                choice.label,
                log.display()
            ),
            Ok(libcdept::launch::Launched::Reused(text)) => {
                tracing::info!(target: "cdept::tools", "{text}")
            }
            Err(e) => self.ctx.error(format!("{}: {e:#}", choice.label)),
        }
    }

    /// How many tools the "Open with" question offers, while it is up (test scripts).
    pub(crate) fn script_open_choice(&self) -> Option<usize> {
        self.ctx.open_choice.as_ref().map(|(_, c, _)| c.len())
    }

    pub(crate) fn script_close_open_choice(&mut self) {
        self.ctx.open_choice = None;
    }

    /// What a test script's `open` does: the first tab that can show the view.
    pub(crate) fn script_open_action(&self, selection: Selection) -> Action {
        let tabs: Vec<libcdept::launch::Choice> = self
            .view_entry(&selection)
            .map(libcdept::launch::choices_for)
            .unwrap_or_default()
            .into_iter()
            .filter(|c| c.embedded)
            .collect();
        match tabs.as_slice() {
            [first, _, ..] => Action::OpenWith(selection, first.id.to_string()),
            _ => Action::OpenView(selection),
        }
    }

    /// Start a tool's window without a file (tool bar, Tools menu).
    fn launch(&mut self, choice: libcdept::launch::Choice) {
        match self
            .ctx
            .launcher
            .open(self.ctx.project.as_ref(), choice, None)
        {
            Ok(libcdept::launch::Launched::Started { log, .. }) => {
                tracing::info!(target: "cdept::tools", "{} started (log {})", choice.label, log.display())
            }
            Ok(libcdept::launch::Launched::Reused(text)) => {
                tracing::info!(target: "cdept::tools", "{text}")
            }
            Err(e) => self.ctx.error(format!("{}: {e:#}", choice.label)),
        }
    }

    /// The question "open with which tool?" after a double click.
    fn open_choice_ui(&mut self, ctx: &Context) {
        let Some((selection, choices, mut remember)) = self.ctx.open_choice.clone() else {
            return;
        };
        let mut picked = None;
        let mut cancel = false;
        let modal = egui::Modal::new(egui::Id::new("open-with")).show(ctx, |ui| {
            ui.set_max_width(460.0);
            ui.heading("Open with");
            ui.label(format!(
                "{}/{}/{}",
                selection.library.as_deref().unwrap_or(""),
                selection.cell.as_deref().unwrap_or(""),
                selection.view.as_deref().unwrap_or("")
            ));
            ui.add_space(6.0);
            for c in &choices {
                if ui.button(c.label).clicked() {
                    picked = Some(c.id);
                }
            }
            ui.add_space(6.0);
            ui.checkbox(&mut remember, "Always open this kind of view this way");
            if ui.button("Cancel").clicked() {
                cancel = true;
            }
        });
        if modal.should_close() {
            cancel = true;
        }
        if let Some(tool) = picked {
            if remember && let Some(format) = self.view_format(&selection) {
                self.ctx
                    .settings
                    .open_with
                    .insert(format.label().to_string(), tool.to_string());
            }
            self.ctx.open_choice = None;
            self.open_with(ctx, selection, tool);
        } else if cancel {
            self.ctx.open_choice = None;
        } else if let Some(state) = self.ctx.open_choice.as_mut() {
            state.2 = remember;
        }
    }

    /// Open a file in a document tab: as the view of a library cell when it is one.
    fn open_file(&mut self, ctx: &Context, file: &std::path::Path) {
        let same = |a: &std::path::Path| match (a.canonicalize(), file.canonicalize()) {
            (Ok(a), Ok(b)) => a == b,
            _ => a == file,
        };
        let view = self.ctx.project.as_ref().and_then(|project| {
            project.libraries.iter().find_map(|library| {
                library.cells.iter().find_map(|cell| {
                    let entry = cell
                        .entries
                        .iter()
                        .find(|e| e.main_file.as_deref().is_some_and(same))?;
                    Some(DocRef::view(&library.name, &cell.name, &entry.name))
                })
            })
        });
        let doc = view.unwrap_or_else(|| DocRef::file(file.to_path_buf()));
        self.open_document(ctx, doc);
    }

    fn open_document(&mut self, ctx: &Context, doc: DocRef) {
        if self.document_file(&doc).is_none() {
            let name = match &doc.file {
                Some(file) => file.display().to_string(),
                None => format!("{}/{}/{}", doc.library, doc.cell, doc.view),
            };
            tracing::info!(
                target: "cdept::library",
                "{name}: no embedded viewer for this kind of view yet"
            );
            return;
        }
        let (id, win, new) = self.layout.open_document(doc.clone());
        if new {
            self.start_document(ctx, id, doc);
        }
        Self::focus(ctx, win);
    }

    /// The document a test script acts on: the active one of the main window, else of
    /// another window, else the one opened last.
    pub(crate) fn script_document(&self) -> Option<Tab> {
        let active = |win: &WindowId| match self.active_docs.get(win) {
            Some(tab @ Tab::Document(id)) if self.layout.documents.get(*id).is_some() => Some(*tab),
            _ => None,
        };
        self.layout
            .window_ids()
            .iter()
            .find_map(active)
            .or_else(|| {
                let last = self.layout.documents.iter().last()?;
                Some(Tab::Document(last.0))
            })
    }

    /// The document the script acts on, whatever it is.
    pub(crate) fn script_view(&mut self) -> Option<&mut Document> {
        match self.script_document()? {
            Tab::Document(id) => self.views.document_mut(id),
            _ => None,
        }
    }

    pub(crate) fn script_schematic(&mut self) -> Option<&mut SchematicView> {
        self.script_view()?.as_schematic_mut()
    }

    /// The script's document when it runs Tcl: a schematic (xschem) or a Magic layout.
    pub(crate) fn script_tcl_view(&mut self) -> Option<&mut crate::views::Document> {
        self.script_view().filter(|d| d.runs_tcl())
    }

    pub(crate) fn script_layout(&mut self) -> Option<&mut LayoutView> {
        self.script_view()?.as_layout_mut()
    }

    /// Place a cell of the project into the script's layout.
    pub(crate) fn script_place(&mut self, selection: &Selection) -> Result<u64, String> {
        let project = self.ctx.project.as_ref().ok_or("no project is open")?;
        let (library, cell, view) = match (&selection.library, &selection.cell, &selection.view) {
            (Some(l), Some(c), Some(v)) => (l, c, v),
            _ => return Err("place needs a library, a cell and a view".into()),
        };
        let file = project
            .library(library)
            .and_then(|l| l.cell(cell))
            .and_then(|c| c.entry(view))
            .and_then(|e| e.main_file.clone())
            .ok_or("there is no such view")?;
        let Some(Tab::Document(id)) = self.script_document() else {
            return Err("no layout is open".into());
        };
        let layout = self
            .views
            .document_mut(id)
            .and_then(Document::as_layout_mut)
            .ok_or("no layout is open")?;
        Ok(layout.place(project, cell, &file))
    }

    /// Where a position given in frame pixels of the script's schematic is on screen.
    /// Without a schematic on screen the position is taken as window points.
    pub(crate) fn script_screen(&self, pos: egui::Pos2) -> egui::Pos2 {
        let view = match self.script_document() {
            Some(Tab::Document(id)) => self.views.document(id),
            _ => None,
        };
        view.and_then(|v| v.frame_to_screen(pos.x, pos.y))
            .unwrap_or(pos)
    }

    pub(crate) fn script_request(&mut self, action: Action) {
        self.ctx.request(action);
    }

    /// Close the script's document the way the X on its tab does: a document with
    /// unsaved changes asks first. Returns whether it is asking.
    pub(crate) fn script_close_tab(&mut self) -> Option<bool> {
        let tab = self.script_document()?;
        let view = self.script_view()?;
        if view.may_close() {
            self.ctx.request(Action::CloseTab(tab));
        }
        Some(self.script_view()?.is_closing())
    }

    /// Answer the question about unsaved changes of the script's document.
    pub(crate) fn script_close_with(&mut self, choice: CloseChoice) -> bool {
        let Some(Tab::Document(id)) = self.script_document() else {
            return false;
        };
        match self.views.document_mut(id) {
            Some(view) if view.asks_to_close() => {
                view.close_with(choice, &mut self.ctx);
                true
            }
            _ => false,
        }
    }

    /// Title of the script's document tab.
    pub(crate) fn script_title(&mut self) -> Option<String> {
        let tab = self.script_document()?;
        Some(self.views.get(tab).title(&self.ctx))
    }

    pub(crate) fn script_document_count(&self) -> usize {
        self.layout.documents.iter().count()
    }

    /// Bring the panel with this title (`Hierarchy`, `Inspector`, …) to the front.
    pub(crate) fn script_show_panel(&mut self, name: &str) -> bool {
        let tab = Tab::ALL.into_iter().find(|tab| {
            self.views
                .get(*tab)
                .title(&self.ctx)
                .eq_ignore_ascii_case(name.trim())
        });
        tab.inspect(|tab| self.ctx.request(Action::OpenTab(*tab)))
            .is_some()
    }

    /// A waveform tab on the run points of a testbench view (all, or the ones named):
    /// every point folder, also one without a raw file (it shows as failed). Returns the
    /// tab's id.
    fn open_wave_family(
        &mut self,
        ctx: &Context,
        run: (String, String, String),
        only: Option<&[String]>,
    ) -> Option<u64> {
        let Some(project) = self.ctx.project.clone() else {
            self.ctx
                .error("Waveform Tab (family): no project is open".into());
            return None;
        };
        let (library, cell, view) = &run;
        let dir = libcdept::sim::runs_dir(&project, library, cell, view);
        let mut files: Vec<(String, std::path::PathBuf)> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().is_dir())
            .map(|e| {
                (
                    e.file_name().to_string_lossy().into_owned(),
                    crate::views::wave::script::point_raw(&e.path()),
                )
            })
            .filter(|(p, _)| only.is_none_or(|o| o.contains(p)))
            .collect();
        files.sort_by(|a, b| natural_cmp(&a.0, &b.0));
        // point folders an earlier run plan left (a sweep that had other values): only
        // the points of the newest setup's runs, when the results store knows them
        if only.is_none()
            && let Ok(store) = libcdept::sim::results::Store::open(&project.root)
            && let Ok(runs) = store.runs(library, cell, view)
            && let Some(newest) = runs.last()
            && let Ok(current) = store.latest_of_setup(library, cell, view, &newest.setup_hash)
        {
            let names: std::collections::HashSet<String> =
                current.iter().map(|r| r.point.name.clone()).collect();
            let before = files.len();
            let kept: Vec<_> = files
                .iter()
                .filter(|(p, _)| names.contains(p))
                .cloned()
                .collect();
            if !kept.is_empty() && kept.len() < before {
                tracing::info!(
                    target: "cdept::wave",
                    "{library}/{cell}/{view}: {} point folder(s) of an earlier run plan left out of the family",
                    before - kept.len()
                );
                files = kept;
            }
        }
        if files.is_empty() {
            self.ctx.error(format!(
                "Waveform Tab (family): {library}/{cell}/{view} has no run point (run the testbench first)"
            ));
            return None;
        }
        let show = libcdept::wave::default_show(&project, library, cell, view);
        let title = if files.len() == 1 {
            format!("{cell} {}", files[0].0)
        } else {
            format!("{cell} {view} ({} points)", files.len())
        };
        let tab = self
            .views
            .add_wave_files(ctx, title, files, Some(run.clone()), Some(show));
        let win = self.layout.open_tab(tab);
        Self::focus(ctx, win);
        match tab {
            Tab::Wave(id) => Some(id),
            _ => None,
        }
    }

    /// "Send to tab" of the calculator: into the newest waveform tab of that testbench
    /// view, else a new tab on the points evaluated.
    fn wave_send(&mut self, ctx: &Context, send: crate::views::simulation::TabSend) {
        let id = match self.views.wave_for_run(&send.run) {
            Some(id) => {
                let win = self.layout.open_tab(Tab::Wave(id));
                Self::focus(ctx, win);
                Some(id)
            }
            None => {
                let names: Vec<String> = send.points.iter().map(|p| p.point.clone()).collect();
                self.open_wave_family(ctx, send.run.clone(), Some(&names))
            }
        };
        let Some(view) = id.and_then(|id| self.views.wave_mut(id)) else {
            return;
        };
        let names: Vec<String> = send.traces.iter().map(|(_, t)| t.name.clone()).collect();
        for (point, t) in send.traces {
            view.add_series(
                &point,
                cdept_wavedata::Series::new(
                    t.name,
                    t.unit.unwrap_or_default(),
                    t.x_unit.unwrap_or_default(),
                    t.x,
                    t.y,
                ),
            );
        }
        view.set_markers(send.markers);
        view.calc_layout(&names, send.layout);
    }

    /// File › Open in UberWave…: a picked path starts an open in a thread; a failure
    /// goes to the log and to a dialog.
    fn poll_uberwave_opens(&mut self, ctx: &Context) {
        if let Some(rx) = &self.uberwave_pick {
            match rx.try_recv() {
                Ok(Some(path)) => {
                    self.uberwave_pick = None;
                    self.script_uberwave_open(ctx, path);
                }
                Ok(None) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.uberwave_pick = None
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        let mut errors = Vec::new();
        self.uberwave_opens.retain(|rx| match rx.try_recv() {
            Ok((_, Ok(line))) => {
                tracing::info!(target: "cdept::wave", "UberWave: {line}");
                false
            }
            Ok((path, Err(e))) => {
                errors.push(format!("Open in UberWave: {}: {e}", path.display()));
                false
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => true,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => false,
        });
        for e in errors {
            self.ctx.error(e);
        }
    }

    /// What File › Open in UberWave… does with the picked path (`wave menu_open`).
    pub(crate) fn script_uberwave_open(&mut self, ctx: &Context, path: std::path::PathBuf) {
        let rx = crate::uberwave::open_any_in_thread(self.ctx.project.as_ref(), path, ctx);
        self.uberwave_opens.push(rx);
    }

    /// The message of the error dialog that is up, if one is.
    pub(crate) fn script_error_dialog(&self) -> Option<String> {
        match crate::dialogs::open(&self.ctx) {
            Some(crate::dialogs::Dialog::Error(message)) => Some(message.clone()),
            _ => None,
        }
    }

    /// Close the dialog that is up (its OK or Cancel).
    pub(crate) fn script_dismiss_dialog(&mut self) -> bool {
        self.ctx.dialog.take().is_some()
    }

    /// The opens of File › Open in UberWave… still running.
    pub(crate) fn script_uberwave_opens(&self) -> usize {
        self.uberwave_opens.len()
    }

    /// `detach_panel <title>`: the panel moves to a window of its own.
    pub(crate) fn script_detach_panel(&mut self, name: &str) -> bool {
        let tab = Tab::ALL.into_iter().find(|tab| {
            self.views
                .get(*tab)
                .title(&self.ctx)
                .eq_ignore_ascii_case(name.trim())
        });
        tab.inspect(|tab| self.ctx.request(Action::DetachTab(*tab)))
            .is_some()
    }

    /// `tech …` of a test script: the New Project wizard and the technology.
    pub(crate) fn script_tech(&mut self, ctx: &Context, args: &[String]) -> Result<(), String> {
        crate::newproject::script(&mut self.ctx, ctx, args)
    }

    /// `sim …` of a test script (views/simulation.rs).
    pub(crate) fn script_sim(&mut self, args: &[String]) -> Result<(), String> {
        crate::views::simulation::script(self.views.simulation_mut(), &mut self.ctx, args)
    }

    /// `organise …` of a test script: Go to Cell, categories, Import Project, and the
    /// Search panel (`search <query>`, `expect_hits <n>`, `expect_hit <text>`).
    pub(crate) fn script_organise(&mut self, ctx: &Context, args: &[String]) -> Result<(), String> {
        let text = args.get(1..).unwrap_or_default().join(" ");
        match args.first().map(String::as_str) {
            Some("search") => {
                let project = self.ctx.project.as_ref().ok_or("no project is open")?;
                self.views.search_mut().script_search(ctx, project, &text);
                Ok(())
            }
            Some("expect_hits" | "expect_hit") if self.views.search_mut().script_busy() => {
                Err("the search is still running".into())
            }
            Some("expect_hits") => {
                let hits = self.views.search_mut().script_hits();
                let wanted: usize = text
                    .trim()
                    .parse()
                    .map_err(|_| "not a number".to_string())?;
                if hits.len() == wanted {
                    Ok(())
                } else {
                    Err(format!("{} hits, expected {wanted}: {hits:?}", hits.len()))
                }
            }
            Some("expect_hit") => {
                let hits = self.views.search_mut().script_hits();
                if hits.iter().any(|hit| hit.contains(text.trim())) {
                    Ok(())
                } else {
                    Err(format!("no hit has '{}': {hits:?}", text.trim()))
                }
            }
            _ => crate::import::script(&mut self.ctx, ctx, args),
        }
    }

    /// The lines of the Hierarchy panel.
    /// Run a script command of refactor.rs.
    pub(crate) fn script_refactor(
        &mut self,
        command: crate::refactor::ScriptCommand,
    ) -> Result<(), String> {
        crate::refactor::script_run(command, &mut self.ctx, &mut self.views)
    }

    pub(crate) fn script_hierarchy(&self) -> Vec<String> {
        self.views.hierarchy().lines()
    }

    /// The schematic of a cell shown in the Hierarchy panel.
    pub(crate) fn script_hierarchy_file(&self, cell: &str) -> Option<std::path::PathBuf> {
        self.views.hierarchy().schematic_of(cell)
    }

    /// Whether the Inspector shows the picture of the selected view.
    pub(crate) fn script_preview(&self) -> bool {
        let file = self.ctx.selected_view().and_then(|e| e.main_file.as_ref());
        file.is_some_and(|file| self.ctx.engines.preview_shows(file))
    }

    pub(crate) fn script_select(&mut self, selection: Selection) {
        self.ctx.selection = selection;
        self.ctx.validate_selection();
    }

    /// For the commands of `background::script`: the shared state and the document
    /// the script acts on.
    pub(crate) fn script_disk(&mut self) -> (&mut AppCtx, Option<&mut Document>) {
        let doc = match self.script_document() {
            Some(Tab::Document(id)) => self.views.document_mut(id),
            _ => None,
        };
        (&mut self.ctx, doc)
    }

    /// The shared state, for the test-script commands of other modules.
    pub(crate) fn script_ctx(&mut self) -> &mut AppCtx {
        &mut self.ctx
    }

    /// The waveform tab of `wv ...` commands: the one chosen with `wv select`, else
    /// the newest.
    pub(crate) fn script_wave(&mut self) -> Result<&mut crate::views::wave::WaveView, String> {
        let ids = self.views.wave_ids();
        let id = match self.script_wave {
            Some(id) if ids.contains(&id) => id,
            Some(id) => return Err(format!("waveform tab {id} is closed")),
            None => *ids.last().ok_or("no waveform tab is open")?,
        };
        self.views
            .wave_mut(id)
            .ok_or_else(|| "no waveform tab".into())
    }

    /// The ids of the waveform tabs, oldest first, and the one `wv` acts on.
    pub(crate) fn script_wave_ids(&self) -> (Vec<u64>, Option<u64>) {
        let ids = self.views.wave_ids();
        let current = self
            .script_wave
            .filter(|id| ids.contains(id))
            .or(ids.last().copied());
        (ids, current)
    }

    pub(crate) fn script_select_wave(&mut self, id: Option<u64>) {
        self.script_wave = id;
    }

    pub(crate) fn script_project(&self) -> Option<&Project> {
        self.ctx.project.as_ref()
    }

    /// What the Inspector shows as selected in the schematic in front.
    pub(crate) fn script_selected(&self) -> Vec<String> {
        let selection = self
            .ctx
            .schematic
            .as_ref()
            .and_then(|s| s.selection.as_ref());
        selection.map_or_else(Vec::new, |s| {
            s.instances.iter().map(|i| i.name.clone()).collect()
        })
    }

    pub(crate) fn script_netlisting(&self) -> bool {
        self.ctx.engines.netlisting()
    }

    /// Answer the question about unsaved changes asked before closing the project or
    /// quitting. Returns whether it was being asked.
    pub(crate) fn script_leave_with(&mut self, ctx: &Context, choice: LeaveChoice) -> bool {
        let asking = self.guard.as_ref().is_some_and(|g| g.saving.is_none());
        if asking {
            self.leave_with(ctx, choice);
        }
        asking
    }

    /// End the program without asking about unsaved changes.
    pub(crate) fn script_quit(&mut self, ctx: &Context) {
        self.guard = None;
        self.quit_confirmed = true;
        ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, ViewportCommand::Close);
    }

    /// A script command failed: end the program at the end of this frame, without
    /// asking about unsaved changes.
    pub(crate) fn script_fail(&mut self) {
        self.guard = None;
        self.quit_confirmed = true;
        self.ctx.request(Action::Quit);
    }

    /// The documents with changes that are not saved.
    fn modified_documents(&self) -> Vec<DocId> {
        let mut modified: Vec<DocId> = self
            .views
            .documents()
            .filter(|(_, view)| view.is_modified())
            .map(|(id, _)| id)
            .collect();
        modified.sort();
        modified
    }

    /// Do something that closes the open documents: right away when none has unsaved
    /// changes, else after asking.
    fn leave(&mut self, ctx: &Context, leaving: Leaving) {
        // Opening the project that is open keeps its documents.
        let stays = match &leaving {
            Leaving::LoadProject(path) | Leaving::NewProject(path) => {
                let root = Project::find_root(path).unwrap_or_else(|| path.clone());
                self.ctx.project.as_ref().is_some_and(|p| p.root == root)
            }
            Leaving::CloseProject | Leaving::Quit => false,
        };
        let documents = self.modified_documents();
        let agreed = leaving == Leaving::Quit && self.quit_confirmed;
        if stays || agreed || documents.is_empty() {
            return self.do_leave(ctx, leaving);
        }
        self.guard = Some(Guard {
            leaving,
            documents,
            saving: None,
        });
        Self::focus(ctx, WindowId::Main);
    }

    fn do_leave(&mut self, ctx: &Context, leaving: Leaving) {
        self.guard = None;
        self.ctx.window = WindowId::Main;
        match leaving {
            Leaving::CloseProject => self.ctx.close_project(),
            Leaving::LoadProject(path) => crate::background::load(&mut self.ctx, ctx, &path),
            Leaving::NewProject(path) => self.ctx.new_project(&path),
            Leaving::Quit => {
                self.quit_confirmed = true;
                ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, ViewportCommand::Close);
            }
        }
    }

    fn leave_with(&mut self, ctx: &Context, choice: LeaveChoice) {
        let Some(guard) = &mut self.guard else {
            return;
        };
        match choice {
            LeaveChoice::Cancel => self.guard = None,
            LeaveChoice::DiscardAll => {
                let leaving = guard.leaving.clone();
                self.do_leave(ctx, leaving);
            }
            LeaveChoice::SaveAll => {
                let requests = guard
                    .documents
                    .iter()
                    .filter_map(|id| Some((*id, self.views.document_mut(*id)?.save())))
                    .collect();
                guard.saving = Some(requests);
            }
        }
    }

    /// The question about unsaved changes, and the wait for the saves.
    fn guard_ui(&mut self, ctx: &Context) {
        let Some(guard) = &mut self.guard else {
            return;
        };
        if let Some(saving) = &mut guard.saving {
            // xschem may ask where to save (a new file): no dialog of ours in its way.
            let mut failed = None;
            saving.retain(|(id, request)| {
                let Some(view) = self.views.document_mut(*id) else {
                    return false;
                };
                match view.take_saved(*request) {
                    None => view.is_running(),
                    Some(Ok(())) => false,
                    Some(Err(error)) => {
                        let title = view.title(&self.ctx);
                        let title = title.trim_end_matches('*');
                        failed = Some(format!("{title} was not saved: {error}"));
                        false
                    }
                }
            });
            if let Some(message) = failed {
                self.guard = None;
                self.ctx.window = WindowId::Main;
                self.ctx.error(message);
            } else if saving.is_empty() {
                let leaving = guard.leaving.clone();
                self.do_leave(ctx, leaving);
            }
            return;
        }

        let titles: Vec<String> = guard
            .documents
            .iter()
            .filter_map(|id| {
                let title = self.views.document(*id)?.title(&self.ctx);
                Some(title.trim_end_matches('*').to_string())
            })
            .collect();
        let heading = match guard.leaving {
            Leaving::Quit => "Quit with unsaved changes?",
            _ => "Close the project with unsaved changes?",
        };
        let mut choice = None;
        let response = egui::Modal::new(Id::new("unsaved-documents")).show(ctx, |ui| {
            ui.set_max_width(440.0);
            ui.heading(heading);
            ui.label("These documents have changes that are not saved:");
            for title in &titles {
                ui.label(format!("    {title}"));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                for (label, answer) in [
                    ("Save All", LeaveChoice::SaveAll),
                    ("Discard All", LeaveChoice::DiscardAll),
                    ("Cancel", LeaveChoice::Cancel),
                ] {
                    if ui.button(label).clicked() {
                        choice = Some(answer);
                    }
                }
            });
        });
        if response.should_close() {
            choice.get_or_insert(LeaveChoice::Cancel);
        }
        if let Some(choice) = choice {
            self.leave_with(ctx, choice);
        }
    }

    /// Tell the panels which schematic is in front: the active document of the main
    /// window, else of another window.
    fn publish_schematic(&mut self) {
        let active =
            self.layout
                .window_ids()
                .into_iter()
                .find_map(|win| match self.active_docs.get(&win) {
                    Some(Tab::Document(id)) => Some(*id),
                    _ => None,
                });
        self.ctx.open_documents = self.layout.documents.iter().count();
        self.ctx.refactor.documents = crate::refactor::open_files(&self.views);
        self.ctx.schematic = active.and_then(|id| {
            let view = self.views.document(id)?.as_schematic()?;
            Some(ActiveSchematic {
                id,
                doc: view.doc().clone(),
                file: view.file().to_path_buf(),
                title: view.title(&self.ctx),
                modified: view.is_modified(),
                selection: view.selection().cloned(),
            })
        });
    }

    /// Start writing the netlist of a cell into its `netlist` view.
    fn netlist_to_view(&mut self, ctx: &Context, library: &str, cell: &str, format: NetlistFormat) {
        let Some(project) = &self.ctx.project else {
            return;
        };
        let Some(lib) = project.library(library) else {
            tracing::error!(target: "cdept::netlist", "no library '{library}'");
            return;
        };
        // The netlist comes from the files: say so when a document has moved on.
        let schematic = lib.cell(cell).and_then(schematic_file);
        let unsaved = self.views.documents().any(|(_, view)| {
            view.is_modified() && Some(view.file()) == schematic.map(|f| f.as_path())
        });
        if unsaved {
            tracing::warn!(
                target: "cdept::netlist",
                "{library}/{cell} has changes that are not saved: the netlist is made from \
                 the file on disk"
            );
        }
        tracing::info!(
            target: "cdept::netlist",
            "writing the {} netlist of {library}/{cell}…",
            format.name()
        );
        self.ctx.engines.netlist(project, ctx, lib, cell, format);
    }

    /// Take in the netlists that are done: show the new views, report failures.
    fn finish_netlists(&mut self) {
        let done = self.ctx.engines.finished_netlists();
        let mut written = false;
        for (cell, result) in done {
            match result {
                Ok(file) => {
                    written = true;
                    tracing::info!(target: "cdept::netlist", "{cell}: wrote {}", file.display());
                }
                Err(error) => {
                    tracing::error!(target: "cdept::netlist", "{cell}: no netlist: {error}");
                }
            }
        }
        if written {
            self.ctx.reload_project();
        }
    }

    /// Keep the documents in step with the tabs and the project: a closed tab closes
    /// its document, another project closes them all.
    fn sync_documents(&mut self) {
        if self.ctx.background.restoring() {
            return;
        }
        let root = self.ctx.project.as_ref().map(|p| p.root.clone());
        if self.layout.documents.project != root {
            self.layout.retain_documents(|_| false);
            self.layout.documents.project = root;
            self.ctx.engines.reset();
        }
        self.layout.sync_documents();
        let documents = &self.layout.documents;
        self.views
            .retain_documents(|id| documents.get(id).is_some());
        self.views.retain_waves(&self.layout.wave_tabs());
    }

    fn active_doc(&self, win: WindowId) -> Option<Tab> {
        self.active_docs.get(&win).copied()
    }

    /// Track the focused document of `win`, dropping it once it left the window.
    fn update_active_doc(&mut self, win: WindowId) {
        let Some(dock) = self.layout.dock_mut(win) else {
            self.active_docs.remove(&win);
            return;
        };
        if let Some((_, tab)) = dock.find_active_focused()
            && tab.is_document()
        {
            self.active_docs.insert(win, *tab);
        }
        if let Some(tab) = self.active_docs.get(&win)
            && dock.find_tab(tab).is_none()
        {
            self.active_docs.remove(&win);
        }
        if let std::collections::hash_map::Entry::Vacant(entry) = self.active_docs.entry(win)
            // Fall back to any open document so its menus are reachable.
            && let Some(tab) = dock
                .iter_all_tabs()
                .map(|(_, t)| *t)
                .find(|t| t.is_document())
        {
            entry.insert(tab);
        }
    }

    fn refresh_window_names(&mut self) {
        let mut names = Vec::new();
        for (n, win) in self.layout.window_ids().into_iter().enumerate() {
            let name = match win {
                WindowId::Main => "Main Window".to_string(),
                WindowId::Detached(_) => {
                    let tab = self.active_doc(win).or_else(|| {
                        self.layout
                            .dock(win)?
                            .iter_all_tabs()
                            .map(|(_, t)| *t)
                            .next()
                    });
                    match tab {
                        Some(tab) => {
                            format!("Window {n}: {}", self.views.get(tab).title(&self.ctx))
                        }
                        None => format!("Window {n}"),
                    }
                }
            };
            names.push((win, name));
        }
        self.window_names = names;
    }

    fn window_name(&self, win: WindowId) -> &str {
        self.window_names
            .iter()
            .find(|(w, _)| *w == win)
            .map_or("", |(_, name)| name.as_str())
    }

    /// Menu bar, toolbar and dock area of one native window.
    fn window_ui(&mut self, ui: &mut Ui, win: WindowId) {
        if ui.input_mut(|i| i.consume_shortcut(&QUIT)) {
            self.ctx.request(Action::Quit);
        }
        self.update_active_doc(win);
        self.ctx.window = win;

        egui::Panel::top(Id::new(("menu-bar", win))).show(ui, |ui| self.menu_bar(ui, win));
        egui::Panel::top(Id::new(("toolbar", win))).show(ui, |ui| self.toolbar(ui, win));
        if win == WindowId::Main {
            egui::Panel::bottom("status-bar").show(ui, |ui| self.status_bar(ui));
        }

        let style = Style::from_egui(ui.style().as_ref());
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                let Some(dock) = self.layout.dock_mut(win) else {
                    return;
                };
                DockArea::new(dock)
                    .id(Id::new(("dock-area", win)))
                    .style(style)
                    .show_leaf_collapse_buttons(false)
                    .show_inside(
                        ui,
                        &mut Viewer {
                            views: &mut self.views,
                            ctx: &mut self.ctx,
                            here: win,
                            windows: &self.window_names,
                        },
                    );
            });

        if self.about == Some(win) {
            self.about(ui.ctx());
        }
        if win == WindowId::Main {
            self.guard_ui(ui.ctx());
            self.open_choice_ui(ui.ctx());
        }
        dialogs::show(ui.ctx(), &mut self.ctx);
        // CDePT's own path prompts (and the native dialogs behind them).
        crate::pathfield::show(ui.ctx(), win);
        crate::newproject::show(ui.ctx(), &mut self.ctx);
        // Last: over the dialogs, and after every widget has said where it is.
        if let Some(guide) = &mut self.guide {
            if win == WindowId::Main {
                let project = self.ctx.project.as_ref();
                let project = project.map(|p| p.root.display().to_string());
                crate::guide::show_card(ui.ctx(), guide, project.as_deref());
            }
            crate::guide::highlight(ui.ctx(), guide);
        }
    }

    /// Follow a tutorial from its first step.
    pub(crate) fn start_tutorial(&mut self, tutorial: crate::guide::Tutorial) {
        tracing::info!(target: "cdept::guide", "tutorial '{}' started", tutorial.id);
        let baseline = self.ctx.project.as_ref().map(|p| p.root.clone());
        self.guide = Some(crate::guide::Guide::new(tutorial, baseline));
    }

    /// The facts of the current step that hold, and the checklists of its circuits.
    fn guide_facts(&mut self, ctx: &Context, guide: &mut crate::guide::Guide) -> Vec<String> {
        let Some(step) = guide.step() else {
            return Vec::new();
        };
        let asked: Vec<String> = step.facts().cloned().collect();
        let mut starts = asked.clone();
        if step.start_button {
            starts.push("project.started".into());
        }

        // The circuits: ask the schematics that are open.
        let circuits = guide.asked_circuits();
        let mut checks = Vec::new();
        if !circuits.is_empty() {
            let ids: Vec<DocId> = self.views.documents().map(|(id, _)| id).collect();
            let mut probes = Vec::new();
            for id in ids {
                if let Some(view) = self
                    .views
                    .document_mut(id)
                    .and_then(|d| d.as_schematic_mut())
                    && let Some(probe) = view.probe()
                {
                    probes.push(probe.clone());
                }
            }
            for id in circuits {
                let Some(spec) = guide.tutorial.circuit(&id) else {
                    continue;
                };
                let file = self
                    .ctx
                    .project
                    .as_ref()
                    .and_then(|p| crate::guide::view_file(p, &spec.view))
                    .and_then(|f| f.canonicalize().ok());
                let lines = probes
                    .iter()
                    .find(|p| file.is_some() && p.file.canonicalize().ok() == file)
                    .map(|p| crate::guide::check(spec, &p.instances));
                checks.push((id, lines));
            }
        }

        let front = match self.active_docs.get(&WindowId::Main) {
            Some(Tab::Document(id)) => Some(*id),
            _ => None,
        };
        let documents = self
            .views
            .documents()
            .map(|(id, view)| {
                let name = self.layout.documents.get(id);
                crate::guide::Open {
                    name: match name {
                        Some(doc) if doc.file.is_none() => {
                            format!("{}/{}/{}", doc.library, doc.cell, doc.view)
                        }
                        _ => String::new(),
                    },
                    front: front == Some(id),
                    modified: view.is_modified(),
                }
            })
            .collect();
        let started = guide.started().map(std::path::Path::to_path_buf);
        let baseline = guide.baseline.clone();
        let world = crate::guide::World {
            project: self.ctx.project.as_ref(),
            selected: (
                self.ctx.selection.library.as_deref(),
                self.ctx.selection.cell.as_deref(),
            ),
            documents,
            baseline: baseline.as_deref(),
            started: started.as_deref(),
        };
        let mut holding: Vec<String> = Vec::new();
        for fact in starts {
            let holds = if let Some(id) = fact.strip_prefix("circuit:") {
                checks
                    .iter()
                    .any(|(known, lines): &(String, Option<Vec<_>>)| {
                        known == id && lines.as_deref().is_some_and(crate::guide::passes)
                    })
            } else {
                guide
                    .cache
                    .derived(&world, &fact)
                    .unwrap_or_else(|| crate::guide::has_fact(ctx, &fact))
            };
            if holds {
                holding.push(fact);
            }
        }
        guide.checks = checks;
        holding
    }

    /// The starting project the card asked for: scan the tutorial's PDK, make the
    /// project, open it.
    fn guide_start(&mut self, ctx: &Context, guide: &mut crate::guide::Guide) {
        use crate::guide::Start;
        match &mut guide.start {
            Start::Asked => {
                let wanted = guide.tutorial.pdk.clone().unwrap_or_default();
                let search = crate::pdkscan::search(&self.ctx);
                let found = crate::pdkscan::find_pdks(&search);
                guide.start = match found.iter().find(|e| e.pdk().name == wanted) {
                    Some(entry) => Start::Scanning(Box::new(crate::pdkscan::ScanJob::start(
                        ctx,
                        entry.pdk().clone(),
                        true,
                        Vec::new(),
                    ))),
                    None => Start::Failed(format!(
                        "The PDK {wanted} was not found. Set PDK_ROOT before starting CDePT, or \
                         add its folder in File > New Project… (\"Other folder\"): see the \
                         tutorial 'Set up a project for a PDK'."
                    )),
                };
            }
            Start::Scanning(job) => {
                if let Some((tech, catalog)) = job.poll() {
                    let pdk = job.pdk.clone();
                    let made = crate::guide::base_dir()
                        .ok_or_else(|| "HOME is not set".to_string())
                        .and_then(|base| {
                            crate::guide::make_start(&guide.tutorial, &pdk, &tech, &catalog, &base)
                        });
                    guide.start = match made {
                        Ok(dir) => {
                            tracing::info!(
                                target: "cdept::guide",
                                "made the starting project {}",
                                dir.display()
                            );
                            let dir = dir.canonicalize().unwrap_or(dir);
                            self.ctx.window = WindowId::Main;
                            self.ctx.request(Action::LoadProject(dir.clone()));
                            Start::Made(dir)
                        }
                        Err(error) => Start::Failed(error),
                    };
                    ctx.request_repaint();
                }
            }
            Start::None | Start::Made(_) | Start::Failed(_) => {}
        }
    }

    /// At the start of a frame: move the tutorial on as the frame before showed, and
    /// collect anew.
    fn guide_frame(&mut self, ctx: &Context) {
        if self.guide.as_ref().is_some_and(|g| g.closed) {
            self.guide = None;
        }
        // Not while a project is being read: it is gone for a moment.
        if !self.ctx.background.busy()
            && let Some(mut guide) = self.guide.take()
        {
            self.guide_start(ctx, &mut guide);
            let facts = self.guide_facts(ctx, &mut guide);
            if guide.update(facts) {
                let step = guide.step().map_or("finished", |s| s.id.as_str());
                tracing::info!(target: "cdept::guide", "tutorial step: {step}");
                ctx.request_repaint();
            }
            self.guide = Some(guide);
        }
        if self.guide.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
        let wanted = self
            .guide
            .as_ref()
            .and_then(|g| g.step())
            .map(|s| {
                let points = s.point.iter().map(|p| p.at.clone());
                s.facts().cloned().chain(points).collect()
            })
            .unwrap_or_default();
        crate::guide::set_wanted(ctx, wanted);
        crate::guide::next_frame(ctx, self.guide.is_some());
    }

    pub(crate) fn script_guide(&self) -> Option<&crate::guide::Guide> {
        self.guide.as_ref()
    }

    /// A detached window, as an immediate viewport (a real OS window where supported).
    fn show_detached(&mut self, ctx: &Context, id: u64) {
        let win = WindowId::Detached(id);
        let Some(window) = self.layout.detached_mut(id) else {
            return;
        };
        let geometry = *self.spawn_geometry.entry(id).or_insert(window.geometry);
        let mut builder = ViewportBuilder::default()
            .with_app_id(APP_ID)
            .with_title(format!("CDePT: {}", self.window_name(win)))
            .with_min_inner_size([400.0, 300.0]);
        builder = match geometry {
            Some(g) => builder.with_position(g.pos).with_inner_size(g.size),
            None => builder.with_inner_size(DETACHED_SIZE),
        };

        let started = std::time::Instant::now();
        self.frame_clock.inside = 0.0;
        ctx.show_viewport_immediate(win.viewport(), builder, |ui, _class| {
            let inside = std::time::Instant::now();
            self.window_ui(ui, win);
            self.frame_clock.inside += inside.elapsed().as_secs_f64() * 1e3;
            let (close, geometry) = ui.input(|i| {
                let vp = i.viewport();
                let geometry = vp
                    .outer_rect
                    .zip(vp.inner_rect)
                    .map(|(outer, inner)| Geometry {
                        pos: outer.min.into(),
                        size: inner.size().into(),
                    });
                (vp.close_requested(), geometry)
            });
            if let Some(g) = geometry
                && let Some(window) = self.layout.detached_mut(id)
            {
                window.geometry = Some(g);
            }
            if close {
                self.ctx.request(Action::CloseWindow(id));
            }
        });
        let whole = started.elapsed().as_secs_f64() * 1e3;
        let inside = self.frame_clock.inside;
        self.frame_clock.detached.push((id, inside, whole));
    }

    fn menu_bar(&mut self, ui: &mut Ui, win: WindowId) {
        egui::MenuBar::new().ui(ui, |ui| {
            let file = ui.menu_button("File", |ui| {
                let new_project = ui.button("New Project…");
                guide::anchor(ui, "menu.file.new_project", &new_project);
                if new_project.clicked() {
                    crate::newproject::open(&mut self.ctx, ui.ctx());
                }
                if ui.button("Open Project…").clicked() {
                    self.ctx.request(Action::PickFolder(PickFor::OpenProject));
                }
                crate::import::file_menu(ui);
                if ui
                    .button("Open Waveform File…")
                    .on_hover_text("A raw file (ngspice, Xyce, VACASK), a Spectre PSF file or a .uwraw in a waveform tab of CDePT's own viewer")
                    .clicked()
                {
                    let directory = self.ctx.project.as_ref().map(|p| p.root.join("simulations"));
                    self.wave_pick = Some(crate::pathfield::ask(
                        ui.ctx(),
                        win,
                        crate::pathfield::Request {
                            kind: crate::pathfield::Kind::File,
                            title: "Open Waveform File".into(),
                            directory: directory.filter(|d| d.is_dir()),
                            filters: vec![(
                                "Waveforms".into(),
                                ["raw", "uwraw", "tran", "ac", "dc", "noise", "xf", "sp", "pss"]
                                    .iter()
                                    .map(|e| e.to_string())
                                    .collect(),
                            )],
                            must_exist: true,
                            ..Default::default()
                        },
                    ));
                }
                if ui
                    .button("Open PSF Results Folder…")
                    .on_hover_text("A Spectre PSF result folder (with its logFile, or the files alone) in a waveform tab; its analyses are offered in the tab's toolbar")
                    .clicked()
                {
                    let directory = self.ctx.project.as_ref().map(|p| p.root.join("simulations"));
                    self.wave_pick = Some(crate::pathfield::ask(
                        ui.ctx(),
                        win,
                        crate::pathfield::Request {
                            kind: crate::pathfield::Kind::Folder,
                            title: "Open PSF Results Folder".into(),
                            directory: directory.filter(|d| d.is_dir()),
                            must_exist: true,
                            ..Default::default()
                        },
                    ));
                }
                if ui
                    .button("Open in UberWave…")
                    .on_hover_text("Any file `cdept wave` takes, in UberWave's own viewer (a separate window): a raw file (ngspice, Xyce, VACASK), a Spectre PSF file, a CSV table (as traces; built-in UberWave only); a digital file opens in Surfer or GTKWave")
                    .clicked()
                {
                    let directory = self.ctx.project.as_ref().map(|p| p.root.join("simulations"));
                    self.uberwave_pick = Some(crate::pathfield::ask(
                        ui.ctx(),
                        win,
                        crate::pathfield::Request {
                            kind: crate::pathfield::Kind::File,
                            title: "Open in UberWave".into(),
                            directory: directory.filter(|d| d.is_dir()),
                            filters: vec![
                                (
                                    "Waveforms".into(),
                                    ["raw", "csv", "tsv", "txt", "dat", "vcd", "fst", "ghw", "tran", "ac", "dc", "noise", "pss"]
                                        .iter()
                                        .map(|e| e.to_string())
                                        .collect(),
                                ),
                                ("All files".into(), vec!["*".into()]),
                            ],
                            must_exist: true,
                            ..Default::default()
                        },
                    ));
                }
                ui.menu_button("Recent Projects", |ui| {
                    if self.ctx.settings.recent_projects.is_empty() {
                        ui.weak("None");
                    }
                    let mut open = None;
                    for path in &self.ctx.settings.recent_projects {
                        let background = &mut self.ctx.background;
                        if crate::background::recent_button(ui, background, path, false) {
                            open = Some(path.clone());
                        }
                    }
                    if let Some(path) = open {
                        self.ctx.request(Action::LoadProject(path));
                    }
                });
                if ui
                    .add_enabled(self.ctx.project.is_some(), Button::new("Close Project"))
                    .clicked()
                {
                    self.ctx.request(Action::CloseProject);
                }
                ui.separator();
                // Entries of the active document (export, ...).
                if let Some(tab) = self.active_doc(win) {
                    self.views.get(tab).file_menu(ui, &mut self.ctx);
                }
                if let WindowId::Detached(id) = win
                    && ui.button("Close Window").clicked()
                {
                    self.ctx.request(Action::CloseWindow(id));
                }
                if ui
                    .add(Button::new("Quit").shortcut_text(ui.ctx().format_shortcut(&QUIT)))
                    .clicked()
                {
                    self.ctx.request(Action::Quit);
                }
            });
            guide::anchor(ui, "menu.file", &file.response);

            self.edit_menu(ui, win);

            ui.menu_button("View", |ui| {
                ui.menu_button("Theme", |ui| {
                    egui::widgets::global_theme_preference_buttons(ui)
                });
                ui.menu_button("Zoom", egui::gui_zoom::zoom_menu_buttons);
            });

            // Contextual menus of this window's active document.
            if let Some(tab) = self.active_doc(win) {
                self.views.get(tab).menus(ui, &mut self.ctx);
            }

            let project_menu = ui.menu_button("Project", |ui| {
                let open = self.ctx.project.is_some();
                if ui
                    .add_enabled(open, Button::new("Reload"))
                    .on_hover_text("Read cdept.toml and the library folders again")
                    .clicked()
                {
                    self.ctx.request(Action::ReloadProject);
                }
                if ui
                    .add_enabled(open, Button::new("Copy Project File Path"))
                    .on_hover_text(
                        "Libraries are defined in cdept.toml, which can be edited by hand",
                    )
                    .clicked()
                    && let Some(project) = &self.ctx.project
                {
                    ui.ctx().copy_text(project.file.display().to_string());
                }
                ui.separator();
                if ui
                    .add_enabled(open, Button::new("Project Settings…"))
                    .on_hover_text("Name, description, technology (PDK) and PDK folder")
                    .clicked()
                    && let Some(project) = &self.ctx.project
                {
                    let dialog = Dialog::project_settings(project);
                    self.ctx.open_dialog(dialog);
                }
                let technology = ui
                    .add_enabled(open, Button::new("Technology…"))
                    .on_hover_text(
                        "What CDePT read out of the PDK: layers, decks, corners, libraries",
                    );
                guide::anchor(ui, "menu.project.technology", &technology);
                if technology.clicked() {
                    self.ctx.request(Action::OpenTab(Tab::Technology));
                }
                crate::background::menu_ui(ui, &mut self.ctx);
            });
            guide::anchor(ui, "menu.project", &project_menu.response);

            let tools_menu = ui.menu_button("Tools", |ui| {
                let manager = ui.button("Library Manager");
                guide::anchor(ui, "menu.tools.library_manager", &manager);
                if manager.clicked() {
                    self.ctx.request(Action::OpenTab(Tab::LibraryManager));
                }
                crate::views::newtest::menu(ui, &mut self.ctx, "menu.tools");
                crate::import::tools_menu(ui, &mut self.ctx);
                ui.separator();
                for name in LAUNCHERS {
                    let choice = launcher_choice(name).and_then(libcdept::launch::choice);
                    let button = ui
                        .add_enabled(choice.is_some(), Button::new(format!("Launch {name}")))
                        .on_disabled_hover_text("Runs inside CDePT (testbenches)");
                    if button.clicked()
                        && let Some(choice) = choice
                    {
                        self.launch(choice);
                        ui.close();
                    }
                }
                ui.separator();
                if ui.button("Detect Tools").clicked() {
                    self.ctx.request(Action::DetectTools);
                }
            });
            guide::anchor(ui, "menu.tools", &tools_menu.response);

            ui.menu_button("Window", |ui| self.window_menu(ui, win));

            ui.menu_button("Help", |ui| {
                ui.menu_button("Tutorials", |ui| {
                    let mut series = String::new();
                    for tutorial in crate::guide::builtins() {
                        if series != tutorial.series {
                            series = tutorial.series.clone();
                            let title = crate::guide::SERIES.iter().find(|(s, _)| *s == series);
                            ui.strong(title.map_or("", |(_, title)| title));
                        }
                        if ui
                            .button(&tutorial.title)
                            .on_hover_text(&tutorial.goal)
                            .clicked()
                        {
                            self.start_tutorial(tutorial);
                            ui.close();
                        }
                    }
                });
                if ui.button("About CDePT").clicked() {
                    self.ctx.request(Action::ShowAbout(win));
                }
            });
        });
    }

    fn window_menu(&mut self, ui: &mut Ui, win: WindowId) {
        for tab in Tab::ALL {
            let mut open = self.layout.find(tab).is_some();
            let title = self.views.get(tab).title(&self.ctx);
            if ui.checkbox(&mut open, title).changed() {
                if open {
                    self.ctx.request(Action::OpenTab(tab));
                } else {
                    self.layout.close_tab(tab);
                }
            }
        }
        let documents: Vec<DocId> = self.layout.documents.iter().map(|(id, _)| id).collect();
        if !documents.is_empty() {
            ui.separator();
        }
        for id in documents {
            let tab = Tab::Document(id);
            let title = self.views.get(tab).title(&self.ctx);
            if ui.button(title).clicked() {
                self.ctx.request(Action::OpenTab(tab));
            }
        }
        ui.separator();
        let active = self.active_doc(win);
        if ui
            .add_enabled(active.is_some(), Button::new("Open Document in New Window"))
            .on_hover_text("Right-click any tab to move it to another window.")
            .clicked()
            && let Some(tab) = active
        {
            self.ctx.request(Action::DetachTab(tab));
        }
        ui.separator();
        for (other, name) in &self.window_names {
            if ui
                .add_enabled(*other != win, Button::new(name).selected(*other == win))
                .clicked()
            {
                self.ctx.request(Action::FocusWindow(*other));
            }
        }
        ui.separator();
        if let WindowId::Detached(id) = win
            && ui
                .button("Close Window")
                .on_hover_text("Its tabs return to the main window.")
                .clicked()
        {
            self.ctx.request(Action::CloseWindow(id));
        }
        if ui
            .button("Reset Layout")
            .on_hover_text("Close all detached windows and restore the default layout.")
            .clicked()
        {
            self.ctx.request(Action::ResetLayout);
        }
    }

    /// Edit actions are routed to the active document; unsupported ones are greyed out.
    fn edit_menu(&mut self, ui: &mut Ui, win: WindowId) {
        let active = self.active_doc(win);
        let caps = active
            .map(|t| self.views.get(t).edit_caps())
            .unwrap_or_default();
        let mut chosen = None;
        ui.menu_button("Edit", |ui| {
            let mut item = |ui: &mut Ui, enabled: bool, label: &str, shortcut: &str, action| {
                if ui
                    .add_enabled(enabled, Button::new(label).shortcut_text(shortcut))
                    .clicked()
                {
                    chosen = Some(action);
                }
            };
            item(ui, caps.undo, "Undo", "Ctrl+Z", EditAction::Undo);
            item(ui, caps.redo, "Redo", "Ctrl+Y", EditAction::Redo);
            ui.separator();
            item(ui, caps.cut, "Cut", "Ctrl+X", EditAction::Cut);
            item(ui, caps.copy, "Copy", "Ctrl+C", EditAction::Copy);
            item(ui, caps.paste, "Paste", "Ctrl+V", EditAction::Paste);
            ui.separator();
            item(
                ui,
                caps.select_all,
                "Select All",
                "Ctrl+A",
                EditAction::SelectAll,
            );
        });
        if let (Some(action), Some(tab)) = (chosen, active) {
            self.views.get(tab).edit(action, &mut self.ctx);
        }
    }

    fn toolbar(&mut self, ui: &mut Ui, win: WindowId) {
        ui.horizontal_wrapped(|ui| {
            match &self.ctx.project {
                Some(project) => ui
                    .label(format!("📁 {}", project.name))
                    .on_hover_text(project.root.display().to_string()),
                None => ui.weak("📁 No project"),
            };
            ui.separator();
            pdk_label(ui, &self.ctx);
            ui.separator();
            for name in LAUNCHERS {
                let choice = launcher_choice(name).and_then(libcdept::launch::choice);
                let found = matches!(self.ctx.tools.find(name), Some(t) if t.found());
                let hover = match (choice, self.ctx.tools.find(name)) {
                    (None, _) => "Runs inside CDePT (testbenches)".to_string(),
                    (Some(_), Some(t)) if t.found() => {
                        format!("Start a {name} window with the project's set-up")
                    }
                    (Some(_), Some(_)) => format!("{name} not found on PATH"),
                    (Some(_), None) => "Detecting…".to_string(),
                };
                let button = ui
                    .add_enabled(choice.is_some() && found, Button::new(format!("▶ {name}")))
                    .on_hover_text(&hover)
                    .on_disabled_hover_text(hover);
                if button.clicked()
                    && let Some(choice) = choice
                {
                    self.launch(choice);
                }
            }
            if let Some(tab) = self.active_doc(win) {
                ui.separator();
                self.views.get(tab).toolbar(ui, &mut self.ctx);
            }
        });
    }

    fn status_bar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            match &self.ctx.project {
                Some(p) => {
                    ui.label(p.root.display().to_string());
                    let problems =
                        p.warnings.len() + p.libraries.iter().filter(|l| l.error.is_some()).count();
                    if problems > 0 {
                        ui.colored_label(ui.visuals().warn_fg_color, format!("⚠ {problems}"))
                            .on_hover_text("Problems in the library definitions; see the Console");
                    }
                }
                None => {
                    ui.weak("No project");
                }
            };
            ui.separator();
            pdk_label(ui, &self.ctx);
            if let Some(root) = self.ctx.pdk_root() {
                ui.weak(format!("in {root}"));
            }
            crate::background::status_ui(ui, &self.ctx);
            ui.separator();
            ui.label("Jobs: 0");
            ui.separator();
            match self.ctx.tools.tools() {
                Some(tools) => {
                    let found = tools.iter().filter(|t| t.found()).count();
                    ui.label(format!("Tools: {found}/{}", tools.len()))
                }
                None => ui.weak("Tools: …"),
            };
            if !self.layout.windows.is_empty() {
                ui.separator();
                ui.label(format!("Windows: {}", self.layout.windows.len() + 1));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.weak(format!("CDePT {}", libcdept::version::long_version()));
            });
        });
    }

    fn about(&mut self, ctx: &Context) {
        let response = egui::Modal::new(Id::new("about")).show(ctx, |ui| {
            ui.set_width(360.0);
            ui.heading("CDePT");
            ui.label("Chip Design Project Tool");
            ui.add_space(4.0);
            ui.label(format!("Version {}", libcdept::version::long_version()));
            ui.label("An open-source orchestrator for xschem, KLayout, Magic and ngspice.");
            ui.weak("MIT License");
            ui.add_space(8.0);
            if ui.button("Close").clicked() {
                ui.close();
            }
        });
        if response.should_close() {
            self.about = None;
        }
    }

    fn focus(ctx: &Context, win: WindowId) {
        ctx.send_viewport_cmd_to(win.viewport(), ViewportCommand::Focus);
    }

    fn apply(&mut self, ctx: &Context, action: Action) {
        match action {
            Action::OpenTab(tab) => {
                let win = self.layout.open_tab(tab);
                Self::focus(ctx, win);
            }
            Action::CloseTab(tab) => self.layout.close_tab(tab),
            Action::OpenView(selection) => self.open_view(ctx, selection),
            Action::OpenWith(selection, tool) => self.open_with(ctx, selection, &tool),
            Action::OpenFile(file) => self.open_file(ctx, &file),
            Action::OpenWave(file, title) => {
                let tab = self.views.add_wave(ctx, file, title);
                let win = self.layout.open_tab(tab);
                Self::focus(ctx, win);
            }
            Action::OpenWaveFiles(files, title) => {
                let tab = self.views.add_wave_files(ctx, title, files, None, None);
                let win = self.layout.open_tab(tab);
                Self::focus(ctx, win);
            }
            Action::OpenWaveFamily(library, cell, view, points) => {
                let only = (!points.is_empty()).then_some(points.as_slice());
                self.open_wave_family(ctx, (library, cell, view), only);
            }
            Action::WaveSend(send) => self.wave_send(ctx, *send),
            Action::DetachTab(tab) => {
                if let Some(id) = self.layout.detach(tab) {
                    Self::focus(ctx, WindowId::Detached(id));
                }
            }
            Action::MoveTab(tab, to) => {
                self.layout.move_tab(tab, to);
                Self::focus(ctx, to);
            }
            Action::FocusWindow(win) => Self::focus(ctx, win),
            Action::CloseWindow(id) => {
                self.layout.close_window(id);
                self.forget_window(id);
            }
            Action::ResetLayout => {
                self.layout.reset();
                self.active_docs.clear();
                self.spawn_geometry.clear();
                self.about = self.about.map(|_| WindowId::Main);
                if let Some((owner, _)) = &mut self.ctx.dialog {
                    *owner = WindowId::Main;
                }
            }
            Action::PickFolder(purpose) => self.ctx.pick_folder(ctx, purpose),
            Action::LoadProject(path) => self.leave(ctx, Leaving::LoadProject(path)),
            Action::ReloadProject => crate::background::reload(&mut self.ctx, ctx),
            Action::CloseProject => self.leave(ctx, Leaving::CloseProject),
            Action::NetlistToView {
                library,
                cell,
                format,
            } => self.netlist_to_view(ctx, &library, &cell, format),
            Action::DetectTools => self.ctx.tools.start_detection(ctx),
            Action::OpenSimulation { selection, run } => {
                let target = self
                    .ctx
                    .project
                    .as_ref()
                    .and_then(|p| crate::views::simulation::testbench_of(p, &selection));
                match (target, self.ctx.project.as_ref()) {
                    (Some(target), Some(project)) => {
                        self.views.simulation_mut().open(project, target, run);
                        let win = self.layout.open_tab(Tab::Simulation);
                        Self::focus(ctx, win);
                    }
                    _ => tracing::info!(
                        target: "cdept::sim",
                        "{}: no testbench view to simulate",
                        selection.cell.as_deref().unwrap_or("selection")
                    ),
                }
            }
            Action::UsedBy(selection) => {
                if let (Some(library), Some(cell)) = (selection.library, selection.cell) {
                    self.ctx.refactor.show = Some(crate::refactor::Show {
                        library,
                        cell,
                        used_by: true,
                    });
                    let win = self.layout.open_tab(Tab::Hierarchy);
                    Self::focus(ctx, win);
                }
            }
            Action::RestartDocuments => self.restart_documents(ctx),
            Action::ShowAbout(win) => self.about = Some(win),
            Action::Quit => self.leave(ctx, Leaving::Quit),
        }
    }

    fn forget_window(&mut self, id: u64) {
        let win = WindowId::Detached(id);
        self.active_docs.remove(&win);
        self.spawn_geometry.remove(&id);
        if self.about == Some(win) {
            self.about = Some(WindowId::Main);
        }
        if let Some((owner, _)) = &mut self.ctx.dialog
            && *owner == win
        {
            *owner = WindowId::Main;
        }
    }

    /// Act on the folder chosen in the native folder dialog.
    fn picked(&mut self, ctx: &Context, purpose: PickFor, path: std::path::PathBuf) {
        // Dialogs opened from here belong to the main window.
        self.ctx.window = WindowId::Main;
        match purpose {
            PickFor::NewProject => self.leave(ctx, Leaving::NewProject(path)),
            PickFor::OpenProject => self.leave(ctx, Leaving::LoadProject(path)),
            PickFor::AddLibrary => {
                if let Some(project) = &self.ctx.project {
                    let dialog = Dialog::add_library(project, &path);
                    self.ctx.open_dialog(dialog);
                }
            }
        }
    }
}

impl eframe::App for CdeptApp {
    fn ui(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        let ctx = &ui.ctx().clone();
        let frame_start = std::time::Instant::now();
        self.guide_frame(ctx);
        let interval = self
            .frame_clock
            .last
            .map(|t| (frame_start - t).as_secs_f64() * 1e3);
        self.frame_clock.detached.clear();
        self.refresh_window_names();
        self.publish_schematic();
        // Go to Cell, the categories, Import Project (before the views take the keys).
        crate::import::frame(ctx, &mut self.ctx);

        // The close button of the main window: not with unsaved changes unasked.
        if ctx.input(|i| i.viewport().close_requested()) && !self.quit_confirmed {
            if self.modified_documents().is_empty() {
                self.quit_confirmed = true;
            } else {
                ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, ViewportCommand::CancelClose);
                if self.guard.is_none() {
                    self.leave(ctx, Leaving::Quit);
                }
            }
        }

        let main_start = std::time::Instant::now();
        self.window_ui(ui, WindowId::Main);
        let main_ms = main_start.elapsed().as_secs_f64() * 1e3;
        let detached: Vec<u64> = self.layout.windows.iter().map(|w| w.id).collect();
        for id in detached {
            self.show_detached(ctx, id);
        }

        self.ctx.window = WindowId::Main;
        for action in self.ctx.take_actions() {
            self.apply(ctx, action);
        }
        if let Some((purpose, path)) = self.ctx.take_picked() {
            self.picked(ctx, purpose, path);
        }
        if let Some(rx) = &self.wave_pick {
            match rx.try_recv() {
                Ok(Some(path)) => {
                    self.wave_pick = None;
                    self.apply(ctx, Action::OpenWave(path, None));
                }
                Ok(None) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.wave_pick = None
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        self.poll_uberwave_opens(ctx);
        if let Some(send) = self.views.simulation_mut().calc.take_tab_send() {
            self.apply(ctx, Action::WaveSend(Box::new(send)));
        }
        self.finish_netlists();
        // What changed on disk, and what the worker threads finished.
        if crate::background::tick(&mut self.ctx, &mut self.views, ctx).restore {
            self.restore_documents(ctx);
        }
        self.views.coordinate_layouts();
        self.views.tick_waves(ctx);
        self.views.coordinate_links(&mut self.ctx);
        // A detached window whose last tab was closed or dragged away disappears.
        let before: Vec<u64> = self.layout.windows.iter().map(|w| w.id).collect();
        self.sync_documents();
        if let Some(mut script) = self.script.take() {
            // The script acts on the active document as the actions above left it.
            for win in self.layout.window_ids() {
                self.update_active_doc(win);
            }
            // Not while the project is being read or scanned: the script looks at it.
            if !self.ctx.background.busy() {
                script.advance(self, ctx);
            }
            self.script = Some(script);
        }
        self.layout.prune();
        for id in before {
            if self.layout.detached_mut(id).is_none() {
                self.forget_window(id);
            }
        }
        let now = std::time::Instant::now();
        let ui_ms = (now - frame_start).as_secs_f64() * 1e3;
        // The interval before this frame holds the painting of the one before.
        let slow = FrameClock::slow_ms();
        if ui_ms > slow || interval.is_some_and(|i| i > slow) {
            let windows: Vec<String> = self
                .frame_clock
                .detached
                .iter()
                .map(|(id, inside, whole)| {
                    format!("window {id}: {inside:.1} ms drawing, {whole:.1} ms with its viewport")
                })
                .collect();
            tracing::debug!(
                target: "cdept::frame",
                "slow frame: {:.1} ms since the frame before (that frame: {:.1} ms of CPU without the buffer swap), {ui_ms:.1} ms in the app (main window {main_ms:.1} ms{}{})",
                interval.unwrap_or(0.0),
                frame.info().cpu_usage.unwrap_or(0.0) * 1e3,
                if windows.is_empty() { "" } else { "; " },
                windows.join("; ")
            );
        }
        self.frame_clock.last = Some(now);
    }

    fn raw_input_hook(&mut self, _ctx: &Context, raw_input: &mut egui::RawInput) {
        if let Some(mut script) = self.script.take() {
            script.hook(self, raw_input);
            self.script = Some(script);
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, LAYOUT_KEY, &self.layout);
        eframe::set_value(storage, SETTINGS_KEY, &self.ctx.settings);
    }
}

/// Point names in a human order: `mc_r2` before `mc_r10`, `vdd1p08` before `vdd1p2`
/// (`1p08` is 1.08, as a sweep value is written in a point name), `1.2` after `1.08`.
pub(crate) fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    // a number: digits, then maybe 'p' or '.' and more digits (a decimal point)
    fn number(s: &[char], i: &mut usize) -> f64 {
        let mut text = String::new();
        while *i < s.len() && s[*i].is_ascii_digit() {
            text.push(s[*i]);
            *i += 1;
        }
        if *i + 1 < s.len() && (s[*i] == 'p' || s[*i] == '.') && s[*i + 1].is_ascii_digit() {
            text.push('.');
            *i += 1;
            while *i < s.len() && s[*i].is_ascii_digit() {
                text.push(s[*i]);
                *i += 1;
            }
        }
        text.parse().unwrap_or(f64::INFINITY)
    }
    let (x, y): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let (mut i, mut j) = (0, 0);
    loop {
        match (x.get(i), y.get(j)) {
            (None, None) => return a.cmp(b),
            (None, _) => return std::cmp::Ordering::Less,
            (_, None) => return std::cmp::Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let (n, m) = (number(&x, &mut i), number(&y, &mut j));
                match n.partial_cmp(&m) {
                    Some(std::cmp::Ordering::Equal) | None => {}
                    Some(o) => return o,
                }
            }
            (Some(c), Some(d)) => {
                if c != d {
                    return c.cmp(d);
                }
                i += 1;
                j += 1;
            }
        }
    }
}

/// "PDK: name" with the folder and where both come from as the tooltip.
fn pdk_label(ui: &mut Ui, ctx: &AppCtx) {
    let Some(project) = &ctx.project else {
        ui.weak("PDK: none");
        return;
    };
    let tip = format!(
        "PDK: {}\nPDK_ROOT: {}\nChange them in Project > Project Settings…",
        crate::dialogs::pdk_text(&project.pdk.name),
        crate::dialogs::pdk_text(&project.pdk.root)
    );
    match ctx.pdk() {
        Some(pdk) => ui.label(format!("PDK: {pdk}")).on_hover_text(tip),
        // `$PDK` offers several and the project has not chosen one.
        None if !project.pdk.available.is_empty() => ui
            .weak(format!(
                "PDK: choose one of {}",
                project.pdk.available.len()
            ))
            .on_hover_text(format!(
                "$PDK offers {}.\nChoose one in Project > Project Settings…",
                project.pdk.available.join(", ")
            )),
        None => ui.weak("PDK: none").on_hover_text(tip),
    };
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::*;

    #[test]
    fn point_names_sort_by_their_values() {
        let mut v = vec![
            "vdd1p2",
            "vdd1p08",
            "mc_r10",
            "mc_r2",
            "tt_100",
            "tt_27",
            "x1.2",
            "x1.08",
            "a",
            "vdd1p08_ss",
        ];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(
            v,
            [
                "a",
                "mc_r2",
                "mc_r10",
                "tt_27",
                "tt_100",
                "vdd1p08",
                "vdd1p08_ss",
                "vdd1p2",
                "x1.08",
                "x1.2"
            ]
        );
        assert_eq!(natural_cmp("p01", "p001"), std::cmp::Ordering::Greater);
    }

    /// A project with one cell: a schematic, a symbol and a layout.
    fn project(root: &Path) {
        for (view, file) in [
            ("schematic", "ota.sch"),
            ("symbol", "ota.sym"),
            ("layout", "ota.gds"),
        ] {
            let dir = root.join("libs/analog/ota").join(view);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join(file), "").unwrap();
        }
        fs::write(
            root.join(libcdept::PROJECT_FILE),
            "name = \"demo\"\n[libraries]\nanalog = \"libs/analog\"\n",
        )
        .unwrap();
    }

    fn app(ctx: &Context, root: &Path, layout: Layout) -> CdeptApp {
        let options = GuiOptions {
            project: Some(root.to_path_buf()),
            script: None,
            tutorial: None,
        };
        CdeptApp::start(
            ctx,
            layout,
            Settings::default(),
            options,
            LogBuffer::new(),
            None,
        )
    }

    fn open(app: &mut CdeptApp, ctx: &Context, view: &str) {
        app.open_view(ctx, Selection::view("analog", "ota", view));
    }

    fn titles(app: &mut CdeptApp) -> Vec<String> {
        let ids: Vec<DocId> = app.layout.documents.iter().map(|(id, _)| id).collect();
        ids.into_iter()
            .map(|id| app.views.get(Tab::Document(id)).title(&app.ctx))
            .collect()
    }

    /// The layout as the next start finds it.
    fn saved(app: &CdeptApp) -> Layout {
        let text = ron::to_string(&app.layout).unwrap();
        ron::from_str::<Layout>(&text).unwrap().restored()
    }

    #[test]
    fn views_open_once_in_document_tabs() {
        let tmp = tempfile::tempdir().unwrap();
        project(tmp.path());
        let ctx = Context::default();
        let mut app = app(&ctx, tmp.path(), Layout::default());

        open(&mut app, &ctx, "schematic");
        open(&mut app, &ctx, "schematic");
        assert_eq!(titles(&mut app), ["ota (schematic)"]);
        open(&mut app, &ctx, "symbol");
        // Layouts have a viewer (KLayout; not started here), a missing view has none.
        assert!(
            app.document_file(&DocRef::view("analog", "ota", "layout"))
                .is_some_and(|file| file.ends_with("ota.gds"))
        );
        open(&mut app, &ctx, "nothing");
        assert_eq!(titles(&mut app), ["ota (schematic)", "ota (symbol)"]);
        assert_eq!(app.script_document_count(), 2);

        // A file opens as the view it is; a file outside the libraries as itself.
        let symbol_file = tmp.path().join("libs/analog/ota/symbol/ota.sym");
        app.open_file(&ctx, &symbol_file);
        let loose = tmp.path().join("scratch.sch");
        fs::write(&loose, "").unwrap();
        app.open_file(&ctx, &loose);
        app.open_file(&ctx, &loose);
        app.open_file(&ctx, &tmp.path().join("notes.txt"));
        assert_eq!(
            titles(&mut app),
            ["ota (schematic)", "ota (symbol)", "scratch.sch"]
        );
        let scratch = app.script_document().unwrap();
        app.layout.close_tab(scratch);
        app.sync_documents();

        // The X on the symbol's tab closes that document and ends its xschem.
        let symbol = app.script_document().unwrap();
        let path = app.layout.main.find_tab(&symbol).unwrap();
        app.layout.main.remove_tab(path);
        app.sync_documents();
        assert_eq!(titles(&mut app), ["ota (schematic)"]);
        assert!(matches!(symbol, Tab::Document(id) if app.views.document(id).is_none()));

        // Closing the project closes what is left.
        app.ctx.close_project();
        app.sync_documents();
        assert_eq!(app.script_document_count(), 0);
        assert_eq!(app.layout.documents.project, None);
    }

    #[test]
    fn leaving_without_unsaved_changes_does_not_ask() {
        let tmp = tempfile::tempdir().unwrap();
        project(tmp.path());
        let ctx = Context::default();
        let mut app = app(&ctx, tmp.path(), Layout::default());
        open(&mut app, &ctx, "schematic");
        assert!(app.modified_documents().is_empty());

        // Opening the project that is open keeps it and its documents.
        app.apply(&ctx, Action::LoadProject(tmp.path().join("libs")));
        app.sync_documents();
        assert!(app.guard.is_none());
        assert_eq!(app.script_document_count(), 1);

        app.apply(&ctx, Action::CloseProject);
        app.sync_documents();
        assert!(app.guard.is_none());
        assert!(app.ctx.project.is_none());
        assert_eq!(app.script_document_count(), 0);

        // Nothing to answer when nothing was asked.
        assert!(!app.script_leave_with(&ctx, LeaveChoice::Cancel));
        app.apply(&ctx, Action::Quit);
        assert!(app.quit_confirmed);
    }

    #[test]
    fn the_schematic_of_a_cell_is_found() {
        let _env = libcdept::testutil::hermetic_env();
        let tmp = tempfile::tempdir().unwrap();
        project(tmp.path());
        let dir = tmp.path().join("libs/analog/ota/schematic_old");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("ota.sch"), "").unwrap();
        let project = Project::load(tmp.path()).unwrap();
        let cell = project.library("analog").unwrap().cell("ota").unwrap();
        // The view named `schematic` wins over other schematics.
        assert_eq!(
            schematic_file(cell),
            Some(&tmp.path().join("libs/analog/ota/schematic/ota.sch"))
        );
        let none = Cell::default();
        assert_eq!(schematic_file(&none), None);
    }

    #[test]
    fn documents_come_back_in_their_project_only() {
        let tmp = tempfile::tempdir().unwrap();
        project(tmp.path());
        let ctx = Context::default();
        let mut first = app(&ctx, tmp.path(), Layout::default());
        open(&mut first, &ctx, "schematic");
        open(&mut first, &ctx, "symbol");
        let layout = saved(&first);
        drop(first);

        // The same project: both are back, each with a view behind its tab.
        let mut again = app(&ctx, tmp.path(), layout);
        assert_eq!(titles(&mut again), ["ota (schematic)", "ota (symbol)"]);
        let ids: Vec<DocId> = again.layout.documents.iter().map(|(id, _)| id).collect();
        assert!(ids.iter().all(|id| again.views.document(*id).is_some()));
        assert!(
            ids.iter()
                .all(|id| again.layout.find(Tab::Document(*id)).is_some())
        );

        // A view that is gone from disk does not come back.
        let layout = saved(&again);
        drop(again);
        fs::remove_dir_all(tmp.path().join("libs/analog/ota/symbol")).unwrap();
        let mut fewer = app(&ctx, tmp.path(), layout);
        assert_eq!(titles(&mut fewer), ["ota (schematic)"]);

        // Another project: nothing comes back, and no tab is left over.
        let layout = saved(&fewer);
        drop(fewer);
        let other = tempfile::tempdir().unwrap();
        project(other.path());
        let elsewhere = app(&ctx, other.path(), layout);
        assert_eq!(elsewhere.script_document_count(), 0);
        let stray = elsewhere
            .layout
            .main
            .iter_all_tabs()
            .any(|(_, tab)| matches!(tab, Tab::Document(_)));
        assert!(!stray);
    }
}
