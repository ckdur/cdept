//! The device map: which layout a schematic symbol becomes (docs/LINKING.md §3).
//!
//! Our own TOML (`format = "cdept-devices"`, version 1):
//!
//! ```toml
//! format = "cdept-devices"
//! version = 1
//!
//! [pins]                 # optional: labels for ipin/opin/iopin
//! layer = "8/25"         # layer/datatype of the text
//! size = 0.1             # text size in µm
//!
//! [[device]]
//! symbol = "sg13g2_pr/sg13_lv_nmos.sym"   # as the schematic refers to it
//! library = "SG13_dev"                    # a KLayout library ...
//! cell = "nmos"                           # ... and its PCell
//! params = { w = "@{w|u}", l = "@{l|u}", ng = "@ng", m = "@m" }
//! terminals = { D = "D", G = "G", S = "S", B = "B" }   # optional, see below
//! ```
//!
//! `terminals` (optional) names, for each pin of the symbol, the terminal of the device
//! KLayout's extraction makes of the PCell (used by the net comparison, docs/LINKING.md
//! §8). Without it a pin goes to the terminal of the same name (any case), else the
//! pins go to the terminals in order.
//!
//! A parameter template replaces `@name` with the instance's attribute `name` (else
//! the symbol's default from `template=`), and `@{name|u}` with that value read as a
//! SPICE number and written in units of `u` (µ) with the suffix: `0.5e-6` → `0.5u`.
//! The units are `f p n u m k` (suffix written) and `num` (a plain number, no suffix).
//!
//! The PDK's map lives in `pdk/<PDK>/devices.toml` (installed under
//! `share/cdept/pdk/`); the project's `[klayout] devices` file comes on top: an entry
//! for the same symbol replaces the PDK's.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use eyre::{Context, Result, eyre};
use toml_edit::{DocumentMut, Item};

use crate::Project;

pub const FORMAT: &str = "cdept-devices";
pub const VERSION: i64 = 1;

/// One entry of a device map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    /// The symbol reference, e.g. `sg13g2_pr/sg13_lv_nmos.sym`; a name without folder
    /// (`sg13_lv_nmos.sym`) matches the file name in any folder.
    pub symbol: String,
    /// The KLayout library of the PCell (or fixed cell).
    pub library: String,
    pub cell: String,
    /// PCell parameter → template.
    pub params: BTreeMap<String, String>,
    /// Symbol pin → terminal of the extracted device (empty: by name, else in order).
    pub terminals: BTreeMap<String, String>,
}

impl Device {
    /// True when the schematic's symbol reference `reference` is this entry's symbol.
    pub fn matches(&self, reference: &str) -> bool {
        let reference = reference.trim_start_matches("./");
        if self.symbol.contains('/') {
            reference == self.symbol || reference.ends_with(&format!("/{}", self.symbol))
        } else {
            Path::new(reference).file_name().and_then(|n| n.to_str()) == Some(&self.symbol)
        }
    }
}

/// How pins become labels in the layout.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Pins {
    /// `layer/datatype` of the label text; `None`: pins are not drawn.
    pub layer: Option<String>,
    /// Text size in µm.
    pub size: Option<f64>,
}

/// The device maps that apply to a project, the project's last.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DeviceMap {
    pub devices: Vec<Device>,
    pub pins: Pins,
    /// The files read, in order.
    pub files: Vec<PathBuf>,
}

impl DeviceMap {
    /// The PDK's map (if there is one for the project's `$PDK`), then the project's.
    pub fn for_project(project: &Project) -> Result<DeviceMap> {
        let mut map = DeviceMap::default();
        if let Some(pdk) = project.pdk.var("PDK")
            && let Some(file) = pdk_map(pdk)
        {
            map.add_file(&file)?;
        }
        if let Some(file) = &project.klayout.devices {
            map.add_file(file)?;
        }
        Ok(map)
    }

    /// Add the entries of a file; they replace earlier entries for the same symbol.
    pub fn add_file(&mut self, file: &Path) -> Result<()> {
        let text = std::fs::read_to_string(file)
            .wrap_err_with(|| format!("cannot read {}", file.display()))?;
        self.add_text(&text)
            .wrap_err_with(|| format!("{} is not a device map", file.display()))?;
        self.files.push(file.to_path_buf());
        Ok(())
    }

