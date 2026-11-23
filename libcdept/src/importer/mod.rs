//! Bringing design files from outside the project into a library as cells and views.
//!
//! [`plan_import`] reads the files and says what would come in: which cells, in which
//! views, written where. The result is a [`Plan`] of [`crate::refactor`], so it is
//! shown as a dry run and applied with [`crate::refactor::apply`] (every file written
//! or nothing, with the same rollback journal as a rename or a copy).
//! [`plan_import_report`] also gives the [`Report`]: what came in and what could not.
//!
//! # What a file gives
//!
//! | format | with `split` (the default) | without |
//! |---|---|---|
//! | GDSII | one cell per top structure, with what it places | one cell named after the file |
//! | SPICE, CDL | one cell per `.subckt`, with the subcircuits it places and the header of the file (`.include`, `.model`, `.param`) | one cell |
//! | LEF | one cell per `MACRO`; layers and vias of a technology part become the cell `<file>_tech` (view `techlef`) | one cell |
//! | Liberty | one cell per `cell` group, with the library attributes | one cell |
//! | Verilog, SystemVerilog | one cell per module, with the modules it instantiates that the file defines | one cell |
//! | xschem, Qucs-S, Magic, VHDL, Verilog-A, OASIS, KLayout PCell | the file as it is, the cell named after it | same |
//!
//! Cells of the same name from several files (a GDS and a SPICE file of `inv`) are one
//! cell with several views. A view is a folder of the cell: GDSII `layout`, OASIS
//! `oasis`, Magic `magic`, SPICE `netlist`, CDL `cdl`, LEF `abstract`, Liberty
//! `liberty`, Verilog `verilog`, VHDL `vhdl`, Verilog-A `veriloga`, xschem
//! `schematic` and `symbol`, Qucs-S `qucs`, a PCell `pcell`.
//!
//! A file that is already in the library of a cell with the same view is a clash:
//! see [`OnClash`].

mod split;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use eyre::{Result, bail, eyre};

pub use split::{Piece, Split};

use crate::hierarchy::ViewId;
use crate::library::validate_name;
use crate::refactor::{Change, ChangeKind, Operation, Plan, Step};
use crate::{Format, Library, LibraryLayout, Project};

/// What to do with a view that the library has already.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OnClash {
    /// Leave the existing view alone and do not import that one; the other views of
    /// the cell still come in.
    #[default]
    Skip,
    /// Import the cell under another name (`inv_1`, `inv_2`…).
    Rename,
    /// Replace the file of the existing view. Never the default.
    Overwrite,
}

impl OnClash {
    pub fn name(self) -> &'static str {
        match self {
            OnClash::Skip => "skip",
            OnClash::Rename => "rename",
            OnClash::Overwrite => "overwrite",
        }
    }

    pub fn from_name(name: &str) -> Option<OnClash> {
        match name {
            "skip" => Some(OnClash::Skip),
            "rename" => Some(OnClash::Rename),
            "overwrite" => Some(OnClash::Overwrite),
            _ => None,
        }
    }
}

/// What to import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportRequest {
    /// The files, in any folder. Folders are not walked.
    pub files: Vec<PathBuf>,
    /// The library the cells go to (a `cells` or `flat` library of the project).
    pub library: String,
    /// Take the cells out of files that hold several (the default). Off: a file is
    /// one cell named after it, whatever is inside.
    pub split: bool,
    /// Put in front of every cell name.
    pub prefix: String,
    /// Cell names in the files → names in the library (before the prefix).
    pub names: BTreeMap<String, String>,
    pub on_clash: OnClash,
    /// Only files of these formats; empty: every recognised one.
    pub views: Vec<Format>,
}

impl ImportRequest {
    pub fn new(library: &str, files: Vec<PathBuf>) -> ImportRequest {
        ImportRequest {
            files,
            library: library.into(),
            split: true,
            prefix: String::new(),
            names: BTreeMap::new(),
            on_clash: OnClash::Skip,
            views: Vec::new(),
        }
    }

    /// The name a cell of a file gets: the name map, the prefix, then `renamed` (cells
    /// that had to be renamed because the library has them).
    fn name_of(&self, source: &str, renamed: &BTreeMap<String, String>) -> String {
        let name = self.names.get(source).map_or(source, String::as_str);
        let name = format!("{}{name}", self.prefix);
        renamed.get(&name).cloned().unwrap_or(name)
    }
}

/// One view that comes in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Imported {
    pub cell: String,
    pub view: String,
    /// The file the view is made from.
    pub source: PathBuf,
    /// Where the view's file will be.
    pub to: PathBuf,
    /// What was taken from the source.
    pub what: String,
}

