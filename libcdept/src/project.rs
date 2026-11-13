//! A [`Project`] is a folder with a `cdept.toml` file, which names the project and
//! defines its libraries:
//!
//! ```toml
//! name = "sar_adc"
//! description = "10-bit SAR ADC"
//! technology = "ihp-sg13g2"
//!
//! # More definition files (same `include` and `[libraries]` keys), e.g. one shared by
//! # a team or kept next to a PDK.
//! include = ["../shared/libraries.toml"]
//!
//! [libraries]
//! analog = "libs/analog"
//! sg13g2_pr = { path = "$PDK_ROOT/ihp-sg13g2/libs.tech/xschem/sg13g2_pr", read_only = true }
//! ```
//!
//! Paths are relative to the file that contains them; `$VAR`, `${VAR}` and a leading
//! `~` are expanded. `layout = "cells" | "flat"` (see [`LibraryLayout`]) is detected
//! from the folder when left out. The file is meant to be edited by hand as well as by
//! CDePT, so edits made here keep its comments and formatting.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use eyre::{Context, Result, bail, eyre};
use toml_edit::{DocumentMut, InlineTable, Item, TableLike, Value, value};

use crate::library::{self, validate_name};
use crate::{Library, LibraryLayout};

/// The project file, in the project's root folder.
pub const PROJECT_FILE: &str = "cdept.toml";

const NEW_PROJECT: &str = r#"# CDePT project file. Edit it by hand or from the Library Manager.
name = ""

# Libraries, as  name = "folder"  (relative to this file; $VAR and ~ are expanded).
# Long form, for reference libraries such as PDK primitives:
#   sg13g2_pr = { path = "$PDK_ROOT/ihp-sg13g2/libs.tech/xschem/sg13g2_pr", read_only = true }
# Optional  layout = "cells"  (<library>/<cell>/<view>/<files>)  or  "flat"
# (<library>/<cell>.<ext>, one view per file); detected from the folder when left out.
[libraries]
"#;

/// Options for [`Project::add_library`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AddLibrary {
    /// `None` detects the layout from the folder each time the project is loaded.
    pub layout: Option<LibraryLayout>,
    pub read_only: bool,
    /// Create the folder (and its `lib.toml`) if it does not exist.
    pub create: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    /// The folder holding `cdept.toml`.
    pub root: PathBuf,
    /// The project file.
    pub file: PathBuf,
    pub name: String,
    pub description: Option<String>,
    /// Technology (PDK) name. Used from Phase 2 on.
    pub technology: Option<String>,
    /// Libraries in definition order: the project file first, then included files.
    pub libraries: Vec<Library>,
    /// Problems found while loading that did not prevent it.
    pub warnings: Vec<String>,
}

impl Project {
    /// The closest folder at or above `start` that holds a project file.
    pub fn find_root(start: &Path) -> Option<PathBuf> {
        start
            .ancestors()
            .find(|dir| dir.join(PROJECT_FILE).is_file())
            .map(Path::to_path_buf)
    }

    /// Create a project in `dir` (created if missing). `name` defaults to the folder name.
    pub fn create(dir: &Path, name: Option<&str>) -> Result<Project> {
        let file = dir.join(PROJECT_FILE);
        if file.exists() {
            bail!("{} already exists", file.display());
        }
        fs::create_dir_all(dir).wrap_err_with(|| format!("cannot create {}", dir.display()))?;
        let name = match name {
            Some(name) => name.to_string(),
            None => dir
                .canonicalize()
                .ok()
                .and_then(|d| Some(d.file_name()?.to_string_lossy().into_owned()))
                .unwrap_or_else(|| "project".into()),
        };
        let mut doc: DocumentMut = NEW_PROJECT.parse().expect("the template is valid TOML");
        doc["name"] = value(name);
        write(&file, &doc.to_string())?;
        Self::load(dir)
    }

