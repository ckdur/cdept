//! The PCell libraries of a KLayout technology: asked from KLayout itself, or read
//! out of the Python sources when it cannot be asked.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::Deserialize;

use super::{Pcell, PcellLibrary, PcellParameter};
use crate::klayout::setup;
use crate::tech::pdk::ScanContext;

/// The script KLayout runs, in the helper folder.
pub const SCRIPT: &str = "cdept_klayout/techscan.py";
/// Seconds KLayout gets to list the libraries (start-up and the PDK's autorun
/// macros included); `$CDEPT_TECH_SCAN_TIMEOUT` changes it.
pub const TIMEOUT_SECONDS: u64 = 60;
/// KLayout's own library of basic shapes: not the PDK's.
const BUILT_IN: &str = "Basic";

/// What KLayout said.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Asked {
    /// KLayout's version text.
    pub program: String,
    pub libraries: Vec<PcellLibrary>,
    /// The technologies KLayout loaded (names).
    pub technologies: Vec<String>,
    /// What KLayout printed that the user should know (a PDK asking for a Python
    /// package, a failed macro), and errors of single PCells.
    pub messages: Vec<String>,
    pub seconds: f64,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawOutput {
    klayout: String,
    technologies: Vec<RawTechnology>,
    libraries: Vec<RawLibrary>,
    errors: Vec<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawTechnology {
    name: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawLibrary {
    name: String,
    description: String,
    technologies: Vec<String>,
    cells: Vec<String>,
    pcells: Vec<RawPcell>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawPcell {
    name: String,
    description: String,
    parameters: Vec<RawParameter>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawParameter {
    name: String,
    #[serde(rename = "type")]
    kind: String,
    default: String,
    description: String,
    group: String,
    unit: String,
    choices: Vec<RawChoice>,
    hidden: bool,
    readonly: bool,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawChoice {
    label: String,
    value: String,
}

/// Turn the script's JSON into libraries (KLayout's built-in library left out).
pub fn parse_output(json: &str) -> Result<Asked, String> {
    let raw: RawOutput = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let libraries = raw
        .libraries
        .into_iter()
        .filter(|l| l.name != BUILT_IN)
        .map(|l| PcellLibrary {
            name: l.name,
            description: l.description,
            technologies: l.technologies,
            cells: l.cells,
            pcells: l
                .pcells
                .into_iter()
                .map(|p| Pcell {
                    // KLayout's display text of the defaults ("nmos (...)") says
                    // nothing the name does not.
                    description: if p.description.starts_with(&p.name) {
                        String::new()
                    } else {
                        p.description
                    },
                    name: p.name,
                    parameters: p
                        .parameters
                        .into_iter()
                        .map(|q| PcellParameter {
                            name: q.name,
                            kind: q.kind,
                            default: q.default,
                            description: q.description,
                            unit: q.unit,
                            choices: q.choices.iter().map(|c| c.value.clone()).collect(),
                            choice_labels: q
                                .choices
                                .iter()
                                .map(|c| c.label.trim_matches('\'').to_string())
                                .collect(),
                            group: q.group,
                            hidden: q.hidden,
                            readonly: q.readonly,
                        })
                        .collect(),
                })
                .collect(),
        })
        .collect();
    Ok(Asked {
        program: raw.klayout,
        libraries,
        technologies: raw.technologies.into_iter().map(|t| t.name).collect(),
        messages: raw.errors,
        seconds: 0.0,
    })
}

fn timeout() -> Duration {
    let seconds = std::env::var("CDEPT_TECH_SCAN_TIMEOUT")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|s| *s > 0)
        .unwrap_or(TIMEOUT_SECONDS);
    Duration::from_secs(seconds)
}

/// A folder of our own for one run (the output file and KLayout's messages).
fn scratch_dir() -> std::io::Result<PathBuf> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let dir = std::env::temp_dir().join(format!("cdept-techscan-{}-{stamp}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Ask KLayout for the PCell libraries it registers with `klayout_path` on
/// `KLAYOUT_PATH`: `klayout -zz -nc -r techscan.py -rd out_file=…`. Not batch mode
/// (`-b`): that leaves out the autorun macros that register a PDK's libraries.
/// An error says why KLayout could not be asked.
pub fn ask(ctx: &ScanContext, klayout_path: &[PathBuf]) -> Result<Asked, String> {
    let klayout = setup::find_klayout().ok_or_else(|| {
        format!(
            "KLayout is not installed (build it with cdept-klayout/build.sh, or set {})",
            setup::KLAYOUT_ENV
        )
    })?;
    let script = setup::find_helper_dir()
        .map(|dir| dir.join(SCRIPT))
        .filter(|s| s.is_file())
        .ok_or_else(|| format!("the helper script {SCRIPT} is not installed"))?;
    let dir = scratch_dir().map_err(|e| format!("no scratch folder: {e}"))?;
    let result = run(ctx, &klayout, &script, klayout_path, &dir);
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn run(
    ctx: &ScanContext,
    klayout: &Path,
    script: &Path,
    klayout_path: &[PathBuf],
    dir: &Path,
) -> Result<Asked, String> {
    let out = dir.join("techscan.json");
    let log = dir.join("klayout.log");
    let log_file = std::fs::File::create(&log).map_err(|e| e.to_string())?;
    let mut define = OsString::from("out_file=");
    define.push(&out);
    let mut command = Command::new(klayout);
    command
        .args(["-zz", "-nc", "-r"])
        .arg(script)
        .arg("-rd")
        .arg(define)
        .current_dir(dir)
        .envs(setup::helper_env(klayout))
        .envs(ctx.env.iter().map(|(k, v)| (k.clone(), v.clone())))
        .stdin(Stdio::null())
        .stdout(log_file.try_clone().map_err(|e| e.to_string())?)
        .stderr(log_file);
    // The PDK's folders only: what the user has on KLAYOUT_PATH is not the PDK's.
    match std::env::join_paths(klayout_path) {
        Ok(joined) if !klayout_path.is_empty() => command.env("KLAYOUT_PATH", joined),
        _ => command.env_remove("KLAYOUT_PATH"),
    };
    let started = Instant::now();
    let limit = timeout();
    let mut child = command
        .spawn()
        .map_err(|e| format!("cannot start {}: {e}", klayout.display()))?;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() > limit => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "KLayout did not answer within {} s (set CDEPT_TECH_SCAN_TIMEOUT for more)",
                    limit.as_secs()
                ));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => return Err(format!("waiting for KLayout: {e}")),
        }
    };
    let seconds = started.elapsed().as_secs_f64();
    let printed = std::fs::read_to_string(&log).unwrap_or_default();
    let json = std::fs::read_to_string(&out).map_err(|_| {
        let tail: Vec<&str> = printed.lines().rev().take(6).collect();
        let tail: Vec<&str> = tail.into_iter().rev().collect();
        if tail.is_empty() {
            format!("KLayout wrote no answer ({status})")
        } else {
            format!("KLayout wrote no answer ({status}): {}", tail.join(" | "))
        }
    })?;
    let mut asked = parse_output(&json)?;
    asked.seconds = seconds;
    asked.messages.extend(messages(&printed));
    Ok(asked)
}

/// The lines KLayout printed that matter: errors, warnings and notices (`Info:`), each
/// once, a handful at most.
fn messages(printed: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in printed.lines() {
        let line = line.trim();
        let lower = line.to_lowercase();
        if !(lower.starts_with("error")
            || lower.starts_with("warning")
            || lower.starts_with("info:")
            || lower.contains("modulenotfounderror")
            || lower.contains("importerror"))
        {
            continue;
        }
        let mut text: String = line.chars().take(240).collect();
        if text.len() < line.len() {
            text.push('…');
        }
        if !out.contains(&text) {
            out.push(text);
        }
        if out.len() >= 6 {
            break;
        }
    }
    out
}

// -- from the Python sources ---------------------------------------------------

/// What the Python sources say about PCell libraries.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sources {
    pub libraries: Vec<PcellLibrary>,
    /// PCells registered under a name made at run time: these cannot be read.
    pub computed: usize,
}

/// Read PCell libraries out of the Python sources below `dirs`: `register_pcell("name",
/// Class())` and `.register("library")` say which PCells a library has,
/// `self.param("name", self.TypeX, "description", default=…)` in the class says its
/// parameters. Defaults that are expressions stay as written. Good for PDKs whose
/// libraries are plain `PCellDeclarationHelper` classes (sky130); libraries built at
/// run time (IHP's) cannot be read this way.
pub fn from_sources(dirs: &[PathBuf]) -> Sources {
    let mut files = Vec::new();
    for dir in dirs {
        python_files(dir, 0, &mut files);
    }
    let mut seen = BTreeSet::new();
    files.retain(|f| seen.insert(f.canonicalize().unwrap_or(f.clone())));
    let texts: Vec<(PathBuf, String)> = files
        .into_iter()
        .filter_map(|f| {
            let text = std::fs::read_to_string(&f).ok()?;
            Some((f, text))
        })
        .collect();

    // Class name → its bases and the parameters it declares itself, over all files.
    let mut classes: BTreeMap<String, Class> = BTreeMap::new();
    for (_, text) in &texts {
        for class in classes_in(text) {
            match classes.get(&class.name) {
                // The same name twice: the one that declares parameters counts.
                Some(old) if !old.parameters.is_empty() || class.parameters.is_empty() => {}
                _ => {
                    classes.insert(class.name.clone(), class);
                }
            }
        }
    }
    // The parameters of a class: its own, else those of the first base that has any.
    fn parameters_of(
        classes: &BTreeMap<String, Class>,
        name: &str,
        depth: usize,
    ) -> Vec<PcellParameter> {
        let Some(class) = classes.get(name) else {
            return Vec::new();
        };
        if !class.parameters.is_empty() || depth > 4 {
            return class.parameters.clone();
        }
        class
            .bases
            .iter()
            .map(|base| parameters_of(classes, base, depth + 1))
            .find(|p| !p.is_empty())
            .unwrap_or_default()
    }

    let mut libraries: Vec<PcellLibrary> = Vec::new();
    let mut computed = 0;
    for (_, text) in &texts {
        let registered = calls_at(text, "register_pcell");
        if registered.is_empty() {
            continue;
        }
        // The libraries of the file: `self.register("name")`, each after the PCells
        // it registers.
        let named: Vec<(usize, usize, String)> = calls_at(text, "register")
            .into_iter()
            .filter_map(|(at, end, args)| Some((at, end, string_literal(args.first()?)?)))
            .collect();
        for (at, _, args) in registered {
            let Some((index, (_, _, name))) = named
                .iter()
                .enumerate()
                .find(|(_, library)| library.0 > at)
                .or_else(|| named.iter().enumerate().next_back())
            else {
                continue;
            };
            let Some(pcell) = args.first().and_then(|a| string_literal(a)) else {
                // `register_pcell(match.group(1), …)`: the name is made at run time.
                computed += 1;
                continue;
            };
            // `pfet()` → class `pfet`.
            let class = args
                .get(1)
                .map(|a| a.split('(').next().unwrap_or_default().trim().to_string())
                .unwrap_or_default();
            let pcell = Pcell {
                name: pcell,
                description: String::new(),
                parameters: parameters_of(&classes, &class, 0),
            };
            match libraries.iter_mut().find(|l| l.name == *name) {
                Some(library) => library.pcells.push(pcell),
                None => {
                    // Its description is set between the library before and its own
                    // `register`.
                    let from = index.checked_sub(1).map_or(0, |before| named[before].1);
                    libraries.push(PcellLibrary {
                        name: name.clone(),
                        description: assigned_string(
                            &text[from..named[index].0],
                            "self.description",
                        )
                        .unwrap_or_default(),
                        pcells: vec![pcell],
                        ..PcellLibrary::default()
                    })
                }
            }
        }
    }
    for library in &mut libraries {
        library.pcells.sort_by(|a, b| a.name.cmp(&b.name));
        library.pcells.dedup_by(|a, b| a.name == b.name);
    }
    Sources {
        libraries,
        computed,
    }
}

fn python_files(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > 5 || out.len() > 2000 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            if !name.starts_with('.') && name != "__pycache__" && name != "testing" {
                python_files(&path, depth + 1, out);
            }
        } else if path.extension().is_some_and(|e| e == "py" || e == "lym")
            && path.metadata().is_ok_and(|m| m.len() < 2_000_000)
        {
            out.push(path);
        }
    }
}

