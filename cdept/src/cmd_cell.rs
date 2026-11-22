//! `cdept cell …`, `cdept view …`, `cdept lib rename`, `cdept hier`, `cdept used-by`:
//! cells and views from the command line, with what refers to them following
//! (`libcdept::refactor`), and the hierarchy and the users of a cell read from the
//! files (`libcdept::hierarchy`). No tool is started.

use clap::{Args, Subcommand, ValueEnum};
use eyre::{Result, bail, eyre};
use libcdept::hierarchy::{Index, Reference, TreeKind};
use libcdept::refactor::{self, Operation, Options, Plan};
use libcdept::trash::Removal;
use libcdept::{Project, Template};

/// What a rename, copy or move does besides the files themselves.
#[derive(Args, Clone, Copy)]
pub struct Follow {
    /// Say what would be done (files moved, files rewritten) and change nothing.
    #[arg(long, short = 'n')]
    dry_run: bool,
    /// Move or copy the files only: leave every reference to them as it is.
    #[arg(long)]
    keep_references: bool,
}

/// A kind of view that can be made from scratch.
#[derive(Clone, Copy, ValueEnum)]
pub enum NewView {
    /// xschem schematic
    Schematic,
    /// xschem symbol
    Symbol,
    /// GDSII layout
    Layout,
    /// SPICE netlist
    Netlist,
    /// Verilog
    Hdl,
    /// KLayout PCell in Python
    Pcell,
    /// Magic layout
    Magic,
}

impl NewView {
    fn template(self) -> Template {
        match self {
            NewView::Schematic => Template::XschemSchematic,
            NewView::Symbol => Template::XschemSymbol,
            NewView::Layout => Template::GdsLayout,
            NewView::Netlist => Template::SpiceNetlist,
            NewView::Hdl => Template::Verilog,
            NewView::Pcell => Template::KlayoutPcell,
            NewView::Magic => Template::MagicLayout,
        }
    }
}

