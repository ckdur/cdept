//! `cdept sim …`: testbench views, decks and ngspice runs (docs/SIMULATION.md).

use std::sync::Arc;

use clap::Subcommand;
use libcdept::jobs::{JobState, Listener};
use eyre::{Result, bail, eyre};
use libcdept::Project;
use libcdept::sim::results::{self, RunRecord};
use libcdept::sim;

#[derive(Subcommand)]
pub enum SimCommand {
    /// Netlist a testbench, run every corner and temperature, print the spec table.
    ///
    /// The testbench is a view with a setup.toml. ngspice is `$CDEPT_NGSPICE` or the
    /// one on PATH. Files go to <project>/simulations/<library>/<cell>/<view>/<run>/.
    Run {
        library: String,
        cell: String,
        /// The testbench view. Default: the only one of the cell.
        #[arg(long)]
        view: Option<String>,
        /// Run only these corners (repeat), instead of the setup's.
        #[arg(long = "corner")]
        corners: Vec<String>,
        /// Simulators running at once. Default: the smaller of the points and the cores.
        #[arg(long, short = 'j')]
        jobs: Option<usize>,
        /// Monte Carlo: this many seeded runs of the statistical corner (overrides the
        /// setup's [montecarlo] runs, or turns Monte Carlo on).
        #[arg(long)]
        runs: Option<u32>,
        /// What the Monte Carlo runs draw: process, mismatch or both (default: the setup's).
        #[arg(long)]
        mode: Option<sim::setup::McMode>,
        /// Base seed of the Monte Carlo runs.
        #[arg(long)]
        seed: Option<u64>,
        /// Run the corner/temperature/sweep grid although the setup has [montecarlo].
        #[arg(long)]
        no_mc: bool,
        /// Do not write result.json / results.db.
        #[arg(long)]
        no_store: bool,
        /// ngspice as a process per point (`ngspice -b`) or in-process through libngspice
        /// ($CDEPT_LIBNGSPICE). Default: the setup's simulator_backend, else process.
        #[arg(long, value_parser = ["inproc", "process"])]
        backend: Option<String>,
        /// Run with this simulator instead of the setup's (`simulator` of setup.toml).
        #[arg(long, value_parser = ["ngspice", "xyce", "vacask"])]
        simulator: Option<String>,
    },
    /// Show the stored results of a testbench: the latest run of every point and, over
    /// them, pass/fail counts and min/max/mean/std of every measurement.
    Results {
        library: String,
        cell: String,
        #[arg(long)]
        view: Option<String>,
        /// Every stored run, not only the latest of each point.
        #[arg(long)]
        all: bool,
        /// Rebuild results.db from the result.json files first.
        #[arg(long)]
        reindex: bool,
        /// Statistics of the latest runs: percentiles (p5/p50/p95), text histograms and
        /// the yield per spec (measured, and estimated from mean and sigma).
        #[arg(long)]
        stats: bool,
        /// Histogram bins for --stats.
        #[arg(long, default_value_t = 10)]
        bins: usize,
        /// Sigmas of the mean +- k sigma yield estimate for --stats.
        #[arg(long, default_value_t = 3.0)]
        sigma: f64,
    },
    /// List the test templates (built-in, the technology's and the project's), or show one.
    ///
    /// A `setup.toml` picks a template with `template = "ac_response"` and
    /// `[template_args]` (docs/TESTS.md).
    Templates {
        #[command(subcommand)]
        action: Option<TemplatesCommand>,
    },
    /// Create a testbench view from a template: a setup.toml, and with --dut a starter
    /// schematic that instantiates the DUT with the template's sources.
    New {
        library: String,
        cell: String,
        view: String,
        #[arg(long)]
        template: String,
        /// A template parameter, `name=value` (repeat).
        #[arg(long = "arg", value_name = "NAME=VALUE")]
        args: Vec<String>,
        /// The DUT, `library/cell` (needs its xschem symbol): write the cell's schematic.
        #[arg(long)]
        dut: Option<String>,
    },
    /// Run a suite (a view with a suite.toml): its testbenches together, one table.
    ///
    /// Exits non-zero when a test cannot run, a run fails or a spec fails or is missing.
    Suite {
        library: String,
        cell: String,
        /// The suite view. Default: the only one of the cell.
        #[arg(long)]
        view: Option<String>,
        /// Simulators running at once. Default: the cores.
        #[arg(long, short = 'j')]
        jobs: Option<usize>,
        /// A column for every measurement, not only the ones with a spec.
        #[arg(long)]
        all_columns: bool,
        /// Do not write result.json / results.db / suite.json.
        #[arg(long)]
        no_store: bool,
        /// Print the report as JSON.
        #[arg(long)]
        json: bool,
        /// Run every test with this simulator instead of the tests' (and the suite's
        /// `simulator`).
        #[arg(long, value_parser = ["ngspice", "xyce", "vacask"])]
        simulator: Option<String>,
    },
    /// Print the deck of the first run without starting the simulator.
    Deck {
        library: String,
        cell: String,
        #[arg(long)]
        view: Option<String>,
        /// The deck for this simulator instead of the setup's.
        #[arg(long, value_parser = ["ngspice", "xyce", "vacask"])]
        simulator: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum TemplatesCommand {
    /// List the templates.
    List,
    /// Document one: parameters, outputs, default specs, requirements.
    Show { name: String },
}

fn parse_args(items: &[String]) -> Result<std::collections::BTreeMap<String, String>> {
    items
        .iter()
        .map(|a| {
            a.split_once('=')
                .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
                .ok_or_else(|| eyre!("--arg wants name=value, not '{a}'"))
        })
        .collect()
}

fn suite_view_of(project: &Project, library: &str, cell: &str, view: Option<String>) -> Result<String> {
    if let Some(view) = view {
        return Ok(view);
    }
    let found = sim::suite::suites_of(project, library, cell);
    match found.as_slice() {
        [one] => Ok(one.clone()),
        [] => Err(eyre!("{library}/{cell} has no suite view (a view with a suite.toml)")),
        many => Err(eyre!("{library}/{cell} has several suites ({}); pick one with --view", many.join(", "))),
    }
}

fn view_of(project: &Project, library: &str, cell: &str, view: Option<String>) -> Result<String> {
    if let Some(view) = view {
        return Ok(view);
    }
    let found = project
        .library(library)
        .and_then(|l| l.cell(cell))
        .ok_or_else(|| eyre!("no cell {library}/{cell}"))?;
    let tbs: Vec<&str> = found
        .entries
        .iter()
        .filter(|e| e.path.join(sim::setup::SETUP_FILE).is_file())
        .map(|e| e.name.as_str())
        .collect();
    match tbs.as_slice() {
        [one] => Ok(one.to_string()),
        [] => Err(eyre!("{library}/{cell} has no testbench view (a view with a setup.toml)")),
        many => Err(eyre!("{library}/{cell} has several testbenches ({}); pick one with --view", many.join(", "))),
    }
}

pub fn run(command: SimCommand, project: &Project) -> Result<()> {
    match command {
        SimCommand::Templates { action } => {
            let set = sim::template::TemplateSet::for_project(project);
            match action.unwrap_or(TemplatesCommand::List) {
                TemplatesCommand::List => print!("{}", sim::template::format_list(&set)),
                TemplatesCommand::Show { name } => print!("{}", sim::template::format_show(set.find(&name)?)?),
            }
            Ok(())
        }
        SimCommand::New { library, cell, view, template, args, dut } => {
            let set = sim::template::TemplateSet::for_project(project);
            let t = set.find(&template)?;
            let made =
                sim::starter::create_testbench(project, &library, &cell, &view, t, &parse_args(&args)?, dut.as_deref())?;
            println!("wrote {}", made.setup.display());
            if let Some(s) = &made.schematic {
                println!("wrote {}", s.display());
            }
            for n in &made.notes {
                println!("note: {n}");
            }
            println!("run it with: cdept sim run {library} {cell} --view {view}");
            Ok(())
        }
        SimCommand::Suite { library, cell, view, jobs, all_columns, no_store, json, simulator } => {
            let view = suite_view_of(project, &library, &cell, view)?;
            let on_event: Listener = Arc::new(|e| {
                let i = &e.info;
                match i.state {
                    JobState::Done => eprintln!("{:<24} done     {:.1}s", i.name, i.elapsed.as_secs_f32()),
                    JobState::Failed => eprintln!("{:<24} failed   {:.1}s", i.name, i.elapsed.as_secs_f32()),
                    _ => {}
                }
            });
            let options =
                sim::suite::SuiteOptions { jobs: jobs.unwrap_or(0), on_event: Some(on_event), no_store, simulator, ..Default::default() };
            let report = sim::suite::run(project, &library, &cell, &view, &options)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print!("{}", sim::suite::format_report(&report, all_columns));
            }
            let s = report.summary();
            if !s.ok() {
                bail!(
                    "suite {library}/{cell}/{view}: {} failed, {} errored run(s), {} failing and {} missing spec(s), {} test(s) did not run",
                    s.fail,
                    s.error,
                    s.spec_fail,
                    s.spec_missing,
                    s.test_errors
                );
            }
            Ok(())
        }
        SimCommand::Deck { library, cell, view, simulator } => {
            let view = view_of(project, &library, &cell, view)?;
            let mut tb = sim::open(project, &library, &cell, &view)?;
            if let Some(name) = &simulator {
                tb.setup.set_simulator(name)?;
            }
            let tech = project.tech.as_ref().ok_or_else(|| eyre!("no technology snapshot"))?;
            let set = if tb.setup.models.is_empty() {
                tech.models.default.clone().unwrap_or_default()
            } else {
                tb.setup.models.clone()
            };
            let corner = tech.models.corner.clone().unwrap_or_default();
            let point = sim::deck::points(&tb.setup, &corner).remove(0);
            let file = std::env::temp_dir().join(format!("cdept-deck-{}.spice", std::process::id()));
            sim::netlist_for(project, &tb.schematic, &file, &tb.setup.simulator)?;
            let netlist = std::fs::read_to_string(&file)?;
            let _ = std::fs::remove_file(&file);
            let title = format!("{library}/{cell}/{view}");
            if tb.setup.simulator == "vacask" {
                let ready = libcdept::vacask::models::ready(project)?;
                print!("{}", libcdept::vacask::build_point(project, &ready, &tb.setup, &set, &point, &title, &netlist)?.text);
                return Ok(());
            }
            let lines = tech.models.lines(&set, &point.corner, |n| project.var(n)).map_err(|e| eyre!(e))?;
            print!("{}", sim::build_deck(&tb.setup, &point, &title, &lines, &netlist));
            Ok(())
        }
        SimCommand::Run { library, cell, view, corners, jobs, runs, mode, seed, no_mc, no_store, backend, simulator } => {
            let view = view_of(project, &library, &cell, view)?;
            let mut tb = sim::open(project, &library, &cell, &view)?;
            if let Some(name) = &simulator {
                tb.setup.set_simulator(name)?;
            }
            let backend = backend.as_deref().and_then(sim::run::Backend::of);
            if backend == Some(sim::run::Backend::Inproc) {
                // No ngspice program is needed then.
                tb.setup.simulator_backend = "inproc".into();
            }
            let simulator = sim::Simulator::for_setup(&tb.setup).ok_or_else(|| {
                eyre!(
                    "the simulator '{}' was not found: put it on PATH or set {} (ngspice), {} (Xyce) or {} (VACASK)",
                    tb.setup.simulator,
                    sim::run::NGSPICE_ENV,
                    libcdept::batch::xyce::XYCE_ENV,
                    libcdept::batch::vacask::VACASK_ENV
                )
            })?;
            // One line per point as it starts and ends (the events come from worker threads;
            // progress events of a running point are not printed).
            let started = std::sync::Mutex::new(std::collections::HashSet::new());
            let on_event: Listener = Arc::new(move |e| {
                let i = &e.info;
                match i.state {
                    JobState::Queued => {}
                    JobState::Running if !started.lock().unwrap().insert(i.id) => {}
                    JobState::Running => eprintln!("{:<10} running", i.name),
                    JobState::Done => eprintln!("{:<10} done     {:.1}s", i.name, i.elapsed.as_secs_f32()),
                    JobState::Failed => eprintln!("{:<10} failed   {:.1}s", i.name, i.elapsed.as_secs_f32()),
                    JobState::Cancelled => eprintln!("{:<10} cancelled", i.name),
                }
            });
            let options = sim::RunOptions {
                jobs: jobs.unwrap_or(0),
                on_event: Some(on_event),
                mc_runs: runs,
                mc_seed: seed,
                mc_mode: mode,
                no_mc,
                no_store,
                backend,
                ..Default::default()
            };
            let results = sim::run_testbench_with(project, &tb, &simulator, &corners, &options)?;
            let records: Vec<RunRecord> = results.iter().map(|r| r.run.clone()).collect();
            print!("{}", results::format_table(&records));
            let agg = results::aggregate(&records);
            println!();
            if records.len() > 1 && (!tb.setup.sweeps.is_empty() || results.iter().any(|r| r.point.seed.is_some())) {
                print!("{}", results::format_aggregate(&agg));
            } else {
                println!("{} run(s): {} pass, {} fail, {} error", agg.runs, agg.pass, agg.fail, agg.error);
            }
            if records.len() > 1 && results.iter().any(|r| r.point.seed.is_some()) {
                println!();
                print!("{}", sim::stats::format_report(&sim::stats::analyze(&records, 10, 3.0)));
            }
            if results.iter().any(|r| r.record.is_some()) {
                println!("index: {}", results::Store::path(&project.root).display());
            }
            let bad = agg.fail + agg.error;
            if bad > 0 {
                bail!("{bad} of {} run(s) failed", agg.runs);
            }
            Ok(())
        }
        SimCommand::Results { library, cell, view, all, reindex, stats, bins, sigma } => {
            let view = view_of(project, &library, &cell, view)?;
            let mut db = results::Store::open(&project.root)?;
            if reindex {
                println!("reindexed {} run(s)", db.reindex(&project.root)?);
            }
            let records = if all { db.runs(&library, &cell, &view)? } else { db.latest(&library, &cell, &view)? };
            if records.is_empty() {
                bail!("no stored runs of {library}/{cell}/{view} (cdept sim run, or --reindex)");
            }
            print!("{}", results::format_table(&records));
            println!();
            let latest = db.latest(&library, &cell, &view)?;
            print!("{}", results::format_aggregate(&results::aggregate(&latest)));
            if stats {
                println!();
                print!("{}", sim::stats::format_report(&sim::stats::analyze(&latest, bins, sigma)));
            }
            Ok(())
        }
    }
}
