//! SPICE and CDL netlists: the subcircuits of a file with their pins, the models and
//! the files it includes. Device lines are not read.

use serde::{Deserialize, Serialize};

/// What a netlist file holds.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SpiceIndex {
    /// Subcircuits in file order, each name once.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub subckts: Vec<Subckt>,
    /// `.model` names.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<String>,
    /// Files named by `.include` and `.lib file section` (as written).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub includes: Vec<String>,
    /// Names of the `.lib name` … `.endl` sections the file defines.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sections: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Subckt {
    pub name: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pins: Vec<String>,
    /// Parameters with their defaults, as written (`w=1u`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub parameters: Vec<String>,
    /// Number of device and instance lines inside.
    pub devices: usize,
}

fn lower_starts(line: &[u8], word: &[u8]) -> bool {
    line.len() >= word.len()
        && line[..word.len()].eq_ignore_ascii_case(word)
        && line.get(word.len()).is_none_or(|b| b.is_ascii_whitespace())
}

/// Read a netlist's text. Never fails: what is not understood is skipped.
pub fn parse(bytes: &[u8]) -> SpiceIndex {
    let mut index = SpiceIndex::default();
    // The `.subckt` card being read (with its `+` continuation lines).
    let mut card: Option<Vec<u8>> = None;
    // Index of the subcircuit whose body is being read.
    let mut inside: Option<usize> = None;
    let finish = |card: &mut Option<Vec<u8>>, index: &mut SpiceIndex| -> Option<usize> {
        let text = card.take()?;
        let text = String::from_utf8_lossy(&text);
        let mut words = text.split_whitespace();
        let name = words.next()?.to_string();
        let mut subckt = Subckt {
            name,
            ..Subckt::default()
        };
        let mut parameters = false;
        for word in words {
            let lower = word.to_ascii_lowercase();
            if lower == "params:" || lower == "param:" {
                parameters = true;
            } else if parameters || word.contains('=') {
                parameters = true;
                subckt.parameters.push(word.to_string());
            } else if word.starts_with('$') || word.starts_with(';') {
                break;
            } else {
                subckt.pins.push(word.to_string());
            }
        }
        if let Some(at) = index.subckts.iter().position(|s| s.name == subckt.name) {
            return Some(at);
        }
        index.subckts.push(subckt);
        Some(index.subckts.len() - 1)
    };
    for line in bytes.split(|b| *b == b'\n') {
        let line = line.trim_ascii();
        let Some(&first) = line.first() else { continue };
        if first == b'+' {
            if let Some(card) = &mut card {
                card.push(b' ');
                card.extend_from_slice(&line[1..]);
            }
            continue;
        }
        if first == b'*' {
            continue;
        }
        if card.is_some() {
            inside = finish(&mut card, &mut index);
        }
        if first != b'.' {
            if let Some(at) = inside
                && first.is_ascii_alphabetic()
            {
                index.subckts[at].devices += 1;
            }
            continue;
        }
        let words = || String::from_utf8_lossy(line).into_owned();
        if lower_starts(line, b".subckt") || lower_starts(line, b".macro") {
            let at = line
                .iter()
                .position(|b| b.is_ascii_whitespace())
                .unwrap_or(line.len());
            card = Some(line[at..].to_vec());
        } else if lower_starts(line, b".ends") || lower_starts(line, b".eom") {
            inside = None;
        } else if lower_starts(line, b".model") {
            if let Some(name) = words().split_whitespace().nth(1) {
                index.models.push(name.to_string());
            }
        } else if lower_starts(line, b".include") || lower_starts(line, b".inc") {
            if let Some(file) = words().split_whitespace().nth(1) {
                index
                    .includes
                    .push(file.trim_matches(['"', '\'']).to_string());
            }
        } else if lower_starts(line, b".lib") {
            let text = words();
            let mut parts = text.split_whitespace().skip(1);
            match (parts.next(), parts.next()) {
                (Some(file), Some(_section)) => {
                    index
                        .includes
                        .push(file.trim_matches(['"', '\'']).to_string());
                }
                (Some(section), None) => index.sections.push(section.to_string()),
                _ => {}
            }
        }
    }
    if card.is_some() {
        finish(&mut card, &mut index);
    }
    index
}

