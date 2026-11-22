//! xschem section of the technology snapshot: `libs.tech/xschem`.
//!
//! The PDK's `xschemrc` is a Tcl program: what it sets cannot be read off its text in
//! general. When the scan may start helper programs and the xschem helper is built,
//! the file is evaluated in the engine and the snapshot keeps what changed: the
//! symbol search path, the variables that point into the PDK (the model folders the
//! symbols netlist with), the Tcl files it loads. Without the helper the same is read
//! from the text, tolerantly, and a note says so.
//!
//! `[xschem]` in `tech.toml` ([`XschemTech`]) has the start-up file, the libraries
//! found on the search path (primitives, standard cells; tests and examples are
//! found but not enabled) and the variables. The catalog ([`XschemCatalog`]) has
//! every symbol of the enabled libraries: its type, netlist formats, parameters with
//! defaults, model, pins.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::pdk::{PdkDir, ScanContext, Scanned, TechLibrary, TechPath};
use crate::xschem::symbol;

/// `[xschem]` in `tech.toml`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct XschemTech {
    /// The PDK's start-up file (`xschemrc`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rc: Option<TechPath>,
    /// The start-up file was evaluated by xschem (else: read from its text).
    pub evaluated: bool,
    /// The schematic the PDK opens first (its start page), a good place to look
    /// around.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_window: Option<TechPath>,
    /// Where the start-up file sends netlists and simulation results (`$PWD` is the
    /// folder xschem was started in).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub netlist_dir: Option<String>,
    /// The folders of the PDK the start-up file puts on xschem's symbol search path,
    /// in order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub library_path: Vec<TechPath>,
    /// Tcl files the start-up file loads (DRC procedures, menus, back-annotation).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tcl_files: Vec<TechPath>,
    /// Symbols that put model libraries into the netlist (sky130's `corner.sym`), as
    /// a schematic refers to them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub model_symbols: Vec<String>,
    /// Global variables of the start-up file that point into the PDK, e.g. the model
    /// folders the symbols and testbenches name (`SKYWATER_MODELS`, `SG13G2_MODELS`).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub variables: BTreeMap<String, String>,
    /// Per library, what a schematic writes before a symbol's file name with the
    /// PDK's search path: `sg13g2_pr/` or nothing.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub references: BTreeMap<String, String>,
    /// The symbol libraries the PDK brings.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub libraries: Vec<TechLibrary>,
}

/// The xschem part of the catalog.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct XschemCatalog {
    /// The symbols of the enabled libraries.
    pub symbols: Vec<Symbol>,
    /// Symbols per library, the libraries that are not enabled included.
    pub counts: BTreeMap<String, usize>,
    /// Other plain settings of the start-up file (`autotrim_wires`, …).
    pub settings: BTreeMap<String, String>,
    /// The simulator commands, when the start-up file sets them (xschem's `sim`
    /// array).
    pub simulators: BTreeMap<String, String>,
}

/// A symbol of one of the PDK's libraries.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Symbol {
    /// The library it is in ([`TechLibrary::name`]).
    pub library: String,
    /// The symbol name without `.sym`.
    pub name: String,
    /// xschem's `type` attribute: `nmos`, `pmos`, `resistor`, `subcircuit`, …
    pub kind: String,
    /// The model or subcircuit it netlists as, when the symbol says one.
    pub model: String,
    /// The SPICE `format` attribute.
    pub format: String,
    /// The `template` attribute: parameter name → default.
    pub template: BTreeMap<String, String>,
    /// Pin names in order.
    pub pins: Vec<String>,
    /// Pin directions (`in`, `out`, `inout`), like `pins`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub directions: Vec<String>,
    /// The parameters in the order of the template (`name` left out).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub parameters: Vec<String>,
    /// The first letters of the netlist line (`X`, `M`, `R`).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub prefix: String,
    /// The `lvs_format` attribute.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub lvs_format: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub verilog_format: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub vhdl_format: String,
    /// The symbol writes model libraries into the netlist: its `.lib`/`.include`
    /// lines as the format has them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub includes: Vec<String>,
    /// There is a schematic of the same name next to it.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub schematic: bool,
}

// ---------------------------------------------------------------------------
// Portable paths
// ---------------------------------------------------------------------------

/// Turns paths of this machine into the snapshot's `$PDK_ROOT/<pdk>/…`, also when a
/// tool followed a symbolic link (ciel installs a PDK as a link into its versions).
#[derive(Debug, Clone)]
pub(crate) struct Portable {
    pub pdk: PdkDir,
    /// (prefix on this machine, what it becomes), longest first.
    prefixes: Vec<(String, String)>,
}

impl Portable {
    pub fn new(pdk: &PdkDir) -> Portable {
        let dir_text = format!("$PDK_ROOT/{}", pdk.name);
        let mut prefixes: Vec<(String, String)> = Vec::new();
        let mut add = |path: PathBuf, to: &str| {
            let text = path.to_string_lossy().trim_end_matches('/').to_string();
            if !text.is_empty() && !prefixes.iter().any(|(p, _)| *p == text) {
                prefixes.push((text, to.to_string()));
            }
        };
        add(pdk.dir(), &dir_text);
        if let Ok(real) = pdk.dir().canonicalize() {
            add(real, &dir_text);
        }
        add(pdk.root.clone(), "$PDK_ROOT");
        if let Ok(real) = pdk.root.canonicalize() {
            add(real, "$PDK_ROOT");
        }
        // The PDK folder before the root that contains it.
        prefixes.sort_by(|a, b| {
            let rank = |to: &str| usize::from(to == "$PDK_ROOT");
            rank(&a.1).cmp(&rank(&b.1)).then(b.0.len().cmp(&a.0.len()))
        });
        Portable {
            pdk: pdk.clone(),
            prefixes,
        }
    }

