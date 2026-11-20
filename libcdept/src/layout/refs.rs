//! Resolving the ghost cells of layout files through the project (docs/KLAYOUT.md §3).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

use eyre::{Result, bail, eyre};
use serde::{Deserialize, Serialize};

use super::gds;
use super::meta::{Meta, MetaRef};
use crate::{Cell, Entry, Format, Library, Project, ViewKind};

/// Every file reachable from `root`, with each ghost name resolved. Sent to the helper
/// as is (JSON, docs/KLAYOUT.md §3.3).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct RefTable {
    pub root: PathBuf,
    pub files: BTreeMap<PathBuf, FileRefs>,
    /// The project's PCell views (docs/KLAYOUT.md §3.6): CDePT library → cell → its
    /// `<cell>.pcell.py`. The helper registers one KLayout library per CDePT library.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub pcells: BTreeMap<String, BTreeMap<String, PathBuf>>,
}

/// The PCell views of the project's libraries, for [`RefTable::pcells`].
pub fn pcell_views(project: &Project) -> BTreeMap<String, BTreeMap<String, PathBuf>> {
    let mut out: BTreeMap<String, BTreeMap<String, PathBuf>> = BTreeMap::new();
    for lib in &project.libraries {
        for cell in &lib.cells {
            let file = cell
                .entries
                .iter()
                .filter(|e| e.format == Some(Format::KlayoutPcell))
                .find_map(|e| e.main_file.clone());
            if let Some(file) = file {
                out.entry(lib.name.clone())
                    .or_default()
                    .insert(cell.name.clone(), file);
            }
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct FileRefs {
    /// `library/cell/view`, or the file name for a file outside the libraries.
    pub label: String,
    /// The file belongs to a read-only library: its cells are not edited in place
    /// (docs/KLAYOUT.md §3.4).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub read_only: bool,
    /// Key: ghost structure name in this file.
    pub refs: BTreeMap<String, Ref>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Ref {
    /// The layout file holding that cell; `None` when it cannot be found.
    pub file: Option<PathBuf>,
    /// `library/cell/view` (or the bare name when nothing is known).
    pub label: String,
    /// Last known bounding box in µm (for the "cannot be found" marker).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bbox: Option<[f64; 4]>,
    /// Why it was not found (missing references only).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
}

/// What the helper's `save` reports (docs/KLAYOUT.md §4): every ghost structure in the
/// written file, with where it came from.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SaveReport {
    pub file: PathBuf,
    pub top: String,
    #[serde(default)]
    pub references: BTreeMap<String, SavedRef>,
    /// Other files written by the same save: external cells edited in place, each with
    /// its own references (docs/KLAYOUT.md §3.4).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub also: Vec<SaveReport>,
}

impl SaveReport {
    /// Every file the save wrote: this one, then those of [`also`](Self::also).
    pub fn files(&self) -> Vec<&Path> {
        let mut out = vec![self.file.as_path()];
        out.extend(self.also.iter().flat_map(SaveReport::files));
        out
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SavedRef {
    /// The layout file the cell was loaded from; `None` for a missing reference.
    pub file: Option<PathBuf>,
    #[serde(default)]
    pub missing: bool,
    /// `[left, bottom, right, top]` in µm, in the cell's own coordinates.
    pub bbox: Option<[f64; 4]>,
}

/// How one ghost name was resolved.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum How {
    /// The sidecar names the cell and it exists.
    Sidecar,
    /// No sidecar entry; a cell of that name was found in the libraries.
    Search,
    Missing,
}

#[derive(Debug, Clone)]
pub(super) struct Resolved {
    pub(super) how: How,
    pub(super) file: Option<PathBuf>,
    /// `library/cell/view` of the target, or the bare name.
    pub(super) label: String,
    /// What to record in the sidecar (not for missing references).
    pub(super) meta: Option<MetaRef>,
    pub(super) bbox: Option<[f64; 4]>,
    pub(super) reason: String,
}

fn label(library: &str, cell: &str, view: &str) -> String {
    format!("{library}/{cell}/{view}")
}

/// The GDS or OASIS file of a layout view.
pub(super) fn layout_file(entry: &Entry) -> Option<&Path> {
    match entry.format? {
        Format::Gds | Format::Oasis => entry.main_file.as_deref(),
        _ => None,
    }
}

/// The layout view to use for a cell found by name: `layout` first, then other GDS
/// views, then OASIS views.
fn find_layout(cell: &Cell) -> Option<(&Entry, &Path)> {
    let layouts = || {
        cell.entries
            .iter()
            .filter(|e| e.kind == ViewKind::Layout)
            .filter_map(|e| layout_file(e).map(|f| (e, f)))
    };
    let rank = |(e, _): &(&Entry, &Path)| (e.format != Some(Format::Gds), e.name != "layout");
    layouts().min_by_key(rank)
}

/// The first library (in project order) with a cell `name` that has a layout view.
fn search(project: &Project, name: &str) -> Option<Resolved> {
    project.libraries.iter().find_map(|lib| {
        let cell = lib.cell(name)?;
        let (entry, file) = find_layout(cell)?;
        Some(Resolved {
            how: How::Search,
            file: Some(file.to_path_buf()),
            label: label(&lib.name, &cell.name, &entry.name),
            meta: Some(MetaRef {
                library: lib.name.clone(),
                cell: cell.name.clone(),
                view: entry.name.clone(),
                bbox: None,
            }),
            bbox: None,
            reason: String::new(),
        })
    })
}

/// Resolve one ghost name of a file whose sidecar is `meta`.
///
/// The sidecar entry wins when its view exists. When it does not (a library or view
/// renamed or deleted behind CDePT's back), the cell it names, then the ghost name, are
/// searched for in the libraries; a hit counts as found by search, which `fix`
/// records.
pub(super) fn resolve_name(project: &Project, meta: Option<&Meta>, name: &str) -> Resolved {
    if let Some(r) = meta.and_then(|m| m.references.get(name)) {
        let label = label(&r.library, &r.cell, &r.view);
        let target = || -> Result<PathBuf, String> {
            let lib = project
                .library(&r.library)
                .ok_or_else(|| format!("library '{}' is not in the project", r.library))?;
            let cell = lib
                .cell(&r.cell)
                .ok_or_else(|| format!("library '{}' has no cell '{}'", r.library, r.cell))?;
            let entry = cell
                .entry(&r.view)
                .ok_or_else(|| format!("cell '{}' has no view '{}'", r.cell, r.view))?;
            let file = layout_file(entry)
                .ok_or_else(|| format!("view '{}' of '{}' is not a layout", r.view, r.cell))?;
            if !file.is_file() {
                return Err(format!("{} does not exist", file.display()));
            }
            Ok(file.to_path_buf())
        };
        return match target() {
            Ok(file) => Resolved {
                how: How::Sidecar,
                file: Some(file),
                label,
                meta: Some(r.clone()),
                bbox: r.bbox,
                reason: String::new(),
            },
            Err(reason) => {
                let found = search(project, &r.cell)
                    .or_else(|| (r.cell != name).then(|| search(project, name)).flatten());
                match found {
                    Some(mut found) => {
                        found.bbox = r.bbox;
                        if let Some(m) = &mut found.meta {
                            m.bbox = r.bbox;
                        }
                        found
                    }
                    None => Resolved {
                        how: How::Missing,
                        file: None,
                        label,
                        meta: None,
                        bbox: r.bbox,
                        reason,
                    },
                }
            }
        };
    }
    search(project, name).unwrap_or_else(|| Resolved {
        how: How::Missing,
        file: None,
        label: name.to_string(),
        meta: None,
        bbox: None,
        reason: format!("no cell '{name}' with a layout view in the project's libraries"),
    })
}

fn same_file(a: &Path, b: &Path) -> bool {
    a == b || matches!((a.canonicalize(), b.canonicalize()), (Ok(a), Ok(b)) if a == b)
}

/// Which `library/cell/view` of the project a layout file is.
fn locate<'a>(project: &'a Project, file: &Path) -> Option<(&'a Library, &'a Cell, &'a Entry)> {
    for lib in &project.libraries {
        for cell in &lib.cells {
            for entry in &cell.entries {
                if entry
                    .main_file
                    .as_deref()
                    .is_some_and(|f| same_file(f, file))
                {
                    return Some((lib, cell, entry));
                }
            }
        }
    }
    None
}

/// Ghost names of a file; empty (with a warning) when it cannot be read, and for
/// compressed files, which are not read.
fn ghosts_of(file: &Path) -> Vec<String> {
    if !super::is_readable(file) {
        return Vec::new();
    }
    match super::read_summary(file) {
        Ok(summary) => summary.ghosts(),
        Err(e) => {
            tracing::warn!("{e:#}");
            Vec::new()
        }
    }
}

fn read_meta(file: &Path) -> Option<Meta> {
    Meta::read_for(file)
        .inspect_err(|e| tracing::warn!("{e:#}"))
        .ok()
        .flatten()
}

fn file_label(project: &Project, file: &Path, meta: Option<&Meta>) -> String {
    if let Some((lib, cell, entry)) = locate(project, file) {
        return label(&lib.name, &cell.name, &entry.name);
    }
    match meta {
        Some(m) => m.label(),
        None => file
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
    }
}

/// The reference table of `file` (a layout file, usually a view of the project): its
/// ghost cells, then theirs, resolved through sidecars and by searching the libraries
/// (§3.1). Never fails: what cannot be found becomes a [`Ref`] with `file: None`.
///
/// Every reachable file appears once, so cycles end. GDSII and OASIS files are
/// followed; compressed files are leaves, and so are unreadable files (with a warning
/// in the log).
pub fn resolve(project: &Project, file: &Path) -> RefTable {
    let mut table = RefTable {
        root: file.to_path_buf(),
        files: BTreeMap::new(),
        pcells: pcell_views(project),
    };
    let mut todo = vec![file.to_path_buf()];
    while let Some(current) = todo.pop() {
        if table.files.contains_key(&current) {
            continue;
        }
        let meta = read_meta(&current);
        let mut entry = FileRefs {
            label: file_label(project, &current, meta.as_ref()),
            read_only: locate(project, &current).is_some_and(|(lib, _, _)| lib.read_only),
            refs: BTreeMap::new(),
        };
        for name in ghosts_of(&current) {
            let r = resolve_name(project, meta.as_ref(), &name);
            if let Some(next) = &r.file {
                todo.push(next.clone());
            }
            entry.refs.insert(
                name,
                Ref {
                    file: r.file,
                    label: r.label,
                    bbox: r.bbox,
                    reason: r.reason,
                },
            );
        }
        table.files.insert(current, entry);
    }
    table
}

/// Write the sidecar of the saved file from the helper's report.
///
/// The file's own library/cell/view/top are kept from its sidecar, or taken from the
/// project when it has none (a file outside the libraries without a sidecar is left
/// alone). `references` becomes exactly what the report lists: each cell loaded from a
/// view of the project is recorded as `library/cell/view` with its bounding box; a
/// missing reference keeps the entry the old sidecar had (bounding box updated);
/// names no longer in the file are dropped.
///
/// The files of [`SaveReport::also`] (external cells edited in place) get theirs the
/// same way.
pub fn update_after_save(project: &Project, report: &SaveReport) -> Result<()> {
    update_one_after_save(project, report)?;
    for other in &report.also {
        update_after_save(project, other)?;
    }
    Ok(())
}

fn update_one_after_save(project: &Project, report: &SaveReport) -> Result<()> {
    let file = &report.file;
    let old = Meta::read_for(file)?;
    let mut meta = match old.clone() {
        Some(meta) => meta,
        None => match locate(project, file) {
            Some((lib, cell, entry)) => Meta::new(&lib.name, &cell.name, &entry.name, &report.top),
            None => return Ok(()),
        },
    };
    meta.references.clear();
    for (name, saved) in &report.references {
        let previous = old.as_ref().and_then(|m| m.references.get(name));
        let located =
            saved
                .file
                .as_deref()
                .and_then(|f| locate(project, f))
                .map(|(lib, cell, entry)| MetaRef {
                    library: lib.name.clone(),
                    cell: cell.name.clone(),
                    view: entry.name.clone(),
                    bbox: None,
                });
        let Some(mut r) = located.or_else(|| previous.cloned()) else {
            continue;
        };
        r.bbox = saved.bbox.or(previous.and_then(|p| p.bbox));
        meta.references.insert(name.clone(), r);
    }
    meta.write_for(file)
}

/// What [`check`] found about one layout file or one of its ghost names.
#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    /// The layout file.
    pub file: PathBuf,
    /// The ghost name; empty for a finding about the file itself.
    pub name: String,
    pub kind: FindingKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FindingKind {
    /// The sidecar names the cell and it exists. Nothing to do.
    Resolved { to: String },
    /// No sidecar entry, but a cell of that name was found by searching the libraries.
    /// `fix` records it.
    Found { to: String },
    /// Not found, by sidecar or by search. `fix` cannot help.
    Missing { reason: String },
    /// The sidecar says the file is another cell (it was copied behind CDePT's back).
    /// `fix` makes it say what the file is.
    WrongIdentity { sidecar: String, actual: String },
    /// The sidecar has an entry for a name that is no longer a ghost in the file.
    /// `fix` drops it.
    Stale,
    /// The file has no sidecar. `fix` creates one.
    NoSidecar,
}

impl Finding {
    /// True when [`fix`] would change the sidecar because of this finding.
    pub fn fixable(&self) -> bool {
        matches!(
            self.kind,
            FindingKind::Found { .. }
                | FindingKind::WrongIdentity { .. }
                | FindingKind::Stale
                | FindingKind::NoSidecar
        )
    }

    /// True for a problem a sidecar change cannot solve.
    pub fn is_error(&self) -> bool {
        matches!(self.kind, FindingKind::Missing { .. })
    }
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let file = self
            .file
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let name = &self.name;
        match &self.kind {
            FindingKind::Resolved { to } => write!(f, "{file}: {name} -> {to}"),
            FindingKind::Found { to } => write!(f, "{file}: {name} -> {to} (found by name)"),
            FindingKind::Missing { reason } => {
                write!(f, "{file}: {name} cannot be found: {reason}")
            }
            FindingKind::WrongIdentity { sidecar, actual } => write!(
                f,
                "{file}: the sidecar says this is {sidecar}, but the file is {actual}"
            ),
            FindingKind::Stale => write!(f, "{file}: {name} is in the sidecar but not in the file"),
            FindingKind::NoSidecar => write!(f, "{file}: no sidecar"),
        }
    }
}

/// The layout views (GDS, OASIS) of `library/cell`, as `(view, file)`.
fn cell_layouts(project: &Project, library: &str, cell: &str) -> Result<Vec<(String, PathBuf)>> {
    let lib = project
        .library(library)
        .ok_or_else(|| eyre!("no library '{library}' in the project"))?;
    let c = lib
        .cell(cell)
        .ok_or_else(|| eyre!("no cell '{cell}' in library '{library}'"))?;
    let views: Vec<_> = c
        .entries
        .iter()
        .filter_map(|e| Some((e.name.clone(), layout_file(e)?.to_path_buf())))
        .filter(|(_, f)| super::is_readable(f))
        .collect();
    if views.is_empty() {
        bail!("cell '{library}/{cell}' has no GDS or OASIS layout view");
    }
    Ok(views)
}

/// Compare one layout file with its sidecar: the findings, and the sidecar it should
/// have (what `fix` writes).
fn examine(
    project: &Project,
    file: &Path,
    library: &str,
    cell: &str,
    view: &str,
) -> (Vec<Finding>, Meta) {
    let finding = |name: &str, kind| Finding {
        file: file.to_path_buf(),
        name: name.to_string(),
        kind,
    };
    let mut findings = Vec::new();
    let summary = match super::read_summary(file) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("{e:#}");
            gds::GdsSummary::default()
        }
    };
    let old = read_meta(file);
    let actual = label(library, cell, view);
    let mut meta = match &old {
        None => {
            findings.push(finding("", FindingKind::NoSidecar));
            Meta::new(library, cell, view, cell)
        }
        Some(m) => {
            if (&m.library, &m.cell, &m.view)
                != (&library.to_string(), &cell.to_string(), &view.to_string())
            {
                findings.push(finding(
                    "",
                    FindingKind::WrongIdentity {
                        sidecar: m.label(),
                        actual,
                    },
                ));
            }
            let mut m = m.clone();
            m.library = library.to_string();
            m.cell = cell.to_string();
            m.view = view.to_string();
            m
        }
    };
    // The top: keep it when the file defines it, else the file's only top cell.
    if !summary.defines(&meta.top) {
        match summary.top_cells().as_slice() {
            [only] => meta.top = only.clone(),
            _ if summary.defines(cell) => meta.top = cell.to_string(),
            _ => {}
        }
    }
    let ghosts: BTreeSet<String> = summary.ghosts().into_iter().collect();
    let mut references = BTreeMap::new();
    for name in &ghosts {
        let r = resolve_name(project, old.as_ref(), name);
        let kind = match r.how {
            How::Sidecar => FindingKind::Resolved { to: r.label },
            How::Search => FindingKind::Found { to: r.label },
            How::Missing => FindingKind::Missing { reason: r.reason },
        };
        findings.push(finding(name, kind));
        // A missing reference keeps what the sidecar knew about it.
        let kept = old.as_ref().and_then(|m| m.references.get(name)).cloned();
        match (r.meta, kept) {
            (Some(mut found), kept) => {
                found.bbox = found.bbox.or(kept.and_then(|k| k.bbox));
                references.insert(name.clone(), found);
            }
            (None, Some(kept)) => {
                references.insert(name.clone(), kept);
            }
            (None, None) => {}
        }
    }
    if let Some(old) = &old {
        for name in old.references.keys().filter(|n| !ghosts.contains(*n)) {
            findings.push(finding(name, FindingKind::Stale));
        }
    }
    meta.references = references;
    (findings, meta)
}

