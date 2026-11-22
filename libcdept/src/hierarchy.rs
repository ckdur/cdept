//! Who uses what: the references between the views of a project, read from the files.
//!
//! [`Index::build`] reads every design file of the project's libraries without starting
//! a tool and lists what it refers to:
//!
//! - xschem schematics and symbols: the symbols they place (`C {lib/cell/…/x.sym}`
//!   records) and the schematic a symbol stands for (its `schematic=` attribute, or
//!   the `.sch` file next to it);
//! - layouts (GDSII, OASIS): the cells of other views they place (ghost cells,
//!   resolved through the sidecar or by name, as the editor does), the PCells they
//!   place (KLayout's context records, GDSII only), and the schematic they are linked
//!   with (sidecar `source`);
//! - SPICE and CDL netlists: the subcircuits they instantiate (`X…` lines);
//! - Verilog and SystemVerilog: the modules they instantiate.
//!
//! Each [`Reference`] is resolved to a view of the project, to a file outside it
//! (xschem's own devices), or left unresolved. From the index come the hierarchy below
//! a cell ([`Index::tree`]), the users of a cell ([`Index::used_by`]) and the files a
//! rename has to rewrite ([`crate::refactor`]).
//!
//! Read-only libraries (PDKs) are read too, but only their text files up to
//! [`MAX_TEXT`] bytes and none of their layouts: they cannot be rewritten and their
//! layout files are large.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ops::Range;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::layout::{self, gds::CONTEXT_INFO, meta::Meta};
use crate::link::schematic;
use crate::xschem::symbol::{self, Link};
use crate::{Cell, Entry, Format, Library, Project, ViewKind};

/// Text files larger than this are not read (generated netlists of whole chips).
pub const MAX_TEXT: u64 = 8 * 1024 * 1024;
/// The most nodes [`Index::tree`] makes: a cell used many times at several levels
/// unfolds into very many.
pub const MAX_NODES: usize = 20_000;

/// `library/cell/view`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub struct ViewId {
    pub library: String,
    pub cell: String,
    pub view: String,
}

impl ViewId {
    pub fn new(library: &str, cell: &str, view: &str) -> ViewId {
        ViewId {
            library: library.into(),
            cell: cell.into(),
            view: view.into(),
        }
    }

    pub fn label(&self) -> String {
        format!("{}/{}/{}", self.library, self.cell, self.view)
    }

    pub fn is_cell(&self, library: &str, cell: &str) -> bool {
        self.library == library && self.cell == cell
    }
}

impl std::fmt::Display for ViewId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}/{}", self.library, self.cell, self.view)
    }
}

/// How one file refers to another view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RefKind {
    /// A symbol placed in an xschem schematic (or symbol).
    Instance,
    /// The schematic an xschem symbol stands for.
    SchematicLink,
    /// A cell of another view placed in a layout.
    Placement,
    /// A PCell placed in a layout.
    Pcell,
    /// The schematic a layout is linked with (docs/LINKING.md).
    Source,
    /// A subcircuit instantiated in a netlist.
    Subckt,
    /// A module instantiated in Verilog.
    Module,
}

impl RefKind {
    pub fn name(self) -> &'static str {
        match self {
            RefKind::Instance => "instance",
            RefKind::SchematicLink => "schematic of the symbol",
            RefKind::Placement => "placement",
            RefKind::Pcell => "PCell",
            RefKind::Source => "linked schematic",
            RefKind::Subckt => "subcircuit",
            RefKind::Module => "module",
        }
    }
}

/// What a reference points at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Target {
    /// A view of the project and its file.
    View {
        id: ViewId,
        file: PathBuf,
    },
    /// A cell of the project, found by its name (no view is named).
    Cell {
        library: String,
        cell: String,
    },
    /// A file outside the project's libraries, e.g. one of xschem's own devices.
    External(PathBuf),
    /// A PCell library of KLayout that is not a library of the project (the PDK's).
    Tool(String),
    Unresolved,
}

impl Target {
    /// The library and cell, for a target inside the project.
    pub fn cell(&self) -> Option<(&str, &str)> {
        match self {
            Target::View { id, .. } => Some((&id.library, &id.cell)),
            Target::Cell { library, cell } => Some((library, cell)),
            _ => None,
        }
    }

    pub fn view(&self) -> Option<&ViewId> {
        match self {
            Target::View { id, .. } => Some(id),
            _ => None,
        }
    }

    pub fn file(&self) -> Option<&Path> {
        match self {
            Target::View { file, .. } | Target::External(file) => Some(file),
            _ => None,
        }
    }

    pub fn is_unresolved(&self) -> bool {
        matches!(self, Target::Unresolved)
    }
}

/// One thing a file refers to, however often.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Reference {
    /// The view the file belongs to.
    pub from: ViewId,
    pub file: PathBuf,
    /// The file is in a read-only library: it cannot follow a rename.
    pub read_only: bool,
    pub kind: RefKind,
    /// As written in the file: the symbol reference, the structure name, the
    /// subcircuit or module name, `LIB/PCELL`, the `schematic=` value (empty for the
    /// `.sch` next to the symbol).
    pub text: String,
    /// How many times.
    pub count: usize,
    /// The instance names, where the file has them.
    pub instances: Vec<String>,
    pub target: Target,
}

/// The references of a project.
#[derive(Debug, Clone, Default)]
pub struct Index {
    pub references: Vec<Reference>,
    /// What looked wrong while reading: files that could not be read, libraries whose
    /// folders share a name.
    pub warnings: Vec<String>,
    by_file: HashMap<PathBuf, Vec<usize>>,
    files: HashMap<PathBuf, ViewId>,
}

// ---------------------------------------------------------------------------------
// Reading files
// ---------------------------------------------------------------------------------

/// Byte ranges of the symbol references of an xschem file: for every `C {…}` record
/// outside embedded symbols, the text between the braces of its first group.
pub(crate) fn instance_ranges(text: &str) -> Vec<Range<usize>> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut embedded = 0usize;
    let mut line_start = true;
    let mut tag = 0u8;
    let mut first_group = false;
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'\n' {
            line_start = true;
            i += 1;
            continue;
        }
        if line_start && !c.is_ascii_whitespace() {
            line_start = false;
            tag = c;
            first_group = true;
            match c {
                b'[' => embedded += 1,
                b']' => embedded = embedded.saturating_sub(1),
                _ => {}
            }
        }
        if c != b'{' {
            i += 1;
            continue;
        }
        let start = i + 1;
        let mut end = start;
        while end < bytes.len() && bytes[end] != b'}' {
            end += if bytes[end] == b'\\' { 2 } else { 1 };
        }
        let end = end.min(bytes.len());
        if first_group && embedded == 0 && tag == b'C' {
            out.push(start..end);
        }
        first_group = false;
        i = end + 1;
    }
    out
}

