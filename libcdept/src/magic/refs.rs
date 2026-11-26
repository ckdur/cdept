//! References between Magic layout views of the project (docs/MAGIC.md, "Libraries").
//!
//! A `.mag` view places other cells with `use` lines. CDePT keeps two things right:
//!
//! - the **path** on each `use` line of a cell of the project is the folder of that
//!   cell's view, *relative* to the folder of the placing file (Magic reads relative
//!   paths that way), so a project can be moved or cloned. Magic itself writes
//!   absolute paths (with `~`) on save: [`normalize`] turns them back;
//! - the **sidecar** `<cell>.mag.json` (the `cdept-layout` schema of GDS views,
//!   [`crate::layout::meta`]) records, for every placed cell of the project, which
//!   library/cell/view it is. Renaming a cell or library ([`cell_renamed`],
//!   [`library_renamed`]) and copying a view to another folder ([`retarget`]) use it
//!   to rewrite the `use` lines.
//!
//! A placed cell is found by, in order: its sidecar entry, the path of its `use` line,
//! a project cell of that name with a Magic view. Cells of the PDK (paths with
//! `$PDKPATH`, `$PDK_ROOT`) are left alone. A cell found by none of these is
//! [`Target::Missing`].

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use eyre::{Result, WrapErr};

use super::mag::{self, Use, UseEdit};
use crate::entry::Format;
use crate::layout::follow::{Followed, FollowedKind};
use crate::layout::meta::{Meta, MetaRef};
use crate::{Library, Project};

/// A Magic layout view of the project.
#[derive(Debug, Clone, PartialEq)]
pub struct MagView {
    pub library: String,
    pub cell: String,
    pub view: String,
    pub file: PathBuf,
    pub read_only: bool,
}

impl MagView {
    pub fn label(&self) -> String {
        format!("{}/{}/{}", self.library, self.cell, self.view)
    }
}

/// Every Magic view of the project (members of other files excluded).
pub fn mag_views(project: &Project) -> Vec<MagView> {
    let mut out = Vec::new();
    for lib in project.libraries.iter().filter(|l| l.error.is_none()) {
        for cell in &lib.cells {
            for entry in cell.entries.iter().filter(|e| !e.is_member()) {
                if entry.format == Some(Format::Magic)
                    && let Some(file) = &entry.main_file
                {
                    out.push(MagView {
                        library: lib.name.clone(),
                        cell: cell.name.clone(),
                        view: entry.name.clone(),
                        file: file.clone(),
                        read_only: lib.read_only,
                    });
                }
            }
        }
    }
    out
}

/// The folders holding Magic views of the project's own (writable) libraries, each
/// once: Magic's search path (`addpath`) for cells placed without a usable path. The
/// PDK's libraries are on the path the PDK's start-up file sets.
pub fn search_dirs(project: &Project) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    for v in mag_views(project).into_iter().filter(|v| !v.read_only) {
        if let Some(dir) = v.file.parent()
            && !dirs.iter().any(|d| d == dir)
        {
            dirs.push(dir.to_path_buf());
        }
    }
    dirs
}

/// Where a placed cell is.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    /// A Magic view of the project.
    Project(MagView),
    /// A file outside the project (the PDK's cells).
    External(PathBuf),
    /// Not found.
    Missing,
}

/// How a placed cell was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum How {
    Sidecar,
    Path,
    Name,
    Missing,
}

/// One `use` line and what it places.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    pub placed: Use,
    pub target: Target,
    pub how: How,
}

/// Variables `use` paths may name: the project's tool environment, and `$PDKPATH`
/// (`$PDK_ROOT/$PDK`) as Magic sets it.
fn path_vars(project: &Project) -> Vec<(String, String)> {
    let mut vars: Vec<(String, String)> = project
        .tool_env()
        .into_iter()
        .map(|(k, v)| (k.to_string_lossy().into_owned(), v.to_string_lossy().into_owned()))
        .collect();
    let root = vars.iter().find(|(k, _)| k == "PDK_ROOT").map(|(_, v)| v.clone());
    let pdk = vars.iter().find(|(k, _)| k == "PDK").map(|(_, v)| v.clone());
    if let (Some(root), Some(pdk)) = (root, pdk)
        && !vars.iter().any(|(k, _)| k == "PDKPATH")
    {
        vars.push(("PDKPATH".into(), format!("{root}/{pdk}")));
    }
    vars
}

fn canonical(p: &Path) -> PathBuf {
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
}

fn view_of_file<'a>(views: &'a [MagView], file: &Path) -> Option<&'a MagView> {
    let file = canonical(file);
    views.iter().find(|v| canonical(&v.file) == file)
}

