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
//!
//! `format = 1` says which format of the file this is (a newer one loads with a
//! warning), and `[suffixes]` what a file suffix means in this project
//! (`".lib" = "spice"`; see [`crate::formats`]). Definitions are changed where they are
//! defined, also in an included file: [`crate::project_edit`].
//!
//! Two more files may sit next to it (docs/TECH.md):
//!
//! - `tech.toml` ([`crate::tech`]): the technology snapshot, what CDePT read out of the
//!   PDK when the project was set up for it. It gives the defaults for `[xschem] rc`
//!   and `[klayout]` and brings the PDK's libraries (read-only). Whatever `cdept.toml`
//!   says wins over it.
//! - `cdept.local.toml` ([`LOCAL_FILE`]): what differs per user and machine and is not
//!   committed: `pdk_root`, and an `[env]` table of variables for `$VAR` in paths, also
//!   given to every tool. It wins over `cdept.toml`.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use eyre::{Context, Result, bail, eyre};
use toml_edit::{DocumentMut, InlineTable, Item, TableLike, Value, value};

use crate::library::{self, validate_name};
use crate::tech::{Catalog, PdkDir, Tech};
use crate::technology::{Pdk, Resolved, Source};
use crate::{Library, LibraryLayout};

/// The project file, in the project's root folder.
pub const PROJECT_FILE: &str = "cdept.toml";

/// The user's own settings for a project, next to the project file and not meant for
/// version control: `pdk_root` and `[env]`.
pub const LOCAL_FILE: &str = "cdept.local.toml";

const NEW_LOCAL: &str = r#"# CDePT settings of this user and this machine for the project next to this file.
# Not for version control (CDePT adds the file to .gitignore). What is set here wins
# over cdept.toml.

# Where PDKs are installed on this machine ($PDK_ROOT):
# pdk_root = "/opt/ext/OpenPDKs"

# Variables for $VAR in the project's paths, also given to every tool CDePT starts:
# [env]
# MY_LIBS = "/work/libs"
"#;

const NEW_PROJECT: &str = r#"# CDePT project file. Edit it by hand or from the Library Manager.
format = 1
name = ""

# The PDK: its name ($PDK). The folder PDKs are installed in ($PDK_ROOT) differs per
# machine: it is `pdk_root` in cdept.local.toml next to this file (not committed), else
# PDK_ROOT of the environment CDePT is started in. Both are used in the paths below
# and given to every tool CDePT starts.
# `cdept init --technology <pdk>` and `cdept tech scan` read the PDK's files and write
# tech.toml next to this file: tool set-ups, decks, model corners, the PDK's libraries.
# What is set in this file wins over tech.toml.
# technology = "ihp-sg13g2"

# Libraries, as  name = "folder"  (relative to this file; $VAR and ~ are expanded).
# Long form, for reference libraries such as PDK primitives:
#   sg13g2_pr = { path = "$PDK_ROOT/ihp-sg13g2/libs.tech/xschem/sg13g2_pr", read_only = true }
# Optional  layout = "cells"  (<library>/<cell>/<view>/<files>)  or  "flat"
# (<library>/<cell>.<ext>, one view per file); detected from the folder when left out.
[libraries]

# What a file suffix means in this project, where CDePT's guess is wrong or missing
# (spice, cdl, verilog, liberty, gds, …; "none": not a design file):
#   [suffixes]
#   ".lib" = "spice"
#   ".net" = "spice"

# Start-up file of the PDK for the embedded xschem (it is read before the libraries
# above are added to xschem's search path; default: the one tech.toml names):
#   [xschem]
#   rc = "$PDK_ROOT/ihp-sg13g2/libs.tech/xschem/xschemrc"

# The embedded KLayout: folders for KLAYOUT_PATH (default: the PDK's libs.tech/klayout),
# the technology (a name or a .lyt file), the layer properties (.lyp) and the DRC and
# LVS decks (defaults: what tech.toml names, else found in the KLAYOUT_PATH folders):
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

# Magic in batch mode (cdept magic extract|drc|lef): a run that takes longer is stopped
# (default 15m; "0" is no limit):
#   [magic]
#   timeout = "10m"

# Simulator runs: a run point that takes longer is stopped and ends as an error (default
# 30m; "0" is no limit; a testbench's setup.toml `timeout` and `cdept sim run --timeout`
# come first). `openvaf_timeout` bounds one OpenVAF compilation of the VACASK models
# (default 20m):
#   [sim]
#   timeout = "30m"
#   openvaf_timeout = "20m"
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

#[derive(Debug, Clone, PartialEq)]
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
    /// `pdk_root` as written (`$VAR` and `~` not expanded): the one of the user's
    /// `cdept.local.toml`, else the one of the project file.
    pub pdk_root: Option<String>,
    /// `[env]` of the user's `cdept.local.toml`, expanded: variables for `$VAR` in the
    /// project's paths, also given to every tool ([`Project::tool_env`]).
    pub local_env: BTreeMap<String, String>,
    /// `$PDK_ROOT` and `$PDK` as resolved: the user's local file, the project file,
    /// the technology snapshot (its PDK name), then the environment.
    pub pdk: Pdk,
    /// Start-up files (`xschemrc`, Tcl) the embedded xschem reads before the project's
    /// libraries are added, typically the PDK's. From `[xschem] rc = …`.
    pub xschem_rc: Vec<PathBuf>,
    /// Settings of the embedded KLayout, from `[klayout]`.
    pub klayout: KlayoutSettings,
    /// `[magic]`: the batch runs of Magic.
    pub magic: MagicSettings,
    /// `[sim]`: time limits of the simulator runs.
    pub sim: SimSettings,
    /// The technology snapshot (`tech.toml` next to the project file), when the
    /// project has one. See [`crate::tech`].
    pub tech: Option<Tech>,
    /// The snapshot is in use: it was made for the project's PDK. (One made for
    /// another PDK stays in [`Project::tech`], with a warning, and is not used.)
    pub tech_in_use: bool,
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
    /// `lvs_variables`: variables of the LVS deck (`-rd name=value`) that every LVS run
    /// of the project passes: what `cdept.toml` sets, else what `tech.toml` says for the
    /// PDK's deck. `-D` of `cdept layout lvs` comes on top.
    pub lvs_variables: std::collections::BTreeMap<String, String>,
    /// `[klayout.flylines]`: how flylines look in this project (on top of the user's
    /// settings), docs/LINKING.md §8.
    pub flylines: crate::link::nets::FlylineOverride,
}

/// `[magic]` in the project file (docs/MAGIC.md).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MagicSettings {
    /// `timeout = "10m"`: a batch run that takes longer is stopped. `None`: not set
    /// (the default of [`crate::magic::DEFAULT_TIMEOUT`]); zero: no limit.
    pub timeout: Option<std::time::Duration>,
}

