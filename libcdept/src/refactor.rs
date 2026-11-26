//! Renaming, copying and moving cells, views and libraries, with what refers to them
//! following.
//!
//! Every operation is planned first ([`plan`]): which folders and files move or are
//! copied, and which files are rewritten and how. The [`Plan`] can be shown (a dry
//! run) and then applied ([`apply`]). Applying keeps a journal; when a step fails,
//! what was done is put back.
//!
//! What follows a moved cell, view or library folder (found with
//! [`crate::hierarchy::Index`]):
//!
//! - xschem: the symbol references of the schematics that place it, and the
//!   `schematic=` attribute of symbols (written, changed or removed so that xschem
//!   still finds the schematic);
//! - layouts: the sidecar entries of the views that place it (and the structure name
//!   in the file when it was the cell name), a layout's own sidecar identity and top
//!   structure, the schematic it is linked with, the linked instances, and the
//!   `LIB=`/`PCELL=` records of placed PCell views (GDSII);
//! - netlists and HDL: `.subckt`/`.ends`/`module` names in the views of a renamed
//!   cell and the instances of them in other views.
//!
//! Read-only libraries are never changed: their views that use the moved thing are
//! listed as not followed. Files the caller says are open with unsaved changes
//! ([`Options::busy`]) block the operation: an editor must not have its file rewritten
//! behind it.
//!
//! A copy rewrites only the copies: references among the copied files point at the
//! copies, everything else is left alone.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};

use eyre::{Context, Result, bail, eyre};

use crate::hierarchy::{
    self, GDS_ENDEL, GDS_ENDSTR, GDS_PROPVALUE, GDS_SREF, GDS_STRNAME, Index, RefKind, Reference,
    SpiceKind, VerilogKind, ViewId,
};
use crate::layout::{self, gds::CONTEXT_INFO, meta};
use crate::library::validate_name;
use crate::xschem::symbol::{self, Link};
use crate::{Cell, Entry, Format, Library, LibraryLayout, Project};

/// What to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operation {
    RenameCell {
        library: String,
        cell: String,
        new: String,
    },
    /// Move a cell into another library, possibly under another name.
    MoveCell {
        library: String,
        cell: String,
        to_library: String,
        new: String,
    },
    CopyCell {
        library: String,
        cell: String,
        to_library: String,
        new: String,
        /// Also copy the cells of the same library it is built from (into another
        /// library, under their names).
        hierarchy: bool,
    },
    DeleteCell {
        library: String,
        cell: String,
    },
    RenameView {
        library: String,
        cell: String,
        view: String,
        new: String,
    },
    CopyView {
        library: String,
        cell: String,
        view: String,
        to_library: String,
        to_cell: String,
        new: String,
    },
    DeleteView {
        library: String,
        cell: String,
        view: String,
    },
    /// Rename a library in the project file; with `folder` also its folder, which is
    /// the name xschem knows it by.
    RenameLibrary {
        library: String,
        new: String,
        folder: bool,
    },
    /// Files from outside the project come into a library as cells and views; the
    /// plan is made by [`crate::importer`].
    Import {
        library: String,
        summary: String,
    },
}

impl fmt::Display for Operation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Operation::RenameCell { library, cell, new } => {
                write!(f, "rename cell {library}/{cell} to {new}")
            }
            Operation::MoveCell {
                library,
                cell,
                to_library,
                new,
            } => write!(f, "move cell {library}/{cell} to {to_library}/{new}"),
            Operation::CopyCell {
                library,
                cell,
                to_library,
                new,
                hierarchy,
            } => write!(
                f,
                "copy cell {library}/{cell} to {to_library}/{new}{}",
                if *hierarchy {
                    " with the cells below it"
                } else {
                    ""
                }
            ),
            Operation::DeleteCell { library, cell } => write!(f, "delete cell {library}/{cell}"),
            Operation::RenameView {
                library,
                cell,
                view,
                new,
            } => write!(f, "rename view {library}/{cell}/{view} to {new}"),
            Operation::CopyView {
                library,
                cell,
                view,
                to_library,
                to_cell,
                new,
            } => write!(
                f,
                "copy view {library}/{cell}/{view} to {to_library}/{to_cell}/{new}"
            ),
            Operation::DeleteView {
                library,
                cell,
                view,
            } => write!(f, "delete view {library}/{cell}/{view}"),
            Operation::RenameLibrary {
                library,
                new,
                folder,
            } => write!(
                f,
                "rename library {library} to {new}{}",
                if *folder { " (with its folder)" } else { "" }
            ),
            Operation::Import { library, summary } => write!(f, "import into {library}: {summary}"),
        }
    }
}

/// What the caller knows that the files do not say.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Options {
    /// Files open in an editor with unsaved changes. A plan that would move or
    /// rewrite one of them is blocked.
    pub busy: BTreeSet<PathBuf>,
    /// Move or copy the files only; leave every reference as it is.
    pub keep_references: bool,
}

/// One line of a plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub kind: ChangeKind,
    /// The file or folder, as it is before the operation.
    pub path: PathBuf,
    /// Where it is afterwards, when that differs.
    pub to: Option<PathBuf>,
    /// `library/cell/view` of a rewritten file, when it is a view of the project.
    pub label: String,
    /// What changes in it, one sentence each.
    pub details: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Move,
    Copy,
    /// The file's contents change.
    Rewrite,
    Delete,
    /// A view that uses the thing and cannot follow (read-only library, a format
    /// that cannot be rewritten): it will be wrong afterwards.
    NotFollowed,
}

impl ChangeKind {
    pub fn name(self) -> &'static str {
        match self {
            ChangeKind::Move => "move",
            ChangeKind::Copy => "copy",
            ChangeKind::Rewrite => "rewrite",
            ChangeKind::Delete => "delete",
            ChangeKind::NotFollowed => "not followed",
        }
    }
}

