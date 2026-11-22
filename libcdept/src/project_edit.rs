//! Editing library definitions, and the checks and file handling around the project
//! file.
//!
//! - [`Project::set_library`] changes a library definition (folder, read-only,
//!   layout, description) in the file that defines it, the project file or one it
//!   includes, keeping that file's comments and formatting.
//!   [`Project::library_definition`] shows a definition as written.
//! - [`FORMAT`]: the `format = 1` key of `cdept.toml` and `lib.toml`. A file without
//!   it is format 1; a newer one loads with a warning.
//! - Checks after loading (`after_load`): names that differ only by case, file
//!   names that are not UTF-8.
//! - [`write_file`]: a project file is replaced through a temporary file, and stays
//!   the symbolic link it was, with the permissions it had.
//! - [`portable`]: a folder inside the project is written relative, also when it was
//!   reached through a symbolic link.
//! - [`CopyRule`]: what a cell copy leaves out (simulation results), from the
//!   library's `lib.toml`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use eyre::{Context, Result, bail, eyre};
use toml_edit::{DocumentMut, InlineTable, Item, TableLike, Value};

use crate::library::LIBRARY_FILE;
use crate::{LibraryLayout, Project};

/// The key in `cdept.toml` and `lib.toml` that says which format of the file it is.
pub const FORMAT_KEY: &str = "format";
/// The format this CDePT writes. A file without the key is format 1.
pub const FORMAT: i64 = 1;

/// The `[copy]` table of `lib.toml`.
pub const COPY_KEY: &str = "copy";
/// What a cell copy leaves out when the library does not say: result folders of
/// testbench views.
pub const COPY_EXCLUDE_DEFAULT: &[&str] = &["results"];

/// A library definition as written in the file that holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryDefinition {
    pub name: String,
    /// The file that defines the library: the project file, or one it includes.
    pub file: PathBuf,
    /// The definition is in an included file, not in the project file.
    pub included: bool,
    /// The folder as written (`$VAR` and `~` not expanded, relative to `file`).
    pub path: String,
    /// The layout as written; `None` when it is detected from the folder.
    pub layout: Option<LibraryLayout>,
    pub read_only: bool,
    /// From the `lib.toml` of the library folder.
    pub description: String,
}

/// Where a library of the project is defined.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// In the project file.
    Project,
    /// In a file the project file includes.
    Included(PathBuf),
    /// The PDK brings it: listed by the technology snapshot (`tech.toml`).
    Snapshot(PathBuf),
}

impl Origin {
    /// The file to look into, and what it is, for display.
    pub fn describe(&self, project: &Project) -> String {
        match self {
            Origin::Project => format!("{} (the project file)", project.file.display()),
            Origin::Included(file) => format!("{} (included)", file.display()),
            Origin::Snapshot(file) => {
                format!("{} (the PDK's libraries)", file.display())
            }
        }
    }
}

/// What [`Project::set_library`] changes. `None` leaves a value as it is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LibraryChange {
    /// The folder, written as given (relative to the defining file, `$VAR` kept).
    pub path: Option<String>,
    pub read_only: Option<bool>,
    /// `Some(None)` removes the key: the layout is detected from the folder again.
    pub layout: Option<Option<LibraryLayout>>,
    /// Written to the `lib.toml` of the library folder.
    pub description: Option<String>,
}

impl LibraryChange {
    pub fn is_empty(&self) -> bool {
        *self == LibraryChange::default()
    }
}

