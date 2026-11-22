//! Categories: named groups of the cells of a library.
//!
//! A cell can be in several categories and categories nest: `analog/amplifiers` is the
//! category `amplifiers` inside `analog`. A category holds the cells listed for it and,
//! for browsing, those of the categories below it.
//!
//! Where they are kept (our own format, plain TOML):
//!
//! - in the library, in its `lib.toml`:
//!
//!   ```toml
//!   [categories]
//!   "analog/amplifiers" = ["ota5t", "ota_folded"]
//!   digital = ["inv", "buf"]
//!   ```
//!
//!   That is where changes go for a library CDePT may write to and that has (or may
//!   get) a `lib.toml`: a library with view folders. The categories travel with the
//!   library.
//!
//! - in the project, in `categories.toml` next to `cdept.toml`, one table per library:
//!
//!   ```toml
//!   [sg13g2_pr]
//!   mos = ["sg13_lv_nmos", "sg13_lv_pmos"]
//!   ```
//!
//!   That is where changes go for read-only libraries (a PDK's) and for flat libraries
//!   (a `lib.toml` dropped into a folder of design files would make it look like a
//!   library with view folders). Both files are read for every library and joined.
//!
//! - IHP LibMan (DOOR) keeps a category as a file `<library>/<name>.group` listing cell
//!   names. Those are read too, never written; [`adopt_libman`] copies them into our
//!   own store.
//!
//! Names of cells that do not exist (any more) stay in the files and are not shown.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use eyre::{Context, Result, bail, eyre};
use toml_edit::{Array, DocumentMut, Item, Table, value};

use crate::library::LIBRARY_FILE;
use crate::{Format, Library, LibraryLayout, Project};

/// The project's own category file, next to `cdept.toml`.
pub const PROJECT_CATEGORIES_FILE: &str = "categories.toml";
/// Suffix of a LibMan category file.
pub const LIBMAN_SUFFIX: &str = ".group";
/// The key of the table in `lib.toml`.
const KEY: &str = "categories";

/// The categories of one library: category path → cells listed for it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Categories {
    map: BTreeMap<String, BTreeSet<String>>,
}

/// A category in the tree of a library's categories.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Node {
    /// The last part of the path: `amplifiers`.
    pub name: String,
    /// The whole path: `analog/amplifiers`.
    pub path: String,
    /// The cells listed for this category itself, sorted.
    pub cells: Vec<String>,
    pub children: Vec<Node>,
}

impl Node {
    /// The cells of this category and of every category below it.
    pub fn count(&self) -> usize {
        let mut all = BTreeSet::new();
        self.collect(&mut all);
        all.len()
    }

    fn collect<'a>(&'a self, into: &mut BTreeSet<&'a str>) {
        into.extend(self.cells.iter().map(String::as_str));
        for child in &self.children {
            child.collect(into);
        }
    }
}

/// `analog / amplifiers ` → `analog/amplifiers`; an error for a name that cannot be one.
pub fn normalise(category: &str) -> Result<String> {
    let parts: Vec<&str> = category.split('/').map(str::trim).collect();
    if parts.iter().any(|p| p.is_empty()) {
        bail!("a category name cannot be empty (in '{category}')");
    }
    if let Some(bad) = category.chars().find(|c| c.is_control() || *c == '"') {
        bail!("{bad:?} is not allowed in a category name");
    }
    Ok(parts.join("/"))
}

/// `category` is `ancestor` or below it.
fn within(category: &str, ancestor: &str) -> bool {
    category == ancestor
        || category
            .strip_prefix(ancestor)
            .is_some_and(|rest| rest.starts_with('/'))
}

impl Categories {
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Every category, parents that only exist through their children included, sorted.
    pub fn names(&self) -> Vec<String> {
        let mut all = BTreeSet::new();
        for name in self.map.keys() {
            let mut end = 0;
            for part in name.split('/') {
                end += part.len();
                all.insert(name[..end].to_string());
                end += 1;
            }
        }
        all.into_iter().collect()
    }

    pub fn has(&self, category: &str) -> bool {
        self.map.keys().any(|name| within(name, category))
    }

    /// The cells listed for `category` itself.
    pub fn cells(&self, category: &str) -> Vec<&str> {
        self.map
            .get(category)
            .map(|cells| cells.iter().map(String::as_str).collect())
            .unwrap_or_default()
    }