#[derive(Subcommand)]
pub enum CellCommand {
    /// Create a cell, empty or with a first view.
    New {
        library: String,
        cell: String,
        /// The first view of the cell. A flat library has no empty cells: give one.
        #[arg(long, value_enum)]
        view: Option<NewView>,
    },
    /// Copy a cell, into the same library or another one.
    ///
    /// The copy's own files point at each other (its symbol at its schematic, its
    /// sidecars name it); what it places stays the original, unless the cells below
    /// are copied too.
    Copy {
        library: String,
        cell: String,
        /// Name of the copy.
        new: String,
        /// The library the copy goes into. Default: the same library.
        #[arg(long, value_name = "LIBRARY")]
        to: Option<String>,
        /// Also copy the cells of the same library it is built from, under their
        /// names (needs another library), and make the copies use each other.
        #[arg(long)]
        with_hierarchy: bool,
        #[command(flatten)]
        follow: Follow,
    },
    /// Rename a cell. The schematics, layouts, netlists and HDL views that use it
    /// follow.
    Rename {
        library: String,
        cell: String,
        new: String,
        #[command(flatten)]
        follow: Follow,
    },
    /// Move a cell into another library. What uses it follows.
    Move {
        library: String,
        cell: String,
        to_library: String,
        /// Another name in that library. Default: the same name.
        #[arg(long)]
        name: Option<String>,
        #[command(flatten)]
        follow: Follow,
    },
    /// Delete a cell with all its views (to the trash). Lists what uses it first.
    Rm {
        library: String,
        cell: String,
        /// Delete for good instead of moving to the trash.
        #[arg(long)]
        permanent: bool,
        /// Say what would be deleted and who uses it, and change nothing.
        #[arg(long, short = 'n')]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
pub enum ViewCommand {
    /// Create a view of a cell from a template (the cell is created if needed).
    New {
        library: String,
        cell: String,
        #[arg(value_enum)]
        kind: NewView,
    },
    /// Copy a view, into the same cell under another name or into another cell.
    Copy {
        library: String,
        cell: String,
        view: String,
        /// Name of the copy. Default: the same name.
        #[arg(long)]
        name: Option<String>,
        /// The library the copy goes into. Default: the same library.
        #[arg(long, value_name = "LIBRARY")]
        to_library: Option<String>,
        /// The cell the copy goes into. Default: the same cell.
        #[arg(long, value_name = "CELL")]
        to_cell: Option<String>,
        #[command(flatten)]
        follow: Follow,
    },
    /// Rename a view (a view folder of a `cells` library). What names it follows:
    /// the symbol's schematic= attribute, the layouts that place or are linked with it.
    Rename {
        library: String,
        cell: String,
        view: String,
        new: String,
        #[command(flatten)]
        follow: Follow,
    },
    /// Delete a view (to the trash). Lists what uses it first.
    Rm {
        library: String,
        cell: String,
        view: String,
        /// Delete for good instead of moving to the trash.
        #[arg(long)]
        permanent: bool,
        /// Say what would be deleted and who uses it, and change nothing.
        #[arg(long, short = 'n')]
        dry_run: bool,
    },
}

/// `cdept lib rename`.
#[derive(Args)]
pub struct LibRename {
    library: String,
    new: String,
    /// Also rename the library folder. xschem names a library by its folder, so the
    /// schematics that use its cells are rewritten.
    #[arg(long)]
    folder: bool,
    #[command(flatten)]
    follow: Follow,
}

/// `cdept hier`.
#[derive(Args)]
pub struct HierArgs {
    library: String,
    cell: String,
    /// Which hierarchy: through the schematics, the layouts, the netlists or the HDL
    /// views. Default: the first of these the cell has a view of.
    #[arg(long, value_enum)]
    view: Option<HierView>,
    /// Also list what has nothing below: devices, pins, PCells.
    #[arg(long)]
    all: bool,
    /// Print JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum HierView {
    Schematic,
    Layout,
    Netlist,
    Hdl,
}

impl HierView {
    fn kind(self) -> TreeKind {
        match self {
            HierView::Schematic => TreeKind::Schematic,
            HierView::Layout => TreeKind::Layout,
            HierView::Netlist => TreeKind::Netlist,
            HierView::Hdl => TreeKind::Hdl,
        }
    }
}

/// `cdept used-by`.
#[derive(Args)]
pub struct UsedByArgs {
    library: String,
    cell: String,
    /// Only what uses this view of the cell.
    #[arg(long)]
    view: Option<String>,
    /// Print JSON.
    #[arg(long)]
    json: bool,
}

fn options(follow: Follow) -> Options {
    Options {
        keep_references: follow.keep_references,
        ..Options::default()
    }
}

fn print_plan(project: &Project, plan: &Plan) {
    for line in plan.lines(&project.root) {
        println!("{line}");
    }
}

/// Plan, then show (dry run) or apply.
fn refactor(project: &Project, operation: Operation, follow: Follow) -> Result<()> {
    let plan = refactor::plan(project, &operation, &options(follow))?;
    if follow.dry_run {
        print_plan(project, &plan);
        println!("(dry run: nothing was changed)");
        return Ok(());
    }
    let outcome = refactor::apply(project, &plan)?;
    for line in &outcome.lines {
        println!("{line}");
    }
    for note in &plan.notes {
        println!("note: {note}");
    }
    for problem in &outcome.problems {
        eprintln!("warning: {problem}");
    }
    Ok(())
}

/// Delete through the plan (who uses it), then to the trash or for good.
fn remove(
    project: &Project,
    library: &str,
    cell: &str,
    view: Option<&str>,
    permanent: bool,
    dry_run: bool,
) -> Result<()> {
    let operation = match view {
        Some(view) => Operation::DeleteView {
            library: library.into(),
            cell: cell.into(),
            view: view.into(),
        },
        None => Operation::DeleteCell {
            library: library.into(),
            cell: cell.into(),
        },
    };
    let plan = refactor::plan(project, &operation, &Options::default())?;
    if dry_run {
        print_plan(project, &plan);
        println!("(dry run: nothing was changed)");
        return Ok(());
    }
    let lib = project
        .library(library)
        .ok_or_else(|| eyre!("no library '{library}' in the project"))?;
    let how = if permanent {
        Removal::Permanent
    } else {
        Removal::Trash
    };
    let trashed = match view {
        Some(view) => lib.remove_view(cell, view, how)?,
        None => lib.remove_cell(cell, how)?,
    };
    println!(
        "{operation}{}",
        if trashed.is_empty() {
            ""
        } else {
            " (moved to the trash)"
        }
    );
    for change in plan.not_followed() {
        eprintln!(
            "warning: {} uses it and now points at nothing",
            change.label
        );
    }
    Ok(())
}

pub fn cell(project: &Project, command: CellCommand) -> Result<()> {
    match command {
        CellCommand::New {
            library,
            cell,
            view,
        } => {
            let lib = project
                .library(&library)
                .ok_or_else(|| eyre!("no library '{library}' in the project"))?;
            if lib.cell(&cell).is_some() {
                bail!("'{cell}' already exists in library '{library}'");
            }
            let made = match view {
                Some(view) => {
                    lib.create_view_with(&cell, view.template(), &project.template_setup())?
                }
                None => lib.create_cell(&cell)?,
            };
            println!("created {}", made.display());
            Ok(())
        }
        CellCommand::Copy {
            library,
            cell,
            new,
            to,
            with_hierarchy,
            follow,
        } => refactor(
            project,
            Operation::CopyCell {
                to_library: to.unwrap_or_else(|| library.clone()),
                library,
                cell,
                new,
                hierarchy: with_hierarchy,
            },
            follow,
        ),
        CellCommand::Rename {
            library,
            cell,
            new,
            follow,
        } => refactor(
            project,
            Operation::RenameCell { library, cell, new },
            follow,
        ),
        CellCommand::Move {
            library,
            cell,
            to_library,
            name,
            follow,
        } => refactor(
            project,
            Operation::MoveCell {
                new: name.unwrap_or_else(|| cell.clone()),
                library,
                cell,
                to_library,
            },
            follow,
        ),
        CellCommand::Rm {
            library,
            cell,
            permanent,
            dry_run,
        } => remove(project, &library, &cell, None, permanent, dry_run),
    }
}

pub fn view(project: &Project, command: ViewCommand) -> Result<()> {
    match command {
        ViewCommand::New {
            library,
            cell,
            kind,
        } => {
            let lib = project
                .library(&library)
                .ok_or_else(|| eyre!("no library '{library}' in the project"))?;
            let file = lib.create_view_with(&cell, kind.template(), &project.template_setup())?;
            println!("created {}", file.display());
            Ok(())
        }
        ViewCommand::Copy {
            library,
            cell,
            view,
            name,
            to_library,
            to_cell,
            follow,
        } => refactor(
            project,
            Operation::CopyView {
                to_library: to_library.unwrap_or_else(|| library.clone()),
                to_cell: to_cell.unwrap_or_else(|| cell.clone()),
                new: name.unwrap_or_else(|| view.clone()),
                library,
                cell,
                view,
            },
            follow,
        ),
        ViewCommand::Rename {
            library,
            cell,
            view,
            new,
            follow,
        } => refactor(
            project,
            Operation::RenameView {
                library,
                cell,
                view,
                new,
            },
            follow,
        ),
        ViewCommand::Rm {
            library,
            cell,
            view,
            permanent,
            dry_run,
        } => remove(project, &library, &cell, Some(&view), permanent, dry_run),
    }
}

pub fn lib_rename(project: &Project, args: LibRename) -> Result<()> {
    refactor(
        project,
        Operation::RenameLibrary {
            library: args.library,
            new: args.new,
            folder: args.folder,
        },
        args.follow,
    )
}

fn existing_cell(project: &Project, library: &str, cell: &str) -> Result<()> {
    let lib = project
        .library(library)
        .ok_or_else(|| eyre!("no library '{library}' in the project"))?;
    lib.cell(cell)
        .ok_or_else(|| eyre!("no cell '{cell}' in library '{library}'"))?;
    Ok(())
}

pub fn hier(project: &Project, args: HierArgs) -> Result<()> {
    existing_cell(project, &args.library, &args.cell)?;
    let index = Index::build(project);
    for warning in &index.warnings {
        tracing::warn!("{warning}");
    }
    let kinds: Vec<TreeKind> = match args.view {
        Some(view) => vec![view.kind()],
        None => TreeKind::ALL.to_vec(),
    };
    let tree = kinds
        .iter()
        .find_map(|kind| index.tree(project, &args.library, &args.cell, *kind));
    let Some(tree) = tree else {
        let what = match args.view {
            Some(view) => view.kind().name(),
            None => "schematic, layout, netlist or HDL",
        };
        bail!("cell '{}/{}' has no {what} view", args.library, args.cell);
    };
    if args.json {
        println!("{}", serde_json::to_string_pretty(&tree)?);
        return Ok(());
    }
    for line in tree.lines(args.all) {
        println!("{line}");
    }
    Ok(())
}

pub fn used_by(project: &Project, args: UsedByArgs) -> Result<()> {
    existing_cell(project, &args.library, &args.cell)?;
    let index = Index::build(project);
    for warning in &index.warnings {
        tracing::warn!("{warning}");
    }
    let mut users: Vec<&Reference> = index.used_by(&args.library, &args.cell, args.view.as_deref());
    users.sort_by(|a, b| (&a.from, a.kind).cmp(&(&b.from, b.kind)));
    if args.json {
        println!("{}", serde_json::to_string_pretty(&users)?);
        return Ok(());
    }
    if users.is_empty() {
        println!("nothing in the project uses {}/{}", args.library, args.cell);
        return Ok(());
    }
    for r in users {
        let mut line = format!("{}  {} ×{}", r.from, r.kind.name(), r.count);
        if !r.instances.is_empty() {
            line.push_str(&format!(" ({})", r.instances.join(", ")));
        }
        if r.read_only {
            line.push_str("  [read-only]");
        }
        println!("{line}");
    }
    Ok(())
}
