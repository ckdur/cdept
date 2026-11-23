//! The files of a PDK's libraries as the LibreLane or OpenLane configuration names
//! them (`libs.tech/librelane/config.tcl`, `libs.tech/openlane/config.tcl` and the
//! `config.tcl` of each library).
//!
//! The configuration is Tcl, but only a few things of it matter: `set ::env(NAME)
//! value`, `dict set ::env(NAME) key value`, `if {![info exist ::env(X)]} { … }`,
//! `source`, `glob` and `$::env(X)` in strings. [`Evaluator`] understands just
//! that, without running Tcl, and reports in its notes what it did not understand.
//! Nothing in it fails: a PDK without configuration, or one that is not read as
//! expected, simply gives fewer files.
//!
//! The values it ends with are looked at for paths below `libs.ref/<library>/`; each
//! such file belongs to that library, and its extension says which format it is.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use super::pdk::PdkDir;

/// A value: a string (a list is words joined by spaces) or a dict.
#[derive(Debug, Clone, PartialEq)]
pub enum Val {
    Str(String),
    /// Key path → value, in the order set.
    Dict(Vec<(Vec<String>, String)>),
}

impl Val {
    /// As Tcl would print it.
    pub fn text(&self) -> String {
        match self {
            Val::Str(s) => s.clone(),
            Val::Dict(items) => items
                .iter()
                .map(|(keys, value)| format!("{} {}", keys.join(" "), value))
                .collect::<Vec<_>>()
                .join(" "),
        }
    }
}

/// One word of a command: its text, and whether it was written in braces (and so is
/// not substituted).
#[derive(Debug, Clone, PartialEq)]
struct Word {
    text: String,
    braced: bool,
}

/// Split a script into commands of words. Comments are dropped.
fn commands(text: &str) -> Vec<Vec<Word>> {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut i = 0;
    let mut out = Vec::new();
    let continued = |chars: &[char], i: usize| chars[i] == '\\' && chars.get(i + 1) == Some(&'\n');
    while i < n {
        let c = chars[i];
        if c.is_whitespace() || c == ';' {
            i += 1;
            continue;
        }
        if c == '#' {
            while i < n && chars[i] != '\n' {
                i += if continued(&chars, i) { 2 } else { 1 };
            }
            continue;
        }
        let mut words = Vec::new();
        loop {
            while i < n && (chars[i] == ' ' || chars[i] == '\t' || chars[i] == '\r') {
                i += 1;
            }
            if i < n && continued(&chars, i) {
                i += 2;
                continue;
            }
            if i >= n || chars[i] == '\n' || chars[i] == ';' {
                i += 1;
                break;
            }
            let mut word = String::new();
            let braced = chars[i] == '{';
            if braced {
                let mut depth = 0;
                while i < n {
                    let c = chars[i];
                    if c == '\\' && i + 1 < n {
                        if chars[i + 1] == '\n' {
                            word.push(' ');
                        } else {
                            word.push(c);
                            word.push(chars[i + 1]);
                        }
                        i += 2;
                        continue;
                    }
                    i += 1;
                    if c == '{' {
                        depth += 1;
                        if depth == 1 {
                            continue;
                        }
                    } else if c == '}' {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    word.push(c);
                }
            } else if chars[i] == '"' {
                i += 1;
                let mut brackets = 0;
                while i < n {
                    let c = chars[i];
                    if c == '\\' && i + 1 < n {
                        word.push(c);
                        word.push(chars[i + 1]);
                        i += 2;
                        continue;
                    }
                    i += 1;
                    match c {
                        '[' => brackets += 1,
                        ']' if brackets > 0 => brackets -= 1,
                        '"' if brackets == 0 => break,
                        _ => {}
                    }
                    word.push(c);
                }
            } else {
                let mut brackets = 0;
                while i < n {
                    let c = chars[i];
                    if brackets == 0 && (c.is_whitespace() || c == ';') {
                        break;
                    }
                    if c == '\\' && i + 1 < n {
                        if chars[i + 1] == '\n' && brackets == 0 {
                            break;
                        }
                        word.push(c);
                        word.push(chars[i + 1]);
                        i += 2;
                        continue;
                    }
                    match c {
                        '[' => brackets += 1,
                        ']' if brackets > 0 => brackets -= 1,
                        _ => {}
                    }
                    word.push(c);
                    i += 1;
                }
            }
            words.push(Word { text: word, braced });
        }
        if !words.is_empty() {
            out.push(words);
        }
    }
    out
}

/// `::env(NAME)` → `NAME`.
fn env_name(word: &str) -> Option<&str> {
    word.strip_prefix("::env(")?.strip_suffix(')')
}

/// Whether `name` matches a glob pattern of `*` and `?`.
fn wild(pattern: &[char], name: &[char]) -> bool {
    match pattern.split_first() {
        None => name.is_empty(),
        Some(('*', rest)) => (0..=name.len()).any(|at| wild(rest, &name[at..])),
        Some(('?', rest)) => !name.is_empty() && wild(rest, &name[1..]),
        Some((c, rest)) => name.first() == Some(c) && wild(rest, &name[1..]),
    }
}

/// The files and folders a glob pattern matches, sorted.
fn glob(pattern: &str) -> Vec<PathBuf> {
    let mut current = vec![if pattern.starts_with('/') {
        PathBuf::from("/")
    } else {
        PathBuf::new()
    }];
    for part in pattern.split('/').filter(|part| !part.is_empty()) {
        let mut next = Vec::new();
        for base in &current {
            if part.contains(['*', '?']) {
                let dir = if base.as_os_str().is_empty() {
                    Path::new(".")
                } else {
                    base.as_path()
                };
                let pattern: Vec<char> = part.chars().collect();
                for item in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                    let name = item.file_name().to_string_lossy().into_owned();
                    if (!name.starts_with('.') || part.starts_with('.'))
                        && wild(&pattern, &name.chars().collect::<Vec<_>>())
                    {
                        next.push(base.join(&name));
                    }
                }
            } else {
                let path = base.join(part);
                if path.exists() {
                    next.push(path);
                }
            }
        }
        current = next;
    }
    current.sort();
    current
}

