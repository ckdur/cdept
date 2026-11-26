//! Magic layouts made from and checked against xschem schematics (docs/MAGIC.md,
//! "Layout from schematic").
//!
//! The schematic is read with [`crate::link::read_schematic`] and a device map in the
//! format of the KLayout one (`cdept-devices`), whose entries name the PDK's Magic
//! device generators: `library` is the generator's Tcl namespace (`sky130`,
//! `sg13g2`), `cell` the device, `params` its parameters
//! (`pdk/<PDK>/magic-devices.toml`). Subcircuits of project cells become instances of
//! their Magic views.
//!
//! The link between a schematic instance and the layout is the Magic instance name
//! (the `use` id): `M1` in the schematic is the use `M1` in the layout. What each
//! instance was made into is recorded in the layout's sidecar (`source`, `instances`),
//! as for KLayout layouts.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use eyre::{Result, WrapErr, bail, eyre};

use super::mag;
use super::tcl::word;
use crate::layout::meta::{Linked, Meta, Source};
use crate::link::devices::{DeviceMap, spice_number};
use crate::link::{Schematic, Target, Wanted};
use crate::{Format, Project};

/// The shipped Magic device map of a PDK (`pdk/<pdk>/magic-devices.toml`, searched
/// where [`crate::link::devices::pdk_map`] looks).
pub fn pdk_map(pdk: &str) -> Option<PathBuf> {
    let klayout = crate::link::devices::pdk_map(pdk);
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(dir) = std::env::var_os("CDEPT_PDK_DATA").filter(|v| !v.is_empty()) {
        dirs.push(PathBuf::from(dir));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(prefix) = exe.parent().and_then(Path::parent)
    {
        dirs.push(prefix.join("share/cdept/pdk"));
    }
    if let Some(tree) = Path::new(env!("CARGO_MANIFEST_DIR")).parent() {
        dirs.push(tree.join("pdk"));
    }
    if let Some(dir) = klayout
        .as_deref()
        .and_then(Path::parent)
        .and_then(Path::parent)
    {
        dirs.insert(0, dir.to_path_buf());
    }
    dirs.into_iter()
        .map(|d| d.join(pdk).join("magic-devices.toml"))
        .find(|f| f.is_file())
}

/// The device map for Magic in `project`: the PDK's.
pub fn device_map(project: &Project) -> Result<DeviceMap> {
    let mut map = DeviceMap::default();
    if let Some(pdk) = project.pdk.var("PDK")
        && let Some(file) = pdk_map(pdk)
    {
        map.add_file(&file)?;
    }
    Ok(map)
}

/// The Magic view of a project cell.
fn mag_view(project: &Project, library: &str, cell: &str) -> Option<(String, PathBuf)> {
    let c = project.library(library)?.cell(cell)?;
    let mut views = c
        .entries
        .iter()
        .filter(|e| e.format == Some(Format::Magic) && e.main_file.is_some());
    let e = views
        .clone()
        .find(|e| e.name == "layout")
        .or_else(|| views.next())?;
    Some((e.name.clone(), e.main_file.clone()?))
}

/// The schematic `file` (the view `source`) read for a Magic layout: devices through
/// the Magic device map, subcircuits as the Magic views of their cells.
pub fn read_schematic(project: &Project, source: Source, file: &Path) -> Result<Schematic> {
    let map = device_map(project)?;
    let mut s = crate::link::read_schematic(project, &map, source, file)?;
    // Project cells: their Magic view, whatever layout KLayout would have taken.
    for w in &mut s.wanted {
        if let Target::Cell { library, cell, .. } = &w.target
            && let Some((view, file)) = mag_view(project, library, cell)
        {
            w.target = Target::Cell {
                library: library.clone(),
                cell: cell.clone(),
                view,
                file,
            };
        }
    }
    // "has no layout view" (no GDS): fine when there is a Magic one.
    let mut problems = Vec::new();
    for p in std::mem::take(&mut s.problems) {
        let found = p
            .reason
            .strip_suffix(" has no layout view")
            .and_then(|lc| lc.split_once('/'))
            .and_then(|(l, c)| Some((l, c, mag_view(project, l, c)?)));
        match found {
            Some((library, cell, (view, file))) => s.wanted.push(Wanted {
                name: p.name.clone(),
                symbol: p.symbol.clone(),
                target: Target::Cell {
                    library: library.into(),
                    cell: cell.into(),
                    view,
                    file,
                },
            }),
            None => problems.push(p),
        }
    }
    s.problems = problems;
    Ok(s)
}

/// One instance of the layout's top cell.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Placed {
    pub instance: String,
    /// The cell placed (`sky130_fd_pr__nfet_01v8_YNZVB4`, `inv`).
    pub cell: String,
    /// For a generated device: generator library and device, and its parameters.
    pub generator: Option<(String, String)>,
    pub params: BTreeMap<String, String>,
    /// Position (internal units) and placed cell's box, for "where to put new ones".
    pub transform: Option<[i64; 6]>,
    pub bbox: Option<[i64; 4]>,
}

