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
        /// Base seed of the Monte Carlo runs.
        #[arg(long)]
        seed: Option<u64>,
        /// Run the corner/temperature/sweep grid although the setup has [montecarlo].
        #[arg(long)]
        no_mc: bool,
        /// Do not write result.json / results.db.
        #[arg(long)]
        no_store: bool,
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
    },
    /// Print the deck of the first run without starting the simulator.
    Deck {
        library: String,
        cell: String,
        #[arg(long)]
        view: Option<String>,
    },
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
        SimCommand::Deck { library, cell, view } => {
            let view = view_of(project, &library, &cell, view)?;
            let tb = sim::open(project, &library, &cell, &view)?;
            let tech = project.tech.as_ref().ok_or_else(|| eyre!("no technology snapshot"))?;
            let set = if tb.setup.models.is_empty() {
                tech.models.default.clone().unwrap_or_default()
            } else {
                tb.setup.models.clone()
            };
            let corner = tech.models.corner.clone().unwrap_or_default();
            let point = sim::deck::points(&tb.setup, &corner).remove(0);
            let lines = tech.models.lines(&set, &point.corner, |n| project.var(n)).map_err(|e| eyre!(e))?;
            let file = std::env::temp_dir().join(format!("cdept-deck-{}.spice", std::process::id()));
            sim::netlist(project, &tb.schematic, &file)?;
            let netlist = std::fs::read_to_string(&file)?;
            let _ = std::fs::remove_file(&file);
            print!("{}", sim::build_deck(&tb.setup, &point, &format!("{library}/{cell}/{view}"), &lines, &netlist));
            Ok(())
        }
        SimCommand::Run { library, cell, view, corners, jobs, runs, seed, no_mc, no_store } => {
            let view = view_of(project, &library, &cell, view)?;
            let tb = sim::open(project, &library, &cell, &view)?;
            let simulator = sim::Simulator::for_setup(&tb.setup).ok_or_else(|| {
                eyre!(
                    "the simulator '{}' was not found: put it on PATH or set {} (ngspice) or {} (Xyce)",
                    tb.setup.simulator,
                    sim::run::NGSPICE_ENV,
                    libcdept::batch::xyce::XYCE_ENV
                )
            })?;
            // One line per point as it starts and ends (the events come from worker threads).
            let on_event: Listener = Arc::new(|e| {
                let i = &e.info;
                match i.state {
                    JobState::Queued => {}
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
                no_mc,
                no_store,
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
            if results.iter().any(|r| r.record.is_some()) {
                println!("index: {}", results::Store::path(&project.root).display());
            }
            let bad = agg.fail + agg.error;
            if bad > 0 {
                bail!("{bad} of {} run(s) failed", agg.runs);
            }
            Ok(())
        }
        SimCommand::Results { library, cell, view, all, reindex } => {
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
            print!("{}", results::format_aggregate(&results::aggregate(&db.latest(&library, &cell, &view)?)));
            Ok(())
        }
    }
}