    /// [`add_file`](Self::add_file) on text.
    pub fn add_text(&mut self, text: &str) -> Result<()> {
        let doc: DocumentMut = text.parse().map_err(|e| eyre!("{e}"))?;
        match doc.get("format").and_then(Item::as_str) {
            Some(FORMAT) => {}
            Some(other) => return Err(eyre!("format is '{other}', expected '{FORMAT}'")),
            None => return Err(eyre!("no format = \"{FORMAT}\"")),
        }
        match doc.get("version").and_then(Item::as_integer) {
            Some(v) if (1..=VERSION).contains(&v) => {}
            Some(v) => {
                return Err(eyre!(
                    "version {v} is not supported (this CDePT reads up to {VERSION})"
                ));
            }
            None => return Err(eyre!("no version")),
        }
        if let Some(pins) = doc.get("pins") {
            if let Some(layer) = pins.get("layer").and_then(Item::as_str) {
                self.pins.layer = Some(layer.to_string());
            }
            if let Some(size) = pins.get("size") {
                self.pins.size = size
                    .as_float()
                    .or_else(|| size.as_integer().map(|i| i as f64));
            }
        }
        let Some(list) = doc.get("device") else {
            return Ok(());
        };
        let tables = list
            .as_array_of_tables()
            .ok_or_else(|| eyre!("'device' must be a list of [[device]] tables"))?;
        for (n, table) in tables.iter().enumerate() {
            let text = |key: &str| -> Result<String> {
                table
                    .get(key)
                    .and_then(Item::as_str)
                    .map(str::to_string)
                    .ok_or_else(|| eyre!("device {}: '{key}' is missing or not a text", n + 1))
            };
            let mut params = BTreeMap::new();
            if let Some(item) = table.get("params") {
                let inline = item
                    .as_inline_table()
                    .map(|t| t.iter().map(|(k, v)| (k.to_string(), v.as_str())).collect());
                let plain = item
                    .as_table()
                    .map(|t| t.iter().map(|(k, v)| (k.to_string(), v.as_str())).collect());
                let pairs: Vec<(String, Option<&str>)> = inline
                    .or(plain)
                    .ok_or_else(|| eyre!("device {}: 'params' must be a table", n + 1))?;
                for (key, value) in pairs {
                    let value = value.ok_or_else(|| {
                        eyre!("device {}: parameter '{key}' must be a text", n + 1)
                    })?;
                    params.insert(key, value.to_string());
                }
            }
            let mut terminals = BTreeMap::new();
            if let Some(item) = table.get("terminals") {
                let inline = item
                    .as_inline_table()
                    .map(|t| t.iter().map(|(k, v)| (k.to_string(), v.as_str())).collect());
                let plain = item
                    .as_table()
                    .map(|t| t.iter().map(|(k, v)| (k.to_string(), v.as_str())).collect());
                let pairs: Vec<(String, Option<&str>)> = inline
                    .or(plain)
                    .ok_or_else(|| eyre!("device {}: 'terminals' must be a table", n + 1))?;
                for (key, value) in pairs {
                    let value = value.ok_or_else(|| {
                        eyre!("device {}: terminal of pin '{key}' must be a text", n + 1)
                    })?;
                    terminals.insert(key, value.to_string());
                }
            }
            let device = Device {
                symbol: text("symbol")?,
                library: text("library")?,
                cell: text("cell")?,
                params,
                terminals,
            };
            self.devices.retain(|d| d.symbol != device.symbol);
            self.devices.push(device);
        }
        Ok(())
    }

    /// The entry for a symbol reference: an exact (folder) match before a file-name
    /// match; among equals the later file wins.
    pub fn lookup(&self, reference: &str) -> Option<&Device> {
        let exact = self
            .devices
            .iter()
            .rev()
            .find(|d| d.symbol.contains('/') && d.matches(reference));
        exact.or_else(|| self.devices.iter().rev().find(|d| d.matches(reference)))
    }
}

/// The shipped device map of a PDK: `$CDEPT_PDK_DATA/<pdk>/devices.toml`,
/// `<prefix>/share/cdept/pdk/<pdk>/devices.toml` next to the running program, or
/// `pdk/<pdk>/devices.toml` of the development tree.
pub fn pdk_map(pdk: &str) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(dir) = std::env::var_os("CDEPT_PDK_DATA").filter(|v| !v.is_empty()) {
        dirs.push(PathBuf::from(dir));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(prefix) = exe.parent().and_then(Path::parent)
    {
        dirs.push(prefix.join("share/cdept/pdk"));
    }
    if let Some(tree) = Path::new(env!("CARGO_MANIFEST_DIR")).parent() {
        dirs.push(tree.join("pdk"));
    }
    dirs.into_iter()
        .map(|d| d.join(pdk).join("devices.toml"))
        .find(|f| f.is_file())
}