/// The instances of `file`, with the generator parameters of the devices (read from
/// the device cells' files next to it, or found on the search path).
pub fn placed(project: &Project, file: &Path) -> Result<Vec<Placed>> {
    let m = mag::read(file)?;
    let resolved = super::refs::resolve_file(project, file).unwrap_or_default();
    let dir = file.parent().unwrap_or(Path::new("."));
    let mut out = Vec::new();
    for u in &m.uses {
        let child = resolved
            .iter()
            .find(|r| r.placed.instance == u.instance && r.placed.cell == u.cell)
            .and_then(|r| match &r.target {
                super::refs::Target::Project(v) => Some(v.file.clone()),
                super::refs::Target::External(f) => Some(f.clone()),
                super::refs::Target::Missing => None,
            })
            .or_else(|| {
                let f = dir.join(format!("{}.mag", u.cell));
                f.is_file().then_some(f)
            });
        let mut p = Placed {
            instance: u.instance.clone(),
            cell: u.cell.clone(),
            transform: u.transform,
            bbox: u.bbox,
            ..Default::default()
        };
        if let Some(child) = child
            && let Ok(cm) = mag::read(&child)
            && let Some(device) = cm.properties.get("gencell")
        {
            let library = cm.properties.get("library").cloned().unwrap_or_default();
            p.generator = Some((library, device.clone()));
            p.params = tcl_dict(cm.properties.get("parameters").map_or("", |s| s.as_str()));
        }
        out.push(p);
    }
    Ok(out)
}

/// A flat Tcl list of key value pairs (Magic's `parameters` property). Values in
/// braces are taken as one.
fn tcl_dict(text: &str) -> BTreeMap<String, String> {
    let words = crate::xschem::tcl::parse_list(text).unwrap_or_default();
    words
        .chunks(2)
        .filter(|c| c.len() == 2)
        .map(|c| (c[0].clone(), c[1].clone()))
        .collect()
}

/// What generate/update does to one instance.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Add(Wanted),
    /// Same instance name, other device or other parameters: (wanted, what is there).
    Update(Wanted, Placed),
    /// In the layout, not in the schematic.
    Extra(Placed),
}

/// The plan to bring a layout in line with its schematic.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Plan {
    pub actions: Vec<Action>,
    /// Instances that are as they should be.
    pub ok: Vec<String>,
    /// Schematic instances that cannot become layout, with the reason.
    pub problems: Vec<(String, String)>,
}

impl Plan {
    pub fn in_sync(&self) -> bool {
        self.actions.is_empty() && self.problems.is_empty()
    }

    /// One line per difference, for the user.
    pub fn lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        for a in &self.actions {
            out.push(match a {
                Action::Add(w) => {
                    format!("{}: missing in the layout ({})", w.name, w.target.label())
                }
                Action::Update(w, p) => format!(
                    "{}: {} in the layout, the schematic says {}{}",
                    w.name,
                    describe_as(p, &w.target),
                    w.target.label(),
                    params_text(&w.target)
                ),
                Action::Extra(p) => {
                    format!("{}: {} is not in the schematic", p.instance, describe(p))
                }
            });
        }
        for (name, reason) in &self.problems {
            out.push(format!("{name}: {reason}"));
        }
        out
    }
}

fn params_text(t: &Target) -> String {
    match t {
        Target::Pcell { params, .. } if !params.is_empty() => {
            let p: Vec<String> = params.iter().map(|(k, v)| format!("{k}={v}")).collect();
            format!(" ({})", p.join(" "))
        }
        _ => String::new(),
    }
}

