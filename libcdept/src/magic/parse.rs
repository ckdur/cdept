//! Readers for what Magic leaves behind: the marked lines of its output, the SPICE of
//! `ext2spice` and the LEF of `lef write`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::tcl::MARK;

/// The payload of every `CDEPT: <kind> <payload>` line of `output`, for one `kind`.
pub fn marked<'a>(output: &'a str, kind: &str) -> Vec<&'a str> {
    let head = format!("{MARK} {kind}");
    output
        .lines()
        .filter_map(|l| l.strip_prefix(head.as_str()))
        .filter_map(|rest| match rest {
            "" => Some(""),
            r => r.strip_prefix([' ', '\t']),
        })
        .collect()
}

/// The error the script reported, if any.
pub fn script_error(output: &str) -> Option<String> {
    marked(output, "error").first().map(|s| s.to_string())
}

/// Whether the script ran to its end.
pub fn finished(output: &str) -> bool {
    output
        .lines()
        .any(|l| l.trim_end() == format!("{MARK} done"))
}

/// The last `n` lines of `text`.
pub fn tail(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

// -- DRC --------------------------------------------------------------------------

/// A rule that is violated and the places it is violated at.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Violation {
    /// The rule's id as the technology writes it at the end of the message
    /// (`met1.2`), when it does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule: Option<String>,
    /// The whole message (`Metal1 spacing < 0.14 (met1.2)`).
    pub why: String,
    /// `[llx, lly, urx, ury]` in µm, in the coordinates of the top cell.
    pub boxes: Vec<[f64; 4]>,
}

/// Rule id out of a message: the last `(…)` without spaces.
pub fn rule_id(why: &str) -> Option<String> {
    let end = why.rfind(')')?;
    let start = why[..end].rfind('(')?;
    let id = &why[start + 1..end];
    (!id.is_empty() && !id.contains(char::is_whitespace)).then(|| id.to_string())
}

/// The violations in the output of [`super::tcl::drc`], in order of first appearance.
pub fn violations(output: &str) -> Vec<Violation> {
    let mut found: Vec<Violation> = Vec::new();
    for payload in marked(output, "drc") {
        let mut parts = payload.splitn(2, '\t');
        let (Some(why), Some(coords)) = (parts.next(), parts.next()) else {
            continue;
        };
        let nums: Vec<f64> = coords
            .split_whitespace()
            .filter_map(|n| n.parse().ok())
            .collect();
        let [a, b, c, d] = nums[..] else { continue };
        match found.iter_mut().find(|v| v.why == why) {
            Some(v) => v.boxes.push([a, b, c, d]),
            None => found.push(Violation {
                rule: rule_id(why),
                why: why.to_string(),
                boxes: vec![[a, b, c, d]],
            }),
        }
    }
    found
}

/// The DRC style Magic reports it used.
pub fn drc_style(output: &str) -> Option<String> {
    marked(output, "style").first().map(|s| s.to_string())
}

// -- SPICE ------------------------------------------------------------------------

/// A subcircuit of an extracted netlist.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Subckt {
    pub name: String,
    pub ports: Vec<String>,
}

/// What an extracted netlist holds.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SpiceSummary {
    pub subckts: Vec<Subckt>,
    /// Instances by model (transistors: the model; `X` lines: the subcircuit;
    /// resistors, capacitors and diodes: `R`, `C`, `D`).
    pub devices: BTreeMap<String, usize>,
}

/// Read the subcircuits and count the devices of SPICE text.
pub fn spice(text: &str) -> SpiceSummary {
    // Join continuation lines.
    let mut lines: Vec<String> = Vec::new();
    for raw in text.lines() {
        if let Some(rest) = raw.strip_prefix('+')
            && let Some(last) = lines.last_mut()
        {
            last.push(' ');
            last.push_str(rest.trim());
            continue;
        }
        lines.push(raw.trim().to_string());
    }
    let mut out = SpiceSummary::default();
    for line in &lines {
        let mut tokens = line.split_whitespace();
        let Some(first) = tokens.next() else { continue };
        if first.eq_ignore_ascii_case(".subckt") {
            let Some(name) = tokens.next() else { continue };
            out.subckts.push(Subckt {
                name: name.to_string(),
                ports: tokens
                    .take_while(|t| !t.contains('='))
                    .map(String::from)
                    .collect(),
            });
            continue;
        }
        let args: Vec<&str> = line.split_whitespace().collect();
        let model = match first.chars().next().map(|c| c.to_ascii_uppercase()) {
            Some('X') => args
                .iter()
                .skip(1)
                .take_while(|t| !t.contains('='))
                .last()
                .map(|s| s.to_string()),
            Some('M') => args.get(5).map(|s| s.to_string()),
            Some(c @ ('R' | 'C' | 'D' | 'L')) => Some(c.to_string()),
            _ => None,
        };
        if let Some(model) = model {
            *out.devices.entry(model).or_default() += 1;
        }
    }
    out
}

