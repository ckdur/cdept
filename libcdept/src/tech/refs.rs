//! Reference libraries section of the technology snapshot: `libs.ref/<library>/
//! <format>/…` (standard cells, IO, memories, primitives), and the digital flow
//! set-up that goes with them (`libs.tech/librelane` or `openlane`).
//!
//! Each library is read as a CDePT library of the `ref` layout
//! ([`crate::LibraryLayout::Ref`]), so what the snapshot lists is what the project
//! will show. `tech.toml` gets, per library: what it is for, the formats it has, the
//! number of cells, its technology LEF files and its Liberty corners (process,
//! voltage, temperature). The catalog gets the cells with their views and a few
//! facts (LEF class and size, Liberty area), every Liberty file, the layers and sites
//! of the technology LEFs, and every setting of the digital flow.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::pdk::{PdkDir, ScanContext, Scanned, TechLibrary, TechPath};
use crate::container::{self, Cache, Index, Reader};
use crate::{Library, LibraryLayout, ViewKind};

/// `[refs]` in `tech.toml`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RefsTech {
    /// The folder of the reference libraries (`libs.ref`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<TechPath>,
    /// `[[refs.library]]`, sorted by name.
    #[serde(rename = "library", skip_serializing_if = "Vec::is_empty")]
    pub libraries: Vec<RefLibrary>,
    /// `[refs.digital]`: the digital flow set-up of the PDK.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub digital: Option<Digital>,
}

/// One library of `libs.ref`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RefLibrary {
    pub name: String,
    pub path: TechPath,
    /// The [`LibraryLayout`] name: `ref` (or `flat`/`cells` for a folder laid out so).
    pub layout: String,
    /// `primitives`, `stdcells`, `io`, `memory` or `other`.
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Listed in a new project.
    pub enabled: bool,
    /// Number of cells.
    pub cells: usize,
    /// The format folders it has (`gds`, `lef`, `lib`, …).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub formats: Vec<String>,
    /// Technology LEF files (layers, vias, sites; no macros).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub techlef: Vec<TechPath>,
    /// The corner of [`RefLibrary::liberty`] to start with (the typical one).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_corner: Option<String>,
    /// The placement sites its cells use: name → width and height in micron
    /// (`unithd = [0.46, 2.72]`).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub sites: BTreeMap<String, (f64, f64)>,
    /// The corners its Liberty files were characterised at: the corner as the file
    /// names say it (`tt_025C_1v80`, `typ_1p20V_25C`) → the file. A corner with a
    /// file per cell (IO cells, memories) gives the pattern of their names
    /// (`…/lib/*_typ_1p20V_25C.lib`). Process, voltage and temperature of each
    /// corner, and every file, are in the catalog ([`RefLibraryCatalog::corners`]).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub liberty: BTreeMap<String, TechPath>,
    /// The files of the library by format: from the LibreLane/OpenLane configuration
    /// when it names them, else from the folders.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files: Option<LibraryFiles>,
}

/// `[refs.library.files]`: which file is the library's GDS, SPICE netlist, CDL, LEF,
/// Verilog model and Liberty file. A folder of many files (one per cell) is written
/// as its pattern (`$PDK_ROOT/…/lef/*.lef`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LibraryFiles {
    /// `librelane config` (the PDK's `config.tcl` names the files) or `folder scan`
    /// (guessed from the folders; also for a format the configuration does not name,
    /// when the other formats come from it).
    pub source: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub gds: Vec<TechPath>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub spice: Vec<TechPath>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cdl: Vec<TechPath>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub lef: Vec<TechPath>,
    /// The technology LEF (layers, vias, sites).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tech_lef: Vec<TechPath>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub verilog: Vec<TechPath>,
    /// Liberty files by corner: the pattern of corners the configuration gives
    /// (`*_typ_1p20V_25C`), else the corner as the file name says it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub liberty: Vec<LibertyRef>,
    /// The power pins of the cells (`VDD`, `VPWR VPB`), when the configuration says.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub power_pins: Vec<String>,
    /// The ground pins.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ground_pins: Vec<String>,
}

/// A Liberty file and the corner it is for.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LibertyRef {
    pub corner: String,
    pub file: TechPath,
}

impl LibraryFiles {
    /// The files that exist, and the entries that name nothing there as
    /// `(format, as written)`. `resolve` turns an entry into a path on this machine
    /// (`None`: not possible, which counts as missing). A pattern
    /// (`…/lef/*.lef`) stands for the files of that folder with that extension.
    pub fn existing(
        &self,
        resolve: impl Fn(&TechPath) -> Option<PathBuf>,
    ) -> (BTreeSet<PathBuf>, Vec<(&'static str, String)>) {
        let mut found = BTreeSet::new();
        let mut missing = Vec::new();
        let liberty: Vec<TechPath> = self.liberty.iter().map(|l| l.file.clone()).collect();
        let lists: [(&'static str, &[TechPath]); 7] = [
            ("gds", &self.gds),
            ("spice", &self.spice),
            ("cdl", &self.cdl),
            ("lef", &self.lef),
            ("tech_lef", &self.tech_lef),
            ("verilog", &self.verilog),
            ("liberty", &liberty),
        ];
        for (format, list) in lists {
            for entry in list {
                let mut matched = false;
                match resolve(entry) {
                    Some(path)
                        if path
                            .file_name()
                            .is_some_and(|n| n.to_string_lossy().contains('*')) =>
                    {
                        let name = path.file_name().unwrap().to_string_lossy().into_owned();
                        let (head, tail) = name.split_once('*').unwrap_or((&name, ""));
                        let dir = path.parent().unwrap_or(Path::new("."));
                        for item in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                            let file = item.file_name().to_string_lossy().into_owned();
                            if item.path().is_file()
                                && file.starts_with(head)
                                && file.ends_with(tail)
                            {
                                found.insert(item.path());
                                matched = true;
                            }
                        }
                    }
                    Some(path) if path.is_file() => {
                        found.insert(path);
                        matched = true;
                    }
                    _ => {}
                }
                if !matched {
                    missing.push((format, entry.0.clone()));
                }
            }
        }
        (found, missing)
    }

    /// The files of every format, as `(format, lines)`; a Liberty line is
    /// `corner = file`.
    pub fn formats(&self) -> Vec<(&'static str, Vec<String>)> {
        let names = |list: &[TechPath]| list.iter().map(|p| p.0.clone()).collect::<Vec<_>>();
        vec![
            ("gds", names(&self.gds)),
            ("spice", names(&self.spice)),
            ("cdl", names(&self.cdl)),
            ("lef", names(&self.lef)),
            ("tech_lef", names(&self.tech_lef)),
            ("verilog", names(&self.verilog)),
            (
                "liberty",
                self.liberty
                    .iter()
                    .map(|l| format!("{} = {}", l.corner, l.file))
                    .collect(),
            ),
        ]
    }
}

/// One corner of a library's Liberty files (in the catalog).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LibertyCorner {
    /// The corner as the file names say it: `tt_025C_1v80`, `typ_1p20V_25C`.
    pub name: String,
    /// `tt`, `ff`, `ss`, `sf`, `fs`, `typ`, `fast`, `slow`, when the name says.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process: Option<String>,
    /// Volt: from the file's header, else from its name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voltage: Option<f64>,
    /// Every voltage the name says, for cells with several supplies (`1v80_3v30`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub voltages: Vec<f64>,
    /// Degrees Celsius: from the file's header, else from its name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    /// Number of files (one for a library with all its cells in a file).
    pub files: usize,
}

