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

pub mod caps;
pub mod deck;
pub mod mc;
pub mod remote;
pub mod results;
pub mod run;
pub mod setup;
pub mod spec;
pub mod starter;
pub mod stats;
pub mod suite;
pub mod template;
pub mod xyce_mm;

pub use deck::{RunPoint, build as build_deck};
pub use run::{Outcome, Simulator, run, run_in_job, run_in_job_limited};
pub use setup::{Analysis, Setup, Spec};

use std::path::{Path, PathBuf};

use eyre::{Result, WrapErr, bail, eyre};

use crate::Project;

/// Where the runs of a testbench go: `<project>/simulations/<library>/<cell>/<view>`.
pub fn runs_dir(project: &Project, library: &str, cell: &str, view: &str) -> PathBuf {
    project
        .root
        .join("simulations")
        .join(library)
        .join(cell)
        .join(view)
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
        return Err(eyre!(
            "{library}/{cell}/{view} is not a testbench (no {})",
            setup::SETUP_FILE
        ));
    }
    let setup = Setup::load_with(&file, &template::TemplateSet::for_project(project))?;
    let (sch_lib, sch_cell) = match setup.schematic.split_once('/') {
        Some((l, c)) => (l.to_string(), c.to_string()),
        None if setup.schematic.is_empty() => (library.to_string(), cell.to_string()),
        None => return Err(eyre!("schematic '{}' is not library/cell", setup.schematic)),
    };
    let view_name = (!setup.schematic_view.is_empty()).then_some(setup.schematic_view.as_str());
    let schematic = crate::batch::qucs::schematic_for(
        project,
        &sch_lib,
        &sch_cell,
        view_name,
        &setup.schematic_kind,
    )?;
    Ok(Testbench {
        library: library.into(),
        cell: cell.into(),
        view: view.into(),
        setup,
        schematic,
    })
}

/// The result of one run point.
pub struct PointResult {
    pub point: RunPoint,
    pub outcome: Result<Outcome>,
    pub specs: Vec<spec::Row>,
    /// When the run started (RFC 3339) and how long it took.
    pub started: String,
    pub duration_ms: u64,
    /// The run's `result.json` ([`results::RunRecord`]) when it was written.
    pub record: Option<PathBuf>,
    /// The same as a record (point, hashes, measures, verdicts, paths), written or not.
    pub run: results::RunRecord,
}

/// How [`run_testbench_with`] runs the points.
#[derive(Clone, Default)]
pub struct RunOptions {
    /// Simulators running at once; 0 means the smaller of the number of points and the cores.
    pub jobs: usize,
    /// Called on every job event (a point starts, finishes, fails), from worker threads.
    pub on_event: Option<crate::jobs::Listener>,
    /// Monte Carlo runs: run this many seeded points of the statistical corner. Turns
    /// Monte Carlo on for a setup without `[montecarlo]`, else overrides its `runs`.
    pub mc_runs: Option<u32>,
    /// The base seed of the Monte Carlo runs (default: the setup's, else 1).
    pub mc_seed: Option<u64>,
    /// Run the corner × temperature × sweep grid although the setup has `[montecarlo]`.
    pub no_mc: bool,
    /// What the Monte Carlo runs draw (default: the setup's, else process variation).
    pub mc_mode: Option<setup::McMode>,
    /// Do not write `result.json` files or add to `results.db`.
    pub no_store: bool,
    /// Put the points on this queue (a UI's, shown live) instead of a private one. Then
    /// `jobs` and `on_event` do nothing (listen on the queue itself) and only this
    /// run's jobs are waited for.
    pub queue: Option<std::sync::Arc<crate::jobs::Queue>>,
    /// Set to stop the run: checked after the netlist, before each point is queued.
    pub cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    /// ngspice as a process or in-process (`--backend`); default: the setup's
    /// `simulator_backend`, else the process.
    pub backend: Option<run::Backend>,
    /// Send the points to this remote of `[jobs.remote.*]` (`--remote`); default: the
    /// setup's `remote`, else the project's `[jobs] default`, else run here.
    pub remote: Option<String>,
    /// The time limit of each run point (`--timeout`); `Some(0)` is no limit. Default:
    /// the setup's `timeout`, else the project's `[sim] timeout`, else 30 minutes.
    pub timeout: Option<std::time::Duration>,
}

