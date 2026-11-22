//! LEF: the macros of a cell LEF (class, size, site, pins) and the layers, sites and
//! units of a technology LEF. Geometry (`PORT`, `OBS`) is skipped.

use serde::{Deserialize, Serialize};

/// What a LEF file holds.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LefIndex {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub version: String,
    /// `UNITS DATABASE MICRONS`: database units per micron.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database_microns: Option<f64>,
    /// `MANUFACTURINGGRID` in micron.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manufacturing_grid: Option<f64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<LefLayer>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sites: Vec<LefSite>,
    /// Names of the `VIA` definitions.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub vias: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub macros: Vec<LefMacro>,
}

impl LefIndex {
    /// A technology LEF: layers or sites, no macros.
    pub fn is_technology(&self) -> bool {
        self.macros.is_empty() && !(self.layers.is_empty() && self.sites.is_empty())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LefLayer {
    pub name: String,
    /// `ROUTING`, `CUT`, `MASTERSLICE`, `OVERLAP`, …
    #[serde(skip_serializing_if = "String::is_empty")]
    pub kind: String,
    /// `HORIZONTAL` or `VERTICAL` (routing layers).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub direction: String,
    /// Micron; the first value when the pitch is given per direction.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pitch: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LefSite {
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub class: String,
    /// Width and height in micron.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<(f64, f64)>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub symmetry: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LefMacro {
    pub name: String,
    /// `CORE`, `CORE SPACER`, `BLOCK`, `PAD INOUT`, `ENDCAP …`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub class: String,
    /// Width and height in micron.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<(f64, f64)>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub site: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub symmetry: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pins: Vec<LefPin>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LefPin {
    pub name: String,
    /// `INPUT`, `OUTPUT`, `INOUT`, `FEEDTHRU` (`OUTPUT TRISTATE` reads `OUTPUT TRISTATE`).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub direction: String,
    /// `SIGNAL`, `POWER`, `GROUND`, `CLOCK`, `ANALOG`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub usage: String,
}

/// Where the reader is.
enum Block {
    Top,
    /// `UNITS`, `PROPERTYDEFINITIONS`, `VIA`, `VIARULE`, `SPACING`, `NONDEFAULTRULE`, …:
    /// skipped up to its `END <name>`.
    Skip(Vec<u8>),
    Layer,
    Site,
    Macro,
    Pin,
    /// `PORT` or `OBS` inside a macro: geometry up to a bare `END`. The flag says
    /// whether it is inside a pin.
    Geometry(bool),
}

fn tokens(line: &[u8]) -> impl Iterator<Item = &[u8]> {
    line.split(|b| b.is_ascii_whitespace() || *b == b';')
        .filter(|t| !t.is_empty())
}

fn text(token: &[u8]) -> String {
    String::from_utf8_lossy(token).into_owned()
}

fn number(token: Option<&[u8]>) -> Option<f64> {
    std::str::from_utf8(token?).ok()?.parse().ok()
}

/// The words after the keyword, joined (`CLASS CORE SPACER ;` → `CORE SPACER`).
fn rest<'a>(mut words: impl Iterator<Item = &'a [u8]>) -> String {
    let mut out = String::new();
    for word in words.by_ref() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&String::from_utf8_lossy(word));
    }
    out
}

/// `SIZE w BY h`.
fn size<'a>(mut words: impl Iterator<Item = &'a [u8]>) -> Option<(f64, f64)> {
    let w = number(words.next())?;
    words.next()?;
    let h = number(words.next())?;
    Some((w, h))
}

