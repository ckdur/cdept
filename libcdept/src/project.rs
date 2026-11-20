//! A [`Project`] is a folder with a `cdept.toml` file, which names the project and
//! defines its libraries:
//!
//! ```toml
//! name = "sar_adc"
//! description = "10-bit SAR ADC"
//! technology = "ihp-sg13g2"        # the PDK name: $PDK
//! pdk_root = "/opt/ext/OpenPDKs"    # where PDKs are installed: $PDK_ROOT
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
//! `~` are expanded. `$PDK_ROOT` and `$PDK` mean the project's `pdk_root` and `technology`
//! when set, the environment's variables otherwise (see [`Pdk`]). `layout = "cells" | "flat"` (see [`LibraryLayout`]) is detected
//! from the folder when left out. The file is meant to be edited by hand as well as by
//! CDePT, so edits made here keep its comments and formatting.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use eyre::{Context, Result, bail, eyre};
use toml_edit::{DocumentMut, InlineTable, Item, TableLike, Value, value};

use crate::library::{self, validate_name};
use crate::technology::Pdk;
use crate::{Library, LibraryLayout};

/// The project file, in the project's root folder.
pub const PROJECT_FILE: &str = "cdept.toml";

const NEW_PROJECT: &str = r#"# CDePT project file. Edit it by hand or from the Library Manager.
name = ""

# The PDK: its name ($PDK) and the folder PDKs are installed in ($PDK_ROOT). What is
# left out is taken from the environment CDePT is started in. Both are also used in
# the paths below and in the environment of every tool CDePT starts.
# technology = "ihp-sg13g2"
# pdk_root = "/opt/ext/OpenPDKs"

# Libraries, as  name = "folder"  (relative to this file; $VAR and ~ are expanded).
# Long form, for reference libraries such as PDK primitives:
#   sg13g2_pr = { path = "$PDK_ROOT/ihp-sg13g2/libs.tech/xschem/sg13g2_pr", read_only = true }
# Optional  layout = "cells"  (<library>/<cell>/<view>/<files>)  or  "flat"
# (<library>/<cell>.<ext>, one view per file); detected from the folder when left out.
[libraries]

# Start-up file of the PDK for the embedded xschem (it is read before the libraries
# above are added to xschem's search path):
#   [xschem]
#   rc = "$PDK_ROOT/ihp-sg13g2/libs.tech/xschem/xschemrc"

# The embedded KLayout: folders for KLAYOUT_PATH (default: the PDK's libs.tech/klayout),
# the technology (a name or a .lyt file), the layer properties (.lyp) and the DRC and
# LVS decks (default: found in the KLAYOUT_PATH folders):
#   [klayout]
#   path = ["$PDK_ROOT/ihp-sg13g2/libs.tech/klayout"]
#   tech = "sg13g2"
#   layer_properties = "$PDK_ROOT/ihp-sg13g2/libs.tech/klayout/tech/sg13g2.lyp"
#   drc = "$PDK_ROOT/ihp-sg13g2/libs.tech/klayout/tech/drc/ihp-sg13g2.drc"
#   lvs = "$PDK_ROOT/ihp-sg13g2/libs.tech/klayout/tech/lvs/sg13g2.lvs"
#   devices = "devices.toml"   # schematic symbol -> layout PCell (docs/LINKING.md)
# How flylines look in this project (on top of each user's own settings):
#   [klayout.flylines]
#   color = '#ff40ff'   # also highlight, short
#   width = 2
#   line = "dashed"     # solid, dashed, dotted, dash-dotted
#   opacity = 80        # percent
#   live = true         # follow the edits
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
    /// `technology` as written in the project file: the PDK name. [`Project::pdk`] has
    /// the value in use, which may come from the environment.
    pub technology: Option<String>,
    /// `pdk_root` as written in the project file (`$VAR` and `~` not expanded).
    pub pdk_root: Option<String>,
    /// `$PDK_ROOT` and `$PDK` as resolved: the project file first, then the environment.
    pub pdk: Pdk,
    /// Start-up files (`xschemrc`, Tcl) the embedded xschem reads before the project's
    /// libraries are added, typically the PDK's. From `[xschem] rc = …`.
    pub xschem_rc: Vec<PathBuf>,
    /// Settings of the embedded KLayout, from `[klayout]`.
    pub klayout: KlayoutSettings,
    /// Libraries in definition order: the project file first, then included files.
    pub libraries: Vec<Library>,
    /// Problems found while loading that did not prevent it.
    pub warnings: Vec<String>,
    /// Reads the environment. A function pointer so tests can replace it.
    env: EnvFn,
}