/// What an operation will do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub operation: Operation,
    /// File system steps, then rewrites, then what cannot follow.
    pub changes: Vec<Change>,
    /// Why the plan cannot be applied as things are (files with unsaved changes).
    pub blockers: Vec<String>,
    /// Things worth knowing that do not stop it.
    pub notes: Vec<String>,
    /// `library`, `cell`, `view` (empty where there is none) of the result, for the
    /// caller to select.
    pub result: ViewId,
    steps: Vec<Step>,
    writes: Vec<Write>,
    delete: Option<(String, String, Option<String>)>,
    /// The project file changes: load the project again afterwards.
    pub reload: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Step {
    MakeDir(PathBuf),
    Move {
        from: PathBuf,
        to: PathBuf,
    },
    Copy {
        from: PathBuf,
        to: PathBuf,
    },
    /// Remove a folder that holds nothing but empty folders.
    RemoveEmpty(PathBuf),
    /// A new file made from one cell of a file that holds several: `bytes` are written
    /// to `to`; `from` is the file the cell is taken from and `what` says how.
    Extract {
        from: PathBuf,
        to: PathBuf,
        bytes: Vec<u8>,
        what: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Write {
    /// Where the file is after the steps.
    file: PathBuf,
    bytes: Vec<u8>,
}

impl Plan {
    /// A plan made outside this module, from file system steps (see [`crate::importer`]).
    pub(crate) fn from_steps(
        operation: Operation,
        changes: Vec<Change>,
        notes: Vec<String>,
        result: ViewId,
        steps: Vec<Step>,
    ) -> Plan {
        Plan {
            operation,
            changes,
            blockers: Vec::new(),
            notes,
            result,
            steps,
            writes: Vec::new(),
            delete: None,
            reload: false,
        }
    }

    /// The plan as text, for a dry run. Paths are shown relative to `root`.
    pub fn lines(&self, root: &Path) -> Vec<String> {
        let show = |p: &Path| p.strip_prefix(root).unwrap_or(p).display().to_string();
        let mut out = vec![self.operation.to_string()];
        for change in &self.changes {
            let mut line = format!("  {} {}", change.kind.name(), show(&change.path));
            if let Some(to) = &change.to {
                line.push_str(&format!(" -> {}", show(to)));
            }
            if !change.label.is_empty() {
                line.push_str(&format!("  [{}]", change.label));
            }
            out.push(line);
            for detail in &change.details {
                out.push(format!("      {detail}"));
            }
        }
        for note in &self.notes {
            out.push(format!("  note: {note}"));
        }
        for blocker in &self.blockers {
            out.push(format!("  BLOCKED: {blocker}"));
        }
        out
    }

    /// The files whose contents change, where they are afterwards.
    pub fn rewritten(&self) -> Vec<&Path> {
        self.writes.iter().map(|w| w.file.as_path()).collect()
    }

    /// Files and folders that move: `(before, after)`.
    pub fn moved(&self) -> Vec<(&Path, &Path)> {
        self.steps
            .iter()
            .filter_map(|s| match s {
                Step::Move { from, to } => Some((from.as_path(), to.as_path())),
                _ => None,
            })
            .collect()
    }

    /// Views that use the thing and will not follow.
    pub fn not_followed(&self) -> Vec<&Change> {
        self.changes
            .iter()
            .filter(|c| c.kind == ChangeKind::NotFollowed)
            .collect()
    }
}

// ---------------------------------------------------------------------------------
// Where things go
// ---------------------------------------------------------------------------------

/// Old path → new path: single files, and whole folders.
#[derive(Debug, Clone, Default)]
struct PathMap {
    files: BTreeMap<PathBuf, PathBuf>,
    folders: Vec<(PathBuf, PathBuf)>,
}

impl PathMap {
    fn get(&self, path: &Path) -> Option<PathBuf> {
        if let Some(to) = self.files.get(path) {
            return Some(to.clone());
        }
        self.folders
            .iter()
            .filter_map(|(from, to)| Some((from, to.join(path.strip_prefix(from).ok()?))))
            .max_by_key(|(from, _)| from.as_os_str().len())
            .map(|(_, to)| to)
    }

    fn after(&self, path: &Path) -> PathBuf {
        self.get(path).unwrap_or_else(|| path.to_path_buf())
    }
}

/// Old `library/cell/view` → new.
#[derive(Debug, Clone, Default)]
struct IdMap {
    views: Vec<(ViewId, ViewId)>,
    cells: Vec<((String, String), (String, String))>,
    library: Option<(String, String)>,
}

impl IdMap {
    fn cell(&self, library: &str, cell: &str) -> Option<(String, String)> {
        if let Some((_, to)) = self
            .cells
            .iter()
            .find(|((l, c), _)| l == library && c == cell)
        {
            return Some(to.clone());
        }
        match &self.library {
            Some((old, new)) if old == library => Some((new.clone(), cell.to_string())),
            _ => None,
        }
    }

    fn view(&self, id: &ViewId) -> Option<ViewId> {
        if let Some((_, to)) = self.views.iter().find(|(from, _)| from == id) {
            return Some(to.clone());
        }
        let (library, cell) = self.cell(&id.library, &id.cell)?;
        Some(ViewId::new(&library, &cell, &id.view))
    }

    /// The name a cell gets, when it changes.
    fn renamed(&self, library: &str, cell: &str) -> Option<String> {
        self.cell(library, cell)
            .map(|(_, new)| new)
            .filter(|new| new != cell)
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The new name of a file named after cell `old`: `old.sch` → `new.sch`.
fn renamed(file_name: &str, old: &str, new: &str) -> Option<String> {
    let rest = file_name.strip_prefix(old)?;
    (rest.is_empty() || rest.starts_with('.')).then(|| format!("{new}{rest}"))
}

/// Visible items of a folder, sorted.
fn list(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut items = Vec::new();
    for item in fs::read_dir(dir).wrap_err_with(|| format!("cannot read {}", dir.display()))? {
        items.push(item?.path());
    }
    items.sort();
    Ok(items)
}

/// Every file below a folder, with its path relative to the folder.
fn walk(dir: &Path) -> Result<Vec<(PathBuf, PathBuf)>> {
    fn inner(dir: &Path, rel: &Path, out: &mut Vec<(PathBuf, PathBuf)>) -> Result<()> {
        for path in list(dir)? {
            let rel = rel.join(file_name(&path));
            if path.is_dir() {
                inner(&path, &rel, out)?;
            } else {
                out.push((path, rel));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    inner(dir, Path::new(""), &mut out)?;
    Ok(out)
}

/// The files of a view that sits in a flat library or loose in a cell folder, with
/// the sidecars of its layouts.
fn entry_files(entry: &Entry) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for file in &entry.files {
        out.push(file.clone());
        let sidecar = meta::sidecar_path(file);
        if sidecar.is_file() {
            out.push(sidecar);
        }
    }
    out
}

/// Where the files of a cell or view go, and the steps that take them there.
#[derive(Debug, Default)]
struct Transfer {
    steps: Vec<Step>,
    map: PathMap,
    notes: Vec<String>,
}

impl Transfer {
    fn item(&mut self, from: &Path, to: PathBuf, copy: bool) -> Result<()> {
        if to.exists() || self.map.files.values().any(|t| *t == to) {
            bail!("{} already exists", to.display());
        }
        self.steps.push(if copy {
            Step::Copy {
                from: from.to_path_buf(),
                to: to.clone(),
            }
        } else {
            Step::Move {
                from: from.to_path_buf(),
                to: to.clone(),
            }
        });
        self.map.files.insert(from.to_path_buf(), to);
        Ok(())
    }

    /// The view `entry`, one cell (`cell` of its library) of a file that holds several,
    /// is copied as a file of its own named `new`, made in the folder `dir` (made
    /// first when `make_dir`). Only layouts in GDSII and SPICE/CDL netlists can be
    /// taken apart: any other view is left out, with a note.
    fn member(
        &mut self,
        entry: &Entry,
        cell: &str,
        new: &str,
        dir: &Path,
        make_dir: bool,
    ) -> Result<()> {
        let file = entry.main_file.as_deref().unwrap_or(&entry.path);
        let inside = entry.member_or(cell);
        let name = file_name(file);
        let suffix = Format::split_name(&name)
            .map(|(stem, _)| name[stem.len()..].to_string())
            .unwrap_or_default();
        let (bytes, what) = match entry.format {
            Some(Format::Gds) => {
                let bytes =
                    fs::read(file).wrap_err_with(|| format!("cannot read {}", file.display()))?;
                if bytes.starts_with(&[0x1f, 0x8b]) {
                    self.notes.push(format!(
                        "view '{}' is in {name}, a compressed file: not copied",
                        entry.name
                    ));
                    return Ok(());
                }
                let mut out =
                    layout::gds::extract_structure(&bytes, inside).wrap_err_with(|| {
                        format!("cannot take '{inside}' out of {}", file.display())
                    })?;
                if inside != new {
                    let map = BTreeMap::from([(inside.to_string(), new.to_string())]);
                    if let Some(renamed) = layout::gds::rename_bytes(&out, &map)
                        .wrap_err_with(|| format!("cannot name the structure '{new}'"))?
                    {
                        out = renamed;
                    }
                }
                (
                    out,
                    format!("structure '{inside}' and what it places, as '{new}'"),
                )
            }
            Some(Format::Spice | Format::Cdl) => {
                let text = String::from_utf8_lossy(
                    &fs::read(file).wrap_err_with(|| format!("cannot read {}", file.display()))?,
                )
                .into_owned();
                let out = crate::container::spice::extract_subckt(&text, inside, new)
                    .wrap_err_with(|| {
                        format!("cannot take '{inside}' out of {}", file.display())
                    })?;
                (
                    out.into_bytes(),
                    format!("subcircuit '{inside}' and those it places, as '{new}'"),
                )
            }
            _ => {
                self.notes.push(format!(
                    "view '{}' is one cell of {name}, a file of {}: only GDSII layouts and \
                     SPICE/CDL netlists can be copied out of such a file, so it is not copied",
                    entry.name,
                    entry.format.map_or("another kind", Format::label)
                ));
                return Ok(());
            }
        };
        let to = dir.join(format!("{new}{suffix}"));
        if to.exists()
            || self.map.files.values().any(|t| *t == to)
            || self
                .steps
                .iter()
                .any(|s| matches!(s, Step::Extract { to: t, .. } if *t == to))
        {
            bail!("{} already exists", to.display());
        }
        if make_dir {
            self.steps.push(Step::MakeDir(dir.to_path_buf()));
        }
        self.steps.push(Step::Extract {
            from: file.to_path_buf(),
            to,
            bytes,
            what,
        });
        Ok(())
    }

    /// A whole folder goes to `to`; the files named after `old` in it (and in its
    /// folders, down to `depth`) are then renamed after `new`.
    fn folder(
        &mut self,
        from: &Path,
        to: &Path,
        old: &str,
        new: &str,
        depth: usize,
        copy: bool,
    ) -> Result<()> {
        if to.exists() {
            bail!("{} already exists", to.display());
        }
        self.steps.push(if copy {
            Step::Copy {
                from: from.to_path_buf(),
                to: to.to_path_buf(),
            }
        } else {
            Step::Move {
                from: from.to_path_buf(),
                to: to.to_path_buf(),
            }
        });
        self.map
            .folders
            .push((from.to_path_buf(), to.to_path_buf()));
        if old == new {
            return Ok(());
        }
        for (file, rel) in walk(from)? {
            if rel.components().count() > depth {
                continue;
            }
            if let Some(name) = renamed(&file_name(&file), old, new) {
                let moved = to.join(&rel);
                let target = moved.with_file_name(name);
                self.steps.push(Step::Move {
                    from: moved,
                    to: target.clone(),
                });
                self.map.files.insert(file, target);
            }
        }
        Ok(())
    }
}

/// Move or copy a cell: `cell` of `src` becomes `new` in `dst`.
fn transfer_cell(cell: &Cell, dst: &Library, new: &str, copy: bool) -> Result<Transfer> {
    let mut t = Transfer::default();
    let old = cell.name.as_str();
    if dst.layout == LibraryLayout::Cells {
        let target = dst.path.join(new);
        if target.exists() {
            bail!("'{new}' already exists in library '{}'", dst.name);
        }
        match &cell.path {
            // A cell folder: it goes as it is; files named after the cell, in it and
            // in its view folders, are renamed.
            Some(dir) => t.folder(dir, &target, old, new, 2, copy)?,
            // From a flat library: each view gets its folder.
            None => {
                for entry in &cell.entries {
                    let dir = target.join(&entry.name);
                    if entry.is_member() {
                        t.member(entry, old, new, &dir, true)?;
                        continue;
                    }
                    t.steps.push(Step::MakeDir(dir.clone()));
                    for file in entry_files(entry) {
                        let name = file_name(&file);
                        let name = renamed(&name, old, new).unwrap_or(name);
                        t.item(&file, dir.join(name), copy)?;
                    }
                }
            }
        }
    } else if dst.layout == LibraryLayout::Flat {
        // Only files named after the cell have a place in a flat library.
        let files: Vec<PathBuf> = match &cell.path {
            Some(dir) => walk(dir)?
                .into_iter()
                .filter(|(_, rel)| rel.components().count() <= 2)
                .map(|(file, _)| file)
                .collect(),
            None => {
                let mut files = Vec::new();
                for entry in &cell.entries {
                    if entry.is_member() {
                        t.member(entry, old, new, &dst.path, false)?;
                    } else {
                        files.extend(entry_files(entry));
                    }
                }
                files
            }
        };
        let mut left = Vec::new();
        for file in files {
            match renamed(&file_name(&file), old, new) {
                Some(name) => t.item(&file, dst.path.join(name), copy)?,
                None => left.push(file),
            }
        }
        if let Some(dir) = &cell.path {
            let all = walk(dir)?.len();
            if all != t.map.files.len() && !copy {
                bail!(
                    "cell '{old}' has files that are not named after it: they have no \
                     place in the flat library '{}' (copy the cell instead)",
                    dst.name
                );
            }
            if !copy {
                t.steps.push(Step::RemoveEmpty(dir.clone()));
            }
        }
        if !left.is_empty() {
            t.notes.push(format!(
                "{} file(s) not named after the cell are not copied: a flat library has \
                 no place for them",
                left.len()
            ));
        }
        if t.map.files.is_empty() && t.steps.is_empty() {
            bail!("cell '{old}' has no file named after it to put in a flat library");
        }
    } else {
        bail!(
            "library '{}' is laid out as '{}': cells cannot be put there",
            dst.name,
            dst.layout.name()
        );
    }
    if t.steps.is_empty() && cell.entries.iter().any(Entry::is_member) {
        bail!(
            "no view of '{old}' can be copied out of the files that hold it: {}",
            t.notes.join("; ")
        );
    }
    Ok(t)
}

// ---------------------------------------------------------------------------------
// Rewriting files
// ---------------------------------------------------------------------------------

/// Why a file that should follow does not.
#[derive(Debug, Clone, PartialEq, Eq)]
struct NotFollowed {
    file: PathBuf,
    label: String,
    reason: String,
}

/// The files being rewritten, in memory until the plan is applied.
struct Work<'a> {
    project: &'a Project,
    index: &'a Index,
    options: &'a Options,
    map: PathMap,
    ids: IdMap,
    /// A copy: only the copies are rewritten, at their new place.
    copy: bool,
    search: Vec<PathBuf>,
    /// New contents, by the file's path before the operation.
    contents: BTreeMap<PathBuf, Vec<u8>>,
    details: BTreeMap<PathBuf, Vec<String>>,
    not_followed: Vec<NotFollowed>,
    blockers: BTreeSet<String>,
}

impl Work<'_> {
    fn label(&self, file: &Path) -> String {
        let base = match file.to_str().and_then(|s| s.strip_suffix(".json")) {
            Some(layout) if meta::is_sidecar_name(&file_name(file)) => Path::new(layout),
            _ => file,
        };
        self.index
            .view_of(base)
            .map(ViewId::label)
            .unwrap_or_default()
    }

    fn library_of(&self, file: &Path) -> Option<&Library> {
        self.project
            .libraries
            .iter()
            .filter(|l| l.error.is_none() && file.starts_with(&l.path))
            .max_by_key(|l| l.path.as_os_str().len())
    }

    /// May `file` be rewritten? Says why not otherwise (once per file).
    fn may_rewrite(&mut self, file: &Path, what: &str) -> bool {
        if self.copy && self.map.get(file).is_none() {
            return false;
        }
        // A copy leaves its source alone, so a read-only or busy source is fine.
        if self.copy {
            return true;
        }
        if let Some(lib) = self.library_of(file)
            && lib.read_only
        {
            let reason = format!("library '{}' is read-only: {what}", lib.name);
            self.skip(file, reason);
            return false;
        }
        if self.options.busy.contains(file) {
            self.blockers.insert(format!(
                "{} has unsaved changes: save or close it first",
                file.display()
            ));
            return false;
        }
        true
    }

    fn skip(&mut self, file: &Path, reason: String) {
        let item = NotFollowed {
            file: file.to_path_buf(),
            label: self.label(file),
            reason,
        };
        if !self.not_followed.contains(&item) {
            self.not_followed.push(item);
        }
    }

    fn bytes(&self, file: &Path) -> Result<Vec<u8>> {
        match self.contents.get(file) {
            Some(bytes) => Ok(bytes.clone()),
            None => fs::read(file).wrap_err_with(|| format!("cannot read {}", file.display())),
        }
    }

    /// The text of a file as it stands (with the changes so far); `None`, with the
    /// file listed as not followed, when it cannot be read as UTF-8.
    fn text(&mut self, file: &Path) -> Option<String> {
        match self.bytes(file).map(String::from_utf8) {
            Ok(Ok(text)) => Some(text),
            Ok(Err(_)) => {
                self.skip(file, "the file is not UTF-8 text".into());
                None
            }
            Err(e) => {
                self.skip(file, format!("{e:#}"));
                None
            }
        }
    }

    fn put(&mut self, file: &Path, bytes: Vec<u8>, detail: String) {
        self.contents.insert(file.to_path_buf(), bytes);
        let details = self.details.entry(file.to_path_buf()).or_default();
        if !details.contains(&detail) {
            details.push(detail);
        }
    }

    /// The folder of a library after the operation.
    fn library_after(&self, library: &Library) -> PathBuf {
        self.map.after(&library.path)
    }

    /// How xschem refers to a file after the operation: its path below the parent of
    /// its library folder. `None` for a file outside the libraries.
    fn reference_parts(&self, file_after: &Path) -> Option<Vec<String>> {
        let lib = self
            .project
            .libraries
            .iter()
            .filter(|l| l.error.is_none())
            .map(|l| self.library_after(l))
            .filter(|path| file_after.starts_with(path))
            .max_by_key(|path| path.as_os_str().len())?;
        let relative = file_after.strip_prefix(lib.parent()?).ok()?;
        Some(
            relative
                .components()
                .filter_map(|c| match c {
                    Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
                    _ => None,
                })
                .collect(),
        )
    }

    /// The reference to write in place of `old`, for a target now at `file_after`.
    fn reference_text(&self, old: &str, file_after: &Path) -> String {
        if Path::new(old).is_absolute() {
            return file_after.to_string_lossy().into_owned();
        }
        match self.reference_parts(file_after) {
            Some(parts) => parts.join("/"),
            None => file_after.to_string_lossy().into_owned(),
        }
    }

    /// The file an xschem reference names today.
    fn resolve_xschem(&self, reference: &str) -> Option<PathBuf> {
        let path = Path::new(reference);
        if path.is_absolute() {
            return Some(path.to_path_buf());
        }
        self.search
            .iter()
            .map(|dir| dir.join(path))
            .find(|c| self.index.view_of(c).is_some() || c.is_file())
    }

    // -- xschem ---------------------------------------------------------------

    /// The symbol references of the schematics that place something that moves.
    fn follow_instances(&mut self) {
        let mut by_file: BTreeMap<PathBuf, BTreeMap<String, String>> = BTreeMap::new();
        for r in self.index.references.iter() {
            if r.kind != RefKind::Instance {
                continue;
            }
            let Some(to) = r.target.file().and_then(|f| self.map.get(f)) else {
                continue;
            };
            let new = self.reference_text(&r.text, &to);
            if new != r.text {
                by_file
                    .entry(r.file.clone())
                    .or_default()
                    .insert(r.text.clone(), new);
            }
        }
        for (file, renames) in by_file {
            if !self.may_rewrite(&file, "its instances keep the old reference") {
                continue;
            }
            let Some(text) = self.text(&file) else {
                continue;
            };
            let mut out = String::with_capacity(text.len() + 64);
            let mut last = 0;
            let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
            for range in hierarchy::instance_ranges(&text) {
                let written = symbol::unescape(&text[range.clone()]);
                if let Some((old, new)) = renames.get_key_value(&written) {
                    out.push_str(&text[last..range.start]);
                    out.push_str(&escape(new));
                    last = range.end;
                    *counts.entry(old.as_str()).or_default() += 1;
                }
            }
            if last == 0 {
                continue;
            }
            out.push_str(&text[last..]);
            for (old, n) in counts {
                let detail = format!("{n} instance(s) of {old} -> {}", renames[old]);
                self.put(&file, out.clone().into_bytes(), detail);
            }
        }
    }

    /// The value of `schematic=` for a symbol named `stem` whose schematic is at
    /// `schematic` after the operation.
    fn link_value(&self, schematic: &Path, stem: &str) -> String {
        let Some(parts) = self.reference_parts(schematic) else {
            return schematic.to_string_lossy().into_owned();
        };
        match parts.as_slice() {
            // `<library folder>/<cell>/<view>/<file>`: generic when the symbol is
            // named after the cell.
            [folder, cell, inside @ ..] if !inside.is_empty() => {
                symbol::reference(folder, cell, inside, stem)
            }
            [folder, file] => match file.strip_prefix(stem) {
                Some(rest) if rest.starts_with('.') => format!("{folder}/@symname{rest}"),
                _ => format!("{folder}/{file}"),
            },
            _ => parts.join("/"),
        }
    }

    /// The `schematic=` attribute of symbols: after the operation xschem must find the
    /// same schematic (or its copy).
    fn follow_links(&mut self) {
        let links: Vec<Reference> = self
            .index
            .references
            .iter()
            .filter(|r| r.kind == RefKind::SchematicLink)
            .cloned()
            .collect();
        for r in links {
            let Some(target) = r.target.file() else {
                continue;
            };
            let symbol_to = self.map.get(&r.file);
            let target_to = self.map.get(target);
            if symbol_to.is_none() && target_to.is_none() {
                continue;
            }
            // A copy whose schematic is not copied keeps pointing at the original.
            if self.copy && target_to.is_none() {
                continue;
            }
            let symbol_after = symbol_to.unwrap_or_else(|| r.file.clone());
            let target_after = target_to.unwrap_or_else(|| target.to_path_buf());
            let stem = hierarchy::stem(&symbol_after);
            let wanted = if target_after == symbol_after.with_extension("sch") {
                None
            } else {
                Some(self.link_value(&target_after, &stem))
            };
            let Ok(text) = fs::read_to_string(&r.file) else {
                continue;
            };
            // What the attribute as it is would mean for the symbol at its new place.
            let stands = match symbol::link(&text, &stem) {
                Link::Default => None,
                Link::Path(path) => Some(path),
                Link::Dynamic(_) => continue,
            };
            if stands == wanted.as_ref().map(|w| w.replace("@symname", &stem)) {
                continue;
            }
            if !self.may_rewrite(&r.file, "its schematic= attribute keeps the old path") {
                continue;
            }
            let Some(text) = self.text(&r.file) else {
                continue;
            };
            match symbol::set_schematic(&text, wanted.as_deref()) {
                Ok(new) => {
                    let detail = match &wanted {
                        Some(value) => format!("schematic={value}"),
                        None => "schematic= removed (the schematic is next to the symbol)".into(),
                    };
                    self.put(&r.file, new.into_bytes(), detail);
                }
                Err(e) => self.skip(&r.file, format!("schematic= cannot be written: {e}")),
            }
        }
    }

    // -- layouts --------------------------------------------------------------

    /// Layout files and their sidecars: identity, top structure, placed cells,
    /// linked schematic and instances, placed PCell views.
    fn follow_layouts(&mut self) {
        let mut files: Vec<(ViewId, PathBuf, bool)> = Vec::new();
        for lib in self.project.libraries.iter().filter(|l| l.error.is_none()) {
            for cell in &lib.cells {
                for entry in &cell.entries {
                    for (file, format) in hierarchy::design_files(entry) {
                        if matches!(format, Format::Gds | Format::Oasis)
                            && layout::is_readable(&file)
                        {
                            let id = ViewId::new(&lib.name, &cell.name, &entry.name);
                            files.push((id, file, lib.read_only));
                        }
                    }
                }
            }
        }
        for (id, file, read_only) in files {
            if let Err(e) = self.follow_layout(&id, &file, read_only) {
                self.skip(&file, format!("{e:#}"));
            }
        }
    }

    fn follow_layout(&mut self, id: &ViewId, file: &Path, read_only: bool) -> Result<()> {
        let sidecar = meta::sidecar_path(file);
        let refs: Vec<Reference> = self.index.from_file(file).cloned().collect();
        let new_id = self.ids.view(id);
        let moved = new_id.is_some() || self.map.get(file).is_some();
        let placed: Vec<(&Reference, ViewId)> = refs
            .iter()
            .filter(|r| r.kind == RefKind::Placement)
            .filter_map(|r| Some((r, self.ids.view(r.target.view()?)?)))
            .collect();
        let pcells: BTreeMap<(String, String), (String, String)> = refs
            .iter()
            .filter(|r| r.kind == RefKind::Pcell)
            .filter_map(|r| {
                let target = r.target.view()?;
                let to = self.ids.cell(&target.library, &target.cell)?;
                Some(((target.library.clone(), target.cell.clone()), to))
            })
            .collect();
        let old_meta = if sidecar.is_file() {
            Some(meta::Meta::read(&sidecar)?)
        } else {
            None
        };
        // Anything to do here at all?
        let sidecar_names = old_meta.as_ref().is_some_and(|m| {
            m.source
                .as_ref()
                .is_some_and(|s| self.ids.cell(&s.library, &s.cell).is_some())
                || m.instances.values().any(|l| {
                    self.ids.cell(&l.library, &l.cell).is_some()
                        || self
                            .resolve_xschem(&l.symbol)
                            .is_some_and(|f| self.map.get(&f).is_some())
                })
        });
        if !moved && placed.is_empty() && pcells.is_empty() && !sidecar_names {
            return Ok(());
        }
        if read_only && !self.copy {
            if !placed.is_empty() || !pcells.is_empty() {
                let reason = format!(
                    "library '{}' is read-only: it will place a cell that cannot be found",
                    id.library
                );
                self.skip(file, reason);
            }
            return Ok(());
        }
        if !self.may_rewrite(file, "") {
            return Ok(());
        }

        let summary = layout::read_summary(file)?;
        let mut used: BTreeSet<String> =
            summary.structures.iter().map(|s| s.name.clone()).collect();
        used.extend(summary.ghosts());
        let after = new_id.clone().unwrap_or_else(|| id.clone());
        let mut meta = old_meta.clone();
        let mut renames: BTreeMap<String, String> = BTreeMap::new();
        let mut details: Vec<String> = Vec::new();

        // The file's own identity, and its top structure when named after the cell.
        if let Some(new_id) = &new_id {
            let old_top = meta.as_ref().map_or(id.cell.clone(), |m| m.top.clone());
            if new_id.cell != id.cell
                && old_top == id.cell
                && summary.defines(&old_top)
                && !used.contains(&new_id.cell)
            {
                used.insert(new_id.cell.clone());
                renames.insert(old_top.clone(), new_id.cell.clone());
                details.push(format!("top structure {old_top} -> {}", new_id.cell));
            }
            if let Some(m) = &mut meta {
                m.library = new_id.library.clone();
                m.cell = new_id.cell.clone();
                m.view = new_id.view.clone();
                if let Some(top) = renames.get(&m.top) {
                    m.top = top.clone();
                }
                details.push(format!("sidecar: this is {}", new_id.label()));
            }
        }

        // The cells it places.
        if !placed.is_empty() {
            let m = meta.get_or_insert_with(|| {
                let top = match summary.top_cells().as_slice() {
                    [only] => renames.get(only).unwrap_or(only).clone(),
                    _ => after.cell.clone(),
                };
                meta::Meta::new(&after.library, &after.cell, &after.view, &top)
            });
            for (r, to) in &placed {
                let old = r.target.view().expect("placed targets are views");
                let bbox = m.references.remove(&r.text).and_then(|e| e.bbox);
                let mut key = r.text.clone();
                if r.text == old.cell && to.cell != old.cell && !used.contains(&to.cell) {
                    used.insert(to.cell.clone());
                    renames.insert(r.text.clone(), to.cell.clone());
                    key = to.cell.clone();
                    details.push(format!("structure {} -> {}", r.text, to.cell));
                }
                m.references.insert(
                    key,
                    meta::MetaRef {
                        library: to.library.clone(),
                        cell: to.cell.clone(),
                        view: to.view.clone(),
                        bbox,
                    },
                );
                details.push(format!(
                    "sidecar: places {} (was {})",
                    to.label(),
                    old.label()
                ));
            }
        }

        // The schematic it is linked with, and the linked instances.
        if let Some(m) = &mut meta {
            if let Some(source) = &mut m.source {
                let old = ViewId::new(&source.library, &source.cell, &source.view);
                if let Some(to) = self.ids.view(&old) {
                    details.push(format!(
                        "sidecar: linked with {} (was {})",
                        to.label(),
                        old.label()
                    ));
                    source.library = to.library;
                    source.cell = to.cell;
                    source.view = to.view;
                }
            }
            let mut touched = 0;
            for linked in m.instances.values_mut() {
                if let Some((library, cell)) = self.ids.cell(&linked.library, &linked.cell)
                    && (linked.kind == "cell" || self.project.library(&linked.library).is_some())
                {
                    linked.library = library;
                    linked.cell = cell;
                    touched += 1;
                }
                if let Some(to) = self
                    .resolve_xschem(&linked.symbol)
                    .and_then(|f| self.map.get(&f))
                {
                    linked.symbol = self.reference_text(&linked.symbol, &to);
                    touched += 1;
                }
            }
            if touched > 0 {
                details.push("sidecar: linked instances follow".into());
            }
        }

        // Write: the layout file first (structure names, PCell records), then the sidecar.
        let mut bytes = None;
        if !renames.is_empty() {
            let old = self.bytes(file)?;
            bytes = if layout::is_oasis(file) {
                layout::oasis::rename_bytes(&old, &renames)?
            } else {
                layout::gds::rename_bytes(&old, &renames)?
            };
        }
        if !pcells.is_empty() {
            if layout::is_oasis(file) {
                self.skip(
                    file,
                    "PCells placed in an OASIS file cannot be renamed here".into(),
                );
            } else {
                let old = match bytes.take() {
                    Some(b) => b,
                    None => self.bytes(file)?,
                };
                let (new, n) = rewrite_pcell_contexts(&old, &pcells);
                if n > 0 {
                    for ((lib, cell), (to_lib, to_cell)) in &pcells {
                        details.push(format!("PCell {lib}/{cell} -> {to_lib}/{to_cell}"));
                    }
                }
                bytes = Some(new);
            }
        }
        let layout_details: Vec<String> = details
            .iter()
            .filter(|d| !d.starts_with("sidecar:"))
            .cloned()
            .collect();
        if let Some(bytes) = bytes {
            for detail in layout_details {
                self.put(file, bytes.clone(), detail);
            }
        }
        if let Some(m) = meta
            && old_meta.as_ref() != Some(&m)
        {
            let text = m.to_json()?;
            for detail in details.iter().filter_map(|d| d.strip_prefix("sidecar: ")) {
                self.put(&sidecar, text.clone().into_bytes(), detail.to_string());
            }
            if !self.contents.contains_key(&sidecar) {
                self.put(&sidecar, text.into_bytes(), "sidecar written".into());
            }
        }
        Ok(())
    }

    // -- Magic views ----------------------------------------------------------

    /// Magic views (docs/MAGIC.md): the `use` lines placing a moved or renamed cell
    /// name it and its new folder (relative), a moved view's relative `use` paths are
    /// written again for its new folder, and the sidecars follow.
    fn follow_mags(&mut self) {
        let mut files: Vec<(ViewId, PathBuf, bool)> = Vec::new();
        for lib in self.project.libraries.iter().filter(|l| l.error.is_none()) {
            for cell in &lib.cells {
                for entry in cell.entries.iter().filter(|e| !e.is_member()) {
                    if entry.format == Some(Format::Magic)
                        && let Some(file) = &entry.main_file
                    {
                        let id = ViewId::new(&lib.name, &cell.name, &entry.name);
                        files.push((id, file.clone(), lib.read_only));
                    }
                }
            }
        }
        for (id, file, read_only) in files {
            if let Err(e) = self.follow_mag(&id, &file, read_only) {
                self.skip(&file, format!("{e:#}"));
            }
        }
    }

    fn follow_mag(&mut self, id: &ViewId, file: &Path, read_only: bool) -> Result<()> {
        use crate::magic::mag;
        let sidecar = meta::sidecar_path(file);
        let new_id = self.ids.view(id);
        let new_file = self.map.get(file);
        let moved = new_id.is_some() || new_file.is_some();
        let resolved = crate::magic::refs::resolve_file(self.project, file)?;
        // (use cell name) -> (new cell name, new folder of its file), for cells that move
        let mut targets: BTreeMap<String, (String, Option<PathBuf>, ViewId)> = BTreeMap::new();
        for r in &resolved {
            if let crate::magic::refs::Target::Project(v) = &r.target {
                let old = ViewId::new(&v.library, &v.cell, &v.view);
                let to = self.ids.view(&old);
                let to_file = self.map.get(&v.file);
                if to.is_some() || to_file.is_some() || moved {
                    let to_id = to.clone().unwrap_or(old.clone());
                    let after = to_file.unwrap_or_else(|| v.file.clone());
                    let name = if to.as_ref().is_some_and(|t| t.cell != v.cell) {
                        to_id.cell.clone()
                    } else {
                        r.placed.cell.clone()
                    };
                    targets.insert(
                        r.placed.cell.clone(),
                        (name, after.parent().map(Path::to_path_buf), to_id),
                    );
                }
            }
        }
        // Relative paths to things outside the project, when this view moves.
        let old_dir = file.parent().unwrap_or(Path::new(".")).to_path_buf();
        if !moved && targets.is_empty() {
            return Ok(());
        }
        if read_only && !self.copy {
            if !targets.is_empty() {
                let reason = format!(
                    "library '{}' is read-only: it will place a cell that cannot be found",
                    id.library
                );
                self.skip(file, reason);
            }
            return Ok(());
        }
        if !self.may_rewrite(file, "") {
            return Ok(());
        }
        let dir_after = new_file
            .as_deref()
            .unwrap_or(file)
            .parent()
            .unwrap_or(Path::new("."))
            .to_path_buf();
        let canon = |p: &Path| -> PathBuf {
            // the folder may not exist yet: canonicalize the longest existing part
            let mut existing = p.to_path_buf();
            let mut rest = Vec::new();
            while !existing.exists() {
                match (existing.file_name(), existing.parent()) {
                    (Some(n), Some(parent)) => {
                        rest.push(n.to_os_string());
                        existing = parent.to_path_buf();
                    }
                    _ => break,
                }
            }
            let mut out = existing.canonicalize().unwrap_or(existing);
            for n in rest.into_iter().rev() {
                out.push(n);
            }
            out
        };
        let dir_after_c = canon(&dir_after);
        let text = match self.text(file) {
            Some(t) => t,
            None => return Ok(()),
        };
        let mut details: Vec<String> = Vec::new();
        let new_text = mag::rewrite_uses(&text, |u| {
            if let Some((name, dir, to)) = targets.get(&u.cell) {
                let path = dir.as_ref().map(|d| {
                    mag::relative_dir(&dir_after_c, &canon(d))
                        .to_string_lossy()
                        .into_owned()
                });
                if name != &u.cell {
                    details.push(format!("use {} -> {name} ({})", u.cell, to.label()));
                }
                return Some(mag::UseEdit {
                    cell: Some(name.clone()),
                    path,
                });
            }
            if moved && !u.path.is_empty() && !u.path.starts_with(['/', '~', '$']) {
                let target = canon(&old_dir.join(&u.path));
                return Some(mag::UseEdit {
                    cell: None,
                    path: Some(
                        mag::relative_dir(&dir_after_c, &target)
                            .to_string_lossy()
                            .into_owned(),
                    ),
                });
            }
            None
        });
        if let Some(new_text) = new_text {
            if details.is_empty() {
                details.push("use paths follow".into());
            }
            for d in &details {
                self.put(file, new_text.clone().into_bytes(), d.clone());
            }
        }
        // The sidecar: identity and references.
        let old_meta = if sidecar.is_file() {
            meta::Meta::read(&sidecar).ok()
        } else {
            None
        };
        let after = new_id.clone().unwrap_or_else(|| id.clone());
        let top = new_file
            .as_deref()
            .unwrap_or(file)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| after.cell.clone());
        let mut m = old_meta
            .clone()
            .unwrap_or_else(|| meta::Meta::new(&after.library, &after.cell, &after.view, &top));
        m.library = after.library.clone();
        m.cell = after.cell.clone();
        m.view = after.view.clone();
        m.top = top;
        for (old_name, (name, _, to)) in &targets {
            let bbox = m.references.remove(old_name).and_then(|r| r.bbox);
            m.references.insert(
                name.clone(),
                meta::MetaRef {
                    library: to.library.clone(),
                    cell: to.cell.clone(),
                    view: to.view.clone(),
                    bbox,
                },
            );
        }
        // A sidecar that does not exist yet is only made for a view that stays where it
        // is (a new one would not be renamed with a moved view); the next save makes it.
        if old_meta.as_ref() != Some(&m) && (old_meta.is_some() || new_file.is_none()) {
            let text = m.to_json()?;
            self.put(
                &sidecar,
                text.into_bytes(),
                format!("sidecar: this is {}", after.label()),
            );
        }
        Ok(())
    }

    // -- netlists and HDL -----------------------------------------------------

    /// `.subckt`, `.ends` and `module` names in the views of renamed cells, and the
    /// instances of them elsewhere.
    fn follow_names(&mut self) {
        // file → (old name → new name), separately for definitions and uses.
        let mut defined: BTreeMap<PathBuf, BTreeMap<String, String>> = BTreeMap::new();
        let mut used: BTreeMap<PathBuf, BTreeMap<String, String>> = BTreeMap::new();
        for lib in self.project.libraries.iter().filter(|l| l.error.is_none()) {
            for cell in &lib.cells {
                let Some(new) = self.ids.renamed(&lib.name, &cell.name) else {
                    continue;
                };
                for entry in &cell.entries {
                    for (file, format) in hierarchy::design_files(entry) {
                        if is_netlist(format) || is_verilog(format) {
                            defined
                                .entry(file)
                                .or_default()
                                .insert(cell.name.clone(), new.clone());
                        }
                    }
                }
            }
        }
        for r in self.index.references.iter() {
            if !matches!(r.kind, RefKind::Subckt | RefKind::Module) {
                continue;
            }
            let Some((library, cell)) = r.target.cell() else {
                continue;
            };
            let Some(new) = self.ids.renamed(library, cell) else {
                continue;
            };
            let same = if r.kind == RefKind::Subckt {
                r.text.eq_ignore_ascii_case(cell)
            } else {
                r.text == cell
            };
            if same {
                used.entry(r.file.clone())
                    .or_default()
                    .insert(cell.to_string(), new);
            }
        }
        let files: BTreeSet<PathBuf> = defined.keys().chain(used.keys()).cloned().collect();
        let none = BTreeMap::new();
        for file in files {
            let defs = defined.get(&file).unwrap_or(&none);
            let uses = used.get(&file).unwrap_or(&none);
            if !self.may_rewrite(&file, "it keeps the old cell name") {
                continue;
            }
            let Some(format) = Format::of_file(&file) else {
                continue;
            };
            let Some(text) = self.text(&file) else {
                continue;
            };
            // (range, new name, what)
            let mut edits: Vec<(std::ops::Range<usize>, &String, &str)> = Vec::new();
            if is_netlist(format) {
                let find = find_name;
                for item in hierarchy::spice_items(&text) {
                    match item.kind {
                        SpiceKind::Subckt | SpiceKind::Ends => {
                            if let Some(new) = find(defs, &item.name) {
                                edits.push((item.range, new, "subcircuit"));
                            }
                        }
                        SpiceKind::Instance => {
                            // An instance of a subcircuit defined in this same file
                            // follows its definition.
                            if let Some(new) = find(uses, &item.name).or(find(defs, &item.name)) {
                                edits.push((item.range, new, "instance"));
                            }
                        }
                    }
                }
            } else {
                for item in hierarchy::verilog_items(&text) {
                    let map = match item.kind {
                        VerilogKind::Module => defs,
                        VerilogKind::Instance => uses,
                    };
                    let what = match item.kind {
                        VerilogKind::Module => "module",
                        VerilogKind::Instance => "instance",
                    };
                    if let Some(new) = map.get(&item.name).or_else(|| {
                        (item.kind == VerilogKind::Instance)
                            .then(|| defs.get(&item.name))
                            .flatten()
                    }) {
                        edits.push((item.range, new, what));
                    }
                }
            }
            if edits.is_empty() {
                continue;
            }
            let mut out = String::with_capacity(text.len() + 16);
            let mut last = 0;
            let mut counts: BTreeMap<(String, String, &str), usize> = BTreeMap::new();
            for (range, new, what) in &edits {
                let old = text[range.clone()].to_string();
                out.push_str(&text[last..range.start]);
                out.push_str(new);
                last = range.end;
                *counts.entry((old, (*new).clone(), what)).or_default() += 1;
            }
            out.push_str(&text[last..]);
            for ((old, new, what), n) in counts {
                let detail = match what {
                    "instance" => format!("{n} instance(s) of {old} -> {new}"),
                    other => format!("{other} {old} -> {new}"),
                };
                self.put(&file, out.clone().into_bytes(), detail);
            }
        }
    }

    /// Files that move while open with unsaved changes.
    fn check_busy_moves(&mut self) {
        if self.copy {
            return;
        }
        let busy: Vec<PathBuf> = self
            .options
            .busy
            .iter()
            .filter(|f| self.map.get(f).is_some())
            .cloned()
            .collect();
        for file in busy {
            self.blockers.insert(format!(
                "{} has unsaved changes: save or close it first",
                file.display()
            ));
        }
    }

    fn follow_all(&mut self) {
        self.check_busy_moves();
        if self.options.keep_references {
            return;
        }
        self.follow_instances();
        self.follow_links();
        self.follow_layouts();
        self.follow_mags();
        self.follow_names();
    }
}

/// The new name for `name` in a map of old → new names, ignoring case (SPICE).
fn find_name<'a>(map: &'a BTreeMap<String, String>, name: &str) -> Option<&'a String> {
    map.iter()
        .find(|(old, _)| old.eq_ignore_ascii_case(name))
        .map(|(_, new)| new)
}

fn is_netlist(format: Format) -> bool {
    matches!(format, Format::Spice | Format::Cdl)
}

fn is_verilog(format: Format) -> bool {
    matches!(format, Format::Verilog | Format::SystemVerilog)
}

/// Escape what a `{…}` group of an xschem file cannot hold as it is.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '\\' | '{' | '}') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Rewrite the `LIB=` and `PCELL=` records of KLayout's context structure: every
/// placed PCell `(library, pcell)` that is a key of `map` gets the value. Returns the
/// stream and how many PCells changed.
fn rewrite_pcell_contexts(
    bytes: &[u8],
    map: &BTreeMap<(String, String), (String, String)>,
) -> (Vec<u8>, usize) {
    fn record(out: &mut Vec<u8>, kind: u16, data: &[u8]) {
        out.extend_from_slice(&((data.len() + 4) as u16).to_be_bytes());
        out.extend_from_slice(&kind.to_be_bytes());
        out.extend_from_slice(data);
    }
    fn value(text: String) -> Vec<u8> {
        let mut data = text.into_bytes();
        if data.len() % 2 == 1 {
            data.push(0);
        }
        data
    }
    let mut out = Vec::with_capacity(bytes.len() + 16);
    let mut changed = 0;
    let mut inside = false;
    // The records of the reference being read, in the context structure.
    let mut pending: Option<Vec<(u16, Vec<u8>)>> = None;
    let mut consumed = 0;
    for (kind, data, whole) in hierarchy::gds_records(bytes) {
        consumed += whole.len();
        match kind {
            GDS_STRNAME => inside = hierarchy::gds_text(data) == CONTEXT_INFO,
            GDS_ENDSTR => inside = false,
            _ => {}
        }
        if inside && kind == GDS_SREF {
            pending = Some(vec![(kind, data.to_vec())]);
            continue;
        }
        let Some(records) = &mut pending else {
            out.extend_from_slice(whole);
            continue;
        };
        records.push((kind, data.to_vec()));
        if kind != GDS_ENDEL {
            continue;
        }
        let mut records = pending.take().expect("checked above");
        let find = |records: &[(u16, Vec<u8>)], prefix: &str| {
            records.iter().find_map(|(k, d)| {
                (*k == GDS_PROPVALUE)
                    .then(|| hierarchy::gds_text(d))
                    .and_then(|v| v.strip_prefix(prefix).map(str::to_string))
            })
        };
        let key = (
            find(&records, "LIB=").unwrap_or_default(),
            find(&records, "PCELL=").unwrap_or_default(),
        );
        if let Some((library, pcell)) = map.get(&key) {
            for (k, d) in &mut records {
                if *k != GDS_PROPVALUE {
                    continue;
                }
                let text = hierarchy::gds_text(d);
                if text.starts_with("LIB=") {
                    *d = value(format!("LIB={library}"));
                } else if text.starts_with("PCELL=") {
                    *d = value(format!("PCELL={pcell}"));
                }
            }
            changed += 1;
        }
        for (k, d) in records {
            record(&mut out, k, &d);
        }
    }
    // Whatever follows the last whole record (tape padding) stays.
    out.extend_from_slice(&bytes[consumed..]);
    (out, changed)
}

// ---------------------------------------------------------------------------------
// Planning
// ---------------------------------------------------------------------------------

fn library<'a>(project: &'a Project, name: &str) -> Result<&'a Library> {
    project
        .library(name)
        .ok_or_else(|| eyre!("no library '{name}' in the project"))
}

