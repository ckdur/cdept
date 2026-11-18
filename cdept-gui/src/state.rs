//! State shared by all views of the main window.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;

use libcdept::tools::{self, ToolInfo};
use libcdept::xschem::symbol::{self, Problem, Unlinked};
use libcdept::xschem::{Engine, NetlistFormat};
use libcdept::{Cell, Entry, Library, Project};
use serde::{Deserialize, Serialize};

use crate::dialogs::{Dialog, DialogState};
use crate::dock::{DocId, DocRef, Tab, WindowId};
use crate::engines::Engines;
use crate::log::LogBuffer;
use crate::views::Selected;

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
    CloseTab(Tab),
    /// Open a view of a cell in its embedded tool, as a document tab.
    OpenView(Selection),
    /// Open a file in its embedded tool: as the library view it is, if it is one.
    OpenFile(PathBuf),
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
    /// Write the netlist of a cell's schematic into its `netlist` view.
    NetlistToView {
        library: String,
        cell: String,
        format: NetlistFormat,
    },
    DetectTools,
    /// Show the About dialog over the given window.
    ShowAbout(WindowId),
    Quit,
}

/// The embedded xschem engine: the helper program `cdept-xschem`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HelperInfo {
    /// Where the program is, if it was found.
    pub path: Option<PathBuf>,
    /// What it says about itself (`--version`), e.g. `xschem 3.4.8RC, protocol 2`.
    pub version: Option<String>,
}

impl HelperInfo {
    /// Look for the helper and ask it for its version.
    fn detect() -> HelperInfo {
        let Some(path) = Engine::find_helper() else {
            return HelperInfo::default();
        };
        let output = std::process::Command::new(&path).arg("--version").output();
        let version = output.ok().filter(|o| o.status.success()).map(|o| {
            let text = String::from_utf8_lossy(&o.stdout);
            helper_version(text.trim())
        });
        HelperInfo {
            path: Some(path),
            version,
        }
    }
}

/// `xschem 3.4.8RC, protocol 2` out of `cdept-xschem 0.1.0 (xschem 3.4.8RC, protocol 2)`.
fn helper_version(line: &str) -> String {
    match (line.find('('), line.rfind(')')) {
        (Some(open), Some(close)) if open < close => line[open + 1..close].to_string(),
        _ => line.to_string(),
    }
}

/// External tool detection, run on a background thread.
#[derive(Default)]
pub struct ToolsState {
    result: Arc<Mutex<Option<Vec<ToolInfo>>>>,
    helper: Arc<Mutex<Option<HelperInfo>>>,
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
        let helper = self.helper.clone();
        thread::spawn(move || {
            let found = HelperInfo::detect();
            match &found.path {
                Some(path) => {
                    tracing::debug!(target: "cdept::tools", "xschem engine: {}", path.display())
                }
                None => tracing::warn!(
                    target: "cdept::tools",
                    "the embedded xschem engine (cdept-xschem) was not found: schematics \
                     cannot be opened"
                ),
            }
            *helper.lock().unwrap() = Some(found);
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

    /// The embedded xschem engine, once looked for.
    pub fn helper(&self) -> Option<HelperInfo> {
        self.helper.lock().unwrap().clone()
    }
}

/// The schematic document that is in front, for the panels that follow it (Hierarchy,
/// Inspector).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveSchematic {
    pub id: DocId,
    pub doc: DocRef,
    /// The file the document was opened for.
    pub file: PathBuf,
    pub title: String,
    /// xschem has changes that are not saved.
    pub modified: bool,
    /// What is selected in xschem, if anything.
    pub selection: Option<Selected>,
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
    /// Symbols of the project whose `schematic=` link is broken, checked when the
    /// libraries are scanned (see [`refresh_links`](Self::refresh_links)).
    pub links: Vec<Unlinked>,
    pub selection: Selection,
    /// xschem engines of the project that are not documents (jobs, the preview).
    pub engines: Engines,
    /// The schematic document in front, refreshed every frame.
    pub schematic: Option<ActiveSchematic>,
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
            links: Vec::new(),
            selection: Selection::default(),
            engines: Engines::default(),
            schematic: None,
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

    /// The PDK of the project (`$PDK`): its `technology`, else the environment's.
    pub fn pdk(&self) -> Option<String> {
        Some(self.project.as_ref()?.pdk.name.as_ref()?.value.clone())
    }

    /// Where PDKs are installed (`$PDK_ROOT`), resolved the same way.
    pub fn pdk_root(&self) -> Option<String> {
        Some(self.project.as_ref()?.pdk.root.as_ref()?.value.clone())
    }

    /// Save the project's name, description, technology and PDK folder, resolve the PDK
    /// again and rescan the libraries.
    pub fn change_settings(&mut self, settings: &libcdept::Settings) -> eyre::Result<()> {
        let project = self
            .project
            .as_mut()
            .ok_or_else(|| eyre::eyre!("no project is open"))?;
        let before = project.pdk.clone();
        project.set_settings(settings)?;
        log_problems(project);
        let changed = project.pdk != before;
        tracing::info!(
            target: "cdept::project",
            "project settings saved: PDK {}, PDK_ROOT {}",
            crate::dialogs::pdk_text(&project.pdk.name),
            crate::dialogs::pdk_text(&project.pdk.root)
        );
        if changed {
            tracing::warn!(
                target: "cdept::project",
                "the PDK changed: reopen the xschem tabs that are open, they still run with the old one"
            );
        }
        self.refresh_links();
        self.validate_selection();
        Ok(())
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
        self.refresh_links();
        self.validate_selection();
    }

    pub fn reload_project(&mut self) {
        let Some(project) = &mut self.project else {
            return;
        };
        match project.reload() {
            Ok(()) => {
                log_problems(project);
                self.refresh_links();
                self.validate_selection();
            }
            Err(e) => self.error(format!("{e:#}")),
        }
    }

    /// Check the `schematic=` links of the symbols again. It reads the attribute block
    /// of every symbol file of the `cells` libraries, so it runs when the libraries
    /// are scanned, not every frame.
    pub fn refresh_links(&mut self) {
        self.links = match &self.project {
            Some(project) => symbol::check(project, None, None),
            None => Vec::new(),
        };
        if !self.links.is_empty() {
            tracing::warn!(
                target: "cdept::project",
                "{} symbol(s) not linked to the schematic of their cell",
                self.links.len()
            );
        }
    }

    /// Write the expected `schematic=` into these symbols, and check again. Each fix
    /// and each failure is logged; returns how many symbols were fixed.
    pub fn fix_links(&mut self, items: &[Unlinked]) -> usize {
        let Some(project) = &self.project else {
            return 0;
        };
        let mut fixed = 0;
        for item in items {
            let name = project.portable_path(&item.symbol);
            match symbol::repair(project, item) {
                Ok(()) => {
                    fixed += 1;
                    tracing::info!(
                        target: "cdept::library",
                        "{name}: set schematic={}",
                        item.expected
                    );
                }
                Err(e) => tracing::error!(target: "cdept::library", "{name}: not fixed: {e:#}"),
            }
        }
        self.refresh_links();
        fixed
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

/// The broken link of the symbol file `file`, if it has one.
pub fn link_of<'a>(links: &'a [Unlinked], file: &Path) -> Option<&'a Unlinked> {
    links.iter().find(|u| u.symbol == file)
}

/// The broken link of a symbol view of `cell`.
pub fn view_link<'a>(links: &'a [Unlinked], entry: &Entry) -> Option<&'a Unlinked> {
    entry.files.iter().find_map(|f| link_of(links, f))
}