/// A tolerant evaluator of the Tcl a LibreLane/OpenLane configuration is made of.
pub struct Evaluator {
    /// `$::env(NAME)`.
    pub env: BTreeMap<String, Val>,
    locals: BTreeMap<String, Val>,
    /// What was not understood or not found, each once.
    pub notes: BTreeSet<String>,
    /// Files read by `source`.
    pub sourced: BTreeSet<PathBuf>,
    script: PathBuf,
    depth: usize,
    budget: usize,
}

impl Evaluator {
    pub fn new(env: impl IntoIterator<Item = (String, String)>) -> Evaluator {
        Evaluator {
            env: env.into_iter().map(|(k, v)| (k, Val::Str(v))).collect(),
            locals: BTreeMap::new(),
            notes: BTreeSet::new(),
            sourced: BTreeSet::new(),
            script: PathBuf::new(),
            depth: 0,
            budget: 50_000,
        }
    }

    fn note(&mut self, text: String) {
        let file = self
            .script
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.notes.insert(if file.is_empty() {
            text
        } else {
            format!("{file}: {text}")
        });
    }

    /// Run a script given as text. `file` is what `info script` says.
    pub fn run(&mut self, text: &str, file: &Path) {
        let before = std::mem::replace(&mut self.script, file.to_path_buf());
        self.script_text(text);
        self.script = before;
    }

    /// Run the commands of `text`; the value of the last one.
    fn script_text(&mut self, text: &str) -> Val {
        let mut last = Val::Str(String::new());
        for words in commands(text) {
            if self.budget == 0 {
                self.note("stopped reading: too many commands".into());
                break;
            }
            self.budget -= 1;
            last = self.command(&words);
        }
        last
    }

