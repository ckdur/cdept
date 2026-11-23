//! `cdept`: opens the GUI by default; subcommands run headless.

mod cmd_calc;
mod cmd_cell;
mod cmd_find;
mod cmd_import;
mod cmd_qucs;
mod cmd_lib;
mod cmd_ls;
mod cmd_ngspice;
mod cmd_magic;
mod cmd_pdk;
mod cmd_batch;
mod cmd_sim;
mod cmd_status;
mod cmd_tech;
mod cmd_vacask;

use std::path::PathBuf;

use cdept_gui::{GuiOptions, LogBuffer};
use clap::{Parser, Subcommand, ValueEnum};
use eyre::{Result, bail, eyre};
use libcdept::klayout::verify::{self, Check};
use libcdept::klayout::{Engine as KlayoutEngine, EngineOptions as KlayoutOptions};
use libcdept::link;
use libcdept::xschem::symbol::{self, Problem};
use libcdept::xschem::{Engine, Hierarchy, NetlistFormat};
use libcdept::{AddLibrary, Format, LibraryLayout, Project};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::prelude::*;

#[derive(Parser)]
#[command(name = "cdept", version = libcdept::version::VERSION, about = "Chip Design Project Tool")]
struct Cli {
    /// Project folder (the one holding cdept.toml). Defaults to the project the
    /// current folder belongs to.
    #[arg(short, long, global = true)]
    project: Option<PathBuf>,

    /// More log output (-v debug, -vv trace). `RUST_LOG` overrides this.
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Open the main window (default).
    Gui {
        /// Run a script of GUI commands, for testing (see docs/GUI-TESTING.md).
        #[arg(long)]
        script: Option<PathBuf>,
    },
    /// Create a project (a cdept.toml file) in a folder.
    Init {
        /// Folder of the new project.
        #[arg(default_value = ".")]
        dir: PathBuf,
        /// Project name. Defaults to the folder name.
        #[arg(long)]
        name: Option<String>,
        /// PDK name (`technology` in cdept.toml, `$PDK` for tools), e.g. ihp-sg13g2,
        /// or the path of a PDK folder. The PDK is scanned and what was found is
        /// written next to cdept.toml (tech.toml, see `cdept tech`).
        #[arg(long)]
        technology: Option<String>,
        /// Folder PDKs are installed in (`$PDK_ROOT` for tools); kept in
        /// cdept.local.toml, as it differs per machine. Default: where the PDK is found.
        #[arg(long)]
        pdk_root: Option<String>,
        /// Do not scan the PDK: only name it in cdept.toml.
        #[arg(long)]
        no_scan: bool,
        /// Scan files only: do not start KLayout (PCells) or xschem (xschemrc).
        #[arg(long)]
        files_only: bool,
    },
    /// Show the project: its file, the PDK in use ($PDK, $PDK_ROOT) and where each
    /// value comes from, the libraries.
    Info,
    /// List the libraries, cells and views of the project.
    Ls {
        /// Only this library.
        library: Option<String>,
        /// Only this cell of the library: its views, their files and what the files
        /// say about it (LEF class and size, Liberty area and pins, …).
        cell: Option<String>,
        /// Only list libraries, not their cells.
        #[arg(short, long)]
        libraries: bool,
        /// One line per view, with its file and, when the file holds many cells (a
        /// standard-cell GDS, LEF, Liberty file), the cell inside it.
        #[arg(short, long)]
        files: bool,
    },
    /// Change the library definitions in cdept.toml.
    #[command(subcommand)]
    Lib(LibCommand),
    /// Write the netlist of a cell's schematic, with its hierarchy (uses xschem).
    Netlist {
        library: String,
        cell: String,
        /// Output file. Defaults to <cell>.<suffix of the format> in the current folder.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Write the netlist into the cell itself, as its `netlist` view.
        #[arg(long, conflicts_with_all = ["output", "view"])]
        into_view: bool,
        #[arg(short, long, value_enum, default_value = "spice")]
        format: NetlistKind,
        /// The view to netlist, when the cell has several schematics.
        #[arg(long)]
        view: Option<String>,
    },
    /// Use the embedded xschem engine directly.
    #[command(subcommand)]
    Xschem(XschemCommand),
    /// Layout views: references between them, export, pictures (uses KLayout).
    #[command(subcommand)]
    Layout(LayoutCommand),
    /// Use the embedded KLayout directly.
    #[command(subcommand)]
    Klayout(KlayoutCommand),
    /// The PDKs installed on this machine and their versions (ciel).
    #[command(subcommand)]
    Pdk(cmd_pdk::PdkCommand),
    /// ngspice in-process (libngspice, loaded at run time).
    #[command(subcommand)]
    Ngspice(cmd_ngspice::NgspiceCommand),
    /// Magic in batch mode: extraction, DRC, LEF export.
    #[command(subcommand)]
    Magic(cmd_magic::MagicCommand),
    /// Testbenches: decks and ngspice runs with corners and specs.
    #[command(subcommand)]
    Sim(cmd_sim::SimCommand),
    /// LVS with netgen (batch).
    #[command(subcommand)]
    Netgen(cmd_batch::NetgenCommand),
    /// Synthesis with Yosys (batch).
    #[command(subcommand)]
    Yosys(cmd_batch::YosysCommand),
    /// Run OpenROAD scripts (batch).
    #[command(subcommand)]
    Openroad(cmd_batch::OpenroadCommand),
    /// Qucs-S: schematic to SPICE netlist (headless), open in Qucs-S, S-parameter viewer.
    #[command(subcommand)]
    Qucs(cmd_qucs::QucsCommand),
    /// VACASK models of the technology (made from its ngspice models and Verilog-A).
    #[command(subcommand)]
    Vacask(cmd_vacask::VacaskCommand),
    /// OpenVAF: compile Verilog-A to OSDI, cached per technology.
    #[command(subcommand)]
    Openvaf(cmd_vacask::OpenvafCommand),
    /// The project's technology snapshot (tech.toml): scan the PDK, show, refresh.
    #[command(subcommand)]
    Tech(cmd_tech::TechCommand),
    /// Evaluate a calculator expression on a SPICE raw file (`cdept calc --functions` lists them).
    Calc(cmd_calc::CalcArgs),
    /// Search the libraries: cells and views by name, kind, format and category,
    /// optionally inside the files (see `cdept find --help` for the query words).
    Find(cmd_find::FindArgs),
    /// Categories: named groups of the cells of a library.
    #[command(subcommand)]
    Category(cmd_find::CategoryCommand),
    /// Propose a project (cdept.toml) for an existing design folder: its libraries
    /// from the xschemrc, the folders with design files or a LibMan project file.
    Import(cmd_import::ImportArgs),
    /// IHP LibMan: write the libraries as a LibMan project file, start LibMan.
    #[command(subcommand)]
    Libman(cmd_import::LibmanCommand),
    /// Cells: create, copy, rename, move, delete. What uses a cell follows.
    #[command(subcommand)]
    Cell(cmd_cell::CellCommand),
    /// Views of a cell: create, copy, rename, delete.
    #[command(subcommand)]
    View(cmd_cell::ViewCommand),
    /// The hierarchy below a cell, read from the files (schematic, layout, netlist, HDL).
    Hier(cmd_cell::HierArgs),
    /// Which cells use a cell, through which views, how often.
    UsedBy(cmd_cell::UsedByArgs),
    /// The git status of the project, by library, cell and view.
    Status(cmd_status::StatusArgs),
    /// List the external tools found on PATH.
    Tools,
    /// Print version and build information.
    Version,
}

