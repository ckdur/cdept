//! Taking the cells out of files that hold many: one self-contained text (or stream)
//! per cell. Everything here works on bytes in memory and is meant to be unit tested
//! with canned input.

use std::collections::BTreeSet;
use std::path::Path;

use eyre::{Result, eyre};

use crate::container::spice;
use crate::layout::gds;

/// One cell taken out of a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Piece {
    /// The cell's name in the source file.
    pub source: String,
    /// The cell's name in the piece (the source name unless it was renamed).
    pub name: String,
    pub bytes: Vec<u8>,
    /// What the piece says about itself, one sentence for the plan.
    pub what: String,
    /// Names of the other cells of the file that travel in the piece.
    pub travels: Vec<String>,
    /// Names the cell uses and the file does not define.
    pub unresolved: Vec<String>,
}

/// The cells of a file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Split {
    pub pieces: Vec<Piece>,
    /// A technology part that came with the cells (a tech LEF section), as a file of
    /// its own.
    pub tech: Option<Vec<u8>>,
    /// Things worth saying about the file as a whole.
    pub notes: Vec<String>,
}

/// Maps a name of the file to the name its cell gets.
pub type Names<'a> = &'a dyn Fn(&str) -> String;

fn travels(all: &[String], top: &str) -> Vec<String> {
    all.iter().filter(|n| *n != top).cloned().collect()
}

// ---------------------------------------------------------------------------------
// GDSII
// ---------------------------------------------------------------------------------

/// One piece per top structure (not placed by another one), with what it places.
pub fn gds(bytes: &[u8], names: Names) -> Result<Split> {
    let summary = gds::parse(bytes)?;
    let tops: Vec<String> = summary
        .top_cells()
        .into_iter()
        .filter(|n| !n.starts_with("$$$"))
        .collect();
    let mut split = Split::default();
    if tops.is_empty() {
        split
            .notes
            .push("the stream has no top structure (every structure is placed by another)".into());
    }
    for top in tops {
        let new = names(&top);
        let mut out = gds::extract_structure(bytes, &top)?;
        if new != top {
            let map = std::collections::BTreeMap::from([(top.clone(), new.clone())]);
            if let Some(renamed) = gds::rename_bytes(&out, &map)? {
                out = renamed;
            }
        }
        let inside = gds::parse(&out)?;
        let others: Vec<String> = inside
            .structures
            .iter()
            .map(|s| s.name.clone())
            .filter(|n| *n != new)
            .collect();
        split.pieces.push(Piece {
            source: top.clone(),
            name: new.clone(),
            bytes: out,
            what: format!(
                "structure '{top}'{}{}",
                if others.is_empty() {
                    String::new()
                } else {
                    format!(" and the {} it places", others.len())
                },
                if new != top {
                    format!(", as '{new}'")
                } else {
                    String::new()
                }
            ),
            travels: others,
            unresolved: inside.ghosts(),
        });
    }
    Ok(split)
}

// ---------------------------------------------------------------------------------
// SPICE and CDL
// ---------------------------------------------------------------------------------

const ANALYSIS: &[&str] = &[
    ".tran", ".ac", ".dc", ".op", ".noise", ".tf", ".control", ".endc", ".end", ".print", ".plot",
    ".probe", ".meas", ".measure", ".save", ".ic", ".nodeset", ".step", ".four", ".disto", ".pz",
    ".sens", ".run", ".alter",
];