/// The values of a definition: `(path, layout, read_only)`.
fn written(name: &str, item: &Item) -> Result<(String, Option<LibraryLayout>, bool)> {
    if let Some(path) = item.as_str() {
        return Ok((path.to_string(), None, false));
    }
    let table = item
        .as_table_like()
        .ok_or_else(|| eyre!("library '{name}' must be a folder or a table with `path`"))?;
    let path = table
        .get("path")
        .and_then(Item::as_str)
        .ok_or_else(|| eyre!("library '{name}' needs a `path`"))?;
    let layout = match table.get("layout") {
        None => None,
        Some(item) => Some(
            item.as_str()
                .and_then(LibraryLayout::from_name)
                .ok_or_else(|| eyre!("library '{name}': `layout` is not a known layout"))?,
        ),
    };
    let read_only = table
        .get("read_only")
        .and_then(Item::as_bool)
        .unwrap_or(false);
    Ok((path.to_string(), layout, read_only))
}

/// The entry of library `name` in a definition file.
fn entry<'a>(doc: &'a mut DocumentMut, name: &str, file: &Path) -> Result<&'a mut Item> {
    doc.get_mut("libraries")
        .and_then(Item::as_table_like_mut)
        .and_then(|libraries| libraries.get_mut(name))
        .ok_or_else(|| eyre!("library '{name}' is not in {}", file.display()))
}

/// Set `key` of a definition table, keeping the comments around an old value.
fn set_key(table: &mut dyn TableLike, key: &str, mut new: Value) {
    match table.get_mut(key) {
        Some(item) if item.is_value() => {
            if let Some(old) = item.as_value() {
                *new.decor_mut() = old.decor().clone();
            }
            *item = Item::Value(new);
        }
        _ => {
            table.insert(key, Item::Value(new));
        }
    }
}

/// Write a definition into its entry: the short form (`name = "folder"`) stays short
/// while nothing but the folder is said.
fn write_entry(item: &mut Item, path: &str, layout: Option<LibraryLayout>, read_only: bool) {
    if item.as_table_like().is_some() {
        let table = item.as_table_like_mut().expect("checked above");
        set_key(table, "path", path.into());
        match layout {
            Some(layout) => set_key(table, "layout", layout.name().into()),
            None => {
                table.remove("layout");
            }
        }
        if read_only {
            set_key(table, "read_only", true.into());
        } else {
            table.remove("read_only");
        }
        if let Some(inline) = item.as_inline_table_mut() {
            inline.fmt();
        }
        return;
    }
    let decor = item.as_value().map(|v| v.decor().clone());
    let mut new = if layout.is_none() && !read_only {
        Value::from(path)
    } else {
        let mut table = InlineTable::new();
        table.insert("path", path.into());
        if let Some(layout) = layout {
            table.insert("layout", layout.name().into());
        }
        if read_only {
            table.insert("read_only", true.into());
        }
        Value::InlineTable(table)
    };
    if let Some(decor) = decor {
        *new.decor_mut() = decor;
    }
    *item = Item::Value(new);
}

impl Project {
    /// Where library `name` is defined.
    pub fn library_origin(&self, name: &str) -> Option<Origin> {
        let file = &self.library(name)?.defined_in;
        Some(if *file == self.file {
            Origin::Project
        } else if file
            .file_name()
            .is_some_and(|n| n == crate::tech::TECH_FILE)
            && file.parent() == Some(self.root.as_path())
        {
            Origin::Snapshot(file.clone())
        } else {
            Origin::Included(file.clone())
        })
    }

    /// A library the PDK brings is not a definition to edit.
    pub(crate) fn editable(&self, name: &str) -> Result<()> {
        if let Some(Origin::Snapshot(file)) = self.library_origin(name) {
            bail!(
                "library '{name}' comes with the PDK (it is listed in {}): define a library \
                 named '{name}' in the project file to use another folder, or set \
                 `enabled = false` for it there to leave it out",
                file.display()
            );
        }
        Ok(())
    }

    /// The definition of a library as written in the file that holds it.
    pub fn library_definition(&self, name: &str) -> Result<LibraryDefinition> {
        let library = self
            .library(name)
            .ok_or_else(|| eyre!("no library '{name}'"))?;
        self.editable(name)?;
        let file = library.defined_in.clone();
        let mut doc = read_doc(&file)?;
        let (path, layout, read_only) = written(name, entry(&mut doc, name, &file)?)?;
        Ok(LibraryDefinition {
            name: name.to_string(),
            included: file != self.file,
            file,
            path,
            layout,
            read_only,
            description: library.description.clone().unwrap_or_default(),
        })
    }