/// Check the sidecars of the GDS layout views of `library/cell` against the files and
/// the project: for every ghost name, how it resolves (sidecar, search, or not at all);
/// a sidecar that names another cell; entries that are no longer ghosts; a missing
/// sidecar. Changes nothing. Fails if the library or cell does not exist or the cell has
/// no GDS layout view. Used by `cdept layout refs <lib> <cell>`.
pub fn check(project: &Project, library: &str, cell: &str) -> Result<Vec<Finding>> {
    let mut all = Vec::new();
    for (view, file) in cell_layouts(project, library, cell)? {
        all.extend(examine(project, &file, library, cell, &view).0);
    }
    Ok(all)
}

/// Like [`check`], and write the sidecars it found wrong: creates a missing one,
/// records the cells found by search, fixes the identity (`library`/`cell`/`view`),
/// drops stale entries. Missing references keep their old entry. Sidecars with nothing
/// to fix are not rewritten. Returns the findings as [`check`] did before the fix. Used
/// by `cdept layout refs <lib> <cell> --fix`.
pub fn fix(project: &Project, library: &str, cell: &str) -> Result<Vec<Finding>> {
    let mut all = Vec::new();
    for (view, file) in cell_layouts(project, library, cell)? {
        let (findings, meta) = examine(project, &file, library, cell, &view);
        if findings.iter().any(Finding::fixable) {
            meta.write_for(&file)?;
        }
        all.extend(findings);
    }
    Ok(all)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::gds::tests::build;
    use crate::project::AddLibrary;
    use std::fs;

    /// Project with libraries `a` (inv, buf) and `b` (inv2); buf places inv and res
    /// (missing), inv2 places buf (the other library's cell, found by search).
    fn setup() -> (tempfile::TempDir, Project) {
        let tmp = tempfile::tempdir().unwrap();
        let mut project = Project::create(tmp.path(), Some("p")).unwrap();
        let options = AddLibrary {
            create: true,
            ..Default::default()
        };
        project.add_library("a", "libs/a", options).unwrap();
        project.add_library("b", "libs/b", options).unwrap();
        let write = |lib: &str, cell: &str, refs: &[&str]| {
            let dir = tmp.path().join(format!("libs/{lib}/{cell}/layout"));
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join(format!("{cell}.gds")), build(&[(cell, refs)])).unwrap();
        };
        write("a", "inv", &[]);
        write("a", "buf", &["inv", "res"]);
        write("b", "inv2", &["buf"]);
        project.rescan();
        (tmp, project)
    }

    fn gds(tmp: &tempfile::TempDir, lib: &str, cell: &str) -> PathBuf {
        tmp.path()
            .join(format!("libs/{lib}/{cell}/layout/{cell}.gds"))
    }

    #[test]
    fn resolves_by_search_without_sidecar_and_follows_closure() {
        let (tmp, project) = setup();
        let table = resolve(&project, &gds(&tmp, "b", "inv2"));
        assert_eq!(table.files.len(), 3);
        let root = &table.files[&gds(&tmp, "b", "inv2")];
        assert_eq!(root.label, "b/inv2/layout");
        assert_eq!(root.refs["buf"].file, Some(gds(&tmp, "a", "buf")));
        assert_eq!(root.refs["buf"].label, "a/buf/layout");
        let buf = &table.files[&gds(&tmp, "a", "buf")];
        assert_eq!(buf.refs["inv"].file, Some(gds(&tmp, "a", "inv")));
        let res = &buf.refs["res"];
        assert_eq!(res.file, None);
        assert_eq!(res.label, "res");
        assert!(!res.reason.is_empty());
        assert!(table.files[&gds(&tmp, "a", "inv")].refs.is_empty());
    }

    #[test]
    fn sidecar_entry_wins_and_unresolvable_entry_is_missing() {
        let (tmp, project) = setup();
        let buf = gds(&tmp, "a", "buf");
        let mut meta = Meta::new("a", "buf", "layout", "buf");
        let r = |lib: &str, cell: &str, view: &str| MetaRef {
            library: lib.into(),
            cell: cell.into(),
            view: view.into(),
            bbox: Some([0.0, 0.0, 2.0, 2.0]),
        };
        // "inv" is said to be b/inv2 (any cell will do to prove the sidecar wins).
        meta.references
            .insert("inv".into(), r("b", "inv2", "layout"));
        meta.references
            .insert("res".into(), r("a", "res", "layout"));
        meta.write_for(&buf).unwrap();
        let table = resolve(&project, &buf);
        let root = &table.files[&buf];
        assert_eq!(root.refs["inv"].file, Some(gds(&tmp, "b", "inv2")));
        let res = &root.refs["res"];
        assert_eq!(res.file, None);
        assert_eq!(res.label, "a/res/layout");
        assert_eq!(res.bbox, Some([0.0, 0.0, 2.0, 2.0]));
        assert!(res.reason.contains("no cell 'res'"), "{}", res.reason);
    }

    #[test]
    fn cycles_end() {
        let (tmp, project) = setup();
        // inv places buf, which places inv.
        fs::write(gds(&tmp, "a", "inv"), build(&[("inv", &["buf"])])).unwrap();
        let table = resolve(&project, &gds(&tmp, "a", "buf"));
        assert_eq!(table.files.len(), 2);
    }

    #[test]
    fn oasis_is_followed_and_unreadable_files_are_leaves() {
        let (tmp, mut project) = setup();
        let dir = tmp.path().join("libs/a/pad/layout");
        fs::create_dir_all(&dir).unwrap();
        // pad.oas places inv (a ghost there).
        fs::write(
            dir.join("pad.oas"),
            super::super::oasis::tests::build(&[("pad", &["inv"])], true, false),
        )
        .unwrap();
        fs::write(gds(&tmp, "a", "inv"), "garbage").unwrap();
        project.rescan();
        let table = resolve(&project, &gds(&tmp, "a", "buf"));
        assert!(table.files[&gds(&tmp, "a", "inv")].refs.is_empty());
        // A cell found only as OASIS is used.
        fs::write(gds(&tmp, "a", "buf"), build(&[("buf", &["pad"])])).unwrap();
        let table = resolve(&project, &gds(&tmp, "a", "buf"));
        let pad = dir.join("pad.oas");
        assert_eq!(
            table.files[&gds(&tmp, "a", "buf")].refs["pad"].file,
            Some(pad.clone())
        );
        assert_eq!(
            table.files[&pad].refs["inv"].file,
            Some(gds(&tmp, "a", "inv"))
        );
        // Garbage in an OASIS file: a leaf.
        fs::write(&pad, "x").unwrap();
        let table = resolve(&project, &gds(&tmp, "a", "buf"));
        assert!(table.files[&pad].refs.is_empty());
    }

    #[test]
    fn search_prefers_gds_and_the_layout_view() {
        let (tmp, mut project) = setup();
        let dir = tmp.path().join("libs/b/inv/layout_oas");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("inv.oas"), "x").unwrap();
        // Library `a` comes first in the project: its inv wins over b's.
        project.rescan();
        let table = resolve(&project, &gds(&tmp, "a", "buf"));
        assert_eq!(
            table.files[&gds(&tmp, "a", "buf")].refs["inv"].file,
            Some(gds(&tmp, "a", "inv"))
        );
    }

    #[test]
    fn update_after_save_writes_and_prunes() {
        let (tmp, project) = setup();
        let buf = gds(&tmp, "a", "buf");
        let mut report = SaveReport {
            file: buf.clone(),
            top: "buf".into(),
            ..Default::default()
        };
        report.references.insert(
            "inv".into(),
            SavedRef {
                file: Some(gds(&tmp, "a", "inv")),
                missing: false,
                bbox: Some([0.0, 0.0, 1.0, 2.0]),
            },
        );
        report.references.insert(
            "res".into(),
            SavedRef {
                file: None,
                missing: true,
                bbox: Some([0.0, 0.0, 2.0, 2.0]),
            },
        );
        // No sidecar yet: identity comes from the project; `res` is unknown, so skipped.
        update_after_save(&project, &report).unwrap();
        let meta = Meta::read_for(&buf).unwrap().unwrap();
        assert_eq!(meta.label(), "a/buf/layout");
        assert_eq!(meta.top, "buf");
        assert_eq!(meta.references.len(), 1);
        let inv = &meta.references["inv"];
        assert_eq!((inv.library.as_str(), inv.cell.as_str()), ("a", "inv"));
        assert_eq!(inv.bbox, Some([0.0, 0.0, 1.0, 2.0]));

        // Second save: `inv` gone, `res` known from a hand-added entry stays with new bbox.
        let mut meta = meta;
        meta.top = "buf_top".into();
        meta.references.insert(
            "res".into(),
            MetaRef {
                library: "a".into(),
                cell: "res".into(),
                view: "layout".into(),
                bbox: None,
            },
        );
        meta.write_for(&buf).unwrap();
        report.references.remove("inv");
        update_after_save(&project, &report).unwrap();
        let meta = Meta::read_for(&buf).unwrap().unwrap();
        assert_eq!(meta.top, "buf_top");
        assert_eq!(meta.references.keys().collect::<Vec<_>>(), ["res"]);
        assert_eq!(meta.references["res"].bbox, Some([0.0, 0.0, 2.0, 2.0]));
    }

    #[test]
    fn a_save_that_wrote_an_external_cell_too_updates_both_sidecars() {
        let (tmp, project) = setup();
        let buf = gds(&tmp, "a", "buf");
        let inv = gds(&tmp, "a", "inv");
        let mut report = SaveReport {
            file: buf.clone(),
            top: "buf".into(),
            also: vec![SaveReport {
                file: inv.clone(),
                top: "inv".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        report.references.insert(
            "inv".into(),
            SavedRef {
                file: Some(inv.clone()),
                missing: false,
                bbox: Some([0.0, 0.0, 1.0, 2.0]),
            },
        );
        assert_eq!(report.files(), [buf.as_path(), inv.as_path()]);
        update_after_save(&project, &report).unwrap();
        assert_eq!(Meta::read_for(&buf).unwrap().unwrap().references.len(), 1);
        let meta = Meta::read_for(&inv).unwrap().unwrap();
        assert_eq!(meta.label(), "a/inv/layout");
        assert!(meta.references.is_empty());
        // The table tells the helper what may be edited in place.
        let table = resolve(&project, &buf);
        assert!(!table.files[&inv].read_only);
    }

    #[test]
    fn update_after_save_ignores_unknown_files() {
        let (tmp, project) = setup();
        let outside = tmp.path().join("loose.gds");
        fs::write(&outside, build(&[("x", &[])])).unwrap();
        let report = SaveReport {
            file: outside.clone(),
            ..Default::default()
        };
        update_after_save(&project, &report).unwrap();
        assert!(!super::super::meta::sidecar_path(&outside).exists());
    }

    #[test]
    fn check_and_fix() {
        let (tmp, project) = setup();
        let buf = gds(&tmp, "a", "buf");
        let findings = check(&project, "a", "buf").unwrap();
        let kinds: Vec<_> = findings
            .iter()
            .map(|f| (f.name.as_str(), &f.kind))
            .collect();
        assert!(matches!(kinds[0], ("", FindingKind::NoSidecar)));
        assert!(matches!(kinds[1], ("inv", FindingKind::Found { to }) if to == "a/inv/layout"));
        assert!(matches!(kinds[2], ("res", FindingKind::Missing { .. })));
        assert!(findings.iter().any(Finding::fixable));
        assert!(findings.iter().any(Finding::is_error));
        // check changes nothing.
        assert!(Meta::read_for(&buf).unwrap().is_none());

        fix(&project, "a", "buf").unwrap();
        let meta = Meta::read_for(&buf).unwrap().unwrap();
        assert_eq!(meta.label(), "a/buf/layout");
        assert_eq!(meta.top, "buf");
        assert_eq!(meta.references.keys().collect::<Vec<_>>(), ["inv"]);
        let again = check(&project, "a", "buf").unwrap();
        assert!(matches!(again[0].kind, FindingKind::Resolved { .. }));
        assert!(!again.iter().any(Finding::fixable));
        assert!(again[1].to_string().contains("cannot be found"));

        // Copied behind our back: the sidecar names another cell; a stale entry too.
        let copy = tmp.path().join("libs/a/buf2/layout");
        fs::create_dir_all(&copy).unwrap();
        fs::copy(&buf, copy.join("buf2.gds")).unwrap();
        let mut stale = meta.clone();
        stale
            .references
            .insert("gone".into(), stale.references["inv"].clone());
        stale.write_for(&copy.join("buf2.gds")).unwrap();
        let mut project = project;
        project.rescan();
        let findings = check(&project, "a", "buf2").unwrap();
        assert!(findings.iter().any(|f| matches!(&f.kind,
            FindingKind::WrongIdentity { sidecar, actual }
            if sidecar == "a/buf/layout" && actual == "a/buf2/layout")));
        assert!(
            findings
                .iter()
                .any(|f| f.name == "gone" && f.kind == FindingKind::Stale)
        );
        fix(&project, "a", "buf2").unwrap();
        let m = Meta::read_for(&copy.join("buf2.gds")).unwrap().unwrap();
        assert_eq!(m.label(), "a/buf2/layout");
        assert_eq!(m.top, "buf");
        assert!(!m.references.contains_key("gone"));
        assert!(
            !check(&project, "a", "buf2")
                .unwrap()
                .iter()
                .any(Finding::fixable)
        );
    }

    #[test]
    fn rename_and_copy_keep_references_working() {
        let (tmp, mut project) = setup();
        fix(&project, "a", "buf").unwrap();
        let lib = project.library("a").unwrap().clone();
        lib.copy_cell("buf", project.library("b").unwrap(), "buf_b")
            .unwrap();
        lib.rename_cell("buf", "buf_r").unwrap();
        project.rescan();
        let renamed = tmp.path().join("libs/a/buf_r/layout/buf_r.gds");
        let meta = Meta::read_for(&renamed).unwrap().unwrap();
        assert_eq!(
            (meta.label().as_str(), meta.top.as_str()),
            ("a/buf_r/layout", "buf_r")
        );
        // The copy, in library b, resolves `inv` through the copied sidecar.
        let copy = tmp.path().join("libs/b/buf_b/layout/buf_b.gds");
        assert_eq!(gds::read_structure(&copy).unwrap().top_cells(), ["buf_b"]);
        let table = resolve(&project, &copy);
        assert_eq!(table.files[&copy].label, "b/buf_b/layout");
        assert_eq!(
            table.files[&copy].refs["inv"].file,
            Some(gds(&tmp, "a", "inv"))
        );
        assert_eq!(table.files[&copy].refs["res"].file, None);
    }

    #[test]
    fn check_errors() {
        let (_tmp, project) = setup();
        assert!(check(&project, "zzz", "buf").is_err());
        assert!(check(&project, "a", "zzz").is_err());
    }
}