fn first_word(line: &str) -> String {
    line.split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// A `.include`/`.lib file` line with a relative file name made absolute when the file
/// is found next to the source, so the card still works from another folder.
fn absolute_include(line: &str, dir: &Path) -> String {
    let lead = line.len() - line.trim_start().len();
    let word = first_word(line);
    let words = line.split_whitespace().count();
    let is_include = word == ".include" || word == ".inc" || (word == ".lib" && words >= 3);
    if !is_include {
        return line.to_string();
    }
    let rest = &line[lead + word.len()..];
    let arg_start = rest.len() - rest.trim_start().len();
    let arg_text = &rest[arg_start..];
    let arg_end = if let Some(q) = arg_text.chars().next().filter(|c| *c == '"' || *c == '\'') {
        arg_text[1..].find(q).map_or(arg_text.len(), |i| i + 2)
    } else {
        arg_text.find(char::is_whitespace).unwrap_or(arg_text.len())
    };
    let arg = &arg_text[..arg_end];
    let bare = arg.trim_matches(['"', '\'']);
    if bare.is_empty() || Path::new(bare).is_absolute() || !dir.join(bare).exists() {
        return line.to_string();
    }
    let quote = if arg.starts_with('\'') { "'" } else { "\"" };
    format!(
        "{}{}{}{quote}{}{quote}{}",
        &line[..lead],
        word,
        &rest[..arg_start],
        dir.join(bare).display(),
        &arg_text[arg_end..]
    )
}

/// The header every netlist of a file gets: what lies outside the subcircuits
/// (`.include`, `.model`, `.param`, comments), without circuit cards and analyses.
/// The second value counts the lines left out.
fn spice_header(text: &str, dir: &Path) -> (String, usize) {
    let mut out = String::new();
    let mut left_out = 0;
    // Continuation lines follow the card before them.
    let mut keep = true;
    for line in spice::outside_lines(text) {
        let trimmed = line.trim_start();
        if trimmed.starts_with('+') {
            if keep {
                out.push_str(line);
                out.push('\n');
            } else {
                left_out += 1;
            }
            continue;
        }
        keep = if trimmed.is_empty() || trimmed.starts_with(['*', ';', '$']) {
            true
        } else if trimmed.starts_with('.') {
            !ANALYSIS.contains(&first_word(trimmed).as_str())
        } else {
            // A circuit card (`R1 a b 1k`, `X1 …`): not a library part.
            false
        };
        if keep {
            out.push_str(&absolute_include(line, dir));
            out.push('\n');
        } else {
            left_out += 1;
        }
    }
    (out.trim_matches('\n').to_string(), left_out)
}

/// One piece per subcircuit: the header of the file, the subcircuits it places that
/// the file defines, and the subcircuit itself. `dir` is the folder of the file, for
/// the relative `.include` cards of the header.
pub fn spice(text: &str, dir: &Path, names: Names) -> Result<Split> {
    let index = spice::parse(text.as_bytes());
    let (header, left_out) = spice_header(text, dir);
    let mut split = Split::default();
    if left_out > 0 {
        split.notes.push(format!(
            "{left_out} top-level circuit or analysis line(s) are not part of any subcircuit: \
             left out"
        ));
    }
    for sub in &index.subckts {
        let new = names(&sub.name);
        let body = spice::extract_subckt(text, &sub.name, &new)
            .map_err(|e| eyre!("subcircuit '{}': {e}", sub.name))?;
        let placed: Vec<String> = spice::parse(body.as_bytes())
            .subckts
            .into_iter()
            .map(|s| s.name)
            .filter(|n| !n.eq_ignore_ascii_case(&new))
            .collect();
        let mut bytes = String::new();
        if !header.trim().is_empty() {
            bytes.push_str(&header);
            bytes.push_str("\n\n");
        }
        bytes.push_str(&body);
        split.pieces.push(Piece {
            source: sub.name.clone(),
            name: new.clone(),
            bytes: bytes.into_bytes(),
            what: format!(
                "subcircuit '{}'{}{}",
                sub.name,
                if placed.is_empty() {
                    String::new()
                } else {
                    format!(" and the {} it places", placed.len())
                },
                if new != sub.name {
                    format!(", as '{new}'")
                } else {
                    String::new()
                }
            ),
            travels: placed,
            unresolved: spice::unresolved(text, &sub.name),
        });
    }
    Ok(split)
}

// ---------------------------------------------------------------------------------
// Verilog
// ---------------------------------------------------------------------------------

struct Module {
    name: String,
    /// Byte range of the whole `module` … `endmodule`.
    range: (usize, usize),
    /// Byte range of the name after `module`.
    name_at: (usize, usize),
    /// Every identifier of the body.
    words: BTreeSet<String>,
}

fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

/// The modules of a Verilog text with their byte ranges. Comments and strings are
/// skipped.
fn verilog_modules(text: &str) -> Vec<Module> {
    let bytes = text.as_bytes();
    let mut out: Vec<Module> = Vec::new();
    let mut i = 0;
    type Open = (usize, Option<(usize, usize)>, BTreeSet<String>);
    let mut current: Option<Open> = None;
    while i < bytes.len() {
        match bytes[i] {
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                i += 2;
            }
            b'"' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    if bytes[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
                i += 1;
            }
            b'`' => {
                // A directive: its word is not an identifier of the design.
                i += 1;
                while i < bytes.len() && is_word(bytes[i]) {
                    i += 1;
                }
            }
            b if is_word(b) && !b.is_ascii_digit() => {
                let start = i;
                while i < bytes.len() && is_word(bytes[i]) {
                    i += 1;
                }
                let word = &text[start..i];
                match (&mut current, word) {
                    (None, "module" | "macromodule") => {
                        current = Some((start, None, BTreeSet::new()));
                    }
                    (Some((_, name_at, _)), "automatic" | "static") if name_at.is_none() => {}
                    (Some((_, name_at, _)), _) if name_at.is_none() => {
                        *name_at = Some((start, i));
                    }
                    (Some(_), "endmodule") => {
                        let (from, name_at, words) = current.take().expect("inside a module");
                        if let Some(name_at) = name_at {
                            out.push(Module {
                                name: text[name_at.0..name_at.1].to_string(),
                                range: (from, i),
                                name_at,
                                words,
                            });
                        }
                    }
                    (Some((_, _, words)), _) => {
                        words.insert(word.to_string());
                    }
                    (None, _) => {}
                }
            }
            _ => i += 1,
        }
    }
    out
}

