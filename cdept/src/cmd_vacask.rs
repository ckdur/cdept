//! `cdept vacask models` and `cdept openvaf compile` (docs/VACASK.md).

use std::path::PathBuf;

use clap::Subcommand;
use eyre::{Result, bail, eyre};
use libcdept::Project;
use libcdept::batch::openvaf;
use libcdept::vacask::models;

#[derive(Subcommand)]
pub enum VacaskCommand {
    /// Make the technology's VACASK models from its ngspice models and Verilog-A
    /// sources (VACASK's ng2vc and OpenVAF), record them as `[models.vacask]` in
    /// tech.toml and print what could and could not be done.
    ///
    /// Done on the first VACASK run anyway; this shows the result and `--force` makes
    /// it again. Files are cached in <project>/.cdept/osdi/<pdk>-<hash>/.
    Models {
        /// Make them again even when the cache has them.
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand)]
pub enum OpenvafCommand {
    /// Compile a Verilog-A file to an OSDI library with OpenVAF (`$CDEPT_OPENVAF`, else
    /// openvaf-r / openvaf on PATH).
    ///
    /// Without `--out` the result is cached in <project>/.cdept/osdi/<pdk>-<hash>/
    /// (the hash covers the sources and flags): an unchanged model is not compiled
    /// twice. The path of the OSDI file is printed.
    Compile {
        source: PathBuf,
        /// Write the OSDI file here instead of the cache (always compiles).
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Macro for the preprocessor: `NAME` or `NAME=VALUE`.
        #[arg(short = 'D', long = "define")]
        defines: Vec<String>,
        /// Folder searched for `include files.
        #[arg(short = 'I', long = "include")]
        includes: Vec<PathBuf>,
    },
}

pub fn vacask(command: VacaskCommand, project: &Project) -> Result<()> {
    let VacaskCommand::Models { force } = command;
    let tech = project
        .snapshot()
        .ok_or_else(|| eyre!("the project has no technology snapshot (cdept tech scan)"))?;
    let plan = models::Plan::new(project, tech)?;
    if force {
        let dir = plan.dir(project);
        if dir.is_dir() {
            std::fs::remove_dir_all(&dir)?;
        }
    }
    let g = models::generate(project, tech, &plan)?;
    models::record(project, &g)?;
    println!("technology: {}", g.pdk);
    println!("{}", libcdept::vacask::availability(project).line());
    println!("cache:      {}", plan.dir(project).display());
    println!(
        "converted:  {} file(s): {}",
        g.converted.len(),
        g.converted.join(", ")
    );
    println!(
        "compiled:   {} library(ies): {}",
        g.compiled.len(),
        g.compiled.join(", ")
    );
    println!("defines:    {} subcircuits and models", g.defined.len());
    let corners: Vec<&str> = g.set.corner_names();
    println!("corners:    {}", corners.join(", "));
    if g.notes.is_empty() {
        println!("nothing was left out");
    } else {
        println!("left out:");
        for n in &g.notes {
            println!("  {n}");
        }
    }
    Ok(())
}

pub fn openvaf(command: OpenvafCommand, project: Option<&Project>) -> Result<()> {
    let OpenvafCommand::Compile {
        source,
        out,
        defines,
        includes,
    } = command;
    let program = openvaf::ADAPTER.require()?;
    if !source.is_file() {
        bail!("{} is not a file", source.display());
    }
    let env = openvaf::ADAPTER
        .locate()
        .map(|p| libcdept::vacask::tool_env_for(&p))
        .unwrap_or_default();
    let mut c = openvaf::Compile {
        source: source.clone(),
        out: PathBuf::new(),
        defines,
        includes,
    };
    if let Some(out) = out {
        c.out = out;
        openvaf::compile(&program, &c, &env)?;
        println!("{}", c.out.display());
        return Ok(());
    }
    let project =
        project.ok_or_else(|| eyre!("not inside a project: give --out for the OSDI file"))?;
    let name = project
        .snapshot()
        .map(|t| t.pdk.name.clone())
        .unwrap_or_else(|| "project".into());
    let hash = openvaf::source_hash(&program, &c)?;
    let dir = openvaf::cache_dir(&project.root, &name, &hash);
    let r = openvaf::compile_cached(&program, &c, &dir, &env)?;
    println!("{}", r.osdi.display());
    eprintln!("{}", if r.compiled { "compiled" } else { "cached" });
    Ok(())
}
