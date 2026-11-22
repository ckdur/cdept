//! Importing an existing design folder: find its libraries and propose a `cdept.toml`.
//!
//! [`scan`] looks at a folder that has no CDePT project yet and returns a [`Plan`]:
//! the libraries it found, each with its folder, layout (`cells` or `flat`), whether
//! it is the PDK's (read-only) and what is in it; the PDK the folder works with; the
//! tools' set-up files; and what was left out as generated output. The plan is plain
//! data the user can read and change (tick libraries off, rename them) before
//! [`Plan::write`] makes the project, or [`Plan::add_to`] adds the libraries to an
//! open one. Nothing in the scanned folder is moved or changed, apart from the new
//! `cdept.toml`.
//!
//! Where libraries come from, in this order:
//!
//! 1. An IHP LibMan project file (`*.projects`, see [`libman`]) in the folder.
//! 2. The folder's `xschemrc`: every folder on its symbol search path (see
//!    [`xschem`]); the file is run by the xschem helper when that is built.
//! 3. The folder itself: every folder below it that holds design files (schematics,
//!    symbols, GDS/OASIS, Magic, netlists, HDL), as a `flat` library, or as a `cells`
//!    library when its folders are cells.
//!
//! Folders of generated files (`simulations`, `results`, anything with raw files
//! only) are listed as left out.

pub mod klayout;
pub mod libman;
pub mod xschem;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use eyre::{Context, Result, bail, eyre};
use toml_edit::{DocumentMut, InlineTable, Item, Table, Value, value};

use crate::library::{LIBRARY_FILE, validate_name};
use crate::project::{AddLibrary, PROJECT_FILE};
use crate::{Format, LibraryLayout, Project, ViewKind};

use self::xschem::PathEntry;

/// How to scan.
#[derive(Debug, Clone)]
pub struct Options {
    /// The project name; the folder name when `None`.
    pub name: Option<String>,
    /// Where `cdept.toml` goes; the scanned folder when `None`.
    pub into: Option<PathBuf>,
    /// Run the `xschemrc` with the xschem helper when it is built.
    pub helpers: bool,
    /// A LibMan project file to import from; those in the folder when `None`.
    pub libman: Option<PathBuf>,
    /// How many folder levels to look down.
    pub max_depth: usize,
    /// Reads the environment (`PDK_ROOT`, `PDK`, `HOME`).
    pub env: fn(&str) -> Option<String>,
}

fn process_env(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

impl Default for Options {
    fn default() -> Self {
        Options {
            name: None,
            into: None,
            helpers: true,
            libman: None,
            max_depth: 6,
            env: process_env,
        }
    }
}

/// How a library was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// A folder on the `xschemrc`'s symbol search path.
    XschemPath,
    /// A folder with design files below the scanned folder.
    Folder,
    /// Defined in a LibMan project file.
    LibMan,
}

impl Origin {
    pub fn label(self) -> &'static str {
        match self {
            Origin::XschemPath => "xschemrc",
            Origin::Folder => "folder",
            Origin::LibMan => "LibMan",
        }
    }
}

/// A library the plan proposes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedLibrary {
    pub name: String,
    /// The library folder (absolute).
    pub folder: PathBuf,
    pub layout: LibraryLayout,
    /// The PDK's and other reference libraries.
    pub read_only: bool,
    /// Listed in the project. The user may change it.
    pub enabled: bool,
    pub origin: Origin,
    pub cells: usize,
    /// Views by kind.
    pub views: BTreeMap<ViewKind, usize>,
    pub note: Option<String>,
}

impl PlannedLibrary {
    /// `12 cells: 12 symbol, 3 schematic`.
    pub fn summary(&self) -> String {
        let views: Vec<String> = self
            .views
            .iter()
            .map(|(kind, count)| format!("{count} {}", kind.name()))
            .collect();
        let cells = if self.cells == 1 { "cell" } else { "cells" };
        if views.is_empty() {
            format!("{} {cells}", self.cells)
        } else {
            format!("{} {cells}: {}", self.cells, views.join(", "))
        }
    }
}

/// A folder that was left out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    pub folder: PathBuf,
    pub reason: String,
}

/// What an import would write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// The scanned folder.
    pub source: PathBuf,
    /// The folder `cdept.toml` goes into.
    pub root: PathBuf,
    pub name: String,
    /// The PDK the folder works with (`$PDK`), when one was recognised.
    pub technology: Option<String>,
    /// The folder the PDK is installed in (`$PDK_ROOT`).
    pub pdk_root: Option<PathBuf>,
    /// `pdk_root` is written (into `cdept.local.toml`, this machine's settings): the
    /// environment does not give the same.
    pub write_pdk_root: bool,
    /// The `xschemrc` the embedded xschem should read (`[xschem] rc`).
    pub xschem_rc: Option<PathBuf>,
    /// A KLayout technology file found in the folder (`[klayout] tech`).
    pub klayout_tech: Option<PathBuf>,
    /// A KLayout layer properties file found in the folder.
    pub layer_properties: Option<PathBuf>,
    pub libraries: Vec<PlannedLibrary>,
    pub skipped: Vec<Skipped>,
    pub notes: Vec<String>,
}