/// What is placed, with the parameters the schematic sets (not all the generator has).
fn describe_as(p: &Placed, wanted: &Target) -> String {
    match (&p.generator, wanted) {
        (Some((lib, dev)), Target::Pcell { params, .. }) => {
            let shown: Vec<String> = params
                .keys()
                .map(|k| format!("{k}={}", p.params.get(k).map_or("-", |v| v.as_str())))
                .collect();
            format!("{lib}/{dev} ({})", shown.join(" "))
        }
        _ => describe(p),
    }
}

fn describe(p: &Placed) -> String {
    match &p.generator {
        Some((lib, dev)) => {
            let params: Vec<String> = p.params.iter().map(|(k, v)| format!("{k}={v}")).collect();
            let shown: Vec<&String> = params.iter().take(6).collect();
            format!(
                "{lib}/{dev} ({})",
                shown
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            )
        }
        None => p.cell.clone(),
    }
}

/// Two parameter values are the same: equal text, or equal numbers.
fn same_value(a: &str, b: &str) -> bool {
    if a.trim() == b.trim() {
        return true;
    }
    match (spice_number(a), spice_number(b)) {
        (Some(x), Some(y)) => (x - y).abs() <= 1e-9 * x.abs().max(y.abs()).max(1e-30),
        _ => false,
    }
}

/// Compare the schematic with what the layout has.
pub fn plan(schematic: &Schematic, placed: &[Placed]) -> Plan {
    let mut out = Plan {
        problems: schematic
            .problems
            .iter()
            .map(|p| (p.name.clone(), p.reason.clone()))
            .collect(),
        ..Default::default()
    };
    let by_name: BTreeMap<&str, &Placed> =
        placed.iter().map(|p| (p.instance.as_str(), p)).collect();
    for w in &schematic.wanted {
        let Some(p) = by_name.get(w.name.as_str()) else {
            out.actions.push(Action::Add(w.clone()));
            continue;
        };
        let matches = match &w.target {
            Target::Pcell {
                library,
                cell,
                params,
            } => {
                p.generator.as_ref() == Some(&(library.clone(), cell.clone()))
                    && params.iter().all(|(k, v)| {
                        // `m` of a single device may be left out by the generator
                        p.params.get(k).map_or(v == "1", |have| same_value(v, have))
                    })
            }
            Target::Cell { cell, .. } => p.cell == *cell,
        };
        if matches {
            out.ok.push(w.name.clone());
        } else {
            out.actions.push(Action::Update(w.clone(), (*p).clone()));
        }
    }
    let wanted: std::collections::BTreeSet<&str> =
        schematic.wanted.iter().map(|w| w.name.as_str()).collect();
    for p in placed {
        if !wanted.contains(p.instance.as_str()) {
            out.actions.push(Action::Extra(p.clone()));
        }
    }
    out
}

/// The Tcl that carries out `plan` in Magic, on the loaded edit cell `top` (Magic's
/// working folder is the layout's folder). New instances go in a row to the right of
/// what is there. With `remove`, extra instances are deleted.
pub fn apply_script(top: &str, plan: &Plan, placed: &[Placed], remove: bool) -> String {
    // Where the row starts: right of everything placed, in internal units.
    let mut right = 0i64;
    for p in placed {
        if let (Some(t), Some(b)) = (p.transform, p.bbox) {
            right = right.max(t[2] + b[2].max(b[0]));
        }
    }
    let mut x = right + 200;
    let mut t = String::new();
    t += "units internal\n";
    for a in &plan.actions {
        match a {
            Action::Add(w) | Action::Update(w, _) => {
                let update = matches!(a, Action::Update(..));
                if update {
                    // A different device type or another cell: start over at its place.
                    let differs = match (&w.target, a) {
                        (Target::Pcell { library, cell, .. }, Action::Update(_, p)) => {
                            p.generator.as_ref() != Some(&(library.clone(), cell.clone()))
                        }
                        (Target::Cell { .. }, _) => true,
                        _ => false,
                    };
                    if differs {
                        t += &format!(
                            "select clear\nselect cell {}\nset cdept_box [box values]\ndelete\n\
                             box position [lindex $cdept_box 0] [lindex $cdept_box 1]\n",
                            word(&w.name)
                        );
                    }
                } else {
                    t += &format!("box position {x} 0\n");
                    x += 1500;
                }
                match &w.target {
                    Target::Pcell {
                        library,
                        cell,
                        params,
                    } => {
                        let mut args = String::new();
                        for (k, v) in params {
                            args += &format!(" {} {}", word(k), word(v));
                        }
                        t += &format!(
                            "magic::gencell {}::{} {}{args}\n",
                            library,
                            cell,
                            word(&w.name)
                        );
                    }
                    Target::Cell { cell, file, .. } => {
                        if let Some(dir) = file.parent() {
                            t += &format!("catch {{addpath {}}}\n", word(&dir.to_string_lossy()));
                        }
                        t += &format!("getcell {}\nidentify {}\n", word(cell), word(&w.name));
                    }
                }
                t += "select clear\n";
            }
            Action::Extra(p) if remove => {
                t += &format!("select clear\nselect cell {}\ndelete\n", word(&p.instance));
            }
            Action::Extra(_) => {}
        }
    }
    t += &format!("select clear\nload {}\n", word(top));
    t
}