/// The Magic view of `library/cell` (view `view`, else `layout`, else the first).
fn find_view<'a>(views: &'a [MagView], library: &str, cell: &str, view: Option<&str>) -> Option<&'a MagView> {
    let mut of_cell = views.iter().filter(|v| v.library == library && v.cell == cell);
    let all: Vec<&MagView> = of_cell.by_ref().collect();
    view.and_then(|w| all.iter().find(|v| v.view == w).copied())
        .or_else(|| all.iter().find(|v| v.view == "layout").copied())
        .or_else(|| all.first().copied())
}

/// The Magic view whose file is `<name>.mag`, searching the placing file's own library
/// first.
fn find_by_name<'a>(views: &'a [MagView], own_library: Option<&str>, name: &str) -> Option<&'a MagView> {
    let named = |v: &&MagView| v.file.file_stem().is_some_and(|s| s == name);
    views
        .iter()
        .filter(named)
        .find(|v| Some(v.library.as_str()) == own_library)
        .or_else(|| views.iter().find(named))
}

fn resolve_with(
    project: &Project,
    views: &[MagView],
    file: &Path,
    mag_file: &mag::MagFile,
    meta: Option<&Meta>,
) -> Vec<Resolved> {
    let parent = file.parent().unwrap_or(Path::new("."));
    let vars = path_vars(project);
    let own = view_of_file(views, file).map(|v| v.library.clone());
    mag_file
        .uses
        .iter()
        .map(|u| {
            // 1. the sidecar
            if let Some(r) = meta.and_then(|m| m.references.get(&u.cell))
                && let Some(v) = find_view(views, &r.library, &r.cell, Some(&r.view))
            {
                return Resolved {
                    placed: u.clone(),
                    target: Target::Project(v.clone()),
                    how: How::Sidecar,
                };
            }
            // 2. the path of the use line (none: the placing file's folder, where Magic
            //    looks first; generated devices are written there)
            let dir = if u.path.is_empty() {
                Some(parent.to_path_buf())
            } else {
                mag::use_dir(&u.path, parent, &vars)
            };
            if let Some(dir) = dir {
                let candidate = dir.join(format!("{}.mag", u.cell));
                if candidate.is_file() {
                    let target = match view_of_file(views, &candidate) {
                        Some(v) => Target::Project(v.clone()),
                        None => Target::External(candidate),
                    };
                    return Resolved {
                        placed: u.clone(),
                        target,
                        how: How::Path,
                    };
                }
            }
            // 3. a project cell of that name (a sidecar entry that no longer resolves
            //    is not used for this: the cell was deleted or renamed outside CDePT)
            let sidecar_named = meta.is_some_and(|m| m.references.contains_key(&u.cell));
            if !sidecar_named && let Some(v) = find_by_name(views, own.as_deref(), &u.cell) {
                return Resolved {
                    placed: u.clone(),
                    target: Target::Project(v.clone()),
                    how: How::Name,
                };
            }
            Resolved {
                placed: u.clone(),
                target: Target::Missing,
                how: How::Missing,
            }
        })
        .collect()
}

/// What the `use` lines of `file` place.
pub fn resolve_file(project: &Project, file: &Path) -> Result<Vec<Resolved>> {
    let mag_file = mag::read(file)?;
    let meta = Meta::read_for(file).ok().flatten();
    let views = mag_views(project);
    Ok(resolve_with(project, &views, file, &mag_file, meta.as_ref()))
}

/// What [`normalize`] did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Normalized {
    /// `use` lines whose path was rewritten.
    pub paths: usize,
    /// The sidecar was written.
    pub sidecar: bool,
    /// Cells placed that cannot be found: (instance, cell).
    pub missing: Vec<(String, String)>,
    pub warnings: Vec<String>,
}

