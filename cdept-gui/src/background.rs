//! Keeping the project in step with the disk and with git, off the UI thread.
//!
//! - **Watching.** The files of the open project are watched
//!   ([`libcdept::watch`]): a change in a library folder scans that library again, a
//!   change of `cdept.toml` (or a file it includes, or `tech.toml`) reads the project
//!   again, a change of a git index reads the git status again.
//! - **Scanning.** Loading a project, reading it again and scanning libraries run on
//!   worker threads. The window keeps showing what it had; the result replaces it when
//!   it is ready, unless the project moved on meanwhile (then the work is done again).
//! - **Documents.** A schematic or layout tab whose file changed on disk behind its
//!   back opens it again when it has no changes of its own, and shows a notice with
//!   Reload / Keep Mine when it has (see [`disk`]).
//! - **Git.** The status per library, cell and view, for the badges of the library
//!   views (`views::vcs`) and the status bar.
//! - **Deleting** goes to the trash unless asked otherwise ([`remove_cell`]).
//!
//! The shell calls [`tick`] once per frame.

mod disk;
pub(crate) mod script;
mod ui;

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Instant;

use libcdept::trash::{Removal, Trashed};
use libcdept::vcs::Status;
use libcdept::watch::{self, Change, Plan, Watcher};
use libcdept::xschem::symbol::{self, Unlinked};
use libcdept::{Library, PROJECT_FILE, Project};

use crate::dialogs::Dialog;
use crate::state::AppCtx;
use crate::views::Views;

pub(crate) use ui::{
    delete_note, loading_ui, menu_ui, recent_button, removal, stale_ui, status_ui,
};

const TARGET: &str = "cdept::disk";

/// What a worker thread hands back.
enum Outcome {
    /// A project was read for the first time.
    Loaded {
        root: PathBuf,
        result: Result<Box<Project>, String>,
    },
    /// The open project was read again.
    Reloaded {
        before: Box<Project>,
        result: Result<Box<Project>, String>,
        /// The user asked for it (Reload), so failures are shown, not only logged.
        explicit: bool,
    },
    /// Libraries were scanned again.
    Scanned {
        root: PathBuf,
        /// Each library as it was when the scan started, and as it is on disk.
        libraries: Vec<(Library, Library)>,
        links: Vec<Unlinked>,
    },
    Git {
        root: PathBuf,
        status: Status,
    },
}

/// What decides what is watched: when it changes, the watch plan is made again.
#[derive(Debug, Clone, PartialEq, Eq)]
struct WatchKey {
    root: PathBuf,
    libraries: Vec<(String, PathBuf, bool, bool, PathBuf)>,
}

impl WatchKey {
    fn of(project: &Project) -> WatchKey {
        WatchKey {
            root: project.root.clone(),
            libraries: project
                .libraries
                .iter()
                .map(|l| {
                    (
                        l.name.clone(),
                        l.path.clone(),
                        l.read_only,
                        l.error.is_some(),
                        l.defined_in.clone(),
                    )
                })
                .collect(),
        }
    }
}

/// The state of everything that runs behind the window; a field of [`AppCtx`].
pub struct Background {
    egui: Option<egui::Context>,
    sender: Sender<Outcome>,
    results: Receiver<Outcome>,

    watcher: Option<Watcher>,
    watch_key: Option<WatchKey>,
    /// The folder of the project the git status is about.
    root: Option<PathBuf>,

    /// The project being read for the first time.
    loading: Option<PathBuf>,
    /// The documents of the last session wait for the project to be read.
    restoring: bool,
    reloading: bool,
    reload_again: bool,
    /// Libraries being scanned, and the ones to scan after that.
    scanning: BTreeSet<String>,
    scan_wanted: BTreeSet<String>,
    /// How many scans changed what the window shows.
    pub(crate) scans: u64,

    git: Arc<Status>,
    git_running: bool,
    git_wanted: bool,

    /// How the next delete of the dialogs deletes. The dialog sets it; anything else
    /// that runs a delete gets [`Removal::Permanent`], as before there was a trash.
    pub(crate) removal: Removal,
    /// What the last delete put into the trash.
    pub(crate) trashed: Vec<Trashed>,

