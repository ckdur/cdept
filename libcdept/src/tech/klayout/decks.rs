//! DRC and LVS decks, macros and layer map files of a KLayout technology.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::xml;
use crate::tech::pdk::{PdkDir, TechPath};

/// A deck in detail (catalog): what it is and the switches it reads.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DeckDetail {
    /// `drc` or `lvs`.
    pub check: String,
    pub name: String,
    pub file: TechPath,
    /// `script` (`.drc`, `.lvs`: Ruby text) or `macro` (`.lydrc`, `.lylvs`: the text
    /// inside KLayout's macro XML).
    pub form: String,
    pub description: String,
    /// The deck CDePT runs unless the project names another.
    pub default: bool,
    /// Why it is where it is in the list: what made it the default, or not.
    pub why: String,
    /// Checks a part only (antenna, density, …), not the whole rule set.
    pub partial: bool,
    /// The `-rd name=value` variables the deck reads.
    pub switches: Vec<DeckSwitch>,
    /// Rule files next to the deck that it loads (`rule_decks/…`), counted.
    pub rule_files: usize,
    /// A command-line wrapper next to the deck (`run_drc.py`, `run_lvs.py`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wrapper: Option<TechPath>,
}

/// A variable a deck reads (`$name` in its text, set with `klayout -rd name=value`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DeckSwitch {
    pub name: String,
    /// `flag` (true/false), `number`, `file`, `text`.
    pub kind: String,
    /// The value the deck takes when the variable is not set, when the text says it.
    pub default: String,
    /// What CDePT uses it for, when it is one of the usual ones: `input`, `topcell`,
    /// `report`, `schematic`, `target_netlist`, `log`, `threads`, `run_mode`.
    pub role: String,
    /// The comment above its first use, when there is one.
    pub description: String,
}

/// A KLayout macro file (`.lym`, also a deck in macro form).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Macro {
    pub name: String,
    pub file: TechPath,
    pub description: String,
    pub category: String,
    /// `ruby`, `python`, `dsl`, `text`.
    pub interpreter: String,
    pub autorun: bool,
    pub autorun_early: bool,
    pub show_in_menu: bool,
    pub menu_path: String,
    pub group_name: String,
    pub shortcut: String,
}

/// A line of a LEF/DEF layer map file (`.map`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LayerMapEntry {
    /// The LEF layer, or `NAME` for a label rule.
    pub name: String,
    /// `NET`, `PIN`, `LEFPIN`, `VIA`, `FILL`, `LEFOBS`, …; for `NAME`, `layer/purpose`.
    pub purposes: Vec<String>,
    pub layer: i32,
    pub datatype: i32,
}

/// DRC or LVS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Check {
    Drc,
    Lvs,
}

impl Check {
    pub fn name(self) -> &'static str {
        match self {
            Check::Drc => "drc",
            Check::Lvs => "lvs",
        }
    }

    fn suffixes(self) -> [&'static str; 2] {
        match self {
            Check::Drc => ["drc", "lydrc"],
            Check::Lvs => ["lvs", "lylvs"],
        }
    }
}

/// Words in a deck's name that say it checks a part only (as
/// `klayout::verify::find_deck` has them).
const PARTIAL: [&str; 6] = ["antenna", "density", "maximal", "fill", "options", "test"];

fn files_in(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_file())
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    files
}

