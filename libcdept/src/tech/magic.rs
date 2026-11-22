//! Magic section of the technology snapshot: `libs.tech/magic`.
//!
//! Read from the files, magic itself is not started: the start-up file (`.magicrc`:
//! the technology it loads, the Tcl it sources, the supply names, the cell paths),
//! the technology file (`.tech` and what it includes: name and version, planes and
//! layer types, the CIF/GDS, extraction and DRC styles, GDS numbers, the devices the
//! extraction knows), the device generators of the PDK's Tcl (`<name>_defaults`
//! procedures) and the auxiliary scripts next to them.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::pdk::{PdkDir, ScanContext, Scanned, TechPath};

/// `[magic]` in `tech.toml`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MagicTech {
    /// The start-up file (`<pdk>.magicrc`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rc: Option<TechPath>,
    /// The technology name magic knows it by (the `tech` line of a `.mag` file).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tech: Option<String>,
    /// The technology file (`<pdk>.tech`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tech_file: Option<TechPath>,
    /// The version the technology file states.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The magic the technology file asks for (`magic-8.3.617`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requires: Option<String>,
    /// Files the technology file includes.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub includes: Vec<TechPath>,
    /// The Tcl the start-up file sources: device generators, menus.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tcl: Option<TechPath>,
    /// Internal units per lambda as the styles state it (`scalefactor 10 nanometers`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale: Option<String>,
    /// Supply and substrate names the start-up file sets (`VDD`, `GND`, `SUB`).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub globals: BTreeMap<String, String>,
    /// Settings the start-up file makes (`drc euclidean on`, `ext2spice scale off`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub settings: Vec<String>,
    /// Cell search paths the start-up file adds, those that exist.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cell_paths: Vec<TechPath>,
    /// GDS/CIF output styles, the default first.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cifoutput: Vec<String>,
    /// GDS/CIF input styles, the default first.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cifinput: Vec<String>,
    /// Extraction styles, the default first.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub extract: Vec<String>,
    /// DRC styles, the default first.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub drc: Vec<String>,
    /// Auxiliary files next to the technology: antenna and density checks, fill and
    /// seal ring generators, key bindings, a technology of the vendor's GDS layers.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub scripts: Vec<Script>,
}

/// An auxiliary file of the PDK's magic folder.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Script {
    /// `antenna`, `density`, `fill`, `seal-ring`, `drc`, `bind-keys`, `bump-bond`,
    /// `gds-tech`.
    pub role: String,
    pub file: TechPath,
}