    /// Open documents and the files they show.
    docs: disk::Documents,
    /// Whether the project file of a recent project is there, and when that was
    /// looked at.
    recent: HashMap<PathBuf, (Instant, bool)>,
}

impl Default for Background {
    fn default() -> Self {
        let (sender, results) = mpsc::channel();
        Background {
            egui: None,
            sender,
            results,
            watcher: None,
            watch_key: None,
            root: None,
            loading: None,
            restoring: false,
            reloading: false,
            reload_again: false,
            scanning: BTreeSet::new(),
            scan_wanted: BTreeSet::new(),
            scans: 0,
            git: Arc::new(Status::default()),
            git_running: false,
            git_wanted: false,
            removal: Removal::Permanent,
            trashed: Vec::new(),
            docs: disk::Documents::default(),
            recent: HashMap::new(),
        }
    }
}

impl Background {
    /// A project is being read for the first time: there is none to show yet.
    pub fn loading(&self) -> Option<&Path> {
        self.loading.as_deref()
    }

    /// The documents of the last session are waiting for the project: the shell must
    /// not close them for belonging to "another project" meanwhile.
    pub fn restoring(&self) -> bool {
        self.restoring
    }

    /// Something is on its way that changes what the project holds (for the test
    /// driver, which waits for it).
    pub fn busy(&self) -> bool {
        self.loading.is_some()
            || self.reloading
            || !self.scanning.is_empty()
            || !self.scan_wanted.is_empty()
    }

    /// Libraries are being scanned.
    pub fn scanning(&self) -> bool {
        !self.scanning.is_empty()
    }

    /// The git status as last read.
    pub fn git(&self) -> &Arc<Status> {
        &self.git
    }

    /// Whether the files are being watched.
    pub fn watching(&self) -> bool {
        self.watcher.is_some()
    }

    fn spawn(&self, name: &str, job: impl FnOnce() -> Outcome + Send + 'static) {
        let (sender, egui) = (self.sender.clone(), self.egui.clone());
        let spawned = thread::Builder::new()
            .name(format!("cdept-{name}"))
            .spawn(move || {
                let _ = sender.send(job());
                if let Some(egui) = egui {
                    egui.request_repaint();
                }
            });
        if let Err(error) = spawned {
            tracing::error!(target: TARGET, "cannot start a worker thread: {error}");
        }
    }
}

/// What [`tick`] asks of the shell.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Tick {
    /// The project given at start-up is there (or failed): reopen the documents of
    /// the last session now.
    pub restore: bool,
}

/// Open the project `path` is in without holding up the window. The project that is
/// open closes first (the caller has dealt with unsaved changes); asking for the
/// project that is open reads it again and keeps its documents.
pub fn load(app: &mut AppCtx, egui: &egui::Context, path: &Path) {
    if cfg!(test) {
        // Unit tests look at the project right after asking for it.
        return app.load_project(path);
    }
    let root = Project::find_root(path).unwrap_or_else(|| path.to_path_buf());
    if app.project.as_ref().is_some_and(|p| p.root == root) {
        return reload(app, egui);
    }
    if !root.join(PROJECT_FILE).is_file() && forget_recent(app, &root) {
        return;
    }
    app.background.egui.get_or_insert_with(|| egui.clone());
    app.close_project();
    start_load(app, root);
}

/// Open the project given at start-up; the documents of the last session are reopened
/// once it is there ([`Tick::restore`]).
pub fn load_at_start(app: &mut AppCtx, egui: &egui::Context, path: &Path) {
    if cfg!(test) {
        return app.load_project(path);
    }
    app.background.egui.get_or_insert_with(|| egui.clone());
    app.background.restoring = true;
    let root = Project::find_root(path).unwrap_or_else(|| path.to_path_buf());
    start_load(app, root);
}

fn start_load(app: &mut AppCtx, root: PathBuf) {
    tracing::info!(target: "cdept::project", "opening {}…", root.display());
    app.background.loading = Some(root.clone());
    app.background.spawn("load", move || {
        let result = Project::load(&root)
            .map(Box::new)
            .map_err(|e| format!("{e:#}"));
        Outcome::Loaded { root, result }
    });
}

