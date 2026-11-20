//! Linking a schematic with a layout, Layout XL style (docs/LINKING.md).
//!
//! - [`schematic`]: the instances of an xschem schematic and the defaults of their
//!   symbols;
//! - [`devices`]: the device map, symbol → PCell of a KLayout library;
//! - here: what each schematic instance should become ([`Schematic`]), what the layout
//!   has ([`LayoutState`]), the difference ([`Plan`]), the request for the helper's
//!   `link_apply` and the record in the layout's sidecar.
//!
//! A layout instance belongs to a schematic instance through a GDS property,
//! [`NAME_PROPERTY`], holding the schematic instance name. The sidecar
//! (`<layout>.json`, version 2) records the schematic (`source`) and what each instance
//! was made into at the last sync (`instances`).

pub mod devices;
pub mod schematic;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

use eyre::{Result, eyre};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::layout::{Linked, Meta, Source};
use crate::{Format, Project};
pub use devices::DeviceMap;

/// GDS property attribute (`PROPATTR`) of a layout instance holding the name of the
/// schematic instance it stands for. KLayout keeps it as the integer property key 61,
/// in GDS and (as `S_GDS_PROPERTY`) in OASIS.
pub const NAME_PROPERTY: u16 = 61;

/// What a schematic instance becomes in the layout.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Target {
    /// A PCell (or fixed cell) of a KLayout library, with its parameters.
    Pcell {
        library: String,
        cell: String,
        params: BTreeMap<String, String>,
    },
    /// The layout view of a cell of the project (a reference, as `place` makes).
    Cell {
        library: String,
        cell: String,
        view: String,
        file: PathBuf,
    },
}

impl Target {
    fn kind(&self) -> &'static str {
        match self {
            Target::Pcell { .. } => "pcell",
            Target::Cell { .. } => "cell",
        }
    }

    fn library_cell(&self) -> (&str, &str) {
        match self {
            Target::Pcell { library, cell, .. } | Target::Cell { library, cell, .. } => {
                (library, cell)
            }
        }
    }

    fn params(&self) -> BTreeMap<String, String> {
        match self {
            Target::Pcell { params, .. } => params.clone(),
            Target::Cell { .. } => BTreeMap::new(),
        }
    }

    /// `SG13_dev/nmos` or `logic/inv/layout`.
    pub fn label(&self) -> String {
        match self {
            Target::Pcell { library, cell, .. } => format!("{library}/{cell}"),
            Target::Cell {
                library,
                cell,
                view,
                ..
            } => format!("{library}/{cell}/{view}"),
        }
    }
}

/// One schematic instance and what it should be in the layout.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Wanted {
    pub name: String,
    pub symbol: String,
    pub target: Target,
}

/// A pin of the schematic (`ipin`, `opin`, `iopin`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pin {
    pub name: String,
    /// `in`, `out` or `inout`.
    pub direction: String,
}

/// A schematic instance that cannot be made into layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub name: String,
    pub symbol: String,
    pub reason: String,
}

/// A schematic, read for linking.
#[derive(Debug, Clone, PartialEq)]
pub struct Schematic {
    pub source: Source,
    pub file: PathBuf,
    pub wanted: Vec<Wanted>,
    pub pins: Vec<Pin>,
    pub problems: Vec<Problem>,
    /// Instances left out on purpose (labels, `lvs_ignore`, …), by name.
    pub skipped: Vec<String>,
}

/// Symbol types that never become layout.
const NOT_DEVICES: &[&str] = &[
    "label",
    "probe",
    "noconn",
    "logo",
    "title",
    "netlist_commands",
    "spice_probe",
    "launcher",
    "architecture",
    "port_attributes",
    "use",
    "package",
    "show_label",
    "code",
    "annotate",
];

fn truthy(value: Option<&String>) -> bool {
    value.is_some_and(|v| matches!(v.trim(), "true" | "1" | "yes" | "short" | "open"))
}