fn cell<'a>(library: &'a Library, name: &str) -> Result<&'a Cell> {
    library
        .cell(name)
        .ok_or_else(|| eyre!("no cell '{name}' in library '{}'", library.name))
}

fn view<'a>(library: &Library, cell: &'a Cell, name: &str) -> Result<&'a Entry> {
    cell.entry(name)
        .ok_or_else(|| eyre!("cell '{}/{}' has no view '{name}'", library.name, cell.name))
}

fn free_cell(library: &Library, name: &str) -> Result<()> {
    validate_name("cell", name)?;
    if library.cell(name).is_some() || library.path.join(name).exists() {
        bail!("'{name}' already exists in library '{}'", library.name);
    }
    Ok(())
}

/// The cells of `library` that `cell` is built from, at any depth (not itself).
fn cells_below(index: &Index, library: &str, cell: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    let mut todo = vec![cell.to_string()];
    while let Some(current) = todo.pop() {
        for r in index.uses(library, &current) {
            if let Some((l, c)) = r.target.cell()
                && l == library
                && c != cell
                && !found.iter().any(|f| f == c)
            {
                found.push(c.to_string());
                todo.push(c.to_string());
            }
        }
    }
    found.sort();
    found
}

/// Take over what a transfer planned.
fn add(t: Transfer, steps: &mut Vec<Step>, map: &mut PathMap, notes: &mut Vec<String>) {
    steps.extend(t.steps);
    map.files.extend(t.map.files);
    map.folders.extend(t.map.folders);
    notes.extend(t.notes);
}

