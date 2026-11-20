//! Nets: the schematic's connectivity against the layout's (docs/LINKING.md §8).
//!
//! - The schematic side: xschem's own connectivity (`xschem instance_nodemap`: for each
//!   instance, symbol pin → net, the netlister's view of wires and labels), read for
//!   the schematic and, through the instances of project cells, for the schematics
//!   below it, and flattened ([`FlatSchematic`]): every device terminal of the
//!   hierarchy (`x1/M1` pin `G`) on one flat net.
//! - The layout side: KLayout's extraction with the PDK's LVS deck (devices with
//!   terminals, nets, all flattened into the top cell), described by the helper script
//!   `connectivity.py` ([`Connectivity`]). The devices are matched to schematic
//!   instances by place: a device belongs to the bound instance (GDS property 61,
//!   through bound cells) whose box holds it.
//! - [`compare`]: per schematic net, the layout nets its terminals are on. More than
//!   one: an **open**, drawn as flylines between the pieces (a minimum spanning tree
//!   over the nearest terminals). A layout net holding terminals of several schematic
//!   nets: a **short**. Terminals of devices that are not in the layout, and schematic
//!   pins without a layout net of that name (a label on the wire), are listed too.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use eyre::{Context, Result, bail, eyre};
use serde::{Deserialize, Serialize};

use super::{DeviceMap, Schematic, Target, read_schematic, schematic_view};
use crate::layout::{Parallel, Source};
use crate::xschem::tcl;
use crate::{Project, xschem};

/// Deepest hierarchy followed below the schematic.
const MAX_DEPTH: usize = 32;

// -- the schematic side -----------------------------------------------------------

/// For each instance of one schematic: its pins and their nets, in the symbol's order.
pub type NodeMap = BTreeMap<String, Vec<(String, String)>>;

/// The Tcl that lists every instance with its nets:
/// `{name {pin net pin net …}} …`.
const NODEMAP_SCRIPT: &str = r#"
set cdept_out {}
set cdept_n [xschem get instances]
for {set i 0} {$i < $cdept_n} {incr i} {
  set cdept_name [xschem getprop instance $i name]
  set cdept_map [xschem instance_nodemap $cdept_name]
  lappend cdept_out [list $cdept_name [lrange $cdept_map 1 end]]
}
set cdept_out
"#;

/// Parse what [`NODEMAP_SCRIPT`] returns.
pub fn parse_nodemap(text: &str) -> Result<NodeMap> {
    let mut out = NodeMap::new();
    for item in tcl::parse_list(text).map_err(|e| eyre!("{e}"))? {
        let fields = tcl::parse_list(&item).map_err(|e| eyre!("{e}"))?;
        let [name, pins] = fields.as_slice() else {
            bail!("not an instance and its nets: {item}");
        };
        let pins = tcl::parse_list(pins).map_err(|e| eyre!("{e}"))?;
        let pairs = pins
            .chunks(2)
            .filter(|p| p.len() == 2)
            .map(|p| (p[0].clone(), p[1].clone()))
            .collect();
        out.insert(name.clone(), pairs);
    }
    Ok(out)
}

/// The nets of the instances of the schematic `file`, from xschem.
pub fn nodemap(engine: &mut xschem::Engine, file: &Path) -> Result<NodeMap> {
    engine.load(file)?;
    let text = engine.tcl(NODEMAP_SCRIPT)?;
    parse_nodemap(&text)
}

/// A terminal of a device of the flattened schematic.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SchTerminal {
    /// Instance path through the hierarchy: `M1`, `x1/M2`.
    pub path: String,
    /// The symbol pin.
    pub pin: String,
    /// Index into [`FlatSchematic::nets`].
    pub net: usize,
    /// The symbol, as the schematic refers to it.
    pub symbol: String,
}

/// A net of the flattened schematic.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FlatNet {
    /// Its name: the name in the top schematic when it reaches it, else the name in
    /// the highest schematic, with the instance path (`x1/n3`).
    pub name: String,
    /// Every name it has, top first.
    pub names: Vec<String>,
}

/// A pin of the top schematic.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SchPort {
    pub name: String,
    pub direction: String,
    pub net: usize,
}

/// The schematic, flattened: every device terminal of the hierarchy on a net.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct FlatSchematic {
    pub nets: Vec<FlatNet>,
    pub terminals: Vec<SchTerminal>,
    pub ports: Vec<SchPort>,
    /// Leaf instance paths (devices) with their symbols.
    pub devices: BTreeMap<String, String>,
    /// What could not be followed: `path: reason`.
    pub problems: Vec<String>,
}

impl FlatSchematic {
    /// The net named `name` in the top schematic.
    pub fn net_named(&self, name: &str) -> Option<usize> {
        self.nets.iter().position(|n| n.name == name)
    }
}

/// One schematic of the hierarchy, as read: what links needs and what xschem says.
pub struct SheetData {
    pub schematic: Schematic,
    pub nodemap: NodeMap,
}

/// Reads the schematics of a hierarchy (so that tests can give them without xschem).
pub trait Sheets {
    /// The schematic of a project cell (`Target::Cell`).
    fn sheet(&mut self, library: &str, cell: &str) -> Result<SheetData>;
}

/// [`Sheets`] through the project and an xschem engine.
pub struct ProjectSheets<'a> {
    pub project: &'a Project,
    pub map: &'a DeviceMap,
    pub engine: xschem::Engine,
}

impl ProjectSheets<'_> {
    pub fn read(&mut self, source: Source, file: &Path) -> Result<SheetData> {
        let schematic = read_schematic(self.project, self.map, source, file)?;
        let nodemap = nodemap(&mut self.engine, file)?;
        Ok(SheetData { schematic, nodemap })
    }
}

impl Sheets for ProjectSheets<'_> {
    fn sheet(&mut self, library: &str, cell: &str) -> Result<SheetData> {
        let (source, file) = schematic_view(self.project, library, cell, None)?;
        self.read(source, &file)
    }
}

/// Union–find over net keys (`prefix` + net name).
#[derive(Default)]
struct Union {
    index: BTreeMap<String, usize>,
    keys: Vec<String>,
    parent: Vec<usize>,
}

impl Union {
    fn id(&mut self, key: &str) -> usize {
        if let Some(&i) = self.index.get(key) {
            return i;
        }
        let i = self.keys.len();
        self.index.insert(key.to_string(), i);
        self.keys.push(key.to_string());
        self.parent.push(i);
        i
    }

    fn find(&mut self, mut i: usize) -> usize {
        while self.parent[i] != i {
            self.parent[i] = self.parent[self.parent[i]];
            i = self.parent[i];
        }
        i
    }

    fn join(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            // The key nearer the top names the net.
            let depth = |k: &str| k.matches('/').count();
            let (keep, other) = if (depth(&self.keys[a]), self.keys[a].len())
                <= (depth(&self.keys[b]), self.keys[b].len())
            {
                (a, b)
            } else {
                (b, a)
            };
            self.parent[other] = keep;
        }
    }
}

/// Flatten the schematic `top` and the schematics of the project cells it places.
pub fn flatten(top: SheetData, sheets: &mut dyn Sheets) -> FlatSchematic {
    let mut union = Union::default();
    let mut flat = FlatSchematic::default();
    // (path, pin, key, symbol)
    let mut terminals: Vec<(String, String, usize, String)> = Vec::new();
    let mut ports = Vec::new();
    for pin in &top.schematic.pins {
        ports.push((pin.name.clone(), pin.direction.clone(), union.id(&pin.name)));
    }
    walk(
        &top,
        "",
        sheets,
        &mut union,
        &mut terminals,
        &mut flat,
        0,
        &mut BTreeSet::new(),
    );
    // Number the nets: by their root, in the order they were first seen.
    let mut number: BTreeMap<usize, usize> = BTreeMap::new();
    let mut nets: Vec<FlatNet> = Vec::new();
    let mut members: Vec<Vec<String>> = Vec::new();
    for i in 0..union.keys.len() {
        let root = union.find(i);
        let n = *number.entry(root).or_insert_with(|| {
            nets.push(FlatNet {
                name: union.keys[root].clone(),
                names: Vec::new(),
            });
            members.push(Vec::new());
            nets.len() - 1
        });
        members[n].push(union.keys[i].clone());
    }
    for (net, mut names) in nets.iter_mut().zip(members) {
        names.sort_by_key(|k| (k.matches('/').count(), k.len(), k.clone()));
        net.names = names;
    }
    let net_of = |union: &mut Union, key: usize| number[&union.find(key)];
    flat.terminals = terminals
        .into_iter()
        .map(|(path, pin, key, symbol)| SchTerminal {
            path,
            pin,
            net: net_of(&mut union, key),
            symbol,
        })
        .collect();
    flat.ports = ports
        .into_iter()
        .map(|(name, direction, key)| SchPort {
            name,
            direction,
            net: net_of(&mut union, key),
        })
        .collect();
    flat.nets = nets;
    flat
}