/// Folder names that hold generated files, not design data.
const GENERATED: [&str; 22] = [
    "simulations",
    "simulation",
    "results",
    "result",
    "output",
    "outputs",
    "out",
    "build",
    "runs",
    "tmp",
    "temp",
    "target",
    "node_modules",
    "__pycache__",
    "venv",
    "logs",
    "log",
    "work",
    "backup",
    "backups",
    "drc_reports",
    "lvs_reports",
];

/// A folder name that says "generated files".
pub fn is_generated_name(name: &str) -> bool {
    GENERATED.contains(&name.to_ascii_lowercase().as_str())
}

/// `<root>/<pdk>` for a path inside a PDK: the folder above `libs.tech` or `libs.ref`.
pub fn pdk_of(path: &Path) -> Option<(PathBuf, String)> {
    let mut dir = PathBuf::new();
    let mut previous: Option<(PathBuf, String)> = None;
    for part in path.components() {
        let text = part.as_os_str().to_string_lossy();
        if (text == "libs.tech" || text == "libs.ref")
            && let Some((root, name)) = &previous
        {
            return Some((root.clone(), name.clone()));
        }
        previous = Some((dir.clone(), text.into_owned()));
        dir.push(part);
    }
    None
}

/// A PDK's tests and examples are found but not listed: whether `dir` is one, and the
/// note that says so.
fn pdk_extra(dir: &Path, in_pdk: bool) -> (bool, Option<String>) {
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let extra = in_pdk
        && ["test", "example", "demo", "mips_cpu", "import"]
            .iter()
            .any(|w| name.contains(w));
    (
        extra,
        extra.then(|| "tests or examples of the PDK".to_string()),
    )
}

/// What a folder holds, one level deep.
#[derive(Default)]
struct Listing {
    /// Design files directly in the folder: (stem, format).
    design: Vec<(String, Format)>,
    /// Folders: (name, path), hidden ones left out.
    dirs: Vec<(String, PathBuf)>,
    raw_files: usize,
    room_files: usize,
    has_lib_toml: bool,
}

fn listing(dir: &Path) -> Listing {
    let mut out = Listing::default();
    let Ok(items) = std::fs::read_dir(dir) else {
        return out;
    };
    for item in items.flatten() {
        let Ok(name) = item.file_name().into_string() else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        let path = item.path();
        if path.is_dir() {
            out.dirs.push((name, path));
        } else if name == LIBRARY_FILE {
            out.has_lib_toml = true;
        } else if name.ends_with(".raw") {
            out.raw_files += 1;
        } else if name.ends_with(".room") {
            out.room_files += 1;
        } else if let Some((stem, format)) = Format::split_name(&name) {
            out.design.push((stem.to_string(), format));
        }
    }
    out.dirs.sort();
    out.design.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// What a library folder holds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Contents {
    cells: usize,
    views: BTreeMap<ViewKind, usize>,
    room_files: usize,
}

/// The views of `dir` as a cell folder named `cell`: files named after the cell
/// directly in it, and view folders. `None` when it does not look like a cell.
fn cell_views(cell: &str, dir: &Path) -> Option<(Vec<ViewKind>, usize)> {
    let inside = listing(dir);
    // A folder with a lib.toml is a library, whatever is in it.
    if inside.has_lib_toml {
        return None;
    }
    // A folder of loose files is a cell only when the files are all its own.
    let own = |stem: &str| stem == cell || stem.starts_with(&format!("{cell}."));
    if inside.design.iter().any(|(stem, _)| !own(stem)) {
        return None;
    }
    let mut views: Vec<ViewKind> = inside.design.iter().map(|(_, f)| f.kind()).collect();
    for (name, path) in &inside.dirs {
        let files = listing(path);
        let named = ViewKind::from_view_name(name);
        let main = files.design.iter().find(|(stem, _)| stem == cell);
        match (named, main, files.design.first()) {
            (_, Some((_, format)), _) => views.push(format.kind()),
            (Some(kind), None, Some(_)) => views.push(kind),
            _ => {}
        }
    }
    let rooms = inside.room_files;
    (!views.is_empty() || rooms > 0).then_some((views, rooms))
}

