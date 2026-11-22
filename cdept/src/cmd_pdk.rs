//! `cdept pdk …`: the PDKs installed on this machine and their versions.

use std::path::{Path, PathBuf};

use clap::Subcommand;
use eyre::{Result, bail, eyre};
use libcdept::Project;
use libcdept::tech::ciel;
use libcdept::tech::discover::{self, Found, Search};

#[derive(Subcommand)]
pub enum PdkCommand {
    /// List the PDKs found on this machine.
    ///
    /// Looked at, in this order: the project's PDK folder, $PDK_ROOT, the folders in
    /// $CDEPT_PDK_PATH, ~/.ciel, ~/.volare, /usr/share/pdk, /usr/local/share/pdk,
    /// ~/share/pdk. A PDK is a folder with libs.tech in it.
    Ls {
        /// Look in this folder too (may be given several times).
        #[arg(long = "root", value_name = "DIR")]
        roots: Vec<PathBuf>,
        /// Look only in the folders given with --root.
        #[arg(long)]
        only: bool,
        /// Print JSON.
        #[arg(long)]
        json: bool,
    },
    /// Show what a PDK says about itself: version, where it comes from, its tools.
    Info {
        /// A PDK name (`ihp-sg13g2`) or the path of a PDK folder. Default: the
        /// project's PDK.
        pdk: Option<String>,
        /// Look in this folder too (may be given several times).
        #[arg(long = "root", value_name = "DIR")]
        roots: Vec<PathBuf>,
        /// Print JSON.
        #[arg(long)]
        json: bool,
    },
    /// List the PDK versions ciel (or volare) has installed.
    Versions {
        /// The PDK family (`sky130`, `gf180mcu`, `ihp-sg13g2`). Default: all.
        family: Option<String>,
        /// The PDK folder ciel manages. Default: every folder `cdept pdk ls` looks at.
        #[arg(long = "root", value_name = "DIR")]
        root: Option<PathBuf>,
        /// Also list the versions that can be fetched (needs the ciel program).
        #[arg(long)]
        remote: bool,
        /// Print JSON.
        #[arg(long)]
        json: bool,
    },
    /// Put an installed PDK version in use (what `ciel enable` does).
    ///
    /// The PDKs of that version are linked into the PDK folder. A version that is not
    /// installed is fetched with the ciel program when it is on PATH.
    Enable {
        /// The PDK family (`sky130`, `gf180mcu`, `ihp-sg13g2`).
        family: String,
        /// The version (an open_pdks commit), or the start of it.
        version: String,
        /// The PDK folder ciel manages. Default: the first folder that has versions
        /// of this family.
        #[arg(long = "root", value_name = "DIR")]
        root: Option<PathBuf>,
    },
}

/// Where to look: the project's PDK folder first when inside a project.
fn search(project: Option<&Project>, roots: Vec<PathBuf>, only: bool) -> Search {
    let search = Search::new().given(roots);
    if only {
        return search.only();
    }
    search.project(
        project
            .and_then(|p| p.pdk.root.as_ref())
            .map(|root| PathBuf::from(&root.value)),
    )
}

fn short(hash: &str) -> &str {
    // Commits are shown by their first 12 digits; anything else in full.
    if hash.len() == 40 && hash.chars().all(|c| c.is_ascii_hexdigit()) {
        &hash[..12]
    } else {
        hash
    }
}

fn info_json(found: &Found) -> serde_json::Value {
    let info = found.info();
    serde_json::json!({
        "name": found.pdk.name,
        "family": found.pdk.family.name(),
        "root": found.pdk.root,
        "folder": found.real,
        "found_through": found.origin.name(),
        "version": info.version,
        "commit": info.commit,
        "branch": info.branch,
        "source": info.source,
        "description": info.description,
        "tools": info.tools,
        "tested_with": info.tested_with,
    })
}