    /// Change the definition of a library, in the file that defines it (the project
    /// file or one it includes), then load the project again.
    ///
    /// A new folder must exist. The description is kept in the `lib.toml` of the
    /// library folder, so a read-only library cannot get one. The definition is
    /// written first: when the description then cannot be written, the error says
    /// that the rest is done.
    pub fn set_library(&mut self, name: &str, change: &LibraryChange) -> Result<()> {
        let library = self
            .library(name)
            .ok_or_else(|| eyre!("no library '{name}'"))?
            .clone();
        self.editable(name)?;
        let file = library.defined_in.clone();
        let dir = file.parent().unwrap_or(Path::new(".")).to_path_buf();
        let mut doc = read_doc(&file)?;
        let item = entry(&mut doc, name, &file)?;
        let (old_path, old_layout, old_read_only) = written(name, item)?;

        let path = match &change.path {
            Some(path) if path.trim().is_empty() => {
                return Err(eyre!("the library needs a folder"));
            }
            Some(path) => path.trim().to_string(),
            None => old_path.clone(),
        };
        let folder = self.resolve(&path, &dir).map_err(|e| eyre!(e))?;
        if change.path.is_some() && path != old_path && !folder.is_dir() {
            bail!("folder not found: {}", folder.display());
        }
        let read_only = change.read_only.unwrap_or(old_read_only);
        let mut layout = change.layout.unwrap_or(old_layout);

        let description = change
            .description
            .as_ref()
            .map(|d| d.trim().to_string())
            .filter(|d| *d != library.description.clone().unwrap_or_default());
        if description.is_some() {
            if read_only {
                bail!(
                    "library '{name}' is read-only: its description is in {}, which CDePT \
                     does not change",
                    folder.join(LIBRARY_FILE).display()
                );
            }
            // A `lib.toml` makes a folder look like a `cells` library: say the layout
            // before one appears in a library that had it detected.
            if layout.is_none() && !folder.join(LIBRARY_FILE).is_file() && folder.is_dir() {
                layout = Some(LibraryLayout::detect(&folder));
            }
        }

        if (path.as_str(), layout, read_only) != (old_path.as_str(), old_layout, old_read_only) {
            write_entry(item, &path, layout, read_only);
            write_file(&file, &doc.to_string())?;
        }
        if let Some(description) = description {
            write_description(&folder, name, &description).wrap_err_with(|| {
                format!(
                    "the definition of '{name}' in {} was changed, but not its description",
                    file.display()
                )
            })?;
        }
        self.reload()
    }

    /// Why a library `name` in the folder `path` (as it would be written in the
    /// project file) would get in the way of another one: the same folder twice, or
    /// two folders with the same name, which xschem cannot tell apart. `None` when
    /// it is fine.
    pub fn folder_clash(&self, name: &str, path: &str) -> Option<String> {
        let folder = self.resolve(path.trim(), &self.root).ok()?;
        let real = folder.canonicalize().unwrap_or_else(|_| folder.clone());
        let last = folder.file_name()?;
        for other in self.libraries.iter().filter(|l| l.name != name) {
            let other_real = other
                .path
                .canonicalize()
                .unwrap_or_else(|_| other.path.clone());
            if other_real == real {
                return Some(format!(
                    "{} is already library '{}'",
                    folder.display(),
                    other.name
                ));
            }
            if other.path.file_name() == Some(last) {
                return Some(format!(
                    "library '{}' is in another folder that is also called '{}' ({}): xschem \
                     names a cell by its folder ('{}/<cell>'), so it cannot tell the two \
                     libraries apart and uses the first one on its search path",
                    other.name,
                    last.to_string_lossy(),
                    other.path.display(),
                    last.to_string_lossy(),
                ));
            }
        }
        None
    }
}

