//! Simulation models section of the technology snapshot: `libs.tech/ngspice`,
//! `libs.tech/xyce`, …: model files, their corner sections, start-up files.
//!
//! A PDK serves one or more *model sets*: a folder of model files for one simulator
//! (`libs.tech/ngspice`, `libs.tech/xyce`; sky130 also has `libs.tech/combined`, a
//! second set for ngspice). A set has library files with `.lib <section>` blocks, one
//! per corner, and the files those include.
//!
//! `[models]` in `tech.toml` ([`ModelsTech`]) says, per set, where the files are, what
//! the simulator has to read at start-up, and the named *corner sets* a designer picks
//! from (`tt`, `ff`, `ss`, `sf`, `fs`, `mc`, `tt_mm`, …). A corner set is a list of
//! `"<file> <section>"` entries (what follows `.lib` in a testbench), `"<file>"` alone
//! being a plain `.include`; a file is relative to the set's `dir`:
//!
//! ```toml
//! [models]
//! default = "ngspice"
//! corner = "tt"
//!
//! [models.ngspice]
//! simulator = "ngspice"
//! dir = "$PDK_ROOT/ihp-sg13g2/libs.tech/ngspice/models"
//! libraries = ["cornerMOSlv.lib", "cornerMOShv.lib", "cornerRES.lib"]
//! init = ["$PDK_ROOT/ihp-sg13g2/libs.tech/ngspice/.spiceinit"]
//!
//! [models.ngspice.corners]
//! tt = ["cornerMOSlv.lib mos_tt", "cornerMOShv.lib mos_tt", "cornerRES.lib res_typ"]
//! ff = ["cornerMOSlv.lib mos_ff", "cornerMOShv.lib mos_ff", "cornerRES.lib res_bcs"]
//! ```
//!
//! The catalog ([`ModelsCatalog`]) has every file and section with its class and
//! corner, and every subcircuit and model the files define, with the library that
//! brings it. [`ModelsTech::lines`] gives the lines for a testbench;
//! [`ModelsCatalog::libs_for`] only those a given list of models needs.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::pdk::{ScanContext, Scanned, TechPath};
use super::xschem::Portable;

/// The order corner sets are offered in.
pub const CORNER_ORDER: [&str; 5] = ["tt", "ff", "ss", "sf", "fs"];

/// `[models]` in `tech.toml`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelsTech {
    /// The model set a new testbench uses: the one the PDK's `xschemrc` points at,
    /// else `ngspice`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    /// The corner set a new testbench uses.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub corner: Option<String>,
    /// The folder of Verilog-A sources (compiled with OpenVAF into OSDI).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verilog_a: Option<TechPath>,
    /// The model sets by name: the folder under `libs.tech` (`ngspice`, `xyce`,
    /// `combined`).
    #[serde(flatten, deserialize_with = "sets_only")]
    pub sets: BTreeMap<String, ModelSet>,
}

/// The model sets among the keys of `[models]` this version has no field for: the
/// tables that read as a [`ModelSet`]. Anything else (a key added by hand, a setting
/// of a newer CDePT) is left out instead of failing the whole file;
/// `tech::file::parse_tech` says which keys were ignored.
fn sets_only<'de, D>(deserializer: D) -> Result<BTreeMap<String, ModelSet>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let all = BTreeMap::<String, toml::Value>::deserialize(deserializer)?;
    Ok(all
        .into_iter()
        .filter(|(_, value)| value.is_table())
        .filter_map(|(name, value)| Some((name, value.try_into::<ModelSet>().ok()?)))
        .collect())
}

/// One model set: `[models.<name>]`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelSet {
    /// The simulator the files are written for: `ngspice`, `xyce`.
    pub simulator: String,
    /// The folder the library files are in. Entries of `libraries`, `includes` and
    /// `corners` are relative to it.
    pub dir: TechPath,
    /// The files with corner sections (`.lib <section>` … `.endl`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub libraries: Vec<String>,
    /// Files without sections that no library brings in and that define devices: a
    /// testbench using one of those devices includes the file itself.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub includes: Vec<String>,
    /// Start-up files the simulator must read (`.spiceinit`): copy or source them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub init: Vec<TechPath>,
    /// The settings in the start-up files (`set ngbehavior=hsa`, `option klu`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    /// Folders the start-up files put on the simulator's search path, so that
    /// `.lib cornerMOSlv.lib mos_tt` works without a folder.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub search_path: Vec<TechPath>,
    /// Compiled Verilog-A models (OSDI) the simulator loads.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub osdi: Vec<TechPath>,
    /// Simulator plugins (Xyce `.so`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub plugins: Vec<TechPath>,
    /// Nominal temperature of the models in °C, when stated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tnom: Option<f64>,
    /// Corner sets: name → `"<file> <section>"` entries (`"<file>"`: an include).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub corners: BTreeMap<String, Vec<String>>,
}

/// One line of a testbench: `.lib <file> <section>`, or `.include <file>` when the
/// section is empty.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LibRef {
    pub file: TechPath,
    pub section: String,
}

impl LibRef {
    /// The SPICE line. `var` gives the value of a variable (`PDK_ROOT`, …).
    pub fn line(&self, var: impl Fn(&str) -> Option<String>) -> Result<String, String> {
        let path = self.file.resolve(var)?;
        let path = path.to_string_lossy();
        let path = if path.contains(char::is_whitespace) {
            format!("\"{path}\"")
        } else {
            path.into_owned()
        };
        Ok(if self.section.is_empty() {
            format!(".include {path}")
        } else {
            format!(".lib {path} {}", self.section)
        })
    }
}

impl ModelSet {
    /// An entry of `corners`, `libraries` or `includes` as a file and a section.
    pub fn entry(&self, entry: &str) -> LibRef {
        let mut parts = entry.split_whitespace();
        let file = parts.next().unwrap_or_default();
        let section = parts.next().unwrap_or_default().to_string();
        LibRef {
            file: self.file(file),
            section,
        }
    }

    /// A file of the set: relative names are below `dir`.
    pub fn file(&self, name: &str) -> TechPath {
        if name.starts_with(['/', '$', '~']) {
            TechPath(name.to_string())
        } else {
            TechPath(format!(
                "{}/{}",
                self.dir.as_str().trim_end_matches('/'),
                name
            ))
        }
    }

    /// The names of the corner sets, the usual ones first (`tt ff ss sf fs`), then the
    /// rest by name.
    pub fn corner_names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = CORNER_ORDER
            .iter()
            .copied()
            .filter(|c| self.corners.contains_key(*c))
            .collect();
        for name in self.corners.keys() {
            if !names.contains(&name.as_str()) {
                names.push(name);
            }
        }
        names
    }

    /// The libraries of a corner set.
    pub fn corner(&self, corner: &str) -> Option<Vec<LibRef>> {
        let entries = self.corners.get(corner)?;
        Some(entries.iter().map(|e| self.entry(e)).collect())
    }
}