    /// A path as the snapshot stores it.
    pub fn path(&self, path: &Path) -> TechPath {
        TechPath(self.text(&path.to_string_lossy()))
    }

    /// A text with every path below the PDK root written with `$PDK_ROOT`.
    pub fn text(&self, text: &str) -> String {
        let mut out = text.to_string();
        for (from, to) in &self.prefixes {
            out = replace_path(&out, from, to);
        }
        out
    }

    /// The path is inside the PDK root.
    pub fn inside(&self, path: &Path) -> bool {
        self.text(&path.to_string_lossy()).starts_with("$PDK_ROOT")
    }
}

/// Replace `from` by `to` where `from` is a whole path prefix: at the start of the
/// text or after a separator, and followed by `/` or the end of a path.
fn replace_path(text: &str, from: &str, to: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(from) {
        let before_ok = rest[..at]
            .chars()
            .next_back()
            .is_none_or(|c| !(c.is_alphanumeric() || matches!(c, '/' | '_' | '-' | '.' | '$')));
        let after = &rest[at + from.len()..];
        let after_ok = after
            .chars()
            .next()
            .is_none_or(|c| !(c.is_alphanumeric() || matches!(c, '_' | '-' | '.')));
        out.push_str(&rest[..at]);
        out.push_str(if before_ok && after_ok { to } else { from });
        rest = after;
    }
    out.push_str(rest);
    out
}

// ---------------------------------------------------------------------------
// The start-up file
// ---------------------------------------------------------------------------

/// What the PDK's `xschemrc` sets, with the paths of this machine.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct RcValues {
    /// The file, when the PDK has one.
    pub file: Option<PathBuf>,
    pub evaluated: bool,
    /// Every plain global variable the file sets.
    pub variables: BTreeMap<String, String>,
    /// `XSCHEM_LIBRARY_PATH` in order.
    pub library_path: Vec<String>,
    /// The files it sources or lists in `tcl_files`.
    pub tcl_files: Vec<String>,
    /// xschem's `sim` array, when the file sets it.
    pub simulators: BTreeMap<String, String>,
    /// xschem's own share folder (`$XSCHEM_SHAREDIR`), when known.
    pub sharedir: Option<String>,
    /// The folder the file was evaluated in (what `$PWD` was).
    pub cwd: Option<String>,
    pub error: Option<String>,
}

/// The PDK's start-up file.
pub(crate) fn find_rc(pdk: &PdkDir) -> Option<PathBuf> {
    let dir = pdk.tech("xschem");
    ["xschemrc", ".xschemrc"]
        .into_iter()
        .map(|name| dir.join(name))
        .find(|file| file.is_file())
}

/// Read the start-up file from its text: `set`, `append XSCHEM_LIBRARY_PATH`,
/// `lappend tcl_files` and `source` lines outside procedures, with `$PDK_ROOT`,
/// `$PDK` and the variables set before replaced. Conditions are not evaluated: every
/// branch counts.
pub(crate) fn read_rc_text(pdk: &PdkDir) -> RcValues {
    let mut out = RcValues {
        file: find_rc(pdk),
        ..RcValues::default()
    };
    let Some(file) = out.file.clone() else {
        return out;
    };
    match std::fs::read_to_string(&file) {
        Ok(text) => parse_rc_text(&text, &file, pdk, &mut out),
        Err(e) => out.error = Some(format!("cannot read {}: {e}", file.display())),
    }
    out
}

