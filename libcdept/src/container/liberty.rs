//! Liberty (`.lib`): the library header (units, nominal conditions, operating
//! conditions) and, per cell, its area, pins and whether it is sequential. Timing
//! and power tables are skipped, so files of tens of megabytes read in a fraction of
//! a second.

use serde::{Deserialize, Serialize};

/// What a Liberty file holds.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LibertyIndex {
    /// The name in `library (…)`.
    pub library: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub delay_model: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub time_unit: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub voltage_unit: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub current_unit: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub leakage_power_unit: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub pulling_resistance_unit: String,
    /// `capacitive_load_unit (1, pf)` reads `1pf`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub capacitive_load_unit: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nom_process: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nom_voltage: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nom_temperature: Option<f64>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub default_operating_conditions: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub operating_conditions: Vec<OperatingConditions>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub revision: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub date: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cells: Vec<LibertyCell>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OperatingConditions {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voltage: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LibertyCell {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub area: Option<f64>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub footprint: String,
    /// `ff`, `latch` or `statetable` when the cell has such a group.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub sequential: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub is_macro: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub dont_use: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pins: Vec<LibertyPin>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LibertyPin {
    pub name: String,
    /// `input`, `output`, `inout`, `internal`; for a power pin its `pg_type`
    /// (`primary_power`, `primary_ground`, `nwell`, …).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub direction: String,
    /// `pin`, `bus`, `bundle` or `pg_pin`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub kind: String,
}

impl LibertyIndex {
    /// The conditions the library was characterised at: the default operating
    /// conditions when the header names them, else the nominal values.
    pub fn conditions(&self) -> OperatingConditions {
        let named = self
            .operating_conditions
            .iter()
            .find(|c| c.name == self.default_operating_conditions)
            .or(match self.operating_conditions.as_slice() {
                [only] => Some(only),
                _ => None,
            });
        OperatingConditions {
            name: named.map(|c| c.name.clone()).unwrap_or_default(),
            process: named.and_then(|c| c.process).or(self.nom_process),
            voltage: named.and_then(|c| c.voltage).or(self.nom_voltage),
            temperature: named.and_then(|c| c.temperature).or(self.nom_temperature),
        }
    }
}

/// What an open group is, as far as the reader cares.
#[derive(Clone, Copy, PartialEq)]
enum Group {
    Library,
    Conditions,
    Cell,
    Pin,
    Other,
}

fn trim(mut text: &[u8]) -> &[u8] {
    while let [first, rest @ ..] = text
        && (first.is_ascii_whitespace() || *first == b'\\')
    {
        text = rest;
    }
    while let [rest @ .., last] = text
        && (last.is_ascii_whitespace() || *last == b'\\')
    {
        text = rest;
    }
    text
}

fn unquote(text: &[u8]) -> String {
    let text = trim(text);
    let text = text.strip_prefix(b"\"").unwrap_or(text);
    let text = text.strip_suffix(b"\"").unwrap_or(text);
    String::from_utf8_lossy(trim(text)).into_owned()
}

fn number(text: &[u8]) -> Option<f64> {
    unquote(text).parse().ok()
}

fn truth(text: &[u8]) -> bool {
    unquote(text).eq_ignore_ascii_case("true")
}

/// `name (args)` → `(name, args)`; `name : value` → none.
fn header(text: &[u8]) -> Option<(&[u8], &[u8])> {
    let text = trim(text);
    let open = memchr::memchr(b'(', text)?;
    let close = memchr::memrchr(b')', text)?;
    (close > open).then(|| (trim(&text[..open]), &text[open + 1..close]))
}

/// `name : value` → `(name, value)`.
fn attribute(text: &[u8]) -> Option<(&[u8], &[u8])> {
    let text = trim(text);
    let colon = memchr::memchr(b':', text)?;
    Some((trim(&text[..colon]), trim(&text[colon + 1..])))
}