/// The Magic part of the catalog.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MagicCatalog {
    pub planes: Vec<Plane>,
    /// The layer types of the technology.
    pub layers: Vec<MagicLayer>,
    /// Names for sets of types (`allnfets`).
    pub aliases: Vec<TypeAlias>,
    /// Every style with its section: `cifoutput`, `cifinput`, `extract`, `drc`.
    pub styles: Vec<Style>,
    /// GDS numbers per CIF layer, for output and for input.
    pub gds: Vec<GdsLayer>,
    /// The layers LEF names (`lef` section).
    pub lef: Vec<LefLayer>,
    /// The devices the extraction recognises and the model each is written as.
    pub extracted: Vec<ExtractedDevice>,
    /// The device generators of the PDK's Tcl.
    pub devices: Vec<MagicDevice>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Plane {
    pub name: String,
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MagicLayer {
    pub name: String,
    pub aliases: Vec<String>,
    pub plane: String,
    /// Written with a leading `-` in the technology file: not offered for drawing.
    pub locked: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TypeAlias {
    pub name: String,
    pub types: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Style {
    /// `cifoutput`, `cifinput`, `extract`, `drc`.
    pub section: String,
    /// As magic names it: `gdsii`, `ngspice(lvs)`.
    pub name: String,
    /// The first of its section.
    pub default: bool,
    /// `scalefactor` of a CIF style, `lambda`/`units` of an extraction style.
    pub scale: String,
}

/// GDS numbers of a CIF layer in one style.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GdsLayer {
    /// `output` or `input`.
    pub direction: String,
    /// The style without variant (`gdsii`, `sky130`).
    pub style: String,
    /// The CIF layer name (`NWELL`).
    pub name: String,
    pub layer: i32,
    pub datatype: i32,
    /// The magic types: written out from (output), read into (input).
    pub types: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LefLayer {
    /// `routing`, `cut`, `obs`, `masterslice`, …
    pub kind: String,
    /// The magic type.
    pub layer: String,
    /// The names LEF/DEF files use for it.
    pub names: Vec<String>,
}

/// A `device` line of the extraction.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExtractedDevice {
    /// The style it was first seen in, without variant.
    pub style: String,
    /// `mosfet`, `msubcircuit`, `rsubcircuit`, `csubcircuit`, `dsubcircuit`,
    /// `subcircuit`, `resistor`, `capacitor`, `diode`, `bjt`, `veriloga`, …
    pub kind: String,
    /// The model or subcircuit it is written as (`sg13_lv_nmos`).
    pub model: String,
    /// The magic type that makes it (`nfet`).
    pub layer: String,
    /// The rest of the line: terminals, substrate, parameters.
    pub terminals: String,
}

/// A device generator: `<namespace>::<name>_defaults` and friends.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MagicDevice {
    /// The device, usually the model name (`sg13_lv_nmos`, `sky130_fd_pr__nfet_01v8`).
    pub name: String,
    /// The Tcl namespace (`sg13g2`, `sky130`): `magic::gencell <namespace>::<name>`.
    pub namespace: String,
    /// The label of its entry in magic's device menu, when it has one.
    pub description: String,
    /// The `class` of its defaults (`mosfet`, `resistor`, …).
    pub class: String,
    /// Devices the dialog lets it be changed into.
    pub compatible: Vec<String>,
    pub parameters: Vec<MagicParameter>,
    /// There is a `<name>_draw` procedure: it can be generated.
    pub draw: bool,
    /// The file the defaults are in.
    pub file: TechPath,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MagicParameter {
    pub name: String,
    /// The default, as text.
    pub default: String,
}

// -- the technology file --------------------------------------------------------

/// What a technology file (with its includes) says.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TechFile {
    pub name: String,
    pub format: String,
    pub version: String,
    pub description: String,
    pub requires: String,
    pub includes: Vec<PathBuf>,
    pub catalog: MagicCatalog,
}

const SECTIONS: [&str; 21] = [
    "tech",
    "version",
    "planes",
    "types",
    "contact",
    "aliases",
    "styles",
    "compose",
    "connect",
    "cifoutput",
    "cifinput",
    "mzrouter",
    "drc",
    "extract",
    "wiring",
    "router",
    "plowing",
    "plot",
    "lef",
    "images",
    "specialtypes",
];

/// The logical lines of a technology file: comments dropped, `\` continuations
/// joined, `include`d files read in place.
fn logical_lines(file: &Path, depth: usize, includes: &mut Vec<PathBuf>, out: &mut Vec<String>) {
    let Ok(text) = std::fs::read_to_string(file) else {
        return;
    };
    let mut pending = String::new();
    for raw in text.lines() {
        let line = raw.trim_end();
        if pending.is_empty() && line.trim_start().starts_with('#') {
            continue;
        }
        if let Some(head) = line.strip_suffix('\\') {
            pending.push_str(head);
            pending.push(' ');
            continue;
        }
        pending.push_str(line);
        let logical = std::mem::take(&mut pending);
        let trimmed = logical.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(name) = trimmed.strip_prefix("include ")
            && depth < 6
        {
            let name = name.trim().trim_matches('"');
            let folder = file.parent().unwrap_or(Path::new("."));
            let found = [folder.join(name), folder.join(format!("{name}.tech"))]
                .into_iter()
                .find(|p| p.is_file());
            if let Some(found) = found {
                includes.push(found.clone());
                logical_lines(&found, depth + 1, includes, out);
            }
            continue;
        }
        out.push(trimmed.to_string());
    }
}

/// `name variants (),(lvs)` → [`name`, `name(lvs)`]; without variants, [`name`].
fn style_names(words: &[&str]) -> Vec<String> {
    let Some(name) = words.first() else {
        return Vec::new();
    };
    let variants = match words.get(1) {
        Some(&"variants") => words[2..].concat(),
        _ => return vec![name.to_string()],
    };
    variants
        .split(',')
        .map(|v| v.trim().trim_start_matches('(').trim_end_matches(')'))
        .map(|v| {
            if v.is_empty() {
                name.to_string()
            } else {
                format!("{name}({v})")
            }
        })
        .collect()
}

/// Read a technology file and the files it includes.
pub fn read_tech(file: &Path) -> TechFile {
    let mut tech = TechFile::default();
    let mut lines = Vec::new();
    logical_lines(file, 0, &mut tech.includes, &mut lines);
    parse_tech(&lines, &mut tech);
    tech
}

fn parse_tech(lines: &[String], tech: &mut TechFile) {
    let catalog = &mut tech.catalog;
    let mut section = "";
    // CIF style being read, without variant, and the layer the next `calma` is for.
    let mut style = String::new();
    let mut cif_layer: Option<(String, String)> = None;
    // CIF layer name → magic types read from it (cifinput).
    let mut read_into: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    let mut seen_gds: BTreeSet<(String, String, String, i32, i32)> = BTreeSet::new();
    let mut seen_devices: BTreeSet<(String, String, String)> = BTreeSet::new();
    let mut input_calma: Vec<GdsLayer> = Vec::new();

    for line in lines {
        let words: Vec<&str> = line.split_whitespace().collect();
        let Some(&first) = words.first() else {
            continue;
        };
        if section.is_empty() {
            if words.len() == 1
                && let Some(name) = SECTIONS.iter().find(|s| **s == first)
            {
                section = name;
                style.clear();
                cif_layer = None;
            }
            continue;
        }
        if words == ["end"] {
            section = "";
            continue;
        }
        match section {
            "tech" => match first {
                "format" => tech.format = words.get(1).unwrap_or(&"").to_string(),
                name if tech.name.is_empty() => tech.name = name.to_string(),
                _ => {}
            },
            "version" => {
                let rest = line[first.len()..].trim().trim_matches('"').to_string();
                match first {
                    "version" => tech.version = rest,
                    "description" => tech.description = rest,
                    "requires" => tech.requires = rest,
                    _ => {}
                }
            }
            "planes" => {
                let mut names = first.split(',').map(str::to_string);
                if let Some(name) = names.next() {
                    catalog.planes.push(Plane {
                        name,
                        aliases: names.collect(),
                    });
                }
            }
            "types" => {
                let (Some(plane), Some(names)) = (words.first(), words.get(1)) else {
                    continue;
                };
                let mut names = names.split(',').map(str::to_string);
                if let Some(name) = names.next() {
                    catalog.layers.push(MagicLayer {
                        name,
                        aliases: names.collect(),
                        plane: plane.trim_start_matches('-').to_string(),
                        locked: plane.starts_with('-'),
                    });
                }
            }
            "aliases" => {
                if words.len() >= 2 {
                    catalog.aliases.push(TypeAlias {
                        name: first.to_string(),
                        types: words[1..].concat(),
                    });
                }
            }
            "cifoutput" | "cifinput" | "extract" | "drc" => match first {
                "style" => {
                    let names = style_names(&words[1..]);
                    style = words.get(1).unwrap_or(&"").to_string();
                    cif_layer = None;
                    for name in names {
                        if catalog
                            .styles
                            .iter()
                            .any(|s| s.section == section && s.name == name)
                        {
                            continue;
                        }
                        let default = !catalog.styles.iter().any(|s| s.section == section);
                        catalog.styles.push(Style {
                            section: section.to_string(),
                            name,
                            default,
                            scale: String::new(),
                        });
                    }
                }
                "scalefactor" | "lambda" | "units" => {
                    let text = match first {
                        "scalefactor" => words[1..].join(" "),
                        other => format!("{other} {}", words[1..].join(" ")),
                    };
                    for s in catalog.styles.iter_mut().filter(|s| {
                        s.section == section
                            && (s.name == style || s.name.starts_with(&format!("{style}(")))
                    }) {
                        if first == "scalefactor" || s.scale.is_empty() {
                            s.scale = text.clone();
                        } else if !s.scale.contains(first) {
                            s.scale = format!("{}, {text}", s.scale);
                        }
                    }
                }
                "layer" | "templayer" | "labels" if section.starts_with("cif") => {
                    let name = words.get(1).unwrap_or(&"").to_string();
                    let operand = words.get(2).unwrap_or(&"").to_string();
                    if section == "cifoutput" {
                        if first != "labels" {
                            cif_layer = Some((name, operand));
                        }
                    } else if first == "layer" {
                        // cifinput: `layer <magic type> <CIF layers>`.
                        for cif in operand.split(',').filter(|c| !c.is_empty()) {
                            let types = read_into
                                .entry((style.clone(), cif.to_string()))
                                .or_default();
                            if !types.contains(&name) {
                                types.push(name.clone());
                            }
                        }
                    }
                }
                "calma" if section == "cifoutput" => {
                    let (Some((name, types)), Some(layer), Some(datatype)) = (
                        cif_layer.clone(),
                        words.get(1).and_then(|w| w.parse::<i32>().ok()),
                        words.get(2).and_then(|w| w.parse::<i32>().ok()),
                    ) else {
                        continue;
                    };
                    if seen_gds.insert((
                        "output".to_string(),
                        style.clone(),
                        name.clone(),
                        layer,
                        datatype,
                    )) {
                        catalog.gds.push(GdsLayer {
                            direction: "output".to_string(),
                            style: style.clone(),
                            name,
                            layer,
                            datatype,
                            types,
                        });
                    }
                }
                "calma" if section == "cifinput" => {
                    let (Some(name), Some(layer), Some(datatype)) = (
                        words.get(1),
                        words.get(2).and_then(|w| w.parse::<i32>().ok()),
                        words.get(3).and_then(|w| w.parse::<i32>().ok()),
                    ) else {
                        continue;
                    };
                    if seen_gds.insert((
                        "input".to_string(),
                        style.clone(),
                        name.to_string(),
                        layer,
                        datatype,
                    )) {
                        input_calma.push(GdsLayer {
                            direction: "input".to_string(),
                            style: style.clone(),
                            name: name.to_string(),
                            layer,
                            datatype,
                            types: String::new(),
                        });
                    }
                }
                "device" if section == "extract" => {
                    let (Some(kind), Some(model), Some(layer)) =
                        (words.get(1), words.get(2), words.get(3))
                    else {
                        continue;
                    };
                    if seen_devices.insert((kind.to_string(), model.to_string(), layer.to_string()))
                    {
                        catalog.extracted.push(ExtractedDevice {
                            style: style.clone(),
                            kind: kind.to_string(),
                            model: model.to_string(),
                            layer: layer.to_string(),
                            terminals: words[4..].join(" "),
                        });
                    }
                }
                _ => {}
            },
            "lef"
                if words.len() >= 3
                    && matches!(
                        first,
                        "routing" | "route" | "cut" | "contact" | "obs" | "masterslice" | "overlap"
                    ) =>
            {
                catalog.lef.push(LefLayer {
                    kind: first.to_string(),
                    layer: words[1].to_string(),
                    names: words[2..].iter().map(|w| w.to_string()).collect(),
                });
            }
            _ => {}
        }
    }
    for mut gds in input_calma {
        if let Some(types) = read_into.get(&(gds.style.clone(), gds.name.clone())) {
            gds.types = types.join(",");
        }
        catalog.gds.push(gds);
    }
}

// -- the start-up file ------------------------------------------------------------

/// What a `.magicrc` says.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MagicRc {
    /// The technology file it loads.
    pub tech_file: Option<PathBuf>,
    /// The technology name it checks for (`if {[tech name] != "…"}`).
    pub tech_name: Option<String>,
    /// The Tcl files it sources.
    pub sources: Vec<PathBuf>,
    pub globals: BTreeMap<String, String>,
    pub settings: Vec<String>,
    pub cell_paths: Vec<PathBuf>,
}