/// Set `description` in the `lib.toml` of a library folder, creating the file when
/// there is none.
fn write_description(folder: &Path, name: &str, description: &str) -> Result<()> {
    let file = folder.join(LIBRARY_FILE);
    let mut doc = if file.is_file() {
        read_doc(&file)?
    } else {
        format!("# CDePT library '{name}'\n{FORMAT_KEY} = {FORMAT}\n")
            .parse()
            .expect("valid TOML")
    };
    set_key(doc.as_table_mut(), "description", description.into());
    write_file(&file, &doc.to_string())
}

pub(crate) fn read_doc(file: &Path) -> Result<DocumentMut> {
    let text =
        fs::read_to_string(file).wrap_err_with(|| format!("cannot read {}", file.display()))?;
    text.parse::<DocumentMut>()
        .map_err(|e| eyre!("{}: {}", file.display(), e.to_string().trim_end()))
}

/// Replace a text file through a temporary file, so a failure cannot truncate it.
///
/// A symbolic link stays one (the file it points to is replaced), and the new file
/// has the permissions of the old one. Where the folder does not take a new file but
/// the file itself can be written, it is written in place.
pub fn write_file(file: &Path, text: &str) -> Result<()> {
    // The file a symbolic link leads to; a link to nowhere is followed one step.
    let target = match fs::symlink_metadata(file) {
        Ok(meta) if meta.file_type().is_symlink() => match file.canonicalize() {
            Ok(real) => real,
            Err(_) => {
                let link = fs::read_link(file)
                    .wrap_err_with(|| format!("cannot read the link {}", file.display()))?;
                file.parent().unwrap_or(Path::new(".")).join(link)
            }
        },
        _ => file.to_path_buf(),
    };
    let mut name = target
        .file_name()
        .ok_or_else(|| eyre!("{} is not a file name", target.display()))?
        .to_os_string();
    name.push(".tmp");
    let tmp = target.with_file_name(name);
    let permissions = fs::metadata(&target).ok().map(|m| m.permissions());

    if let Err(error) = fs::write(&tmp, text) {
        // No new file in this folder (read-only folder, writable file): in place.
        if permissions.is_some() && fs::write(&target, text).is_ok() {
            return Ok(());
        }
        return Err(error).wrap_err_with(|| format!("cannot write {}", tmp.display()));
    }
    if let Some(permissions) = permissions
        && let Err(error) = fs::set_permissions(&tmp, permissions)
    {
        tracing::debug!("{}: permissions not kept: {error}", tmp.display());
    }
    fs::rename(&tmp, &target)
        .inspect_err(|_| {
            let _ = fs::remove_file(&tmp);
        })
        .wrap_err_with(|| format!("cannot write {}", target.display()))
}

/// `path` as it should be written in a project file in `root` (canonical): relative
/// when it is inside the project, absolute otherwise.
///
/// "Inside" also when the path was given through a symbolic link: the project opened
/// as `~/proj` → `/data/proj` and a folder picked as `~/proj/libs/a` give `libs/a`.
/// Links below the project folder are kept as written (`libs/pdk` stays `libs/pdk`
/// when it links to a folder outside).
pub fn portable(root: &Path, path: &Path) -> String {
    let text = |rel: &Path| {
        if rel.as_os_str().is_empty() {
            ".".to_string()
        } else {
            rel.to_string_lossy().into_owned()
        }
    };
    if let Ok(rel) = path.strip_prefix(root) {
        return text(rel);
    }
    if path.is_absolute() {
        // An ancestor of the path that is the project folder under another name.
        for ancestor in path.ancestors().skip(1) {
            if ancestor.canonicalize().is_ok_and(|real| real == root)
                && let Ok(rel) = path.strip_prefix(ancestor)
            {
                return text(rel);
            }
        }
        // A link somewhere on the way that ends inside the project.
        if let Ok(real) = path.canonicalize()
            && let Ok(rel) = real.strip_prefix(root)
        {
            return text(rel);
        }
    }
    path.to_string_lossy().into_owned()
}

