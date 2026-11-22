//! Git status of the project's files, per file, view, cell and library.
//!
//! CDePT asks the `git` program (`git status --porcelain=v2`), it does not link a git
//! library. A library can be in the project's repository or be a repository of its
//! own; each repository is asked once, for the folders CDePT shows. Outside a
//! repository, or without `git` on `PATH`, there simply is no status: no errors.
//!
//! ```text
//! file  → State                      (modified, untracked, …)
//! view  → Summary of its files       (Status::entry)
//! cell  → Summary of its folder      (Status::cell)
//! lib   → Summary of its folder      (Status::library)
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;

use crate::watch::repository_of;
use crate::{Cell, Entry, Library, Project};

/// What git says about a file, from harmless to urgent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum State {
    /// Tracked, as committed.
    Clean,
    /// Left out by a `.gitignore`.
    Ignored,
    /// Tracked; its changes are all in the index, ready to commit.
    Staged,
    /// New, and added to the index.
    Added,
    /// New, and not known to git yet.
    Untracked,
    /// Tracked, with changes that are not in the index (also: deleted).
    Modified,
    /// A merge left it in conflict.
    Conflicted,
}

impl State {
    /// One character for a badge; empty for [`State::Clean`].
    pub fn mark(self) -> &'static str {
        match self {
            State::Clean => "",
            State::Ignored => "I",
            State::Staged => "S",
            State::Added => "A",
            State::Untracked => "?",
            State::Modified => "M",
            State::Conflicted => "!",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            State::Clean => "clean",
            State::Ignored => "ignored",
            State::Staged => "staged",
            State::Added => "added",
            State::Untracked => "untracked",
            State::Modified => "modified",
            State::Conflicted => "in conflict",
        }
    }

    /// The file is in the repository's history or index: git can bring it back (as
    /// committed or as staged).
    pub fn is_tracked(self) -> bool {
        !matches!(self, State::Untracked | State::Ignored)
    }
}

/// The states of the files of a view, a cell or a library.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Summary {
    pub conflicted: usize,
    pub modified: usize,
    pub untracked: usize,
    pub added: usize,
    pub staged: usize,
    /// The whole thing is ignored (it is, or is below, an ignored folder or file).
    pub ignored: bool,
}

impl Summary {
    fn count(&mut self, state: State) {
        match state {
            State::Conflicted => self.conflicted += 1,
            State::Modified => self.modified += 1,
            State::Untracked => self.untracked += 1,
            State::Added => self.added += 1,
            State::Staged => self.staged += 1,
            State::Ignored | State::Clean => {}
        }
    }

    fn add(&mut self, other: Summary) {
        self.conflicted += other.conflicted;
        self.modified += other.modified;
        self.untracked += other.untracked;
        self.added += other.added;
        self.staged += other.staged;
    }

    /// How many files are not as committed.
    pub fn changed(&self) -> usize {
        self.conflicted + self.modified + self.untracked + self.added + self.staged
    }

    /// The most urgent state among the files.
    pub fn state(&self) -> State {
        if self.conflicted > 0 {
            State::Conflicted
        } else if self.modified > 0 {
            State::Modified
        } else if self.untracked > 0 {
            State::Untracked
        } else if self.added > 0 {
            State::Added
        } else if self.staged > 0 {
            State::Staged
        } else if self.ignored {
            State::Ignored
        } else {
            State::Clean
        }
    }

    /// "2 modified, 1 untracked", or the single state when nothing is counted.
    pub fn describe(&self) -> String {
        let parts: Vec<String> = [
            (self.conflicted, "in conflict"),
            (self.modified, "modified"),
            (self.untracked, "untracked"),
            (self.added, "added"),
            (self.staged, "staged"),
        ]
        .iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, what)| format!("{n} {what}"))
        .collect();
        if parts.is_empty() {
            self.state().label().to_string()
        } else {
            parts.join(", ")
        }
    }
}

