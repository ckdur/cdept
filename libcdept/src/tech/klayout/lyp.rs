//! KLayout's layer properties file (`.lyp`).

use serde::{Deserialize, Serialize};

use super::Layer;
use super::xml::{self, Node};

/// A stipple the file defines itself (`<custom-dither-pattern>`); layers name it
/// `C<order>`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DitherPattern {
    pub order: u32,
    pub name: String,
    /// The rows from top to bottom, `*` set and `.` clear.
    pub lines: Vec<String>,
}

/// A line style the file defines itself (`<custom-line-style>`); layers name it
/// `C<order>`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LineStyle {
    pub order: u32,
    pub name: String,
    /// `*` drawn and `.` not, repeated along the line.
    pub pattern: String,
}

/// What a `.lyp` file says.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Lyp {
    /// The layers in the order of the file, groups flattened ([`Layer::group`]).
    pub layers: Vec<Layer>,
    pub dither_patterns: Vec<DitherPattern>,
    pub line_styles: Vec<LineStyle>,
}

pub fn parse(text: &str) -> Result<Lyp, String> {
    let root = xml::parse(text)?;
    if root.name != "layer-properties" {
        return Err(format!(
            "the root element is <{}>, not <layer-properties>",
            root.name
        ));
    }
    let mut lyp = Lyp::default();
    let mut group = Vec::new();
    for node in root.all("properties") {
        layer(node, &mut group, &mut lyp.layers);
    }
    for node in root.all("custom-dither-pattern") {
        lyp.dither_patterns.push(DitherPattern {
            order: node.number("order").unwrap_or_default(),
            name: node.text_of("name").unwrap_or_default().to_string(),
            lines: node
                .child("pattern")
                .map(|p| p.all("line").map(|l| l.text.trim().to_string()).collect())
                .unwrap_or_default(),
        });
    }
    for node in root.all("custom-line-style") {
        lyp.line_styles.push(LineStyle {
            order: node.number("order").unwrap_or_default(),
            name: node.text_of("name").unwrap_or_default().to_string(),
            pattern: node.text_of("pattern").unwrap_or_default().to_string(),
        });
    }
    Ok(lyp)
}

fn layer(node: &Node, group: &mut Vec<String>, out: &mut Vec<Layer>) {
    let name = node.text_of("name").unwrap_or_default().to_string();
    let members: Vec<&Node> = node.all("group-members").collect();
    if !members.is_empty() {
        // A group: its members carry its name.
        group.push(name);
        for member in members {
            layer(member, group, out);
        }
        group.pop();
        return;
    }
    let source = node.text_of("source").unwrap_or_default().to_string();
    let (source_name, numbers) = split_source(&source);
    let name = match (name.is_empty(), source_name) {
        (false, _) => name,
        (true, Some(n)) => n.to_string(),
        (true, None) => match numbers {
            Some((l, d)) => format!("{l}/{d}"),
            None => source.clone(),
        },
    };
    let (base, purpose) = split_name(&name);
    let colour = |key: &str| node.text_of(key).unwrap_or_default().to_string();
    out.push(Layer {
        base,
        purpose,
        name,
        layer: numbers.map_or(-1, |n| n.0),
        datatype: numbers.map_or(-1, |n| n.1),
        source,
        frame_color: colour("frame-color"),
        fill_color: colour("fill-color"),
        frame_brightness: node.number("frame-brightness").unwrap_or_default(),
        fill_brightness: node.number("fill-brightness").unwrap_or_default(),
        dither_pattern: colour("dither-pattern"),
        line_style: colour("line-style"),
        valid: node.flag("valid").unwrap_or(true),
        visible: node.flag("visible").unwrap_or(true),
        transparent: node.flag("transparent").unwrap_or(false),
        width: node.number("width").unwrap_or(1),
        marked: node.flag("marked").unwrap_or(false),
        xfill: node.flag("xfill").unwrap_or(false),
        animation: node.number("animation").unwrap_or_default(),
        group: group.clone(),
    });
}

/// A layer source: `8/0`, `8/0@1`, `Metal1 8/0`, `Metal1 (8/0)`, `*/*@*`. The name in
/// it, when it has one, and layer/datatype, when they are numbers.
fn split_source(source: &str) -> (Option<&str>, Option<(i32, i32)>) {
    let source = source.trim();
    let before_index = source.split('@').next().unwrap_or(source).trim();
    let (name, numbers) = match before_index.rsplit_once(char::is_whitespace) {
        Some((name, numbers)) => (Some(name.trim()), numbers),
        None => (None, before_index),
    };
    let numbers = numbers.trim_matches(|c| c == '(' || c == ')');
    let parsed = numbers
        .split_once('/')
        .and_then(|(l, d)| Some((l.trim().parse::<i32>().ok()?, d.trim().parse::<i32>().ok()?)));
    match (name, parsed) {
        (name, Some(numbers)) => (name.filter(|n| !n.is_empty()), Some(numbers)),
        // No numbers: the whole text may be a name ("Metal1").
        (_, None) if !before_index.contains('/') && !before_index.is_empty() => {
            (Some(before_index), None)
        }
        _ => (None, None),
    }
}