impl ModelsTech {
    /// A model set by its name (`ngspice`, `combined`, `xyce`) or by its simulator: for
    /// `ngspice` the default set when it is one for ngspice, else the set of that
    /// name, else the first for that simulator.
    pub fn set(&self, name: &str) -> Option<(&str, &ModelSet)> {
        if let Some(default) = &self.default
            && let Some((key, set)) = self.sets.get_key_value(default)
            && set.simulator == name
        {
            return Some((key, set));
        }
        if let Some((key, set)) = self.sets.get_key_value(name) {
            return Some((key, set));
        }
        self.sets
            .iter()
            .find(|(_, set)| set.simulator == name)
            .map(|(key, set)| (key.as_str(), set))
    }

    /// The simulators that have a model set.
    pub fn simulators(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for set in self.sets.values() {
            if !out.contains(&set.simulator.as_str()) {
                out.push(&set.simulator);
            }
        }
        out
    }

    /// The corner sets of a model set or simulator.
    pub fn corners(&self, set: &str) -> Vec<&str> {
        self.set(set)
            .map(|(_, s)| s.corner_names())
            .unwrap_or_default()
    }

    /// The libraries of a corner set, as (file, section) pairs.
    pub fn corner(&self, set: &str, corner: &str) -> Option<Vec<LibRef>> {
        self.set(set)?.1.corner(corner)
    }

    /// The lines to put in a testbench for a model set (or simulator) and a corner
    /// set: `.lib <file> <section>` and `.include <file>`, with the paths of this
    /// machine. `var` gives the value of a variable (`|n| project.var(n)`).
    pub fn lines(
        &self,
        set: &str,
        corner: &str,
        var: impl Fn(&str) -> Option<String>,
    ) -> Result<Vec<String>, String> {
        let (name, models) = self
            .set(set)
            .ok_or_else(|| format!("the technology has no models for '{set}'"))?;
        let libs = models.corner(corner).ok_or_else(|| {
            format!(
                "the models '{name}' have no corner '{corner}' (there are: {})",
                models.corner_names().join(", ")
            )
        })?;
        libs.iter().map(|lib| lib.line(&var)).collect()
    }
}

/// The models part of the catalog.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelsCatalog {
    /// Per model set (the keys of `[models]`).
    pub sets: BTreeMap<String, ModelSetCatalog>,
    /// The Verilog-A sources.
    pub verilog_a: Vec<VerilogA>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelSetCatalog {
    /// The files of the set's folder (not the files they include elsewhere).
    pub files: Vec<ModelFile>,
    /// Every subcircuit and model the set defines at top level.
    pub definitions: Vec<ModelDef>,
    /// How many files the libraries read in all.
    pub files_read: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelFile {
    /// Relative to the set's `dir`.
    pub name: String,
    /// `library` (has sections), `include` (no sections, defines devices, no library
    /// brings it), `support` (a library includes it), `other`.
    pub role: String,
    /// The device class when all sections agree: `mos_lv`, `mos_hv`, `res`, `cap`,
    /// `bjt`, `diode`, `moscap`, …; `all` for a library covering everything.
    pub class: String,
    pub sections: Vec<ModelSection>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelSection {
    pub name: String,
    pub class: String,
    /// `tt`, `ff`, `ss`, `sf`, `fs` (typ/bcs/wcs read as tt/ff/ss), or empty.
    pub corner: String,
    /// Resistor/capacitor corner of sky130 (`ll`, `hh`, `hl`, `lh`), or empty.
    pub rc: String,
    pub mismatch: bool,
    pub statistical: bool,
    /// What the section reads: `"<file>"` or `"<file> <section>"`.
    pub includes: Vec<String>,
}

/// A subcircuit or model defined by a set.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelDef {
    pub name: String,
    /// `subckt` or `model`.
    pub kind: String,
    /// For a model its type (`nmos`, `d`, `psp103va`, `r3_cmc`).
    pub device: String,
    /// Subcircuit pins in order.
    pub pins: Vec<String>,
    /// Subcircuit parameters with their defaults.
    pub parameters: BTreeMap<String, String>,
    /// The file that defines it; for a file included by itself (`library` empty) the
    /// file to include.
    pub file: TechPath,
    /// The library (relative to the set's `dir`) whose sections bring it; empty: the
    /// file is included by itself.
    pub library: String,
    pub class: String,
}

/// A Verilog-A model of the PDK.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VerilogA {
    pub name: String,
    pub files: Vec<TechPath>,
    /// There is a compiled OSDI file of that name.
    pub compiled: bool,
}

impl ModelSetCatalog {
    /// The definition of a subcircuit or model (names compare without case).
    pub fn definition(&self, name: &str) -> Option<&ModelDef> {
        self.definitions
            .iter()
            .find(|d| d.name.eq_ignore_ascii_case(name))
    }
}

impl ModelsCatalog {
    /// The libraries of a corner: a corner set of `tech`, else a section of that name
    /// in one of the set's libraries (sky130's `sf_ll_mm`, which is no corner set).
    pub fn corner(&self, tech: &ModelsTech, set: &str, corner: &str) -> Option<Vec<LibRef>> {
        let (name, model_set) = tech.set(set)?;
        if let Some(libs) = model_set.corner(corner) {
            return Some(libs);
        }
        let file = self.sets.get(name)?.files.iter().find(|f| {
            f.role == "library"
                && f.sections
                    .iter()
                    .any(|s| s.name.eq_ignore_ascii_case(corner))
        })?;
        Some(vec![LibRef {
            file: model_set.file(&file.name),
            section: corner.to_string(),
        }])
    }

    /// The libraries a testbench needs for the given models (subcircuit or model
    /// names as the netlist has them) in a corner set: only the entries of the corner
    /// set that bring one of them, and the files to include by themselves. `None`
    /// when the set or the corner does not exist. Models nobody defines are returned
    /// in the second list.
    pub fn libs_for(
        &self,
        tech: &ModelsTech,
        set: &str,
        corner: &str,
        models: &[&str],
    ) -> Option<(Vec<LibRef>, Vec<String>)> {
        let (name, model_set) = tech.set(set)?;
        let all = self.corner(tech, set, corner)?;
        let catalog = self.sets.get(name)?;
        let mut wanted: BTreeSet<LibRef> = BTreeSet::new();
        let mut includes: Vec<LibRef> = Vec::new();
        let mut unknown = Vec::new();
        for model in models {
            let Some(def) = catalog.definition(model) else {
                unknown.push(model.to_string());
                continue;
            };
            if def.library.is_empty() {
                let lib = LibRef {
                    file: def.file.clone(),
                    section: String::new(),
                };
                if !includes.contains(&lib) {
                    includes.push(lib);
                }
            } else {
                let file = model_set.file(&def.library);
                wanted.extend(all.iter().filter(|l| l.file == file).cloned());
            }
        }
        let mut out: Vec<LibRef> = all.into_iter().filter(|l| wanted.contains(l)).collect();
        out.extend(includes);
        Some((out, unknown))
    }
}

impl PartialOrd for LibRef {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for LibRef {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (&self.file, &self.section).cmp(&(&other.file, &other.section))
    }
}

// ---------------------------------------------------------------------------
// Section names
// ---------------------------------------------------------------------------