/// Plan an operation on the project as scanned. Nothing is changed.
pub fn plan(project: &Project, operation: &Operation, options: &Options) -> Result<Plan> {
    let index = Index::build(project);
    plan_with(project, &index, operation, options)
}

/// A cell or view that lives inside a file holding several cells (a standard-cell GDS,
/// a Liberty file: [`crate::Entry::member`]) cannot be renamed, moved or deleted by
/// itself: the file belongs to every cell in it. It can be copied: the copy is a file
/// of its own (GDSII layouts and SPICE/CDL netlists; other views are left out).
fn not_in_a_collection(project: &Project, operation: &Operation) -> Result<()> {
    let (library, cell, view) = match operation {
        Operation::RenameCell { library, cell, .. }
        | Operation::MoveCell { library, cell, .. }
        | Operation::DeleteCell { library, cell } => (library, cell, None),
        // Copies are made by taking the cell out of the file ([`Transfer::member`]).
        Operation::CopyCell { .. } | Operation::CopyView { .. } => return Ok(()),
        Operation::RenameView {
            library,
            cell,
            view,
            ..
        }
        | Operation::DeleteView {
            library,
            cell,
            view,
        } => (library, cell, Some(view)),
        Operation::RenameLibrary { .. } | Operation::Import { .. } => return Ok(()),
    };
    let Some(found) = project.library(library).and_then(|l| l.cell(cell)) else {
        return Ok(());
    };
    let inside = found
        .entries
        .iter()
        .filter(|e| view.is_none_or(|v| &e.name == v))
        .find(|e| e.is_member());
    if let Some(entry) = inside {
        let file = entry.main_file.as_deref().unwrap_or(&entry.path);
        bail!(
            "cannot {operation}: view '{}' of {library}/{cell} is one cell of {}, a file \
             that holds several cells",
            entry.name,
            file.display()
        );
    }
    Ok(())
}