/// A recent project whose folder is gone: drop it from the list, say so. `false` when
/// it was not in the list (then the usual error tells what is wrong).
fn forget_recent(app: &mut AppCtx, root: &Path) -> bool {
    let before = app.settings.recent_projects.len();
    app.settings.recent_projects.retain(|p| p != root);
    if app.settings.recent_projects.len() == before {
        return false;
    }
    app.background.recent.remove(root);
    tracing::warn!(
        target: "cdept::project",
        "{}: no project there any more; removed from the recent projects",
        root.display()
    );
    true
}

/// Read `cdept.toml` and the library folders again (the Reload command), without
/// holding up the window.
pub fn reload(app: &mut AppCtx, egui: &egui::Context) {
    if cfg!(test) {
        return app.reload_project();
    }
    app.background.egui.get_or_insert_with(|| egui.clone());
    start_reload(app, true);
}

fn start_reload(app: &mut AppCtx, explicit: bool) {
    let Some(project) = &app.project else {
        return;
    };
    if app.background.reloading {
        app.background.reload_again = true;
        return;
    }
    app.background.reloading = true;
    let before = Box::new(project.clone());
    app.background.spawn("reload", move || {
        let mut after = before.clone();
        let result = after.reload().map(|()| after).map_err(|e| format!("{e:#}"));
        Outcome::Reloaded {
            before,
            result,
            explicit,
        }
    });
}

/// Scan the wanted libraries, when no scan is running.
fn start_scan(app: &mut AppCtx) {
    let background = &mut app.background;
    if !background.scanning.is_empty() || background.scan_wanted.is_empty() {
        return;
    }
    let Some(project) = &app.project else {
        background.scan_wanted.clear();
        return;
    };
    let names: Vec<String> = std::mem::take(&mut background.scan_wanted)
        .into_iter()
        .filter(|name| project.library(name).is_some())
        .collect();
    if names.is_empty() {
        return;
    }
    background.scanning = names.iter().cloned().collect();
    let mut snapshot = project.clone();
    background.spawn("scan", move || {
        let before: Vec<Library> = names
            .iter()
            .filter_map(|name| snapshot.library(name).cloned())
            .collect();
        for name in &names {
            snapshot.rescan_library(name);
        }
        let links = symbol::check(&snapshot, None, None);
        let libraries = before
            .into_iter()
            .filter_map(|b| {
                let after = snapshot.library(&b.name)?.clone();
                Some((b, after))
            })
            .collect();
        Outcome::Scanned {
            root: snapshot.root.clone(),
            libraries,
            links,
        }
    });
}

fn start_git(app: &mut AppCtx) {
    let background = &mut app.background;
    if background.git_running || !background.git_wanted {
        return;
    }
    background.git_wanted = false;
    let Some(project) = &app.project else {
        background.git = Arc::new(Status::default());
        return;
    };
    background.git_running = true;
    let (root, paths) = (project.root.clone(), libcdept::vcs::project_paths(project));
    background.spawn("git", move || Outcome::Git {
        root,
        status: Status::read(&paths),
    });
}

fn log_problems(project: &Project) {
    for warning in &project.warnings {
        tracing::warn!(target: "cdept::project", "{warning}");
    }
    for library in &project.libraries {
        if let Some(error) = &library.error {
            tracing::warn!(target: "cdept::project", "library '{}': {error}", library.name);
        }
    }
}

