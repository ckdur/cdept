//! `cdept magic extract|drc|lef …`: Magic in batch mode (docs/MAGIC.md).

use std::time::Duration;

use clap::{Subcommand, ValueEnum};
use eyre::{Result, WrapErr, eyre};
use libcdept::Project;
use libcdept::magic::{self, ExtractOptions, Magic, Parasitics, Source};

#[derive(Clone, Copy, ValueEnum)]
pub enum Pex {
    /// Devices and connectivity only.
    None,
    /// With capacitances.
    C,
}

#[derive(Subcommand)]
pub enum MagicCommand {
    /// Extract a layout to SPICE with the PDK's extraction style.
    ///
    /// Magic is `$CDEPT_MAGIC` or the one on PATH. Files go to
    /// <project>/magic/<library>/<cell>/ (<cell>.spice and Magic's .ext files).
    Extract {
        library: String,
        cell: String,
        /// Extraction style (default: the technology's).
        #[arg(long)]
        style: Option<String>,
        /// Parasitics to extract.
        #[arg(long, value_enum, default_value = "none")]
        pex: Pex,
        /// Stop Magic after this long: 90s, 10m, 1h; 0 for no limit (default: [magic]
        /// timeout in cdept.toml, else 15m).
        #[arg(long, value_parser = parse_timeout)]
        timeout: Option<Duration>,
    },
    /// Check a layout against the PDK's DRC rules; prints the rules violated.
    ///
    /// Writes <cell>.drc.json next to the other files. Exit status 1 when not clean.
    Drc {
        library: String,
        cell: String,
        /// DRC style, e.g. `drc(full)` (default: the technology's).
        #[arg(long)]
        style: Option<String>,
        /// Stop Magic after this long: 90s, 10m, 1h; 0 for no limit (default: [magic]
        /// timeout in cdept.toml, else 15m).
        #[arg(long, value_parser = parse_timeout)]
        timeout: Option<Duration>,
    },
    /// Write the LEF abstract of a layout.
    Lef {
        library: String,
        cell: String,
        /// Hide the inside of the cell (`lef write -hide`).
        #[arg(long)]
        hide: bool,
        /// Stop Magic after this long: 90s, 10m, 1h; 0 for no limit (default: [magic]
        /// timeout in cdept.toml, else 15m).
        #[arg(long, value_parser = parse_timeout)]
        timeout: Option<Duration>,
    },
}

fn parse_timeout(text: &str) -> Result<Duration, String> {
    libcdept::jobs::parse_duration(text).map_err(|e| format!("{e}"))
}

pub fn run(command: MagicCommand, project: &Project) -> Result<()> {
    let mut magic = Magic::for_project(project)?;
    let (library, cell, timeout) = match &command {
        MagicCommand::Extract {
            library,
            cell,
            timeout,
            ..
        }
        | MagicCommand::Drc {
            library,
            cell,
            timeout,
            ..
        }
        | MagicCommand::Lef {
            library,
            cell,
            timeout,
            ..
        } => (library.clone(), cell.clone(), *timeout),
    };
    if let Some(limit) = timeout {
        magic = magic.with_timeout(limit);
    }
    let src = Source::of_cell(project, &library, &cell)?;
    let dir = magic::out_dir(project, &library, &cell);
    match command {
        MagicCommand::Extract { style, pex, .. } => {
            let options = ExtractOptions {
                style,
                parasitics: match pex {
                    Pex::None => Parasitics::None,
                    Pex::C => Parasitics::Capacitance,
                },
            };
            let e = magic.extract(&src, &dir, &options)?;
            println!("{}", e.spice.display());
            for s in &e.summary.subckts {
                println!("  .subckt {} ({} ports)", s.name, s.ports.len());
            }
            for (model, n) in &e.summary.devices {
                println!("  {n:>5} x {model}");
            }
        }
        MagicCommand::Drc { style, .. } => {
            let report = magic.drc(&src, &dir, style.as_deref())?;
            let file = dir.join(format!("{}.drc.json", report.cell));
            std::fs::write(&file, serde_json::to_string_pretty(&report)?)
                .wrap_err_with(|| format!("writing {}", file.display()))?;
            if report.is_clean() {
                println!(
                    "{library}/{cell}: DRC clean ({})",
                    report.style.as_deref().unwrap_or("-")
                );
            } else {
                println!(
                    "{library}/{cell}: {} violation(s) of {} rule(s), report {}",
                    report.count(),
                    report.violations.len(),
                    file.display()
                );
                for (rule, n) in report.by_rule() {
                    println!("  {n:>5}  {rule}");
                }
                std::process::exit(1);
            }
        }
        MagicCommand::Lef { hide, .. } => {
            let l = magic.lef(&src, &dir, hide)?;
            println!("{}", l.file.display());
            let s = l.summary.ok_or_else(|| eyre!("the LEF has no macro"))?;
            if let Some([w, h]) = s.size {
                println!("  {} {w} x {h} um, {} pins", s.name, s.pins.len());
            }
        }
    }
    Ok(())
}
