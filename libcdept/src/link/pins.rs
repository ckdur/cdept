//! Pins of a layout against its schematic and its stream (docs/LINKING.md §9).
//!
//! A pin is a CDePT object: the layout's sidecar lists it ([`crate::layout::Pin`]).
//! What the stream holds of it depends on the technology ([`PinRules`]): a shape on the
//! conductor's pin layer and a text on its label layer, both carrying the GDS property
//! [`PIN_PROPERTY`] with the pin's name; nothing, when the technology has no pin layer
//! (the editor draws such pins over the layout).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;

use eyre::Result;
use serde::Serialize;

use super::devices::PinRules;
use crate::layout::{Meta, Pin as LayoutPin};

/// The GDS property (`PROPATTR`) holding a pin's name on its shapes and label.
pub const PIN_PROPERTY: u16 = 62;

/// `input`, `output` or `inout` from xschem's `i`/`o`/`io` (ipin, opin, iopin).
pub fn direction(text: &str) -> &'static str {
    match text.trim().to_ascii_lowercase().as_str() {
        "i" | "in" | "ipin" | "input" => "input",
        "o" | "out" | "opin" | "output" => "output",
        _ => "inout",
    }
}

/// What is wrong with a pin, or that it is fine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PinState {
    /// The schematic has it and so does the layout.
    Placed,
    /// A pin of the schematic the layout does not have.
    NotPlaced,
    /// A pin of the layout the schematic does not have.
    Extra,
    /// Not the schematic's direction.
    Direction,
    /// Its name is written outside its shapes.
    LabelOff,
    /// The technology has a pin layer for its conductor, but the stream has no shape
    /// of it there (the file was changed outside CDePT).
    NotInStream,
    /// The pin has no shapes.
    NoShape,
}

impl PinState {
    pub fn label(self) -> &'static str {
        match self {
            PinState::Placed => "placed",
            PinState::NotPlaced => "not placed",
            PinState::Extra => "not in the schematic",
            PinState::Direction => "other direction",
            PinState::LabelOff => "label off the pin",
            PinState::NotInStream => "not in the layout file",
            PinState::NoShape => "no shape",
        }
    }

    /// Fine (placed, nothing else).
    pub fn is_ok(self) -> bool {
        self == PinState::Placed
    }
}

/// One pin and what was found about it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PinCheck {
    pub name: String,
    /// The layout's direction, else the schematic's.
    pub direction: String,
    /// The conductors its shapes are on.
    pub layers: Vec<String>,
    /// Every finding: `[Placed]` when fine.
    pub states: Vec<PinState>,
    /// For people: e.g. "layout input, schematic output".
    pub detail: String,
}

impl PinCheck {
    pub fn is_ok(&self) -> bool {
        self.states.iter().all(|s| s.is_ok())
    }
}

/// A pin of the schematic: name and direction (`i`, `o`, `io` or the long forms).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wanted<'a> {
    pub name: &'a str,
    pub direction: &'a str,
}

