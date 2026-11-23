//! The reference netlist for KLayout's LVS: a copy of the netlist with the PDK's device
//! calls written the way the deck's schematic reader takes them.
//!
//! A PDK's SPICE netlist writes a transistor as a subcircuit call
//! (`XN0 Y A VSS VSS sg13_lv_nmos w=740n l=130n`), because that is what its simulation
//! models are. KLayout's SPICE reader makes a *subcircuit* of such a line, and the LVS
//! deck compares *devices*: the schematic side comes out empty. The calls whose model is a
//! device of the technology (the snapshot's device list, `tech.catalog.json`) are
//! therefore written as `M`, `R`, `C`, `D` or `Q` lines in a copy of the netlist; the
//! PDK's own file is never edited. Calls of anything else (a standard cell, a block) stay
//! as they are, and are reported.

use crate::tech::Catalog;
use std::collections::{BTreeMap, BTreeSet};

/// The models that are devices of the technology, each with the letter of the SPICE
/// element that stands for it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviceModels {
    models: BTreeMap<String, char>,
    /// No snapshot was found: models are told by their names.
    guess: bool,
}

impl DeviceModels {
    /// The devices of the snapshot's catalog. Without devices there (a project without
    /// a snapshot) the names of the models decide: `nmos`, `pmos`, `nfet`, `pfet`.
    pub fn from_catalog(catalog: Option<&Catalog>) -> DeviceModels {
        let mut out = DeviceModels::default();
        if let Some(catalog) = catalog {
            for device in &catalog.devices.devices {
                let model = if device.model.is_empty() {
                    &device.name
                } else {
                    &device.model
                };
                if let Some(letter) = element_letter(&device.kind, &device.class) {
                    out.models.insert(model.to_lowercase(), letter);
                }
            }
        }
        out.guess = out.models.is_empty();
        out
    }

    /// Models given by name and element letter.
    pub fn from_pairs(pairs: &[(&str, char)]) -> DeviceModels {
        DeviceModels {
            models: pairs
                .iter()
                .map(|(model, letter)| (model.to_lowercase(), *letter))
                .collect(),
            guess: false,
        }
    }

    fn letter(&self, model: &str) -> Option<char> {
        let key = model.to_lowercase();
        if let Some(letter) = self.models.get(&key) {
            return Some(*letter);
        }
        if self.guess
            && ["nmos", "pmos", "nfet", "pfet"]
                .iter()
                .any(|word| key.contains(word))
        {
            return Some('M');
        }
        None
    }
}

/// The SPICE element of a device of the snapshot: from what xschem calls its type
/// (`nmos`, `res`, ...) and the class CDePT gave it.
fn element_letter(kind: &str, class: &str) -> Option<char> {
    match (kind, class) {
        ("nmos" | "pmos", _) | (_, "mos_lv" | "mos_hv" | "mos") => Some('M'),
        (_, "bjt") | ("vertical_npn" | "lateral_pnp" | "pnp" | "npn", _) => Some('Q'),
        (_, "diode") => Some('D'),
        (_, "res") => Some('R'),
        (_, "cap") => Some('C'),
        _ => None,
    }
}

/// What [`device_calls`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rewrite {
    /// The netlist with the device calls rewritten.
    pub text: String,
    /// How many calls were rewritten.
    pub converted: usize,
    /// The models of the device calls rewritten (lower case).
    pub models: BTreeSet<String>,
    /// The models of the `X` calls that were kept: no device of the technology.
    pub kept: BTreeSet<String>,
}

impl Rewrite {
    /// One sentence for the output of a check; empty when nothing was changed or kept.
    pub fn note(&self) -> String {
        let mut parts = Vec::new();
        if self.converted > 0 {
            parts.push(format!(
                "{} device call(s) written as devices ({}) in the reference netlist",
                self.converted,
                self.models.iter().cloned().collect::<Vec<_>>().join(", ")
            ));
        }
        if !self.kept.is_empty() {
            parts.push(format!(
                "kept as subcircuit calls: {}",
                self.kept.iter().cloned().collect::<Vec<_>>().join(", ")
            ));
        }
        parts.join("; ")
    }
}

/// Write the calls of `models` in `text` as devices. Every other line is kept as it is,
/// continuation lines (`+`) of a rewritten call are joined into it.
pub fn device_calls(text: &str, models: &DeviceModels) -> Rewrite {
    let lines: Vec<&str> = text.lines().collect();
    // The names in use, to keep the new ones apart from them.
    let mut names: BTreeSet<String> = lines
        .iter()
        .filter_map(|l| l.split_whitespace().next())
        .map(str::to_lowercase)
        .collect();
    let mut out = Rewrite::default();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        // The logical line: this one and the `+` lines that follow.
        let mut end = i + 1;
        while end < lines.len() && lines[end].trim_start().starts_with('+') {
            end += 1;
        }
        let first = line.trim_start();
        let is_call = first.starts_with(['X', 'x']);
        if is_call {
            let mut joined = line.trim_end().to_string();
            for more in &lines[i + 1..end] {
                joined.push(' ');
                joined.push_str(more.trim_start().trim_start_matches('+').trim());
            }
            if let Some(done) = rewrite_call(&joined, models, &mut names, &mut out) {
                out.text.push_str(&done);
                out.text.push('\n');
                i = end;
                continue;
            }
        }
        for original in &lines[i..end] {
            out.text.push_str(original);
            out.text.push('\n');
        }
        i = end;
    }
    out
}

