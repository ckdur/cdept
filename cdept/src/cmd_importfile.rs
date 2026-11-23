//! `cdept import-files`: bring design files from outside the project into a library as
//! cells and views.

use std::path::PathBuf;

use clap::{Args, ValueEnum};
use eyre::{Result, bail};
use libcdept::importer::{self, ImportRequest, OnClash};
use libcdept::{Project, refactor};

#[derive(Clone, Copy, ValueEnum)]
enum Clash {
    /// Keep what the library has; the other views of the cell still come in.
    Skip,
    /// Import the cell under another name (`inv_1`).
    Rename,
    /// Replace the existing file of the view.
    Overwrite,
}

/// Import files into a library.
///
/// Reads the files and prints what would come in: which cells, in which views, written
/// where. Nothing is written without `--write`.
///
/// A file that holds several cells is taken apart: a GDSII file gives one cell per
/// top structure (with the structures it places), a SPICE or CDL file one per
/// `.subckt` (with the subcircuits it uses and the header of the file: `.include`,
/// `.model`, `.param`), a LEF file one per `MACRO` (its layers and vias become the cell
/// `<file>_tech`), a Liberty file one per `cell` group, a Verilog file one per module.
/// Use `--no-split` to take each file as one cell named after it. xschem, Qucs-S,
/// Magic, VHDL, Verilog-A and OASIS files are copied as they are.
///
/// Cells of the same name from several files (an `inv` in a GDS and in a SPICE file)
/// become one cell with several views: `layout`, `netlist`, `cdl`, `abstract`,
/// `liberty`, `verilog`, `schematic`, `symbol`, ...
///
/// Compressed files (`.gds.gz`) are refused: decompress them first. Files of unknown
/// format are reported, not guessed.
#[derive(Args)]
pub struct ImportFilesArgs {
    /// The files to import.
    #[arg(required = true)]
    files: Vec<PathBuf>,
    /// The library they go into (a `cells` or `flat` library of the project).
    #[arg(long, value_name = "LIBRARY")]
    to: String,
    /// Take the cells out of files that hold several (the default).
    #[arg(long, overrides_with = "no_split")]
    split: bool,
    /// Take every file as one cell named after it, whatever is inside.
    #[arg(long = "no-split", overrides_with = "split")]
    no_split: bool,
    /// Put this in front of every cell name.
    #[arg(long, default_value = "")]
    prefix: String,
    /// Give a cell of a file another name: OLD=NEW (before the prefix). May be repeated.
    #[arg(long = "name", value_name = "OLD=NEW")]
    names: Vec<String>,
    /// What to do with a view the library has already.
    #[arg(long, value_enum, default_value = "skip")]
    on_clash: Clash,
    /// Take only files of these formats (names as in `[suffixes]`: gds, spice, cdl,
    /// lef, liberty, verilog, xschem-schematic, ...). Comma separated or repeated.
    #[arg(long, value_delimiter = ',', value_name = "FORMAT")]
    views: Vec<String>,
    /// Write the files. Without it the plan is only printed.
    #[arg(long)]
    write: bool,
    /// List every view that would come in, not only the summary.
    #[arg(long, short = 'l')]
    long: bool,
}

pub fn import_files(args: ImportFilesArgs, project: &Project) -> Result<()> {
    let mut request = ImportRequest::new(&args.to, args.files.clone());
    request.split = !args.no_split;
    request.prefix = args.prefix.clone();
    request.on_clash = match args.on_clash {
        Clash::Skip => OnClash::Skip,
        Clash::Rename => OnClash::Rename,
        Clash::Overwrite => OnClash::Overwrite,
    };
    for pair in &args.names {
        let Some((old, new)) = pair.split_once('=') else {
            bail!("--name {pair}: expected OLD=NEW");
        };
        request.names.insert(old.into(), new.into());
    }
    for name in &args.views {
        match libcdept::formats::format_from_name(name) {
            Some(Some(format)) => request.views.push(format),
            _ => {
                let known: Vec<&str> = libcdept::formats::NAMES.iter().map(|(n, _)| *n).collect();
                bail!("--views {name}: not a format; use {}", known.join(", "));
            }
        }
    }
    let planned = importer::plan_import_report(project, &request)?;
    let lines = planned.plan.lines(&project.root);
    // The plan lists every view; the short form keeps the summary and the notes.
    for line in &lines {
        if args.long || !line.starts_with("  copy ") && !line.starts_with("      ") {
            println!("{line}");
        }
    }
    let report = &planned.report;
    println!(
        "\n{} view(s) of {} cell(s); {} skipped, {} problem(s)",
        report.imported.len(),
        report.cells().len(),
        report.skipped.len(),
        report.problems.len()
    );
    if !args.write {
        eprintln!(
            "\nNothing written. --write imports the files; -l lists every view; --on-clash, \
             --prefix and --no-split change what comes in."
        );
        return Ok(());
    }
    if report.imported.is_empty() {
        bail!("nothing to import");
    }
    let outcome = refactor::apply(project, &planned.plan)?;
    eprintln!(
        "\nImported {} view(s) of {} cell(s) into {}.",
        report.imported.len(),
        report.cells().len(),
        args.to
    );
    for problem in &outcome.problems {
        eprintln!("warning: {problem}");
    }
    Ok(())
}