/// The schematic view of a cell: `view`, else `schematic`, else the first xschem
/// schematic.
pub fn schematic_view(
    project: &Project,
    library: &str,
    cell: &str,
    view: Option<&str>,
) -> Result<(Source, PathBuf)> {
    let lib = project
        .library(library)
        .ok_or_else(|| eyre!("there is no library '{library}'"))?;
    let c = lib
        .cell(cell)
        .ok_or_else(|| eyre!("library '{library}' has no cell '{cell}'"))?;
    let schematics = || {
        c.entries
            .iter()
            .filter(|e| e.format == Some(Format::XschemSchematic) && e.main_file.is_some())
    };
    let entry = match view {
        Some(view) => schematics()
            .find(|e| e.name == view)
            .ok_or_else(|| eyre!("{library}/{cell} has no schematic view '{view}'"))?,
        None => schematics()
            .find(|e| e.name == "schematic")
            .or_else(|| schematics().next())
            .ok_or_else(|| eyre!("{library}/{cell} has no xschem schematic"))?,
    };
    Ok((
        Source {
            library: library.to_string(),
            cell: cell.to_string(),
            view: entry.name.clone(),
        },
        entry.main_file.clone().unwrap_or_default(),
    ))
}

/// A project cell: library, cell, and its layout view (view name, file) if it has one.
type ProjectCell = (String, String, Option<(String, PathBuf)>);

/// The cell of the project a symbol file belongs to, with its layout view.
fn project_cell(project: &Project, symbol: &Path) -> Option<ProjectCell> {
    for lib in &project.libraries {
        if !symbol.starts_with(&lib.path) {
            continue;
        }
        for cell in &lib.cells {
            let owns = cell.entries.iter().any(|e| {
                e.files.iter().any(|f| f == symbol) || e.main_file.as_deref() == Some(symbol)
            });
            if !owns {
                continue;
            }
            let layout = crate::klayout::cell_layout(lib, &cell.name)
                .ok()
                .map(|file| {
                    let view = cell
                        .entries
                        .iter()
                        .find(|e| e.main_file.as_deref() == Some(file.as_path()))
                        .map_or_else(|| "layout".to_string(), |e| e.name.clone());
                    (view, file)
                });
            return Some((lib.name.clone(), cell.name.clone(), layout));
        }
    }
    None
}

