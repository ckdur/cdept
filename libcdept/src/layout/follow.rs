//! Making the layout views that place a cell follow when that cell is renamed (or its
//! library is), and finding the views that place a deleted one (docs/KLAYOUT.md §3).
//!
//! These take the project as it was *before* the change (not rescanned yet): the
//! views placing a cell by name without a sidecar entry are found by the same search
//! the resolver does, which needs the old names.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

use super::meta::{Meta, MetaRef};
use super::refs::{How, layout_file, resolve_name};
use crate::{Cell, Entry, Library, Project};

/// What happened to one layout view that places the cell.
#[derive(Debug, Clone, PartialEq)]
pub struct Followed {
    /// The layout file that places it.
    pub file: PathBuf,
    /// `library/cell/view` of that file.
    pub label: String,
    pub kind: FollowedKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FollowedKind {
    /// The sidecar names the new cell. `structure`: the structure in the file was
    /// renamed too (old, new); `None` when it keeps its name (it was not the cell
    /// name, or the new name is already used in the file).
    Updated { structure: Option<(String, String)> },
    /// The view is in a read-only (or unusable) library and still names the old cell;
    /// it will show "cannot be found". `cdept layout refs --fix` cannot change it either.
    ReadOnly { library: String },
    /// The view could not be rewritten.
    Failed { error: String },
    /// The view places (as `name`) a cell or view that was deleted.
    Orphaned { name: String },
}

impl Followed {
    /// True for a view that is now (or still) wrong.
    pub fn is_problem(&self) -> bool {
        !matches!(self.kind, FollowedKind::Updated { .. })
    }
}

impl fmt::Display for Followed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = &self.label;
        match &self.kind {
            FollowedKind::Updated {
                structure: Some((old, new)),
            } => write!(f, "{label}: updated (structure {old} renamed to {new})"),
            FollowedKind::Updated { structure: None } => write!(f, "{label}: updated"),
            FollowedKind::ReadOnly { library } => write!(
                f,
                "{label}: not updated, library '{library}' is read-only; it now places a \
                 cell that cannot be found"
            ),
            FollowedKind::Failed { error } => write!(f, "{label}: not updated: {error}"),
            FollowedKind::Orphaned { name } => {
                write!(f, "{label}: places {name}, which now cannot be found")
            }
        }
    }
}

/// A layout view of the project.
#[derive(Clone, Copy)]
struct View<'a> {
    lib: &'a Library,
    cell: &'a Cell,
    entry: &'a Entry,
    file: &'a Path,
}

impl View<'_> {
    fn label(&self) -> String {
        format!("{}/{}/{}", self.lib.name, self.cell.name, self.entry.name)
    }
}

/// Every layout view of the project that exists and can be read.
fn layout_views(project: &Project) -> Vec<View<'_>> {
    let mut out = Vec::new();
    for lib in &project.libraries {
        for cell in &lib.cells {
            for entry in &cell.entries {
                if let Some(file) = layout_file(entry)
                    && super::is_readable(file)
                    && file.is_file()
                {
                    out.push(View {
                        lib,
                        cell,
                        entry,
                        file,
                    });
                }
            }
        }
    }
    out
}

/// Which ghost names of a file place a cell matching `wanted`, with the reference each
/// stands for (from the sidecar, or found by search when it has no entry).
fn placing(
    project: &Project,
    ghosts: &[String],
    meta: Option<&Meta>,
    wanted: &dyn Fn(&MetaRef) -> bool,
) -> Vec<(String, MetaRef)> {
    let mut out = Vec::new();
    for name in ghosts {
        let r = match meta.and_then(|m| m.references.get(name)) {
            Some(r) => Some(r.clone()),
            None => {
                let found = resolve_name(project, meta, name);
                (found.how == How::Search).then_some(found.meta).flatten()
            }
        };
        if let Some(r) = r.filter(|r| wanted(r)) {
            out.push((name.clone(), r));
        }
    }
    out
}