fn suffix(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The decks of a technology for `check`, the default first. `klayout` is the
/// `KLAYOUT_PATH` folder (`libs.tech/klayout`), `base` the technology's folder (where
/// the `.lyt` is). The order is the one `klayout::verify::find_deck` picks by:
/// `<base>/<check>`, `<klayout>/<check>`, `<base>` itself, then the macro folders; in a
/// folder whole decks before partial ones, decks named after the PDK first, plain
/// scripts before macros.
pub fn find(pdk: &PdkDir, klayout: &Path, base: &Path, check: Check) -> Vec<DeckDetail> {
    let mut dirs: Vec<PathBuf> = vec![
        klayout.join("tech").join(check.name()),
        base.join(check.name()),
        klayout.join(check.name()),
        klayout.join("tech"),
        base.to_path_buf(),
        base.join("macros"),
        klayout.join("macros"),
    ];
    let mut seen_dirs = BTreeSet::new();
    dirs.retain(|d| d.is_dir() && seen_dirs.insert(d.canonicalize().unwrap_or(d.clone())));

    let pdk_name = pdk.name.to_lowercase();
    let mut found: Vec<(usize, bool, bool, bool, String, PathBuf)> = Vec::new();
    for (index, dir) in dirs.iter().enumerate() {
        for file in files_in(dir) {
            if !check.suffixes().contains(&suffix(&file).as_str()) {
                continue;
            }
            let lower = stem(&file).to_lowercase();
            let partial = PARTIAL.iter().any(|w| lower.contains(w));
            let named = pdk_name
                .split(['-', '_'])
                .filter(|w| w.len() > 2)
                .any(|w| lower.contains(w));
            let macro_form = suffix(&file).starts_with("ly");
            found.push((index, partial, !named, macro_form, lower, file));
        }
    }
    found.sort();

    let mut decks = Vec::new();
    for (place, (index, partial, unnamed, macro_form, _, file)) in found.into_iter().enumerate() {
        let source = read_deck(&file);
        let folder = file.parent().unwrap_or(Path::new("."));
        let wrapper = folder.join(format!("run_{}.py", check.name()));
        let mut why = Vec::new();
        why.push(format!(
            "in {}",
            dirs[index]
                .strip_prefix(klayout)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| dirs[index].to_string_lossy().into_owned())
        ));
        if partial {
            why.push("a partial check by its name".to_string());
        }
        why.push(
            if unnamed {
                "not named after the PDK"
            } else {
                "named after the PDK"
            }
            .to_string(),
        );
        why.push(
            if macro_form {
                "a macro"
            } else {
                "a plain script"
            }
            .to_string(),
        );
        decks.push(DeckDetail {
            check: check.name().to_string(),
            name: stem(&file),
            file: pdk.portable(&file),
            form: if macro_form { "macro" } else { "script" }.to_string(),
            description: source.description,
            default: place == 0,
            why: why.join(", "),
            partial,
            switches: switches(&source.text),
            rule_files: count_rule_files(folder, check),
            wrapper: wrapper.is_file().then(|| pdk.portable(&wrapper)),
        });
    }
    decks
}

/// Files of the deck's kind in `rule_decks` below the deck's folder.
fn count_rule_files(folder: &Path, check: Check) -> usize {
    fn walk(dir: &Path, check: Check, depth: usize) -> usize {
        if depth > 4 {
            return 0;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return 0;
        };
        entries
            .flatten()
            .map(|e| e.path())
            .map(|p| {
                if p.is_dir() {
                    walk(&p, check, depth + 1)
                } else {
                    usize::from(check.suffixes().contains(&suffix(&p).as_str()))
                }
            })
            .sum()
    }
    walk(&folder.join("rule_decks"), check, 0)
}

/// The Ruby text of a deck and its description.
struct DeckSource {
    text: String,
    description: String,
}

fn read_deck(file: &Path) -> DeckSource {
    let Ok(raw) = std::fs::read_to_string(file) else {
        return DeckSource {
            text: String::new(),
            description: String::new(),
        };
    };
    if suffix(file).starts_with("ly")
        && let Ok(root) = xml::parse(&raw)
    {
        return DeckSource {
            text: root
                .child("text")
                .map(|t| t.text.clone())
                .unwrap_or_default(),
            description: root.text_of("description").unwrap_or_default().to_string(),
        };
    }
    DeckSource {
        description: title_comment(&raw),
        text: raw,
    }
}

/// What comment rulers are made of.
fn ruler(c: char) -> bool {
    matches!(c, '#' | '-' | '=' | '*' | '~') || c.is_whitespace()
}

/// The first comment line of a script that reads like a title: not a licence line,
/// not a ruler.
fn title_comment(text: &str) -> String {
    for line in text.lines().take(60) {
        let Some(comment) = line.trim().strip_prefix('#') else {
            if line.trim().is_empty() {
                continue;
            }
            break;
        };
        let comment = comment.trim_matches(ruler);
        let lower = comment.to_lowercase();
        if comment.len() < 8
            || !comment.chars().any(char::is_alphabetic)
            || [
                "copyright",
                "licen",
                "warrant",
                "apache",
                "you may",
                "http",
                "unless",
                "distributed",
                "see the",
                "limitations",
                "frozen_string",
                "spdx",
                "coding:",
                "klayout -",
            ]
            .iter()
            .any(|w| lower.contains(w))
        {
            continue;
        }
        // "DRC  for  SKY130 according to :" → "DRC for SKY130 according to"
        let words: Vec<&str> = comment.split_whitespace().collect();
        return words.join(" ").trim_end_matches([' ', ':']).to_string();
    }
    String::new()
}