/// What the name of a `.lib` section says.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SectionKind {
    /// The class prefix as written (`mos`, `res`, `hbt`), empty when there is none.
    pub prefix: String,
    pub corner: String,
    pub rc: String,
    pub mismatch: bool,
    pub statistical: bool,
}

/// Read a section name: `mos_tt`, `res_typ_mismatch`, `hbt_typ_stat`, `sf_ll_mm`,
/// `mc`, `typical`, `res_statistical`.
pub fn section_kind(name: &str) -> SectionKind {
    let mut kind = SectionKind::default();
    let mut prefix: Vec<&str> = Vec::new();
    let lower = name.to_ascii_lowercase();
    for token in lower.split('_').filter(|t| !t.is_empty()) {
        match token {
            "tt" | "typ" | "typical" | "nom" | "nominal" => kind.corner = "tt".into(),
            "ff" | "bcs" | "fast" => kind.corner = "ff".into(),
            "ss" | "wcs" | "slow" => kind.corner = "ss".into(),
            "sf" => kind.corner = "sf".into(),
            "fs" => kind.corner = "fs".into(),
            "ll" | "hh" | "hl" | "lh" => kind.rc = token.into(),
            "mm" | "mismatch" => kind.mismatch = true,
            "stat" | "statistical" | "mc" | "montecarlo" => kind.statistical = true,
            other => prefix.push(other),
        }
    }
    kind.prefix = prefix.join("_");
    kind
}

/// The device class of a section: from its prefix, refined by the file name
/// (`cornerMOShv.lib` → `mos_hv`); without a prefix from the file name alone, else
/// `all`.
pub fn class_of(prefix: &str, file: &str) -> String {
    let stem = Path::new(file)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(file)
        .to_ascii_lowercase();
    let stem = stem.split('.').next().unwrap_or_default();
    let stem = stem
        .strip_prefix("corners")
        .or_else(|| stem.strip_prefix("corner"))
        .unwrap_or(stem);
    let voltage = |base: &str| {
        if stem.contains("hv") {
            format!("{base}_hv")
        } else if stem.contains("lv") {
            format!("{base}_lv")
        } else {
            base.to_string()
        }
    };
    match prefix {
        "" => {
            if stem.starts_with("moscap") {
                "moscap".into()
            } else if stem.starts_with("mos") {
                voltage("mos")
            } else if stem.starts_with("res") {
                "res".into()
            } else if stem.starts_with("cap") || stem.starts_with("mim") {
                "cap".into()
            } else if stem.starts_with("hbt") || stem.starts_with("bjt") || stem.starts_with("bip")
            {
                "bjt".into()
            } else if stem.starts_with("dio") {
                "diode".into()
            } else {
                "all".into()
            }
        }
        "mos" | "fet" | "nmos" | "pmos" => voltage("mos"),
        "res" | "resistor" | "resistors" => "res".into(),
        "cap" | "mim" | "mimcap" | "capacitor" | "capacitors" => "cap".into(),
        "hbt" | "bjt" | "bip" | "npn" | "pnp" => "bjt".into(),
        "dio" | "diode" | "diodes" => "diode".into(),
        other => other.to_string(),
    }
}

/// A section of a library, for building the corner sets.
#[derive(Debug, Clone)]
struct LibSection {
    lib: String,
    name: String,
    class: String,
    kind: SectionKind,
}

/// The named corner sets from the sections of the libraries: per library and class
/// one section, the typical one where a class has no section for the corner.
fn corner_sets(sections: &[LibSection]) -> BTreeMap<String, Vec<String>> {
    // Groups in order of appearance: (library, class).
    let mut groups: Vec<(String, String, Vec<&LibSection>)> = Vec::new();
    for section in sections {
        match groups
            .iter_mut()
            .find(|(lib, class, _)| *lib == section.lib && *class == section.class)
        {
            Some((_, _, list)) => list.push(section),
            None => groups.push((section.lib.clone(), section.class.clone(), vec![section])),
        }
    }
    let plain = |s: &LibSection, corner: &str| {
        s.kind.corner == corner && !s.kind.mismatch && !s.kind.statistical && s.kind.rc.is_empty()
    };
    let typical = |list: &[&LibSection]| -> String {
        list.iter()
            .find(|s| plain(s, "tt"))
            .or_else(|| {
                list.iter()
                    .find(|s| !s.kind.mismatch && !s.kind.statistical && s.kind.rc.is_empty())
            })
            .unwrap_or(&list[0])
            .name
            .clone()
    };
    let mut sets: BTreeMap<String, Vec<String>> = BTreeMap::new();
    // One set: per group the first section the tests accept, in order, else typical.
    let mut build = |name: &str, tests: &[&dyn Fn(&LibSection) -> bool]| {
        let entries = groups
            .iter()
            .map(|(lib, _, list)| {
                let section = tests
                    .iter()
                    .find_map(|test| list.iter().find(|s| test(s)).map(|s| s.name.clone()))
                    .unwrap_or_else(|| typical(list));
                format!("{lib} {section}")
            })
            .collect();
        sets.insert(name.to_string(), entries);
    };
    let any = |test: &dyn Fn(&LibSection) -> bool| sections.iter().any(test);

    for corner in CORNER_ORDER {
        if any(&|s| plain(s, corner)) {
            build(corner, &[&|s| plain(s, corner)]);
        }
        let mismatch = |s: &LibSection| {
            s.kind.corner == corner
                && s.kind.mismatch
                && !s.kind.statistical
                && s.kind.rc.is_empty()
        };
        if any(&mismatch) {
            build(&format!("{corner}_mm"), &[&mismatch, &|s| plain(s, corner)]);
        }
    }
    // Resistor/capacitor corners on their own (sky130: ll, hh, hl, lh).
    let mut rcs: Vec<&str> = Vec::new();
    for s in sections {
        if !s.kind.rc.is_empty()
            && s.kind.corner.is_empty()
            && !s.kind.mismatch
            && !s.kind.statistical
            && !rcs.contains(&s.kind.rc.as_str())
        {
            rcs.push(&s.kind.rc);
        }
    }
    for rc in rcs {
        build(
            rc,
            &[&|s| {
                s.kind.rc == rc
                    && s.kind.corner.is_empty()
                    && !s.kind.mismatch
                    && !s.kind.statistical
            }],
        );
    }
    // Monte Carlo: the statistical sections (process variation).
    let stat = |s: &LibSection| {
        s.kind.statistical
            && !s.kind.mismatch
            && s.kind.rc.is_empty()
            && (s.kind.corner.is_empty() || s.kind.corner == "tt")
    };
    if any(&stat) {
        build("mc", &[&stat]);
    }
    // Process variation and mismatch together, when most classes have it.
    let both = |s: &LibSection| s.kind.statistical && s.kind.mismatch && s.kind.rc.is_empty();
    let with_both = groups
        .iter()
        .filter(|(_, _, list)| list.iter().any(|s| both(s)))
        .count();
    if with_both > 0 && with_both * 2 >= groups.len() {
        build("mc_mm", &[&both, &stat]);
    }
    sets
}

// ---------------------------------------------------------------------------
// Reading SPICE model files
// ---------------------------------------------------------------------------

/// An `.include` or `.lib <file> <section>` line.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Include {
    file: String,
    section: Option<String>,
}