/// One layout view that places the moved cell: rewrite its sidecar (and the structure
/// names in the file). `to_cell`: the new cell name (`None` for a library rename).
fn retarget_view(
    view: View<'_>,
    summary: &super::gds::GdsSummary,
    meta: Option<Meta>,
    hits: Vec<(String, MetaRef)>,
    to_library: &str,
    to_cell: Option<&str>,
) -> Vec<Followed> {
    let View {
        lib,
        cell,
        entry,
        file,
    } = view;
    let label = view.label();
    let result = |kind| Followed {
        file: file.to_path_buf(),
        label: label.clone(),
        kind,
    };
    if lib.read_only || lib.error.is_some() {
        return vec![result(FollowedKind::ReadOnly {
            library: lib.name.clone(),
        })];
    }
    let mut meta = meta.unwrap_or_else(|| {
        let top = match summary.top_cells().as_slice() {
            [only] => only.clone(),
            _ => cell.name.clone(),
        };
        Meta::new(&lib.name, &cell.name, &entry.name, &top)
    });
    // Names already used in the file: a structure is renamed only to a free name.
    let mut used: BTreeSet<String> = summary.structures.iter().map(|s| s.name.clone()).collect();
    used.extend(summary.ghosts());
    let mut renames = BTreeMap::new();
    let mut entries = Vec::new();
    for (name, mut r) in hits {
        let old_cell = r.cell.clone();
        r.library = to_library.to_string();
        if let Some(new) = to_cell {
            r.cell = new.to_string();
            if name == old_cell && !used.contains(new) {
                used.insert(new.to_string());
                renames.insert(name.clone(), new.to_string());
            }
        }
        meta.references.remove(&name);
        entries.push((name, r));
    }
    let mut out = Vec::new();
    if !renames.is_empty()
        && let Err(e) = super::rename_in_file(file, &renames)
    {
        out.push(result(FollowedKind::Failed {
            error: format!("{e:#}; the structure keeps its name"),
        }));
        renames.clear();
    }
    for (name, r) in entries {
        let key = renames.get(&name).cloned().unwrap_or(name);
        meta.references.insert(key, r);
    }
    match meta.write_for(file) {
        Ok(()) => {
            let structure = renames.into_iter().next();
            out.push(result(FollowedKind::Updated { structure }));
        }
        Err(e) => out.push(result(FollowedKind::Failed {
            error: format!("{e:#}"),
        })),
    }
    out
}

/// Retarget every view placing a cell `wanted` picks.
fn follow(
    project: &Project,
    wanted: &dyn Fn(&MetaRef) -> bool,
    to_library: &dyn Fn(&MetaRef) -> String,
    to_cell: Option<&str>,
) -> Vec<Followed> {
    let mut out = Vec::new();
    for view in layout_views(project) {
        let summary = match super::read_summary(view.file) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("{e:#}");
                continue;
            }
        };
        let meta = match Meta::read_for(view.file) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!("{e:#}");
                continue;
            }
        };
        let hits = placing(project, &summary.ghosts(), meta.as_ref(), wanted);
        let Some((_, first)) = hits.first() else {
            continue;
        };
        let to_lib = to_library(first);
        out.extend(retarget_view(view, &summary, meta, hits, &to_lib, to_cell));
    }
    out
}

/// After cell `old` of `library` was renamed to `new` (same library): every layout view
/// placing it gets its sidecar entry pointed at `library/new`, and the structure in the
/// file is renamed from `old` to `new` when it had the cell's name (the sidecar is
/// re-keyed). Views without a sidecar that found the cell by name get one. `project`
/// is the project before the rename (not rescanned).
pub fn cell_renamed(project: &Project, library: &str, old: &str, new: &str) -> Vec<Followed> {
    follow(
        project,
        &|r| r.library == library && r.cell == old,
        &|_| library.to_string(),
        Some(new),
    )
}

/// After library `old` was renamed to `new`: the sidecar entries naming it name `new`.
/// Structure names do not change. `project` is the project before the rename.
pub fn library_renamed(project: &Project, old: &str, new: &str) -> Vec<Followed> {
    follow(project, &|r| r.library == old, &|_| new.to_string(), None)
}

