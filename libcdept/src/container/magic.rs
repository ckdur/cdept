//! Magic (`.mag`): one cell per file. The technology it was drawn in, the cells it
//! uses, its ports and what its properties say (LEF class, fixed bounding box, the
//! GDS file an abstract view stands for).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// What a `.mag` file holds.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MagicIndex {
    /// The `tech` line.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub tech: String,
    /// `magscale a b`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub magscale: String,
    /// Cells placed with `use`, each once, sorted.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub uses: Vec<String>,
    /// Port labels (`port` lines follow them), in order of their number.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ports: Vec<String>,
    /// `string NAME value` properties.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub properties: BTreeMap<String, String>,
}

/// Read a `.mag` file's text. Never fails.
pub fn parse(bytes: &[u8]) -> MagicIndex {
    let mut index = MagicIndex::default();
    let mut uses = std::collections::BTreeSet::new();
    let mut ports: Vec<(u32, String)> = Vec::new();
    let mut label: Option<String> = None;
    let text = String::from_utf8_lossy(bytes);
    for line in text.lines() {
        let mut words = line.split_whitespace();
        match words.next() {
            Some("tech") => index.tech = words.next().unwrap_or_default().to_string(),
            Some("magscale") => index.magscale = words.collect::<Vec<_>>().join(" "),
            Some("use") => {
                if let Some(cell) = words.next() {
                    uses.insert(cell.to_string());
                }
            }
            Some("rlabel" | "flabel") => label = words.next_back().map(str::to_string),
            Some("port") => {
                if let (Some(name), Some(number)) = (label.take(), words.next()) {
                    let number = number.parse().unwrap_or(u32::MAX);
                    if !ports.iter().any(|(_, n)| *n == name) {
                        ports.push((number, name));
                    }
                }
            }
            Some("string") => {
                if let Some(name) = words.next() {
                    index
                        .properties
                        .insert(name.to_string(), words.collect::<Vec<_>>().join(" "));
                }
            }
            _ => {}
        }
    }
    ports.sort_by_key(|(number, _)| *number);
    index.uses = uses.into_iter().collect();
    index.ports = ports.into_iter().map(|(_, name)| name).collect();
    index
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tech_uses_ports_properties() {
        let text = "magic\ntech sky130A\nmagscale 1 2\ntimestamp 1\n<< metal1 >>\nrect 0 0 1 1\n\
            << labels >>\nrlabel metal1 0 0 1 1 0 Y\nport 2 nsew signal output\n\
            flabel metal1 s 0 0 1 1 0 FreeSans 200 0 0 0 A\nport 1 nsew signal input\n\
            use via_1 via_1_0\ntransform 1 0 0 0 1 0\nuse via_1 via_1_1\nuse nfet nfet_0\n\
            << properties >>\nstring LEFclass CORE\nstring FIXED_BBOX 0 0 276 544\n<< end >>\n";
        let index = parse(text.as_bytes());
        assert_eq!(index.tech, "sky130A");
        assert_eq!(index.magscale, "1 2");
        assert_eq!(index.uses, ["nfet", "via_1"]);
        assert_eq!(index.ports, ["A", "Y"]);
        assert_eq!(index.properties["LEFclass"], "CORE");
        assert_eq!(index.properties["FIXED_BBOX"], "0 0 276 544");
    }
}
