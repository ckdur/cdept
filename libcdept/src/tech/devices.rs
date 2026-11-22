//! Devices section of the technology snapshot: the PDK's devices, joined from what
//! the other sections found (xschem symbol → model → KLayout PCell, Magic generator).
//!
//! A device is a symbol of the PDK's primitives library that netlists as a model or
//! subcircuit. For each one the catalog ([`DevicesCatalog`]) has its class, terminals,
//! parameters with defaults, which model set has its model and through which library,
//! and its layout: the KLayout PCell and the Magic generator.
//!
//! The PCell comes from the device map shipped with CDePT (`pdk/<PDK>/devices.toml`,
//! docs/LINKING.md §3) when there is one: that map is written by hand and is what
//! "Generate from Schematic" uses. For a PDK without a shipped map the scan *guesses*:
//! a PCell with a choice parameter offering the device's model, a PCell of the same
//! name, a PCell whose name is part of the device's. Every guess is marked as one,
//! with its reason. [`draft_map`] writes the guesses as a device map to start from.
//!
//! `[devices]` in `tech.toml` ([`DevicesTech`]) only says where the layout side came
//! from and counts.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::klayout::{Pcell, PcellLibrary, PcellParameter};
use super::pdk::{ScanContext, Scanned};
use super::xschem::Symbol;
use super::{Catalog, Tech};
use crate::link::devices as map;

/// `[devices]` in `tech.toml`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DevicesTech {
    /// Where the KLayout PCell of a device comes from: `map` (the device map shipped
    /// with CDePT for this PDK), `guess` (matched by the scan: check them, see
    /// `cdept tech devices` for a draft map), `none`.
    pub pcells: String,
    /// Devices found (symbols of the primitives libraries that netlist as a model).
    pub count: usize,
    /// Devices with a KLayout PCell.
    pub with_pcell: usize,
    /// Of those, the ones that were guessed.
    pub guessed: usize,
    /// Devices with a Magic generator.
    pub with_magic: usize,
    /// Devices whose model no model set defines.
    pub without_model: usize,
    /// Devices per class.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub classes: BTreeMap<String, usize>,
}

/// The devices part of the catalog.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DevicesCatalog {
    pub devices: Vec<Device>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Device {
    /// The symbol name (`sg13_lv_nmos`, `nfet_01v8`).
    pub name: String,
    /// `mos_lv`, `mos_hv`, `mos`, `res`, `cap`, `bjt`, `diode`, `varactor`, `esd`,
    /// `tap`, `ind`, `pad`, `other`.
    pub class: String,
    /// The symbol as a schematic refers to it: `sg13g2_pr/sg13_lv_nmos.sym`.
    pub symbol: String,
    /// The xschem library of the symbol.
    pub library: String,
    /// xschem's type of the symbol (`nmos`, `subcircuit`, …).
    pub kind: String,
    /// The model or subcircuit it netlists as (`sky130_fd_pr__nfet_01v8`).
    pub model: String,
    /// The first letters of its netlist line.
    pub prefix: String,
    /// The symbol's pins in netlist order.
    pub terminals: Vec<String>,
    pub parameters: Vec<DeviceParameter>,
    /// Model set → the library file that brings the model (`cornerMOSlv.lib`), or the
    /// file to include by itself.
    pub models: BTreeMap<String, String>,
    /// The simulators that have a model for it.
    pub simulators: Vec<String>,
    /// The pins of the model's subcircuit, when it has another number of pins than
    /// the symbol (something to look at).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub model_pins: Vec<String>,
    /// The KLayout PCell.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pcell: Option<PcellMatch>,
    /// The Magic device generator.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub magic: Option<GeneratorMatch>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DeviceParameter {
    pub name: String,
    /// The default of new instances, as the symbol writes it.
    pub default: String,
    /// `m`, `um`, `m^2`, `um^2` for sizes when the default tells; else empty.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub unit: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PcellMatch {
    /// The KLayout library (`SG13_dev`).
    pub library: String,
    pub cell: String,
    /// PCell parameter → template (`@{w|u}`, docs/LINKING.md §3).
    pub params: BTreeMap<String, String>,
    /// Matched by the scan, not by a device map.
    pub guess: bool,
    /// Why this PCell.
    pub reason: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneratorMatch {
    /// `<namespace>::<name>`, what `magic::gencell` takes.
    pub name: String,
    pub guess: bool,
    pub reason: String,
}

/// xschem symbol types that are not devices.
const NOT_DEVICES: [&str; 20] = [
    "",
    "netlist_commands",
    "label",
    "ipin",
    "opin",
    "iopin",
    "launcher",
    "logo",
    "title",
    "noconn",
    "probe",
    "show_label",
    "bus_tap",
    "architecture",
    "package",
    "port_attributes",
    "use",
    "scope",
    "ngprobe",
    "attributes",
];

fn is_device(symbol: &Symbol) -> bool {
    !NOT_DEVICES.contains(&symbol.kind.as_str())
        && !symbol.pins.is_empty()
        && (!symbol.format.is_empty() || !symbol.lvs_format.is_empty())
        && !symbol.name.starts_with("annotate")
}

/// The class of a device from its xschem type and its names; `model_class` is the
/// class of the library that brings its model (it tells low from high voltage).
fn class_of(symbol: &Symbol, model_class: &str) -> String {
    let kind = symbol.kind.to_ascii_lowercase();
    let name = format!("{} {}", symbol.name, symbol.model).to_ascii_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| name.contains(w));
    let by_name = if has(&["esd", "diodevdd", "diodevss", "nmoscl", "scr"]) {
        "esd"
    } else if has(&["tap"]) {
        "tap"
    } else if has(&["varicap", "varactor", "cap_var", "moscap"]) {
        "varactor"
    } else if kind.starts_with("nmos") || kind.starts_with("pmos") {
        "mos"
    } else if kind.starts_with("npn") || kind.starts_with("pnp") {
        "bjt"
    } else if kind == "diode" {
        "diode"
    } else if kind == "resistor" {
        "res"
    } else if kind == "capacitor" {
        "cap"
    } else if kind == "inductor" {
        "ind"
    } else if has(&["pad"]) {
        "pad"
    } else if has(&["cap", "cmim", "mim", "cmom"]) {
        "cap"
    } else if has(&["res", "rsil", "rppd", "rhigh"]) {
        "res"
    } else if has(&["ind"]) {
        "ind"
    } else if has(&["npn", "pnp"]) {
        "bjt"
    } else if has(&["dio", "schottky", "antenna"]) {
        "diode"
    } else if has(&["fet", "mos"]) {
        "mos"
    } else {
        "other"
    };
    match by_name {
        "mos" if model_class.starts_with("mos_") => model_class.to_string(),
        "other" if !matches!(model_class, "" | "all" | "mixed") => model_class.to_string(),
        other => other.to_string(),
    }
}