/// Read a Liberty file's text. Never fails: what is not understood is skipped.
pub fn parse(bytes: &[u8]) -> LibertyIndex {
    let mut index = LibertyIndex::default();
    let mut stack: Vec<Group> = Vec::new();
    // Start of the statement being read.
    let mut start = 0;
    let mut i = 0;
    let n = bytes.len();
    while i < n {
        // Inside timing and power groups nothing is of interest: only braces, strings
        // and comments are followed there.
        let deep = stack.last() == Some(&Group::Other) && stack.len() > 2;
        let b = bytes[i];
        match b {
            b'"' => {
                // To the closing quote (a backslash escapes, also a line end).
                let mut j = i + 1;
                loop {
                    match memchr::memchr(b'"', &bytes[j..]) {
                        Some(at) if at > 0 && bytes[j + at - 1] == b'\\' => j += at + 1,
                        Some(at) => {
                            i = j + at;
                            break;
                        }
                        None => {
                            i = n;
                            break;
                        }
                    }
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                let end = memchr::memmem::find(&bytes[i + 2..], b"*/").map(|at| i + 2 + at + 1);
                // A comment is not part of a statement.
                if trim(&bytes[start..i]).is_empty() {
                    start = end.map_or(n, |e| e + 1);
                }
                i = end.unwrap_or(n);
            }
            b'{' => {
                let group = match header(&bytes[start..i]) {
                    Some((name, args)) => open(&mut index, &stack, name, args),
                    None => Group::Other,
                };
                stack.push(group);
                start = i + 1;
            }
            b'}' => {
                stack.pop();
                start = i + 1;
            }
            b';' | b'\n' if deep => start = i + 1,
            b';' => {
                statement(&mut index, &stack, &bytes[start..i]);
                start = i + 1;
            }
            b'\n' => {
                // An attribute may end with the line (no semicolon); a group header
                // or a continued line (backslash) goes on.
                let text = &bytes[start..i];
                let continued = text.last() == Some(&b'\\') || text.ends_with(b"\\\r");
                // `name (args)` at a line end is a group header when a brace follows,
                // else a complex attribute written without its semicolon.
                let is_header = header(text).is_some()
                    && bytes[i..].iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'{');
                if !continued && !is_header {
                    if attribute(text).is_some() || header(text).is_some() {
                        statement(&mut index, &stack, text);
                    }
                    start = i + 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    index
}

/// A group opens: note what it is and what it names.
fn open(index: &mut LibertyIndex, stack: &[Group], name: &[u8], args: &[u8]) -> Group {
    match (stack.last(), name) {
        (None, b"library") => {
            index.library = unquote(args);
            Group::Library
        }
        (Some(Group::Library), b"operating_conditions") => {
            index.operating_conditions.push(OperatingConditions {
                name: unquote(args),
                ..OperatingConditions::default()
            });
            Group::Conditions
        }
        (Some(Group::Library), b"cell") => {
            index.cells.push(LibertyCell {
                name: unquote(args),
                ..LibertyCell::default()
            });
            Group::Cell
        }
        (Some(Group::Cell), b"pin" | b"bus" | b"bundle" | b"pg_pin") => {
            if let Some(cell) = index.cells.last_mut() {
                cell.pins.push(LibertyPin {
                    name: unquote(args),
                    direction: String::new(),
                    kind: String::from_utf8_lossy(name).into_owned(),
                });
            }
            Group::Pin
        }
        (Some(Group::Cell), b"ff" | b"latch" | b"statetable" | b"ff_bank" | b"latch_bank") => {
            if let Some(cell) = index.cells.last_mut() {
                let kind = String::from_utf8_lossy(name);
                cell.sequential = kind.trim_end_matches("_bank").to_string();
            }
            Group::Other
        }
        _ => Group::Other,
    }
}

/// A simple or complex attribute ended.
fn statement(index: &mut LibertyIndex, stack: &[Group], text: &[u8]) {
    let Some(group) = stack.last() else { return };
    match group {
        Group::Library => {
            if let Some((name, value)) = attribute(text) {
                let field = match name {
                    b"delay_model" => &mut index.delay_model,
                    b"time_unit" => &mut index.time_unit,
                    b"voltage_unit" => &mut index.voltage_unit,
                    b"current_unit" => &mut index.current_unit,
                    b"leakage_power_unit" => &mut index.leakage_power_unit,
                    b"pulling_resistance_unit" => &mut index.pulling_resistance_unit,
                    b"default_operating_conditions" => &mut index.default_operating_conditions,
                    b"revision" => &mut index.revision,
                    b"date" => &mut index.date,
                    b"nom_process" => {
                        index.nom_process = number(value);
                        return;
                    }
                    b"nom_voltage" => {
                        index.nom_voltage = number(value);
                        return;
                    }
                    b"nom_temperature" => {
                        index.nom_temperature = number(value);
                        return;
                    }
                    _ => return,
                };
                *field = unquote(value);
            } else if let Some((b"capacitive_load_unit", args)) = header(text) {
                index.capacitive_load_unit = String::from_utf8_lossy(args)
                    .split(',')
                    .map(|part| part.trim().trim_matches('"').to_string())
                    .collect::<Vec<_>>()
                    .join("");
            }
        }
        Group::Conditions => {
            let (Some((name, value)), Some(conditions)) =
                (attribute(text), index.operating_conditions.last_mut())
            else {
                return;
            };
            match name {
                b"process" => conditions.process = number(value),
                b"voltage" => conditions.voltage = number(value),
                b"temperature" => conditions.temperature = number(value),
                _ => {}
            }
        }
        Group::Cell => {
            let (Some((name, value)), Some(cell)) = (attribute(text), index.cells.last_mut())
            else {
                return;
            };
            match name {
                b"area" => cell.area = number(value),
                b"cell_footprint" => cell.footprint = unquote(value),
                b"is_macro_cell" => cell.is_macro = truth(value),
                b"dont_use" => cell.dont_use = truth(value),
                _ => {}
            }
        }
        Group::Pin => {
            let (Some((name, value)), Some(pin)) = (
                attribute(text),
                index.cells.last_mut().and_then(|c| c.pins.last_mut()),
            ) else {
                return;
            };
            if matches!(name, b"direction" | b"pg_type") {
                pin.direction = unquote(value);
            }
        }
        Group::Other => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIB: &str = r#"/* a comment with a brace { and a quote " inside */
library ("demo_tt_025C_1v80") {
    define(def_sim_opt,library,string);
    technology("cmos");
    delay_model : "table_lookup";
    time_unit : "1ns";
    voltage_unit : "1V";
    capacitive_load_unit(1.0000000000, "pf");
    nom_process : 1
    nom_temperature : 25;
    nom_voltage : 1.8;
    default_operating_conditions : "tt_025C_1v80";
    operating_conditions ("tt_025C_1v80") {
        voltage : 1.8000000000;
        process : 1.0000000000;
        temperature : 25.000000000;
        tree_type : "balanced_tree";
    }
    lu_table_template ("del_1_7_7") {
        index_1("1, 2, 3");
        variable_1 : "input_net_transition";
    }
    cell ("inv_1") {
        leakage_power () {
            value : 0.0104575000;
            when : "A";
        }
        area : 3.7536000000;
        cell_footprint : "inv";
        pg_pin ("VPWR") {
            pg_type : "primary_power";
            voltage_name : "VPWR";
        }
        pin ("A") {
            capacitance : 0.0023020000;
            direction : "input";
        }
        pin ("Y") {
            direction : "output";
            function : "!A";
            timing () {
                cell_fall ("del_1_7_7") {
                    index_1("0.01, 0.02");
                    values("0.1, 0.2", \
                           "0.3, 0.4 } ; cell (\"fake\") {");
                }
                related_pin : "A";
            }
        }
    }
    cell (dff_1) {
        area : 20.0
        dont_use : true;
        ff ("IQ","IQ_N") {
            clocked_on : "CLK";
            next_state : "D";
        }
        test_cell () {
            pin (D) { direction : input; }
        }
        pin (D) { direction : input; }
        bus (Q) { bus_type : b; direction : output; pin (Q[0]) { } }
    }
    cell (ram) { is_macro_cell : true; area : 1e4; }
}
"#;

    #[test]
    fn header_and_cells() {
        let index = parse(LIB.as_bytes());
        assert_eq!(index.library, "demo_tt_025C_1v80");
        assert_eq!(index.delay_model, "table_lookup");
        assert_eq!(index.time_unit, "1ns");
        assert_eq!(index.capacitive_load_unit, "1.0000000000pf");
        assert_eq!(index.nom_process, Some(1.0));
        assert_eq!(index.nom_voltage, Some(1.8));
        assert_eq!(index.operating_conditions.len(), 1);
        let conditions = index.conditions();
        assert_eq!(conditions.name, "tt_025C_1v80");
        assert_eq!(conditions.voltage, Some(1.8));
        assert_eq!(conditions.temperature, Some(25.0));

        let names: Vec<_> = index.cells.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["inv_1", "dff_1", "ram"]);
        let inv = &index.cells[0];
        assert_eq!(inv.area, Some(3.7536));
        assert_eq!(inv.footprint, "inv");
        assert_eq!(inv.sequential, "");
        let pins: Vec<_> = inv
            .pins
            .iter()
            .map(|p| (p.name.as_str(), p.direction.as_str(), p.kind.as_str()))
            .collect();
        assert_eq!(
            pins,
            [
                ("VPWR", "primary_power", "pg_pin"),
                ("A", "input", "pin"),
                ("Y", "output", "pin")
            ]
        );
        let dff = &index.cells[1];
        assert_eq!(dff.sequential, "ff");
        assert!(dff.dont_use);
        assert_eq!(dff.area, Some(20.0));
        // The test cell's pins are not the cell's; the bus is one pin.
        let pins: Vec<_> = dff.pins.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(pins, ["D", "Q"]);
        assert_eq!(dff.pins[1].direction, "output");
        assert!(index.cells[2].is_macro);
    }

    #[test]
    fn nominal_values_stand_in_for_conditions() {
        let index = parse(
            b"library (x) {\n  nom_process : 1;\n  nom_temperature : -40;\n  nom_voltage : 1.32;\n}\n",
        );
        let conditions = index.conditions();
        assert_eq!(conditions.name, "");
        assert_eq!(conditions.voltage, Some(1.32));
        assert_eq!(conditions.temperature, Some(-40.0));
        assert!(index.cells.is_empty());
    }
}