    /// The cells of `category` and of the categories below it.
    pub fn cells_under(&self, category: &str) -> BTreeSet<&str> {
        self.map
            .iter()
            .filter(|(name, _)| within(name, category))
            .flat_map(|(_, cells)| cells.iter().map(String::as_str))
            .collect()
    }

    /// `cell` is in `category` or in a category below it.
    pub fn contains(&self, category: &str, cell: &str) -> bool {
        self.map
            .iter()
            .any(|(name, cells)| within(name, category) && cells.contains(cell))
    }

    /// The categories `cell` is listed for.
    pub fn of_cell(&self, cell: &str) -> Vec<&str> {
        self.map
            .iter()
            .filter(|(_, cells)| cells.contains(cell))
            .map(|(name, _)| name.as_str())
            .collect()
    }

    /// `cell` is in no category.
    pub fn is_uncategorised(&self, cell: &str) -> bool {
        !self.map.values().any(|cells| cells.contains(cell))
    }

    /// The categories as a tree.
    pub fn tree(&self) -> Vec<Node> {
        fn insert(nodes: &mut Vec<Node>, parts: &[&str], prefix: &str, cells: &BTreeSet<String>) {
            let Some((first, rest)) = parts.split_first() else {
                return;
            };
            let path = if prefix.is_empty() {
                first.to_string()
            } else {
                format!("{prefix}/{first}")
            };
            let at = match nodes.iter().position(|n| n.name == *first) {
                Some(at) => at,
                None => {
                    nodes.push(Node {
                        name: first.to_string(),
                        path: path.clone(),
                        ..Node::default()
                    });
                    nodes.len() - 1
                }
            };
            if rest.is_empty() {
                nodes[at].cells = cells.iter().cloned().collect();
            } else {
                insert(&mut nodes[at].children, rest, &path, cells);
            }
        }
        fn sort(nodes: &mut Vec<Node>) {
            nodes.sort_by_key(|node| node.name.to_lowercase());
            for node in nodes {
                sort(&mut node.children);
            }
        }
        let mut roots = Vec::new();
        for (name, cells) in &self.map {
            let parts: Vec<&str> = name.split('/').collect();
            insert(&mut roots, &parts, "", cells);
        }
        sort(&mut roots);
        roots
    }

    /// Make a category (empty if it is new).
    pub fn add(&mut self, category: &str) {
        self.map.entry(category.to_string()).or_default();
    }

    /// Remove a category and those below it. Says whether there was one.
    pub fn remove(&mut self, category: &str) -> bool {
        let before = self.map.len();
        self.map.retain(|name, _| !within(name, category));
        self.map.len() != before
    }

    /// Rename a category; those below it move with it. Cells of categories that end
    /// up with the same name are joined. Says whether there was one.
    pub fn rename(&mut self, old: &str, new: &str) -> bool {
        let moved: Vec<String> = self
            .map
            .keys()
            .filter(|name| within(name, old))
            .cloned()
            .collect();
        for name in &moved {
            let cells = self.map.remove(name).unwrap_or_default();
            let target = format!("{new}{}", &name[old.len()..]);
            self.map.entry(target).or_default().extend(cells);
        }
        !moved.is_empty()
    }

    pub fn assign(&mut self, category: &str, cell: &str) -> bool {
        self.map
            .entry(category.to_string())
            .or_default()
            .insert(cell.to_string())
    }

    /// Take `cell` out of `category` (not out of the categories below it).
    pub fn unassign(&mut self, category: &str, cell: &str) -> bool {
        self.map
            .get_mut(category)
            .is_some_and(|cells| cells.remove(cell))
    }

    pub fn rename_cell(&mut self, old: &str, new: &str) -> bool {
        let mut changed = false;
        for cells in self.map.values_mut() {
            if cells.remove(old) {
                cells.insert(new.to_string());
                changed = true;
            }
        }
        changed
    }

    pub fn remove_cell(&mut self, cell: &str) -> bool {
        let mut changed = false;
        for cells in self.map.values_mut() {
            changed |= cells.remove(cell);
        }
        changed
    }

    /// Add everything of `other`.
    pub fn merge(&mut self, other: &Categories) {
        for (name, cells) in &other.map {
            self.map
                .entry(name.clone())
                .or_default()
                .extend(cells.iter().cloned());
        }
    }

    /// Leave out the cells that are not in `existing`.
    pub fn only_cells(&mut self, existing: &BTreeSet<&str>) {
        for cells in self.map.values_mut() {
            cells.retain(|cell| existing.contains(cell.as_str()));
        }
    }

