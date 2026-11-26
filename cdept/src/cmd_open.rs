//! `cdept open <library> <cell> [view] [--tool T] [--list]`: open a view in a tool's
//! own window (docs/tools.md, "Opening views"). The embedded editors are the GUI's.

use clap::Args;
use eyre::{Result, eyre};
use libcdept::Project;
use libcdept::launch::{self, Launched, Launcher};

#[derive(Args)]
pub struct OpenArgs {
    pub library: String,
    pub cell: String,
    /// The view (default: the first that has a tool).
    pub view: Option<String>,
    /// The tool: xschem, klayout, magic, qucs, editor (default: the first external
    /// tool of the view's format).
    #[arg(long)]
    pub tool: Option<String>,
    /// Only list the tools that can open the view.
    #[arg(long)]
    pub list: bool,
}

pub fn run(args: OpenArgs, project: &Project) -> Result<()> {
    let lib = project
        .library(&args.library)
        .ok_or_else(|| eyre!("no library '{}'", args.library))?;
    let cell = lib
        .cell(&args.cell)
        .ok_or_else(|| eyre!("library '{}' has no cell '{}'", args.library, args.cell))?;
    let entry = match &args.view {
        Some(v) => cell
            .entry(v)
            .ok_or_else(|| eyre!("cell '{}' has no view '{v}'", args.cell))?,
        None => cell
            .entries
            .iter()
            .find(|e| !launch::choices_for(e).is_empty())
            .ok_or_else(|| eyre!("no view of '{}' has a tool", args.cell))?,
    };
    let choices = launch::choices_for(entry);
    if args.list {
        for c in &choices {
            let place = if c.embedded { "in the GUI" } else { "window" };
            println!("{:<15} {} ({place})", c.id, c.label);
        }
        return Ok(());
    }
    let choice = match &args.tool {
        Some(t) => {
            let id = format!("{}-window", t.to_lowercase().replace("-s", ""));
            choices
                .iter()
                .find(|c| c.id == id)
                .or_else(|| choices.iter().find(|c| c.id == t.as_str()))
                .copied()
                .ok_or_else(|| eyre!("{t} cannot open view '{}'", entry.name))?
        }
        None => choices
            .iter()
            .find(|c| !c.embedded)
            .copied()
            .ok_or_else(|| eyre!("view '{}' opens only in the GUI", entry.name))?,
    };
    if choice.embedded {
        return Err(eyre!("{} opens in a tab of `cdept gui`", choice.label));
    }
    let launcher = Launcher::new();
    match launcher.open(Some(project), choice, entry.main_file.as_deref())? {
        Launched::Started { pid, log } => {
            println!("{}: started (process {pid}), log {}", choice.label, log.display())
        }
        Launched::Reused(text) => println!("{text}"),
    }
    Ok(())
}