/// A line of `git status`: a file, or a folder that is untracked or ignored as a whole.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Change {
    /// Absolute.
    pub path: PathBuf,
    pub state: State,
    /// Stands for everything below it.
    pub folder: bool,
}

/// The status of one repository, for the folders that were asked about.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Repository {
    /// The work tree.
    pub root: PathBuf,
    /// The branch, or `None` with a detached `HEAD`.
    pub branch: Option<String>,
    /// The commit `HEAD` is at (abbreviated), `None` before the first commit.
    pub commit: Option<String>,
    pub upstream: Option<String>,
    pub ahead: usize,
    pub behind: usize,
    /// Everything that is not clean, sorted by path.
    pub changes: Vec<Change>,
    /// The folders that were asked about; empty: the whole repository.
    pub scope: Vec<PathBuf>,
}

impl Repository {
    /// How many files (or whole folders) are not as committed, ignored ones aside.
    pub fn dirty(&self) -> usize {
        self.changes
            .iter()
            .filter(|c| c.state != State::Ignored)
            .count()
    }

    /// "main", "main ↑2 ↓1", "detached at 1a2b3c4".
    pub fn head(&self) -> String {
        let mut text = match (&self.branch, &self.commit) {
            (Some(branch), _) => branch.clone(),
            (None, Some(commit)) => format!("detached at {commit}"),
            (None, None) => "no commit yet".into(),
        };
        if self.ahead > 0 {
            text.push_str(&format!(" ↑{}", self.ahead));
        }
        if self.behind > 0 {
            text.push_str(&format!(" ↓{}", self.behind));
        }
        text
    }

    fn covers(&self, path: &Path) -> bool {
        path.starts_with(&self.root)
            && (self.scope.is_empty()
                || self
                    .scope
                    .iter()
                    .any(|s| path.starts_with(s) || s.starts_with(path)))
    }
}

/// The git status of a set of folders: one [`Repository`] per repository they are in.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Status {
    pub repositories: Vec<Repository>,
    /// Path → index into the changes, for lookups.
    #[serde(skip)]
    index: BTreeMap<PathBuf, (State, bool)>,
}

impl Status {
    /// Ask git about the folders in `paths`. Folders outside a repository are left
    /// out; without `git` the result is empty.
    pub fn read(paths: &[PathBuf]) -> Status {
        let mut by_root: BTreeMap<PathBuf, Vec<PathBuf>> = BTreeMap::new();
        for path in paths {
            if let Some(repository) = repository_of(path) {
                by_root
                    .entry(repository.root)
                    .or_default()
                    .push(path.clone());
            }
        }
        let mut status = Status::default();
        for (root, mut scope) in by_root {
            if scope.contains(&root) {
                scope.clear();
            }
            scope.sort();
            scope.dedup();
            match read_repository(&root, &scope) {
                Ok(repository) => status.repositories.push(repository),
                Err(error) => {
                    tracing::debug!(target: "cdept::vcs", "{}: no git status: {error}", root.display());
                }
            }
        }
        status.reindex();
        status
    }

    /// The status of a project: its folder (the project file and what is next to it)
    /// and every library that is not a reference library.
    pub fn of_project(project: &Project) -> Status {
        Status::read(&project_paths(project))
    }

    fn reindex(&mut self) {
        self.index = self
            .repositories
            .iter()
            .flat_map(|r| &r.changes)
            .map(|c| (c.path.clone(), (c.state, c.folder)))
            .collect();
    }

    /// Nothing is known: no repository, or no git.
    pub fn is_empty(&self) -> bool {
        self.repositories.is_empty()
    }

    /// The repository `path` is in, when it was asked about.
    pub fn repository(&self, path: &Path) -> Option<&Repository> {
        self.repositories
            .iter()
            .filter(|r| r.covers(path))
            .max_by_key(|r| r.root.components().count())
    }

    /// The state an untracked or ignored folder above `path` gives it.
    fn inherited(&self, path: &Path) -> Option<State> {
        path.ancestors()
            .skip(1)
            .find_map(|dir| match self.index.get(dir) {
                Some((state, true)) => Some(*state),
                _ => None,
            })
    }