/// Parameters that are lengths and areas.
const LENGTHS: [&str; 10] = ["w", "l", "wf", "lf", "wfeed", "pd", "ps", "sa", "sb", "sd"];
const AREAS: [&str; 4] = ["ad", "as", "a", "area"];

fn unit_of(name: &str, default: &str) -> String {
    let lower = name.to_ascii_lowercase();
    let power = if LENGTHS.contains(&lower.as_str()) {
        ""
    } else if AREAS.contains(&lower.as_str()) {
        "^2"
    } else {
        return String::new();
    };
    let Some(value) = map::spice_number(default) else {
        return String::new();
    };
    // `0.15u` is metres; a plain `1` or `0.15` is the PDK's scale, micron.
    let scaled = default
        .trim()
        .chars()
        .next_back()
        .is_some_and(char::is_alphabetic)
        || (value != 0.0 && value.abs() < 1e-3);
    format!("{}{power}", if scaled { "m" } else { "um" })
}

// ---------------------------------------------------------------------------
// Matching PCells
// ---------------------------------------------------------------------------

/// Names that mean the same parameter.
const ALIASES: [&[&str]; 4] = [
    &["w", "width"],
    &["l", "length"],
    &["nf", "ng", "nfingers", "n_fingers", "fingers"],
    &["m", "mult", "multiplier", "multipliers"],
];

fn same_parameter(a: &str, b: &str) -> bool {
    let (a, b) = (a.to_ascii_lowercase(), b.to_ascii_lowercase());
    a == b
        || ALIASES
            .iter()
            .any(|group| group.contains(&a.as_str()) && group.contains(&b.as_str()))
}

/// The template that carries a symbol parameter into a PCell parameter: with a unit
/// when the PCell wants `0.15u` texts (`@{w|u}`), as a plain number of micron when the
/// symbol counts in metres and the PCell in micron (`@{w|um}`), else as written.
fn param_template(symbol_param: &str, symbol_default: &str, pcell: &PcellParameter) -> String {
    let pcell_default = pcell.default.trim();
    let symbol_value = map::spice_number(symbol_default);
    let symbol_scaled = symbol_default
        .trim()
        .chars()
        .next_back()
        .is_some_and(char::is_alphabetic);
    if pcell.kind == "string"
        && pcell_default.ends_with(['u', 'n', 'p', 'm'])
        && map::spice_number(pcell_default).is_some()
        && symbol_value.is_some()
    {
        let unit = pcell_default.chars().next_back().unwrap_or('u');
        return format!("@{{{symbol_param}|{unit}}}");
    }
    if matches!(pcell.kind.as_str(), "double" | "int")
        && symbol_scaled
        && let (Some(symbol), Ok(wanted)) = (symbol_value, pcell_default.parse::<f64>())
        && symbol != 0.0
        && wanted != 0.0
        && (wanted / symbol) > 1e3
    {
        // The symbol says 0.15u (metres), the PCell 0.15 (micron).
        return format!("@{{{symbol_param}|um}}");
    }
    format!("@{symbol_param}")
}