/// `dir` as a library with cell folders, when it looks like one.
fn as_cells(found: &Listing) -> Option<Contents> {
    if !found.design.is_empty() && !found.has_lib_toml {
        return None;
    }
    let mut contents = Contents::default();
    for (name, path) in &found.dirs {
        if let Some((views, rooms)) = cell_views(name, path) {
            contents.cells += 1;
            contents.room_files += rooms;
            for kind in views {
                *contents.views.entry(kind).or_default() += 1;
            }
        }
    }
    let enough = contents.cells > 0 && contents.cells * 2 >= found.dirs.len();
    (found.has_lib_toml || enough).then_some(contents)
}

/// `dir` as a flat library: the design files directly in it.
fn as_flat(found: &Listing) -> Option<Contents> {
    if found.design.is_empty() {
        return None;
    }
    let mut contents = Contents::default();
    let stems: BTreeSet<&str> = found.design.iter().map(|(stem, _)| stem.as_str()).collect();
    contents.cells = stems.len();
    for (_, format) in &found.design {
        *contents.views.entry(format.kind()).or_default() += 1;
    }
    Some(contents)
}

/// A name CDePT accepts for a library, from a folder name.
fn library_name(folder: &Path) -> String {
    let base = folder
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "library".into());
    let mut name: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if !name
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        name.insert(0, '_');
    }
    name
}

/// The name a library gets from its folder: the folder's name, except inside a PDK's
/// reference libraries (`libs.ref/<library>/sym/xschem`), where the folders are named
/// after file formats and the library's name is further up.
fn default_name(folder: &Path) -> String {
    let parts: Vec<String> = folder
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    match parts.iter().position(|p| p == "libs.ref") {
        Some(at) if at + 2 < parts.len() => library_name(Path::new(&parts[at + 1])),
        _ => library_name(folder),
    }
}

struct Scanner<'a> {
    options: &'a Options,
    plan: Plan,
    /// Library folders already in the plan (resolved).
    seen: BTreeSet<PathBuf>,
    /// PDKs the libraries were found in: (root, name), in order.
    pdks: Vec<(PathBuf, String)>,
    dirs_visited: usize,
}

/// The most folders a scan looks into.
const MAX_DIRS: usize = 20_000;