    /// Read a `category = ["cell", …]` table.
    fn from_table(table: &dyn toml_edit::TableLike, warnings: &mut Vec<String>) -> Categories {
        let mut out = Categories::default();
        for (name, item) in table.iter() {
            let Ok(name) = normalise(name) else {
                warnings.push(format!("category '{name}': not a usable name; ignored"));
                continue;
            };
            let Some(list) = item.as_array() else {
                warnings.push(format!("category '{name}' must be a list of cell names"));
                continue;
            };
            let cells = out.map.entry(name).or_default();
            cells.extend(list.iter().filter_map(|v| v.as_str()).map(str::to_string));
        }
        out
    }

    /// Write into a `category = ["cell", …]` table, replacing what it holds.
    fn to_table(&self, table: &mut Table) {
        let stale: Vec<String> = table
            .iter()
            .map(|(key, _)| key.to_string())
            .filter(|key| !self.map.contains_key(key))
            .collect();
        for key in stale {
            table.remove(&key);
        }
        for (name, cells) in &self.map {
            let mut list = Array::new();
            for cell in cells {
                list.push(cell.as_str());
            }
            // Long lists: one cell per line reads and diffs better.
            if cells.len() > 6 {
                for item in list.iter_mut() {
                    item.decor_mut().set_prefix("\n    ");
                }
                list.set_trailing("\n");
                list.set_trailing_comma(true);
            }
            table.insert(name, value(list));
        }
    }
}

/// Where the categories of a library are written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Store {
    /// `lib.toml` in the library folder.
    Library,
    /// `categories.toml` in the project folder.
    Project,
}

impl Store {
    pub fn label(self) -> &'static str {
        match self {
            Store::Library => "the library's lib.toml",
            Store::Project => "the project's categories.toml",
        }
    }
}

/// The categories of a library, by where they are kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryCategories {
    /// Everything joined, cells that do not exist left out: what to show.
    pub all: Categories,
    /// From the library's `lib.toml`.
    pub library: Categories,
    /// From the project's `categories.toml`.
    pub project: Categories,
    /// From LibMan `*.group` files in the library folder (read-only).
    pub libman: Categories,
    /// Where changes go.
    pub store: Store,
    /// Problems found while reading.
    pub warnings: Vec<String>,
}

/// Where the categories of `library` are written: its `lib.toml` when CDePT may write
/// to the library and the file is there or may be (a library with view folders), the
/// project's file otherwise.
pub fn store_of(library: &Library) -> Store {
    let writable = !library.read_only && library.error.is_none();
    let has_file = library.path.join(LIBRARY_FILE).is_file();
    if writable && (has_file || library.layout == LibraryLayout::Cells) {
        Store::Library
    } else {
        Store::Project
    }
}

fn project_file(project: &Project) -> PathBuf {
    project.root.join(PROJECT_CATEGORIES_FILE)
}

fn read_doc(file: &Path) -> Result<DocumentMut> {
    if !file.is_file() {
        return Ok(DocumentMut::new());
    }
    let text =
        fs::read_to_string(file).wrap_err_with(|| format!("cannot read {}", file.display()))?;
    text.parse::<DocumentMut>()
        .map_err(|e| eyre!("{}: {}", file.display(), e.to_string().trim_end()))
}

fn write_doc(file: &Path, doc: &DocumentMut) -> Result<()> {
    let tmp = file.with_extension("toml.tmp");
    fs::write(&tmp, doc.to_string()).wrap_err_with(|| format!("cannot write {}", tmp.display()))?;
    fs::rename(&tmp, file).wrap_err_with(|| format!("cannot write {}", file.display()))
}

fn read_table(file: &Path, key: &str, warnings: &mut Vec<String>) -> Categories {
    let doc = match read_doc(file) {
        Ok(doc) => doc,
        Err(e) => {
            warnings.push(format!("{e:#}"));
            return Categories::default();
        }
    };
    match doc.get(key) {
        None => Categories::default(),
        Some(item) => match item.as_table_like() {
            Some(table) => {
                let mut found = Vec::new();
                let out = Categories::from_table(table, &mut found);
                warnings.extend(
                    found
                        .into_iter()
                        .map(|w| format!("{}: {w}", file.display())),
                );
                out
            }
            None => {
                warnings.push(format!("{}: `{key}` must be a table", file.display()));
                Categories::default()
            }
        },
    }
}