/// The words of a name: split at `_`, at digit/letter changes left alone, and at a
/// lower-to-upper case change (`nmosHV` → `nmos`, `hv`).
fn words(name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut previous_lower = false;
    for c in name.chars() {
        if c == '_' || c == '-' || c == '.' || (c.is_ascii_uppercase() && previous_lower) {
            if !word.is_empty() {
                out.push(std::mem::take(&mut word).to_ascii_lowercase());
            }
            if c.is_ascii_uppercase() {
                word.push(c);
            }
        } else {
            word.push(c);
        }
        previous_lower = c.is_ascii_lowercase();
    }
    if !word.is_empty() {
        out.push(word.to_ascii_lowercase());
    }
    out
}

/// Words of a device name that change what the device is: a PCell that does not say
/// them too is another device.
const DISTINCT: [&str; 12] = [
    "hv", "lv", "rf", "lvt", "hvt", "nvt", "zvt", "esd", "iso", "5t", "ring", "cl",
];

/// A match of one PCell: its score, why, and a parameter the match fixes.
type Scored = (i32, String, Option<(String, String)>);

struct Candidate<'a> {
    library: &'a PcellLibrary,
    pcell: &'a Pcell,
    score: i32,
    reason: String,
    /// A parameter the match itself fixes (the choice that names the model).
    fixed: Option<(String, String)>,
}

/// The best PCell for a device, when one is convincing enough.
fn guess_pcell(device: &Device, symbol: &Symbol, libraries: &[PcellLibrary]) -> Option<PcellMatch> {
    let names: Vec<String> = [
        &device.name,
        &device.model,
        &symbol.template.get("model").cloned().unwrap_or_default(),
    ]
    .into_iter()
    .filter(|n| !n.is_empty())
    .map(|n| n.to_ascii_lowercase())
    .collect();
    let device_words: BTreeSet<String> = names.iter().flat_map(|n| words(n)).collect();
    let mut best: Option<Candidate> = None;
    for library in libraries.iter().filter(|l| l.name != "Basic") {
        for pcell in &library.pcells {
            let lower = pcell.name.to_ascii_lowercase();
            let mut candidate: Option<Scored> = None;
            // A choice of one of its parameters is the model.
            for parameter in &pcell.parameters {
                if let Some(choice) = parameter
                    .choices
                    .iter()
                    .find(|c| names.contains(&c.to_ascii_lowercase()))
                {
                    candidate = Some((
                        100,
                        format!(
                            "its parameter '{}' offers '{choice}', the device's model",
                            parameter.name
                        ),
                        Some((parameter.name.clone(), choice.clone())),
                    ));
                    break;
                }
            }
            if candidate.is_none() && names.contains(&lower) {
                candidate = Some((90, "the PCell has the device's name".into(), None));
            }
            if candidate.is_none() {
                let pcell_words = words(&pcell.name);
                let covered =
                    !pcell_words.is_empty() && pcell_words.iter().all(|w| device_words.contains(w));
                if covered {
                    let missing: Vec<&String> = device_words
                        .iter()
                        .filter(|w| DISTINCT.contains(&w.as_str()) && !pcell_words.contains(w))
                        // `lv` is the plain device where the PDK has `nmos` and `nmosHV`.
                        .filter(|w| w.as_str() != "lv")
                        .collect();
                    let score = 50 + 10 * pcell_words.len() as i32 - 30 * missing.len() as i32;
                    candidate = Some((
                        score,
                        format!("the PCell's name ({}) is part of the device's", pcell.name),
                        None,
                    ));
                }
            }
            let Some((mut score, reason, fixed)) = candidate else {
                continue;
            };
            // Parameters in common break ties.
            score += pcell
                .parameters
                .iter()
                .filter(|p| symbol.parameters.iter().any(|s| same_parameter(s, &p.name)))
                .count()
                .min(5) as i32;
            if best.as_ref().is_none_or(|b| score > b.score) {
                best = Some(Candidate {
                    library,
                    pcell,
                    score,
                    reason,
                    fixed,
                });
            }
        }
    }
    let best = best.filter(|b| b.score >= 50)?;
    let mut params: BTreeMap<String, String> = BTreeMap::new();
    for parameter in &best.pcell.parameters {
        if parameter.hidden
            || parameter.readonly
            || matches!(
                parameter.kind.as_str(),
                "shape" | "layer" | "callback" | "none"
            )
        {
            continue;
        }
        if let Some((name, value)) = &best.fixed
            && *name == parameter.name
        {
            params.insert(name.clone(), value.clone());
            continue;
        }
        // The symbol's parameter of that name (exact before alias); `model` and the
        // like are not sizes: only taken when the PCell has the very name.
        let found = symbol
            .parameters
            .iter()
            .find(|s| **s == parameter.name)
            .or_else(|| {
                symbol
                    .parameters
                    .iter()
                    .find(|s| same_parameter(s, &parameter.name))
            });
        if let Some(symbol_param) = found {
            let default = symbol
                .template
                .get(symbol_param)
                .cloned()
                .unwrap_or_default();
            // Expressions (`expr(…)`) cannot be carried over.
            if default.contains('(') || default.contains('@') {
                continue;
            }
            params.insert(
                parameter.name.clone(),
                param_template(symbol_param, &default, parameter),
            );
        }
    }
    Some(PcellMatch {
        library: best.library.name.clone(),
        cell: best.pcell.name.clone(),
        params,
        guess: true,
        reason: best.reason,
    })
}