/// What an operation means for the categories of the cell (`crate::category`). The
/// operation is done by now, so a problem here is reported, not an error.
fn follow_categories(project: &Project, operation: &Operation, problems: &mut Vec<String>) {
    use crate::category;
    let result = match operation {
        Operation::RenameCell { library, cell, new } => {
            category::cell_renamed(project, library, cell, new)
        }
        Operation::DeleteCell { library, cell } => category::cell_removed(project, library, cell),
        Operation::MoveCell { library, cell, .. } => {
            // The categories of the other library are its own: the cell starts there
            // without any.
            category::cell_removed(project, library, cell)
        }
        _ => Ok(()),
    };
    if let Err(e) = result {
        problems.push(format!("categories not updated: {e}"));
    }
}

/// [`plan`] with an index that was built already (for the same state of the project).
pub fn plan_with(
    project: &Project,
    index: &Index,
    operation: &Operation,
    options: &Options,
) -> Result<Plan> {
    not_in_a_collection(project, operation)?;
    let mut steps = Vec::new();
    let mut map = PathMap::default();
    let mut ids = IdMap::default();
    let mut notes = Vec::new();
    let mut copy = false;
    let mut delete = None;
    let mut project_write: Option<(PathBuf, String, String)> = None;
    let result;

    match operation {
        Operation::RenameCell {
            library: lib,
            cell: old,
            new,
        }
        | Operation::MoveCell {
            library: lib,
            cell: old,
            new,
            ..
        } => {
            let to_lib = match operation {
                Operation::MoveCell { to_library, .. } => to_library,
                _ => lib,
            };
            let src = library(project, lib)?;
            let dst = library(project, to_lib)?;
            src.writable()?;
            dst.writable()?;
            let found = cell(src, old)?;
            if lib == to_lib && old == new {
                bail!("'{old}' is its name already");
            }
            free_cell(dst, new)?;
            add(
                transfer_cell(found, dst, new, false)?,
                &mut steps,
                &mut map,
                &mut notes,
            );
            ids.cells
                .push(((lib.clone(), old.clone()), (to_lib.clone(), new.clone())));
            result = ViewId::new(to_lib, new, "");
        }
        Operation::CopyCell {
            library: lib,
            cell: old,
            to_library,
            new,
            hierarchy,
        } => {
            copy = true;
            let src = library(project, lib)?;
            let dst = library(project, to_library)?;
            dst.writable()?;
            let found = cell(src, old)?;
            free_cell(dst, new)?;
            add(
                transfer_cell(found, dst, new, true)?,
                &mut steps,
                &mut map,
                &mut notes,
            );
            ids.cells.push((
                (lib.clone(), old.clone()),
                (to_library.clone(), new.clone()),
            ));
            if *hierarchy {
                if lib == to_library {
                    bail!(
                        "copying with the cells below needs another library: in '{lib}' \
                         they would keep their names"
                    );
                }
                for below in cells_below(index, lib, old) {
                    if below == *new {
                        bail!("'{below}' is used below '{old}' and is also the new name");
                    }
                    free_cell(dst, &below)?;
                    let found = cell(src, &below)?;
                    add(
                        transfer_cell(found, dst, &below, true)?,
                        &mut steps,
                        &mut map,
                        &mut notes,
                    );
                    ids.cells.push((
                        (lib.clone(), below.clone()),
                        (to_library.clone(), below.clone()),
                    ));
                }
            }
            result = ViewId::new(to_library, new, "");
        }
        Operation::DeleteCell {
            library: lib,
            cell: name,
        } => {
            let src = library(project, lib)?;
            src.writable()?;
            cell(src, name)?;
            delete = Some((lib.clone(), name.clone(), None));
            result = ViewId::new(lib, "", "");
        }
        Operation::RenameView {
            library: lib,
            cell: name,
            view: old,
            new,
        } => {
            let src = library(project, lib)?;
            src.writable()?;
            let found = cell(src, name)?;
            let entry = view(src, found, old)?;
            validate_name("view", new)?;
            if found.entry(new).is_some() {
                bail!("cell '{lib}/{name}' already has a view '{new}'");
            }
            let in_folder = src.layout == LibraryLayout::Cells
                && entry.path.is_dir()
                && entry.path.parent() == found.path.as_deref();
            if !in_folder {
                bail!(
                    "view '{old}' of '{lib}/{name}' is a single file: its name comes from \
                     the kind of file, it cannot be renamed"
                );
            }
            let target = entry.path.with_file_name(new);
            let mut t = Transfer::default();
            t.folder(&entry.path, &target, name, name, 0, false)?;
            add(t, &mut steps, &mut map, &mut notes);
            ids.views
                .push((ViewId::new(lib, name, old), ViewId::new(lib, name, new)));
            result = ViewId::new(lib, name, new);
        }
        Operation::CopyView {
            library: lib,
            cell: name,
            view: old,
            to_library,
            to_cell,
            new,
        } => {
            copy = true;
            let src = library(project, lib)?;
            let dst = library(project, to_library)?;
            dst.writable()?;
            let found = cell(src, name)?;
            let entry = view(src, found, old)?;
            validate_name("cell", to_cell)?;
            let mut t = Transfer::default();
            let mut view_name = new.clone();
            if dst.layout == LibraryLayout::Cells {
                validate_name("view", new)?;
                let taken = dst.cell(to_cell).is_some_and(|c| c.entry(new).is_some());
                let target = dst.path.join(to_cell).join(new);
                if taken || target.exists() {
                    bail!("cell '{to_library}/{to_cell}' already has a view '{new}'");
                }
                if entry.is_member() {
                    t.member(entry, name, to_cell, &target, true)?;
                    if t.steps.is_empty() {
                        bail!("{}", t.notes.join("; "));
                    }
                } else if entry.path.is_dir() {
                    t.folder(&entry.path, &target, name, to_cell, 1, true)?;
                } else {
                    t.steps.push(Step::MakeDir(target.clone()));
                    for file in entry_files(entry) {
                        let file_name = file_name(&file);
                        let file_name = renamed(&file_name, name, to_cell).unwrap_or(file_name);
                        t.item(&file, target.join(file_name), true)?;
                    }
                }
            } else if dst.layout == LibraryLayout::Flat && entry.is_member() {
                t.member(entry, name, to_cell, &dst.path, false)?;
                if t.steps.is_empty() {
                    bail!("{}", t.notes.join("; "));
                }
                view_name = entry.kind.name().to_string();
                notes.push(format!(
                    "'{to_library}' is a flat library: the view is named after its kind of file"
                ));
            } else if dst.layout == LibraryLayout::Flat {
                let files: Vec<PathBuf> = if entry.path.is_dir() {
                    walk(&entry.path)?
                        .into_iter()
                        .filter(|(_, rel)| rel.components().count() == 1)
                        .map(|(f, _)| f)
                        .collect()
                } else {
                    entry_files(entry)
                };
                for file in files {
                    if let Some(file_name) = renamed(&file_name(&file), name, to_cell) {
                        t.item(&file, dst.path.join(file_name), true)?;
                    }
                }
                if t.map.files.is_empty() {
                    bail!("view '{old}' has no file named after the cell to put in a flat library");
                }
                // In a flat library the view is named after the kind of file.
                view_name = entry.kind.name().to_string();
                notes.push(format!(
                    "'{to_library}' is a flat library: the view is named after its kind of file"
                ));
            } else {
                bail!(
                    "library '{to_library}' is laid out as '{}': views cannot be put there",
                    dst.layout.name()
                );
            }
            add(t, &mut steps, &mut map, &mut notes);
            ids.views.push((
                ViewId::new(lib, name, old),
                ViewId::new(to_library, to_cell, &view_name),
            ));
            result = ViewId::new(to_library, to_cell, &view_name);
        }
        Operation::DeleteView {
            library: lib,
            cell: name,
            view: old,
        } => {
            let src = library(project, lib)?;
            src.writable()?;
            view(src, cell(src, name)?, old)?;
            delete = Some((lib.clone(), name.clone(), Some(old.clone())));
            result = ViewId::new(lib, name, "");
        }
        Operation::Import { .. } => {
            bail!("an import is planned with importer::plan_import");
        }
        Operation::RenameLibrary {
            library: old,
            new,
            folder,
        } => {
            let src = library(project, old)?;
            if let Some(error) = &src.error {
                bail!("library '{old}' is not usable: {error}");
            }
            validate_name("library", new)?;
            if new.contains('.') {
                bail!("'.' is not allowed in a library name");
            }
            if project.library(new).is_some() {
                bail!("library '{new}' is already defined");
            }
            let mut new_folder = None;
            if *folder {
                if src.read_only {
                    bail!("library '{old}' is read-only: its folder is not renamed");
                }
                let target = src.path.with_file_name(new);
                if target.exists() {
                    bail!("{} already exists", target.display());
                }
                steps.push(Step::Move {
                    from: src.path.clone(),
                    to: target.clone(),
                });
                map.folders.push((src.path.clone(), target));
                new_folder = Some((file_name(&src.path), new.clone()));
            } else if file_name(&src.path) != *old {
                notes.push(format!(
                    "the folder keeps its name '{}': that is the name xschem references use",
                    file_name(&src.path)
                ));
            } else {
                notes.push(format!(
                    "the folder keeps its name '{old}': xschem references still read `{old}/…`"
                ));
            }
            let text = rename_definition(&src.defined_in, old, new, new_folder.as_ref())?;
            project_write = Some((
                src.defined_in.clone(),
                text,
                format!("library '{old}' is defined as '{new}'"),
            ));
            ids.library = Some((old.clone(), new.clone()));
            result = ViewId::new(new, "", "");
        }
    }

    let mut work = Work {
        project,
        index,
        options,
        map,
        ids,
        copy,
        search: crate::link::schematic::search_path(project),
        contents: BTreeMap::new(),
        details: BTreeMap::new(),
        not_followed: Vec::new(),
        blockers: BTreeSet::new(),
    };
    if delete.is_none() {
        work.follow_all();
    }

    // What the plan shows.
    let mut changes = Vec::new();
    for step in &steps {
        if let Step::Extract { from, to, what, .. } = step {
            changes.push(Change {
                kind: ChangeKind::Copy,
                path: from.clone(),
                to: Some(to.clone()),
                label: String::new(),
                details: vec![what.clone()],
            });
            continue;
        }
        let (kind, from, to) = match step {
            Step::Move { from, to } => (ChangeKind::Move, from, to),
            Step::Copy { from, to } => (ChangeKind::Copy, from, to),
            Step::MakeDir(_) | Step::RemoveEmpty(_) | Step::Extract { .. } => continue,
        };
        changes.push(Change {
            kind,
            path: from.clone(),
            to: Some(to.clone()),
            label: String::new(),
            details: Vec::new(),
        });
    }
    let mut writes = Vec::new();
    for (file, bytes) in std::mem::take(&mut work.contents) {
        let after = work.map.after(&file);
        changes.push(Change {
            kind: ChangeKind::Rewrite,
            path: file.clone(),
            to: (after != file).then(|| after.clone()),
            label: work.label(&file),
            details: work.details.remove(&file).unwrap_or_default(),
        });
        writes.push(Write { file: after, bytes });
    }
    if let Some((file, text, detail)) = project_write {
        changes.push(Change {
            kind: ChangeKind::Rewrite,
            path: file.clone(),
            to: None,
            label: String::new(),
            details: vec![detail],
        });
        writes.push(Write {
            file,
            bytes: text.into_bytes(),
        });
    }
    if let Some((lib, name, view_name)) = &delete {
        let src = library(project, lib)?;
        let found = cell(src, name)?;
        let paths: Vec<PathBuf> = match view_name {
            Some(v) => vec![view(src, found, v)?.path.clone()],
            None => match &found.path {
                Some(dir) => vec![dir.clone()],
                None => found.entries.iter().map(|e| e.path.clone()).collect(),
            },
        };
        for path in paths {
            changes.push(Change {
                kind: ChangeKind::Delete,
                path,
                to: None,
                label: String::new(),
                details: Vec::new(),
            });
        }
        // What will be left pointing at nothing.
        let mut users: BTreeMap<(ViewId, PathBuf), Vec<String>> = BTreeMap::new();
        for r in index.used_by(lib, name, view_name.as_deref()) {
            users
                .entry((r.from.clone(), r.file.clone()))
                .or_default()
                .push(format!("{} ×{} ({})", r.text, r.count, r.kind.name()));
        }
        for ((from, file), details) in users {
            changes.push(Change {
                kind: ChangeKind::NotFollowed,
                path: file,
                to: None,
                label: from.label(),
                details: std::iter::once("it uses what is deleted:".to_string())
                    .chain(details)
                    .collect(),
            });
        }
    }
    for item in &work.not_followed {
        changes.push(Change {
            kind: ChangeKind::NotFollowed,
            path: item.file.clone(),
            to: None,
            label: item.label.clone(),
            details: vec![item.reason.clone()],
        });
    }
    if options.keep_references && delete.is_none() {
        notes.push("references are left as they are".into());
    }
    let reload = matches!(operation, Operation::RenameLibrary { .. });
    Ok(Plan {
        operation: operation.clone(),
        changes,
        blockers: work.blockers.into_iter().collect(),
        notes,
        result,
        steps,
        writes,
        delete,
        reload,
    })
}

