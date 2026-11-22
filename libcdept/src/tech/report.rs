//! The snapshot in words: a summary for after a scan, and what changed between two
//! scans. Both work on the files' own shape (tables and lists), so a section that
//! gains fields is covered without a change here.

use serde::Serialize;
use serde_json::Value;

use super::{Catalog, Tech};

/// The sections of `tech.toml`, in file order.
pub const SECTIONS: [&str; 8] = [
    "pdk", "klayout", "magic", "netgen", "xschem", "models", "devices", "refs",
];

/// A section of the snapshot (or of the catalog) as a value tree; `None` for a name
/// that is no section.
pub fn section<T: Serialize>(of: &T, name: &str) -> Option<Value> {
    match serde_json::to_value(of).ok()? {
        Value::Object(mut map) => map.remove(name),
        _ => None,
    }
}

/// Is there nothing in it? (Empty tables and lists, no text, `null`.)
fn is_empty(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(text) => text.is_empty(),
        Value::Array(list) => list.is_empty(),
        Value::Object(map) => map.values().all(is_empty),
        Value::Bool(_) | Value::Number(_) => false,
    }
}

/// A value on one line, shortened: text as it is, a list by its length and first
/// names, a table by its keys.
fn brief(value: &Value) -> String {
    const MOST: usize = 4;
    let named = |item: &Value| match item {
        Value::String(text) => Some(text.clone()),
        Value::Object(map) => ["name", "file", "path"]
            .iter()
            .find_map(|key| map.get(*key).and_then(Value::as_str))
            .map(str::to_string),
        other => Some(other.to_string()),
    };
    match value {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::Array(list) => {
            let names: Vec<String> = list.iter().filter_map(named).collect();
            if names.len() == list.len() && !names.is_empty() {
                let mut shown = names[..names.len().min(MOST)].join(", ");
                if names.len() > MOST {
                    shown.push_str(&format!(", … ({} in all)", names.len()));
                }
                shown
            } else {
                format!("{} entries", list.len())
            }
        }
        Value::Object(map) => {
            let keys: Vec<&str> = map
                .iter()
                .filter(|(_, v)| !is_empty(v))
                .map(|(k, _)| k.as_str())
                .collect();
            let mut shown = keys[..keys.len().min(MOST)].join(", ");
            if keys.len() > MOST {
                shown.push_str(&format!(", … ({} in all)", keys.len()));
            }
            shown
        }
    }
}