/// A class of a Python text.
struct Class {
    name: String,
    /// The classes it derives from, by their last name (`pya.X` → `X`).
    bases: Vec<String>,
    /// The parameters it declares itself (`self.param(…)`).
    parameters: Vec<PcellParameter>,
}

/// Constants a Python text sets at its top level: `name = 0.15`, `name = "text"`.
fn constants_in(text: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        // At the top level only, and a plain name.
        if name.starts_with(char::is_whitespace)
            || !name
                .trim()
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
            || name.trim().is_empty()
        {
            continue;
        }
        let value = value.split('#').next().unwrap_or_default().trim();
        let constant = match string_literal(value) {
            Some(text) => text,
            None if value.parse::<f64>().is_ok() => value.to_string(),
            None if value == "True" => "true".to_string(),
            None if value == "False" => "false".to_string(),
            None => continue,
        };
        out.insert(name.trim().to_string(), constant);
    }
    out
}

/// The classes of a Python text with the parameters each declares.
fn classes_in(text: &str) -> Vec<Class> {
    let constants = constants_in(text);
    let mut out = Vec::new();
    let mut starts: Vec<(usize, String, Vec<String>)> = Vec::new();
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        if let Some(rest) = line.strip_prefix("class ") {
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            let bases = rest
                .split_once('(')
                .and_then(|(_, after)| after.split_once(')'))
                .map(|(inside, _)| {
                    inside
                        .split(',')
                        .map(|b| b.trim().rsplit('.').next().unwrap_or_default().to_string())
                        .filter(|b| !b.is_empty() && !b.contains('='))
                        .collect()
                })
                .unwrap_or_default();
            if !name.is_empty() {
                starts.push((offset, name, bases));
            }
        }
        offset += line.len();
    }
    for (index, (start, name, bases)) in starts.iter().enumerate() {
        let end = starts.get(index + 1).map_or(text.len(), |s| s.0);
        let body = &text[*start..end];
        let declared = calls_at(body, "param");
        let mut parameters = Vec::new();
        for (position, (at, after, args)) in declared.iter().enumerate() {
            let Some(mut parameter) = parameter(args) else {
                continue;
            };
            // A default that is a constant of the file: its value.
            if let Some(value) = constants.get(&parameter.default) {
                parameter.default = value.clone();
            }
            // `handle = self.param(…)` followed by `handle.add_choice(label, value)` and
            // `handle.default = …`, up to the next parameter.
            let line_start = body[..*at].rfind('\n').map_or(0, |i| i + 1);
            let head = body[line_start..*at].trim();
            if let Some((handle, call)) = head.split_once('=')
                && call.trim().ends_with('.')
            {
                let handle = handle.trim();
                let until = declared.get(position + 1).map_or(body.len(), |next| next.0);
                let region = &body[*after..until];
                let owner = format!("{handle}.");
                for (choice_at, _, choice) in calls_at(region, "add_choice") {
                    // Only this handle's choices: the text before the call says whose.
                    if !region[..choice_at].ends_with(&owner) {
                        continue;
                    }
                    // Choices made in a loop over a variable say nothing.
                    if let [label, value] = choice.as_slice()
                        && (string_literal(label).is_some()
                            || string_literal(value).is_some()
                            || value.parse::<f64>().is_ok())
                    {
                        parameter
                            .choice_labels
                            .push(string_literal(label).unwrap_or_else(|| label.clone()));
                        parameter
                            .choices
                            .push(string_literal(value).unwrap_or_else(|| value.clone()));
                    }
                }
                let default = format!("{handle}.default");
                if let Some(value) = region.lines().find_map(|line| {
                    line.trim()
                        .strip_prefix(&default)?
                        .trim_start()
                        .strip_prefix('=')
                        .map(str::trim)
                }) {
                    // `handle.choice_values()[0]`: the first choice.
                    let picked = value
                        .rsplit_once('[')
                        .filter(|(before, _)| before.ends_with("choice_values()"))
                        .and_then(|(_, index)| index.trim_end_matches(']').parse::<usize>().ok())
                        .and_then(|index| parameter.choices.get(index).cloned());
                    parameter.default = picked
                        .or_else(|| string_literal(value))
                        .unwrap_or_else(|| value.to_string());
                }
            }
            parameters.push(parameter);
        }
        out.push(Class {
            name: name.clone(),
            bases: bases.clone(),
            parameters,
        });
    }
    out
}