/// After `file` was written (by Magic): make the `use` paths of project cells relative
/// and record every placed project cell in the sidecar. The file's own identity in the
/// sidecar is `library/cell/view` of the view it is (when it is one).
pub fn normalize(project: &Project, file: &Path) -> Result<Normalized> {
    let views = mag_views(project);
    let mag_file = mag::read(file)?;
    let old_meta = Meta::read_for(file).ok().flatten();
    let resolved = resolve_with(project, &views, file, &mag_file, old_meta.as_ref());
    let parent = canonical(file.parent().unwrap_or(Path::new(".")));
    let mut out = Normalized {
        warnings: mag_file.warnings.clone(),
        ..Default::default()
    };
    // The path each project cell should have.
    let mut wanted: BTreeMap<String, String> = BTreeMap::new();
    for r in &resolved {
        match &r.target {
            Target::Project(v) => {
                let dir = canonical(v.file.parent().unwrap_or(Path::new(".")));
                let rel = mag::relative_dir(&parent, &dir);
                wanted.insert(r.placed.cell.clone(), rel.to_string_lossy().into_owned());
            }
            Target::Missing => out
                .missing
                .push((r.placed.instance.clone(), r.placed.cell.clone())),
            Target::External(_) => {}
        }
    }
    let writable = view_of_file(&views, file).is_none_or(|v| !v.read_only);
    if writable && !wanted.is_empty() {
        let changed = std::cell::Cell::new(0usize);
        mag::rewrite_file(file, |u| {
            let path = wanted.get(&u.cell)?;
            if *path != u.path {
                changed.set(changed.get() + 1);
            }
            Some(UseEdit {
                cell: None,
                path: Some(path.clone()),
            })
        })?;
        out.paths = changed.get();
    }
    // The sidecar.
    let me = view_of_file(&views, file);
    let stem = file
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut meta = match (&old_meta, me) {
        (Some(m), _) => m.clone(),
        (None, Some(v)) => Meta::new(&v.library, &v.cell, &v.view, &stem),
        (None, None) => Meta::new("", &stem, "layout", &stem),
    };
    if let Some(v) = me {
        meta.library = v.library.clone();
        meta.cell = v.cell.clone();
        meta.view = v.view.clone();
    }
    meta.top = stem;
    let mut refs: BTreeMap<String, MetaRef> = BTreeMap::new();
    for r in &resolved {
        match &r.target {
            Target::Project(v) => {
                let scale = 1.0;
                let bbox = r.placed.bbox.map(|b| b.map(|x| x as f64 * scale));
                refs.insert(
                    r.placed.cell.clone(),
                    MetaRef {
                        library: v.library.clone(),
                        cell: v.cell.clone(),
                        view: v.view.clone(),
                        bbox,
                    },
                );
            }
            // Keep what the sidecar knew of a missing cell: a cell made again under
            // that name is found again, and the marker keeps its size.
            Target::Missing => {
                if let Some(old) = old_meta.as_ref().and_then(|m| m.references.get(&r.placed.cell)) {
                    refs.insert(r.placed.cell.clone(), old.clone());
                }
            }
            Target::External(_) => {}
        }
    }
    meta.references = refs;
    let has_identity = !meta.library.is_empty();
    if writable && has_identity && Some(&meta) != old_meta.as_ref() {
        meta.write_for(file)?;
        out.sidecar = true;
    }
    Ok(out)
}

/// After the view file `file` was copied or moved from `old_file`: relative `use`
/// paths are written again for the new folder (so they still reach the same cells),
/// and the sidecar names the view it now is (`library/cell/view`, top `new_cell`).
pub fn retarget(
    file: &Path,
    old_file: &Path,
    library: &str,
    cell: &str,
    view: &str,
) -> Result<()> {
    let old_dir = old_file.parent().unwrap_or(Path::new(".")).to_path_buf();
    let new_dir = canonical(file.parent().unwrap_or(Path::new(".")));
    mag::rewrite_file(file, |u| {
        if u.path.is_empty() || u.path.starts_with(['/', '~', '$']) {
            return None;
        }
        let target = canonical(&old_dir.join(&u.path));
        let rel = mag::relative_dir(&new_dir, &target);
        Some(UseEdit {
            cell: None,
            path: Some(rel.to_string_lossy().into_owned()),
        })
    })?;
    let stem = file
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut meta = Meta::read_for(file)
        .ok()
        .flatten()
        .unwrap_or_else(|| Meta::new(library, cell, view, &stem));
    meta.library = library.into();
    meta.cell = cell.into();
    meta.view = view.into();
    meta.top = stem;
    meta.write_for(file)
}