fn apply(app: &mut AppCtx, outcome: Outcome, tick: &mut Tick) {
    match outcome {
        Outcome::Loaded { root, result } => {
            if app.background.loading.as_ref() != Some(&root) {
                return;
            }
            app.background.loading = None;
            match result {
                Ok(project) => app.set_project(*project),
                Err(error) => {
                    if !forget_recent(app, &root) {
                        app.error(error);
                    }
                }
            }
            if std::mem::take(&mut app.background.restoring) {
                tick.restore = true;
            }
        }
        Outcome::Reloaded {
            before,
            result,
            explicit,
        } => {
            app.background.reloading = false;
            // Reload also reads the git status again.
            app.background.git_wanted |= explicit;
            match result {
                Err(error) if explicit => app.error(error),
                Err(error) => tracing::warn!(
                    target: "cdept::project",
                    "the project file changed on disk but cannot be read ({error}); the \
                     project stays as it was"
                ),
                Ok(after) => {
                    if app.project.as_ref() == Some(&*before) {
                        if *after != *before {
                            log_problems(&after);
                            app.project = Some(*after);
                            app.refresh_links();
                            app.validate_selection();
                            let why = if explicit {
                                ""
                            } else {
                                " (it changed on disk)"
                            };
                            tracing::info!(target: "cdept::project", "project read again{why}");
                        } else if explicit {
                            tracing::info!(target: "cdept::project", "project read again: nothing changed");
                        }
                    } else if app.project.as_ref().is_some_and(|p| p.root == before.root) {
                        // The project moved on while it was being read: read it again.
                        app.background.reload_again = true;
                    }
                }
            }
            if std::mem::take(&mut app.background.reload_again) {
                start_reload(app, false);
            }
        }
        Outcome::Scanned {
            root,
            libraries,
            links,
        } => {
            app.background.scanning.clear();
            let Some(project) = &mut app.project else {
                return;
            };
            if project.root != root {
                return;
            }
            let mut changed = Vec::new();
            for (before, after) in libraries {
                let Some(current) = project.libraries.iter_mut().find(|l| l.name == before.name)
                else {
                    continue;
                };
                if *current != before {
                    // An operation of CDePT changed the library meanwhile: what the
                    // scan saw may be older than what is shown.
                    app.background.scan_wanted.insert(before.name);
                } else if *current != after {
                    changed.push(after.name.clone());
                    *current = after;
                }
            }
            if !changed.is_empty() {
                tracing::debug!(
                    target: TARGET,
                    "changed on disk: {}",
                    changed.join(", ")
                );
                app.background.scans += 1;
                if app.background.scan_wanted.is_empty() {
                    app.links = links;
                } else {
                    app.refresh_links();
                }
                app.validate_selection();
            }
        }
        Outcome::Git { root, status } => {
            app.background.git_running = false;
            if app.project.as_ref().is_some_and(|p| p.root == root) {
                app.background.git = Arc::new(status);
            }
        }
    }
}

/// Keep the watcher on the files of the open project, and act on what it reports.
fn follow(app: &mut AppCtx) {
    // Another project: what was known of git is not about it.
    let root = app.project.as_ref().map(|p| &p.root);
    if root != app.background.root.as_ref() {
        app.background.root = root.cloned();
        app.background.git = Arc::new(Status::default());
        app.background.git_wanted = true;
    }
    let wanted = match &app.project {
        Some(project) if app.settings.watch => Some(WatchKey::of(project)),
        _ => None,
    };
    if wanted != app.background.watch_key {
        let background = &mut app.background;
        match (&wanted, &app.project) {
            (Some(_), Some(project)) => {
                let options = watch::Options::default();
                let plan = Plan::of(project, &options);
                match &mut background.watcher {
                    Some(watcher) => watcher.set_plan(plan),
                    None => {
                        let egui = background.egui.clone();
                        let wake = move || {
                            if let Some(egui) = &egui {
                                egui.request_repaint();
                            }
                        };
                        match Watcher::start(plan, options, wake) {
                            Ok(watcher) => background.watcher = Some(watcher),
                            Err(error) => tracing::warn!(
                                target: TARGET,
                                "changes on disk are not followed: {error:#}"
                            ),
                        }
                    }
                }
                if let Some(watcher) = &background.watcher {
                    for problem in watcher.problems() {
                        tracing::warn!(target: TARGET, "not watched: {problem}");
                    }
                }
            }
            _ => background.watcher = None,
        }
        // The libraries are other ones: so is the git status.
        background.git_wanted = true;
        background.watch_key = wanted;
    }

    let changes = match &app.background.watcher {
        Some(watcher) => watcher.poll(),
        None => Vec::new(),
    };
    let mut reload = false;
    for change in changes {
        match change {
            Change::ProjectFile { file } => {
                tracing::debug!(target: TARGET, "{} changed", file.display());
                reload = true;
            }
            Change::Library { name, files } => {
                tracing::trace!(target: TARGET, "library '{name}': {} file(s) changed", files.len());
                app.background.scan_wanted.insert(name);
                app.background.docs.changed(files);
                app.background.git_wanted = true;
            }
            Change::Repository { .. } => {
                app.background.git_wanted = true;
                // It may be a new repository: look again at what there is to watch.
                app.background.watch_key = None;
            }
        }
    }
    if reload {
        start_reload(app, false);
    }
}

