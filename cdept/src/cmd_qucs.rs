//! `cdept qucs export|open|spar`: Qucs-S schematics (docs/QUCS.md).

use std::path::PathBuf;

use clap::{Subcommand, ValueEnum};
use eyre::{Result, eyre};
use libcdept::Project;
use libcdept::batch::qucs::{self, Flavour};

#[derive(Clone, Copy, ValueEnum)]
pub enum FlavourArg {
    Ngspice,
    Xyce,
    Cdl,
}

#[derive(Subcommand)]
pub enum QucsCommand {
    /// Convert a cell's Qucs-S schematic to a SPICE netlist, without a window
    /// (`qucs-s -n`). Qucs-S is `$CDEPT_QUCS` or the one on PATH. The netlist holds the
    /// analysis of the schematic's own simulation component unless `--for-sim`.
    Export {
        library: String,
        cell: String,
        /// The schematic view when the cell has several. Default: its first Qucs-S one.
        #[arg(long)]
        view: Option<String>,
        #[arg(long, value_enum, default_value = "ngspice")]
        flavour: FlavourArg,
        /// Drop the schematic's own analysis and `.end`, as a testbench uses it.
        #[arg(long)]
        for_sim: bool,
        /// Netlist to write. Default: <project>/batch/qucs/<library>/<cell>.cir.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Open a cell's Qucs-S schematic in the Qucs-S window.
    Open {
        library: String,
        cell: String,
        #[arg(long)]
        view: Option<String>,
    },
    /// Open a Touchstone file (.s2p, .snp) in the Qucs-S S-parameter viewer.
    Spar { file: PathBuf },
}

pub fn run(command: QucsCommand, project: Option<&Project>) -> Result<()> {
    let need = || project.ok_or_else(|| eyre!("no project (run inside one or use --project)"));
    match command {
        QucsCommand::Export {
            library,
            cell,
            view,
            flavour,
            for_sim,
            out,
        } => {
            let project = need()?;
            let file = qucs::schematic_for(project, &library, &cell, view.as_deref(), "qucs")?;
            let flavour = match flavour {
                FlavourArg::Ngspice => Flavour::Ngspice,
                FlavourArg::Xyce => Flavour::Xyce,
                FlavourArg::Cdl => Flavour::Cdl,
            };
            let out = out.unwrap_or_else(|| {
                project
                    .root
                    .join("batch")
                    .join("qucs")
                    .join(&library)
                    .join(format!("{cell}.cir"))
            });
            qucs::export(Some(project), &file, &out, flavour)?;
            if for_sim {
                let text = std::fs::read_to_string(&out)?;
                std::fs::write(&out, qucs::sim_netlist(&text, flavour))?;
            }
            println!("{}", out.display());
        }
        QucsCommand::Open {
            library,
            cell,
            view,
        } => {
            let project = need()?;
            let file = qucs::schematic_for(project, &library, &cell, view.as_deref(), "qucs")?;
            qucs::launch(Some(project), Some(&file))?;
        }
        QucsCommand::Spar { file } => {
            qucs::launch_spar_viewer(project, Some(&file))?;
        }
    }
    Ok(())
}