/// `Metal1.drawing` → (`Metal1`, `drawing`); `met1.pin - 68/16` → (`met1`, `pin`). The
/// purpose is empty when the name has none.
fn split_name(name: &str) -> (String, String) {
    // sky130 writes "name.purpose - L/D".
    let plain = match name.rsplit_once(" - ") {
        Some((plain, numbers)) if numbers.contains('/') => plain,
        _ => name,
    }
    .trim();
    match plain.rsplit_once('.') {
        Some((base, purpose)) if !base.is_empty() && !purpose.is_empty() => {
            (base.to_string(), purpose.to_string())
        }
        _ => (plain.to_string(), String::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LYP: &str = r#"<?xml version='1.0' encoding='UTF-8'?>
<layer-properties>
  <properties>
    <frame-color>#39bfff</frame-color>
    <fill-color>#39bfff</fill-color>
    <frame-brightness>0</frame-brightness>
    <fill-brightness>-8</fill-brightness>
    <dither-pattern>C1</dither-pattern>
    <line-style>C0</line-style>
    <valid>true</valid>
    <visible>false</visible>
    <transparent>false</transparent>
    <width>2</width>
    <marked>false</marked>
    <animation>0</animation>
    <name>Metal1.drawing</name>
    <source>8/0</source>
  </properties>
  <properties>
    <name>Pins</name>
    <group-members>
      <frame-color>#ff0000</frame-color>
      <name>met1.pin - 68/16</name>
      <source>68/16@1</source>
    </group-members>
    <group-members>
      <name/>
      <source>Other 9/1@1</source>
    </group-members>
  </properties>
  <properties>
    <name/>
    <source>*/*@*</source>
  </properties>
  <custom-dither-pattern>
    <pattern>
      <line>*.</line>
      <line>.*</line>
    </pattern>
    <order>1</order>
    <name>dots</name>
  </custom-dither-pattern>
  <custom-line-style>
    <pattern>***..</pattern>
    <order>0</order>
    <name>dashed</name>
  </custom-line-style>
</layer-properties>"#;

    #[test]
    fn layers_groups_and_patterns() {
        let lyp = parse(LYP).unwrap();
        assert_eq!(lyp.layers.len(), 4);
        let m1 = &lyp.layers[0];
        assert_eq!(
            (m1.name.as_str(), m1.layer, m1.datatype),
            ("Metal1.drawing", 8, 0)
        );
        assert_eq!(
            (m1.base.as_str(), m1.purpose.as_str()),
            ("Metal1", "drawing")
        );
        assert_eq!(m1.frame_color, "#39bfff");
        assert_eq!(m1.fill_brightness, -8);
        assert_eq!(m1.dither_pattern, "C1");
        assert!(!m1.visible);
        assert!(m1.valid);
        assert_eq!(m1.width, 2);
        assert!(m1.group.is_empty());

        let pin = &lyp.layers[1];
        assert_eq!((pin.layer, pin.datatype), (68, 16));
        assert_eq!((pin.base.as_str(), pin.purpose.as_str()), ("met1", "pin"));
        assert_eq!(pin.group, ["Pins"]);
        assert_eq!(lyp.layers[2].name, "Other");
        assert_eq!((lyp.layers[2].layer, lyp.layers[2].datatype), (9, 1));
        // The wildcard entry is kept as it is, without numbers.
        assert_eq!((lyp.layers[3].layer, lyp.layers[3].datatype), (-1, -1));
        assert_eq!(lyp.layers[3].source, "*/*@*");

        assert_eq!(lyp.dither_patterns[0].lines, ["*.", ".*"]);
        assert_eq!(lyp.dither_patterns[0].order, 1);
        assert_eq!(lyp.line_styles[0].pattern, "***..");
        assert_eq!(lyp.line_styles[0].name, "dashed");
    }

    #[test]
    fn sources() {
        assert_eq!(split_source("8/0"), (None, Some((8, 0))));
        assert_eq!(split_source("8/0@1"), (None, Some((8, 0))));
        assert_eq!(split_source("Metal1 8/0@1"), (Some("Metal1"), Some((8, 0))));
        assert_eq!(split_source("Metal1 (8/0)"), (Some("Metal1"), Some((8, 0))));
        assert_eq!(split_source("*/*@*"), (None, None));
        assert_eq!(split_source("Metal1"), (Some("Metal1"), None));
    }
}