/// `[refs.digital]`: what LibreLane or OpenLane is told about the PDK.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Digital {
    /// `librelane` or `openlane` (the folder under `libs.tech`).
    pub flow: String,
    /// The PDK's `config.tcl`.
    pub config: TechPath,
    /// `STD_CELL_LIBRARY`: the standard cells used when a design says nothing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_library: Option<String>,
    /// `PAD_CELL_LIBRARY`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pad_library: Option<String>,
    /// `DEFAULT_CORNER`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_corner: Option<String>,
    /// `STA_CORNERS`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub corners: Vec<String>,
    /// Parasitic extraction rules (OpenRCX).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub rcx_rules: Vec<TechPath>,
    /// The libraries with a set-up of their own: name → its `config.tcl`.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub libraries: BTreeMap<String, TechPath>,
    /// A fingerprint of those `config.tcl` files as the scan read them. A project
    /// compares it with the files now to say that the PDK's configuration changed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_hash: Option<String>,
}

/// The fingerprint of the digital flow's `config.tcl` files (FNV-1a over names and
/// contents); `None` when one cannot be read.
pub fn config_hash(
    digital: &Digital,
    resolve: impl Fn(&TechPath) -> Option<PathBuf>,
) -> Option<String> {
    let mut hash = 0xcbf29ce484222325u64;
    let mut eat = |bytes: &[u8]| {
        for b in bytes {
            hash ^= u64::from(*b);
            hash = hash.wrapping_mul(0x100000001b3);
        }
    };
    for path in std::iter::once(&digital.config).chain(digital.libraries.values()) {
        eat(path.as_str().as_bytes());
        eat(&std::fs::read(resolve(path)?).ok()?);
    }
    Some(format!("{hash:016x}"))
}

impl RefsTech {
    /// The reference libraries as the project will list them.
    pub fn libraries(&self) -> Vec<TechLibrary> {
        self.libraries
            .iter()
            .map(|lib| TechLibrary {
                name: lib.name.clone(),
                path: lib.path.clone(),
                layout: lib.layout.clone(),
                role: lib.role.clone(),
                description: lib.description.clone(),
                enabled: lib.enabled,
            })
            .collect()
    }

    pub fn library(&self, name: &str) -> Option<&RefLibrary> {
        self.libraries.iter().find(|lib| lib.name == name)
    }
}

/// The reference libraries part of the catalog.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RefsCatalog {
    pub libraries: Vec<RefLibraryCatalog>,
    /// Every `set ::env(NAME) value` of the digital flow's `config.tcl` files:
    /// `""` (the PDK's) or the library name → name → value as written.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub digital: BTreeMap<String, BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RefLibraryCatalog {
    pub name: String,
    pub formats: Vec<RefFormat>,
    pub cells: Vec<RefCell>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub liberty: Vec<LibertyFile>,
    /// The corners of the Liberty files, sorted by name.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub corners: Vec<LibertyCorner>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub techlef: Vec<TechLef>,
}

impl RefLibraryCatalog {
    pub fn corner(&self, name: &str) -> Option<&LibertyCorner> {
        self.corners.iter().find(|corner| corner.name == name)
    }

    /// The Liberty files of a corner.
    pub fn corner_files<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a LibertyFile> {
        self.liberty.iter().filter(move |file| file.corner == name)
    }
}

/// A format folder of a reference library.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RefFormat {
    /// `gds`, `lef`, `sym/xschem`, …
    pub folder: String,
    /// Number of files.
    pub files: usize,
    pub bytes: u64,
    /// The files that hold several cells, with how many.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub collections: Vec<Collection>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Collection {
    pub file: TechPath,
    pub cells: usize,
}

/// A cell of a reference library.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RefCell {
    pub name: String,
    /// Its views as the project lists them (`layout`, `abstract`, `liberty`, …).
    pub views: Vec<String>,
    /// LEF `CLASS`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub class: String,
    /// LEF `SIZE` in micron.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<(f64, f64)>,
    /// Liberty area (typical corner).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub area: Option<f64>,
    /// `ff`, `latch` or `statetable`, from Liberty.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub sequential: String,
}

/// A Liberty file of a reference library.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LibertyFile {
    pub file: TechPath,
    /// The name in `library (…)`.
    pub library: String,
    /// The corner it belongs to ([`LibertyCorner::name`]).
    pub corner: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voltage: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    pub cells: usize,
}

/// A technology LEF of a reference library.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TechLef {
    pub file: TechPath,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database_microns: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manufacturing_grid: Option<f64>,
    pub layers: Vec<container::lef::LefLayer>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sites: Vec<container::lef::LefSite>,
    /// Number of `VIA` definitions.
    pub vias: usize,
}

pub fn scan(ctx: &ScanContext) -> Scanned<RefsTech, RefsCatalog> {
    scan_with(ctx, &Cache::user())
}

