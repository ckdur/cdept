//! A [`Library`] is a named folder of cells.
//!
//! Three folder layouts are understood:
//!
//! - [`LibraryLayout::Cells`]: `<library>/<cell>/<view>/<files>`. Every folder of the
//!   library is a cell, every folder of a cell is a view. A design file placed directly
//!   in the cell folder also counts as a view.
//! - [`LibraryLayout::Flat`]: `<library>/<cell>.<ext>`. Files sharing a name form a
//!   cell and every file is a view. This is how xschem symbol libraries, GDS folders
//!   and most PDK symbol libraries are laid out, so they can be used in place.
//! - [`LibraryLayout::Ref`]: `<library>/<format>/<files>`, the reference libraries of a
//!   PDK (`libs.ref/sky130_fd_sc_hd/{gds,lef,lib,verilog,spice,cdl,mag,…}`). One folder
//!   per file format; CDePT only reads such a library.
//!
//! # Files that hold many cells
//!
//! In flat and reference libraries a file may hold many cells (the GDS of a
//! standard-cell library, its LEF, its Liberty files). [`crate::container`] says what
//! is inside a GDS, OASIS, LEF, Liberty, Verilog, SPICE or CDL file. From that, a file
//! gives these cells:
//!
//! 1. nothing inside, or not a file of those kinds: the cell named after the file
//!    (in a reference library a file with nothing inside is no cell at all:
//!    a technology LEF, a model file);
//! 2. a cell inside is named like the file: that cell. What else is inside is its
//!    own business (the sub-cells of a layout, the sub-circuits of a netlist);
//! 3. a layout with a single top cell: in a flat library the cell named after the
//!    file (as before), in a reference library the top cell;
//! 4. otherwise the file is a collection and every cell inside is a cell of the
//!    library, its view naming the file and the cell in it ([`Entry::member`]). Of a
//!    layout collection the top cells are listed, and those sub-cells that the
//!    library knows from another file (a LEF macro, a `.mag` file).
//!
//! Views of several files that hold the same cell in the same format are one view
//! (the Liberty files of all corners: `liberty`, with every file in
//! [`Entry::files`] and the typical corner as the main file).
//!
//! An optional `lib.toml` in the library folder holds its description.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use eyre::{Context, Result, bail, eyre};
use toml_edit::DocumentMut;

use crate::container::{self, Cache, Index, Reader};
use crate::layout::{self, meta};
use crate::xschem::symbol;
use crate::{Cell, Entry, Format, Template, ViewKind};

/// Metadata file inside a library folder.
pub const LIBRARY_FILE: &str = "lib.toml";

/// How cells and views map to folders and files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LibraryLayout {
    #[default]
    Cells,
    Flat,
    /// `<library>/<format>/<files>`: a PDK reference library. Read only.
    Ref,
}

impl LibraryLayout {
    pub fn name(self) -> &'static str {
        match self {
            Self::Cells => "cells",
            Self::Flat => "flat",
            Self::Ref => "ref",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "cells" => Some(Self::Cells),
            "flat" => Some(Self::Flat),
            "ref" => Some(Self::Ref),
            _ => None,
        }
    }

    /// Guess the layout of an existing folder: design files directly inside it (and no
    /// `lib.toml`) mean a flat library; nothing but format folders (`gds`, `lef`,
    /// `lib`, …) with design files in them, a reference library.
    pub fn detect(path: &Path) -> Self {
        if path.join(LIBRARY_FILE).is_file() {
            return Self::Cells;
        }
        let Ok(items) = list(path) else {
            return Self::Cells;
        };
        let has_design_files = items
            .iter()
            .any(|(name, p)| p.is_file() && Format::split_name(name).is_some());
        if has_design_files {
            return Self::Flat;
        }
        let folders: Vec<&(String, PathBuf)> = items.iter().filter(|(_, p)| p.is_dir()).collect();
        let known = |name: &str| FORMAT_FOLDERS.contains(&name) || OTHER_FOLDERS.contains(&name);
        let holds_files = |(name, dir): &&(String, PathBuf)| {
            FORMAT_FOLDERS.contains(&name.as_str())
                && list(dir).is_ok_and(|inside| {
                    inside.iter().any(|(file, p)| {
                        (p.is_file()
                            && (Format::split_name(file).is_some() || file.ends_with(".tlef")))
                            || (p.is_dir() && TOOL_FOLDERS.contains(&file.as_str()))
                    })
                })
        };
        if !folders.is_empty()
            && folders.iter().all(|(name, _)| known(name))
            && folders.iter().any(holds_files)
        {
            Self::Ref
        } else {
            Self::Cells
        }
    }

    /// CDePT creates, renames and deletes cells in libraries of this layout. A
    /// reference library is only read.
    pub fn editable(self) -> bool {
        self != Self::Ref
    }
}

/// Folders of a reference library that hold design files, one format each.
const FORMAT_FOLDERS: &[&str] = &[
    "gds", "oas", "oasis", "lef", "techlef", "lib", "liberty", "spice", "cdl", "verilog", "vhdl",
    "mag", "maglef", "sch", "sym",
];
/// Other folders a reference library may have; they hold no cells.
const OTHER_FOLDERS: &[&str] = &["doc", "docs", "scripts", "html", "pdf"];
/// Folders inside a format folder, one per tool (`sym/xschem`, `sch/qucs-s`).
const TOOL_FOLDERS: &[&str] = &["xschem", "qucs-s", "qucs"];

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
    /// Reference libraries: the files that are the main file of a view before the
    /// others (the files `tech.toml` names for the library); empty: the folder scan
    /// alone decides. Set by the project, not read from the PDK here.
    pub preferred: std::collections::BTreeSet<PathBuf>,
}

impl Library {
    pub fn cell(&self, name: &str) -> Option<&Cell> {
        self.cells.iter().find(|c| c.name == name)
    }

