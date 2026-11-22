//! `cdept find` (search across libraries) and `cdept category` (groups of cells).

use clap::{Args, Subcommand};
use eyre::{Result, bail, eyre};
use libcdept::category::{self, Node};
use libcdept::search::{self, Matched, Query};
use libcdept::{Project, ViewKind};

#[derive(Args)]
pub struct FindArgs {
    /// What to look for: words that must be in the cell name (a few letters in order
    /// also match), patterns with * ? [a-z], `library/cell`, and filters written
    /// `lib:NAME kind:KIND view:NAME format:SUFFIX cat:CATEGORY in:content`.
    query: Vec<String>,
    /// Only in this library (a pattern is fine). May be repeated.
    #[arg(short, long = "lib")]
    library: Vec<String>,
    /// Only views of this kind (schematic, symbol, layout, netlist, …). The hits are
    /// then views, not cells.
    #[arg(short, long)]
    kind: Vec<String>,
    /// Only views of this file format, by suffix (gds, sch, v, …).
    #[arg(short, long)]
    format: Vec<String>,
    /// Only cells of this category (or of one below it).
    #[arg(short, long)]
    category: Vec<String>,
    /// Also look inside schematics, symbols, netlists and HDL files: pins, instances
    /// and text lines that have every word.
    #[arg(long)]
    content: bool,
    /// The most hits to print.
    #[arg(short = 'n', long, default_value_t = 50)]
    limit: usize,
    /// Print the hits as JSON.
    #[arg(long)]
    json: bool,
}

