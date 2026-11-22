//! KLayout section of the technology snapshot: `libs.tech/klayout`.
//!
//! From files: the technology (`.lyt`: name, database unit, layer properties file,
//! reader options, connectivity), the layers (`.lyp`: numbers, colours, stipples), the
//! LEF/DEF layer maps (`.map`), the DRC and LVS decks with the switches they read, the
//! macros, the 2.5D and cross-section scripts. Through the program, when it may be
//! started: the PCell libraries the technology registers and every PCell's
//! parameters; else what the Python sources say about them.
//!
//! The section (`[klayout]` in `tech.toml`) is what a person may want to change:
//! paths, the technology name, which deck is the default (the first of `drc`/`lvs`).
//! The long lists are in the catalog.

mod decks;
mod lyp;
mod lyt;
mod pcells;
pub mod xml;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::pdk::{PdkDir, ScanContext, Scanned, TechPath};

pub use decks::{Check, DeckDetail, DeckSwitch, LayerMapEntry, Macro};
pub use lyp::{DitherPattern, LineStyle, Lyp};
pub use lyt::{Connection, LayerSymbol, Lyt};
pub use pcells::{Asked, SCRIPT as PCELL_SCRIPT};

/// `[klayout]` in `tech.toml`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KlayoutTech {
    /// Folders for `KLAYOUT_PATH`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub path: Vec<TechPath>,
    /// The technology name KLayout knows it by (`sg13g2`, `sky130`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tech: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The technology file (`.lyt`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lyt: Option<TechPath>,
    /// The technology's folder: where its macros, decks and the files the `.lyt`
    /// names are looked for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_path: Option<TechPath>,
    /// The layer properties file (`.lyp`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layer_properties: Option<TechPath>,
    /// Database unit in micron.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dbu: Option<f64>,
    /// The grids the technology offers, in micron.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub grids: Vec<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_grid: Option<f64>,
    /// LEF/DEF layer map files (`.map`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub layer_maps: Vec<TechPath>,
    /// LEF files the technology's LEF/DEF reader loads along.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub lef_files: Vec<TechPath>,
    /// What the net tracer connects, as KLayout writes it: `Metal1,Via1,Metal2`. The
    /// names stand for layer expressions, see the catalog's `symbols`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub connectivity: Vec<String>,
    /// The PCell libraries the technology registers (`SG13_dev`); their PCells are in
    /// the catalog.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pcell_libraries: Vec<String>,
    /// DRC decks, the one to use by default first.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub drc: Vec<Deck>,
    /// LVS decks, the one to use by default first.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub lvs: Vec<Deck>,
    /// 2.5D view scripts (`.lyd25`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub d25: Vec<TechPath>,
    /// Cross-section scripts (`.xs`, for the xsection package).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub xsection: Vec<TechPath>,
    /// Other technology files found in the PDK; the one in `lyt` was chosen.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub other_technologies: Vec<TechPath>,
}

/// A DRC or LVS deck.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Deck {
    pub name: String,
    pub file: TechPath,
    /// `script` (`.drc`, `.lvs`) or `macro` (`.lydrc`, `.lylvs`).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub form: String,
    /// Checks a part only (antenna, density, …).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub partial: bool,
}

/// The KLayout part of the catalog.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KlayoutCatalog {
    pub layers: Vec<Layer>,
    pub pcell_libraries: Vec<PcellLibrary>,
    /// Where `pcell_libraries` comes from: `klayout` (asked from the program), `files`
    /// (read out of the Python sources), or empty when there are none.
    pub pcell_source: String,
    /// The KLayout that was asked (`KLayout 0.30.12`).
    pub program: String,
    /// Stipples the layer properties file defines; a layer's `dither_pattern` `C<n>`
    /// names the one with `order` n, `I<n>` one of KLayout's own.
    pub dither_patterns: Vec<DitherPattern>,
    /// Line styles the layer properties file defines (`C<n>` like the stipples).
    pub line_styles: Vec<LineStyle>,
    /// The net tracer's connectivity: conductor, via, conductor.
    pub connections: Vec<Connection>,
    /// The names the connectivity uses for layer expressions.
    pub symbols: Vec<LayerSymbol>,
    /// The technology's reader options that are set (`lefdef.pins-suffix` → `.PIN`).
    pub reader_options: BTreeMap<String, String>,
    pub writer_options: BTreeMap<String, String>,
    /// The LEF/DEF layer map (the first file of the section's `layer_maps`).
    pub layer_map: Vec<LayerMapEntry>,
    /// Every DRC and LVS deck in detail, in the section's order.
    pub decks: Vec<DeckDetail>,
    pub macros: Vec<Macro>,
}

