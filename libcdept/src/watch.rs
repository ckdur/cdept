//! Watching the project's files, so that what changes on disk behind CDePT's back
//! shows up without Reload.
//!
//! A [`Plan`] says what to watch for a project: the library folders (with everything
//! below them), the project file and the files next to it (`tech.toml`, the local
//! override), the files it includes, and the index of the git repositories the
//! libraries are in. A [`Watcher`] watches that with the `notify` crate and gives
//! [`Change`]s: "library X changed", "a project file changed", "a repository
//! changed". Changes are held back until the files have been quiet for a moment
//! ([`Options::delay`]), so a tool writing a view in several steps gives one change.
//!
//! Left out on purpose:
//! - what editors and tools leave behind while they write (backups, swap and lock
//!   files, see [`is_noise`]);
//! - events that do not change what a file holds (opening it, its permissions);
//! - reference libraries outside the project (a PDK's, read-only and huge): only
//!   their top folder is watched unless [`Options::reference_recursive`] is set.
//!
//! No GUI here: the caller gets a wake-up call and asks with [`Watcher::poll`].

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use eyre::{Result, WrapErr};
use notify::event::{EventKind, ModifyKind};
use notify::{RecommendedWatcher, RecursiveMode, Watcher as _};

use crate::{Library, Project};

/// Files next to `cdept.toml` that belong to the project definition.
const PROJECT_FILES: [&str; 4] = [
    crate::PROJECT_FILE,
    "tech.toml",
    "tech.catalog.json",
    "cdept.local.toml",
];

/// How a [`Watcher`] behaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    /// A change is reported once nothing more happened to it for this long.
    pub delay: Duration,
    /// …but no later than this after its first event, however busy the files are.
    pub max_delay: Duration,
    /// Also watch below the top folder of reference libraries (read-only libraries
    /// outside the project, such as a PDK's).
    pub reference_recursive: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            delay: Duration::from_millis(300),
            max_delay: Duration::from_secs(2),
            reference_recursive: false,
        }
    }
}

/// Something changed on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// The project file, a file it includes, `tech.toml` or the local override.
    ProjectFile { file: PathBuf },
    /// Files of a library: created, written, renamed or removed.
    Library { name: String, files: Vec<PathBuf> },
    /// The index or `HEAD` of a git repository (a commit, a checkout, `git add`), or
    /// a repository that was made or removed (`git init`): then the [`Plan`] should
    /// be made again, so that the new repository is followed.
    Repository { root: PathBuf },
}

/// What an event is about.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Target {
    ProjectFile(PathBuf),
    Library(String),
    Repository(PathBuf),
}

/// A folder to watch.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Folder {
    pub path: PathBuf,
    /// With everything below it.
    pub recursive: bool,
}

/// A git repository some of the watched files are in.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Repository {
    /// The work tree.
    pub root: PathBuf,
    /// Where its `index` and `HEAD` are (`<root>/.git`, or where a `.git` file points).
    pub git_dir: PathBuf,
}

/// What to watch for a project.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    /// The files of the project definition.
    pub files: Vec<PathBuf>,
    /// Library name and folder, in definition order.
    pub libraries: Vec<(String, PathBuf)>,
    pub repositories: Vec<Repository>,
    /// The folders to put a watch on: what the above need, without doubles.
    pub folders: Vec<Folder>,
}

/// Whether a library is a reference library: read-only and outside the project.
pub fn is_reference(project: &Project, library: &Library) -> bool {
    library.read_only && !library.path.starts_with(&project.root)
}