    /// Load a project from its folder or its `cdept.toml`.
    pub fn load(path: &Path) -> Result<Project> {
        let file = if path.is_dir() {
            path.join(PROJECT_FILE)
        } else {
            path.to_path_buf()
        };
        if !file.is_file() {
            bail!("no {PROJECT_FILE} in {}", path.display());
        }
        let file = file
            .canonicalize()
            .wrap_err_with(|| format!("cannot resolve {}", file.display()))?;
        let root = file
            .parent()
            .ok_or_else(|| eyre!("{} has no parent folder", file.display()))?
            .to_path_buf();

        let doc = read(&file)?;
        let text = |key: &str| -> Result<Option<String>> {
            match doc.get(key) {
                None => Ok(None),
                Some(item) => match item.as_str() {
                    Some(s) if s.trim().is_empty() => Ok(None),
                    Some(s) => Ok(Some(s.to_string())),
                    None => Err(eyre!("{}: `{key}` must be a string", file.display())),
                },
            }
        };
        let mut project = Project {
            name: text("name")?.unwrap_or_else(|| {
                root.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            }),
            description: text("description")?,
            technology: text("technology")?,
            root,
            file: file.clone(),
            libraries: Vec::new(),
            warnings: Vec::new(),
        };
        let mut seen = HashSet::from([file.clone()]);
        project.load_definitions(&file, &doc, &mut seen)?;
        Ok(project)
    }

    /// Read the project file and all libraries from disk again.
    pub fn reload(&mut self) -> Result<()> {
        *self = Self::load(&self.file)?;
        Ok(())
    }

    /// Scan the library folders again, without re-reading the project file.
    pub fn rescan(&mut self) {
        for lib in &mut self.libraries {
            lib.scan();
        }
    }

    pub fn library(&self, name: &str) -> Option<&Library> {
        self.libraries.iter().find(|l| l.name == name)
    }

    /// `path` as it should be written in the project file: relative when inside the
    /// project folder, absolute otherwise.
    pub fn portable_path(&self, path: &Path) -> String {
        match path.strip_prefix(&self.root) {
            Ok(rel) if rel.as_os_str().is_empty() => ".".into(),
            Ok(rel) => rel.to_string_lossy().into_owned(),
            Err(_) => path.to_string_lossy().into_owned(),
        }
    }

    /// Define a library in the project file. `path` is written as given (it may be
    /// relative to the project folder and use `$VAR`).
    pub fn add_library(&mut self, name: &str, path: &str, options: AddLibrary) -> Result<()> {
        validate_name("library", name)?;
        if name.contains('.') {
            bail!("'.' is not allowed in a library name");
        }
        if self.library(name).is_some() {
            bail!("library '{name}' is already defined");
        }
        if path.trim().is_empty() {
            bail!("the library needs a folder");
        }
        let folder = resolve(path, &self.root).map_err(|e| eyre!(e))?;
        if options.create {
            library::create_library_folder(&folder, name)?;
        } else if !folder.is_dir() {
            bail!("folder not found: {}", folder.display());
        }

        let mut doc = read(&self.file)?;
        let libraries = doc
            .entry("libraries")
            .or_insert(toml_edit::table())
            .as_table_like_mut()
            .ok_or_else(|| eyre!("{}: `libraries` must be a table", self.file.display()))?;
        let definition = if options.layout.is_none() && !options.read_only {
            value(path)
        } else {
            let mut table = InlineTable::new();
            table.insert("path", path.into());
            if let Some(layout) = options.layout {
                table.insert("layout", layout.name().into());
            }
            if options.read_only {
                table.insert("read_only", true.into());
            }
            Item::Value(Value::InlineTable(table))
        };
        libraries.insert(name, definition);
        write(&self.file, &doc.to_string())?;
        self.reload()
    }

    /// Remove a library definition from the project file. The folder is left untouched.
    pub fn remove_library(&mut self, name: &str) -> Result<()> {
        let library = self
            .library(name)
            .ok_or_else(|| eyre!("no library '{name}'"))?;
        if library.defined_in != self.file {
            bail!(
                "library '{name}' is defined in {}; remove it there",
                library.defined_in.display()
            );
        }
        let mut doc = read(&self.file)?;
        let removed = doc
            .get_mut("libraries")
            .and_then(Item::as_table_like_mut)
            .and_then(|libraries| libraries.remove(name));
        if removed.is_none() {
            bail!("library '{name}' is not in {}", self.file.display());
        }
        write(&self.file, &doc.to_string())?;
        self.reload()
    }