/// Read a LEF file's text. Never fails: what is not understood is skipped.
pub fn parse(bytes: &[u8]) -> LefIndex {
    let mut index = LefIndex::default();
    let mut block = Block::Top;
    for line in bytes.split(|b| *b == b'\n') {
        let line = match memchr::memchr(b'#', line) {
            Some(at) => &line[..at],
            None => line,
        };
        let mut words = tokens(line);
        let Some(key) = words.next() else { continue };
        match &mut block {
            Block::Top => match key {
                b"VERSION" => index.version = rest(words),
                b"MANUFACTURINGGRID" => index.manufacturing_grid = number(words.next()),
                b"UNITS" => block = Block::Skip(b"UNITS".to_vec()),
                b"DATABASE" => {}
                b"PROPERTYDEFINITIONS" => block = Block::Skip(key.to_vec()),
                b"LAYER" => {
                    index.layers.push(LefLayer {
                        name: words.next().map(text).unwrap_or_default(),
                        ..LefLayer::default()
                    });
                    block = Block::Layer;
                }
                b"SITE" => {
                    index.sites.push(LefSite {
                        name: words.next().map(text).unwrap_or_default(),
                        ..LefSite::default()
                    });
                    block = Block::Site;
                }
                b"MACRO" => {
                    index.macros.push(LefMacro {
                        name: words.next().map(text).unwrap_or_default(),
                        ..LefMacro::default()
                    });
                    block = Block::Macro;
                }
                b"VIA" | b"VIARULE" | b"NONDEFAULTRULE" => {
                    let name = words.next().unwrap_or_default();
                    if key == b"VIA" {
                        index.vias.push(text(name));
                    }
                    block = Block::Skip(name.to_vec());
                }
                b"SPACING" | b"IRDROP" | b"ARRAY" | b"BEGINEXT" => {
                    block = Block::Skip(key.to_vec());
                }
                _ => {}
            },
            Block::Skip(name) => {
                if key == b"DATABASE"
                    && name.as_slice() == b"UNITS"
                    && words.next() == Some(b"MICRONS")
                {
                    index.database_microns = number(words.next());
                } else if key == b"ENDEXT"
                    || (key == b"END" && words.next().is_some_and(|n| n == name.as_slice()))
                {
                    block = Block::Top;
                }
            }
            Block::Layer => {
                let layer = index.layers.last_mut().expect("in a layer");
                match key {
                    b"TYPE" => layer.kind = rest(words),
                    b"DIRECTION" => layer.direction = rest(words),
                    b"PITCH" => layer.pitch = number(words.next()),
                    b"WIDTH" => layer.width = number(words.next()),
                    b"END" if words.next().is_some_and(|n| n == layer.name.as_bytes()) => {
                        block = Block::Top;
                    }
                    _ => {}
                }
            }
            Block::Site => {
                let site = index.sites.last_mut().expect("in a site");
                match key {
                    b"CLASS" => site.class = rest(words),
                    b"SIZE" => site.size = size(words),
                    b"SYMMETRY" => site.symmetry = rest(words),
                    b"END" => block = Block::Top,
                    _ => {}
                }
            }
            Block::Macro => {
                let cell = index.macros.last_mut().expect("in a macro");
                match key {
                    b"CLASS" => cell.class = rest(words),
                    b"SIZE" => cell.size = size(words),
                    b"SITE" => cell.site = words.next().map(text).unwrap_or_default(),
                    b"SYMMETRY" => cell.symmetry = rest(words),
                    b"PIN" => {
                        cell.pins.push(LefPin {
                            name: words.next().map(text).unwrap_or_default(),
                            ..LefPin::default()
                        });
                        block = Block::Pin;
                    }
                    b"OBS" => block = Block::Geometry(false),
                    b"END" => block = Block::Top,
                    _ => {}
                }
            }
            Block::Pin => {
                let pin = index
                    .macros
                    .last_mut()
                    .and_then(|m| m.pins.last_mut())
                    .expect("in a pin");
                match key {
                    b"DIRECTION" => pin.direction = rest(words),
                    b"USE" => pin.usage = rest(words),
                    b"PORT" => block = Block::Geometry(true),
                    b"END" => block = Block::Macro,
                    _ => {}
                }
            }
            Block::Geometry(in_pin) => {
                if key == b"END" {
                    block = if *in_pin { Block::Pin } else { Block::Macro };
                }
            }
        }
    }
    index
}