/// `XN0 Y A VSS VSS sg13_lv_nmos w=...` as `MN0 Y A VSS VSS sg13_lv_nmos w=...`; `None`
/// when the call is not one of a device model (it is noted in `out`).
fn rewrite_call(
    line: &str,
    models: &DeviceModels,
    names: &mut BTreeSet<String>,
    out: &mut Rewrite,
) -> Option<String> {
    let tokens: Vec<&str> = line.split_whitespace().collect();
    // Name, nets, the model, then `name=value` parameters.
    let model_at = tokens.iter().rposition(|t| !t.contains('='))?;
    if model_at < 2 {
        return None;
    }
    let model = tokens[model_at];
    let Some(letter) = models.letter(model) else {
        out.kept.insert(model.to_lowercase());
        return None;
    };
    let rest = &tokens[0][1..];
    let mut name = format!("{letter}{rest}");
    if names.contains(&name.to_lowercase()) {
        name = format!("{letter}{}", tokens[0]);
    }
    names.insert(name.to_lowercase());
    out.converted += 1;
    out.models.insert(model.to_lowercase());
    let mut done = vec![name.as_str()];
    done.extend(&tokens[1..]);
    Some(done.join(" "))
}

/// The other way round, for netgen: the `M`, `D` and `Q` lines of `models` written as
/// `X` calls of the model, which is how the PDK's own netlists have them. Netgen reads
/// `X sg13_lv_nmos` as a cell of four pins (`1`..`4`) and `M ... sg13_lv_nmos` as a
/// MOSFET (drain, gate, source, bulk): the two never match, whatever the nets are. `R`
/// and `C` lines are not touched (their value comes before the model).
pub fn calls_of_devices(text: &str, models: &DeviceModels) -> Rewrite {
    let lines: Vec<&str> = text.lines().collect();
    let mut names: BTreeSet<String> = lines
        .iter()
        .filter_map(|l| l.split_whitespace().next())
        .map(str::to_lowercase)
        .collect();
    let mut out = Rewrite::default();
    let mut i = 0;
    while i < lines.len() {
        let mut end = i + 1;
        while end < lines.len() && lines[end].trim_start().starts_with('+') {
            end += 1;
        }
        let first = lines[i].trim_start();
        if first.starts_with(['M', 'm', 'D', 'd', 'Q', 'q']) {
            let mut joined = lines[i].trim_end().to_string();
            for more in &lines[i + 1..end] {
                joined.push(' ');
                joined.push_str(more.trim_start().trim_start_matches('+').trim());
            }
            let tokens: Vec<&str> = joined.split_whitespace().collect();
            let letter = first.chars().next().map(|c| c.to_ascii_uppercase());
            if let Some(model_at) = tokens.iter().rposition(|t| !t.contains('='))
                && model_at >= 2
                && models.letter(tokens[model_at]) == letter
            {
                let mut name = format!("X{}", &tokens[0][1..]);
                if names.contains(&name.to_lowercase()) {
                    name = format!("X{}", tokens[0]);
                }
                names.insert(name.to_lowercase());
                out.converted += 1;
                out.models.insert(tokens[model_at].to_lowercase());
                out.text.push_str(&name);
                for token in &tokens[1..] {
                    out.text.push(' ');
                    out.text.push_str(token);
                }
                out.text.push('\n');
                i = end;
                continue;
            }
        }
        for original in &lines[i..end] {
            out.text.push_str(original);
            out.text.push('\n');
        }
        i = end;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ihp() -> DeviceModels {
        DeviceModels::from_pairs(&[
            ("sg13_lv_nmos", 'M'),
            ("sg13_lv_pmos", 'M'),
            ("rsil", 'R'),
            ("cap_cmim", 'C'),
        ])
    }

    const INV: &str = ".subckt sg13g2_inv_1 Y A VDD VSS
XN0 Y A VSS VSS sg13_lv_nmos w=740.00n l=130.00n ng=1 ad=2.516e-13 as=2.516e-13 pd=2.16e-06 ps=2.16e-06 m=1
XP0 Y A VDD VDD sg13_lv_pmos w=1.12u l=130.00n ng=1 ad=3.808e-13 as=3.808e-13 pd=2.92e-06 ps=2.92e-06 m=1
.ends
";

    #[test]
    fn ihp_transistors_become_mos_lines() {
        let out = device_calls(INV, &ihp());
        assert_eq!(out.converted, 2);
        assert!(out.kept.is_empty());
        let lines: Vec<&str> = out.text.lines().collect();
        assert_eq!(lines[0], ".subckt sg13g2_inv_1 Y A VDD VSS");
        assert!(lines[1].starts_with("MN0 Y A VSS VSS sg13_lv_nmos w=740.00n l=130.00n"));
        assert!(lines[2].starts_with("MP0 Y A VDD VDD sg13_lv_pmos w=1.12u"));
        assert_eq!(lines[3], ".ends");
        assert!(out.note().contains("2 device call(s)"));
    }

    #[test]
    fn sky130_numbered_calls_keep_unique_names() {
        let sky = DeviceModels::from_pairs(&[
            ("sky130_fd_pr__nfet_01v8", 'M'),
            ("sky130_fd_pr__pfet_01v8_hvt", 'M'),
        ]);
        let text = ".subckt sky130_fd_sc_hd__inv_1 A VGND VNB VPB VPWR Y
X0 VGND A Y VNB sky130_fd_pr__nfet_01v8 w=650000u l=150000u
X1 VPWR A Y VPB sky130_fd_pr__pfet_01v8_hvt w=1e+06u l=150000u
.ends
";
        let out = device_calls(text, &sky);
        assert_eq!(out.converted, 2);
        assert!(
            out.text
                .contains("M0 VGND A Y VNB sky130_fd_pr__nfet_01v8 w=650000u l=150000u")
        );
        assert!(
            out.text
                .contains("M1 VPWR A Y VPB sky130_fd_pr__pfet_01v8_hvt")
        );
        // A name that already exists is not used twice.
        let text = "M0 a b c d sky130_fd_pr__nfet_01v8\nX0 a b c d sky130_fd_pr__nfet_01v8\n";
        let out = device_calls(text, &sky);
        assert_eq!(
            out.text.lines().nth(1).unwrap().split(' ').next(),
            Some("MX0")
        );
    }

    #[test]
    fn other_calls_and_lines_stay() {
        let text = "* comment\nX1 a b sg13g2_inv_1\nxr1 a b gnd rsil w=1u l=2u\nM5 a b c d nmos\n\
                    XM1 d g s b sg13_lv_nmos\n+ w=1u l=0.13u\n";
        let out = device_calls(text, &ihp());
        assert_eq!(out.converted, 2);
        assert_eq!(
            out.kept.iter().map(String::as_str).collect::<Vec<_>>(),
            ["sg13g2_inv_1"]
        );
        let lines: Vec<&str> = out.text.lines().collect();
        assert_eq!(lines[0], "* comment");
        assert_eq!(lines[1], "X1 a b sg13g2_inv_1");
        assert_eq!(lines[2], "Rr1 a b gnd rsil w=1u l=2u");
        assert_eq!(lines[3], "M5 a b c d nmos");
        assert_eq!(lines[4], "MM1 d g s b sg13_lv_nmos w=1u l=0.13u");
        assert!(
            out.note()
                .contains("kept as subcircuit calls: sg13g2_inv_1")
        );
    }

    #[test]
    fn klayout_devices_become_calls_for_netgen() {
        let text = ".SUBCKT sg13g2_inv_1 A VDD VSS Y
M$1 VSS A Y VSS sg13_lv_nmos L=0.13u W=0.74u AS=0.2516p AD=0.2516p PS=2.16u
+ PD=2.16u rfmode=0
M$2 VDD A Y VDD sg13_lv_pmos L=0.13u W=1.12u
.ENDS sg13g2_inv_1
";
        let out = calls_of_devices(text, &ihp());
        assert_eq!(out.converted, 2);
        let lines: Vec<&str> = out.text.lines().collect();
        assert_eq!(
            lines[1],
            "X$1 VSS A Y VSS sg13_lv_nmos L=0.13u W=0.74u AS=0.2516p AD=0.2516p PS=2.16u PD=2.16u rfmode=0"
        );
        assert!(lines[2].starts_with("X$2 VDD A Y VDD sg13_lv_pmos"));
        assert_eq!(lines[3], ".ENDS sg13g2_inv_1");
        // A device of a model that is not a MOS stays.
        let out = calls_of_devices("M1 a b c d other_model w=1\n", &ihp());
        assert_eq!(out.converted, 0);
    }

    #[test]
    fn without_a_snapshot_the_names_decide() {
        let none = DeviceModels::from_catalog(None);
        let out = device_calls(
            "X0 d g s b sky130_fd_pr__nfet_01v8 w=1\nX1 a b res_x\n",
            &none,
        );
        assert_eq!(out.converted, 1);
        assert_eq!(out.kept.len(), 1);
    }

    #[test]
    fn element_of_a_kind() {
        assert_eq!(element_letter("nmos", "mos_lv"), Some('M'));
        assert_eq!(element_letter("vertical_npn", "bjt"), Some('Q'));
        assert_eq!(element_letter("res", "res"), Some('R'));
        assert_eq!(element_letter("capacitor", "cap"), Some('C'));
        assert_eq!(element_letter("diode", "diode"), Some('D'));
        assert_eq!(element_letter("pad", "pad"), None);
        assert_eq!(element_letter("res", "tap"), None);
    }
}