fn parse_rc_text(text: &str, file: &Path, pdk: &PdkDir, out: &mut RcValues) {
    let rc_dir = file
        .parent()
        .map(|d| d.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut vars: BTreeMap<String, String> = BTreeMap::new();
    vars.insert("PDK_ROOT".into(), pdk.root.to_string_lossy().into_owned());
    vars.insert("PDK".into(), pdk.name.clone());
    let mut proc_depth: i32 = 0;
    for line in text.lines() {
        let line = line.trim();
        if proc_depth > 0 {
            proc_depth += braces(line);
            continue;
        }
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with("proc ") {
            proc_depth = braces(line).max(0);
            continue;
        }
        let line = line.trim_start_matches(['}', ' ']);
        let mut words = line.splitn(3, char::is_whitespace);
        let (Some(command), Some(first)) = (words.next(), words.next()) else {
            continue;
        };
        let rest = words.next().unwrap_or_default();
        match command {
            "set" => {
                let name = first.trim_start_matches("::");
                if name.contains('(') || matches!(name, "PDK_ROOT" | "PDK") || rest.is_empty() {
                    continue;
                }
                let value = substitute(&tcl_word(rest), &vars, &rc_dir);
                if value.contains('[') {
                    continue;
                }
                vars.insert(name.to_string(), value.clone());
                out.variables.insert(name.to_string(), value);
            }
            "append" if first.trim_start_matches("::") == "XSCHEM_LIBRARY_PATH" => {
                let value = substitute(&tcl_word(rest), &vars, &rc_dir);
                for dir in value.split(':').map(str::trim).filter(|d| !d.is_empty()) {
                    if !dir.contains('[') && !out.library_path.iter().any(|d| d == dir) {
                        out.library_path.push(dir.to_string());
                    }
                }
            }
            "lappend" if first.trim_start_matches("::") == "tcl_files" => {
                let value = substitute(&tcl_word(rest), &vars, &rc_dir);
                if !value.contains('[') && !out.tcl_files.contains(&value) {
                    out.tcl_files.push(value);
                }
            }
            "source" => {
                let value = substitute(&tcl_word(first), &vars, &rc_dir);
                if !value.contains('[') && !out.tcl_files.contains(&value) {
                    out.tcl_files.push(value);
                }
            }
            _ => {}
        }
    }
    out.variables.remove("XSCHEM_LIBRARY_PATH");
    out.variables.remove("tcl_files");
}

/// The first word of a Tcl text: a `{…}` group, a `"…"` string or up to the next
/// blank (a `[…]` command counts as part of the word).
fn tcl_word(text: &str) -> String {
    let text = text.trim();
    if let Some(inner) = text.strip_prefix('{') {
        let mut depth = 1;
        for (i, c) in inner.char_indices() {
            match c {
                '{' => depth += 1,
                '}' => depth -= 1,
                _ => {}
            }
            if depth == 0 {
                return inner[..i].trim().to_string();
            }
        }
        return inner.trim().to_string();
    }
    if let Some(inner) = text.strip_prefix('"') {
        return inner.split('"').next().unwrap_or_default().to_string();
    }
    let mut depth = 0;
    for (i, c) in text.char_indices() {
        match c {
            '[' => depth += 1,
            ']' => depth -= 1,
            c if c.is_whitespace() && depth <= 0 => return text[..i].to_string(),
            _ => {}
        }
    }
    text.to_string()
}

/// Opening minus closing braces of a line.
fn braces(line: &str) -> i32 {
    let mut depth = 0;
    let mut escaped = false;
    for c in line.chars() {
        match c {
            _ if escaped => escaped = false,
            '\\' => escaped = true,
            '{' => depth += 1,
            '}' => depth -= 1,
            _ => {}
        }
    }
    depth
}

/// Replace `$NAME`, `${NAME}`, `$::NAME`, `$env(NAME)` and the usual "folder of this
/// script" expressions in a Tcl value. Unknown variables stay as written.
fn substitute(value: &str, vars: &BTreeMap<String, String>, rc_dir: &str) -> String {
    let mut text = value.to_string();
    for script_dir in [
        "[file dirname [file normalize [info script]]]",
        "[file dirname [info script]]",
        "[file normalize [file dirname [info script]]]",
    ] {
        text = text.replace(script_dir, rc_dir);
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text.as_str();
    while let Some(at) = rest.find('$') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let ident = |s: &str| -> usize {
            s.char_indices()
                .find(|(_, c)| !(c.is_ascii_alphanumeric() || *c == '_'))
                .map_or(s.len(), |(i, _)| i)
        };
        // (name, bytes used after the `$`, environment variable)
        let (name, used, env) = if let Some(inner) = after.strip_prefix('{') {
            let end = inner.find('}').unwrap_or(inner.len());
            (&inner[..end], end + 2, false)
        } else {
            let colons = if after.starts_with("::") { 2 } else { 0 };
            let body = &after[colons..];
            if let Some(inner) = body.strip_prefix("env(") {
                let end = inner.find(')').unwrap_or(inner.len());
                (&inner[..end], colons + 4 + end + 1, true)
            } else {
                let n = ident(body);
                (&body[..n], colons + n, false)
            }
        };
        let name = name.trim_start_matches("::");
        let used = used.min(after.len());
        let known = match (env, name) {
            (_, "PDK_ROOT") | (_, "PDK") => vars.get(name).cloned(),
            (true, "PWD") => Some("$PWD".into()),
            (true, "HOME") => Some("$HOME".into()),
            (true, _) => None,
            (false, "") => None,
            (false, _) => vars.get(name).cloned(),
        };
        match known {
            Some(v) => out.push_str(&v),
            None => out.push_str(&rest[at..at + 1 + used]),
        }
        rest = &after[used..];
    }
    out.push_str(rest);
    out
}

/// The Tcl that evaluates a start-up file in the engine and reports what it changed:
/// a list of the result code, the error text, the changed plain globals (name value
/// …), the files it sourced, `XSCHEM_LIBRARY_PATH`, `tcl_files`, `XSCHEM_SHAREDIR`
/// and the `sim` array.
#[cfg(unix)]
fn rc_script(rc: &Path) -> String {
    let file = crate::xschem::tcl::quote(&rc.to_string_lossy());
    format!(
        r#"
proc ::cdept_tech_globals {{}} {{
  set out {{}}
  foreach v [lsort [info globals]] {{
    if {{[string match cdept_tech_* $v]}} continue
    if {{[array exists ::$v] || ![info exists ::$v]}} continue
    lappend out $v [set ::$v]
  }}
  return $out
}}
array set ::cdept_tech_before [::cdept_tech_globals]
set ::cdept_tech_had_sim [array exists ::sim]
set ::cdept_tech_sourced {{}}
rename source ::cdept_tech_source
proc source args {{
  lappend ::cdept_tech_sourced [lindex $args end]
  uplevel 1 [list ::cdept_tech_source {{*}}$args]
}}
set ::cdept_tech_code [catch {{uplevel #0 [list source {file}]}} ::cdept_tech_error]
rename source {{}}
rename ::cdept_tech_source source
set ::cdept_tech_changed {{}}
foreach {{k v}} [::cdept_tech_globals] {{
  if {{![info exists ::cdept_tech_before($k)] || $::cdept_tech_before($k) ne $v}} {{
    lappend ::cdept_tech_changed $k $v
  }}
}}
set ::cdept_tech_get {{name {{expr {{[info exists ::$name] ? [set ::$name] : {{}}}}}}}}
list $::cdept_tech_code $::cdept_tech_error $::cdept_tech_changed $::cdept_tech_sourced \
  [apply $::cdept_tech_get XSCHEM_LIBRARY_PATH] [apply $::cdept_tech_get tcl_files] \
  [apply $::cdept_tech_get XSCHEM_SHAREDIR] \
  [expr {{!$::cdept_tech_had_sim && [array exists ::sim] ? [array get ::sim] : {{}}}}]
"#
    )
}

/// Evaluate the start-up file in the xschem engine.
#[cfg(unix)]
fn evaluate_rc(ctx: &ScanContext, rc: &Path) -> Result<RcValues, String> {
    use crate::xschem::{Engine, EngineOptions, tcl};

    if Engine::find_helper().is_none() {
        return Err("the xschem helper (cdept-xschem) was not found".into());
    }
    // An empty folder of its own: `$PWD` in the file is then recognisable.
    let cwd = std::env::temp_dir().join(format!(
        "cdept-tech-{}-{}",
        std::process::id(),
        ctx.pdk
            .name
            .replace(|c: char| !c.is_ascii_alphanumeric(), "_")
    ));
    std::fs::create_dir_all(&cwd).map_err(|e| format!("cannot create {}: {e}", cwd.display()))?;
    let mut env = ctx.env.clone();
    env.push(("PWD".into(), cwd.clone().into_os_string()));
    let result = (|| {
        let mut engine = Engine::start(EngineOptions {
            cwd: Some(cwd.clone()),
            env,
            ..EngineOptions::default()
        })
        .map_err(|e| format!("{e:#}"))?;
        let reply = engine.tcl(&rc_script(rc)).map_err(|e| format!("{e:#}"));
        let _ = engine.shutdown();
        let fields = tcl::parse_list(&reply?)?;
        let [
            code,
            error,
            changed,
            sourced,
            path,
            tcl_files,
            sharedir,
            sim,
        ] = fields.as_slice()
        else {
            return Err(format!("unexpected reply ({} fields)", fields.len()));
        };
        let pairs = |list: &str| -> Result<BTreeMap<String, String>, String> {
            let items = tcl::parse_list(list)?;
            Ok(items
                .as_chunks::<2>()
                .0
                .iter()
                .map(|[name, value]| (name.clone(), value.clone()))
                .collect())
        };
        let mut out = RcValues {
            file: Some(rc.to_path_buf()),
            evaluated: true,
            variables: pairs(changed)?,
            library_path: path
                .split(':')
                .map(str::trim)
                .filter(|d| !d.is_empty())
                .map(str::to_string)
                .collect(),
            tcl_files: tcl::parse_list(sourced)?,
            simulators: pairs(sim)?,
            sharedir: (!sharedir.is_empty()).then(|| sharedir.clone()),
            cwd: Some(cwd.to_string_lossy().into_owned()),
            error: (code != "0").then(|| error.trim().to_string()),
        };
        for file in tcl::parse_list(tcl_files)? {
            if !out.tcl_files.contains(&file) {
                out.tcl_files.push(file);
            }
        }
        // The file itself is not one it loads.
        let rc_text = rc.to_string_lossy();
        out.tcl_files.retain(|f| *f != rc_text);
        Ok(out)
    })();
    let _ = std::fs::remove_dir_all(&cwd);
    result
}

#[cfg(not(unix))]
fn evaluate_rc(_ctx: &ScanContext, _rc: &Path) -> Result<RcValues, String> {
    Err("the xschem helper runs on Unix only".into())
}

// ---------------------------------------------------------------------------
// Libraries and symbols
// ---------------------------------------------------------------------------

fn files_with(dir: &Path, suffix: &str) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == suffix))
        .collect();
    files.sort();
    files
}

fn subfolders(dir: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    dirs
}

/// What a library is for, from its name.
pub fn role_of(name: &str) -> &'static str {
    let lower = name.to_ascii_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| lower.contains(w));
    if has(&["test", "bench"]) {
        "tests"
    } else if lower.ends_with("_pr") || has(&["_fd_pr", "prim"]) {
        "primitives"
    } else if has(&["example", "mips", "import", "demo", "macro", "tutorial"]) {
        "examples"
    } else if has(&["stdcell", "_sc_"]) {
        "stdcells"
    } else if has(&["_io", "pad"]) {
        "io"
    } else if has(&["sram", "mem"]) {
        "memory"
    } else {
        "other"
    }
}

