//! `cdept lib set|show`: change and show library definitions, wherever they are
//! defined (the project file or one it includes). `add` and `remove` are in main.rs
//! and use the checks here.

use clap::Subcommand;
use eyre::{Result, bail, eyre};
use libcdept::project_edit::{CopyRule, LibraryChange, Origin};
use libcdept::{LibraryLayout, Project, formats};

#[derive(Subcommand)]
pub enum Command {
    /// Change a library definition: its folder, read-only, layout, description.
    ///
    /// The change is written to the file that defines the library (the project file
    /// or one it includes), keeping that file's comments. The description goes into
    /// the lib.toml of the library folder.
    Set {
        name: String,
        /// Another folder, written as given (relative to the defining file; $VAR is
        /// kept). It must exist.
        #[arg(long)]
        path: Option<String>,
        /// Reference library: CDePT will not modify it.
        #[arg(long, conflicts_with = "writable")]
        read_only: bool,
        /// Not a reference library any more.
        #[arg(long)]
        writable: bool,
        /// Folder layout: `cells`, `flat`, …, or `detect` to find it out from the
        /// folder each time the project is loaded.
        #[arg(long)]
        layout: Option<String>,
        #[arg(long)]
        description: Option<String>,
        /// Go on when the new folder gets in the way of another library.
        #[arg(long)]
        force: bool,
    },
    /// Show a library definition: where it is defined, what is written there, what
    /// CDePT makes of it.
    Show { name: String },
}

/// `detect`, or the name of a layout.
fn layout(text: &str) -> Result<Option<LibraryLayout>> {
    if text == "detect" {
        return Ok(None);
    }
    LibraryLayout::from_name(text)
        .map(Some)
        .ok_or_else(|| eyre!("'{text}' is not a library layout (cells, flat, …, or detect)"))
}

/// Before a library is defined in `path`: stop when it gets in the way of another
/// library, unless told to go on.
pub fn check_folder(project: &Project, name: &str, path: &str, force: bool) -> Result<()> {
    match project.folder_clash(name, path) {
        Some(clash) if force => {
            eprintln!("warning: {clash}");
            Ok(())
        }
        Some(clash) => Err(eyre!("{clash}\n(--force defines the library anyway)")),
        None => Ok(()),
    }
}

/// `cdept lib remove`: from the file that defines the library.
pub fn remove(project: &mut Project, name: &str) -> Result<()> {
    let file = project
        .library(name)
        .map(|l| l.defined_in.clone())
        .ok_or_else(|| eyre!("no library '{name}'"))?;
    project.remove_library(name)?;
    println!("removed {name} from {}", file.display());
    Ok(())
}

pub fn run(project: &mut Project, command: Command) -> Result<()> {
    match command {
        Command::Set {
            name,
            path,
            read_only,
            writable,
            layout: wanted,
            description,
            force,
        } => {
            let change = LibraryChange {
                path,
                read_only: match (read_only, writable) {
                    (true, _) => Some(true),
                    (_, true) => Some(false),
                    _ => None,
                },
                layout: wanted.as_deref().map(layout).transpose()?,
                description,
            };
            if change.is_empty() {
                bail!(
                    "nothing to change: give --path, --read-only, --writable, --layout or --description"
                );
            }
            if let Some(path) = &change.path {
                // The check reads paths relative to the project file.
                let origin = project.library_origin(&name);
                if matches!(origin, Some(Origin::Project)) {
                    check_folder(project, &name, path, force)?;
                }
            }
            project.set_library(&name, &change)?;
            show(project, &name)
        }
        Command::Show { name } => show(project, &name),
    }
}

fn show(project: &Project, name: &str) -> Result<()> {
    let library = project
        .library(name)
        .ok_or_else(|| eyre!("no library '{name}'"))?;
    let origin = project.library_origin(name).expect("the library exists");
    println!("{name}");
    println!("  defined in   {}", origin.describe(project));
    match project.library_definition(name) {
        Ok(definition) => {
            println!("  path         {}", definition.path);
            println!(
                "  layout       {}",
                match definition.layout {
                    Some(layout) => layout.name().to_string(),
                    None => format!("{} (detected from the folder)", library.layout.name()),
                }
            );
        }
        // A library the PDK brings has no definition to show.
        Err(_) => println!("  layout       {}", library.layout.name()),
    }
    println!("  folder       {}", library.path.display());
    println!(
        "  read-only    {}",
        if library.read_only { "yes" } else { "no" }
    );
    if let Some(description) = &library.description {
        println!("  description  {description}");
    }
    match &library.error {
        Some(error) => println!("  error        {error}"),
        None => {
            let views: usize = library.cells.iter().map(|c| c.entries.len()).sum();
            println!("  cells        {} ({views} views)", library.cells.len());
            let unknown = formats::unrecognised(library);
            if !unknown.is_empty() {
                println!("  not recognised  {} file(s):", unknown.len());
                for file in unknown.iter().take(10) {
                    let shown = file.strip_prefix(&library.path).unwrap_or(file);
                    println!("    {}", shown.display());
                }
                if unknown.len() > 10 {
                    println!("    … and {} more", unknown.len() - 10);
                }
            }
            let rule = CopyRule::of_library(&library.path);
            if rule != CopyRule::default() {
                println!("  copy leaves out  {}", rule.exclude.join(", "));
            }
        }
    }
    let suffixes = formats::Suffixes::read(&project.file);
    if !suffixes.is_empty() {
        let entries: Vec<String> = suffixes
            .entries()
            .iter()
            .map(|(suffix, format)| format!("{suffix} = {format}"))
            .collect();
        println!("  project suffixes  {}", entries.join(", "));
    }
    Ok(())
}