    /// The state of one file; `None` outside the repositories asked about.
    pub fn file(&self, path: &Path) -> Option<State> {
        self.repository(path)?;
        if let Some((state, _)) = self.index.get(path) {
            return Some(*state);
        }
        Some(self.inherited(path).unwrap_or(State::Clean))
    }

    /// The states of everything in a folder; `None` outside the repositories asked
    /// about.
    pub fn under(&self, dir: &Path) -> Option<Summary> {
        self.repository(dir)?;
        let mut summary = Summary::default();
        match self.index.get(dir).map(|(s, _)| *s).or(self.inherited(dir)) {
            Some(State::Ignored) => summary.ignored = true,
            Some(state) => summary.count(state),
            None => {}
        }
        for (path, (state, _)) in self.index.range(dir.to_path_buf()..) {
            if !path.starts_with(dir) {
                break;
            }
            if path != dir {
                summary.count(*state);
            }
        }
        Some(summary)
    }

    /// The states of the files of a view.
    pub fn entry(&self, entry: &Entry) -> Option<Summary> {
        // A view folder (not one of its own files): everything below it.
        let is_file = entry.files.contains(&entry.path)
            || entry.main_file.as_deref() == Some(entry.path.as_path());
        if !is_file {
            return self.under(&entry.path);
        }
        let mut summary: Option<Summary> = None;
        let mut ignored = true;
        let files = if entry.files.is_empty() {
            std::slice::from_ref(&entry.path)
        } else {
            &entry.files[..]
        };
        for file in files {
            if let Some(state) = self.file(file) {
                let s = summary.get_or_insert_with(Summary::default);
                s.count(state);
                ignored &= state == State::Ignored;
            }
        }
        if let Some(s) = &mut summary {
            s.ignored = ignored;
        }
        summary
    }

    /// The states of the files of a cell: its folder, or (in a flat library) its views.
    pub fn cell(&self, cell: &Cell) -> Option<Summary> {
        if let Some(dir) = &cell.path {
            return self.under(dir);
        }
        let mut summary: Option<Summary> = None;
        let mut ignored = true;
        for entry in &cell.entries {
            if let Some(other) = self.entry(entry) {
                summary.get_or_insert_with(Summary::default).add(other);
                ignored &= other.ignored;
            }
        }
        if let Some(s) = &mut summary {
            s.ignored = ignored && !cell.entries.is_empty();
        }
        summary
    }

    /// The states of the files of a library.
    pub fn library(&self, library: &Library) -> Option<Summary> {
        self.under(&library.path)
    }

    /// How many files are not as committed, over all repositories.
    pub fn dirty(&self) -> usize {
        self.repositories.iter().map(Repository::dirty).sum()
    }
}

/// The folders whose git status a project shows: the project folder (with what is
/// below it) and the libraries that are not reference libraries.
pub fn project_paths(project: &Project) -> Vec<PathBuf> {
    let mut paths = vec![project.root.clone()];
    for library in &project.libraries {
        if library.error.is_none() && !crate::watch::is_reference(project, library) {
            paths.push(library.path.clone());
        }
    }
    paths
}