/// A layer of the layer properties file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Layer {
    /// The name as the file has it (`Metal1.drawing`, `met1.pin - 68/16`); the source
    /// when the file gives no name.
    pub name: String,
    /// GDS layer, -1 when the entry is not one layer (a wildcard).
    pub layer: i32,
    /// GDS datatype, -1 like `layer`.
    pub datatype: i32,
    /// The name without purpose (`Metal1`).
    pub base: String,
    /// The purpose (`drawing`, `pin`, `label`), empty when the name has none.
    pub purpose: String,
    /// The source as written (`8/0`, `68/16@1`, `*/*@*`).
    pub source: String,
    /// `#rrggbb`, empty when not set.
    pub frame_color: String,
    pub fill_color: String,
    pub frame_brightness: i32,
    pub fill_brightness: i32,
    /// `I<n>`: KLayout's built-in stipple n; `C<n>`: the file's own (catalog
    /// `dither_patterns`); empty when not set.
    pub dither_pattern: String,
    /// `I<n>` / `C<n>` like the stipple.
    pub line_style: String,
    pub valid: bool,
    pub visible: bool,
    pub transparent: bool,
    /// Line width in pixels.
    pub width: i32,
    pub marked: bool,
    pub xfill: bool,
    pub animation: i32,
    /// The groups the layer is in, outermost first.
    pub group: Vec<String>,
}

impl Default for Layer {
    fn default() -> Self {
        Layer {
            name: String::new(),
            layer: 0,
            datatype: 0,
            base: String::new(),
            purpose: String::new(),
            source: String::new(),
            frame_color: String::new(),
            fill_color: String::new(),
            frame_brightness: 0,
            fill_brightness: 0,
            dither_pattern: String::new(),
            line_style: String::new(),
            valid: true,
            visible: true,
            transparent: false,
            width: 1,
            marked: false,
            xfill: false,
            animation: 0,
            group: Vec::new(),
        }
    }
}