    /// Read the cells and views from disk again. Files that hold many cells are read
    /// through the user's index cache ([`Cache::user`]).
    ///
    /// Works on this library alone, so libraries can be scanned on other threads.
    pub fn scan(&mut self) {
        self.scan_with(&Cache::user());
    }

    /// [`Library::scan`] with the index cache to use.
    pub fn scan_with(&mut self, cache: &Cache) {
        // The project's own suffixes (`[suffixes]` in cdept.toml) apply to this scan.
        let _suffixes = crate::formats::enter(&self.path);
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
            LibraryLayout::Flat => scan_flat(&self.path, cache),
            LibraryLayout::Ref => scan_ref(&self.path, cache, &self.preferred),
        };
        match cells {
            Ok(cells) => self.cells = cells,
            Err(e) => self.error = Some(format!("{e:#}")),
        }
    }

    pub(crate) fn writable(&self) -> Result<()> {
        if self.read_only {
            bail!("library '{}' is read-only", self.name);
        }
        if let Some(error) = &self.error {
            bail!("library '{}' is not usable: {error}", self.name);
        }
        if !self.layout.editable() {
            bail!(
                "library '{}' is a reference library (layout \"ref\"): CDePT only reads it",
                self.name
            );
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
    ///
    /// An xschem symbol in a `cells` library gets a `schematic=` attribute naming the
    /// schematic view of its cell: see [`symbol`].
    pub fn create_view(&self, cell: &str, template: Template) -> Result<PathBuf> {
        self.create_view_with(cell, template, &crate::template::Setup::default())
    }

    /// [`create_view`](Self::create_view) for a project's technology: the database
    /// unit of a new layout, magic's technology name
    /// ([`Project::template_setup`](crate::Project::template_setup)).
    pub fn create_view_with(
        &self,
        cell: &str,
        template: Template,
        setup: &crate::template::Setup,
    ) -> Result<PathBuf> {
        self.writable()?;
        validate_name("cell", cell)?;
        let file = match self.layout {
            LibraryLayout::Cells => self
                .path
                .join(cell)
                .join(template.view_name())
                .join(template.file_name(cell)),
            LibraryLayout::Flat | LibraryLayout::Ref => self.path.join(template.file_name(cell)),
        };
        let view_taken = self
            .cell(cell)
            .is_some_and(|c| c.entry(template.view_name()).is_some());
        if view_taken || file.exists() {
            bail!("cell '{cell}' already has a {} view", template.view_name());
        }
        let dir = file.parent().expect("a view file has a parent folder");
        fs::create_dir_all(dir).wrap_err_with(|| format!("cannot create {}", dir.display()))?;
        let mut contents = template.contents_with(cell, setup);
        if template == Template::XschemSymbol
            && let Some(schematic) = symbol::for_new_symbol(self, cell, self.cell(cell))
            && let Ok(text) = std::str::from_utf8(&contents)
        {
            contents = symbol::set_schematic(text, Some(&schematic))
                .map_err(|e| eyre!("cannot create the symbol of '{cell}': {e}"))?
                .into_bytes();
        }
        fs::write(&file, contents).wrap_err_with(|| format!("cannot write {}", file.display()))?;
        if template == Template::GdsLayout {
            meta::Meta::new(&self.name, cell, template.view_name(), cell).write_for(&file)?;
        }
        Ok(file)
    }

    /// Rename a cell, together with the files named after it.
    ///
    /// The `schematic=` attribute of the cell's xschem symbols follows, and so does the
    /// top structure name inside its own layout files. References to the cell from other
    /// cells are not rewritten here: xschem instances not yet, layout views by
    /// [`Library::rename_cell_following`].
    pub fn rename_cell(&self, old: &str, new: &str) -> Result<()> {
        self.writable()?;
        let cell = self.existing(old)?;
        self.free_name(new)?;
        not_a_member(cell, "renamed")?;
        match &cell.path {
            Some(dir) => {
                let target = self.path.join(new);
                fs::rename(dir, &target)
                    .wrap_err_with(|| format!("cannot rename {}", dir.display()))?;
                rename_cell_files(&target, old, new)?;
            }
            None => {
                let moves = flat_targets(cell, old, new, &self.path)?;
                for (from, to) in moves {
                    fs::rename(&from, &to)
                        .wrap_err_with(|| format!("cannot rename {}", from.display()))?;
                }
            }
        }
        self.relink_symbols(new, self, old)?;
        self.retarget_layouts(new, old)
    }

    /// [`Library::rename_cell`], then make the layout views of `project` that place the
    /// cell follow ([`layout::follow::cell_renamed`]: sidecar entries, and the structure
    /// names inside the files). `project` holds this library and is not rescanned yet.
    /// Views that could not follow (read-only libraries, write errors) are in the result.
    pub fn rename_cell_following(
        &self,
        project: &crate::Project,
        old: &str,
        new: &str,
    ) -> Result<Vec<layout::follow::Followed>> {
        self.rename_cell(old, new)?;
        Ok(layout::follow::cell_renamed(project, &self.name, old, new))
    }

    /// Copy a cell into `to` (which may be this library) under the name `new`.
    ///
    /// The `schematic=` attribute of the copied xschem symbols is rewritten to point at
    /// the copy.
    pub fn copy_cell(&self, cell: &str, to: &Library, new: &str) -> Result<()> {
        to.writable()?;
        let source = self.existing(cell)?;
        to.free_name(new)?;
        not_a_member(source, "copied")?;
        match (&source.path, to.layout) {
            (Some(dir), LibraryLayout::Cells) => {
                let target = to.path.join(new);
                // Leaves out what the library's copy rule says (result folders).
                crate::project_edit::copy_cell_dir(dir, &target, &self.path)?;
                rename_cell_files(&target, cell, new)?;
            }
            (None, LibraryLayout::Cells) => {
                for entry in &source.entries {
                    let dir = to.path.join(new).join(&entry.name);
                    fs::create_dir_all(&dir)
                        .wrap_err_with(|| format!("cannot create {}", dir.display()))?;
                    for file in &entry.files {
                        let name = file_name(file);
                        let name = renamed(&name, cell, new).unwrap_or(name);
                        for (from, name) in with_sidecar(file, name) {
                            copy_file(&from, &dir.join(name))?;
                        }
                    }
                }
            }
            (_, LibraryLayout::Flat | LibraryLayout::Ref) => {
                for (from, to) in flat_targets(source, cell, new, &to.path)? {
                    copy_file(&from, &to)?;
                }
            }
        }
        to.relink_symbols(new, self, cell)?;
        to.retarget_layouts(new, cell)
    }

    /// After cell `old` was copied or renamed to `new` in this library: the GDS layout
    /// views name their top structure `new` and their sidecars say library/cell/view.
    /// A layout that cannot be rewritten (not valid GDSII) is logged and left as is.
    fn retarget_layouts(&self, new: &str, old: &str) -> Result<()> {
        let cell = match self.layout {
            LibraryLayout::Cells => {
                let dir = self.path.join(new);
                Cell {
                    name: new.to_string(),
                    entries: scan_cell(new, &dir)?,
                    path: Some(dir),
                }
            }
            LibraryLayout::Flat | LibraryLayout::Ref => {
                let Some(cell) = scan_flat(&self.path, &Cache::user())?
                    .into_iter()
                    .find(|c| c.name == new)
                else {
                    return Ok(());
                };
                cell
            }
        };
        for entry in cell.entries.iter().filter(|e| !e.is_member()) {
            if let (Some(Format::Gds | Format::Oasis), Some(file)) =
                (entry.format, &entry.main_file)
                && let Err(e) = layout::retarget(file, &self.name, new, &entry.name, old, new)
            {
                tracing::warn!("{e:#}");
            }
        }
        Ok(())
    }

    /// After cell `old` of `source` was copied or renamed to `new` in this library:
    /// make the `schematic=` attribute of its xschem symbols follow.
    fn relink_symbols(&self, new: &str, source: &Library, old: &str) -> Result<()> {
        let cell = match self.layout {
            LibraryLayout::Cells => {
                let dir = self.path.join(new);
                Cell {
                    name: new.to_string(),
                    entries: scan_cell(new, &dir)?,
                    path: Some(dir),
                }
            }
            LibraryLayout::Flat | LibraryLayout::Ref => {
                let Some(cell) = scan_flat(&self.path, &Cache::user())?
                    .into_iter()
                    .find(|c| c.name == new)
                else {
                    return Ok(());
                };
                cell
            }
        };
        symbol::follow_cell(self, &cell, source, old)
    }

    /// Delete a cell and all its views from disk.
    pub fn delete_cell(&self, name: &str) -> Result<()> {
        self.writable()?;
        let cell = self.existing(name)?;
        not_a_member(cell, "deleted")?;
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

/// Refuse to copy, rename or delete a cell that is (also) one cell of a file holding
/// several: the file belongs to all of them.
fn not_a_member(cell: &Cell, done: &str) -> Result<()> {
    match cell.entries.iter().find(|e| e.is_member()) {
        Some(entry) => Err(eyre!(
            "cell '{}' cannot be {done}: its {} view is one cell of {}, which holds other \
             cells too",
            cell.name,
            entry.name,
            entry.main_file.as_deref().unwrap_or(&entry.path).display()
        )),
        None => Ok(()),
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
        let text = format!("# CDePT library '{name}'\nformat = 1\ndescription = \"\"\n");
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
        let view = unique_view_name(
            |view| entries.iter().any(|e| e.name == view),
            format.kind(),
            &name,
        );
        entries.push(Entry {
            name: view,
            kind: format.kind(),
            path: path.clone(),
            main_file: Some(path.clone()),
            format: Some(format),
            files: vec![path],
            member: None,
        });
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

fn scan_view(cell: &str, name: String, dir: PathBuf) -> Result<Entry> {
    let files: Vec<PathBuf> = list(&dir)?
        .into_iter()
        .filter(|(name, path)| path.is_file() && !meta::is_sidecar_name(name))
        .map(|(_, path)| path)
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
        member: None,
    })
}

/// A design file found in a flat or reference library.
struct Found {
    path: PathBuf,
    file_name: String,
    /// The file name without its suffix.
    stem: String,
    format: Format,
    kind: ViewKind,
    /// The view its cells get in a reference library (from the format folder). `None`
    /// in a flat library, where the name is worked out cell by cell.
    view: Option<String>,
    /// Reads what is inside, when the file may hold several cells.
    reader: Option<Reader>,
}

fn scan_flat(library: &Path, cache: &Cache) -> Result<Vec<Cell>> {
    let mut found = Vec::new();
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
        // A layout with a sidecar is CDePT's own: one cell, named after the file.
        let own = meta::sidecar_path(&path).is_file();
        found.push(Found {
            stem: stem.to_string(),
            reader: Reader::of_format(format).filter(|r| r.holds_cells() && !own),
            view: None,
            kind: format.kind(),
            format,
            file_name: name,
            path,
        });
    }
    Ok(assemble(found, cache, true, &Default::default()))
}

/// The view name and kind of a file in the format folder `folder` of a reference
/// library, `tool` being the folder inside it (`sym/xschem`). The usual folder of a
/// kind gives the kind's name, as in the other layouts (`gds` → `layout`, `lef` →
/// `abstract`, `lib` → `liberty`, `verilog` → `hdl`, `spice` → `netlist`, `sym` →
/// `symbol`, `sch` → `schematic`); any other adds the folder (`mag` → `layout_mag`,
/// `cdl` → `netlist_cdl`), and another tool than xschem its name (`schematic_qucs-s`).
fn ref_view(folder: &str, tool: Option<&str>, format: Format) -> (String, ViewKind) {
    let kind = match folder {
        "maglef" => ViewKind::Abstract,
        _ => format.kind(),
    };
    let usual: &[&str] = match kind {
        ViewKind::Layout => &["gds"],
        ViewKind::Abstract => &["lef"],
        ViewKind::Liberty => &["lib", "liberty"],
        ViewKind::Hdl => &["verilog"],
        ViewKind::Netlist => &["spice"],
        ViewKind::Symbol => &["sym"],
        ViewKind::Schematic => &["sch"],
        _ => &[],
    };
    let mut name = if folder.is_empty() || usual.contains(&folder) {
        kind.name().to_string()
    } else if folder == "maglef" {
        "abstract_mag".to_string()
    } else {
        format!("{}_{folder}", kind.name())
    };
    if let Some(tool) = tool.filter(|tool| *tool != "xschem") {
        name.push('_');
        name.push_str(tool);
    }
    (name, kind)
}

fn scan_ref(
    library: &Path,
    cache: &Cache,
    preferred: &std::collections::BTreeSet<PathBuf>,
) -> Result<Vec<Cell>> {
    // The files `tech.toml` names for this library decide which file is a view's main
    // file, before guessing from the names.
    let mut found = Vec::new();
    let mut add = |folder: &str, tool: Option<&str>, name: String, path: PathBuf| {
        let Some((stem, _)) = Format::split_name(&name) else {
            return;
        };
        let Some(format) = Format::of_file(&path) else {
            return;
        };
        // In another tool's folder only that tool's files count: `sym/qucs-s/*.sym`
        // are not xschem symbols.
        if tool.is_some_and(|tool| tool != "xschem")
            && matches!(format, Format::XschemSchematic | Format::XschemSymbol)
        {
            return;
        }
        let (view, kind) = ref_view(folder, tool, format);
        found.push(Found {
            stem: stem.to_string(),
            reader: Reader::of_format(format).filter(|r| r.holds_cells()),
            view: Some(view),
            kind,
            format,
            file_name: name,
            path,
        });
    };
    for (folder, dir) in list(library)? {
        if dir.is_file() {
            add("", None, folder, dir);
            continue;
        }
        if OTHER_FOLDERS.contains(&folder.as_str()) {
            continue;
        }
        for (name, path) in list(&dir)? {
            if path.is_file() {
                add(&folder, None, name, path);
                continue;
            }
            for (inner, file) in list(&path)? {
                if file.is_file() {
                    add(&folder, Some(&name), inner, file);
                }
            }
        }
    }
    Ok(assemble(found, cache, false, preferred))
}

/// A cell a file gives: its name and, when the file is a collection, the same name
/// as the member to show.
type FileCell = (String, Option<String>);

/// The cells a file gives (rules 1 to 4 of the module documentation). `known`: the
/// cells the library knows from its other files, which a layout collection needs;
/// without it a layout collection gives `None` (asked again later).
fn file_cells(
    found: &Found,
    index: Option<&Index>,
    flat: bool,
    known: Option<&std::collections::BTreeSet<String>>,
) -> Option<Vec<FileCell>> {
    let own = || vec![(found.stem.clone(), None)];
    let member = |name: &str| (name.to_string(), Some(name.to_string()));
    let Some(index) = index else {
        return Some(own());
    };
    let names = index.names();
    if names.is_empty() {
        return Some(if flat { own() } else { Vec::new() });
    }
    if names.contains(&found.stem.as_str()) {
        return Some(own());
    }
    if let Index::Layout(_) = index {
        let tops = index.public();
        if let [top] = tops.as_slice() {
            return Some(if flat { own() } else { vec![member(top)] });
        }
        let known = known?;
        let tops: std::collections::BTreeSet<&str> = tops.into_iter().collect();
        return Some(
            names
                .into_iter()
                .filter(|name| tops.contains(name) || known.contains(*name))
                .map(member)
                .collect(),
        );
    }
    Some(names.into_iter().map(member).collect())
}

/// How good a file is as the main file of a view of several: the cell's own file
/// before a collection, the typical corner before the others, the full Verilog model
/// before black boxes. Smaller is better.
fn main_file_rank(
    path: &Path,
    member: bool,
    preferred: &std::collections::BTreeSet<PathBuf>,
) -> (bool, bool, u8) {
    let name = file_name(path).to_ascii_lowercase();
    let stem = name.split('.').next().unwrap_or_default();
    let has = |word: &str| stem.split('_').any(|part| part == word);
    let mut penalty = 4u8;
    if has("tt") || has("typ") || has("nom") {
        penalty -= 2;
    }
    if has("025c") || has("25c") {
        penalty -= 1;
    }
    if name.contains("blackbox") || name.contains("primitives") {
        penalty += 2;
    }
    if has("pp") {
        penalty += 1;
    }
    (!preferred.contains(path), member, penalty)
}

/// The cells of a flat or reference library from its design files.
fn assemble(
    found: Vec<Found>,
    cache: &Cache,
    flat: bool,
    preferred: &std::collections::BTreeSet<PathBuf>,
) -> Vec<Cell> {
    // What is inside the files that may hold several cells, read side by side.
    let jobs: Vec<(&Path, Reader)> = found
        .iter()
        .filter_map(|f| Some((f.path.as_path(), f.reader?)))
        .collect();
    let mut read = container::read_many(&jobs, cache).into_iter();
    let indexes: Vec<Option<Index>> = found
        .iter()
        .map(|f| {
            f.reader?;
            match read.next()? {
                Ok(index) => Some(index),
                Err(e) => {
                    tracing::debug!("{e:#}");
                    None
                }
            }
        })
        .collect();

    // The cells of each file; layout collections last, when the other names are known.
    let mut known = std::collections::BTreeSet::new();
    let mut cells_of: Vec<Option<Vec<FileCell>>> = found
        .iter()
        .zip(&indexes)
        .map(|(f, index)| {
            let cells = file_cells(f, index.as_ref(), flat, None)?;
            known.extend(cells.iter().map(|(name, _)| name.clone()));
            Some(cells)
        })
        .collect();
    for ((f, index), cells) in found.iter().zip(&indexes).zip(&mut cells_of) {
        if cells.is_none() {
            *cells = file_cells(f, index.as_ref(), flat, Some(&known));
        }
    }

    /// A view being put together: its files, each with the member it shows.
    struct Draft<'a> {
        name: String,
        kind: ViewKind,
        files: Vec<(&'a Found, Option<String>)>,
    }
    let mut cells: BTreeMap<String, Vec<Draft>> = BTreeMap::new();
    for (f, file_cells) in found.iter().zip(cells_of) {
        for (cell, member) in file_cells.unwrap_or_default() {
            let views = cells.entry(cell).or_default();
            // Files that hold the same cell in the same format are one view.
            let same = views.iter_mut().find(|view| match &f.view {
                Some(name) => view.name == *name,
                None => {
                    member.is_some()
                        && view
                            .files
                            .iter()
                            .all(|(other, m)| m.is_some() && other.format == f.format)
                }
            });
            match same {
                Some(view) => view.files.push((f, member)),
                None => {
                    let name = f.view.clone().unwrap_or_else(|| {
                        unique_view_name(
                            |name| views.iter().any(|v| v.name == name),
                            f.kind,
                            &f.file_name,
                        )
                    });
                    views.push(Draft {
                        name,
                        kind: f.kind,
                        files: vec![(f, member)],
                    });
                }
            }
        }
    }
    cells
        .into_iter()
        .map(|(name, views)| {
            let mut entries: Vec<Entry> = views
                .into_iter()
                .map(|view| {
                    let (main, member) = view
                        .files
                        .iter()
                        .min_by_key(|(f, member)| main_file_rank(&f.path, member.is_some(), preferred))
                        .expect("a view has a file");
                    Entry {
                        name: view.name,
                        kind: view.kind,
                        path: main.path.clone(),
                        main_file: Some(main.path.clone()),
                        format: Some(main.format),
                        files: view.files.iter().map(|(f, _)| f.path.clone()).collect(),
                        member: member.clone(),
                    }
                })
                .collect();
            entries.sort_by(|a, b| a.name.cmp(&b.name));
            Cell {
                name,
                path: None,
                entries,
            }
        })
        .collect()
}

/// `layout`, or `layout_oas` when the cell already has a `layout` view.
fn unique_view_name(taken: impl Fn(&str) -> bool, kind: ViewKind, file_name: &str) -> String {
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
            for (from, name) in with_sidecar(file, name) {
                let target = dir.join(name);
                if target.exists() {
                    bail!("{} already exists", target.display());
                }
                moves.push((from, target));
            }
        }
    }
    if moves.is_empty() {
        bail!("cell '{old}' has no file named after it to put in a flat library");
    }
    Ok(moves)
}