pub fn find(project: &Project, args: FindArgs) -> Result<()> {
    let mut query = Query::parse(&args.query.join(" "));
    query.libraries.extend(args.library);
    query.categories.extend(args.category);
    query.content |= args.content;
    query.limit = args.limit;
    for kind in &args.kind {
        let found = ViewKind::ALL.into_iter().find(|k| k.name() == kind);
        query.kinds.push(found.ok_or_else(|| {
            eyre!(
                "'{kind}' is not a kind of view ({})",
                ViewKind::ALL.map(|k| k.name()).join(", ")
            )
        })?);
    }
    for format in &args.format {
        query.formats.push(
            search::format_from_name(format)
                .ok_or_else(|| eyre!("'{format}' is not a file format CDePT knows"))?,
        );
    }
    if let Some(problem) = query.problems.first() {
        bail!("{problem}");
    }
    let results = search::search(project, &query);
    if args.json {
        let hits: Vec<serde_json::Value> = results
            .hits
            .iter()
            .map(|hit| {
                let mut item = serde_json::json!({
                    "library": hit.library,
                    "cell": hit.cell,
                    "view": hit.view,
                    "kind": hit.kind.map(|k| k.name()),
                    "file": hit.file,
                    "score": hit.score,
                });
                if let Matched::Content { line, text, kind } = &hit.matched {
                    item["line"] = (*line).into();
                    item["text"] = text.as_str().into();
                    item["what"] = kind.label().into();
                }
                item
            })
            .collect();
        let out = serde_json::json!({
            "hits": hits,
            "total": results.total,
            "files_scanned": results.files_scanned,
            "truncated": results.truncated,
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        for hit in &results.hits {
            let mut line = format!("{}/{}", hit.library, hit.cell);
            if let Some(view) = &hit.view {
                line.push_str(&format!("/{view}"));
            }
            if let Matched::Content {
                line: number,
                text,
                kind,
            } = &hit.matched
            {
                line.push_str(&format!(":{number}: [{}] {text}", kind.label()));
            }
            println!("{line}");
        }
        if results.total > results.hits.len() {
            eprintln!(
                "{} of {} hits shown (-n for more)",
                results.hits.len(),
                results.total
            );
        }
        if results.truncated {
            eprintln!(
                "looking inside files stopped early after {} files; narrow the search with \
                 lib: or kind:",
                results.files_scanned
            );
        }
    }
    if results.hits.is_empty() {
        bail!("nothing found");
    }
    Ok(())
}

#[derive(Subcommand)]
pub enum CategoryCommand {
    /// List the categories of a library (of every library when none is given) and
    /// their cells.
    Ls {
        library: Option<String>,
        /// Only the category names, not their cells.
        #[arg(long)]
        names: bool,
    },
    /// Put cells into a category (made if it is new). Without cells: make an empty
    /// category. Nested categories are written `analog/amplifiers`.
    Add {
        library: String,
        category: String,
        cells: Vec<String>,
    },
    /// Take cells out of a category. Without cells: remove the category (and those
    /// below it); its cells stay in the library.
    Rm {
        library: String,
        category: String,
        cells: Vec<String>,
    },
    /// Rename or move a category, with the categories below it.
    Rename {
        library: String,
        category: String,
        new: String,
    },
    /// Show the categories a cell is in.
    Of { library: String, cell: String },
    /// Propose categories for a library and, with --write, add them: by the `type` of
    /// the xschem symbols (nmos, resistor, subcircuit, …) or, for standard cells, by
    /// function (and, nand, dff, …).
    Suggest {
        library: String,
        #[arg(long)]
        write: bool,
    },
    /// Copy the categories IHP LibMan keeps in the library folder (*.group files)
    /// into CDePT's own files, so they can be changed here.
    Adopt { library: String },
}

fn print_tree(nodes: &[Node], depth: usize, names_only: bool) {
    for node in nodes {
        let indent = "  ".repeat(depth + 1);
        println!("{indent}{}/  ({})", node.name, node.count());
        if !names_only {
            for cell in &node.cells {
                println!("{indent}  {cell}");
            }
        }
        print_tree(&node.children, depth + 1, names_only);
    }
}

pub fn category(project: &Project, command: CategoryCommand) -> Result<()> {
    let library = |name: &str| {
        project
            .library(name)
            .ok_or_else(|| eyre!("no library '{name}'"))
    };
    match command {
        CategoryCommand::Ls {
            library: name,
            names,
        } => {
            let libraries: Vec<_> = match &name {
                Some(name) => vec![library(name)?],
                None => project.libraries.iter().collect(),
            };
            for lib in libraries {
                let found = category::load(project, lib);
                for warning in &found.warnings {
                    eprintln!("warning: {warning}");
                }
                if found.all.is_empty() && name.is_none() {
                    continue;
                }
                let uncategorised = lib
                    .cells
                    .iter()
                    .filter(|c| found.all.is_uncategorised(&c.name))
                    .count();
                println!(
                    "{}  (kept in {}; {} of {} cells in no category)",
                    lib.name,
                    found.store.label(),
                    uncategorised,
                    lib.cells.len()
                );
                print_tree(&found.all.tree(), 0, names);
            }
        }
        CategoryCommand::Add {
            library: name,
            category: cat,
            cells,
        } => {
            if cells.is_empty() {
                category::add(project, &name, &cat)?;
                println!("category '{cat}' added to {name}");
            } else {
                let cells: Vec<&str> = cells.iter().map(String::as_str).collect();
                category::assign(project, &name, &cat, &cells)?;
                println!("{} cell(s) put into '{cat}' of {name}", cells.len());
            }
        }
        CategoryCommand::Rm {
            library: name,
            category: cat,
            cells,
        } => {
            if cells.is_empty() {
                category::remove(project, &name, &cat)?;
                println!("category '{cat}' removed from {name}");
            } else {
                let cells: Vec<&str> = cells.iter().map(String::as_str).collect();
                category::unassign(project, &name, &cat, &cells)?;
                println!("{} cell(s) taken out of '{cat}' of {name}", cells.len());
            }
        }
        CategoryCommand::Rename {
            library: name,
            category: cat,
            new,
        } => {
            category::rename(project, &name, &cat, &new)?;
            println!("category '{cat}' of {name} is now '{new}'");
        }
        CategoryCommand::Of {
            library: name,
            cell,
        } => {
            let lib = library(&name)?;
            if lib.cell(&cell).is_none() {
                bail!("no cell '{cell}' in library '{name}'");
            }
            for cat in category::load(project, lib).all.of_cell(&cell) {
                println!("{cat}");
            }
        }
        CategoryCommand::Suggest {
            library: name,
            write,
        } => {
            let lib = library(&name)?;
            let suggested = category::suggest(lib);
            if suggested.is_empty() {
                bail!(
                    "nothing to suggest for '{name}': its cells have no xschem symbols with a \
                     type, and their names do not look like a standard-cell library's"
                );
            }
            print_tree(&suggested.tree(), 0, write);
            if write {
                let added = category::add_all(project, &name, &suggested)?;
                println!(
                    "{added} cell(s) put into categories, in {}",
                    category::store_of(lib).label()
                );
            } else {
                println!("(nothing written; --write adds these categories)");
            }
        }
        CategoryCommand::Adopt { library: name } => {
            let added = category::adopt_libman(project, &name)?;
            println!("{added} cell(s) put into categories copied from LibMan's files");
        }
    }
    Ok(())
}