/// What a line of a SPICE netlist names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SpiceKind {
    /// `.subckt name …`
    Subckt,
    /// `.ends name`
    Ends,
    /// `Xname … subckt [param=value …]`
    Instance,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SpiceItem {
    pub kind: SpiceKind,
    /// The subcircuit name as written.
    pub name: String,
    /// Where the name is in the text.
    pub range: Range<usize>,
    /// The instance name (`X1`), for instances.
    pub instance: String,
}

/// The subcircuit definitions and instances of a SPICE or CDL netlist. Continuation
/// lines (`+`) are joined; comments (`*` lines, `;` and `$ ` to the end of the line)
/// are skipped.
pub(crate) fn spice_items(text: &str) -> Vec<SpiceItem> {
    // Logical lines as lists of word ranges.
    let mut lines: Vec<Vec<Range<usize>>> = Vec::new();
    let mut offset = 0;
    for raw in text.split_inclusive('\n') {
        let start = offset;
        offset += raw.len();
        let trimmed = raw.trim_start();
        let lead = raw.len() - trimmed.len();
        if trimmed.starts_with('*') || trimmed.trim().is_empty() {
            continue;
        }
        let continuation = trimmed.starts_with('+');
        let body_start = start + lead + usize::from(continuation);
        let body = &text[body_start..offset];
        // Cut the comment at the end of the line.
        let cut = body
            .char_indices()
            .find(|(i, c)| {
                *c == ';'
                    || (*c == '$'
                        && body[i + 1..].starts_with([' ', '\t'])
                        && (*i == 0 || body[..*i].ends_with([' ', '\t'])))
            })
            .map_or(body.len(), |(i, _)| i);
        let body = &body[..cut];
        let mut words = Vec::new();
        let mut word: Option<usize> = None;
        for (i, c) in body.char_indices() {
            let separator = c.is_whitespace() || matches!(c, '(' | ')' | ',');
            match (separator, word) {
                (false, None) => word = Some(i),
                (true, Some(s)) => {
                    words.push(body_start + s..body_start + i);
                    word = None;
                }
                _ => {}
            }
        }
        if let Some(s) = word {
            words.push(body_start + s..body_start + body.len());
        }
        match (continuation, lines.last_mut()) {
            (true, Some(last)) => last.extend(words),
            _ => lines.push(words),
        }
    }

    let mut out = Vec::new();
    for words in lines {
        let Some(first) = words.first() else {
            continue;
        };
        let head = text[first.clone()].to_ascii_lowercase();
        if head == ".subckt" || head == ".ends" {
            if let Some(name) = words.get(1) {
                out.push(SpiceItem {
                    kind: if head == ".subckt" {
                        SpiceKind::Subckt
                    } else {
                        SpiceKind::Ends
                    },
                    name: text[name.clone()].to_string(),
                    range: name.clone(),
                    instance: String::new(),
                });
            }
            continue;
        }
        if !head.starts_with('x') {
            continue;
        }
        // The subcircuit name is the last word before the parameters.
        let mut end = words.len();
        for (n, w) in words.iter().enumerate().skip(1) {
            let word = &text[w.clone()];
            if word == "=" {
                end = n.saturating_sub(1);
                break;
            }
            if word.contains('=') || word.eq_ignore_ascii_case("params:") {
                end = if word.starts_with('=') {
                    n.saturating_sub(1)
                } else {
                    n
                };
                break;
            }
        }
        if end < 2 {
            continue;
        }
        let name = &words[end - 1];
        out.push(SpiceItem {
            kind: SpiceKind::Instance,
            name: text[name.clone()].to_string(),
            range: name.clone(),
            instance: text[first.clone()].to_string(),
        });
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VerilogKind {
    /// `module name`
    Module,
    /// `name [#(…)] instance (`
    Instance,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VerilogItem {
    pub kind: VerilogKind,
    /// The module name.
    pub name: String,
    pub range: Range<usize>,
    pub instance: String,
}

const VERILOG_KEYWORDS: &[&str] = &[
    "always",
    "always_comb",
    "always_ff",
    "always_latch",
    "and",
    "assert",
    "assign",
    "automatic",
    "begin",
    "bit",
    "buf",
    "bufif0",
    "bufif1",
    "byte",
    "case",
    "casex",
    "casez",
    "cmos",
    "const",
    "defparam",
    "else",
    "end",
    "endcase",
    "endfunction",
    "endgenerate",
    "endmodule",
    "endtask",
    "enum",
    "event",
    "for",
    "forever",
    "function",
    "generate",
    "genvar",
    "if",
    "import",
    "initial",
    "inout",
    "input",
    "int",
    "integer",
    "localparam",
    "logic",
    "longint",
    "macromodule",
    "module",
    "nand",
    "negedge",
    "nmos",
    "nor",
    "not",
    "notif0",
    "notif1",
    "or",
    "output",
    "parameter",
    "pmos",
    "posedge",
    "pulldown",
    "pullup",
    "real",
    "realtime",
    "reg",
    "repeat",
    "return",
    "shortint",
    "signed",
    "static",
    "string",
    "struct",
    "supply0",
    "supply1",
    "task",
    "time",
    "tran",
    "tri",
    "tri0",
    "tri1",
    "typedef",
    "unsigned",
    "wand",
    "while",
    "wire",
    "wor",
    "xnor",
    "xor",
];

/// The module definitions and instances of a Verilog file. A simple scanner: comments
/// and strings are blanked, then `module <name>` and `<name> [#(…)] <instance> [\[…\]]
/// (` after a `;`, `)` or a block keyword are picked up.
pub(crate) fn verilog_items(text: &str) -> Vec<VerilogItem> {
    // Blank comments and strings, keeping every byte offset.
    let mut clean = text.as_bytes().to_vec();
    let mut i = 0;
    while i < clean.len() {
        let rest = &clean[i..];
        if rest.starts_with(b"//") {
            while i < clean.len() && clean[i] != b'\n' {
                clean[i] = b' ';
                i += 1;
            }
        } else if rest.starts_with(b"/*") {
            let end = rest
                .windows(2)
                .position(|w| w == b"*/")
                .map_or(clean.len(), |p| i + p + 2);
            for b in &mut clean[i..end] {
                if *b != b'\n' {
                    *b = b' ';
                }
            }
            i = end;
        } else if clean[i] == b'"' {
            i += 1;
            while i < clean.len() && clean[i] != b'"' {
                if clean[i] == b'\\' && i + 1 < clean.len() {
                    clean[i] = b' ';
                    i += 1;
                }
                if clean[i] != b'\n' {
                    clean[i] = b' ';
                }
                i += 1;
            }
            i += 1;
        } else {
            i += 1;
        }
    }

    let is_ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'$';
    let skip_space = |mut i: usize| {
        while i < clean.len() && clean[i].is_ascii_whitespace() {
            i += 1;
        }
        i
    };
    let ident = |i: usize| -> Option<Range<usize>> {
        if i < clean.len() && (clean[i].is_ascii_alphabetic() || clean[i] == b'_') {
            let mut end = i;
            while end < clean.len() && is_ident(clean[end]) {
                end += 1;
            }
            Some(i..end)
        } else if i < clean.len() && clean[i] == b'\\' {
            // An escaped identifier runs to the next blank.
            let mut end = i;
            while end < clean.len() && !clean[end].is_ascii_whitespace() {
                end += 1;
            }
            Some(i..end)
        } else {
            None
        }
    };
    // After `open` at `i`: the index after its matching close.
    let balanced = |mut i: usize, open: u8, close: u8| -> Option<usize> {
        let mut depth = 0usize;
        while i < clean.len() {
            if clean[i] == open {
                depth += 1;
            } else if clean[i] == close {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            i += 1;
        }
        None
    };
    let word = |r: &Range<usize>| std::str::from_utf8(&clean[r.clone()]).unwrap_or("");

    let mut out = Vec::new();
    // A statement may start here: after `;`, `)`, a block keyword, or at the top.
    let mut can_start = true;
    let mut i = 0;
    while i < clean.len() {
        let c = clean[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let Some(first) = ident(i) else {
            can_start = matches!(c, b';' | b')');
            i += 1;
            continue;
        };
        let name = word(&first).to_string();
        i = first.end;
        if name == "module" || name == "macromodule" {
            let at = skip_space(i);
            if let Some(module) = ident(at) {
                out.push(VerilogItem {
                    kind: VerilogKind::Module,
                    name: word(&module).to_string(),
                    range: module.clone(),
                    instance: String::new(),
                });
                i = module.end;
            }
            can_start = false;
            continue;
        }
        if VERILOG_KEYWORDS.contains(&name.as_str()) {
            can_start = matches!(
                name.as_str(),
                "begin" | "end" | "else" | "generate" | "endgenerate" | "endmodule"
            );
            continue;
        }
        if !std::mem::replace(&mut can_start, false) {
            continue;
        }
        // `name [#(…)] instance [range] (`
        let mut at = skip_space(i);
        if at < clean.len() && clean[at] == b'#' {
            let open = skip_space(at + 1);
            if open < clean.len() && clean[open] == b'(' {
                match balanced(open, b'(', b')') {
                    Some(end) => at = skip_space(end),
                    None => continue,
                }
            } else {
                continue;
            }
        }
        let Some(instance) = ident(at) else {
            continue;
        };
        if VERILOG_KEYWORDS.contains(&word(&instance)) {
            continue;
        }
        let mut after = skip_space(instance.end);
        if after < clean.len() && clean[after] == b'[' {
            match balanced(after, b'[', b']') {
                Some(end) => after = skip_space(end),
                None => continue,
            }
        }
        if after < clean.len() && clean[after] == b'(' {
            out.push(VerilogItem {
                kind: VerilogKind::Instance,
                name,
                range: first,
                instance: word(&instance).to_string(),
            });
            i = instance.end;
        }
    }
    out
}

/// A PCell placed in a GDSII file, from KLayout's context structure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PcellContext {
    /// The proxy structure that holds the PCell's geometry.
    pub proxy: String,
    pub library: String,
    pub pcell: String,
}

/// Walk the records of a GDSII stream: `(type, data, whole record)`.
pub(crate) fn gds_records(bytes: &[u8]) -> impl Iterator<Item = (u16, &[u8], &[u8])> {
    let mut pos = 0;
    std::iter::from_fn(move || {
        if pos + 4 > bytes.len() {
            return None;
        }
        let len = u16::from_be_bytes([bytes[pos], bytes[pos + 1]]) as usize;
        if len < 4 || pos + len > bytes.len() {
            return None;
        }
        let kind = u16::from_be_bytes([bytes[pos + 2], bytes[pos + 3]]);
        let record = &bytes[pos..pos + len];
        pos += len;
        Some((kind, &record[4..], record))
    })
}

pub(crate) const GDS_STRNAME: u16 = 0x0606;
pub(crate) const GDS_ENDSTR: u16 = 0x0700;
pub(crate) const GDS_SREF: u16 = 0x0a00;
pub(crate) const GDS_ENDEL: u16 = 0x1100;
pub(crate) const GDS_SNAME: u16 = 0x1206;
pub(crate) const GDS_PROPVALUE: u16 = 0x2c06;

pub(crate) fn gds_text(data: &[u8]) -> String {
    let end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
    String::from_utf8_lossy(&data[..end]).into_owned()
}

/// The library PCells a GDSII file places: KLayout writes, in a structure named
/// `$$$CONTEXT_INFO$$$`, one reference per proxy cell with `LIB=…` and `PCELL=…`
/// among its properties.
pub(crate) fn pcell_contexts(bytes: &[u8]) -> Vec<PcellContext> {
    let mut out = Vec::new();
    let mut inside = false;
    let mut current: Option<PcellContext> = None;
    for (kind, data, _) in gds_records(bytes) {
        match kind {
            GDS_STRNAME => inside = gds_text(data) == CONTEXT_INFO,
            GDS_ENDSTR => inside = false,
            GDS_SREF if inside => {
                current = Some(PcellContext {
                    proxy: String::new(),
                    library: String::new(),
                    pcell: String::new(),
                })
            }
            GDS_SNAME => {
                if let Some(c) = &mut current {
                    c.proxy = gds_text(data);
                }
            }
            GDS_PROPVALUE => {
                if let Some(c) = &mut current {
                    let value = gds_text(data);
                    if let Some(lib) = value.strip_prefix("LIB=") {
                        c.library = lib.to_string();
                    } else if let Some(pcell) = value.strip_prefix("PCELL=") {
                        c.pcell = pcell.to_string();
                    }
                }
            }
            GDS_ENDEL => {
                if let Some(c) = current.take()
                    && !c.library.is_empty()
                    && !c.pcell.is_empty()
                {
                    out.push(c);
                }
            }
            _ => {}
        }
    }
    out
}

fn read_text(file: &Path, warnings: &mut Vec<String>) -> Option<String> {
    match std::fs::metadata(file) {
        Ok(m) if m.len() > MAX_TEXT => {
            warnings.push(format!(
                "{} is larger than {} MB: not read",
                file.display(),
                MAX_TEXT / (1024 * 1024)
            ));
            return None;
        }
        Err(e) => {
            warnings.push(format!("cannot read {}: {e}", file.display()));
            return None;
        }
        Ok(_) => {}
    }
    match std::fs::read(file) {
        Ok(bytes) => Some(String::from_utf8_lossy(&bytes).into_owned()),
        Err(e) => {
            warnings.push(format!("cannot read {}: {e}", file.display()));
            None
        }
    }
}

/// The design files of a view, with their formats.
pub(crate) fn design_files(entry: &Entry) -> Vec<(PathBuf, Format)> {
    entry
        .files
        .iter()
        .filter_map(|f| Some((f.clone(), Format::of_file(f)?)))
        .collect()
}

/// `ota` for `…/ota.sym`.
pub(crate) fn stem(file: &Path) -> String {
    file.file_name()
        .and_then(|n| n.to_str())
        .and_then(Format::split_name)
        .map(|(stem, _)| stem.to_string())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------------
// Building the index
// ---------------------------------------------------------------------------------

/// A subcircuit or module definition.
#[derive(Debug, Clone)]
struct Definition {
    id: ViewId,
    file: PathBuf,
}

struct Builder<'a> {
    project: &'a Project,
    files: HashMap<PathBuf, ViewId>,
    search: Vec<PathBuf>,
    cache: HashMap<String, Target>,
    subckts: HashMap<String, Vec<Definition>>,
    modules: HashMap<String, Vec<Definition>>,
    /// Text of the netlists and HDL files, read once.
    texts: HashMap<PathBuf, String>,
    references: Vec<Reference>,
    warnings: Vec<String>,
}

/// A reference being counted.
#[derive(Default)]
struct Tally {
    count: usize,
    instances: Vec<String>,
}

impl Builder<'_> {
    /// Where an xschem reference (a symbol, or a `schematic=` value) leads: absolute,
    /// else the first folder of the search path that has it.
    fn xschem(&mut self, reference: &str) -> Target {
        if let Some(found) = self.cache.get(reference) {
            return found.clone();
        }
        let path = Path::new(reference);
        let candidates: Vec<PathBuf> = if path.is_absolute() {
            vec![path.to_path_buf()]
        } else {
            self.search.iter().map(|dir| dir.join(path)).collect()
        };
        let mut target = Target::Unresolved;
        for candidate in candidates {
            if let Some(id) = self.files.get(&candidate) {
                target = Target::View {
                    id: id.clone(),
                    file: candidate,
                };
                break;
            }
            if candidate.is_file() {
                target = Target::External(candidate);
                break;
            }
        }
        self.cache.insert(reference.to_string(), target.clone());
        target
    }

    fn file_target(&self, file: &Path) -> Target {
        match self.files.get(file) {
            Some(id) => Target::View {
                id: id.clone(),
                file: file.to_path_buf(),
            },
            None => Target::External(file.to_path_buf()),
        }
    }

    fn push(
        &mut self,
        from: &ViewId,
        file: &Path,
        read_only: bool,
        kind: RefKind,
        tallies: BTreeMap<String, Tally>,
        target: impl Fn(&mut Self, &str) -> Target,
    ) {
        for (text, tally) in tallies {
            let target = target(self, &text);
            self.references.push(Reference {
                from: from.clone(),
                file: file.to_path_buf(),
                read_only,
                kind,
                text,
                count: tally.count,
                instances: tally.instances,
                target,
            });
        }
    }

    fn xschem_file(&mut self, from: &ViewId, file: &Path, format: Format, read_only: bool) {
        let Some(text) = read_text(file, &mut self.warnings) else {
            return;
        };
        let mut placed: BTreeMap<String, Tally> = BTreeMap::new();
        for instance in schematic::parse(&text) {
            let tally = placed.entry(instance.symbol).or_default();
            tally.count += 1;
            if !instance.name.is_empty() {
                tally.instances.push(instance.name);
            }
        }
        self.push(from, file, read_only, RefKind::Instance, placed, |b, t| {
            b.xschem(t)
        });
        if format != Format::XschemSymbol {
            return;
        }
        // The schematic the symbol stands for.
        let (written, target) = match symbol::link(&text, &stem(file)) {
            Link::Default => {
                let beside = file.with_extension("sch");
                if !beside.is_file() {
                    return;
                }
                (String::new(), self.file_target(&beside))
            }
            Link::Path(path) => (
                symbol::schematic(&text).unwrap_or_default(),
                self.xschem(&path),
            ),
            // Tcl, generators: only xschem can tell.
            Link::Dynamic(_) => return,
        };
        self.references.push(Reference {
            from: from.clone(),
            file: file.to_path_buf(),
            read_only,
            kind: RefKind::SchematicLink,
            text: written,
            count: 1,
            instances: Vec::new(),
            target,
        });
    }

    fn layout_file(&mut self, from: &ViewId, file: &Path, read_only: bool) {
        if !layout::is_readable(file) {
            return;
        }
        // The cells of other views it places.
        let resolved = layout::refs::resolve_file(self.project, file);
        let mut counts: HashMap<String, Tally> = HashMap::new();
        match layout::read_placements(file, crate::link::NAME_PROPERTY) {
            Ok(placements) => {
                for p in placements {
                    if p.structure == CONTEXT_INFO {
                        continue;
                    }
                    let tally = counts.entry(p.cell).or_default();
                    tally.count += 1;
                    tally.instances.extend(p.value);
                }
            }
            Err(e) => self.warnings.push(format!("{e:#}")),
        }
        for (name, r) in &resolved.refs {
            let tally = counts.remove(name).unwrap_or_default();
            let target = match &r.file {
                Some(target) => self.file_target(target),
                None => Target::Unresolved,
            };
            self.references.push(Reference {
                from: from.clone(),
                file: file.to_path_buf(),
                read_only,
                kind: RefKind::Placement,
                text: name.clone(),
                count: tally.count.max(1),
                instances: tally.instances,
                target,
            });
        }
        // The PCells (GDSII only: OASIS keeps the context in properties).
        if !layout::is_oasis(file)
            && let Ok(bytes) = std::fs::read(file)
        {
            let mut pcells: BTreeMap<String, Tally> = BTreeMap::new();
            for context in pcell_contexts(&bytes) {
                let placed = counts.get(&context.proxy).map_or(1, |t| t.count.max(1));
                pcells
                    .entry(format!("{}/{}", context.library, context.pcell))
                    .or_default()
                    .count += placed;
            }
            self.push(from, file, read_only, RefKind::Pcell, pcells, |b, text| {
                let (library, pcell) = text.split_once('/').unwrap_or((text, ""));
                b.pcell(library, pcell)
            });
        }
        // The schematic it is linked with.
        if let Ok(Some(meta)) = Meta::read_for(file)
            && let Some(source) = &meta.source
        {
            let target = self
                .view_file(&source.library, &source.cell, &source.view)
                .map_or(Target::Unresolved, |file| Target::View {
                    id: ViewId::new(&source.library, &source.cell, &source.view),
                    file,
                });
            self.references.push(Reference {
                from: from.clone(),
                file: file.to_path_buf(),
                read_only,
                kind: RefKind::Source,
                text: source.label(),
                count: 1,
                instances: Vec::new(),
                target,
            });
        }
    }

    fn view_file(&self, library: &str, cell: &str, view: &str) -> Option<PathBuf> {
        let entry = self.project.library(library)?.cell(cell)?.entry(view)?;
        entry.main_file.clone()
    }

    /// A PCell `library/pcell` as KLayout names it: a PCell view of the project when
    /// `library` is one of its libraries.
    fn pcell(&self, library: &str, pcell: &str) -> Target {
        let Some(lib) = self.project.library(library) else {
            return Target::Tool(library.to_string());
        };
        let view = lib.cell(pcell).and_then(|cell| {
            cell.entries
                .iter()
                .find(|e| e.format == Some(Format::KlayoutPcell))
        });
        match view.and_then(|e| Some((e, e.main_file.clone()?))) {
            Some((entry, file)) => Target::View {
                id: ViewId::new(library, pcell, &entry.name),
                file,
            },
            None => Target::Unresolved,
        }
    }

    /// A subcircuit or module by name: a definition in the same library first, then
    /// any definition, then a cell of that name.
    fn named(
        &self,
        from: &ViewId,
        name: &str,
        definitions: &HashMap<String, Vec<Definition>>,
        key: &str,
    ) -> Target {
        if let Some(found) = definitions.get(key) {
            let best = found
                .iter()
                .find(|d| d.id.library == from.library)
                .or(found.first());
            if let Some(d) = best {
                return Target::View {
                    id: d.id.clone(),
                    file: d.file.clone(),
                };
            }
        }
        let same = |cell: &&Cell| cell.name == name || cell.name.eq_ignore_ascii_case(key);
        let own = self.project.library(&from.library);
        own.into_iter()
            .chain(self.project.libraries.iter())
            .find_map(|lib| {
                let cell = lib.cells.iter().find(same)?;
                Some(Target::Cell {
                    library: lib.name.clone(),
                    cell: cell.name.clone(),
                })
            })
            .unwrap_or(Target::Unresolved)
    }

    fn netlist_file(&mut self, from: &ViewId, file: &Path, read_only: bool) {
        let Some(text) = self.texts.get(file) else {
            return;
        };
        let items = spice_items(text);
        let own: BTreeSet<String> = items
            .iter()
            .filter(|i| i.kind == SpiceKind::Subckt)
            .map(|i| i.name.to_ascii_lowercase())
            .collect();
        // SPICE names ignore case: `inv` and `INV` are one subcircuit, shown as first
        // written.
        let mut used: BTreeMap<String, Tally> = BTreeMap::new();
        let mut written: HashMap<String, String> = HashMap::new();
        for item in items {
            let lower = item.name.to_ascii_lowercase();
            if item.kind == SpiceKind::Instance && !own.contains(&lower) {
                let name = written.entry(lower).or_insert(item.name).clone();
                let tally = used.entry(name).or_default();
                tally.count += 1;
                tally.instances.push(item.instance);
            }
        }
        let from_id = from.clone();
        self.push(from, file, read_only, RefKind::Subckt, used, |b, name| {
            b.named(&from_id, name, &b.subckts, &name.to_ascii_lowercase())
        });
    }

    fn verilog_file(&mut self, from: &ViewId, file: &Path, read_only: bool) {
        let Some(text) = self.texts.get(file) else {
            return;
        };
        let items = verilog_items(text);
        let own: BTreeSet<&str> = items
            .iter()
            .filter(|i| i.kind == VerilogKind::Module)
            .map(|i| i.name.as_str())
            .collect();
        let mut used: BTreeMap<String, Tally> = BTreeMap::new();
        for item in &items {
            if item.kind == VerilogKind::Instance && !own.contains(item.name.as_str()) {
                let tally = used.entry(item.name.clone()).or_default();
                tally.count += 1;
                tally.instances.push(item.instance.clone());
            }
        }
        let from_id = from.clone();
        self.push(from, file, read_only, RefKind::Module, used, |b, name| {
            b.named(&from_id, name, &b.modules, name)
        });
    }
}

fn is_netlist(format: Format) -> bool {
    matches!(format, Format::Spice | Format::Cdl)
}

fn is_verilog(format: Format) -> bool {
    matches!(format, Format::Verilog | Format::SystemVerilog)
}

/// Libraries whose folders have the same name: xschem names a library by its folder,
/// so only the first is found for references `folder/…`. One sentence per name.
pub fn shadowed_libraries(project: &Project) -> Vec<String> {
    let mut by_folder: BTreeMap<String, Vec<&Library>> = BTreeMap::new();
    for library in project.libraries.iter().filter(|l| l.error.is_none()) {
        if let Some(name) = library.path.file_name() {
            by_folder
                .entry(name.to_string_lossy().into_owned())
                .or_default()
                .push(library);
        }
    }
    by_folder
        .into_iter()
        .filter(|(_, libs)| {
            let folders: BTreeSet<&Path> = libs.iter().map(|l| l.path.as_path()).collect();
            // Two reference libraries (a PDK's) are not the user's to rename.
            folders.len() > 1 && libs.iter().any(|l| !l.read_only)
        })
        .map(|(folder, libs)| {
            let list: Vec<String> = libs
                .iter()
                .map(|l| format!("'{}' ({})", l.name, l.path.display()))
                .collect();
            format!(
                "libraries {} have folders with the same name: xschem names a library by \
                 its folder, so references `{folder}/…` find only the first; rename one \
                 of the folders",
                list.join(" and ")
            )
        })
        .collect()
}

impl Index {
    /// Read the references of every library of the project.
    pub fn build(project: &Project) -> Index {
        let mut files = HashMap::new();
        for lib in &project.libraries {
            for cell in &lib.cells {
                for entry in &cell.entries {
                    let id = ViewId::new(&lib.name, &cell.name, &entry.name);
                    for file in &entry.files {
                        files.entry(file.clone()).or_insert_with(|| id.clone());
                    }
                }
            }
        }
        let mut builder = Builder {
            project,
            files,
            search: schematic::search_path(project),
            cache: HashMap::new(),
            subckts: HashMap::new(),
            modules: HashMap::new(),
            texts: HashMap::new(),
            references: Vec::new(),
            warnings: shadowed_libraries(project),
        };

        // What there is to read: (view, file, format, read-only).
        let mut todo = Vec::new();
        for lib in project.libraries.iter().filter(|l| l.error.is_none()) {
            for cell in &lib.cells {
                for entry in &cell.entries {
                    let id = ViewId::new(&lib.name, &cell.name, &entry.name);
                    for (file, format) in design_files(entry) {
                        todo.push((id.clone(), file, format, lib.read_only));
                    }
                }
            }
        }
        // Definitions first: a netlist may use a subcircuit of a later file.
        for (id, file, format, _) in &todo {
            if !is_netlist(*format) && !is_verilog(*format) {
                continue;
            }
            let Some(text) = read_text(file, &mut builder.warnings) else {
                continue;
            };
            let definition = || Definition {
                id: id.clone(),
                file: file.clone(),
            };
            if is_netlist(*format) {
                for item in spice_items(&text) {
                    if item.kind == SpiceKind::Subckt {
                        builder
                            .subckts
                            .entry(item.name.to_ascii_lowercase())
                            .or_default()
                            .push(definition());
                    }
                }
            } else {
                for item in verilog_items(&text) {
                    if item.kind == VerilogKind::Module {
                        builder
                            .modules
                            .entry(item.name)
                            .or_default()
                            .push(definition());
                    }
                }
            }
            builder.texts.insert(file.clone(), text);
        }
        for (id, file, format, read_only) in &todo {
            match format {
                Format::XschemSchematic | Format::XschemSymbol => {
                    builder.xschem_file(id, file, *format, *read_only)
                }
                Format::Gds | Format::Oasis if !read_only => {
                    builder.layout_file(id, file, *read_only)
                }
                f if is_netlist(*f) => builder.netlist_file(id, file, *read_only),
                f if is_verilog(*f) => builder.verilog_file(id, file, *read_only),
                _ => {}
            }
        }

        let mut by_file: HashMap<PathBuf, Vec<usize>> = HashMap::new();
        for (n, r) in builder.references.iter().enumerate() {
            by_file.entry(r.file.clone()).or_default().push(n);
        }
        Index {
            references: builder.references,
            warnings: builder.warnings,
            by_file,
            files: builder.files,
        }
    }

    /// The view a file of the project belongs to.
    pub fn view_of(&self, file: &Path) -> Option<&ViewId> {
        self.files.get(file)
    }

    /// What a file refers to.
    pub fn from_file(&self, file: &Path) -> impl Iterator<Item = &Reference> {
        self.by_file
            .get(file)
            .into_iter()
            .flatten()
            .map(|n| &self.references[*n])
    }

    /// What the views of a cell refer to.
    pub fn uses(&self, library: &str, cell: &str) -> Vec<&Reference> {
        self.references
            .iter()
            .filter(|r| r.from.is_cell(library, cell))
            .collect()
    }

    /// Every reference to a cell from another cell (optionally to one of its views
    /// only). A reference found by name (subcircuits, modules) counts for every view.
    pub fn used_by(&self, library: &str, cell: &str, view: Option<&str>) -> Vec<&Reference> {
        self.references
            .iter()
            .filter(|r| !r.from.is_cell(library, cell))
            .filter(|r| match &r.target {
                Target::View { id, .. } => {
                    id.is_cell(library, cell) && view.is_none_or(|v| id.view == v)
                }
                Target::Cell {
                    library: l,
                    cell: c,
                } => l == library && c == cell,
                _ => false,
            })
            .collect()
    }

    /// References that lead nowhere, from views of writable libraries. Subcircuits and
    /// modules are left out: models and primitives of the PDK are not in the project.
    pub fn unresolved(&self) -> Vec<&Reference> {
        self.references
            .iter()
            .filter(|r| r.target.is_unresolved() && !r.read_only)
            .filter(|r| !matches!(r.kind, RefKind::Subckt | RefKind::Module))
            .collect()
    }
}

// ---------------------------------------------------------------------------------
// The hierarchy below a cell
// ---------------------------------------------------------------------------------

/// Which hierarchy: through schematics, layouts, netlists or HDL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TreeKind {
    Schematic,
    Layout,
    Netlist,
    Hdl,
}

impl TreeKind {
    pub const ALL: [TreeKind; 4] = [
        TreeKind::Schematic,
        TreeKind::Layout,
        TreeKind::Netlist,
        TreeKind::Hdl,
    ];

    pub fn name(self) -> &'static str {
        match self {
            TreeKind::Schematic => "schematic",
            TreeKind::Layout => "layout",
            TreeKind::Netlist => "netlist",
            TreeKind::Hdl => "hdl",
        }
    }

    pub fn from_name(name: &str) -> Option<TreeKind> {
        Self::ALL.into_iter().find(|k| k.name() == name)
    }

    fn view_kind(self) -> ViewKind {
        match self {
            TreeKind::Schematic => ViewKind::Schematic,
            TreeKind::Layout => ViewKind::Layout,
            TreeKind::Netlist => ViewKind::Netlist,
            TreeKind::Hdl => ViewKind::Hdl,
        }
    }

    fn follows(self, kind: RefKind) -> bool {
        matches!(
            (self, kind),
            (TreeKind::Schematic, RefKind::Instance)
                | (TreeKind::Layout, RefKind::Placement | RefKind::Pcell)
                | (TreeKind::Netlist, RefKind::Subckt)
                | (TreeKind::Hdl, RefKind::Module)
        )
    }

    /// The view of `cell` this hierarchy goes through: the one named after the kind
    /// first.
    pub fn view_of(self, cell: &Cell) -> Option<&Entry> {
        let mut views = cell
            .entries
            .iter()
            .filter(|e| e.kind == self.view_kind() && e.main_file.is_some())
            .filter(|e| self != TreeKind::Schematic || e.format == Some(Format::XschemSchematic));
        views
            .clone()
            .find(|e| e.name == self.name())
            .or_else(|| views.next())
    }
}

/// What a node of the tree is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum NodeKind {
    /// A cell of the project with a view of the tree's kind: it may have a level below.
    Cell,
    /// A cell that contains itself, directly or not: not unfolded again.
    Recursive,
    /// A device, a symbol without schematic, a PCell: nothing below.
    Primitive,
    /// The reference leads nowhere.
    Missing,
}