/// The Magic generator of a device: the catalog's device of the model's name. Read
/// loosely (by field name), so that the Magic section can grow.
fn magic_generator(device: &Device, magic: &serde_json::Value) -> Option<GeneratorMatch> {
    let devices = magic.get("devices")?.as_array()?;
    let text = |d: &serde_json::Value, key: &str| {
        d.get(key)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string()
    };
    let full = |d: &serde_json::Value| {
        let (namespace, name) = (text(d, "namespace"), text(d, "name"));
        if namespace.is_empty() {
            name
        } else {
            format!("{namespace}::{name}")
        }
    };
    let wanted = [
        device.model.to_ascii_lowercase(),
        device.name.to_ascii_lowercase(),
    ];
    for (n, name) in wanted.iter().enumerate() {
        if name.is_empty() {
            continue;
        }
        if let Some(found) = devices
            .iter()
            .find(|d| text(d, "name").to_ascii_lowercase() == *name)
        {
            return Some(GeneratorMatch {
                name: full(found),
                guess: false,
                reason: if n == 0 {
                    "the generator has the model's name".into()
                } else {
                    "the generator has the symbol's name".into()
                },
            });
        }
    }
    // The generator's name ends the model's (`nfet_01v8` for `sky130_fd_pr__nfet_01v8`).
    devices
        .iter()
        .filter(|d| {
            let name = text(d, "name").to_ascii_lowercase();
            name.len() > 3
                && wanted
                    .iter()
                    .any(|w| w.ends_with(&format!("__{name}")) || name.ends_with(&format!("__{w}")))
        })
        .max_by_key(|d| text(d, "name").len())
        .map(|found| GeneratorMatch {
            name: full(found),
            guess: true,
            reason: "the generator's name is the model's without its prefix".into(),
        })
}

// ---------------------------------------------------------------------------
// The scan
// ---------------------------------------------------------------------------

