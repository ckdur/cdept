//! `cdept import`: propose (and write) a project for an existing design folder;
//! `cdept libman`: IHP LibMan interop.

use std::path::PathBuf;

use clap::{Args, Subcommand};
use eyre::{Result, bail};
use libcdept::import::{self, Options, libman};
use libcdept::{PROJECT_FILE, Project};

#[derive(Args)]
pub struct ImportArgs {
    /// The folder to look at: one with an xschemrc, schematic and layout folders, or
    /// an IHP LibMan project file. Default: the current folder.
    folder: Option<PathBuf>,
    /// Write the project file. Without it the plan is only printed.
    #[arg(long)]
    write: bool,
    /// The project name (default: the folder name).
    #[arg(long)]
    name: Option<String>,
    /// Put cdept.toml into this folder instead of the scanned one (created if
    /// missing); the libraries stay where they are.
    #[arg(long)]
    into: Option<PathBuf>,
    /// Import from this LibMan project file (*.projects). Those in the folder are
    /// found by themselves.
    #[arg(long)]
    libman: Option<PathBuf>,
    /// Leave these libraries out (names as the plan prints them). May be repeated.
    #[arg(long = "skip")]
    skip: Vec<String>,
    /// List these libraries although the plan leaves them unticked (a PDK's tests
    /// and examples). May be repeated; `all` ticks every library.
    #[arg(long = "with")]
    with: Vec<String>,
    /// Add the libraries to the project the folder already has (or the one given
    /// with -p) instead of making a new one.
    #[arg(long)]
    add: bool,
    /// Do not run the xschemrc with the xschem helper: read its text only.
    #[arg(long)]
    no_helper: bool,
    /// Print the project file the plan stands for instead of the plan.
    #[arg(long)]
    toml: bool,
}

pub fn import(args: ImportArgs, project: Option<PathBuf>) -> Result<()> {
    let folder = match (&args.folder, &args.libman) {
        (Some(folder), _) => folder.clone(),
        (None, Some(file)) => file
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(".")),
        (None, None) => PathBuf::from("."),
    };
    let options = Options {
        name: args.name.clone(),
        into: args.into.clone(),
        helpers: !args.no_helper,
        libman: args.libman.clone(),
        ..Options::default()
    };
    let mut plan = import::scan(&folder, &options)?;
    for name in &args.skip {
        match plan.libraries.iter_mut().find(|l| &l.name == name) {
            Some(library) => library.enabled = false,
            None => {
                bail!("--skip {name}: the plan has no library of that name");
            }
        }
    }
    for name in &args.with {
        if name == "all" {
            plan.libraries.iter_mut().for_each(|l| l.enabled = true);
            continue;
        }
        match plan.libraries.iter_mut().find(|l| &l.name == name) {
            Some(library) => library.enabled = true,
            None => {
                bail!("--with {name}: the plan has no library of that name");
            }
        }
    }
    if args.toml {
        print!("{}", plan.toml());
    } else {
        print!("{}", plan.describe());
    }
    for problem in plan.problems() {
        eprintln!("warning: {problem}");
    }
    if args.add {
        let root = project
            .or_else(|| Project::find_root(&plan.root))
            .ok_or_else(|| eyre::eyre!("--add: no project in {}", plan.root.display()))?;
        let mut project = Project::load(&root)?;
        let added = plan.add_to(&mut project)?;
        if added.is_empty() {
            eprintln!("\nNothing to add: the project has these libraries already.");
        } else {
            eprintln!(
                "\nAdded to {}: {}",
                project.file.display(),
                added.join(", ")
            );
        }
        return Ok(());
    }
    if !args.write {
        eprintln!(
            "\nNothing written. --write makes {}; --skip NAME and --with NAME change which \
             libraries are listed.",
            plan.root.join(PROJECT_FILE).display()
        );
        return Ok(());
    }
    let project = plan.write()?;
    eprintln!(
        "\nWrote {} ({} libraries).",
        project.file.display(),
        project.libraries.len()
    );
    for warning in &project.warnings {
        eprintln!("warning: {warning}");
    }
    Ok(())
}

#[derive(Subcommand)]
pub enum LibmanCommand {
    /// Print the project's libraries as a LibMan project file (define(...) lines).
    Export {
        /// Write it to this file instead of printing it.
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Start LibMan (libman or door on PATH) on the project's libraries.
    Open,
    /// Show what a LibMan project file defines: libraries, their folders, the PDK.
    Show { file: PathBuf },
}

pub fn libman(command: LibmanCommand, project: Option<Project>) -> Result<()> {
    let need = |project: Option<Project>| {
        project.ok_or_else(|| eyre::eyre!("not inside a project (no cdept.toml)"))
    };
    match command {
        LibmanCommand::Export { output } => {
            let text = libman::export(&need(project)?);
            match output {
                Some(file) => std::fs::write(&file, text)
                    .map_err(|e| eyre::eyre!("cannot write {}: {e}", file.display()))?,
                None => print!("{text}"),
            }
        }
        LibmanCommand::Open => {
            let file = libman::open(&need(project)?)?;
            eprintln!("LibMan started on {}", file.display());
        }
        LibmanCommand::Show { file } => {
            let found = libman::read(&file)?;
            for library in found.resolve() {
                println!("{}", library.name);
                for folder in &library.folders {
                    println!("  {}", folder.display());
                }
                for path in &library.unresolved {
                    println!("  ? {path}");
                }
            }
            if let Some((name, path)) = &found.pdk {
                println!("pdk {name}: {path}");
            }
            for (library, reference) in &found.attached {
                println!("attach {library} -> {reference}");
            }
            for problem in &found.problems {
                eprintln!("warning: {problem}");
            }
        }
    }
    Ok(())
}
