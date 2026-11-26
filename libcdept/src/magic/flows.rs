//! Batch operations on the project's Magic views that combine Magic with other parts
//! of CDePT (docs/MAGIC.md): layout from schematic, LVS against the xschem schematic
//! with netgen, GDS import into a library and GDS export.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use eyre::{Result, WrapErr, bail, eyre};

use super::link::{self, Action, Plan};
use super::tcl::word;
use super::{Magic, Source, parse, tcl};
use crate::batch::netgen;
use crate::{Library, LibraryLayout, Project};

/// The result of [`generate`].
#[derive(Debug, Clone)]
pub struct Generated {
    pub layout: PathBuf,
    /// What was planned (before it was done).
    pub plan: Plan,
    /// `check` only: nothing was changed.
    pub dry_run: bool,
    pub log: String,
}

/// Bring the Magic view of `library/cell` in line with its schematic (made first when
/// the cell has no Magic view): devices of the PDK's generators and instances of
/// project cells are added and updated, instances named as in the schematic. With
/// `remove`, layout instances that are not in the schematic are deleted. With
/// `dry_run` only the plan is made (`check-source`).
pub fn generate(
    magic: &Magic,
    project: &Project,
    library: &str,
    cell: &str,
    schematic_view: Option<&str>,
    remove: bool,
    dry_run: bool,
) -> Result<Generated> {
    let layout = link::layout_file(project, library, cell, !dry_run)?;
    let (source, sch_file) = match schematic_view {
        Some(v) => crate::link::schematic_view(project, library, cell, Some(v))?,
        None => link::source_of(project, library, cell, &layout)?,
    };
    let schematic = link::read_schematic(project, source, &sch_file)?;
    let placed = link::placed(project, &layout)?;
    let plan = link::plan(&schematic, &placed);
    if dry_run || plan.actions.is_empty() {
        if !dry_run {
            link::record(project, &layout, &schematic)?;
        }
        return Ok(Generated {
            layout,
            plan,
            dry_run,
            log: String::new(),
        });
    }
    if project.library(library).is_some_and(|l| l.read_only) {
        bail!("library '{library}' is read-only");
    }
    let top = super::embed::cell_of(&layout);
    let dir = layout.parent().unwrap_or(Path::new(".")).to_path_buf();
    let body = format!(
        "cd {}\nload {}\n{}{}puts \"{} saved\"\n",
        word(&dir.to_string_lossy()),
        word(&top),
        link::apply_script(&top, &plan, &placed, remove),
        link::save_script(&top),
        tcl::MARK
    );
    let script = guard(&body);
    let log = magic.run(&script, &dir)?;
    remove_unused_devices(&dir, &layout);
    link::record(project, &layout, &schematic)?;
    Ok(Generated {
        layout,
        plan,
        dry_run,
        log,
    })
}

/// Delete the device cells (files with a `gencell` property) in `dir` that no `.mag`
/// file of `dir` places any more: the old versions a regenerated device leaves.
/// `layout` itself is never touched.
fn remove_unused_devices(dir: &Path, layout: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mags: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "mag"))
        .collect();
    let mut used = std::collections::BTreeSet::new();
    let mut devices = Vec::new();
    for file in &mags {
        let Ok(m) = super::mag::read(file) else {
            continue;
        };
        for u in &m.uses {
            if u.path.is_empty() || u.path == "." {
                used.insert(u.cell.clone());
            }
        }
        if m.properties.contains_key("gencell") && file != layout {
            devices.push(file.clone());
        }
    }
    for file in devices {
        let name = super::embed::cell_of(&file);
        if !used.contains(&name) {
            let _ = std::fs::remove_file(&file);
        }
    }
}

/// Same as [`tcl`]'s guard: errors are reported and end the run.
fn guard(body: &str) -> String {
    format!(
        "if {{[catch {{\n{body}\n}} cdept_err]}} {{\n\
         puts \"{m} error $cdept_err\"\n\
         quit -noprompt\n\
         }}\n\
         puts \"{m} done\"\n\
         quit -noprompt\n",
        m = tcl::MARK
    )
}