/// Runs after every other section; `tech` and `catalog` hold their results.
pub fn scan(
    ctx: &ScanContext,
    tech: &Tech,
    catalog: &Catalog,
) -> Scanned<DevicesTech, DevicesCatalog> {
    let mut out: Scanned<DevicesTech, DevicesCatalog> = Scanned::default();
    let primitives: Vec<&str> = tech
        .xschem
        .libraries
        .iter()
        .filter(|l| l.role == "primitives")
        .map(|l| l.name.as_str())
        .collect();
    if primitives.is_empty() {
        out.section.pcells = "none".into();
        if !tech.xschem.libraries.is_empty() {
            out.notes.push(
                "devices: no xschem library has the role \"primitives\", so no devices are listed"
                    .into(),
            );
        }
        return out;
    }

    // The layout side: the shipped map, else guesses.
    let mut shipped = map::DeviceMap::default();
    let mut shipped_ok = false;
    if let Some(file) = map::pdk_map(&ctx.pdk.name) {
        match shipped.add_file(&file) {
            Ok(()) => shipped_ok = true,
            Err(e) => out.notes.push(format!(
                "devices: the shipped device map was not read: {e:#}"
            )),
        }
    }
    let pcell_libraries = &catalog.klayout.pcell_libraries;
    let has_pcells = pcell_libraries
        .iter()
        .any(|l| l.name != "Basic" && !l.pcells.is_empty());
    let magic = serde_json::to_value(&catalog.magic).unwrap_or_default();

    for symbol in catalog
        .xschem
        .symbols
        .iter()
        .filter(|s| primitives.contains(&s.library.as_str()) && is_device(s))
    {
        let prefix = tech
            .xschem
            .references
            .get(&symbol.library)
            .cloned()
            .unwrap_or_default();
        // A schematic of a CDePT project names the library folder (the PDK's own
        // search path also takes the bare name).
        let reference = if prefix.is_empty() {
            format!("{}.sym", symbol.name)
        } else {
            format!("{prefix}{}.sym", symbol.name)
        };
        let mut device = Device {
            name: symbol.name.clone(),
            symbol: reference,
            library: symbol.library.clone(),
            kind: symbol.kind.clone(),
            model: symbol.model.clone(),
            prefix: symbol.prefix.clone(),
            terminals: symbol.pins.clone(),
            parameters: symbol
                .parameters
                .iter()
                .filter(|p| !matches!(p.as_str(), "model" | "spiceprefix"))
                .map(|p| {
                    let default = symbol.template.get(p).cloned().unwrap_or_default();
                    DeviceParameter {
                        unit: unit_of(p, &default),
                        name: p.clone(),
                        default,
                    }
                })
                .collect(),
            ..Device::default()
        };
        // The models: which set defines it, and through which library.
        let mut model_class = String::new();
        if !device.model.is_empty() {
            for (name, set) in &catalog.models.sets {
                let Some(def) = set.definition(&device.model) else {
                    continue;
                };
                let through = if def.library.is_empty() {
                    def.file
                        .as_str()
                        .rsplit('/')
                        .next()
                        .unwrap_or_default()
                        .to_string()
                } else {
                    def.library.clone()
                };
                device.models.insert(name.clone(), through);
                if let Some(simulator) = tech.models.sets.get(name).map(|s| s.simulator.clone())
                    && !device.simulators.contains(&simulator)
                {
                    device.simulators.push(simulator);
                }
                if model_class.is_empty() || tech.models.default.as_deref() == Some(name) {
                    model_class = def.class.clone();
                }
                if def.kind == "subckt"
                    && !def.pins.is_empty()
                    && def.pins.len() != device.terminals.len()
                    && device.model_pins.is_empty()
                {
                    device.model_pins = def.pins.clone();
                }
            }
        }
        device.class = class_of(symbol, &model_class);

        if shipped_ok {
            if let Some(entry) = shipped.lookup(&device.symbol) {
                device.pcell = Some(PcellMatch {
                    library: entry.library.clone(),
                    cell: entry.cell.clone(),
                    params: entry.params.clone(),
                    guess: false,
                    reason: "device map shipped with CDePT".into(),
                });
            }
        } else if has_pcells {
            device.pcell = guess_pcell(&device, symbol, pcell_libraries);
        }
        device.magic = magic_generator(&device, &magic);
        out.catalog.devices.push(device);
    }

    let devices = &out.catalog.devices;
    let section = &mut out.section;
    section.count = devices.len();
    section.with_pcell = devices.iter().filter(|d| d.pcell.is_some()).count();
    section.guessed = devices
        .iter()
        .filter(|d| d.pcell.as_ref().is_some_and(|p| p.guess))
        .count();
    section.with_magic = devices.iter().filter(|d| d.magic.is_some()).count();
    section.without_model = devices.iter().filter(|d| d.models.is_empty()).count();
    for device in devices {
        *section.classes.entry(device.class.clone()).or_default() += 1;
    }
    section.pcells = if shipped_ok {
        "map"
    } else if section.guessed > 0 {
        "guess"
    } else {
        "none"
    }
    .into();

    if devices.is_empty() {
        out.notes
            .push("devices: no device symbol found in the primitives libraries".into());
    } else if !shipped_ok {
        if !has_pcells {
            out.notes.push(format!(
                "devices: CDePT ships no device map for {} and the scan has no KLayout PCells \
                 to match (KLayout was not run, or the technology has none): layouts cannot be \
                 generated from schematics until a device map is written (docs/LINKING.md)",
                ctx.pdk.name
            ));
        } else {
            out.notes.push(format!(
                "devices: CDePT ships no device map for {}; {} of {} devices were matched to \
                 KLayout PCells by guessing. Write the draft with `cdept tech devices`, check \
                 it, and name it in [klayout] devices",
                ctx.pdk.name, section.guessed, section.count
            ));
        }
    }
    if section.without_model > 0 && !catalog.models.sets.is_empty() {
        let names: Vec<&str> = devices
            .iter()
            .filter(|d| d.models.is_empty())
            .map(|d| d.name.as_str())
            .take(8)
            .collect();
        out.notes.push(format!(
            "devices: {} device(s) netlist as a model no model library defines ({}{})",
            section.without_model,
            names.join(", "),
            if section.without_model > names.len() {
                ", …"
            } else {
                ""
            }
        ));
    }
    out
}

// ---------------------------------------------------------------------------
// A draft device map
// ---------------------------------------------------------------------------

fn toml_string(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

fn toml_key(text: &str) -> String {
    if !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        text.to_string()
    } else {
        toml_string(text)
    }
}

/// Pin layers guessed from the layer names: a conductor with a `pin` and a `label`
/// (or `text`) purpose of the same base name.
fn guess_pin_layers(catalog: &Catalog) -> Vec<(String, String, Option<String>, Option<String>)> {
    // base name (lower) → purpose → "layer/datatype"
    let mut by_base: Vec<(String, BTreeMap<String, String>)> = Vec::new();
    for layer in &catalog.klayout.layers {
        let name = layer.name.trim();
        let (base, purpose) = match name.split_once('.') {
            Some((base, purpose)) => (base.trim(), purpose.trim()),
            None => (name, "drawing"),
        };
        let purpose = purpose
            .split(|c: char| !c.is_ascii_alphanumeric())
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if base.is_empty() || base.contains(' ') {
            continue;
        }
        let key = base.to_string();
        let place = format!("{}/{}", layer.layer, layer.datatype);
        match by_base.iter_mut().find(|(b, _)| *b == key) {
            Some((_, purposes)) => {
                purposes.entry(purpose).or_insert(place);
            }
            None => by_base.push((key, BTreeMap::from([(purpose, place)]))),
        }
    }
    let conductor = |base: &str| {
        let lower = base.to_ascii_lowercase();
        let numbered = |prefix: &str| {
            lower
                .strip_prefix(prefix)
                .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()))
        };
        numbered("met")
            || numbered("metal")
            || numbered("topmetal")
            || numbered("li")
            || numbered("m")
    };
    by_base
        .into_iter()
        .filter(|(base, _)| conductor(base))
        .filter_map(|(base, purposes)| {
            let drawing = purposes.get("drawing")?.clone();
            let pin = purposes.get("pin").cloned();
            let label = purposes
                .get("label")
                .or_else(|| purposes.get("text"))
                .cloned();
            (pin.is_some() || label.is_some()).then_some((base, drawing, pin, label))
        })
        .collect()
}