/// A path as a `.magicrc` writes it, on this machine: the folder of the script and
/// the PDK variables filled in. `None` when something else is left in it.
fn rc_path(
    text: &str,
    rc: &Path,
    pdk: &PdkDir,
    variables: &BTreeMap<String, String>,
) -> Option<PathBuf> {
    let folder = rc.parent().unwrap_or(Path::new(".")).to_string_lossy();
    let root = pdk.root.to_string_lossy();
    let mut path = text.trim().trim_matches('"').to_string();
    for script_dir in [
        "[file dirname [file normalize [info script]]]",
        "[file dirname [info script]]",
    ] {
        path = path.replace(script_dir, &folder);
    }
    let mut known: Vec<(String, String)> = vec![
        ("PDK_ROOT".into(), root.to_string()),
        ("PDKPATH".into(), pdk.dir().to_string_lossy().into_owned()),
        ("PDK".into(), pdk.name.clone()),
    ];
    known.extend(variables.iter().map(|(k, v)| (k.clone(), v.clone())));
    // Longer names first, so $PDK does not eat $PDK_ROOT.
    known.sort_by_key(|(name, _)| std::cmp::Reverse(name.len()));
    for (name, value) in &known {
        for form in [
            format!("${{{name}}}"),
            format!("$::env({name})"),
            format!("$env({name})"),
            format!("${name}"),
        ] {
            path = path.replace(&form, value);
        }
    }
    (!path.contains('$') && !path.contains('[')).then(|| PathBuf::from(path))
}

