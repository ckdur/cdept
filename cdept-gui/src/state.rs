//! State shared by all views of the main window.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;

use libcdept::tools::{self, ToolInfo};
use libcdept::{Cell, Entry, Library, Project};
use serde::{Deserialize, Serialize};

use crate::dialogs::{Dialog, DialogState};
use crate::dock::{Tab, WindowId};
use crate::log::LogBuffer;

/// User settings persisted across sessions (the dock layout is stored separately).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub recent_projects: Vec<PathBuf>,
    pub console_level: ConsoleLevel,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            recent_projects: Vec::new(),
            console_level: ConsoleLevel::Info,
        }
    }
}

const MAX_RECENT: usize = 10;

impl Settings {
    pub fn add_recent(&mut self, path: PathBuf) {
        self.recent_projects.retain(|p| p != &path);
        self.recent_projects.insert(0, path);
        self.recent_projects.truncate(MAX_RECENT);
    }
}

/// Minimum level shown in the console.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConsoleLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl ConsoleLevel {
    pub const ALL: [ConsoleLevel; 5] = [
        Self::Error,
        Self::Warn,
        Self::Info,
        Self::Debug,
        Self::Trace,
    ];

    pub fn shows(self, level: tracing::Level) -> bool {
        let max = match self {
            Self::Error => tracing::Level::ERROR,
            Self::Warn => tracing::Level::WARN,
            Self::Info => tracing::Level::INFO,
            Self::Debug => tracing::Level::DEBUG,
            Self::Trace => tracing::Level::TRACE,
        };
        level <= max
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Error => "Error",
            Self::Warn => "Warn",
            Self::Info => "Info",
            Self::Debug => "Debug",
            Self::Trace => "Trace",
        }
    }
}

/// Requests from views to the shell, applied after the frame's UI is built.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    OpenTab(Tab),
    /// Move a tab into a new native window.
    DetachTab(Tab),
    /// Move a tab into an existing window.
    MoveTab(Tab, WindowId),
    FocusWindow(WindowId),
    /// Close a detached window; its tabs return to the main window.
    CloseWindow(u64),
    ResetLayout,
    /// Ask for a folder, then do what the purpose says with it.
    PickFolder(PickFor),
    /// Open the project in this folder (or the one it belongs to).
    LoadProject(PathBuf),
    /// Re-read cdept.toml and the library folders.
    ReloadProject,
    CloseProject,
    DetectTools,
    /// Show the About dialog over the given window.
    ShowAbout(WindowId),
    Quit,
}

/// External tool detection, run on a background thread.
#[derive(Default)]
pub struct ToolsState {
    result: Arc<Mutex<Option<Vec<ToolInfo>>>>,
    running: Arc<Mutex<bool>>,
}

impl ToolsState {
    pub fn start_detection(&self, ctx: &egui::Context) {
        {
            let mut running = self.running.lock().unwrap();
            if *running {
                return;
            }
            *running = true;
        }
        let (result, running, ctx) = (self.result.clone(), self.running.clone(), ctx.clone());
        thread::spawn(move || {
            let tools = tools::detect_all();
            let found = tools.iter().filter(|t| t.found()).count();
            tracing::info!(target: "cdept::tools", "found {found} of {} external tools", tools.len());
            for t in tools.iter().filter(|t| !t.found()) {
                tracing::debug!(target: "cdept::tools", "{} not found in PATH", t.spec.name);
            }
            *result.lock().unwrap() = Some(tools);
            *running.lock().unwrap() = false;
            ctx.request_repaint();
        });
    }

    pub fn is_running(&self) -> bool {
        *self.running.lock().unwrap()
    }

    /// Snapshot of the last detection, if any.
    pub fn tools(&self) -> Option<Vec<ToolInfo>> {
        self.result.lock().unwrap().clone()
    }

    pub fn find(&self, name: &str) -> Option<ToolInfo> {
        self.tools()?.into_iter().find(|t| t.spec.name == name)
    }
}

/// What a folder chosen in the native folder dialog is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickFor {
    NewProject,
    OpenProject,
    /// An existing folder to define as a library.
    AddLibrary,
}

struct Picker {
    purpose: PickFor,
    result: Receiver<Option<PathBuf>>,
}

/// The library, cell and view selected in the library views (shared by all of them).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    pub library: Option<String>,
    pub cell: Option<String>,
    pub view: Option<String>,
}

impl Selection {
    pub fn library(library: &str) -> Self {
        Self {
            library: Some(library.into()),
            ..Default::default()
        }
    }

    pub fn cell(library: &str, cell: &str) -> Self {
        Self {
            cell: Some(cell.into()),
            ..Self::library(library)
        }
    }

    pub fn view(library: &str, cell: &str, view: &str) -> Self {
        Self {
            view: Some(view.into()),
            ..Self::cell(library, cell)
        }
    }
}

/// Everything views can read or change, other than the dock layout itself.
pub struct AppCtx {
    pub settings: Settings,
    pub log: LogBuffer,
    pub tools: ToolsState,
    pub project: Option<Project>,
    pub selection: Selection,
    /// The native window whose UI is being built.
    pub window: WindowId,
    /// The open modal dialog and the window it belongs to.
    pub(crate) dialog: Option<(WindowId, DialogState)>,
    picker: Option<Picker>,
    actions: Vec<Action>,
}

