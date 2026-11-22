//! What an `xschemrc` says about where the libraries are.
//!
//! An `xschemrc` is a Tcl program, so the sure way to read it is to run it: when the
//! xschem helper is built, it is started with the file as its start-up file and asked
//! for `XSCHEM_LIBRARY_PATH` afterwards. Without the helper (or when the file fails)
//! the text is read instead: `set`/`append XSCHEM_LIBRARY_PATH` lines, with `$VAR`,
//! `${VAR}`, `$env(VAR)`, `[pwd]` and `[file dirname [info script]]` filled in;
//! conditions are not followed, so this is a best guess and says so.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::xschem::{Engine, EngineOptions, tcl};

/// One folder of the symbol search path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathEntry {
    /// A folder (it may not exist).
    Folder(PathBuf),
    /// One of xschem's own libraries (`devices` and the like): CDePT's xschem has them.
    Own,
    /// Text that could not be turned into a folder (fallback reading only).
    Unresolved(String),
}

/// What was read out of an `xschemrc`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RcInfo {
    pub file: PathBuf,
    /// `XSCHEM_LIBRARY_PATH`, in order.
    pub path: Vec<PathEntry>,
    /// `PDK_ROOT` and `PDK` as the file leaves them, when it sets them.
    pub pdk_root: Option<String>,
    pub pdk: Option<String>,
    /// `netlist_dir`: where netlists and simulation results go (generated output).
    pub netlist_dir: Option<PathBuf>,
    /// The file was run by xschem; otherwise its text was read.
    pub evaluated: bool,
    pub notes: Vec<String>,
}

/// The start-up file of a folder: `xschemrc`, else `.xschemrc`.
pub fn find_rc(folder: &Path) -> Option<PathBuf> {
    ["xschemrc", ".xschemrc"]
        .into_iter()
        .map(|name| folder.join(name))
        .find(|file| file.is_file())
}

/// Read `file`. `folder` is the folder xschem would be started in. With `helpers` the
/// xschem helper is used when it can be found.
pub fn read_rc(
    file: &Path,
    folder: &Path,
    helpers: bool,
    env: &dyn Fn(&str) -> Option<String>,
) -> RcInfo {
    let mut notes = Vec::new();
    if helpers && Engine::find_helper().is_some() {
        match evaluate(file, folder, env) {
            Ok(mut info) => {
                info.notes.append(&mut notes);
                return info;
            }
            Err(error) => notes.push(format!(
                "{}: xschem could not run it ({}); its text was read instead",
                file.display(),
                error.lines().next().unwrap_or("").trim()
            )),
        }
    } else if helpers {
        notes.push(format!(
            "{}: the xschem helper is not built, so the file was read as text (conditions \
             are not followed)",
            file.display()
        ));
    }
    let text = std::fs::read_to_string(file).unwrap_or_default();
    let mut info = parse_rc_text(&text, file, folder, env);
    notes.append(&mut info.notes);
    info.notes = notes;
    info
}