/// Read a `.magicrc`. It is Tcl, so this looks at single commands: `tech load`,
/// `source`, `set`, `addpath` and the plain settings; anything computed is left out.
pub fn read_rc(rc: &Path, pdk: &PdkDir) -> MagicRc {
    let mut out = MagicRc::default();
    let Ok(text) = std::fs::read_to_string(rc) else {
        return out;
    };
    let mut variables: BTreeMap<String, String> = BTreeMap::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let words: Vec<&str> = line.split_whitespace().collect();
        match words.as_slice() {
            ["tech", "load", rest @ ..] => {
                let written: Vec<&str> = rest
                    .iter()
                    .copied()
                    .filter(|w| !w.starts_with('-'))
                    .collect();
                if out.tech_file.is_none() {
                    out.tech_file = rc_path(&written.join(" "), rc, pdk, &variables);
                }
            }
            ["source", rest @ ..] => {
                if let Some(path) = rc_path(&rest.join(" "), rc, pdk, &variables) {
                    out.sources.push(path);
                }
            }
            ["addpath", rest @ ..] => {
                if let Some(path) = rc_path(&rest.join(" "), rc, pdk, &variables)
                    && path.is_dir()
                    && !out.cell_paths.contains(&path)
                {
                    out.cell_paths.push(path);
                }
            }
            ["set", name, value] => {
                let value = value.trim_matches('"');
                if matches!(*name, "VDD" | "GND" | "SUB") {
                    out.globals.insert(name.to_string(), value.to_string());
                } else if !value.contains('[') && !value.contains('$') {
                    variables.insert(name.to_string(), value.to_string());
                }
            }
            ["drc", ..]
            | ["ext2spice", ..]
            | ["snap", ..]
            | ["extract", ..]
            | ["ext2sim", ..]
            | ["cif", ..]
            | ["gds", ..] => {
                let setting = line.trim_end_matches('}').trim().to_string();
                if !out.settings.contains(&setting) {
                    out.settings.push(setting);
                }
            }
            _ => {
                // if {[tech name] != "ihp-sg13g2"} {quit -noprompt}
                if line.contains("[tech name]")
                    && let Some(start) = line.find('"')
                    && let Some(length) = line[start + 1..].find('"')
                {
                    out.tech_name = Some(line[start + 1..start + 1 + length].to_string());
                }
            }
        }
    }
    out
}

// -- device generators --------------------------------------------------------------

/// The words of a Tcl list: braces group, `\` at a line end continues.
fn tcl_words(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    let mut quoted = false;
    let mut in_word = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'\n') => {
                chars.next();
            }
            '{' if !quoted => {
                if depth > 0 {
                    current.push(c);
                }
                depth += 1;
                in_word = true;
            }
            '}' if !quoted && depth > 0 => {
                depth -= 1;
                if depth > 0 {
                    current.push(c);
                }
            }
            '"' if depth == 0 => {
                quoted = !quoted;
                in_word = true;
            }
            c if c.is_whitespace() && depth == 0 && !quoted => {
                if in_word {
                    out.push(std::mem::take(&mut current));
                    in_word = false;
                }
            }
            c => {
                // Inside braces, runs of white space (line breaks) become one blank.
                if depth > 0 && c.is_whitespace() {
                    if !current.ends_with(' ') {
                        current.push(' ');
                    }
                } else {
                    current.push(c);
                }
                in_word = true;
            }
        }
    }
    if in_word {
        out.push(current);
    }
    out
}

