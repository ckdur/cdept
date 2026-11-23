//! `cdept tech …` and the PDK part of `cdept init`: the technology snapshot of a
//! project (`tech.toml` and `tech.catalog.json`, docs/TECH.md).

use std::path::{Path, PathBuf};

use clap::Subcommand;
use eyre::{Result, bail, eyre};
use libcdept::Project;
use libcdept::tech::discover::{self, Search};
use libcdept::tech::{self, Catalog, PdkDir, ScanContext, Tech, file, report};

#[derive(Subcommand)]
pub enum TechCommand {
    /// Scan a PDK and write the project's technology snapshot.
    ///
    /// Reads the PDK's files (KLayout technology, layers and decks, xschemrc and
    /// symbols, magic and netgen set-up, model files and their corners, the libraries
    /// in libs.ref) and writes tech.toml and tech.catalog.json next to cdept.toml. The
    /// PDK becomes the project's (`technology`); where it is on this machine goes
    /// into cdept.local.toml.
    Scan {
        /// A PDK name (`ihp-sg13g2`) or the path of a PDK folder. Default: the
        /// project's PDK.
        pdk: Option<String>,
        /// The folder PDKs are installed in. Default: the project's, $PDK_ROOT and
        /// the usual places (see `cdept pdk ls`).
        #[arg(long, value_name = "DIR")]
        pdk_root: Option<PathBuf>,
        /// Read files only: do not start KLayout (PCells) or xschem (xschemrc).
        #[arg(long)]
        files_only: bool,
        /// Print tech.toml instead of writing anything (needs no project).
        #[arg(long)]
        print: bool,
        /// Replace an existing tech.toml without asking (scripts, no terminal).
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Show the snapshot: a summary, or one section.
    Show {
        /// pdk, klayout, magic, netgen, xschem, models, devices or refs.
        section: Option<String>,
        /// The section's part of the catalog (the long lists) instead.
        #[arg(long)]
        catalog: bool,
        /// Print JSON.
        #[arg(long)]
        json: bool,
    },
    /// Scan the project's PDK again, say what changed, then replace the snapshot.
    ///
    /// tech.toml is replaced as a whole: what was edited by hand in it is lost (the
    /// old file is kept as tech.toml.bak). The differences are shown and, on a
    /// terminal, confirmed; without a terminal give --yes.
    Refresh {
        /// Read files only: do not start KLayout (PCells) or xschem (xschemrc).
        #[arg(long)]
        files_only: bool,
        /// Only say what would change.
        #[arg(long)]
        dry_run: bool,
        /// Replace tech.toml without asking (scripts, no terminal).
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// What the scan could not find or had to guess, and whether the snapshot is
    /// still the PDK's.
    Notes,
    /// Print a draft device map made from the scan: which KLayout PCell each xschem
    /// symbol of the PDK becomes when a layout is generated from a schematic
    /// (docs/LINKING.md). For PDKs CDePT ships no map for. The matches are guesses:
    /// check them, save the file in the project and name it in `[klayout] devices`.
    Devices {
        /// Write the map to this file instead of printing it.
        #[arg(short, long, value_name = "FILE")]
        output: Option<PathBuf>,
    },
}

/// The PDK called `name` (or at the path `name`), looked for in `pdk_root`, then in
/// the project's PDK folder and the usual places.
pub fn find_pdk(name: &str, pdk_root: Option<&Path>, project: Option<&Project>) -> Result<PdkDir> {
    let project_root = project.and_then(|p| p.pdk.var("PDK_ROOT").map(PathBuf::from));
    let mut search = Search::new().project(project_root);
    if let Some(root) = pdk_root {
        // A folder named on the spot is the only place meant.
        search = search.given([root.to_path_buf()]).only();
    }
    discover::find(name, &search).ok_or_else(|| {
        let places: Vec<String> = search
            .roots()
            .iter()
            .map(|root| root.path.display().to_string())
            .collect();
        eyre!(
            "PDK '{name}' not found (looked in: {}). `cdept pdk ls` lists the PDKs on this \
             machine; --pdk-root names the folder that holds it",
            if places.is_empty() {
                "nowhere: no PDK folder is known".to_string()
            } else {
                places.join(", ")
            }
        )
    })
}

/// Scan `pdk`, saying so on stderr (the helpers take a few seconds).
pub fn scan(pdk: &PdkDir, helpers: bool, project: Option<&Project>) -> (Tech, Catalog) {
    eprintln!("scanning {} …", pdk.dir().display());
    let mut ctx = ScanContext::new(pdk.clone());
    ctx.helpers = helpers;
    if let Some(project) = project {
        // The user's own variables, not the project's PDK (the scan may be of another).
        ctx.env.extend(
            project
                .local_env
                .iter()
                .map(|(name, value)| (name.into(), value.into())),
        );
    }
    tech::scan(&ctx)
}

/// The differences between the file now and a fresh scan. They are the PDK's changes
/// and the hand edits together: the file does not say which is which.
fn print_changes(changes: &[String]) {
    if changes.is_empty() {
        println!("nothing differs from the current {}", tech::TECH_FILE);
        return;
    }
    println!(
        "differences between the current {} and the PDK now (changes made by hand \
         are among them, `-` lines are what is lost):",
        tech::TECH_FILE
    );
    for line in changes {
        println!("{line}");
    }
}

/// Say that the file is replaced as a whole and ask (`Replace tech.toml? [y/N]`);
/// `yes` agrees beforehand. Without a terminal and without `yes` it refuses.
fn confirm_replace(yes: bool) -> Result<()> {
    use std::io::{BufRead, IsTerminal, Write};
    println!("warning: {}", file::replace_warning());
    if yes {
        return Ok(());
    }
    if !std::io::stdin().is_terminal() {
        bail!(
            "{} is not replaced: there is no terminal to ask; give --yes to replace it",
            tech::TECH_FILE
        );
    }
    print!("Replace {}? [y/N] ", tech::TECH_FILE);
    std::io::stdout().flush().ok();
    let mut answer = String::new();
    std::io::stdin().lock().read_line(&mut answer)?;
    if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        bail!("{} not replaced", tech::TECH_FILE);
    }
    Ok(())
}

/// What a scan found, for the terminal: the summary, then the notes.
pub fn print_found(tech: &Tech, catalog: &Catalog) {
    print!("{}", report::summary_text(tech, catalog));
    if !tech.notes.is_empty() {
        println!("notes:");
        for note in &tech.notes {
            println!("  - {note}");
        }
    }
}

/// `cdept init`: create the project; with a PDK named, scan it and write the snapshot.
pub fn init(
    dir: &Path,
    name: Option<&str>,
    technology: Option<&str>,
    pdk_root: Option<&str>,
    no_scan: bool,
    files_only: bool,
) -> Result<()> {
    let technology = technology.map(str::trim).filter(|t| !t.is_empty());
    let Some(technology) = technology.filter(|_| !no_scan) else {
        let project = Project::create_with(dir, name, technology, pdk_root)?;
        println!("created {}", project.file.display());
        return Ok(());
    };
    if dir.join(libcdept::PROJECT_FILE).exists() {
        bail!(
            "{} already exists",
            dir.join(libcdept::PROJECT_FILE).display()
        );
    }
    let root = pdk_root.map(|root| std::path::absolute(root).unwrap_or_else(|_| root.into()));
    match find_pdk(technology, root.as_deref(), None) {
        Ok(pdk) => {
            let (tech, catalog) = scan(&pdk, !files_only, None);
            let project = Project::create_for_pdk(dir, name, &pdk, &tech, &catalog)?;
            println!("created {}", project.file.display());
            println!(
                "PDK {} in {} ({})",
                pdk.name,
                pdk.root.display(),
                project
                    .pdk
                    .root
                    .as_ref()
                    .map_or("not set", |r| r.source.name())
            );
            print_found(&tech, &catalog);
            warnings(&project);
            Ok(())
        }
        Err(not_found) => {
            // The project is still made: the PDK may be installed later.
            let name_only = if technology.contains('/') {
                Path::new(technology)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
            } else {
                Some(technology.to_string())
            };
            let project = Project::create_with(dir, name, name_only.as_deref(), pdk_root)?;
            println!("created {}", project.file.display());
            eprintln!("warning: {not_found}");
            eprintln!(
                "no technology snapshot written: run `cdept tech scan` once the PDK is there"
            );
            Ok(())
        }
    }
}

/// The lines of `cdept info` about the technology: PDKs on offer, the snapshot, the
/// user's own variables.
pub fn info(project: &Project) {
    if project.pdk.available.len() > 1 {
        println!("PDKs offered {}  ($PDK)", project.pdk.available.join(", "));
    }
    match &project.tech {
        Some(tech) if project.tech_in_use => println!(
            "snapshot     {}  (PDK {}{})",
            tech::TECH_FILE,
            tech.pdk.name,
            match project.snapshot_stale() {
                Some(why) => format!("; out of date: {why}"),
                None => String::new(),
            }
        ),
        Some(tech) => println!(
            "snapshot     {}  (not used: made for PDK {})",
            tech::TECH_FILE,
            tech.pdk.name
        ),
        None => println!("snapshot     none  (`cdept tech scan` makes one)"),
    }
    for (name, value) in &project.local_env {
        println!("env          {name}={value}  (cdept.local.toml)");
    }
}

fn warnings(project: &Project) {
    for warning in &project.warnings {
        eprintln!("warning: {warning}");
    }
}

/// The project's snapshot, or what to do to get one.
fn snapshot(project: &Project) -> Result<&Tech> {
    project.tech.as_ref().ok_or_else(|| {
        eyre!(
            "the project has no technology snapshot ({}): run `cdept tech scan [PDK]`",
            file::tech_file(&project.root).display()
        )
    })
}

pub fn run(command: TechCommand, project: Option<Project>) -> Result<()> {
    let need = |project: Option<Project>| {
        project.ok_or_else(|| eyre!("no cdept.toml found: run inside a project or name it with -p"))
    };
    match command {
        TechCommand::Scan {
            pdk,
            pdk_root,
            files_only,
            print,
            yes,
        } => {
            let found = match (&pdk, &project) {
                (Some(name), _) => find_pdk(name, pdk_root.as_deref(), project.as_ref())?,
                (None, Some(project)) => match (&pdk_root, project.pdk.var("PDK")) {
                    (Some(_), Some(name)) => find_pdk(name, pdk_root.as_deref(), None)?,
                    _ => project
                        .pdk_dir()
                        .filter(|p| p.dir().is_dir())
                        .ok_or_else(|| {
                            eyre!(
                                "the project names no PDK that is on this machine: give one, \
                             e.g. `cdept tech scan ihp-sg13g2` (`cdept pdk ls` lists them)"
                            )
                        })?,
                },
                (None, None) => {
                    bail!("which PDK? e.g. `cdept tech scan ihp-sg13g2 --print`");
                }
            };
            let (tech, catalog) = scan(&found, !files_only, project.as_ref());
            if print {
                print!("{}", file::tech_text(&tech)?);
                return Ok(());
            }
            let mut project = need(project)?;
            if file::exists(&project.root) {
                let old_catalog = project.catalog()?.unwrap_or_default();
                let changes = match &project.tech {
                    Some(old) => report::changes((old, &old_catalog), (&tech, &catalog)),
                    None => Vec::new(),
                };
                print_changes(&changes);
                confirm_replace(yes)?;
            }
            project.set_technology(&found, &tech, &catalog)?;
            println!(
                "wrote {} and {}",
                file::tech_file(&project.root).display(),
                tech::CATALOG_FILE
            );
            print_found(&tech, &catalog);
            warnings(&project);
            Ok(())
        }
        TechCommand::Show {
            section,
            catalog,
            json,
        } => {
            let project = need(project)?;
            let tech = snapshot(&project)?;
            let listed = project.catalog()?.unwrap_or_default();
            let Some(name) = section else {
                if json {
                    let whole = if catalog {
                        serde_json::to_string_pretty(&listed)?
                    } else {
                        serde_json::to_string_pretty(tech)?
                    };
                    println!("{whole}");
                } else {
                    print!("{}", report::summary_text(tech, &listed));
                    if let Some(why) = project.snapshot_stale() {
                        println!("out of date: {why} (`cdept tech refresh`)");
                    }
                }
                return Ok(());
            };
            if !report::SECTIONS.contains(&name.as_str()) {
                bail!(
                    "no section '{name}': one of {}",
                    report::SECTIONS.join(", ")
                );
            }
            let value = if catalog {
                report::section(&listed, &name)
            } else {
                report::section(tech, &name)
            }
            .unwrap_or(serde_json::Value::Null);
            if json || catalog {
                println!("{}", serde_json::to_string_pretty(&value)?);
            } else {
                // As it is written in tech.toml.
                match file::section_text(tech, &name)? {
                    Some(text) => print!("{text}"),
                    None => println!("[{name}]\n# nothing found"),
                }
            }
            Ok(())
        }
        TechCommand::Refresh {
            files_only,
            dry_run,
            yes,
        } => {
            let mut project = need(project)?;
            let pdk = project
                .pdk_dir()
                .filter(|p| p.dir().is_dir())
                .ok_or_else(|| {
                    eyre!(
                        "the project's PDK is not on this machine ($PDK_ROOT = {}, $PDK = {}): \
                     set `pdk_root` in cdept.local.toml",
                        project.pdk.var("PDK_ROOT").unwrap_or("not set"),
                        project.pdk.var("PDK").unwrap_or("not set")
                    )
                })?;
            let (tech, catalog) = scan(&pdk, !files_only, Some(&project));
            let old_catalog = project.catalog()?.unwrap_or_default();
            let changes = match &project.tech {
                Some(old) => report::changes((old, &old_catalog), (&tech, &catalog)),
                None => vec!["+ the project had no snapshot".into()],
            };
            print_changes(&changes);
            if dry_run {
                return Ok(());
            }
            if file::exists(&project.root) {
                confirm_replace(yes)?;
            }
            // Written also when nothing changed: the files may have been edited.
            project.set_technology(&pdk, &tech, &catalog)?;
            println!("wrote {}", file::tech_file(&project.root).display());
            warnings(&project);
            Ok(())
        }
        TechCommand::Devices { output } => {
            let project = need(project)?;
            let tech = snapshot(&project)?;
            let catalog = project.catalog()?.ok_or_else(|| {
                eyre!(
                    "the project has no {}: run `cdept tech refresh`",
                    tech::CATALOG_FILE
                )
            })?;
            let text = tech::devices::draft_map(tech, &catalog);
            match output {
                Some(file) => {
                    if file.exists() {
                        bail!("{} already exists", file.display());
                    }
                    std::fs::write(&file, text)
                        .map_err(|e| eyre!("cannot write {}: {e}", file.display()))?;
                    println!(
                        "wrote {}; check it, then name it in cdept.toml: [klayout] devices = \"{}\"",
                        file.display(),
                        project.portable_path(&std::path::absolute(&file).unwrap_or(file.clone()))
                    );
                }
                None => print!("{text}"),
            }
            Ok(())
        }
        TechCommand::Notes => {
            let project = need(project)?;
            let tech = snapshot(&project)?;
            if tech.notes.is_empty() {
                println!("the scan of {} left no notes", tech.pdk.name);
            }
            for note in &tech.notes {
                println!("- {note}");
            }
            if let Some(why) = project.snapshot_stale() {
                println!("out of date: {why} (`cdept tech refresh`)");
            }
            warnings(&project);
            Ok(())
        }
    }
}
