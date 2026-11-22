//! KLayout's technology file (`.lyt`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::xml::{self, Node};

/// What a `.lyt` file says.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Lyt {
    pub name: String,
    pub description: String,
    pub group: String,
    pub dbu: Option<f64>,
    /// `<base-path>`: empty means the folder of the file.
    pub base_path: String,
    pub original_base_path: String,
    pub layer_properties_file: String,
    pub add_other_layers: Option<bool>,
    pub grids: Vec<f64>,
    pub default_grid: Option<f64>,
    /// LEF files the LEF/DEF reader loads along (`reader-options/lefdef/lef-files`).
    pub lef_files: Vec<String>,
    /// `format.option` → value, the options that say something (`layer_map()` and
    /// empty values left out).
    pub reader_options: BTreeMap<String, String>,
    pub writer_options: BTreeMap<String, String>,
    pub connections: Vec<Connection>,
    pub symbols: Vec<LayerSymbol>,
}

/// A line of the connectivity: two conductors and what joins them. Each is a layer
/// (`8/0`), a symbol or an expression of them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Connection {
    pub lower: String,
    /// Empty when the two conductors touch directly.
    pub via: String,
    pub upper: String,
}

impl Connection {
    /// As KLayout writes it: `Metal1,Via1,Metal2`.
    pub fn text(&self) -> String {
        if self.via.is_empty() {
            format!("{},{}", self.lower, self.upper)
        } else {
            format!("{},{},{}", self.lower, self.via, self.upper)
        }
    }
}

/// A name of the connectivity for a layer expression: `Metal1 = 8/0-8/29`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LayerSymbol {
    pub name: String,
    pub expression: String,
}

pub fn parse(text: &str) -> Result<Lyt, String> {
    let root = xml::parse(text)?;
    if root.name != "technology" {
        return Err(format!(
            "the root element is <{}>, not <technology>",
            root.name
        ));
    }
    let word = |name: &str| root.text_of(name).unwrap_or_default().to_string();
    let mut lyt = Lyt {
        name: word("name"),
        description: word("description"),
        group: word("group"),
        dbu: root.number("dbu"),
        base_path: word("base-path"),
        original_base_path: word("original-base-path"),
        layer_properties_file: word("layer-properties_file"),
        add_other_layers: root.flag("add-other-layers"),
        ..Lyt::default()
    };
    // "10.0,5.0,1.0,0.005!": the one with "!" is the default.
    for grid in word("default-grids").split(',') {
        let grid = grid.trim();
        let (number, default) = match grid.strip_suffix('!') {
            Some(number) => (number, true),
            None => (grid, false),
        };
        if let Ok(value) = number.trim().parse::<f64>() {
            lyt.grids.push(value);
            if default {
                lyt.default_grid = Some(value);
            }
        }
    }
    if let Some(options) = root.child("reader-options") {
        if let Some(lefdef) = options.child("lefdef") {
            lyt.lef_files = lefdef
                .all("lef-files")
                .map(|n| n.text.trim().to_string())
                .filter(|f| !f.is_empty())
                .collect();
        }
        flatten(options, "", &mut lyt.reader_options);
    }
    if let Some(options) = root.child("writer-options") {
        flatten(options, "", &mut lyt.writer_options);
    }
    if let Some(connectivity) = root.child("connectivity") {
        for connection in connectivity.all("connection") {
            let parts: Vec<&str> = connection.text.split(',').map(str::trim).collect();
            let connection = match parts.as_slice() {
                [lower, via, upper] => Connection {
                    lower: lower.to_string(),
                    via: via.to_string(),
                    upper: upper.to_string(),
                },
                [lower, upper] => Connection {
                    lower: lower.to_string(),
                    via: String::new(),
                    upper: upper.to_string(),
                },
                _ => continue,
            };
            lyt.connections.push(connection);
        }
        for symbol in connectivity.all("symbols") {
            // capm='89/44'
            let Some((name, expression)) = symbol.text.split_once('=') else {
                continue;
            };
            lyt.symbols.push(LayerSymbol {
                name: name.trim().to_string(),
                expression: expression.trim().trim_matches('\'').to_string(),
            });
        }
    }
    Ok(lyt)
}