#[allow(clippy::too_many_arguments)]
fn walk(
    sheet: &SheetData,
    prefix: &str,
    sheets: &mut dyn Sheets,
    union: &mut Union,
    terminals: &mut Vec<(String, String, usize, String)>,
    flat: &mut FlatSchematic,
    depth: usize,
    open: &mut BTreeSet<(String, String)>,
) {
    for wanted in &sheet.schematic.wanted {
        let path = format!("{prefix}{}", wanted.name);
        let Some(pins) = sheet.nodemap.get(&wanted.name) else {
            flat.problems
                .push(format!("{path}: xschem does not list this instance"));
            continue;
        };
        match &wanted.target {
            Target::Pcell { .. } => {
                flat.devices.insert(path.clone(), wanted.symbol.clone());
                for (pin, net) in pins {
                    let key = union.id(&format!("{prefix}{net}"));
                    terminals.push((path.clone(), pin.clone(), key, wanted.symbol.clone()));
                }
            }
            Target::Cell { library, cell, .. } => {
                let id = (library.clone(), cell.clone());
                if depth >= MAX_DEPTH || open.contains(&id) {
                    flat.problems
                        .push(format!("{path}: {library}/{cell} places itself"));
                    continue;
                }
                let child = match sheets.sheet(library, cell) {
                    Ok(child) => child,
                    Err(e) => {
                        flat.problems.push(format!("{path}: {e:#}"));
                        continue;
                    }
                };
                let inner = format!("{path}/");
                for (pin, net) in pins {
                    let outer = union.id(&format!("{prefix}{net}"));
                    let port = union.id(&format!("{inner}{pin}"));
                    union.join(outer, port);
                }
                open.insert(id.clone());
                walk(
                    &child,
                    &inner,
                    sheets,
                    union,
                    terminals,
                    flat,
                    depth + 1,
                    open,
                );
                open.remove(&id);
            }
        }
    }
    // Nets with no device on them still exist (and pins name them).
    for pins in sheet.nodemap.values() {
        for (_, net) in pins {
            union.id(&format!("{prefix}{net}"));
        }
    }
}

// -- the layout side --------------------------------------------------------------

/// A net of the layout (top cell, flattened).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutNet {
    pub id: usize,
    /// The label on it; empty when it has none.
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub bbox: Option<[f64; 4]>,
}

/// A terminal of an extracted device.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutTerminal {
    pub name: String,
    pub net: Option<usize>,
    /// Middle of its shapes, µm.
    pub point: Option<[f64; 2]>,
}

/// A device KLayout extracted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutDevice {
    pub name: String,
    pub class: String,
    pub bbox: Option<[f64; 4]>,
    pub terminals: Vec<LayoutTerminal>,
}

/// A layout instance bound to a schematic instance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoundInstance {
    pub path: String,
    pub cell: String,
    pub bbox: Option<[f64; 4]>,
    /// A device (PCell, library cell) rather than a cell with bound instances inside.
    pub leaf: bool,
}

/// A text of the layout's top cell and the net it names.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Label {
    pub text: String,
    pub point: [f64; 2],
    /// `None`: on no shape of a net of that name.
    pub net: Option<usize>,
}

/// What `connectivity.py` says about the layout.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Connectivity {
    pub top: String,
    pub nets: Vec<LayoutNet>,
    pub devices: Vec<LayoutDevice>,
    #[serde(default)]
    pub instances: Vec<BoundInstance>,
    /// Texts of the top cell (the labels of its pins).
    #[serde(default)]
    pub labels: Vec<Label>,
}

impl Connectivity {
    pub fn read(file: &Path) -> Result<Connectivity> {
        let text = std::fs::read_to_string(file)
            .wrap_err_with(|| format!("cannot read {}", file.display()))?;
        serde_json::from_str(&text).map_err(|e| eyre!("{} is not understood: {e}", file.display()))
    }

    /// The net's name, `#id` for a net without one; `name#id` when nets of placed
    /// cells share the name (a label inside a cell names the net it is on).
    pub fn net_label(&self, id: usize) -> String {
        match self.nets.get(id) {
            Some(net) if !net.name.is_empty() => {
                if self.nets.iter().filter(|n| n.name == net.name).count() > 1 {
                    format!("{}#{id}", net.name)
                } else {
                    net.name.clone()
                }
            }
            _ => format!("#{id}"),
        }
    }

    /// Which bound leaf instance each device belongs to (by place): path → devices.
    pub fn assign(&self) -> (BTreeMap<String, Vec<usize>>, Vec<usize>) {
        let mut out: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        let mut extra = Vec::new();
        for (index, device) in self.devices.iter().enumerate() {
            let center = device.bbox.map(center).or_else(|| {
                let points: Vec<[f64; 2]> =
                    device.terminals.iter().filter_map(|t| t.point).collect();
                (!points.is_empty()).then(|| {
                    let n = points.len() as f64;
                    [
                        points.iter().map(|p| p[0]).sum::<f64>() / n,
                        points.iter().map(|p| p[1]).sum::<f64>() / n,
                    ]
                })
            });
            let owner = center.and_then(|c| {
                self.instances
                    .iter()
                    .filter(|i| i.leaf)
                    .filter_map(|i| i.bbox.map(|b| (i, b)))
                    .filter(|(_, b)| contains(*b, c))
                    .min_by(|(_, a), (_, b)| area(*a).total_cmp(&area(*b)))
                    .map(|(i, _)| i.path.clone())
            });
            match owner {
                Some(path) => out.entry(path).or_default().push(index),
                None => extra.push(index),
            }
        }
        (out, extra)
    }
}

// -- live connectivity ------------------------------------------------------------

/// The layout's connectivity as the editor traces it after edits, without a new
/// extraction (helper `live.py`, docs/LINKING.md §8): every device of the last
/// extraction that is still there, with its terminals on the nets traced now, and
/// where it is now (an instance moved since moves its devices along).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct LiveNets {
    /// The helper's revision of the layout traced.
    pub revision: u64,
    /// Time the tracing took, ms.
    #[serde(default)]
    pub ms: f64,
    /// The nets now (ids are new; names from the extraction).
    pub nets: Vec<LayoutNet>,
    pub devices: Vec<LiveDevice>,
    /// Bound instances of the top cell moved since the extraction: name → the change,
    /// `[m11, m12, m21, m22, dx, dy]` (µm).
    #[serde(default)]
    pub moved: BTreeMap<String, [f64; 6]>,
    /// Per label of the extraction: its net now.
    #[serde(default)]
    pub labels: Vec<Option<usize>>,
    /// Bound instances of the top cell that are new since the extraction (their
    /// devices wait for the next Update Connectivity).
    #[serde(default)]
    pub added: Vec<String>,
    /// Bound instances of the extraction that are gone.
    #[serde(default)]
    pub gone: Vec<String>,
    /// Terminals whose connection could not be traced on the layout (e.g. a MOS bulk
    /// through the well): kept as extracted.
    #[serde(default)]
    pub untraced: usize,
    /// Why the helper turned live updates off (too slow), else empty.
    #[serde(default)]
    pub stopped: String,
}

/// A device of the extraction as it is now.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LiveDevice {
    /// Index into [`Connectivity::devices`] of the extraction.
    pub index: usize,
    pub bbox: Option<[f64; 4]>,
    /// Per terminal of the extracted device: its net and point now.
    pub terminals: Vec<LiveTerminal>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LiveTerminal {
    pub net: Option<usize>,
    pub point: Option<[f64; 2]>,
}

fn transform_point(m: &[f64; 6], p: [f64; 2]) -> [f64; 2] {
    [
        m[0] * p[0] + m[1] * p[1] + m[4],
        m[2] * p[0] + m[3] * p[1] + m[5],
    ]
}

fn transform_box(m: &[f64; 6], b: [f64; 4]) -> [f64; 4] {
    let corners =
        [[b[0], b[1]], [b[2], b[1]], [b[2], b[3]], [b[0], b[3]]].map(|c| transform_point(m, c));
    let xs = corners.map(|c| c[0]);
    let ys = corners.map(|c| c[1]);
    [
        xs.iter().copied().fold(f64::INFINITY, f64::min),
        ys.iter().copied().fold(f64::INFINITY, f64::min),
        xs.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        ys.iter().copied().fold(f64::NEG_INFINITY, f64::max),
    ]
}

impl Connectivity {
    /// This extraction as the editor traced it now: devices with their nets and places
    /// from `live`; bound instances moved or gone; labels on their nets now.
    pub fn with_live(&self, live: &LiveNets) -> Connectivity {
        let devices = live
            .devices
            .iter()
            .filter_map(|d| {
                let base = self.devices.get(d.index)?;
                let terminals = base
                    .terminals
                    .iter()
                    .enumerate()
                    .map(|(i, t)| {
                        let now = d.terminals.get(i);
                        LayoutTerminal {
                            name: t.name.clone(),
                            net: now.and_then(|n| n.net),
                            point: now.and_then(|n| n.point).or(t.point),
                        }
                    })
                    .collect();
                Some(LayoutDevice {
                    name: base.name.clone(),
                    class: base.class.clone(),
                    bbox: d.bbox.or(base.bbox),
                    terminals,
                })
            })
            .collect();
        let top_name = |path: &str| path.split('/').next().unwrap_or(path).to_string();
        let instances = self
            .instances
            .iter()
            .filter(|i| !live.gone.contains(&top_name(&i.path)))
            .map(|i| {
                let mut i = i.clone();
                if let Some(m) = live.moved.get(&top_name(&i.path)) {
                    i.bbox = i.bbox.map(|b| transform_box(m, b));
                }
                i
            })
            .collect();
        let labels = self
            .labels
            .iter()
            .enumerate()
            .map(|(k, l)| Label {
                text: l.text.clone(),
                point: l.point,
                net: live.labels.get(k).copied().unwrap_or(l.net),
            })
            .collect();
        Connectivity {
            top: self.top.clone(),
            nets: live.nets.clone(),
            devices,
            instances,
            labels,
        }
    }
}