/// Ruby's own `$` variables, not switches.
const RUBY_GLOBALS: [&str; 10] = [
    "stdout",
    "stderr",
    "stdin",
    "PROGRAM_NAME",
    "LOAD_PATH",
    "LOADED_FEATURES",
    "DEBUG",
    "VERBOSE",
    "FILENAME",
    "logger",
];

fn role_of(name: &str) -> &'static str {
    match name {
        "input" | "in_gds" | "gds_input" => "input",
        "topcell" | "top_cell" | "top" => "topcell",
        "report" | "report_file" | "output" => "report",
        "schematic" | "schematic_netlist" => "schematic",
        "target_netlist" => "target_netlist",
        "log" | "log_file" => "log",
        "threads" | "thr" => "threads",
        "run_mode" => "run_mode",
        _ => "",
    }
}

/// The variables a deck's Ruby text reads, in the order of first use.
pub fn switches(text: &str) -> Vec<DeckSwitch> {
    let lines: Vec<&str> = text.lines().collect();
    let mut order: Vec<String> = Vec::new();
    let mut first_line: Vec<usize> = Vec::new();
    let mut internal: Vec<String> = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if line.trim_start().starts_with('#') {
            continue;
        }
        for name in variables(line) {
            if RUBY_GLOBALS.contains(&name)
                || name.len() < 2
                || order.iter().any(|n| n == name)
                || internal.iter().any(|n| n == name)
            {
                continue;
            }
            // Set before it is ever read, and not from itself: the deck's own
            // variable, not something to pass in.
            let var = format!("${name}");
            if let Some(value) = line
                .trim()
                .strip_prefix(&var)
                .map(str::trim_start)
                .and_then(|rest| rest.strip_prefix('='))
                .filter(|value| !value.starts_with('=') && !value.starts_with('~'))
                && !mentions(value, &var)
                && !(index > 0 && lines[index - 1].contains("defined?"))
            {
                internal.push(name.to_string());
                continue;
            }
            order.push(name.to_string());
            first_line.push(index);
        }
    }

    order
        .iter()
        .zip(first_line)
        .map(|(name, first)| {
            let var = format!("${name}");
            let uses: Vec<(usize, &str)> = lines
                .iter()
                .enumerate()
                .filter(|(_, l)| !l.trim_start().starts_with('#') && mentions(l, &var))
                .map(|(i, l)| (i, *l))
                .collect();
            let mut default = String::new();
            let mut flag = false;
            let mut number = false;
            for (index, line) in &uses {
                if default.is_empty()
                    && let Some(value) = default_in(line, &var, lines.get(index.wrapping_sub(1)))
                {
                    default = value;
                }
                let compact: String = line.split_whitespace().collect();
                if compact.contains(&format!("bool_check?({var})"))
                    || [
                        "==\"true\"",
                        "=='true'",
                        "==\"false\"",
                        "=='false'",
                        "!=\"true\"",
                        "!='true'",
                        "!=\"false\"",
                        "!='false'",
                    ]
                    .iter()
                    .any(|c| compact.contains(&format!("{var}{c}")))
                {
                    flag = true;
                }
                if compact.contains(&format!("{var}.to_i"))
                    || compact.contains(&format!("{var}.to_f"))
                    || (compact.contains(").to_i") && compact.contains(&format!("({var}||")))
                    || compact.contains(&format!("threads({var})"))
                {
                    number = true;
                }
            }
            let role = role_of(name);
            let lower = name.to_lowercase();
            let kind = if flag || default == "true" || default == "false" {
                "flag"
            } else if number || (!default.is_empty() && default.parse::<f64>().is_ok()) {
                "number"
            } else if matches!(
                role,
                "input" | "report" | "schematic" | "target_netlist" | "log"
            ) || ["netlist", "json", "file", "path", "gds"]
                .iter()
                .any(|w| lower.contains(w))
            {
                "file"
            } else {
                "text"
            };
            if kind == "flag" && default.is_empty() {
                default = "false".to_string();
            }
            DeckSwitch {
                name: name.clone(),
                kind: kind.to_string(),
                default,
                role: role.to_string(),
                description: comment_above(&lines, first),
            }
        })
        .collect()
}