/// The text of the file defining library `old` with the definition renamed to `new`;
/// with `folder` (`old folder name`, `new folder name`) the last part of its path too.
fn rename_definition(
    file: &Path,
    old: &str,
    new: &str,
    folder: Option<&(String, String)>,
) -> Result<String> {
    use toml_edit::{DocumentMut, Item, Key};
    let text =
        fs::read_to_string(file).wrap_err_with(|| format!("cannot read {}", file.display()))?;
    let mut doc: DocumentMut = text.parse().map_err(|e| eyre!("{}: {e}", file.display()))?;
    let libraries = doc
        .get_mut("libraries")
        .and_then(Item::as_table_like_mut)
        .ok_or_else(|| eyre!("{} has no [libraries] table", file.display()))?;
    if libraries.get(old).is_none() {
        bail!("library '{old}' is not defined in {}", file.display());
    }
    // The same place in the table, the same value, a new key.
    // Keys keep their decoration (the comment lines above them).
    let entries: Vec<(Key, Item)> = libraries
        .iter()
        .map(|(name, item)| {
            let key = match libraries.get_key_value(name) {
                Some((key, _)) => key.clone(),
                None => Key::new(name),
            };
            (key, item.clone())
        })
        .collect();
    let keys: Vec<String> = entries.iter().map(|(k, _)| k.get().to_string()).collect();
    for key in &keys {
        libraries.remove(key);
    }
    for (key, mut item) in entries {
        let name = if key.get() == old { new } else { key.get() };
        if key.get() == old
            && let Some((old_folder, new_folder)) = folder
        {
            let path_of = |item: &Item| -> Option<String> {
                match item.as_str() {
                    Some(path) => Some(path.to_string()),
                    None => Some(item.as_table_like()?.get("path")?.as_str()?.to_string()),
                }
            };
            let path = path_of(&item)
                .ok_or_else(|| eyre!("library '{old}' has no path in {}", file.display()))?;
            let trimmed = path.trim_end_matches('/');
            let renamed = match trimmed.rsplit_once('/') {
                Some((head, last)) if last == old_folder => format!("{head}/{new_folder}"),
                None if trimmed == old_folder => new_folder.clone(),
                _ => {
                    return Err(eyre!(
                        "the path of library '{old}' ({path}) does not end in its folder name \
                     '{old_folder}': rename the folder by hand, or rename the library only"
                    ));
                }
            };
            match item.as_table_like_mut() {
                Some(table) => {
                    table.insert("path", toml_edit::value(renamed));
                }
                None => {
                    let decor = item.as_value().map(|v| v.decor().clone());
                    let mut value = toml_edit::Value::from(renamed);
                    if let Some(decor) = decor {
                        *value.decor_mut() = decor;
                    }
                    item = Item::Value(value);
                }
            }
        }
        libraries.insert(name, item);
        if let Some(mut inserted) = libraries.key_mut(name) {
            *inserted.leaf_decor_mut() = key.leaf_decor().clone();
        }
    }
    Ok(doc.to_string())
}