/// Compare the layout's pins (`pins`, the sidecar's) with the schematic's (`wanted`,
/// `None`: no schematic) and the stream (`tagged`: names of the pins with a shape
/// on a pin layer in the top cell, `None`: not read).
pub fn check(
    pins: &BTreeMap<String, LayoutPin>,
    wanted: Option<&[Wanted<'_>]>,
    rules: Option<&PinRules>,
    tagged: Option<&BTreeSet<String>>,
) -> Vec<PinCheck> {
    let mut out = Vec::new();
    let schematic: BTreeMap<&str, &str> = wanted
        .unwrap_or_default()
        .iter()
        .map(|w| (w.name, direction(w.direction)))
        .collect();
    for (name, pin) in pins {
        let mut states = Vec::new();
        let mut detail = Vec::new();
        match schematic.get(name.as_str()) {
            None if wanted.is_some() => states.push(PinState::Extra),
            Some(d) if *d != direction(&pin.direction) => {
                states.push(PinState::Direction);
                detail.push(format!(
                    "layout {}, schematic {d}",
                    direction(&pin.direction)
                ));
            }
            _ => {}
        }
        if pin.shapes.is_empty() {
            states.push(PinState::NoShape);
        } else if !pin.label_on_shape() {
            states.push(PinState::LabelOff);
        }
        if let (Some(rules), Some(tagged)) = (rules, tagged) {
            let has_pin_layer = pin.shapes.iter().any(|s| {
                rules
                    .layers
                    .iter()
                    .any(|l| (l.drawing == s.layer || l.drawing == "*") && l.pin.is_some())
            });
            if has_pin_layer && !tagged.contains(name) {
                states.push(PinState::NotInStream);
            }
        }
        if states.is_empty() {
            states.push(PinState::Placed);
        }
        let mut layers: Vec<String> = pin.shapes.iter().map(|s| s.layer.clone()).collect();
        layers.dedup();
        out.push(PinCheck {
            name: name.clone(),
            direction: direction(&pin.direction).to_string(),
            layers,
            states,
            detail: detail.join("; "),
        });
    }
    for (name, d) in &schematic {
        if !pins.contains_key(*name) {
            out.push(PinCheck {
                name: name.to_string(),
                direction: d.to_string(),
                layers: Vec::new(),
                states: vec![PinState::NotPlaced],
                detail: String::new(),
            });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// The names of the pins with a shape on a pin layer in structure `top` of a GDS file
/// (`None` for a file this cannot read the shapes of: OASIS).
pub fn tagged_in_stream(
    file: &Path,
    top: &str,
    rules: &PinRules,
) -> Result<Option<BTreeSet<String>>> {
    if crate::layout::is_oasis(file) {
        return Ok(None);
    }
    let bytes = std::fs::read(file)?;
    let pin_layers: BTreeSet<&str> = rules
        .layers
        .iter()
        .filter_map(|l| l.pin.as_deref())
        .collect();
    Ok(Some(
        crate::layout::gds::tagged_shapes(&bytes, PIN_PROPERTY)?
            .into_iter()
            .filter(|t| t.structure == top && !t.text)
            .filter(|t| pin_layers.contains(format!("{}/{}", t.layer, t.datatype).as_str()))
            .map(|t| t.value)
            .collect(),
    ))
}

/// The check of a layout file's pins, from its sidecar, its stream and (when given)
/// the schematic's pins.
pub fn check_file(
    file: &Path,
    meta: Option<&Meta>,
    wanted: Option<&[Wanted<'_>]>,
    rules: Option<&PinRules>,
) -> Result<Vec<PinCheck>> {
    let empty = BTreeMap::new();
    let pins = meta.map_or(&empty, |m| &m.pins);
    let tagged = match (rules, meta) {
        (Some(rules), Some(meta)) => tagged_in_stream(file, &meta.top, rules)?,
        _ => None,
    };
    Ok(check(pins, wanted, rules, tagged.as_ref()))
}

/// The pin checks as lines for people.
pub struct Report<'a>(pub &'a [PinCheck]);

impl fmt::Display for Report<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for c in self.0 {
            let states: Vec<&str> = c.states.iter().map(|s| s.label()).collect();
            let layers = if c.layers.is_empty() {
                String::new()
            } else {
                format!(" on {}", c.layers.join(", "))
            };
            write!(
                f,
                "pin {} ({}){}: {}",
                c.name,
                c.direction,
                layers,
                states.join(", ")
            )?;
            if !c.detail.is_empty() {
                write!(f, " ({})", c.detail)?;
            }
            writeln!(f)?;
        }
        let ok = self.0.iter().filter(|c| c.is_ok()).count();
        write!(f, "pins: {ok} ok, {} with findings", self.0.len() - ok)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::PinShape;
    use crate::link::devices::PinLayer;

    fn pin(dir: &str, b: [f64; 4], label: Option<[f64; 2]>) -> LayoutPin {
        LayoutPin {
            direction: dir.into(),
            net: None,
            shapes: vec![PinShape {
                layer: "8/0".into(),
                r#box: Some(b),
                polygon: None,
            }],
            label,
        }
    }

    #[test]
    fn pins_against_schematic_and_stream() {
        let pins = BTreeMap::from([
            ("A".to_string(), pin("input", [0.0, 0.0, 1.0, 1.0], None)),
            (
                "Y".to_string(),
                pin("input", [0.0, 0.0, 1.0, 1.0], Some([5.0, 5.0])),
            ),
            ("Q".to_string(), pin("inout", [0.0, 0.0, 1.0, 1.0], None)),
        ]);
        let wanted = [
            Wanted {
                name: "A",
                direction: "i",
            },
            Wanted {
                name: "Y",
                direction: "o",
            },
            Wanted {
                name: "VDD",
                direction: "io",
            },
        ];
        let rules = PinRules {
            size: 0.1,
            r#box: 0.5,
            generate: Some("8/0".into()),
            layers: vec![PinLayer {
                drawing: "8/0".into(),
                pin: Some("8/2".into()),
                label: Some("8/25".into()),
            }],
        };
        let tagged = BTreeSet::from(["A".to_string(), "Y".to_string()]);
        let found = check(&pins, Some(&wanted), Some(&rules), Some(&tagged));
        let by: BTreeMap<&str, &PinCheck> = found.iter().map(|c| (c.name.as_str(), c)).collect();
        assert_eq!(by["A"].states, [PinState::Placed]);
        assert!(by["A"].is_ok());
        assert_eq!(by["Y"].states, [PinState::Direction, PinState::LabelOff]);
        assert_eq!(by["Y"].detail, "layout input, schematic output");
        assert_eq!(by["Q"].states, [PinState::Extra, PinState::NotInStream]);
        assert_eq!(by["VDD"].states, [PinState::NotPlaced]);
        assert_eq!(by["VDD"].direction, "inout");
        let text = Report(&found).to_string();
        assert!(
            text.contains("pin Y (input) on 8/0: other direction, label off the pin"),
            "{text}"
        );
        assert!(text.ends_with("pins: 1 ok, 3 with findings"), "{text}");
        // Without a schematic nothing is extra; without the stream nothing is missing
        // from it.
        let alone = check(&pins, None, None, None);
        assert!(alone.iter().all(|c| !c.states.contains(&PinState::Extra)));
        assert!(
            alone
                .iter()
                .all(|c| !c.states.contains(&PinState::NotInStream))
        );
    }
}