/// Read a schematic for linking: every instance with what it becomes, through `map`.
pub fn read_schematic(
    project: &Project,
    map: &DeviceMap,
    source: Source,
    file: &Path,
) -> Result<Schematic> {
    let instances = schematic::read(file)?;
    let search = schematic::search_path(project);
    let mut out = Schematic {
        source,
        file: file.to_path_buf(),
        wanted: Vec::new(),
        pins: Vec::new(),
        problems: Vec::new(),
        skipped: Vec::new(),
    };
    let mut infos: BTreeMap<String, schematic::SymbolInfo> = BTreeMap::new();
    let mut names = BTreeSet::new();
    for inst in instances {
        let info = infos
            .entry(inst.symbol.clone())
            .or_insert_with(|| schematic::symbol_info(&inst.symbol, &search))
            .clone();
        let problem = |reason: String| Problem {
            name: inst.name.clone(),
            symbol: inst.symbol.clone(),
            reason,
        };
        let stem = Path::new(&inst.symbol)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        let pin_kind = match info.kind.as_str() {
            "ipin" | "opin" | "iopin" => Some(info.kind.clone()),
            "" if matches!(stem.as_str(), "ipin" | "opin" | "iopin") => Some(stem.clone()),
            _ => None,
        };
        if let Some(kind) = pin_kind {
            let name = inst
                .attributes
                .get("lab")
                .cloned()
                .unwrap_or_else(|| inst.name.clone());
            out.pins.push(Pin {
                name,
                direction: kind.trim_end_matches("pin").to_string(),
            });
            continue;
        }
        if NOT_DEVICES.contains(&info.kind.as_str())
            || truthy(inst.attributes.get("spice_ignore"))
            || truthy(inst.attributes.get("lvs_ignore"))
        {
            out.skipped.push(inst.name.clone());
            continue;
        }
        let device = map.lookup(&inst.symbol);
        let cell = info.file.as_deref().and_then(|f| project_cell(project, f));
        if device.is_none() && cell.is_none() && !info.has_format && info.kind != "subcircuit" {
            // Not a device (no netlist line), e.g. a title or a symbol that is only a
            // drawing.
            out.skipped.push(inst.name.clone());
            continue;
        }
        if inst.name.is_empty() {
            out.problems
                .push(problem("the instance has no name".into()));
            continue;
        }
        if !names.insert(inst.name.clone()) {
            out.problems
                .push(problem("another instance has the same name".into()));
            continue;
        }
        let target = if let Some(device) = device {
            let value = |name: &str| {
                inst.attributes
                    .get(name)
                    .or_else(|| info.template.get(name))
                    .cloned()
            };
            let mut params = BTreeMap::new();
            let mut failed = None;
            for (key, template) in &device.params {
                match devices::expand(template, &value) {
                    Ok(v) => {
                        params.insert(key.clone(), v);
                    }
                    Err(e) => {
                        failed = Some(format!("parameter {key}: {e}"));
                        break;
                    }
                }
            }
            if let Some(reason) = failed {
                out.problems.push(problem(reason));
                continue;
            }
            Target::Pcell {
                library: device.library.clone(),
                cell: device.cell.clone(),
                params,
            }
        } else if let Some((library, cell, layout)) = cell {
            match layout {
                Some((view, file)) => Target::Cell {
                    library,
                    cell,
                    view,
                    file,
                },
                None => {
                    out.problems
                        .push(problem(format!("{library}/{cell} has no layout view")));
                    continue;
                }
            }
        } else {
            let reason = match &info.file {
                None => "the symbol cannot be found".to_string(),
                Some(_) => "no device map entry for this symbol".to_string(),
            };
            out.problems.push(problem(reason));
            continue;
        };
        out.wanted.push(Wanted {
            name: inst.name.clone(),
            symbol: inst.symbol.clone(),
            target,
        });
    }
    Ok(out)
}

/// What the layout has: the instances of its top cell, by schematic instance name.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LayoutState {
    /// Name → placed cells (more than one: duplicates).
    pub bound: BTreeMap<String, Vec<String>>,
    /// Instances of the top cell without a name.
    pub unbound: usize,
}

impl LayoutState {
    /// Read the placements of the top cell `top` of a layout file.
    pub fn read(file: &Path, top: &str) -> Result<LayoutState> {
        let mut state = LayoutState::default();
        for p in crate::layout::read_placements(file, NAME_PROPERTY)? {
            if p.structure != top {
                continue;
            }
            match p.value {
                Some(name) if !name.is_empty() => state.bound.entry(name).or_default().push(p.cell),
                _ => state.unbound += 1,
            }
        }
        Ok(state)
    }
}

/// One instance whose layout differs from the schematic.
#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    pub name: String,
    /// What differs, for people.
    pub what: String,
}

/// The difference between a schematic and its layout.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Plan {
    /// Schematic instances with no layout instance.
    pub add: Vec<String>,
    /// Parameters changed in the schematic since the last sync.
    pub update: Vec<Change>,
    /// The instance is another device or cell now.
    pub replace: Vec<Change>,
    /// Layout instances bound to names the schematic does not have.
    pub extra: Vec<String>,
    /// Names bound to several layout instances (a copied instance).
    pub duplicates: Vec<String>,
    /// Instances of the top cell bound to nothing (not an error).
    pub unbound: usize,
    /// Schematic instances that cannot be made into layout.
    pub problems: Vec<Problem>,
    /// The sidecar links the layout with another schematic.
    pub other_source: Option<String>,
    /// In sync.
    pub ok: Vec<String>,
}