/// The format of a `cdept.toml` or `lib.toml`: a warning when it is newer than this
/// CDePT knows, or not a number.
pub(crate) fn check_format(doc: &DocumentMut, file: &Path, warnings: &mut Vec<String>) {
    let Some(item) = doc.get(FORMAT_KEY) else {
        return;
    };
    match item.as_integer() {
        Some(n) if n > FORMAT => warnings.push(format!(
            "{} is format {n}, this CDePT knows format {FORMAT}: parts of the file may be \
             ignored, and lost when CDePT changes it. Use a newer CDePT.",
            file.display()
        )),
        Some(n) if n < 1 => warnings.push(format!(
            "{}: `{FORMAT_KEY}` must be 1 or more",
            file.display()
        )),
        Some(_) => {}
        None => warnings.push(format!(
            "{}: `{FORMAT_KEY}` must be a number",
            file.display()
        )),
    }
}

/// For [`Project::load`], while the project file is read: its format is checked and
/// its `[suffixes]` apply to everything loaded while the result lives.
pub(crate) fn loading(
    doc: &DocumentMut,
    file: &Path,
    warnings: &mut Vec<String>,
) -> crate::formats::Loading {
    check_format(doc, file, warnings);
    crate::formats::Loading::start(doc, warnings)
}

/// For [`Project::load`], once the libraries are scanned: names that will cause
/// trouble, and the format of each `lib.toml`.
pub(crate) fn after_load(project: &mut Project) {
    let mut warnings = crate::hierarchy::shadowed_libraries(project);
    for group in same_but_case(project.libraries.iter().map(|l| l.name.as_str())) {
        warnings.push(format!(
            "libraries {} differ only by case: tools that ignore case will mix them up",
            quoted(&group)
        ));
    }
    for library in &project.libraries {
        if library.error.is_some() {
            continue;
        }
        let file = library.path.join(LIBRARY_FILE);
        if file.is_file()
            && let Ok(doc) = read_doc(&file)
        {
            check_format(&doc, &file, &mut warnings);
        }
        // Reference libraries (a PDK's) are large and not ours to tidy: not checked.
        if library.read_only {
            continue;
        }
        for group in same_but_case(library.cells.iter().map(|c| c.name.as_str())) {
            warnings.push(format!(
                "library '{}': cells {} differ only by case: they are one file on a file \
                 system that ignores case (macOS, Windows)",
                library.name,
                quoted(&group)
            ));
        }
        let depth = if library.layout == LibraryLayout::Cells {
            3
        } else if library.layout == LibraryLayout::Flat {
            1
        } else {
            2
        };
        let hidden = not_utf8(&library.path, depth);
        if let Some(first) = hidden.first() {
            warnings.push(format!(
                "library '{}': {} file name(s) are not UTF-8 and are not shown, e.g. {}",
                library.name,
                hidden.len(),
                first.display()
            ));
        }
    }
    project.warnings.extend(warnings);
}

fn quoted(names: &[String]) -> String {
    let names: Vec<String> = names.iter().map(|n| format!("'{n}'")).collect();
    names.join(" and ")
}

/// The groups of names that are the same when case is ignored.
fn same_but_case<'a>(names: impl Iterator<Item = &'a str>) -> Vec<Vec<String>> {
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for name in names {
        groups
            .entry(name.to_lowercase())
            .or_default()
            .push(name.to_string());
    }
    groups
        .into_values()
        .filter(|g| g.len() > 1)
        .map(|mut group| {
            group.sort();
            group
        })
        .collect()
}

