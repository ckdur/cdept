//! `cdept netgen lvs`, `cdept yosys synth|script`, `cdept openroad run`: batch runs of
//! external tools (docs/BATCH.md).

use std::path::PathBuf;

use clap::Subcommand;
use eyre::{Result, bail, eyre};
use libcdept::Project;
use libcdept::batch::{netgen, openroad, yosys};

#[derive(Subcommand)]
pub enum NetgenCommand {
    /// Compare two netlists with netgen (`netgen -batch lvs`) and print the result.
    ///
    /// The set-up file is `[netgen] setup` of the project's technology snapshot, else
    /// `--setup`, else an empty one. netgen is `$CDEPT_NETGEN` or the one on PATH.
    /// Exits with an error unless the circuits match.
    Lvs {
        /// SPICE netlist of the layout (extracted) and the cell in it.
        layout: PathBuf,
        layout_cell: String,
        /// SPICE netlist of the schematic and the cell in it.
        schematic: PathBuf,
        schematic_cell: String,
        #[arg(long)]
        setup: Option<PathBuf>,
        /// Where the log goes. Default: <project>/batch/netgen.
        #[arg(long)]
        dir: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
pub enum YosysCommand {
    /// Synthesise Verilog to a netlist, mapped to a Liberty file of the technology.
    ///
    /// The Liberty file comes from `--liberty`, else from the technology snapshot
    /// (`--library`, `--corner`; the digital defaults when left out). `--generic`
    /// maps nothing. Yosys is `$CDEPT_YOSYS` or the one on PATH.
    Synth {
        top: String,
        #[arg(required = true)]
        sources: Vec<PathBuf>,
        #[arg(long)]
        liberty: Option<PathBuf>,
        #[arg(long)]
        library: Option<String>,
        #[arg(long)]
        corner: Option<String>,
        #[arg(long, conflicts_with_all = ["liberty", "library", "corner"])]
        generic: bool,
        /// Netlist to write. Default: <top>.v in the run folder.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Run folder. Default: <project>/batch/yosys.
        #[arg(long)]
        dir: Option<PathBuf>,
    },
    /// Run a Yosys script and print the design size it reports.
    Script {
        script: PathBuf,
        #[arg(long)]
        dir: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
pub enum OpenroadCommand {
    /// Run a Tcl script with OpenROAD; the log is kept. OpenROAD is
    /// `$CDEPT_OPENROAD` or the one on PATH.
    Run {
        script: PathBuf,
        /// Run folder. Default: <project>/batch/openroad.
        #[arg(long)]
        dir: Option<PathBuf>,
    },
}

fn run_dir(project: &Project, dir: Option<PathBuf>, tool: &str) -> PathBuf {
    dir.unwrap_or_else(|| project.root.join("batch").join(tool))
}

pub fn netgen(command: NetgenCommand, project: &Project) -> Result<()> {
    let NetgenCommand::Lvs { layout, layout_cell, schematic, schematic_cell, setup, dir } = command;
    let program = netgen::ADAPTER.require()?;
    let setup = setup.or_else(|| netgen::project_setup(project));
    let dir = run_dir(project, dir, "netgen");
    let lvs = netgen::Lvs { layout, layout_cell, schematic, schematic_cell, setup };
    let report = netgen::run(&program, &lvs, &dir, &project.tool_env())?;
    println!("{}", if report.result.is_empty() { "(no result)" } else { &report.result });
    if let Some((a, b)) = report.devices {
        println!("devices: layout {a}, schematic {b}");
    }
    if let Some((a, b)) = report.nets {
        println!("nets:    layout {a}, schematic {b}");
    }
    for cell in &report.property_errors {
        println!("property errors in {cell}");
    }
    if let Some(log) = &report.log {
        println!("log: {}", log.display());
    }
    if !report.verdict.clean() {
        bail!("LVS is not clean ({:?})", report.verdict);
    }
    Ok(())
}

fn print_stat(stat: &Option<yosys::Stat>) {
    match stat {
        None => println!("(no statistics in the log)"),
        Some(s) => {
            println!("{}: {} cells{}", s.module, s.cells, s.area.map(|a| format!(", area {a}")).unwrap_or_default());
            for (name, n) in &s.by_type {
                println!("    {n:>6}  {name}");
            }
        }
    }
}

pub fn yosys(command: YosysCommand, project: &Project) -> Result<()> {
    let program = yosys::ADAPTER.require()?;
    match command {
        YosysCommand::Synth { top, sources, liberty, library, corner, generic, out, dir } => {
            let liberty = match (generic, liberty) {
                (true, _) => None,
                (false, Some(file)) => Some(file),
                (false, None) => Some(yosys::liberty_of(project, library.as_deref(), corner.as_deref())?),
            };
            let dir = run_dir(project, dir, "yosys");
            let netlist = out.unwrap_or_else(|| PathBuf::from(format!("{top}.v")));
            let synth = yosys::Synth { sources, top, liberty, netlist };
            let r = yosys::synth(&program, &synth, &dir, &project.tool_env())?;
            println!("netlist: {}\nlog:     {}", r.netlist.display(), r.log.display());
            print_stat(&r.stat);
        }
        YosysCommand::Script { script, dir } => {
            let dir = run_dir(project, dir, "yosys");
            let (out, stat) = yosys::run_script(&program, &script, &dir, &project.tool_env())?;
            println!("log: {}", out.log.display());
            print_stat(&stat);
        }
    }
    Ok(())
}

pub fn openroad(command: OpenroadCommand, project: &Project) -> Result<()> {
    let OpenroadCommand::Run { script, dir } = command;
    let program = openroad::ADAPTER.require()?;
    let dir = run_dir(project, dir, "openroad");
    let r = openroad::run_script(&program, &script, &dir, &project.tool_env())?;
    println!("log: {}  ({} warnings)", r.output.log.display(), r.warnings);
    if !r.errors.is_empty() {
        return Err(eyre!("{} error(s) in the log:\n{}", r.errors.len(), r.errors.join("\n")));
    }
    Ok(())
}
