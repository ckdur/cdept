//! Write the form's text into the file it came from without losing what the form does
//! not own: the comments and the layout of every key that did not change.
//!
//! The form makes a canonical text (`toml::to_string_pretty`, no comments). Saving
//! compares it, key by key at the top level, with the file as it is; a key whose value
//! is the same (numbers by value, a number written as text too) keeps its lines, its
//! comments and its place; a changed one gets the new value (keeping the comments
//! around it where it is a plain value or a table), a new one is added, a removed one is
//! dropped.

use toml::Value;
use toml_edit::{DocumentMut, Item};

/// The same value, whatever the spelling: `27` and `27.0`, `1.2` and `"1.2"`.
fn same(a: &Value, b: &Value) -> bool {
    let number = |v: &Value| match v {
        Value::Integer(i) => Some(*i as f64),
        Value::Float(f) => Some(*f),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    };
    match (a, b) {
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| same(p, q))
        }
        (Value::Table(x), Value::Table(y)) => {
            x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| same(v, w)))
        }
        (Value::String(x), Value::String(y)) => x == y,
        _ => match (number(a), number(b)) {
            (Some(x), Some(y)) => x == y,
            _ => a == b,
        },
    }
}

/// `original` (the file) with the keys of `new_text` (the form's text) that differ.
/// The text of `new_text` itself when `original` is not usable.
pub(super) fn merge(original: &str, new_text: &str) -> String {
    let (Ok(mut doc), Ok(old), Ok(new), Ok(new_doc)) = (
        original.parse::<DocumentMut>(),
        toml::from_str::<toml::Table>(original),
        toml::from_str::<toml::Table>(new_text),
        new_text.parse::<DocumentMut>(),
    ) else {
        return new_text.to_string();
    };
    let gone: Vec<String> = old
        .keys()
        .filter(|k| !new.contains_key(*k))
        .cloned()
        .collect();
    for k in gone {
        doc.remove(&k);
    }
    for (k, v) in &new {
        if old.get(k).is_some_and(|o| same(o, v)) {
            continue;
        }
        let Some(mut item) = new_doc.get(k).cloned() else {
            continue;
        };
        match (doc.get_mut(k), &mut item) {
            // A plain value: the comments around it stay.
            (Some(Item::Value(old_value)), Item::Value(new_value)) => {
                *new_value.decor_mut() = old_value.decor().clone();
                *old_value = new_value.clone();
            }
            // A table or a list of tables: the comment above it stays.
            (Some(Item::Table(old_table)), Item::Table(new_table)) => {
                *new_table.decor_mut() = old_table.decor().clone();
                doc.insert(k, item);
            }
            (Some(Item::ArrayOfTables(old_array)), Item::ArrayOfTables(new_array)) => {
                if let (Some(first_old), Some(first_new)) =
                    (old_array.iter().next(), new_array.iter_mut().next())
                {
                    *first_new.decor_mut() = first_old.decor().clone();
                }
                doc.insert(k, item);
            }
            _ => {
                doc.insert(k, item);
            }
        }
    }
    doc.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "# Testbench of the inverter\n\n# which corners\ncorners = [\"tt\"] # typical only\ntemperatures = [27]\n\n# the supply\n[variables]\nvdd = \"1.2\"\n\n# delay\n[[spec]]\nname = \"tpd\"\nmax = 3e-10\n";

    #[test]
    fn unchanged_keys_keep_their_comments() {
        // the form writes the same content in its own spelling
        let form = "corners = [\"tt\"]\ntemperatures = [27.0]\n\n[variables]\nvdd = \"1.2\"\n\n[[spec]]\nname = \"tpd\"\nmax = 3e-10\n";
        assert_eq!(merge(FILE, form), FILE);
    }

    #[test]
    fn changes_keep_the_rest_and_the_comments_around() {
        let form = "corners = [\"tt\", \"ff\"]\ntemperatures = [27.0]\n\n[variables]\nvdd = \"1.2\"\n\n[[spec]]\nname = \"tpd\"\nmax = 4e-10\n\n[[sweep]]\nvariable = \"vdd\"\nvalues = [\"1.08\", \"1.2\"]\n\n[montecarlo]\nruns = 5\nseed = 1\n";
        let out = merge(FILE, form);
        for keep in [
            "# Testbench of the inverter",
            "# which corners",
            "# typical only",
            "# the supply",
            "# delay",
        ] {
            assert!(out.contains(keep), "{keep} lost:\n{out}");
        }
        let setup: toml::Table = toml::from_str(&out).unwrap();
        assert_eq!(setup["corners"].as_array().unwrap().len(), 2);
        assert_eq!(setup["spec"][0]["max"].as_float(), Some(4e-10));
        assert_eq!(setup["sweep"][0]["variable"].as_str(), Some("vdd"));
        assert_eq!(setup["montecarlo"]["runs"].as_integer(), Some(5));
        // removing them again drops only them
        let back = merge(
            &out,
            "corners = [\"tt\", \"ff\"]\ntemperatures = [27.0]\n\n[variables]\nvdd = \"1.2\"\n\n[[spec]]\nname = \"tpd\"\nmax = 4e-10\n",
        );
        assert!(
            !back.contains("sweep") && !back.contains("montecarlo"),
            "{back}"
        );
        assert!(
            back.contains("# the supply") && back.contains("# which corners"),
            "{back}"
        );
    }

    #[test]
    fn a_new_plain_key_goes_above_the_tables() {
        let out = merge(
            FILE,
            "corners = [\"tt\"]\ntemperatures = [27]\nsimulator = \"xyce\"\n\n[variables]\nvdd = \"1.2\"\n\n[[spec]]\nname = \"tpd\"\nmax = 3e-10\n",
        );
        let t: toml::Table = toml::from_str(&out).unwrap();
        assert_eq!(t["simulator"].as_str(), Some("xyce"), "{out}");
        assert!(
            t["variables"]
                .as_table()
                .unwrap()
                .get("simulator")
                .is_none(),
            "{out}"
        );
    }
}