    /// The value of a word: variables and `[commands]` replaced.
    fn word(&mut self, word: &Word) -> Val {
        if word.braced {
            return Val::Str(word.text.clone());
        }
        let text = &word.text;
        // A word that is one command keeps the value (a dict) as it is.
        if text.starts_with('[')
            && text.ends_with(']')
            && self.matching(text, 0) == Some(text.len() - 1)
        {
            return self.script_text(&text[1..text.len() - 1]);
        }
        let chars: Vec<char> = text.chars().collect();
        let mut out = String::new();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            match c {
                '\\' if i + 1 < chars.len() => {
                    i += 1;
                    match chars[i] {
                        '\n' => {
                            while chars.get(i + 1).is_some_and(|c| *c == ' ' || *c == '\t') {
                                i += 1;
                            }
                            out.push(' ');
                        }
                        'n' => out.push('\n'),
                        't' => out.push('\t'),
                        other => out.push(other),
                    }
                    i += 1;
                }
                '$' => {
                    let rest: String = chars[i..].iter().collect();
                    if let Some(after) = rest.strip_prefix("$::env(") {
                        match after.find(')') {
                            Some(end) => {
                                let name = &after[..end];
                                match self.env.get(name) {
                                    Some(value) => out.push_str(&value.text()),
                                    None => self
                                        .note(format!("$::env({name}) is used before it is set")),
                                }
                                i += "$::env(".len() + end + 1;
                            }
                            None => {
                                out.push('$');
                                i += 1;
                            }
                        }
                    } else if let Some(after) = rest.strip_prefix("${") {
                        match after.find('}') {
                            Some(end) => {
                                if let Some(value) = self.locals.get(&after[..end]) {
                                    out.push_str(&value.text());
                                }
                                i += 2 + end + 1;
                            }
                            None => {
                                out.push('$');
                                i += 1;
                            }
                        }
                    } else {
                        let name: String = rest[1..]
                            .chars()
                            .take_while(|c| c.is_alphanumeric() || *c == '_')
                            .collect();
                        if name.is_empty() {
                            out.push('$');
                            i += 1;
                        } else {
                            match self.locals.get(&name) {
                                Some(value) => out.push_str(&value.text()),
                                None => self.note(format!("variable ${name} is not set")),
                            }
                            i += 1 + name.chars().count();
                        }
                    }
                }
                '[' => {
                    let at: usize = chars[..i].iter().map(|c| c.len_utf8()).sum();
                    match self.matching(text, at) {
                        Some(end) => {
                            let value = self.script_text(&text[at + 1..end]);
                            out.push_str(&value.text());
                            i += text[at..=end].chars().count();
                        }
                        None => {
                            out.push('[');
                            i += 1;
                        }
                    }
                }
                other => {
                    out.push(other);
                    i += 1;
                }
            }
        }
        Val::Str(out)
    }

    /// The byte offset of the `]` matching the `[` at byte `at`.
    fn matching(&self, text: &str, at: usize) -> Option<usize> {
        let mut depth = 0;
        let mut quoted = false;
        let mut escaped = false;
        for (offset, c) in text[at..].char_indices() {
            if escaped {
                escaped = false;
                continue;
            }
            match c {
                '\\' => escaped = true,
                '"' => quoted = !quoted,
                '[' if !quoted => depth += 1,
                ']' if !quoted => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(at + offset);
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn command(&mut self, words: &[Word]) -> Val {
        let empty = || Val::Str(String::new());
        let name = words[0].text.clone();
        // `if` evaluates its condition and bodies itself.
        if name == "if" {
            return self.conditional(&words[1..]);
        }
        let args: Vec<Val> = words[1..].iter().map(|w| self.word(w)).collect();
        let text = |i: usize| args.get(i).map(Val::text).unwrap_or_default();
        match name.as_str() {
            "set" => match args.len() {
                1 => match env_name(&text(0)) {
                    Some(var) => self.env.get(var).cloned().unwrap_or_else(empty),
                    None => self.locals.get(&text(0)).cloned().unwrap_or_else(empty),
                },
                2 => {
                    let target = text(0);
                    match env_name(&target) {
                        Some(var) => self.env.insert(var.to_string(), args[1].clone()),
                        None => self.locals.insert(target, args[1].clone()),
                    };
                    args[1].clone()
                }
                _ => {
                    self.note("`set` with another number of arguments".into());
                    empty()
                }
            },
            "dict" => match text(0).as_str() {
                "create" => {
                    let mut items = Vec::new();
                    for pair in args[1..].chunks(2) {
                        if let [key, value] = pair {
                            items.push((vec![key.text()], value.text()));
                        }
                    }
                    Val::Dict(items)
                }
                "set" if args.len() >= 4 => {
                    let target = text(1);
                    let keys: Vec<String> = args[2..args.len() - 1].iter().map(Val::text).collect();
                    let value = text(args.len() - 1)
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ");
                    let Some(var) = env_name(&target) else {
                        self.note(format!("`dict set {target}`: only ::env(…) is followed"));
                        return empty();
                    };
                    let entry = self
                        .env
                        .entry(var.to_string())
                        .or_insert(Val::Dict(Vec::new()));
                    if !matches!(entry, Val::Dict(_)) {
                        *entry = Val::Dict(Vec::new());
                    }
                    if let Val::Dict(items) = entry {
                        match items.iter_mut().find(|(k, _)| *k == keys) {
                            Some(item) => item.1 = value,
                            None => items.push((keys, value)),
                        }
                    }
                    empty()
                }
                other => {
                    self.note(format!("`dict {other}` is not understood"));
                    empty()
                }
            },
            "glob" => {
                let mut nocomplain = false;
                let mut found = Vec::new();
                for arg in &args {
                    let pattern = arg.text();
                    if pattern == "-nocomplain" {
                        nocomplain = true;
                    } else if pattern.starts_with('-') {
                        continue;
                    } else {
                        let matches = glob(&pattern);
                        if matches.is_empty() && !nocomplain {
                            self.note(format!("glob {pattern} matches no file"));
                        }
                        found.extend(matches);
                    }
                }
                Val::Str(
                    found
                        .iter()
                        .map(|p| p.to_string_lossy().into_owned())
                        .collect::<Vec<_>>()
                        .join(" "),
                )
            }
            "source" => {
                let file = PathBuf::from(text(args.len().saturating_sub(1)));
                match std::fs::read_to_string(&file) {
                    Ok(body) if self.depth < 8 => {
                        self.depth += 1;
                        self.sourced.insert(file.clone());
                        self.run(&body, &file);
                        self.depth -= 1;
                    }
                    Ok(_) => self.note("sources nested too deeply".into()),
                    Err(_) => self.note(format!("source: cannot read {}", file.display())),
                }
                empty()
            }
            "info" => match text(0).as_str() {
                "exist" | "exists" => {
                    let var = text(1);
                    let set = match env_name(&var) {
                        Some(var) => self.env.contains_key(var),
                        None => self.locals.contains_key(&var),
                    };
                    Val::Str(if set { "1" } else { "0" }.into())
                }
                "script" => Val::Str(self.script.to_string_lossy().into_owned()),
                other => {
                    self.note(format!("`info {other}` is not understood"));
                    empty()
                }
            },
            "file" => {
                let path = PathBuf::from(text(1));
                match text(0).as_str() {
                    "exists" => Val::Str(if path.exists() { "1" } else { "0" }.into()),
                    "dirname" => Val::Str(
                        path.parent()
                            .map(|p| p.to_string_lossy().into_owned())
                            .unwrap_or_else(|| ".".into()),
                    ),
                    "tail" => Val::Str(
                        path.file_name()
                            .map(|p| p.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                    ),
                    "normalize" => Val::Str(path.to_string_lossy().into_owned()),
                    "join" => {
                        let mut joined = PathBuf::new();
                        for part in &args[1..] {
                            joined.push(part.text());
                        }
                        Val::Str(joined.to_string_lossy().into_owned())
                    }
                    other => {
                        self.note(format!("`file {other}` is not understood"));
                        empty()
                    }
                }
            }
            "puts" | "package" | "namespace" => empty(),
            other => {
                self.note(format!("command `{other}` is not understood, skipped"));
                empty()
            }
        }
    }

    /// `if cond body ?elseif cond body? ?else body?`.
    fn conditional(&mut self, words: &[Word]) -> Val {
        let mut i = 0;
        while let Some(cond) = words.get(i) {
            i += 1;
            if words.get(i).is_some_and(|w| w.text == "then") {
                i += 1;
            }
            let Some(body) = words.get(i) else { break };
            i += 1;
            if self.truth(&cond.text) == Some(true) {
                return self.script_text(&body.text);
            }
            match words.get(i).map(|w| w.text.as_str()) {
                Some("elseif") => i += 1,
                Some("else") => {
                    if let Some(body) = words.get(i + 1) {
                        return self.script_text(&body.text);
                    }
                    break;
                }
                _ => break,
            }
        }
        Val::Str(String::new())
    }

    /// A condition: `[command]`, `![command]`, or `0`/`1`. `None`: not understood
    /// (the branch is skipped).
    fn truth(&mut self, cond: &str) -> Option<bool> {
        let mut cond = cond.trim();
        let mut negate = false;
        while let Some(rest) = cond.strip_prefix('!') {
            negate = !negate;
            cond = rest.trim();
        }
        let value = if cond.starts_with('[') && cond.ends_with(']') {
            self.script_text(&cond[1..cond.len() - 1]).text()
        } else {
            cond.to_string()
        };
        let truth = match value.trim() {
            "1" | "true" | "yes" | "on" => true,
            "0" | "false" | "no" | "off" | "" => false,
            _ => {
                self.note(format!(
                    "condition `{cond}` is not understood, branch skipped"
                ));
                return None;
            }
        };
        Some(truth != negate)
    }
}

/// The files of one library the configuration names, that exist.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConfigFiles {
    pub gds: Vec<PathBuf>,
    pub spice: Vec<PathBuf>,
    pub cdl: Vec<PathBuf>,
    pub lef: Vec<PathBuf>,
    pub tech_lef: Vec<PathBuf>,
    pub verilog: Vec<PathBuf>,
    /// Corner (the pattern the configuration gives, else what the file name says)
    /// and file.
    pub liberty: Vec<(String, PathBuf)>,
    /// The power and ground pins the configuration says the cells have.
    pub power: Vec<String>,
    pub ground: Vec<String>,
}

impl ConfigFiles {
    /// Every file.
    pub fn files(&self) -> BTreeSet<PathBuf> {
        self.gds
            .iter()
            .chain(&self.spice)
            .chain(&self.cdl)
            .chain(&self.lef)
            .chain(&self.tech_lef)
            .chain(&self.verilog)
            .chain(self.liberty.iter().map(|(_, file)| file))
            .cloned()
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.files().is_empty()
    }
}

/// What the configuration of a PDK says.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Config {
    /// `librelane` or `openlane`; empty when the PDK has no configuration.
    pub flow: String,
    /// Library name → its files.
    pub libraries: BTreeMap<String, ConfigFiles>,
    /// What was not understood or not found.
    pub notes: Vec<String>,
}

/// Accumulates the files of one library over several evaluations.
#[derive(Default)]
struct Gathered {
    gds: BTreeSet<PathBuf>,
    spice: BTreeSet<PathBuf>,
    cdl: BTreeSet<PathBuf>,
    lef: BTreeSet<PathBuf>,
    tech_lef: BTreeSet<PathBuf>,
    verilog: BTreeSet<PathBuf>,
    liberty: BTreeMap<PathBuf, (String, bool)>,
    power: Vec<String>,
    ground: Vec<String>,
}

/// The corner a Liberty file's name says: `sky130_fd_sc_hd__tt_025C_1v80.lib` →
/// `tt_025C_1v80`, `sg13g2_stdcell_typ_1p20V_25C.lib` → `typ_1p20V_25C`.
fn corner_of_file(file: &Path, library: &str) -> String {
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = name.strip_suffix(".lib").unwrap_or(&name);
    if let Some((_, corner)) = stem.rsplit_once("__") {
        return corner.to_string();
    }
    stem.strip_prefix(library)
        .map(|rest| rest.trim_start_matches('_'))
        .filter(|rest| !rest.is_empty())
        .unwrap_or(stem)
        .to_string()
}

/// Evaluate the configuration of a PDK.
pub fn evaluate(pdk: &PdkDir) -> Config {
    let Some((flow, dir)) = ["librelane", "openlane"]
        .into_iter()
        .map(|flow| (flow, pdk.tech(flow)))
        .find(|(_, dir)| dir.join("config.tcl").is_file())
    else {
        return Config::default();
    };
    let config = dir.join("config.tcl");
    let Ok(text) = std::fs::read_to_string(&config) else {
        return Config::default();
    };
    let installed = |name: &str| pdk.refs().join(name).is_dir();

    // The runs: with the PDK's own default library, then one for each library with a
    // configuration of its own, that library being the standard-cell library.
    let mut forced: Vec<Option<String>> = vec![None];
    let mut missing = BTreeSet::new();
    let mut folders: Vec<PathBuf> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|item| item.path())
        .filter(|path| path.join("config.tcl").is_file())
        .collect();
    folders.sort();
    for folder in folders {
        let name = folder
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if installed(&name) {
            forced.push(Some(name));
        } else {
            missing.insert(name);
        }
    }

    let mut notes = BTreeSet::new();
    let mut gathered: BTreeMap<String, Gathered> = BTreeMap::new();
    let mut absent_files: BTreeMap<String, BTreeSet<PathBuf>> = BTreeMap::new();
    for library in forced {
        let mut eval = Evaluator::new([
            (
                "PDK_ROOT".to_string(),
                pdk.root.to_string_lossy().into_owned(),
            ),
            ("PDK".to_string(), pdk.name.clone()),
        ]);
        if let Some(library) = &library {
            eval.env
                .insert("STD_CELL_LIBRARY".into(), Val::Str(library.clone()));
        }
        eval.run(&text, &config);
        // A configuration that does not source the library's own file.
        if let Some(library) = &library {
            let own = dir.join(library).join("config.tcl");
            if !eval.sourced.contains(&own)
                && let Ok(body) = std::fs::read_to_string(&own)
            {
                eval.run(&body, &own);
            }
        }
        // A library made the standard-cell library for a run does not have what only
        // another kind of library has (a technology LEF in the IO library).
        let root = pdk.root.to_string_lossy().into_owned();
        notes.extend(
            eval.notes
                .iter()
                .filter(|n| library.is_none() || !n.contains("matches no file"))
                .map(|n| format!("{flow} configuration: {}", n.replace(&root, "$PDK_ROOT"))),
        );

        let value = |name: &str| eval.env.get(name).map(Val::text);
        let words = |name: &str| -> Vec<String> {
            value(name)
                .map(|v| v.split_whitespace().map(str::to_string).collect())
                .unwrap_or_default()
        };
        let standard = value("STD_CELL_LIBRARY");
        let pads = value("PAD_CELL_LIBRARY");
        let first = |names: &[&str]| {
            names
                .iter()
                .map(|n| words(n))
                .find(|w| !w.is_empty())
                .unwrap_or_default()
        };
        let power = first(&["SCL_POWER_PINS", "STD_CELL_POWER_PINS", "VDD_PIN"]);
        let ground = first(&["SCL_GROUND_PINS", "STD_CELL_GROUND_PINS", "GND_PIN"]);

        let root = format!("{}/", pdk.refs().to_string_lossy());
        for (var, val) in &eval.env {
            let pairs: Vec<(Option<String>, String)> = match val {
                Val::Str(s) => vec![(None, s.clone())],
                Val::Dict(items) => items
                    .iter()
                    .map(|(keys, v)| (keys.first().cloned(), v.clone()))
                    .collect(),
            };
            for (key, value) in pairs {
                for token in value.split_whitespace() {
                    let Some(rest) = token.strip_prefix(&root) else {
                        continue;
                    };
                    let Some((lib, _)) = rest.split_once('/') else {
                        continue;
                    };
                    let path = PathBuf::from(token);
                    let lower = token.to_ascii_lowercase();
                    let lower = lower.strip_suffix(".gz").unwrap_or(&lower);
                    let ext = lower.rsplit('.').next().unwrap_or_default();
                    let kind = match ext {
                        "gds" => "gds",
                        "spice" | "sp" | "spi" | "cir" => "spice",
                        "cdl" => "cdl",
                        "tlef" => "tech_lef",
                        "lef" if var.contains("TECH") || rest.contains("/techlef/") => "tech_lef",
                        "lef" => "lef",
                        "v" | "sv" => "verilog",
                        "lib" => "liberty",
                        _ => continue,
                    };
                    if !installed(lib) {
                        missing.insert(lib.to_string());
                        continue;
                    }
                    if !path.is_file() {
                        absent_files
                            .entry(lib.to_string())
                            .or_default()
                            .insert(path);
                        continue;
                    }
                    let entry = gathered.entry(lib.to_string()).or_default();
                    match kind {
                        "gds" => entry.gds.insert(path),
                        "spice" => entry.spice.insert(path),
                        "cdl" => entry.cdl.insert(path),
                        "tech_lef" => entry.tech_lef.insert(path),
                        "lef" => entry.lef.insert(path),
                        "verilog" => entry.verilog.insert(path),
                        _ => {
                            let named = key.clone();
                            let corner =
                                named.clone().unwrap_or_else(|| corner_of_file(&path, lib));
                            let slot = entry
                                .liberty
                                .entry(path)
                                .or_insert_with(|| (corner.clone(), named.is_some()));
                            if named.is_some() && !slot.1 {
                                *slot = (corner, true);
                            }
                            true
                        }
                    };
                }
            }
        }
        // The pins belong to the standard cells of the run.
        if let Some(lib) = standard.filter(|lib| pads.as_ref() != Some(lib))
            && let Some(entry) = gathered.get_mut(&lib)
            && entry.power.is_empty()
            && (entry.gds.len() + entry.lef.len()) > 0
        {
            entry.power = power;
            entry.ground = ground;
        }
    }

    for (lib, files) in absent_files {
        for file in files {
            notes.insert(format!(
                "refs: {lib}: the {flow} configuration names {}, which is not on disk",
                pdk.portable(&file)
            ));
        }
    }
    if !missing.is_empty() {
        notes.insert(format!(
            "refs: the {flow} configuration names libraries that are not installed: {}",
            missing.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }

    let libraries = gathered
        .into_iter()
        .map(|(name, g)| {
            let files = ConfigFiles {
                gds: g.gds.into_iter().collect(),
                spice: g.spice.into_iter().collect(),
                cdl: g.cdl.into_iter().collect(),
                lef: g.lef.into_iter().collect(),
                tech_lef: g.tech_lef.into_iter().collect(),
                verilog: g.verilog.into_iter().collect(),
                liberty: g
                    .liberty
                    .into_iter()
                    .map(|(file, (corner, _))| (corner, file))
                    .collect(),
                power: g.power,
                ground: g.ground,
            };
            (name, files)
        })
        .collect();
    Config {
        flow: flow.to_string(),
        libraries,
        notes: notes.into_iter().collect(),
    }
}

type CacheKey = (PathBuf, Option<SystemTime>);
static CACHE: Mutex<Vec<(CacheKey, Arc<Config>)>> = Mutex::new(Vec::new());

/// [`evaluate`], remembered while the configuration file stays as it is.
pub fn read(pdk: &PdkDir) -> Arc<Config> {
    let stamp = |flow: &str| {
        std::fs::metadata(pdk.tech(flow).join("config.tcl"))
            .and_then(|m| m.modified())
            .ok()
    };
    let key = (
        pdk.dir(),
        stamp("librelane")
            .into_iter()
            .chain(stamp("openlane"))
            .max(),
    );
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, config)) = cache.iter().find(|(k, _)| *k == key) {
        return config.clone();
    }
    let config = Arc::new(evaluate(pdk));
    cache.retain(|(k, _)| k.0 != key.0);
    cache.push((key, config.clone()));
    config
}