/// The Tcl that saves the edit cell `top` and every modified cell it places (but no
/// leftover cells a generator made and nothing uses any more).
pub fn save_script(top: &str) -> String {
    format!(
        "set cdept_used [concat [list {top}] [cellname list children {top}]]\n\
         set cdept_write {{}}\n\
         foreach c [cellname list modified] {{\n\
           if {{[lsearch -exact $cdept_used $c] >= 0}} {{ lappend cdept_write $c }}\n\
         }}\n\
         if {{[llength $cdept_write]}} {{ writeall force {{*}}$cdept_write }}\n",
        top = word(top)
    )
}

/// Record the link in the layout's sidecar: the schematic, and what each instance
/// was made into.
pub fn record(project: &Project, file: &Path, schematic: &Schematic) -> Result<()> {
    super::refs::normalize(project, file)?;
    let stem = super::embed::cell_of(file);
    let mut meta = Meta::read_for(file)
        .ok()
        .flatten()
        .unwrap_or_else(|| Meta::new("", &stem, "layout", &stem));
    if meta.library.is_empty() {
        bail!("{} is not a view of the project", file.display());
    }
    meta.source = Some(schematic.source.clone());
    meta.instances = schematic
        .wanted
        .iter()
        .map(|w| {
            let (kind, library, cell, params) = match &w.target {
                Target::Pcell {
                    library,
                    cell,
                    params,
                } => ("gencell", library.clone(), cell.clone(), params.clone()),
                Target::Cell { library, cell, .. } => {
                    ("cell", library.clone(), cell.clone(), BTreeMap::new())
                }
            };
            (
                w.name.clone(),
                Linked {
                    symbol: w.symbol.clone(),
                    kind: kind.into(),
                    library,
                    cell,
                    params,
                },
            )
        })
        .collect();
    meta.version = meta.version.max(2);
    meta.write_for(file).wrap_err("cannot write the sidecar")
}

/// The schematic a Magic layout is linked with (its sidecar's `source`), else the
/// cell's own schematic.
pub fn source_of(
    project: &Project,
    library: &str,
    cell: &str,
    file: &Path,
) -> Result<(Source, PathBuf)> {
    if let Ok(Some(meta)) = Meta::read_for(file)
        && let Some(s) = meta.source
        && let Ok(found) = crate::link::schematic_view(project, &s.library, &s.cell, Some(&s.view))
    {
        return Ok(found);
    }
    crate::link::schematic_view(project, library, cell, None)
}

/// The Magic view of `library/cell` (made from the template when `create` and it has
/// none).
pub fn layout_file(project: &Project, library: &str, cell: &str, create: bool) -> Result<PathBuf> {
    if let Some((_, file)) = mag_view(project, library, cell) {
        return Ok(file);
    }
    if !create {
        bail!("{library}/{cell} has no Magic layout view");
    }
    let lib = project
        .library(library)
        .ok_or_else(|| eyre!("no library '{library}'"))?;
    lib.create_view_with(
        cell,
        crate::template::Template::MagicLayout,
        &project.template_setup(),
    )
}