#[derive(Subcommand)]
enum LibCommand {
    /// Define a library. The folder is created if it does not exist.
    Add {
        name: String,
        /// Library folder, written to cdept.toml as given ($VAR is kept).
        path: String,
        /// Reference library: CDePT will not modify it.
        #[arg(long)]
        read_only: bool,
        /// Folder layout. Detected from the folder when left out.
        #[arg(long)]
        layout: Option<Layout>,
        /// Define it also when its folder gets in the way of another library (the
        /// same folder, or a folder of the same name, which xschem cannot tell apart).
        #[arg(long)]
        force: bool,
    },
    /// Remove a library definition. Its folder is left untouched.
    Remove { name: String },
    /// Rename a library (and, with --folder, its folder). What names it follows.
    Rename(cmd_cell::LibRename),
    #[command(flatten)]
    Definition(cmd_lib::Command),
}

#[derive(Subcommand)]
enum XschemCommand {
    /// Draw a schematic or symbol view into an SVG file.
    Svg {
        library: String,
        cell: String,
        /// Output file. Defaults to <cell>.svg in the current folder.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// The view to draw. Defaults to the schematic, else the symbol.
        #[arg(long)]
        view: Option<String>,
    },
    /// List the instances placed in a schematic: name, symbol, type.
    Instances {
        library: String,
        cell: String,
        #[arg(long)]
        view: Option<String>,
    },
    /// Show the hierarchy below a schematic: which cells it uses, through which
    /// instances.
    Hierarchy {
        library: String,
        cell: String,
        #[arg(long)]
        view: Option<String>,
        /// Also list the instances that are not subcircuits (transistors, pins, …).
        #[arg(short, long)]
        all: bool,
    },
    /// Make the symbol of a cell from the pins of its schematic.
    ///
    /// The same as xschem's "Make symbol from schematic". In a `cells` library the
    /// symbol goes into the cell's `symbol` view and names its schematic.
    MakeSymbol {
        library: String,
        cell: String,
        /// Replace the symbol if the cell has one. Its drawing is lost.
        #[arg(short, long)]
        force: bool,
    },
    /// Make the schematic of a cell from the pins of its symbol.
    ///
    /// The same as xschem's "Make schematic from symbol": a pin of the schematic for
    /// each pin of the symbol, nothing else. In a `cells` library the schematic goes
    /// where the symbol's schematic= attribute says, normally the `schematic` view.
    MakeSchematic {
        library: String,
        cell: String,
        /// Replace the schematic if the cell has one. Its contents are lost.
        #[arg(short, long)]
        force: bool,
    },
    /// Check that xschem will find the schematic behind every symbol.
    ///
    /// In a `cells` library the schematic is not next to its symbol, so the symbol
    /// names it with a `schematic=` attribute. CDePT writes the attribute and keeps
    /// it right; this finds the symbols where it is missing or wrong, e.g. after files
    /// were copied by hand. Without it xschem netlists the cell empty.
    Link {
        /// Only this library.
        library: Option<String>,
        /// Only this cell.
        cell: Option<String>,
        /// Write the attribute where it is missing or names a file that does not exist.
        #[arg(long)]
        fix: bool,
        /// With --fix: also rewrite attributes that name another cell's schematic.
        #[arg(long, requires = "fix")]
        foreign: bool,
    },
    /// Evaluate a Tcl script in the engine and print the result (for debugging).
    Tcl { script: String },
    /// Print the version of the engine.
    Version,
}

