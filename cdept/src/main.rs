//! `cdept`: opens the GUI by default; subcommands run headless.

use std::path::PathBuf;

use cdept_gui::{GuiOptions, LogBuffer};
use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::prelude::*;

#[derive(Parser)]
#[command(name = "cdept", version = libcdept::version::VERSION, about = "Chip Design Project Tool")]
struct Cli {
    /// Project directory.
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
    /// List the external tools found on PATH.
    Tools,
    /// Print version and build information.
    Version,
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

fn main() -> eyre::Result<()> {
    let cli = Cli::parse();
    match cli.command.unwrap_or(Command::Gui) {
        Command::Gui => {
            let log = LogBuffer::new();
            init_tracing(cli.verbose, Some(&log));
            cdept_gui::run(
                GuiOptions {
                    project: cli.project,
                },
                log,
            )
        }
        Command::Tools => {
            init_tracing(cli.verbose, None);
            for t in libcdept::tools::detect_all() {
                let status = match (&t.path, &t.version) {
                    (Some(p), Some(v)) => format!("{v}  ({})", p.display()),
                    (Some(p), None) => p.display().to_string(),
                    (None, _) => "not found".into(),
                };
                println!("{:<10} {:<12} {status}", t.spec.name, t.spec.role);
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