/// The categories LibMan keeps in a library folder: every `<name>.group` file is a
/// category, its text the names of its cells.
pub fn read_libman(folder: &Path) -> Categories {
    let mut out = Categories::default();
    let Ok(items) = fs::read_dir(folder) else {
        return out;
    };
    for item in items.flatten() {
        let Ok(name) = item.file_name().into_string() else {
            continue;
        };
        let Some(stem) = name.strip_suffix(LIBMAN_SUFFIX) else {
            continue;
        };
        let (Ok(category), Ok(text)) = (normalise(stem), fs::read_to_string(item.path())) else {
            continue;
        };
        let cells = out.map.entry(category).or_default();
        cells.extend(text.split_whitespace().map(str::to_string));
    }
    out
}

/// Read the categories of a library from everywhere they are kept.
pub fn load(project: &Project, library: &Library) -> LibraryCategories {
    load_at(&project.root, library)
}

/// The files the categories of `library` are read from, for a project in `root`:
/// to tell when they changed.
pub fn files(root: &Path, library: &Library) -> [PathBuf; 2] {
    [
        library.path.join(LIBRARY_FILE),
        root.join(PROJECT_CATEGORIES_FILE),
    ]
}

/// [`load`] for a project by its folder.
pub fn load_at(root: &Path, library: &Library) -> LibraryCategories {
    let mut warnings = Vec::new();
    let from_library = read_table(&library.path.join(LIBRARY_FILE), KEY, &mut warnings);
    let from_project = read_table(
        &root.join(PROJECT_CATEGORIES_FILE),
        &library.name,
        &mut warnings,
    );
    let libman = read_libman(&library.path);
    let mut all = from_library.clone();
    all.merge(&from_project);
    all.merge(&libman);
    let existing: BTreeSet<&str> = library.cells.iter().map(|c| c.name.as_str()).collect();
    all.only_cells(&existing);
    LibraryCategories {
        all,
        library: from_library,
        project: from_project,
        libman,
        store: store_of(library),
        warnings,
    }
}

/// Change the categories of a library where they are kept. `change` is run on the
/// categories of each of our two files and says whether it changed them; the file a
/// library's changes go to is the one given `true`.
fn edit(
    project: &Project,
    library: &Library,
    mut change: impl FnMut(&mut Categories, bool) -> bool,
) -> Result<bool> {
    let store = store_of(library);
    let mut any = false;

    // The library's own file, when CDePT may write to the library.
    let lib_file = library.path.join(LIBRARY_FILE);
    if !library.read_only
        && library.error.is_none()
        && (lib_file.is_file() || store == Store::Library)
    {
        let mut doc = read_doc(&lib_file)?;
        let mut warnings = Vec::new();
        let mut categories = match doc.get(KEY).and_then(Item::as_table_like) {
            Some(table) => Categories::from_table(table, &mut warnings),
            None => Categories::default(),
        };
        if change(&mut categories, store == Store::Library) {
            let item = doc.entry(KEY).or_insert(Item::Table(Table::new()));
            let table = item
                .as_table_mut()
                .ok_or_else(|| eyre!("{}: `{KEY}` must be a table", lib_file.display()))?;
            categories.to_table(table);
            write_doc(&lib_file, &doc)?;
            any = true;
        }
    }

    let file = project_file(project);
    let mut doc = read_doc(&file)?;
    let mut warnings = Vec::new();
    let mut categories = match doc.get(&library.name).and_then(Item::as_table_like) {
        Some(table) => Categories::from_table(table, &mut warnings),
        None => Categories::default(),
    };
    if change(&mut categories, store == Store::Project) {
        if doc.as_table().is_empty() && doc.decor().prefix().is_none() {
            doc.decor_mut().set_prefix(
                "# CDePT: categories of cells, for the libraries that cannot keep them in\n\
                 # their own lib.toml (read-only and flat libraries). One table per library:\n\
                 #   [library]\n\
                 #   \"analog/amplifiers\" = [\"ota5t\"]\n\n",
            );
        }
        if categories.is_empty() {
            doc.remove(&library.name);
        } else {
            let item = doc
                .entry(&library.name)
                .or_insert(Item::Table(Table::new()));
            let table = item
                .as_table_mut()
                .ok_or_else(|| eyre!("{}: `{}` must be a table", file.display(), library.name))?;
            categories.to_table(table);
        }
        write_doc(&file, &doc)?;
        any = true;
    }
    Ok(any)
}