impl Scanner<'_> {
    fn resolved(path: &Path) -> PathBuf {
        path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
    }

    fn unique_name(&self, wanted: &str, folder: &Path) -> String {
        let taken = |name: &str| self.plan.libraries.iter().any(|l| l.name == name);
        if !taken(wanted) {
            return wanted.to_string();
        }
        // The folder above tells two libraries of the same name apart.
        if let Some(parent) = folder.parent() {
            let name = format!("{}_{wanted}", library_name(parent));
            if !taken(&name) {
                return name;
            }
        }
        (2..)
            .map(|n| format!("{wanted}_{n}"))
            .find(|name| !taken(name))
            .expect("a free name")
    }

    /// Add a library to the plan, unless its folder is there already. Returns whether
    /// it was added.
    #[allow(clippy::too_many_arguments)]
    fn add(
        &mut self,
        name: Option<&str>,
        folder: &Path,
        layout: LibraryLayout,
        contents: Contents,
        origin: Origin,
        enabled: bool,
        note: Option<String>,
    ) -> bool {
        if !self.seen.insert(Self::resolved(folder)) {
            return false;
        }
        let pdk = pdk_of(folder);
        if let Some(pdk) = &pdk
            && !self.pdks.contains(pdk)
        {
            self.pdks.push(pdk.clone());
        }
        let wanted = name.map_or_else(|| default_name(folder), str::to_string);
        let name = self.unique_name(&wanted, folder);
        let mut note = note;
        if contents.room_files > 0 {
            let text = format!(
                "{} ROOM view(s) of LibMan/DOOR are not readable by CDePT",
                contents.room_files
            );
            note = Some(match note {
                Some(n) => format!("{n}; {text}"),
                None => text,
            });
        }
        self.plan.libraries.push(PlannedLibrary {
            name,
            folder: folder.to_path_buf(),
            layout,
            read_only: pdk.is_some(),
            enabled,
            origin,
            cells: contents.cells,
            views: contents.views,
            note,
        });
        true
    }

    /// Libraries below one folder of xschem's search path: the folder itself when it
    /// holds symbols or schematics, and the folders in it (references read
    /// `folder/cell.sym`).
    fn search_path_entry(&mut self, entry: &Path) {
        if !entry.is_dir() {
            self.plan.notes.push(format!(
                "{}: on xschem's search path, but not found",
                entry.display()
            ));
            return;
        }
        let in_pdk = pdk_of(entry).is_some();
        let found = listing(entry);
        if let Some(contents) = as_cells(&found) {
            self.add(
                None,
                entry,
                LibraryLayout::Cells,
                contents,
                Origin::XschemPath,
                true,
                None,
            );
            return;
        }
        for (name, path) in &found.dirs {
            if !is_generated_name(name) {
                self.search_path_folder(path, in_pdk, 1);
            }
        }
        if let Some(contents) = as_flat(&found) {
            self.add(
                None,
                entry,
                LibraryLayout::Flat,
                contents,
                Origin::XschemPath,
                true,
                None,
            );
        }
    }

    fn search_path_folder(&mut self, dir: &Path, in_pdk: bool, depth: usize) -> bool {
        let found = listing(dir);
        let (extra, note) = pdk_extra(dir, in_pdk);
        if let Some(contents) = as_cells(&found) {
            return self.add(
                None,
                dir,
                LibraryLayout::Cells,
                contents,
                Origin::XschemPath,
                !extra,
                note,
            );
        }
        let mut any = false;
        if let Some(contents) = as_flat(&found) {
            any |= self.add(
                None,
                dir,
                LibraryLayout::Flat,
                contents,
                Origin::XschemPath,
                !extra,
                note,
            );
        }
        if depth < 2 {
            for (name, path) in &found.dirs {
                if !is_generated_name(name) {
                    any |= self.search_path_folder(path, in_pdk, depth + 1);
                }
            }
        }
        any
    }

    /// Every folder with design files at or below `dir`.
    fn walk(&mut self, dir: &Path, depth: usize, top: bool) {
        self.dirs_visited += 1;
        if self.dirs_visited > MAX_DIRS {
            if self.dirs_visited == MAX_DIRS + 1 {
                self.plan.notes.push(format!(
                    "more than {MAX_DIRS} folders: the scan stopped early"
                ));
            }
            return;
        }
        let found = listing(dir);
        let has_schematics = found.design.iter().any(|(_, f)| {
            matches!(
                f,
                Format::XschemSchematic | Format::XschemSymbol | Format::Gds | Format::Oasis
            )
        });
        if found.raw_files > 0 && !has_schematics && !top {
            self.plan.skipped.push(Skipped {
                folder: dir.to_path_buf(),
                reason: "simulation results (raw files)".into(),
            });
            return;
        }
        if self.seen.contains(&Self::resolved(dir)) {
            // Already a library (from the xschemrc or LibMan): a library with cell
            // folders is not looked into, a flat one may have more below.
            let cells = self.plan.libraries.iter().any(|l| {
                l.layout == LibraryLayout::Cells && Self::resolved(&l.folder) == Self::resolved(dir)
            });
            if cells {
                return;
            }
        } else if let Some(contents) = as_cells(&found) {
            let (extra, note) = pdk_extra(dir, pdk_of(dir).is_some());
            self.add(
                None,
                dir,
                LibraryLayout::Cells,
                contents,
                Origin::Folder,
                !extra,
                note,
            );
            return;
        } else if let Some(contents) = as_flat(&found) {
            let (extra, note) = pdk_extra(dir, pdk_of(dir).is_some());
            self.add(
                None,
                dir,
                LibraryLayout::Flat,
                contents,
                Origin::Folder,
                !extra,
                note,
            );
        }
        if depth == 0 {
            return;
        }
        for (name, path) in &found.dirs {
            if is_generated_name(name) {
                let has_files =
                    std::fs::read_dir(path).is_ok_and(|mut items| items.next().is_some());
                let known = self.plan.skipped.iter().any(|s| &s.folder == path);
                if has_files && !known {
                    self.plan.skipped.push(Skipped {
                        folder: path.clone(),
                        reason: "generated files, by its name".into(),
                    });
                }
                continue;
            }
            self.walk(path, depth - 1, false);
        }
    }

    fn libman(&mut self, file: &Path) {
        let project = match libman::read(file) {
            Ok(project) => project,
            Err(error) => {
                self.plan.notes.push(format!("{error:#}"));
                return;
            }
        };
        let shown = file.display();
        for problem in &project.problems {
            self.plan.notes.push(format!("{shown}: {problem}"));
        }
        for library in project.resolve() {
            if library.folders.is_empty() {
                self.plan.notes.push(format!(
                    "{shown}: library '{}': no folder found for {}",
                    library.name,
                    library.unresolved.join(", ")
                ));
                continue;
            }
            for (n, folder) in library.folders.iter().enumerate() {
                let found = listing(folder);
                let (layout, contents) = match (as_cells(&found), as_flat(&found)) {
                    (Some(contents), _) => (LibraryLayout::Cells, contents),
                    (None, Some(contents)) => (LibraryLayout::Flat, contents),
                    (None, None) => (LibraryLayout::Cells, Contents::default()),
                };
                // A CDePT library is one folder: more folders become more libraries.
                let (name, note) = if n == 0 {
                    (Some(library.name.as_str()), None)
                } else {
                    (
                        None,
                        Some(format!("also part of LibMan's library '{}'", library.name)),
                    )
                };
                self.add(name, folder, layout, contents, Origin::LibMan, true, note);
                let groups = crate::category::read_libman(folder);
                if !groups.is_empty() {
                    self.plan.notes.push(format!(
                        "{}: {} LibMan categor{} (*.group): shown as they are; `cdept category \
                         adopt` copies them into CDePT's own files",
                        folder.display(),
                        groups.names().len(),
                        if groups.names().len() == 1 {
                            "y"
                        } else {
                            "ies"
                        }
                    ));
                }
            }
        }
        if let Some((name, path)) = &project.pdk {
            let dir = project.base().join(path);
            match dir.parent().filter(|_| dir.join("libs.tech").is_dir()) {
                Some(root) => self.pdks.insert(0, (root.to_path_buf(), name.clone())),
                None => {
                    self.plan.technology.get_or_insert(name.clone());
                    self.plan.notes.push(format!(
                        "{shown}: the PDK '{name}' is not at {path} on this machine"
                    ));
                }
            }
        }
        if !project.attached.is_empty() {
            self.plan.notes.push(format!(
                "{shown}: {} attach() line(s) (reference libraries of a library) have no \
                 counterpart in CDePT: every library of the project is searched",
                project.attached.len()
            ));
        }
        for (group, members) in &project.groups {
            self.plan.notes.push(format!(
                "{shown}: group '{group}' ({}) is not kept: CDePT has no library groups",
                members.join(", ")
            ));
        }
    }
}