/// Rewrite the views placing cells matched by `wanted`: the `use` line names the
/// new cell (`new_cell`, when the cell was renamed) at its new folder, and the sidecar
/// entry names `new_library(...)` / `new_cell`. `project` is the project *before*
/// the change; `new_dir` gives the folder of the renamed cell's view afterwards.
fn follow(
    project: &Project,
    wanted: &dyn Fn(&MagView) -> bool,
    new_library: &dyn Fn(&str) -> String,
    new_cell: Option<&str>,
    new_dir: &dyn Fn(&MagView) -> Option<PathBuf>,
) -> Vec<Followed> {
    let views = mag_views(project);
    let mut out = Vec::new();
    for v in &views {
        if wanted(v) {
            continue; // the renamed view itself is retargeted by the caller
        }
        let Ok(mag_file) = mag::read(&v.file) else {
            continue;
        };
        let meta = Meta::read_for(&v.file).ok().flatten();
        let resolved = resolve_with(project, &views, &v.file, &mag_file, meta.as_ref());
        let hits: Vec<(String, MagView)> = resolved
            .iter()
            .filter_map(|r| match &r.target {
                Target::Project(t) if wanted(t) => Some((r.placed.cell.clone(), t.clone())),
                _ => None,
            })
            .collect();
        if hits.is_empty() {
            continue;
        }
        if v.read_only {
            out.push(Followed {
                file: v.file.clone(),
                label: v.label(),
                kind: FollowedKind::ReadOnly {
                    library: v.library.clone(),
                },
            });
            continue;
        }
        let parent = v.file.parent().unwrap_or(Path::new(".")).to_path_buf();
        let edits: BTreeMap<String, (String, Option<String>)> = hits
            .iter()
            .map(|(name, t)| {
                let cell = new_cell.map_or_else(|| name.clone(), str::to_string);
                let path = new_dir(t).map(|d| {
                    mag::relative_dir(&canonical(&parent), &d)
                        .to_string_lossy()
                        .into_owned()
                });
                (name.clone(), (cell, path))
            })
            .collect();
        let result = mag::rewrite_file(&v.file, |u| {
            let (cell, path) = edits.get(&u.cell)?;
            Some(UseEdit {
                cell: Some(cell.clone()),
                path: path.clone(),
            })
        })
        .and_then(|_| {
            let stem = v
                .file
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let mut meta =
                meta.clone()
                    .unwrap_or_else(|| Meta::new(&v.library, &v.cell, &v.view, &stem));
            for (name, t) in &hits {
                meta.references.remove(name);
                let cell = new_cell.map_or_else(|| t.cell.clone(), str::to_string);
                let key = new_cell.map_or_else(|| name.clone(), str::to_string);
                meta.references.insert(
                    key,
                    MetaRef {
                        library: new_library(&t.library),
                        cell,
                        view: t.view.clone(),
                        bbox: None,
                    },
                );
            }
            meta.write_for(&v.file)
        });
        out.push(Followed {
            file: v.file.clone(),
            label: v.label(),
            kind: match result {
                Ok(()) => FollowedKind::Updated {
                    structure: new_cell.and_then(|n| {
                        hits.first().map(|(old, _)| (old.clone(), n.to_string()))
                    }),
                },
                Err(e) => FollowedKind::Failed {
                    error: format!("{e:#}"),
                },
            },
        });
    }
    out
}

fn cell_dir(library: &Library, cell: &str, view_file: &Path) -> PathBuf {
    let root = canonical(&library.path);
    match library.layout {
        // <lib>/<cell>/<view folder>/<cell>.mag: the view folder keeps its name
        crate::LibraryLayout::Cells => {
            let mut dir = root.join(cell);
            if let Some(v) = view_file.parent().and_then(|d| d.file_name()) {
                dir.push(v);
            }
            dir
        }
        _ => root,
    }
}

/// After cell `old` of `library` was renamed to `new`: the views placing it name `new`
/// at its new folder. `project` is the project before the rename.
pub fn cell_renamed(project: &Project, library: &str, old: &str, new: &str) -> Vec<Followed> {
    let Some(lib) = project.library(library) else {
        return Vec::new();
    };
    follow(
        project,
        &|v| v.library == library && v.cell == old,
        &|l| l.to_string(),
        Some(new),
        &|t| {
            let renamed = t.file.with_file_name(format!("{new}.mag"));
            Some(cell_dir(lib, new, &renamed))
        },
    )
}

/// After cell `old` of `from` was moved to library `to` as `new`. `project` is the
/// project before the move; `to_lib` the destination library.
pub fn cell_moved(
    project: &Project,
    from: &str,
    old: &str,
    to_lib: &Library,
    new: &str,
) -> Vec<Followed> {
    let to = to_lib.name.clone();
    follow(
        project,
        &|v| v.library == from && v.cell == old,
        &|_| to.clone(),
        (old != new).then_some(new),
        &|t| {
            let renamed = t.file.with_file_name(format!("{new}.mag"));
            Some(cell_dir(to_lib, new, &renamed))
        },
    )
}