fn library<'a>(project: &'a Project, name: &str) -> Result<&'a Library> {
    project
        .library(name)
        .ok_or_else(|| eyre!("no library '{name}'"))
}

/// A category that only LibMan's file defines cannot be changed by us.
fn not_libman_only(project: &Project, lib: &Library, category: &str) -> Result<()> {
    let found = load(project, lib);
    if found.libman.has(category) && !found.library.has(category) && !found.project.has(category) {
        bail!(
            "category '{category}' of library '{}' is kept by LibMan (a {LIBMAN_SUFFIX} file in \
             the library folder); adopt LibMan's categories first to change it",
            lib.name
        );
    }
    Ok(())
}

/// Make a new, empty category.
pub fn add(project: &Project, library_name: &str, category: &str) -> Result<()> {
    let lib = library(project, library_name)?;
    let category = normalise(category)?;
    if load(project, lib).all.has(&category) {
        bail!("library '{library_name}' already has a category '{category}'");
    }
    edit(project, lib, |categories, target| {
        if target {
            categories.add(&category);
        }
        target
    })?;
    Ok(())
}

/// Remove a category and the categories below it. The cells stay.
pub fn remove(project: &Project, library_name: &str, category: &str) -> Result<()> {
    let lib = library(project, library_name)?;
    let category = normalise(category)?;
    not_libman_only(project, lib, &category)?;
    if !edit(project, lib, |categories, _| categories.remove(&category))? {
        bail!("library '{library_name}' has no category '{category}'");
    }
    Ok(())
}

/// Rename (or move: `a/b` → `c/b`) a category, with the categories below it.
pub fn rename(project: &Project, library_name: &str, old: &str, new: &str) -> Result<()> {
    let lib = library(project, library_name)?;
    let (old, new) = (normalise(old)?, normalise(new)?);
    if within(&new, &old) && new != old {
        bail!("a category cannot be moved into itself");
    }
    not_libman_only(project, lib, &old)?;
    if !edit(project, lib, |categories, _| categories.rename(&old, &new))? {
        bail!("library '{library_name}' has no category '{old}'");
    }
    Ok(())
}

/// Put cells into a category (made if it is new). Cells the library does not have are
/// refused.
pub fn assign(project: &Project, library_name: &str, category: &str, cells: &[&str]) -> Result<()> {
    let lib = library(project, library_name)?;
    let category = normalise(category)?;
    if let Some(missing) = cells.iter().find(|cell| lib.cell(cell).is_none()) {
        bail!("no cell '{missing}' in library '{library_name}'");
    }
    edit(project, lib, |categories, target| {
        if !target {
            return false;
        }
        let mut changed = !categories.map.contains_key(&category);
        categories.add(&category);
        for cell in cells {
            changed |= categories.assign(&category, cell);
        }
        changed
    })?;
    Ok(())
}

/// Take cells out of a category (not out of the categories below it).
pub fn unassign(
    project: &Project,
    library_name: &str,
    category: &str,
    cells: &[&str],
) -> Result<()> {
    let lib = library(project, library_name)?;
    let category = normalise(category)?;
    let found = load(project, lib);
    if let Some(cell) = cells.iter().find(|cell| {
        found.libman.cells(&category).contains(*cell)
            && !found.library.cells(&category).contains(*cell)
            && !found.project.cells(&category).contains(*cell)
    }) {
        bail!(
            "'{cell}' is in category '{category}' through LibMan's {LIBMAN_SUFFIX} file; adopt \
             LibMan's categories first to change it"
        );
    }
    edit(project, lib, |categories, _| {
        let mut changed = false;
        for cell in cells {
            changed |= categories.unassign(&category, cell);
        }
        changed
    })?;
    Ok(())
}

/// A cell was renamed: keep it in its categories.
pub fn cell_renamed(project: &Project, library_name: &str, old: &str, new: &str) -> Result<()> {
    let lib = library(project, library_name)?;
    edit(project, lib, |categories, _| {
        categories.rename_cell(old, new)
    })?;
    Ok(())
}

/// A cell was deleted: take it out of its categories.
pub fn cell_removed(project: &Project, library_name: &str, cell: &str) -> Result<()> {
    let lib = library(project, library_name)?;
    edit(project, lib, |categories, _| categories.remove_cell(cell))?;
    Ok(())
}