/// Once per frame: take in what the workers finished, follow the files, start what is
/// wanted next.
pub fn tick(app: &mut AppCtx, views: &mut Views, egui: &egui::Context) -> Tick {
    let mut tick = Tick::default();
    app.background.egui.get_or_insert_with(|| egui.clone());
    let outcomes: Vec<Outcome> = app.background.results.try_iter().collect();
    for outcome in outcomes {
        apply(app, outcome, &mut tick);
    }
    if app.project.is_none() {
        app.background.scan_wanted.clear();
    }
    follow(app);
    disk::check(app, views);
    start_scan(app);
    start_git(app);
    crate::views::vcs::publish(egui, &app.background.git);
    tick
}

/// After a dialog ran its operation: scan what it touched, now, so that the window
/// shows the result in the same frame. Other libraries are left alone (they are
/// followed by the watcher).
pub fn rescan_after(project: &mut Project, dialog: &Dialog) {
    let touched: Option<Vec<&String>> = match dialog {
        Dialog::NewCell { library, .. }
        | Dialog::NewView { library, .. }
        | Dialog::RenameCell { library, .. }
        | Dialog::DeleteCell { library, .. }
        | Dialog::DeleteView { library, .. } => Some(vec![library]),
        Dialog::CopyCell { library, to, .. } => Some(vec![library, to]),
        _ => None,
    };
    match touched {
        Some(names) => {
            for name in names {
                project.rescan_library(name);
            }
        }
        None => project.rescan(),
    }
}

fn report_trashed(background: &mut Background, what: &str, trashed: Vec<Trashed>) {
    for item in &trashed {
        tracing::info!(
            target: "cdept::library",
            "{what}: {} moved to the trash ({})",
            item.original.display(),
            item.file.display()
        );
    }
    background.trashed = trashed;
}

/// Delete a cell the way the dialog asked ([`Background::removal`]): to the trash, or
/// for good.
pub fn remove_cell(background: &mut Background, library: &Library, cell: &str) -> eyre::Result<()> {
    let how = std::mem::replace(&mut background.removal, Removal::Permanent);
    let trashed = library.remove_cell(cell, how)?;
    report_trashed(background, &format!("{}/{cell}", library.name), trashed);
    Ok(())
}