    /// Collect the libraries of one definition file, then of the files it includes.
    fn load_definitions(
        &mut self,
        file: &Path,
        doc: &DocumentMut,
        seen: &mut HashSet<PathBuf>,
    ) -> Result<()> {
        let dir = file.parent().unwrap_or(Path::new("."));
        match doc.get("libraries") {
            None => {}
            Some(item) => {
                let table = item
                    .as_table_like()
                    .ok_or_else(|| eyre!("{}: `libraries` must be a table", file.display()))?;
                for (name, item) in table.iter() {
                    match self.define(name, item, file, dir) {
                        Ok(library) => self.libraries.push(library),
                        Err(e) => self.warnings.push(format!("{}: {e}", file.display())),
                    }
                }
            }
        }

        let includes = match doc.get("include") {
            None => Vec::new(),
            Some(item) => item
                .as_array()
                .and_then(|a| a.iter().map(Value::as_str).collect::<Option<Vec<_>>>())
                .ok_or_else(|| eyre!("{}: `include` must be a list of files", file.display()))?,
        };
        for include in includes {
            let path = match resolve(include, dir) {
                Ok(path) => path,
                Err(e) => {
                    self.warnings
                        .push(format!("{}: include '{include}': {e}", file.display()));
                    continue;
                }
            };
            let Ok(path) = path.canonicalize() else {
                self.warnings.push(format!(
                    "{}: included file not found: {}",
                    file.display(),
                    path.display()
                ));
                continue;
            };
            if !seen.insert(path.clone()) {
                continue;
            }
            match read(&path).and_then(|doc| self.load_definitions(&path, &doc, seen)) {
                Ok(()) => {}
                Err(e) => self.warnings.push(format!("{e:#}")),
            }
        }
        Ok(())
    }

    fn define(&self, name: &str, item: &Item, file: &Path, dir: &Path) -> Result<Library, String> {
        if self.library(name).is_some() {
            return Err(format!("library '{name}' is already defined; ignored"));
        }
        let (path, layout, read_only) = match item.as_str() {
            Some(path) => (path, None, false),
            None => {
                let table = item.as_table_like().ok_or_else(|| {
                    format!("library '{name}' must be a folder or a table with `path`")
                })?;
                definition(name, table)?
            }
        };
        let mut library = Library {
            name: name.to_string(),
            path: PathBuf::from(path),
            layout: layout.unwrap_or_default(),
            read_only,
            description: None,
            defined_in: file.to_path_buf(),
            error: None,
            cells: Vec::new(),
        };
        // A library that cannot be resolved stays listed, with its error.
        match resolve(path, dir) {
            Ok(folder) => {
                library.layout = layout.unwrap_or_else(|| LibraryLayout::detect(&folder));
                library.path = folder;
                library.scan();
            }
            Err(e) => library.error = Some(e),
        }
        Ok(library)
    }
}

/// The fields of a long-form library definition.
fn definition<'a>(
    name: &str,
    table: &'a dyn TableLike,
) -> Result<(&'a str, Option<LibraryLayout>, bool), String> {
    let mut path = None;
    let mut layout = None;
    let mut read_only = false;
    for (key, item) in table.iter() {
        match key {
            "path" => path = item.as_str(),
            "layout" => {
                layout = Some(
                    item.as_str()
                        .and_then(LibraryLayout::from_name)
                        .ok_or_else(|| {
                            format!("library '{name}': `layout` must be \"cells\" or \"flat\"")
                        })?,
                );
            }
            "read_only" => {
                read_only = item.as_bool().ok_or_else(|| {
                    format!("library '{name}': `read_only` must be true or false")
                })?;
            }
            other => return Err(format!("library '{name}': unknown key `{other}`")),
        }
    }
    let path = path.ok_or_else(|| format!("library '{name}' needs a `path`"))?;
    Ok((path, layout, read_only))
}