fn evaluate(
    file: &Path,
    folder: &Path,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<RcInfo, String> {
    // The PDK as the caller sees it (it may know it from the folder being scanned).
    let pdk_env = ["PDK_ROOT", "PDK"]
        .into_iter()
        .filter_map(|name| Some((name.into(), env(name)?.into())))
        .collect();
    let options = EngineOptions {
        env: pdk_env,
        startup: Some(format!("source {}\n", tcl::quote(&file.to_string_lossy()))),
        cwd: Some(folder.to_path_buf()),
        ..EngineOptions::default()
    };
    let mut engine = Engine::start(options).map_err(|e| format!("{e:#}"))?;
    let mut var = |name: &str| -> Option<String> {
        let script = format!("if {{[info exists ::{name}]}} {{set ::{name}}} {{set _ {{}}}}");
        engine
            .tcl(&script)
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    let path = var("XSCHEM_LIBRARY_PATH").unwrap_or_default();
    let sharedir = var("XSCHEM_SHAREDIR").map(PathBuf::from);
    let conf = var("USER_CONF_DIR").map(PathBuf::from);
    let pdk_root = var("PDK_ROOT");
    let pdk = var("PDK");
    let netlist_dir = var("netlist_dir").map(|d| folder.join(d));
    let _ = engine.shutdown();

    let mut entries = Vec::new();
    for item in path.split(':').filter(|i| !i.trim().is_empty()) {
        let folder_path = folder.join(item.trim());
        let own = sharedir
            .as_ref()
            .is_some_and(|s| folder_path.starts_with(s))
            || conf.as_ref().is_some_and(|c| folder_path.starts_with(c));
        let entry = if own {
            PathEntry::Own
        } else {
            PathEntry::Folder(folder_path)
        };
        if !entries.contains(&entry) {
            entries.push(entry);
        }
    }
    Ok(RcInfo {
        file: file.to_path_buf(),
        path: entries,
        pdk_root,
        pdk,
        netlist_dir,
        evaluated: true,
        notes: Vec::new(),
    })
}

/// Fill in the variables of one Tcl word. `None` when something in it is not known.
fn substitute(
    word: &str,
    vars: &dyn Fn(&str) -> Option<String>,
    here: &Path,
    cwd: &Path,
) -> Option<String> {
    let word = word.trim();
    let word = word
        .strip_prefix('{')
        .and_then(|w| w.strip_suffix('}'))
        .or_else(|| word.strip_prefix('"').and_then(|w| w.strip_suffix('"')))
        .unwrap_or(word);
    let dir_of_script = here.to_string_lossy().into_owned();
    let mut text = word.to_string();
    for call in [
        "[file dirname [info script]]",
        "[file dirname [file normalize [info script]]]",
        "[file normalize [file dirname [info script]]]",
    ] {
        text = text.replace(call, &dir_of_script);
    }
    text = text.replace("[pwd]", &cwd.to_string_lossy());
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '[' || c == ']' {
            return None; // a command we do not run
        }
        if c != '$' {
            out.push(c);
            i += 1;
            continue;
        }
        i += 1;
        let mut name = String::new();
        if chars.get(i) == Some(&'{') {
            i += 1;
            while i < chars.len() && chars[i] != '}' {
                name.push(chars[i]);
                i += 1;
            }
            i += 1;
        } else {
            while i < chars.len()
                && (chars[i].is_ascii_alphanumeric() || chars[i] == '_' || chars[i] == ':')
            {
                name.push(chars[i]);
                i += 1;
            }
            // $env(NAME)
            if chars.get(i) == Some(&'(') {
                name.push('(');
                i += 1;
                while i < chars.len() && chars[i] != ')' {
                    name.push(chars[i]);
                    i += 1;
                }
                name.push(')');
                i += 1;
            }
        }
        let name = name.trim_start_matches(':');
        out.push_str(&vars(name)?);
    }
    Some(out)
}

/// Read the text of an `xschemrc` without running it.
pub fn parse_rc_text(
    text: &str,
    file: &Path,
    folder: &Path,
    env: &dyn Fn(&str) -> Option<String>,
) -> RcInfo {
    let here = file.parent().unwrap_or(folder).to_path_buf();
    let mut set: BTreeMap<String, String> = BTreeMap::new();
    let mut entries: Vec<PathEntry> = Vec::new();
    let mut unknown = 0;
    let lookup = |set: &BTreeMap<String, String>, name: &str| -> Option<String> {
        if let Some(inner) = name
            .strip_prefix("env(")
            .or_else(|| name.strip_prefix("::env("))
            .and_then(|n| n.strip_suffix(')'))
        {
            return env(inner);
        }
        match name {
            "PWD" => Some(folder.to_string_lossy().into_owned()),
            "USER_CONF_DIR" => env("HOME").map(|home| format!("{home}/.xschem")),
            // The environment's PDK wins over what the file would fall back to.
            "PDK_ROOT" | "PDK" => env(name).or_else(|| set.get(name).cloned()),
            _ => set.get(name).cloned().or_else(|| env(name)),
        }
    };
    let add = |entries: &mut Vec<PathEntry>, entry: PathEntry| {
        if !entries.contains(&entry) {
            entries.push(entry);
        }
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut words = line.splitn(3, char::is_whitespace);
        let (Some(command), Some(name), rest) = (words.next(), words.next(), words.next()) else {
            continue;
        };
        let value = rest.unwrap_or("").trim();
        // A comment after the value.
        let value = value.split(" ;#").next().unwrap_or(value).trim();
        if name != "XSCHEM_LIBRARY_PATH" {
            if command == "set" && !value.is_empty() && !set.contains_key(name) {
                let vars = |n: &str| lookup(&set, n);
                if let Some(v) = substitute(value, &vars, &here, folder) {
                    set.insert(name.to_string(), v);
                }
            }
            continue;
        }
        match command {
            "set" if value == "{}" || value == "\"\"" || value.is_empty() => entries.clear(),
            "set" | "append" | "lappend" => {
                // xschem's own libraries: named through its share folder.
                for item in value.split(':').filter(|i| !i.trim().is_empty()) {
                    if item.contains("XSCHEM_SHAREDIR") || item.contains("USER_CONF_DIR") {
                        add(&mut entries, PathEntry::Own);
                        continue;
                    }
                    // `$XSCHEM_LIBRARY_PATH:more`: the path so far stays.
                    if item.contains("XSCHEM_LIBRARY_PATH") {
                        continue;
                    }
                    let vars = |n: &str| lookup(&set, n);
                    match substitute(item, &vars, &here, folder) {
                        Some(path) if !path.trim().is_empty() => {
                            add(&mut entries, PathEntry::Folder(folder.join(path.trim())));
                        }
                        _ => {
                            unknown += 1;
                            add(&mut entries, PathEntry::Unresolved(item.trim().to_string()));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    let mut notes = Vec::new();
    if unknown > 0 {
        notes.push(format!(
            "{}: {unknown} entr{} of XSCHEM_LIBRARY_PATH could not be worked out from the text",
            file.display(),
            if unknown == 1 { "y" } else { "ies" }
        ));
    }
    let pdk_root = lookup(&set, "PDK_ROOT");
    let pdk = lookup(&set, "PDK");
    let netlist_dir = set.get("netlist_dir").map(|d| folder.join(d));
    RcInfo {
        file: file.to_path_buf(),
        path: entries,
        pdk_root,
        pdk,
        netlist_dir,
        evaluated: false,
        notes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(name: &str) -> Option<String> {
        match name {
            "HOME" => Some("/home/u".into()),
            "PDK_ROOT" => Some("/pdks".into()),
            _ => None,
        }
    }

    #[test]
    fn the_text_of_an_rc_gives_the_path() {
        let text = r#"
# comment
set XSCHEM_LIBRARY_PATH {}
append XSCHEM_LIBRARY_PATH :${XSCHEM_SHAREDIR}/xschem_library/devices
append XSCHEM_LIBRARY_PATH :$USER_CONF_DIR/xschem_library
if {[info exists env(PDK_ROOT)]} {
  set PDK_ROOT $env(PDK_ROOT)
} else {
  set PDK_ROOT /usr/share/pdk
}
set PDK ihp-sg13g2
set netlist_dir $env(PWD)/simulations
append XSCHEM_LIBRARY_PATH :${PDK_ROOT}/${PDK}/libs.tech/xschem
append XSCHEM_LIBRARY_PATH :[file dirname [info script]]/mylib
append XSCHEM_LIBRARY_PATH :[some command]/x
append XSCHEM_LIBRARY_PATH :$HOME/libs:relative/lib
"#;
        let info = parse_rc_text(text, Path::new("/proj/xschemrc"), Path::new("/proj"), &env);
        assert!(!info.evaluated);
        assert_eq!(
            info.path,
            [
                PathEntry::Own,
                PathEntry::Folder("/pdks/ihp-sg13g2/libs.tech/xschem".into()),
                PathEntry::Folder("/proj/mylib".into()),
                PathEntry::Unresolved("[some command]/x".into()),
                PathEntry::Folder("/home/u/libs".into()),
                PathEntry::Folder("/proj/relative/lib".into()),
            ]
        );
        assert_eq!(info.pdk_root.as_deref(), Some("/pdks"));
        assert_eq!(info.pdk.as_deref(), Some("ihp-sg13g2"));
        assert_eq!(info.notes.len(), 1);
    }

    #[test]
    fn without_the_environment_the_fallback_of_the_file_counts() {
        fn none(_: &str) -> Option<String> {
            None
        }
        let text = "set PDK_ROOT $env(PDK_ROOT)\nset PDK_ROOT /usr/share/pdk\n\
                    set XSCHEM_LIBRARY_PATH $PDK_ROOT/sky130A/libs.tech/xschem\n";
        let info = parse_rc_text(text, Path::new("/p/xschemrc"), Path::new("/p"), &none);
        assert_eq!(
            info.path,
            [PathEntry::Folder(
                "/usr/share/pdk/sky130A/libs.tech/xschem".into()
            )]
        );
    }
}