impl Plan {
    /// Compare a schematic with the layout (`state`) and the sidecar's record (`meta`).
    pub fn new(schematic: &Schematic, meta: Option<&Meta>, state: &LayoutState) -> Plan {
        let mut plan = Plan {
            unbound: state.unbound,
            problems: schematic.problems.clone(),
            ..Plan::default()
        };
        if let Some(source) = meta.and_then(|m| m.source.as_ref())
            && source != &schematic.source
        {
            plan.other_source = Some(source.label());
        }
        let record = meta.map(|m| &m.instances);
        for w in &schematic.wanted {
            if !state.bound.contains_key(&w.name) {
                plan.add.push(w.name.clone());
                continue;
            }
            let Some(linked) = record.and_then(|r| r.get(&w.name)) else {
                plan.update.push(Change {
                    name: w.name.clone(),
                    what: "not synced yet".into(),
                });
                continue;
            };
            let (library, cell) = w.target.library_cell();
            if linked.kind != w.target.kind() || linked.library != library || linked.cell != cell {
                plan.replace.push(Change {
                    name: w.name.clone(),
                    what: format!("{}/{} → {}", linked.library, linked.cell, w.target.label()),
                });
                continue;
            }
            let params = w.target.params();
            let changed: Vec<String> = params
                .iter()
                .filter(|(k, v)| linked.params.get(*k) != Some(v))
                .map(|(k, v)| match linked.params.get(k) {
                    Some(old) => format!("{k} {old} → {v}"),
                    None => format!("{k} = {v}"),
                })
                .collect();
            if changed.is_empty() {
                plan.ok.push(w.name.clone());
            } else {
                plan.update.push(Change {
                    name: w.name.clone(),
                    what: changed.join(", "),
                });
            }
        }
        let wanted: BTreeSet<&str> = schematic.wanted.iter().map(|w| w.name.as_str()).collect();
        for (name, cells) in &state.bound {
            if !wanted.contains(name.as_str()) {
                plan.extra.push(name.clone());
            }
            if cells.len() > 1 {
                plan.duplicates.push(name.clone());
            }
        }
        plan
    }

    /// Nothing to do and nothing wrong.
    pub fn in_sync(&self) -> bool {
        self.add.is_empty()
            && self.update.is_empty()
            && self.replace.is_empty()
            && self.extra.is_empty()
            && self.duplicates.is_empty()
            && self.problems.is_empty()
            && self.other_source.is_none()
    }
}

impl fmt::Display for Plan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(other) = &self.other_source {
            writeln!(f, "linked with another schematic: {other}")?;
        }
        for name in &self.add {
            writeln!(f, "add      {name}")?;
        }
        for c in &self.update {
            writeln!(f, "update   {}: {}", c.name, c.what)?;
        }
        for c in &self.replace {
            writeln!(f, "replace  {}: {}", c.name, c.what)?;
        }
        for name in &self.extra {
            writeln!(f, "extra    {name} (not in the schematic)")?;
        }
        for name in &self.duplicates {
            writeln!(
                f,
                "twice    {name} (several layout instances have this name)"
            )?;
        }
        for p in &self.problems {
            let name = if p.name.is_empty() { "?" } else { &p.name };
            writeln!(f, "problem  {name} ({}): {}", p.symbol, p.reason)?;
        }
        if self.unbound > 0 {
            writeln!(
                f,
                "note     {} layout instance(s) not linked to the schematic",
                self.unbound
            )?;
        }
        write!(
            f,
            "{} in sync, {} to add, {} to update, {} to replace, {} extra",
            self.ok.len(),
            self.add.len(),
            self.update.len(),
            self.replace.len(),
            self.extra.len()
        )
    }
}

/// The sidecar of a layout file, its top cell, and the layout state.
pub fn layout_state(file: &Path) -> Result<(Option<Meta>, LayoutState)> {
    let meta = Meta::read_for(file)?;
    let top = match &meta {
        Some(m) => m.top.clone(),
        None => crate::layout::read_summary(file)?
            .top_cells()
            .into_iter()
            .next()
            .unwrap_or_default(),
    };
    let state = LayoutState::read(file, &top)?;
    Ok((meta, state))
}