/// The name of a library folder: for a folder of symbols inside a reference library
/// (`libs.ref/<library>/sym/xschem`) the reference library's name.
fn library_name(dir: &Path) -> String {
    let parts: Vec<String> = dir
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    if let Some(at) = parts.iter().position(|p| p == "libs.ref")
        && let Some(name) = parts.get(at + 1)
    {
        return name.clone();
    }
    parts.last().cloned().unwrap_or_default()
}

/// The libraries on the PDK's part of the search path: a folder with symbols is a
/// library, and so is every folder with symbols or schematics directly below a
/// folder of the path. Returns the libraries with their folders and what a
/// schematic writes before a symbol's file name.
fn find_libraries(
    path: &[PathBuf],
    portable: &Portable,
) -> (Vec<(TechLibrary, PathBuf)>, BTreeMap<String, String>) {
    let mut found: Vec<(TechLibrary, PathBuf)> = Vec::new();
    let mut references: BTreeMap<String, String> = BTreeMap::new();
    let mut add = |dir: &Path, prefix: String, found: &mut Vec<(TechLibrary, PathBuf)>| {
        let real = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
        if let Some((library, _)) = found
            .iter()
            .find(|(_, d)| d.canonicalize().unwrap_or_else(|_| d.clone()) == real)
        {
            // Seen through its parent and by itself: both ways of naming work; the
            // one with the folder is what CDePT's own libraries use.
            if !prefix.is_empty() {
                references.insert(library.name.clone(), prefix);
            }
            return;
        }
        let name = library_name(dir);
        if found.iter().any(|(l, _)| l.name == name) {
            return;
        }
        let role = role_of(&name);
        let symbols = files_with(dir, "sym").len();
        let schematics = files_with(dir, "sch").len();
        let what = match role {
            "primitives" => "device symbols",
            "stdcells" => "standard cell symbols",
            "tests" => "test benches of the PDK",
            "examples" => "examples of the PDK",
            _ => "symbols",
        };
        references.insert(name.clone(), prefix);
        found.push((
            TechLibrary {
                name,
                path: portable.path(dir),
                layout: "flat".into(),
                role: role.into(),
                description: Some(format!(
                    "xschem: {what} ({symbols} symbols, {schematics} schematics)"
                )),
                enabled: !matches!(role, "tests" | "examples"),
            },
            dir.to_path_buf(),
        ));
    };
    for entry in path {
        if !entry.is_dir() {
            continue;
        }
        if !files_with(entry, "sym").is_empty() {
            add(entry, String::new(), &mut found);
        }
        for dir in subfolders(entry) {
            if files_with(&dir, "sym").is_empty() && files_with(&dir, "sch").is_empty() {
                continue;
            }
            let folder = dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            add(&dir, format!("{folder}/"), &mut found);
        }
    }
    (found, references)
}