/// Scan `folder` and propose a project.
pub fn scan(folder: &Path, options: &Options) -> Result<Plan> {
    if !folder.is_dir() {
        bail!("{} is not a folder", folder.display());
    }
    let source = std::path::absolute(folder)
        .wrap_err_with(|| format!("cannot resolve {}", folder.display()))?;
    let root = match &options.into {
        Some(into) => std::path::absolute(into)
            .wrap_err_with(|| format!("cannot resolve {}", into.display()))?,
        None => source.clone(),
    };
    let name = options.name.clone().unwrap_or_else(|| {
        Scanner::resolved(&root)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "project".into())
    });
    let mut scanner = Scanner {
        options,
        plan: Plan {
            source: source.clone(),
            root,
            name,
            technology: None,
            pdk_root: None,
            write_pdk_root: false,
            xschem_rc: None,
            klayout_tech: None,
            layer_properties: None,
            libraries: Vec::new(),
            skipped: Vec::new(),
            notes: Vec::new(),
        },
        seen: BTreeSet::new(),
        pdks: Vec::new(),
        dirs_visited: 0,
    };
    if source.join(PROJECT_FILE).is_file() {
        scanner.plan.notes.push(format!(
            "{} already has a {PROJECT_FILE}: the libraries found can be added to that project",
            source.display()
        ));
    }

    // 1. LibMan.
    let libman_files = match &options.libman {
        Some(file) => vec![file.clone()],
        None => libman::find(&source),
    };
    for file in &libman_files {
        scanner.libman(file);
    }

    // 2. The xschemrc.
    let env = options.env;
    let mut rc_pdk: Option<(PathBuf, String)> = None;
    if let Some(rc) = xschem::find_rc(&source) {
        // A folder inside a PDK says which PDK, when the environment does not.
        let here = pdk_of(&source);
        let pdk_env = |name: &str| {
            env(name).or_else(|| match (name, &here) {
                ("PDK_ROOT", Some((root, _))) => Some(root.to_string_lossy().into_owned()),
                ("PDK", Some((_, pdk))) => Some(pdk.clone()),
                _ => None,
            })
        };
        let info = xschem::read_rc(&rc, &source, options.helpers, &pdk_env);
        scanner.plan.notes.extend(info.notes.iter().cloned());
        let mut own = false;
        for entry in &info.path {
            match entry {
                PathEntry::Own => own = true,
                PathEntry::Folder(path) => scanner.search_path_entry(path),
                PathEntry::Unresolved(_) => {}
            }
        }
        if own {
            scanner.plan.notes.push(
                "xschem's own libraries (devices, …) are on the search path: CDePT's xschem \
                 has them, they are not listed"
                    .into(),
            );
        }
        if let Some(dir) = &info.netlist_dir
            && dir.is_dir()
            && dir.starts_with(&source)
            && dir != &source
        {
            scanner.plan.skipped.push(Skipped {
                folder: dir.clone(),
                reason: "xschem's netlist and simulation folder".into(),
            });
            scanner.seen.insert(Scanner::resolved(dir));
        }
        if let (Some(root), Some(pdk)) = (&info.pdk_root, &info.pdk) {
            // `$PDK` may list several PDKs: the first that is there.
            rc_pdk = pdk
                .split(':')
                .find(|name| Path::new(root).join(name).join("libs.tech").is_dir())
                .map(|name| (PathBuf::from(root), name.to_string()));
        }
        scanner.plan.xschem_rc = Some(rc);
    }

    // 3. The folder itself.
    scanner.walk(&source, scanner.options.max_depth, true);
    // Folders left out by the rc are not libraries, even when the walk met them.
    let left_out: Vec<PathBuf> = scanner
        .plan
        .skipped
        .iter()
        .map(|s| Scanner::resolved(&s.folder))
        .collect();
    scanner.plan.libraries.retain(|l| {
        !left_out
            .iter()
            .any(|s| Scanner::resolved(&l.folder).starts_with(s))
    });

    // A PDK folder that only collects libraries usually has a stray start page or
    // two next to them: found, but not listed unless asked for.
    let folders: Vec<PathBuf> = scanner
        .plan
        .libraries
        .iter()
        .map(|l| l.folder.clone())
        .collect();
    for library in &mut scanner.plan.libraries {
        let collects = folders
            .iter()
            .any(|f| f != &library.folder && f.starts_with(&library.folder));
        if library.read_only && library.layout == LibraryLayout::Flat && collects && library.enabled
        {
            library.enabled = false;
            library.note = Some("files next to the PDK's libraries".into());
        }
        // A PDK's start-up file adds what CDePT's xschem has anyway.
        if library.read_only && library.origin == Origin::XschemPath {
            let own = library
                .folder
                .components()
                .any(|c| c.as_os_str() == "xschem_library");
            if own && library.enabled {
                library.enabled = false;
                library.note = Some("a copy of xschem's own library".into());
            }
        }
    }

    // Set-up files of the layout tools.
    let tools = klayout::find(&source, 2);
    if let Some((file, technology)) = tools.lyt.first() {
        scanner.plan.klayout_tech = Some(file.clone());
        if tools.lyt.len() > 1 {
            scanner.plan.notes.push(format!(
                "{} KLayout technology files found; {} is proposed{}",
                tools.lyt.len(),
                file.display(),
                technology
                    .as_ref()
                    .map(|t| format!(" (technology '{t}')"))
                    .unwrap_or_default()
            ));
        }
    }
    if let Some(file) = tools.lyp.first() {
        scanner.plan.layer_properties = Some(file.clone());
    }
    for file in &tools.magicrc {
        scanner.plan.notes.push(format!(
            "{}: a Magic start-up file (not used by CDePT yet)",
            file.display()
        ));
    }

    // The PDK: the one the libraries are in, else the one the xschemrc names.
    let mut plan = scanner.plan;
    let found = scanner.pdks.first().cloned().or(rc_pdk.clone());
    // A PDK is often reached through a link (ciel keeps versions and links the one
    // in use), and an xschemrc may resolve it. Spell the PDK folder the way the user
    // does: as the scanned folder has it, as the environment has it, as the xschemrc
    // says, in this order.
    let mut spellings: Vec<(PathBuf, String)> = pdk_of(&source).into_iter().collect();
    if let (Some(root), Some(names)) = (env("PDK_ROOT"), env("PDK")) {
        spellings.extend(
            names
                .split(':')
                .map(|n| (PathBuf::from(&root), n.to_string())),
        );
    }
    spellings.extend(rc_pdk);
    let pdk = found.map(|(root, name)| {
        let actual = Scanner::resolved(&root.join(&name));
        let spelled = spellings
            .into_iter()
            .find(|(r, n)| Scanner::resolved(&r.join(n)) == actual)
            .unwrap_or((root, name));
        let dir = spelled.0.join(&spelled.1);
        let respell = |path: &mut PathBuf| {
            if !path.starts_with(&dir)
                && let Ok(rest) = Scanner::resolved(path).strip_prefix(&actual)
            {
                *path = dir.join(rest);
            }
        };
        plan.libraries
            .iter_mut()
            .for_each(|l| respell(&mut l.folder));
        plan.skipped.iter_mut().for_each(|s| respell(&mut s.folder));
        if let Some(rc) = &mut plan.xschem_rc {
            respell(rc);
        }
        spelled
    });
    if scanner.pdks.len() > 1 {
        let names: Vec<&str> = scanner.pdks.iter().map(|(_, n)| n.as_str()).collect();
        plan.notes.push(format!(
            "libraries of several PDKs are on the search path ({}); the first is taken as \
             the project's",
            names.join(", ")
        ));
    }
    if let Some((root, name)) = pdk {
        plan.write_pdk_root = env("PDK_ROOT").map(PathBuf::from).as_ref() != Some(&root);
        if plan.write_pdk_root {
            plan.notes.push(format!(
                "$PDK_ROOT is not set to {} here: it is written as pdk_root into \
                 cdept.local.toml, the settings of this machine",
                root.display()
            ));
        }
        plan.technology = Some(name);
        plan.pdk_root = Some(root);
    }
    if plan.libraries.is_empty() {
        plan.notes
            .push("no folder with design files was found".into());
    }
    Ok(plan)
}