/// The text between the brace at `open` and the one closing it.
fn braced(text: &str, open: usize) -> Option<&str> {
    let mut depth = 0usize;
    for (index, c) in text[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(&text[open + 1..open + index]);
                }
            }
            _ => {}
        }
    }
    None
}

/// The device generators in a Tcl text: every `proc <ns>::<name>_defaults {} {
/// return {name value …} }`.
pub fn generators(text: &str) -> Vec<MagicDevice> {
    let draws: BTreeSet<&str> = text
        .lines()
        .filter_map(|line| {
            let name = line
                .trim()
                .strip_prefix("proc ")?
                .split_whitespace()
                .next()?;
            name.strip_suffix("_draw")
        })
        .collect();
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(found) = text[from..].find("_defaults") {
        let at = from + found;
        from = at + "_defaults".len();
        // The line must be `proc <ns>::<name>_defaults {…} {`.
        let line_start = text[..at].rfind('\n').map_or(0, |i| i + 1);
        let Some(full) = text[line_start..at].trim_start().strip_prefix("proc ") else {
            continue;
        };
        let full = full.trim();
        if full.contains(char::is_whitespace) {
            continue;
        }
        let (namespace, name) = match full.rsplit_once("::") {
            Some((namespace, name)) => (namespace.trim_start_matches("::"), name),
            None => ("", full),
        };
        // Skip the argument list, then take the body.
        let Some(arguments) = text[from..].find('{').map(|i| from + i) else {
            continue;
        };
        let Some(arguments_text) = braced(text, arguments) else {
            continue;
        };
        let after = arguments + arguments_text.len() + 2;
        let Some(body_open) = text[after..].find('{').map(|i| after + i) else {
            continue;
        };
        let Some(body) = braced(text, body_open) else {
            continue;
        };
        let Some(list) = body
            .find("return")
            .and_then(|r| body[r..].find('{').map(|i| r + i))
            .and_then(|open| braced(body, open))
        else {
            continue;
        };
        let words = tcl_words(list);
        let mut device = MagicDevice {
            name: name.to_string(),
            namespace: namespace.to_string(),
            draw: draws.contains(full),
            ..MagicDevice::default()
        };
        for pair in words.chunks(2) {
            let [key, value] = pair else {
                continue;
            };
            match key.as_str() {
                "class" => device.class = value.clone(),
                "compatible" => {
                    device.compatible = value.split_whitespace().map(str::to_string).collect()
                }
                _ => device.parameters.push(MagicParameter {
                    name: key.clone(),
                    default: value.clone(),
                }),
            }
        }
        out.push(device);
    }
    out
}

/// The entries of magic's device menu: generator (`ns::name`) → label.
/// `magic::add_toolkit_command $layoutframe "label" "magic::gencell ns::name" pdk1`.
pub fn menu_labels(text: &str) -> BTreeMap<String, String> {
    let joined = text.replace("\\\n", " ");
    let mut out = BTreeMap::new();
    for line in joined.lines() {
        let Some(rest) = line.trim().strip_prefix("magic::add_toolkit_command") else {
            continue;
        };
        let words = tcl_words(rest);
        let (Some(label), Some(command)) = (words.get(1), words.get(2)) else {
            continue;
        };
        if let Some(generator) = command.strip_prefix("magic::gencell ") {
            out.entry(generator.trim().to_string())
                .or_insert_with(|| label.clone());
        }
    }
    out
}

fn files_in(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| entries.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    files.sort();
    files
}

fn role_of(file_name: &str, is_dir: bool) -> Option<&'static str> {
    let lower = file_name.to_lowercase();
    if lower.ends_with("-gds.tech") {
        Some("gds-tech")
    } else if lower.ends_with("-bindkeys") || lower.contains("bindkeys") {
        Some("bind-keys")
    } else if lower.contains("antenna") && !is_dir {
        Some("antenna")
    } else if lower.contains("density") {
        Some("density")
    } else if lower.contains("fill") {
        Some("fill")
    } else if lower.contains("seal") && (lower.contains("generat") || is_dir) {
        Some("seal-ring")
    } else if lower.contains("bump_bond") {
        Some("bump-bond")
    } else if lower.contains("run_standard_drc") || lower.contains("run_drc") {
        Some("drc")
    } else {
        None
    }
}