impl Include {
    fn text(&self) -> String {
        match &self.section {
            Some(section) => format!("{} {section}", self.file),
            None => self.file.clone(),
        }
    }
}

/// A `.subckt` or `.model` at top level.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct RawDef {
    name: String,
    kind: &'static str,
    device: String,
    pins: Vec<String>,
    parameters: BTreeMap<String, String>,
}

/// The directives of a file, or of one `.lib` section of it.
#[derive(Debug, Clone, Default, PartialEq)]
struct Block {
    includes: Vec<Include>,
    defs: Vec<RawDef>,
    tnom: Option<f64>,
}

/// A model file: what is outside any section, and its sections.
#[derive(Debug, Clone, Default, PartialEq)]
struct Parsed {
    top: Block,
    sections: Vec<(String, Block)>,
}

fn unquote(token: &str) -> &str {
    token.trim().trim_matches(|c| c == '"' || c == '\'').trim()
}

/// `a = 1 b= 2` → `a=1 b=2`, so that `key=value` pairs are single tokens.
fn tighten(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_whitespace() {
            let mut next = chars.peek().copied();
            while next.is_some_and(char::is_whitespace) {
                chars.next();
                next = chars.peek().copied();
            }
            if next == Some('=') || out.ends_with('=') {
                continue;
            }
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    out
}

/// `tnom = 27` somewhere in a line.
fn tnom_of(line: &str) -> Option<f64> {
    let lower = line.to_ascii_lowercase();
    let at = lower.find("tnom")?;
    let rest = lower[at + 4..].trim_start().strip_prefix('=')?;
    let value: String = rest
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e'))
        .collect();
    value.parse().ok()
}

fn block_mut(parsed: &mut Parsed, current: Option<usize>) -> &mut Block {
    match current {
        Some(i) => &mut parsed.sections[i].1,
        None => &mut parsed.top,
    }
}

/// Read the directives of a SPICE model file. Only lines starting with a dot matter
/// (and the continuation lines of a `.subckt`).
fn parse_spice(text: &str) -> Parsed {
    let mut parsed = Parsed::default();
    let mut current: Option<usize> = None;
    let mut depth = 0usize;
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        let line = line.trim_start();
        if !line.starts_with('.') {
            continue;
        }
        let line = line.split(';').next().unwrap_or(line);
        let mut tokens = line.split_whitespace();
        let directive = tokens.next().unwrap_or_default().to_ascii_lowercase();
        match directive.as_str() {
            ".lib" => {
                let first = tokens.next().map(unquote).unwrap_or_default();
                let second = tokens.next().map(unquote);
                match second {
                    Some(section) if !section.is_empty() && !section.starts_with('$') => {
                        block_mut(&mut parsed, current).includes.push(Include {
                            file: first.to_string(),
                            section: Some(section.to_string()),
                        });
                    }
                    _ if first.contains('/') || first.contains('.') => {
                        block_mut(&mut parsed, current).includes.push(Include {
                            file: first.to_string(),
                            section: None,
                        });
                    }
                    _ if !first.is_empty() => {
                        parsed.sections.push((first.to_string(), Block::default()));
                        current = Some(parsed.sections.len() - 1);
                        depth = 0;
                    }
                    _ => {}
                }
            }
            ".endl" => {
                current = None;
                depth = 0;
            }
            ".include" | ".inc" | ".incl" => {
                let file = unquote(line[directive.len()..].trim());
                if !file.is_empty() {
                    block_mut(&mut parsed, current).includes.push(Include {
                        file: file.to_string(),
                        section: None,
                    });
                }
            }
            ".subckt" => {
                if depth == 0 {
                    let mut whole = line[directive.len()..].to_string();
                    while let Some(next) = lines.peek() {
                        let next = next.trim_start();
                        match next.strip_prefix('+') {
                            Some(rest) => {
                                whole.push(' ');
                                whole.push_str(rest.split(';').next().unwrap_or(rest));
                                lines.next();
                            }
                            None if next.is_empty() || next.starts_with('*') => {
                                // A comment between continuation lines.
                                lines.next();
                            }
                            None => break,
                        }
                    }
                    let whole = tighten(&whole);
                    let mut tokens = whole.split_whitespace();
                    if let Some(name) = tokens.next() {
                        let mut def = RawDef {
                            name: name.to_string(),
                            kind: "subckt",
                            ..RawDef::default()
                        };
                        for token in tokens {
                            if token.eq_ignore_ascii_case("params:") {
                                continue;
                            }
                            match token.split_once('=') {
                                Some((key, value)) => {
                                    def.parameters.insert(key.to_string(), value.to_string());
                                }
                                None if def.parameters.is_empty() => {
                                    def.pins.push(token.to_string())
                                }
                                None => {}
                            }
                        }
                        block_mut(&mut parsed, current).defs.push(def);
                    }
                }
                depth += 1;
            }
            ".ends" => depth = depth.saturating_sub(1),
            ".model" => {
                let block = block_mut(&mut parsed, current);
                if block.tnom.is_none() {
                    block.tnom = tnom_of(line);
                }
                if depth == 0
                    && let Some(name) = tokens.next()
                {
                    // Binned models (`name.0`, `name.1`, …) are one model.
                    let name = match name.rsplit_once('.') {
                        Some((base, bin)) if bin.chars().all(|c| c.is_ascii_digit()) => base,
                        _ => name,
                    };
                    let device = tokens
                        .next()
                        .map(|t| t.split('(').next().unwrap_or(t).to_ascii_lowercase())
                        .unwrap_or_default();
                    if block.defs.last().is_none_or(|d| d.name != name) {
                        block.defs.push(RawDef {
                            name: name.to_string(),
                            kind: "model",
                            device,
                            ..RawDef::default()
                        });
                    }
                }
            }
            ".option" | ".options" => {
                let block = block_mut(&mut parsed, current);
                if let Some(tnom) = tnom_of(line) {
                    block.tnom = Some(tnom);
                }
            }
            _ => {}
        }
    }
    parsed
}

/// Files already read, by path.
#[derive(Default)]
struct Reader {
    cache: HashMap<PathBuf, Option<Arc<Parsed>>>,
    bytes: u64,
}

/// No file larger than this is read, and no more than [`MAX_BYTES`] in all.
const MAX_FILE: u64 = 64 << 20;
const MAX_BYTES: u64 = 512 << 20;
const MAX_FILES: usize = 5000;

impl Reader {
    fn read(&mut self, file: &Path) -> Option<Arc<Parsed>> {
        if let Some(known) = self.cache.get(file) {
            return known.clone();
        }
        let parsed = (|| {
            let size = std::fs::metadata(file).ok()?.len();
            if size > MAX_FILE || self.bytes + size > MAX_BYTES || self.cache.len() >= MAX_FILES {
                return None;
            }
            let bytes = std::fs::read(file).ok()?;
            self.bytes += size;
            Some(Arc::new(parse_spice(&String::from_utf8_lossy(&bytes))))
        })();
        self.cache.insert(file.to_path_buf(), parsed.clone());
        parsed
    }
}

