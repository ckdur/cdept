//! Simulation (Phase 4, first slice): a testbench view, the deck CDePT wraps around
//! the schematic's netlist, a batch ngspice run and what it left behind.
//!
//! A testbench is a view of a cell with a `setup.toml` in it ([`setup::Setup`]). It
//! names the schematic to netlist (the xschem testbench, which holds the DUT, sources
//! and its own code blocks), the model set and corners from the technology snapshot,
//! temperatures, design variables, analyses, measurements and the specs they must
//! meet. [`deck::build`] makes one SPICE deck per run point (corner × temperature),
//! [`run::run`] starts the simulator on it, and [`spec::check`] compares the
//! measurements with the specs. Format and design: docs/SIMULATION.md.

pub mod deck;
pub mod run;
pub mod setup;
pub mod spec;

pub use deck::{RunPoint, build as build_deck};
pub use run::{Outcome, Simulator, run, run_in_job};
pub use setup::{Analysis, Setup, Spec};

use std::path::{Path, PathBuf};

use eyre::{Result, WrapErr, eyre};

use crate::Project;

/// Where the runs of a testbench go: `<project>/simulations/<library>/<cell>/<view>`.
pub fn runs_dir(project: &Project, library: &str, cell: &str, view: &str) -> PathBuf {
    project.root.join("simulations").join(library).join(cell).join(view)
}

/// A testbench found in a project.
pub struct Testbench {
    pub library: String,
    pub cell: String,
    pub view: String,
    pub setup: Setup,
    /// The xschem schematic that is netlisted.
    pub schematic: PathBuf,
}

/// Open the testbench view `library/cell/view`.
pub fn open(project: &Project, library: &str, cell: &str, view: &str) -> Result<Testbench> {
    let entry = project
        .library(library)
        .and_then(|l| l.cell(cell))
        .and_then(|c| c.entry(view))
        .ok_or_else(|| eyre!("no view '{view}' of {library}/{cell}"))?;
    let file = entry.path.join(setup::SETUP_FILE);
    if !file.is_file() {
        return Err(eyre!("{library}/{cell}/{view} is not a testbench (no {})", setup::SETUP_FILE));
    }
    let setup = Setup::load(&file)?;
    let (sch_lib, sch_cell) = match setup.schematic.split_once('/') {
        Some((l, c)) => (l.to_string(), c.to_string()),
        None if setup.schematic.is_empty() => (library.to_string(), cell.to_string()),
        None => return Err(eyre!("schematic '{}' is not library/cell", setup.schematic)),
    };
    let view_name = (!setup.schematic_view.is_empty()).then_some(setup.schematic_view.as_str());
    let schematic = crate::klayout::verify::schematic_file(project, &sch_lib, &sch_cell, view_name)?;
    Ok(Testbench { library: library.into(), cell: cell.into(), view: view.into(), setup, schematic })
}

/// The result of one run point.
pub struct PointResult {
    pub point: RunPoint,
    pub outcome: Result<Outcome>,
    pub specs: Vec<spec::Row>,
}

/// How [`run_testbench_with`] runs the points.
#[derive(Clone, Default)]
pub struct RunOptions {
    /// Simulators running at once; 0 means the smaller of the number of points and the cores.
    pub jobs: usize,
    /// Called on every job event (a point starts, finishes, fails), from worker threads.
    pub on_event: Option<crate::jobs::Listener>,
}

/// Netlist the testbench's schematic (once) and run every point (or only `only`,
/// corner names), in parallel as far as there are cores ([`RunOptions::jobs`]); the
/// results are in point order. Files go to [`runs_dir`]/`<point>/` (`job.log` is the
/// job's own log).
pub fn run_testbench(
    project: &Project,
    tb: &Testbench,
    simulator: &Simulator,
    only: &[String],
) -> Result<Vec<PointResult>> {
    run_testbench_with(project, tb, simulator, only, &RunOptions::default())
}