/// What an import does and what it cannot do.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    pub imported: Vec<Imported>,
    /// Not imported because the library has it (see [`OnClash`]) or another file
    /// gave it first.
    pub skipped: Vec<String>,
    /// Could not be read or understood, and what is missing from what did come in:
    /// ghost structures of a layout, subcircuits that are used and not defined,
    /// files of unknown format.
    pub problems: Vec<String>,
}

impl Report {
    /// The cells that come in, sorted, each once.
    pub fn cells(&self) -> Vec<&str> {
        let set: BTreeSet<&str> = self.imported.iter().map(|i| i.cell.as_str()).collect();
        set.into_iter().collect()
    }

    pub fn lines(&self) -> Vec<String> {
        let mut out = vec![format!(
            "{} view(s) of {} cell(s) imported",
            self.imported.len(),
            self.cells().len()
        )];
        for line in &self.skipped {
            out.push(format!("skipped: {line}"));
        }
        for line in &self.problems {
            out.push(format!("problem: {line}"));
        }
        out
    }
}

/// A plan with its report.
#[derive(Debug, Clone)]
pub struct ImportPlan {
    pub plan: Plan,
    pub report: Report,
}

/// The view a format goes into.
pub fn view_name(format: Format) -> &'static str {
    match format {
        Format::XschemSchematic => "schematic",
        Format::XschemSymbol => "symbol",
        Format::QucsSchematic => "qucs",
        Format::Gds => "layout",
        Format::Oasis => "oasis",
        Format::Magic => "magic",
        Format::Spice => "netlist",
        Format::Cdl => "cdl",
        Format::Verilog | Format::SystemVerilog => "verilog",
        Format::Vhdl => "vhdl",
        Format::VerilogA => "veriloga",
        Format::Lef => "abstract",
        Format::Liberty => "liberty",
        Format::KlayoutPcell => "pcell",
    }
}

/// One file to write.
struct Candidate {
    cell: String,
    view: String,
    /// The file name suffix with its dot (`.gds`, `.pcell.py`).
    suffix: String,
    bytes: Vec<u8>,
    source: PathBuf,
    what: String,
}

fn suffix_of(name: &str) -> String {
    Format::split_name(name)
        .map(|(stem, _)| name[stem.len()..].to_string())
        .unwrap_or_default()
}

/// Plan an import; see [`plan_import_report`] for what it also says.
pub fn plan_import(project: &Project, request: &ImportRequest) -> Result<Plan> {
    Ok(plan_import_report(project, request)?.plan)
}

/// Plan an import. Fails when the library cannot take files; what is wrong with a single
/// file goes into the [`Report`] and the other files still come in.
pub fn plan_import_report(project: &Project, request: &ImportRequest) -> Result<ImportPlan> {
    let _suffixes = crate::formats::enter(&project.root);
    let library = project
        .library(&request.library)
        .ok_or_else(|| eyre!("no library '{}' in the project", request.library))?;
    library.writable()?;
    if request.files.is_empty() {
        bail!("no files to import");
    }
    validate_name("name prefix", &format!("x{}", request.prefix))?;

    let mut renamed = BTreeMap::new();
    let (mut by_cell, mut report) = gather(request, &renamed);
    if request.on_clash == OnClash::Rename {
        let taken: BTreeSet<String> = by_cell.keys().cloned().collect();
        for (cell, views) in &by_cell {
            if !views.iter().any(|c| destination(library, cell, c).1) {
                continue;
            }
            let free = (1..)
                .map(|n| format!("{cell}_{n}"))
                .find(|name| {
                    !taken.contains(name)
                        && library.cell(name).is_none()
                        && !library.path.join(name).exists()
                        && !views.iter().any(|c| destination(library, name, c).1)
                })
                .expect("a free name");
            renamed.insert(cell.clone(), free);
        }
        if !renamed.is_empty() {
            (by_cell, report) = gather(request, &renamed);
            for (cell, name) in &renamed {
                report.skipped.push(format!(
                    "{cell}: the library has it already, imported as '{name}'"
                ));
            }
        }
    }
    let mut placed: Vec<(Candidate, PathBuf, bool)> = Vec::new();
    for (cell, views) in by_cell {
        for c in views {
            let (to, exists) = destination(library, &cell, &c);
            if exists && request.on_clash != OnClash::Overwrite {
                report.skipped.push(format!(
                    "{cell}/{}: the library has it already ({})",
                    c.view,
                    to.display()
                ));
                continue;
            }
            placed.push((c, to, exists));
        }
    }

    // Steps.
    let mut steps = Vec::new();
    let mut changes = Vec::new();
    let mut made: BTreeSet<PathBuf> = BTreeSet::new();
    let mut notes = Vec::new();
    for (c, to, exists) in &placed {
        if let Some(dir) = to.parent()
            && library.layout == LibraryLayout::Cells
        {
            if let Some(cell_dir) = dir.parent()
                && !cell_dir.exists()
                && made.insert(cell_dir.to_path_buf())
            {
                steps.push(Step::MakeDir(cell_dir.to_path_buf()));
            }
            if !dir.exists() && made.insert(dir.to_path_buf()) {
                steps.push(Step::MakeDir(dir.to_path_buf()));
            }
        }
        steps.push(Step::Extract {
            from: c.source.clone(),
            to: to.clone(),
            bytes: c.bytes.clone(),
            what: c.what.clone(),
        });
        changes.push(Change {
            kind: ChangeKind::Copy,
            path: c.source.clone(),
            to: Some(to.clone()),
            label: format!("{}/{}/{}", library.name, c.cell, c.view),
            details: vec![if *exists {
                format!("{} (replaces the existing file)", c.what)
            } else {
                c.what.clone()
            }],
        });
        report.imported.push(Imported {
            cell: c.cell.clone(),
            view: c.view.clone(),
            source: c.source.clone(),
            to: to.clone(),
            what: c.what.clone(),
        });
    }
    if placed.is_empty() {
        notes.push("nothing to import".to_string());
    }
    for line in &report.skipped {
        notes.push(format!("skipped: {line}"));
    }
    for line in &report.problems {
        notes.push(line.clone());
    }
    let result = placed
        .first()
        .map(|(c, _, _)| ViewId::new(&library.name, &c.cell, &c.view))
        .unwrap_or_else(|| ViewId::new(&library.name, "", ""));
    let operation = Operation::Import {
        library: library.name.clone(),
        summary: format!(
            "{} file(s) give {} view(s) of {} cell(s)",
            request.files.len(),
            report.imported.len(),
            report.cells().len()
        ),
    };
    Ok(ImportPlan {
        plan: Plan::from_steps(operation, changes, notes, result, steps),
        report,
    })
}