/// What a file, or a section of it, brings in through its includes.
#[derive(Default)]
struct Reached {
    files: BTreeSet<PathBuf>,
    defs: Vec<(RawDef, PathBuf)>,
    tnom: Option<f64>,
}

fn reach(reader: &mut Reader, file: &Path, section: Option<&str>, search: &[PathBuf]) -> Reached {
    let mut out = Reached::default();
    let mut seen: BTreeSet<(PathBuf, Option<String>)> = BTreeSet::new();
    let mut queue: VecDeque<(PathBuf, Option<String>)> = VecDeque::new();
    queue.push_back((file.to_path_buf(), section.map(str::to_ascii_lowercase)));
    while let Some((path, section)) = queue.pop_front() {
        if !seen.insert((path.clone(), section.clone())) {
            continue;
        }
        let Some(parsed) = reader.read(&path) else {
            continue;
        };
        out.files.insert(path.clone());
        let block = match &section {
            None => Some(&parsed.top),
            Some(name) => parsed
                .sections
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(name))
                .map(|(_, b)| b),
        };
        let Some(block) = block else { continue };
        if out.tnom.is_none() {
            out.tnom = block.tnom;
        }
        out.defs
            .extend(block.defs.iter().map(|d| (d.clone(), path.clone())));
        for include in &block.includes {
            let name = Path::new(&include.file);
            let target = if name.is_absolute() {
                Some(name.to_path_buf())
            } else {
                path.parent()
                    .map(|dir| dir.join(name))
                    .into_iter()
                    .chain(search.iter().map(|dir| dir.join(name)))
                    .find(|candidate| candidate.is_file())
            };
            if let Some(target) = target {
                let target = normalize(&target);
                queue.push_back((
                    target,
                    include.section.as_ref().map(|s| s.to_ascii_lowercase()),
                ));
            }
        }
    }
    out
}

/// A path without `.` and `..` (symbolic links are left alone).
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// The suffixes of model files (the last suffix of the name).
const MODEL_SUFFIXES: [&str; 12] = [
    "lib", "spice", "ngspice", "xyce", "hspice", "mod", "l", "inc", "cir", "sp", "ckt", "pm3",
];

fn is_model_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| MODEL_SUFFIXES.contains(&e.to_ascii_lowercase().as_str()))
}

fn sorted_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    files.sort();
    files
}

// ---------------------------------------------------------------------------
// Start-up files
// ---------------------------------------------------------------------------

/// What a simulator start-up file (`.spiceinit`) sets.
#[derive(Debug, Clone, Default, PartialEq)]
struct Init {
    options: Vec<String>,
    osdi: Vec<String>,
    search_path: Vec<String>,
    tnom: Option<f64>,
}

fn parse_init(text: &str) -> Init {
    let mut init = Init::default();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('*') || line.starts_with('#') {
            continue;
        }
        let line = line.split(';').next().unwrap_or(line).trim();
        let mut tokens = line.split_whitespace();
        let word = tokens.next().unwrap_or_default().to_ascii_lowercase();
        match word.as_str() {
            "osdi" | "pre_osdi" => {
                if let Some(file) = tokens.next() {
                    init.osdi.push(unquote(file).to_string());
                }
            }
            "setcs" | "set" if line.to_ascii_lowercase().contains("sourcepath") => {
                let inner = line
                    .split_once('(')
                    .and_then(|(_, rest)| rest.rsplit_once(')'))
                    .map(|(inner, _)| inner)
                    .unwrap_or_default();
                for dir in inner.split_whitespace() {
                    let dir = unquote(dir);
                    if !dir.eq_ignore_ascii_case("$sourcepath")
                        && !init.search_path.iter().any(|d| d == dir)
                    {
                        init.search_path.push(dir.to_string());
                    }
                }
            }
            "set" | "option" | "options" | ".option" | ".options" | "setcs" => {
                if let Some(tnom) = tnom_of(line) {
                    init.tnom = Some(tnom);
                }
                init.options
                    .push(line.split_whitespace().collect::<Vec<_>>().join(" "));
            }
            _ => {}
        }
    }
    init
}

/// A path of a start-up file as the snapshot stores it: `$PDK` becomes the PDK's
/// name, the rest is left as written.
fn init_path(text: &str, pdk: &str) -> TechPath {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find('$') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let (name, braces) = match after.strip_prefix('{') {
            Some(inner) => (inner.split('}').next().unwrap_or_default(), 2),
            None => (
                after
                    .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                    .next()
                    .unwrap_or_default(),
                0,
            ),
        };
        if name == "PDK" {
            out.push_str(pdk);
        } else {
            out.push_str(&rest[at..at + 1 + name.len() + braces]);
        }
        rest = &after[name.len() + braces..];
    }
    out.push_str(rest);
    TechPath(out)
}

// ---------------------------------------------------------------------------
// The scan
// ---------------------------------------------------------------------------

/// The folders under `libs.tech` that hold a model set, and the simulator of each.
const SET_FOLDERS: [(&str, &str); 3] = [
    ("ngspice", "ngspice"),
    ("combined", "ngspice"),
    ("xyce", "xyce"),
];

/// The folder with the library files of a set: the folder itself, else `models`
/// below it.
fn library_dir(root: &Path, reader: &mut Reader) -> Option<PathBuf> {
    [root.to_path_buf(), root.join("models")]
        .into_iter()
        .filter(|dir| dir.is_dir())
        .find(|dir| {
            sorted_files(dir)
                .iter()
                .filter(|f| is_model_file(f))
                .any(|f| reader.read(f).is_some_and(|p| !p.sections.is_empty()))
        })
}