impl AppCtx {
    pub fn new(settings: Settings, log: LogBuffer) -> Self {
        Self {
            settings,
            log,
            tools: ToolsState::default(),
            project: None,
            selection: Selection::default(),
            window: WindowId::Main,
            dialog: None,
            picker: None,
            actions: Vec::new(),
        }
    }

    pub fn request(&mut self, action: Action) {
        if !self.actions.contains(&action) {
            self.actions.push(action);
        }
    }

    pub(crate) fn take_actions(&mut self) -> Vec<Action> {
        std::mem::take(&mut self.actions)
    }

    /// The `$PDK` environment variable, as set by open_pdks / IIC-OSIC-TOOLS.
    pub fn pdk(&self) -> Option<String> {
        std::env::var("PDK").ok().filter(|s| !s.is_empty())
    }

    /// Show a modal dialog over the window being built.
    pub fn open_dialog(&mut self, dialog: Dialog) {
        self.dialog = Some((self.window, DialogState::new(dialog)));
    }

    /// Report a failure: in the console and as a dialog.
    pub fn error(&mut self, message: String) {
        tracing::error!(target: "cdept::project", "{message}");
        self.open_dialog(Dialog::Error(message));
    }

    pub fn selected_library(&self) -> Option<&Library> {
        self.project
            .as_ref()?
            .library(self.selection.library.as_deref()?)
    }

    pub fn selected_cell(&self) -> Option<&Cell> {
        self.selected_library()?
            .cell(self.selection.cell.as_deref()?)
    }

    pub fn selected_view(&self) -> Option<&Entry> {
        self.selected_cell()?.entry(self.selection.view.as_deref()?)
    }

    /// Drop the parts of the selection that no longer exist.
    pub fn validate_selection(&mut self) {
        if self.selected_library().is_none() {
            self.selection = Selection::default();
        } else if self.selected_cell().is_none() {
            self.selection.cell = None;
            self.selection.view = None;
        } else if self.selected_view().is_none() {
            self.selection.view = None;
        }
    }

    /// Open the project `path` is in (its folder, or any folder below it).
    pub fn load_project(&mut self, path: &Path) {
        let root = Project::find_root(path).unwrap_or_else(|| path.to_path_buf());
        match Project::load(&root) {
            Ok(project) => self.set_project(project),
            Err(e) => self.error(format!("{e:#}")),
        }
    }

    /// Create a project in `dir`, or open the one that is already there.
    pub fn new_project(&mut self, dir: &Path) {
        if dir.join(libcdept::PROJECT_FILE).is_file() {
            tracing::info!(target: "cdept::project", "{} already holds a project; opening it", dir.display());
            return self.load_project(dir);
        }
        match Project::create(dir, None) {
            Ok(project) => self.set_project(project),
            Err(e) => self.error(format!("{e:#}")),
        }
    }

    fn set_project(&mut self, project: Project) {
        let cells: usize = project.libraries.iter().map(|l| l.cells.len()).sum();
        tracing::info!(
            target: "cdept::project",
            "project '{}' ({}): {} libraries, {cells} cells",
            project.name,
            project.root.display(),
            project.libraries.len()
        );
        log_problems(&project);
        self.settings.add_recent(project.root.clone());
        self.project = Some(project);
        self.validate_selection();
    }

    pub fn reload_project(&mut self) {
        let Some(project) = &mut self.project else {
            return;
        };
        match project.reload() {
            Ok(()) => {
                log_problems(project);
                self.validate_selection();
            }
            Err(e) => self.error(format!("{e:#}")),
        }
    }

    pub fn close_project(&mut self) {
        if let Some(project) = self.project.take() {
            tracing::info!(target: "cdept::project", "closed project '{}'", project.name);
        }
        self.selection = Selection::default();
    }

    /// Open the native folder dialog on a background thread; the choice is picked up
    /// by [`take_picked`](Self::take_picked).
    pub fn pick_folder(&mut self, ctx: &egui::Context, purpose: PickFor) {
        if self.picker.is_some() {
            return;
        }
        let title = match purpose {
            PickFor::NewProject => "Folder for the new project",
            PickFor::OpenProject => "Open project folder",
            PickFor::AddLibrary => "Library folder",
        };
        let start = self
            .project
            .as_ref()
            .map(|p| p.root.clone())
            .or_else(|| std::env::current_dir().ok());
        let (sender, result) = mpsc::channel();
        let ctx = ctx.clone();
        thread::spawn(move || {
            let mut dialog = rfd::FileDialog::new().set_title(title);
            if let Some(dir) = start {
                dialog = dialog.set_directory(dir);
            }
            let _ = sender.send(dialog.pick_folder());
            ctx.request_repaint();
        });
        self.picker = Some(Picker { purpose, result });
    }

    /// The folder chosen in the folder dialog, once the user has answered.
    pub(crate) fn take_picked(&mut self) -> Option<(PickFor, PathBuf)> {
        let picker = self.picker.as_ref()?;
        match picker.result.try_recv() {
            Ok(choice) => {
                let purpose = picker.purpose;
                self.picker = None;
                choice.map(|path| (purpose, path))
            }
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.picker = None;
                None
            }
        }
    }
}

fn log_problems(project: &Project) {
    for warning in &project.warnings {
        tracing::warn!(target: "cdept::project", "{warning}");
    }
    for lib in &project.libraries {
        if let Some(error) = &lib.error {
            tracing::warn!(target: "cdept::project", "library '{}': {error}", lib.name);
        }
    }
}