/// Entries whose name is not UTF-8, down to `depth` folders below `dir`.
fn not_utf8(dir: &Path, depth: usize) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(items) = fs::read_dir(dir) else {
        return found;
    };
    for item in items.flatten() {
        let name = item.file_name();
        match name.to_str() {
            None => found.push(item.path()),
            Some(name) if name.starts_with('.') => {}
            Some(_) if depth > 1 && item.file_type().is_ok_and(|t| t.is_dir()) => {
                found.extend(not_utf8(&item.path(), depth - 1));
            }
            Some(_) => {}
        }
    }
    found
}

/// What a cell copy leaves out: names inside the view folders, from the `lib.toml`
/// of the library the cell is in.
///
/// ```toml
/// [copy]
/// exclude = ["results", "*.raw"]   # `*` stands for any text
/// ```
///
/// Without the key it is `results`: the run folders of a testbench view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyRule {
    pub exclude: Vec<String>,
}

impl Default for CopyRule {
    fn default() -> Self {
        CopyRule {
            exclude: COPY_EXCLUDE_DEFAULT.iter().map(|s| s.to_string()).collect(),
        }
    }
}

impl CopyRule {
    /// The rule of the library in `folder`. A `lib.toml` that cannot be read, or an
    /// `exclude` that is not a list of texts, gives the default.
    pub fn of_library(folder: &Path) -> CopyRule {
        let exclude = read_doc(&folder.join(LIBRARY_FILE)).ok().and_then(|doc| {
            let list = doc.get(COPY_KEY)?.get("exclude")?.as_array()?;
            list.iter()
                .map(|v| v.as_str().map(str::to_string))
                .collect::<Option<Vec<_>>>()
        });
        match exclude {
            Some(exclude) => CopyRule { exclude },
            None => CopyRule::default(),
        }
    }

    /// Whether a file or folder of this name inside a view folder is left out.
    pub fn excludes(&self, name: &str) -> bool {
        self.exclude.iter().any(|pattern| glob(pattern, name))
    }
}

/// `*` matches any text (also none); everything else matches itself.
fn glob(pattern: &str, name: &str) -> bool {
    let mut parts = pattern.split('*');
    let first = parts.next().unwrap_or("");
    let Some(mut rest) = name.strip_prefix(first) else {
        return false;
    };
    let parts: Vec<&str> = parts.collect();
    let Some((last, middle)) = parts.split_last() else {
        return rest.is_empty();
    };
    for part in middle {
        match rest.find(part) {
            Some(pos) => rest = &rest[pos + part.len()..],
            None => return false,
        }
    }
    rest.ends_with(last)
}

/// What [`copy_cell_dir`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Copied {
    pub files: usize,
    /// What the library's [`CopyRule`] left out.
    pub skipped: Vec<PathBuf>,
}

/// Copy the folder of a cell to `to` (which does not exist yet), leaving out what the
/// [`CopyRule`] of `library` (the folder of the cell's library) says. When it fails
/// in the middle the partial copy is removed again; if that fails too, the error
/// says what was left behind.
pub fn copy_cell_dir(from: &Path, to: &Path, library: &Path) -> Result<Copied> {
    let rule = CopyRule::of_library(library);
    let mut copied = Copied::default();
    let existed = to.exists();
    if let Err(error) = copy_tree(from, to, &rule, 0, &mut copied) {
        if existed {
            return Err(error.wrap_err(format!(
                "{} file(s) were copied into {} before, and are left there",
                copied.files,
                to.display()
            )));
        }
        return Err(match fs::remove_dir_all(to) {
            Ok(()) => error.wrap_err("nothing was copied"),
            Err(_) if !to.exists() => error.wrap_err("nothing was copied"),
            Err(left) => error.wrap_err(format!(
                "{} file(s) were copied into {} before, and could not be removed again ({left})",
                copied.files,
                to.display()
            )),
        });
    }
    for skipped in &copied.skipped {
        tracing::info!(
            "not copied (the library's copy rule): {}",
            skipped.display()
        );
    }
    Ok(copied)
}