/// The `$name` variables in a line of Ruby.
fn variables(line: &str) -> Vec<&str> {
    let bytes = line.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$'
            && i + 1 < bytes.len()
            && (bytes[i + 1].is_ascii_alphabetic() || bytes[i + 1] == b'_')
            // "\$x" and "a$b" are not variables.
            && (i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'\\'))
        {
            let start = i + 1;
            let mut end = start;
            while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
                end += 1;
            }
            out.push(&line[start..end]);
            i = end;
        } else {
            i += 1;
        }
    }
    out
}

/// `line` uses the variable `var` (`$name`), not a longer one starting the same.
fn mentions(line: &str, var: &str) -> bool {
    variables(line).iter().any(|name| *name == &var[1..])
}

/// The default a line gives a variable: `$x || 'main'`, `$x ||= 4`, `$x = "false"`
/// right after `if not defined? $x`, `$x ? f($x) : f(4)`.
fn default_in(line: &str, var: &str, previous: Option<&&str>) -> Option<String> {
    let literal = |text: &str| -> Option<String> {
        let text = text.trim_start();
        let first = text.chars().next()?;
        if first == '\'' || first == '"' {
            let end = text[1..].find(first)?;
            return Some(text[1..1 + end].to_string());
        }
        let word: String = text
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '-'))
            .collect();
        // A call (`File.expand_path(…)`) is computed, not a default to show.
        let called = text[word.len()..].starts_with('(');
        (!word.is_empty() && word != "nil" && !called).then_some(word)
    };
    for pattern in [
        format!("{var} ||="),
        format!("{var} ||"),
        format!("{var}||"),
    ] {
        if let Some(at) = line.find(&pattern) {
            let rest = &line[at + pattern.len()..];
            // `$x || $y`: another variable is not a default.
            if rest.trim_start().starts_with('$') {
                continue;
            }
            if let Some(value) = literal(rest) {
                return Some(value);
            }
        }
    }
    let trimmed = line.trim();
    if let Some(rest) = trimmed.strip_prefix(var)
        && let Some(value) = rest.trim_start().strip_prefix('=')
        && !value.starts_with('=')
        && previous.is_some_and(|p| p.contains("defined?") && mentions(p, var))
    {
        return literal(value);
    }
    // `$thr ? threads($thr) : threads(4)`
    if let Some(rest) = trimmed.strip_prefix(&format!("{var} ?"))
        && let Some((_, otherwise)) = rest.rsplit_once(':')
        && let Some(open) = otherwise.find('(')
    {
        return literal(&otherwise[open + 1..]);
    }
    None
}

/// The comment right above line `index`, when it reads like a description.
fn comment_above(lines: &[&str], index: usize) -> String {
    let mut at = index;
    // Step over the guard a default is set under: `if not defined? $x`.
    while at > 0 && (lines[at - 1].contains("defined?") || lines[at - 1].trim() == "begin") {
        at -= 1;
    }
    if at == 0 {
        return String::new();
    }
    let above = lines[at - 1].trim();
    let Some(comment) = above.strip_prefix('#') else {
        return String::new();
    };
    let comment = comment.trim_matches(ruler);
    // Not a description: a ruler, commented-out code, a usage line.
    if comment.len() > 120
        || !comment.chars().any(char::is_alphabetic)
        || comment.contains('$')
        || comment.contains("klayout -")
    {
        return String::new();
    }
    comment.to_string()
}

/// The macros (`.lym`) in the macro folders of a technology.
pub fn macros(pdk: &PdkDir, klayout: &Path, base: &Path) -> Vec<Macro> {
    let mut dirs = Vec::new();
    for root in [base, klayout] {
        for sub in ["macros", "pymacros", "ruby", "drc", "lvs"] {
            dirs.push(root.join(sub));
        }
    }
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for dir in dirs {
        for file in files_in(&dir) {
            if !matches!(suffix(&file).as_str(), "lym" | "lydrc" | "lylvs")
                || !seen.insert(file.canonicalize().unwrap_or(file.clone()))
            {
                continue;
            }
            let Ok(root) = std::fs::read_to_string(&file)
                .map_err(|e| e.to_string())
                .and_then(|text| xml::parse(&text))
            else {
                continue;
            };
            let word = |name: &str| root.text_of(name).unwrap_or_default().to_string();
            let interpreter = match word("interpreter").as_str() {
                "dsl" => {
                    let dsl = word("dsl-interpreter-name");
                    if dsl.is_empty() {
                        "dsl".to_string()
                    } else {
                        dsl
                    }
                }
                other => other.to_string(),
            };
            out.push(Macro {
                name: stem(&file),
                file: pdk.portable(&file),
                description: word("description"),
                category: word("category"),
                interpreter,
                autorun: root.flag("autorun").unwrap_or(false),
                autorun_early: root.flag("autorun-early").unwrap_or(false),
                show_in_menu: root.flag("show-in-menu").unwrap_or(false),
                menu_path: word("menu-path"),
                group_name: word("group-name"),
                shortcut: word("shortcut"),
            });
        }
    }
    out
}