/// The candidates of every file, one view per cell and view (the first file that gives
/// it wins), grouped by cell.
fn gather(
    request: &ImportRequest,
    renamed: &BTreeMap<String, String>,
) -> (BTreeMap<String, Vec<Candidate>>, Report) {
    let mut report = Report::default();
    let mut candidates: Vec<Candidate> = Vec::new();
    let mut seen_files = BTreeSet::new();
    for file in &request.files {
        if !seen_files.insert(file.clone()) {
            continue;
        }
        if let Err(e) = read_file(request, renamed, file, &mut candidates, &mut report) {
            report.problems.push(format!("{}: {e:#}", file.display()));
        }
    }
    let mut taken: BTreeMap<(String, String), PathBuf> = BTreeMap::new();
    let mut by_cell: BTreeMap<String, Vec<Candidate>> = BTreeMap::new();
    for c in candidates {
        if let Err(e) = validate_name("cell", &c.cell) {
            report.problems.push(format!(
                "{}: '{}' is not a name CDePT can use for a cell ({e}); give it another with \
                 a name map",
                c.source.display(),
                c.cell
            ));
            continue;
        }
        match taken.get(&(c.cell.clone(), c.view.clone())) {
            Some(first) => report.skipped.push(format!(
                "{}/{}: {} also has it, {} came first",
                c.cell,
                c.view,
                c.source.display(),
                first.display()
            )),
            None => {
                taken.insert((c.cell.clone(), c.view.clone()), c.source.clone());
                by_cell.entry(c.cell.clone()).or_default().push(c);
            }
        }
    }
    (by_cell, report)
}

/// Where a candidate goes in the library, and whether something is there already.
fn destination(library: &Library, cell: &str, c: &Candidate) -> (PathBuf, bool) {
    let file = format!("{cell}{}", c.suffix);
    match library.layout {
        LibraryLayout::Cells => {
            let to = library.path.join(cell).join(&c.view).join(&file);
            let view_there = library
                .cell(cell)
                .is_some_and(|k| k.entry(&c.view).is_some())
                || library.path.join(cell).join(&c.view).is_dir();
            (to.clone(), view_there || to.exists())
        }
        _ => {
            let to = library.path.join(&file);
            let taken = library
                .cell(cell)
                .is_some_and(|k| k.entries.iter().any(|e| e.name == c.view || e.path == to));
            (to.clone(), taken || to.exists())
        }
    }
}

