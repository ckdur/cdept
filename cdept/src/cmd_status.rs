//! `cdept status`: the git status of the project, by library, cell and view.

use eyre::{Result, bail};
use libcdept::Project;
use libcdept::vcs::{State, Status, Summary};
use serde_json::json;

#[derive(clap::Args)]
pub struct StatusArgs {
    /// Only this library.
    library: Option<String>,
    /// Also list the cells that are as committed.
    #[arg(long)]
    all: bool,
    /// Print as JSON.
    #[arg(long)]
    json: bool,
}

fn mark(state: State) -> &'static str {
    match state {
        State::Clean => " ",
        other => other.mark(),
    }
}

/// "schematic modified, symbol staged" for the views of a cell that are not clean.
fn views(status: &Status, cell: &libcdept::Cell) -> String {
    cell.entries
        .iter()
        .filter_map(|entry| {
            let state = status.entry(entry)?.state();
            (state != State::Clean).then(|| format!("{} {}", entry.name, state.label()))
        })
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn run(args: StatusArgs, project: &Project) -> Result<()> {
    if let Some(name) = &args.library
        && project.library(name).is_none()
    {
        bail!("no library '{name}'");
    }
    let status = Status::of_project(project);
    let libraries = project
        .libraries
        .iter()
        .filter(|l| args.library.as_deref().is_none_or(|name| name == l.name));

    if args.json {
        let libraries: Vec<_> = libraries
            .map(|library| {
                let summary = status.library(library);
                let cells: Vec<_> = library
                    .cells
                    .iter()
                    .filter_map(|cell| {
                        let summary = status.cell(cell)?;
                        (args.all || summary.state() != State::Clean).then(|| {
                            let views: Vec<_> = cell
                                .entries
                                .iter()
                                .filter_map(|e| {
                                    let s = status.entry(e)?;
                                    Some(json!({"name": e.name, "state": s.state()}))
                                })
                                .collect();
                            json!({
                                "name": cell.name,
                                "state": summary.state(),
                                "summary": summary,
                                "views": views,
                            })
                        })
                    })
                    .collect();
                json!({
                    "name": library.name,
                    "path": library.path,
                    "repository": status.repository(&library.path).map(|r| &r.root),
                    "state": summary.map(|s| s.state()),
                    "summary": summary,
                    "cells": cells,
                })
            })
            .collect();
        let out = json!({
            "repositories": status.repositories,
            "project_file": status.file(&project.file),
            "libraries": libraries,
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    if status.is_empty() {
        println!("not in a git repository (or git is not installed)");
        return Ok(());
    }
    for repository in &status.repositories {
        let dirty = match repository.dirty() {
            0 => "clean".to_string(),
            n => format!("{n} changed"),
        };
        println!(
            "{}  on {}, {dirty}",
            repository.root.display(),
            repository.head()
        );
    }
    if let Some(state) = status.file(&project.file)
        && state != State::Clean
    {
        println!("project file: {}", state.label());
    }
    for library in libraries {
        let Some(summary) = status.library(library) else {
            let why = if libcdept::watch::is_reference(project, library) {
                "reference library, not looked at"
            } else {
                "not in a git repository"
            };
            println!("library {} ({why})", library.name);
            continue;
        };
        println!("library {} ({})", library.name, describe(&summary));
        let mut listed = 0;
        for cell in &library.cells {
            let Some(summary) = status.cell(cell) else {
                continue;
            };
            let state = summary.state();
            if state == State::Clean && !args.all {
                continue;
            }
            listed += 1;
            println!(
                "  {}  {:<24} {}",
                mark(state),
                cell.name,
                views(&status, cell)
            );
        }
        // Deleted cells and stray files: changes that belong to no cell that is left.
        let strays = summary.changed().saturating_sub(
            library
                .cells
                .iter()
                .filter_map(|c| status.cell(c))
                .map(|s| s.changed())
                .sum(),
        );
        if strays > 0 {
            println!("  …  {strays} more change(s) outside the cells (deleted cells, other files)");
        } else if listed == 0 && !args.all && summary.state() != State::Clean {
            println!(
                "  (the library folder itself is {})",
                summary.state().label()
            );
        }
    }
    Ok(())
}

fn describe(summary: &Summary) -> String {
    summary.describe()
}
