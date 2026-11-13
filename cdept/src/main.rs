//! `cdept`: opens the GUI by default; subcommands run headless.

use std::path::PathBuf;

use cdept_gui::{GuiOptions, LogBuffer};
use clap::{Parser, Subcommand, ValueEnum};
use eyre::{Result, eyre};
use libcdept::{AddLibrary, LibraryLayout, Project};
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
    Gui,
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
    match cli.command.unwrap_or(Command::Gui) {
        Command::Gui => {
            let log = LogBuffer::new();
            init_tracing(cli.verbose, Some(&log));
            cdept_gui::run(
                GuiOptions {
                    project: project_root(cli.project),
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
