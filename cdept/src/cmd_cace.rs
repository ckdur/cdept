//! `cdept cace …`: CACE datasheets in and out (docs/reference/files/cace.md).

use std::path::PathBuf;

use clap::Subcommand;
use eyre::{Result, bail, eyre};
use libcdept::Project;
use libcdept::cace::{export, import};

#[derive(Subcommand)]
pub enum CaceCommand {
    /// Read a CACE datasheet (YAML, cace_format 5.x) into testbench views and a suite.
    ///
    /// Every parameter simulated with ngspice becomes a testbench view of the cell (its
    /// conditions are corners, temperatures, [variables], [[sweep]] or [montecarlo];
    /// its spec limits [[spec]] bounds), and a suite view `datasheet` lists them and
    /// keeps the rest of the datasheet. Prints the plan and the report of what was
    /// mapped, what is only kept and what to look at; nothing is written without
    /// `--write`.
    Import {
        /// The datasheet.
        datasheet: PathBuf,
        /// The library/cell the views go into (made when it does not exist).
        #[arg(long, value_name = "LIBRARY/CELL")]
        into: String,
        /// Put this in front of every view name.
        #[arg(long, default_value = "")]
        view_prefix: String,
        /// The library/cell of the testbench schematic. Default: the cell's own.
        #[arg(long, value_name = "LIBRARY/CELL", default_value = "")]
        schematic: String,
        /// Replace views that exist.
        #[arg(long)]
        overwrite: bool,
        /// Write the views. Without it the plan is only printed.
        #[arg(long)]
        write: bool,
    },
    /// Write a suite or testbench as a CACE datasheet, with the results of its latest runs.
    ///
    /// The spec limits, conditions and results (minimum, typical and maximum over the
    /// run points, pass/fail) come from the CDePT files and the results store; what
    /// CDePT does not use comes back from the [cace] tables of an import.
    Export {
        /// The library/cell.
        #[arg(value_name = "LIBRARY/CELL")]
        cell: String,
        /// The suite or testbench view. Default: the cell's only suite, else its only testbench.
        #[arg(long)]
        view: Option<String>,
        /// The file to write. Default: standard output.
        #[arg(long, short = 'o')]
        output: Option<PathBuf>,
        /// Leave the measured results out.
        #[arg(long)]
        no_results: bool,
    },
}

fn split(s: &str) -> Result<(String, String)> {
    s.split_once('/')
        .filter(|(l, c)| !l.is_empty() && !c.is_empty() && !c.contains('/'))
        .map(|(l, c)| (l.to_string(), c.to_string()))
        .ok_or_else(|| eyre!("'{s}' is not LIBRARY/CELL"))
}

pub fn run(command: CaceCommand, project: &Project) -> Result<()> {
    match command {
        CaceCommand::Import {
            datasheet,
            into,
            view_prefix,
            schematic,
            overwrite,
            write,
        } => {
            let (library, cell) = split(&into)?;
            let options = import::ImportOptions {
                library,
                cell,
                prefix: view_prefix,
                schematic,
                overwrite,
                source: datasheet.display().to_string(),
            };
            let plan = import::plan_file(project, &datasheet, &options)?;
            for line in plan.lines(&project.root) {
                println!("{line}");
            }
            if !plan.ok() {
                bail!("cannot import: {}", plan.blockers.join("; "));
            }
            if !write {
                eprintln!(
                    "\nNothing written. --write makes the views; --view-prefix, --schematic and --overwrite change them."
                );
                return Ok(());
            }
            let written = import::apply(&plan)?;
            eprintln!(
                "\nWrote {} file(s) into {}/{}.",
                written.len(),
                plan.library,
                plan.cell
            );
        }
        CaceCommand::Export {
            cell,
            view,
            output,
            no_results,
        } => {
            let (library, cell) = split(&cell)?;
            let out = export::export(
                project,
                &library,
                &cell,
                &export::ExportOptions {
                    view,
                    results: !no_results,
                },
            )?;
            match &output {
                Some(file) => {
                    std::fs::write(file, &out.text)
                        .map_err(|e| eyre!("cannot write {}: {e}", file.display()))?;
                    eprintln!(
                        "Wrote {} ({} parameter(s) of {library}/{cell}/{}).",
                        file.display(),
                        out.parameters.len(),
                        out.view
                    );
                }
                None => print!("{}", out.text),
            }
            for note in &out.notes {
                eprintln!("note: {note}");
            }
        }
    }
    Ok(())
}