/// Run `git status` in `root` for the folders of `scope` (all of it when empty).
fn read_repository(root: &Path, scope: &[PathBuf]) -> Result<Repository, String> {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(root)
        .args([
            "--no-optional-locks",
            "status",
            "--porcelain=v2",
            "--branch",
            "-z",
            "--untracked-files=normal",
            "--ignored=traditional",
        ])
        .env("LC_ALL", "C")
        .stdin(std::process::Stdio::null());
    if !scope.is_empty() {
        command.arg("--");
        for path in scope {
            let relative = path.strip_prefix(root).unwrap_or(path);
            command.arg(relative);
        }
    }
    let output = command.output().map_err(|e| format!("git: {e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    let mut repository = parse(root, &output.stdout);
    repository.scope = scope.to_vec();
    Ok(repository)
}

/// Read the output of `git status --porcelain=v2 --branch -z`.
pub fn parse(root: &Path, output: &[u8]) -> Repository {
    let mut repository = Repository {
        root: root.to_path_buf(),
        ..Repository::default()
    };
    let mut records = output.split(|b| *b == 0).filter(|r| !r.is_empty());
    while let Some(record) = records.next() {
        let text = String::from_utf8_lossy(record);
        let (kind, rest) = text.split_once(' ').unwrap_or((&text, ""));
        let mut push = |path: &str, state: State| {
            let folder = path.ends_with('/');
            repository.changes.push(Change {
                path: root.join(path.trim_end_matches('/')),
                state,
                folder,
            });
        };
        match kind {
            "#" => {
                let (key, value) = rest.split_once(' ').unwrap_or((rest, ""));
                match key {
                    "branch.oid" if value != "(initial)" => {
                        repository.commit = Some(value.chars().take(7).collect());
                    }
                    "branch.head" if value != "(detached)" => {
                        repository.branch = Some(value.to_string());
                    }
                    "branch.upstream" => repository.upstream = Some(value.to_string()),
                    "branch.ab" => {
                        for part in value.split(' ') {
                            if let Some(n) = part.strip_prefix('+') {
                                repository.ahead = n.parse().unwrap_or(0);
                            } else if let Some(n) = part.strip_prefix('-') {
                                repository.behind = n.parse().unwrap_or(0);
                            }
                        }
                    }
                    _ => {}
                }
            }
            // 1 XY sub mH mI mW hH hI path
            "1" => {
                let mut fields = rest.splitn(8, ' ');
                let xy = fields.next().unwrap_or("..");
                if let Some(path) = fields.nth(6) {
                    push(path, tracked_state(xy));
                }
            }
            // 2 XY sub mH mI mW hH hI Xscore path, then the old path as a record.
            "2" => {
                let mut fields = rest.splitn(9, ' ');
                let xy = fields.next().unwrap_or("..");
                if let Some(path) = fields.nth(7) {
                    push(path, tracked_state(xy));
                }
                records.next();
            }
            // u XY sub m1 m2 m3 mW h1 h2 h3 path
            "u" => {
                if let Some(path) = rest.splitn(10, ' ').nth(9) {
                    push(path, State::Conflicted);
                }
            }
            "?" => push(rest, State::Untracked),
            "!" => push(rest, State::Ignored),
            _ => {}
        }
    }
    repository.changes.sort_by(|a, b| a.path.cmp(&b.path));
    repository
}

/// The state of a tracked file from its `XY` pair (index, work tree).
fn tracked_state(xy: &str) -> State {
    let mut chars = xy.chars();
    let (index, tree) = (chars.next().unwrap_or('.'), chars.next().unwrap_or('.'));
    if tree != '.' {
        State::Modified
    } else if index == 'A' {
        State::Added
    } else {
        State::Staged
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(root: &str, records: &[&str]) -> Status {
        let mut output = Vec::new();
        for record in records {
            output.extend_from_slice(record.as_bytes());
            output.push(0);
        }
        let mut status = Status {
            repositories: vec![parse(Path::new(root), &output)],
            ..Status::default()
        };
        status.reindex();
        status
    }

    const H: &str = "N... 100644 100644 100644 1111111 2222222";

    #[test]
    fn the_porcelain_output_is_read() {
        let s = status(
            "/r",
            &[
                "# branch.oid 0123456789abcdef",
                "# branch.head main",
                "# branch.upstream origin/main",
                "# branch.ab +2 -1",
                &format!("1 .M {H} libs/a/inv/schematic/inv.sch"),
                &format!("1 M. {H} libs/a/inv/symbol/inv.sym"),
                &format!("1 A. {H} libs/a/buf/schematic/buf.sch"),
                &format!("1 .D {H} libs/a/old/schematic/old.sch"),
                &format!("2 R. {H} R100 libs/a/new name.sch"),
                "libs/a/old name.sch",
                "u UU N... 100644 100644 100644 100644 1 2 3 libs/a/clash/layout/clash.gds",
                "? libs/a/fresh/",
                "? notes with spaces.txt",
                "! simulations/",
            ],
        );
        let r = &s.repositories[0];
        assert_eq!(r.branch.as_deref(), Some("main"));
        assert_eq!(r.commit.as_deref(), Some("0123456"));
        assert_eq!((r.ahead, r.behind), (2, 1));
        assert_eq!(r.head(), "main ↑2 ↓1");
        assert_eq!(r.changes.len(), 9);
        assert_eq!(r.dirty(), 8);

        let file = |p: &str| s.file(Path::new(p));
        assert_eq!(
            file("/r/libs/a/inv/schematic/inv.sch"),
            Some(State::Modified)
        );
        assert_eq!(file("/r/libs/a/inv/symbol/inv.sym"), Some(State::Staged));
        assert_eq!(file("/r/libs/a/buf/schematic/buf.sch"), Some(State::Added));
        assert_eq!(file("/r/libs/a/new name.sch"), Some(State::Staged));
        assert_eq!(
            file("/r/libs/a/clash/layout/clash.gds"),
            Some(State::Conflicted)
        );
        assert_eq!(file("/r/notes with spaces.txt"), Some(State::Untracked));
        // Everything below an untracked or ignored folder is that.
        assert_eq!(
            file("/r/libs/a/fresh/schematic/fresh.sch"),
            Some(State::Untracked)
        );
        assert_eq!(file("/r/simulations/x.raw"), Some(State::Ignored));
        assert_eq!(file("/r/libs/a/other/x.sch"), Some(State::Clean));
        assert_eq!(file("/elsewhere/x.sch"), None);

        let under = |p: &str| s.under(Path::new(p)).unwrap();
        let inv = under("/r/libs/a/inv");
        assert_eq!((inv.modified, inv.staged), (1, 1));
        assert_eq!(inv.state(), State::Modified);
        assert_eq!(inv.describe(), "1 modified, 1 staged");
        // A deleted file still counts for its cell.
        assert_eq!(under("/r/libs/a/old").state(), State::Modified);
        assert_eq!(under("/r/libs/a/fresh").state(), State::Untracked);
        assert_eq!(under("/r/libs/a/fresh/schematic").state(), State::Untracked);
        assert_eq!(under("/r/simulations").state(), State::Ignored);
        assert_eq!(under("/r/libs/a/other").state(), State::Clean);
        assert_eq!(under("/r/libs/a").changed(), 7);
        assert_eq!(under("/r/libs/a").state(), State::Conflicted);
        // A name that only starts the same is another cell.
        assert_eq!(under("/r/libs/a/in").state(), State::Clean);
    }

    #[test]
    fn a_new_repository_has_no_commit_and_a_detached_head_no_branch() {
        let s = status("/r", &["# branch.oid (initial)", "# branch.head main"]);
        assert_eq!(s.repositories[0].commit, None);
        assert_eq!(s.repositories[0].head(), "main");
        let s = status(
            "/r",
            &["# branch.oid abcdef0123", "# branch.head (detached)"],
        );
        assert_eq!(s.repositories[0].head(), "detached at abcdef0");
    }

    #[test]
    fn states_order_by_urgency() {
        assert!(State::Conflicted > State::Modified);
        assert!(State::Modified > State::Untracked);
        assert!(State::Untracked > State::Added);
        assert!(State::Added > State::Staged);
        assert!(State::Staged > State::Ignored);
        assert!(State::Ignored > State::Clean);
        assert!(State::Modified.is_tracked() && !State::Untracked.is_tracked());
        let mut summary = Summary::default();
        assert_eq!(summary.state(), State::Clean);
        summary.ignored = true;
        assert_eq!(summary.state(), State::Ignored);
        summary.count(State::Staged);
        assert_eq!(summary.state(), State::Staged);
    }
}