/// [`run_testbench`] with options.
pub fn run_testbench_with(
    project: &Project,
    tb: &Testbench,
    simulator: &Simulator,
    only: &[String],
    options: &RunOptions,
) -> Result<Vec<PointResult>> {
    use crate::jobs::{JobSpec, Queue};
    use std::sync::{Arc, Mutex};

    let tech = project
        .tech
        .as_ref()
        .ok_or_else(|| eyre!("the project has no technology snapshot (cdept tech scan)"))?;
    let models = &tech.models;
    let set = if !tb.setup.models.is_empty() {
        tb.setup.models.clone()
    } else {
        models.default.clone().ok_or_else(|| eyre!("the technology names no default model set"))?
    };
    let default_corner = models
        .corner
        .clone()
        .ok_or_else(|| eyre!("the technology names no default corner"))?;
    let mut points = deck::points(&tb.setup, &default_corner);
    if !only.is_empty() {
        points = vec![];
        let all = deck::points(&Setup { corners: only.to_vec(), ..tb.setup.clone() }, &default_corner);
        points.extend(all);
    }
    let base = runs_dir(project, &tb.library, &tb.cell, &tb.view);
    let netlist_file = base.join("netlist.spice");
    netlist(project, &tb.schematic, &netlist_file)?;
    let netlist = std::fs::read_to_string(&netlist_file)
        .wrap_err_with(|| format!("cannot read {}", netlist_file.display()))?;
    let (_, model_set) = models.set(&set).ok_or_else(|| eyre!("the technology has no models '{set}'"))?;
    let init = deck::spiceinit(model_set, |n| project.var(n)).map_err(|e| eyre!(e))?;
    let title = format!("{}/{}/{}", tb.library, tb.cell, tb.view);
    let env = project.tool_env();
    let workers = if options.jobs > 0 { options.jobs } else { Queue::default_workers().min(points.len()) };
    let queue = Queue::new(workers);
    if let Some(f) = &options.on_event {
        let f = f.clone();
        queue.on_event(move |e| f(e));
    }
    type Slot = Arc<Mutex<Option<Result<Outcome>>>>;
    let mut slots: Vec<Slot> = vec![];
    for point in &points {
        let lines = models.lines(&set, &point.corner, |n| project.var(n)).map_err(|e| eyre!(e))?;
        let text = deck::build(&tb.setup, point, &title, &lines, &netlist);
        let dir = base.join(point.name());
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join(".spiceinit"), &init)?;
        let slot: Slot = Arc::default();
        slots.push(slot.clone());
        let (simulator, env, log) = (simulator.clone(), env.clone(), dir.join("job.log"));
        let spec = JobSpec::func(point.name(), move |ctx| {
            let outcome = run::run_in_job(&simulator, &text, &dir, &env, ctx);
            let verdict = match &outcome {
                Ok(_) => Ok(()),
                Err(e) => Err(eyre!("{e:#}")),
            };
            *slot.lock().unwrap() = Some(outcome);
            verdict
        })
        .log(log);
        queue.submit(spec);
    }
    queue.wait_all();
    let results = points
        .into_iter()
        .zip(slots)
        .map(|(point, slot)| {
            let outcome = slot.lock().unwrap().take().unwrap_or_else(|| Err(eyre!("cancelled")));
            let specs = match &outcome {
                Ok(o) => spec::check(&tb.setup.specs, &o.measures),
                Err(_) => vec![],
            };
            PointResult { point, outcome, specs }
        })
        .collect();
    Ok(results)
}

/// Netlist the schematic `file` for simulation (SPICE, not the LVS flavour).
pub fn netlist(project: &Project, file: &Path, out: &Path) -> Result<()> {
    let mut engine = crate::xschem::Engine::for_project(project)?;
    let outcome = (|| {
        engine.load(file)?;
        engine.netlist(crate::xschem::NetlistFormat::Spice, out)
    })();
    let _ = engine.shutdown();
    outcome
}