/// How many things a catalog section lists, per list: `layers 205, pcell_libraries 2`.
/// Lists inside tables are counted under `table.list`; lists inside lists are summed
/// (`pcell_libraries.pcells 31`).
fn counts(value: &Value) -> Vec<(String, usize)> {
    fn walk(value: &Value, path: &str, out: &mut Vec<(String, usize)>) {
        match value {
            Value::Object(map) => {
                for (key, item) in map {
                    let path = if path.is_empty() {
                        key.clone()
                    } else {
                        format!("{path}.{key}")
                    };
                    walk(item, &path, out);
                }
            }
            Value::Array(list) if !path.is_empty() => {
                match out.iter_mut().find(|(p, _)| p == path) {
                    Some((_, n)) => *n += list.len(),
                    None => out.push((path.to_string(), list.len())),
                }
                // One level further: the lists of the list's entries, summed.
                for item in list {
                    if let Value::Object(map) = item {
                        for (key, inner) in map {
                            if let Value::Array(inner) = inner
                                && inner.iter().any(Value::is_object)
                            {
                                let path = format!("{path}.{key}");
                                match out.iter_mut().find(|(p, _)| *p == path) {
                                    Some((_, n)) => *n += inner.len(),
                                    None => out.push((path, inner.len())),
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(value, "", &mut out);
    out.retain(|(_, n)| *n > 0);
    out
}

/// One line of the summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    /// The section (`klayout`), repeated on each of its lines.
    pub section: String,
    /// The key (`tech`, `drc`), or `catalog` for the counts of the catalog.
    pub key: String,
    pub text: String,
}

/// What the scan found, section by section: every setting that is set, briefly, and
/// what the catalog lists. A section with nothing found gets one line saying so.
pub fn summary(tech: &Tech, catalog: &Catalog) -> Vec<Line> {
    let mut lines = Vec::new();
    for name in SECTIONS {
        let mut found = false;
        let mut line = |key: &str, text: String| {
            lines.push(Line {
                section: name.to_string(),
                key: key.to_string(),
                text,
            });
        };
        if let Some(Value::Object(map)) = section(tech, name) {
            for (key, value) in &map {
                if !is_empty(value) {
                    found = true;
                    line(key, brief(value));
                }
            }
        }
        if let Some(value) = section(catalog, name) {
            let counted = counts(&value);
            if !counted.is_empty() {
                found = true;
                let text: Vec<String> = counted
                    .iter()
                    .map(|(what, n)| format!("{n} {what}"))
                    .collect();
                line("catalog", text.join(", "));
            }
        }
        if !found {
            line("", "nothing found".into());
        }
    }
    lines
}

/// The most characters of a value in [`summary_text`].
const LINE: usize = 96;

/// `text` cut to `most` characters, with `…` when something was cut.
fn shorten(text: &str, most: usize) -> String {
    if text.chars().count() <= most {
        return text.to_string();
    }
    let mut out: String = text.chars().take(most.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// The summary as text, the sections side by side with their keys.
pub fn summary_text(tech: &Tech, catalog: &Catalog) -> String {
    let lines = summary(tech, catalog);
    let width = lines.iter().map(|l| l.key.len()).max().unwrap_or(0);
    let mut out = String::new();
    let mut last = "";
    for line in &lines {
        let section = if line.section == last {
            ""
        } else {
            line.section.as_str()
        };
        last = &line.section;
        out.push_str(&format!(
            "{section:<8} {:<width$}  {}\n",
            line.key,
            shorten(&line.text, LINE)
        ));
    }
    out
}

/// What differs between two value trees, as lines: `klayout.tech: sg13g2 → sg13g2_v2`,
/// `+ magic.rc: …`, `- netgen.setup`. Lists are compared as a whole.
fn differences(old: &Value, new: &Value, path: &str, out: &mut Vec<String>) {
    match (old, new) {
        (Value::Object(a), Value::Object(b)) => {
            let mut keys: Vec<&String> = a.keys().chain(b.keys()).collect();
            keys.sort();
            keys.dedup();
            for key in keys {
                let path = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                let none = Value::Null;
                let (x, y) = (a.get(key).unwrap_or(&none), b.get(key).unwrap_or(&none));
                // A table that appears or goes is told key by key.
                let table = |v: &Value| v.is_object() || v.is_null();
                if (x.is_object() || y.is_object()) && table(x) && table(y) {
                    let empty = Value::Object(Default::default());
                    let x = if x.is_null() { &empty } else { x };
                    let y = if y.is_null() { &empty } else { y };
                    differences(x, y, &path, out);
                    continue;
                }
                match (is_empty(x), is_empty(y)) {
                    (true, true) => {}
                    (true, false) => out.push(format!("+ {path}: {}", brief(y))),
                    (false, true) => out.push(format!("- {path}: {}", brief(x))),
                    (false, false) => differences(x, y, &path, out),
                }
            }
        }
        (Value::Array(a), Value::Array(b)) if a != b => {
            if a.len() != b.len() {
                out.push(format!("~ {path}: {} → {} entries", a.len(), b.len()));
            } else {
                let changed = a.iter().zip(b).filter(|(x, y)| x != y).count();
                out.push(format!(
                    "~ {path}: {changed} of {} entries changed",
                    a.len()
                ));
            }
        }
        (a, b) if a != b => out.push(format!("~ {path}: {} → {}", brief(a), brief(b))),
        _ => {}
    }
}

/// What a new scan changes: the settings of `tech.toml` that differ and the lists of
/// the catalog that differ. Empty when nothing does.
pub fn changes(old: (&Tech, &Catalog), new: (&Tech, &Catalog)) -> Vec<String> {
    let mut out = Vec::new();
    let value = |tech: &Tech| serde_json::to_value(tech).unwrap_or(Value::Null);
    differences(&value(old.0), &value(new.0), "", &mut out);
    let value = |catalog: &Catalog| serde_json::to_value(catalog).unwrap_or(Value::Null);
    let mut catalog = Vec::new();
    differences(&value(old.1), &value(new.1), "", &mut catalog);
    out.extend(catalog.into_iter().map(|line| {
        let (mark, rest) = line.split_at(2);
        format!("{mark}catalog {rest}")
    }));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tech::klayout::{Deck, Layer, Pcell, PcellLibrary};
    use crate::tech::{PdkDir, TechPath};

    fn sample() -> (Tech, Catalog) {
        let pdk = PdkDir::new("/p", "ihp-sg13g2");
        let mut tech = Tech::default();
        tech.pdk.name = pdk.name.clone();
        tech.klayout.tech = Some("sg13g2".into());
        tech.klayout.dbu = Some(0.001);
        for name in ["a", "b", "c", "d", "e"] {
            tech.klayout.drc.push(Deck {
                name: name.into(),
                file: TechPath(format!("$PDK_ROOT/x/{name}.drc")),
                ..Default::default()
            });
        }
        let mut catalog = Catalog::default();
        for n in 0..3 {
            catalog.klayout.layers.push(Layer {
                name: format!("M{n}"),
                layer: n,
                ..Default::default()
            });
        }
        for lib in ["dev", "basic"] {
            catalog.klayout.pcell_libraries.push(PcellLibrary {
                name: lib.into(),
                pcells: vec![Pcell::default(), Pcell::default()],
                ..Default::default()
            });
        }
        (tech, catalog)
    }

    #[test]
    fn the_summary_names_what_is_set_and_counts_the_catalog() {
        let (tech, catalog) = sample();
        let lines = summary(&tech, &catalog);
        let of = |section: &str, key: &str| {
            lines
                .iter()
                .find(|l| l.section == section && l.key == key)
                .map(|l| l.text.as_str())
        };
        assert_eq!(of("klayout", "tech"), Some("sg13g2"));
        assert_eq!(of("klayout", "dbu"), Some("0.001"));
        assert_eq!(of("klayout", "drc"), Some("a, b, c, d, … (5 in all)"));
        let counted = of("klayout", "catalog").unwrap();
        assert!(counted.contains("3 layers"), "{counted}");
        assert!(counted.contains("2 pcell_libraries"), "{counted}");
        assert!(counted.contains("4 pcell_libraries.pcells"), "{counted}");
        assert_eq!(of("netgen", ""), Some("nothing found"));
        assert_eq!(of("pdk", "name"), Some("ihp-sg13g2"));
        let text = summary_text(&tech, &catalog);
        assert!(text.contains("klayout "), "{text}");
        // Every section is there, in file order.
        let mut order: Vec<&str> = lines.iter().map(|l| l.section.as_str()).collect();
        order.dedup();
        assert_eq!(order, SECTIONS);
    }

    #[test]
    fn changes_between_two_scans() {
        let (old_tech, old_catalog) = sample();
        assert!(changes((&old_tech, &old_catalog), (&old_tech, &old_catalog)).is_empty());
        let (mut tech, mut catalog) = sample();
        tech.klayout.tech = Some("sg13g2_v2".into());
        tech.klayout.dbu = None;
        tech.magic.tech = Some("ihp".into());
        tech.klayout.drc.pop();
        catalog.klayout.layers[0].name = "Metal0".into();
        let lines = changes((&old_tech, &old_catalog), (&tech, &catalog));
        let has = |text: &str| lines.iter().any(|l| l == text);
        assert!(has("~ klayout.tech: sg13g2 → sg13g2_v2"), "{lines:?}");
        assert!(has("- klayout.dbu: 0.001"), "{lines:?}");
        assert!(has("+ magic.tech: ihp"), "{lines:?}");
        assert!(has("~ klayout.drc: 5 → 4 entries"), "{lines:?}");
        assert!(
            has("~ catalog klayout.layers: 1 of 3 entries changed"),
            "{lines:?}"
        );
        assert_eq!(lines.len(), 5, "{lines:?}");
    }

    #[test]
    fn sections_by_name() {
        let (tech, catalog) = sample();
        assert!(section(&tech, "klayout").unwrap()["tech"] == "sg13g2");
        assert!(section(&catalog, "klayout").unwrap()["layers"].is_array());
        assert!(section(&tech, "nope").is_none());
    }
}