/// `[klayout]` in the project file (docs/KLAYOUT.md §5).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KlayoutSettings {
    /// `path`: folders for `KLAYOUT_PATH` (technologies, macros, PCell libraries).
    /// Empty: the PDK's `libs.tech/klayout` folder is used, see
    /// [`Project::klayout_path`].
    pub path: Vec<PathBuf>,
    /// `tech`: the technology, as a name KLayout knows or a `.lyt` file.
    pub tech: Option<String>,
    /// `layer_properties`: a `.lyp` file.
    pub layer_properties: Option<PathBuf>,
    /// `drc`: the DRC deck (`.drc`, `.lydrc`). `None`: looked for in the
    /// `KLAYOUT_PATH` folders, see [`crate::klayout::verify::find_deck`].
    pub drc: Option<PathBuf>,
    /// `lvs`: the LVS deck (`.lvs`, `.lylvs`), like `drc`.
    pub lvs: Option<PathBuf>,
    /// `devices`: the project's device map for linking schematics with layouts
    /// (docs/LINKING.md), on top of the PDK's. `None`: the PDK's only.
    pub devices: Option<PathBuf>,
    /// `[klayout.flylines]`: how flylines look in this project (on top of the user's
    /// settings), docs/LINKING.md §8.
    pub flylines: crate::link::nets::FlylineOverride,
}

/// The environment lookup of a project (not part of what makes two projects equal).
#[derive(Clone, Copy)]
struct EnvFn(fn(&str) -> Option<String>);

impl PartialEq for EnvFn {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}
impl Eq for EnvFn {}
impl std::fmt::Debug for EnvFn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("EnvFn")
    }
}

/// The fields of the project file that are not libraries, for [`Project::set_settings`].
/// An empty text removes the key.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub name: String,
    pub description: String,
    pub technology: String,
    pub pdk_root: String,
}