// ---------------------------------------------------------------------------------
// Applying
// ---------------------------------------------------------------------------------

/// What was done, to put it back when a later step fails.
enum Undo {
    /// Remove a file or folder that was made.
    Remove(PathBuf),
    MoveBack {
        from: PathBuf,
        to: PathBuf,
    },
    /// Give a file its old contents back; `None`: it did not exist.
    Restore(PathBuf, Option<Vec<u8>>),
}

#[derive(Default)]
struct Journal {
    undo: Vec<Undo>,
    /// Sources of moves across file systems: removed once everything worked.
    remove_later: Vec<PathBuf>,
    /// Folders to remove when they hold nothing any more.
    empty_later: Vec<PathBuf>,
}

fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    if from.is_dir() {
        fs::create_dir_all(to).wrap_err_with(|| format!("cannot create {}", to.display()))?;
        for item in list(from)? {
            copy_tree(&item, &to.join(file_name(&item)))?;
        }
        Ok(())
    } else {
        fs::copy(from, to)
            .map(|_| ())
            .wrap_err_with(|| format!("cannot copy {} to {}", from.display(), to.display()))
    }
}

fn remove_tree(path: &Path) -> std::io::Result<()> {
    if path.is_dir() {
        fs::remove_dir_all(path)
    } else if path.exists() {
        fs::remove_file(path)
    } else {
        Ok(())
    }
}

/// Remove a folder if nothing but empty folders is in it.
fn remove_empty(dir: &Path) {
    if let Ok(items) = list(dir) {
        for item in items.iter().filter(|p| p.is_dir()) {
            remove_empty(item);
        }
    }
    let _ = fs::remove_dir(dir);
}