impl Plan {
    /// `path` as it is written into the project file: `$PDK_ROOT/…` inside the PDK
    /// folder, relative inside the project folder, absolute otherwise.
    pub fn path_text(&self, path: &Path) -> String {
        if let Some(root) = &self.pdk_root
            && let Ok(rest) = path.strip_prefix(root)
            && !rest.as_os_str().is_empty()
        {
            return format!("$PDK_ROOT/{}", rest.to_string_lossy());
        }
        match path.strip_prefix(&self.root) {
            Ok(rest) if rest.as_os_str().is_empty() => ".".into(),
            Ok(rest) => rest.to_string_lossy().into_owned(),
            Err(_) => path.to_string_lossy().into_owned(),
        }
    }

    /// The libraries that will be listed.
    pub fn enabled(&self) -> impl Iterator<Item = &PlannedLibrary> {
        self.libraries.iter().filter(|l| l.enabled)
    }

    /// Problems that would stop [`write`](Self::write): names used twice or not
    /// usable.
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut names = BTreeSet::new();
        for library in self.enabled() {
            if let Err(error) = validate_name("library", &library.name) {
                out.push(format!("library '{}': {error}", library.name));
            } else if library.name.contains('.') {
                out.push(format!(
                    "library '{}': '.' is not allowed in a library name",
                    library.name
                ));
            }
            if !names.insert(library.name.as_str()) {
                out.push(format!("two libraries are named '{}'", library.name));
            }
        }
        out
    }

    /// Put the definitions into a project file.
    fn apply(&self, doc: &mut DocumentMut) -> Result<()> {
        let libraries = doc
            .entry("libraries")
            .or_insert(Item::Table(Table::new()))
            .as_table_like_mut()
            .ok_or_else(|| eyre!("`libraries` must be a table"))?;
        for library in self.enabled() {
            let mut table = InlineTable::new();
            table.insert("path", self.path_text(&library.folder).into());
            table.insert("layout", library.layout.name().into());
            if library.read_only {
                table.insert("read_only", true.into());
            }
            libraries.insert(&library.name, Item::Value(Value::InlineTable(table)));
        }
        if let Some(rc) = &self.xschem_rc {
            let table = doc
                .entry("xschem")
                .or_insert(Item::Table(Table::new()))
                .as_table_like_mut()
                .ok_or_else(|| eyre!("`xschem` must be a table"))?;
            table.insert("rc", value(self.path_text(rc)));
        }
        if self.klayout_tech.is_some() || self.layer_properties.is_some() {
            let table = doc
                .entry("klayout")
                .or_insert(Item::Table(Table::new()))
                .as_table_like_mut()
                .ok_or_else(|| eyre!("`klayout` must be a table"))?;
            if let Some(file) = &self.klayout_tech {
                table.insert("tech", value(self.path_text(file)));
            }
            if let Some(file) = &self.layer_properties {
                table.insert("layer_properties", value(self.path_text(file)));
            }
        }
        Ok(())
    }

    fn pdk_root_text(&self) -> Option<String> {
        self.pdk_root
            .as_ref()
            .filter(|_| self.write_pdk_root)
            .map(|root| root.to_string_lossy().into_owned())
    }

    /// The project file the plan stands for (without the comments of a new project
    /// file). The PDK folder is not in it: it goes into `cdept.local.toml`.
    pub fn toml(&self) -> String {
        let mut doc = DocumentMut::new();
        doc["name"] = value(self.name.as_str());
        if let Some(technology) = &self.technology {
            doc["technology"] = value(technology.as_str());
        }
        match self.apply(&mut doc) {
            Ok(()) => doc.to_string(),
            Err(error) => format!("# {error}\n"),
        }
    }

    /// The plan as text, for a terminal.
    pub fn describe(&self) -> String {
        let mut out = format!("Project '{}' in {}\n", self.name, self.root.display());
        if self.source != self.root {
            out.push_str(&format!("  scanned folder: {}\n", self.source.display()));
        }
        if let Some(technology) = &self.technology {
            out.push_str(&format!("  technology: {technology}"));
            if let Some(root) = &self.pdk_root {
                out.push_str(&format!("  (PDK folder {})", root.display()));
            }
            out.push('\n');
        }
        if let Some(rc) = &self.xschem_rc {
            out.push_str(&format!("  xschem start-up file: {}\n", self.path_text(rc)));
        }
        if let Some(file) = &self.klayout_tech {
            out.push_str(&format!("  KLayout technology: {}\n", self.path_text(file)));
        }
        out.push_str(&format!("\nLibraries ({}):\n", self.libraries.len()));
        let width = self
            .libraries
            .iter()
            .map(|l| l.name.len())
            .max()
            .unwrap_or(0);
        for library in &self.libraries {
            let mut line = format!(
                "  [{}] {:width$}  {:5}  {}",
                if library.enabled { 'x' } else { ' ' },
                library.name,
                library.layout.name(),
                self.path_text(&library.folder),
            );
            line.push_str(&format!("  ({}", library.summary()));
            if library.read_only {
                line.push_str(", read-only");
            }
            line.push_str(&format!(", from {})", library.origin.label()));
            if let Some(note) = &library.note {
                line.push_str(&format!("  — {note}"));
            }
            out.push_str(&line);
            out.push('\n');
        }
        if !self.skipped.is_empty() {
            out.push_str("\nLeft out:\n");
            for skipped in &self.skipped {
                out.push_str(&format!(
                    "  {}  ({})\n",
                    self.path_text(&skipped.folder),
                    skipped.reason
                ));
            }
        }
        if !self.notes.is_empty() {
            out.push_str("\nNotes:\n");
            for note in &self.notes {
                out.push_str(&format!("  - {note}\n"));
            }
        }
        out
    }

    /// Make the project: a new `cdept.toml` in [`root`](Self::root) with the enabled
    /// libraries, the PDK and the tools' set-up files. Fails when the folder has a
    /// project already.
    pub fn write(&self) -> Result<Project> {
        let problems = self.problems();
        if !problems.is_empty() {
            bail!("{}", problems.join("; "));
        }
        let project = Project::create_with(
            &self.root,
            Some(&self.name),
            self.technology.as_deref(),
            self.pdk_root_text().as_deref(),
        )?;
        let file = project.file.clone();
        let text = std::fs::read_to_string(&file)
            .wrap_err_with(|| format!("cannot read {}", file.display()))?;
        let mut doc: DocumentMut = text.parse().map_err(|e| eyre!("{}: {e}", file.display()))?;
        self.apply(&mut doc)?;
        let tmp = file.with_extension("toml.tmp");
        std::fs::write(&tmp, doc.to_string())
            .wrap_err_with(|| format!("cannot write {}", tmp.display()))?;
        std::fs::rename(&tmp, &file)
            .wrap_err_with(|| format!("cannot write {}", file.display()))?;
        Project::load(&file)
    }

    /// Add the enabled libraries to an open project, those it has already (by name or
    /// by folder) left out. Returns the names added.
    pub fn add_to(&self, project: &mut Project) -> Result<Vec<String>> {
        let mut added = Vec::new();
        for library in self.enabled() {
            let folder = Scanner::resolved(&library.folder);
            let known = project
                .libraries
                .iter()
                .any(|l| l.name == library.name || Scanner::resolved(&l.path) == folder);
            if known {
                continue;
            }
            // Paths as this project writes them (its folder, its PDK).
            let path = match project
                .pdk
                .var("PDK_ROOT")
                .and_then(|root| library.folder.strip_prefix(root).ok())
            {
                Some(rest) => format!("$PDK_ROOT/{}", rest.to_string_lossy()),
                None => project.portable_path(&library.folder),
            };
            project.add_library(
                &library.name,
                &path,
                AddLibrary {
                    layout: Some(library.layout),
                    read_only: library.read_only,
                    create: false,
                },
            )?;
            added.push(library.name.clone());
        }
        Ok(added)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pdk_of_a_path() {
        assert_eq!(
            pdk_of(Path::new("/opt/pdks/sky130A/libs.tech/xschem/sky130_fd_pr")),
            Some(("/opt/pdks".into(), "sky130A".into()))
        );
        assert_eq!(
            pdk_of(Path::new("/opt/pdks/ihp-sg13g2/libs.ref/sg13g2_stdcell")),
            Some(("/opt/pdks".into(), "ihp-sg13g2".into()))
        );
        assert_eq!(pdk_of(Path::new("/home/u/project/libs")), None);
        assert_eq!(pdk_of(Path::new("libs.tech/xschem")), None);
    }

    #[test]
    fn library_names_from_folders() {
        assert_eq!(library_name(Path::new("/a/my lib.v2")), "my_lib_v2");
        assert_eq!(library_name(Path::new("/a/-x")), "_-x");
        assert!(is_generated_name("Simulations"));
        assert!(!is_generated_name("sim"));
    }
}