/// Files with one of `suffixes` in `dirs`, each once.
pub fn files_with(dirs: &[PathBuf], suffixes: &[&str]) -> Vec<PathBuf> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for dir in dirs {
        for file in files_in(dir) {
            if suffixes.contains(&suffix(&file).as_str())
                && seen.insert(file.canonicalize().unwrap_or(file.clone()))
            {
                out.push(file);
            }
        }
    }
    out
}

/// Read a LEF/DEF layer map (`.map`): `<layer> <purposes> <gds layer> <datatype>`.
pub fn layer_map(text: &str) -> Vec<LayerMapEntry> {
    text.lines()
        .filter_map(|line| {
            let line = line.split('#').next().unwrap_or_default();
            let words: Vec<&str> = line.split_whitespace().collect();
            let [name, purposes, layer, datatype] = words.as_slice() else {
                return None;
            };
            Some(LayerMapEntry {
                name: name.to_string(),
                purposes: purposes.split(',').map(str::to_string).collect(),
                layer: layer.parse().ok()?,
                datatype: datatype.parse().ok()?,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const IHP: &str = r#"
# frozen_string_literal: true
#==========================================================================
# Copyright 2025 IHP PDK Authors
#==========================================================================
file_logger = Logger.new($log)
$stdout.sync = true
logger.info("Starting running IHP-SG13G2 KLayout DRC runset on #{$input}")
if $input
  if $topcell
    source($input, $topcell)
  end
end
if $report
  report('Main DRC Run Report at', $report)
end
TABLES = ($tables || 'main').split

# Threads
$threads = ($threads || Etc.nprocessors).to_i
threads($threads)

# Run mode
$run_mode = $run_mode || 'deep'

# FEOL
FEOL = !bool_check?($no_feol)
# Minimum DRC (run_drc.py --minimum_drc sets precheck_drc)
PRECHECK_DRC = bool_check?($precheck_drc)
$drc_json = $drc_json || File.expand_path(File.join(script_dir, 'x.json'))
"#;

    fn by_name<'a>(switches: &'a [DeckSwitch], name: &str) -> &'a DeckSwitch {
        switches
            .iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("no switch {name} in {switches:?}"))
    }

    #[test]
    fn switches_of_an_ihp_style_deck() {
        let found = switches(IHP);
        let names: Vec<&str> = found.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "log",
                "input",
                "topcell",
                "report",
                "tables",
                "threads",
                "run_mode",
                "no_feol",
                "precheck_drc",
                "drc_json"
            ]
        );
        assert_eq!(by_name(&found, "input").role, "input");
        assert_eq!(by_name(&found, "input").kind, "file");
        assert_eq!(by_name(&found, "topcell").role, "topcell");
        assert_eq!(by_name(&found, "tables").default, "main");
        assert_eq!(by_name(&found, "tables").kind, "text");
        let threads = by_name(&found, "threads");
        assert_eq!(
            (threads.kind.as_str(), threads.role.as_str()),
            ("number", "threads")
        );
        assert_eq!(threads.default, "Etc.nprocessors");
        assert_eq!(threads.description, "Threads");
        let mode = by_name(&found, "run_mode");
        assert_eq!(
            (mode.default.as_str(), mode.role.as_str()),
            ("deep", "run_mode")
        );
        let feol = by_name(&found, "no_feol");
        assert_eq!(
            (
                feol.kind.as_str(),
                feol.default.as_str(),
                feol.description.as_str()
            ),
            ("flag", "false", "FEOL")
        );
        assert!(
            by_name(&found, "precheck_drc")
                .description
                .starts_with("Minimum DRC")
        );
        assert_eq!(by_name(&found, "drc_json").kind, "file");
        assert_eq!(
            by_name(&found, "drc_json").default,
            "",
            "a call is not a default"
        );
    }

    #[test]
    fn switches_of_a_sky130_style_deck() {
        let text = r#"
source($input, $top_cell) if $input
# feol switch
if not defined? $feol
  $feol = "false"
end
if $feol == "0" || $feol == "false"
  FEOL = false
end
$thr ? threads($thr) : threads(4)
if $seal == "0" || $seal == "false"
end
# $errs = 0
$errs = 0
$errs += 1 if $input
#=== RUN MODE ===
if $run_mode == "tiling"
end
"#;
        let found = switches(text);
        assert_eq!(by_name(&found, "top_cell").role, "topcell");
        let feol = by_name(&found, "feol");
        assert_eq!(
            (feol.kind.as_str(), feol.default.as_str()),
            ("flag", "false")
        );
        assert_eq!(feol.description, "feol switch");
        let thr = by_name(&found, "thr");
        assert_eq!(
            (thr.kind.as_str(), thr.default.as_str(), thr.role.as_str()),
            ("number", "4", "threads")
        );
        assert_eq!(by_name(&found, "seal").kind, "flag");
        assert!(
            found.iter().all(|s| s.name != "errs"),
            "the deck's own variable"
        );
        assert_eq!(by_name(&found, "run_mode").description, "RUN MODE");
    }

    #[test]
    fn titles_skip_the_licence() {
        assert_eq!(
            title_comment(
                "# Copyright 2025 X\n#\n# Licensed under the Apache License\n#====\n\
                 #------- DRC for SKY130 according to the rules -------\nx = 1\n"
            ),
            "DRC for SKY130 according to the rules"
        );
        assert_eq!(title_comment("x = 1\n# later\n"), "");
    }

    #[test]
    fn layer_map_lines() {
        let map = layer_map(
            "# comment\nMetal1  NET,SPNET,PIN  8  0\nNAME  Metal1/PIN  8  25\nbroken line\n",
        );
        assert_eq!(map.len(), 2);
        assert_eq!(map[0].purposes, ["NET", "SPNET", "PIN"]);
        assert_eq!(
            (map[1].name.as_str(), map[1].layer, map[1].datatype),
            ("NAME", 8, 25)
        );
    }

    #[test]
    fn the_default_deck_is_the_one_named_after_the_pdk() {
        let dir = tempfile::tempdir().unwrap();
        let pdk = PdkDir::new(dir.path(), "sky130A");
        let klayout = pdk.tech("klayout");
        let drc = klayout.join("drc");
        std::fs::create_dir_all(&drc).unwrap();
        std::fs::create_dir_all(klayout.join("tech")).unwrap();
        std::fs::write(drc.join("zeroarea.rb.drc"), "source($input)\n").unwrap();
        std::fs::write(
            drc.join("sky130A_mr.drc"),
            "# The SKY130 rules\nsource($input)\n",
        )
        .unwrap();
        std::fs::write(
            drc.join("sky130A.lydrc"),
            "<klayout-macro><description>macro form</description><category>drc</category>\
             <text>report(\"x\", $report)</text></klayout-macro>",
        )
        .unwrap();
        std::fs::write(drc.join("met_density.lydrc"), "<klayout-macro/>").unwrap();
        std::fs::write(drc.join("run_drc.py"), "").unwrap();
        let decks = find(&pdk, &klayout, &klayout.join("tech"), Check::Drc);
        let names: Vec<&str> = decks.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(
            names,
            ["sky130A_mr", "sky130A", "zeroarea.rb", "met_density"]
        );
        assert!(decks[0].default && !decks[1].default);
        assert_eq!(decks[0].form, "script");
        assert_eq!(decks[0].description, "The SKY130 rules");
        assert_eq!(
            decks[0].file.as_str(),
            "$PDK_ROOT/sky130A/libs.tech/klayout/drc/sky130A_mr.drc"
        );
        assert!(decks[0].why.contains("named after the PDK"));
        assert_eq!(
            decks[0].wrapper.as_ref().unwrap().as_str(),
            "$PDK_ROOT/sky130A/libs.tech/klayout/drc/run_drc.py"
        );
        assert_eq!(decks[1].form, "macro");
        assert_eq!(decks[1].description, "macro form");
        assert_eq!(decks[1].switches[0].name, "report");
        assert!(decks[3].partial);
        assert!(find(&pdk, &klayout, &klayout.join("tech"), Check::Lvs).is_empty());
    }
}