impl Plan {
    /// What to watch for `project`.
    pub fn of(project: &Project, options: &Options) -> Plan {
        let mut plan = Plan::default();
        let mut folders = BTreeSet::new();

        for name in PROJECT_FILES {
            plan.files.push(project.root.join(name));
        }
        for library in &project.libraries {
            if !plan.files.contains(&library.defined_in) {
                plan.files.push(library.defined_in.clone());
            }
        }
        for file in &plan.files {
            if let Some(dir) = file.parent() {
                folders.insert(Folder {
                    path: dir.to_path_buf(),
                    recursive: false,
                });
            }
        }

        let mut repositories = BTreeSet::new();
        repositories.extend(repository_of(&project.root));
        for library in &project.libraries {
            if library.error.is_some() || !library.path.is_dir() {
                continue;
            }
            plan.libraries
                .push((library.name.clone(), library.path.clone()));
            let reference = is_reference(project, library);
            folders.insert(Folder {
                path: library.path.clone(),
                recursive: !reference || options.reference_recursive,
            });
            if !reference {
                repositories.extend(repository_of(&library.path));
            }
        }
        for repository in &repositories {
            folders.insert(Folder {
                path: repository.git_dir.clone(),
                recursive: false,
            });
        }
        plan.repositories = repositories.into_iter().collect();

        // A folder below one that is watched with everything below it needs no watch.
        let all: Vec<Folder> = folders.into_iter().collect();
        plan.folders = all
            .iter()
            .filter(|f| {
                !all.iter().any(|other| {
                    other.recursive && other.path != f.path && f.path.starts_with(&other.path)
                })
            })
            .cloned()
            .collect();
        // The same folder asked for both ways: with everything below it wins.
        plan.folders.dedup_by(|b, a| {
            if a.path == b.path {
                a.recursive |= b.recursive;
                true
            } else {
                false
            }
        });
        plan
    }

    /// What a changed path is about; `None` when it is nothing CDePT follows.
    pub fn classify(&self, path: &Path) -> Option<Target> {
        for repository in &self.repositories {
            if let Ok(rest) = path.strip_prefix(&repository.git_dir) {
                return (rest == Path::new("index") || rest == Path::new("HEAD"))
                    .then(|| Target::Repository(repository.root.clone()));
            }
        }
        // A repository made or removed in a watched folder (`git init`): the plan
        // has to be made again to follow its index.
        if path.file_name().is_some_and(|n| n == ".git")
            && let Some(parent) = path.parent()
        {
            return Some(Target::Repository(parent.to_path_buf()));
        }
        if path.components().any(|c| c.as_os_str() == ".git") {
            return None;
        }
        if self.files.iter().any(|f| f == path) {
            return Some(Target::ProjectFile(path.to_path_buf()));
        }
        if is_noise(path) {
            return None;
        }
        // The library with the longest folder holding the path (libraries may nest).
        self.libraries
            .iter()
            .filter(|(_, dir)| path.starts_with(dir))
            .max_by_key(|(_, dir)| dir.components().count())
            .map(|(name, _)| Target::Library(name.clone()))
    }
}

/// The git repository `path` is in, found by walking up to a `.git`.
pub fn repository_of(path: &Path) -> Option<Repository> {
    let mut dir = Some(path);
    while let Some(d) = dir {
        let dot_git = d.join(".git");
        if dot_git.is_dir() {
            return Some(Repository {
                root: d.to_path_buf(),
                git_dir: dot_git,
            });
        }
        if dot_git.is_file() {
            // A work tree or a submodule: `gitdir: <path>`.
            let text = std::fs::read_to_string(&dot_git).ok()?;
            let target = text.lines().find_map(|l| l.strip_prefix("gitdir:"))?.trim();
            return Some(Repository {
                root: d.to_path_buf(),
                git_dir: d.join(target),
            });
        }
        dir = d.parent();
    }
    None
}

/// What editors and tools leave behind while they write: backups (`x~`, `#x#`),
/// swap and lock files, temporary files. A change to one of these is not a change to
/// the library.
pub fn is_noise(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let lower = name.to_ascii_lowercase();
    name.ends_with('~')
        || name.starts_with(".#")
        || (name.starts_with('#') && name.ends_with('#') && name.len() > 1)
        || name.starts_with(".goutputstream-")
        || name.starts_with(".~lock.")
        || name.starts_with(".nfs")
        || name == "4913"
        || name == ".DS_Store"
        || name == ".directory"
        || [
            ".swp",
            ".swo",
            ".swx",
            ".tmp",
            ".lock",
            ".kate-swp",
            ".part",
        ]
        .iter()
        .any(|suffix| lower.ends_with(suffix))
}