#[derive(Subcommand)]
enum LayoutCommand {
    /// Check the references of a cell's layout to cells of other views.
    ///
    /// A layout holds its own cell; the cells it places are found through its sidecar
    /// (`<file>.json`) or by searching the libraries for a cell of that name. Lists
    /// every reference and how it was resolved; fails if one cannot be found.
    Refs {
        library: String,
        cell: String,
        /// Record what was found by searching, and fix a sidecar that names another
        /// cell, in the sidecar.
        #[arg(long)]
        fix: bool,
    },
    /// Write a self-contained GDS (or OASIS) of a cell's layout, with every
    /// referenced cell filled in, e.g. for DRC, LVS and tape-out.
    Export {
        library: String,
        cell: String,
        /// Output file; `.oas` writes OASIS.
        #[arg(short, long)]
        output: PathBuf,
        /// The layout view, when the cell has several.
        #[arg(long)]
        view: Option<String>,
    },
    /// Draw a cell's layout into a PNG file.
    Png {
        library: String,
        cell: String,
        /// Output file. Defaults to <cell>.png in the current folder.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Picture size in pixels, as WIDTHxHEIGHT.
        #[arg(long, default_value = "800x600", value_parser = parse_size)]
        size: (u32, u32),
        #[arg(long)]
        view: Option<String>,
    },
    /// Run the PDK's DRC deck on a cell's layout (KLayout in batch mode).
    ///
    /// The deck is `[klayout] drc` in cdept.toml, else found in the PDK's
    /// libs.tech/klayout. Prints the violations per rule; fails if there are any.
    Drc {
        library: String,
        cell: String,
        #[command(flatten)]
        check: CheckArgs,
    },
    /// Compare a cell's layout with its schematic, with the PDK's LVS deck.
    ///
    /// The schematic is netlisted by xschem for LVS. The deck is `[klayout] lvs` in
    /// cdept.toml, else found in the PDK. Fails if the netlists do not match.
    Lvs {
        library: String,
        cell: String,
        /// The schematic view, when the cell has several.
        #[arg(long)]
        schematic: Option<String>,
        /// Compare with this SPICE netlist instead of netlisting the schematic.
        #[arg(long, conflicts_with = "schematic")]
        netlist: Option<PathBuf>,
        #[command(flatten)]
        check: CheckArgs,
    },
    // -- linking with the schematic (docs/LINKING.md) --------------------------------
    /// Generate or update a cell's layout from its schematic (Layout XL style).
    ///
    /// Every device of the schematic becomes a PCell of the PDK's KLayout library (see
    /// the device map, pdk/<PDK>/devices.toml and `[klayout] devices`), every instance
    /// of a project cell a reference to that cell's layout. New instances are placed in
    /// rows next to what the layout has; changed parameters are updated; the link is
    /// recorded in the layout's sidecar. Creates the layout view if there is none.
    Link {
        library: String,
        cell: String,
        /// The schematic view, when the cell has several.
        #[arg(long)]
        schematic: Option<String>,
        /// The layout view, when the cell has several.
        #[arg(long)]
        view: Option<String>,
        /// Also delete layout instances whose schematic instance is gone.
        #[arg(long)]
        remove: bool,
        /// Only print what would change.
        #[arg(long)]
        dry_run: bool,
    },
    /// Compare a cell's layout with its schematic; fails if they are not in sync.
    CheckSource {
        library: String,
        cell: String,
        #[arg(long)]
        schematic: Option<String>,
        #[arg(long)]
        view: Option<String>,
    },
    /// List the pins of a cell's layout and check them (docs/LINKING.md §9): against
    /// the schematic's pins (when the cell has a schematic), their labels, and the
    /// layout file (pins on the technology's pin layers). Fails on a finding.
    Pins {
        library: String,
        cell: String,
        #[arg(long)]
        schematic: Option<String>,
        #[arg(long)]
        view: Option<String>,
        /// Print the pins and the findings as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Compare the nets of a cell's layout with its schematic's: opens, shorts,
    /// unconnected terminals, pins without a label (docs/LINKING.md §8).
    ///
    /// The layout's nets are extracted with the PDK's LVS deck; the schematic's come
    /// from xschem, through the hierarchy. Layout instances are matched to schematic
    /// instances by the link (`cdept layout link`). Fails if the nets do not agree.
    CheckNets {
        library: String,
        cell: String,
        #[arg(long)]
        schematic: Option<String>,
        #[arg(long)]
        view: Option<String>,
        /// Print the whole comparison as JSON.
        #[arg(long)]
        json: bool,
        /// Folder for the stream, the database and KLayout's output. Defaults to
        /// ~/.cache/cdept/verify/<project>/<library>/<cell>/nets.
        #[arg(long)]
        dir: Option<PathBuf>,
    },
    /// Devices in parallel (an array of transistors): list the groups found in the
    /// schematic, and record them in the layout's sidecar with your choices.
    ///
    /// A joined group gets one flyline per open net, and one layout device may stand
    /// for all its members (an extraction that combines them). Without options the
    /// groups are listed and recorded.
    Parallel {
        library: String,
        cell: String,
        #[arg(long)]
        schematic: Option<String>,
        #[arg(long)]
        view: Option<String>,
        /// Do not join the group holding this instance (repeat for more).
        #[arg(long, value_name = "INSTANCE")]
        off: Vec<String>,
        /// Join the group holding this instance again.
        #[arg(long, value_name = "INSTANCE")]
        on: Vec<String>,
        /// Make a group of your own: instance paths separated by commas.
        #[arg(long, value_name = "A,B,…")]
        group: Vec<String>,
        /// Remove a group of your own (any of its instances).
        #[arg(long, value_name = "INSTANCE")]
        ungroup: Vec<String>,
    },
}

/// What `layout drc` and `layout lvs` share.
#[derive(clap::Args)]
struct CheckArgs {
    /// The deck to run instead of the project's.
    #[arg(long)]
    deck: Option<PathBuf>,
    /// The layout view, when the cell has several.
    #[arg(long)]
    view: Option<String>,
    /// Further variables for the deck, as NAME=VALUE (`-rd` of KLayout).
    #[arg(short = 'D', long = "define", value_parser = parse_define)]
    defines: Vec<(String, String)>,
    /// Folder for the stream, the report and KLayout's output. Defaults to
    /// ~/.cache/cdept/verify/<project>/<library>/<cell>/<drc|lvs>.
    #[arg(long)]
    dir: Option<PathBuf>,
}

fn parse_define(text: &str) -> Result<(String, String), String> {
    match text.split_once('=') {
        Some((name, value)) if !name.is_empty() => Ok((name.to_string(), value.to_string())),
        _ => Err(format!("'{text}' is not NAME=VALUE")),
    }
}

/// `cdept layout pins`.
fn layout_pins(
    project: &Project,
    library: &str,
    cell: &str,
    schematic: Option<&str>,
    view: Option<&str>,
    json: bool,
) -> Result<()> {
    let file = layout_view(project, library, cell, view)?;
    let meta = libcdept::layout::Meta::read_for(&file)?;
    let map = link::DeviceMap::for_project(project)?;
    let rules = map.pins.rules();
    // The schematic's pins, when the cell has a schematic.
    let sch = match link::schematic_view(project, library, cell, schematic) {
        Ok((source, sch_file)) => Some(link::read_schematic(project, &map, source, &sch_file)?),
        Err(e) if schematic.is_some() => return Err(e),
        Err(_) => None,
    };
    let wanted: Option<Vec<link::pins::Wanted<'_>>> = sch.as_ref().map(|s| {
        s.pins
            .iter()
            .map(|p| link::pins::Wanted {
                name: &p.name,
                direction: &p.direction,
            })
            .collect()
    });
    let checks = link::pins::check_file(&file, meta.as_ref(), wanted.as_deref(), Some(&rules))?;
    if json {
        let pins = meta.as_ref().map(|m| m.pins.clone()).unwrap_or_default();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({"pins": pins, "checks": checks}))?
        );
    } else {
        match &sch {
            Some(s) => println!("{} ↔ {}", s.source.label(), file.display()),
            None => println!("{} (no schematic)", file.display()),
        }
        if rules.layers.is_empty() {
            println!("note: this technology describes no pin layers: pins are CDePT's only");
        }
        println!("{}", link::pins::Report(&checks));
    }
    if checks.iter().any(|c| !c.is_ok()) {
        std::process::exit(1);
    }
    Ok(())
}