fn print_info(found: &Found) {
    let info = found.info();
    println!("{}", found.pdk.name);
    println!("  family       {}", found.pdk.family.name());
    if let Some(text) = &info.description {
        println!("  description  {text}");
    }
    println!("  PDK_ROOT     {}", found.pdk.root.display());
    if found.real != found.pdk.dir() {
        println!("  folder       {}", found.real.display());
    }
    println!("  found by     {}", found.origin.name());
    if let Some(text) = &info.source {
        println!("  installed by {text}");
    }
    if let Some(text) = &info.version {
        println!("  version      {text}");
    }
    if let Some(text) = &info.branch {
        println!("  branch       {text}");
    }
    if let Some(text) = &info.commit {
        println!("  commit       {text}");
    }
    if !info.tools.is_empty() {
        println!("  tools        {}", info.tools.join(", "));
    }
    if !info.tested_with.is_empty() {
        let tools: Vec<String> = info
            .tested_with
            .iter()
            .map(|(tool, version)| format!("{tool} {version}"))
            .collect();
        println!("  names tools  {}", tools.join(", "));
    }
}

fn version_json(root: &Path, version: &ciel::Version) -> serde_json::Value {
    serde_json::json!({
        "root": root,
        "store": version.store,
        "family": version.family,
        "version": version.version,
        "path": version.path,
        "pdks": version.pdks,
        "current": version.current,
    })
}