/// One piece per module: the text of the file outside its modules (`` `timescale ``,
/// `` `define ``, primitives, comments), the modules it instantiates that the file
/// defines, and the module itself.
pub fn verilog(text: &str, names: Names) -> Result<Split> {
    let every = verilog_modules(text);
    let modules = &every;
    // The first definition of a name counts (`ifdef branches).
    let mut seen = BTreeSet::new();
    let modules: Vec<&Module> = modules
        .iter()
        .filter(|m| seen.insert(m.name.clone()))
        .collect();
    let mut header = String::new();
    let mut at = 0;
    for m in &every {
        header.push_str(&text[at..m.range.0]);
        at = m.range.1;
    }
    header.push_str(&text[at..]);
    let header = header.trim().to_string();
    let defined: Vec<String> = modules.iter().map(|m| m.name.clone()).collect();
    let mut split = Split::default();
    for m in &modules {
        let mut wanted: BTreeSet<&str> = BTreeSet::new();
        let mut todo = vec![m.name.as_str()];
        while let Some(n) = todo.pop() {
            if !wanted.insert(n) {
                continue;
            }
            if let Some(found) = modules.iter().find(|x| x.name == n) {
                todo.extend(
                    found
                        .words
                        .iter()
                        .filter(|w| defined.contains(w))
                        .map(String::as_str),
                );
            }
        }
        let new = names(&m.name);
        let mut bytes = String::new();
        if !header.is_empty() {
            bytes.push_str(&header);
            bytes.push_str("\n\n");
        }
        for x in modules.iter().filter(|x| wanted.contains(x.name.as_str())) {
            let body = &text[x.range.0..x.range.1];
            if x.name == m.name && new != m.name {
                let (a, b) = (x.name_at.0 - x.range.0, x.name_at.1 - x.range.0);
                bytes.push_str(&format!("{}{new}{}", &body[..a], &body[b..]));
            } else {
                bytes.push_str(body);
            }
            bytes.push_str("\n\n");
        }
        // Names the module instantiates that are neither defined here nor plain words
        // are not guessed at: Verilog bodies are not parsed.
        let placed = travels(
            &wanted.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            &m.name,
        );
        split.pieces.push(Piece {
            source: m.name.clone(),
            name: new.clone(),
            bytes: bytes.into_bytes(),
            what: format!(
                "module '{}'{}{}",
                m.name,
                if placed.is_empty() {
                    String::new()
                } else {
                    format!(" and the {} it instantiates", placed.len())
                },
                if new != m.name {
                    format!(", as '{new}'")
                } else {
                    String::new()
                }
            ),
            travels: placed,
            unresolved: Vec::new(),
        });
    }
    Ok(split)
}

// ---------------------------------------------------------------------------------
// LEF
// ---------------------------------------------------------------------------------

/// Blocks of a technology LEF.
const LEF_TECH: &[&str] = &[
    "LAYER",
    "VIA",
    "VIARULE",
    "NONDEFAULTRULE",
    "PROPERTYDEFINITIONS",
    "SPACING",
    "BEGINEXT",
    "IRDROP",
    "ARRAY",
];
/// Blocks that end with `END <keyword>` and not `END <name>`.
const LEF_KEYWORD_END: &[&str] = &["UNITS", "PROPERTYDEFINITIONS", "SPACING", "IRDROP", "ARRAY"];