/// Run DRC or LVS on `library/cell` from the command line.
/// `cdept layout check-nets`.
fn layout_check_nets(
    project: &Project,
    library: &str,
    cell: &str,
    schematic: Option<&str>,
    view: Option<&str>,
    json: bool,
    dir: Option<PathBuf>,
) -> Result<()> {
    let (source, schematic) = link::schematic_view(project, library, cell, schematic)?;
    let file = layout_view(project, library, cell, view)?;
    let dir = match dir {
        Some(dir) => {
            std::fs::create_dir_all(&dir)?;
            dir
        }
        None => verify::work_dir_named(project, library, cell, "nets")?,
    };
    let stream = verify::export_stream(project, &file, &dir.join(format!("{cell}.gds")))?;
    if !stream.missing.is_empty() {
        tracing::warn!(
            "cells not found, checked as empty: {} (see `cdept layout refs`)",
            stream.missing.join(", ")
        );
    }
    let recorded = link::nets::recorded_parallel(&file);
    let pins = libcdept::layout::Meta::read_for(&file)?
        .map(|m| m.pins)
        .unwrap_or_default();
    let run = link::nets::run(
        project,
        source.clone(),
        &schematic,
        &stream,
        &dir,
        &recorded,
        &pins,
    )?;
    if json {
        println!("{}", serde_json::to_string_pretty(&run.report)?);
    } else {
        println!("{} ↔ {}", source.label(), file.display());
        for line in link::nets::report_lines(&run.report, &run.connectivity) {
            println!("{line}");
        }
        let (ok, open, short) = run.report.counts();
        println!("nets: {ok} ok, {open} open, {short} shorted");
        for group in &run.parallel {
            println!(
                "parallel: {}{}",
                group.members.join(" "),
                if group.joined { "" } else { " (not joined)" }
            );
        }
        println!("database {}", run.lvsdb.display());
    }
    if !run.report.clean() {
        std::process::exit(1);
    }
    Ok(())
}

/// What `cdept layout parallel` changes.
struct ParallelEdit {
    off: Vec<String>,
    on: Vec<String>,
    group: Vec<String>,
    ungroup: Vec<String>,
}

/// `cdept layout parallel`.
fn layout_parallel(
    project: &Project,
    library: &str,
    cell: &str,
    schematic: Option<&str>,
    view: Option<&str>,
    edit: &ParallelEdit,
) -> Result<()> {
    use libcdept::layout::Parallel;
    let (source, schematic) = link::schematic_view(project, library, cell, schematic)?;
    let file = layout_view(project, library, cell, view)?;
    let map = link::DeviceMap::for_project(project)?;
    let flat = link::nets::read_flat(project, &map, source, &schematic)?;
    let mut groups = link::nets::merge_parallel(
        &link::nets::parallel_groups(&flat),
        &link::nets::recorded_parallel(&file),
    );
    let holding = |groups: &[Parallel], name: &str| {
        groups
            .iter()
            .position(|g| g.members.iter().any(|m| m == name))
    };
    for (names, joined) in [(&edit.off, false), (&edit.on, true)] {
        for name in names {
            let at = holding(&groups, name)
                .ok_or_else(|| eyre!("{name} is in no group of devices in parallel"))?;
            groups[at].joined = joined;
        }
    }
    for list in &edit.group {
        let mut members: Vec<String> = list
            .split(',')
            .map(str::trim)
            .filter(|m| !m.is_empty())
            .map(String::from)
            .collect();
        members.sort();
        members.dedup();
        if members.len() < 2 {
            return Err(eyre!("a group needs two instances at least: {list}"));
        }
        for m in &members {
            if !flat.devices.contains_key(m) {
                return Err(eyre!("the schematic has no device {m}"));
            }
        }
        groups.retain(|g| !g.members.iter().any(|m| members.contains(m)));
        groups.push(Parallel {
            members,
            joined: true,
            manual: true,
        });
    }
    for name in &edit.ungroup {
        let at = holding(&groups, name).ok_or_else(|| eyre!("{name} is in no group"))?;
        if !groups[at].manual {
            return Err(eyre!(
                "{name} is in a group found in the schematic; use --off to stop joining it"
            ));
        }
        groups.remove(at);
    }
    for g in &groups {
        let mut flags = Vec::new();
        if !g.joined {
            flags.push("not joined");
        }
        if g.manual {
            flags.push("yours");
        }
        let flags = if flags.is_empty() {
            String::new()
        } else {
            format!(" ({})", flags.join(", "))
        };
        println!("{}{flags}", g.members.join(" "));
    }
    if groups.is_empty() {
        println!("no devices in parallel");
    }
    link::nets::record_parallel(&file, &groups)?;
    Ok(())
}

fn layout_check(
    project: &Project,
    library: &str,
    cell: &str,
    check: Check,
    args: &CheckArgs,
    schematic: Option<(Option<&str>, Option<&std::path::Path>)>,
) -> Result<()> {
    let deck = match &args.deck {
        Some(deck) => deck.clone(),
        None => verify::find_deck(project, check).ok_or_else(|| {
            eyre!(
                "no {} deck: set [klayout] {} in cdept.toml or pass --deck (none found in \
                 the PDK's libs.tech/klayout)",
                check.label(),
                check.name()
            )
        })?,
    };
    let file = layout_view(project, library, cell, args.view.as_deref())?;
    let dir = match &args.dir {
        Some(dir) => {
            std::fs::create_dir_all(dir)?;
            dir.clone()
        }
        None => verify::work_dir(project, library, cell, check)?,
    };
    let stream = verify::export_stream(project, &file, &dir.join(format!("{cell}.gds")))?;
    if !stream.missing.is_empty() {
        tracing::warn!(
            "cells not found, checked as empty: {} (see `cdept layout refs`)",
            stream.missing.join(", ")
        );
    }
    println!("deck    {}", deck.display());
    println!(
        "layout  {} (top cell {})",
        stream.file.display(),
        stream.top
    );
    if check == Check::Drc {
        let run = verify::run_drc(project, &deck, &stream, &dir, &args.defines)?;
        let counts = run.rdb.counts();
        let total: usize = counts.iter().map(|(_, n)| n).sum();
        for (category, n) in &counts {
            println!("{n:6}  {}  {}", category.name(), category.description);
        }
        println!("report  {}", run.report.display());
        if total > 0 {
            bail!("{total} DRC violation(s) in {} rule(s)", counts.len());
        }
        println!("DRC clean");
        return Ok(());
    }
    let (view, netlist) = schematic.unwrap_or_default();
    let netlist = match netlist {
        Some(netlist) => netlist.to_path_buf(),
        None => {
            let schematic = verify::schematic_file(project, library, cell, view)?;
            verify::schematic_netlist(project, &schematic, &dir.join(format!("{cell}.spice")))?
        }
    };
    println!("netlist {}", netlist.display());
    let run = verify::run_lvs(project, &deck, &stream, &netlist, &dir, &args.defines)?;
    for circuit in &run.summary.circuits {
        println!(
            "{:<16} layout {} / schematic {}",
            circuit.status, circuit.layout, circuit.schematic
        );
        for (kind, pair) in circuit.pairs() {
            let name = |n: &str| {
                if n.is_empty() {
                    "-".to_string()
                } else {
                    n.to_string()
                }
            };
            println!(
                "  {:<16} {kind:<10} {} / {}",
                pair.status,
                name(&pair.layout),
                name(&pair.schematic)
            );
        }
    }
    println!("report  {}", run.report.display());
    println!("extracted {}", run.extracted.display());
    if !run.summary.matched {
        bail!("the layout does not match the schematic");
    }
    println!("LVS match");
    Ok(())
}