/// One cell of the hierarchy, under the cell that uses it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Node {
    /// The cell name, or the reference as written for what is not a cell.
    pub name: String,
    pub kind: NodeKind,
    /// The view the tree goes through, for cells.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view: Option<ViewId>,
    /// The file of that view (or the external file).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<PathBuf>,
    /// How many times the parent places it.
    pub count: usize,
    /// The instance names in the parent, where known.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub instances: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Node>,
}

impl Node {
    /// The text of the node's line: `inv ×2 (x1, x2)`.
    pub fn label(&self) -> String {
        let mut text = self.name.clone();
        if self.count > 1 {
            text.push_str(&format!(" ×{}", self.count));
        }
        if !self.instances.is_empty() {
            const SHOWN: usize = 6;
            let mut names: Vec<&str> = self.instances.iter().map(String::as_str).collect();
            let more = names.len().saturating_sub(SHOWN);
            names.truncate(SHOWN);
            text.push_str(&format!(" ({}", names.join(", ")));
            if more > 0 {
                text.push_str(&format!(", +{more}"));
            }
            text.push(')');
        }
        match self.kind {
            NodeKind::Cell | NodeKind::Primitive => {}
            NodeKind::Recursive => text.push_str(" (contains itself)"),
            NodeKind::Missing => text.push_str(" (cannot be found)"),
        }
        text
    }