/// Add `more` to the categories of a library, in the file its changes go to. Returns
/// how many (category, cell) pairs were new.
pub fn add_all(project: &Project, library_name: &str, more: &Categories) -> Result<usize> {
    let lib = library(project, library_name)?;
    let mut added = 0;
    edit(project, lib, |categories, target| {
        if !target {
            return false;
        }
        let mut changed = false;
        for (name, cells) in &more.map {
            changed |= !categories.map.contains_key(name);
            categories.add(name);
            for cell in cells {
                if categories.assign(name, cell) {
                    added += 1;
                    changed = true;
                }
            }
        }
        changed
    })?;
    Ok(added)
}

/// Copy the categories LibMan keeps in the library folder (`*.group`) into our own
/// store, so they can be changed. LibMan's files are left as they are.
pub fn adopt_libman(project: &Project, library_name: &str) -> Result<usize> {
    let lib = library(project, library_name)?;
    let found = read_libman(&lib.path);
    if found.is_empty() {
        bail!("library '{library_name}' has no LibMan category files ({LIBMAN_SUFFIX})");
    }
    add_all(project, library_name, &found)
}

/// Categories CDePT can work out by itself for a library, for the user to accept:
///
/// - cells with an xschem symbol: by the symbol's `type` (`nmos`, `resistor`,
///   `subcircuit`, …), pins and labels left out;
/// - otherwise, when the names look like a standard-cell library's
///   (`<prefix>_and2_1`): by the function (`and`, `nand`, `dff`, …).
///
/// Nothing is written: pass the result to [`add_all`].
pub fn suggest(library: &Library) -> Categories {
    let mut out = Categories::default();
    for cell in &library.cells {
        let symbol = cell
            .entries
            .iter()
            .find(|e| e.format == Some(Format::XschemSymbol))
            .and_then(|e| e.main_file.as_ref());
        let Some(file) = symbol else {
            continue;
        };
        let Ok(text) = fs::read_to_string(file) else {
            continue;
        };
        let kind = crate::link::schematic::SymbolInfo::parse(&text).kind;
        let kind = kind.trim();
        if !kind.is_empty() && normalise(kind).is_ok() {
            out.assign(kind, &cell.name);
        }
    }
    // One type for (nearly) everything says nothing: standard-cell symbols are all
    // `primitive` or `subcircuit`. Their names say more.
    let one_type = out.map.len() == 1
        && out
            .map
            .values()
            .all(|cells| cells.len() * 10 >= library.cells.len() * 9);
    if !out.is_empty() && !one_type {
        return out;
    }
    let names: Vec<&str> = library.cells.iter().map(|c| c.name.as_str()).collect();
    let mut by_name = Categories::default();
    for (function, cells) in by_function(&names) {
        for cell in cells {
            by_name.assign(&function, cell);
        }
    }
    if by_name.is_empty() { out } else { by_name }
}

