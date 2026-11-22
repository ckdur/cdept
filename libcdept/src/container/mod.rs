//! Files that hold many cells: what is inside them, cheaply.
//!
//! A standard-cell library ships one GDS with every cell, one LEF with every macro,
//! one Liberty file per corner, one Verilog and one SPICE file. This module reads the
//! names and a few cheap facts out of such files (never the geometry or the timing
//! tables), so a library can list the cells and the Inspector can say something about
//! each:
//!
//! | file | [`Index`] | cells | facts |
//! |---|---|---|---|
//! | GDSII, OASIS | [`LayoutIndex`] | structures | what each places, database unit |
//! | LEF | [`lef::LefIndex`] | macros | class, size, site, pins; layers and sites of a technology LEF |
//! | Liberty | [`liberty::LibertyIndex`] | cells | area, pins, sequential; units and conditions |
//! | Verilog | [`verilog::VerilogIndex`] | modules | ports |
//! | SPICE, CDL | [`spice::SpiceIndex`] | subcircuits | pins, parameters |
//! | Magic | [`magic::MagicIndex`] | (the file is one cell) | technology, cells used, ports |
//!
//! [`read`] reads a file; [`cache::Cache`] keeps the result so files of tens of
//! megabytes are read once.

pub mod cache;
pub mod lef;
pub mod liberty;
pub mod magic;
pub mod spice;
pub mod verilog;

use std::collections::BTreeSet;
use std::fs::File;
use std::path::Path;

use eyre::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::Format;
pub use cache::Cache;

/// The cells of a GDSII or OASIS file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LayoutIndex {
    /// The library name in the stream (GDSII `LIBNAME`).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub library: String,
    /// Database unit in micron (0.001 = 1 nm).
    pub dbu: f64,
    /// Cells in file order.
    pub cells: Vec<LayoutCell>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LayoutCell {
    pub name: String,
    /// The cells it places, sorted.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub refs: Vec<String>,
}

impl LayoutIndex {
    fn from_summary(summary: crate::layout::gds::GdsSummary) -> LayoutIndex {
        LayoutIndex {
            library: summary.library,
            dbu: summary.user_per_dbu,
            cells: summary
                .structures
                .into_iter()
                .map(|s| LayoutCell {
                    name: s.name,
                    refs: s.refs.into_iter().collect(),
                })
                .collect(),
        }
    }

    /// Cells no other cell of the file places, in file order.
    pub fn tops(&self) -> Vec<&str> {
        let placed: BTreeSet<&str> = self
            .cells
            .iter()
            .flat_map(|c| c.refs.iter().map(String::as_str))
            .collect();
        self.cells
            .iter()
            .map(|c| c.name.as_str())
            .filter(|name| !placed.contains(name))
            .collect()
    }
}

/// What a file holds, by its kind.
// One index per file being read, never kept in bulk: the size of the largest variant
// does not matter.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Index {
    Layout(LayoutIndex),
    Lef(lef::LefIndex),
    Liberty(liberty::LibertyIndex),
    Verilog(verilog::VerilogIndex),
    Spice(spice::SpiceIndex),
    Magic(magic::MagicIndex),
}

/// The reader for a file, by its name. `.tlef` is LEF too. `None`: not a file this
/// module reads (schematics, symbols, compressed streams, …).
pub fn reader_of(path: &Path) -> Option<Reader> {
    let name = path.file_name()?.to_str()?;
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".gz") {
        return None;
    }
    if lower.ends_with(".tlef") {
        return Some(Reader::Lef);
    }
    Reader::of_format(Format::split_name(name)?.1)
}

/// Which reader a file gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Reader {
    Gds,
    Oasis,
    Lef,
    Liberty,
    Verilog,
    Spice,
    Magic,
}

impl Reader {
    pub fn of_format(format: Format) -> Option<Reader> {
        Some(match format {
            Format::Gds => Reader::Gds,
            Format::Oasis => Reader::Oasis,
            Format::Lef => Reader::Lef,
            Format::Liberty => Reader::Liberty,
            Format::Verilog | Format::SystemVerilog => Reader::Verilog,
            Format::Spice | Format::Cdl => Reader::Spice,
            Format::Magic => Reader::Magic,
            _ => return None,
        })
    }

    /// The file may hold several cells (a `.mag` file is always one).
    pub fn holds_cells(self) -> bool {
        self != Reader::Magic
    }
}