    /// The tree as indented lines, this node first. `all`: with the primitives.
    pub fn lines(&self, all: bool) -> Vec<String> {
        fn walk(node: &Node, depth: usize, all: bool, out: &mut Vec<String>) {
            for child in &node.children {
                if child.kind == NodeKind::Primitive && !all {
                    continue;
                }
                out.push(format!("{}{}", "  ".repeat(depth), child.label()));
                walk(child, depth + 1, all, out);
            }
        }
        let mut out = vec![self.name.clone()];
        walk(self, 1, all, &mut out);
        out
    }

    /// Number of nodes below this one.
    pub fn size(&self) -> usize {
        self.children.iter().map(|c| 1 + c.size()).sum()
    }
}

impl Index {
    /// The hierarchy below `library/cell` through its views of `kind`; `None` when the
    /// cell has no such view.
    pub fn tree(
        &self,
        project: &Project,
        library: &str,
        cell: &str,
        kind: TreeKind,
    ) -> Option<Node> {
        let found = project.library(library)?.cell(cell)?;
        let entry = kind.view_of(found)?;
        let file = entry.main_file.clone()?;
        let mut budget = MAX_NODES;
        let mut path = vec![file.clone()];
        let children = self.children(project, kind, &file, &mut path, &mut budget);
        Some(Node {
            name: cell.to_string(),
            kind: NodeKind::Cell,
            view: Some(ViewId::new(library, cell, &entry.name)),
            file: Some(file),
            count: 1,
            instances: Vec::new(),
            children,
        })
    }

