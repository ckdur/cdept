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

#[cfg(test)]
mod tests {
    use super::*;

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