// -- LEF --------------------------------------------------------------------------

/// A pin of a LEF macro.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LefPin {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direction: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<String>,
}

/// The macro of a LEF file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LefSummary {
    pub name: String,
    /// Width and height in µm.
    pub size: Option<[f64; 2]>,
    pub pins: Vec<LefPin>,
}

/// Name, size and pins of the first macro of LEF text.
pub fn lef(text: &str) -> Option<LefSummary> {
    let mut out: Option<LefSummary> = None;
    let mut pin: Option<LefPin> = None;
    for line in text.lines() {
        let t: Vec<&str> = line
            .trim()
            .trim_end_matches(';')
            .split_whitespace()
            .collect();
        match (t.as_slice(), out.as_mut()) {
            (["MACRO", name], None) => {
                out = Some(LefSummary {
                    name: name.to_string(),
                    ..Default::default()
                })
            }
            (["MACRO", ..], Some(_)) => break,
            (["SIZE", w, "BY", h], Some(m)) => {
                if let (Ok(w), Ok(h)) = (w.parse(), h.parse()) {
                    m.size = Some([w, h]);
                }
            }
            (["PIN", name], Some(_)) => {
                pin = Some(LefPin {
                    name: name.to_string(),
                    direction: None,
                    usage: None,
                })
            }
            (["DIRECTION", d, ..], Some(_)) => {
                if let Some(p) = pin.as_mut() {
                    p.direction = Some(d.to_string());
                }
            }
            (["USE", u], Some(_)) => {
                if let Some(p) = pin.as_mut() {
                    p.usage = Some(u.to_string());
                }
            }
            (["END", name], Some(m)) => {
                if pin.as_ref().is_some_and(|p| p.name == *name) {
                    m.pins.extend(pin.take());
                } else if *name == m.name {
                    break;
                }
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drc_lines_group_by_message() {
        let out = "Total DRC errors found: 3\n\
CDEPT: drc\tMetal1 spacing < 0.14 (met1.2)\t1.1 0 1.14 1\n\
CDEPT: drc\tMetal1 minimum area < 0.083 (met1.6)\t3 0 3.05 1\n\
CDEPT: drc\tMetal1 spacing < 0.14 (met1.2)\t0.96 0 1 1\n\
CDEPT: style drc(fast)\n\
CDEPT: done\n";
        let v = violations(out);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].rule.as_deref(), Some("met1.2"));
        assert_eq!(v[0].boxes, [[1.1, 0.0, 1.14, 1.0], [0.96, 0.0, 1.0, 1.0]]);
        assert_eq!(v[1].rule.as_deref(), Some("met1.6"));
        assert_eq!(drc_style(out).as_deref(), Some("drc(fast)"));
        assert!(finished(out));
        assert_eq!(script_error(out), None);
    }

    #[test]
    fn rule_ids() {
        assert_eq!(rule_id("Spacing < 0.2 (m1.2)").as_deref(), Some("m1.2"));
        assert_eq!(rule_id("No parens"), None);
        assert_eq!(rule_id("Text (see rule list)"), None);
    }

    #[test]
    fn errors_are_found() {
        let out = "junk\nCDEPT: error no such cell in the GDS file\n";
        assert_eq!(
            script_error(out).as_deref(),
            Some("no such cell in the GDS file")
        );
        assert!(!finished(out));
    }

    #[test]
    fn spice_summary() {
        let text = "* extracted\n.subckt inv_1 A VGND VNB VPB VPWR Y\n\
X0 Y A VGND VNB sky130_fd_pr__nfet_01v8 w=0.65 l=0.15\n\
+ ad=0.1\n\
M1 Y A VPWR VPB pfet w=1 l=1\n\
C0 A VGND 1f\n\
.ends\n";
        let s = spice(text);
        assert_eq!(s.subckts.len(), 1);
        assert_eq!(s.subckts[0].ports, ["A", "VGND", "VNB", "VPB", "VPWR", "Y"]);
        assert_eq!(s.devices["sky130_fd_pr__nfet_01v8"], 1);
        assert_eq!(s.devices["pfet"], 1);
        assert_eq!(s.devices["C"], 1);
    }

    #[test]
    fn lef_summary() {
        let text = "VERSION 5.7 ;\nMACRO inv\n  CLASS CORE ;\n  SIZE 1.38 BY 2.72 ;\n  PIN A\n    DIRECTION INPUT ;\n    USE SIGNAL ;\n    PORT\n    END\n  END A\n  PIN VPWR\n    DIRECTION INOUT ;\n    USE POWER ;\n  END VPWR\nEND inv\nEND LIBRARY\n";
        let l = lef(text).unwrap();
        assert_eq!(l.name, "inv");
        assert_eq!(l.size, Some([1.38, 2.72]));
        assert_eq!(l.pins.len(), 2);
        assert_eq!(l.pins[1].usage.as_deref(), Some("POWER"));
    }
}