/// Options of [`apply_request`].
#[derive(Debug, Clone, Default)]
pub struct ApplyOptions {
    /// Delete layout instances bound to names the schematic does not have.
    pub remove: bool,
}

/// The parameters of the helper's `link_apply` (docs/LINKING.md §5): every wanted
/// instance (cells with their reference table), the pins, and the options.
pub fn apply_request(
    project: &Project,
    map: &DeviceMap,
    schematic: &Schematic,
    options: &ApplyOptions,
) -> Value {
    let items: Vec<Value> = schematic
        .wanted
        .iter()
        .map(|w| {
            let mut item = json!({ "name": w.name, "symbol": w.symbol });
            let target = serde_json::to_value(&w.target).unwrap_or(Value::Null);
            if let (Some(obj), Value::Object(t)) = (item.as_object_mut(), target) {
                obj.extend(t);
            }
            if let Target::Cell { file, .. } = &w.target {
                item["refs"] = serde_json::to_value(crate::layout::refs::resolve(project, file))
                    .unwrap_or(Value::Null);
            }
            item
        })
        .collect();
    json!({
        "items": items,
        "pins": schematic.pins,
        "pin_layer": map.pins.layer,
        "pin_size": map.pins.size,
        "remove": options.remove,
    })
}

/// What the helper's `link_apply` did.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Applied {
    #[serde(default)]
    pub added: Vec<String>,
    /// Instances that were bound to nothing and were taken over.
    #[serde(default)]
    pub bound: Vec<String>,
    #[serde(default)]
    pub updated: Vec<String>,
    #[serde(default)]
    pub replaced: Vec<String>,
    #[serde(default)]
    pub unchanged: Vec<String>,
    #[serde(default)]
    pub removed: Vec<String>,
    /// Bound to names the schematic does not have, left in place.
    #[serde(default)]
    pub extra: Vec<String>,
    #[serde(default)]
    pub duplicates: Vec<String>,
    #[serde(default)]
    pub pins: Vec<String>,
    #[serde(default)]
    pub failed: Vec<Failed>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Failed {
    pub name: String,
    pub error: String,
}

impl Applied {
    /// The schematic instances the layout has after the apply.
    pub fn present(&self) -> Vec<String> {
        let mut all: Vec<String> = [
            &self.added,
            &self.bound,
            &self.updated,
            &self.replaced,
            &self.unchanged,
        ]
        .into_iter()
        .flatten()
        .cloned()
        .collect();
        all.sort();
        all
    }
}

impl fmt::Display for Applied {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let list = |v: &[String]| {
            if v.is_empty() {
                String::new()
            } else {
                format!(" ({})", v.join(", "))
            }
        };
        writeln!(f, "added {}{}", self.added.len(), list(&self.added))?;
        writeln!(f, "taken over {}{}", self.bound.len(), list(&self.bound))?;
        writeln!(f, "updated {}{}", self.updated.len(), list(&self.updated))?;
        writeln!(
            f,
            "replaced {}{}",
            self.replaced.len(),
            list(&self.replaced)
        )?;
        writeln!(f, "removed {}{}", self.removed.len(), list(&self.removed))?;
        if !self.extra.is_empty() {
            writeln!(
                f,
                "left in place, not in the schematic: {}",
                self.extra.join(", ")
            )?;
        }
        if !self.duplicates.is_empty() {
            writeln!(f, "names used twice: {}", self.duplicates.join(", "))?;
        }
        for x in &self.failed {
            writeln!(f, "failed {}: {}", x.name, x.error)?;
        }
        write!(
            f,
            "{} unchanged, {} pin label(s) added",
            self.unchanged.len(),
            self.pins.len()
        )
    }
}