/// After library `old` was renamed to `new` (its folder possibly moved to `new_path`):
/// sidecar entries name `new`, paths reach the new folder.
pub fn library_renamed(
    project: &Project,
    old: &str,
    new: &str,
    new_path: Option<&Path>,
) -> Vec<Followed> {
    let old_root = project.library(old).map(|l| canonical(&l.path));
    follow(
        project,
        &|v| v.library == old,
        &|_| new.to_string(),
        None,
        &|t| {
            let (Some(root), Some(new_root)) = (&old_root, new_path) else {
                return None;
            };
            let dir = canonical(t.file.parent()?);
            Some(new_root.join(dir.strip_prefix(root).ok()?))
        },
    )
}

/// The Magic views placing `library/cell` (any view, or only `view`): what a deletion
/// leaves without its cell. Changes nothing.
pub fn placed_by(project: &Project, library: &str, cell: &str, view: Option<&str>) -> Vec<Followed> {
    let views = mag_views(project);
    let mut out = Vec::new();
    for v in &views {
        if v.library == library && v.cell == cell {
            continue;
        }
        let Ok(mag_file) = mag::read(&v.file) else {
            continue;
        };
        let meta = Meta::read_for(&v.file).ok().flatten();
        for r in resolve_with(project, &views, &v.file, &mag_file, meta.as_ref()) {
            if let Target::Project(t) = &r.target
                && t.library == library
                && t.cell == cell
                && view.is_none_or(|w| t.view == w)
            {
                out.push(Followed {
                    file: v.file.clone(),
                    label: v.label(),
                    kind: FollowedKind::Orphaned {
                        name: r.placed.cell.clone(),
                    },
                });
            }
        }
    }
    out
}

/// One line of [`check`]: a view and what is wrong with its references.
#[derive(Debug, Clone, PartialEq)]
pub struct Problem {
    pub view: String,
    pub file: PathBuf,
    pub text: String,
}

/// Check every Magic view of the project's own (writable) libraries: unreadable
/// files, cells that cannot be found, `use` paths that are absolute or not normalized.
/// With `fix`, run [`normalize`] on each. The PDK's (read-only) libraries are not
/// looked at: they are not the project's to change.
pub fn check(project: &Project, fix: bool) -> Vec<Problem> {
    let views = mag_views(project);
    let mut out = Vec::new();
    for v in views.iter().filter(|v| !v.read_only) {
        let problem = |text: String| Problem {
            view: v.label(),
            file: v.file.clone(),
            text,
        };
        if fix && !v.read_only {
            match normalize(project, &v.file) {
                Ok(n) => {
                    if n.paths > 0 {
                        out.push(problem(format!("fixed: {} use path(s) made relative", n.paths)));
                    }
                    for (inst, cell) in n.missing {
                        out.push(problem(format!("{inst}: cell '{cell}' cannot be found")));
                    }
                }
                Err(e) => out.push(problem(format!("{e:#}"))),
            }
            continue;
        }
        let mag_file = match mag::read(&v.file) {
            Ok(m) => m,
            Err(e) => {
                out.push(problem(format!("{e:#}")));
                continue;
            }
        };
        for w in &mag_file.warnings {
            out.push(problem(w.clone()));
        }
        let meta = Meta::read_for(&v.file);
        if let Err(e) = &meta {
            out.push(problem(format!("sidecar: {e:#}")));
        }
        let meta = meta.ok().flatten();
        for r in resolve_with(project, &views, &v.file, &mag_file, meta.as_ref()) {
            let u = &r.placed;
            match &r.target {
                Target::Missing => out.push(problem(format!(
                    "{}: cell '{}' cannot be found",
                    u.instance, u.cell
                ))),
                Target::Project(t) => {
                    if u.path.starts_with(['/', '~']) {
                        out.push(problem(format!(
                            "{}: absolute path to {} (cdept magic refs --fix)",
                            u.instance,
                            t.label()
                        )));
                    } else if r.how != How::Path && r.how != How::Sidecar {
                        out.push(problem(format!(
                            "{}: '{}' found by name only, as {} (cdept magic refs --fix)",
                            u.instance,
                            u.cell,
                            t.label()
                        )));
                    }
                }
                Target::External(_) => {}
            }
        }
    }
    out
}

/// Write `file`'s sidecar (identity only) when it has none: views made by CDePT.
pub fn ensure_sidecar(file: &Path, library: &str, cell: &str, view: &str) -> Result<()> {
    if crate::layout::meta::sidecar_path(file).exists() {
        return Ok(());
    }
    let stem = file
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    Meta::new(library, cell, view, &stem)
        .write_for(file)
        .wrap_err("cannot write the sidecar")
}