/// One `.subckt` … `.ends` block of a netlist (top level only), as lines.
struct Block<'a> {
    name: &'a str,
    lines: Vec<&'a str>,
}

/// The word after the first word of a line (`.subckt NAME …`), with where it starts.
fn second_word(line: &str) -> Option<(usize, &str)> {
    let first_end = line.find(char::is_whitespace)?;
    let rest = &line[first_end..];
    let start = first_end + (rest.len() - rest.trim_start().len());
    let word = line[start..].split_whitespace().next()?;
    Some((start, word))
}

fn blocks(text: &str) -> Vec<Block<'_>> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut current: Option<Block> = None;
    for line in text.lines() {
        let trimmed = line.trim_start().as_bytes();
        if lower_starts(trimmed, b".subckt") || lower_starts(trimmed, b".macro") {
            if depth == 0 {
                let name = second_word(line.trim_start()).map_or("", |(_, w)| w);
                current = Some(Block {
                    name,
                    lines: Vec::new(),
                });
            }
            depth += 1;
        }
        if let Some(block) = &mut current {
            block.lines.push(line);
        }
        if (lower_starts(trimmed, b".ends") || lower_starts(trimmed, b".eom")) && depth > 0 {
            depth -= 1;
            if depth == 0 {
                out.extend(current.take());
            }
        }
    }
    out
}

/// The subcircuits the instance lines of a block place: the last word before the
/// parameters of each `X` card, `+` continuations joined.
fn placed(block: &Block) -> Vec<String> {
    let mut cards: Vec<String> = Vec::new();
    for line in block.lines.iter().skip(1) {
        let line = line.trim();
        if let Some(more) = line.strip_prefix('+') {
            if let Some(last) = cards.last_mut() {
                last.push(' ');
                last.push_str(more);
            }
        } else {
            cards.push(line.to_string());
        }
    }
    cards
        .iter()
        .filter(|c| c.starts_with(['x', 'X']))
        .filter_map(|c| {
            c.split_whitespace()
                .skip(1)
                .take_while(|w| !w.contains('=') && !w.eq_ignore_ascii_case("params:"))
                .last()
                .map(str::to_string)
        })
        .collect()
}

/// `line` (a `.subckt` or `.ends` card) with the word after the first one replaced.
fn with_name(line: &str, new: &str) -> String {
    match second_word(line) {
        Some((start, word)) => format!("{}{new}{}", &line[..start], &line[start + word.len()..]),
        None => line.to_string(),
    }
}

/// The subcircuit `name` of a netlist with the ones it places that the file defines
/// (at any depth), as a netlist of its own, in the order of the file. The top one is
/// named `new`. Nothing else is taken: no models, includes, parameters or other cells.
pub fn extract_subckt(text: &str, name: &str, new: &str) -> eyre::Result<String> {
    let all = blocks(text);
    let find = |n: &str| all.iter().position(|b| b.name.eq_ignore_ascii_case(n));
    let Some(top) = find(name) else {
        eyre::bail!("the netlist has no subcircuit '{name}'");
    };
    let mut wanted = vec![false; all.len()];
    let mut todo = vec![top];
    while let Some(at) = todo.pop() {
        if std::mem::replace(&mut wanted[at], true) {
            continue;
        }
        todo.extend(placed(&all[at]).iter().filter_map(|n| find(n)));
    }
    let mut out = String::new();
    for (at, block) in all.iter().enumerate().filter(|(at, _)| wanted[*at]) {
        if at != top && block.name.eq_ignore_ascii_case(new) {
            eyre::bail!("'{new}' is also the name of a subcircuit used by '{name}'");
        }
        for (i, line) in block.lines.iter().enumerate() {
            let trimmed = line.trim_start().as_bytes();
            // `.subckt NAME`, and `.ends NAME` of the top one, carry the new name.
            let named_end = i + 1 == block.lines.len()
                && (lower_starts(trimmed, b".ends") || lower_starts(trimmed, b".eom"))
                && second_word(line).is_some_and(|(_, w)| w.eq_ignore_ascii_case(name));
            if at == top && (i == 0 || named_end) {
                out.push_str(&with_name(line, new));
            } else {
                out.push_str(line);
            }
            out.push('\n');
        }
    }
    Ok(out)
}