/// The layout views placing `library/cell` (any of its views, or only `view`), as
/// [`FollowedKind::Orphaned`]: what a deletion leaves without its cell. Changes
/// nothing; sidecar entries are kept so the "cannot be found" marker keeps its size and
/// a cell made again under that name is found again.
pub fn placed_by(
    project: &Project,
    library: &str,
    cell: &str,
    view: Option<&str>,
) -> Vec<Followed> {
    let mut out = Vec::new();
    for v in layout_views(project) {
        if v.lib.name == library && v.cell.name == cell {
            continue;
        }
        let Ok(summary) = super::read_summary(v.file) else {
            continue;
        };
        let meta = Meta::read_for(v.file).ok().flatten();
        let wanted = |r: &MetaRef| {
            r.library == library && r.cell == cell && view.is_none_or(|v| r.view == v)
        };
        for (name, _) in placing(project, &summary.ghosts(), meta.as_ref(), &wanted) {
            out.push(Followed {
                file: v.file.to_path_buf(),
                label: v.label(),
                kind: FollowedKind::Orphaned { name },
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::layout::gds::tests::build;
    use crate::layout::oasis;

    /// Libraries `a` (buf places inv, top places buf and inv) and read-only `ro`
    /// (chip places inv), all with sidecars except `a/top`, and `a/pad` in OASIS
    /// placing inv.
    fn setup() -> (tempfile::TempDir, Project) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let file = |lib: &str, cell: &str, ext: &str| {
            let dir = root.join(format!("libs/{lib}/{cell}/layout"));
            fs::create_dir_all(&dir).unwrap();
            dir.join(format!("{cell}.{ext}"))
        };
        let inv_ref = || MetaRef {
            library: "a".into(),
            cell: "inv".into(),
            view: "layout".into(),
            bbox: Some([0.0, 0.0, 1.0, 1.0]),
        };
        fs::write(file("a", "inv", "gds"), build(&[("inv", &[])])).unwrap();
        let buf = file("a", "buf", "gds");
        fs::write(&buf, build(&[("buf", &["inv"])])).unwrap();
        let mut m = Meta::new("a", "buf", "layout", "buf");
        m.references.insert("inv".into(), inv_ref());
        m.write_for(&buf).unwrap();
        fs::write(file("a", "top", "gds"), build(&[("top", &["buf", "inv"])])).unwrap();
        let pad = file("a", "pad", "oas");
        fs::write(&pad, oasis::tests::build(&[("pad", &["inv"])], true, false)).unwrap();
        let chip = file("ro", "chip", "gds");
        fs::write(&chip, build(&[("chip", &["inv"])])).unwrap();
        let mut m = Meta::new("ro", "chip", "layout", "chip");
        m.references.insert("inv".into(), inv_ref());
        m.write_for(&chip).unwrap();
        let mut project = Project::create(root, Some("t")).unwrap();
        let options = crate::AddLibrary {
            create: true,
            ..Default::default()
        };
        project.add_library("a", "libs/a", options).unwrap();
        let read_only = crate::AddLibrary {
            read_only: true,
            ..options
        };
        project.add_library("ro", "libs/ro", read_only).unwrap();
        project.rescan();
        (tmp, project)
    }

    #[test]
    fn rename_rewrites_every_view_that_places_the_cell() {
        let (tmp, project) = setup();
        let root = tmp.path();
        let lib = project.library("a").unwrap();
        lib.rename_cell("inv", "inv2").unwrap();
        let done = cell_renamed(&project, "a", "inv", "inv2");
        let mut labels: Vec<String> = done.iter().map(ToString::to_string).collect();
        labels.sort();
        assert_eq!(
            labels,
            [
                "a/buf/layout: updated (structure inv renamed to inv2)",
                "a/pad/layout: updated (structure inv renamed to inv2)",
                "a/top/layout: updated (structure inv renamed to inv2)",
                "ro/chip/layout: not updated, library 'ro' is read-only; it now places a \
                 cell that cannot be found",
            ]
        );
        for (cell, ext) in [("buf", "gds"), ("top", "gds"), ("pad", "oas")] {
            let file = root.join(format!("libs/a/{cell}/layout/{cell}.{ext}"));
            let summary = crate::layout::read_summary(&file).unwrap();
            assert!(summary.ghosts().contains(&"inv2".to_string()), "{cell}");
            let meta = Meta::read_for(&file).unwrap().unwrap();
            let r = &meta.references["inv2"];
            assert_eq!((r.library.as_str(), r.cell.as_str()), ("a", "inv2"));
            assert!(!meta.references.contains_key("inv"));
        }
        // The sidecar of buf kept the bounding box; top got a sidecar.
        let buf = Meta::read_for(&root.join("libs/a/buf/layout/buf.gds"))
            .unwrap()
            .unwrap();
        assert_eq!(buf.references["inv2"].bbox, Some([0.0, 0.0, 1.0, 1.0]));
        let top = Meta::read_for(&root.join("libs/a/top/layout/top.gds"))
            .unwrap()
            .unwrap();
        assert_eq!(top.label(), "a/top/layout");
        assert_eq!(top.top, "top");
        // top's reference to buf was not touched.
        assert!(!top.references.contains_key("buf"));
        // The read-only view still names inv.
        let chip = Meta::read_for(&root.join("libs/ro/chip/layout/chip.gds"))
            .unwrap()
            .unwrap();
        assert_eq!(chip.references["inv"].cell, "inv");
        // Everything resolves after a rescan, except the read-only view.
        let mut project = project;
        project.rescan();
        let table = crate::layout::refs::resolve(&project, &root.join("libs/a/top/layout/top.gds"));
        assert!(
            table
                .files
                .values()
                .all(|f| f.refs.values().all(|r| r.file.is_some()))
        );
    }

    #[test]
    fn a_used_name_keeps_the_structure_name() {
        let (tmp, project) = setup();
        let buf = tmp.path().join("libs/a/buf/layout/buf.gds");
        // buf already has a structure named inv2.
        fs::write(&buf, build(&[("buf", &["inv"]), ("inv2", &[])])).unwrap();
        project
            .library("a")
            .unwrap()
            .rename_cell("inv", "inv2")
            .unwrap();
        let done = cell_renamed(&project, "a", "inv", "inv2");
        let buf_done = done.iter().find(|f| f.file == buf).unwrap();
        assert_eq!(buf_done.kind, FollowedKind::Updated { structure: None });
        let meta = Meta::read_for(&buf).unwrap().unwrap();
        assert_eq!(meta.references["inv"].cell, "inv2");
    }

    #[test]
    fn library_rename_rewrites_sidecars_only() {
        let (tmp, project) = setup();
        let done = library_renamed(&project, "a", "logic");
        assert_eq!(done.iter().filter(|f| !f.is_problem()).count(), 3);
        let buf = tmp.path().join("libs/a/buf/layout/buf.gds");
        let meta = Meta::read_for(&buf).unwrap().unwrap();
        assert_eq!(meta.references["inv"].library, "logic");
        assert!(
            crate::layout::read_summary(&buf)
                .unwrap()
                .ghosts()
                .contains(&"inv".to_string())
        );
    }

    #[test]
    fn deleting_reports_the_views_left_without_their_cell() {
        let (_tmp, project) = setup();
        let mut labels: Vec<String> = placed_by(&project, "a", "inv", None)
            .iter()
            .map(|f| f.label.clone())
            .collect();
        labels.sort();
        assert_eq!(
            labels,
            [
                "a/buf/layout",
                "a/pad/layout",
                "a/top/layout",
                "ro/chip/layout"
            ]
        );
        assert!(placed_by(&project, "a", "inv", Some("other")).is_empty());
        assert!(placed_by(&project, "a", "top", None).is_empty());
    }

    #[test]
    fn fix_repairs_a_library_renamed_behind_cdepts_back() {
        let (tmp, project) = setup();
        let buf = tmp.path().join("libs/a/buf/layout/buf.gds");
        let mut meta = Meta::read_for(&buf).unwrap().unwrap();
        meta.references.get_mut("inv").unwrap().library = "old_name".into();
        meta.write_for(&buf).unwrap();
        // Resolves by searching the cell name, and fix records the new place.
        let table = crate::layout::refs::resolve(&project, &buf);
        assert!(table.files[&buf].refs["inv"].file.is_some());
        let findings = crate::layout::refs::fix(&project, "a", "buf").unwrap();
        assert!(findings.iter().any(|f| f.fixable()), "{findings:?}");
        let meta = Meta::read_for(&buf).unwrap().unwrap();
        assert_eq!(meta.references["inv"].library, "a");
        assert_eq!(meta.references["inv"].bbox, Some([0.0, 0.0, 1.0, 1.0]));
        // A cell renamed behind CDePT's back cannot be guessed: missing.
        let mut project = project;
        fs::rename(
            tmp.path().join("libs/a/inv"),
            tmp.path().join("libs/a/inv_x"),
        )
        .unwrap();
        project.rescan();
        let findings = crate::layout::refs::check(&project, "a", "buf").unwrap();
        assert!(findings.iter().any(|f| f.is_error()), "{findings:?}");
    }
}