/// Whether an event changes what is on disk (not just a file being opened or its
/// permissions).
fn is_change(kind: &EventKind) -> bool {
    match kind {
        EventKind::Access(_) => false,
        EventKind::Modify(ModifyKind::Metadata(_)) => false,
        EventKind::Create(_) | EventKind::Remove(_) | EventKind::Modify(_) => true,
        EventKind::Any | EventKind::Other => true,
    }
}

/// Holds changes back until their files have been quiet for a moment.
#[derive(Debug)]
pub struct Debouncer {
    delay: Duration,
    max_delay: Duration,
    pending: BTreeMap<Target, Pending>,
}

#[derive(Debug)]
struct Pending {
    first: Instant,
    last: Instant,
    files: BTreeSet<PathBuf>,
}

impl Debouncer {
    pub fn new(delay: Duration, max_delay: Duration) -> Debouncer {
        Debouncer {
            delay,
            max_delay: max_delay.max(delay),
            pending: BTreeMap::new(),
        }
    }

    /// Something happened to `file` of `target` at `now`.
    pub fn push(&mut self, target: Target, file: PathBuf, now: Instant) {
        let pending = self.pending.entry(target).or_insert_with(|| Pending {
            first: now,
            last: now,
            files: BTreeSet::new(),
        });
        pending.last = now;
        pending.files.insert(file);
    }

    fn due(&self, pending: &Pending) -> Instant {
        (pending.last + self.delay).min(pending.first + self.max_delay)
    }

    /// The changes whose files have been quiet long enough at `now`.
    pub fn ready(&mut self, now: Instant) -> Vec<Change> {
        let due: Vec<Target> = self
            .pending
            .iter()
            .filter(|(_, p)| self.due(p) <= now)
            .map(|(t, _)| t.clone())
            .collect();
        due.into_iter()
            .filter_map(|target| {
                let pending = self.pending.remove(&target)?;
                Some(match target {
                    Target::ProjectFile(file) => Change::ProjectFile { file },
                    Target::Library(name) => Change::Library {
                        name,
                        files: pending.files.into_iter().collect(),
                    },
                    Target::Repository(root) => Change::Repository { root },
                })
            })
            .collect()
    }

    /// When the next change is due, if any is held back.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.pending.values().map(|p| self.due(p)).min()
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}

/// Watches the files of a [`Plan`]; see the module documentation.
pub struct Watcher {
    notify: RecommendedWatcher,
    plan: Arc<Mutex<Plan>>,
    changes: Receiver<Vec<Change>>,
    problems: Vec<String>,
}

impl Watcher {
    /// Start watching. `wake` is called (from another thread) when changes are ready
    /// for [`poll`](Self::poll).
    pub fn start(
        plan: Plan,
        options: Options,
        wake: impl Fn() + Send + 'static,
    ) -> Result<Watcher> {
        let (raw_tx, raw_rx) = mpsc::channel::<notify::Result<notify::Event>>();
        let mut notify = notify::recommended_watcher(move |event| {
            let _ = raw_tx.send(event);
        })
        .wrap_err("cannot watch files on this system")?;
        let mut problems = Vec::new();
        for folder in &plan.folders {
            if let Err(problem) = watch(&mut notify, folder) {
                problems.push(problem);
            }
        }
        let plan = Arc::new(Mutex::new(plan));
        let (tx, changes) = mpsc::channel();
        let shared = plan.clone();
        thread::Builder::new()
            .name("cdept-watch".into())
            .spawn(move || {
                let mut debouncer = Debouncer::new(options.delay, options.max_delay);
                loop {
                    let event = match debouncer.next_deadline() {
                        Some(deadline) => {
                            let wait = deadline.saturating_duration_since(Instant::now());
                            match raw_rx.recv_timeout(wait) {
                                Ok(event) => Some(event),
                                Err(RecvTimeoutError::Timeout) => None,
                                Err(RecvTimeoutError::Disconnected) => break,
                            }
                        }
                        None => match raw_rx.recv() {
                            Ok(event) => Some(event),
                            Err(_) => break,
                        },
                    };
                    match event {
                        Some(Ok(event)) if is_change(&event.kind) => {
                            let plan = shared.lock().unwrap();
                            let now = Instant::now();
                            for path in event.paths {
                                if let Some(target) = plan.classify(&path) {
                                    debouncer.push(target, path, now);
                                }
                            }
                        }
                        Some(Ok(_)) => {}
                        Some(Err(error)) => {
                            tracing::debug!(target: "cdept::watch", "watch error: {error}");
                        }
                        None => {}
                    }
                    let ready = debouncer.ready(Instant::now());
                    if !ready.is_empty() {
                        if tx.send(ready).is_err() {
                            break;
                        }
                        wake();
                    }
                }
            })
            .wrap_err("cannot start the watch thread")?;
        Ok(Watcher {
            notify,
            plan,
            changes,
            problems,
        })
    }