fn scan_set(
    simulator: &str,
    root: &Path,
    portable: &Portable,
    pdk_name: &str,
    notes: &mut Vec<String>,
) -> Option<(ModelSet, ModelSetCatalog)> {
    let mut reader = Reader::default();
    let dir = library_dir(root, &mut reader).or_else(|| {
        // No library with sections: a folder of plain model files still counts.
        [root.to_path_buf(), root.join("models")]
            .into_iter()
            .find(|d| d.is_dir() && sorted_files(d).iter().any(|f| is_model_file(f)))
    })?;
    let mut set = ModelSet {
        simulator: simulator.to_string(),
        dir: portable.path(&dir),
        ..ModelSet::default()
    };
    let mut catalog = ModelSetCatalog::default();
    let relative = |file: &Path| {
        file.strip_prefix(&dir)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| file.to_string_lossy().into_owned())
    };

    // Start-up files: next to the libraries or one level up.
    let mut init_dirs = vec![root.to_path_buf()];
    if dir != root {
        init_dirs.push(dir.clone());
    }
    let mut search: Vec<PathBuf> = Vec::new();
    for init_dir in &init_dirs {
        for name in [".spiceinit", "spinit", ".xyceinit"] {
            let file = init_dir.join(name);
            let Ok(text) = std::fs::read_to_string(&file) else {
                continue;
            };
            let init = parse_init(&text);
            set.init.push(portable.path(&file));
            for option in init.options {
                if !set.options.contains(&option) {
                    set.options.push(option);
                }
            }
            for file in init.osdi {
                let path = init_path(&file, pdk_name);
                if !set.osdi.contains(&path) {
                    set.osdi.push(path);
                }
            }
            for folder in init.search_path {
                let path = init_path(&folder, pdk_name);
                if let Ok(local) = path.resolve_in(&portable.pdk) {
                    search.push(local);
                }
                if !set.search_path.contains(&path) {
                    set.search_path.push(path);
                }
            }
            if set.tnom.is_none() {
                set.tnom = init.tnom;
            }
        }
    }
    // Compiled models and plugins lying in the usual folders.
    for file in sorted_files(&root.join("osdi")) {
        if file.extension().is_some_and(|e| e == "osdi") {
            let path = portable.path(&file);
            if !set.osdi.contains(&path) {
                set.osdi.push(path);
            }
        }
    }
    for file in sorted_files(&root.join("plugins")) {
        if file.extension().is_some_and(|e| e == "so") {
            set.plugins.push(portable.path(&file));
        }
    }

    // The files of the folder.
    let files: Vec<PathBuf> = sorted_files(&dir)
        .into_iter()
        .filter(|f| is_model_file(f))
        .collect();
    let libraries: Vec<&PathBuf> = files
        .iter()
        .filter(|f| reader.read(f).is_some_and(|p| !p.sections.is_empty()))
        .collect();

    // What each library brings, section by section.
    let mut reached_all: BTreeSet<PathBuf> = BTreeSet::new();
    let mut reached_by_other: BTreeSet<PathBuf> = BTreeSet::new();
    let mut sections: Vec<LibSection> = Vec::new();
    let mut definitions: Vec<ModelDef> = Vec::new();
    let mut known: BTreeSet<String> = BTreeSet::new();
    let mut tnom = None;
    let mut per_library: Vec<(PathBuf, Vec<ModelSection>)> = Vec::new();
    for library in &libraries {
        let parsed = reader.read(library).expect("read before");
        let name = relative(library);
        let mut listed = Vec::new();
        for (section, block) in &parsed.sections {
            let kind = section_kind(section);
            let class = class_of(&kind.prefix, &name);
            let reached = reach(&mut reader, library, Some(section), &search);
            for file in &reached.files {
                if file != *library {
                    reached_by_other.insert(file.clone());
                }
                reached_all.insert(file.clone());
            }
            if tnom.is_none() {
                tnom = reached.tnom;
            }
            for (def, file) in reached.defs {
                if known.insert(def.name.to_ascii_lowercase()) {
                    definitions.push(ModelDef {
                        name: def.name,
                        kind: def.kind.to_string(),
                        device: def.device,
                        pins: def.pins,
                        parameters: def.parameters,
                        file: portable.path(&file),
                        library: name.clone(),
                        class: class.clone(),
                    });
                }
            }
            listed.push(ModelSection {
                name: section.clone(),
                class: class.clone(),
                corner: kind.corner.clone(),
                rc: kind.rc.clone(),
                mismatch: kind.mismatch,
                statistical: kind.statistical,
                includes: block.includes.iter().map(Include::text).collect(),
            });
            sections.push(LibSection {
                lib: name.clone(),
                name: section.clone(),
                class,
                kind,
            });
        }
        per_library.push(((*library).clone(), listed));
    }
    // A library another library reads is not one a testbench names.
    let top: Vec<String> = libraries
        .iter()
        .filter(|l| !reached_by_other.contains(**l))
        .map(|l| relative(l))
        .collect();
    let sections: Vec<LibSection> = sections
        .into_iter()
        .filter(|s| top.contains(&s.lib))
        .collect();
    set.libraries = top.clone();
    set.corners = corner_sets(&sections);

    // The other files of the folder.
    for file in &files {
        let name = relative(file);
        if let Some((_, listed)) = per_library.iter().find(|(l, _)| l == file) {
            let mut classes: Vec<&str> = listed.iter().map(|s| s.class.as_str()).collect();
            classes.dedup();
            catalog.files.push(ModelFile {
                role: if top.contains(&name) {
                    "library".into()
                } else {
                    "support".into()
                },
                class: match classes.as_slice() {
                    [one] => one.to_string(),
                    _ => "mixed".into(),
                },
                name,
                sections: listed.clone(),
            });
            continue;
        }
        if reached_all.contains(file) {
            catalog.files.push(ModelFile {
                name,
                role: "support".into(),
                ..ModelFile::default()
            });
            continue;
        }
        // Not read by any library: it counts when it defines something by itself.
        let reached = reach(&mut reader, file, None, &search);
        let mut defines = false;
        for (def, _) in reached.defs {
            if known.insert(def.name.to_ascii_lowercase()) {
                defines = true;
                definitions.push(ModelDef {
                    class: class_of("", &name),
                    name: def.name,
                    kind: def.kind.to_string(),
                    device: def.device,
                    pins: def.pins,
                    parameters: def.parameters,
                    // What a testbench includes: this file, whatever it reads.
                    file: portable.path(file),
                    library: String::new(),
                });
            }
        }
        if defines {
            set.includes.push(name.clone());
        }
        reached_all.extend(reached.files);
        catalog.files.push(ModelFile {
            name,
            role: if defines { "include" } else { "other" }.into(),
            ..ModelFile::default()
        });
    }
    if set.tnom.is_none() {
        set.tnom = tnom;
    }
    catalog.files_read = reader.cache.values().filter(|p| p.is_some()).count();
    catalog.definitions = definitions;
    if set.libraries.is_empty() {
        notes.push(format!(
            "models {}: no library file with corner sections (.lib) in {}",
            root.file_name().unwrap_or_default().to_string_lossy(),
            set.dir
        ));
    }
    Some((set, catalog))
}

/// The Verilog-A sources of the PDK: one model per folder (or per file) of
/// `libs.tech/verilog-a`.
fn verilog_a(root: &Path, osdi: &BTreeSet<String>, portable: &Portable) -> Vec<VerilogA> {
    let mut out = Vec::new();
    let mut entries: Vec<PathBuf> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .collect();
    entries.sort();
    let is_va = |p: &Path| p.extension().is_some_and(|e| e == "va" || e == "vams");
    for entry in entries {
        let name = entry
            .file_stem()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let files: Vec<PathBuf> = if entry.is_dir() {
            let mut found = Vec::new();
            let mut stack = vec![entry.clone()];
            while let Some(dir) = stack.pop() {
                for file in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                    let path = file.path();
                    if path.is_dir() {
                        stack.push(path);
                    } else if is_va(&path) {
                        found.push(path);
                    }
                }
            }
            found.sort();
            found
        } else if is_va(&entry) {
            vec![entry.clone()]
        } else {
            continue;
        };
        if files.is_empty() {
            continue;
        }
        let compiled = osdi.contains(&name.to_ascii_lowercase())
            || files.iter().any(|f| {
                f.file_stem()
                    .is_some_and(|s| osdi.contains(&s.to_string_lossy().to_ascii_lowercase()))
            });
        out.push(VerilogA {
            name,
            files: files.iter().map(|f| portable.path(f)).collect(),
            compiled,
        });
    }
    out
}