/// `self.param("name", self.TypeDouble, "description", default=1.0, unit="um", …)`.
fn parameter(args: &[String]) -> Option<PcellParameter> {
    let name = string_literal(args.first()?)?;
    let kind = match args
        .get(1)
        .map(|a| a.rsplit('.').next().unwrap_or_default().trim())
    {
        Some("TypeInt") => "int",
        Some("TypeDouble") => "double",
        Some("TypeString") => "string",
        Some("TypeBoolean") => "boolean",
        Some("TypeList") => "list",
        Some("TypeLayer") => "layer",
        Some("TypeShape") => "shape",
        Some("TypeCallback") => "callback",
        _ => "none",
    };
    let mut parameter = PcellParameter {
        name,
        kind: kind.to_string(),
        description: args
            .get(2)
            .and_then(|a| string_literal(a))
            .unwrap_or_default(),
        ..PcellParameter::default()
    };
    for arg in args.iter().skip(3) {
        let Some((key, value)) = arg.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "default" => {
                parameter.default = string_literal(value).unwrap_or_else(|| match value {
                    "True" => "true".to_string(),
                    "False" => "false".to_string(),
                    other => other.to_string(),
                })
            }
            "unit" => parameter.unit = string_literal(value).unwrap_or_default(),
            "hidden" => parameter.hidden = value == "True",
            "readonly" => parameter.readonly = value == "True",
            "choices" => {
                // [("label", value), …] or (("label", value), …)
                let inner = match value.chars().next() {
                    Some('[' | '(') => balanced(&value[1..]).unwrap_or(value),
                    _ => value,
                };
                for item in split_arguments(inner) {
                    let item = item.trim();
                    let item = match item.chars().next() {
                        Some('[' | '(') => balanced(&item[1..]).unwrap_or(item),
                        _ => item,
                    };
                    let parts = split_arguments(item);
                    if let [label, choice] = parts.as_slice() {
                        parameter
                            .choice_labels
                            .push(string_literal(label).unwrap_or_else(|| label.trim().into()));
                        parameter
                            .choices
                            .push(string_literal(choice).unwrap_or_else(|| choice.trim().into()));
                    }
                }
            }
            _ => {}
        }
    }
    Some(parameter)
}