/// The attribute groups of the pins of a symbol: the `{…}` of every `B 5 …` record
/// (a group may run over several lines).
fn pin_groups(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut line_start = 0;
    while line_start < bytes.len() {
        let line_end = text[line_start..]
            .find('\n')
            .map_or(bytes.len(), |n| line_start + n);
        let line = &text[line_start..line_end];
        line_start = line_end + 1;
        if !line.starts_with("B 5 ") {
            continue;
        }
        let Some(open) = line.find('{') else { continue };
        let start = line_end - line.len() + open + 1;
        let mut end = start;
        while end < bytes.len() && bytes[end] != b'}' {
            end += if bytes[end] == b'\\' { 2 } else { 1 };
        }
        let end = end.min(bytes.len());
        out.push(symbol::unescape(&text[start..end]));
        line_start = line_start.max(end + 1);
    }
    out
}

/// The model or subcircuit name a netlist line carries: the word after the pins,
/// with the template's values put in.
fn netlist_model(format: &str, template: &BTreeMap<String, String>, symname: &str) -> String {
    let words: Vec<&str> = format.split_whitespace().collect();
    // After `@pinlist`, or after the last explicitly listed pin (`@@D`).
    let after_pins = words
        .iter()
        .position(|w| *w == "@pinlist")
        .or_else(|| words.iter().rposition(|w| w.starts_with("@@")))
        .map(|i| i + 1);
    let Some(start) = after_pins else {
        return String::new();
    };
    // Between the pins and the parameters: extra nodes (`@body`), then the model.
    // The word that says `@model` or `@symname`, else the last one.
    let candidates: Vec<&str> = words[start.min(words.len())..]
        .iter()
        .copied()
        .take_while(|w| !w.contains('=') && !w.starts_with('+'))
        .collect();
    let Some(word) = candidates
        .iter()
        .find(|w| w.contains("@model") || w.contains("@symname"))
        .or(candidates.last())
    else {
        return String::new();
    };
    // Put the attributes in, longest names first (`@model` before `@m`).
    let mut names: Vec<(&str, &str)> = template
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    names.push(("symname", symname));
    names.sort_by_key(|(k, _)| std::cmp::Reverse(k.len()));
    let mut out = word.to_string();
    for (name, value) in names {
        out = out.replace(&format!("@{name}"), value);
    }
    let ok = out
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && out
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '+' | '-'));
    if ok { out } else { String::new() }
}

/// Read a symbol file.
pub fn parse_symbol(text: &str, library: &str, name: &str) -> Symbol {
    let global = symbol::global_attributes(text).unwrap_or_default();
    let items: BTreeMap<String, String> = symbol::attribute_items(&global).into_iter().collect();
    let get = |key: &str| items.get(key).cloned().unwrap_or_default();
    let template_items = items
        .get("template")
        .map(|t| symbol::attribute_items(t))
        .unwrap_or_default();
    let template: BTreeMap<String, String> = template_items.iter().cloned().collect();
    let mut parameters: Vec<String> = Vec::new();
    for (key, _) in &template_items {
        if key != "name" && !parameters.contains(key) {
            parameters.push(key.clone());
        }
    }
    let format = get("format");
    let lvs_format = get("lvs_format");
    let mut pins = Vec::new();
    let mut directions = Vec::new();
    for group in pin_groups(text) {
        let attrs: BTreeMap<String, String> = symbol::attribute_items(&group).into_iter().collect();
        if let Some(pin) = attrs.get("name").filter(|n| !n.is_empty()) {
            pins.push(pin.clone());
            directions.push(attrs.get("dir").cloned().unwrap_or_default());
        }
    }
    if directions.iter().all(String::is_empty) {
        directions.clear();
    }
    let line = if format.is_empty() {
        &lvs_format
    } else {
        &format
    };
    let mut model = netlist_model(line, &template, name);
    if model.is_empty() {
        model = netlist_model(&lvs_format, &template, name);
    }
    if model.is_empty() {
        model = template.get("model").cloned().unwrap_or_default();
    }
    // `@spiceprefix@name …`, or a literal first letter.
    let prefix = match template.get("spiceprefix") {
        Some(prefix) if line.contains("@spiceprefix") => prefix.clone(),
        _ => line
            .trim_start_matches("tcleval(")
            .trim_start()
            .split("@name")
            .next()
            .filter(|_| line.contains("@name"))
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphabetic()))
            .unwrap_or_default(),
    };
    let includes: Vec<String> = format
        .lines()
        .map(|l| l.trim().trim_start_matches("tcleval(").trim())
        .filter(|l| {
            let lower = l.to_ascii_lowercase();
            lower.starts_with(".lib ")
                || lower.starts_with(".include ")
                || lower.starts_with(".inc ")
        })
        .map(|l| l.trim_end_matches(')').trim().to_string())
        .collect();
    Symbol {
        library: library.to_string(),
        name: name.to_string(),
        kind: get("type"),
        model,
        format,
        template,
        pins,
        directions,
        parameters,
        prefix,
        lvs_format,
        verilog_format: get("verilog_format"),
        vhdl_format: get("vhdl_format"),
        includes,
        schematic: false,
    }
}

