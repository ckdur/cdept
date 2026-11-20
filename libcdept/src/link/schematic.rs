//! Reading the instances of an xschem schematic (`.sch`) and the defaults of their
//! symbols (docs/LINKING.md §2).
//!
//! Only `C` records (component instances) are read: symbol reference, position and
//! attributes. Wires, texts and the embedded copies of symbols (`[ ... ]`) are skipped.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use eyre::{Context, Result};

use crate::Project;
use crate::xschem::symbol;

/// One instance of a schematic.
#[derive(Debug, Clone, PartialEq)]
pub struct Instance {
    /// The symbol as the schematic refers to it, e.g. `sg13g2_pr/sg13_lv_nmos.sym`.
    pub symbol: String,
    /// `name=`; empty when the instance has none.
    pub name: String,
    /// The instance's own attributes (`name` included), later items winning.
    pub attributes: BTreeMap<String, String>,
    pub x: f64,
    pub y: f64,
    pub rotation: u8,
    pub flip: u8,
}

/// One record of the file: its tag and its fields (groups without their braces and
/// escapes).
fn records(text: &str) -> Vec<(char, Vec<String>)> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        // Start of a line: skip blanks.
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        if i >= chars.len() {
            break;
        }
        let tag = chars[i];
        i += 1;
        if tag == '[' {
            // An embedded symbol, up to the `]` that starts a line.
            let mut depth = 1;
            let mut line_start = false;
            while i < chars.len() && depth > 0 {
                let c = chars[i];
                if line_start && c == '[' {
                    depth += 1;
                } else if line_start && c == ']' {
                    depth -= 1;
                }
                if c == '{' {
                    i = skip_group(&chars, i);
                    line_start = false;
                    continue;
                }
                line_start = c == '\n' || (line_start && c.is_whitespace());
                i += 1;
            }
            continue;
        }
        let mut fields = Vec::new();
        loop {
            while i < chars.len() && chars[i] != '\n' && chars[i].is_whitespace() {
                i += 1;
            }
            if i >= chars.len() || chars[i] == '\n' {
                break;
            }
            if chars[i] == '{' {
                let end = skip_group(&chars, i);
                let inner: String = chars[i + 1..end.saturating_sub(1).max(i + 1)]
                    .iter()
                    .collect();
                fields.push(symbol::unescape(&inner));
                i = end;
            } else {
                let start = i;
                while i < chars.len() && !chars[i].is_whitespace() {
                    i += 1;
                }
                fields.push(chars[start..i].iter().collect());
            }
        }
        out.push((tag, fields));
    }
    out
}

/// Index after the `}` closing the group that starts at `i` (a `{`).
fn skip_group(chars: &[char], mut i: usize) -> usize {
    i += 1;
    while i < chars.len() && chars[i] != '}' {
        i += if chars[i] == '\\' { 2 } else { 1 };
    }
    (i + 1).min(chars.len())
}

/// The attributes of an attribute string, later items winning.
pub fn attributes(text: &str) -> BTreeMap<String, String> {
    symbol::attribute_items(text).into_iter().collect()
}

/// The instances of a schematic, in file order.
pub fn parse(text: &str) -> Vec<Instance> {
    records(text)
        .into_iter()
        .filter(|(tag, fields)| *tag == 'C' && !fields.is_empty())
        .map(|(_, fields)| {
            let number = |n: usize| fields.get(n).and_then(|v| v.parse::<f64>().ok());
            let attributes = fields.get(5).map(|a| attributes(a)).unwrap_or_default();
            Instance {
                symbol: fields[0].clone(),
                name: attributes.get("name").cloned().unwrap_or_default(),
                attributes,
                x: number(1).unwrap_or(0.0),
                y: number(2).unwrap_or(0.0),
                rotation: number(3).unwrap_or(0.0) as u8,
                flip: number(4).unwrap_or(0.0) as u8,
            }
        })
        .collect()
}

/// Read the instances of a schematic file.
pub fn read(file: &Path) -> Result<Vec<Instance>> {
    let text = std::fs::read_to_string(file)
        .wrap_err_with(|| format!("cannot read {}", file.display()))?;
    Ok(parse(&text))
}

/// What a symbol says about its instances.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SymbolInfo {
    /// The file, when it was found.
    pub file: Option<PathBuf>,
    /// `type=` (`nmos`, `subcircuit`, `ipin`, `label`, …).
    pub kind: String,
    /// The symbol has a `format=` (it becomes a line of the netlist).
    pub has_format: bool,
    /// The defaults of new instances (`template=`).
    pub template: BTreeMap<String, String>,
}

impl SymbolInfo {
    /// Read the global attributes of a symbol file's text.
    pub fn parse(text: &str) -> SymbolInfo {
        let global = symbol::global_attributes(text).unwrap_or_default();
        let items = attributes(&global);
        SymbolInfo {
            file: None,
            kind: items.get("type").cloned().unwrap_or_default(),
            has_format: items.contains_key("format") || items.contains_key("lvs_format"),
            template: items
                .get("template")
                .map(|t| attributes(t))
                .unwrap_or_default(),
        }
    }
}