/// A SPICE number: `0.5e-6`, `2u`, `1.5meg`, `10k`, `3` (a trailing unit such as the
/// `F` of `1pF` is ignored). `None` for anything else (expressions, parameters).
pub fn spice_number(text: &str) -> Option<f64> {
    let text = text.trim();
    let lower = text.to_ascii_lowercase();
    let b = lower.as_bytes();
    let mut end = 0;
    if end < b.len() && (b[end] == b'+' || b[end] == b'-') {
        end += 1;
    }
    while end < b.len() && (b[end].is_ascii_digit() || b[end] == b'.') {
        end += 1;
    }
    // An exponent: `e` followed by an optional sign and digits.
    if end < b.len() && b[end] == b'e' {
        let mut e = end + 1;
        if e < b.len() && (b[e] == b'+' || b[e] == b'-') {
            e += 1;
        }
        if e < b.len() && b[e].is_ascii_digit() {
            while e < b.len() && b[e].is_ascii_digit() {
                e += 1;
            }
            end = e;
        }
    }
    let value: f64 = lower[..end].parse().ok()?;
    let rest = &lower[end..];
    let scale = if rest.starts_with("meg") {
        1e6
    } else if rest.starts_with("mil") {
        25.4e-6
    } else {
        match rest.chars().next() {
            None => 1.0,
            Some('t') => 1e12,
            Some('g') => 1e9,
            Some('k') => 1e3,
            Some('m') => 1e-3,
            Some('u') | Some('µ') => 1e-6,
            Some('n') => 1e-9,
            Some('p') => 1e-12,
            Some('f') => 1e-15,
            Some('a') => 1e-18,
            Some(c) if c.is_alphabetic() => 1.0, // a unit: `V`, `Ohm`
            Some(_) => return None,
        }
    };
    let rest_ok = rest.chars().all(|c| c.is_alphabetic() || c == 'µ');
    rest_ok.then_some(value * scale)
}

/// A number without noise: at most 9 decimals, no trailing zeros.
pub fn format_number(value: f64) -> String {
    let rounded = format!("{value:.9}");
    let trimmed = rounded.trim_end_matches('0').trim_end_matches('.');
    if trimmed == "-0" || trimmed.is_empty() {
        "0".into()
    } else {
        trimmed.to_string()
    }
}