/// The argument lists of every call `….name(…)` (or `name(…)`) in a Python text, with
/// where each call's name starts and where the call ends.
fn calls_at(text: &str, name: &str) -> Vec<(usize, usize, Vec<String>)> {
    let mut out = Vec::new();
    let pattern = format!("{name}(");
    let mut from = 0;
    while let Some(found) = text[from..].find(&pattern) {
        let at = from + found;
        from = at + pattern.len();
        // The whole name: not `xregister(`, and not a `def`.
        let before = text[..at].chars().next_back();
        if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
            continue;
        }
        let line_start = text[..at].rfind('\n').map_or(0, |i| i + 1);
        let line_head = text[line_start..at].trim_start();
        if line_head.starts_with('#') || line_head.starts_with("def ") {
            continue;
        }
        let Some(inside) = balanced(&text[from..]) else {
            continue;
        };
        out.push((at, from + inside.len() + 1, split_arguments(inside)));
    }
    out
}

/// The text up to the parenthesis that closes one already open.
fn balanced(text: &str) -> Option<&str> {
    let mut depth = 1usize;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (index, c) in text.char_indices() {
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => quote = Some(c),
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[..index]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Split at the commas that are not inside brackets or quotes; comments removed.
fn split_arguments(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut comment = false;
    for c in text.chars() {
        if comment {
            if c == '\n' {
                comment = false;
            }
            continue;
        }
        if let Some(q) = quote {
            current.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '#' => comment = true,
            '"' | '\'' => {
                quote = Some(c);
                current.push(c);
            }
            '(' | '[' | '{' => {
                depth += 1;
                current.push(c);
            }
            ')' | ']' | '}' => {
                depth = depth.saturating_sub(1);
                current.push(c);
            }
            ',' if depth == 0 => {
                out.push(current.trim().to_string());
                current.clear();
            }
            _ => current.push(c),
        }
    }
    if !current.trim().is_empty() {
        out.push(current.trim().to_string());
    }
    out
}

/// The text of a Python string literal (`"x"`, `'x'`, with an `f`/`r` prefix too).
fn string_literal(text: &str) -> Option<String> {
    let text = text.trim().trim_start_matches(['f', 'r', 'b', 'u']);
    let quote = text.chars().next().filter(|c| *c == '"' || *c == '\'')?;
    let inner = text[1..].strip_suffix(quote)?;
    (!inner.contains(quote)).then(|| inner.to_string())
}

/// `<target> = "text"` in a Python text.
fn assigned_string(text: &str, target: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let rest = line.trim().strip_prefix(target)?.trim_start();
        string_literal(rest.strip_prefix('=')?)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scripts_answer() {
        let asked = parse_output(
            r#"{"klayout": "KLayout 0.30.12",
                "technologies": [{"name": "sg13g2", "dbu": 0.001}],
                "libraries": [
                  {"name": "Basic", "pcells": [{"name": "CIRCLE", "parameters": []}]},
                  {"name": "SG13_dev", "description": "IHP SG13G2 Pcells",
                   "technologies": ["sg13g2"], "cells": ["fixed"],
                   "pcells": [{"name": "nmos", "description": "nmos (...)", "parameters": [
                     {"name": "w", "type": "string", "default": "0.15u",
                      "description": "Width", "group": "", "unit": "", "choices": [],
                      "hidden": false, "readonly": false},
                     {"name": "Display", "type": "string", "default": "Selected",
                      "choices": [{"label": "'All'", "value": "All"},
                                  {"label": "'Selected'", "value": "Selected"}],
                      "hidden": true}
                   ]}]}],
                "errors": ["SG13_dev/esd: boom"]}"#,
        )
        .unwrap();
        assert_eq!(asked.program, "KLayout 0.30.12");
        assert_eq!(asked.technologies, ["sg13g2"]);
        assert_eq!(asked.libraries.len(), 1, "Basic is KLayout's own");
        let library = &asked.libraries[0];
        assert_eq!(library.technologies, ["sg13g2"]);
        assert_eq!(library.cells, ["fixed"]);
        let nmos = &library.pcells[0];
        assert_eq!(nmos.description, "");
        assert_eq!(nmos.parameters[0].default, "0.15u");
        assert_eq!(nmos.parameters[0].description, "Width");
        assert_eq!(nmos.parameters[1].choices, ["All", "Selected"]);
        assert_eq!(nmos.parameters[1].choice_labels, ["All", "Selected"]);
        assert!(nmos.parameters[1].hidden);
        assert_eq!(asked.messages, ["SG13_dev/esd: boom"]);
        assert!(parse_output("nonsense").is_err());
    }

    #[test]
    fn messages_worth_telling() {
        let found = messages(
            "Info: To enable the sky130 PCells you need to install gdsfactory\n\
             Python sys.path: [...]\nINFO  loading\nERROR: boom\nERROR: boom\n",
        );
        assert_eq!(found.len(), 2);
        assert!(found[0].contains("gdsfactory"));
        assert_eq!(found[1], "ERROR: boom");
    }

    const FET: &str = r#"
import pya

fet_l = 0.15  # minimum

class pfet(pya.PCellDeclarationHelper):
    """
    PMOS Generator
    """

    def __init__(self):
        super(pfet, self).__init__()
        self.Type_handle = self.param("type", self.TypeList, "Device Type")
        self.Type_handle.add_choice("sky130_fd_pr__pfet_01v8", "sky130_fd_pr__pfet_01v8")
        self.Type_handle.add_choice("lvt", "sky130_fd_pr__pfet_01v8_lvt")
        self.other.add_choice("not", "mine")
        self.Type_handle.default = self.Type_handle.choice_values()[1]
        self.param("l", self.TypeDouble, "Length", default=fet_l, unit="um")
        self.param("w", self.TypeDouble, "Width", default=fet_w, unit="um")
        self.param("nf", self.TypeInt, "Number of Fingers", default=1)
        self.param("gr", self.TypeBoolean, "Guard Ring", default=True)  # a, comment
        self.param("bulk", self.TypeList, "Bulk Type",
                   choices=(("None", "None"), ("bulk tie", "bulk tie")), default="None")
        self.param("area", self.TypeDouble, "Area", readonly=True, hidden=True, unit="um^2")

class other:
    def param(self, a):
        pass

class pfet_hv(pfet):
    def __init__(self):
        super().__init__()
"#;

    const LIB: &str = r#"
from .fet import *

class Sky130(pya.Library):
    def __init__(self):
        self.description = "Skywater 130nm Pcells"
        self.layout().register_pcell("pfet", pfet())
        # self.layout().register_pcell("gone", gone())
        self.layout().register_pcell("vias_gen", vias_gen())   # vias generator
        self.layout().register_pcell("pfet_hv", pfet_hv())
        self.register("skywater130")

class Second(pya.Library):
    def __init__(self):
        self.description = "Second library"
        for name in names:
            self.layout().register_pcell(match.group(1), Wrapper(name))
        self.layout().register_pcell("via", via())
        self.register("second")
"#;

    #[test]
    fn libraries_out_of_python_sources() {
        let dir = tempfile::tempdir().unwrap();
        let cells = dir.path().join("python/cells");
        std::fs::create_dir_all(&cells).unwrap();
        std::fs::write(cells.join("fet.py"), FET).unwrap();
        std::fs::write(cells.join("__init__.py"), LIB).unwrap();
        let sources = from_sources(&[dir.path().to_path_buf()]);
        assert_eq!(sources.computed, 1);
        let libraries = &sources.libraries;
        assert_eq!(libraries.len(), 2);
        assert_eq!(libraries[1].name, "second");
        assert_eq!(libraries[1].description, "Second library");
        assert_eq!(libraries[1].pcells[0].name, "via");
        let library = &libraries[0];
        assert_eq!(library.name, "skywater130");
        assert_eq!(library.description, "Skywater 130nm Pcells");
        let names: Vec<&str> = library.pcells.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["pfet", "pfet_hv", "vias_gen"]);
        assert_eq!(
            library.pcells[1].parameters.len(),
            7,
            "a class without parameters of its own has its base's"
        );
        let pfet = &library.pcells[0];
        let by = |name: &str| pfet.parameters.iter().find(|p| p.name == name).unwrap();
        assert_eq!(pfet.parameters.len(), 7);
        assert_eq!(by("type").kind, "list");
        assert_eq!(
            by("type").choices,
            ["sky130_fd_pr__pfet_01v8", "sky130_fd_pr__pfet_01v8_lvt"]
        );
        assert_eq!(by("type").choice_labels[1], "lvt");
        assert_eq!(by("type").default, "sky130_fd_pr__pfet_01v8_lvt");
        assert_eq!(by("l").default, "0.15", "a constant of the file");
        assert_eq!(
            (by("w").kind.as_str(), by("w").unit.as_str()),
            ("double", "um")
        );
        assert_eq!(by("w").default, "fet_w", "an expression stays as written");
        assert_eq!(by("nf").default, "1");
        assert_eq!(by("gr").default, "true");
        assert_eq!(by("bulk").choices, ["None", "bulk tie"]);
        assert_eq!(by("bulk").default, "None");
        assert!(by("area").readonly && by("area").hidden);
        assert!(library.pcells[2].parameters.is_empty(), "class not found");
    }

    #[test]
    fn arguments_split_outside_brackets() {
        assert_eq!(
            split_arguments(r#""a, b", f(1, 2), [3, 4], x=(5, "6)")"#),
            [r#""a, b""#, "f(1, 2)", "[3, 4]", r#"x=(5, "6)")"#]
        );
        assert_eq!(balanced("a(b)c) rest"), Some("a(b)c"));
        assert_eq!(string_literal(" 'x' "), Some("x".into()));
        assert_eq!(string_literal("x"), None);
    }
}