/// A device map (`cdept-devices` version 1, docs/LINKING.md §3) drafted from the
/// scan: an entry for every device with a KLayout PCell, each with the reason of the
/// match; devices without one are listed as comments. It is a starting point to
/// check by hand: save it in the project and name it in `[klayout] devices`.
pub fn draft_map(tech: &Tech, catalog: &Catalog) -> String {
    let pdk = &tech.pdk.name;
    let mut out = String::new();
    out.push_str(&format!(
        "# DRAFT device map for {pdk}, made by CDePT from a scan of the PDK.\n\
         # Which KLayout PCell an xschem symbol becomes when a layout is generated from a\n\
         # schematic (docs/LINKING.md §3). The entries were matched by name and by\n\
         # parameters: CHECK EVERY ONE before relying on it, then save the file in the\n\
         # project and name it in cdept.toml:  [klayout] devices = \"devices.toml\"\n\
         # Templates: @attr is the instance's attribute, @{{attr|u}} that value written in\n\
         # micron with \"u\".\n"
    ));
    out.push_str(&format!(
        "format = \"{}\"\nversion = {}\n",
        map::FORMAT,
        map::VERSION
    ));

    let pins = guess_pin_layers(catalog);
    if !pins.is_empty() {
        out.push_str(
            "\n# Pins (docs/LINKING.md §9), guessed from the layer names: per conductor its pin\n\
             # layer and the layer its name goes on as a text. Check against the LVS deck.\n\
             [pins]\n",
        );
        // Generated pins go on the first metal (not on local interconnect).
        let first = pins
            .iter()
            .find(|(base, ..)| base.to_ascii_lowercase().starts_with('m'))
            .unwrap_or(&pins[0]);
        out.push_str(&format!("generate = {}\n", toml_string(&first.1)));
        out.push_str("layers = [\n");
        for (base, drawing, pin, label) in &pins {
            let mut fields = vec![format!("drawing = {}", toml_string(drawing))];
            if let Some(pin) = pin {
                fields.push(format!("pin = {}", toml_string(pin)));
            }
            if let Some(label) = label {
                fields.push(format!("label = {}", toml_string(label)));
            }
            out.push_str(&format!("  {{ {} }},  # {base}\n", fields.join(", ")));
        }
        out.push_str("]\n");
    }

    let mut without: Vec<&Device> = Vec::new();
    for device in &catalog.devices.devices {
        let Some(pcell) = &device.pcell else {
            without.push(device);
            continue;
        };
        out.push_str(&format!(
            "\n# {} ({}): {}{}\n[[device]]\nsymbol = {}\nlibrary = {}\ncell = {}\n",
            device.name,
            device.class,
            if pcell.guess { "guess: " } else { "" },
            pcell.reason,
            toml_string(&device.symbol),
            toml_string(&pcell.library),
            toml_string(&pcell.cell),
        ));
        if !pcell.params.is_empty() {
            let params: Vec<String> = pcell
                .params
                .iter()
                .map(|(k, v)| format!("{} = {}", toml_key(k), toml_string(v)))
                .collect();
            out.push_str(&format!("params = {{ {} }}\n", params.join(", ")));
        }
    }
    if !without.is_empty() {
        out.push_str(
            "\n# Devices without a PCell match (add entries by hand where a PCell exists):\n",
        );
        for device in without {
            out.push_str(&format!(
                "#   {} ({}), model {}\n",
                device.symbol,
                device.class,
                if device.model.is_empty() {
                    "-"
                } else {
                    &device.model
                }
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::klayout::{KlayoutCatalog, Layer};
    use super::super::models::{ModelDef, ModelSet, ModelSetCatalog};
    use super::super::pdk::{PdkDir, TechLibrary, TechPath};
    use super::*;

    fn symbol(name: &str, kind: &str, model: &str, params: &[(&str, &str)]) -> Symbol {
        Symbol {
            library: "toy_pr".into(),
            name: name.into(),
            kind: kind.into(),
            model: model.into(),
            format: "@spiceprefix@name @pinlist @model".into(),
            template: params
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            pins: vec!["D".into(), "G".into(), "S".into(), "B".into()],
            parameters: params.iter().map(|(k, _)| k.to_string()).collect(),
            prefix: "X".into(),
            ..Symbol::default()
        }
    }

    fn parameter(name: &str, kind: &str, default: &str, choices: &[&str]) -> PcellParameter {
        PcellParameter {
            name: name.into(),
            kind: kind.into(),
            default: default.into(),
            choices: choices.iter().map(|c| c.to_string()).collect(),
            ..PcellParameter::default()
        }
    }

    fn toy() -> (Tech, Catalog) {
        let mut tech = Tech::default();
        tech.pdk.name = "toy130".into();
        tech.xschem.libraries.push(TechLibrary {
            name: "toy_pr".into(),
            path: TechPath("$PDK_ROOT/toy130/libs.tech/xschem/toy_pr".into()),
            layout: "flat".into(),
            role: "primitives".into(),
            description: None,
            enabled: true,
        });
        tech.xschem
            .references
            .insert("toy_pr".into(), "toy_pr/".into());
        tech.models.default = Some("ngspice".into());
        tech.models.sets.insert(
            "ngspice".into(),
            ModelSet {
                simulator: "ngspice".into(),
                ..ModelSet::default()
            },
        );
        let mut catalog = Catalog::default();
        catalog.xschem.symbols = vec![
            symbol(
                "nfet_01v8",
                "nmos",
                "toy__nfet_01v8",
                &[
                    ("W", "1"),
                    ("L", "0.15"),
                    ("nf", "1"),
                    ("mult", "1"),
                    ("model", "nfet_01v8"),
                ],
            ),
            symbol(
                "lv_nmos",
                "nmos",
                "lv_nmos",
                &[("w", "0.15u"), ("l", "0.13u"), ("ng", "1")],
            ),
            symbol(
                "hv_nmos",
                "nmos",
                "hv_nmos",
                &[("w", "0.3u"), ("l", "0.45u"), ("ng", "1")],
            ),
            symbol("oddball", "subcircuit", "oddball", &[]),
            Symbol {
                kind: "netlist_commands".into(),
                ..symbol("corner", "", "", &[])
            },
        ];
        catalog.models.sets.insert(
            "ngspice".into(),
            ModelSetCatalog {
                definitions: vec![
                    ModelDef {
                        name: "toy__nfet_01v8".into(),
                        kind: "subckt".into(),
                        pins: vec!["d".into(), "g".into(), "s".into(), "b".into()],
                        library: "toy.lib.spice".into(),
                        class: "all".into(),
                        ..ModelDef::default()
                    },
                    ModelDef {
                        name: "HV_NMOS".into(),
                        kind: "subckt".into(),
                        pins: vec!["d".into(), "g".into(), "s".into()],
                        library: "cornerMOShv.lib".into(),
                        class: "mos_hv".into(),
                        ..ModelDef::default()
                    },
                ],
                ..ModelSetCatalog::default()
            },
        );
        catalog.klayout = KlayoutCatalog {
            layers: vec![
                Layer {
                    name: "met1.drawing".into(),
                    layer: 68,
                    datatype: 20,
                    ..Layer::default()
                },
                Layer {
                    name: "met1.pin".into(),
                    layer: 68,
                    datatype: 16,
                    ..Layer::default()
                },
                Layer {
                    name: "met1.label".into(),
                    layer: 68,
                    datatype: 5,
                    ..Layer::default()
                },
                Layer {
                    name: "poly.drawing".into(),
                    layer: 66,
                    datatype: 20,
                    ..Layer::default()
                },
            ],
            pcell_libraries: vec![PcellLibrary {
                name: "toylib".into(),
                pcells: vec![
                    Pcell {
                        name: "nfet".into(),
                        parameters: vec![
                            parameter(
                                "type",
                                "list",
                                "toy__nfet_01v8",
                                &["toy__nfet_01v8", "toy__nfet_lvt"],
                            ),
                            parameter("w", "double", "0.42", &[]),
                            parameter("l", "double", "0.15", &[]),
                            parameter("nf", "int", "1", &[]),
                        ],
                        ..Pcell::default()
                    },
                    Pcell {
                        name: "nmos".into(),
                        parameters: vec![
                            parameter("w", "string", "0.15u", &[]),
                            parameter("l", "string", "0.13u", &[]),
                            parameter("ng", "string", "1", &[]),
                        ],
                        ..Pcell::default()
                    },
                    Pcell {
                        name: "nmosHV".into(),
                        parameters: vec![parameter("w", "string", "0.3u", &[])],
                        ..Pcell::default()
                    },
                ],
                ..PcellLibrary::default()
            }],
            ..KlayoutCatalog::default()
        };
        catalog.magic = serde_json::from_value(serde_json::json!({})).unwrap();
        (tech, catalog)
    }

    #[test]
    fn devices_are_joined_and_guessed() {
        let (tech, catalog) = toy();
        let ctx = ScanContext::new(PdkDir::new("/nowhere", "toy130")).files_only();
        let scanned = scan(&ctx, &tech, &catalog);
        let devices = &scanned.catalog.devices;
        let names: Vec<&str> = devices.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["nfet_01v8", "lv_nmos", "hv_nmos", "oddball"]);

        let nfet = &devices[0];
        assert_eq!(nfet.symbol, "toy_pr/nfet_01v8.sym");
        assert_eq!(nfet.class, "mos");
        assert_eq!(nfet.models["ngspice"], "toy.lib.spice");
        assert_eq!(nfet.simulators, ["ngspice"]);
        assert_eq!(nfet.parameters[0].unit, "um");
        let pcell = nfet.pcell.as_ref().unwrap();
        assert_eq!(
            (pcell.library.as_str(), pcell.cell.as_str()),
            ("toylib", "nfet")
        );
        assert!(
            pcell.guess && pcell.reason.contains("offers"),
            "{}",
            pcell.reason
        );
        assert_eq!(pcell.params["type"], "toy__nfet_01v8");
        assert_eq!(pcell.params["w"], "@W");
        assert_eq!(pcell.params["nf"], "@nf");

        let lv = &devices[1];
        let pcell = lv.pcell.as_ref().unwrap();
        assert_eq!(pcell.cell, "nmos");
        assert_eq!(pcell.params["w"], "@{w|u}");
        assert_eq!(pcell.params["ng"], "@ng");
        assert_eq!(lv.parameters[0].unit, "m");
        assert!(lv.models.is_empty());

        let hv = &devices[2];
        assert_eq!(hv.pcell.as_ref().unwrap().cell, "nmosHV");
        assert_eq!(hv.class, "mos_hv");
        assert_eq!(hv.model_pins, ["d", "g", "s"]);
        assert!(devices[3].pcell.is_none());

        let s = &scanned.section;
        assert_eq!(
            (s.count, s.with_pcell, s.guessed, s.without_model),
            (4, 3, 3, 2)
        );
        assert_eq!(s.pcells, "guess");
        assert!(scanned.notes.iter().any(|n| n.contains("guessing")));
        assert!(scanned.notes.iter().any(|n| n.contains("lv_nmos")));
    }

    #[test]
    fn the_draft_is_a_device_map() {
        let (mut tech, mut catalog) = toy();
        let ctx = ScanContext::new(PdkDir::new("/nowhere", "toy130")).files_only();
        let scanned = scan(&ctx, &tech, &catalog);
        tech.devices = scanned.section;
        catalog.devices = scanned.catalog;
        let text = draft_map(&tech, &catalog);
        assert!(text.contains("DRAFT"));
        let mut parsed = map::DeviceMap::default();
        parsed
            .add_text(&text)
            .unwrap_or_else(|e| panic!("{e:#}\n{text}"));
        assert_eq!(parsed.devices.len(), 3);
        let nfet = parsed.lookup("toy_pr/nfet_01v8.sym").unwrap();
        assert_eq!(nfet.cell, "nfet");
        assert_eq!(nfet.params["type"], "toy__nfet_01v8");
        assert_eq!(parsed.pins.generate.as_deref(), Some("68/20"));
        assert_eq!(parsed.pins.layers.len(), 1);
        assert_eq!(parsed.pins.layers[0].pin.as_deref(), Some("68/16"));
        assert_eq!(parsed.pins.layers[0].label.as_deref(), Some("68/5"));
        assert!(text.contains("#   toy_pr/oddball.sym"));
    }

    #[test]
    fn names_into_words() {
        assert_eq!(words("nmosHV"), ["nmos", "hv"]);
        assert_eq!(words("sg13_lv_nmos"), ["sg13", "lv", "nmos"]);
        assert_eq!(words("npn13G2"), ["npn13g2"]);
        assert!(same_parameter("W", "w") && same_parameter("ng", "nf"));
        assert!(!same_parameter("w", "l"));
        // A symbol in metres, a PCell in micron: the template writes plain micron.
        let pcell = parameter("w", "double", "0.22", &[]);
        assert_eq!(param_template("W", "0.22u", &pcell), "@{W|um}");
        let value = |_: &str| Some("0.5e-6".to_string());
        assert_eq!(map::expand("@{W|um}", &value).unwrap(), "0.5");
    }

    #[test]
    fn magic_generators_by_name() {
        let device = Device {
            name: "nfet_01v8".into(),
            model: "sky130_fd_pr__nfet_01v8".into(),
            ..Device::default()
        };
        let magic = serde_json::json!({ "devices": [
            { "name": "sky130_fd_pr__pfet_01v8", "namespace": "sky130" },
            { "name": "sky130_fd_pr__nfet_01v8", "namespace": "sky130" },
        ]});
        let found = magic_generator(&device, &magic).unwrap();
        assert_eq!(found.name, "sky130::sky130_fd_pr__nfet_01v8");
        assert!(!found.guess);
        assert!(magic_generator(&device, &serde_json::json!({})).is_none());
    }
}