    /// Start watching the files of `project`.
    pub fn for_project(
        project: &Project,
        options: Options,
        wake: impl Fn() + Send + 'static,
    ) -> Result<Watcher> {
        Watcher::start(Plan::of(project, &options), options, wake)
    }

    /// What is watched.
    pub fn plan(&self) -> Plan {
        self.plan.lock().unwrap().clone()
    }

    /// Watch something else from now on (the project's libraries changed): folders no
    /// longer in the plan are let go, new ones are added.
    pub fn set_plan(&mut self, plan: Plan) {
        let old = self.plan();
        if old == plan {
            return;
        }
        for folder in old.folders.iter().filter(|f| !plan.folders.contains(f)) {
            let _ = self.notify.unwatch(&folder.path);
        }
        self.problems.retain(|p| {
            plan.folders
                .iter()
                .any(|f| p.starts_with(&f.path.display().to_string()))
        });
        for folder in plan.folders.iter().filter(|f| !old.folders.contains(f)) {
            if let Err(problem) = watch(&mut self.notify, folder) {
                self.problems.push(problem);
            }
        }
        *self.plan.lock().unwrap() = plan;
    }

    /// The changes that are ready, oldest first. Never waits.
    pub fn poll(&self) -> Vec<Change> {
        let mut all: Vec<Change> = Vec::new();
        while let Ok(batch) = self.changes.try_recv() {
            for change in batch {
                merge(&mut all, change);
            }
        }
        all
    }

    /// Wait up to `timeout` for changes (for tests and command-line use).
    pub fn wait(&self, timeout: Duration) -> Vec<Change> {
        let mut all = Vec::new();
        if let Ok(batch) = self.changes.recv_timeout(timeout) {
            for change in batch {
                merge(&mut all, change);
            }
        }
        for change in self.poll() {
            merge(&mut all, change);
        }
        all
    }

    /// Folders that could not be watched, each as "<folder>: <why>".
    pub fn problems(&self) -> &[String] {
        &self.problems
    }
}

fn watch(notify: &mut RecommendedWatcher, folder: &Folder) -> Result<(), String> {
    let mode = if folder.recursive {
        RecursiveMode::Recursive
    } else {
        RecursiveMode::NonRecursive
    };
    notify
        .watch(&folder.path, mode)
        .map_err(|e| format!("{}: {e}", folder.path.display()))
}

/// Add `change` to `all`, joining it with an earlier change of the same thing.
fn merge(all: &mut Vec<Change>, change: Change) {
    if let Change::Library { name, files } = &change {
        for earlier in all.iter_mut() {
            if let Change::Library {
                name: other,
                files: known,
            } = earlier
                && other == name
            {
                for file in files {
                    if !known.contains(file) {
                        known.push(file.clone());
                    }
                }
                return;
            }
        }
    } else if all.contains(&change) {
        return;
    }
    all.push(change);
}