/// One piece per macro: the version and unit statements and the sites of the file,
/// then the macro. The layers and vias of a technology part go to [`Split::tech`].
pub fn lef(text: &str, names: Names) -> Result<Split> {
    let mut header = String::new();
    let mut tech = String::new();
    let mut macros: Vec<(String, String)> = Vec::new();
    // `(kind, end line to wait for)`: 0 header, 1 tech, 2 macro.
    let mut open: Option<(u8, String)> = None;
    let mut current = String::new();
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim();
        let mut words = trimmed.split_whitespace();
        let key = words.next().unwrap_or("");
        match &open {
            None => {
                if key == "MACRO" {
                    let name = words.next().unwrap_or("").to_string();
                    open = Some((2, name));
                    current = line.to_string();
                } else if matches!(key, "LAYER" | "VIA" | "VIARULE" | "NONDEFAULTRULE" | "SITE" | "UNITS" | "PROPERTYDEFINITIONS" | "SPACING" | "BEGINEXT" | "IRDROP" | "ARRAY")
                {
                    let end = if key == "BEGINEXT" {
                        "ENDEXT".to_string()
                    } else if LEF_KEYWORD_END.contains(&key) {
                        key.to_string()
                    } else {
                        words.next().unwrap_or("").to_string()
                    };
                    let kind = u8::from(LEF_TECH.contains(&key));
                    open = Some((kind, end));
                    current = line.to_string();
                } else if key == "END" && words.next() == Some("LIBRARY") {
                } else if !trimmed.is_empty() {
                    header.push_str(line);
                } else {
                    // Blank lines between statements are dropped.
                }
            }
            Some((kind, end)) => {
                current.push_str(line);
                let closes = if end == "ENDEXT" {
                    key == "ENDEXT"
                } else {
                    key == "END" && words.next() == Some(end.as_str())
                };
                if closes {
                    match kind {
                        2 => macros.push((end.clone(), std::mem::take(&mut current))),
                        1 => tech.push_str(&std::mem::take(&mut current)),
                        _ => header.push_str(&std::mem::take(&mut current)),
                    }
                    open = None;
                }
            }
        }
    }
    let mut split = Split::default();
    let wrap = |parts: &[&str]| {
        let mut s = String::new();
        for p in parts {
            if !p.is_empty() {
                s.push_str(p);
                if !p.ends_with('\n') {
                    s.push('\n');
                }
                s.push('\n');
            }
        }
        s.push_str("END LIBRARY\n");
        s.into_bytes()
    };
    for (name, body) in &macros {
        let new = names(name);
        let body = if *name == new {
            body.clone()
        } else {
            rename_lef_macro(body, name, &new)
        };
        split.pieces.push(Piece {
            source: name.clone(),
            name: new.clone(),
            bytes: wrap(&[&header, &body]),
            what: if *name == new {
                format!("macro '{name}'")
            } else {
                format!("macro '{name}', as '{new}'")
            },
            travels: Vec::new(),
            unresolved: Vec::new(),
        });
    }
    if !tech.is_empty() {
        split.tech = Some(wrap(&[&header, &tech]));
    }
    Ok(split)
}

fn rename_lef_macro(body: &str, old: &str, new: &str) -> String {
    let mut out = String::new();
    let lines: Vec<&str> = body.split_inclusive('\n').collect();
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim();
        let swap = (i == 0 && t.starts_with("MACRO"))
            || (i + 1 == lines.len() && t.starts_with("END"))
            || (i + 2 == lines.len() && t.starts_with("END") && lines[i + 1].trim().is_empty());
        if swap && t.split_whitespace().nth(1) == Some(old) {
            out.push_str(&line.replacen(old, new, 1));
        } else {
            out.push_str(line);
        }
    }
    out
}

// ---------------------------------------------------------------------------------
// Liberty
// ---------------------------------------------------------------------------------