/// [`scan`] with the index cache to use.
pub fn scan_with(ctx: &ScanContext, cache: &Cache) -> Scanned<RefsTech, RefsCatalog> {
    let mut out = Scanned::<RefsTech, RefsCatalog>::default();
    let pdk = &ctx.pdk;
    let root = pdk.refs();
    if root.is_dir() {
        out.section.path = Some(pdk.portable(&root));
        let mut folders: Vec<PathBuf> = std::fs::read_dir(&root)
            .into_iter()
            .flatten()
            .flatten()
            .map(|item| item.path())
            .filter(|path| {
                path.is_dir()
                    && !path
                        .file_name()
                        .is_some_and(|name| name.to_string_lossy().starts_with('.'))
            })
            .collect();
        folders.sort();
        // What the digital flow's configuration says about the libraries' files.
        let config = super::librelane::read(pdk);
        out.notes.extend(config.notes.iter().cloned());
        // The libraries side by side: each reads files of its own.
        let scanned: Vec<(RefLibrary, RefLibraryCatalog, Vec<String>)> =
            std::thread::scope(|scope| {
                let config = &config;
                let handles: Vec<_> = folders
                    .iter()
                    .map(|folder| {
                        scope.spawn(move || {
                            library(pdk, folder, cache, config.libraries.get(&name_of(folder)))
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|handle| handle.join().expect("library scan"))
                    .collect()
            });
        for (section, catalog, notes) in scanned {
            out.section.libraries.push(section);
            out.catalog.libraries.push(catalog);
            out.notes.extend(notes);
        }
    } else {
        out.notes.push(format!(
            "refs: no libs.ref folder in {}",
            pdk.dir().display()
        ));
    }
    if let Some((digital, settings)) = digital(pdk) {
        out.section.digital = Some(digital);
        out.catalog.digital = settings;
    }
    out
}

fn name_of(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The files of a folder and of the folders inside it (one level), sorted.
fn files_in(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut read = |dir: &Path, deeper: bool| -> Vec<PathBuf> {
        let mut folders = Vec::new();
        for item in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = item.path();
            if name_of(&path).starts_with('.') {
                continue;
            }
            if path.is_file() {
                files.push(path);
            } else if deeper && path.is_dir() {
                folders.push(path);
            }
        }
        folders
    };
    for folder in read(dir, true) {
        read(&folder, false);
    }
    files.sort();
    files
}

/// What a library is for: by its name, else by the class of its LEF macros.
fn role(name: &str, classes: &BTreeMap<String, usize>) -> &'static str {
    let lower = name.to_ascii_lowercase();
    let has = |word: &str| lower.split(['_', '-']).any(|part| part == word);
    if has("sc") || lower.contains("stdcell") || lower.contains("_sc_") {
        return "stdcells";
    }
    if has("io") || lower.ends_with("io") || lower.contains("_io_") || has("pad") || has("pads") {
        return "io";
    }
    if lower.contains("sram") || lower.contains("ram") || lower.contains("rom") || has("mem") {
        return "memory";
    }
    if has("pr") || lower.contains("primitive") {
        return "primitives";
    }
    let most = classes
        .iter()
        .max_by_key(|(_, count)| **count)
        .map(|(class, _)| class.as_str())
        .unwrap_or_default();
    if most.starts_with("CORE") {
        "stdcells"
    } else if most.starts_with("PAD") {
        "io"
    } else {
        "other"
    }
}

fn description(role: &str, cells: usize) -> String {
    let what = match role {
        "stdcells" => "Standard cells",
        "io" => "IO cells",
        "memory" => "Memory macros",
        "primitives" => "Device primitives (fixed layouts)",
        _ => "Reference library",
    };
    format!("{what} ({cells} cells)")
}

/// The process, voltages and temperature a corner name says: `tt_025C_1v80`,
/// `typ_1p20V_25C`, `fast_1p32V_m40C`, `ff_n40C_1v95_5v50`, `TT_1p8V_25C`.
pub fn parse_corner(name: &str) -> (Option<String>, Vec<f64>, Option<f64>) {
    let mut process = None;
    let mut voltages = Vec::new();
    let mut temperature = None;
    for part in name.split(['_', '-', '.']) {
        let lower = part.to_ascii_lowercase();
        if process.is_none()
            && matches!(
                lower.as_str(),
                "tt" | "ff" | "ss" | "sf" | "fs" | "typ" | "fast" | "slow" | "nom" | "min" | "max"
            )
        {
            process = Some(lower);
            continue;
        }
        // Temperature: digits, an optional sign letter in front, `c` behind.
        if let Some(body) = lower.strip_suffix('c') {
            let (sign, digits) = match body.strip_prefix(['n', 'm']) {
                Some(rest) => (-1.0, rest),
                None => (1.0, body),
            };
            if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
                temperature = digits.parse::<f64>().ok().map(|t| sign * t);
                continue;
            }
        }
        // Voltage: `1v80`, `1p20v`, `1p8v`, `5v50`.
        let body = lower.strip_suffix('v').unwrap_or(&lower);
        let (whole, fraction) = match body.split_once(['v', 'p']) {
            Some(parts) => parts,
            None if body.len() < lower.len() => (body, ""),
            None => continue,
        };
        let digits = |text: &str| text.bytes().all(|b| b.is_ascii_digit());
        if !whole.is_empty()
            && digits(whole)
            && digits(fraction)
            && lower.contains('v')
            && let Ok(volt) = format!(
                "{whole}.{}",
                if fraction.is_empty() { "0" } else { fraction }
            )
            .parse()
        {
            voltages.push(volt);
        }
    }
    (process, voltages, temperature)
}

/// How typical a corner is: the smaller the more.
fn corner_rank(corner: &LibertyCorner) -> (u8, u8, i64) {
    let process = match corner.process.as_deref() {
        Some("tt" | "typ" | "nom") => 0,
        None => 1,
        _ => 2,
    };
    let room = match corner.temperature {
        Some(t) if (20.0..=30.0).contains(&t) => 0,
        _ => 1,
    };
    // Among typical corners at room temperature the lowest voltage is the core one.
    let voltage = corner.voltage.map_or(i64::MAX, |v| (v * 1000.0) as i64);
    (process, room, voltage)
}

/// The corner a Liberty file is for, from its name (`stem`, without `.lib`).
fn corner_of(stem: &str, library: &str, index: &container::liberty::LibertyIndex) -> String {
    // From the first process word (or run of them: `ff_ss`) that a voltage or a
    // temperature follows directly.
    let parts: Vec<&str> = stem.split('_').collect();
    let is_process = |part: &str| {
        matches!(
            part.to_ascii_lowercase().as_str(),
            "tt" | "ff" | "ss" | "sf" | "fs" | "typ" | "fast" | "slow"
        )
    };
    for at in 0..parts.len() {
        if !is_process(parts[at]) || (at > 0 && is_process(parts[at - 1])) {
            continue;
        }
        let next = parts[at..].iter().find(|part| !is_process(part));
        let (_, voltages, temperature) = parse_corner(next.copied().unwrap_or_default());
        if !voltages.is_empty() || temperature.is_some() {
            return parts[at..].join("_");
        }
    }
    let only_cell = match index.cells.as_slice() {
        [cell] => Some(cell.name.as_str()),
        _ => None,
    };
    let rest = [only_cell, Some(library)]
        .into_iter()
        .flatten()
        .find_map(|prefix| stem.strip_prefix(prefix))
        .map(|rest| rest.trim_start_matches(['_', '-']))
        .filter(|rest| !rest.is_empty());
    match rest {
        Some(rest) => rest.to_string(),
        None if !index.library.is_empty() && index.library != stem => index.library.clone(),
        None => stem.to_string(),
    }
}

/// The pattern the names of several files share: their folder, then `*` and the
/// longest common end of their names (`lib/*_typ_1p20V_25C.lib`).
fn pattern(files: &[PathBuf]) -> PathBuf {
    let names: Vec<String> = files.iter().map(|f| name_of(f)).collect();
    let first = names.first().map(String::as_str).unwrap_or_default();
    let mut common = first.len();
    for name in &names[1..] {
        let same = first
            .bytes()
            .rev()
            .zip(name.bytes().rev())
            .take_while(|(a, b)| a == b)
            .count();
        common = common.min(same);
    }
    let mut start = first.len() - common;
    while !first.is_char_boundary(start) {
        start += 1;
    }
    let folder = files
        .first()
        .and_then(|f| f.parent())
        .unwrap_or(Path::new(""));
    folder.join(format!("*{}", &first[start..]))
}

/// One library of `libs.ref`: its part of `tech.toml`, of the catalog, and notes.
fn library(
    pdk: &PdkDir,
    folder: &Path,
    cache: &Cache,
    config: Option<&super::librelane::ConfigFiles>,
) -> (RefLibrary, RefLibraryCatalog, Vec<String>) {
    let name = name_of(folder);
    let mut notes = Vec::new();
    let mut lib = Library {
        name: name.clone(),
        path: folder.to_path_buf(),
        layout: LibraryLayout::detect(folder),
        read_only: true,
        description: None,
        defined_in: PathBuf::new(),
        error: None,
        cells: Vec::new(),
        preferred: config.map(|c| c.files()).unwrap_or_default(),
    };
    lib.scan_with(cache);
    if let Some(error) = &lib.error {
        notes.push(format!("refs: {name}: {error}"));
    }
    if lib.layout != LibraryLayout::Ref {
        notes.push(format!(
            "refs: {name} is not laid out as <library>/<format>/<files>: read as a `{}` library",
            lib.layout.name()
        ));
    }

    // The format folders, the collections in them, the LEF and Liberty files.
    let mut formats = Vec::new();
    let mut lef: Vec<(PathBuf, container::lef::LefIndex)> = Vec::new();
    let mut liberty: Vec<(PathBuf, container::liberty::LibertyIndex)> = Vec::new();
    let mut folders: Vec<PathBuf> = std::fs::read_dir(folder)
        .into_iter()
        .flatten()
        .flatten()
        .map(|item| item.path())
        .filter(|path| path.is_dir() && !name_of(path).starts_with('.'))
        .collect();
    folders.sort();
    for dir in &folders {
        let files = files_in(dir);
        let mut format = RefFormat {
            folder: name_of(dir),
            files: files.len(),
            bytes: files
                .iter()
                .filter_map(|f| f.metadata().ok())
                .map(|m| m.len())
                .sum(),
            collections: Vec::new(),
        };
        let readable: Vec<(&Path, Reader)> = files
            .iter()
            .filter_map(|file| {
                let reader = container::reader_of(file).filter(|r| r.holds_cells())?;
                Some((file.as_path(), reader))
            })
            .collect();
        for ((file, _), index) in readable.iter().zip(container::read_many(&readable, cache)) {
            let Ok(index) = index else { continue };
            let count = index.names().len();
            if count > 1 {
                format.collections.push(Collection {
                    file: pdk.portable(file),
                    cells: count,
                });
            }
            match index {
                Index::Lef(index) => lef.push((file.to_path_buf(), index)),
                Index::Liberty(index) => liberty.push((file.to_path_buf(), index)),
                _ => {}
            }
        }
        formats.push(format);
    }

    // LEF: technology files, sites, macro classes and sizes.
    let mut sites: BTreeMap<String, (f64, f64)> = BTreeMap::new();
    let mut classes: BTreeMap<String, usize> = BTreeMap::new();
    let mut macros: BTreeMap<&str, &container::lef::LefMacro> = BTreeMap::new();
    let mut techlef = Vec::new();
    for (file, index) in &lef {
        for site in &index.sites {
            if let Some(size) = site.size {
                sites.entry(site.name.clone()).or_insert(size);
            }
        }
        for cell in &index.macros {
            *classes.entry(cell.class.clone()).or_default() += 1;
            macros.entry(cell.name.as_str()).or_insert(cell);
        }
        if index.is_technology() {
            techlef.push(TechLef {
                file: pdk.portable(file),
                database_microns: index.database_microns,
                manufacturing_grid: index.manufacturing_grid,
                layers: index.layers.clone(),
                sites: index.sites.clone(),
                vias: index.vias.len(),
            });
        }
    }
    // Only the sites the macros use are the library's (a technology LEF lists more).
    let used: BTreeSet<&str> = macros.values().map(|m| m.site.as_str()).collect();
    if sites.keys().any(|name| used.contains(name.as_str())) {
        sites.retain(|name, _| used.contains(name.as_str()));
    }

    // Liberty: one corner per distinct name. The name is the end of the file name
    // from its process word on (`…__tt_025C_1v80`), else what the file name adds to
    // the library's or the cell's name.
    let mut corners: BTreeMap<String, (LibertyCorner, Vec<PathBuf>)> = BTreeMap::new();
    let mut liberty_files = Vec::new();
    for (file, index) in &liberty {
        let stem = name_of(file);
        let stem = stem.strip_suffix(".lib").unwrap_or(&stem);
        let corner_name = corner_of(stem, &name, index);
        let (process, voltages, name_temperature) = parse_corner(&corner_name);
        let conditions = index.conditions();
        let voltage = conditions.voltage.or(voltages.first().copied());
        let temperature = conditions.temperature.or(name_temperature);
        liberty_files.push(LibertyFile {
            file: pdk.portable(file),
            library: index.library.clone(),
            corner: corner_name.clone(),
            voltage,
            temperature,
            cells: index.cells.len(),
        });
        let entry = corners.entry(corner_name.clone()).or_insert_with(|| {
            (
                LibertyCorner {
                    name: corner_name,
                    process,
                    voltage,
                    voltages: if voltages.len() > 1 {
                        voltages
                    } else {
                        Vec::new()
                    },
                    temperature,
                    files: 0,
                },
                Vec::new(),
            )
        });
        entry.0.files += 1;
        entry.1.push(file.clone());
    }
    // In `tech.toml`: the file of each corner, or the pattern its files share.
    let liberty_paths: BTreeMap<String, TechPath> = corners
        .values()
        .map(|(corner, files)| {
            let path = match files.as_slice() {
                [file] => pdk.portable(file),
                files => pdk.portable(&pattern(files)),
            };
            (corner.name.clone(), path)
        })
        .collect();
    let liberty_corners: Vec<LibertyCorner> =
        corners.into_values().map(|(corner, _)| corner).collect();
    let default_corner = liberty_corners
        .iter()
        .min_by_key(|corner| corner_rank(corner))
        .map(|corner| corner.name.clone());

    // The cells, with what the LEF and the typical Liberty file say.
    let typical: BTreeMap<&str, &container::liberty::LibertyCell> = {
        let mut files: Vec<&(PathBuf, container::liberty::LibertyIndex)> = liberty.iter().collect();
        // The files of the default corner first, so their values win.
        files.sort_by_key(|(file, _)| {
            let portable = pdk.portable(file);
            !liberty_files
                .iter()
                .any(|l| l.file == portable && Some(&l.corner) == default_corner.as_ref())
        });
        let mut cells = BTreeMap::new();
        for (_, index) in files {
            for cell in &index.cells {
                cells.entry(cell.name.as_str()).or_insert(cell);
            }
        }
        cells
    };
    let cells: Vec<RefCell> = lib
        .cells
        .iter()
        .map(|cell| {
            let lef = macros.get(cell.name.as_str());
            let lib = typical.get(cell.name.as_str());
            RefCell {
                name: cell.name.clone(),
                views: cell.entries.iter().map(|e| e.name.clone()).collect(),
                class: lef.map(|m| m.class.clone()).unwrap_or_default(),
                size: lef.and_then(|m| m.size),
                area: lib.and_then(|c| c.area),
                sequential: lib.map(|c| c.sequential.clone()).unwrap_or_default(),
            }
        })
        .collect();

    let files = library_files(
        pdk,
        &lib,
        config,
        techlef.iter().map(|t| t.file.clone()).collect(),
        &liberty_paths,
    );
    let role = role(&name, &classes);
    let has_layout = lib
        .cells
        .iter()
        .any(|c| c.entries.iter().any(|e| e.kind == ViewKind::Layout));
    if lib.cells.is_empty() {
        notes.push(format!("refs: {name}: no cells found"));
    }
    let section = RefLibrary {
        name: name.clone(),
        path: pdk.portable(folder),
        layout: lib.layout.name().to_string(),
        role: role.to_string(),
        description: Some(description(role, lib.cells.len())),
        // Everything the PDK is used with; a library that is neither (fonts, test
        // structures) or has nothing to look at is found but left out.
        enabled: role != "other" && !lib.cells.is_empty() && (has_layout || !macros.is_empty()),
        cells: lib.cells.len(),
        formats: formats.iter().map(|f| f.folder.clone()).collect(),
        sites,
        techlef: techlef.iter().map(|t| t.file.clone()).collect(),
        default_corner,
        liberty: liberty_paths,
        files,
    };
    let catalog = RefLibraryCatalog {
        name,
        formats,
        cells,
        liberty: liberty_files,
        corners: liberty_corners,
        techlef,
    };
    (section, catalog, notes)
}

/// Most files listed one by one; more are written as the pattern they share.
const LISTED: usize = 8;

/// Files of one format as the snapshot lists them.
fn listed(pdk: &PdkDir, files: &BTreeSet<PathBuf>) -> Vec<TechPath> {
    if files.len() > LISTED {
        let files: Vec<PathBuf> = files.iter().cloned().collect();
        let first = &files[0];
        let ext = first
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default();
        let folder = first.parent().unwrap_or(Path::new(""));
        return vec![pdk.portable(&folder.join(format!("*{ext}")))];
    }
    files.iter().map(|f| pdk.portable(f)).collect()
}

/// The files of a library by format: what the configuration names, and for a format
/// it does not name what the folders have.
fn library_files(
    pdk: &PdkDir,
    lib: &Library,
    config: Option<&super::librelane::ConfigFiles>,
    techlef: Vec<TechPath>,
    liberty: &BTreeMap<String, TechPath>,
) -> Option<LibraryFiles> {
    use std::ffi::OsStr;
    // The folder's files by format, from the views of the cells.
    let mut guessed: BTreeMap<&str, BTreeSet<PathBuf>> = BTreeMap::new();
    for entry in lib.cells.iter().flat_map(|cell| &cell.entries) {
        for file in &entry.files {
            let name = file
                .file_name()
                .map(|n| n.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            let name = name.strip_suffix(".gz").unwrap_or(&name);
            let ext = Path::new(name).extension().and_then(OsStr::to_str);
            let format = match (entry.kind, ext) {
                (ViewKind::Layout, Some("gds")) => "gds",
                (ViewKind::Abstract, Some("lef")) => "lef",
                (ViewKind::Netlist, Some("cdl")) => "cdl",
                (ViewKind::Netlist, Some("spice" | "sp" | "spi" | "cir")) => "spice",
                (ViewKind::Hdl, Some("v" | "sv")) => "verilog",
                _ => continue,
            };
            guessed.entry(format).or_default().insert(file.clone());
        }
    }
    // A technology LEF is not a cell view of the library.
    let tech: BTreeSet<PathBuf> = techlef
        .iter()
        .filter_map(|t| t.resolve_in(pdk).ok())
        .collect();
    if let Some(lef) = guessed.get_mut("lef") {
        lef.retain(|file| !tech.contains(file));
    }
    let set = |files: &[PathBuf]| files.iter().cloned().collect::<BTreeSet<_>>();
    let config = config.filter(|c| !c.is_empty());
    let mut from_config = false;
    let mut pick = |named: Option<BTreeSet<PathBuf>>, format: &str| -> Vec<TechPath> {
        match named.filter(|files| !files.is_empty()) {
            Some(files) => {
                from_config = true;
                listed(pdk, &files)
            }
            None => guessed
                .get(format)
                .map(|files| listed(pdk, files))
                .unwrap_or_default(),
        }
    };
    let mut files = LibraryFiles {
        gds: pick(config.map(|c| set(&c.gds)), "gds"),
        spice: pick(config.map(|c| set(&c.spice)), "spice"),
        cdl: pick(config.map(|c| set(&c.cdl)), "cdl"),
        lef: pick(config.map(|c| set(&c.lef)), "lef"),
        verilog: pick(config.map(|c| set(&c.verilog)), "verilog"),
        ..LibraryFiles::default()
    };
    match config.map(|c| set(&c.tech_lef)).filter(|f| !f.is_empty()) {
        Some(named) => {
            from_config = true;
            files.tech_lef = listed(pdk, &named);
        }
        None => files.tech_lef = techlef,
    }
    match config.filter(|c| !c.liberty.is_empty()) {
        Some(config) => {
            from_config = true;
            files.liberty = config
                .liberty
                .iter()
                .map(|(corner, file)| LibertyRef {
                    corner: corner.clone(),
                    file: pdk.portable(file),
                })
                .collect();
        }
        None => {
            files.liberty = liberty
                .iter()
                .map(|(corner, file)| LibertyRef {
                    corner: corner.clone(),
                    file: file.clone(),
                })
                .collect()
        }
    }
    if let Some(config) = config {
        files.power_pins = config.power.clone();
        files.ground_pins = config.ground.clone();
    }
    files.source = if from_config {
        "librelane config"
    } else {
        "folder scan"
    }
    .into();
    let nothing = files.formats().iter().all(|(_, list)| list.is_empty());
    (!nothing).then_some(files)
}

/// Every `set ::env(NAME) value` of a LibreLane/OpenLane `config.tcl` (the first of
/// each name), with
/// `$::env(X)` written `$X`. Values computed by a command (`[glob …]`) are kept as
/// written.
pub fn tcl_settings(text: &str) -> BTreeMap<String, String> {
    let mut settings = BTreeMap::new();
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("set ::env(") else {
            continue;
        };
        let Some((name, value)) = rest.split_once(')') else {
            continue;
        };
        let mut value = value.trim().to_string();
        // A quoted or braced value may go on over several lines.
        let (open, close) = match value.chars().next() {
            Some('"') => ('"', '"'),
            Some('{') => ('{', '}'),
            _ => (' ', ' '),
        };
        if open != ' ' {
            let closed = |value: &str| {
                let body = &value[1..];
                match open {
                    '"' => body.contains('"'),
                    _ => body.matches('}').count() > body.matches('{').count(),
                }
            };
            while !closed(&value) {
                let Some(next) = lines.next() else { break };
                value = format!(
                    "{} {}",
                    value.trim_end_matches('\\').trim_end(),
                    next.trim()
                );
            }
            let end = match open {
                '"' => value[1..].find('"').map(|at| at + 1),
                _ => value.rfind(close),
            };
            if let Some(end) = end {
                value = value[1..end].to_string();
            }
        } else if let Some(at) = value.find(" ;") {
            // A trailing comment.
            value.truncate(at);
        }
        let value = env_names(&value)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        // The first value counts: the PDK's default (a file may set a variable again
        // for a moment, to work something out for another library).
        settings.entry(name.to_string()).or_insert(value);
    }
    settings
}

/// `$::env(NAME)` → `$NAME` everywhere in a value.
fn env_names(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(at) = rest.find("$::env(") {
        out.push_str(&rest[..at]);
        let after = &rest[at + "$::env(".len()..];
        match after.find(')') {
            Some(end) => {
                out.push('$');
                out.push_str(&after[..end]);
                rest = &after[end + 1..];
            }
            None => {
                out.push_str(&rest[at..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// The settings of the digital flow's `config.tcl` files: `""` (the PDK's) or the
/// library name → name → value.
type FlowSettings = BTreeMap<String, BTreeMap<String, String>>;

/// The digital flow set-up: `libs.tech/librelane`, else `libs.tech/openlane`.
fn digital(pdk: &PdkDir) -> Option<(Digital, FlowSettings)> {
    let (flow, dir) = ["librelane", "openlane"]
        .into_iter()
        .map(|flow| (flow, pdk.tech(flow)))
        .find(|(_, dir)| dir.join("config.tcl").is_file())?;
    let config = dir.join("config.tcl");
    let settings = tcl_settings(&std::fs::read_to_string(&config).ok()?);
    let mut all = BTreeMap::new();
    let mut digital = Digital {
        flow: flow.to_string(),
        config: pdk.portable(&config),
        default_library: settings.get("STD_CELL_LIBRARY").cloned(),
        pad_library: settings.get("PAD_CELL_LIBRARY").cloned(),
        default_corner: settings.get("DEFAULT_CORNER").cloned(),
        corners: settings
            .get("STA_CORNERS")
            .map(|corners| corners.split_whitespace().map(str::to_string).collect())
            .unwrap_or_default(),
        ..Digital::default()
    };
    all.insert(String::new(), settings);
    let mut items: Vec<PathBuf> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|item| item.path())
        .collect();
    items.sort();
    for item in items {
        let name = name_of(&item);
        if item.is_file() && name.contains("openrcx") {
            digital.rcx_rules.push(pdk.portable(&item));
        } else if item.is_dir() && name == "openrcx" {
            let mut rules: Vec<PathBuf> = std::fs::read_dir(&item)
                .into_iter()
                .flatten()
                .flatten()
                .map(|rule| rule.path())
                .filter(|rule| rule.is_file())
                .collect();
            rules.sort();
            digital
                .rcx_rules
                .extend(rules.iter().map(|rule| pdk.portable(rule)));
        } else if item.is_dir() && item.join("config.tcl").is_file() {
            let config = item.join("config.tcl");
            if let Ok(text) = std::fs::read_to_string(&config) {
                all.insert(name.clone(), tcl_settings(&text));
            }
            digital.libraries.insert(name, pdk.portable(&config));
        }
    }
    digital.config_hash = config_hash(&digital, |path| path.resolve_in(pdk).ok());
    Some((digital, all))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corner_names() {
        let corner = |name: &str| {
            let (process, voltages, temperature) = parse_corner(name);
            (process.unwrap_or_default(), voltages, temperature)
        };
        assert_eq!(corner("tt_025C_1v80"), ("tt".into(), vec![1.8], Some(25.0)));
        assert_eq!(
            corner("typ_1p20V_25C"),
            ("typ".into(), vec![1.2], Some(25.0))
        );
        assert_eq!(
            corner("fast_1p32V_m40C"),
            ("fast".into(), vec![1.32], Some(-40.0))
        );
        assert_eq!(
            corner("ff_ff_n40C_1v95_5v50"),
            ("ff".into(), vec![1.95, 5.5], Some(-40.0))
        );
        assert_eq!(corner("TT_1p8V_25C"), ("tt".into(), vec![1.8], Some(25.0)));
        assert_eq!(
            corner("slow_1p08V_3p0V_125C"),
            ("slow".into(), vec![1.08, 3.0], Some(125.0))
        );
        assert_eq!(
            corner("ss_100C_1v60"),
            ("ss".into(), vec![1.6], Some(100.0))
        );
        // Words that only look like one.
        assert_eq!(corner("dummy"), (String::new(), vec![], None));
        assert_eq!(corner("v2_nointpwr"), (String::new(), vec![], None));
    }

    #[test]
    fn roles_by_name_and_class() {
        let none = BTreeMap::new();
        assert_eq!(role("sky130_fd_sc_hd", &none), "stdcells");
        assert_eq!(role("sg13g2_stdcell", &none), "stdcells");
        assert_eq!(role("sky130_fd_io", &none), "io");
        assert_eq!(role("sg13g2_io", &none), "io");
        assert_eq!(role("sky130_sram_macros", &none), "memory");
        assert_eq!(role("sg13g2_sram", &none), "memory");
        assert_eq!(role("sky130_fd_pr", &none), "primitives");
        assert_eq!(role("sky130_ml_xx_hd", &none), "other");
        let core = BTreeMap::from([("CORE".to_string(), 10), ("BLOCK".to_string(), 1)]);
        assert_eq!(role("mycells", &core), "stdcells");
        let pads = BTreeMap::from([("PAD INOUT".to_string(), 4)]);
        assert_eq!(role("ring", &pads), "io");
    }

    #[test]
    fn flow_settings() {
        let text = r#"
# a comment
set ::env(PROCESS) 130
set ::env(VDD_PIN) "VPWR"
if { ![info exist ::env(STD_CELL_LIBRARY)] } {
	set ::env(STD_CELL_LIBRARY) sg13g2_stdcell
}
set ::env(STA_CORNERS) "\
    nom_fast_1p32V_m40C \
    nom_typ_1p20V_25C \
"
set ::env(CELL_GDS) "$::env(PDK_ROOT)/$::env(PDK)/libs.ref/$::env(STD_CELL_LIBRARY)/gds/x.gds"
set ::env(GPL_CELL_PADDING) {0}
set ::env(OUTPUT_CAP_LOAD) "33.442" ; # femtofarad
set ::env(LIB_TYPICAL) $::env(LIB_SYNTH)
set ::env(TECH_LEFS) [dict create]
"#;
        let settings = tcl_settings(text);
        assert_eq!(settings["PROCESS"], "130");
        assert_eq!(settings["VDD_PIN"], "VPWR");
        assert_eq!(settings["STD_CELL_LIBRARY"], "sg13g2_stdcell");
        assert_eq!(
            settings["STA_CORNERS"],
            "nom_fast_1p32V_m40C nom_typ_1p20V_25C"
        );
        assert_eq!(
            settings["CELL_GDS"],
            "$PDK_ROOT/$PDK/libs.ref/$STD_CELL_LIBRARY/gds/x.gds"
        );
        assert_eq!(settings["GPL_CELL_PADDING"], "0");
        assert_eq!(settings["OUTPUT_CAP_LOAD"], "33.442");
        assert_eq!(settings["LIB_TYPICAL"], "$LIB_SYNTH");
        assert_eq!(settings["TECH_LEFS"], "[dict create]");
    }

    /// A small PDK folder with one standard-cell library.
    fn demo_pdk(root: &Path) -> PdkDir {
        let pdk = PdkDir::new(root, "demo");
        let write = |name: &str, text: &str| {
            let file = pdk.dir().join(name);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, text).unwrap();
        };
        let liberty = |voltage: &str, temperature: &str| {
            format!(
                "library (x) {{\n nom_voltage : {voltage};\n nom_temperature : {temperature};\n \
                 cell (inv_1) {{ area : 3.5; }}\n cell (dff_1) {{ area : 20; ff (IQ,IQN) {{ }} }}\n}}\n"
            )
        };
        write(
            "libs.ref/demo_sc/lef/demo_sc.lef",
            "SITE unit\n CLASS CORE ;\n SIZE 0.46 BY 2.72 ;\nEND unit\n\
             MACRO inv_1\n CLASS CORE ;\n SIZE 1.38 BY 2.72 ;\n SITE unit ;\nEND inv_1\n\
             MACRO dff_1\n CLASS CORE ;\n SIZE 9.2 BY 2.72 ;\n SITE unit ;\nEND dff_1\n",
        );
        write(
            "libs.ref/demo_sc/techlef/demo_sc__nom.tlef",
            "UNITS\n DATABASE MICRONS 1000 ;\nEND UNITS\nLAYER met1\n TYPE ROUTING ;\n \
             DIRECTION HORIZONTAL ;\n PITCH 0.34 ;\nEND met1\n",
        );
        write(
            "libs.ref/demo_sc/lib/demo_sc__tt_025C_1v80.lib",
            &liberty("1.8", "25"),
        );
        write(
            "libs.ref/demo_sc/lib/demo_sc__ff_n40C_1v95.lib",
            &liberty("1.95", "-40"),
        );
        write(
            "libs.ref/demo_sc/verilog/demo_sc.v",
            "module inv_1 (A, Y); endmodule\nmodule dff_1 (D, CLK, Q); endmodule\n",
        );
        write("libs.ref/demo_fonts/mag/a.mag", "magic\ntech demo\n");
        write(
            "libs.tech/openlane/config.tcl",
            "set ::env(STD_CELL_LIBRARY) demo_sc\nset ::env(DEFAULT_CORNER) nom_tt\n",
        );
        write(
            "libs.tech/openlane/demo_sc/config.tcl",
            "set ::env(PLACE_SITE) \"unit\"\n",
        );
        write("libs.tech/openlane/rules.openrcx.demo.nom", "");
        pdk
    }

    #[test]
    fn a_small_pdk() {
        let tmp = tempfile::tempdir().unwrap();
        let pdk = demo_pdk(tmp.path());
        let scanned = scan_with(&ScanContext::new(pdk.clone()).files_only(), &Cache::off());
        assert_eq!(scanned.notes, Vec::<String>::new());
        let refs = &scanned.section;
        assert_eq!(
            refs.path.as_ref().unwrap().as_str(),
            "$PDK_ROOT/demo/libs.ref"
        );
        let names: Vec<_> = refs.libraries.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["demo_fonts", "demo_sc"]);

        let fonts = &refs.libraries[0];
        assert_eq!(
            (fonts.role.as_str(), fonts.enabled, fonts.cells),
            ("other", false, 1)
        );

        let sc = refs.library("demo_sc").unwrap();
        assert_eq!(sc.path.as_str(), "$PDK_ROOT/demo/libs.ref/demo_sc");
        assert_eq!(sc.layout, "ref");
        assert_eq!(sc.role, "stdcells");
        assert!(sc.enabled);
        assert_eq!(sc.cells, 2);
        assert_eq!(sc.formats, ["lef", "lib", "techlef", "verilog"]);
        assert_eq!(
            sc.sites,
            BTreeMap::from([("unit".to_string(), (0.46, 2.72))])
        );
        assert_eq!(
            sc.techlef[0].as_str(),
            "$PDK_ROOT/demo/libs.ref/demo_sc/techlef/demo_sc__nom.tlef"
        );
        assert_eq!(sc.default_corner.as_deref(), Some("tt_025C_1v80"));
        assert_eq!(
            sc.liberty.keys().collect::<Vec<_>>(),
            ["ff_n40C_1v95", "tt_025C_1v80"]
        );
        assert_eq!(
            sc.liberty["tt_025C_1v80"].as_str(),
            "$PDK_ROOT/demo/libs.ref/demo_sc/lib/demo_sc__tt_025C_1v80.lib"
        );
        // The project lists what the section says.
        let listed = refs.libraries();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[1].name, "demo_sc");
        assert_eq!(listed[1].layout, "ref");
        assert!(listed[1].enabled && !listed[0].enabled);

        let digital = refs.digital.as_ref().unwrap();
        assert_eq!(digital.flow, "openlane");
        assert_eq!(digital.default_library.as_deref(), Some("demo_sc"));
        assert_eq!(digital.default_corner.as_deref(), Some("nom_tt"));
        assert_eq!(digital.rcx_rules.len(), 1);
        assert_eq!(
            digital.libraries["demo_sc"].as_str(),
            "$PDK_ROOT/demo/libs.tech/openlane/demo_sc/config.tcl"
        );

        let catalog = &scanned.catalog;
        assert_eq!(catalog.digital["demo_sc"]["PLACE_SITE"], "unit");
        let sc = catalog
            .libraries
            .iter()
            .find(|l| l.name == "demo_sc")
            .unwrap();
        let dff = sc.cells.iter().find(|c| c.name == "dff_1").unwrap();
        assert_eq!(dff.views, ["abstract", "hdl", "liberty"]);
        assert_eq!(dff.class, "CORE");
        assert_eq!(dff.size, Some((9.2, 2.72)));
        assert_eq!(dff.area, Some(20.0));
        assert_eq!(dff.sequential, "ff");
        assert_eq!(sc.liberty.len(), 2);
        let corners: Vec<_> = sc
            .corners
            .iter()
            .map(|c| {
                (
                    c.name.as_str(),
                    c.process.as_deref(),
                    c.voltage,
                    c.temperature,
                    c.files,
                )
            })
            .collect();
        assert_eq!(
            corners,
            [
                ("ff_n40C_1v95", Some("ff"), Some(1.95), Some(-40.0), 1),
                ("tt_025C_1v80", Some("tt"), Some(1.8), Some(25.0), 1),
            ]
        );
        assert_eq!(sc.corner_files("tt_025C_1v80").count(), 1);
        assert_eq!(sc.techlef[0].layers[0].name, "met1");
        assert_eq!(sc.techlef[0].database_microns, Some(1000.0));
        let lef = sc.formats.iter().find(|f| f.folder == "lef").unwrap();
        assert_eq!(lef.collections[0].cells, 2);

        // Both files survive TOML and JSON.
        let text = toml::to_string(refs).unwrap();
        assert!(text.contains("[[library]]"), "{text}");
        assert_eq!(&toml::from_str::<RefsTech>(&text).unwrap(), refs);
        let json = serde_json::to_string(catalog).unwrap();
        assert_eq!(
            &serde_json::from_str::<RefsCatalog>(&json).unwrap(),
            catalog
        );
    }

    #[test]
    fn corners_of_file_names() {
        let index = container::liberty::LibertyIndex::default();
        let corner = |stem: &str, library: &str| corner_of(stem, library, &index);
        assert_eq!(
            corner("sky130_fd_sc_hd__tt_025C_1v80", "sky130_fd_sc_hd"),
            "tt_025C_1v80"
        );
        assert_eq!(
            corner(
                "sky130_ef_io__gpiov2_pad_tt_tt_025C_1v80_3v30",
                "sky130_fd_io"
            ),
            "tt_tt_025C_1v80_3v30"
        );
        assert_eq!(
            corner(
                "RM_IHPSG13_1P_1024x16_c2_bm_bist_fast_1p32V_m55C",
                "sg13g2_sram"
            ),
            "fast_1p32V_m55C"
        );
        assert_eq!(
            corner("sram_32_256_TT_1p8V_25C", "sky130_sram_macros"),
            "TT_1p8V_25C"
        );
        assert_eq!(corner("sg13g2_io_dummy", "sg13g2_io"), "dummy");
        // A process word in a cell name is not the corner.
        assert_eq!(corner("lib_fast_adder_ss_100C_1v60", "lib"), "ss_100C_1v60");
        assert_eq!(
            pattern(&[
                PathBuf::from("/p/lib/ram8_typ_1p20V_25C.lib"),
                PathBuf::from("/p/lib/ram16_typ_1p20V_25C.lib"),
            ]),
            PathBuf::from("/p/lib/*_typ_1p20V_25C.lib")
        );
    }

    #[test]
    fn no_reference_libraries() {
        let tmp = tempfile::tempdir().unwrap();
        let pdk = PdkDir::new(tmp.path(), "bare");
        std::fs::create_dir_all(pdk.dir()).unwrap();
        let scanned = scan_with(&ScanContext::new(pdk).files_only(), &Cache::off());
        assert!(scanned.section.libraries.is_empty());
        assert!(scanned.section.path.is_none());
        assert_eq!(scanned.notes.len(), 1);
    }
}