/// Read one file into candidates.
fn read_file(
    request: &ImportRequest,
    renamed: &BTreeMap<String, String>,
    file: &Path,
    out: &mut Vec<Candidate>,
    report: &mut Report,
) -> Result<()> {
    let name = file
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| eyre!("not a file name"))?;
    if !file.is_file() {
        bail!("not a file");
    }
    let Some((stem, _)) = Format::split_name(name) else {
        report
            .problems
            .push(format!("{}: unknown format, not imported", file.display()));
        return Ok(());
    };
    let format = Format::of_file(file).expect("the suffix is known");
    if !request.views.is_empty() && !request.views.contains(&format) {
        report.skipped.push(format!(
            "{}: {} is not among the formats asked for",
            file.display(),
            format.label()
        ));
        return Ok(());
    }
    let suffix = suffix_of(name);
    let bytes = fs::read(file).map_err(|e| eyre!("cannot read: {e}"))?;
    if bytes.starts_with(&[0x1f, 0x8b]) {
        bail!(
            "compressed ({}): decompress it first (gunzip) and import the result",
            format.label()
        );
    }
    let view = view_name(format).to_string();
    let whole = |out: &mut Vec<Candidate>, note: &str| {
        out.push(Candidate {
            cell: request.name_of(stem, renamed),
            view: view.clone(),
            suffix: suffix.clone(),
            bytes: bytes.clone(),
            source: file.to_path_buf(),
            what: note.to_string(),
        });
    };
    let dir = std::path::absolute(file.parent().unwrap_or(Path::new(".")))
        .unwrap_or_else(|_| PathBuf::from("."));
    let dir = dir.as_path();
    let text = || String::from_utf8_lossy(&bytes).into_owned();
    let names = |n: &str| request.name_of(n, renamed);
    let split = if request.split {
        match format {
            Format::Gds => Some(split::gds(&bytes, &names)?),
            Format::Spice | Format::Cdl => Some(split::spice(&text(), dir, &names)?),
            Format::Verilog | Format::SystemVerilog => Some(split::verilog(&text(), &names)?),
            Format::Lef => Some(split::lef(&text(), &names)?),
            Format::Liberty => Some(split::liberty(&text(), &names)?),
            _ => None,
        }
    } else {
        None
    };
    let Some(split) = split else {
        let note = match format {
            Format::Gds => crate::layout::gds::parse(&bytes)
                .map(|s| format!("{} structure(s), kept as one file", s.structures.len()))?,
            _ => "the file as it is".to_string(),
        };
        whole(out, &note);
        return Ok(());
    };
    for n in &split.notes {
        report.problems.push(format!("{}: {n}", file.display()));
    }
    if split.pieces.is_empty() && split.tech.is_none() {
        // Nothing inside (a netlist of top-level cards, a Verilog file without a module):
        // the file is the cell.
        whole(out, "the file as it is (no cells found inside)");
        return Ok(());
    }
    // What cells use and the file does not define, one line per name.
    let mut missing: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for piece in &split.pieces {
        for g in &piece.unresolved {
            missing
                .entry(g.clone())
                .or_default()
                .push(piece.name.clone());
        }
    }
    let what = if format == Format::Gds {
        "is placed and not defined in the file (an external or ghost cell)"
    } else {
        "is used and not defined in the file"
    };
    for (g, cells) in missing {
        let shown: Vec<String> = cells.iter().take(3).map(|c| format!("'{c}'")).collect();
        let more = if cells.len() > 3 {
            format!(" and {} more", cells.len() - 3)
        } else {
            String::new()
        };
        report.problems.push(format!(
            "{}: '{g}' {what}, by {}{more}",
            file.display(),
            shown.join(", ")
        ));
    }
    for piece in split.pieces {
        out.push(Candidate {
            cell: piece.name,
            view: view.clone(),
            suffix: suffix.clone(),
            bytes: piece.bytes,
            source: file.to_path_buf(),
            what: piece.what,
        });
    }
    if let Some(tech) = split.tech {
        let cell = if out.iter().any(|c| c.source == file) {
            request.name_of(&format!("{stem}_tech"), renamed)
        } else {
            request.name_of(stem, renamed)
        };
        out.push(Candidate {
            cell,
            view: "techlef".into(),
            suffix,
            bytes: tech,
            source: file.to_path_buf(),
            what: "the technology part (layers, vias, rules)".into(),
        });
    }
    Ok(())
}

/// Plan and apply in one go.
pub fn run(
    project: &Project,
    request: &ImportRequest,
) -> Result<(Report, crate::refactor::Outcome)> {
    let ImportPlan { plan, report } = plan_import_report(project, request)?;
    let outcome = crate::refactor::apply(project, &plan)?;
    Ok((report, outcome))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_names_are_distinct_per_format() {
        let mut names = BTreeSet::new();
        for (_, f) in crate::formats::NAMES {
            if !matches!(f, Format::SystemVerilog) {
                assert!(names.insert(view_name(*f)), "{f:?}");
            }
        }
    }

    #[test]
    fn clash_names_round_trip() {
        for c in [OnClash::Skip, OnClash::Rename, OnClash::Overwrite] {
            assert_eq!(OnClash::from_name(c.name()), Some(c));
        }
        assert_eq!(OnClash::from_name("x"), None);
    }
}