impl RunOptions {
    /// The limit of a point: `timeout`, else the setup's, else the project's, else
    /// [`run::DEFAULT_TIMEOUT`]; `None` for no limit.
    pub fn time_limit(
        &self,
        project: &Project,
        setup: &Setup,
    ) -> Result<Option<std::time::Duration>> {
        let limit = match self.timeout {
            Some(t) => t,
            None => match setup.time_limit()? {
                Some(t) => t,
                None => project.sim.timeout.unwrap_or(run::DEFAULT_TIMEOUT),
            },
        };
        Ok((!limit.is_zero()).then_some(limit))
    }
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

/// The corner sets (and switches) a Monte Carlo setup draws from in the model set `set`
/// of `models`; the error says what is missing and which corner sets there are. The
/// Simulation tab's form asks it before a run.
pub fn monte_carlo_plan(
    project: &Project,
    models: &crate::tech::models::ModelsTech,
    set: &str,
    simulator: &str,
    mc: &setup::MonteCarlo,
) -> Result<mc::Plan> {
    let (_, set_models) = models
        .set(set)
        .ok_or_else(|| eyre!("the technology has no models '{set}'"))?;
    let mentions = |corner: &str, word: &str| {
        set_models.corner(corner).is_some_and(|libs| {
            libs.iter().any(|l| {
                l.file
                    .resolve(|n| project.var(n))
                    .ok()
                    .and_then(|p| std::fs::read(p).ok())
                    .is_some_and(|t| {
                        String::from_utf8_lossy(&t)
                            .to_ascii_lowercase()
                            .contains(word)
                    })
            })
        })
    };
    let known = |c: &str| set_models.corner(c).is_some();
    mc::plan(mc, set, simulator == "xyce", known, mentions)
        .map_err(|e| eyre!("{e} (there are: {})", set_models.corner_names().join(", ")))
}

/// The PDK's Xyce model folder and the folder with the same files plus the transistor
/// mismatch ([`xyce_mm::overlay`], in `.cdept/xyce-mm/<set>` of the project).
fn xyce_mismatch_models(
    project: &Project,
    models: &crate::tech::models::ModelsTech,
    xyce: &crate::tech::models::ModelSet,
) -> Result<(PathBuf, PathBuf)> {
    let var = |n: &str| project.var(n);
    let from = xyce.dir.resolve(var).map_err(|e| eyre!(e))?;
    let (_, ngspice) = models.set("ngspice").ok_or_else(|| {
        eyre!("Xyce mismatch takes its sigmas from the ngspice models, which the technology lacks")
    })?;
    let sigmas = ngspice.dir.resolve(var).map_err(|e| eyre!(e))?;
    let pdk = project
        .tech
        .as_ref()
        .map_or("pdk".to_string(), |t| t.pdk.name.clone());
    let to = project.root.join(".cdept").join("xyce-mm").join(pdk);
    let to = xyce_mm::overlay(&from, &sigmas, &to)?;
    Ok((from, to))
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
    // VACASK: the technology's ngspice models converted for it (made on first use).
    let vacask_ready = if tb.setup.simulator == "vacask" {
        // Refused before anything is made when VACASK cannot run here (docs/VACASK.md).
        crate::vacask::availability(project).check()?;
        Some(crate::vacask::models::ready_watched(
            project,
            options.cancel.clone(),
        )?)
    } else {
        None
    };
    let tech = vacask_ready.as_ref().map_or(tech, |r| &r.tech);
    let models = &tech.models;
    let set = if !tb.setup.models.is_empty() {
        tb.setup.models.clone()
    } else {
        models
            .default
            .clone()
            .ok_or_else(|| eyre!("the technology names no default model set"))?
    };
    let remote = project
        .jobs
        .pick(options.remote.as_deref(), &tb.setup.remote)?
        .cloned();
    if let Some(r) = &remote
        && (options.backend == Some(run::Backend::Inproc)
            || (options.backend.is_none() && tb.setup.simulator_backend == "inproc"))
    {
        bail!(
            "the in-process backend cannot run on the remote '{}': use simulator_backend = \"process\" (or --backend process)",
            r.name
        );
    }
    let backend = run::Backend::choose(options.backend, &tb.setup)?;
    let time_limit = options.time_limit(project, &tb.setup)?;
    let default_corner = models
        .corner
        .clone()
        .ok_or_else(|| eyre!("the technology names no default corner"))?;
    let mc = match (&tb.setup.montecarlo, options.mc_runs) {
        _ if options.no_mc => None,
        (None, None) => None,
        (setup_mc, runs) => {
            let base = setup_mc.clone().unwrap_or_default();
            Some(setup::MonteCarlo {
                runs: runs.unwrap_or(base.runs),
                seed: options.mc_seed.unwrap_or(base.seed),
                mode: options.mc_mode.unwrap_or(base.mode),
                ..base
            })
        }
    };
    // The corner sets (and switches) the Monte Carlo mode draws from.
    let mc_plan = match &mc {
        Some(mc) => Some(monte_carlo_plan(
            project,
            models,
            &set,
            &tb.setup.simulator,
            mc,
        )?),
        None => None,
    };
    let mut points = if let (Some(mc), Some(plan)) = (&mc, &mc_plan) {
        deck::mc_points(&tb.setup, &plan.label, mc.runs, mc.seed)
    } else {
        deck::points(&tb.setup, &default_corner)
    };
    if !only.is_empty() && mc.is_none() {
        points = vec![];
        let all = deck::points(
            &Setup {
                corners: only.to_vec(),
                ..tb.setup.clone()
            },
            &default_corner,
        );
        points.extend(all);
    }
    let base = runs_dir(project, &tb.library, &tb.cell, &tb.view);
    let netlist_file = base.join("netlist.spice");
    netlist_for(project, &tb.schematic, &netlist_file, &tb.setup.simulator)?;
    let netlist = std::fs::read_to_string(&netlist_file)
        .wrap_err_with(|| format!("cannot read {}", netlist_file.display()))?;
    template::check_netlist(&tb.setup, &netlist)?;
    let (_, model_set) = models
        .set(&set)
        .ok_or_else(|| eyre!("the technology has no models '{set}'"))?;
    let title = format!("{}/{}/{}", tb.library, tb.cell, tb.view);
    // Xyce mismatch: the PDK's Xyce models have none, CDePT's overlay folder has it.
    let xyce_overlay = match &mc_plan {
        Some(plan) if plan.mismatch_models => {
            Some(xyce_mismatch_models(project, models, model_set)?)
        }
        _ => None,
    };
    let env = project.tool_env();
    let workers = if options.jobs > 0 {
        options.jobs
    } else if let Some(r) = &remote {
        // The remote's own limit, not this machine's cores.
        r.max_jobs.min(points.len().max(1))
    } else {
        Queue::default_workers().min(points.len())
    };
    // One libngspice runs one point at a time: more workers would only wait for it.
    let workers = if backend == run::Backend::Inproc {
        1
    } else {
        workers
    };
    let own;
    let queue: &Queue = match &options.queue {
        Some(q) => q,
        None => {
            own = Queue::new(workers);
            if let Some(f) = &options.on_event {
                let f = f.clone();
                own.on_event(move |e| f(e));
            }
            &own
        }
    };
    let cancelled = || {
        options
            .cancel
            .as_ref()
            .is_some_and(|c| c.load(std::sync::atomic::Ordering::Relaxed))
    };
    let mut ids = vec![];
    type Slot = Arc<Mutex<Option<(Result<Outcome>, String, u64)>>>;
    let mut slots: Vec<Slot> = vec![];
    let mut decks: Vec<String> = vec![];
    for point in &points {
        if cancelled() {
            break;
        }
        let dir = base.join(point.name());
        std::fs::create_dir_all(&dir)?;
        let text = if let Some(ready) = &vacask_ready {
            let d = crate::vacask::build_point(
                project, ready, &tb.setup, &set, point, &title, &netlist,
            )?;
            for w in &d.warnings {
                tracing::warn!("{}: {w}", point.name());
            }
            d.text
        } else {
            let init = deck::spiceinit(model_set, |n| project.var(n)).map_err(|e| eyre!(e))?;
            let lines = match &mc_plan {
                Some(plan) => {
                    let mut all = vec![];
                    for c in &plan.corners {
                        all.extend(
                            models
                                .lines(&set, c, |n| project.var(n))
                                .map_err(|e| eyre!(e))?,
                        );
                    }
                    all.extend(plan.extra.iter().cloned());
                    match &xyce_overlay {
                        Some((from, to)) => xyce_mm::redirect(&all, from, to),
                        None => all,
                    }
                }
                None => models
                    .lines(&set, &point.corner, |n| project.var(n))
                    .map_err(|e| eyre!(e))?,
            };
            std::fs::write(dir.join(".spiceinit"), &init)?;
            deck::build(&tb.setup, point, &title, &lines, &netlist)
        };
        decks.push(text.clone());
        let slot: Slot = Arc::default();
        slots.push(slot.clone());
        let mut env = env.clone();
        if let Some(args) = deck::xyce_args(&tb.setup, point, model_set, |n| project.var(n))
            .map_err(|e| eyre!(e))?
        {
            env.push((run::XYCE_ARGS_ENV.into(), args.into()));
        }
        let (simulator, log) = (simulator.clone(), dir.join("job.log"));
        let remote = remote.clone();
        let spec = JobSpec::func(point.name(), move |ctx| {
            let started = chrono::Local::now().to_rfc3339();
            let clock = std::time::Instant::now();
            let outcome = match backend {
                _ if remote.is_some() => remote::run_in_job(
                    remote.as_ref().unwrap(),
                    &simulator,
                    &text,
                    &dir,
                    &env,
                    ctx,
                    time_limit,
                ),
                run::Backend::Process => {
                    run::run_in_job_limited(&simulator, &text, &dir, &env, ctx, time_limit)
                }
                run::Backend::Inproc => {
                    crate::ngspice::backend::run_in_job_limited(&text, &dir, &env, ctx, time_limit)
                }
            };
            let verdict = match &outcome {
                Ok(_) => Ok(()),
                Err(e) => Err(eyre!("{e:#}")),
            };
            *slot.lock().unwrap() = Some((outcome, started, clock.elapsed().as_millis() as u64));
            verdict
        })
        .log(log);
        ids.push(queue.submit(spec));
    }
    for id in ids {
        queue.wait(id);
    }
    let mut store = if options.no_store {
        None
    } else {
        results::Store::open(&project.root)
            .map_err(|e| tracing::warn!("results database: {e:#}"))
            .ok()
    };
    let mut results = vec![];
    for ((point, slot), deck_text) in points.into_iter().zip(slots).zip(decks) {
        let (outcome, started, duration_ms) = slot.lock().unwrap().take().unwrap_or_else(|| {
            (
                Err(eyre!("cancelled")),
                chrono::Local::now().to_rfc3339(),
                0,
            )
        });
        let (outcome, calc_errors) = crate::calc::sim::apply(&tb.setup, &point, outcome);
        let specs = match &outcome {
            Ok(o) => spec::check(&tb.setup.specs, &o.measures),
            Err(_) => vec![],
        };
        let mut rec = results::RunRecord::new(
            (&tb.library, &tb.cell, &tb.view),
            &tb.setup,
            &deck_text,
            &point,
            &base.join(point.name()),
            &outcome,
            &specs,
            started.clone(),
            duration_ms,
        );
        crate::calc::sim::mark(&mut rec, &tb.setup, calc_errors);
        let mut record = None;
        if !options.no_store {
            match rec.write() {
                Ok(f) => record = Some(f),
                Err(e) => tracing::warn!("{}: {e:#}", point.name()),
            }
            if let Some(db) = &mut store
                && let Err(e) = db.insert(&rec)
            {
                tracing::warn!("{}: results database: {e:#}", point.name());
            }
        }
        results.push(PointResult {
            point,
            outcome,
            specs,
            started,
            duration_ms,
            record,
            run: rec,
        });
    }
    Ok(results)
}

/// [`netlist`] for the testbench's simulator: a Qucs-S schematic goes through
/// `qucs-s -n` (its own analysis dropped), anything else through xschem.
pub fn netlist_for(project: &Project, file: &Path, out: &Path, simulator: &str) -> Result<()> {
    if crate::batch::qucs::is_qucs_schematic(file) {
        crate::batch::qucs::netlist_for_sim(project, file, out, simulator)
    } else {
        netlist(project, file, out)
    }
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