/// A layout file and its sidecar (if it has one), each with the file name it gets.
fn with_sidecar(file: &Path, name: String) -> Vec<(PathBuf, String)> {
    let mut items = vec![(file.to_path_buf(), name.clone())];
    let sidecar = meta::sidecar_path(file);
    if sidecar.is_file() {
        items.push((sidecar, format!("{name}.json")));
    }
    items
}

fn copy_file(from: &Path, to: &Path) -> Result<()> {
    fs::copy(from, to)
        .map(|_| ())
        .wrap_err_with(|| format!("cannot copy {} to {}", from.display(), to.display()))
}

fn remove_entry(entry: &Entry) -> Result<()> {
    if entry.is_member() {
        bail!(
            "view '{}' cannot be deleted: it is one cell of {}, which holds other cells too",
            entry.name,
            entry.path.display()
        );
    }
    let result = if entry.path.is_dir() {
        fs::remove_dir_all(&entry.path)
    } else {
        fs::remove_file(&entry.path)
    };
    result.wrap_err_with(|| format!("cannot delete {}", entry.path.display()))?;
    // A loose layout file takes its sidecar with it.
    let sidecar = meta::sidecar_path(&entry.path);
    if sidecar.is_file() {
        fs::remove_file(&sidecar)
            .wrap_err_with(|| format!("cannot delete {}", sidecar.display()))?;
    }
    Ok(())
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
            preferred: Default::default(),
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

    fn top_of(file: &Path) -> Vec<String> {
        layout::gds::read_structure(file).unwrap().top_cells()
    }

    #[test]
    fn layout_views_have_sidecars_that_follow_rename_and_copy() {
        let tmp = tempfile::tempdir().unwrap();
        let mut lib = library(tmp.path(), LibraryLayout::Cells);
        lib.name = "analog".into();
        let gds = lib.create_view("inv", Template::GdsLayout).unwrap();
        lib.scan();
        let side = meta::sidecar_path(&gds);
        let m = meta::Meta::read(&side).unwrap();
        assert_eq!(m, meta::Meta::new("analog", "inv", "layout", "inv"));
        // The sidecar is not a view or a file of the view.
        let layout = lib.cell("inv").unwrap().entry("layout").unwrap();
        assert_eq!(layout.files, std::slice::from_ref(&gds));
        assert_eq!(lib.cell("inv").unwrap().entries.len(), 1);

        lib.rename_cell("inv", "inv1").unwrap();
        lib.scan();
        let gds = tmp.path().join("inv1/layout/inv1.gds");
        assert_eq!(top_of(&gds), ["inv1"]);
        assert!(!tmp.path().join("inv1/layout/inv.gds.json").exists());
        let m = meta::Meta::read_for(&gds).unwrap().unwrap();
        assert_eq!((m.cell.as_str(), m.top.as_str()), ("inv1", "inv1"));

        let mut other = library(&tmp.path().join("other"), LibraryLayout::Cells);
        fs::create_dir(tmp.path().join("other")).unwrap();
        other.name = "digital".into();
        other.scan();
        lib.copy_cell("inv1", &other, "inv2").unwrap();
        let copy = tmp.path().join("other/inv2/layout/inv2.gds");
        assert_eq!(top_of(&copy), ["inv2"]);
        let m = meta::Meta::read_for(&copy).unwrap().unwrap();
        assert_eq!(m.label(), "digital/inv2/layout");
        assert_eq!(m.top, "inv2");
        // The original is untouched.
        assert_eq!(top_of(&gds), ["inv1"]);
        assert_eq!(
            meta::Meta::read_for(&gds).unwrap().unwrap().label(),
            "analog/inv1/layout"
        );
    }

    #[test]
    fn flat_layout_sidecars_follow_their_file() {
        let tmp = tempfile::tempdir().unwrap();
        let mut lib = library(tmp.path(), LibraryLayout::Flat);
        lib.name = "gds".into();
        lib.create_view("inv", Template::GdsLayout).unwrap();
        lib.scan();
        assert_eq!(lib.cells.len(), 1);
        assert!(tmp.path().join("inv.gds.json").is_file());
        lib.copy_cell("inv", &lib, "buf").unwrap();
        lib.rename_cell("inv", "inv9").unwrap();
        lib.scan();
        for (cell, other) in [("buf", "inv9"), ("inv9", "buf")] {
            let file = tmp.path().join(format!("{cell}.gds"));
            assert_eq!(top_of(&file), [cell]);
            let m = meta::Meta::read_for(&file).unwrap().unwrap();
            assert_eq!((m.cell.as_str(), m.top.as_str()), (cell, cell));
            assert!(tmp.path().join(format!("{other}.gds")).is_file());
        }
        assert!(!tmp.path().join("inv.gds.json").exists());
        // Into a cells library.
        let dir = tempfile::tempdir().unwrap();
        let mut cells = library(dir.path(), LibraryLayout::Cells);
        cells.name = "c".into();
        lib.copy_cell("buf", &cells, "buf2").unwrap();
        let copy = dir.path().join("buf2/layout/buf2.gds");
        assert_eq!(top_of(&copy), ["buf2"]);
        assert_eq!(
            meta::Meta::read_for(&copy).unwrap().unwrap().label(),
            "c/buf2/layout"
        );
        // Deleting the cell deletes its sidecar.
        lib.delete_cell("buf").unwrap();
        assert!(!tmp.path().join("buf.gds.json").exists());
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

    /// A GDSII stream with these structures, each placing the named ones.
    fn gds(structures: &[(&str, &[&str])]) -> Vec<u8> {
        fn record(out: &mut Vec<u8>, kind: u16, data: &[u8]) {
            let mut data = data.to_vec();
            if data.len() % 2 == 1 {
                data.push(0);
            }
            out.extend(((data.len() + 4) as u16).to_be_bytes());
            out.extend(kind.to_be_bytes());
            out.extend(data);
        }
        let mut out = Vec::new();
        record(&mut out, 0x0002, &[0, 5]);
        record(&mut out, 0x0102, &[0; 24]);
        record(&mut out, 0x0206, b"LIB");
        record(&mut out, 0x0305, &[0; 16]);
        for (name, refs) in structures {
            record(&mut out, 0x0502, &[0; 24]);
            record(&mut out, 0x0606, name.as_bytes());
            for placed in *refs {
                record(&mut out, 0x0a00, &[]);
                record(&mut out, 0x1206, placed.as_bytes());
                record(&mut out, 0x1003, &[0; 8]);
                record(&mut out, 0x1100, &[]);
            }
            record(&mut out, 0x0700, &[]);
        }
        record(&mut out, 0x0400, &[]);
        out
    }

    fn liberty(cells: &[&str]) -> String {
        let mut text = String::from("library (demo) {\n");
        for cell in cells {
            text.push_str(&format!("  cell ({cell}) {{ area : 1; }}\n"));
        }
        text + "}\n"
    }

    fn members(lib: &Library) -> Vec<(String, String, Option<String>)> {
        lib.cells
            .iter()
            .flat_map(|c| {
                c.entries
                    .iter()
                    .map(|e| (c.name.clone(), e.name.clone(), e.member.clone()))
            })
            .collect()
    }

    fn triples(items: &[(&str, &str, Option<&str>)]) -> Vec<(String, String, Option<String>)> {
        items
            .iter()
            .map(|(c, v, m)| (c.to_string(), v.to_string(), m.map(str::to_string)))
            .collect()
    }

    #[test]
    fn flat_library_lists_the_cells_inside_collection_files() {
        let tmp = tempfile::tempdir().unwrap();
        let write = |name: &str, bytes: &[u8]| fs::write(tmp.path().join(name), bytes).unwrap();
        // A collection: two top cells, one of them with a sub-cell.
        write(
            "stdcells.gds",
            &gds(&[("via", &[]), ("inv_1", &["via"]), ("nand2_1", &[])]),
        );
        write("stdcells_tt.lib", liberty(&["inv_1", "nand2_1"]).as_bytes());
        write("stdcells_ff.lib", liberty(&["inv_1", "nand2_1"]).as_bytes());
        write(
            "stdcells.v",
            b"module inv_1 (input A, output Y); endmodule\nmodule nand2_1 (A, B, Y); endmodule\n",
        );
        // A design file with its own hierarchy: one cell, named after the file.
        write("adc.gds", &gds(&[("sar", &[]), ("adc", &["sar"])]));
        write(
            "adc.spice",
            b".subckt sar a b\n.ends\n.subckt adc in out\nX1 in out sar\n.ends\n",
        );
        // One top cell with another name: still the file's cell.
        write("pad.gds", &gds(&[("TOP", &[])]));
        // Nothing inside.
        write("tb.spice", b"* a testbench\nV1 a 0 1\n.end\n");
        write("inv_1.sym", b"v {xschem}");

        let mut lib = library(tmp.path(), LibraryLayout::Flat);
        lib.scan_with(&Cache::off());
        assert_eq!(
            members(&lib),
            triples(&[
                ("adc", "layout", None),
                ("adc", "netlist", None),
                ("inv_1", "hdl", Some("inv_1")),
                ("inv_1", "layout", Some("inv_1")),
                ("inv_1", "liberty", Some("inv_1")),
                ("inv_1", "symbol", None),
                ("nand2_1", "hdl", Some("nand2_1")),
                ("nand2_1", "layout", Some("nand2_1")),
                ("nand2_1", "liberty", Some("nand2_1")),
                ("pad", "layout", None),
                ("tb", "netlist", None),
            ])
        );
        // Both corners are one view; the typical one is its main file.
        let liberty = lib.cell("inv_1").unwrap().entry("liberty").unwrap();
        assert_eq!(liberty.files.len(), 2);
        assert_eq!(liberty.main_file, Some(tmp.path().join("stdcells_tt.lib")));
        assert_eq!(liberty.member_or("inv_1"), "inv_1");

        // A cell of a collection cannot be deleted, renamed or copied: the file
        // belongs to the others too.
        assert!(lib.delete_cell("inv_1").is_err());
        assert!(lib.delete_view("nand2_1", "layout").is_err());
        assert!(lib.rename_cell("nand2_1", "x").is_err());
        assert!(lib.copy_cell("nand2_1", &lib, "x").is_err());
        assert!(tmp.path().join("stdcells.gds").is_file());
        assert!(tmp.path().join("inv_1.sym").is_file());
        // The others still can.
        lib.rename_cell("tb", "tb2").unwrap();
        assert!(tmp.path().join("tb2.spice").is_file());
    }

    #[test]
    fn reference_library_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sc");
        let write = |name: &str, bytes: &[u8]| {
            let file = root.join(name);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, bytes).unwrap();
        };
        write(
            "gds/sc.gds",
            &gds(&[
                ("via", &[]),
                ("sub", &[]),
                ("inv_1", &["via", "sub"]),
                ("buf_1", &["via"]),
            ]),
        );
        write(
            "lef/sc.lef",
            b"MACRO inv_1\n CLASS CORE ;\n SIZE 1 BY 2 ;\nEND inv_1\nMACRO buf_1\nEND buf_1\n",
        );
        write("lef/sc_tech.lef", b"LAYER m1\n TYPE ROUTING ;\nEND m1\n");
        write(
            "techlef/sc__nom.tlef",
            b"LAYER m1\n TYPE ROUTING ;\nEND m1\n",
        );
        write(
            "lib/sc_ff_n40C_1v95.lib",
            liberty(&["inv_1", "buf_1"]).as_bytes(),
        );
        write(
            "lib/sc_tt_025C_1v80.lib",
            liberty(&["inv_1", "buf_1"]).as_bytes(),
        );
        write(
            "verilog/sc__blackbox.v",
            b"module inv_1 (A, Y); endmodule\n",
        );
        write(
            "verilog/sc.v",
            b"module inv_1 (A, Y); endmodule\nmodule buf_1 (A, X); endmodule\n",
        );
        write(
            "verilog/primitives.v",
            b"primitive udp (a, b); endprimitive\n",
        );
        write(
            "spice/sc.spice",
            b".subckt inv_1 A Y\n.ends\n.subckt buf_1 A X\n.ends\n",
        );
        write("spice/fill_1.spice", b".subckt fill_1 VGND VPWR\n.ends\n");
        write("cdl/sc.cdl", b".SUBCKT inv_1 A Y\n.ENDS\n");
        write("mag/inv_1.mag", b"magic\ntech demo\n");
        // A sub-cell the library knows from its `.mag` file is listed, `via` is not.
        write("mag/sub.mag", b"magic\ntech demo\n");
        write("maglef/inv_1.mag", b"magic\ntech demo\n");
        write("sym/xschem/inv_1.sym", b"v {xschem}");
        write("sch/xschem/inv_1.sch", b"v {xschem}");
        write("sch/qucs-s/inv_1.sch", b"<Qucs Schematic 25.2.0>");
        write("sym/qucs-s/INV.sym", b"<!-- qucs symbol -->");
        write("doc/sc.celllist", b"inv_1\n");
        write("doc/notes.v", b"module not_a_cell; endmodule\n");

        assert_eq!(LibraryLayout::detect(&root), LibraryLayout::Ref);
        assert_eq!(LibraryLayout::from_name("ref"), Some(LibraryLayout::Ref));
        let mut lib = library(&root, LibraryLayout::Ref);
        lib.scan_with(&Cache::off());
        assert_eq!(lib.error, None);
        assert_eq!(
            members(&lib),
            triples(&[
                ("buf_1", "abstract", Some("buf_1")),
                ("buf_1", "hdl", Some("buf_1")),
                ("buf_1", "layout", Some("buf_1")),
                ("buf_1", "liberty", Some("buf_1")),
                ("buf_1", "netlist", Some("buf_1")),
                ("fill_1", "netlist", None),
                ("inv_1", "abstract", Some("inv_1")),
                ("inv_1", "abstract_mag", None),
                ("inv_1", "hdl", Some("inv_1")),
                ("inv_1", "layout", Some("inv_1")),
                ("inv_1", "layout_mag", None),
                ("inv_1", "liberty", Some("inv_1")),
                ("inv_1", "netlist", Some("inv_1")),
                ("inv_1", "netlist_cdl", Some("inv_1")),
                ("inv_1", "schematic", None),
                ("inv_1", "schematic_qucs-s", None),
                ("inv_1", "symbol", None),
                ("sub", "layout", Some("sub")),
                ("sub", "layout_mag", None),
            ])
        );
        let inv = lib.cell("inv_1").unwrap();
        assert!(inv.path.is_none());
        let liberty = inv.entry("liberty").unwrap();
        assert_eq!(liberty.kind, ViewKind::Liberty);
        assert_eq!(liberty.files.len(), 2);
        assert_eq!(
            liberty.main_file,
            Some(root.join("lib/sc_tt_025C_1v80.lib"))
        );
        // The full model, not the black box.
        let hdl = inv.entry("hdl").unwrap();
        assert_eq!(hdl.main_file, Some(root.join("verilog/sc.v")));
        assert_eq!(hdl.files.len(), 2);
        assert_eq!(inv.entry("abstract_mag").unwrap().kind, ViewKind::Abstract);
        assert_eq!(
            inv.entry("abstract_mag").unwrap().format,
            Some(Format::Magic)
        );
        assert_eq!(
            inv.entry("schematic_qucs-s").unwrap().format,
            Some(Format::QucsSchematic)
        );

        // CDePT only reads a reference library, read-only or not.
        assert!(!lib.read_only);
        assert!(lib.create_cell("x").is_err());
        assert!(lib.create_view("x", Template::Verilog).is_err());
        assert!(lib.rename_cell("fill_1", "x").is_err());
        assert!(lib.delete_cell("fill_1").is_err());
        assert!(lib.delete_view("inv_1", "symbol").is_err());
        assert!(root.join("spice/fill_1.spice").is_file());
        // Copying a cell out of it works for a cell with files of its own only.
        let out = tempfile::tempdir().unwrap();
        let cells = library(out.path(), LibraryLayout::Cells);
        assert!(lib.copy_cell("inv_1", &cells, "inv_1").is_err());
        lib.copy_cell("fill_1", &cells, "fill_2").unwrap();
        assert!(out.path().join("fill_2/netlist/fill_2.spice").is_file());
    }

    #[test]
    fn a_reference_library_with_one_layout_per_cell() {
        // Macros: one GDS per cell, its sub-cells inside; a Liberty file per corner
        // named after the cell and the corner.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sram");
        let write = |name: &str, bytes: &[u8]| {
            let file = root.join(name);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, bytes).unwrap();
        };
        write(
            "gds/ram8.gds",
            &gds(&[("bitcell", &[]), ("ram8", &["bitcell"])]),
        );
        write(
            "gds/ram16.gds",
            &gds(&[("bitcell", &[]), ("ram16_top", &["bitcell"])]),
        );
        write("lib/ram8_tt_1p8V_25C.lib", liberty(&["ram8"]).as_bytes());
        write("lib/ram8_ff_1p8V_25C.lib", liberty(&["ram8"]).as_bytes());
        write(
            "cdl/ram8.cdl",
            b".subckt bitcell bl wl\n.ends\n.subckt ram8 a d\n.ends\n",
        );
        let mut lib = library(&root, LibraryLayout::Ref);
        lib.scan_with(&Cache::off());
        assert_eq!(
            members(&lib),
            triples(&[
                ("ram16_top", "layout", Some("ram16_top")),
                ("ram8", "layout", None),
                ("ram8", "liberty", Some("ram8")),
                ("ram8", "netlist_cdl", None),
            ])
        );
    }

    #[test]
    fn the_preferred_files_name_the_main_file_of_a_view() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("demo/libs.ref/std");
        let write = |name: &str, text: &str| {
            let file = tmp.path().join("demo").join(name);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, text).unwrap();
        };
        write("libs.ref/std/lib/std_tt_1p8V_25C.lib", &liberty(&["inv"]));
        write("libs.ref/std/lib/std_ff_1p8V_25C.lib", &liberty(&["inv"]));
        let main = |lib: &Library| {
            lib.cell("inv")
                .and_then(|c| c.entries.first())
                .and_then(|e| e.main_file.clone())
                .and_then(|f| f.file_name().map(|n| n.to_string_lossy().into_owned()))
                .unwrap_or_default()
        };
        // Nothing preferred: the typical corner by its name.
        let mut lib = library(&root, LibraryLayout::Ref);
        lib.scan_with(&Cache::off());
        assert_eq!(main(&lib), "std_tt_1p8V_25C.lib");
        // The snapshot names another one; a configuration in the PDK is not read.
        write(
            "libs.tech/librelane/config.tcl",
            "set ::env(CELL_LIBS) [dict create]\n\
             dict set ::env(CELL_LIBS) \"*_tt\" \"$::env(PDK_ROOT)/$::env(PDK)/libs.ref/std/lib/std_tt_1p8V_25C.lib\"\n",
        );
        lib.preferred
            .insert(root.join("lib/std_ff_1p8V_25C.lib"));
        lib.scan_with(&Cache::off());
        assert_eq!(main(&lib), "std_ff_1p8V_25C.lib");
    }

    #[test]
    fn layouts_are_detected_from_the_folder() {
        let tmp = tempfile::tempdir().unwrap();
        // A cells library whose cell is called like a format folder stays `cells`.
        let cells = tmp.path().join("cells");
        fs::create_dir_all(cells.join("lib/schematic")).unwrap();
        fs::write(cells.join("lib/schematic/lib.sch"), "").unwrap();
        fs::create_dir_all(cells.join("ota/schematic")).unwrap();
        assert_eq!(LibraryLayout::detect(&cells), LibraryLayout::Cells);
        // Symbols only, in a tool folder.
        let syms = tmp.path().join("syms");
        fs::create_dir_all(syms.join("sym/xschem")).unwrap();
        fs::write(syms.join("sym/xschem/a.sym"), "").unwrap();
        fs::create_dir_all(syms.join("doc")).unwrap();
        assert_eq!(LibraryLayout::detect(&syms), LibraryLayout::Ref);
        // An empty folder is a new `cells` library.
        let empty = tmp.path().join("empty");
        fs::create_dir_all(&empty).unwrap();
        assert_eq!(LibraryLayout::detect(&empty), LibraryLayout::Cells);
        assert!(LibraryLayout::Cells.editable() && !LibraryLayout::Ref.editable());
    }

    #[test]
    fn missing_folder_is_reported_not_fatal() {
        let lib = library(Path::new("/nonexistent/cdept-lib"), LibraryLayout::Cells);
        assert!(lib.error.is_some());
        assert!(lib.cells.is_empty());
    }
}