impl Project {
    /// Scan one library again; `false` when the project has no such library.
    pub fn rescan_library(&mut self, name: &str) -> bool {
        match self.libraries.iter_mut().find(|l| l.name == name) {
            Some(library) => {
                library.scan();
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan() -> Plan {
        Plan {
            files: vec!["/p/cdept.toml".into(), "/p/tech.toml".into()],
            libraries: vec![
                ("analog".into(), "/p/libs/analog".into()),
                ("inner".into(), "/p/libs/analog/inner".into()),
            ],
            repositories: vec![Repository {
                root: "/p".into(),
                git_dir: "/p/.git".into(),
            }],
            folders: Vec::new(),
        }
    }

    #[test]
    fn paths_are_told_apart() {
        let plan = plan();
        let lib = |name: &str| Some(Target::Library(name.into()));
        assert_eq!(
            plan.classify(Path::new("/p/cdept.toml")),
            Some(Target::ProjectFile("/p/cdept.toml".into()))
        );
        assert_eq!(plan.classify(Path::new("/p/notes.txt")), None);
        assert_eq!(
            plan.classify(Path::new("/p/libs/analog/ota/schematic/ota.sch")),
            lib("analog")
        );
        // The innermost library wins.
        assert_eq!(
            plan.classify(Path::new("/p/libs/analog/inner/x.sch")),
            lib("inner")
        );
        // A sidecar is a real change; a backup or a swap file is not.
        assert_eq!(
            plan.classify(Path::new("/p/libs/analog/ota/layout/ota.gds.json")),
            lib("analog")
        );
        for noise in ["ota.sch~", ".ota.sch.swp", "#ota.sch#", ".#ota.sch", "4913"] {
            let path = Path::new("/p/libs/analog/ota/schematic").join(noise);
            assert_eq!(plan.classify(&path), None, "{noise}");
        }
        assert_eq!(
            plan.classify(Path::new("/p/.git/index")),
            Some(Target::Repository("/p".into()))
        );
        assert_eq!(plan.classify(Path::new("/p/.git/index.lock")), None);
        // `git init` in a library folder.
        assert_eq!(
            plan.classify(Path::new("/p/libs/analog/.git")),
            Some(Target::Repository("/p/libs/analog".into()))
        );
        assert_eq!(plan.classify(Path::new("/p/.git/objects/ab/cd")), None);
        assert_eq!(plan.classify(Path::new("/p/libs/analog/.git/config")), None);
    }

    #[test]
    fn changes_wait_until_the_files_are_quiet() {
        let mut d = Debouncer::new(Duration::from_millis(100), Duration::from_millis(350));
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let lib = Target::Library("a".into());
        d.push(lib.clone(), "/a/x.sch".into(), at(0));
        d.push(lib.clone(), "/a/y.sch".into(), at(60));
        assert!(d.ready(at(100)).is_empty(), "y was written 40 ms ago");
        assert_eq!(d.next_deadline(), Some(at(160)));
        let ready = d.ready(at(160));
        assert_eq!(
            ready,
            [Change::Library {
                name: "a".into(),
                files: vec!["/a/x.sch".into(), "/a/y.sch".into()],
            }]
        );
        assert!(d.is_empty());

        // A file written over and over is reported after max_delay all the same.
        for ms in (0..400).step_by(50) {
            d.push(lib.clone(), "/a/x.sch".into(), at(1000 + ms));
        }
        assert!(d.ready(at(1340)).is_empty());
        assert_eq!(d.ready(at(1350)).len(), 1);
    }

    #[test]
    fn changes_of_one_library_are_joined() {
        let mut all = Vec::new();
        let change = |file: &str| Change::Library {
            name: "a".into(),
            files: vec![file.into()],
        };
        merge(&mut all, change("/a/x"));
        merge(&mut all, Change::Repository { root: "/p".into() });
        merge(&mut all, change("/a/y"));
        merge(&mut all, change("/a/x"));
        merge(&mut all, Change::Repository { root: "/p".into() });
        assert_eq!(
            all,
            [
                Change::Library {
                    name: "a".into(),
                    files: vec!["/a/x".into(), "/a/y".into()],
                },
                Change::Repository { root: "/p".into() },
            ]
        );
    }

    #[test]
    fn events_that_change_nothing_are_dropped() {
        use notify::event::{AccessKind, CreateKind, DataChange, MetadataKind, RemoveKind};
        assert!(!is_change(&EventKind::Access(AccessKind::Any)));
        assert!(!is_change(&EventKind::Modify(ModifyKind::Metadata(
            MetadataKind::Permissions
        ))));
        assert!(is_change(&EventKind::Modify(ModifyKind::Data(
            DataChange::Any
        ))));
        assert!(is_change(&EventKind::Create(CreateKind::File)));
        assert!(is_change(&EventKind::Remove(RemoveKind::Any)));
    }
}