/// Record the link in the sidecar of the saved layout `file`: the schematic, and what
/// each instance in `present` was made into. The sidecar must exist (it does after
/// [`crate::layout::refs::update_after_save`]).
pub fn record(file: &Path, schematic: &Schematic, present: &[String]) -> Result<()> {
    let mut meta = Meta::read_for(file)?
        .ok_or_else(|| eyre!("{} has no sidecar to record the link in", file.display()))?;
    meta.source = Some(schematic.source.clone());
    meta.instances = schematic
        .wanted
        .iter()
        .filter(|w| present.contains(&w.name))
        .map(|w| {
            let (library, cell) = w.target.library_cell();
            (
                w.name.clone(),
                Linked {
                    symbol: w.symbol.clone(),
                    kind: w.target.kind().to_string(),
                    library: library.to_string(),
                    cell: cell.to_string(),
                    params: w.target.params(),
                },
            )
        })
        .collect();
    meta.write_for(file)
}

/// The schematic a layout file is linked with, from its sidecar.
pub fn source_of(file: &Path) -> Option<Source> {
    Meta::read_for(file).ok().flatten().and_then(|m| m.source)
}

/// The layout views linked with a schematic view, found through the sidecars of the
/// cell's layout views (a layout of the same cell without a link counts too: it is
/// what "Generate" would link).
pub fn layouts_of(project: &Project, source: &Source) -> Vec<PathBuf> {
    let Some(cell) = project
        .library(&source.library)
        .and_then(|l| l.cell(&source.cell))
    else {
        return Vec::new();
    };
    cell.entries
        .iter()
        .filter(|e| matches!(e.format, Some(Format::Gds | Format::Oasis)))
        .filter_map(|e| e.main_file.clone())
        .filter(|f| source_of(f).is_none_or(|s| &s == source))
        .collect()
}

/// The layout view to link a cell's schematic with: `view`, else the cell's layout
/// (`layout`, else the first GDS/OASIS view). `create`: make a GDS `layout` view from
/// the template when the cell has none (the project is scanned again).
pub fn layout_view(
    project: &mut Project,
    library: &str,
    cell: &str,
    view: Option<&str>,
    create: bool,
) -> Result<PathBuf> {
    let lib = project
        .library(library)
        .ok_or_else(|| eyre!("there is no library '{library}'"))?;
    if let Some(view) = view {
        return lib
            .cell(cell)
            .and_then(|c| c.entry(view))
            .filter(|e| matches!(e.format, Some(Format::Gds | Format::Oasis)))
            .and_then(|e| e.main_file.clone())
            .ok_or_else(|| eyre!("{library}/{cell} has no GDS or OASIS view '{view}'"));
    }
    if let Ok(file) = crate::klayout::cell_layout(lib, cell) {
        return Ok(file);
    }
    if !create {
        return Err(eyre!("{library}/{cell} has no layout view"));
    }
    let file = lib.create_view(cell, crate::Template::GdsLayout)?;
    project.rescan();
    Ok(file)
}

/// Read everything needed to compare `library/cell`'s schematic with its layout.
pub fn prepare(
    project: &Project,
    library: &str,
    cell: &str,
    schematic: Option<&str>,
) -> Result<(DeviceMap, Schematic)> {
    let map = DeviceMap::for_project(project)?;
    let (source, file) = schematic_view(project, library, cell, schematic)?;
    let schematic = read_schematic(project, &map, source, &file)?;
    Ok((map, schematic))
}

/// The plan for a layout file against a schematic.
pub fn check(schematic: &Schematic, layout: &Path) -> Result<Plan> {
    let (meta, state) = layout_state(layout)?;
    Ok(Plan::new(schematic, meta.as_ref(), &state))
}