/// Group standard-cell names by function: the common prefix (`sg13g2_`,
/// `sky130_fd_sc_hd__`) and the drive strength (`_1`, `_2`) go, the letters that
/// follow are the function. Empty unless most names fit and several share a function.
fn by_function<'a>(names: &[&'a str]) -> BTreeMap<String, Vec<&'a str>> {
    let mut out: BTreeMap<String, Vec<&'a str>> = BTreeMap::new();
    if names.len() < 8 {
        return out;
    }
    // The common prefix, cut after its last '_'.
    let first = names[0];
    let mut common = first.len();
    for name in names {
        common = common.min(
            first
                .bytes()
                .zip(name.bytes())
                .take_while(|(a, b)| a == b)
                .count(),
        );
    }
    let prefix = first[..common].rfind('_').map_or(0, |at| at + 1);
    let mut fitting = 0;
    let mut with_drive = 0;
    for name in names {
        let rest = &name[prefix..];
        // `and2_1` → `and2`: the part after the last '_' is a drive strength when it
        // starts with a digit.
        let base = match rest.rfind('_') {
            Some(at) if rest[at + 1..].starts_with(|c: char| c.is_ascii_digit()) => {
                with_drive += 1;
                &rest[..at]
            }
            _ => rest,
        };
        let function: String = base
            .chars()
            .take_while(|c| c.is_ascii_alphabetic())
            .collect::<String>()
            .to_ascii_lowercase();
        if function.is_empty() {
            continue;
        }
        fitting += 1;
        out.entry(function).or_default().push(name);
    }
    let shared = out.values().filter(|cells| cells.len() > 1).count();
    // A library's own prefix, or drive strengths on most names, make it one.
    let looks_like_one = prefix > 0 || with_drive * 10 >= names.len() * 8;
    if !looks_like_one || fitting * 10 < names.len() * 8 || shared < 3 {
        out.clear();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Categories {
        let mut c = Categories::default();
        c.assign("analog/amplifiers", "ota5t");
        c.assign("analog/amplifiers", "ota_folded");
        c.assign("analog", "bandgap");
        c.assign("digital", "inv");
        c.assign("favourites", "ota5t");
        c
    }

    #[test]
    fn categories_nest_and_cells_can_be_in_several() {
        let c = sample();
        assert_eq!(
            c.names(),
            ["analog", "analog/amplifiers", "digital", "favourites"]
        );
        assert_eq!(c.cells("analog"), ["bandgap"]);
        assert_eq!(
            c.cells_under("analog").into_iter().collect::<Vec<_>>(),
            ["bandgap", "ota5t", "ota_folded"]
        );
        assert!(c.contains("analog", "ota5t"));
        assert!(!c.contains("digital", "ota5t"));
        assert_eq!(c.of_cell("ota5t"), ["analog/amplifiers", "favourites"]);
        assert!(c.is_uncategorised("buf"));
        // "analogue" is not below "analog".
        let mut d = c.clone();
        d.assign("analogue", "x");
        assert!(!d.cells_under("analog").contains("x"));

        let tree = c.tree();
        assert_eq!(tree.len(), 3);
        assert_eq!(tree[0].name, "analog");
        assert_eq!(tree[0].children[0].path, "analog/amplifiers");
        assert_eq!(tree[0].count(), 3);
    }

    #[test]
    fn a_parent_can_exist_only_through_its_children() {
        let mut c = Categories::default();
        c.assign("a/b/c", "x");
        assert_eq!(c.names(), ["a", "a/b", "a/b/c"]);
        assert!(c.has("a/b"));
        let tree = c.tree();
        assert_eq!(tree[0].children[0].children[0].cells, ["x"]);
        assert!(tree[0].cells.is_empty());
    }

    #[test]
    fn rename_moves_the_categories_below() {
        let mut c = sample();
        assert!(c.rename("analog", "ana"));
        assert_eq!(
            c.names(),
            ["ana", "ana/amplifiers", "digital", "favourites"]
        );
        // Renaming onto an existing category joins them.
        assert!(c.rename("favourites", "digital"));
        assert_eq!(c.cells("digital"), ["inv", "ota5t"]);
        assert!(!c.rename("nothing", "x"));
        assert!(c.remove("ana"));
        assert_eq!(c.names(), ["digital"]);
    }

    #[test]
    fn cells_follow_renames_and_deletions() {
        let mut c = sample();
        assert!(c.rename_cell("ota5t", "ota"));
        assert_eq!(c.of_cell("ota"), ["analog/amplifiers", "favourites"]);
        assert!(c.remove_cell("ota"));
        assert!(c.is_uncategorised("ota"));
        // The category stays, empty.
        assert!(c.has("favourites"));
    }

    #[test]
    fn names_are_normalised() {
        assert_eq!(normalise(" analog / amps ").unwrap(), "analog/amps");
        assert!(normalise("a//b").is_err());
        assert!(normalise("").is_err());
        assert!(normalise("a/").is_err());
    }

    #[test]
    fn standard_cells_group_by_function() {
        let names = [
            "sg13g2_and2_1",
            "sg13g2_and2_2",
            "sg13g2_and3_1",
            "sg13g2_nand2_1",
            "sg13g2_nand2_2",
            "sg13g2_inv_1",
            "sg13g2_inv_2",
            "sg13g2_dfrbp_1",
            "sg13g2_a21oi_1",
        ];
        let groups = by_function(&names);
        assert_eq!(groups["and"].len(), 3);
        assert_eq!(groups["nand"].len(), 2);
        assert_eq!(groups["inv"], ["sg13g2_inv_1", "sg13g2_inv_2"]);
        assert_eq!(groups["a"], ["sg13g2_a21oi_1"]);
        // Names without a common prefix are no standard-cell library.
        let other = ["ota", "bandgap", "inv", "buf", "nand", "dff", "mux", "adc"];
        assert!(by_function(&other).is_empty());
    }
}