// -- how flylines look ------------------------------------------------------------

/// A line style for flylines (KLayout's built-in ones).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LineStyle {
    #[default]
    Solid,
    Dashed,
    Dotted,
    DashDotted,
}

impl LineStyle {
    pub const ALL: [LineStyle; 4] = [
        LineStyle::Solid,
        LineStyle::Dashed,
        LineStyle::Dotted,
        LineStyle::DashDotted,
    ];

    /// The name the helper takes (`nets_flylines` `style`).
    pub fn name(self) -> &'static str {
        match self {
            LineStyle::Solid => "solid",
            LineStyle::Dashed => "dashed",
            LineStyle::Dotted => "dotted",
            LineStyle::DashDotted => "dash-dotted",
        }
    }

    pub fn parse(text: &str) -> Option<LineStyle> {
        LineStyle::ALL.into_iter().find(|s| s.name() == text)
    }
}

/// How flylines and net highlights are drawn, and whether flylines follow the edits.
/// The user's (GUI settings), with the project's `[klayout.flylines]` on top
/// ([`FlylineOverride`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FlylineStyle {
    /// `#rrggbb`.
    pub color: String,
    /// Pixels.
    pub width: u32,
    pub line: LineStyle,
    /// Percent; below 100 the colour is blended with the background.
    pub opacity: u8,
    /// Colour of a net shown (highlighted).
    pub highlight: String,
    /// Colour of a shown net that is shorted with another.
    pub short: String,
    /// Flylines follow the edits (traced again on the layout after each change).
    pub live: bool,
    /// Quiet time after the last change before tracing, ms.
    pub delay_ms: u32,
}

impl Default for FlylineStyle {
    fn default() -> Self {
        FlylineStyle {
            color: "#ff40ff".into(),
            width: 2,
            line: LineStyle::Solid,
            opacity: 100,
            highlight: "#ffd400".into(),
            short: "#ff3030".into(),
            live: true,
            delay_ms: 300,
        }
    }
}

/// The project's choices for flylines (`[klayout.flylines]` in cdept.toml); what is not
/// given is the user's.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FlylineOverride {
    pub color: Option<String>,
    pub width: Option<u32>,
    pub line: Option<LineStyle>,
    pub opacity: Option<u8>,
    pub highlight: Option<String>,
    pub short: Option<String>,
    pub live: Option<bool>,
    pub delay_ms: Option<u32>,
}

impl FlylineOverride {
    pub fn is_empty(&self) -> bool {
        *self == FlylineOverride::default()
    }
}

impl FlylineStyle {
    /// This style with the project's choices on top.
    pub fn with(&self, project: &FlylineOverride) -> FlylineStyle {
        FlylineStyle {
            color: project.color.clone().unwrap_or_else(|| self.color.clone()),
            width: project.width.unwrap_or(self.width),
            line: project.line.unwrap_or(self.line),
            opacity: project.opacity.unwrap_or(self.opacity).min(100),
            highlight: project
                .highlight
                .clone()
                .unwrap_or_else(|| self.highlight.clone()),
            short: project.short.clone().unwrap_or_else(|| self.short.clone()),
            live: project.live.unwrap_or(self.live),
            delay_ms: project.delay_ms.unwrap_or(self.delay_ms),
        }
    }
}

/// `#rrggbb` (or `rrggbb`) as bytes.
pub fn parse_color(text: &str) -> Option<[u8; 3]> {
    let hex = text.trim().trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(hex, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

// -- parallel devices -------------------------------------------------------------

/// Groups of schematic devices in parallel: the same symbol and the same nets on
/// every pin (a MOS's S and D, and the two pins of a two-pin device, either way
/// round). Each group sorted, two members at least; groups sorted.
pub fn parallel_groups(flat: &FlatSchematic) -> Vec<Vec<String>> {
    let mut pins: BTreeMap<&str, Vec<(&str, usize)>> = BTreeMap::new();
    for t in &flat.terminals {
        pins.entry(t.path.as_str())
            .or_default()
            .push((t.pin.as_str(), t.net));
    }
    /// The symbol and its pins' nets, swappable pins as one sorted pair (`~`).
    type Key = (String, Vec<(String, Vec<usize>)>);
    let mut by_key: BTreeMap<Key, Vec<String>> = BTreeMap::new();
    for (path, list) in pins {
        let symbol = flat.devices.get(path).cloned().unwrap_or_default();
        let find = |n: &str| list.iter().position(|(p, _)| p.eq_ignore_ascii_case(n));
        let mut key: Vec<(String, Vec<usize>)> = Vec::new();
        let mut used = vec![false; list.len()];
        let pair = match (find("S"), find("D")) {
            (Some(a), Some(b)) => Some((a, b)),
            _ if list.len() == 2 => Some((0, 1)),
            _ => None,
        };
        if let Some((a, b)) = pair {
            let mut nets = vec![list[a].1, list[b].1];
            nets.sort_unstable();
            key.push(("~".into(), nets));
            used[a] = true;
            used[b] = true;
        }
        for (i, (pin, net)) in list.iter().enumerate() {
            if !used[i] {
                key.push((pin.to_ascii_uppercase(), vec![*net]));
            }
        }
        key.sort();
        by_key
            .entry((symbol, key))
            .or_default()
            .push(path.to_string());
    }
    let mut out: Vec<Vec<String>> = by_key
        .into_values()
        .filter(|g| g.len() > 1)
        .map(|mut g| {
            g.sort();
            g
        })
        .collect();
    out.sort();
    out
}

/// The parallel groups to use: those found in the schematic, each with the user's
/// choice when the sidecar recorded the same group, plus the user's own groups.
pub fn merge_parallel(detected: &[Vec<String>], recorded: &[Parallel]) -> Vec<Parallel> {
    let mut out: Vec<Parallel> = detected
        .iter()
        .map(|members| Parallel {
            members: members.clone(),
            joined: recorded
                .iter()
                .find(|r| &r.members == members)
                .is_none_or(|r| r.joined),
            manual: false,
        })
        .collect();
    for r in recorded.iter().filter(|r| r.manual) {
        if !out.iter().any(|o| o.members == r.members) {
            out.push(r.clone());
        }
    }
    out
}

/// The members of the groups that are joined.
pub fn joined_groups(groups: &[Parallel]) -> Vec<Vec<String>> {
    groups
        .iter()
        .filter(|g| g.joined)
        .map(|g| g.members.clone())
        .collect()
}

fn overlaps(a: [f64; 4], b: [f64; 4]) -> bool {
    a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3]
}

fn center(b: [f64; 4]) -> [f64; 2] {
    [(b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0]
}

fn contains(b: [f64; 4], p: [f64; 2]) -> bool {
    const SLACK: f64 = 1e-6;
    p[0] >= b[0] - SLACK && p[0] <= b[2] + SLACK && p[1] >= b[1] - SLACK && p[1] <= b[3] + SLACK
}

fn area(b: [f64; 4]) -> f64 {
    (b[2] - b[0]).max(0.0) * (b[3] - b[1]).max(0.0)
}

fn distance(a: [f64; 2], b: [f64; 2]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

// -- the comparison ---------------------------------------------------------------

/// A schematic terminal and where it is in the layout.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Placed {
    pub path: String,
    pub pin: String,
    /// The extracted device's terminal it is on.
    pub terminal: String,
    pub net: Option<usize>,
    pub point: Option<[f64; 2]>,
}

/// One piece of a schematic net in the layout: a layout net (or a terminal that is on
/// no net) and the schematic terminals on it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Piece {
    /// The layout net; `None`: a terminal that is not connected to anything.
    pub net: Option<usize>,
    pub terminals: Vec<Placed>,
    /// A schematic pin whose label names this layout net.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pins: Vec<String>,
}

/// A line to draw between two pieces of an open net, µm.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Flyline {
    pub from: [f64; 2],
    pub to: [f64; 2],
}

/// The state of a schematic net in the layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetStatus {
    /// All its terminals on one layout net, no other net's.
    Ok,
    /// On several layout nets.
    Open,
    /// On a layout net with terminals of another schematic net.
    Short,
    /// Both.
    OpenShort,
    /// None of its terminals are in the layout.
    Unplaced,
}

impl NetStatus {
    pub fn label(self) -> &'static str {
        match self {
            NetStatus::Ok => "ok",
            NetStatus::Open => "open",
            NetStatus::Short => "short",
            NetStatus::OpenShort => "open, short",
            NetStatus::Unplaced => "not placed",
        }
    }
}

/// A schematic net compared.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NetResult {
    pub name: String,
    pub status: NetStatus,
    pub pieces: Vec<Piece>,
    pub flylines: Vec<Flyline>,
    /// Other schematic nets sharing a layout net with this one (by name).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shorted_with: Vec<String>,
    /// Schematic terminals whose instance has no device in the layout.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing: Vec<String>,
}