/// The lines of a netlist that are not inside a subcircuit (`.include`, `.model`,
/// comments, top-level circuit cards), in file order.
pub fn outside_lines(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    for line in text.lines() {
        let trimmed = line.trim_start().as_bytes();
        let opens = lower_starts(trimmed, b".subckt") || lower_starts(trimmed, b".macro");
        let closes = lower_starts(trimmed, b".ends") || lower_starts(trimmed, b".eom");
        if depth == 0 && !opens {
            out.push(line);
        }
        if opens {
            depth += 1;
        }
        if closes && depth > 0 {
            depth -= 1;
        }
    }
    out
}

/// Subcircuits that `name` and what it places (in the file) instantiate and the file
/// does not define, sorted. Empty when the file has no subcircuit `name`.
pub fn unresolved(text: &str, name: &str) -> Vec<String> {
    let all = blocks(text);
    let find = |n: &str| all.iter().position(|b| b.name.eq_ignore_ascii_case(n));
    let Some(top) = find(name) else {
        return Vec::new();
    };
    let mut seen = vec![false; all.len()];
    let mut todo = vec![top];
    let mut missing = std::collections::BTreeSet::new();
    while let Some(at) = todo.pop() {
        if std::mem::replace(&mut seen[at], true) {
            continue;
        }
        for n in placed(&all[at]) {
            match find(&n) {
                Some(i) => todo.push(i),
                None => {
                    missing.insert(n);
                }
            }
        }
    }
    missing.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_a_subckt_with_the_ones_it_places() {
        let text = ".include \"m.spice\"\n.subckt inv A Y\nXN0 Y A 0 0 nmos w=1u\n.ends\n\
* c\n.subckt buf A Y VDD\nX1 A n inv\nX2 n Y inv m=1\n.ends buf\n\
.subckt other A\nR1 A 0 1\n.ends\n";
        let out = extract_subckt(text, "buf", "buf_copy").unwrap();
        assert_eq!(
            out,
            ".subckt inv A Y\nXN0 Y A 0 0 nmos w=1u\n.ends\n\
.subckt buf_copy A Y VDD\nX1 A n inv\nX2 n Y inv m=1\n.ends buf_copy\n"
        );
        assert_eq!(parse(out.as_bytes()).subckts.len(), 2);
        let alone = extract_subckt(text, "INV", "x").unwrap();
        assert!(alone.starts_with(".subckt x A Y"), "{alone}");
        assert!(!alone.contains("other"));
        assert!(extract_subckt(text, "zzz", "x").is_err());
        assert!(extract_subckt(text, "buf", "inv").is_err());
    }

    #[test]
    fn subcircuits_with_pins_and_parameters() {
        let text = r#"* a library
.include "models.spice"
.lib 'corners.lib' tt
.SUBCKT inv_1 A VGND VNB
+ VPB VPWR Y
X0 Y A VGND VNB nfet w=650000u l=150000u
X1 Y A VPWR VPB pfet w=1e+06u l=150000u
.ENDS
.subckt res_gen a b w=1u l=2u
R1 a b r='l/w'
.ends res_gen
.subckt opamp inp inn out params: gain=1e5
E1 out 0 inp inn {gain}
.ends
.model nch nmos level=1
.lib tt
.endl
.subckt inv_1 A Y
.ends
"#;
        let index = parse(text.as_bytes());
        let names: Vec<_> = index.subckts.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["inv_1", "res_gen", "opamp"]);
        assert_eq!(
            index.subckts[0].pins,
            ["A", "VGND", "VNB", "VPB", "VPWR", "Y"]
        );
        assert_eq!(index.subckts[0].devices, 2);
        assert_eq!(index.subckts[1].pins, ["a", "b"]);
        assert_eq!(index.subckts[1].parameters, ["w=1u", "l=2u"]);
        assert_eq!(index.subckts[2].pins, ["inp", "inn", "out"]);
        assert_eq!(index.subckts[2].parameters, ["gain=1e5"]);
        assert_eq!(index.models, ["nch"]);
        assert_eq!(index.includes, ["models.spice", "corners.lib"]);
        assert_eq!(index.sections, ["tt"]);
    }
}