/// `level` 0 is the cell folder, 1 a view folder: the rule applies from there down.
fn copy_tree(
    from: &Path,
    to: &Path,
    rule: &CopyRule,
    level: usize,
    copied: &mut Copied,
) -> Result<()> {
    fs::create_dir_all(to).wrap_err_with(|| format!("cannot create {}", to.display()))?;
    for item in fs::read_dir(from).wrap_err_with(|| format!("cannot read {}", from.display()))? {
        let item = item?;
        let path = item.path();
        if level >= 1 && rule.excludes(&item.file_name().to_string_lossy()) {
            copied.skipped.push(path);
            continue;
        }
        let target = to.join(item.file_name());
        if path.is_dir() {
            copy_tree(&path, &target, rule, level + 1, copied)?;
        } else {
            fs::copy(&path, &target).wrap_err_with(|| {
                format!("cannot copy {} to {}", path.display(), target.display())
            })?;
            copied.files += 1;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs() {
        assert!(glob("results", "results"));
        assert!(!glob("results", "results2"));
        assert!(glob("*.raw", "tran.raw"));
        assert!(glob("*.raw", ".raw"));
        assert!(!glob("*.raw", "tran.raw.txt"));
        assert!(glob("run_*", "run_12"));
        assert!(glob("a*b*c", "a--b--c"));
        assert!(!glob("a*b*c", "a--c--b"));
        assert!(glob("*", "anything"));
        assert!(!glob("", "x"));
    }

    #[test]
    fn names_that_differ_only_by_case() {
        let groups = same_but_case(["ota", "OTA", "inv", "Ota", "buf"].into_iter());
        assert_eq!(groups, [["OTA", "Ota", "ota"]]);
        assert!(same_but_case(["a", "b"].into_iter()).is_empty());
    }

    #[test]
    fn a_definition_keeps_its_shape_and_comments() {
        let mut doc: DocumentMut = "[libraries]\n\
            a = \"libs/a\"  # mine\n\
            b = { path = \"libs/b\", read_only = true } # theirs\n\
            c = \"libs/c\"\n"
            .parse()
            .unwrap();
        let file = Path::new("cdept.toml");
        // Only the folder: the short form stays.
        write_entry(entry(&mut doc, "a", file).unwrap(), "libs/a2", None, false);
        // More than the folder: a table, the comment kept.
        write_entry(
            entry(&mut doc, "c", file).unwrap(),
            "libs/c",
            Some(LibraryLayout::Flat),
            true,
        );
        // A table loses and gains keys in place.
        write_entry(
            entry(&mut doc, "b", file).unwrap(),
            "$PDK_ROOT/b",
            Some(LibraryLayout::Cells),
            false,
        );
        assert_eq!(
            doc.to_string(),
            "[libraries]\n\
             a = \"libs/a2\"  # mine\n\
             b = { path = \"$PDK_ROOT/b\", layout = \"cells\" } # theirs\n\
             c = { path = \"libs/c\", layout = \"flat\", read_only = true }\n"
        );
        assert_eq!(
            written("b", entry(&mut doc, "b", file).unwrap()).unwrap(),
            ("$PDK_ROOT/b".to_string(), Some(LibraryLayout::Cells), false)
        );
        assert!(entry(&mut doc, "nope", file).is_err());
    }

    #[test]
    fn newer_formats_are_warned_about() {
        let check = |text: &str| {
            let doc: DocumentMut = text.parse().unwrap();
            let mut warnings = Vec::new();
            check_format(&doc, Path::new("cdept.toml"), &mut warnings);
            warnings
        };
        assert!(check("name = \"x\"\n").is_empty());
        assert!(check("format = 1\n").is_empty());
        assert!(check("format = 2\n")[0].contains("format 2"));
        assert!(check("format = \"one\"\n")[0].contains("number"));
        assert!(check("format = 0\n")[0].contains("1 or more"));
    }
}