impl NetResult {
    /// The layout nets this net is on.
    pub fn layout_nets(&self) -> Vec<usize> {
        let mut out: Vec<usize> = self.pieces.iter().filter_map(|p| p.net).collect();
        out.sort_unstable();
        out.dedup();
        out
    }
}

/// A layout net holding terminals of several schematic nets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Short {
    pub net: usize,
    pub label: String,
    pub schematic_nets: Vec<String>,
}

/// A schematic pin and its label in the layout.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PinResult {
    pub name: String,
    /// The layout net with that label, if any.
    pub net: Option<usize>,
    /// `ok`, or what is wrong.
    pub status: String,
}

/// The comparison of a schematic's nets with a layout's.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct NetReport {
    pub nets: Vec<NetResult>,
    pub shorts: Vec<Short>,
    pub pins: Vec<PinResult>,
    /// Schematic terminals on no layout net: `x1/M1.G`.
    pub unconnected: Vec<String>,
    /// Schematic devices with no device of the layout: paths.
    pub unplaced: Vec<String>,
    /// Devices of the layout that belong to no bound instance: `class name`.
    pub extra_devices: Vec<String>,
    pub problems: Vec<String>,
}

impl NetReport {
    /// Nothing open, shorted, unconnected or unplaced, and every pin labelled.
    pub fn clean(&self) -> bool {
        self.nets
            .iter()
            .all(|n| matches!(n.status, NetStatus::Ok | NetStatus::Unplaced))
            && self.shorts.is_empty()
            && self.unconnected.is_empty()
            && self.unplaced.is_empty()
            && self.pins.iter().all(|p| p.status == "ok")
    }

    pub fn net(&self, name: &str) -> Option<&NetResult> {
        self.nets.iter().find(|n| n.name == name)
    }

    /// The schematic nets on layout net `id`.
    pub fn nets_on(&self, id: usize) -> Vec<&NetResult> {
        self.nets
            .iter()
            .filter(|n| n.pieces.iter().any(|p| p.net == Some(id)))
            .collect()
    }

    pub fn flylines(&self) -> Vec<Flyline> {
        self.nets.iter().flat_map(|n| n.flylines.clone()).collect()
    }

    /// Counts for one line: (ok, open, short).
    pub fn counts(&self) -> (usize, usize, usize) {
        let mut out = (0, 0, 0);
        for net in &self.nets {
            match net.status {
                NetStatus::Ok => out.0 += 1,
                NetStatus::Open => out.1 += 1,
                NetStatus::Short => out.2 += 1,
                NetStatus::OpenShort => {
                    out.1 += 1;
                    out.2 += 1;
                }
                NetStatus::Unplaced => {}
            }
        }
        out
    }
}

/// Pairs of terminals that may be swapped (a MOS's source and drain, the two ends of
/// a resistor).
fn swappable(names: &[String]) -> Option<(usize, usize)> {
    let find = |n: &str| names.iter().position(|t| t.eq_ignore_ascii_case(n));
    if let (Some(s), Some(d)) = (find("S"), find("D")) {
        return Some((s, d));
    }
    if let (Some(a), Some(b)) = (find("A"), find("B"))
        && names.len() <= 3
    {
        return Some((a, b));
    }
    let one = names.iter().position(|t| t.ends_with("_1"));
    let two = names.iter().position(|t| t.ends_with("_2"));
    if let (Some(a), Some(b)) = (one, two)
        && names[a].trim_end_matches("_1") == names[b].trim_end_matches("_2")
    {
        return Some((a, b));
    }
    None
}

/// Which terminal of a device each symbol pin is on: index into the device's
/// terminals. `explicit`: the device map's `terminals`.
fn pin_terminals(
    pins: &[String],
    terminals: &[String],
    explicit: &BTreeMap<String, String>,
) -> Vec<Option<usize>> {
    let by_name = |name: &str| terminals.iter().position(|t| t.eq_ignore_ascii_case(name));
    if !explicit.is_empty() {
        return pins
            .iter()
            .map(|p| explicit.get(p).and_then(|t| by_name(t)))
            .collect();
    }
    let named: Vec<Option<usize>> = pins.iter().map(|p| by_name(p)).collect();
    if named.iter().all(Option::is_some) {
        return named;
    }
    // In order: rsil's P M → rsil_1 rsil_2, cmim's c0 c1 → mim_top mim_btm.
    (0..pins.len())
        .map(|i| (i < terminals.len()).then_some(i))
        .collect()
}