pub fn scan(ctx: &ScanContext) -> Scanned<MagicTech, MagicCatalog> {
    let pdk = &ctx.pdk;
    let mut scanned: Scanned<MagicTech, MagicCatalog> = Scanned::default();
    let dir = pdk.tech("magic");
    if !dir.is_dir() {
        scanned.notes.push(format!(
            "Magic: the PDK has no {} folder",
            pdk.portable(&dir)
        ));
        return scanned;
    }
    let section = &mut scanned.section;
    let notes = &mut scanned.notes;
    let files = files_in(&dir);
    let name_of = |p: &Path| {
        p.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    };

    // -- the start-up file ----------------------------------------------------
    let rcs: Vec<&PathBuf> = files
        .iter()
        .filter(|f| f.is_file() && name_of(f).ends_with("magicrc"))
        .collect();
    let rc = rcs
        .iter()
        .find(|f| name_of(f) == format!("{}.magicrc", pdk.name))
        .or(rcs.first())
        .map(|f| (*f).clone());
    let mut rc_says = MagicRc::default();
    match &rc {
        Some(rc) => {
            section.rc = Some(pdk.portable(rc));
            rc_says = read_rc(rc, pdk);
            section.globals = rc_says.globals.clone();
            section.settings = rc_says.settings.clone();
            section.cell_paths = rc_says.cell_paths.iter().map(|p| pdk.portable(p)).collect();
            if rcs.len() > 1 {
                notes.push(format!(
                    "Magic: {} start-up files, took {}",
                    rcs.len(),
                    pdk.portable(rc)
                ));
            }
        }
        None => notes.push("Magic: no start-up file (.magicrc)".to_string()),
    }

    // -- the technology file ----------------------------------------------------
    let tech_file = rc_says
        .tech_file
        .clone()
        .filter(|f| f.is_file())
        .or_else(|| {
            let named = dir.join(format!("{}.tech", pdk.name));
            if named.is_file() {
                return Some(named);
            }
            // Any technology file that is not the vendor-GDS one or an include.
            files
                .iter()
                .find(|f| {
                    let name = name_of(f);
                    name.ends_with(".tech") && !name.contains('-')
                })
                .cloned()
        });
    match &tech_file {
        Some(file) => {
            if rc.is_some() && rc_says.tech_file.as_ref() != Some(file) {
                notes.push(format!(
                    "Magic: the technology file the start-up file loads could not be read out of \
                     it; took {}",
                    pdk.portable(file)
                ));
            }
            let tech = read_tech(file);
            section.tech_file = Some(pdk.portable(file));
            section.tech = Some(tech.name.clone()).filter(|n| !n.is_empty());
            section.version = Some(tech.version.clone()).filter(|v| !v.is_empty());
            section.description = Some(tech.description.clone()).filter(|v| !v.is_empty());
            section.requires = Some(tech.requires.clone()).filter(|v| !v.is_empty());
            section.includes = tech.includes.iter().map(|p| pdk.portable(p)).collect();
            let of = |name: &str| -> Vec<String> {
                tech.catalog
                    .styles
                    .iter()
                    .filter(|s| s.section == name)
                    .map(|s| s.name.clone())
                    .collect()
            };
            section.cifoutput = of("cifoutput");
            section.cifinput = of("cifinput");
            section.extract = of("extract");
            section.drc = of("drc");
            section.scale = tech
                .catalog
                .styles
                .iter()
                .find(|s| s.section == "cifoutput" && !s.scale.is_empty())
                .map(|s| s.scale.clone());
            if tech.name.is_empty() {
                notes.push(format!("Magic: {} names no technology", pdk.portable(file)));
            }
            if let Some(checked) = &rc_says.tech_name
                && !tech.name.is_empty()
                && *checked != tech.name
            {
                notes.push(format!(
                    "Magic: the start-up file expects the technology {checked}, the technology \
                     file is {}",
                    tech.name
                ));
            }
            scanned.catalog = tech.catalog;
        }
        None => {
            section.tech = rc_says.tech_name.clone();
            notes.push("Magic: no technology file (.tech)".to_string());
        }
    }

    // -- device generators --------------------------------------------------------
    let main_tcl = rc_says
        .sources
        .iter()
        .find(|f| f.is_file())
        .cloned()
        .or_else(|| {
            let named = dir.join(format!("{}.tcl", pdk.name));
            named.is_file().then_some(named)
        });
    section.tcl = main_tcl.as_ref().map(|f| pdk.portable(f));
    let mut labels = BTreeMap::new();
    let mut devices: Vec<MagicDevice> = Vec::new();
    let mut draws: BTreeSet<String> = BTreeSet::new();
    for file in files
        .iter()
        .filter(|f| f.is_file() && f.extension().is_some_and(|e| e == "tcl"))
    {
        let Ok(text) = std::fs::read_to_string(file) else {
            continue;
        };
        labels.extend(menu_labels(&text));
        for line in text.lines() {
            if let Some(name) = line
                .trim()
                .strip_prefix("proc ")
                .and_then(|r| r.split_whitespace().next())
                .and_then(|n| n.strip_suffix("_draw"))
            {
                draws.insert(name.trim_start_matches("::").to_string());
            }
        }
        for mut device in generators(&text) {
            device.file = pdk.portable(file);
            devices.push(device);
        }
    }
    for device in &mut devices {
        let full = if device.namespace.is_empty() {
            device.name.clone()
        } else {
            format!("{}::{}", device.namespace, device.name)
        };
        // A `_draw` procedure may be in another file than the defaults.
        device.draw = device.draw || draws.contains(&full);
        if let Some(label) = labels.get(&full) {
            device.description = label.clone();
        }
    }
    scanned.catalog.devices = devices;

    // -- auxiliary files ------------------------------------------------------------
    for file in &files {
        if let Some(role) = role_of(&name_of(file), file.is_dir()) {
            section.scripts.push(Script {
                role: role.to_string(),
                file: pdk.portable(file),
            });
        }
    }
    scanned
}

#[cfg(test)]
mod tests {
    use super::*;

