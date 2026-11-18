//! `cdept`: opens the GUI by default; subcommands run headless.

use std::path::PathBuf;

use cdept_gui::{GuiOptions, LogBuffer};
use clap::{Parser, Subcommand, ValueEnum};
use eyre::{Result, bail, eyre};
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
    },
    /// List the libraries, cells and views of the project.
    Ls {
        /// Only this library.
        library: Option<String>,
        /// Only list libraries, not their cells.
        #[arg(short, long)]
        libraries: bool,
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
    },
    /// Remove a library definition. Its folder is left untouched.
    Remove { name: String },
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
        Command::Init { dir, name } => {
            init_tracing(cli.verbose, None);
            let project = Project::create(&dir, name.as_deref())?;
            println!("created {}", project.file.display());
            Ok(())
        }
        Command::Ls { library, libraries } => {
            init_tracing(cli.verbose, None);
            list(&open_project(cli.project)?, library.as_deref(), libraries)
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
                } => {
                    let options = AddLibrary {
                        layout: layout.map(|l| match l {
                            Layout::Cells => LibraryLayout::Cells,
                            Layout::Flat => LibraryLayout::Flat,
                        }),
                        read_only,
                        create: !read_only,
                    };
                    project.add_library(&name, &path, options)?;
                    let lib = project.library(&name).expect("just added");
                    println!("added {name} ({} cells)", lib.cells.len());
                }
                LibCommand::Remove { name } => {
                    project.remove_library(&name)?;
                    println!("removed {name} from {}", project.file.display());
                }
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