#[derive(Subcommand)]
enum KlayoutCommand {
    /// Print the version of KLayout the helper runs in.
    Version,
    /// Run Python in the helper and print the value of the last expression (for
    /// debugging).
    Python { code: String },
}

fn parse_size(text: &str) -> Result<(u32, u32), String> {
    let (width, height) = text
        .split_once(['x', 'X'])
        .ok_or_else(|| format!("'{text}' is not WIDTHxHEIGHT"))?;
    let number = |v: &str| {
        v.trim()
            .parse::<u32>()
            .ok()
            .filter(|n| (1..=16384).contains(n))
            .ok_or_else(|| format!("'{v}' is not a size between 1 and 16384"))
    };
    Ok((number(width)?, number(height)?))
}

#[derive(Clone, Copy, ValueEnum)]
enum NetlistKind {
    Spice,
    Spectre,
    Verilog,
    Vhdl,
    Tedax,
}

impl From<NetlistKind> for NetlistFormat {
    fn from(kind: NetlistKind) -> Self {
        match kind {
            NetlistKind::Spice => NetlistFormat::Spice,
            NetlistKind::Spectre => NetlistFormat::Spectre,
            NetlistKind::Verilog => NetlistFormat::Verilog,
            NetlistKind::Vhdl => NetlistFormat::Vhdl,
            NetlistKind::Tedax => NetlistFormat::Tedax,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum Layout {
    /// <library>/<cell>/<view>/<files>
    Cells,
    /// <library>/<cell>.<ext>, one view per file
    Flat,
    /// <library>/<format>/<files>: a PDK reference library (libs.ref), read only
    Ref,
}

/// The project named by `--project`, or the one the current folder belongs to.
fn project_root(explicit: Option<PathBuf>) -> Option<PathBuf> {
    explicit.or_else(|| Project::find_root(&std::env::current_dir().ok()?))
}

fn open_project(explicit: Option<PathBuf>) -> Result<Project> {
    let root = project_root(explicit)
        .ok_or_else(|| eyre!("not inside a project (no cdept.toml); try `cdept init`"))?;
    let project = Project::load(&root)?;
    for warning in &project.warnings {
        tracing::warn!("{warning}");
    }
    Ok(project)
}

/// Print the instances of cell `index`, indented, descending into subcircuits.
/// `path` holds the cells being printed, to stop at a schematic that contains itself.
fn print_hierarchy(
    hierarchy: &Hierarchy,
    index: usize,
    depth: usize,
    all: bool,
    path: &mut Vec<usize>,
) {
    let indent = "  ".repeat(depth);
    for item in &hierarchy.cells[index].instances {
        let instance = &item.instance;
        match item.child {
            Some(child) => {
                let name = &hierarchy.cells[child].name;
                if path.contains(&child) {
                    println!("{indent}{} ({name}, recursive)", instance.name);
                    continue;
                }
                println!("{indent}{} ({name})", instance.name);
                path.push(child);
                print_hierarchy(hierarchy, child, depth + 1, all, path);
                path.pop();
            }
            None if instance.kind == "subcircuit" => {
                println!(
                    "{indent}{} ({}, no schematic)",
                    instance.name, instance.symbol
                );
            }
            None if all => println!("{indent}{} [{}]", instance.name, instance.kind),
            None => {}
        }
    }
}

/// `cdept xschem link`: report the symbols whose schematic xschem would not find, and
/// repair them with `fix`. Fails when problems are left.
fn link_symbols(
    project: &Project,
    library: Option<&str>,
    cell: Option<&str>,
    fix: bool,
    foreign: bool,
) -> Result<()> {
    if let Some(name) = library
        && project.library(name).is_none()
    {
        bail!("no library '{name}'");
    }
    let mut left = 0;
    for unlinked in symbol::check(project, library, cell) {
        let symbol = project.portable_path(&unlinked.symbol);
        let wanted = fix && (foreign || !matches!(unlinked.problem, Problem::Foreign { .. }));
        let outcome = if !wanted {
            Err(None)
        } else {
            symbol::repair(project, &unlinked).map_err(Some)
        };
        match outcome {
            Ok(()) => println!("{symbol}: set schematic={}", unlinked.expected),
            Err(error) => {
                left += 1;
                println!("{symbol}: {}", unlinked.problem);
                println!("    expected schematic={}", unlinked.expected);
                if let Some(error) = error {
                    println!("    not fixed: {error:#}");
                }
            }
        }
    }
    if left > 0 {
        let hint = match (fix, foreign) {
            (false, _) => " (--fix writes the expected value)",
            (true, false) => " (--foreign also rewrites links to another cell)",
            (true, true) => "",
        };
        bail!("{left} symbol(s) not linked to the schematic of their cell{hint}");
    }
    Ok(())
}

/// The xschem file of a cell's view: the named view, else the schematic, else (when
/// `or_symbol`) the symbol.
fn xschem_view(
    project: &Project,
    library: &str,
    cell: &str,
    view: Option<&str>,
    or_symbol: bool,
) -> Result<PathBuf> {
    let lib = project
        .library(library)
        .ok_or_else(|| eyre!("no library '{library}'"))?;
    if let Some(error) = &lib.error {
        return Err(eyre!("library '{library}': {error}"));
    }
    let found = lib
        .cell(cell)
        .ok_or_else(|| eyre!("no cell '{cell}' in library '{library}'"))?;
    let is_xschem = |format| matches!(format, Some(Format::XschemSchematic | Format::XschemSymbol));
    let entry = match view {
        Some(name) => found
            .entry(name)
            .ok_or_else(|| eyre!("cell '{cell}' has no view '{name}'"))?,
        None => found
            .entries
            .iter()
            .find(|e| e.format == Some(Format::XschemSchematic))
            .or_else(|| {
                found
                    .entries
                    .iter()
                    .find(|e| or_symbol && e.format == Some(Format::XschemSymbol))
            })
            .ok_or_else(|| eyre!("cell '{cell}' has no xschem schematic view"))?,
    };
    if !is_xschem(entry.format) {
        return Err(eyre!(
            "view '{}' of '{cell}' is not an xschem file",
            entry.name
        ));
    }
    entry
        .main_file
        .clone()
        .ok_or_else(|| eyre!("view '{}' of '{cell}' has no file", entry.name))
}

/// What `cdept info` prints: the project and the PDK in use.
/// The layout file of a cell: the view `view`, else the one named `layout`, else the
/// first GDS or OASIS view.
fn layout_view(
    project: &Project,
    library: &str,
    cell: &str,
    view: Option<&str>,
) -> Result<PathBuf> {
    let lib = project
        .library(library)
        .ok_or_else(|| eyre!("no library '{library}'"))?;
    let Some(view) = view else {
        return libcdept::klayout::cell_layout(lib, cell);
    };
    lib.cell(cell)
        .ok_or_else(|| eyre!("library '{library}' has no cell '{cell}'"))?
        .entry(view)
        .filter(|e| matches!(e.format, Some(Format::Gds | Format::Oasis)))
        .and_then(|e| e.main_file.clone())
        .ok_or_else(|| eyre!("cell '{cell}' has no GDS or OASIS view '{view}'"))
}

/// `cdept layout link`: generate or update the layout from the schematic.
fn layout_link(
    mut project: Project,
    library: &str,
    cell: &str,
    schematic: Option<&str>,
    view: Option<&str>,
    remove: bool,
    dry_run: bool,
) -> Result<()> {
    let (map, sch) = link::prepare(&project, library, cell, schematic)?;
    if dry_run {
        let file = layout_view(&project, library, cell, view);
        match file {
            Ok(file) => println!("{}", link::check(&sch, &file)?),
            Err(_) => println!("no layout yet: {} instance(s) to add", sch.wanted.len()),
        }
        return Ok(());
    }
    let file = link::layout_view(&mut project, library, cell, view, true)?;
    let mut engine = KlayoutEngine::for_project(&project)?;
    let options = link::ApplyOptions { remove };
    let applied = link::generate(&project, &mut engine, &map, &sch, &file, &options)?;
    engine.shutdown()?;
    println!("{} → {}", sch.source.label(), file.display());
    println!("{applied}");
    for p in &sch.problems {
        println!("problem {} ({}): {}", p.name, p.symbol, p.reason);
    }
    if !applied.failed.is_empty() {
        std::process::exit(1);
    }
    Ok(())
}

fn layout_refs(project: &Project, library: &str, cell: &str, fix: bool) -> Result<()> {
    let findings = if fix {
        libcdept::layout::refs::fix(project, library, cell)?
    } else {
        libcdept::layout::refs::check(project, library, cell)?
    };
    if findings.is_empty() {
        println!("no references");
    }
    for finding in &findings {
        let mark = if finding.is_error() {
            "error"
        } else if finding.fixable() {
            if fix { "fixed" } else { "fix" }
        } else {
            "ok"
        };
        println!("{mark:<6} {finding}");
    }
    let errors = findings.iter().filter(|f| f.is_error()).count();
    let fixable = findings.iter().filter(|f| f.fixable()).count();
    if fixable > 0 && !fix {
        println!("{fixable} can be fixed with --fix");
    }
    if errors > 0 {
        bail!("{errors} reference(s) cannot be found");
    }
    Ok(())
}

fn info(project: &Project) {
    println!("project      {}", project.name);
    println!("file         {}", project.file.display());
    let shown = |value: &Option<libcdept::Resolved>| match value {
        Some(r) => format!("{}  ({})", r.value, r.source.name()),
        None => "not set".to_string(),
    };
    println!("PDK          {}", shown(&project.pdk.name));
    println!("PDK_ROOT     {}", shown(&project.pdk.root));
    cmd_tech::info(project);
    for rc in &project.xschem_rc {
        println!("xschem rc    {}", rc.display());
    }
    let errors = project
        .libraries
        .iter()
        .filter(|l| l.error.is_some())
        .count();
    println!(
        "libraries    {} ({errors} with errors)",
        project.libraries.len()
    );
}

fn list(project: &Project, only: Option<&str>, libraries_only: bool) -> Result<()> {
    if let Some(name) = only
        && project.library(name).is_none()
    {
        return Err(eyre!("no library '{name}'"));
    }
    for lib in &project.libraries {
        if only.is_some_and(|name| name != lib.name) {
            continue;
        }
        let mut flags = vec![lib.layout.name().to_string()];
        if lib.read_only {
            flags.push("read-only".into());
        }
        if let Some(error) = &lib.error {
            flags.push(format!("error: {error}"));
        }
        println!(
            "{}  {}  [{}]",
            lib.name,
            lib.path.display(),
            flags.join(", ")
        );
        if libraries_only {
            continue;
        }
        for cell in &lib.cells {
            let views: Vec<&str> = cell.entries.iter().map(|e| e.name.as_str()).collect();
            println!("  {:<32} {}", cell.name, views.join(" "));
        }
    }
    Ok(())
}

/// Quiet down the GUI stack unless explicitly asked for via `RUST_LOG`.
const NOISY: &str =
    "egui=warn,eframe=warn,egui_glow=warn,winit=warn,sctk=warn,calloop=warn,arboard=warn";

fn init_tracing(verbose: u8, console: Option<&LogBuffer>) {
    let level = match verbose {
        0 => "info",
        1 => "debug",
        _ => "trace",
    };
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("{level},{NOISY}")));
    let fmt = tracing_subscriber::fmt::layer().with_writer(std::io::stderr);
    tracing_subscriber::registry()
        .with(filter)
        .with(fmt)
        .with(console.map(LogBuffer::layer))
        .init();
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command.unwrap_or(Command::Gui { script: None }) {
        Command::Gui { script } => {
            let log = LogBuffer::new();
            init_tracing(cli.verbose, Some(&log));
            cdept_gui::run(
                GuiOptions {
                    project: project_root(cli.project),
                    script,
                },
                log,
            )
        }
        Command::Init {
            dir,
            name,
            technology,
            pdk_root,
            no_scan,
            files_only,
        } => {
            init_tracing(cli.verbose, None);
            cmd_tech::init(
                &dir,
                name.as_deref(),
                technology.as_deref(),
                pdk_root.as_deref(),
                no_scan,
                files_only,
            )
        }
        Command::Tech(command) => {
            init_tracing(cli.verbose, None);
            // `scan --print` works outside a project too.
            let project = match project_root(cli.project) {
                Some(root) => Some(Project::load(&root)?),
                None => None,
            };
            cmd_tech::run(command, project)
        }
        Command::Ngspice(command) => cmd_ngspice::run(command),
        Command::Magic(command) => {
            init_tracing(cli.verbose, None);
            cmd_magic::run(command, &open_project(cli.project)?)
        }
        Command::Sim(command) => {
            init_tracing(cli.verbose, None);
            cmd_sim::run(command, &open_project(cli.project)?)
        }
        Command::Netgen(command) => {
            init_tracing(cli.verbose, None);
            cmd_batch::netgen(command, &open_project(cli.project)?)
        }
        Command::Yosys(command) => {
            init_tracing(cli.verbose, None);
            cmd_batch::yosys(command, &open_project(cli.project)?)
        }
        Command::Openroad(command) => {
            init_tracing(cli.verbose, None);
            cmd_batch::openroad(command, &open_project(cli.project)?)
        }
        Command::Qucs(command) => {
            init_tracing(cli.verbose, None);
            let project = match project_root(cli.project) {
                Some(root) => Some(Project::load(&root)?),
                None => None,
            };
            cmd_qucs::run(command, project.as_ref())
        }
        Command::Vacask(command) => {
            init_tracing(cli.verbose, None);
            cmd_vacask::vacask(command, &open_project(cli.project)?)
        }
        Command::Openvaf(command) => {
            init_tracing(cli.verbose, None);
            let project = project_root(cli.project).and_then(|r| Project::load(&r).ok());
            cmd_vacask::openvaf(command, project.as_ref())
        }
        Command::Info => {
            init_tracing(cli.verbose, None);
            info(&open_project(cli.project)?);
            Ok(())
        }
        Command::Ls {
            library,
            cell,
            libraries,
            files,
        } => {
            init_tracing(cli.verbose, None);
            let project = open_project(cli.project)?;
            match (&library, &cell) {
                (Some(library), Some(cell)) => cmd_ls::cell(&project, library, cell),
                _ if files => cmd_ls::files(&project, library.as_deref()),
                _ => list(&project, library.as_deref(), libraries),
            }
        }
        Command::Lib(command) => {
            init_tracing(cli.verbose, None);
            let mut project = open_project(cli.project)?;
            match command {
                LibCommand::Add {
                    name,
                    path,
                    read_only,
                    layout,
                    force,
                } => {
                    cmd_lib::check_folder(&project, &name, &path, force)?;
                    let options = AddLibrary {
                        layout: layout.map(|l| match l {
                            Layout::Cells => LibraryLayout::Cells,
                            Layout::Flat => LibraryLayout::Flat,
                            Layout::Ref => LibraryLayout::Ref,
                        }),
                        read_only,
                        create: !read_only,
                    };
                    project.add_library(&name, &path, options)?;
                    let lib = project.library(&name).expect("just added");
                    println!("added {name} ({} cells)", lib.cells.len());
                }
                LibCommand::Remove { name } => cmd_lib::remove(&mut project, &name)?,
                LibCommand::Rename(args) => cmd_cell::lib_rename(&project, args)?,
                LibCommand::Definition(command) => cmd_lib::run(&mut project, command)?,
            }
            Ok(())
        }
        Command::Netlist {
            library,
            cell,
            output,
            into_view,
            format,
            view,
        } => {
            init_tracing(cli.verbose, None);
            let project = open_project(cli.project)?;
            let format = NetlistFormat::from(format);
            let file = xschem_view(&project, &library, &cell, view.as_deref(), false)?;
            let unlinked = symbol::check(&project, None, None);
            if !unlinked.is_empty() {
                tracing::warn!(
                    "xschem will not find the schematic of {} symbol(s), their cells may \
                     come out empty: see `cdept xschem link`",
                    unlinked.len()
                );
            }
            let mut engine = Engine::for_project(&project)?;
            let output = if into_view {
                let library = project.library(&library).expect("checked by xschem_view");
                engine.netlist_view(library, &cell, format)?
            } else {
                let output =
                    output.unwrap_or_else(|| PathBuf::from(format!("{cell}.{}", format.suffix())));
                engine.load(&file)?;
                engine.netlist(format, &output)?;
                output
            };
            engine.shutdown()?;
            println!("{}", output.display());
            Ok(())
        }
        Command::Xschem(command) => {
            init_tracing(cli.verbose, None);
            let project = open_project(cli.project)?;
            if let XschemCommand::Link {
                library,
                cell,
                fix,
                foreign,
            } = command
            {
                return link_symbols(&project, library.as_deref(), cell.as_deref(), fix, foreign);
            }
            let mut engine = Engine::for_project(&project)?;
            match command {
                XschemCommand::Link { .. } => unreachable!("handled above"),
                XschemCommand::Svg {
                    library,
                    cell,
                    output,
                    view,
                } => {
                    let file = xschem_view(&project, &library, &cell, view.as_deref(), true)?;
                    let output = output.unwrap_or_else(|| PathBuf::from(format!("{cell}.svg")));
                    engine.load(&file)?;
                    engine.export_svg(&output)?;
                    println!("{}", output.display());
                }
                XschemCommand::Hierarchy {
                    library,
                    cell,
                    view,
                    all,
                } => {
                    let file = xschem_view(&project, &library, &cell, view.as_deref(), false)?;
                    let hierarchy = engine.hierarchy(&file)?;
                    println!("{}", hierarchy.cells[0].name);
                    print_hierarchy(&hierarchy, 0, 1, all, &mut vec![0]);
                }
                XschemCommand::Instances {
                    library,
                    cell,
                    view,
                } => {
                    let file = xschem_view(&project, &library, &cell, view.as_deref(), false)?;
                    engine.load(&file)?;
                    for instance in engine.instances()? {
                        println!(
                            "{:<16} {:<40} {}",
                            instance.name, instance.symbol, instance.kind
                        );
                    }
                }
                XschemCommand::MakeSymbol {
                    library,
                    cell,
                    force,
                } => {
                    let library = project
                        .library(&library)
                        .ok_or_else(|| eyre!("no library '{library}'"))?;
                    let symbol = engine.make_symbol(library, &cell, force)?;
                    println!("{}", symbol.display());
                }
                XschemCommand::MakeSchematic {
                    library,
                    cell,
                    force,
                } => {
                    let library = project
                        .library(&library)
                        .ok_or_else(|| eyre!("no library '{library}'"))?;
                    let schematic = engine.make_schematic(library, &cell, force)?;
                    println!("{}", schematic.display());
                }
                XschemCommand::Tcl { script } => println!("{}", engine.tcl(&script)?),
                XschemCommand::Version => println!("xschem {}", engine.xschem_version()),
            }
            engine.shutdown()?;
            Ok(())
        }
        Command::Layout(command) => {
            init_tracing(cli.verbose, None);
            let project = open_project(cli.project)?;
            match command {
                LayoutCommand::Refs { library, cell, fix } => {
                    layout_refs(&project, &library, &cell, fix)
                }
                LayoutCommand::Export {
                    library,
                    cell,
                    output,
                    view,
                } => {
                    let file = layout_view(&project, &library, &cell, view.as_deref())?;
                    let refs = libcdept::layout::refs::resolve(&project, &file);
                    let mut engine = KlayoutEngine::for_project(&project)?;
                    engine.export(&refs, &output)?;
                    engine.shutdown()?;
                    println!("{}", output.display());
                    Ok(())
                }
                LayoutCommand::Png {
                    library,
                    cell,
                    output,
                    size: (width, height),
                    view,
                } => {
                    let file = layout_view(&project, &library, &cell, view.as_deref())?;
                    let refs = libcdept::layout::refs::resolve(&project, &file);
                    let output = output.unwrap_or_else(|| PathBuf::from(format!("{cell}.png")));
                    let mut engine = KlayoutEngine::for_project(&project)?;
                    // One cell of a file that holds several (a standard-cell GDS).
                    let member = cmd_ls::member_of(&project, &library, &cell, &file);
                    engine.render_cell_png(&refs, member.as_deref(), width, height, &output)?;
                    engine.shutdown()?;
                    println!("{}", output.display());
                    Ok(())
                }
                LayoutCommand::Link {
                    library,
                    cell,
                    schematic,
                    view,
                    remove,
                    dry_run,
                } => layout_link(
                    project,
                    &library,
                    &cell,
                    schematic.as_deref(),
                    view.as_deref(),
                    remove,
                    dry_run,
                ),
                LayoutCommand::CheckSource {
                    library,
                    cell,
                    schematic,
                    view,
                } => {
                    let (_, sch) = link::prepare(&project, &library, &cell, schematic.as_deref())?;
                    let file = layout_view(&project, &library, &cell, view.as_deref())?;
                    let plan = link::check(&sch, &file)?;
                    println!("{} ↔ {}", sch.source.label(), file.display());
                    println!("{plan}");
                    if !plan.in_sync() {
                        std::process::exit(1);
                    }
                    Ok(())
                }
                LayoutCommand::Pins {
                    library,
                    cell,
                    schematic,
                    view,
                    json,
                } => layout_pins(
                    &project,
                    &library,
                    &cell,
                    schematic.as_deref(),
                    view.as_deref(),
                    json,
                ),
                LayoutCommand::CheckNets {
                    library,
                    cell,
                    schematic,
                    view,
                    json,
                    dir,
                } => layout_check_nets(
                    &project,
                    &library,
                    &cell,
                    schematic.as_deref(),
                    view.as_deref(),
                    json,
                    dir,
                ),
                LayoutCommand::Parallel {
                    library,
                    cell,
                    schematic,
                    view,
                    off,
                    on,
                    group,
                    ungroup,
                } => layout_parallel(
                    &project,
                    &library,
                    &cell,
                    schematic.as_deref(),
                    view.as_deref(),
                    &ParallelEdit {
                        off,
                        on,
                        group,
                        ungroup,
                    },
                ),
                LayoutCommand::Drc {
                    library,
                    cell,
                    check,
                } => layout_check(&project, &library, &cell, Check::Drc, &check, None),
                LayoutCommand::Lvs {
                    library,
                    cell,
                    schematic,
                    netlist,
                    check,
                } => layout_check(
                    &project,
                    &library,
                    &cell,
                    Check::Lvs,
                    &check,
                    Some((schematic.as_deref(), netlist.as_deref())),
                ),
            }
        }
        Command::Klayout(command) => {
            init_tracing(cli.verbose, None);
            // Works outside a project too, with KLayout's defaults.
            let options = match open_project(cli.project) {
                Ok(project) => KlayoutOptions::for_project(&project),
                Err(_) => KlayoutOptions::default(),
            };
            let mut engine = KlayoutEngine::start(options)?;
            match command {
                KlayoutCommand::Version => println!("{}", engine.klayout_version()),
                KlayoutCommand::Python { code } => println!("{}", engine.python(&code)?),
            }
            engine.shutdown()?;
            Ok(())
        }
        Command::Pdk(command) => {
            init_tracing(cli.verbose, None);
            // Inside a project its PDK folder is looked at first; outside is fine too.
            let project = project_root(cli.project).and_then(|root| Project::load(&root).ok());
            cmd_pdk::run(command, project.as_ref())
        }
        Command::Calc(args) => cmd_calc::run(args),
        Command::Find(args) => {
            init_tracing(cli.verbose, None);
            cmd_find::find(&open_project(cli.project)?, args)
        }
        Command::Category(command) => {
            init_tracing(cli.verbose, None);
            cmd_find::category(&open_project(cli.project)?, command)
        }
        Command::Import(args) => {
            init_tracing(cli.verbose, None);
            cmd_import::import(args, cli.project)
        }
        Command::Libman(command) => {
            init_tracing(cli.verbose, None);
            let project = match project_root(cli.project) {
                Some(root) => Some(Project::load(&root)?),
                None => None,
            };
            cmd_import::libman(command, project)
        }
        Command::Cell(command) => {
            init_tracing(cli.verbose, None);
            cmd_cell::cell(&open_project(cli.project)?, command)
        }
        Command::View(command) => {
            init_tracing(cli.verbose, None);
            cmd_cell::view(&open_project(cli.project)?, command)
        }
        Command::Hier(args) => {
            init_tracing(cli.verbose, None);
            cmd_cell::hier(&open_project(cli.project)?, args)
        }
        Command::UsedBy(args) => {
            init_tracing(cli.verbose, None);
            cmd_cell::used_by(&open_project(cli.project)?, args)
        }
        Command::Status(args) => {
            init_tracing(cli.verbose, None);
            cmd_status::run(args, &open_project(cli.project)?)
        }
        Command::Tools => {
            init_tracing(cli.verbose, None);
            for t in libcdept::tools::detect_all() {
                let status = match (&t.path, &t.version) {
                    (Some(p), Some(v)) => format!("{v}  ({})", p.display()),
                    (Some(p), None) => p.display().to_string(),
                    (None, _) => "not found".into(),
                };
                let phase = t.spec.phase.map_or("-".into(), |p| p.to_string());
                println!(
                    "{:<20} {:<16} {phase:<2} {status}",
                    t.spec.name, t.spec.role
                );
            }
            Ok(())
        }
        Command::Version => {
            println!("cdept {}", libcdept::version::long_version());
            println!("commit {}", libcdept::version::GIT_SHA);
            Ok(())
        }
    }
}