    /// The view of the tree's kind behind a reference, with its file.
    fn below(
        &self,
        project: &Project,
        kind: TreeKind,
        reference: &Reference,
    ) -> Option<(ViewId, PathBuf)> {
        let (library, cell) = reference.target.cell()?;
        if kind == TreeKind::Schematic {
            // Through the symbol to the schematic it stands for.
            if let Some(file) = reference.target.file() {
                let link = self
                    .from_file(file)
                    .find(|r| r.kind == RefKind::SchematicLink);
                if let Some(Target::View { id, file }) = link.map(|r| &r.target) {
                    return Some((id.clone(), file.clone()));
                }
            }
        } else if let Target::View { id, file } = &reference.target
            && reference.kind != RefKind::Pcell
        {
            return Some((id.clone(), file.clone()));
        }
        if reference.kind == RefKind::Pcell {
            return None;
        }
        let found = project.library(library)?.cell(cell)?;
        let entry = kind.view_of(found)?;
        Some((
            ViewId::new(library, cell, &entry.name),
            entry.main_file.clone()?,
        ))
    }

    fn children(
        &self,
        project: &Project,
        kind: TreeKind,
        file: &Path,
        path: &mut Vec<PathBuf>,
        budget: &mut usize,
    ) -> Vec<Node> {
        let mut out = Vec::new();
        for reference in self.from_file(file).filter(|r| kind.follows(r.kind)) {
            if *budget == 0 {
                break;
            }
            *budget -= 1;
            let mut node = Node {
                name: reference.text.clone(),
                kind: NodeKind::Primitive,
                view: None,
                file: reference.target.file().map(Path::to_path_buf),
                count: reference.count,
                instances: reference.instances.clone(),
                children: Vec::new(),
            };
            if let Some((_, cell)) = reference.target.cell() {
                node.name = cell.to_string();
            }
            match self.below(project, kind, reference) {
                Some((id, below)) => {
                    node.name = id.cell.clone();
                    node.view = Some(id);
                    node.file = Some(below.clone());
                    if path.contains(&below) {
                        node.kind = NodeKind::Recursive;
                    } else {
                        node.kind = NodeKind::Cell;
                        path.push(below.clone());
                        node.children = self.children(project, kind, &below, path, budget);
                        path.pop();
                    }
                }
                None if reference.target.is_unresolved()
                    && !matches!(reference.kind, RefKind::Subckt | RefKind::Module) =>
                {
                    node.kind = NodeKind::Missing;
                }
                None => {}
            }
            out.push(node);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_references_are_located() {
        let text = "v {xschem version=3.4.5 file_version=1.2\n}\nG {}\nK {}\n\
                    N 0 0 10 0 {lab=A}\n\
                    C {logic/inv/symbol/inv.sym} 0 0 0 0 {name=x1\nnote=\"C {no.sym}\"}\n\
                    [\nC {inside.sym} 0 0 0 0 {}\n]\n\
                    C {ipin.sym} 1 2 0 0 {name=p1 lab=A}\n";
        let found: Vec<&str> = instance_ranges(text)
            .into_iter()
            .map(|r| &text[r])
            .collect();
        assert_eq!(found, ["logic/inv/symbol/inv.sym", "ipin.sym"]);
    }

    #[test]
    fn subcircuits_and_instances_of_a_netlist() {
        let text = "* a comment with x1 a b inv\n\
                    .subckt buf A Y VDD VSS\n\
                    x1 A n1 VDD VSS inv ; first\n\
                    X2 n1 Y\n\
                    + VDD VSS INV m=2 w = 1u\n\
                    XM1 d g s b sg13_lv_nmos w=1u l=0.13u\n\
                    R1 a b 1k\n\
                    x3 (a b) sub params: p=1\n\
                    .ends buf\n";
        let items = spice_items(text);
        let seen: Vec<(SpiceKind, &str, &str)> = items
            .iter()
            .map(|i| (i.kind, i.name.as_str(), i.instance.as_str()))
            .collect();
        assert_eq!(
            seen,
            [
                (SpiceKind::Subckt, "buf", ""),
                (SpiceKind::Instance, "inv", "x1"),
                (SpiceKind::Instance, "INV", "X2"),
                (SpiceKind::Instance, "sg13_lv_nmos", "XM1"),
                (SpiceKind::Instance, "sub", "x3"),
                (SpiceKind::Ends, "buf", ""),
            ]
        );
        for item in &items {
            assert_eq!(&text[item.range.clone()], item.name);
        }
    }

    #[test]
    fn modules_and_instances_of_a_verilog_file() {
        let text = "// inv u9 (a);\n\
                    module buf #(parameter N = 1) (input a, output y);\n\
                      wire n1; /* inv u8 (x); */\n\
                      inv u1 (.a(a), .y(n1));\n\
                      inv #(.W(2)) u2 (.a(n1), .y(y));\n\
                      and g1 (y, a, n1);\n\
                      assign y = f(a);\n\
                      generate if (N > 1) begin\n\
                        pad p [3:0] (.x(a));\n\
                      end endgenerate\n\
                      initial $display(\"inv u7 (\");\n\
                    endmodule\n\
                    module inv(input a, output y); assign y = ~a; endmodule\n";
        let items = verilog_items(text);
        let seen: Vec<(VerilogKind, &str, &str)> = items
            .iter()
            .map(|i| (i.kind, i.name.as_str(), i.instance.as_str()))
            .collect();
        assert_eq!(
            seen,
            [
                (VerilogKind::Module, "buf", ""),
                (VerilogKind::Instance, "inv", "u1"),
                (VerilogKind::Instance, "inv", "u2"),
                (VerilogKind::Instance, "pad", "p"),
                (VerilogKind::Module, "inv", ""),
            ]
        );
        for item in &items {
            assert_eq!(&text[item.range.clone()], item.name);
        }
    }

    #[test]
    fn pcells_are_read_from_the_context_structure() {
        use crate::layout::gds::tests::rec;
        let name = |t: &str| {
            let mut b = t.as_bytes().to_vec();
            if b.len() % 2 == 1 {
                b.push(0);
            }
            b
        };
        let mut gds = Vec::new();
        rec(&mut gds, 0x0502, &[0; 24]);
        rec(&mut gds, GDS_STRNAME, &name(CONTEXT_INFO));
        for (proxy, pcell) in [("rpoly$1", "rpoly"), ("CIRCLE", "CIRCLE")] {
            rec(&mut gds, GDS_SREF, &[]);
            rec(&mut gds, GDS_SNAME, &name(proxy));
            rec(&mut gds, 0x2b02, &1i16.to_be_bytes());
            rec(&mut gds, GDS_PROPVALUE, &name(&format!("PCELL={pcell}")));
            rec(&mut gds, 0x2b02, &0i16.to_be_bytes());
            let lib = if pcell == "rpoly" { "logic" } else { "Basic" };
            rec(&mut gds, GDS_PROPVALUE, &name(&format!("LIB={lib}")));
            rec(&mut gds, GDS_ENDEL, &[]);
        }
        rec(&mut gds, GDS_ENDSTR, &[]);
        rec(&mut gds, 0x0502, &[0; 24]);
        rec(&mut gds, GDS_STRNAME, &name("top"));
        rec(&mut gds, GDS_SREF, &[]);
        rec(&mut gds, GDS_SNAME, &name("rpoly$1"));
        rec(&mut gds, GDS_ENDEL, &[]);
        rec(&mut gds, GDS_ENDSTR, &[]);
        let found = pcell_contexts(&gds);
        assert_eq!(found.len(), 2);
        assert_eq!(
            (found[0].proxy.as_str(), found[0].library.as_str()),
            ("rpoly$1", "logic")
        );
        assert_eq!(found[1].pcell, "CIRCLE");
    }
}