    const TECH: &str = r#"
# A comment
tech
  format 35
  demo
end

version
 version 1.0.1
 description "Demo: rules"
 requires magic-8.3.617
end

planes
  well,w
  active,a
  metal1,m1
end

types
  well nwell,nw
 -well obswell
  active nmos,ntransistor,nfet
  metal1 metal1,m1
end

aliases
  allnfets	   nfet, \
		   hvnfet
end

include demo-cifout

cifinput
style  demo variants (),(vendor)
 scalefactor 10 nanometers
 layer nwell NWELL,WELLPIN
 layer m1 MET1
 calma NWELL 64 20
 calma MET1 68 20
end

extract
 style ngspice variants (),(lvs)
 lambda 1.0
 units	microns
 variants (),(lvs)
 device msubcircuit sg13_lv_nmos nfet *ndiff *ndiff pwell,space/w error \
	l=l w=w
 device msubcircuit sg13_lv_nmos nfet *ndiff *ndiff pwell,space/w error l=l w=w
 device rsubcircuit rsil nres *nsd w=w l=l
end

drc
 style drc variants (fast),(full)
 scalefactor 10
end

lef
 routing m1 Metal1 metal1 M1
 cut via1 Via1
end
"#;

    const CIFOUT: &str = r#"
cifoutput
style gdsii
 scalefactor 10  nanometers
 options calma-permissive-labels
 layer NWELL 	allnwell
 	calma 	31 0
 templayer tmp nwell
 layer MET1 m1
 labels m1
	calma 8 0
 layer MET1 m1fill
	calma 8 22
 layer MET1 m1
	calma 8 0
style drc
 scalefactor 10
end
"#;

    fn demo(dir: &Path) -> PdkDir {
        let pdk = PdkDir::new(dir, "demo");
        let magic = pdk.tech("magic");
        std::fs::create_dir_all(&magic).unwrap();
        std::fs::write(magic.join("demo.tech"), TECH).unwrap();
        std::fs::write(magic.join("demo-cifout.tech"), CIFOUT).unwrap();
        std::fs::write(
            magic.join("demo-GDS.tech"),
            "tech\n format 32\n demo-GDS\nend\n",
        )
        .unwrap();
        std::fs::write(
            magic.join("demo.magicrc"),
            "puts stdout \"Sourcing\"\n\
             drc euclidean on\n\
             ext2spice scale off\n\
             tech load [file dirname [file normalize [info script]]]/demo.tech\n\
             if {[tech name] != \"demo\"} {quit -noprompt}\n\
             source [file dirname [file normalize [info script]]]/demo.tcl\n\
             set VDD VPWR\nset GND VGND\nset SUB sub!\n\
             addpath ${PDK_ROOT}/demo/libs.ref/mag\n\
             addpath $PDK_ROOT/demo/libs.ref/missing\n",
        )
        .unwrap();
        std::fs::create_dir_all(pdk.refs().join("mag")).unwrap();
        std::fs::write(
            magic.join("demo.tcl"),
            "   magic::add_toolkit_command $layoutframe \"nmos (MOSFET)\" \\\n\
             \t    \"magic::gencell demo::demo_nmos\" pdk1\n\
             source ${script_path}/demo-fet.tcl\n",
        )
        .unwrap();
        std::fs::write(
            magic.join("demo-fet.tcl"),
            "proc demo::demo_nmos_defaults {} {\n\
             \x20   return {w 0.6 l 0.13 m 1 \\\n\
             \t\tclass mosfet compatible {demo_nmos demo_hv_nmos} \\\n\
             \t\tfull_metal 1}\n}\n\
             proc demo::demo_nmos_draw {parameters} {\n    return 1\n}\n\
             proc demo::helper_defaults {} {\n    return {a 1}\n}\n\
             proc demo::unrelated {x} { set y \"_defaults\" }\n",
        )
        .unwrap();
        std::fs::write(magic.join("check_antenna.py"), "").unwrap();
        std::fs::write(magic.join("demo-BindKeys"), "").unwrap();
        pdk
    }