/// `[sim]` in the project file (docs/reference/files/cdept-toml.md).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SimSettings {
    /// `timeout = "30m"`: a run point that takes longer is stopped (`None`: not set, the
    /// default of [`crate::sim::run::DEFAULT_TIMEOUT`]; zero: no limit).
    pub timeout: Option<std::time::Duration>,
    /// `openvaf_timeout = "20m"`: the limit of one OpenVAF compilation of the VACASK
    /// models (`None`: [`crate::batch::openvaf::DEFAULT_TIMEOUT`]; zero: no limit).
    pub openvaf_timeout: Option<std::time::Duration>,
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

    /// Like [`Project::create`], also setting `technology` (in the project file) and
    /// `pdk_root` (in the user's `cdept.local.toml`, as it differs per machine).
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
        if let Some(technology) = technology.filter(|s| !s.trim().is_empty()) {
            doc["technology"] = value(technology.trim());
        }
        write(&file, &doc.to_string())?;
        if let Some(pdk_root) = pdk_root.filter(|s| !s.trim().is_empty()) {
            set_local_pdk_root(dir, Some(pdk_root.trim()))?;
        }
        Self::load(dir)
    }

    /// Create a project in `dir` for the PDK `pdk`, with the snapshot `tech` and
    /// `catalog` of a scan (see [`crate::tech::scan`]).
    pub fn create_for_pdk(
        dir: &Path,
        name: Option<&str>,
        pdk: &PdkDir,
        tech: &Tech,
        catalog: &Catalog,
    ) -> Result<Project> {
        let mut project = Self::create(dir, name)?;
        project.set_technology(pdk, tech, catalog)?;
        Ok(project)
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
        let mut warnings = Vec::new();
        // The format of the file; its `[suffixes]` apply to all that is loaded below.
        let _suffixes = crate::project_edit::loading(&doc, &file, &mut warnings);
        // The user's own file: the PDK folder of this machine and more variables.
        let local = Local::read(&root, env, &mut warnings);
        let lookup = |name: &str| local.env.get(name).cloned().or_else(|| env(name));
        let (pdk_root, root_source) = match (&local.pdk_root, text("pdk_root")?) {
            (Some(local), _) => (Some(local.clone()), Source::Local),
            (None, project) => (project, Source::Project),
        };
        // `pdk_root` may use `$VAR` and `~`, but not `$PDK_ROOT` itself.
        let expanded_root =
            pdk_root
                .as_deref()
                .and_then(|raw| match expand(raw, lookup).map(|e| root.join(e)) {
                    Ok(path) => Some(Resolved {
                        value: path.to_string_lossy().into_owned(),
                        source: root_source,
                    }),
                    Err(e) => {
                        warnings.push(format!("pdk_root '{raw}': {e}; using the environment"));
                        None
                    }
                });
        // The snapshot: a project that does not name its PDK takes the snapshot's.
        let tech = match crate::tech::file::load_tech_with(&root, &mut warnings) {
            Ok(tech) => tech,
            Err(e) => {
                warnings.push(format!("{e:#}"));
                None
            }
        };
        let snapshot_pdk = tech
            .as_ref()
            .map(|t| t.pdk.name.clone())
            .filter(|name| !name.is_empty());
        let mut pdk = Pdk::resolve_from(
            expanded_root,
            technology.as_deref().or(snapshot_pdk.as_deref()),
            &lookup,
            &mut warnings,
        );
        if technology.is_none()
            && let Some(name) = &mut pdk.name
            && snapshot_pdk.is_some()
        {
            name.source = Source::Snapshot;
        }
        let mut project = Project {
            name: text("name")?.unwrap_or_else(|| {
                root.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            }),
            description: text("description")?,
            technology,
            pdk_root,
            local_env: local.env,
            pdk,
            xschem_rc: Vec::new(),
            klayout: KlayoutSettings::default(),
            magic: MagicSettings::default(),
            sim: SimSettings::default(),
            tech,
            tech_in_use: false,
            root,
            file: file.clone(),
            libraries: Vec::new(),
            warnings,
            env: EnvFn(env),
        };
        project.check_snapshot();
        project.load_xschem_rc(&doc);
        project.load_klayout(&doc);
        project.load_magic(&doc);
        project.load_sim(&doc);
        project.apply_snapshot();
        let mut seen = HashSet::from([file.clone()]);
        project.load_definitions(&file, &doc, &mut seen)?;
        project.load_snapshot_libraries();
        crate::project_edit::after_load(&mut project);
        Ok(project)
    }

    /// Is the snapshot made for the project's PDK, and is that PDK on this machine?
    /// Sets [`Project::tech_in_use`]; says in `warnings` why not.
    fn check_snapshot(&mut self) {
        let Some(tech) = &self.tech else {
            return;
        };
        let made_for = tech.pdk.name.as_str();
        if let Some(name) = self.pdk.var("PDK")
            && !made_for.is_empty()
            && name != made_for
        {
            self.warnings.push(format!(
                "{} was made for the PDK \"{made_for}\" but the project uses \"{name}\": \
                 it is not used; run `cdept tech refresh` to make it again",
                crate::tech::TECH_FILE
            ));
            return;
        }
        self.tech_in_use = true;
        match self.pdk.var("PDK_ROOT") {
            None => self.warnings.push(format!(
                "{} names files below $PDK_ROOT, which is not set: set `pdk_root` in \
                 {LOCAL_FILE} or PDK_ROOT in the environment",
                crate::tech::TECH_FILE
            )),
            Some(root) if !made_for.is_empty() && !Path::new(root).join(made_for).is_dir() => {
                self.warnings.push(format!(
                    "the PDK \"{made_for}\" is not in {root}: set `pdk_root` in {LOCAL_FILE} \
                     to the folder that holds it on this machine"
                ))
            }
            Some(_) => {
                // The PDK is there: is it still the one that was scanned?
                if let Some(why) = self.snapshot_stale() {
                    self.warnings.push(format!(
                        "{} is out of date ({why}): run `cdept tech refresh`",
                        crate::tech::TECH_FILE
                    ));
                }
            }
        }
    }

    /// A path of the snapshot on this machine; `None` when it cannot be worked out
    /// (`$PDK_ROOT` not set) or, with `must_exist`, is not there. Nothing is reported:
    /// [`Project::check_snapshot`] has said what is wrong with the PDK folder.
    fn snapshot_path(&self, path: &crate::tech::TechPath, must_exist: bool) -> Option<PathBuf> {
        let path = self.root.join(path.resolve(|name| self.var(name)).ok()?);
        (!must_exist || path.exists()).then_some(path)
    }

    /// Fill what the project file leaves open from the snapshot: `[xschem] rc` and the
    /// `[klayout]` settings. What the project file sets stays.
    fn apply_snapshot(&mut self) {
        if !self.tech_in_use {
            return;
        }
        let Some(tech) = self.tech.clone() else {
            return;
        };
        if self.xschem_rc.is_empty()
            && let Some(rc) = &tech.xschem.rc
            && let Some(file) = self.snapshot_path(rc, true)
        {
            self.xschem_rc.push(file);
        }
        let klayout = &tech.klayout;
        if self.klayout.path.is_empty() {
            self.klayout.path = klayout
                .path
                .iter()
                .filter_map(|dir| self.snapshot_path(dir, true))
                .collect();
        }
        if self.klayout.tech.is_none() {
            self.klayout.tech = klayout.tech.clone().or_else(|| {
                let lyt = self.snapshot_path(klayout.lyt.as_ref()?, true)?;
                Some(lyt.to_string_lossy().into_owned())
            });
        }
        if self.klayout.layer_properties.is_none() {
            self.klayout.layer_properties = klayout
                .layer_properties
                .as_ref()
                .and_then(|lyp| self.snapshot_path(lyp, true));
        }
        if self.klayout.drc.is_none() {
            self.klayout.drc = klayout
                .drc
                .first()
                .and_then(|deck| self.snapshot_path(&deck.file, true));
        }
        if self.klayout.lvs.is_none() {
            self.klayout.lvs = klayout
                .lvs
                .first()
                .and_then(|deck| self.snapshot_path(&deck.file, true));
        }
        // The project file's variables win over the snapshot's, name by name.
        for (name, value) in &klayout.lvs_variables {
            self.klayout
                .lvs_variables
                .entry(name.clone())
                .or_insert_with(|| value.clone());
        }
    }

    /// The libraries the PDK brings ([`Tech::project_libraries`]: those marked
    /// `enabled`, each name once), after the project's own: read-only, defined in
    /// `tech.toml`. A name the project defines wins. A PDK library left out because
    /// another of its name is listed was noted by the scan (`notes` in `tech.toml`);
    /// here it is only logged, or every load of the project would warn.
    fn load_snapshot_libraries(&mut self) {
        if !self.tech_in_use {
            return;
        }
        let Some(tech) = self.tech.clone() else {
            return;
        };
        let file = crate::tech::file::tech_file(&self.root);
        let (entries, left_out) = tech.project_libraries();
        for (listed, other) in &left_out {
            tracing::debug!(
                "PDK library '{}': {} is listed, {} is left out",
                listed.name,
                listed.path,
                other.path
            );
        }
        for entry in entries {
            if self.library(&entry.name).is_some() {
                // Defined by the project: that one is meant.
                continue;
            }
            let mut library = new_library(&entry.name, Path::new(entry.path.as_str()), &file);
            library.read_only = true;
            match entry.path.resolve(|name| self.var(name)) {
                Ok(folder) => {
                    let folder = self.root.join(folder);
                    library.layout = LibraryLayout::from_name(&entry.layout)
                        .unwrap_or_else(|| LibraryLayout::detect(&folder));
                    library.path = folder;
                    if library.layout == LibraryLayout::Ref {
                        let warnings = self.apply_library_files(&mut library, &tech);
                        self.warnings.extend(warnings);
                    }
                    library.scan();
                }
                Err(e) => library.error = Some(e),
            }
            if library.description.is_none() {
                library.description = entry.description.clone();
            }
            self.libraries.push(library);
        }
    }

    /// The files `tech.toml` names for a reference library (`[[refs.library]] files`)
    /// are the preferred main files of its views, as they stand in the file. A file
    /// that is not there is a warning; the folder scan decides that format.
    fn apply_library_files(&self, library: &mut Library, tech: &crate::tech::Tech) -> Vec<String> {
        let mut warnings = Vec::new();
        let Some(files) = tech
            .refs
            .libraries
            .iter()
            .find(|entry| self.snapshot_path(&entry.path, false).as_deref() == Some(&library.path))
            .and_then(|entry| entry.files.as_ref())
        else {
            return warnings;
        };
        let (found, missing) = files.existing(|path| self.snapshot_path(path, false));
        library.preferred = found;
        for (format, file) in missing {
            warnings.push(format!(
                "{}: library {} names {format} {file}, which does not exist; the PDK may \
                 have moved or been updated: run `cdept tech refresh` (this replaces {}, \
                 see its warning) or correct the file",
                crate::tech::TECH_FILE,
                library.name,
                crate::tech::TECH_FILE
            ));
        }
        warnings
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
                "color" | "highlight" | "short" | "pin" => {
                    match text.filter(|t| parse_color(t).is_some()) {
                        Some(t) => {
                            let t = Some(t.to_string());
                            match key {
                                "color" => out.color = t,
                                "highlight" => out.highlight = t,
                                "pin" => out.pin = t,
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

    /// `[magic]`: `timeout`, a duration (`"10m"`, `"90s"`, a number of seconds). A value
    /// that cannot be read is left out, with a warning.
    fn load_magic(&mut self, doc: &DocumentMut) {
        if let Some(d) = self.duration_setting(doc, "magic", "timeout") {
            self.magic.timeout = Some(d);
        }
    }

    /// `[sim]`: `timeout` and `openvaf_timeout`, durations like `[magic] timeout`.
    fn load_sim(&mut self, doc: &DocumentMut) {
        if let Some(d) = self.duration_setting(doc, "sim", "timeout") {
            self.sim.timeout = Some(d);
        }
        if let Some(d) = self.duration_setting(doc, "sim", "openvaf_timeout") {
            self.sim.openvaf_timeout = Some(d);
        }
    }

    /// A duration setting (`"10m"`, `"90s"`, a number of seconds) of `[table]`. One that
    /// cannot be read is left out, with a warning.
    fn duration_setting(
        &mut self,
        doc: &DocumentMut,
        table: &str,
        key: &str,
    ) -> Option<std::time::Duration> {
        let item = doc.get(table).and_then(|t| t.get(key))?;
        let parsed = match (item.as_str(), item.as_integer(), item.as_float()) {
            (Some(text), _, _) => crate::jobs::parse_duration(text).map_err(|e| format!("{e}")),
            (_, Some(n), _) if n >= 0 => Ok(std::time::Duration::from_secs(n as u64)),
            (_, _, Some(f)) if f >= 0.0 && f.is_finite() => {
                Ok(std::time::Duration::from_secs_f64(f))
            }
            _ => Err("it must be a duration such as \"10m\"".to_string()),
        };
        match parsed {
            Ok(d) => Some(d),
            Err(e) => {
                self.warnings.push(format!("[{table}] {key}: {e}"));
                None
            }
        }
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
        if let Some(item) = table.get("lvs_variables") {
            match item.as_table_like() {
                Some(vars) => {
                    for (name, value) in vars.iter() {
                        let text = match value.as_value() {
                            Some(v) if v.is_str() => v.as_str().map(str::to_string),
                            Some(v) if v.is_bool() => v.as_bool().map(|b| b.to_string()),
                            Some(v) if v.is_integer() => v.as_integer().map(|n| n.to_string()),
                            Some(v) if v.is_float() => v.as_float().map(|n| n.to_string()),
                            _ => None,
                        };
                        match text {
                            Some(text) => {
                                self.klayout.lvs_variables.insert(name.to_string(), text);
                            }
                            None => self.warnings.push(format!(
                                "[klayout] lvs_variables: {name} must be a text, a number or true/false"
                            )),
                        }
                    }
                }
                None => self
                    .warnings
                    .push("[klayout] lvs_variables must be a table".into()),
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
    /// `$PDK_ROOT` and `$PDK` from [`Project::pdk`], then `[env]` of the user's local
    /// file, the rest from the environment.
    pub fn var(&self, name: &str) -> Option<String> {
        // `$CDEPT_PROJECT` in a snapshot's paths: the project folder (generated files live there).
        if name == "CDEPT_PROJECT" {
            return Some(self.root.to_string_lossy().into_owned());
        }
        match self.pdk.var(name) {
            Some(value) => Some(value.to_string()),
            None => self
                .local_env
                .get(name)
                .cloned()
                .or_else(|| (self.env.0)(name)),
        }
    }

    /// Expand `$VAR` and `~` in `path` and make it absolute relative to `base`.
    pub fn resolve(&self, path: &str, base: &Path) -> Result<PathBuf, String> {
        let expanded = expand(path, |name| self.var(name))?;
        Ok(base.join(expanded))
    }

    /// Environment variables for every tool CDePT starts for this project, set on top
    /// of whatever the tool would inherit: `[env]` of the user's local file, `PDK_ROOT`
    /// and `PDK` as resolved (one PDK, also when the environment lists several) and,
    /// with both known, `PDKPATH` (`$PDK_ROOT/$PDK`, as open_pdks scripts expect it).
    pub fn tool_env(&self) -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
        let mut env: Vec<(std::ffi::OsString, std::ffi::OsString)> = self
            .local_env
            .iter()
            .filter(|(name, _)| !matches!(name.as_str(), "PDK_ROOT" | "PDK" | "PDKPATH"))
            .map(|(name, value)| (name.into(), value.into()))
            .collect();
        env.extend(self.pdk.env());
        if let Some(pdk) = self.pdk.dir() {
            env.push(("PDKPATH".into(), pdk.dir().into_os_string()));
        }
        env
    }

    /// The user's local file of this project (it may not exist).
    pub fn local_file(&self) -> PathBuf {
        self.root.join(LOCAL_FILE)
    }

    /// The project's PDK folder, when `$PDK_ROOT` and `$PDK` are both known.
    pub fn pdk_dir(&self) -> Option<PdkDir> {
        self.pdk.dir()
    }

    /// The snapshot, when the project has one and it is in use (made for its PDK).
    pub fn snapshot(&self) -> Option<&Tech> {
        self.tech.as_ref().filter(|_| self.tech_in_use)
    }

    /// A file or folder the snapshot names, on this machine, when it is there.
    pub fn tech_path(&self, path: &crate::tech::TechPath) -> Option<PathBuf> {
        self.snapshot_path(path, true)
    }

    /// Magic's start-up file of the PDK (`<pdk>.magicrc`), from the snapshot.
    pub fn magic_rc(&self) -> Option<PathBuf> {
        self.tech_path(self.snapshot()?.magic.rc.as_ref()?)
    }

    /// The technology name magic knows the PDK by (the `tech` line of a `.mag` file),
    /// from the snapshot.
    pub fn magic_tech(&self) -> Option<String> {
        self.snapshot()?.magic.tech.clone()
    }

    /// Netgen's LVS set-up file of the PDK, from the snapshot.
    pub fn netgen_setup(&self) -> Option<PathBuf> {
        self.tech_path(self.snapshot()?.netgen.setup.as_ref()?)
    }

    /// The model sets or simulators the snapshot has models for (`ngspice`, `xyce`).
    pub fn model_simulators(&self) -> Vec<String> {
        self.snapshot()
            .map(|tech| {
                tech.models
                    .simulators()
                    .iter()
                    .map(|s| s.to_string())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The corner sets of a simulator's models (`tt`, `ff`, `ss`, …), the usual ones
    /// first. Empty without a snapshot.
    pub fn model_corners(&self, simulator: &str) -> Vec<String> {
        self.snapshot()
            .map(|tech| {
                let corners = tech.models.corners(simulator);
                corners.iter().map(|c| c.to_string()).collect()
            })
            .unwrap_or_default()
    }

    /// The lines a testbench needs for the models of `simulator` at `corner`:
    /// `.lib <file> <section>` and `.include <file>`, with this machine's paths.
    pub fn model_lines(&self, simulator: &str, corner: &str) -> Result<Vec<String>, String> {
        let tech = self.snapshot().ok_or_else(|| {
            format!(
                "the project has no technology snapshot ({}): run `cdept tech scan`",
                crate::tech::TECH_FILE
            )
        })?;
        tech.models.lines(simulator, corner, |name| self.var(name))
    }

    /// The start-up files a simulator must read for the PDK's models (ngspice's
    /// `.spiceinit`), those that are on this machine.
    pub fn model_init(&self, simulator: &str) -> Vec<PathBuf> {
        let Some((_, set)) = self.snapshot().and_then(|tech| tech.models.set(simulator)) else {
            return Vec::new();
        };
        set.init
            .iter()
            .filter_map(|file| self.tech_path(file))
            .collect()
    }

    /// The database unit of new layouts in micron: the snapshot's (KLayout
    /// technology), else 1 nm.
    pub fn dbu(&self) -> f64 {
        self.snapshot()
            .and_then(|tech| tech.klayout.dbu)
            .filter(|dbu| dbu.is_finite() && *dbu > 0.0)
            .unwrap_or(0.001)
    }

    /// What new views of this project start with: the database unit and the magic
    /// technology of the snapshot. See [`crate::template::Setup`].
    pub fn template_setup(&self) -> crate::template::Setup {
        crate::template::Setup {
            dbu: self.dbu(),
            magic_tech: self.magic_tech(),
            ..crate::template::Setup::default()
        }
    }

    /// The catalog behind the technology snapshot (`tech.catalog.json`): layers,
    /// PCells, symbols, cells. Read from disk each time; `None` when there is none.
    pub fn catalog(&self) -> Result<Option<Catalog>> {
        crate::tech::file::load_catalog(&self.root)
    }

    /// Why the snapshot should be made again, if so: it was made for another PDK, or
    /// the PDK on this machine is another version than the one scanned. Looks at the
    /// PDK folder ([`crate::tech::discover::info`]), so it is not part of loading.
    pub fn snapshot_stale(&self) -> Option<String> {
        let tech = self.tech.as_ref()?;
        if !self.tech_in_use {
            return Some(format!(
                "made for the PDK \"{}\", the project uses \"{}\"",
                tech.pdk.name,
                self.pdk.var("PDK").unwrap_or("none")
            ));
        }
        let pdk = self.pdk.dir().filter(|pdk| pdk.dir().is_dir())?;
        let now = crate::tech::discover::info(&pdk);
        let changed = |what: &str, then: &Option<String>, now: &Option<String>| match (then, now) {
            (Some(then), Some(now)) if then != now => Some(format!(
                "the PDK's {what} was {then} when scanned and is {now} now"
            )),
            _ => None,
        };
        changed("commit", &tech.pdk.commit, &now.commit)
            .or_else(|| changed("version", &tech.pdk.version, &now.version))
            .or_else(|| self.config_changed(tech))
    }

    /// The PDK's LibreLane/OpenLane configuration files are not the ones the scan
    /// read (their contents were recorded as `[refs.digital] config_hash`).
    fn config_changed(&self, tech: &crate::tech::Tech) -> Option<String> {
        let digital = tech.refs.digital.as_ref()?;
        let then = digital.config_hash.as_ref()?;
        let now = crate::tech::refs::config_hash(digital, |path| self.snapshot_path(path, false));
        (now.as_ref() != Some(then))
            .then(|| "the PDK's configuration changed since the scan".to_string())
    }

    /// Make `pdk` the PDK of this project: write the snapshot (`tech.toml` and
    /// `tech.catalog.json`), set `technology` in the project file and, when the
    /// environment does not already say so, the PDK folder of this machine in the
    /// user's local file; then load the project again.
    pub fn set_technology(&mut self, pdk: &PdkDir, tech: &Tech, catalog: &Catalog) -> Result<()> {
        crate::tech::file::save(&self.root, tech, catalog)?;
        let mut doc = read(&self.file)?;
        set_key(&mut doc, "technology", &pdk.name);
        write(&self.file, &doc.to_string())?;
        // Where would $PDK_ROOT point without the local file?
        let env = self.env.0;
        let without = doc
            .get("pdk_root")
            .and_then(Item::as_str)
            .and_then(|raw| expand(raw, env).ok())
            .map(|path| self.root.join(path))
            .or_else(|| {
                env("PDK_ROOT")
                    .filter(|v| !v.trim().is_empty())
                    .map(PathBuf::from)
            });
        if without
            .as_deref()
            .is_some_and(|dir| same_folder(dir, &pdk.root))
        {
            if Local::has_pdk_root(&self.root) {
                set_local_pdk_root(&self.root, None)?;
            }
        } else {
            set_local_pdk_root(&self.root, Some(&pdk.root.to_string_lossy()))?;
        }
        self.reload()
    }

    /// Scan the project's PDK again and write the snapshot. The PDK is the one in use
    /// (`technology` and the PDK folder); fails when that is not a PDK folder.
    pub fn scan_technology(&mut self, helpers: bool) -> Result<()> {
        let pdk = self.pdk.dir().ok_or_else(|| {
            eyre!(
                "the project has no PDK: set `technology` in {PROJECT_FILE} and `pdk_root` \
                 in {LOCAL_FILE} (or PDK and PDK_ROOT in the environment)"
            )
        })?;
        if !pdk.dir().is_dir() {
            bail!("the PDK folder {} does not exist", pdk.dir().display());
        }
        let mut ctx = crate::tech::ScanContext::new(pdk.clone());
        ctx.helpers = helpers;
        ctx.env.extend(self.tool_env());
        let (tech, catalog) = crate::tech::scan(&ctx);
        self.set_technology(&pdk, &tech, &catalog)
    }

    /// Change `name`, `description` and `technology` in the project file, keeping its
    /// comments and everything else, and the PDK folder of this machine (`pdk_root`,
    /// in the user's local file); then load the project again (the PDK is resolved
    /// anew, libraries are scanned again).
    pub fn set_settings(&mut self, settings: &Settings) -> Result<()> {
        let mut doc = read(&self.file)?;
        for (key, text) in [
            ("name", &settings.name),
            ("description", &settings.description),
            ("technology", &settings.technology),
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
                set_key(&mut doc, key, text);
            }
        }
        // The PDK folder differs per machine: it goes into the local file. One written
        // in the project file by hand is kept as long as it is the one meant.
        let pdk_root = settings.pdk_root.trim();
        let in_project = doc
            .get("pdk_root")
            .and_then(Item::as_str)
            .map(str::to_string);
        if pdk_root.is_empty() {
            doc.remove("pdk_root");
            if Local::has_pdk_root(&self.root) {
                set_local_pdk_root(&self.root, None)?;
            }
        } else if in_project.as_deref() == Some(pdk_root) {
            if Local::has_pdk_root(&self.root) {
                set_local_pdk_root(&self.root, None)?;
            }
        } else {
            set_local_pdk_root(&self.root, Some(pdk_root))?;
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
        // Also when the folder was reached through a symbolic link to the project.
        crate::project_edit::portable(&self.root, path)
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
        // The layout is always written: detecting it again later may give another
        // answer once files are added to the folder.
        let layout = options
            .layout
            .unwrap_or_else(|| LibraryLayout::detect(&folder));
        let definition = {
            let mut table = InlineTable::new();
            table.insert("path", path.into());
            table.insert("layout", layout.name().into());
            if options.read_only {
                table.insert("read_only", true.into());
            }
            Item::Value(Value::InlineTable(table))
        };
        libraries.insert(name, definition);
        write(&self.file, &doc.to_string()).wrap_err_with(|| {
            if options.create {
                format!(
                    "library '{name}' is not defined; its folder {} was made and is left there",
                    folder.display()
                )
            } else {
                format!("library '{name}' is not defined")
            }
        })?;
        self.reload()
    }

    /// Remove a library definition from the file that holds it: the project file, or
    /// the included file that defines the library. The folder is left untouched.
    pub fn remove_library(&mut self, name: &str) -> Result<()> {
        let library = self
            .library(name)
            .ok_or_else(|| eyre!("no library '{name}'"))?;
        let file = library.defined_in.clone();
        // A library the PDK brings is not a definition to remove.
        self.editable(name)?;
        let mut doc = read(&file)?;
        let removed = doc
            .get_mut("libraries")
            .and_then(Item::as_table_like_mut)
            .and_then(|libraries| libraries.remove(name));
        if removed.is_none() {
            bail!("library '{name}' is not in {}", file.display());
        }
        write(&file, &doc.to_string())?;
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
                        Ok((library, warnings)) => {
                            self.libraries.push(library);
                            self.warnings.extend(warnings);
                        }
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

    fn define(
        &self,
        name: &str,
        item: &Item,
        file: &Path,
        dir: &Path,
    ) -> Result<(Library, Vec<String>), String> {
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
        let mut library = new_library(name, Path::new(path), file);
        library.layout = layout.unwrap_or_default();
        library.read_only = read_only;
        let mut warnings = Vec::new();
        // A library that cannot be resolved stays listed, with its error.
        match self.resolve(path, dir) {
            Ok(folder) => {
                library.layout = layout.unwrap_or_else(|| LibraryLayout::detect(&folder));
                library.path = folder;
                if library.layout == LibraryLayout::Ref
                    && self.tech_in_use
                    && let Some(tech) = self.tech.clone()
                {
                    warnings = self.apply_library_files(&mut library, &tech);
                }
                library.scan();
            }
            Err(e) => library.error = Some(e),
        }
        Ok((library, warnings))
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

/// A library not yet read from disk. The one place that fills in every field.
fn new_library(name: &str, path: &Path, defined_in: &Path) -> Library {
    Library {
        name: name.to_string(),
        path: path.to_path_buf(),
        layout: LibraryLayout::default(),
        read_only: false,
        description: None,
        defined_in: defined_in.to_path_buf(),
        error: None,
        cells: Vec::new(),
        preferred: Default::default(),
    }
}

/// Set a top-level text key, keeping the comment after an old value.
fn set_key(doc: &mut DocumentMut, key: &str, text: &str) {
    match doc.get_mut(key) {
        Some(item) if item.is_value() => {
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

/// Do two paths name the same folder? (Symbolic links and `..` resolved when both
/// exist; compared as written otherwise.)
fn same_folder(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// The user's local file (`cdept.local.toml`) as read.
#[derive(Debug, Default)]
struct Local {
    /// `pdk_root` as written.
    pdk_root: Option<String>,
    /// `[env]`, values expanded (`$VAR` of the environment, a leading `~`).
    env: BTreeMap<String, String>,
}

impl Local {
    /// Read the local file of the project in `root`. A file that cannot be read or a
    /// value of the wrong kind becomes a warning; the rest is used.
    fn read(root: &Path, env: fn(&str) -> Option<String>, warnings: &mut Vec<String>) -> Local {
        let file = root.join(LOCAL_FILE);
        let mut local = Local::default();
        if !file.is_file() {
            return local;
        }
        let doc = match read(&file) {
            Ok(doc) => doc,
            Err(e) => {
                warnings.push(format!("{e:#}"));
                return local;
            }
        };
        for (key, item) in doc.iter() {
            match key {
                "pdk_root" => match item.as_str() {
                    Some(text) if text.trim().is_empty() => {}
                    Some(text) => local.pdk_root = Some(text.to_string()),
                    None => warnings.push(format!("{LOCAL_FILE}: `pdk_root` must be a text")),
                },
                "env" => {
                    let Some(table) = item.as_table_like() else {
                        warnings.push(format!("{LOCAL_FILE}: `env` must be a table"));
                        continue;
                    };
                    for (name, item) in table.iter() {
                        let valid = !name.is_empty()
                            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                            && !name.starts_with(|c: char| c.is_ascii_digit());
                        match item
                            .as_str()
                            .filter(|_| valid)
                            .map(|text| expand(text, env))
                        {
                            Some(Ok(text)) => {
                                local.env.insert(name.to_string(), text);
                            }
                            Some(Err(e)) => {
                                warnings.push(format!("{LOCAL_FILE}: [env] {name}: {e}"))
                            }
                            None => warnings.push(format!(
                                "{LOCAL_FILE}: [env] {name} must be a variable name with a text"
                            )),
                        }
                    }
                }
                other => warnings.push(format!(
                    "{LOCAL_FILE}: unknown key `{other}` (known: pdk_root, [env])"
                )),
            }
        }
        local
    }

    /// Does the local file of the project in `root` set `pdk_root`?
    fn has_pdk_root(root: &Path) -> bool {
        read(&root.join(LOCAL_FILE)).is_ok_and(|doc| doc.get("pdk_root").is_some())
    }
}

/// Set (or with `None` remove) `pdk_root` in the local file of the project in `root`,
/// keeping whatever else is in it. A new file starts from a commented template and is
/// added to `.gitignore` when the project is in a git repository.
fn set_local_pdk_root(root: &Path, pdk_root: Option<&str>) -> Result<()> {
    let file = root.join(LOCAL_FILE);
    let is_new = !file.is_file();
    if is_new && pdk_root.is_none() {
        return Ok(());
    }
    let mut doc = if is_new {
        NEW_LOCAL
            .parse::<DocumentMut>()
            .expect("the template is valid TOML")
    } else {
        read(&file)?
    };
    // In a file that still has the template's commented line, the setting takes that
    // line's place (a key added to the document would come before the comments).
    const PLACE: &str = "# pdk_root = ";
    let text_now = doc.to_string();
    let text = match pdk_root {
        Some(text) if doc.get("pdk_root").is_none() && text_now.contains(PLACE) => {
            let line = format!("pdk_root = {}", Value::from(text));
            let mut out = String::new();
            let mut placed = false;
            for old in text_now.lines() {
                if !placed && old.starts_with(PLACE) {
                    out.push_str(&line);
                    placed = true;
                } else {
                    out.push_str(old);
                }
                out.push('\n');
            }
            out
        }
        Some(text) => {
            set_key(&mut doc, "pdk_root", text);
            doc.to_string()
        }
        None => {
            // The line stays as a comment, so that setting it again finds its place.
            let mut out = String::new();
            let mut top_level = true;
            for old in text_now.lines() {
                top_level &= !old.trim_start().starts_with('[');
                let is_key = top_level
                    && old
                        .strip_prefix("pdk_root")
                        .is_some_and(|rest| rest.trim_start().starts_with('='));
                if is_key {
                    out.push_str("# ");
                }
                out.push_str(old);
                out.push('\n');
            }
            out
        }
    };
    write(&file, &text)?;
    if is_new && let Err(e) = ignore_in_git(root, LOCAL_FILE) {
        tracing::warn!("{LOCAL_FILE} could not be added to .gitignore: {e:#}");
    }
    Ok(())
}

/// Add `name` to the `.gitignore` of `root` when `root` is inside a git repository and
/// the file does not list it yet.
fn ignore_in_git(root: &Path, name: &str) -> Result<()> {
    if !root.ancestors().any(|dir| dir.join(".git").exists()) {
        return Ok(());
    }
    let file = root.join(".gitignore");
    let old = match fs::read_to_string(&file) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).wrap_err_with(|| format!("cannot read {}", file.display())),
    };
    let listed = old.lines().any(|line| {
        let line = line.trim();
        line == name || line.strip_prefix('/') == Some(name) || line == "*.local.toml"
    });
    if listed {
        return Ok(());
    }
    let mut text = old;
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str("# CDePT: this user's settings for the project (PDK folder of this machine)\n");
    text.push_str(name);
    text.push('\n');
    fs::write(&file, text).wrap_err_with(|| format!("cannot write {}", file.display()))
}

fn read(file: &Path) -> Result<DocumentMut> {
    let text =
        fs::read_to_string(file).wrap_err_with(|| format!("cannot read {}", file.display()))?;
    text.parse::<DocumentMut>()
        .map_err(|e| eyre!("{}: {}", file.display(), e.to_string().trim_end()))
}

/// Write through a temporary file so a failure cannot truncate the project file.
/// A symbolic link stays one and the permissions are kept: [`crate::project_edit::write_file`].
fn write(file: &Path, text: &str) -> Result<()> {
    crate::project_edit::write_file(file, text)
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
    fn lvs_variables_of_the_project_file() {
        let _env = crate::testutil::hermetic_env();
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join(PROJECT_FILE),
            "name = \"v\"\n[klayout.lvs_variables]\nscale = \"false\"\nlvs_sub = \"VNB\"\nn = 3\nflag = true\nbad = [1]\n",
        )
        .unwrap();
        let project = Project::load(tmp.path()).unwrap();
        let vars = &project.klayout.lvs_variables;
        assert_eq!(vars.get("scale").map(String::as_str), Some("false"));
        assert_eq!(vars.get("lvs_sub").map(String::as_str), Some("VNB"));
        assert_eq!(vars.get("n").map(String::as_str), Some("3"));
        assert_eq!(vars.get("flag").map(String::as_str), Some("true"));
        assert!(!vars.contains_key("bad"));
        assert!(
            project
                .warnings
                .iter()
                .any(|w| w.contains("lvs_variables: bad"))
        );
    }

    #[test]
    fn magic_timeout() {
        let _env = crate::testutil::hermetic_env();
        let tmp = tempfile::tempdir().unwrap();
        let load = |text: &str| {
            std::fs::write(tmp.path().join(PROJECT_FILE), text).unwrap();
            Project::load(tmp.path()).unwrap()
        };
        assert_eq!(load("name = \"p\"\n").magic.timeout, None);
        let d = |s| Some(std::time::Duration::from_secs(s));
        assert_eq!(load("[magic]\ntimeout = \"10m\"\n").magic.timeout, d(600));
        assert_eq!(load("[magic]\ntimeout = 90\n").magic.timeout, d(90));
        assert_eq!(load("[magic]\ntimeout = \"0\"\n").magic.timeout, d(0));
        let bad = load("[magic]\ntimeout = \"soon\"\n");
        assert_eq!(bad.magic.timeout, None);
        assert!(
            bad.warnings
                .iter()
                .any(|w| w.starts_with("[magic] timeout")),
            "{:?}",
            bad.warnings
        );
    }

    #[test]
    fn create_add_remove_keeps_hand_edits() {
        let _env = crate::testutil::hermetic_env();
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

        // A library from an included file is removed from that file.
        let included = prims.defined_in.clone();
        let mut project = project;
        project.remove_library("prims").unwrap();
        assert!(project.library("prims").is_none());
        assert!(!fs::read_to_string(included).unwrap().contains("prims ="));
        assert_eq!(Project::find_root(&root.join("libs/a")), Some(root.clone()));
    }

    #[test]
    fn library_operations_through_a_project() {
        let _env = crate::testutil::hermetic_env();
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
        let _env = crate::testutil::hermetic_env();
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("p");
        let mut project = Project::create_with(&root, Some("p"), Some("ihp"), None).unwrap();
        let text = fs::read_to_string(&project.file).unwrap();
        assert!(text.contains("cdept.local.toml"), "{text}");
        assert!(text.contains("technology = \"ihp\""));
        assert!(!project.local_file().exists());
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
        // The PDK folder is this machine's: it is in the local file, not the project's.
        assert!(!text.contains("pdk_root = \"/opt/x\""), "{text}");
        let local = fs::read_to_string(project.local_file()).unwrap();
        assert!(local.contains("pdk_root = \"/opt/x\""), "{local}");
        assert!(local.contains("# CDePT settings of this user"));
        assert_eq!(project.pdk_root.as_deref(), Some("/opt/x"));
        assert_eq!(project.pdk.root.as_ref().unwrap().source, Source::Local);

        project
            .set_settings(&Settings {
                name: "q".into(),
                ..Default::default()
            })
            .unwrap();
        assert!(project.technology.is_none() && project.pdk_root.is_none());
        assert!(project.description.is_none());
        let local = fs::read_to_string(project.local_file()).unwrap();
        assert!(!local.contains("\npdk_root = "), "{local}");
        assert!(local.contains("\n# pdk_root = \"/opt/x\"\n"), "{local}");
        // Set again: the line is back where it was, below its comment.
        set_local_pdk_root(&project.root, Some("/opt/y")).unwrap();
        let local = fs::read_to_string(project.local_file()).unwrap();
        let at = local
            .find("\npdk_root = \"/opt/y\"\n")
            .expect("the setting");
        assert!(
            local.find("# Where PDKs are installed").unwrap() < at,
            "{local}"
        );
        assert!(
            local.starts_with("# CDePT settings of this user"),
            "{local}"
        );
        assert_eq!(local.matches("pdk_root").count(), 1, "{local}");
    }

    #[test]
    fn the_local_file_wins_and_brings_variables() {
        let _env = crate::testutil::hermetic_env();
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("mine/ihp/libs")).unwrap();
        fs::create_dir_all(root.join("work/shared")).unwrap();
        fs::write(
            root.join(PROJECT_FILE),
            "name = \"p\"\ntechnology = \"ihp\"\npdk_root = \"/team/pdks\"\n[libraries]\n\
             prims = \"$PDK_ROOT/$PDK/libs\"\nshared = \"$WORK/shared\"\n",
        )
        .unwrap();
        // Without the local file: the project's folder, and $WORK is not set.
        let project = Project::load_with(root, |_| None).unwrap();
        assert_eq!(project.pdk.var("PDK_ROOT"), Some("/team/pdks"));
        assert_eq!(project.pdk.root.as_ref().unwrap().source, Source::Project);
        assert!(project.library("shared").unwrap().error.is_some());

        fs::write(
            root.join(LOCAL_FILE),
            "pdk_root = \"mine\"\nshade = 1\n[env]\nWORK = \"~/work\"\n\"bad name\" = \"x\"\nN = 3\n",
        )
        .unwrap();
        fn env(name: &str) -> Option<String> {
            // HOME is the test's folder, given through a variable the test sets up.
            match name {
                "HOME" => std::env::var("CDEPT_TEST_LOCAL_HOME").ok(),
                "PDK_ROOT" => Some("/env/pdks".into()),
                _ => None,
            }
        }
        // SAFETY: only this test reads the variable.
        unsafe { std::env::set_var("CDEPT_TEST_LOCAL_HOME", root) };
        let project = Project::load_with(root, env).unwrap();
        let root = root.canonicalize().unwrap();
        assert_eq!(
            project.pdk.var("PDK_ROOT"),
            Some(root.join("mine").to_str().unwrap())
        );
        assert_eq!(project.pdk.root.as_ref().unwrap().source, Source::Local);
        assert_eq!(project.pdk_root.as_deref(), Some("mine"));
        assert_eq!(project.library("prims").unwrap().error, None);
        assert_eq!(project.library("shared").unwrap().error, None);
        assert_eq!(
            project.var("WORK").as_deref(),
            Some(tmp.path().join("work").to_str().unwrap())
        );
        // shade, "bad name", N: three warnings.
        assert_eq!(project.warnings.len(), 3, "{:?}", project.warnings);
        let tool_env = project.tool_env();
        assert!(tool_env.iter().any(|(name, _)| name == "WORK"));
        assert!(tool_env.contains(&("PDK".into(), "ihp".into())));
        assert!(tool_env.contains(&("PDKPATH".into(), root.join("mine/ihp").into_os_string())));
    }

    #[test]
    fn the_local_file_is_ignored_by_git() {
        let _env = crate::testutil::hermetic_env();
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        fs::create_dir_all(repo.join(".git")).unwrap();
        let root = repo.join("designs/p");
        Project::create_with(&root, None, Some("ihp"), Some("/opt/pdks")).unwrap();
        let ignore = fs::read_to_string(root.join(".gitignore")).unwrap();
        assert!(ignore.lines().any(|l| l == LOCAL_FILE), "{ignore}");
        // Not twice, and an existing file keeps its lines.
        fs::write(root.join(".gitignore"), "/target\n/cdept.local.toml").unwrap();
        fs::remove_file(root.join(LOCAL_FILE)).unwrap();
        set_local_pdk_root(&root, Some("/x")).unwrap();
        assert_eq!(
            fs::read_to_string(root.join(".gitignore")).unwrap(),
            "/target\n/cdept.local.toml"
        );
        // Outside a repository there is nothing to ignore.
        let alone = tmp.path().join("alone");
        Project::create_with(&alone, None, None, Some("/opt/pdks")).unwrap();
        assert!(alone.join(LOCAL_FILE).is_file());
        assert!(!alone.join(".gitignore").exists());
    }

    /// A made-up PDK folder and its snapshot, for the tests below.
    fn snapshot_fixture(pdks: &Path) -> (PdkDir, Tech, Catalog) {
        let pdk = PdkDir::new(pdks, "mypdk");
        let klayout = pdk.tech("klayout");
        fs::create_dir_all(klayout.join("tech")).unwrap();
        for file in ["tech/my.lyt", "tech/my.lyp", "tech/my.drc", "tech/my.lvs"] {
            fs::write(klayout.join(file), "").unwrap();
        }
        let xschem = pdk.tech("xschem");
        fs::create_dir_all(xschem.join("my_pr")).unwrap();
        fs::create_dir_all(xschem.join("my_tests")).unwrap();
        fs::write(xschem.join("xschemrc"), "").unwrap();
        fs::write(xschem.join("my_pr/nmos.sym"), "").unwrap();
        fs::create_dir_all(pdk.tech("magic")).unwrap();
        fs::write(pdk.tech("magic").join("mypdk.magicrc"), "").unwrap();

        let mut tech = Tech::default();
        tech.pdk.name = pdk.name.clone();
        tech.klayout.path = vec![pdk.portable(&klayout)];
        tech.klayout.tech = Some("my".into());
        tech.klayout.dbu = Some(0.005);
        tech.klayout.layer_properties = Some(pdk.portable(&klayout.join("tech/my.lyp")));
        for (decks, file) in [
            (&mut tech.klayout.drc, "tech/my.drc"),
            (&mut tech.klayout.lvs, "tech/my.lvs"),
        ] {
            decks.push(crate::tech::klayout::Deck {
                name: "my".into(),
                file: pdk.portable(&klayout.join(file)),
                ..Default::default()
            });
        }
        tech.xschem.rc = Some(pdk.portable(&xschem.join("xschemrc")));
        for (name, role, enabled) in [("my_pr", "primitives", true), ("my_tests", "tests", false)] {
            tech.xschem.libraries.push(crate::tech::TechLibrary {
                name: name.into(),
                path: pdk.portable(&xschem.join(name)),
                layout: "flat".into(),
                role: role.into(),
                description: Some(format!("{role} of mypdk")),
                enabled,
            });
        }
        tech.magic.rc = Some(pdk.portable(&pdk.tech("magic").join("mypdk.magicrc")));
        tech.magic.tech = Some("mypdk".into());
        tech.netgen.setup = Some(pdk.portable(&pdk.tech("netgen").join("setup.tcl")));
        (pdk, tech, Catalog::default())
    }

    #[test]
    fn a_snapshot_fills_what_the_project_leaves_open() {
        let _env = crate::testutil::hermetic_env();
        let tmp = tempfile::tempdir().unwrap();
        let pdks = tmp.path().join("pdks");
        let (pdk, tech, catalog) = snapshot_fixture(&pdks);
        let root = tmp.path().join("p");
        let project = Project::create_for_pdk(&root, Some("p"), &pdk, &tech, &catalog).unwrap();
        assert!(project.warnings.is_empty(), "{:?}", project.warnings);
        assert!(project.tech_in_use && project.snapshot().is_some());
        assert_eq!(project.technology.as_deref(), Some("mypdk"));
        assert_eq!(project.pdk.root.as_ref().unwrap().source, Source::Local);
        assert_eq!(project.pdk_dir(), Some(pdk.clone()));
        let klayout = pdk.tech("klayout");
        assert_eq!(project.xschem_rc, [pdk.tech("xschem").join("xschemrc")]);
        assert_eq!(project.klayout.path, std::slice::from_ref(&klayout));
        assert_eq!(project.klayout_path(), std::slice::from_ref(&klayout));
        assert_eq!(project.klayout.tech.as_deref(), Some("my"));
        assert_eq!(
            project.klayout.layer_properties,
            Some(klayout.join("tech/my.lyp"))
        );
        assert_eq!(project.klayout.drc, Some(klayout.join("tech/my.drc")));
        assert_eq!(project.klayout.lvs, Some(klayout.join("tech/my.lvs")));
        assert_eq!(
            project.magic_rc(),
            Some(pdk.tech("magic").join("mypdk.magicrc"))
        );
        assert_eq!(project.magic_tech().as_deref(), Some("mypdk"));
        // Named by the snapshot but not on this machine.
        assert_eq!(project.netgen_setup(), None);
        assert_eq!(project.dbu(), 0.005);
        assert_eq!(
            project.template_setup().magic_tech.as_deref(),
            Some("mypdk")
        );
        // The PDK's enabled libraries are the project's, read-only.
        let names: Vec<_> = project.libraries.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["my_pr"]);
        let prims = project.library("my_pr").unwrap();
        assert!(prims.read_only && prims.error.is_none());
        assert_eq!(prims.layout, LibraryLayout::Flat);
        assert_eq!(prims.cells.len(), 1);
        assert_eq!(
            prims.defined_in,
            crate::tech::file::tech_file(&project.root)
        );
        assert_eq!(prims.description.as_deref(), Some("primitives of mypdk"));
        assert!(project.catalog().unwrap().is_some());

        // What the project file sets wins: its rc, its deck, its own library `my_pr`.
        fs::create_dir_all(root.join("own_pr")).unwrap();
        fs::write(root.join("own.rc"), "").unwrap();
        fs::write(root.join("own.drc"), "").unwrap();
        let mut text = fs::read_to_string(&project.file).unwrap();
        text = text.replace("[libraries]\n", "[libraries]\nmy_pr = \"own_pr\"\n");
        text.push_str(
            "[xschem]\nrc = \"own.rc\"\n[klayout]\ntech = \"other\"\ndrc = \"own.drc\"\n",
        );
        fs::write(&project.file, text).unwrap();
        let project = Project::load(&root).unwrap();
        assert!(project.warnings.is_empty(), "{:?}", project.warnings);
        assert_eq!(project.xschem_rc, [project.root.join("own.rc")]);
        assert_eq!(project.klayout.tech.as_deref(), Some("other"));
        assert_eq!(project.klayout.drc, Some(project.root.join("own.drc")));
        assert_eq!(project.klayout.lvs, Some(klayout.join("tech/my.lvs")));
        let prims = project.library("my_pr").unwrap();
        assert_eq!(prims.path, project.root.join("own_pr"));
        assert!(!prims.read_only);
        assert_eq!(project.libraries.len(), 1);
    }

    /// A PDK with a reference library of two Liberty corners, and a snapshot that
    /// names `file` as its Liberty file.
    fn ref_snapshot(pdks: &Path, liberty: &[&str]) -> (PdkDir, Tech) {
        let (pdk, mut tech, _) = snapshot_fixture(pdks);
        let lib = pdk.refs().join("cells");
        fs::create_dir_all(lib.join("lib")).unwrap();
        for corner in ["tt", "ff", "ss"] {
            fs::write(
                lib.join(format!("lib/cells_{corner}.lib")),
                "library (cells) {\n  cell (inv) {\n    area : 1.0 ;\n  }\n}\n",
            )
            .unwrap();
        }
        tech.refs.libraries.push(crate::tech::refs::RefLibrary {
            name: "cells".into(),
            path: pdk.portable(&lib),
            layout: "ref".into(),
            enabled: true,
            files: Some(crate::tech::refs::LibraryFiles {
                source: "librelane config".into(),
                liberty: liberty
                    .iter()
                    .map(|name| crate::tech::refs::LibertyRef {
                        corner: (*name).into(),
                        file: pdk.portable(&lib.join(format!("lib/{name}"))),
                    })
                    .collect(),
                ..Default::default()
            }),
            ..Default::default()
        });
        (pdk, tech)
    }

    fn main_liberty(project: &Project) -> Option<String> {
        let lib = project.library("cells")?;
        let entry = lib
            .cell("inv")?
            .entries
            .iter()
            .find(|e| e.kind == crate::ViewKind::Liberty)?;
        entry
            .main_file
            .as_ref()
            .map(|f| f.file_name().unwrap().to_string_lossy().into_owned())
    }

    #[test]
    fn the_files_of_the_snapshot_pick_the_main_file_and_follow_hand_edits() {
        let _env = crate::testutil::hermetic_env();
        let tmp = tempfile::tempdir().unwrap();
        let (pdk, mut tech) = ref_snapshot(&tmp.path().join("pdks"), &["cells_ss.lib"]);
        let root = tmp.path().join("p");
        let mut project =
            Project::create_for_pdk(&root, None, &pdk, &tech, &Catalog::default()).unwrap();
        assert!(project.warnings.is_empty(), "{:?}", project.warnings);
        // The snapshot names the slow corner: that one, not the typical one.
        assert_eq!(main_liberty(&project).as_deref(), Some("cells_ss.lib"));

        // Edited by hand: another corner, written back and read again.
        let file = &mut tech.refs.libraries[0].files.as_mut().unwrap().liberty[0];
        file.file = pdk.portable(&pdk.refs().join("cells/lib/cells_ff.lib"));
        crate::tech::file::save(&root, &tech, &Catalog::default()).unwrap();
        project.reload().unwrap();
        assert_eq!(main_liberty(&project).as_deref(), Some("cells_ff.lib"));

        // Without `files` the folder scan decides, as before: the typical corner.
        tech.refs.libraries[0].files = None;
        crate::tech::file::save(&root, &tech, &Catalog::default()).unwrap();
        project.reload().unwrap();
        assert_eq!(main_liberty(&project).as_deref(), Some("cells_tt.lib"));
        assert!(project.warnings.is_empty(), "{:?}", project.warnings);
    }

    #[test]
    fn a_file_the_snapshot_names_that_is_gone_is_a_warning() {
        let tmp = tempfile::tempdir().unwrap();
        let (pdk, mut tech) = ref_snapshot(&tmp.path().join("pdks"), &["cells_gone.lib"]);
        tech.refs.libraries[0].files.as_mut().unwrap().gds =
            vec![pdk.portable(&pdk.refs().join("cells/gds/cells.gds"))];
        let root = tmp.path().join("p");
        let project =
            Project::create_for_pdk(&root, None, &pdk, &tech, &Catalog::default()).unwrap();
        let warnings = project.warnings.join("\n");
        assert!(
            warnings.contains("library cells names liberty"),
            "{warnings}"
        );
        assert!(
            warnings.contains("cells_gone.lib, which does not exist"),
            "{warnings}"
        );
        assert!(warnings.contains("names gds"), "{warnings}");
        assert!(warnings.contains("cdept tech refresh"), "{warnings}");
        // The folder scan still gives the view.
        assert_eq!(main_liberty(&project).as_deref(), Some("cells_tt.lib"));
    }

    #[test]
    fn a_changed_pdk_configuration_is_a_warning() {
        let _env = crate::testutil::hermetic_env();
        let tmp = tempfile::tempdir().unwrap();
        let (pdk, mut tech) = ref_snapshot(&tmp.path().join("pdks"), &["cells_tt.lib"]);
        let config = pdk.tech("librelane").join("config.tcl");
        fs::create_dir_all(config.parent().unwrap()).unwrap();
        fs::write(&config, "set ::env(A) 1\n").unwrap();
        tech.refs.digital = Some(crate::tech::refs::Digital {
            flow: "librelane".into(),
            config: pdk.portable(&config),
            ..Default::default()
        });
        let digital = tech.refs.digital.as_mut().unwrap();
        digital.config_hash = crate::tech::refs::config_hash(digital, |p| p.resolve_in(&pdk).ok());
        let root = tmp.path().join("p");
        let mut project =
            Project::create_for_pdk(&root, None, &pdk, &tech, &Catalog::default()).unwrap();
        assert!(project.warnings.is_empty(), "{:?}", project.warnings);
        fs::write(&config, "set ::env(A) 2\n").unwrap();
        project.reload().unwrap();
        let warnings = project.warnings.join("\n");
        assert!(
            warnings.contains("configuration changed since the scan"),
            "{warnings}"
        );
    }

    #[test]
    fn saving_a_snapshot_over_one_keeps_a_backup() {
        let tmp = tempfile::tempdir().unwrap();
        let (pdk, tech) = ref_snapshot(&tmp.path().join("pdks"), &["cells_tt.lib"]);
        let root = tmp.path().join("p");
        Project::create_for_pdk(&root, None, &pdk, &tech, &Catalog::default()).unwrap();
        assert!(!root.join(crate::tech::file::BACKUP_FILE).exists());
        let first = fs::read_to_string(root.join("tech.toml")).unwrap();
        let mut other = tech.clone();
        other.klayout.tech = Some("other".into());
        crate::tech::file::save(&root, &other, &Catalog::default()).unwrap();
        let backup = fs::read_to_string(root.join(crate::tech::file::BACKUP_FILE)).unwrap();
        assert_eq!(backup, first);
        assert!(
            fs::read_to_string(root.join("tech.toml"))
                .unwrap()
                .contains("other")
        );
    }

    #[test]
    fn a_snapshot_for_another_pdk_or_machine_is_reported() {
        let _env = crate::testutil::hermetic_env();
        let tmp = tempfile::tempdir().unwrap();
        let pdks = tmp.path().join("pdks");
        let (pdk, mut tech, catalog) = snapshot_fixture(&pdks);
        // The PDK has a symbol library and a reference library of one name.
        tech.xschem.libraries.push(crate::tech::TechLibrary {
            name: "my_pr".into(),
            path: pdk.portable(&pdk.refs().join("my_pr")),
            layout: "flat".into(),
            role: "primitives".into(),
            description: None,
            enabled: true,
        });
        // And one whose symbols are a folder inside its reference library: the outer
        // folder is the library, whichever comes first.
        fs::create_dir_all(pdk.refs().join("my_cells/sym/xschem")).unwrap();
        fs::create_dir_all(pdk.refs().join("my_io/sym/xschem")).unwrap();
        for (name, path) in [
            ("my_cells", "my_cells/sym/xschem"),
            ("my_cells", "my_cells"),
            ("my_io", "my_io"),
            ("my_io", "my_io/sym/xschem"),
        ] {
            tech.xschem.libraries.push(crate::tech::TechLibrary {
                name: name.into(),
                path: pdk.portable(&pdk.refs().join(path)),
                layout: "flat".into(),
                role: "stdcells".into(),
                description: None,
                enabled: true,
            });
        }
        let (_, left_out) = tech.project_libraries();
        assert_eq!(left_out.len(), 1);
        assert_eq!(left_out[0].1.path, pdk.portable(&pdk.refs().join("my_pr")));
        let root = tmp.path().join("p");
        let mut project = Project::create_for_pdk(&root, None, &pdk, &tech, &catalog).unwrap();
        // Told by the scan (its notes), not at every load.
        assert!(project.warnings.is_empty(), "{:?}", project.warnings);
        let names: Vec<_> = project.libraries.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["my_pr", "my_cells", "my_io"]);
        assert_eq!(
            project.library("my_cells").unwrap().path,
            pdk.refs().join("my_cells")
        );
        assert_eq!(
            project.library("my_io").unwrap().path,
            pdk.refs().join("my_io")
        );
        assert_eq!(
            project.library("my_pr").unwrap().path,
            pdk.tech("xschem").join("my_pr")
        );

        // Another PDK chosen: the snapshot is kept but not used.
        project
            .set_settings(&Settings {
                name: "p".into(),
                technology: "otherpdk".into(),
                pdk_root: pdks.to_string_lossy().into_owned(),
                ..Default::default()
            })
            .unwrap();
        assert!(project.tech.is_some() && !project.tech_in_use);
        assert!(project.snapshot().is_none() && project.libraries.is_empty());
        assert!(project.xschem_rc.is_empty() && project.klayout.tech.is_none());
        assert!(
            project
                .warnings
                .iter()
                .any(|w| w.contains("was made for the PDK \"mypdk\""))
        );
        assert!(project.snapshot_stale().unwrap().contains("otherpdk"));

        // The project file names no PDK: the snapshot's is used.
        let text = fs::read_to_string(&project.file).unwrap();
        fs::write(&project.file, text.replace("technology = \"otherpdk\"", "")).unwrap();
        let project = Project::load_with(&root, |_| None).unwrap();
        assert_eq!(project.pdk.var("PDK"), Some("mypdk"));
        assert_eq!(project.pdk.name.as_ref().unwrap().source, Source::Snapshot);
        assert!(project.tech_in_use);

        // The PDK folder of another machine: one warning says what to set.
        fs::write(project.local_file(), "pdk_root = \"/nowhere/pdks\"\n").unwrap();
        let project = Project::load_with(&root, |_| None).unwrap();
        assert!(
            project
                .warnings
                .iter()
                .any(|w| w.contains("is not in /nowhere/pdks")),
            "{:?}",
            project.warnings
        );
        assert!(project.xschem_rc.is_empty());
        assert!(project.library("my_pr").unwrap().error.is_some());
        // No PDK folder at all.
        fs::remove_file(project.local_file()).unwrap();
        let project = Project::load_with(&root, |_| None).unwrap();
        assert!(
            project
                .warnings
                .iter()
                .any(|w| w.contains("$PDK_ROOT, which is not set")),
            "{:?}",
            project.warnings
        );
        // A snapshot that cannot be read is a warning, not a failure.
        fs::write(crate::tech::file::tech_file(&root), "schema = 3\n").unwrap();
        let project = Project::load_with(&root, |_| None).unwrap();
        assert!(project.tech.is_none());
        assert_eq!(project.warnings.len(), 1, "{:?}", project.warnings);
    }

    #[test]
    fn klayout_settings_and_the_pdk_default() {
        let _env = crate::testutil::hermetic_env();
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
        let _env = crate::testutil::hermetic_env();
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
        let _env = crate::testutil::hermetic_env();
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join(PROJECT_FILE), "name = [1\n").unwrap();
        assert!(Project::load(tmp.path()).is_err());
        fs::write(tmp.path().join(PROJECT_FILE), "libraries = 3\n").unwrap();
        assert!(Project::load(tmp.path()).is_err());
        assert!(Project::load(&tmp.path().join("nope")).is_err());
    }
}