/// The folders symbol references are looked up in, as xschem does: the parents of the
/// project's libraries, the PDK's `libs.tech/xschem`, then xschem's own library
/// (`devices/` and the like) when it can be found.
pub fn search_path(project: &Project) -> Vec<PathBuf> {
    let mut out = crate::xschem::setup::search_path(project);
    if let (Some(root), Some(pdk)) = (project.pdk.var("PDK_ROOT"), project.pdk.var("PDK")) {
        let dir = Path::new(&root).join(pdk).join("libs.tech/xschem");
        if dir.is_dir() {
            out.push(dir);
        }
    }
    for dir in xschem_libraries() {
        if !out.contains(&dir) {
            out.push(dir);
        }
    }
    out
}

/// xschem's own symbol folders: the `xschem_library` of the development build or of an
/// installed xschem.
fn xschem_libraries() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(tree) = Path::new(env!("CARGO_MANIFEST_DIR")).parent() {
        roots.push(tree.join("cdept-xschem/xschem/xschem_library"));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(prefix) = exe.parent().and_then(Path::parent)
    {
        roots.push(prefix.join("share/xschem/xschem_library"));
    }
    roots.push(PathBuf::from("/usr/local/share/xschem/xschem_library"));
    roots.push(PathBuf::from("/usr/share/xschem/xschem_library"));
    let mut out = Vec::new();
    for root in roots.into_iter().filter(|r| r.is_dir()) {
        out.push(root.clone());
        if let Ok(entries) = std::fs::read_dir(&root) {
            let mut dirs: Vec<PathBuf> = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect();
            dirs.sort();
            out.extend(dirs);
        }
    }
    out
}

/// The file of a symbol reference: absolute, else the first folder of `search` that has
/// it.
pub fn find_symbol(reference: &str, search: &[PathBuf]) -> Option<PathBuf> {
    let path = Path::new(reference);
    if path.is_absolute() {
        return path.is_file().then(|| path.to_path_buf());
    }
    search
        .iter()
        .map(|dir| dir.join(path))
        .find(|candidate| candidate.is_file())
}

/// Read the symbol behind a reference. A symbol that cannot be found gives an empty
/// [`SymbolInfo`] (with `file: None`).
pub fn symbol_info(reference: &str, search: &[PathBuf]) -> SymbolInfo {
    let Some(file) = find_symbol(reference, search) else {
        return SymbolInfo::default();
    };
    let mut info = std::fs::read_to_string(&file)
        .map(|text| SymbolInfo::parse(&text))
        .unwrap_or_default();
    info.file = Some(file);
    info
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCH: &str = r#"v {xschem version=3.4.5 file_version=1.2
}
G {}
K {}
V {}
S {}
E {}
N -20 -50 -20 50 {lab=A}
C {ipin.sym} -60 0 0 0 {name=p1 lab=A}
C {sg13g2_pr/sg13_lv_pmos.sym} 0 -50 0 1 {name=M2
l=0.13u
w=2u
ng=2
m=1
model=sg13_lv_pmos
spiceprefix=X
}
C {logic/inv/symbol/inv.sym} 60 0 2 0 {name=x1 note="a \{braced\} text"}
[
v {xschem version=3.4.5 file_version=1.2}
K {type=subcircuit}
C {fake.sym} 0 0 0 0 {name=nope}
]
T {a text with {braces}} 0 0 0 0 0.4 0.4 {}
C {nmos4.sym} 10 20 0 0 {name="M 3" w=1u}
"#;

    #[test]
    fn instances_are_read() {
        let all = parse(SCH);
        let names: Vec<&str> = all.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["p1", "M2", "x1", "M 3"]);
        let m2 = &all[1];
        assert_eq!(m2.symbol, "sg13g2_pr/sg13_lv_pmos.sym");
        assert_eq!(m2.attributes["w"], "2u");
        assert_eq!(m2.attributes["ng"], "2");
        assert_eq!((m2.x, m2.y, m2.rotation, m2.flip), (0.0, -50.0, 0, 1));
        assert_eq!(all[2].attributes["note"], "a {braced} text");
        assert_eq!(all[2].rotation, 2);
        assert_eq!(all[3].attributes["w"], "1u");
    }

    #[test]
    fn symbol_defaults() {
        let sym = r#"v {xschem version=3.4.8RC file_version=1.3
}
G {}
K {type=nmos
lvs_format="M@name @pinlist @model w=@w l=@l ng=@ng m=@m"
format="@spiceprefix@name @pinlist @model w=@w l=@l ng=@ng m=@m"
template="name=M1
l=0.13u
w=0.15u
ng=1
m=1
model=sg13_lv_nmos
spiceprefix=X
"
}
"#;
        let info = SymbolInfo::parse(sym);
        assert_eq!(info.kind, "nmos");
        assert!(info.has_format);
        assert_eq!(info.template["w"], "0.15u");
        assert_eq!(info.template["model"], "sg13_lv_nmos");
        assert_eq!(info.template["name"], "M1");
    }
}