fn read_symbols(library: &TechLibrary, dir: &Path) -> Vec<Symbol> {
    files_with(dir, "sym")
        .into_iter()
        .filter_map(|file| {
            let name = file.file_stem()?.to_string_lossy().into_owned();
            let text = std::fs::read_to_string(&file).ok()?;
            let mut symbol = parse_symbol(&text, &library.name, &name);
            symbol.schematic = file.with_extension("sch").is_file();
            Some(symbol)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// The scan
// ---------------------------------------------------------------------------

pub fn scan(ctx: &ScanContext) -> Scanned<XschemTech, XschemCatalog> {
    let mut out: Scanned<XschemTech, XschemCatalog> = Scanned::default();
    let pdk = &ctx.pdk;
    let portable = Portable::new(pdk);
    let xschem_dir = pdk.tech("xschem");
    if !xschem_dir.is_dir() {
        out.notes
            .push("xschem: the PDK has no libs.tech/xschem folder".into());
        return out;
    }

    // The start-up file: evaluated when possible, else read.
    let rc = find_rc(pdk);
    let values = match &rc {
        None => {
            out.notes.push(
                "xschem: no xschemrc in libs.tech/xschem; libraries taken from the folder".into(),
            );
            RcValues::default()
        }
        Some(file) => {
            let evaluated = if ctx.helpers {
                evaluate_rc(ctx, file)
            } else {
                Err("the scan read files only".into())
            };
            match evaluated {
                Ok(values) => {
                    if let Some(error) = &values.error {
                        out.notes.push(format!(
                            "xschem: the xschemrc stopped with an error, what it set before is \
                             kept: {}",
                            error.lines().next().unwrap_or_default()
                        ));
                    }
                    values
                }
                Err(reason) => {
                    out.notes.push(format!(
                        "xschem: the xschemrc was read as text, not evaluated ({reason}): \
                         conditions and computed values are not followed"
                    ));
                    read_rc_text(pdk)
                }
            }
        }
    };
    let section = &mut out.section;
    section.rc = rc.as_deref().map(|f| portable.path(f));
    section.evaluated = values.evaluated;

    // Machine-specific prefixes besides the PDK: xschem's own share folder and the
    // folder the file was evaluated in.
    let generic = |text: &str| -> String {
        let mut text = portable.text(text);
        if let Some(sharedir) = &values.sharedir {
            text = replace_path(&text, sharedir, "$XSCHEM_SHAREDIR");
        }
        if let Some(cwd) = &values.cwd {
            text = replace_path(&text, cwd, "$PWD");
        }
        text
    };

    for (name, value) in &values.variables {
        let text = generic(value);
        match name.as_str() {
            "XSCHEM_LIBRARY_PATH" | "pathlist" | "PDK_ROOT" | "PDK" | "tcl_files" => {}
            "XSCHEM_START_WINDOW" => {
                // Relative: below one of the folders of the search path.
                let file = if Path::new(value).is_absolute() {
                    Some(PathBuf::from(value))
                } else {
                    values
                        .library_path
                        .iter()
                        .map(|dir| Path::new(dir).join(value))
                        .find(|f| f.is_file())
                };
                section.start_window = file.map(|f| portable.path(&f));
            }
            "netlist_dir" => section.netlist_dir = Some(text),
            _ if text.contains("$PDK_ROOT") && text.len() < 400 => {
                section.variables.insert(name.clone(), text);
            }
            _ if !value.contains('\n') && value.len() < 120 && !value.is_empty() => {
                out.catalog.settings.insert(name.clone(), text);
            }
            _ => {}
        }
    }
    out.catalog.simulators = values
        .simulators
        .iter()
        .map(|(k, v)| (k.clone(), generic(v)))
        .collect();
    for file in &values.tcl_files {
        let path = TechPath(generic(file));
        // Only what the PDK or xschem bring, not this user's own files.
        if !path.as_str().starts_with('$') {
            continue;
        }
        if !section.tcl_files.contains(&path) {
            section.tcl_files.push(path);
        }
    }

    // The PDK's part of the search path, and the libraries on it.
    let mut path: Vec<PathBuf> = values
        .library_path
        .iter()
        .map(PathBuf::from)
        .filter(|dir| portable.inside(dir))
        .collect();
    if path.is_empty() {
        path.push(xschem_dir.clone());
    }
    section.library_path = path.iter().map(|dir| portable.path(dir)).collect();
    let (libraries, references) = find_libraries(&path, &portable);
    section.references = references;
    if libraries.is_empty() {
        out.notes
            .push("xschem: no symbol library found on the PDK's search path".into());
    }
    for (library, dir) in &libraries {
        out.catalog
            .counts
            .insert(library.name.clone(), files_with(dir, "sym").len());
        if !library.enabled {
            continue;
        }
        let symbols = read_symbols(library, dir);
        let prefix = &section.references[&library.name];
        for symbol in symbols.iter().filter(|s| !s.includes.is_empty()) {
            section
                .model_symbols
                .push(format!("{prefix}{}.sym", symbol.name));
        }
        out.catalog.symbols.extend(symbols);
    }
    section.libraries = libraries.into_iter().map(|(library, _)| library).collect();
    if !section.libraries.iter().any(|l| l.role == "primitives") && !section.libraries.is_empty() {
        out.notes.push(
            "xschem: no library looks like the PDK's device primitives (a name ending in \
             _pr): set the role of the right library to \"primitives\" in tech.toml"
                .into(),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const NMOS: &str = r#"v {xschem version=3.4.8RC file_version=1.3
* a licence text with {braces}
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
V {}
S {}
E {}
L 4 7.5 -22.5 7.5 22.5 {}
B 5 17.5 -32.5 22.5 -27.5 {name=D dir=inout}
B 5 -22.5 -2.5 -17.5 2.5 {name=G dir=in}
B 5 17.5 27.5 22.5 32.5 {name=S dir=inout}
B 5 19.5 -2.5 22.5 2.5 {name=B dir=in
pinnumber=4}
B 4 0 0 1 1 {name=notapin}
T {@name} 5 -30 0 1 0.2 0.2 {}
"#;

    #[test]
    fn a_device_symbol() {
        let s = parse_symbol(NMOS, "sg13g2_pr", "sg13_lv_nmos");
        assert_eq!(s.kind, "nmos");
        assert_eq!(s.model, "sg13_lv_nmos");
        assert_eq!(s.prefix, "X");
        assert_eq!(s.pins, ["D", "G", "S", "B"]);
        assert_eq!(s.directions, ["inout", "in", "inout", "in"]);
        assert_eq!(s.parameters, ["l", "w", "ng", "m", "model", "spiceprefix"]);
        assert_eq!(s.template["w"], "0.15u");
        assert!(s.lvs_format.starts_with("M@name"));
        assert!(s.includes.is_empty());
    }

    #[test]
    fn models_in_netlist_lines() {
        let t: BTreeMap<String, String> = [("model", "nfet_01v8"), ("m", "1"), ("mult", "1")]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        assert_eq!(
            netlist_model(
                "@spiceprefix@name @pinlist sky130_fd_pr__@model L=@L W=@W",
                &t,
                "nfet_01v8"
            ),
            "sky130_fd_pr__nfet_01v8"
        );
        assert_eq!(netlist_model("@name @pinlist @symname", &t, "inv"), "inv");
        // An extra node between the pins and the model.
        assert_eq!(
            netlist_model(
                "@spiceprefix@name @pinlist @body sky130_fd_pr__@model L=@L",
                &t,
                "x"
            ),
            "sky130_fd_pr__nfet_01v8"
        );
        assert_eq!(netlist_model("R@name @@P @@M @value m=@m", &t, "res"), "");
        assert_eq!(netlist_model("C@name @pinlist c=1p", &t, "cap"), "");

        let corner = r#"v {xschem version=3.4.5 file_version=1.2}
G {}
K {type=netlist_commands
template="name=CORNER only_toplevel=false corner=tt"
format="tcleval(.lib $::SKYWATER_MODELS/sky130.lib.spice @corner )"
}
"#;
        let s = parse_symbol(corner, "sky130_fd_pr", "corner");
        assert_eq!(s.kind, "netlist_commands");
        assert_eq!(
            s.includes,
            [".lib $::SKYWATER_MODELS/sky130.lib.spice @corner"]
        );
        assert_eq!(s.template["corner"], "tt");
        assert!(s.pins.is_empty());
    }

    const RC: &str = r#"
set XSCHEM_LIBRARY_PATH {}
append XSCHEM_LIBRARY_PATH :${XSCHEM_SHAREDIR}/xschem_library/devices
append XSCHEM_LIBRARY_PATH :[file dirname [file normalize [info script]]]
set netlist_dir $env(PWD)/simulations
set tcl_files {}
lappend tcl_files ${XSCHEM_SHAREDIR}/ngspice_backannotate.tcl
if { [info exists env(PDK_ROOT)] && $env(PDK_ROOT) ne {} } {
  set PDK_ROOT $env(PDK_ROOT)
} else {
  if {[file isdir /usr/share/pdk]} {set PDK_ROOT /usr/share/pdk
  }
}
if {[info exists PDK_ROOT]} {
  set SG13G2_MODELS ${PDK_ROOT}/${PDK}/libs.tech/ngspice/models
  puts stderr "MODELS_NGSPICE: $MODELS_NGSPICE"
}
proc fet_drc {instance} {
  set res {}
  if {1} { set inner 2 }
  return $res
}
set XSCHEM_START_WINDOW ${PDK_ROOT}/${PDK}/libs.tech/xschem/start_page.sch
append XSCHEM_LIBRARY_PATH :${PDK_ROOT}/${PDK}/libs.tech/xschem/sg13g2_pr
if { [file exists x] } {
    source ${PDK_ROOT}/${PDK}/libs.tech/xschem/sg13g2_pr/cap_cmomi.tcl
}
set dircolor(sg13g2_pr$) blue
"#;

    #[test]
    fn start_up_file_as_text() {
        let pdk = PdkDir::new("/opt/pdks", "ihp-sg13g2");
        let mut values = RcValues::default();
        let file = pdk.tech("xschem").join("xschemrc");
        parse_rc_text(RC, &file, &pdk, &mut values);
        assert_eq!(
            values.library_path,
            [
                "${XSCHEM_SHAREDIR}/xschem_library/devices",
                "/opt/pdks/ihp-sg13g2/libs.tech/xschem",
                "/opt/pdks/ihp-sg13g2/libs.tech/xschem/sg13g2_pr"
            ]
        );
        assert_eq!(values.variables["netlist_dir"], "$PWD/simulations");
        assert_eq!(
            values.variables["SG13G2_MODELS"],
            "/opt/pdks/ihp-sg13g2/libs.tech/ngspice/models"
        );
        assert!(!values.variables.contains_key("res"));
        assert!(!values.variables.contains_key("inner"));
        assert!(!values.variables.contains_key("PDK_ROOT"));
        assert_eq!(
            values.tcl_files,
            [
                "${XSCHEM_SHAREDIR}/ngspice_backannotate.tcl",
                "/opt/pdks/ihp-sg13g2/libs.tech/xschem/sg13g2_pr/cap_cmomi.tcl"
            ]
        );
    }

    #[test]
    fn portable_texts() {
        let pdk = PdkDir::new("/opt/pdks", "sky130A");
        let p = Portable::new(&pdk);
        assert_eq!(
            p.text("/opt/pdks/sky130A/libs.tech/combined"),
            "$PDK_ROOT/sky130A/libs.tech/combined"
        );
        assert_eq!(p.text(":/opt/pdks/other:/x"), ":$PDK_ROOT/other:/x");
        // Not a path prefix.
        assert_eq!(p.text("/opt/pdks-old/x"), "/opt/pdks-old/x");
        assert!(p.inside(Path::new("/opt/pdks/sky130A/libs.tech")));
        assert!(!p.inside(Path::new("/usr/share")));
        assert_eq!(replace_path("/a/b/c", "/a/b", "$X"), "$X/c");
    }

    #[test]
    fn libraries_of_a_made_up_pdk() {
        let tmp = tempfile::tempdir().unwrap();
        let pdk = PdkDir::new(tmp.path(), "toy");
        let xschem = pdk.tech("xschem");
        for (dir, file, text) in [
            ("toy_pr", "nmos.sym", NMOS),
            ("toy_tests", "tb.sch", "v {}\n"),
            ("toy_tests", "tb.sym", "v {}\nK {type=subcircuit}\n"),
            ("scripts", "x.tcl", ""),
        ] {
            std::fs::create_dir_all(xschem.join(dir)).unwrap();
            std::fs::write(xschem.join(dir).join(file), text).unwrap();
        }
        let stdcells = pdk.refs().join("toy_stdcell/sym/xschem");
        std::fs::create_dir_all(&stdcells).unwrap();
        std::fs::write(
            stdcells.join("inv.sym"),
            "v {}\nK {type=subcircuit\nformat=\"@name @pinlist @symname\"\n\
             template=\"name=x1\"}\nB 5 0 0 1 1 {name=A dir=in}\nB 5 0 0 1 1 {name=Y dir=out}\n",
        )
        .unwrap();
        std::fs::write(
            xschem.join("xschemrc"),
            "append XSCHEM_LIBRARY_PATH :${PDK_ROOT}/${PDK}/libs.tech/xschem\n\
             append XSCHEM_LIBRARY_PATH :${PDK_ROOT}/${PDK}/libs.tech/xschem/toy_pr\n\
             append XSCHEM_LIBRARY_PATH :${PDK_ROOT}/${PDK}/libs.ref/toy_stdcell/sym/xschem\n\
             set TOY_MODELS ${PDK_ROOT}/${PDK}/libs.tech/ngspice\nset autotrim_wires 1\n",
        )
        .unwrap();
        let scanned = scan(&ScanContext::new(pdk).files_only());
        let s = &scanned.section;
        assert_eq!(
            s.rc.as_ref().unwrap().as_str(),
            "$PDK_ROOT/toy/libs.tech/xschem/xschemrc"
        );
        assert!(!s.evaluated);
        assert_eq!(s.variables["TOY_MODELS"], "$PDK_ROOT/toy/libs.tech/ngspice");
        let libs: Vec<(&str, &str, bool)> = s
            .libraries
            .iter()
            .map(|l| (l.name.as_str(), l.role.as_str(), l.enabled))
            .collect();
        assert_eq!(
            libs,
            [
                ("toy_pr", "primitives", true),
                ("toy_tests", "tests", false),
                ("toy_stdcell", "stdcells", true)
            ]
        );
        assert_eq!(s.references["toy_pr"], "toy_pr/");
        assert_eq!(s.references["toy_stdcell"], "");
        assert_eq!(
            s.libraries[2].path.as_str(),
            "$PDK_ROOT/toy/libs.ref/toy_stdcell/sym/xschem"
        );
        let names: Vec<&str> = scanned
            .catalog
            .symbols
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(names, ["nmos", "inv"]);
        assert_eq!(scanned.catalog.symbols[1].model, "inv");
        assert_eq!(scanned.catalog.counts["toy_tests"], 1);
        assert_eq!(scanned.catalog.settings["autotrim_wires"], "1");
        assert!(scanned.notes.iter().any(|n| n.contains("read as text")));

        let text = toml::to_string_pretty(s).unwrap();
        let back: XschemTech = toml::from_str(&text).unwrap();
        assert_eq!(&back, s, "{text}");
    }
}