/// The broken link of any symbol of a cell.
pub fn cell_link<'a>(links: &'a [Unlinked], library: &str, cell: &str) -> Option<&'a Unlinked> {
    links
        .iter()
        .find(|u| u.library == library && u.cell == cell)
}

/// Whether repairing writes where descending goes, rather than filling a gap: the link
/// points to another cell's schematic, so it is only rewritten on request.
pub fn is_foreign(unlinked: &Unlinked) -> bool {
    matches!(unlinked.problem, Problem::Foreign { .. })
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

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::log::LogBuffer;

    #[test]
    fn the_helper_says_which_xschem_it_holds() {
        assert_eq!(
            helper_version("cdept-xschem 0.1.0 (xschem 3.4.8RC, protocol 2)"),
            "xschem 3.4.8RC, protocol 2"
        );
        assert_eq!(helper_version("something else"), "something else");
    }

    fn broken_project(root: &Path) -> AppCtx {
        let cell = root.join("libs/l/inv");
        fs::create_dir_all(cell.join("symbol")).unwrap();
        fs::create_dir_all(cell.join("schematic")).unwrap();
        fs::write(cell.join("schematic/inv.sch"), "v {}\n").unwrap();
        fs::write(
            cell.join("symbol/inv.sym"),
            "v {}\nG {}\nK {type=subcircuit\n}\nV {}\n",
        )
        .unwrap();
        fs::write(
            root.join(libcdept::PROJECT_FILE),
            "name = \"t\"\n[libraries]\nl = \"libs/l\"\n",
        )
        .unwrap();
        let mut app = AppCtx::new(Settings::default(), LogBuffer::new());
        app.load_project(root);
        app
    }

    #[test]
    fn broken_links_are_found_when_the_project_loads_and_fixed_on_request() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = broken_project(tmp.path());
        assert_eq!(app.links.len(), 1);
        let item = app.links[0].clone();
        assert_eq!(item.problem, Problem::Missing);
        assert!(!is_foreign(&item));
        let project = app.project.as_ref().unwrap();
        let entry = project
            .library("l")
            .unwrap()
            .cell("inv")
            .unwrap()
            .entry("symbol");
        assert!(view_link(&app.links, entry.unwrap()).is_some());
        assert!(cell_link(&app.links, "l", "inv").is_some());
        assert!(cell_link(&app.links, "l", "other").is_none());

        assert_eq!(app.fix_links(std::slice::from_ref(&item)), 1);
        assert!(app.links.is_empty());
        let text = fs::read_to_string(&item.symbol).unwrap();
        assert!(
            text.contains("schematic=l/@symname/schematic/@symname.sch"),
            "{text}"
        );
    }

    #[test]
    fn a_read_only_library_is_not_fixed() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = broken_project(tmp.path());
        fs::write(
            tmp.path().join(libcdept::PROJECT_FILE),
            "name = \"t\"\n[libraries]\nl = { path = \"libs/l\", read_only = true }\n",
        )
        .unwrap();
        app.reload_project();
        assert_eq!(app.links.len(), 1);
        let item = app.links[0].clone();
        assert_eq!(app.fix_links(&[item]), 0);
        assert_eq!(app.links.len(), 1);
    }
}