/// Skip a comment, a string or a line continuation starting at `i`; returns the new
/// position or `None` when nothing starts there.
fn liberty_skip(b: &[u8], i: usize) -> Option<usize> {
    match b[i] {
        b'/' if b.get(i + 1) == Some(&b'*') => {
            let mut j = i + 2;
            while j + 1 < b.len() && !(b[j] == b'*' && b[j + 1] == b'/') {
                j += 1;
            }
            Some((j + 2).min(b.len()))
        }
        b'/' if b.get(i + 1) == Some(&b'/') => {
            let mut j = i;
            while j < b.len() && b[j] != b'\n' {
                j += 1;
            }
            Some(j)
        }
        b'"' => {
            let mut j = i + 1;
            while j < b.len() && b[j] != b'"' {
                if b[j] == b'\\' {
                    j += 1;
                }
                j += 1;
            }
            Some((j + 1).min(b.len()))
        }
        _ => None,
    }
}

/// The text between the parentheses starting at `i` (which is `(`), and the position
/// after the closing one.
fn parenthesised(b: &[u8], i: usize) -> Option<(String, usize)> {
    let mut j = i + 1;
    let mut inner = String::new();
    while j < b.len() {
        if let Some(next) = liberty_skip(b, j) {
            inner.push_str(&String::from_utf8_lossy(&b[j..next]));
            j = next;
            continue;
        }
        if b[j] == b')' {
            return Some((inner, j + 1));
        }
        inner.push(b[j] as char);
        j += 1;
    }
    None
}

/// The position after the `}` that closes the group whose `{` is at `i`.
fn group_end(b: &[u8], i: usize) -> usize {
    let mut depth = 0usize;
    let mut j = i;
    while j < b.len() {
        if let Some(next) = liberty_skip(b, j) {
            j = next;
            continue;
        }
        match b[j] {
            b'{' => depth += 1,
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return j + 1;
                }
            }
            _ => {}
        }
        j += 1;
    }
    b.len()
}