pub fn scan(ctx: &ScanContext) -> Scanned<ModelsTech, ModelsCatalog> {
    let mut out: Scanned<ModelsTech, ModelsCatalog> = Scanned::default();
    let portable = Portable::new(&ctx.pdk);
    let libs_tech = ctx.pdk.dir().join("libs.tech");
    for (folder, simulator) in SET_FOLDERS {
        let root = libs_tech.join(folder);
        if !root.is_dir() {
            continue;
        }
        if let Some((set, catalog)) =
            scan_set(simulator, &root, &portable, &ctx.pdk.name, &mut out.notes)
        {
            out.section.sets.insert(folder.to_string(), set);
            out.catalog.sets.insert(folder.to_string(), catalog);
        }
    }
    if out.section.sets.is_empty() {
        out.notes.push(format!(
            "models: no simulation models found under {}",
            portable.path(&libs_tech)
        ));
        return out;
    }

    // The default: the set the PDK's xschemrc points its symbols at.
    let rc = super::xschem::read_rc_text(&ctx.pdk);
    let pointed = out.section.sets.iter().find(|(_, set)| {
        set.simulator == "ngspice"
            && rc.variables.iter().any(|(name, value)| {
                name.to_ascii_uppercase().contains("MODEL")
                    && portable.text(value) == set.dir.as_str()
            })
    });
    let default = pointed
        .map(|(name, _)| name.clone())
        .or_else(|| {
            out.section
                .sets
                .contains_key("ngspice")
                .then(|| "ngspice".to_string())
        })
        .or_else(|| out.section.sets.keys().next().cloned());
    if let Some(name) = &default {
        let set = &out.section.sets[name];
        out.section.corner = set
            .corners
            .contains_key("tt")
            .then(|| "tt".to_string())
            .or_else(|| set.corner_names().first().map(|c| c.to_string()));
    }
    out.section.default = default;

    // Verilog-A sources and whether a compiled model is there.
    let va_root = libs_tech.join("verilog-a");
    if va_root.is_dir() {
        let osdi: BTreeSet<String> = out
            .section
            .sets
            .values()
            .flat_map(|s| s.osdi.iter())
            .filter_map(|p| {
                Path::new(p.as_str())
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_ascii_lowercase())
            })
            .collect();
        out.catalog.verilog_a = verilog_a(&va_root, &osdi, &portable);
        if !out.catalog.verilog_a.is_empty() {
            out.section.verilog_a = Some(portable.path(&va_root));
            if osdi.is_empty() {
                out.notes.push(
                    "models: the PDK has Verilog-A sources but no compiled OSDI files: \
                     compile them with OpenVAF before simulating"
                        .into(),
                );
            }
        }
    }
    for (name, set) in &out.section.sets {
        for file in set.osdi.iter() {
            if let Ok(path) = file.resolve_in(&ctx.pdk)
                && !path.is_file()
            {
                out.notes.push(format!(
                    "models {name}: the start-up file loads {file}, which does not exist \
                     (compile the Verilog-A models with OpenVAF)"
                ));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn section_names() {
        let k = section_kind("mos_tt");
        assert_eq!((k.prefix.as_str(), k.corner.as_str()), ("mos", "tt"));
        let k = section_kind("res_typ_mismatch");
        assert_eq!(
            (k.prefix.as_str(), k.corner.as_str(), k.mismatch),
            ("res", "tt", true)
        );
        let k = section_kind("hbt_bcs");
        assert_eq!((k.prefix.as_str(), k.corner.as_str()), ("hbt", "ff"));
        let k = section_kind("res_stat_mismatch");
        assert!(k.statistical && k.mismatch && k.corner.is_empty());
        let k = section_kind("sf_ll_mm");
        assert_eq!(
            (k.corner.as_str(), k.rc.as_str(), k.mismatch),
            ("sf", "ll", true)
        );
        let k = section_kind("mc");
        assert!(k.statistical && k.prefix.is_empty());
        let k = section_kind("moscap_typical");
        assert_eq!((k.prefix.as_str(), k.corner.as_str()), ("moscap", "tt"));
        assert_eq!(class_of("mos", "cornerMOShv.lib"), "mos_hv");
        assert_eq!(class_of("mos", "cornerMOSlv.lib"), "mos_lv");
        assert_eq!(class_of("hbt", "cornerHBT.lib"), "bjt");
        assert_eq!(class_of("", "sky130.lib.spice"), "all");
        assert_eq!(class_of("", "cornerMOSCAP.lib"), "moscap");
        assert_eq!(class_of("dio", "x.lib"), "diode");
    }

    const LIB: &str = r#"* a library
.param global = 1
.LIB mos_tt
  .param a = 1
  .include mod.lib
.ENDL mos_tt
.lib mos_ff
.include "mod.lib"
.lib 'other.lib' fast   ; a nested library
.endl
.subckt outside 1 2
.ends
"#;

    const MOD: &str = r#"
.subckt sg13_lv_nmos d g s b
+ w=0.35u l=0.34u
+ ng = 1 m=1
.model inner psp103va type=1
Nx d g s b inner
.ends sg13_lv_nmos
.subckt dantenna 1 2 l=780n w=780n
.ends
.MODEL darea D (level=3 tnom=27 is=2.3E-19)
.model binned.0 nmos level=54
.model binned.1 nmos level=54
"#;

    #[test]
    fn spice_directives() {
        let lib = parse_spice(LIB);
        assert_eq!(lib.sections.len(), 2);
        assert_eq!(lib.sections[0].0, "mos_tt");
        assert_eq!(lib.sections[0].1.includes[0].text(), "mod.lib");
        assert_eq!(lib.sections[1].1.includes[1].text(), "other.lib fast");
        assert_eq!(lib.top.defs[0].name, "outside");

        let m = parse_spice(MOD);
        let names: Vec<&str> = m.top.defs.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["sg13_lv_nmos", "dantenna", "darea", "binned"]);
        let nmos = &m.top.defs[0];
        assert_eq!(nmos.pins, ["d", "g", "s", "b"]);
        assert_eq!(nmos.parameters["ng"], "1");
        assert_eq!(nmos.parameters["w"], "0.35u");
        assert_eq!(m.top.defs[1].parameters["l"], "780n");
        assert_eq!(m.top.defs[2].device, "d");
        assert_eq!(m.top.tnom, Some(27.0));
    }

    fn sections(list: &[(&str, &str)]) -> Vec<LibSection> {
        list.iter()
            .map(|(lib, name)| {
                let kind = section_kind(name);
                LibSection {
                    lib: lib.to_string(),
                    name: name.to_string(),
                    class: class_of(&kind.prefix, lib),
                    kind,
                }
            })
            .collect()
    }

    #[test]
    fn corner_sets_per_class() {
        let all = sections(&[
            ("cornerMOSlv.lib", "mos_tt"),
            ("cornerMOSlv.lib", "mos_tt_mismatch"),
            ("cornerMOSlv.lib", "mos_tt_stat"),
            ("cornerMOSlv.lib", "mos_ss"),
            ("cornerMOSlv.lib", "mos_ff"),
            ("cornerMOSlv.lib", "mos_sf"),
            ("cornerRES.lib", "res_typ"),
            ("cornerRES.lib", "res_typ_mismatch"),
            ("cornerRES.lib", "res_stat"),
            ("cornerRES.lib", "res_bcs"),
            ("cornerRES.lib", "res_wcs"),
            ("cornerRES.lib", "res_stat_mismatch"),
        ]);
        let sets = corner_sets(&all);
        assert_eq!(
            sets["tt"],
            ["cornerMOSlv.lib mos_tt", "cornerRES.lib res_typ"]
        );
        assert_eq!(
            sets["ff"],
            ["cornerMOSlv.lib mos_ff", "cornerRES.lib res_bcs"]
        );
        assert_eq!(
            sets["ss"],
            ["cornerMOSlv.lib mos_ss", "cornerRES.lib res_wcs"]
        );
        // No sf for resistors: typical.
        assert_eq!(
            sets["sf"],
            ["cornerMOSlv.lib mos_sf", "cornerRES.lib res_typ"]
        );
        assert!(!sets.contains_key("fs"));
        assert_eq!(
            sets["tt_mm"],
            [
                "cornerMOSlv.lib mos_tt_mismatch",
                "cornerRES.lib res_typ_mismatch"
            ]
        );
        assert_eq!(
            sets["mc"],
            ["cornerMOSlv.lib mos_tt_stat", "cornerRES.lib res_stat"]
        );
        assert_eq!(
            sets["mc_mm"],
            [
                "cornerMOSlv.lib mos_tt_stat",
                "cornerRES.lib res_stat_mismatch"
            ]
        );
    }

    #[test]
    fn corner_sets_of_one_library() {
        let all = sections(&[
            ("sky130.lib.spice", "tt"),
            ("sky130.lib.spice", "sf"),
            ("sky130.lib.spice", "ll"),
            ("sky130.lib.spice", "sf_ll"),
            ("sky130.lib.spice", "tt_mm"),
            ("sky130.lib.spice", "mc"),
        ]);
        let sets = corner_sets(&all);
        let names: Vec<&str> = sets.keys().map(String::as_str).collect();
        assert_eq!(names, ["ll", "mc", "sf", "tt", "tt_mm"]);
        assert_eq!(sets["mc"], ["sky130.lib.spice mc"]);
    }

    #[test]
    fn start_up_file() {
        let init = parse_init(
            "* comment\nset ngbehavior=hsa   ; compat\noption  klu\n*option tnom=28\n\
             setcs sourcepath = (  $sourcepath $PDK_ROOT/$PDK/libs.tech/ngspice/models )\n\
             osdi  '$PDK_ROOT/$PDK/libs.tech/ngspice/osdi/psp103.osdi'\n",
        );
        assert_eq!(init.options, ["set ngbehavior=hsa", "option klu"]);
        assert_eq!(
            init.search_path,
            ["$PDK_ROOT/$PDK/libs.tech/ngspice/models"]
        );
        assert_eq!(
            init_path(&init.osdi[0], "ihp-sg13g2").as_str(),
            "$PDK_ROOT/ihp-sg13g2/libs.tech/ngspice/osdi/psp103.osdi"
        );
        assert_eq!(
            init_path("${PDK}/x/$PDK_ROOT", "a").as_str(),
            "a/x/$PDK_ROOT"
        );
    }

    #[test]
    fn a_made_up_pdk() {
        let tmp = tempfile::tempdir().unwrap();
        let pdk = super::super::PdkDir::new(tmp.path(), "toy");
        let models = pdk.tech("ngspice").join("models");
        std::fs::create_dir_all(&models).unwrap();
        std::fs::write(models.join("cornerMOSlv.lib"), LIB).unwrap();
        std::fs::write(models.join("mod.lib"), MOD).unwrap();
        std::fs::write(models.join("esd.lib"), ".subckt clamp a b\n.ends\n").unwrap();
        std::fs::write(models.join("notes.lib"), "* nothing\n").unwrap();
        std::fs::write(
            pdk.tech("ngspice").join(".spiceinit"),
            "set ngbehavior=hsa\nosdi '$PDK_ROOT/$PDK/libs.tech/ngspice/osdi/psp103.osdi'\n",
        )
        .unwrap();
        let scanned = scan(&ScanContext::new(pdk.clone()).files_only());
        let tech = &scanned.section;
        assert_eq!(tech.default.as_deref(), Some("ngspice"));
        assert_eq!(tech.corner.as_deref(), Some("tt"));
        let set = &tech.sets["ngspice"];
        assert_eq!(set.dir.as_str(), "$PDK_ROOT/toy/libs.tech/ngspice/models");
        assert_eq!(set.libraries, ["cornerMOSlv.lib"]);
        assert_eq!(set.includes, ["esd.lib"]);
        assert_eq!(set.corners["ff"], ["cornerMOSlv.lib mos_ff"]);
        assert_eq!(set.tnom, Some(27.0));
        assert_eq!(set.options, ["set ngbehavior=hsa"]);
        // The osdi file is named but missing: a note.
        assert!(
            scanned.notes.iter().any(|n| n.contains("psp103.osdi")),
            "{:?}",
            scanned.notes
        );

        let catalog = &scanned.catalog.sets["ngspice"];
        let nmos = catalog.definition("SG13_LV_NMOS").unwrap();
        assert_eq!(nmos.library, "cornerMOSlv.lib");
        assert_eq!(nmos.class, "mos_lv");
        assert_eq!(catalog.definition("clamp").unwrap().library, "");
        let roles: Vec<(&str, &str)> = catalog
            .files
            .iter()
            .map(|f| (f.name.as_str(), f.role.as_str()))
            .collect();
        assert_eq!(
            roles,
            [
                ("cornerMOSlv.lib", "library"),
                ("esd.lib", "include"),
                ("mod.lib", "support"),
                ("notes.lib", "other")
            ]
        );

        // The lines of a testbench.
        let root = tmp.path().to_string_lossy().into_owned();
        let var = |name: &str| (name == "PDK_ROOT").then(|| root.clone());
        let lines = tech.lines("ngspice", "tt", var).unwrap();
        assert_eq!(
            lines,
            [format!(
                ".lib {root}/toy/libs.tech/ngspice/models/cornerMOSlv.lib mos_tt"
            )]
        );
        assert!(tech.lines("ngspice", "nope", var).is_err());
        // A section that is no corner set, through the catalog.
        assert!(scanned.catalog.corner(tech, "ngspice", "nope").is_none());
        assert_eq!(
            scanned
                .catalog
                .corner(tech, "ngspice", "MOS_FF")
                .unwrap()
                .len(),
            1
        );
        let (libs, unknown) = scanned
            .catalog
            .libs_for(tech, "ngspice", "ff", &["sg13_lv_nmos", "clamp", "ghost"])
            .unwrap();
        assert_eq!(unknown, ["ghost"]);
        assert_eq!(libs.len(), 2);
        assert_eq!(libs[0].section, "mos_ff");
        assert_eq!(
            libs[1].line(var).unwrap(),
            format!(".include {root}/toy/libs.tech/ngspice/models/esd.lib")
        );

        // tech.toml round trip of the section.
        let text = toml::to_string_pretty(tech).unwrap();
        let back: ModelsTech = toml::from_str(&text).unwrap();
        assert_eq!(&back, tech, "{text}");
    }
}