/// Whether a file of this format may hold several cells.
pub fn holds_cells(format: Format) -> bool {
    Reader::of_format(format).is_some_and(Reader::holds_cells)
}

/// The bytes of a file, memory-mapped (an empty file maps to nothing).
struct Bytes(Option<memmap2::Mmap>);

impl Bytes {
    fn open(path: &Path) -> Result<Bytes> {
        let file = File::open(path).wrap_err_with(|| format!("cannot read {}", path.display()))?;
        let len = file.metadata().map(|m| m.len()).unwrap_or(0);
        if len == 0 {
            return Ok(Bytes(None));
        }
        // SAFETY: the mapping is read-only and dropped before this function's caller
        // returns; a file truncated meanwhile is the usual risk of mapping.
        let map = unsafe { memmap2::Mmap::map(&file) }
            .wrap_err_with(|| format!("cannot map {}", path.display()))?;
        Ok(Bytes(Some(map)))
    }

    fn get(&self) -> &[u8] {
        self.0.as_deref().unwrap_or_default()
    }
}

/// Read what a file holds, without the cache. Fails for files this module does not
/// read and for layout streams that are not valid.
pub fn read(path: &Path) -> Result<Index> {
    let Some(reader) = reader_of(path) else {
        bail!("{}: not a file with cells inside", path.display());
    };
    read_as(path, reader)
}

/// [`read`] with the reader given.
pub fn read_as(path: &Path, reader: Reader) -> Result<Index> {
    let bytes = Bytes::open(path)?;
    let bytes = bytes.get();
    Ok(match reader {
        Reader::Gds => {
            if bytes.starts_with(&[0x1f, 0x8b]) {
                bail!("{} is compressed (gzip)", path.display());
            }
            Index::Layout(LayoutIndex::from_summary(
                crate::layout::gds::parse(bytes)
                    .wrap_err_with(|| format!("{} is not a valid GDSII stream", path.display()))?,
            ))
        }
        Reader::Oasis => Index::Layout(LayoutIndex::from_summary(
            crate::layout::oasis::parse(bytes)
                .wrap_err_with(|| format!("{} is not a valid OASIS file", path.display()))?,
        )),
        Reader::Lef => Index::Lef(lef::parse(bytes)),
        Reader::Liberty => {
            // `.lib` is also what SPICE model libraries are called.
            let index = liberty::parse(bytes);
            if index.library.is_empty() && index.cells.is_empty() {
                let netlist = spice::parse(bytes);
                if netlist != spice::SpiceIndex::default() {
                    return Ok(Index::Spice(netlist));
                }
            }
            Index::Liberty(index)
        }
        Reader::Verilog => Index::Verilog(verilog::parse(bytes)),
        Reader::Spice => Index::Spice(spice::parse(bytes)),
        Reader::Magic => Index::Magic(magic::parse(bytes)),
    })
}

/// Read several files side by side (through the cache), each with the reader given.
/// Results in the order of `files`.
pub fn read_many(files: &[(&Path, Reader)], cache: &Cache) -> Vec<Result<Index>> {
    let threads = std::thread::available_parallelism()
        .map_or(4, |n| n.get())
        .min(8)
        .min(files.len());
    if threads < 2 {
        return files
            .iter()
            .map(|(path, reader)| cache.get_as(path, *reader))
            .collect();
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    let mut results: Vec<Option<Result<Index>>> = (0..files.len()).map(|_| None).collect();
    let slots = std::sync::Mutex::new(&mut results);
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                loop {
                    let at = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some((path, reader)) = files.get(at) else {
                        break;
                    };
                    let result = cache.get_as(path, *reader);
                    slots.lock().expect("no reader panics")[at] = Some(result);
                }
            });
        }
    });
    results
        .into_iter()
        .map(|r| r.expect("every file was read"))
        .collect()
}

/// What the files of a view say about its cell, as `(name, value)` lines: what is
/// known of the cell itself (LEF class and size, Liberty area and pins, sub-circuit
/// pins, …), then of the file. `cell` is the name of the cell the view belongs to.
/// Nothing for views whose file this module does not read.
pub fn entry_facts(entry: &crate::Entry, cell: &str, cache: &Cache) -> Vec<(String, String)> {
    let Some(file) = &entry.main_file else {
        return Vec::new();
    };
    let Some(reader) = entry.format.and_then(Reader::of_format) else {
        return Vec::new();
    };
    let Ok(index) = cache.get_as(file, reader) else {
        return Vec::new();
    };
    let mut facts = Vec::new();
    if let Some(member) = &entry.member {
        facts.push(fact("Cell in file", member));
    }
    facts.extend(index.member_facts(entry.member_or(cell)));
    if entry.files.len() > 1 {
        facts.push(fact("Files", entry.files.len()));
    }
    facts.extend(index.file_facts());
    facts
}