/// Generate or update `layout` from `schematic` with a KLayout engine: apply, save,
/// write the sidecar (references and link). Returns what the helper did.
pub fn generate(
    project: &Project,
    engine: &mut crate::klayout::Engine,
    map: &DeviceMap,
    schematic: &Schematic,
    layout: &Path,
    options: &ApplyOptions,
) -> Result<Applied> {
    let refs = crate::layout::refs::resolve(project, layout);
    engine.open(&refs, true)?;
    let value = engine.call(
        "link_apply",
        apply_request(project, map, schematic, options),
    )?;
    let applied: Applied = serde_json::from_value(value)
        .map_err(|e| eyre!("the helper's link_apply answer is not understood: {e}"))?;
    let report = engine.save(None)?;
    crate::layout::refs::update_after_save(project, &report)?;
    record(layout, schematic, &applied.present())?;
    Ok(applied)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wanted(name: &str, w: &str) -> Wanted {
        Wanted {
            name: name.into(),
            symbol: "sg13g2_pr/sg13_lv_nmos.sym".into(),
            target: Target::Pcell {
                library: "SG13_dev".into(),
                cell: "nmos".into(),
                params: BTreeMap::from([("w".to_string(), w.to_string())]),
            },
        }
    }

    fn schematic(wanted: Vec<Wanted>) -> Schematic {
        Schematic {
            source: Source {
                library: "a".into(),
                cell: "inv".into(),
                view: "schematic".into(),
            },
            file: PathBuf::from("/x/inv.sch"),
            wanted,
            pins: Vec::new(),
            problems: Vec::new(),
            skipped: Vec::new(),
        }
    }

    #[test]
    fn plans_compare_schematic_layout_and_record() {
        let sch = schematic(vec![
            wanted("M1", "1u"),
            wanted("M2", "2u"),
            wanted("M3", "1u"),
            wanted("M4", "1u"),
        ]);
        let mut meta = Meta::new("a", "inv", "layout", "inv");
        meta.source = Some(sch.source.clone());
        for (name, w) in [("M2", "1u"), ("M3", "1u")] {
            meta.instances.insert(
                name.into(),
                Linked {
                    symbol: "s".into(),
                    kind: "pcell".into(),
                    library: "SG13_dev".into(),
                    cell: "nmos".into(),
                    params: BTreeMap::from([("w".to_string(), w.to_string())]),
                },
            );
        }
        meta.instances.insert(
            "M4".into(),
            Linked {
                symbol: "s".into(),
                kind: "pcell".into(),
                library: "SG13_dev".into(),
                cell: "pmos".into(),
                params: BTreeMap::new(),
            },
        );
        let state = LayoutState {
            bound: BTreeMap::from([
                ("M2".to_string(), vec!["nmos".to_string()]),
                (
                    "M3".to_string(),
                    vec!["nmos".to_string(), "nmos".to_string()],
                ),
                ("M4".to_string(), vec!["pmos".to_string()]),
                ("M9".to_string(), vec!["nmos".to_string()]),
            ]),
            unbound: 2,
        };
        let plan = Plan::new(&sch, Some(&meta), &state);
        assert_eq!(plan.add, ["M1"]);
        assert_eq!(plan.update.len(), 1);
        assert_eq!(plan.update[0].name, "M2");
        assert!(
            plan.update[0].what.contains("w 1u → 2u"),
            "{}",
            plan.update[0].what
        );
        assert_eq!(plan.replace[0].name, "M4");
        assert_eq!(plan.extra, ["M9"]);
        assert_eq!(plan.duplicates, ["M3"]);
        assert_eq!(plan.ok, ["M3"]);
        assert_eq!(plan.unbound, 2);
        assert!(!plan.in_sync());
        let text = plan.to_string();
        assert!(text.contains("add      M1"), "{text}");

        // In sync once the layout and the record match.
        let sch = schematic(vec![wanted("M3", "1u")]);
        let state = LayoutState {
            bound: BTreeMap::from([("M3".to_string(), vec!["nmos".to_string()])]),
            unbound: 0,
        };
        let plan = Plan::new(&sch, Some(&meta), &state);
        assert!(plan.in_sync(), "{plan}");
        // Linked with another schematic: not in sync.
        let mut other = meta.clone();
        other.source.as_mut().unwrap().cell = "buf".into();
        assert!(Plan::new(&sch, Some(&other), &state).other_source.is_some());
    }
}