fn read(file: &Path) -> Result<DocumentMut> {
    let text =
        fs::read_to_string(file).wrap_err_with(|| format!("cannot read {}", file.display()))?;
    text.parse::<DocumentMut>()
        .map_err(|e| eyre!("{}: {}", file.display(), e.to_string().trim_end()))
}

/// Write through a temporary file so a failure cannot truncate the project file.
fn write(file: &Path, text: &str) -> Result<()> {
    let tmp = file.with_extension("toml.tmp");
    fs::write(&tmp, text).wrap_err_with(|| format!("cannot write {}", tmp.display()))?;
    fs::rename(&tmp, file).wrap_err_with(|| format!("cannot write {}", file.display()))
}

/// Expand variables in `path` and make it absolute relative to `base`.
fn resolve(path: &str, base: &Path) -> Result<PathBuf, String> {
    let expanded = expand(path, |name| std::env::var(name).ok())?;
    Ok(base.join(expanded))
}

/// Expand `$VAR`, `${VAR}` and a leading `~`.
pub fn expand(text: &str, var: impl Fn(&str) -> Option<String>) -> Result<String, String> {
    let lookup = |name: &str| var(name).ok_or_else(|| format!("${name} is not set"));
    let mut out = String::new();
    let mut rest = text;
    if rest == "~" || rest.starts_with("~/") {
        out.push_str(&lookup("HOME")?);
        rest = &rest[1..];
    }
    while let Some(pos) = rest.find('$') {
        out.push_str(&rest[..pos]);
        let after = &rest[pos + 1..];
        if let Some(braced) = after.strip_prefix('{') {
            let end = braced
                .find('}')
                .ok_or_else(|| format!("unclosed ${{ in '{text}'"))?;
            out.push_str(&lookup(&braced[..end])?);
            rest = &braced[end + 1..];
        } else {
            let end = after
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .unwrap_or(after.len());
            if end == 0 {
                out.push('$');
            } else {
                out.push_str(&lookup(&after[..end])?);
            }
            rest = &after[end..];
        }
    }
    out.push_str(rest);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Template;

    fn env(name: &str) -> Option<String> {
        match name {
            "HOME" => Some("/home/me".into()),
            "PDK_ROOT" => Some("/pdk".into()),
            _ => None,
        }
    }

    #[test]
    fn expands_variables() {
        assert_eq!(expand("libs/a", env).unwrap(), "libs/a");
        assert_eq!(expand("$PDK_ROOT/x", env).unwrap(), "/pdk/x");
        assert_eq!(expand("${PDK_ROOT}x/$PDK_ROOT", env).unwrap(), "/pdkx//pdk");
        assert_eq!(expand("~/libs", env).unwrap(), "/home/me/libs");
        assert_eq!(expand("a~b$", env).unwrap(), "a~b$");
        assert!(expand("$NOPE/x", env).unwrap_err().contains("NOPE"));
        assert!(expand("${PDK_ROOT", env).is_err());
    }

    #[test]
    fn create_add_remove_keeps_hand_edits() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("adc");
        let mut project = Project::create(&root, None).unwrap();
        assert_eq!(project.name, "adc");
        assert!(project.libraries.is_empty());
        assert!(Project::create(&root, None).is_err());

        let options = AddLibrary {
            create: true,
            ..Default::default()
        };
        project
            .add_library("analog", "libs/analog", options)
            .unwrap();
        assert!(root.join("libs/analog/lib.toml").is_file());
        assert!(
            project
                .add_library("analog", "libs/other", options)
                .is_err()
        );
        assert!(project.add_library("bad name", "libs/x", options).is_err());
        assert!(
            project
                .add_library("missing", "libs/missing", AddLibrary::default())
                .is_err()
        );

        // A hand edit with a comment, then another edit through the API.
        let mut text = fs::read_to_string(&project.file).unwrap();
        text.push_str("# my reference library\nref = { path = \"ref\", read_only = true }\n");
        fs::write(&project.file, text).unwrap();
        fs::create_dir(root.join("ref")).unwrap();
        fs::write(root.join("ref/nmos.sym"), "").unwrap();
        let flat = AddLibrary {
            layout: Some(LibraryLayout::Flat),
            create: true,
            ..Default::default()
        };
        project
            .add_library("digital", "libs/digital", flat)
            .unwrap();

        let names: Vec<_> = project.libraries.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["analog", "ref", "digital"]);
        let reference = project.library("ref").unwrap();
        assert!(reference.read_only);
        assert_eq!(reference.layout, LibraryLayout::Flat);
        assert_eq!(reference.cells.len(), 1);
        assert_eq!(
            project.library("digital").unwrap().layout,
            LibraryLayout::Flat
        );

        project.remove_library("analog").unwrap();
        let text = fs::read_to_string(&project.file).unwrap();
        assert!(text.contains("# my reference library"));
        assert!(text.contains("# CDePT project file"));
        assert!(!text.contains("analog ="));
        // Removing a definition never deletes the folder.
        assert!(root.join("libs/analog").is_dir());
        assert!(project.library("analog").is_none());
    }

    #[test]
    fn loads_includes_and_reports_problems() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("proj");
        let shared = tmp.path().join("shared");
        fs::create_dir_all(root.join("libs/a")).unwrap();
        fs::create_dir_all(shared.join("prims")).unwrap();
        fs::write(
            root.join(PROJECT_FILE),
            r#"
name = "demo"
technology = "ihp-sg13g2"
include = ["../shared/libraries.toml", "nowhere.toml"]

[libraries]
a = "libs/a"
gone = "libs/gone"
unset = "$CDEPT_TEST_SURELY_UNSET/lib"

[libraries.broken]
layout = "flat"
"#,
        )
        .unwrap();
        // The shared file includes the project back (a cycle) and redefines `a`.
        fs::write(
            shared.join("libraries.toml"),
            "include = [\"../proj/cdept.toml\"]\n[libraries]\nprims = \"prims\"\na = \"prims\"\n",
        )
        .unwrap();

        let project = Project::load(&root).unwrap();
        assert_eq!(project.name, "demo");
        assert_eq!(project.technology.as_deref(), Some("ihp-sg13g2"));
        let names: Vec<_> = project.libraries.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["a", "gone", "unset", "prims"]);
        assert!(project.library("a").unwrap().error.is_none());
        assert!(project.library("gone").unwrap().error.is_some());
        assert!(project.library("unset").unwrap().error.is_some());
        let prims = project.library("prims").unwrap();
        assert_eq!(prims.path, shared.canonicalize().unwrap().join("prims"));
        assert_ne!(prims.defined_in, project.file);
        // broken (no path), nowhere.toml, duplicate `a`.
        assert_eq!(project.warnings.len(), 3, "{:?}", project.warnings);

        // Libraries from an included file cannot be removed from the project file.
        let mut project = project;
        assert!(project.remove_library("prims").is_err());
        assert_eq!(Project::find_root(&root.join("libs/a")), Some(root.clone()));
    }

    #[test]
    fn library_operations_through_a_project() {
        let tmp = tempfile::tempdir().unwrap();
        let mut project = Project::create(tmp.path(), Some("p")).unwrap();
        let options = AddLibrary {
            create: true,
            ..Default::default()
        };
        project.add_library("a", "libs/a", options).unwrap();
        project
            .library("a")
            .unwrap()
            .create_view("ota", Template::XschemSchematic)
            .unwrap();
        project.rescan();
        assert_eq!(project.library("a").unwrap().cells.len(), 1);
        assert_eq!(
            project.portable_path(&project.root.join("libs/a")),
            "libs/a"
        );
        assert_eq!(project.portable_path(Path::new("/elsewhere")), "/elsewhere");
    }

    #[test]
    fn rejects_malformed_files() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join(PROJECT_FILE), "name = [1\n").unwrap();
        assert!(Project::load(tmp.path()).is_err());
        fs::write(tmp.path().join(PROJECT_FILE), "libraries = 3\n").unwrap();
        assert!(Project::load(tmp.path()).is_err());
        assert!(Project::load(&tmp.path().join("nope")).is_err());
    }
}