#[cfg(test)]
mod tests {
    use super::*;

    const CELLS: &str = r#"VERSION 5.7 ;
  BUSBITCHARS "[]" ;
MACRO inv_1
  CLASS CORE ;
  FOREIGN inv_1 ;
  ORIGIN 0.000 0.000 ;
  SIZE 1.380 BY 2.720 ;
  SYMMETRY X Y R90 ;
  SITE unithd ;
  PIN A
    DIRECTION INPUT ;
    USE SIGNAL ;
    PORT
      LAYER li1 ;
        RECT 0.320 1.075 0.650 1.315 ;
    END
  END A
  PIN VGND
    DIRECTION INOUT ;
    USE GROUND ;
    SHAPE ABUTMENT ;
    PORT
      LAYER met1 ;
        RECT 0.000 -0.240 1.380 0.240 ;
    END
  END VGND
  OBS
      LAYER li1 ;
        RECT 0.0 0.0 1.0 1.0 ;
  END
END inv_1
MACRO fill_1
  CLASS CORE SPACER ;
  SIZE 0.46 BY 2.72 ;
END fill_1
END LIBRARY
"#;

    const TECH: &str = r#"# a comment
VERSION 5.7 ;
UNITS
  DATABASE MICRONS 1000 ;
END UNITS
MANUFACTURINGGRID 0.005 ;
PROPERTYDEFINITIONS
  LAYER contactLimit INTEGER ;
END PROPERTYDEFINITIONS
SITE CoreSite
  CLASS CORE ;
  SYMMETRY Y ;
  SIZE 0.48 BY 3.78 ;
END CoreSite
LAYER Cont
  TYPE CUT ;
  WIDTH 0.16 ;
END Cont
LAYER Metal1
  TYPE ROUTING ;
  DIRECTION HORIZONTAL ;
  PITCH 0.42 ;
  WIDTH 0.16 ;
END Metal1
VIA Via1_XY DEFAULT
  LAYER Metal1 ;
    RECT -0.1 -0.1 0.1 0.1 ;
END Via1_XY
END LIBRARY
"#;

    #[test]
    fn macros_with_pins() {
        let index = parse(CELLS.as_bytes());
        assert_eq!(index.version, "5.7");
        assert!(!index.is_technology());
        let names: Vec<_> = index.macros.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["inv_1", "fill_1"]);
        let inv = &index.macros[0];
        assert_eq!(inv.class, "CORE");
        assert_eq!(inv.size, Some((1.38, 2.72)));
        assert_eq!(inv.site, "unithd");
        assert_eq!(inv.symmetry, "X Y R90");
        assert_eq!(
            inv.pins,
            [
                LefPin {
                    name: "A".into(),
                    direction: "INPUT".into(),
                    usage: "SIGNAL".into()
                },
                LefPin {
                    name: "VGND".into(),
                    direction: "INOUT".into(),
                    usage: "GROUND".into()
                },
            ]
        );
        assert_eq!(index.macros[1].class, "CORE SPACER");
    }

    #[test]
    fn technology_layers_sites_units() {
        let index = parse(TECH.as_bytes());
        assert!(index.is_technology());
        assert_eq!(index.database_microns, Some(1000.0));
        assert_eq!(index.manufacturing_grid, Some(0.005));
        assert_eq!(index.sites.len(), 1);
        assert_eq!(index.sites[0].size, Some((0.48, 3.78)));
        assert_eq!(index.sites[0].class, "CORE");
        let layers: Vec<_> = index
            .layers
            .iter()
            .map(|l| (l.name.as_str(), l.kind.as_str()))
            .collect();
        assert_eq!(layers, [("Cont", "CUT"), ("Metal1", "ROUTING")]);
        assert_eq!(index.layers[1].direction, "HORIZONTAL");
        assert_eq!(index.layers[1].pitch, Some(0.42));
        assert_eq!(index.vias, ["Via1_XY"]);
    }
}
