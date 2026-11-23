//! `cdept sim …`: testbench views, decks and ngspice runs (docs/SIMULATION.md).

use std::sync::Arc;

use clap::Subcommand;
use libcdept::jobs::{JobState, Listener};
use eyre::{Result, bail, eyre};
use libcdept::Project;
use libcdept::sim::{self, spec::Verdict};

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
        SimCommand::Run { library, cell, view, corners, jobs } => {
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
            let options = sim::RunOptions { jobs: jobs.unwrap_or(0), on_event: Some(on_event), ..Default::default() };
            let results = sim::run_testbench_with(project, &tb, &simulator, &corners, &options)?;
            let mut bad = 0;
            for r in &results {
                match &r.outcome {
                    Err(e) => {
                        bad += 1;
                        println!("{:<10} FAILED  {e:#}", r.point.name());
                    }
                    Ok(o) => {
                        println!("{:<10} ok      {}", r.point.name(), o.raw.display());
                        for (name, v) in &o.measures {
                            println!("    {name:<16} {v:e}");
                        }
                        for row in &r.specs {
                            let (word, v) = match row.verdict {
                                Verdict::Pass => ("PASS", false),
                                Verdict::Fail => ("FAIL", true),
                                Verdict::Missing => ("MISSING", true),
                            };
                            bad += v as usize;
                            println!("    spec {:<11} {word}", row.name);
                        }
                    }
                }
            }
            if bad > 0 {
                bail!("{bad} run(s) or spec(s) failed");
            }
            Ok(())
        }
    }
}