/// `a/b/c` → `a.b.c = text`, leaves only; empty values and `layer_map()` say nothing.
fn flatten(node: &Node, prefix: &str, out: &mut BTreeMap<String, String>) {
    for child in &node.children {
        let key = if prefix.is_empty() {
            child.name.clone()
        } else {
            format!("{prefix}.{}", child.name)
        };
        if child.children.is_empty() {
            let value = child.text.trim();
            if !value.is_empty() && value != "layer_map()" {
                // A repeated element (lef-files) keeps every value.
                match out.get_mut(&key) {
                    Some(old) => {
                        old.push(' ');
                        old.push_str(value);
                    }
                    None => {
                        out.insert(key, value.to_string());
                    }
                }
            }
        } else {
            flatten(child, &key, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LYT: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<technology>
 <name>sg13g2</name>
 <description>IHP SiGe 130nm technology</description>
 <group/>
 <dbu>0.001</dbu>
 <base-path></base-path>
 <layer-properties_file>sg13g2.lyp</layer-properties_file>
 <add-other-layers>true</add-other-layers>
 <default-grids>10.0,5.0,0.01,0.005!</default-grids>
 <reader-options>
  <gds2><box-mode>1</box-mode></gds2>
  <common><layer-map>layer_map()</layer-map><enable-properties>true</enable-properties></common>
  <lefdef>
   <pins-suffix-string>.PIN</pins-suffix-string>
   <lef-files>a.lef</lef-files>
   <lef-files>b.lef</lef-files>
  </lefdef>
 </reader-options>
 <writer-options><gds2><libname>LIB</libname></gds2></writer-options>
 <connectivity>
  <connection>GatPoly,Cont,Metal1</connection>
  <connection>Metal1,Via1,Metal2</connection>
  <connection>a,b</connection>
  <symbols>Metal1='8/0-8/29'</symbols>
  <symbols>Via1='19/0'</symbols>
 </connectivity>
</technology>"#;

    #[test]
    fn a_technology_file() {
        let lyt = parse(LYT).unwrap();
        assert_eq!(lyt.name, "sg13g2");
        assert_eq!(lyt.description, "IHP SiGe 130nm technology");
        assert_eq!(lyt.dbu, Some(0.001));
        assert_eq!(lyt.base_path, "");
        assert_eq!(lyt.layer_properties_file, "sg13g2.lyp");
        assert_eq!(lyt.add_other_layers, Some(true));
        assert_eq!(lyt.grids, [10.0, 5.0, 0.01, 0.005]);
        assert_eq!(lyt.default_grid, Some(0.005));
        assert_eq!(lyt.lef_files, ["a.lef", "b.lef"]);
        assert_eq!(lyt.reader_options["gds2.box-mode"], "1");
        assert_eq!(lyt.reader_options["lefdef.pins-suffix-string"], ".PIN");
        assert_eq!(lyt.reader_options["lefdef.lef-files"], "a.lef b.lef");
        assert!(!lyt.reader_options.contains_key("common.layer-map"));
        assert_eq!(lyt.writer_options["gds2.libname"], "LIB");
        assert_eq!(lyt.connections.len(), 3);
        assert_eq!(lyt.connections[0].text(), "GatPoly,Cont,Metal1");
        assert_eq!(lyt.connections[2].via, "");
        assert_eq!(lyt.connections[2].text(), "a,b");
        assert_eq!(
            lyt.symbols[0],
            LayerSymbol {
                name: "Metal1".into(),
                expression: "8/0-8/29".into()
            }
        );
    }

    #[test]
    fn other_documents_are_refused() {
        assert!(parse("<layer-properties/>").is_err());
        assert!(parse("not xml").is_err());
    }
}