/// The files the configuration names for the library at `path` (a folder of a PDK's
/// `libs.ref`); empty for any other folder or a PDK without configuration.
pub fn library_files(path: &Path) -> BTreeSet<PathBuf> {
    let Some(refs) = path.parent() else {
        return BTreeSet::new();
    };
    if refs.file_name().is_none_or(|n| n != "libs.ref") {
        return BTreeSet::new();
    }
    let Some(pdk) = refs.parent() else {
        return BTreeSet::new();
    };
    let (Some(root), Some(name)) = (pdk.parent(), pdk.file_name()) else {
        return BTreeSet::new();
    };
    let config = read(&PdkDir::new(root, name.to_string_lossy()));
    path.file_name()
        .and_then(|n| config.libraries.get(n.to_string_lossy().as_ref()))
        .map(ConfigFiles::files)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(text: &str, env: &[(&str, &str)]) -> Evaluator {
        let mut e = Evaluator::new(env.iter().map(|(k, v)| (k.to_string(), v.to_string())));
        e.run(text, Path::new("/x/config.tcl"));
        e
    }

    fn get(e: &Evaluator, name: &str) -> String {
        e.env.get(name).map(Val::text).unwrap_or_default()
    }

    #[test]
    fn set_defaults_and_substitution() {
        let e = eval(
            r#"
# comment \
   continued comment
set ::env(A) "$::env(PDK_ROOT)/$::env(PDK)/libs.ref"
if { ![info exist ::env(LIB)] } {
    set ::env(LIB) mylib
}
if { ![info exist ::env(LIB)] } {
    set ::env(LIB) other
}
set ::env(F) "$::env(A)/$::env(LIB)/lef/x.lef"; set ::env(G) {$::env(LIB)}
set ::env(LIST) "\
    a \
    b\
"
"#,
            &[("PDK_ROOT", "/p"), ("PDK", "d")],
        );
        assert_eq!(get(&e, "F"), "/p/d/libs.ref/mylib/lef/x.lef");
        assert_eq!(get(&e, "G"), "$::env(LIB)");
        assert_eq!(
            get(&e, "LIST").split_whitespace().collect::<Vec<_>>(),
            ["a", "b"]
        );
        assert!(e.notes.is_empty(), "{:?}", e.notes);
        // A preset value wins over the default.
        let e = eval(
            "if { ![info exist ::env(LIB)] } { set ::env(LIB) mylib }",
            &[("LIB", "mine")],
        );
        assert_eq!(get(&e, "LIB"), "mine");
    }

    #[test]
    fn dicts() {
        let e = eval(
            r#"
set ::env(CELL_LIBS) [dict create]
dict set ::env(CELL_LIBS) "*_typ" "\
    /p/a_typ.lib\
"
dict set ::env(CELL_LIBS) "*_fast" /p/a_fast.lib
dict set ::env(CELL_LIBS) "*_fast" /p/b_fast.lib
dict set ::env(RC) "*" Metal1 res 1e-3
"#,
            &[],
        );
        let Some(Val::Dict(items)) = e.env.get("CELL_LIBS") else {
            panic!("dict");
        };
        assert_eq!(items.len(), 2);
        assert_eq!(
            items[0],
            (vec!["*_typ".to_string()], "/p/a_typ.lib".to_string())
        );
        assert_eq!(items[1].1, "/p/b_fast.lib");
        let Some(Val::Dict(rc)) = e.env.get("RC") else {
            panic!("dict");
        };
        assert_eq!(rc[0].0, ["*", "Metal1", "res"]);
    }

    #[test]
    fn unknown_things_are_noted_not_fatal() {
        let e = eval(
            "foreach x {a b} { set ::env(Z) $x }\nset ::env(OK) 1\nset ::env(U) $::env(NOPE)\n\
             if {$a > 1} { set ::env(NO) 1 }\nset ::env(G) [glob /definitely/not/here/*.lef]",
            &[],
        );
        assert_eq!(get(&e, "OK"), "1");
        assert_eq!(get(&e, "NO"), "");
        let notes: Vec<&String> = e.notes.iter().collect();
        assert!(notes.iter().any(|n| n.contains("`foreach`")), "{notes:?}");
        assert!(notes.iter().any(|n| n.contains("NOPE")), "{notes:?}");
        assert!(notes.iter().any(|n| n.contains("condition")), "{notes:?}");
        assert!(
            notes.iter().any(|n| n.contains("matches no file")),
            "{notes:?}"
        );
    }

    #[test]
    fn glob_file_exists_and_source() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("lef")).unwrap();
        std::fs::write(root.join("lef/a.lef"), "").unwrap();
        std::fs::write(root.join("lef/b.lef"), "").unwrap();
        std::fs::write(root.join("lef/c.txt"), "").unwrap();
        std::fs::write(
            root.join("sub.tcl"),
            "set ::env(SUB) \"from [file tail [info script]]\"",
        )
        .unwrap();
        let e = eval(
            &format!(
                "set ::env(L) [glob \"{r}/lef/*.lef\"]\n\
                 if {{ [file exists \"{r}/lef/a.lef\"] }} {{ set ::env(Y) yes }} else {{ set ::env(Y) no }}\n\
                 if {{ [file exists \"{r}/none\"] }} {{ set ::env(N) yes }} else {{ set ::env(N) no }}\n\
                 set tmp $::env(Y)\nset ::env(T) ${{tmp}}!\nsource \"{r}/sub.tcl\"",
                r = root.display()
            ),
            &[],
        );
        let listed = get(&e, "L");
        assert_eq!(listed.split_whitespace().count(), 2, "{listed}");
        assert_eq!(get(&e, "Y"), "yes");
        assert_eq!(get(&e, "N"), "no");
        assert_eq!(get(&e, "T"), "yes!");
        assert_eq!(get(&e, "SUB"), "from sub.tcl");
    }

    #[test]
    fn a_pdk_folder_is_read_by_its_files() {
        let dir = tempfile::tempdir().unwrap();
        let pdk = PdkDir::new(dir.path(), "demo");
        let write = |name: &str, text: &str| {
            let file = pdk.dir().join(name);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, text).unwrap();
        };
        for f in [
            "std/gds/std.gds",
            "std/lef/std.lef",
            "std/lef/std_tech.lef",
            "std/spice/std.spice",
            "std/lib/std_typ_1p20V_25C.lib",
            "io/gds/io.gds",
        ] {
            write(&format!("libs.ref/{f}"), "");
        }
        write(
            "libs.tech/librelane/config.tcl",
            r#"
set ::env(SCL_POWER_PINS) "VDD"
set ::env(SCL_GROUND_PINS) "VSS"
if { ![info exist ::env(STD_CELL_LIBRARY)] } { set ::env(STD_CELL_LIBRARY) std }
if { ![info exist ::env(PAD_CELL_LIBRARY)] } { set ::env(PAD_CELL_LIBRARY) io }
set R "$::env(PDK_ROOT)/$::env(PDK)/libs.ref"
set ::env(TECH_LEFS) [dict create]
dict set ::env(TECH_LEFS) "*" [glob "${R}/$::env(STD_CELL_LIBRARY)/lef/std_tech.lef"]
set ::env(CELL_LEFS) "${R}/$::env(STD_CELL_LIBRARY)/lef/$::env(STD_CELL_LIBRARY).lef"
set ::env(CELL_GDS) "${R}/$::env(STD_CELL_LIBRARY)/gds/$::env(STD_CELL_LIBRARY).gds"
set ::env(CELL_SPICE_MODELS) "${R}/$::env(STD_CELL_LIBRARY)/spice/$::env(STD_CELL_LIBRARY).spice"
set ::env(CELL_CDLS) "${R}/$::env(STD_CELL_LIBRARY)/cdl/$::env(STD_CELL_LIBRARY).cdl"
set ::env(PAD_GDS) "${R}/io/gds/io.gds"
"#,
        );
        write(
            "libs.tech/librelane/std/config.tcl",
            r#"
set ::env(CELL_LIBS) [dict create]
dict set ::env(CELL_LIBS) "*_typ_1p20V_25C" "$::env(PDK_ROOT)/$::env(PDK)/libs.ref/$::env(STD_CELL_LIBRARY)/lib/std_typ_1p20V_25C.lib"
"#,
        );
        let config = evaluate(&pdk);
        assert_eq!(config.flow, "librelane");
        let std = &config.libraries["std"];
        assert_eq!(std.gds, [pdk.refs().join("std/gds/std.gds")]);
        assert_eq!(std.lef, [pdk.refs().join("std/lef/std.lef")]);
        assert_eq!(std.tech_lef, [pdk.refs().join("std/lef/std_tech.lef")]);
        assert_eq!(std.spice.len(), 1);
        assert_eq!(std.liberty[0].0, "*_typ_1p20V_25C");
        assert_eq!(std.power, ["VDD"]);
        assert!(std.cdl.is_empty());
        assert_eq!(
            config.libraries["io"].gds,
            [pdk.refs().join("io/gds/io.gds")]
        );
        assert!(config.libraries["io"].power.is_empty());
        // The missing CDL file is said.
        assert!(
            config
                .notes
                .iter()
                .any(|n| n.contains("std.cdl") && n.contains("not on disk")),
            "{:?}",
            config.notes
        );
        // Through the cache, by the library's folder.
        let preferred = library_files(&pdk.refs().join("std"));
        assert!(preferred.contains(&pdk.refs().join("std/gds/std.gds")));
        assert!(library_files(&pdk.refs().join("nothing")).is_empty());
    }

    #[test]
    fn corner_names_of_files() {
        let f = |name: &str, lib: &str| corner_of_file(Path::new(name), lib);
        assert_eq!(
            f("/a/sky130_fd_sc_hd__tt_025C_1v80.lib", "sky130_fd_sc_hd"),
            "tt_025C_1v80"
        );
        assert_eq!(
            f("/a/sg13g2_stdcell_typ_1p20V_25C.lib", "sg13g2_stdcell"),
            "typ_1p20V_25C"
        );
    }
}