fn process_env(name: &str) -> Option<String> {
    std::env::var(name).ok()
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
        Self::create_with(dir, name, None, None)
    }

    /// Like [`Project::create`], also setting `technology` and `pdk_root`.
    pub fn create_with(
        dir: &Path,
        name: Option<&str>,
        technology: Option<&str>,
        pdk_root: Option<&str>,
    ) -> Result<Project> {
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
        for (key, setting) in [("technology", technology), ("pdk_root", pdk_root)] {
            if let Some(setting) = setting.filter(|s| !s.trim().is_empty()) {
                doc[key] = value(setting.trim());
            }
        }
        write(&file, &doc.to_string())?;
        Self::load(dir)
    }

    /// Load a project from its folder or its `cdept.toml`.
    pub fn load(path: &Path) -> Result<Project> {
        Self::load_with(path, process_env)
    }

    /// [`Project::load`] with another source of environment variables.
    pub fn load_with(path: &Path, env: fn(&str) -> Option<String>) -> Result<Project> {
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
        let technology = text("technology")?;
        let pdk_root = text("pdk_root")?;
        let mut warnings = Vec::new();
        // `pdk_root` may use `$VAR` and `~`, but not `$PDK_ROOT` itself.
        let expanded_root =
            pdk_root
                .as_deref()
                .and_then(|raw| match expand(raw, env).map(|e| root.join(e)) {
                    Ok(path) => Some(path.to_string_lossy().into_owned()),
                    Err(e) => {
                        warnings.push(format!("pdk_root '{raw}': {e}; using the environment"));
                        None
                    }
                });
        let pdk = Pdk::resolve(expanded_root, technology.as_deref(), env, &mut warnings);
        let mut project = Project {
            name: text("name")?.unwrap_or_else(|| {
                root.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            }),
            description: text("description")?,
            technology,
            pdk_root,
            pdk,
            xschem_rc: Vec::new(),
            klayout: KlayoutSettings::default(),
            root,
            file: file.clone(),
            libraries: Vec::new(),
            warnings,
            env: EnvFn(env),
        };
        project.load_xschem_rc(&doc);
        project.load_klayout(&doc);
        let mut seen = HashSet::from([file.clone()]);
        project.load_definitions(&file, &doc, &mut seen)?;
        Ok(project)
    }

    /// `[xschem] rc`: one file or a list. A file that cannot be found is left out,
    /// with a warning.
    fn load_xschem_rc(&mut self, doc: &DocumentMut) {
        let Some(item) = doc.get("xschem").and_then(|table| table.get("rc")) else {
            return;
        };
        let names: Vec<&str> = match (item.as_str(), item.as_array()) {
            (Some(name), _) => vec![name],
            (None, Some(list)) => list.iter().filter_map(|v| v.as_str()).collect(),
            (None, None) => {
                self.warnings
                    .push("[xschem] rc must be a file name or a list of them".into());
                return;
            }
        };
        for name in names {
            match self.resolve(name, &self.root) {
                Ok(file) if file.is_file() => self.xschem_rc.push(file),
                Ok(file) => self
                    .warnings
                    .push(format!("[xschem] rc: {} not found", file.display())),
                Err(error) => self.warnings.push(format!("[xschem] rc '{name}': {error}")),
            }
        }
    }

    /// `[klayout]`: `path` (one folder or a list), `tech`, `layer_properties`. Folders
    /// and files that cannot be found are left out, with a warning.
    /// `[klayout.flylines]`: color, width, line, opacity, highlight, short, live,
    /// delay_ms. A value of the wrong kind is left out, with a warning.
    fn load_flylines(&mut self, item: &toml_edit::Item) {
        use crate::link::nets::{LineStyle, parse_color};
        let Some(table) = item.as_table_like() else {
            self.warnings
                .push("[klayout] flylines must be a table".into());
            return;
        };
        let mut warnings = Vec::new();
        let mut warn = |key: &str, what: &str| {
            warnings.push(format!("[klayout.flylines] {key} must be {what}"));
        };
        let mut out = crate::link::nets::FlylineOverride::default();
        for (key, value) in table.iter() {
            let text = value.as_str();
            let number = value.as_integer();
            match key {
                "color" | "highlight" | "short" => {
                    match text.filter(|t| parse_color(t).is_some()) {
                        Some(t) => {
                            let t = Some(t.to_string());
                            match key {
                                "color" => out.color = t,
                                "highlight" => out.highlight = t,
                                _ => out.short = t,
                            }
                        }
                        None => warn(key, "a colour \"#rrggbb\""),
                    }
                }
                "width" => match number.filter(|n| (1..=16).contains(n)) {
                    Some(n) => out.width = Some(n as u32),
                    None => warn(key, "a number of pixels from 1 to 16"),
                },
                "opacity" => match number.filter(|n| (0..=100).contains(n)) {
                    Some(n) => out.opacity = Some(n as u8),
                    None => warn(key, "a percentage from 0 to 100"),
                },
                "delay_ms" => match number.filter(|n| (0..=10_000).contains(n)) {
                    Some(n) => out.delay_ms = Some(n as u32),
                    None => warn(key, "a number of milliseconds"),
                },
                "line" => match text.and_then(LineStyle::parse) {
                    Some(l) => out.line = Some(l),
                    None => warn(key, "solid, dashed, dotted or dash-dotted"),
                },
                "live" => match value.as_bool() {
                    Some(b) => out.live = Some(b),
                    None => warn(key, "true or false"),
                },
                other => warn(
                    other,
                    "one of color, width, line, opacity, highlight, short, \
                     live, delay_ms (not known)",
                ),
            }
        }
        self.warnings.extend(warnings);
        self.klayout.flylines = out;
    }

    fn load_klayout(&mut self, doc: &DocumentMut) {
        let Some(table) = doc.get("klayout") else {
            return;
        };
        if let Some(fly) = table.get("flylines") {
            self.load_flylines(fly);
        }
        if let Some(item) = table.get("path") {
            let names: Vec<&str> = match (item.as_str(), item.as_array()) {
                (Some(name), _) => vec![name],
                (None, Some(list)) => list.iter().filter_map(|v| v.as_str()).collect(),
                (None, None) => {
                    self.warnings
                        .push("[klayout] path must be a folder or a list of them".into());
                    Vec::new()
                }
            };
            for name in names {
                match self.resolve(name, &self.root) {
                    Ok(dir) if dir.is_dir() => self.klayout.path.push(dir),
                    Ok(dir) => self
                        .warnings
                        .push(format!("[klayout] path: {} not found", dir.display())),
                    Err(error) => self
                        .warnings
                        .push(format!("[klayout] path '{name}': {error}")),
                }
            }
        }
        if let Some(tech) = table.get("tech") {
            match tech.as_str() {
                Some(name) if name.ends_with(".lyt") => match self.resolve(name, &self.root) {
                    Ok(file) if file.is_file() => {
                        self.klayout.tech = Some(file.to_string_lossy().into_owned())
                    }
                    Ok(file) => self
                        .warnings
                        .push(format!("[klayout] tech: {} not found", file.display())),
                    Err(error) => self
                        .warnings
                        .push(format!("[klayout] tech '{name}': {error}")),
                },
                Some(name) => self.klayout.tech = Some(name.to_string()),
                None => self.warnings.push("[klayout] tech must be a text".into()),
            }
        }
        if let Some(lyp) = table.get("layer_properties") {
            match lyp
                .as_str()
                .map(|name| (name, self.resolve(name, &self.root)))
            {
                Some((_, Ok(file))) if file.is_file() => self.klayout.layer_properties = Some(file),
                Some((_, Ok(file))) => self.warnings.push(format!(
                    "[klayout] layer_properties: {} not found",
                    file.display()
                )),
                Some((name, Err(error))) => self
                    .warnings
                    .push(format!("[klayout] layer_properties '{name}': {error}")),
                None => self
                    .warnings
                    .push("[klayout] layer_properties must be a file name".into()),
            }
        }
        for key in ["drc", "lvs", "devices"] {
            let Some(item) = table.get(key) else {
                continue;
            };
            let deck = match item
                .as_str()
                .map(|name| (name, self.resolve(name, &self.root)))
            {
                Some((_, Ok(file))) if file.is_file() => Some(file),
                Some((_, Ok(file))) => {
                    let text = format!("[klayout] {key}: {} not found", file.display());
                    self.warnings.push(text);
                    None
                }
                Some((name, Err(error))) => {
                    let text = format!("[klayout] {key} '{name}': {error}");
                    self.warnings.push(text);
                    None
                }
                None => {
                    let text = format!("[klayout] {key} must be a file name");
                    self.warnings.push(text);
                    None
                }
            };
            match key {
                "drc" => self.klayout.drc = deck,
                "lvs" => self.klayout.lvs = deck,
                _ => self.klayout.devices = deck,
            }
        }
    }

    /// Folders for `KLAYOUT_PATH`: `[klayout] path`, else the PDK's
    /// `$PDK_ROOT/$PDK/libs.tech/klayout` when it exists (and `$PDK` names one PDK).
    pub fn klayout_path(&self) -> Vec<PathBuf> {
        if !self.klayout.path.is_empty() {
            return self.klayout.path.clone();
        }
        let (Some(root), Some(name)) = (self.pdk.var("PDK_ROOT"), self.pdk.var("PDK")) else {
            return Vec::new();
        };
        if name.is_empty() || name.contains(':') {
            return Vec::new();
        }
        let dir = Path::new(root).join(name).join("libs.tech").join("klayout");
        if dir.is_dir() { vec![dir] } else { Vec::new() }
    }

    /// The value of environment variable `name` as seen in the project's paths:
    /// `$PDK_ROOT` and `$PDK` from [`Project::pdk`], the rest from the environment.
    pub fn var(&self, name: &str) -> Option<String> {
        match self.pdk.var(name) {
            Some(value) => Some(value.to_string()),
            None => (self.env.0)(name),
        }
    }

    /// Expand `$VAR` and `~` in `path` and make it absolute relative to `base`.
    pub fn resolve(&self, path: &str, base: &Path) -> Result<PathBuf, String> {
        let expanded = expand(path, |name| self.var(name))?;
        Ok(base.join(expanded))
    }

    /// Environment variables for every tool CDePT starts for this project: `PDK_ROOT`
    /// and `PDK` as resolved, set on top of whatever the tool would inherit.
    pub fn tool_env(&self) -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
        self.pdk.env()
    }

    /// Change `name`, `description`, `technology` and `pdk_root` in the project file,
    /// keeping its comments and everything else, then load it again (the PDK is
    /// resolved anew, libraries are scanned again).
    pub fn set_settings(&mut self, settings: &Settings) -> Result<()> {
        let mut doc = read(&self.file)?;
        for (key, text) in [
            ("name", &settings.name),
            ("description", &settings.description),
            ("technology", &settings.technology),
            ("pdk_root", &settings.pdk_root),
        ] {
            let text = text.trim();
            if text.is_empty() {
                // A project always has a `name` line, as the template does.
                if key == "name" {
                    doc[key] = value("");
                } else {
                    doc.remove(key);
                }
            } else {
                match doc.get_mut(key) {
                    Some(item) if item.is_value() => {
                        // Keep the comment after the old value.
                        let suffix = item.as_value().and_then(|v| v.decor().suffix().cloned());
                        let mut new = Value::from(text);
                        if let Some(suffix) = suffix {
                            new.decor_mut().set_suffix(suffix);
                        }
                        *item = Item::Value(new);
                    }
                    _ => doc[key] = value(text),
                }
            }
        }
        write(&self.file, &doc.to_string())?;
        let env = self.env.0;
        *self = Self::load_with(&self.file, env)?;
        Ok(())
    }

    /// Read the project file and all libraries from disk again.
    pub fn reload(&mut self) -> Result<()> {
        *self = Self::load_with(&self.file, self.env.0)?;
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
        let folder = self.resolve(path, &self.root).map_err(|e| eyre!(e))?;
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
            let path = match self.resolve(include, dir) {
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
        match self.resolve(path, dir) {
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
    use crate::technology::Source;

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

        let project = Project::load_with(&root, |_| None).unwrap();
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

    fn pdk_env(name: &str) -> Option<String> {
        match name {
            "HOME" => Some("/home/me".into()),
            "PDK_ROOT" => Some("/env/pdks".into()),
            "PDK" => Some("sky130A".into()),
            "PDKS" => Some("/opt/pdks".into()),
            _ => None,
        }
    }

    #[test]
    fn pdk_comes_from_the_project_then_the_environment() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        for dir in ["pdks/ihp/libs", "envpdks/sky130A/libs"] {
            fs::create_dir_all(root.join(dir)).unwrap();
        }
        fs::write(root.join("pdks/ihp/libs/inv.sym"), "").unwrap();
        fs::write(root.join("pdks/ihp/xschemrc"), "").unwrap();
        fs::write(
            root.join(PROJECT_FILE),
            r#"
name = "p"
technology = "ihp"
pdk_root = "pdks"

[libraries]
prims = { path = "$PDK_ROOT/$PDK/libs", read_only = true }

[xschem]
rc = "$PDK_ROOT/$PDK/xschemrc"
"#,
        )
        .unwrap();
        let project = Project::load_with(root, pdk_env).unwrap();
        let pdk_root = root.canonicalize().unwrap().join("pdks");
        assert_eq!(project.pdk.var("PDK"), Some("ihp"));
        assert_eq!(
            project.pdk.var("PDK_ROOT"),
            Some(pdk_root.to_str().unwrap())
        );
        assert_eq!(project.pdk.root.as_ref().unwrap().source, Source::Project);
        assert_eq!(project.library("prims").unwrap().error, None);
        assert_eq!(project.library("prims").unwrap().cells.len(), 1);
        assert_eq!(project.xschem_rc, [pdk_root.join("ihp/xschemrc")]);
        // $PDK of the environment differs: one warning, the project wins.
        assert_eq!(project.warnings.len(), 1, "{:?}", project.warnings);
        assert!(project.warnings[0].contains("sky130A"));
        let tool_env = project.tool_env();
        assert!(tool_env.contains(&("PDK".into(), "ihp".into())));

        // Nothing in the project file: the environment fills in.
        fs::write(
            root.join(PROJECT_FILE),
            "name = \"p\"\n[libraries]\nl = \"$PDK_ROOT/$PDK/libs\"\n",
        )
        .unwrap();
        let project = Project::load_with(root, pdk_env).unwrap();
        assert_eq!(
            project.pdk.name.as_ref().unwrap().source,
            Source::Environment
        );
        assert_eq!(
            project.library("l").unwrap().path,
            Path::new("/env/pdks/sky130A/libs")
        );
        assert!(project.warnings.is_empty());

        // Nothing anywhere: the library keeps its error, nothing is set.
        let project = Project::load_with(root, |_| None).unwrap();
        assert!(project.pdk.root.is_none() && project.tool_env().is_empty());
        assert!(project.library("l").unwrap().error.is_some());

        // `pdk_root` expands other variables and `~`, and a bad one falls back.
        fs::write(root.join(PROJECT_FILE), "pdk_root = \"$PDKS/x\"\n").unwrap();
        let project = Project::load_with(root, pdk_env).unwrap();
        assert_eq!(project.pdk.var("PDK_ROOT"), Some("/opt/pdks/x"));
        fs::write(root.join(PROJECT_FILE), "pdk_root = \"$NOPE/x\"\n").unwrap();
        let project = Project::load_with(root, pdk_env).unwrap();
        assert_eq!(project.pdk.var("PDK_ROOT"), Some("/env/pdks"));
        assert_eq!(project.warnings.len(), 1);
    }

    #[test]
    fn settings_keep_comments_and_other_keys() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("p");
        let mut project = Project::create_with(&root, Some("p"), Some("ihp"), None).unwrap();
        let text = fs::read_to_string(&project.file).unwrap();
        assert!(text.contains("# pdk_root = "), "{text}");
        assert!(text.contains("technology = \"ihp\""));
        let mut with_comment = text.replace("technology = \"ihp\"", "technology = \"ihp\" # mine");
        with_comment.push_str("# note\n[extra]\nkey = 1\n");
        fs::write(&project.file, with_comment).unwrap();
        project.reload().unwrap();

        project
            .set_settings(&Settings {
                name: "q".into(),
                description: "desc".into(),
                technology: "sky130A".into(),
                pdk_root: "/opt/x".into(),
            })
            .unwrap();
        let text = fs::read_to_string(&project.file).unwrap();
        assert!(text.contains("# mine") && text.contains("# note") && text.contains("[extra]"));
        assert!(text.contains("# CDePT project file"));
        assert_eq!(project.name, "q");
        assert_eq!(project.technology.as_deref(), Some("sky130A"));
        assert_eq!(project.pdk.var("PDK_ROOT"), Some("/opt/x"));
        // The keys come before the tables, so they stay top-level keys.
        assert!(text.find("pdk_root = ").unwrap() < text.find("[libraries]").unwrap());

        project
            .set_settings(&Settings {
                name: "q".into(),
                ..Default::default()
            })
            .unwrap();
        assert!(project.technology.is_none() && project.pdk_root.is_none());
        assert!(project.description.is_none());
    }

    #[test]
    fn klayout_settings_and_the_pdk_default() {
        let tmp = tempfile::tempdir().unwrap();
        let pdks = tmp.path().join("pdks");
        let tech = pdks.join("ihp/libs.tech/klayout");
        fs::create_dir_all(tech.join("tech")).unwrap();
        fs::write(tech.join("tech/a.lyp"), "").unwrap();
        fs::write(tech.join("tech/a.drc"), "").unwrap();
        let root = tmp.path().join("p");
        let project =
            Project::create_with(&root, Some("p"), Some("ihp"), Some(pdks.to_str().unwrap()))
                .unwrap();
        assert!(project.klayout.path.is_empty());
        assert_eq!(project.klayout_path(), vec![tech.clone()]);
        let text = fs::read_to_string(&project.file).unwrap();
        assert!(text.contains("#   [klayout]"));

        let mut text = text;
        text.push_str(
            "[klayout]\npath = [\"$PDK_ROOT/ihp/libs.tech/klayout\", \"nope\"]\ntech = \"sg13g2\"\n\
             layer_properties = \"$PDK_ROOT/ihp/libs.tech/klayout/tech/a.lyp\"\n\
             drc = \"$PDK_ROOT/ihp/libs.tech/klayout/tech/a.drc\"\nlvs = \"no.lvs\"\n",
        );
        fs::write(&project.file, text).unwrap();
        let project = Project::load(&root).unwrap();
        assert_eq!(project.klayout.path, vec![tech.clone()]);
        assert_eq!(project.klayout.tech.as_deref(), Some("sg13g2"));
        assert_eq!(
            project.klayout.layer_properties,
            Some(tech.join("tech/a.lyp"))
        );
        assert_eq!(project.klayout.drc, Some(tech.join("tech/a.drc")));
        assert_eq!(project.klayout.lvs, None);
        assert!(project.warnings.iter().any(|w| w.contains("[klayout] lvs")));
        assert!(
            project.warnings.iter().any(|w| w.contains("nope")),
            "{:?}",
            project.warnings
        );
    }

    #[test]
    fn flylines_of_the_project() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join(PROJECT_FILE),
            "name = \"p\"\n[klayout.flylines]\ncolor = \"#00ff00\"\nwidth = 3\nline = \"dotted\"\n\
             opacity = 150\nlive = false\nshade = 1\n",
        )
        .unwrap();
        let project = Project::load(tmp.path()).unwrap();
        let fly = &project.klayout.flylines;
        assert_eq!(fly.color.as_deref(), Some("#00ff00"));
        assert_eq!(fly.width, Some(3));
        assert_eq!(fly.line, Some(crate::link::nets::LineStyle::Dotted));
        assert_eq!(fly.live, Some(false));
        assert_eq!(fly.opacity, None);
        let warned = |what: &str| project.warnings.iter().any(|w| w.contains(what));
        assert!(warned("opacity"), "{:?}", project.warnings);
        assert!(warned("shade"), "{:?}", project.warnings);
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