/// A PCell library the technology registers (`SG13_dev`, `skywater130`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PcellLibrary {
    pub name: String,
    pub description: String,
    /// The technologies the library is bound to (empty: all).
    pub technologies: Vec<String>,
    pub pcells: Vec<Pcell>,
    /// The fixed cells of the library (not PCells).
    pub cells: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Pcell {
    pub name: String,
    pub description: String,
    pub parameters: Vec<PcellParameter>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PcellParameter {
    pub name: String,
    /// `int`, `double`, `string`, `boolean`, `layer`, `shape`, `list`, `callback`, `none`.
    pub kind: String,
    /// The default, as text.
    pub default: String,
    pub description: String,
    pub unit: String,
    /// The values to choose from, as text.
    pub choices: Vec<String>,
    /// What the form shows for each of `choices`.
    pub choice_labels: Vec<String>,
    /// The group of the form the parameter is in.
    pub group: String,
    pub hidden: bool,
    pub readonly: bool,
}

/// Technology files (`.lyt`) below a `KLAYOUT_PATH` folder, where KLayout looks:
/// `tech/` and its sub-folders, also the folder itself.
fn technology_files(klayout: &Path) -> Vec<PathBuf> {
    fn collect(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        paths.sort();
        for path in paths {
            if path.is_dir() {
                if depth < 2 {
                    collect(&path, depth + 1, out);
                }
            } else if path.extension().is_some_and(|e| e == "lyt") {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    collect(&klayout.join("tech"), 0, &mut out);
    if out.is_empty() {
        collect(klayout, 1, &mut out);
    }
    out
}

/// Of several technology files, the one meant for the PDK: its file or technology
/// name is the PDK's name, or part of it.
fn choose_technology(pdk: &PdkDir, files: &[(PathBuf, Lyt)]) -> usize {
    let wanted = pdk.name.to_lowercase();
    let score = |(file, lyt): &(PathBuf, Lyt)| {
        let stem = file
            .file_stem()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let name = lyt.name.to_lowercase();
        if stem == wanted || name == wanted {
            0
        } else if (stem.len() > 2 && wanted.contains(&stem))
            || (name.len() > 2 && wanted.contains(&name))
        {
            1
        } else if stem.contains(&wanted) {
            2
        } else {
            3
        }
    };
    (0..files.len())
        .min_by_key(|i| (score(&files[*i]), *i))
        .unwrap_or(0)
}

fn existing_dirs(candidates: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let mut seen = BTreeSet::new();
    candidates
        .into_iter()
        .filter(|d| d.is_dir() && seen.insert(d.canonicalize().unwrap_or(d.clone())))
        .collect()
}

fn deck_of(detail: &DeckDetail) -> Deck {
    Deck {
        name: detail.name.clone(),
        file: detail.file.clone(),
        form: detail.form.clone(),
        partial: detail.partial,
    }
}

pub fn scan(ctx: &ScanContext) -> Scanned<KlayoutTech, KlayoutCatalog> {
    let pdk = &ctx.pdk;
    let mut scanned: Scanned<KlayoutTech, KlayoutCatalog> = Scanned::default();
    let klayout = pdk.tech("klayout");
    if !klayout.is_dir() {
        scanned.notes.push(format!(
            "KLayout: the PDK has no {} folder",
            pdk.portable(&klayout)
        ));
        return scanned;
    }
    let section = &mut scanned.section;
    let catalog = &mut scanned.catalog;
    let notes = &mut scanned.notes;
    section.path = vec![pdk.portable(&klayout)];

    // -- the technology file -------------------------------------------------
    let mut technologies: Vec<(PathBuf, Lyt)> = Vec::new();
    for file in technology_files(&klayout) {
        match std::fs::read_to_string(&file)
            .map_err(|e| e.to_string())
            .and_then(|text| lyt::parse(&text))
        {
            Ok(lyt) => technologies.push((file, lyt)),
            Err(error) => notes.push(format!(
                "KLayout: {} cannot be read: {error}",
                pdk.portable(&file)
            )),
        }
    }
    let mut base = klayout.join("tech");
    if technologies.is_empty() {
        notes.push(format!(
            "KLayout: no technology file (.lyt) below {}",
            pdk.portable(&klayout)
        ));
        if !base.is_dir() {
            base = klayout.clone();
        }
    } else {
        let chosen = choose_technology(pdk, &technologies);
        let (file, lyt) = technologies.remove(chosen);
        if !technologies.is_empty() {
            section.other_technologies =
                technologies.iter().map(|(f, _)| pdk.portable(f)).collect();
            notes.push(format!(
                "KLayout: {} technology files, took {} ({})",
                technologies.len() + 1,
                pdk.portable(&file),
                lyt.name
            ));
        }
        let folder = file.parent().unwrap_or(&klayout).to_path_buf();
        // An empty <base-path> is the folder of the file; a set one may hold
        // variables of another machine, so it only counts when it exists here.
        base = match lyt.base_path.as_str() {
            "" => folder.clone(),
            written => {
                let path = TechPath(written.to_string())
                    .resolve_in(pdk)
                    .unwrap_or_else(|_| PathBuf::from(written));
                if path.is_dir() { path } else { folder.clone() }
            }
        };
        section.tech = Some(lyt.name.clone()).filter(|n| !n.is_empty());
        section.description = Some(lyt.description.clone()).filter(|d| !d.is_empty());
        section.lyt = Some(pdk.portable(&file));
        section.base_path = Some(pdk.portable(&base));
        section.dbu = lyt.dbu;
        section.grids = lyt.grids.clone();
        section.default_grid = lyt.default_grid;
        section.connectivity = lyt.connections.iter().map(Connection::text).collect();
        if lyt.name.is_empty() {
            notes.push(format!(
                "KLayout: {} names no technology",
                pdk.portable(&file)
            ));
        }
        if !lyt.layer_properties_file.is_empty() {
            let lyp = base.join(&lyt.layer_properties_file);
            if lyp.is_file() {
                section.layer_properties = Some(pdk.portable(&lyp));
            } else {
                notes.push(format!(
                    "KLayout: the technology names the layer properties file {}, which is not there",
                    pdk.portable(&lyp)
                ));
            }
        }
        for lef in &lyt.lef_files {
            let path = base.join(lef);
            if path.is_file() {
                section.lef_files.push(pdk.portable(&path));
            } else {
                notes.push(format!(
                    "KLayout: the technology's LEF/DEF reader names {lef}, which is not in {}",
                    pdk.portable(&base)
                ));
            }
        }
        catalog.connections = lyt.connections;
        catalog.symbols = lyt.symbols;
        catalog.reader_options = lyt.reader_options;
        catalog.writer_options = lyt.writer_options;
    }

    // -- layers --------------------------------------------------------------
    if section.layer_properties.is_none() {
        // No technology, or it names none: a single .lyp in the folder is the one.
        let found = decks::files_with(&existing_dirs([base.clone(), klayout.clone()]), &["lyp"]);
        match found.as_slice() {
            [] => notes.push("KLayout: no layer properties file (.lyp)".to_string()),
            [only] => section.layer_properties = Some(pdk.portable(only)),
            [first, ..] => {
                section.layer_properties = Some(pdk.portable(first));
                notes.push(format!(
                    "KLayout: {} layer properties files, took {}",
                    found.len(),
                    pdk.portable(first)
                ));
            }
        }
    }
    if let Some(file) = section
        .layer_properties
        .as_ref()
        .and_then(|p| p.resolve_in(pdk).ok())
    {
        match std::fs::read_to_string(&file)
            .map_err(|e| e.to_string())
            .and_then(|text| lyp::parse(&text))
        {
            Ok(lyp) => {
                catalog.layers = lyp.layers;
                catalog.dither_patterns = lyp.dither_patterns;
                catalog.line_styles = lyp.line_styles;
            }
            Err(error) => notes.push(format!(
                "KLayout: {} cannot be read: {error}",
                pdk.portable(&file)
            )),
        }
    }

    // -- layer maps ----------------------------------------------------------
    let maps = decks::files_with(&existing_dirs([base.clone(), klayout.clone()]), &["map"]);
    section.layer_maps = maps.iter().map(|f| pdk.portable(f)).collect();
    if let Some(first) = maps.first()
        && let Ok(text) = std::fs::read_to_string(first)
    {
        catalog.layer_map = decks::layer_map(&text);
    }

    // -- decks ---------------------------------------------------------------
    for check in [Check::Drc, Check::Lvs] {
        let found = decks::find(pdk, &klayout, &base, check);
        if found.is_empty() {
            notes.push(format!(
                "KLayout: no {} deck in the PDK",
                check.name().to_uppercase()
            ));
        }
        let list: Vec<Deck> = found.iter().map(deck_of).collect();
        match check {
            Check::Drc => section.drc = list,
            Check::Lvs => section.lvs = list,
        }
        catalog.decks.extend(found);
    }

    // -- macros, 2.5D, cross-sections ----------------------------------------
    catalog.macros = decks::macros(pdk, &klayout, &base);
    section.d25 = decks::files_with(
        &existing_dirs([base.join("d25"), klayout.join("d25")]),
        &["lyd25"],
    )
    .iter()
    .map(|f| pdk.portable(f))
    .collect();
    section.xsection = decks::files_with(
        &existing_dirs([
            base.join("xsect"),
            klayout.join("xsect"),
            klayout.join("tech").join("xsect"),
        ]),
        &["xs"],
    )
    .iter()
    .map(|f| pdk.portable(f))
    .collect();

    // -- PCell libraries -----------------------------------------------------
    let mut asked_note = None;
    if ctx.helpers {
        match pcells::ask(ctx, std::slice::from_ref(&klayout)) {
            Ok(asked) => {
                tracing::debug!(
                    "KLayout listed {} PCell libraries in {:.2} s",
                    asked.libraries.len(),
                    asked.seconds
                );
                for message in &asked.messages {
                    notes.push(format!("KLayout said: {message}"));
                }
                if let Some(tech) = &section.tech
                    && !asked.technologies.is_empty()
                    && !asked.technologies.contains(tech)
                {
                    notes.push(format!(
                        "KLayout did not load the technology {tech} (it has: {})",
                        asked.technologies.join(", ")
                    ));
                }
                catalog.program = asked.program;
                if !asked.libraries.is_empty() {
                    catalog.pcell_libraries = asked.libraries;
                    catalog.pcell_source = "klayout".to_string();
                }
            }
            Err(error) => asked_note = Some(error),
        }
    } else {
        asked_note = Some("KLayout was not started".to_string());
    }
    if catalog.pcell_libraries.is_empty() {
        let sources = pcells::from_sources(std::slice::from_ref(&klayout));
        let why = asked_note.unwrap_or_else(|| "KLayout registered no PCell library".to_string());
        if sources.libraries.is_empty() && sources.computed == 0 {
            notes.push(format!("KLayout: PCells not listed: {why}"));
        } else {
            let count: usize = sources.libraries.iter().map(|l| l.pcells.len()).sum();
            let mut note = format!(
                "KLayout: {count} PCell{} read out of the PDK's Python sources ({why}); defaults \
                 that are expressions are kept as written",
                if count == 1 { "" } else { "s" }
            );
            if sources.computed > 0 {
                note.push_str(
                    "; the PDK also registers PCells under names made at run time, which only \
                     KLayout itself can list",
                );
            }
            notes.push(note);
            if !sources.libraries.is_empty() {
                catalog.pcell_libraries = sources.libraries;
                catalog.pcell_source = "files".to_string();
            }
        }
    }
    section.pcell_libraries = catalog
        .pcell_libraries
        .iter()
        .map(|l| l.name.clone())
        .collect();

    scanned
}

/// Ask KLayout for its PCell libraries with the given folders on `KLAYOUT_PATH` (what
/// [`scan`] does for a PDK), e.g. to refresh the catalog for a project's own path.
pub fn ask_pcells(ctx: &ScanContext, klayout_path: &[PathBuf]) -> Result<Asked, String> {
    pcells::ask(ctx, klayout_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LYT: &str = r#"<technology>
 <name>demo</name>
 <description>Demo technology</description>
 <dbu>0.005</dbu>
 <base-path/>
 <layer-properties_file>demo.lyp</layer-properties_file>
 <default-grids>0.1,0.05!</default-grids>
 <reader-options><lefdef><lef-files>missing.lef</lef-files><lef-files>tech.lef</lef-files></lefdef></reader-options>
 <connectivity>
  <connection>m1,v1,m2</connection>
  <symbols>m1='1/0'</symbols>
 </connectivity>
</technology>"#;

    const LYP: &str = r#"<layer-properties>
 <properties><name>m1.drawing</name><source>1/0@1</source><fill-color>#ff0000</fill-color></properties>
 <properties><name>m2.drawing</name><source>2/0@1</source></properties>
</layer-properties>"#;

    fn demo_pdk(dir: &Path) -> PdkDir {
        let pdk = PdkDir::new(dir, "demoA");
        let tech = pdk.tech("klayout").join("tech");
        std::fs::create_dir_all(tech.join("drc")).unwrap();
        std::fs::create_dir_all(tech.join("lvs")).unwrap();
        std::fs::create_dir_all(tech.join("macros")).unwrap();
        std::fs::create_dir_all(tech.join("xsect")).unwrap();
        std::fs::write(tech.join("demo.lyt"), LYT).unwrap();
        std::fs::write(tech.join("demo.lyp"), LYP).unwrap();
        std::fs::write(tech.join("demo.map"), "m1 NET,PIN 1 0\n").unwrap();
        std::fs::write(tech.join("tech.lef"), "").unwrap();
        std::fs::write(
            tech.join("drc/demo.drc"),
            "# Demo rules\nsource($input, $topcell)\nreport('x', $report)\n",
        )
        .unwrap();
        std::fs::write(tech.join("drc/demo_antenna.drc"), "source($input)\n").unwrap();
        std::fs::write(tech.join("lvs/demo.lvs"), "schematic($schematic)\n").unwrap();
        std::fs::write(
            tech.join("macros/hello.lym"),
            "<klayout-macro><description>Say hello</description><autorun>true</autorun>\
             <interpreter>python</interpreter><menu-path>tools_menu.end</menu-path>\
             <text>print(1)</text></klayout-macro>",
        )
        .unwrap();
        std::fs::write(tech.join("xsect/demo.xs"), "").unwrap();
        pdk
    }

    #[test]
    fn a_small_pdk_from_files() {
        let dir = tempfile::tempdir().unwrap();
        let pdk = demo_pdk(dir.path());
        let scanned = scan(&ScanContext::new(pdk).files_only());
        let s = &scanned.section;
        assert_eq!(s.path[0].as_str(), "$PDK_ROOT/demoA/libs.tech/klayout");
        assert_eq!(s.tech.as_deref(), Some("demo"));
        assert_eq!(s.description.as_deref(), Some("Demo technology"));
        assert_eq!(
            s.lyt.as_ref().unwrap().as_str(),
            "$PDK_ROOT/demoA/libs.tech/klayout/tech/demo.lyt"
        );
        assert_eq!(
            s.base_path.as_ref().unwrap().as_str(),
            "$PDK_ROOT/demoA/libs.tech/klayout/tech"
        );
        assert_eq!(
            s.layer_properties.as_ref().unwrap().as_str(),
            "$PDK_ROOT/demoA/libs.tech/klayout/tech/demo.lyp"
        );
        assert_eq!(s.dbu, Some(0.005));
        assert_eq!(s.default_grid, Some(0.05));
        assert_eq!(s.connectivity, ["m1,v1,m2"]);
        assert_eq!(s.layer_maps.len(), 1);
        assert_eq!(s.lef_files.len(), 1, "the missing one is a note");
        assert_eq!(s.xsection.len(), 1);
        let drc: Vec<&str> = s.drc.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(drc, ["demo", "demo_antenna"]);
        assert!(s.drc[1].partial && !s.drc[0].partial);
        assert_eq!(s.lvs[0].name, "demo");
        assert!(s.pcell_libraries.is_empty());

        let c = &scanned.catalog;
        assert_eq!(c.layers.len(), 2);
        assert_eq!(c.layers[0].fill_color, "#ff0000");
        assert_eq!(c.symbols[0].expression, "1/0");
        assert_eq!(c.layer_map[0].name, "m1");
        assert_eq!(c.decks.len(), 3);
        assert!(c.decks[0].default);
        assert_eq!(c.decks[0].description, "Demo rules");
        let roles: Vec<&str> = c.decks[0]
            .switches
            .iter()
            .map(|s| s.role.as_str())
            .collect();
        assert_eq!(roles, ["input", "topcell", "report"]);
        assert_eq!(c.decks[2].check, "lvs");
        assert_eq!(c.macros.len(), 1);
        assert!(c.macros[0].autorun);
        assert_eq!(c.macros[0].interpreter, "python");
        assert_eq!(c.pcell_source, "");

        let notes = scanned.notes.join("\n");
        assert!(notes.contains("missing.lef"), "{notes}");
        assert!(notes.contains("PCells not listed"), "{notes}");

        // The section goes through TOML unchanged, the catalog through JSON.
        let text = toml::to_string(&scanned.section).unwrap();
        assert_eq!(
            toml::from_str::<KlayoutTech>(&text).unwrap(),
            scanned.section
        );
        let json = serde_json::to_string(&scanned.catalog).unwrap();
        assert_eq!(
            serde_json::from_str::<KlayoutCatalog>(&json).unwrap(),
            scanned.catalog
        );
    }

    #[test]
    fn a_pdk_without_klayout_is_a_note() {
        let dir = tempfile::tempdir().unwrap();
        let scanned = scan(&ScanContext::new(PdkDir::new(dir.path(), "none")).files_only());
        assert_eq!(scanned.section, KlayoutTech::default());
        assert_eq!(scanned.notes.len(), 1);
    }

    #[test]
    fn the_technology_named_like_the_pdk_wins() {
        let pdk = PdkDir::new("/r", "sky130A");
        let lyt = |name: &str| Lyt {
            name: name.into(),
            ..Lyt::default()
        };
        let files = vec![
            (PathBuf::from("/r/a/other.lyt"), lyt("other")),
            (PathBuf::from("/r/a/sky130A.lyt"), lyt("sky130")),
        ];
        assert_eq!(choose_technology(&pdk, &files), 1);
        let files = vec![
            (PathBuf::from("/r/a/x.lyt"), lyt("x")),
            (PathBuf::from("/r/a/y.lyt"), lyt("y")),
        ];
        assert_eq!(choose_technology(&pdk, &files), 0);
    }
}