/// Delete a view the way the dialog asked.
pub fn remove_view(
    background: &mut Background,
    library: &Library,
    cell: &str,
    view: &str,
) -> eyre::Result<()> {
    let how = std::mem::replace(&mut background.removal, Removal::Permanent);
    let trashed = library.remove_view(cell, view, how)?;
    report_trashed(
        background,
        &format!("{}/{cell}/{view}", library.name),
        trashed,
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::log::LogBuffer;
    use crate::state::{Selection, Settings};

    /// A project with a `cells` library `a` (cell `inv`) and a flat library `b`.
    fn app(root: &Path) -> AppCtx {
        fs::create_dir_all(root.join("libs/a/inv/schematic")).unwrap();
        fs::create_dir_all(root.join("libs/b")).unwrap();
        fs::write(root.join("libs/a/inv/schematic/inv.sch"), "v {}\n").unwrap();
        fs::write(root.join("libs/b/r.sym"), "v {}\n").unwrap();
        fs::write(
            root.join(PROJECT_FILE),
            "name = \"t\"\n[libraries]\na = { path = \"libs/a\", layout = \"cells\" }\nb = \"libs/b\"\n",
        )
        .unwrap();
        let mut app = AppCtx::new(Settings::default(), LogBuffer::new());
        app.load_project(root);
        app
    }

    fn library(app: &AppCtx, name: &str) -> Library {
        app.project.as_ref().unwrap().library(name).unwrap().clone()
    }

    fn scanned(app: &AppCtx, name: &str) -> Library {
        let mut library = library(app, name);
        library.scan();
        library
    }

    fn cell_dir(root: &Path, name: &str) {
        let dir = root.join("libs/a").join(name).join("schematic");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(format!("{name}.sch")), "v {}\n").unwrap();
    }

    #[test]
    fn a_scan_is_taken_when_the_library_did_not_move_on() {
        let _env = libcdept::testutil::hermetic_env();
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app(tmp.path());
        let root = app.project.as_ref().unwrap().root.clone();
        app.selection = Selection::cell("a", "inv");
        let mut tick = Tick::default();

        // Another program adds a cell; the scan started from what the window shows.
        cell_dir(&root, "nand");
        let outcome = Outcome::Scanned {
            root: root.clone(),
            libraries: vec![(library(&app, "a"), scanned(&app, "a"))],
            links: Vec::new(),
        };
        apply(&mut app, outcome, &mut tick);
        assert!(library(&app, "a").cell("nand").is_some());
        assert_eq!(app.background.scans, 1);
        assert!(app.background.scan_wanted.is_empty());
        // What was selected stays selected.
        assert_eq!(app.selection, Selection::cell("a", "inv"));

        // A scan that started before CDePT itself changed the library is not taken:
        // it would bring the old state back. The library is scanned again instead.
        let before = library(&app, "a");
        let old_scan = scanned(&app, "a");
        cell_dir(&root, "nor");
        app.project.as_mut().unwrap().rescan_library("a");
        let outcome = Outcome::Scanned {
            root: root.clone(),
            libraries: vec![(before, old_scan)],
            links: Vec::new(),
        };
        apply(&mut app, outcome, &mut tick);
        assert!(
            library(&app, "a").cell("nor").is_some(),
            "the newer state stays"
        );
        assert!(app.background.scan_wanted.contains("a"));
        assert_eq!(app.background.scans, 1);

        // A scan of another project's library is dropped.
        let outcome = Outcome::Scanned {
            root: root.join("elsewhere"),
            libraries: vec![(library(&app, "b"), Library::clone(&library(&app, "a")))],
            links: Vec::new(),
        };
        apply(&mut app, outcome, &mut tick);
        assert_eq!(library(&app, "b").cells.len(), 1);
    }

    #[test]
    fn a_selected_cell_that_is_gone_is_deselected() {
        let _env = libcdept::testutil::hermetic_env();
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app(tmp.path());
        let root = app.project.as_ref().unwrap().root.clone();
        app.selection = Selection::view("a", "inv", "schematic");
        fs::remove_dir_all(root.join("libs/a/inv")).unwrap();
        let outcome = Outcome::Scanned {
            root,
            libraries: vec![(library(&app, "a"), scanned(&app, "a"))],
            links: Vec::new(),
        };
        apply(&mut app, outcome, &mut Tick::default());
        assert_eq!(app.selection, Selection::library("a"));
    }

    #[test]
    fn a_reload_is_taken_only_for_the_project_it_started_from() {
        let _env = libcdept::testutil::hermetic_env();
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app(tmp.path());
        let root = app.project.as_ref().unwrap().root.clone();
        let before = Box::new(app.project.clone().unwrap());

        // cdept.toml edited by hand: a library less.
        fs::write(
            root.join(PROJECT_FILE),
            "name = \"t\"\n[libraries]\nb = \"libs/b\"\n",
        )
        .unwrap();
        let mut after = before.clone();
        after.reload().unwrap();
        app.background.reloading = true;
        let outcome = Outcome::Reloaded {
            before: before.clone(),
            result: Ok(after.clone()),
            explicit: false,
        };
        apply(&mut app, outcome, &mut Tick::default());
        assert!(!app.background.reloading);
        assert_eq!(app.project.as_ref().unwrap().libraries.len(), 1);

        // The same result again, now that the project is another one: not taken, and
        // a new reload starts (from the project as it is).
        app.project.as_mut().unwrap().name = "moved on".into();
        let outcome = Outcome::Reloaded {
            before,
            result: Ok(after),
            explicit: false,
        };
        apply(&mut app, outcome, &mut Tick::default());
        assert_eq!(app.project.as_ref().unwrap().name, "moved on");
        assert!(app.background.reloading, "read again");

        // A project file that cannot be read keeps the project (and is not a dialog
        // unless the user asked for the reload).
        let outcome = Outcome::Reloaded {
            before: Box::new(app.project.clone().unwrap()),
            result: Err("bad".into()),
            explicit: false,
        };
        apply(&mut app, outcome, &mut Tick::default());
        assert!(app.project.is_some());
        assert!(app.dialog.is_none());
    }

    #[test]
    fn a_dialog_scans_the_library_it_touched() {
        let _env = libcdept::testutil::hermetic_env();
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app(tmp.path());
        let root = app.project.as_ref().unwrap().root.clone();
        cell_dir(&root, "nand");
        fs::write(root.join("libs/b/c.sym"), "v {}\n").unwrap();
        let project = app.project.as_mut().unwrap();
        let dialog = Dialog::DeleteCell {
            library: "a".into(),
            cell: "x".into(),
        };
        rescan_after(project, &dialog);
        assert!(project.library("a").unwrap().cell("nand").is_some());
        assert!(project.library("b").unwrap().cell("c").is_none());
        // A dialog this does not know scans everything.
        rescan_after(project, &Dialog::Error("x".into()));
        assert!(project.library("b").unwrap().cell("c").is_some());
    }

    #[test]
    fn deleting_is_for_good_unless_the_dialog_asked_for_the_trash() {
        let _env = libcdept::testutil::hermetic_env();
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app(tmp.path());
        let root = app.project.as_ref().unwrap().root.clone();
        assert_eq!(app.background.removal, Removal::Permanent);
        let a = library(&app, "a");
        remove_view(&mut app.background, &a, "inv", "schematic").unwrap();
        assert!(!root.join("libs/a/inv/schematic").exists());
        assert!(app.background.trashed.is_empty());
        assert_eq!(removal(false), Removal::Trash);
        assert_eq!(removal(true), Removal::Permanent);
    }

    #[test]
    fn a_recent_project_that_is_gone_is_forgotten() {
        let _env = libcdept::testutil::hermetic_env();
        let tmp = tempfile::tempdir().unwrap();
        let mut app = AppCtx::new(Settings::default(), LogBuffer::new());
        let gone = tmp.path().join("gone");
        app.settings.add_recent(gone.clone());
        app.settings.add_recent(tmp.path().join("other"));
        assert!(forget_recent(&mut app, &gone));
        assert_eq!(app.settings.recent_projects, [tmp.path().join("other")]);
        // Not in the list: the caller reports the error as usual.
        assert!(!forget_recent(&mut app, &gone));
    }

    #[test]
    fn a_first_load_arrives_and_asks_for_the_documents() {
        let _env = libcdept::testutil::hermetic_env();
        let tmp = tempfile::tempdir().unwrap();
        let root = {
            let app = app(tmp.path());
            app.project.as_ref().unwrap().root.clone()
        };
        let mut app = AppCtx::new(Settings::default(), LogBuffer::new());
        app.background.restoring = true;
        app.background.loading = Some(root.clone());
        assert!(app.background.busy());
        let mut tick = Tick::default();
        let outcome = Outcome::Loaded {
            root: root.clone(),
            result: Project::load(&root)
                .map(Box::new)
                .map_err(|e| e.to_string()),
        };
        apply(&mut app, outcome, &mut tick);
        assert!(tick.restore);
        assert!(!app.background.busy() && !app.background.restoring());
        assert_eq!(app.project.as_ref().unwrap().root, root);
        assert_eq!(app.settings.recent_projects, [root]);
    }
}
