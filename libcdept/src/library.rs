//! A [`Library`] is a named folder of cells.
//!
//! Two folder layouts are understood:
//!
//! - [`LibraryLayout::Cells`]: `<library>/<cell>/<view>/<files>`. Every folder of the
//!   library is a cell, every folder of a cell is a view. A design file placed directly
//!   in the cell folder also counts as a view.
//! - [`LibraryLayout::Flat`]: `<library>/<cell>.<ext>`. Files sharing a name form a
//!   cell and every file is a view. This is how xschem symbol libraries, GDS folders
//!   and most PDK reference libraries are laid out, so they can be used in place.
//!
//! An optional `lib.toml` in the library folder holds its description.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use eyre::{Context, Result, bail, eyre};
use toml_edit::DocumentMut;

use crate::{Cell, Entry, Format, Template, ViewKind};

/// Metadata file inside a library folder.
pub const LIBRARY_FILE: &str = "lib.toml";

/// How cells and views map to folders and files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LibraryLayout {
    #[default]
    Cells,
    Flat,
}

impl LibraryLayout {
    pub fn name(self) -> &'static str {
        match self {
            Self::Cells => "cells",
            Self::Flat => "flat",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "cells" => Some(Self::Cells),
            "flat" => Some(Self::Flat),
            _ => None,
        }
    }

    /// Guess the layout of an existing folder: design files directly inside it (and no
    /// `lib.toml`) mean a flat library.
    pub fn detect(path: &Path) -> Self {
        if path.join(LIBRARY_FILE).is_file() {
            return Self::Cells;
        }
        let has_design_files = list(path).is_ok_and(|items| {
            items
                .iter()
                .any(|(name, p)| p.is_file() && Format::split_name(name).is_some())
        });
        if has_design_files {
            Self::Flat
        } else {
            Self::Cells
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Library {
    pub name: String,
    /// The library folder (absolute).
    pub path: PathBuf,
    pub layout: LibraryLayout,
    /// Reference libraries (e.g. PDK primitives) cannot be modified.
    pub read_only: bool,
    pub description: Option<String>,
    /// The file that defines this library (the project file or one it includes).
    pub defined_in: PathBuf,
    /// Why the library could not be read, e.g. a missing folder.
    pub error: Option<String>,
    /// Cells, sorted by name.
    pub cells: Vec<Cell>,
}

impl Library {
    pub fn cell(&self, name: &str) -> Option<&Cell> {
        self.cells.iter().find(|c| c.name == name)
    }

    /// Read the cells and views from disk again.
    pub fn scan(&mut self) {
        self.cells.clear();
        self.description = None;
        self.error = None;
        if !self.path.is_dir() {
            self.error = Some(format!("folder not found: {}", self.path.display()));
            return;
        }
        self.description = read_description(&self.path);
        let cells = match self.layout {
            LibraryLayout::Cells => scan_cells(&self.path),
            LibraryLayout::Flat => scan_flat(&self.path),
        };
        match cells {
            Ok(cells) => self.cells = cells,
            Err(e) => self.error = Some(format!("{e:#}")),
        }
    }

    fn writable(&self) -> Result<()> {
        if self.read_only {
            bail!("library '{}' is read-only", self.name);
        }
        if let Some(error) = &self.error {
            bail!("library '{}' is not usable: {error}", self.name);
        }
        Ok(())
    }

    fn existing(&self, cell: &str) -> Result<&Cell> {
        self.cell(cell)
            .ok_or_else(|| eyre!("no cell '{cell}' in library '{}'", self.name))
    }

    fn free_name(&self, cell: &str) -> Result<()> {
        validate_name("cell", cell)?;
        if self.cell(cell).is_some() || self.path.join(cell).exists() {
            bail!("'{cell}' already exists in library '{}'", self.name);
        }
        Ok(())
    }

    /// Create an empty cell. A flat library has no empty cells: use
    /// [`create_view`](Self::create_view) there.
    pub fn create_cell(&self, name: &str) -> Result<PathBuf> {
        self.writable()?;
        self.free_name(name)?;
        if self.layout == LibraryLayout::Flat {
            bail!(
                "library '{}' is flat: a cell exists once it has a view",
                self.name
            );
        }
        let dir = self.path.join(name);
        fs::create_dir(&dir).wrap_err_with(|| format!("cannot create {}", dir.display()))?;
        Ok(dir)
    }

    /// Create a view from a template, creating the cell if needed. Returns the new file.
    pub fn create_view(&self, cell: &str, template: Template) -> Result<PathBuf> {
        self.writable()?;
        validate_name("cell", cell)?;
        let file = match self.layout {
            LibraryLayout::Cells => self
                .path
                .join(cell)
                .join(template.view_name())
                .join(template.file_name(cell)),
            LibraryLayout::Flat => self.path.join(template.file_name(cell)),
        };
        let view_taken = self
            .cell(cell)
            .is_some_and(|c| c.entry(template.view_name()).is_some());
        if view_taken || file.exists() {
            bail!("cell '{cell}' already has a {} view", template.view_name());
        }
        let dir = file.parent().expect("a view file has a parent folder");
        fs::create_dir_all(dir).wrap_err_with(|| format!("cannot create {}", dir.display()))?;
        fs::write(&file, template.contents(cell))
            .wrap_err_with(|| format!("cannot write {}", file.display()))?;
        Ok(file)
    }

    /// Rename a cell, together with the files named after it.
    ///
    /// References to the cell from other cells (xschem instances, GDS cell references)
    /// and the structure name inside layout files are not rewritten yet.
    pub fn rename_cell(&self, old: &str, new: &str) -> Result<()> {
        self.writable()?;
        let cell = self.existing(old)?;
        self.free_name(new)?;
        match &cell.path {
            Some(dir) => {
                let target = self.path.join(new);
                fs::rename(dir, &target)
                    .wrap_err_with(|| format!("cannot rename {}", dir.display()))?;
                rename_cell_files(&target, old, new)
            }
            None => {
                let moves = flat_targets(cell, old, new, &self.path)?;
                for (from, to) in moves {
                    fs::rename(&from, &to)
                        .wrap_err_with(|| format!("cannot rename {}", from.display()))?;
                }
                Ok(())
            }
        }
    }

    /// Copy a cell into `to` (which may be this library) under the name `new`.
    pub fn copy_cell(&self, cell: &str, to: &Library, new: &str) -> Result<()> {
        to.writable()?;
        let source = self.existing(cell)?;
        to.free_name(new)?;
        match (&source.path, to.layout) {
            (Some(dir), LibraryLayout::Cells) => {
                let target = to.path.join(new);
                copy_dir(dir, &target)?;
                rename_cell_files(&target, cell, new)
            }
            (None, LibraryLayout::Cells) => {
                for entry in &source.entries {
                    let dir = to.path.join(new).join(&entry.name);
                    fs::create_dir_all(&dir)
                        .wrap_err_with(|| format!("cannot create {}", dir.display()))?;
                    for file in &entry.files {
                        let name = file_name(file);
                        let name = renamed(&name, cell, new).unwrap_or(name);
                        copy_file(file, &dir.join(name))?;
                    }
                }
                Ok(())
            }
            (_, LibraryLayout::Flat) => {
                for (from, to) in flat_targets(source, cell, new, &to.path)? {
                    copy_file(&from, &to)?;
                }
                Ok(())
            }
        }
    }

    /// Delete a cell and all its views from disk.
    pub fn delete_cell(&self, name: &str) -> Result<()> {
        self.writable()?;
        let cell = self.existing(name)?;
        match &cell.path {
            Some(dir) => {
                fs::remove_dir_all(dir).wrap_err_with(|| format!("cannot delete {}", dir.display()))
            }
            None => {
                for entry in &cell.entries {
                    remove_entry(entry)?;
                }
                Ok(())
            }
        }
    }

    /// Delete one view of a cell from disk.
    pub fn delete_view(&self, cell: &str, view: &str) -> Result<()> {
        self.writable()?;
        let entry = self
            .existing(cell)?
            .entry(view)
            .ok_or_else(|| eyre!("cell '{cell}' has no view '{view}'"))?;
        remove_entry(entry)
    }
}

/// Names CDePT creates: letters, digits, `_`, `-` and `.`, not starting with `.` or `-`.
/// Existing folders with other names are still listed.
pub fn validate_name(what: &str, name: &str) -> Result<()> {
    let mut chars = name.chars();
    let first_ok = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
    if !first_ok {
        bail!("a {what} name must start with a letter, a digit or '_'");
    }
    if let Some(bad) = chars.find(|c| !(c.is_ascii_alphanumeric() || "_-.".contains(*c))) {
        bail!("'{bad}' is not allowed in a {what} name (use letters, digits, '_', '-', '.')");
    }
    Ok(())
}

/// Create a library folder with its `lib.toml`, if they do not exist yet.
pub(crate) fn create_library_folder(path: &Path, name: &str) -> Result<()> {
    fs::create_dir_all(path).wrap_err_with(|| format!("cannot create {}", path.display()))?;
    let file = path.join(LIBRARY_FILE);
    if !file.exists() {
        let text = format!("# CDePT library '{name}'\ndescription = \"\"\n");
        fs::write(&file, text).wrap_err_with(|| format!("cannot write {}", file.display()))?;
    }
    Ok(())
}

fn read_description(library: &Path) -> Option<String> {
    let text = fs::read_to_string(library.join(LIBRARY_FILE)).ok()?;
    let doc = text.parse::<DocumentMut>().ok()?;
    let description = doc.get("description")?.as_str()?.trim();
    (!description.is_empty()).then(|| description.to_string())
}

/// Visible items of a folder as `(name, path)`, sorted by name.
fn list(dir: &Path) -> Result<Vec<(String, PathBuf)>> {
    let mut items = Vec::new();
    for item in fs::read_dir(dir).wrap_err_with(|| format!("cannot read {}", dir.display()))? {
        let item = item?;
        let Ok(name) = item.file_name().into_string() else {
            continue;
        };
        if !name.starts_with('.') {
            items.push((name, item.path()));
        }
    }
    items.sort();
    Ok(items)
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn scan_cells(library: &Path) -> Result<Vec<Cell>> {
    let mut cells = Vec::new();
    for (name, path) in list(library)? {
        if !path.is_dir() {
            continue;
        }
        let entries = scan_cell(&name, &path)?;
        cells.push(Cell {
            name,
            path: Some(path),
            entries,
        });
    }
    Ok(cells)
}

fn scan_cell(cell: &str, dir: &Path) -> Result<Vec<Entry>> {
    let mut entries: Vec<Entry> = Vec::new();
    let mut loose = Vec::new();
    for (name, path) in list(dir)? {
        if path.is_dir() {
            entries.push(scan_view(cell, name, path)?);
        } else if let Some(format) = Format::of_file(&path) {
            loose.push((name, path, format));
        }
    }
    // Design files directly in the cell folder: one view each, named after their kind.
    for (name, path, format) in loose {
        let view = unique_view_name(&entries, format.kind(), &name);
        entries.push(Entry {
            name: view,
            kind: format.kind(),
            path: path.clone(),
            main_file: Some(path.clone()),
            format: Some(format),
            files: vec![path],
        });
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

fn scan_view(cell: &str, name: String, dir: PathBuf) -> Result<Entry> {
    let files: Vec<PathBuf> = list(&dir)?
        .into_iter()
        .map(|(_, path)| path)
        .filter(|path| path.is_file())
        .collect();
    // The main file is the design file named after the cell, else the first design file.
    let formats: Vec<(&PathBuf, &str, Format)> = files
        .iter()
        .filter_map(|path| {
            let (stem, _) = Format::split_name(path.file_name()?.to_str()?)?;
            Some((path, stem, Format::of_file(path)?))
        })
        .collect();
    let main = formats
        .iter()
        .find(|(_, stem, _)| *stem == cell)
        .or(formats.first());
    let has_setup = files.iter().any(|f| file_name(f) == "setup.toml");
    let kind = if has_setup {
        ViewKind::Testbench
    } else if let Some((_, _, format)) = main {
        format.kind()
    } else {
        ViewKind::from_view_name(&name).unwrap_or(ViewKind::Other)
    };
    Ok(Entry {
        name,
        kind,
        main_file: main.map(|(path, _, _)| (*path).clone()),
        format: main.map(|(_, _, format)| *format),
        path: dir,
        files,
    })
}

fn scan_flat(library: &Path) -> Result<Vec<Cell>> {
    let mut cells: BTreeMap<String, Vec<Entry>> = BTreeMap::new();
    for (name, path) in list(library)? {
        if !path.is_file() {
            continue;
        }
        let Some((stem, _)) = Format::split_name(&name) else {
            continue;
        };
        let Some(format) = Format::of_file(&path) else {
            continue;
        };
        let entries = cells.entry(stem.to_string()).or_default();
        let view = unique_view_name(entries, format.kind(), &name);
        entries.push(Entry {
            name: view,
            kind: format.kind(),
            path: path.clone(),
            main_file: Some(path.clone()),
            format: Some(format),
            files: vec![path],
        });
    }
    Ok(cells
        .into_iter()
        .map(|(name, mut entries)| {
            entries.sort_by(|a, b| a.name.cmp(&b.name));
            Cell {
                name,
                path: None,
                entries,
            }
        })
        .collect())
}

/// `layout`, or `layout_oas` when the cell already has a `layout` view.
fn unique_view_name(entries: &[Entry], kind: ViewKind, file_name: &str) -> String {
    let taken = |name: &str| entries.iter().any(|e| e.name == name);
    if !taken(kind.name()) {
        return kind.name().to_string();
    }
    let ext = file_name
        .split('.')
        .skip(1)
        .collect::<Vec<_>>()
        .join("_")
        .to_ascii_lowercase();
    let base = format!("{}_{ext}", kind.name());
    let mut name = base.clone();
    let mut n = 2;
    while taken(&name) {
        name = format!("{base}_{n}");
        n += 1;
    }
    name
}

/// The new name of a file named after cell `old`: `old.sch` → `new.sch`.
fn renamed(file_name: &str, old: &str, new: &str) -> Option<String> {
    let rest = file_name.strip_prefix(old)?;
    (rest.is_empty() || rest.starts_with('.')).then(|| format!("{new}{rest}"))
}

/// Rename the files named after the cell, in the cell folder and its view folders.
fn rename_cell_files(cell_dir: &Path, old: &str, new: &str) -> Result<()> {
    let mut dirs = vec![cell_dir.to_path_buf()];
    dirs.extend(
        list(cell_dir)?
            .into_iter()
            .map(|(_, path)| path)
            .filter(|path| path.is_dir()),
    );
    for dir in dirs {
        for (name, path) in list(&dir)? {
            if path.is_file()
                && let Some(new_name) = renamed(&name, old, new)
            {
                fs::rename(&path, dir.join(new_name))
                    .wrap_err_with(|| format!("cannot rename {}", path.display()))?;
            }
        }
    }
    Ok(())
}

/// Where the files of `cell` go in the flat folder `dir` under the name `new`.
/// Only files named after the cell have a place in a flat library.
fn flat_targets(cell: &Cell, old: &str, new: &str, dir: &Path) -> Result<Vec<(PathBuf, PathBuf)>> {
    let mut moves = Vec::new();
    for file in cell.entries.iter().flat_map(|e| &e.files) {
        if let Some(name) = renamed(&file_name(file), old, new) {
            let target = dir.join(name);
            if target.exists() {
                bail!("{} already exists", target.display());
            }
            moves.push((file.clone(), target));
        }
    }
    if moves.is_empty() {
        bail!("cell '{old}' has no file named after it to put in a flat library");
    }
    Ok(moves)
}

fn copy_file(from: &Path, to: &Path) -> Result<()> {
    fs::copy(from, to)
        .map(|_| ())
        .wrap_err_with(|| format!("cannot copy {} to {}", from.display(), to.display()))
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to).wrap_err_with(|| format!("cannot create {}", to.display()))?;
    for item in fs::read_dir(from).wrap_err_with(|| format!("cannot read {}", from.display()))? {
        let item = item?;
        let target = to.join(item.file_name());
        if item.path().is_dir() {
            copy_dir(&item.path(), &target)?;
        } else {
            copy_file(&item.path(), &target)?;
        }
    }
    Ok(())
}

fn remove_entry(entry: &Entry) -> Result<()> {
    let result = if entry.path.is_dir() {
        fs::remove_dir_all(&entry.path)
    } else {
        fs::remove_file(&entry.path)
    };
    result.wrap_err_with(|| format!("cannot delete {}", entry.path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn library(dir: &Path, layout: LibraryLayout) -> Library {
        let mut lib = Library {
            name: "lib".into(),
            path: dir.to_path_buf(),
            layout,
            read_only: false,
            description: None,
            defined_in: PathBuf::new(),
            error: None,
            cells: Vec::new(),
        };
        lib.scan();
        lib
    }

    fn names(lib: &Library) -> Vec<(String, Vec<String>)> {
        lib.cells
            .iter()
            .map(|c| {
                (
                    c.name.clone(),
                    c.entries.iter().map(|e| e.name.clone()).collect(),
                )
            })
            .collect()
    }

    fn pairs(items: &[(&str, &[&str])]) -> Vec<(String, Vec<String>)> {
        items
            .iter()
            .map(|(c, v)| (c.to_string(), v.iter().map(|s| s.to_string()).collect()))
            .collect()
    }

    #[test]
    fn cells_layout_create_scan_rename_copy_delete() {
        let tmp = tempfile::tempdir().unwrap();
        let mut lib = library(tmp.path(), LibraryLayout::Cells);
        lib.create_cell("empty").unwrap();
        lib.create_view("ota", Template::XschemSchematic).unwrap();
        lib.scan();
        lib.create_view("ota", Template::GdsLayout).unwrap();
        // A hand-made testbench view and a loose file in the cell folder.
        fs::create_dir(tmp.path().join("ota/tb_ac")).unwrap();
        fs::write(tmp.path().join("ota/tb_ac/setup.toml"), "").unwrap();
        fs::write(tmp.path().join("ota/ota.sym"), "v {xschem}").unwrap();
        lib.scan();
        assert_eq!(
            names(&lib),
            pairs(&[
                ("empty", &[]),
                ("ota", &["layout", "schematic", "symbol", "tb_ac"])
            ])
        );
        let ota = lib.cell("ota").unwrap();
        assert_eq!(ota.entry("tb_ac").unwrap().kind, ViewKind::Testbench);
        assert_eq!(ota.entry("layout").unwrap().format, Some(Format::Gds));
        assert_eq!(
            ota.entry("schematic").unwrap().main_file,
            Some(tmp.path().join("ota/schematic/ota.sch"))
        );
        assert!(lib.create_view("ota", Template::GdsLayout).is_err());
        assert!(lib.create_cell("ota").is_err());
        assert!(lib.create_cell("bad name").is_err());

        lib.rename_cell("ota", "ota2").unwrap();
        lib.scan();
        assert!(tmp.path().join("ota2/schematic/ota2.sch").is_file());
        assert!(tmp.path().join("ota2/ota2.sym").is_file());
        assert!(lib.cell("ota").is_none());

        lib.copy_cell("ota2", &lib, "ota3").unwrap();
        lib.scan();
        assert!(tmp.path().join("ota3/layout/ota3.gds").is_file());
        assert!(tmp.path().join("ota2/layout/ota2.gds").is_file());
        assert!(lib.copy_cell("ota2", &lib, "ota3").is_err());

        lib.delete_view("ota3", "layout").unwrap();
        lib.delete_cell("ota2").unwrap();
        lib.scan();
        assert_eq!(
            names(&lib),
            pairs(&[("empty", &[]), ("ota3", &["schematic", "symbol", "tb_ac"])])
        );
    }

    #[test]
    fn flat_layout_groups_files_by_name() {
        let tmp = tempfile::tempdir().unwrap();
        for file in [
            "nmos.sym",
            "nmos.sch",
            "pmos.sym",
            "inv.gds",
            "inv.oas",
            "README.md",
        ] {
            fs::write(tmp.path().join(file), "").unwrap();
        }
        fs::write(tmp.path().join("filter.sch"), "<Qucs Schematic 24.1.0>").unwrap();
        assert_eq!(LibraryLayout::detect(tmp.path()), LibraryLayout::Flat);
        let mut lib = library(tmp.path(), LibraryLayout::Flat);
        assert_eq!(
            names(&lib),
            pairs(&[
                ("filter", &["schematic"]),
                ("inv", &["layout", "layout_oas"]),
                ("nmos", &["schematic", "symbol"]),
                ("pmos", &["symbol"]),
            ])
        );
        let filter = lib.cell("filter").unwrap().entry("schematic").unwrap();
        assert_eq!(filter.format, Some(Format::QucsSchematic));

        assert!(lib.create_cell("x").is_err());
        lib.create_view("x", Template::XschemSymbol).unwrap();
        lib.rename_cell("nmos", "nfet").unwrap();
        lib.copy_cell("inv", &lib, "buf").unwrap();
        lib.delete_cell("pmos").unwrap();
        lib.scan();
        assert_eq!(
            names(&lib),
            pairs(&[
                ("buf", &["layout", "layout_oas"]),
                ("filter", &["schematic"]),
                ("inv", &["layout", "layout_oas"]),
                ("nfet", &["schematic", "symbol"]),
                ("x", &["symbol"]),
            ])
        );
    }

    #[test]
    fn copies_between_layouts() {
        let flat_dir = tempfile::tempdir().unwrap();
        let cells_dir = tempfile::tempdir().unwrap();
        fs::write(flat_dir.path().join("inv.sch"), "a").unwrap();
        fs::write(flat_dir.path().join("inv.sym"), "b").unwrap();
        let mut flat = library(flat_dir.path(), LibraryLayout::Flat);
        let mut cells = library(cells_dir.path(), LibraryLayout::Cells);

        flat.copy_cell("inv", &cells, "inv_x1").unwrap();
        cells.scan();
        assert!(
            cells_dir
                .path()
                .join("inv_x1/schematic/inv_x1.sch")
                .is_file()
        );
        assert!(cells_dir.path().join("inv_x1/symbol/inv_x1.sym").is_file());

        cells.copy_cell("inv_x1", &flat, "inv_x2").unwrap();
        flat.scan();
        assert_eq!(
            names(&flat),
            pairs(&[
                ("inv", &["schematic", "symbol"]),
                ("inv_x2", &["schematic", "symbol"])
            ])
        );
    }

    #[test]
    fn read_only_libraries_refuse_changes() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir(tmp.path().join("ota")).unwrap();
        let mut lib = library(tmp.path(), LibraryLayout::Cells);
        lib.read_only = true;
        assert!(lib.create_cell("x").is_err());
        assert!(lib.create_view("ota", Template::Verilog).is_err());
        assert!(lib.rename_cell("ota", "x").is_err());
        assert!(lib.delete_cell("ota").is_err());
        assert!(tmp.path().join("ota").is_dir());
    }

    #[test]
    fn missing_folder_is_reported_not_fatal() {
        let lib = library(Path::new("/nonexistent/cdept-lib"), LibraryLayout::Cells);
        assert!(lib.error.is_some());
        assert!(lib.cells.is_empty());
    }
}