fn fact(name: &str, value: impl std::fmt::Display) -> (String, String) {
    (name.to_string(), value.to_string())
}

fn number(value: f64) -> String {
    let text = format!("{value:.6}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

impl Index {
    /// `GDSII/OASIS`, `LEF`, … for messages.
    pub fn label(&self) -> &'static str {
        match self {
            Index::Layout(_) => "layout stream",
            Index::Lef(_) => "LEF",
            Index::Liberty(_) => "Liberty",
            Index::Verilog(_) => "Verilog",
            Index::Spice(_) => "netlist",
            Index::Magic(_) => "Magic",
        }
    }

    /// Every cell the file defines, in file order.
    pub fn names(&self) -> Vec<&str> {
        match self {
            Index::Layout(i) => i.cells.iter().map(|c| c.name.as_str()).collect(),
            Index::Lef(i) => i.macros.iter().map(|m| m.name.as_str()).collect(),
            Index::Liberty(i) => i.cells.iter().map(|c| c.name.as_str()).collect(),
            Index::Verilog(i) => i.modules.iter().map(|m| m.name.as_str()).collect(),
            Index::Spice(i) => i.subckts.iter().map(|s| s.name.as_str()).collect(),
            Index::Magic(_) => Vec::new(),
        }
    }

    /// The cells that stand on their own: for a layout the ones no other cell of the
    /// file places, else all of them.
    pub fn public(&self) -> Vec<&str> {
        match self {
            Index::Layout(i) => i.tops(),
            _ => self.names(),
        }
    }

    pub fn defines(&self, name: &str) -> bool {
        self.names().contains(&name)
    }

    /// What the file says about itself, as `(name, value)` lines for the Inspector.
    pub fn file_facts(&self) -> Vec<(String, String)> {
        let mut facts = Vec::new();
        match self {
            Index::Layout(i) => {
                if !i.library.is_empty() {
                    facts.push(fact("Stream library", &i.library));
                }
                if i.dbu > 0.0 {
                    facts.push(fact("Database unit", format!("{} µm", number(i.dbu))));
                }
                facts.push(fact(
                    "Cells",
                    format!("{} ({} top)", i.cells.len(), i.tops().len()),
                ));
            }
            Index::Lef(i) => {
                if !i.version.is_empty() {
                    facts.push(fact("LEF version", &i.version));
                }
                if let Some(units) = i.database_microns {
                    facts.push(fact("Database units per µm", number(units)));
                }
                if let Some(grid) = i.manufacturing_grid {
                    facts.push(fact("Manufacturing grid", format!("{} µm", number(grid))));
                }
                if !i.macros.is_empty() {
                    facts.push(fact("Macros", i.macros.len()));
                }
                if !i.layers.is_empty() {
                    facts.push(fact("Layers", i.layers.len()));
                }
                for site in &i.sites {
                    let size = site
                        .size
                        .map(|(w, h)| format!(" {} × {} µm", number(w), number(h)))
                        .unwrap_or_default();
                    facts.push(fact("Site", format!("{}{size}", site.name)));
                }
            }
            Index::Liberty(i) => {
                facts.push(fact("Liberty library", &i.library));
                let c = i.conditions();
                let mut corner = Vec::new();
                if !c.name.is_empty() {
                    corner.push(c.name.clone());
                }
                if let Some(p) = c.process {
                    corner.push(format!("process {}", number(p)));
                }
                if let Some(v) = c.voltage {
                    corner.push(format!("{} V", number(v)));
                }
                if let Some(t) = c.temperature {
                    corner.push(format!("{} °C", number(t)));
                }
                if !corner.is_empty() {
                    facts.push(fact("Conditions", corner.join(", ")));
                }
                let units: Vec<&str> = [
                    &i.time_unit,
                    &i.voltage_unit,
                    &i.current_unit,
                    &i.capacitive_load_unit,
                    &i.leakage_power_unit,
                ]
                .into_iter()
                .filter(|u| !u.is_empty())
                .map(String::as_str)
                .collect();
                if !units.is_empty() {
                    facts.push(fact("Units", units.join(", ")));
                }
                if !i.delay_model.is_empty() {
                    facts.push(fact("Delay model", &i.delay_model));
                }
                facts.push(fact("Cells", i.cells.len()));
            }
            Index::Verilog(i) => {
                facts.push(fact("Modules", i.modules.len()));
                if !i.primitives.is_empty() {
                    facts.push(fact("Primitives", i.primitives.len()));
                }
            }
            Index::Spice(i) => {
                facts.push(fact("Subcircuits", i.subckts.len()));
                if !i.models.is_empty() {
                    facts.push(fact("Models", i.models.len()));
                }
                if !i.sections.is_empty() {
                    facts.push(fact("Sections", i.sections.join(", ")));
                }
                if !i.includes.is_empty() {
                    facts.push(fact("Includes", i.includes.join(", ")));
                }
            }
            Index::Magic(i) => {
                if !i.tech.is_empty() {
                    facts.push(fact("Technology", &i.tech));
                }
                if !i.ports.is_empty() {
                    facts.push(fact("Ports", i.ports.join(" ")));
                }
                if !i.uses.is_empty() {
                    facts.push(fact("Uses", format!("{} cells", i.uses.len())));
                }
                for name in ["LEFclass", "LEFsite", "FIXED_BBOX", "GDS_FILE"] {
                    if let Some(value) = i.properties.get(name) {
                        facts.push(fact(name, value));
                    }
                }
            }
        }
        facts
    }

    /// What the file says about one of its cells, as `(name, value)` lines.
    pub fn member_facts(&self, member: &str) -> Vec<(String, String)> {
        let mut facts = Vec::new();
        match self {
            Index::Layout(i) => {
                if let Some(cell) = i.cells.iter().find(|c| c.name == member) {
                    let top = i.tops().contains(&member);
                    facts.push(fact(
                        "Places",
                        match cell.refs.len() {
                            0 => "no other cell".to_string(),
                            n => format!("{n} cells"),
                        },
                    ));
                    if !top {
                        facts.push(fact("Placed by", "other cells of the file"));
                    }
                }
            }
            Index::Lef(i) => {
                if let Some(cell) = i.macros.iter().find(|m| m.name == member) {
                    if !cell.class.is_empty() {
                        facts.push(fact("Class", &cell.class));
                    }
                    if let Some((w, h)) = cell.size {
                        facts.push(fact("Size", format!("{} × {} µm", number(w), number(h))));
                    }
                    if !cell.site.is_empty() {
                        facts.push(fact("Site", &cell.site));
                    }
                    if !cell.symmetry.is_empty() {
                        facts.push(fact("Symmetry", &cell.symmetry));
                    }
                    let pins = |wanted: &dyn Fn(&lef::LefPin) -> bool| {
                        cell.pins
                            .iter()
                            .filter(|p| wanted(p))
                            .map(|p| p.name.as_str())
                            .collect::<Vec<_>>()
                            .join(" ")
                    };
                    let supply = |p: &lef::LefPin| matches!(p.usage.as_str(), "POWER" | "GROUND");
                    for (label, direction) in [
                        ("Inputs", "INPUT"),
                        ("Outputs", "OUTPUT"),
                        ("Inouts", "INOUT"),
                    ] {
                        let names = pins(&|p| !supply(p) && p.direction.starts_with(direction));
                        if !names.is_empty() {
                            facts.push(fact(label, names));
                        }
                    }
                    let names = pins(&|p| supply(p));
                    if !names.is_empty() {
                        facts.push(fact("Supplies", names));
                    }
                    let names = pins(&|p| !supply(p) && p.direction.is_empty());
                    if !names.is_empty() {
                        facts.push(fact("Pins", names));
                    }
                }
            }
            Index::Liberty(i) => {
                if let Some(cell) = i.cells.iter().find(|c| c.name == member) {
                    if let Some(area) = cell.area {
                        facts.push(fact("Area", number(area)));
                    }
                    if !cell.footprint.is_empty() {
                        facts.push(fact("Footprint", &cell.footprint));
                    }
                    let mut kind = Vec::new();
                    if !cell.sequential.is_empty() {
                        kind.push(cell.sequential.as_str());
                    }
                    if cell.is_macro {
                        kind.push("macro");
                    }
                    if cell.dont_use {
                        kind.push("dont_use");
                    }
                    if !kind.is_empty() {
                        facts.push(fact("Kind", kind.join(", ")));
                    }
                    for (label, wanted) in [
                        ("Inputs", "input"),
                        ("Outputs", "output"),
                        ("Inouts", "inout"),
                    ] {
                        let names: Vec<&str> = cell
                            .pins
                            .iter()
                            .filter(|p| p.kind != "pg_pin" && p.direction == wanted)
                            .map(|p| p.name.as_str())
                            .collect();
                        if !names.is_empty() {
                            facts.push(fact(label, names.join(" ")));
                        }
                    }
                    let names: Vec<&str> = cell
                        .pins
                        .iter()
                        .filter(|p| p.kind == "pg_pin")
                        .map(|p| p.name.as_str())
                        .collect();
                    if !names.is_empty() {
                        facts.push(fact("Supplies", names.join(" ")));
                    }
                }
            }
            Index::Verilog(i) => {
                if let Some(module) = i.modules.iter().find(|m| m.name == member) {
                    for (label, wanted) in [
                        ("Inputs", "input"),
                        ("Outputs", "output"),
                        ("Inouts", "inout"),
                        ("Ports", ""),
                    ] {
                        let names: Vec<String> = module
                            .ports
                            .iter()
                            .filter(|p| p.direction == wanted)
                            .map(|p| format!("{}{}", p.name, p.range))
                            .collect();
                        if !names.is_empty() {
                            facts.push(fact(label, names.join(" ")));
                        }
                    }
                }
            }
            Index::Spice(i) => {
                if let Some(subckt) = i.subckts.iter().find(|s| s.name == member) {
                    facts.push(fact("Pins", subckt.pins.join(" ")));
                    if !subckt.parameters.is_empty() {
                        facts.push(fact("Parameters", subckt.parameters.join(" ")));
                    }
                    facts.push(fact("Devices", subckt.devices));
                }
            }
            Index::Magic(_) => {}
        }
        facts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readers_by_file_name() {
        assert_eq!(reader_of(Path::new("a/cells.gds")), Some(Reader::Gds));
        assert_eq!(reader_of(Path::new("tech.tlef")), Some(Reader::Lef));
        assert_eq!(reader_of(Path::new("x.lib")), Some(Reader::Liberty));
        assert_eq!(reader_of(Path::new("x.cdl")), Some(Reader::Spice));
        assert_eq!(reader_of(Path::new("x.sv")), Some(Reader::Verilog));
        assert_eq!(reader_of(Path::new("x.mag")), Some(Reader::Magic));
        assert_eq!(reader_of(Path::new("x.gds.gz")), None);
        assert_eq!(reader_of(Path::new("x.sch")), None);
        assert!(holds_cells(Format::Gds));
        assert!(!holds_cells(Format::Magic));
        assert!(!holds_cells(Format::XschemSymbol));
    }

    #[test]
    fn a_spice_library_called_lib_is_a_netlist() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("corner.lib");
        std::fs::write(&file, ".lib tt\n.include 'a.spice'\n.endl\n").unwrap();
        assert!(matches!(read(&file).unwrap(), Index::Spice(_)));
        let file = dir.path().join("cells.lib");
        std::fs::write(&file, "library (x) {\n cell (a) { area : 1; }\n}\n").unwrap();
        let index = read(&file).unwrap();
        assert!(matches!(index, Index::Liberty(_)));
        assert_eq!(index.names(), ["a"]);
        assert_eq!(index.member_facts("a"), [fact("Area", "1")]);
        // An empty file holds nothing.
        let file = dir.path().join("empty.v");
        std::fs::write(&file, "").unwrap();
        assert!(read(&file).unwrap().names().is_empty());
    }

    #[test]
    fn layout_tops() {
        let index = LayoutIndex {
            cells: vec![
                LayoutCell {
                    name: "via".into(),
                    refs: vec![],
                },
                LayoutCell {
                    name: "inv".into(),
                    refs: vec!["via".into(), "ghost".into()],
                },
                LayoutCell {
                    name: "buf".into(),
                    refs: vec![],
                },
            ],
            ..LayoutIndex::default()
        };
        assert_eq!(index.tops(), ["inv", "buf"]);
        let index = Index::Layout(index);
        assert_eq!(index.public(), ["inv", "buf"]);
        assert_eq!(index.names(), ["via", "inv", "buf"]);
        assert!(index.defines("via"));
    }
}