impl Journal {
    /// Create a folder and those above it; the topmost new one is what to remove.
    fn make_dir(&mut self, dir: &Path) -> Result<()> {
        if dir.is_dir() {
            return Ok(());
        }
        let mut top = dir;
        while let Some(parent) = top.parent() {
            if parent.exists() || parent.as_os_str().is_empty() {
                break;
            }
            top = parent;
        }
        fs::create_dir_all(dir).wrap_err_with(|| format!("cannot create {}", dir.display()))?;
        self.undo.push(Undo::Remove(top.to_path_buf()));
        Ok(())
    }

    fn free(&mut self, to: &Path) -> Result<()> {
        if to.exists() {
            bail!("{} already exists", to.display());
        }
        match to.parent() {
            Some(parent) => self.make_dir(parent),
            None => Ok(()),
        }
    }

    fn move_path(&mut self, from: &Path, to: &Path) -> Result<()> {
        self.free(to)?;
        match fs::rename(from, to) {
            Ok(()) => {
                self.undo.push(Undo::MoveBack {
                    from: from.to_path_buf(),
                    to: to.to_path_buf(),
                });
                Ok(())
            }
            // Another file system: copy now, remove the source at the end.
            Err(e) if e.kind() == std::io::ErrorKind::CrossesDevices => {
                self.undo.push(Undo::Remove(to.to_path_buf()));
                copy_tree(from, to)?;
                self.remove_later.push(from.to_path_buf());
                Ok(())
            }
            Err(e) => Err(e)
                .wrap_err_with(|| format!("cannot move {} to {}", from.display(), to.display())),
        }
    }

    fn copy_path(&mut self, from: &Path, to: &Path) -> Result<()> {
        self.free(to)?;
        self.undo.push(Undo::Remove(to.to_path_buf()));
        copy_tree(from, to)
    }

    fn write(&mut self, file: &Path, bytes: &[u8]) -> Result<()> {
        let before = if file.exists() {
            Some(fs::read(file).wrap_err_with(|| format!("cannot read {}", file.display()))?)
        } else {
            None
        };
        self.undo.push(Undo::Restore(file.to_path_buf(), before));
        layout::write_atomic(file, bytes)
    }

    /// Put everything back, last step first. Returns what could not be put back.
    fn roll_back(self) -> Vec<String> {
        let mut failed = Vec::new();
        for undo in self.undo.into_iter().rev() {
            let result = match &undo {
                Undo::Remove(path) => remove_tree(path),
                Undo::MoveBack { from, to } => fs::rename(to, from),
                Undo::Restore(file, Some(bytes)) => fs::write(file, bytes),
                Undo::Restore(file, None) => remove_tree(file),
            };
            if let Err(e) = result {
                failed.push(match undo {
                    Undo::Remove(path) => format!("cannot remove {}: {e}", path.display()),
                    Undo::MoveBack { from, to } => {
                        format!(
                            "cannot move {} back to {}: {e}",
                            to.display(),
                            from.display()
                        )
                    }
                    Undo::Restore(file, _) => format!("cannot restore {}: {e}", file.display()),
                });
            }
        }
        failed
    }

    fn commit(self) {
        for path in &self.remove_later {
            if let Err(e) = remove_tree(path) {
                tracing::warn!("cannot remove {} after moving it: {e}", path.display());
            }
        }
        for dir in &self.empty_later {
            remove_empty(dir);
        }
    }
}

/// What applying a plan did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outcome {
    /// One sentence per thing done, for the log.
    pub lines: Vec<String>,
    /// Views that could not follow and are now wrong.
    pub problems: Vec<String>,
}

/// Apply a plan made for `project` as it still is. When a step fails, what was done
/// is put back and the error says so. Scan (or, for [`Plan::reload`], load) the
/// project again afterwards.
pub fn apply(project: &Project, plan: &Plan) -> Result<Outcome> {
    if !plan.blockers.is_empty() {
        bail!("{}", plan.blockers.join("; "));
    }
    let root = &project.root;
    let show = |p: &Path| p.strip_prefix(root).unwrap_or(p).display().to_string();
    let mut outcome = Outcome::default();

    if let Some((lib, name, view_name)) = &plan.delete {
        let src = library(project, lib)?;
        match view_name {
            Some(v) => src.delete_view(name, v)?,
            None => src.delete_cell(name)?,
        }
        outcome.lines.push(plan.operation.to_string());
    } else {
        let mut journal = Journal::default();
        let run = |journal: &mut Journal| -> Result<()> {
            for step in &plan.steps {
                match step {
                    Step::MakeDir(dir) => journal.make_dir(dir)?,
                    Step::Move { from, to } => journal.move_path(from, to)?,
                    Step::Copy { from, to } => journal.copy_path(from, to)?,
                    Step::RemoveEmpty(dir) => journal.empty_later.push(dir.clone()),
                    Step::Extract { to, bytes, .. } => journal.write(to, bytes)?,
                }
            }
            for write in &plan.writes {
                journal.write(&write.file, &write.bytes)?;
            }
            Ok(())
        };
        if let Err(e) = run(&mut journal) {
            let failed = journal.roll_back();
            if failed.is_empty() {
                return Err(e.wrap_err(format!(
                    "could not {}; everything was put back as it was",
                    plan.operation
                )));
            }
            return Err(e.wrap_err(format!(
                "could not {}, and not everything could be put back: {}",
                plan.operation,
                failed.join("; ")
            )));
        }
        journal.commit();
        outcome.lines.push(plan.operation.to_string());
        for change in &plan.changes {
            if change.kind == ChangeKind::Rewrite {
                let label = if change.label.is_empty() {
                    show(&change.path)
                } else {
                    change.label.clone()
                };
                outcome
                    .lines
                    .push(format!("{label}: {}", change.details.join("; ")));
            }
        }
    }
    for change in plan.not_followed() {
        let label = if change.label.is_empty() {
            show(&change.path)
        } else {
            change.label.clone()
        };
        outcome.problems.push(format!(
            "{label}: not updated: {}",
            change.details.join(" ")
        ));
    }
    follow_categories(project, &plan.operation, &mut outcome.problems);
    Ok(outcome)
}

/// Plan and apply in one go.
pub fn run(project: &Project, operation: &Operation, options: &Options) -> Result<Outcome> {
    apply(project, &plan(project, operation, options)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_and_files_map_to_their_new_place() {
        let mut map = PathMap::default();
        map.folders
            .push((PathBuf::from("/p/lib/inv"), PathBuf::from("/p/lib/inv2")));
        map.files.insert(
            PathBuf::from("/p/lib/inv/symbol/inv.sym"),
            PathBuf::from("/p/lib/inv2/symbol/inv2.sym"),
        );
        assert_eq!(
            map.get(Path::new("/p/lib/inv/symbol/inv.sym")),
            Some(PathBuf::from("/p/lib/inv2/symbol/inv2.sym"))
        );
        assert_eq!(
            map.get(Path::new("/p/lib/inv/schematic/notes.txt")),
            Some(PathBuf::from("/p/lib/inv2/schematic/notes.txt"))
        );
        assert_eq!(map.get(Path::new("/p/lib/buf/x")), None);
        assert_eq!(
            map.after(Path::new("/p/lib/buf/x")),
            PathBuf::from("/p/lib/buf/x")
        );
    }

    #[test]
    fn pcell_records_are_rewritten_in_the_context_structure_only() {
        use crate::layout::gds::tests::rec;
        let name = |t: &str| {
            let mut b = t.as_bytes().to_vec();
            if b.len() % 2 == 1 {
                b.push(0);
            }
            b
        };
        let mut gds = Vec::new();
        rec(&mut gds, 0x0502, &[0; 24]);
        rec(&mut gds, GDS_STRNAME, &name(CONTEXT_INFO));
        for (proxy, lib, pcell) in [("rpoly$1", "logic", "rpoly"), ("C", "Basic", "CIRCLE")] {
            rec(&mut gds, GDS_SREF, &[]);
            rec(&mut gds, hierarchy::GDS_SNAME, &name(proxy));
            rec(&mut gds, 0x2b02, &1i16.to_be_bytes());
            rec(&mut gds, GDS_PROPVALUE, &name(&format!("PCELL={pcell}")));
            rec(&mut gds, 0x2b02, &0i16.to_be_bytes());
            rec(&mut gds, GDS_PROPVALUE, &name(&format!("LIB={lib}")));
            rec(&mut gds, GDS_ENDEL, &[]);
        }
        rec(&mut gds, GDS_ENDSTR, &[]);
        rec(&mut gds, 0x0400, &[]);
        let map = BTreeMap::from([(
            ("logic".to_string(), "rpoly".to_string()),
            ("analog".to_string(), "rpoly_hv".to_string()),
        )]);
        let (out, changed) = rewrite_pcell_contexts(&gds, &map);
        assert_eq!(changed, 1);
        let found = hierarchy::pcell_contexts(&out);
        assert_eq!(found.len(), 2);
        assert_eq!(
            (found[0].library.as_str(), found[0].pcell.as_str()),
            ("analog", "rpoly_hv")
        );
        assert_eq!(found[0].proxy, "rpoly$1");
        assert_eq!(
            (found[1].library.as_str(), found[1].pcell.as_str()),
            ("Basic", "CIRCLE")
        );
        // Nothing to rename: the stream stays byte for byte.
        let (same, changed) = rewrite_pcell_contexts(&gds, &BTreeMap::new());
        assert_eq!((same, changed), (gds, 0));
    }

    #[test]
    fn a_library_definition_is_renamed_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("cdept.toml");
        fs::write(
            &file,
            "name = \"p\"\n\n[libraries]\n# the analog cells\nanalog = \"libs/analog\" # here\n\
             ro = { path = \"$PDK_ROOT/x/analog\", read_only = true }\nlast = \"libs/last\"\n",
        )
        .unwrap();
        let text = rename_definition(&file, "analog", "amp", None).unwrap();
        assert!(
            text.contains("# the analog cells\namp = \"libs/analog\" # here\n"),
            "{text}"
        );
        let order: Vec<usize> = ["amp =", "ro =", "last ="]
            .iter()
            .map(|k| text.find(k).unwrap())
            .collect();
        assert!(order.is_sorted(), "{text}");

        let folder = ("analog".to_string(), "amp".to_string());
        let text = rename_definition(&file, "analog", "amp", Some(&folder)).unwrap();
        assert!(text.contains("amp = \"libs/amp\" # here\n"), "{text}");
        let text = rename_definition(&file, "ro", "pdk", Some(&folder)).unwrap();
        assert!(
            text.contains("pdk = { path = \"$PDK_ROOT/x/amp\", read_only = true }"),
            "{text}"
        );
        // A path that does not end in the folder name is not guessed.
        let other = ("elsewhere".to_string(), "amp".to_string());
        assert!(rename_definition(&file, "last", "amp", Some(&other)).is_err());
        assert!(rename_definition(&file, "gone", "amp", None).is_err());
    }
}