/// Compare the flattened schematic with the layout's connectivity.
///
/// `parallel`: groups of devices in parallel (instance paths, [`joined_groups`]): the
/// pieces of a net that are only members of one group count as one piece (one
/// flyline to the group), and a layout device that stands for several members (the
/// extraction combined them) belongs to each of them.
pub fn compare(
    flat: &FlatSchematic,
    layout: &Connectivity,
    map: &DeviceMap,
    parallel: &[Vec<String>],
) -> NetReport {
    let mut report = NetReport {
        problems: flat.problems.clone(),
        ..NetReport::default()
    };
    let (mut owned, mut extra) = layout.assign();
    // A member of a parallel group without a device of its own shares the one lying
    // over it that the extraction made for the group (combined devices).
    for group in parallel {
        for member in group {
            if owned.contains_key(member.as_str()) {
                continue;
            }
            let Some(own_box) = layout
                .instances
                .iter()
                .find(|i| &i.path == member)
                .and_then(|i| i.bbox)
            else {
                continue;
            };
            let mut found: Vec<usize> = Vec::new();
            for (index, device) in layout.devices.iter().enumerate() {
                let Some(b) = device.bbox else { continue };
                if !overlaps(b, own_box) {
                    continue;
                }
                let of_group = extra.contains(&index)
                    || group.iter().any(|m| {
                        m != member && owned.get(m.as_str()).is_some_and(|d| d.contains(&index))
                    });
                if of_group {
                    found.push(index);
                }
            }
            if !found.is_empty() {
                extra.retain(|i| !found.contains(i));
                owned.insert(member.clone(), found);
            }
        }
    }
    for index in extra {
        let d = &layout.devices[index];
        report.extra_devices.push(format!("{} {}", d.class, d.name));
    }
    // Schematic terminals by instance path.
    let mut by_path: BTreeMap<&str, Vec<&SchTerminal>> = BTreeMap::new();
    for t in &flat.terminals {
        by_path.entry(t.path.as_str()).or_default().push(t);
    }
    // (schematic net, placed) for every terminal of a placed device; per device the
    // two orientations when two terminals may be swapped.
    struct Option2 {
        net: usize,
        straight: Placed,
        swapped: Option<Placed>,
    }
    let mut groups: Vec<Vec<Option2>> = Vec::new();
    let mut missing: Vec<(usize, String)> = Vec::new();
    for (path, terms) in &by_path {
        let Some(devices) = owned.get(*path) else {
            report.unplaced.push((*path).to_string());
            for t in terms {
                missing.push((t.net, format!("{}.{}", t.path, t.pin)));
            }
            continue;
        };
        let symbol = &terms[0].symbol;
        let explicit = map
            .lookup(symbol)
            .map(|d| d.terminals.clone())
            .unwrap_or_default();
        let pins: Vec<String> = terms.iter().map(|t| t.pin.clone()).collect();
        for &index in devices {
            let device = &layout.devices[index];
            let names: Vec<String> = device.terminals.iter().map(|t| t.name.clone()).collect();
            let chosen = pin_terminals(&pins, &names, &explicit);
            let swap = swappable(&names);
            let mut group = Vec::new();
            for (t, choice) in terms.iter().zip(&chosen) {
                let place = |i: Option<usize>| {
                    let lt = i.and_then(|i| device.terminals.get(i));
                    Placed {
                        path: t.path.clone(),
                        pin: t.pin.clone(),
                        terminal: lt.map(|l| l.name.clone()).unwrap_or_default(),
                        net: lt.and_then(|l| l.net),
                        point: lt.and_then(|l| l.point),
                    }
                };
                let swapped = swap.and_then(|(a, b)| match *choice {
                    Some(i) if i == a => Some(place(Some(b))),
                    Some(i) if i == b => Some(place(Some(a))),
                    _ => None,
                });
                group.push(Option2 {
                    net: t.net,
                    straight: place(*choice),
                    swapped,
                });
            }
            groups.push(group);
        }
    }
    // Orientation: start straight; then each swappable device takes the orientation
    // that puts more of its terminals on layout nets its schematic nets already use.
    let mut orient = vec![false; groups.len()];
    for _ in 0..3 {
        let mut owner: BTreeMap<usize, BTreeMap<usize, usize>> = BTreeMap::new();
        for (g, group) in groups.iter().enumerate() {
            for o in group {
                let p = if orient[g] {
                    o.swapped.as_ref().unwrap_or(&o.straight)
                } else {
                    &o.straight
                };
                if let Some(net) = p.net {
                    *owner.entry(net).or_default().entry(o.net).or_default() += 1;
                }
            }
        }
        let mut changed = false;
        for (g, group) in groups.iter().enumerate() {
            if group.iter().all(|o| o.swapped.is_none()) {
                continue;
            }
            // What this device itself adds to the counts, in its present orientation.
            let mut own: BTreeMap<(usize, usize), usize> = BTreeMap::new();
            for o in group {
                let p = if orient[g] {
                    o.swapped.as_ref().unwrap_or(&o.straight)
                } else {
                    &o.straight
                };
                if let Some(net) = p.net {
                    *own.entry((net, o.net)).or_default() += 1;
                }
            }
            let score = |swapped: bool| -> i64 {
                group
                    .iter()
                    .map(|o| {
                        let p = if swapped {
                            o.swapped.as_ref().unwrap_or(&o.straight)
                        } else {
                            &o.straight
                        };
                        let Some(net) = p.net else { return 0 };
                        let Some(counts) = owner.get(&net) else {
                            return 0;
                        };
                        let count = |sch: usize| {
                            let all = *counts.get(&sch).unwrap_or(&0);
                            (all - own.get(&(net, sch)).copied().unwrap_or(0).min(all)) as i64
                        };
                        let mine = count(o.net);
                        let others: i64 = counts
                            .keys()
                            .filter(|k| **k != o.net)
                            .map(|k| count(*k))
                            .sum();
                        mine - others
                    })
                    .sum()
            };
            let want = score(true) > score(false);
            if want != orient[g] {
                orient[g] = want;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    // Per schematic net: its placed terminals.
    let mut placed: Vec<Vec<Placed>> = vec![Vec::new(); flat.nets.len()];
    for (g, group) in groups.into_iter().enumerate() {
        for o in group {
            let p = if orient[g] {
                o.swapped.unwrap_or(o.straight)
            } else {
                o.straight
            };
            if p.net.is_none() {
                report.unconnected.push(format!("{}.{}", p.path, p.pin));
            }
            placed[o.net].push(p);
        }
    }
    // Layout nets → schematic nets on them.
    let mut on_net: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    for (sch, terms) in placed.iter().enumerate() {
        for t in terms {
            if let Some(net) = t.net {
                on_net.entry(net).or_default().insert(sch);
            }
        }
    }
    // Pins: the layout net the pin's label (a text of the top cell) is on.
    let mut pin_nets: BTreeMap<usize, Vec<(String, usize)>> = BTreeMap::new();
    for port in &flat.ports {
        let labels: Vec<&Label> = layout
            .labels
            .iter()
            .filter(|l| l.text == port.name)
            .collect();
        let found = labels.iter().find_map(|l| l.net);
        let status = match found {
            None if labels.is_empty() => "no label in the layout".to_string(),
            None => "the label is on no wire".to_string(),
            Some(id) => match on_net.get(&id) {
                Some(s) if !s.contains(&port.net) => {
                    let names: Vec<String> = s.iter().map(|s| flat.nets[*s].name.clone()).collect();
                    format!("the label is on the layout net of {}", names.join(", "))
                }
                _ => "ok".to_string(),
            },
        };
        if let Some(id) = found {
            pin_nets
                .entry(port.net)
                .or_default()
                .push((port.name.clone(), id));
        }
        report.pins.push(PinResult {
            name: port.name.clone(),
            net: found,
            status,
        });
    }
    for (n, net) in flat.nets.iter().enumerate() {
        let terms = &placed[n];
        let net_missing: Vec<String> = missing
            .iter()
            .filter(|(m, _)| *m == n)
            .map(|(_, t)| t.clone())
            .collect();
        let pins_here = pin_nets.get(&n).cloned().unwrap_or_default();
        if terms.is_empty() && pins_here.is_empty() {
            if !net_missing.is_empty() {
                report.nets.push(NetResult {
                    name: net.name.clone(),
                    status: NetStatus::Unplaced,
                    pieces: Vec::new(),
                    flylines: Vec::new(),
                    shorted_with: Vec::new(),
                    missing: net_missing,
                });
            }
            continue;
        }
        let mut pieces: Vec<Piece> = Vec::new();
        for t in terms {
            match t.net {
                Some(id) => match pieces.iter_mut().find(|p| p.net == Some(id)) {
                    Some(piece) => piece.terminals.push(t.clone()),
                    None => pieces.push(Piece {
                        net: Some(id),
                        terminals: vec![t.clone()],
                        pins: Vec::new(),
                    }),
                },
                None => pieces.push(Piece {
                    net: None,
                    terminals: vec![t.clone()],
                    pins: Vec::new(),
                }),
            }
        }
        for (pin, id) in pins_here {
            match pieces.iter_mut().find(|p| p.net == Some(id)) {
                Some(piece) => piece.pins.push(pin),
                None => pieces.push(Piece {
                    net: Some(id),
                    terminals: Vec::new(),
                    pins: vec![pin],
                }),
            }
        }
        let mut shorted: BTreeSet<String> = BTreeSet::new();
        for piece in &pieces {
            if let Some(id) = piece.net
                && let Some(others) = on_net.get(&id)
            {
                for other in others {
                    if *other != n {
                        shorted.insert(flat.nets[*other].name.clone());
                    }
                }
            }
        }
        // Pieces that are only members of one parallel group count as one.
        let merged = merge_parallel_pieces(&pieces, parallel);
        let open = merged.len() > 1;
        let status = match (open, !shorted.is_empty()) {
            (false, false) => NetStatus::Ok,
            (true, false) => NetStatus::Open,
            (false, true) => NetStatus::Short,
            (true, true) => NetStatus::OpenShort,
        };
        let flylines = if open {
            spanning_tree(&merged, layout)
        } else {
            Vec::new()
        };
        report.nets.push(NetResult {
            name: net.name.clone(),
            status,
            pieces,
            flylines,
            shorted_with: shorted.into_iter().collect(),
            missing: net_missing,
        });
    }
    for (id, schs) in &on_net {
        if schs.len() > 1 {
            report.shorts.push(Short {
                net: *id,
                label: layout.net_label(*id),
                schematic_nets: schs.iter().map(|s| flat.nets[*s].name.clone()).collect(),
            });
        }
    }
    report
}

/// The pieces with those that hold only members of one parallel group made into one:
/// the group gets one flyline (to its nearest member), not one per member.
fn merge_parallel_pieces(pieces: &[Piece], parallel: &[Vec<String>]) -> Vec<Piece> {
    let group_of = |piece: &Piece| -> Option<usize> {
        if !piece.pins.is_empty() || piece.terminals.is_empty() {
            return None;
        }
        let first = parallel
            .iter()
            .position(|g| g.contains(&piece.terminals[0].path))?;
        piece
            .terminals
            .iter()
            .all(|t| parallel[first].contains(&t.path))
            .then_some(first)
    };
    let mut out: Vec<Piece> = Vec::new();
    let mut slot: BTreeMap<usize, usize> = BTreeMap::new();
    for piece in pieces {
        match group_of(piece) {
            Some(g) => match slot.get(&g) {
                Some(&at) => {
                    let into = &mut out[at];
                    into.terminals.extend(piece.terminals.iter().cloned());
                    if into.net != piece.net {
                        into.net = None;
                    }
                }
                None => {
                    slot.insert(g, out.len());
                    out.push(piece.clone());
                }
            },
            None => out.push(piece.clone()),
        }
    }
    out
}

/// The points of a piece flylines may start at: its terminals, else the middle of
/// its layout net.
fn piece_points(piece: &Piece, layout: &Connectivity) -> Vec<[f64; 2]> {
    let mut points: Vec<[f64; 2]> = piece.terminals.iter().filter_map(|t| t.point).collect();
    if points.is_empty()
        && let Some(id) = piece.net
        && let Some(b) = layout.nets.get(id).and_then(|n| n.bbox)
    {
        points.push(center(b));
    }
    points
}

/// Flylines joining the pieces of an open net: a minimum spanning tree (Prim) over the
/// pieces, each edge between the nearest points of the two pieces.
pub fn spanning_tree(pieces: &[Piece], layout: &Connectivity) -> Vec<Flyline> {
    let points: Vec<Vec<[f64; 2]>> = pieces.iter().map(|p| piece_points(p, layout)).collect();
    let nodes: Vec<usize> = (0..pieces.len())
        .filter(|i| !points[*i].is_empty())
        .collect();
    if nodes.len() < 2 {
        return Vec::new();
    }
    let nearest = |a: usize, b: usize| -> (f64, Flyline) {
        let mut best = (
            f64::INFINITY,
            Flyline {
                from: [0.0; 2],
                to: [0.0; 2],
            },
        );
        for p in &points[a] {
            for q in &points[b] {
                let d = distance(*p, *q);
                if d < best.0 {
                    best = (d, Flyline { from: *p, to: *q });
                }
            }
        }
        best
    };
    let mut in_tree = vec![false; pieces.len()];
    in_tree[nodes[0]] = true;
    let mut lines = Vec::new();
    for _ in 1..nodes.len() {
        let mut best: Option<(f64, Flyline, usize)> = None;
        for &a in nodes.iter().filter(|a| in_tree[**a]) {
            for &b in nodes.iter().filter(|b| !in_tree[**b]) {
                let (d, line) = nearest(a, b);
                if best.as_ref().is_none_or(|(bd, _, _)| d < *bd) {
                    best = Some((d, line, b));
                }
            }
        }
        let Some((_, line, b)) = best else { break };
        in_tree[b] = true;
        lines.push(line);
    }
    lines
}

// -- running it ---------------------------------------------------------------------

/// What a connectivity run made.
#[derive(Debug, Clone)]
pub struct NetRun {
    pub report: NetReport,
    pub connectivity: Connectivity,
    /// The schematic as compared (for comparing again with live connectivity).
    pub flat: FlatSchematic,
    pub map: DeviceMap,
    /// The parallel groups used: found in the schematic, with the sidecar's choices.
    pub parallel: Vec<Parallel>,
    /// The LVS database the layout's nets come from (the editor draws them from it).
    pub lvsdb: PathBuf,
    pub json: PathBuf,
    pub log: PathBuf,
}

/// Why there is no net check: the PDK gives no KLayout LVS deck.
pub const NO_DECK: &str = "this PDK has no KLayout LVS deck: net checking is not available \
     (a deck of your own can be set with [klayout] lvs in cdept.toml)";

/// The helper's script that describes an LVS database.
pub const CONNECTIVITY_SCRIPT: &str = "cdept_klayout/connectivity.py";

/// Extract the layout `stream` (with the PDK's LVS deck), read the schematic
/// `source`/`file` and below with xschem, and compare. `dir`: the work folder.
pub fn run(
    project: &Project,
    source: Source,
    file: &Path,
    stream: &crate::klayout::verify::Stream,
    dir: &Path,
    recorded: &[Parallel],
) -> Result<NetRun> {
    use crate::klayout::{setup, verify};
    let deck = verify::find_deck(project, verify::Check::Lvs).ok_or_else(|| eyre!("{NO_DECK}"))?;
    let map = DeviceMap::for_project(project)?;
    let netlist = dir.join(format!("{}.spice", stream.top));
    verify::schematic_netlist(project, file, &netlist)?;
    let lvs = verify::run_lvs_database(project, &deck, stream, &netlist, dir)?;
    let helper = setup::find_helper_dir()
        .ok_or_else(|| eyre!("the cdept-klayout helper scripts are not installed"))?;
    let json = dir.join("connectivity.json");
    let log = dir.join("connectivity.log");
    if json.exists() {
        std::fs::remove_file(&json)
            .wrap_err_with(|| format!("cannot remove {}", json.display()))?;
    }
    let vars = [
        ("lvsdb".to_string(), lvs.display().to_string()),
        (
            "input".to_string(),
            verify::absolute(&stream.file)?.display().to_string(),
        ),
        ("top".to_string(), stream.top.clone()),
        ("out".to_string(), json.display().to_string()),
    ];
    verify::run_batch(project, &helper.join(CONNECTIVITY_SCRIPT), &vars, dir, &log)?;
    let connectivity = Connectivity::read(&json)?;
    let flat = read_flat(project, &map, source, file)?;
    let parallel = merge_parallel(&parallel_groups(&flat), recorded);
    let report = compare(&flat, &connectivity, &map, &joined_groups(&parallel));
    Ok(NetRun {
        report,
        connectivity,
        flat,
        map,
        parallel,
        lvsdb: lvs,
        json,
        log,
    })
}

/// The schematic `source`/`file` and below, flattened (xschem reads them as saved).
pub fn read_flat(
    project: &Project,
    map: &DeviceMap,
    source: Source,
    file: &Path,
) -> Result<FlatSchematic> {
    let engine = xschem::Engine::for_project(project)?;
    let mut sheets = ProjectSheets {
        project,
        map,
        engine,
    };
    let top = sheets.read(source, file);
    let flat = top.map(|top| flatten(top, &mut sheets));
    let _ = sheets.engine.shutdown();
    flat
}

/// The parallel groups recorded in the sidecar of the layout `file` (none without one).
pub fn recorded_parallel(file: &Path) -> Vec<Parallel> {
    crate::layout::Meta::read_for(file)
        .ok()
        .flatten()
        .map(|m| m.parallel)
        .unwrap_or_default()
}

/// Write the parallel groups into the sidecar of the layout `file`: what
/// [`merge_parallel`] made, so the user's choices stay. The sidecar must exist.
pub fn record_parallel(file: &Path, groups: &[Parallel]) -> Result<()> {
    let mut meta = crate::layout::Meta::read_for(file)?
        .ok_or_else(|| eyre!("{} has no sidecar to record the groups in", file.display()))?;
    meta.parallel = groups.to_vec();
    meta.write_for(file)
}

/// The report as text, one finding per line.
pub fn report_lines(report: &NetReport, layout: &Connectivity) -> Vec<String> {
    let mut lines = Vec::new();
    for net in &report.nets {
        match net.status {
            NetStatus::Ok => {}
            NetStatus::Unplaced => lines.push(format!(
                "net {}: not placed ({})",
                net.name,
                net.missing.join(", ")
            )),
            _ => {
                let pieces: Vec<String> = net
                    .pieces
                    .iter()
                    .map(|p| {
                        let mut what: Vec<String> = p
                            .terminals
                            .iter()
                            .map(|t| format!("{}.{}", t.path, t.pin))
                            .collect();
                        what.extend(p.pins.iter().map(|n| format!("pin {n}")));
                        let on = match p.net {
                            Some(id) => layout.net_label(id),
                            None => "nothing".into(),
                        };
                        format!("{{{}}} on {on}", what.join(" "))
                    })
                    .collect();
                let mut line = format!("net {}: {}", net.name, net.status.label());
                if net.pieces.len() > 1 {
                    line.push_str(&format!(
                        ", {} pieces: {}",
                        net.pieces.len(),
                        pieces.join(", ")
                    ));
                }
                if !net.shorted_with.is_empty() {
                    line.push_str(&format!("; shorted with {}", net.shorted_with.join(", ")));
                }
                lines.push(line);
            }
        }
    }
    for short in &report.shorts {
        lines.push(format!(
            "short: layout net {} joins {}",
            short.label,
            short.schematic_nets.join(", ")
        ));
    }
    for t in &report.unconnected {
        lines.push(format!("unconnected: {t}"));
    }
    for p in &report.unplaced {
        lines.push(format!("not in the layout: {p}"));
    }
    for pin in report.pins.iter().filter(|p| p.status != "ok") {
        lines.push(format!("pin {}: {}", pin.name, pin.status));
    }
    for d in &report.extra_devices {
        lines.push(format!(
            "note: layout device {d} belongs to no schematic instance"
        ));
    }
    for p in &report.problems {
        lines.push(format!("problem: {p}"));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link::{Pin, Schematic, Wanted};

    fn pcell(name: &str, symbol: &str) -> Wanted {
        Wanted {
            name: name.into(),
            symbol: symbol.into(),
            target: Target::Pcell {
                library: "L".into(),
                cell: "nmos".into(),
                params: BTreeMap::new(),
            },
        }
    }

    fn cell(name: &str, of: &str) -> Wanted {
        Wanted {
            name: name.into(),
            symbol: format!("cells/{of}/symbol/{of}.sym"),
            target: Target::Cell {
                library: "cells".into(),
                cell: of.into(),
                view: "layout".into(),
                file: PathBuf::new(),
            },
        }
    }

    fn sheet(wanted: Vec<Wanted>, pins: &[&str], map: &str) -> SheetData {
        SheetData {
            schematic: Schematic {
                source: Source {
                    library: "cells".into(),
                    cell: "x".into(),
                    view: "schematic".into(),
                },
                file: PathBuf::new(),
                wanted,
                pins: pins
                    .iter()
                    .map(|p| Pin {
                        name: (*p).into(),
                        direction: "inout".into(),
                    })
                    .collect(),
                problems: Vec::new(),
                skipped: Vec::new(),
            },
            nodemap: parse_nodemap(map).unwrap(),
        }
    }

    /// The inverter of examples/ihp-inv: M1 nmos, M2 pmos.
    fn inv() -> SheetData {
        sheet(
            vec![
                pcell("M1", "sg13g2_pr/sg13_lv_nmos.sym"),
                pcell("M2", "sg13g2_pr/sg13_lv_pmos.sym"),
            ],
            &["A", "Y", "VDD", "VSS"],
            "{M1 {D Y G A S VSS B VSS}} {M2 {D VDD G A S Y B VDD}} {p1 {p A}} {p2 {p Y}} \
             {p3 {p VDD}} {p4 {p VSS}}",
        )
    }

    struct Fixed(BTreeMap<String, fn() -> SheetData>);

    impl Sheets for Fixed {
        fn sheet(&mut self, _library: &str, cell: &str) -> Result<SheetData> {
            self.0
                .get(cell)
                .map(|f| f())
                .ok_or_else(|| eyre!("no schematic {cell}"))
        }
    }

    #[test]
    fn nodemaps_are_read() {
        let map = parse_nodemap("{M1 {D Y G A S VSS B VSS}} {p1 {p A}} {l1 {}}").unwrap();
        assert_eq!(map["M1"][1], ("G".to_string(), "A".to_string()));
        assert_eq!(map["p1"], vec![("p".to_string(), "A".to_string())]);
        assert!(map["l1"].is_empty());
    }

    #[test]
    fn a_hierarchy_is_flattened() {
        let buf = sheet(
            vec![cell("x1", "inv"), cell("x2", "inv")],
            &["in", "out", "VDD", "VSS"],
            "{x1 {A in Y mid VDD VDD VSS VSS}} {x2 {A mid Y out VDD VDD VSS VSS}}",
        );
        let mut sheets = Fixed(BTreeMap::from([(
            "inv".to_string(),
            inv as fn() -> SheetData,
        )]));
        let flat = flatten(buf, &mut sheets);
        assert!(flat.problems.is_empty(), "{:?}", flat.problems);
        let net = |path: &str, pin: &str| {
            let t = flat
                .terminals
                .iter()
                .find(|t| t.path == path && t.pin == pin)
                .unwrap();
            flat.nets[t.net].name.clone()
        };
        assert_eq!(net("x1/M1", "D"), "mid");
        assert_eq!(net("x2/M2", "G"), "mid");
        assert_eq!(net("x1/M1", "G"), "in");
        assert_eq!(net("x2/M1", "S"), "VSS");
        assert_eq!(flat.devices.len(), 4);
        assert_eq!(flat.ports.len(), 4);
        let mid = &flat.nets[flat.net_named("mid").unwrap()];
        assert!(mid.names.contains(&"x1/Y".to_string()), "{:?}", mid.names);
    }

    #[test]
    fn a_cell_placing_itself_is_reported() {
        fn looped() -> SheetData {
            sheet(vec![cell("x1", "loop")], &[], "{x1 {}}")
        }
        let mut sheets = Fixed(BTreeMap::from([(
            "loop".to_string(),
            looped as fn() -> SheetData,
        )]));
        let flat = flatten(looped(), &mut sheets);
        assert!(flat.problems.iter().any(|p| p.contains("places itself")));
    }

    fn terminal(name: &str, net: Option<usize>, x: f64, y: f64) -> LayoutTerminal {
        LayoutTerminal {
            name: name.into(),
            net,
            point: Some([x, y]),
        }
    }

    fn mos(class: &str, nets: [usize; 4], x: f64, y: f64) -> LayoutDevice {
        LayoutDevice {
            name: "$".into(),
            class: class.into(),
            bbox: Some([x - 0.4, y - 0.4, x + 0.4, y + 0.4]),
            terminals: vec![
                terminal("S", Some(nets[0]), x - 0.2, y),
                terminal("G", Some(nets[1]), x, y),
                terminal("D", Some(nets[2]), x + 0.2, y),
                terminal("B", Some(nets[3]), x, y),
            ],
        }
    }

    fn bound(path: &str, x: f64, y: f64) -> BoundInstance {
        BoundInstance {
            path: path.into(),
            cell: "mos".into(),
            bbox: Some([x - 0.5, y - 0.5, x + 0.5, y + 0.5]),
            leaf: true,
        }
    }

    fn nets(names: &[&str]) -> Vec<LayoutNet> {
        names
            .iter()
            .enumerate()
            .map(|(id, n)| LayoutNet {
                id,
                name: (*n).into(),
                bbox: Some([0.0, 0.0, 1.0, 1.0]),
            })
            .collect()
    }

    fn labels(list: &[(&str, Option<usize>)]) -> Vec<Label> {
        list.iter()
            .map(|(text, net)| Label {
                text: (*text).into(),
                point: [0.0, 0.0],
                net: *net,
            })
            .collect()
    }

    fn flat_inv() -> FlatSchematic {
        let mut sheets = Fixed(BTreeMap::new());
        flatten(inv(), &mut sheets)
    }

    #[test]
    fn a_wired_inverter_is_clean() {
        // Nets: 0 A, 1 Y, 2 VDD, 3 VSS. Source and drain as the extraction found
        // them: M1's "D" is on VSS and "S" on Y (swapped against the schematic).
        let layout = Connectivity {
            top: "inv".into(),
            nets: nets(&["A", "Y", "VDD", "VSS"]),
            devices: vec![
                mos("sg13_lv_nmos", [1, 0, 3, 3], 0.0, 0.0),
                mos("sg13_lv_pmos", [1, 0, 2, 2], 0.0, 4.0),
            ],
            instances: vec![bound("M1", 0.0, 0.0), bound("M2", 0.0, 4.0)],
            labels: labels(&[
                ("A", Some(0)),
                ("Y", Some(1)),
                ("VDD", Some(2)),
                ("VSS", Some(3)),
            ]),
        };
        let report = compare(&flat_inv(), &layout, &DeviceMap::default(), &[]);
        assert!(report.clean(), "{:#?}", report_lines(&report, &layout));
        assert_eq!(report.counts(), (4, 0, 0));
    }

    #[test]
    fn opens_get_flylines_and_shorts_are_found() {
        // Nothing wired: every terminal on a net of its own (A's gates apart),
        // except Y and VSS shorted on net 1.
        let layout = Connectivity {
            top: "inv".into(),
            nets: nets(&["", "", "", "", "", "", "", ""]),
            devices: vec![
                mos("sg13_lv_nmos", [0, 1, 2, 3], 0.0, 0.0),
                mos("sg13_lv_pmos", [4, 5, 6, 7], 0.0, 4.0),
            ],
            instances: vec![bound("M1", 0.0, 0.0), bound("M2", 0.0, 4.0)],
            labels: labels(&[("A", None)]),
        };
        let report = compare(&flat_inv(), &layout, &DeviceMap::default(), &[]);
        assert!(!report.clean());
        let a = report.net("A").unwrap();
        assert_eq!(a.status, NetStatus::Open);
        assert_eq!(a.pieces.len(), 2);
        assert_eq!(a.flylines.len(), 1);
        assert_eq!(a.flylines[0].from[1].min(a.flylines[0].to[1]), 0.0);
        // Pins: A's label is on no wire, the others have none.
        assert_eq!(report.pins[0].status, "the label is on no wire");
        assert_eq!(report.pins[1].status, "no label in the layout");

        let mut shorted = layout.clone();
        // M1's drain (Y) and source (VSS) on one net.
        shorted.devices[0].terminals[2].net = Some(0);
        let report = compare(&flat_inv(), &shorted, &DeviceMap::default(), &[]);
        assert_eq!(report.shorts.len(), 1, "{:#?}", report.shorts);
        let mut names = report.shorts[0].schematic_nets.clone();
        names.sort();
        assert_eq!(names, vec!["VSS", "Y"]);
        assert!(matches!(
            report.net("Y").unwrap().status,
            NetStatus::Short | NetStatus::OpenShort
        ));
    }

    #[test]
    fn devices_not_in_the_layout_are_listed() {
        let layout = Connectivity {
            top: "inv".into(),
            nets: nets(&["A", "Y", "VDD", "VSS"]),
            devices: vec![
                mos("sg13_lv_nmos", [1, 0, 3, 3], 0.0, 0.0),
                mos("ntap1", [2, 2, 2, 2], 9.0, 9.0),
            ],
            instances: vec![bound("M1", 0.0, 0.0)],
            labels: Vec::new(),
        };
        let report = compare(&flat_inv(), &layout, &DeviceMap::default(), &[]);
        assert_eq!(report.unplaced, vec!["M2"]);
        assert_eq!(report.extra_devices, vec!["ntap1 $"]);
        assert!(!report.clean());
    }

    #[test]
    fn terminals_by_name_else_in_order_else_as_mapped() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let none = BTreeMap::new();
        assert_eq!(
            pin_terminals(&s(&["D", "G", "S", "B"]), &s(&["S", "G", "D", "B"]), &none),
            vec![Some(2), Some(1), Some(0), Some(3)]
        );
        assert_eq!(
            pin_terminals(
                &s(&["P", "M"]),
                &s(&["rsil_1", "rsil_2", "rsil_sub"]),
                &none
            ),
            vec![Some(0), Some(1)]
        );
        let explicit = BTreeMap::from([
            ("collector".to_string(), "C".to_string()),
            ("emitter".to_string(), "E".to_string()),
        ]);
        assert_eq!(
            pin_terminals(
                &s(&["emitter", "collector"]),
                &s(&["C", "B", "E"]),
                &explicit
            ),
            vec![Some(2), Some(0)]
        );
        assert_eq!(swappable(&s(&["S", "G", "D", "B"])), Some((0, 2)));
        assert_eq!(
            swappable(&s(&["rsil_1", "rsil_2", "rsil_sub"])),
            Some((0, 1))
        );
        assert_eq!(swappable(&s(&["mim_top", "mim_btm"])), None);
    }

    #[test]
    fn the_spanning_tree_takes_the_nearest_points() {
        let piece = |net: usize, points: &[[f64; 2]]| Piece {
            net: Some(net),
            terminals: points
                .iter()
                .map(|p| Placed {
                    path: "M".into(),
                    pin: "D".into(),
                    terminal: "D".into(),
                    net: Some(net),
                    point: Some(*p),
                })
                .collect(),
            pins: Vec::new(),
        };
        let pieces = vec![
            piece(0, &[[0.0, 0.0], [10.0, 0.0]]),
            piece(1, &[[11.0, 0.0]]),
            piece(2, &[[0.0, 5.0]]),
            piece(3, &[[20.0, 0.0]]),
        ];
        let lines = spanning_tree(&pieces, &Connectivity::default());
        assert_eq!(lines.len(), 3);
        let total: f64 = lines.iter().map(|l| distance(l.from, l.to)).sum();
        // 10→11 (1), 0→(0,5) (5), 11→20 (9).
        assert!((total - 15.0).abs() < 1e-9, "{lines:?}");
    }

    /// Two nmos in parallel (M1a, M1b; M1b with S and D the other way round), an
    /// nmos M3 with another gate, a pmos M2. Nets: 0 A, 1 Y, 2 VSS, 3 B, 4 VDD.
    fn flat_parallel() -> FlatSchematic {
        let names = ["A", "Y", "VSS", "B", "VDD"];
        let mut flat = FlatSchematic {
            nets: names
                .iter()
                .map(|n| FlatNet {
                    name: (*n).into(),
                    names: vec![(*n).into()],
                })
                .collect(),
            ..FlatSchematic::default()
        };
        let mut device = |path: &str, symbol: &str, pins: [(&str, usize); 4]| {
            flat.devices.insert(path.into(), symbol.into());
            for (pin, net) in pins {
                flat.terminals.push(SchTerminal {
                    path: path.into(),
                    pin: pin.into(),
                    net,
                    symbol: symbol.into(),
                });
            }
        };
        device("M1a", "nmos.sym", [("D", 1), ("G", 0), ("S", 2), ("B", 2)]);
        device("M1b", "nmos.sym", [("D", 2), ("G", 0), ("S", 1), ("B", 2)]);
        device("M3", "nmos.sym", [("D", 1), ("G", 3), ("S", 2), ("B", 2)]);
        device("M2", "pmos.sym", [("D", 1), ("G", 0), ("S", 4), ("B", 4)]);
        flat
    }

    #[test]
    fn devices_in_parallel_are_grouped() {
        let flat = flat_parallel();
        assert_eq!(
            parallel_groups(&flat),
            vec![vec!["M1a".to_string(), "M1b".into()]]
        );
        let recorded = vec![
            Parallel {
                members: vec!["M1a".into(), "M1b".into()],
                joined: false,
                manual: false,
            },
            Parallel {
                members: vec!["M2".into(), "M3".into()],
                joined: true,
                manual: true,
            },
            Parallel {
                members: vec!["old1".into(), "old2".into()],
                joined: true,
                manual: false,
            },
        ];
        let merged = merge_parallel(&parallel_groups(&flat), &recorded);
        // The user's choice stays, the user's own group stays, a group no longer
        // found goes.
        assert_eq!(merged.len(), 2);
        assert!(!merged[0].joined);
        assert!(merged[1].manual);
        assert_eq!(
            joined_groups(&merged),
            vec![vec!["M2".to_string(), "M3".into()]]
        );
    }

    #[test]
    fn a_parallel_group_gets_one_flyline() {
        // Y: M1a's drain on layout net 1, M1b's on 5, M3's and M2's on 6: three pieces.
        let mut a = mos("nmos", [2, 0, 1, 2], 0.0, 0.0);
        a.terminals[0].net = Some(2);
        let b = mos("nmos", [5, 0, 2, 2], 2.0, 0.0);
        let m3 = mos("nmos", [2, 3, 6, 2], 4.0, 0.0);
        let m2 = mos("pmos", [4, 0, 6, 4], 4.0, 4.0);
        let layout = Connectivity {
            top: "t".into(),
            nets: nets(&["A", "", "VSS", "B", "VDD", "", ""]),
            devices: vec![a, b, m3, m2],
            instances: vec![
                bound("M1a", 0.0, 0.0),
                bound("M1b", 2.0, 0.0),
                bound("M3", 4.0, 0.0),
                bound("M2", 4.0, 4.0),
            ],
            labels: Vec::new(),
        };
        let flat = flat_parallel();
        let alone = compare(&flat, &layout, &DeviceMap::default(), &[]);
        let y = alone.net("Y").unwrap();
        assert_eq!((y.pieces.len(), y.flylines.len()), (3, 2));
        let groups = parallel_groups(&flat);
        let joined = compare(&flat, &layout, &DeviceMap::default(), &groups);
        let y = joined.net("Y").unwrap();
        // The pieces are still all there (to show them); one flyline to the group.
        assert_eq!((y.pieces.len(), y.flylines.len()), (3, 1));
        assert_eq!(y.status, NetStatus::Open);
    }

    #[test]
    fn one_layout_device_may_stand_for_a_parallel_group() {
        // The extraction combined M1a and M1b into one device lying over both.
        let mut combined = mos("nmos", [2, 0, 1, 2], 1.0, 0.0);
        combined.bbox = Some([-0.4, -0.4, 2.4, 0.4]);
        let m3 = mos("nmos", [2, 3, 1, 2], 4.0, 0.0);
        let m2 = mos("pmos", [4, 0, 1, 4], 4.0, 4.0);
        let layout = Connectivity {
            top: "t".into(),
            nets: nets(&["A", "Y", "VSS", "B", "VDD"]),
            devices: vec![combined, m3, m2],
            instances: vec![
                bound("M1a", 0.0, 0.0),
                bound("M1b", 2.0, 0.0),
                bound("M3", 4.0, 0.0),
                bound("M2", 4.0, 4.0),
            ],
            labels: Vec::new(),
        };
        let flat = flat_parallel();
        let alone = compare(&flat, &layout, &DeviceMap::default(), &[]);
        assert!(!alone.unplaced.is_empty() || !alone.extra_devices.is_empty());
        let joined = compare(
            &flat,
            &layout,
            &DeviceMap::default(),
            &parallel_groups(&flat),
        );
        assert!(joined.unplaced.is_empty(), "{:?}", joined.unplaced);
        assert!(
            joined.extra_devices.is_empty(),
            "{:?}",
            joined.extra_devices
        );
        assert_eq!(joined.net("Y").unwrap().status, NetStatus::Ok);
    }

    #[test]
    fn live_connectivity_replaces_nets_and_places() {
        let layout = Connectivity {
            top: "inv".into(),
            nets: nets(&["A", "Y", "VDD", "VSS"]),
            devices: vec![
                mos("sg13_lv_nmos", [1, 0, 3, 3], 0.0, 0.0),
                mos("sg13_lv_pmos", [1, 0, 2, 2], 0.0, 4.0),
            ],
            instances: vec![bound("M1", 0.0, 0.0), bound("M2", 0.0, 4.0)],
            labels: labels(&[("A", Some(0)), ("Y", Some(1))]),
        };
        let t = |net: usize, x: f64, y: f64| LiveTerminal {
            net: Some(net),
            point: Some([x, y]),
        };
        // M2 moved 10 µm right, its device with it; A and Y joined (one net 0).
        let live = LiveNets {
            revision: 7,
            nets: nets(&["A", "VDD", "VSS"]),
            devices: vec![
                LiveDevice {
                    index: 0,
                    bbox: None,
                    terminals: vec![
                        t(0, -0.2, 0.0),
                        t(0, 0.0, 0.0),
                        t(2, 0.2, 0.0),
                        t(2, 0.0, 0.0),
                    ],
                },
                LiveDevice {
                    index: 1,
                    bbox: Some([9.6, 3.6, 10.4, 4.4]),
                    terminals: vec![
                        t(0, 9.8, 4.0),
                        t(0, 10.0, 4.0),
                        t(1, 10.2, 4.0),
                        t(1, 10.0, 4.0),
                    ],
                },
            ],
            moved: BTreeMap::from([("M2".to_string(), [1.0, 0.0, 0.0, 1.0, 10.0, 0.0])]),
            labels: vec![Some(0), Some(0)],
            ..LiveNets::default()
        };
        let now = layout.with_live(&live);
        assert_eq!(now.nets.len(), 3);
        assert_eq!(now.instances[1].bbox, Some([9.5, 3.5, 10.5, 4.5]));
        assert_eq!(now.devices[1].terminals[1].point, Some([10.0, 4.0]));
        assert_eq!(now.labels[1].net, Some(0));
        let report = compare(&flat_inv(), &now, &DeviceMap::default(), &[]);
        assert!(
            report
                .net("A")
                .unwrap()
                .shorted_with
                .contains(&"Y".to_string())
        );
        // An instance gone: its devices are not there, its schematic devices unplaced.
        let gone = LiveNets {
            devices: vec![live.devices[0].clone()],
            gone: vec!["M2".into()],
            ..live
        };
        let now = layout.with_live(&gone);
        assert_eq!(now.devices.len(), 1);
        assert_eq!(now.instances.len(), 1);
        let report = compare(&flat_inv(), &now, &DeviceMap::default(), &[]);
        assert_eq!(report.unplaced, vec!["M2"]);
    }

    #[test]
    fn flyline_style_with_the_projects_choices() {
        let user = FlylineStyle {
            width: 4,
            ..FlylineStyle::default()
        };
        let project = FlylineOverride {
            color: Some("#00ff00".into()),
            line: Some(LineStyle::Dotted),
            ..FlylineOverride::default()
        };
        let style = user.with(&project);
        assert_eq!(
            (style.color.as_str(), style.width, style.line),
            ("#00ff00", 4, LineStyle::Dotted)
        );
        assert_eq!(parse_color("#ff8000"), Some([255, 128, 0]));
        assert_eq!(parse_color("nope"), None);
        assert_eq!(LineStyle::parse("dash-dotted"), Some(LineStyle::DashDotted));
        let text = serde_json::to_string(&style).unwrap();
        assert!(text.contains("\"line\":\"dotted\""), "{text}");
    }
}