/// Write `src` with everything it places as one GDSII file `out`.
pub fn export_gds(magic: &Magic, src: &Source, out: &Path) -> Result<PathBuf> {
    let out = std::path::absolute(out)?;
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let _ = std::fs::remove_file(&out);
    let dir = out.parent().unwrap_or(Path::new(".")).to_path_buf();
    let cell = src.cell();
    let mut body = String::new();
    match src {
        Source::Mag { file } => {
            if let Some(d) = file.parent() {
                body += &format!("addpath {}\n", word(&d.to_string_lossy()));
            }
            body += &format!("load {}\n", word(&cell));
        }
        Source::Gds { .. } => {
            bail!("the layout is GDS already");
        }
    }
    body += &format!(
        "select top cell\ngds write {}\n",
        word(&out.to_string_lossy())
    );
    let log = magic.run(&guard(&body), &dir)?;
    if !out.is_file() {
        bail!(
            "Magic wrote no {}\n{}",
            out.display(),
            parse::tail(&log, 25)
        );
    }
    Ok(out)
}

/// The result of [`import_gds`].
#[derive(Debug, Clone, Default)]
pub struct Imported {
    /// Cells written as views of the library.
    pub cells: Vec<String>,
    /// Cells of the file the library has already (left alone).
    pub skipped: Vec<String>,
}

/// Read every cell of the GDS file `gds` and make each one a Magic view
/// (`<cell>/layout/<cell>.mag` in a `cells` library, `<cell>.mag` in a flat one), with
/// the cells they place found through relative `use` paths and the sidecars written.
/// Cells the library already has are not replaced unless `replace`; `only`: the cells
/// to keep (with what they place), all when empty.
pub fn import_gds(
    magic: &Magic,
    project: &Project,
    gds: &Path,
    library: &Library,
    only: &[String],
    replace: bool,
) -> Result<Imported> {
    library.writable()?;
    let work = WorkDir::new(&project.root)?;
    let read = magic.import_gds(gds, work.path())?;
    // Which cells: the asked ones and what they place (from the written files).
    let mut keep: Vec<String> = if only.is_empty() {
        read.cells.clone()
    } else {
        let mut keep: Vec<String> = Vec::new();
        let mut todo: Vec<String> = only.to_vec();
        while let Some(c) = todo.pop() {
            if keep.contains(&c) {
                continue;
            }
            let file = work.path().join(format!("{c}.mag"));
            if !file.is_file() {
                bail!("{} has no cell '{c}'", gds.display());
            }
            for child in super::mag::read(&file)?.children() {
                todo.push(child.to_string());
            }
            keep.push(c);
        }
        keep
    };
    keep.retain(|c| work.path().join(format!("{c}.mag")).is_file());
    let mut out = Imported::default();
    let target_of = |c: &str| -> PathBuf {
        match library.layout {
            LibraryLayout::Cells => library.path.join(c).join("layout").join(format!("{c}.mag")),
            _ => library.path.join(format!("{c}.mag")),
        }
    };
    let mut moved: BTreeMap<String, PathBuf> = BTreeMap::new();
    for c in &keep {
        crate::library::validate_name("cell", c)?;
        let target = target_of(c);
        if (target.exists() || library.cell(c).is_some()) && !replace {
            out.skipped.push(c.clone());
            continue;
        }
        if let Some(d) = target.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::copy(work.path().join(format!("{c}.mag")), &target)
            .wrap_err_with(|| format!("cannot write {}", target.display()))?;
        moved.insert(c.clone(), target);
        out.cells.push(c.clone());
    }
    // `use` lines between the imported cells: relative paths to their new folders
    // (a cell left out because the library has it is found there by name).
    for file in moved.values() {
        let dir = file
            .parent()
            .unwrap_or(Path::new("."))
            .canonicalize()
            .unwrap_or_default();
        super::mag::rewrite_file(file, |u| {
            let target = moved
                .get(&u.cell)
                .cloned()
                .unwrap_or_else(|| target_of(&u.cell));
            let tdir = target.parent()?.canonicalize().ok()?;
            Some(super::mag::UseEdit {
                cell: None,
                path: Some(
                    super::mag::relative_dir(&dir, &tdir)
                        .to_string_lossy()
                        .into_owned(),
                ),
            })
        })?;
    }
    let project = Project::load(&project.root)?;
    for (c, file) in &moved {
        if let Err(e) = super::refs::normalize(&project, file) {
            tracing::warn!("{c}: {e:#}");
        }
    }
    Ok(out)
}