    #[test]
    fn a_technology_file_with_an_include() {
        let dir = tempfile::tempdir().unwrap();
        let pdk = demo(dir.path());
        let tech = read_tech(&pdk.tech("magic").join("demo.tech"));
        assert_eq!(tech.name, "demo");
        assert_eq!(tech.format, "35");
        assert_eq!(tech.version, "1.0.1");
        assert_eq!(tech.description, "Demo: rules");
        assert_eq!(tech.requires, "magic-8.3.617");
        assert_eq!(tech.includes.len(), 1);
        let c = &tech.catalog;
        assert_eq!(c.planes.len(), 3);
        assert_eq!(c.planes[0].aliases, ["w"]);
        assert_eq!(c.layers.len(), 4);
        assert_eq!(c.layers[2].aliases, ["ntransistor", "nfet"]);
        assert!(c.layers[1].locked && c.layers[1].plane == "well");
        assert_eq!(c.aliases[0].types, "nfet,hvnfet");

        let styles: Vec<(&str, &str, bool)> = c
            .styles
            .iter()
            .map(|s| (s.section.as_str(), s.name.as_str(), s.default))
            .collect();
        assert_eq!(
            styles,
            [
                ("cifoutput", "gdsii", true),
                ("cifoutput", "drc", false),
                ("cifinput", "demo", true),
                ("cifinput", "demo(vendor)", false),
                ("extract", "ngspice", true),
                ("extract", "ngspice(lvs)", false),
                ("drc", "drc(fast)", true),
                ("drc", "drc(full)", false),
            ]
        );
        assert_eq!(c.styles[0].scale, "10 nanometers");
        assert_eq!(c.styles[4].scale, "lambda 1.0, units microns");

        let gds: Vec<(&str, &str, i32, i32, &str)> = c
            .gds
            .iter()
            .map(|g| {
                (
                    g.direction.as_str(),
                    g.name.as_str(),
                    g.layer,
                    g.datatype,
                    g.types.as_str(),
                )
            })
            .collect();
        assert_eq!(
            gds,
            [
                ("output", "NWELL", 31, 0, "allnwell"),
                ("output", "MET1", 8, 0, "m1"),
                ("output", "MET1", 8, 22, "m1fill"),
                ("input", "NWELL", 64, 20, "nwell"),
                ("input", "MET1", 68, 20, "m1"),
            ]
        );
        assert_eq!(c.extracted.len(), 2, "the same device of a variant once");
        assert_eq!(c.extracted[0].model, "sg13_lv_nmos");
        assert_eq!(c.extracted[0].layer, "nfet");
        assert!(c.extracted[0].terminals.ends_with("l=l w=w"));
        assert_eq!(c.extracted[1].kind, "rsubcircuit");
        assert_eq!(c.lef[0].names, ["Metal1", "metal1", "M1"]);
    }

    #[test]
    fn the_whole_folder() {
        let dir = tempfile::tempdir().unwrap();
        let pdk = demo(dir.path());
        let scanned = scan(&ScanContext::new(pdk).files_only());
        let s = &scanned.section;
        assert_eq!(
            s.rc.as_ref().unwrap().as_str(),
            "$PDK_ROOT/demo/libs.tech/magic/demo.magicrc"
        );
        assert_eq!(s.tech.as_deref(), Some("demo"));
        assert_eq!(
            s.tech_file.as_ref().unwrap().as_str(),
            "$PDK_ROOT/demo/libs.tech/magic/demo.tech"
        );
        assert_eq!(
            s.tcl.as_ref().unwrap().as_str(),
            "$PDK_ROOT/demo/libs.tech/magic/demo.tcl"
        );
        assert_eq!(s.globals["VDD"], "VPWR");
        assert_eq!(s.globals["SUB"], "sub!");
        assert_eq!(s.settings, ["drc euclidean on", "ext2spice scale off"]);
        assert_eq!(s.cell_paths.len(), 1, "only the folder that exists");
        assert_eq!(s.cifoutput, ["gdsii", "drc"]);
        assert_eq!(s.extract, ["ngspice", "ngspice(lvs)"]);
        assert_eq!(s.scale.as_deref(), Some("10 nanometers"));
        let roles: Vec<&str> = s.scripts.iter().map(|x| x.role.as_str()).collect();
        assert_eq!(roles, ["antenna", "bind-keys", "gds-tech"]);
        assert!(scanned.notes.is_empty(), "{:?}", scanned.notes);

        let devices = &scanned.catalog.devices;
        assert_eq!(devices.len(), 2);
        let nmos = &devices[0];
        assert_eq!(
            (nmos.namespace.as_str(), nmos.name.as_str()),
            ("demo", "demo_nmos")
        );
        assert_eq!(nmos.description, "nmos (MOSFET)");
        assert_eq!(nmos.class, "mosfet");
        assert_eq!(nmos.compatible, ["demo_nmos", "demo_hv_nmos"]);
        let parameters: Vec<(&str, &str)> = nmos
            .parameters
            .iter()
            .map(|p| (p.name.as_str(), p.default.as_str()))
            .collect();
        assert_eq!(
            parameters,
            [("w", "0.6"), ("l", "0.13"), ("m", "1"), ("full_metal", "1")]
        );
        assert!(nmos.draw);
        assert!(!devices[1].draw);

        let text = toml::to_string(&scanned.section).unwrap();
        assert_eq!(toml::from_str::<MagicTech>(&text).unwrap(), scanned.section);
        let json = serde_json::to_string(&scanned.catalog).unwrap();
        assert_eq!(
            serde_json::from_str::<MagicCatalog>(&json).unwrap(),
            scanned.catalog
        );
    }

    #[test]
    fn no_magic_folder_is_a_note() {
        let dir = tempfile::tempdir().unwrap();
        let scanned = scan(&ScanContext::new(PdkDir::new(dir.path(), "none")));
        assert_eq!(scanned.section, MagicTech::default());
        assert_eq!(scanned.notes.len(), 1);
    }

    #[test]
    fn tcl_lists() {
        assert_eq!(
            tcl_words("w 0.6 compatible {a b \\\n   c} \"x y\" z"),
            ["w", "0.6", "compatible", "a b c", "x y", "z"]
        );
        assert_eq!(style_names(&["gdsii"]), ["gdsii"]);
        assert_eq!(
            style_names(&["ngspice", "variants", "(),(lvs),", "(hrhc)"]),
            ["ngspice", "ngspice(lvs)", "ngspice(hrhc)"]
        );
    }
}