/// Expand a parameter template (see the module documentation). `value` gives the
/// attribute value of a name. The error names what is wrong.
pub fn expand(template: &str, value: &dyn Fn(&str) -> Option<String>) -> Result<String, String> {
    let mut out = String::new();
    let mut rest = template;
    while let Some(at) = rest.find('@') {
        out.push_str(&rest[..at]);
        rest = &rest[at + 1..];
        let (name, unit, after) = if let Some(inner) = rest.strip_prefix('{') {
            let close = inner
                .find('}')
                .ok_or_else(|| format!("'{template}': '@{{' without '}}'"))?;
            let body = &inner[..close];
            let (name, unit) = match body.split_once('|') {
                Some((n, u)) => (n.trim(), Some(u.trim())),
                None => (body.trim(), None),
            };
            (name, unit, &inner[close + 1..])
        } else {
            let end = rest
                .find(|c: char| !(c.is_alphanumeric() || c == '_'))
                .unwrap_or(rest.len());
            (&rest[..end], None, &rest[end..])
        };
        if name.is_empty() {
            out.push('@');
            rest = after;
            continue;
        }
        let raw = value(name).ok_or_else(|| format!("no value for '{name}'"))?;
        match unit {
            None => out.push_str(&raw),
            Some(unit) => {
                let number =
                    spice_number(&raw).ok_or_else(|| format!("{name}={raw} is not a number"))?;
                let (scale, suffix) = match unit {
                    "f" => (1e-15, "f"),
                    "p" => (1e-12, "p"),
                    "n" => (1e-9, "n"),
                    "u" => (1e-6, "u"),
                    "m" => (1e-3, "m"),
                    "k" => (1e3, "k"),
                    "num" => (1.0, ""),
                    other => return Err(format!("unknown unit '{other}' in '{template}'")),
                };
                out.push_str(&format_number(number / scale));
                out.push_str(suffix);
            }
        }
        rest = after;
    }
    out.push_str(rest);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spice_numbers() {
        assert_eq!(spice_number("0.5e-6"), Some(0.5e-6));
        assert_eq!(spice_number("2u"), Some(2e-6));
        assert_eq!(spice_number("1.5meg"), Some(1.5e6));
        assert_eq!(spice_number("10k"), Some(1e4));
        assert_eq!(spice_number("3"), Some(3.0));
        assert_eq!(spice_number("1pF"), Some(1e-12));
        assert_eq!(spice_number("-1e+3"), Some(-1e3));
        assert_eq!(spice_number("{W}"), None);
        assert_eq!(spice_number("'2*w'"), None);
        assert_eq!(spice_number(""), None);
    }

    #[test]
    fn templates() {
        let attrs = BTreeMap::from([
            ("w".to_string(), "0.5e-6".to_string()),
            ("l".to_string(), "130n".to_string()),
            ("ng".to_string(), "2".to_string()),
            ("x".to_string(), "{W}".to_string()),
        ]);
        let get = |n: &str| attrs.get(n).cloned();
        assert_eq!(expand("@{w|u}", &get).unwrap(), "0.5u");
        assert_eq!(expand("@{l|u}", &get).unwrap(), "0.13u");
        assert_eq!(expand("@ng", &get).unwrap(), "2");
        assert_eq!(expand("n=@ng!", &get).unwrap(), "n=2!");
        assert_eq!(expand("@{w|num}", &get).unwrap(), "0.0000005");
        assert_eq!(expand("fixed", &get).unwrap(), "fixed");
        assert!(expand("@{x|u}", &get).unwrap_err().contains("not a number"));
        assert!(expand("@nope", &get).unwrap_err().contains("nope"));
        assert!(expand("@{w|q}", &get).unwrap_err().contains("unit"));
    }

    const MAP: &str = r#"
format = "cdept-devices"
version = 1
[pins]
layer = "8/25"
size = 0.1
[[device]]
symbol = "sg13g2_pr/sg13_lv_nmos.sym"
library = "SG13_dev"
cell = "nmos"
params = { w = "@{w|u}", l = "@{l|u}" }
[[device]]
symbol = "res.sym"
library = "Basic"
cell = "X"
"#;

    #[test]
    fn maps_are_read_and_merged() {
        let mut map = DeviceMap::default();
        map.add_text(MAP).unwrap();
        assert_eq!(map.pins.layer.as_deref(), Some("8/25"));
        assert_eq!(map.pins.size, Some(0.1));
        let nmos = map.lookup("sg13g2_pr/sg13_lv_nmos.sym").unwrap();
        assert_eq!(
            (nmos.library.as_str(), nmos.cell.as_str()),
            ("SG13_dev", "nmos")
        );
        assert_eq!(nmos.params["w"], "@{w|u}");
        assert!(map.lookup("/abs/sg13g2_pr/sg13_lv_nmos.sym").is_some());
        assert!(map.lookup("other/sg13_lv_nmos.sym").is_none());
        assert!(map.lookup("devices/res.sym").is_some());
        // A later file replaces the entry of the same symbol.
        map.add_text(
            "format = \"cdept-devices\"\nversion = 1\n[[device]]\nsymbol = \"res.sym\"\nlibrary = \"L\"\ncell = \"r\"\n",
        )
        .unwrap();
        assert_eq!(map.lookup("res.sym").unwrap().library, "L");
        assert_eq!(map.devices.len(), 2);
    }

    #[test]
    fn bad_maps_say_why() {
        let mut map = DeviceMap::default();
        assert!(
            map.add_text("version = 1")
                .unwrap_err()
                .to_string()
                .contains("format")
        );
        let e = map
            .add_text("format = \"cdept-devices\"\nversion = 9")
            .unwrap_err();
        assert!(e.to_string().contains("version 9"));
        let e = map
            .add_text("format = \"cdept-devices\"\nversion = 1\n[[device]]\nsymbol = \"a\"")
            .unwrap_err();
        assert!(e.to_string().contains("library"), "{e}");
    }

    #[test]
    fn the_ihp_map_reads() {
        let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("../pdk/ihp-sg13g2/devices.toml");
        let mut map = DeviceMap::default();
        map.add_file(&file).unwrap();
        assert!(map.lookup("sg13g2_pr/sg13_lv_nmos.sym").is_some());
        assert!(map.lookup("sg13g2_pr/sg13_lv_pmos.sym").is_some());
    }
}
