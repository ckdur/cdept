//! `cdept magic extract|drc|lef|lvs|link|check-source|refs|import-gds|export-gds …`:
//! Magic in batch mode and the project's Magic views (docs/MAGIC.md).

use std::time::Duration;

use clap::{Subcommand, ValueEnum};
use eyre::{Result, WrapErr, eyre};
use libcdept::Project;
use libcdept::magic::{self, ExtractOptions, Magic, Parasitics, Source, flows};

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
    /// LVS of a Magic layout against the cell's xschem schematic (netgen).
    ///
    /// Magic extracts, xschem netlists the schematic (the layout's linked schematic,
    /// else the cell's), netgen compares with the PDK's set-up file. Exit status 1 when
    /// the netlists do not match.
    Lvs {
        library: String,
        cell: String,
        #[arg(long, value_parser = parse_timeout)]
        timeout: Option<Duration>,
    },
    /// Generate or update the Magic layout of a cell from its schematic, with the
    /// PDK's Magic device generators (pdk/<PDK>/magic-devices.toml).
    ///
    /// Missing devices are added in a row, changed ones regenerated in place; layout
    /// instances share the schematic instance names.
    Link {
        library: String,
        cell: String,
        /// The schematic view (default: the linked one, else `schematic`).
        #[arg(long)]
        schematic: Option<String>,
        /// Also delete layout instances that are not in the schematic.
        #[arg(long)]
        remove: bool,
        #[arg(long, value_parser = parse_timeout)]
        timeout: Option<Duration>,
    },
    /// Compare a Magic layout with its schematic; exit status 1 when out of sync.
    CheckSource { library: String, cell: String },
    /// Check the references between the project's Magic views (use lines, sidecars).
    Refs {
        /// Make project paths relative and write the sidecars.
        #[arg(long)]
        fix: bool,
    },
    /// Read a GDS file into a library as Magic views (one cell each).
    ImportGds {
        file: std::path::PathBuf,
        library: String,
        /// Only these cells (and what they place); default: all.
        #[arg(long = "cell")]
        cells: Vec<String>,
        /// Replace cells the library has already.
        #[arg(long)]
        replace: bool,
    },
    /// Write a Magic layout, with everything it places, as one GDS file.
    ExportGds {
        library: String,
        cell: String,
        /// The file (default: <project>/magic/<library>/<cell>/<cell>.gds).
        #[arg(short, long)]
        output: Option<std::path::PathBuf>,
    },
}

fn parse_timeout(text: &str) -> Result<Duration, String> {
    libcdept::jobs::parse_duration(text).map_err(|e| format!("{e}"))
}

pub fn run(command: MagicCommand, project: &Project) -> Result<()> {
    let mut magic = Magic::for_project(project)?;
    let (library, cell, timeout) = match &command {
        MagicCommand::Refs { fix } => return refs(project, *fix),
        MagicCommand::ImportGds {
            file,
            library,
            cells,
            replace,
        } => {
            let lib = project
                .library(library)
                .ok_or_else(|| eyre!("no library '{library}'"))?;
            let done = flows::import_gds(&magic, project, file, lib, cells, *replace)?;
            for c in &done.cells {
                println!("{library}/{c}");
            }
            for c in &done.skipped {
                println!("{library}/{c}: the library has it already (--replace to replace it)");
            }
            return Ok(());
        }
        MagicCommand::ExportGds {
            library,
            cell,
            output,
        } => {
            let file = flows::link_layout(project, library, cell)?;
            let out = output.clone().unwrap_or_else(|| {
                magic::out_dir(project, library, cell).join(format!("{cell}.gds"))
            });
            let written = flows::export_gds(&magic, &Source::Mag { file }, &out)?;
            println!("{}", written.display());
            return Ok(());
        }
        MagicCommand::CheckSource { library, cell } => {
            let found = flows::generate(&magic, project, library, cell, None, false, true)?;
            if found.plan.in_sync() {
                println!("{library}/{cell}: the layout matches its schematic");
                return Ok(());
            }
            for line in found.plan.lines() {
                println!("  {line}");
            }
            std::process::exit(1);
        }
        MagicCommand::Lvs {
            library,
            cell,
            timeout,
        }
        | MagicCommand::Link {
            library,
            cell,
            timeout,
            ..
        } => (library.clone(), cell.clone(), *timeout),
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
    match &command {
        MagicCommand::Lvs { .. } => {
            let r = flows::lvs(&magic, project, &library, &cell)?;
            for note in &r.notes {
                println!("note: {note}");
            }
            println!("layout:    {}", r.layout_netlist.display());
            println!("schematic: {}", r.schematic_netlist.display());
            if let Some(log) = &r.report.log {
                println!("log:       {}", log.display());
            }
            println!("{library}/{cell}: {}", r.report.result);
            if !r.report.verdict.clean() {
                std::process::exit(1);
            }
            return Ok(());
        }
        MagicCommand::Link {
            schematic, remove, ..
        } => {
            let made = flows::generate(
                &magic,
                project,
                &library,
                &cell,
                schematic.as_deref(),
                *remove,
                false,
            )?;
            println!("{}", made.layout.display());
            if made.plan.actions.is_empty() {
                println!("  in sync, nothing to do");
            }
            for line in flows::plan_summary(&made.plan) {
                println!("  {line}");
            }
            for (name, reason) in &made.plan.problems {
                println!("  ! {name}: {reason}");
            }
            return Ok(());
        }
        _ => {}
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
        MagicCommand::Lvs { .. }
        | MagicCommand::Link { .. }
        | MagicCommand::Refs { .. }
        | MagicCommand::ImportGds { .. }
        | MagicCommand::ExportGds { .. }
        | MagicCommand::CheckSource { .. } => unreachable!("handled above"),
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

fn refs(project: &Project, fix: bool) -> Result<()> {
    let problems = magic::refs::check(project, fix);
    let views = magic::refs::mag_views(project)
        .iter()
        .filter(|v| !v.read_only)
        .count();
    for p in &problems {
        println!("{}: {}", p.view, p.text);
    }
    let bad = problems
        .iter()
        .filter(|p| !p.text.starts_with("fixed:"))
        .count();
    println!("{views} Magic view(s), {bad} problem(s)");
    if bad > 0 {
        std::process::exit(1);
    }
    Ok(())
}