/// One piece per cell group: the `library (…) {` line, the attributes and tables of the
/// library (everything but the cells), the cell, and the closing brace.
pub fn liberty(text: &str, names: Names) -> Result<Split> {
    let b = text.as_bytes();
    // Find `library (name) {`.
    let mut i = 0;
    let mut library_open = None;
    while i < b.len() {
        if let Some(next) = liberty_skip(b, i) {
            i = next;
            continue;
        }
        if b[i..].starts_with(b"library") && b.get(i + 7).is_none_or(|c| !is_word(*c)) {
            let mut j = i + 7;
            while j < b.len() && b[j].is_ascii_whitespace() {
                j += 1;
            }
            if b.get(j) == Some(&b'(')
                && let Some((_, after)) = parenthesised(b, j)
            {
                let mut k = after;
                while k < b.len() && b[k].is_ascii_whitespace() {
                    k += 1;
                }
                if b.get(k) == Some(&b'{') {
                    library_open = Some((i, k + 1));
                    break;
                }
            }
        }
        i += 1;
    }
    let Some((lib_start, body_start)) = library_open else {
        return Err(eyre!("no `library (…) {{` group found"));
    };
    let body_end = group_end(b, body_start - 1) - 1;
    // Cells at depth 1 of the library group.
    let mut cells: Vec<(String, usize, usize)> = Vec::new();
    let mut i = body_start;
    let mut depth = 0usize;
    while i < body_end {
        if let Some(next) = liberty_skip(b, i) {
            i = next;
            continue;
        }
        match b[i] {
            b'{' => depth += 1,
            b'}' => depth = depth.saturating_sub(1),
            b'c' if depth == 0
                && b[i..].starts_with(b"cell")
                && (i == 0 || !is_word(b[i - 1]))
                && b.get(i + 4).is_none_or(|c| !is_word(*c)) =>
            {
                let mut j = i + 4;
                while j < b.len() && b[j].is_ascii_whitespace() {
                    j += 1;
                }
                if b.get(j) == Some(&b'(')
                    && let Some((inner, after)) = parenthesised(b, j)
                {
                    let mut k = after;
                    while k < b.len() && b[k].is_ascii_whitespace() {
                        k += 1;
                    }
                    if b.get(k) == Some(&b'{') {
                        let end = group_end(b, k);
                        cells.push((inner.trim().trim_matches('"').to_string(), i, end));
                        i = end;
                        continue;
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    let prefix = &text[lib_start..body_start];
    let mut header = String::new();
    let mut at = body_start;
    for (_, from, to) in &cells {
        header.push_str(&text[at..*from]);
        at = *to;
    }
    header.push_str(&text[at..body_end]);
    let header = header.trim_matches('\n');
    let mut split = Split::default();
    let mut seen = BTreeSet::new();
    for (name, from, to) in &cells {
        if !seen.insert(name.clone()) {
            continue;
        }
        let new = names(name);
        let mut cell = text[*from..*to].to_string();
        if new != *name {
            cell = cell.replacen(name.as_str(), &new, 1);
        }
        let out = format!("{prefix}\n{header}\n\n  {cell}\n}}\n");
        split.pieces.push(Piece {
            source: name.clone(),
            name: new.clone(),
            bytes: out.into_bytes(),
            what: if *name == new {
                format!("cell '{name}' with the library attributes")
            } else {
                format!("cell '{name}' with the library attributes, as '{new}'")
            },
            travels: Vec::new(),
            unresolved: Vec::new(),
        });
    }
    Ok(split)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn same(n: &str) -> String {
        n.to_string()
    }

    #[test]
    fn gds_pieces_follow_the_top_structures() {
        let bytes = gds::tests::build(&[
            ("leaf", &[]),
            ("top_a", &["leaf"]),
            ("top_b", &["leaf", "ghost"]),
            ("$$$CONTEXT_INFO$$$", &[]),
        ]);
        let split = gds(&bytes, &same).unwrap();
        let names: Vec<_> = split.pieces.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["top_a", "top_b"]);
        let a = gds::parse(&split.pieces[0].bytes).unwrap();
        assert_eq!(a.structures.len(), 2);
        assert!(split.pieces[0].unresolved.is_empty());
        assert_eq!(split.pieces[1].unresolved, ["ghost"]);
        assert_eq!(split.pieces[0].travels, ["leaf"]);
        // Renamed.
        let split = gds(&bytes, &|n| format!("x_{n}")).unwrap();
        assert_eq!(split.pieces[0].name, "x_top_a");
        let a = gds::parse(&split.pieces[0].bytes).unwrap();
        assert_eq!(a.top_cells(), ["x_top_a"]);
        assert!(gds(b"not a stream", &same).is_err());
    }

    const SPICE: &str = "* header comment\n.include \"models.spice\"\n.param vdd=1.8\n\
R9 a b 1k\n.tran 1n 10n\n\
.subckt inv A Y\nMN0 Y A 0 0 nmos w=1u\n.ends\n\
.subckt buf A Y\nX1 A n inv\nX2 n Y inv\nX3 Y q mystery\n.ends buf\n.end\n";

    #[test]
    fn spice_pieces_keep_the_header_and_what_they_place() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("models.spice"), "* m\n").unwrap();
        let split = spice(SPICE, dir.path(), &same).unwrap();
        assert_eq!(split.pieces.len(), 2);
        assert!(split.notes[0].contains("3 top-level"), "{:?}", split.notes);
        let inv = String::from_utf8(split.pieces[0].bytes.clone()).unwrap();
        assert!(inv.contains(".param vdd=1.8"), "{inv}");
        assert!(inv.contains(&format!(".include \"{}", dir.path().display())), "{inv}");
        assert!(!inv.contains("R9") && !inv.contains(".tran") && !inv.contains(".subckt buf"));
        let buf = String::from_utf8(split.pieces[1].bytes.clone()).unwrap();
        assert!(buf.contains(".subckt inv") && buf.contains(".subckt buf"));
        assert_eq!(split.pieces[1].travels, ["inv"]);
        assert_eq!(split.pieces[1].unresolved, ["mystery"]);
        let split = spice(SPICE, dir.path(), &|n| format!("p_{n}")).unwrap();
        let buf = String::from_utf8(split.pieces[1].bytes.clone()).unwrap();
        assert!(buf.contains(".subckt p_buf A Y") && buf.contains(".ends p_buf"), "{buf}");
    }

    const VERILOG: &str = "// lib\n`timescale 1ns/1ps\n\
module inv (input a, output y); assign y = ~a; endmodule\n\
module buf2 (input a, output y);\n  wire n; // inv in a comment\n  inv u1 (.a(a), .y(n));\n  inv u2 (.a(n), .y(y));\nendmodule\n\
module alone (input a); endmodule\n";

    #[test]
    fn verilog_pieces_carry_the_modules_they_instantiate() {
        let split = verilog(VERILOG, &same).unwrap();
        let names: Vec<_> = split.pieces.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["inv", "buf2", "alone"]);
        let buf = String::from_utf8(split.pieces[1].bytes.clone()).unwrap();
        assert!(buf.starts_with("// lib\n`timescale 1ns/1ps"), "{buf}");
        assert!(buf.contains("module inv ") && buf.contains("module buf2 "));
        assert!(!buf.contains("module alone"));
        assert_eq!(split.pieces[1].travels, ["inv"]);
        let alone = String::from_utf8(split.pieces[2].bytes.clone()).unwrap();
        assert!(!alone.contains("module inv"));
        let split = verilog(VERILOG, &|n| format!("c_{n}")).unwrap();
        let buf = String::from_utf8(split.pieces[1].bytes.clone()).unwrap();
        assert!(buf.contains("module c_buf2 "), "{buf}");
        assert!(buf.contains("module inv "), "{buf}");
    }

    const LEF: &str = "VERSION 5.8 ;\nBUSBITCHARS \"[]\" ;\nUNITS\n  DATABASE MICRONS 1000 ;\nEND UNITS\n\
LAYER met1\n  TYPE ROUTING ;\nEND met1\n\
SITE core\n  SIZE 0.46 BY 2.72 ;\nEND core\n\
MACRO inv\n  CLASS CORE ;\n  PIN A\n    DIRECTION INPUT ;\n  END A\nEND inv\n\
MACRO buf\n  CLASS CORE ;\nEND buf\nEND LIBRARY\n";

    #[test]
    fn lef_pieces_and_the_tech_part() {
        let split = lef(LEF, &same).unwrap();
        assert_eq!(split.pieces.len(), 2);
        let inv = String::from_utf8(split.pieces[0].bytes.clone()).unwrap();
        assert!(inv.contains("DATABASE MICRONS 1000"), "{inv}");
        assert!(inv.contains("SITE core") && inv.contains("MACRO inv"));
        assert!(!inv.contains("met1") && !inv.contains("MACRO buf"));
        assert!(inv.trim_end().ends_with("END LIBRARY"));
        let tech = String::from_utf8(split.tech.clone().unwrap()).unwrap();
        assert!(tech.contains("LAYER met1") && !tech.contains("MACRO"));
        let parsed = crate::container::lef::parse(inv.as_bytes());
        assert_eq!(parsed.macros.len(), 1);
        let split = lef(LEF, &|n| format!("z_{n}")).unwrap();
        let inv = String::from_utf8(split.pieces[0].bytes.clone()).unwrap();
        assert!(inv.contains("MACRO z_inv") && inv.contains("END z_inv"), "{inv}");
        assert!(inv.contains("END A"));
        // A LEF with no layers has no tech part.
        assert!(lef("MACRO a\nEND a\n", &same).unwrap().tech.is_none());
    }

    const LIBERTY: &str = "/* header */\nlibrary (\"demo_tt\") {\n  time_unit : \"1ns\";\n\
  lu_table_template (t) { variable_1 : input_net_transition; }\n\
  cell (\"inv_1\") {\n    area : 3.0;\n    pin (A) { direction : input; }\n  }\n\
  cell (buf_1) {\n    area : 5.0; /* } stray brace in comment */\n  }\n}\n";

    #[test]
    fn liberty_pieces_keep_the_library_attributes() {
        let split = liberty(LIBERTY, &same).unwrap();
        let names: Vec<_> = split.pieces.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["inv_1", "buf_1"]);
        let inv = String::from_utf8(split.pieces[0].bytes.clone()).unwrap();
        assert!(inv.contains("library (\"demo_tt\") {") && inv.contains("time_unit"), "{inv}");
        assert!(inv.contains("lu_table_template") && inv.contains("cell (\"inv_1\")"));
        assert!(!inv.contains("buf_1"));
        let parsed = crate::container::liberty::parse(inv.as_bytes());
        assert_eq!(parsed.cells.len(), 1);
        let buf = String::from_utf8(split.pieces[1].bytes.clone()).unwrap();
        assert!(buf.contains("stray brace") && !buf.contains("inv_1"), "{buf}");
        let split = liberty(LIBERTY, &|n| format!("q_{n}")).unwrap();
        let buf = String::from_utf8(split.pieces[1].bytes.clone()).unwrap();
        assert!(buf.contains("cell (q_buf_1)"), "{buf}");
        assert!(liberty("nothing", &same).is_err());
    }
}