/// A scratch folder under `<project>/.cdept/`, removed when dropped.
struct WorkDir(PathBuf);

impl WorkDir {
    fn new(root: &Path) -> Result<WorkDir> {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = root
            .join(".cdept")
            .join(format!("import-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).wrap_err_with(|| format!("cannot make {}", dir.display()))?;
        Ok(WorkDir(dir))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The result of [`lvs`].
#[derive(Debug, Clone)]
pub struct LvsResult {
    pub layout_netlist: PathBuf,
    pub schematic_netlist: PathBuf,
    pub report: netgen::Report,
    pub notes: Vec<String>,
}

/// LVS of the Magic view of `library/cell` against its schematic (the sidecar's
/// `source`, else the cell's own): Magic extracts (`ext2spice lvs`), xschem netlists
/// the schematic for LVS, netgen compares with the PDK's set-up (`[netgen] setup` of
/// the snapshot). Files go to `<project>/magic/<library>/<cell>/`.
pub fn lvs(magic: &Magic, project: &Project, library: &str, cell: &str) -> Result<LvsResult> {
    let layout = link::layout_file(project, library, cell, false)?;
    let (source, sch_file) = link::source_of(project, library, cell, &layout)?;
    let dir = super::out_dir(project, library, cell);
    std::fs::create_dir_all(&dir)?;
    let src = Source::Mag {
        file: layout.clone(),
    };
    let extracted = magic.extract(&src, &dir, &super::ExtractOptions::default())?;
    let sch_out = dir.join(format!("{}.schematic.spice", source.cell));
    crate::klayout::verify::schematic_netlist(project, &sch_file, &sch_out)?;
    let program = netgen::ADAPTER.require()?;
    let lvs = netgen::Lvs {
        layout: extracted.spice.clone(),
        layout_cell: extracted.cell.clone(),
        schematic: sch_out.clone(),
        schematic_cell: source.cell.clone(),
        setup: netgen::project_setup(project),
    };
    let mut notes = Vec::new();
    if lvs.setup.is_none() {
        notes.push("the technology snapshot names no netgen set-up file: devices are compared without the PDK's rules".into());
    }
    let report =
        netgen::run(&program, &lvs, &dir, &project.tool_env()).map_err(|e| eyre!("{e:#}"))?;
    Ok(LvsResult {
        layout_netlist: extracted.spice,
        schematic_netlist: sch_out,
        report,
        notes,
    })
}

/// The Magic view of `library/cell` (an error when it has none).
pub fn link_layout(project: &Project, library: &str, cell: &str) -> Result<PathBuf> {
    link::layout_file(project, library, cell, false)
}

/// A short line for each action of a plan (`+ M1 sky130/nfet`, `~ M2`, `- X9`).
pub fn plan_summary(plan: &Plan) -> Vec<String> {
    plan.actions
        .iter()
        .map(|a| match a {
            Action::Add(w) => format!("+ {} {}", w.name, w.target.label()),
            Action::Update(w, _) => format!("~ {} {}", w.name, w.target.label()),
            Action::Extra(p) => format!("- {} {}", p.instance, p.cell),
        })
        .collect()
}