pub fn run(command: PdkCommand, project: Option<&Project>) -> Result<()> {
    match command {
        PdkCommand::Ls { roots, only, json } => {
            let search = search(project, roots, only);
            let found = discover::discover(&search);
            if json {
                let all: Vec<_> = found.iter().map(info_json).collect();
                println!("{}", serde_json::to_string_pretty(&all)?);
                return Ok(());
            }
            if found.is_empty() {
                println!("no PDK found in:");
                for root in search.roots() {
                    println!("  {} ({})", root.path.display(), root.origin.name());
                }
                println!(
                    "set PDK_ROOT (or {}), or name a folder with --root",
                    discover::PATH_VAR
                );
                return Ok(());
            }
            let in_use = project.and_then(|p| p.pdk.name.as_ref()).map(|n| &n.value);
            let rows: Vec<[String; 4]> = found
                .iter()
                .map(|f| {
                    let info = f.info();
                    let version = match (&info.version, &info.commit) {
                        (Some(v), Some(c)) => format!("{v} ({})", short(c)),
                        (Some(v), None) => v.clone(),
                        (None, Some(c)) => short(c).to_string(),
                        (None, None) => "-".into(),
                    };
                    [
                        f.pdk.name.clone(),
                        version,
                        info.source.unwrap_or_else(|| "-".into()),
                        f.pdk.root.display().to_string(),
                    ]
                })
                .collect();
            let width = |column: usize| rows.iter().map(|row| row[column].len()).max().unwrap_or(0);
            let (names, versions, sources) = (width(0), width(1), width(2));
            for row in &rows {
                let mark = if in_use == Some(&row[0]) { "*" } else { " " };
                println!(
                    "{mark} {:names$}  {:versions$}  {:sources$}  {}",
                    row[0], row[1], row[2], row[3],
                );
            }
            if in_use.is_some() {
                println!("(* the project's PDK)");
            }
            Ok(())
        }
        PdkCommand::Info { pdk, roots, json } => {
            let search = search(project, roots, false);
            let name = match pdk {
                Some(name) => name,
                None => project
                    .and_then(|p| p.pdk.name.as_ref())
                    .map(|n| n.value.clone())
                    .ok_or_else(|| {
                        eyre!("which PDK? name one, or run inside a project with a technology")
                    })?,
            };
            // `$PDK` may list several PDKs: show each.
            let names = if name.contains('/') {
                vec![name]
            } else {
                discover::names(&name)
            };
            let mut all = Vec::new();
            for name in &names {
                let pdk = discover::find(name, &search).ok_or_else(|| {
                    eyre!(
                        "no PDK '{name}' found; `cdept pdk ls` lists what there is, --root DIR \
                         looks in another folder"
                    )
                })?;
                let real = std::fs::canonicalize(pdk.dir()).unwrap_or_else(|_| pdk.dir());
                let origin = search
                    .roots()
                    .into_iter()
                    .find(|root| root.path == pdk.root)
                    .map_or(discover::Origin::Given, |root| root.origin);
                all.push(Found { pdk, origin, real });
            }
            if json {
                let value: Vec<_> = all.iter().map(info_json).collect();
                println!("{}", serde_json::to_string_pretty(&value)?);
            } else {
                for found in &all {
                    print_info(found);
                }
            }
            Ok(())
        }
        PdkCommand::Versions {
            family,
            root,
            remote,
            json,
        } => {
            let roots: Vec<PathBuf> = match root {
                Some(root) => vec![root],
                None => search(project, Vec::new(), false)
                    .roots()
                    .into_iter()
                    .map(|r| r.path)
                    .collect(),
            };
            let mut listed = Vec::new();
            for root in &roots {
                for version in ciel::installed(root, family.as_deref()) {
                    listed.push((root.clone(), version));
                }
            }
            let program = ciel::Program::find();
            let mut fetchable: Vec<(PathBuf, String, Vec<String>)> = Vec::new();
            if remote {
                let Some(program) = &program else {
                    bail!(
                        "listing the versions that can be fetched needs the ciel program on PATH"
                    );
                };
                let families: Vec<String> = match &family {
                    Some(family) => vec![family.clone()],
                    None => {
                        let mut all: Vec<String> =
                            listed.iter().map(|(_, v)| v.family.clone()).collect();
                        all.sort();
                        all.dedup();
                        all
                    }
                };
                let root = roots
                    .first()
                    .ok_or_else(|| eyre!("no PDK folder; name one with --root"))?;
                for family in families {
                    let versions = program.remote(root, &family)?;
                    fetchable.push((root.clone(), family, versions));
                }
            }
            if json {
                let value = serde_json::json!({
                    "installed": listed.iter().map(|(root, v)| version_json(root, v)).collect::<Vec<_>>(),
                    "remote": fetchable.iter().map(|(root, family, versions)| serde_json::json!({
                        "root": root, "family": family, "versions": versions,
                    })).collect::<Vec<_>>(),
                    "program": program.as_ref().map(|p| &p.path),
                });
                println!("{}", serde_json::to_string_pretty(&value)?);
                return Ok(());
            }
            if listed.is_empty() {
                println!("no version installed by ciel in:");
                for root in &roots {
                    println!("  {}", root.display());
                }
            }
            for (root, version) in &listed {
                println!(
                    "{} {:10}  {}  {:18}  {}",
                    if version.current { "*" } else { " " },
                    version.family,
                    version.version,
                    version.pdks.join(" "),
                    root.display()
                );
            }
            if listed.iter().any(|(_, v)| v.current) {
                println!("(* in use)");
            }
            for (root, family, versions) in &fetchable {
                println!("can be fetched for {family} into {}:", root.display());
                for version in versions {
                    println!("  {version}");
                }
            }
            if program.is_none() {
                println!(
                    "the ciel program is not on PATH: only installed versions are listed \
                     (pip install ciel)"
                );
            }
            // The versions LibreLane wants for this project.
            if let Some(project) = project {
                for root in &roots {
                    for wrong in ciel::pin_mismatches(&project.root, root) {
                        println!("note: {wrong} under {}", root.display());
                    }
                }
            }
            Ok(())
        }
        PdkCommand::Enable {
            family,
            version,
            root,
        } => {
            let roots: Vec<PathBuf> = match root {
                Some(root) => vec![root],
                None => search(project, Vec::new(), false)
                    .roots()
                    .into_iter()
                    .map(|r| r.path)
                    .collect(),
            };
            // The first folder that has this version; else the first with the family.
            let with_version = roots
                .iter()
                .find(|root| ciel::resolve(root, &family, &version).is_ok());
            if let Some(root) = with_version {
                let pdks = ciel::enable(root, &family, &version)?;
                let now = ciel::current(root, &family).unwrap_or(version);
                println!(
                    "{family} {now} is in use under {}: {}",
                    root.display(),
                    pdks.join(", ")
                );
                println!(
                    "projects that keep a technology snapshot of it: run `cdept tech refresh`"
                );
                return Ok(());
            }
            let root = roots
                .iter()
                .find(|root| ciel::families(root).contains(&family))
                .or(roots.first())
                .ok_or_else(|| eyre!("no PDK folder; name one with --root"))?;
            match ciel::Program::find() {
                Some(program) => {
                    println!("{}", program.enable_line(root, &family, &version));
                    let output = program.enable(root, &family, &version)?;
                    print!("{output}");
                    Ok(())
                }
                None => {
                    // Say what is there and what would fetch the rest.
                    let error = ciel::resolve(root, &family, &version)
                        .err()
                        .map(|e| e.to_string())
                        .unwrap_or_default();
                    Err(eyre!(
                        "{error}\nfetching a version needs the ciel program (pip install ciel), \
                         then: ciel enable --pdk-root {} --pdk-family {family} {version}",
                        root.display()
                    ))
                }
            }
        }
    }
}
