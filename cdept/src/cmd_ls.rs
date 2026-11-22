//! `cdept ls` with the files behind the views, and one cell in detail.
//!
//! In reference libraries (and flat ones with collection files) a view is one cell of
//! a file that holds many: `--files` shows which file and which cell inside it;
//! naming a cell shows what the files say about it (LEF class and size, Liberty area
//! and pins, sub-circuit pins, …).

use std::path::Path;

use eyre::{Result, eyre};
use libcdept::container::Cache;
use libcdept::{Cell, Entry, Library, Project};

/// The cell inside `file` that the layout view of `cell` shows, when the file holds
/// several cells (`None`: the file is the view).
pub fn member_of(project: &Project, library: &str, cell: &str, file: &Path) -> Option<String> {
    project
        .library(library)?
        .cell(cell)?
        .entries
        .iter()
        .find(|e| e.main_file.as_deref() == Some(file))?
        .member
        .clone()
}

fn relative<'a>(library: &Library, file: &'a Path) -> std::path::Display<'a> {
    file.strip_prefix(&library.path).unwrap_or(file).display()
}

/// `view  file  [cell inside]  (+n more files)`.
fn view_line(library: &Library, entry: &Entry) -> String {
    let mut line = format!("{:<18}", entry.name);
    if let Some(file) = &entry.main_file {
        line.push_str(&format!(" {}", relative(library, file)));
    }
    if let Some(member) = &entry.member {
        line.push_str(&format!("  [{member}]"));
    }
    if entry.files.len() > 1 {
        line.push_str(&format!("  (+{} files)", entry.files.len() - 1));
    }
    line
}

/// `cdept ls <library> --files`: every cell with its views, one view per line.
pub fn files(project: &Project, only: Option<&str>) -> Result<()> {
    if let Some(name) = only
        && project.library(name).is_none()
    {
        return Err(eyre!("no library '{name}'"));
    }
    for library in &project.libraries {
        if only.is_some_and(|name| name != library.name) {
            continue;
        }
        println!(
            "{}  {}  [{}]",
            library.name,
            library.path.display(),
            library.layout.name()
        );
        for cell in &library.cells {
            println!("  {}", cell.name);
            for entry in &cell.entries {
                println!("    {}", view_line(library, entry));
            }
        }
    }
    Ok(())
}

/// `cdept ls <library> <cell>`: the views of one cell, their files and what the files
/// say about the cell.
pub fn cell(project: &Project, library: &str, name: &str) -> Result<()> {
    let library = project
        .library(library)
        .ok_or_else(|| eyre!("no library '{library}'"))?;
    let cell: &Cell = library
        .cell(name)
        .ok_or_else(|| eyre!("no cell '{name}' in library '{}'", library.name))?;
    println!("{}/{}", library.name, cell.name);
    let cache = Cache::user();
    for entry in &cell.entries {
        println!("  {}", view_line(library, entry));
        let kind = entry.format.map_or(entry.kind.name(), |f| f.label());
        println!("    {:<12} {kind}", "format");
        for (fact, value) in libcdept::container::entry_facts(entry, &cell.name, &cache) {
            println!("    {:<12} {value}", fact.to_lowercase());
        }
        if entry.files.len() > 1 {
            for file in &entry.files {
                println!("    {:<12} {}", "file", relative(library, file));
            }
        }
    }
    Ok(())
}
