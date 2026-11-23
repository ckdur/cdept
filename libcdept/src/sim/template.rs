//! Test templates: a family of ready-made testbench set-ups ("measure the AC response",
//! "measure the delay", ...) that a `setup.toml` picks with `template = "ac_response"`.
//!
//! A template is a TOML document with its own schema (docs/TESTS.md):
//!
//! ```toml
//! name = "ac_response"
//! description = "..."
//! requires_nets = ["${in}", "${out}"]      # nets the netlist must have
//! plot = ['db20(v("${out}"))']             # what to show
//! [[param]]                                # typed, with default and doc
//! name = "out"
//! type = "net"
//! default = "out"
//! [[derived]]                              # computed from the parameters
//! name = "fin"
//! expr = "coherent(tone, fs, n)"
//! [setup]                                  # the setup.toml fragment, `${param}` inside strings
//! [[setup.analysis]]
//! kind = "ac"
//! ```
//!
//! [`resolve`] turns a `setup.toml` that names a template into the plain text
//! [`super::Setup`] reads: the template's fragment with the parameters filled in, then
//! the explicit fields of the setup laid over it (arrays of `analysis` by kind, `output`
//! and `spec` by name, `variables`/`options` key by key, everything else replaced).
//!
//! Built-in templates live in `templates/*.toml` of this crate (embedded); a project
//! adds `<project>/templates/*.toml` and a PDK `pdk/<tech>/templates/*.toml`; a later
//! source shadows an earlier one of the same name.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use eyre::{Result, WrapErr, bail, eyre};
use serde::{Deserialize, Serialize};

use crate::Project;
use crate::calc::{self, Env, Value};

/// Built-in templates: (file name, text).
const BUILTIN: &[(&str, &str)] = &[
    ("dc_op", include_str!("templates/dc_op.toml")),
    ("dc_transfer", include_str!("templates/dc_transfer.toml")),
    ("tran_delay", include_str!("templates/tran_delay.toml")),
    ("ac_response", include_str!("templates/ac_response.toml")),
    ("noise", include_str!("templates/noise.toml")),
    ("thd_fft", include_str!("templates/thd_fft.toml")),
    ("psrr_cmrr", include_str!("templates/psrr_cmrr.toml")),
    ("power", include_str!("templates/power.toml")),
    ("inl_dnl", include_str!("templates/inl_dnl.toml")),
    ("startup_settle", include_str!("templates/startup_settle.toml")),
];

/// Where a template came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    Builtin,
    /// `<project>/templates/`.
    Project(PathBuf),
    /// `pdk/<tech>/templates/`.
    Tech(PathBuf),
}

impl Origin {
    pub fn label(&self) -> String {
        match self {
            Origin::Builtin => "built-in".into(),
            Origin::Project(p) => format!("project: {}", p.display()),
            Origin::Tech(p) => format!("technology: {}", p.display()),
        }
    }
}

/// One parameter of a template.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Param {
    pub name: String,
    /// `net`, `string`, `number` (SPICE suffixes), `int`, `bool` or `choice`.
    #[serde(rename = "type")]
    pub kind: String,
    /// Absent: the parameter is required, unless `optional`.
    #[serde(deserialize_with = "scalar_text")]
    pub default: Option<String>,
    /// May be left unset: what uses it (a spec bound) is then dropped.
    pub optional: bool,
    pub choices: Vec<String>,
    pub unit: String,
    pub doc: String,
}

fn scalar_text<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Ok(match toml::Value::deserialize(d)? {
        toml::Value::String(s) => Some(s),
        toml::Value::Integer(i) => Some(i.to_string()),
        toml::Value::Float(f) => Some(crate::sim::setup::fmt_num(f)),
        toml::Value::Boolean(b) => Some(b.to_string()),
        other => return Err(serde::de::Error::custom(format!("expected a scalar, found {}", other.type_str()))),
    })
}

/// A value computed from the parameters.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Derived {
    pub name: String,
    /// A calculator expression over the parameters (numbers) and the earlier derived values.
    pub expr: String,
    /// Or: look the value of this parameter up in `map`.
    pub from: String,
    pub map: BTreeMap<String, String>,
    pub doc: String,
}

/// The template file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TemplateFile {
    pub name: String,
    pub description: String,
    /// What to look at after the run: calculator expressions or vectors.
    pub plot: Vec<String>,
    /// Nets the testbench's netlist must have.
    pub requires_nets: Vec<String>,
    /// Elements (sources, instances) the netlist must have.
    pub requires_elements: Vec<String>,
    /// The value of the input source of a starter schematic (`cdept sim new --dut`),
    /// SPICE with `{variable}` for design variables.
    pub stimulus: String,
    /// Design variables the starter's `setup.toml` defines for the stimulus.
    pub starter_variables: BTreeMap<String, String>,
    #[serde(rename = "param")]
    pub params: Vec<Param>,
    pub derived: Vec<Derived>,
    /// The `setup.toml` fragment.
    pub setup: toml::Table,
}

/// A loaded template.
#[derive(Debug, Clone)]
pub struct Template {
    pub file: TemplateFile,
    pub origin: Origin,
}

impl std::ops::Deref for Template {
    type Target = TemplateFile;
    fn deref(&self) -> &TemplateFile {
        &self.file
    }
}

/// Parameter values after binding: `None` is an optional one left unset.
pub type Bound = BTreeMap<String, Option<String>>;

fn word(s: &str) -> bool {
    !s.is_empty() && !s.contains(char::is_whitespace)
}

impl Template {
    pub fn parse(text: &str, origin: Origin) -> Result<Template> {
        let file: TemplateFile = toml::from_str(text).wrap_err("not a valid test template")?;
        let t = Template { file, origin };
        t.validate()?;
        Ok(t)
    }

    fn validate(&self) -> Result<()> {
        let name = &self.file.name;
        if !word(name) || !name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') {
            bail!("'{name}' is not a valid template name");
        }
        for (i, p) in self.file.params.iter().enumerate() {
            if !word(&p.name) || !p.name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                bail!("template {name}: '{}' is not a valid parameter name", p.name);
            }
            if self.file.params[..i].iter().any(|q| q.name == p.name) {
                bail!("template {name}: parameter '{}' is defined twice", p.name);
            }
            if !matches!(p.kind.as_str(), "" | "net" | "string" | "number" | "int" | "bool" | "choice") {
                bail!("template {name}: parameter '{}' has the unknown type '{}'", p.name, p.kind);
            }
            if p.kind == "choice" && p.choices.is_empty() {
                bail!("template {name}: choice parameter '{}' lists no choices", p.name);
            }
        }
        for d in &self.file.derived {
            if !word(&d.name) || self.file.params.iter().any(|p| p.name == d.name) {
                bail!("template {name}: derived value '{}' is not a valid new name", d.name);
            }
            if d.expr.is_empty() == d.from.is_empty() {
                bail!("template {name}: derived '{}' needs `expr` or `from` + `map`", d.name);
            }
            if !d.expr.is_empty()
                && let Err(e) = calc::parse(&d.expr)
            {
                bail!("template {name}: derived '{}': {}", d.name, e.render(&d.expr));
            }
        }
        // The fragment with every default (a sample for the required ones) must be a setup.
        let sample = self.sample_args();
        let plain = self.instantiate(&sample)?;
        super::Setup::parse_with(&toml::to_string(&plain.table)?, &TemplateSet::default())
            .wrap_err_with(|| format!("template {name}: its setup fragment is not valid"))?;
        Ok(())
    }

    /// Arguments that satisfy every required parameter (for checking the template).
    fn sample_args(&self) -> BTreeMap<String, String> {
        self.file
            .params
            .iter()
            .filter(|p| p.default.is_none() && !p.optional)
            .map(|p| {
                let v = match p.kind.as_str() {
                    "number" | "int" => "1".to_string(),
                    "bool" => "false".into(),
                    "choice" => p.choices[0].clone(),
                    _ => "x".into(),
                };
                (p.name.clone(), v)
            })
            .collect()
    }

    pub fn param(&self, name: &str) -> Option<&Param> {
        self.file.params.iter().find(|p| p.name == name)
    }

    /// Check one argument against its parameter (a GUI checks as the user types). An
    /// unset optional parameter is fine; a required one is only an error once the whole
    /// set is bound.
    pub fn validate_param(&self, name: &str, args: &BTreeMap<String, String>) -> Result<()> {
        let p = self.param(name).ok_or_else(|| eyre!("template '{}' has no parameter '{name}'", self.file.name))?;
        match args.get(name).map(|v| v.trim()).filter(|v| !v.is_empty()) {
            Some(v) => check_value(&self.file.name, p, v.to_string()).map(|_| ()).map_err(|e| {
                let m = format!("{e:#}");
                eyre!(m.split_once(" is not ").map_or(m.clone(), |(_, rest)| format!("not {rest}")))
            }),
            None => Ok(()),
        }
    }

    /// The analyses the template sets up (`tran`, `ac`, ...), from its fragment.
    pub fn analyses(&self) -> Vec<String> {
        let list = self.file.setup.get("analysis").and_then(|v| v.as_array());
        list.into_iter()
            .flatten()
            .filter_map(|a| a.get("kind")?.as_str().map(str::to_string))
            .collect()
    }

    /// Check the arguments against the parameters and fill in defaults and derived values.
    pub fn bind(&self, args: &BTreeMap<String, String>) -> Result<Bound> {
        let name = &self.file.name;
        for k in args.keys() {
            if self.param(k).is_none() {
                let known: Vec<&str> = self.file.params.iter().map(|p| p.name.as_str()).collect();
                let hint = closest(k, &known).map(|c| format!(" (did you mean '{c}'?)")).unwrap_or_default();
                bail!(
                    "template '{name}' has no parameter '{k}'{hint}; its parameters are: {}",
                    known.join(", ")
                );
            }
        }
        let mut bound: Bound = BTreeMap::new();
        for p in &self.file.params {
            let raw = match args.get(&p.name) {
                Some(v) => Some(v.clone()),
                None => match &p.default {
                    Some(d) => Some(substitute(d, &bound, name).wrap_err_with(|| format!("default of '{}'", p.name))?),
                    None if p.optional => None,
                    None => return Err(eyre!("template '{name}' needs the parameter '{}' ({})", p.name, describe(p))),
                },
            };
            let value = match raw {
                Some(v) if v.trim().is_empty() && p.optional => None,
                Some(v) => Some(check_value(name, p, v.trim().to_string())?),
                None => None,
            };
            bound.insert(p.name.clone(), value);
        }
        for d in &self.file.derived {
            let value = if !d.from.is_empty() {
                let Some(Some(src)) = bound.get(&d.from) else {
                    bail!("template '{name}': derived '{}' reads '{}', which is not set", d.name, d.from);
                };
                d.map.get(src).cloned().ok_or_else(|| {
                    eyre!(
                        "template '{name}': derived '{}' has no entry for {}='{src}' (it maps: {})",
                        d.name,
                        d.from,
                        d.map.keys().cloned().collect::<Vec<_>>().join(", ")
                    )
                })?
            } else {
                let mut env = Env::empty();
                for (k, v) in &bound {
                    if let Some(v) = v {
                        env.set_text(k, v);
                    }
                }
                let v = calc::evaluate(&d.expr, &env).map_err(|e| eyre!("template '{name}': derived '{}': {}", d.name, e.render(&d.expr)))?;
                let x = v.as_f64().filter(|x| x.is_finite()).ok_or_else(|| {
                    eyre!("template '{name}': derived '{}' is {}, not a number", d.name, v.kind())
                })?;
                fmt_spice(x)
            };
            bound.insert(d.name.clone(), Some(value));
        }
        Ok(bound)
    }

    /// The fragment with the arguments applied.
    pub fn instantiate(&self, args: &BTreeMap<String, String>) -> Result<Instance> {
        let bound = self.bind(args)?;
        let name = &self.file.name;
        let mut table = toml::Table::new();
        for (k, v) in &self.file.setup {
            if let Some(v) = fill(v, &bound, name, k)? {
                table.insert(k.clone(), v);
            }
        }
        // A spec whose bounds were all left unset is dropped.
        if let Some(toml::Value::Array(specs)) = table.get_mut("spec") {
            specs.retain(|s| s.get("min").is_some() || s.get("max").is_some());
            if specs.is_empty() {
                table.remove("spec");
            }
        }
        let list = |items: &[String]| -> Result<Vec<String>> {
            items.iter().map(|s| substitute(s, &bound, name)).collect()
        };
        Ok(Instance {
            table,
            requires_nets: list(&self.file.requires_nets)?,
            requires_elements: list(&self.file.requires_elements)?,
            plot: list(&self.file.plot)?,
            bound,
        })
    }
}

/// A template with its parameters filled in.
#[derive(Debug, Clone)]
pub struct Instance {
    pub table: toml::Table,
    pub requires_nets: Vec<String>,
    pub requires_elements: Vec<String>,
    pub plot: Vec<String>,
    pub bound: Bound,
}

fn describe(p: &Param) -> String {
    let kind = if p.kind.is_empty() { "string" } else { &p.kind };
    if p.doc.is_empty() { kind.to_string() } else { format!("{kind}: {}", p.doc) }
}

/// Validate and normalise a value of the type of `p`.
fn check_value(template: &str, p: &Param, v: String) -> Result<String> {
    let bad = |what: &str| eyre!("template '{template}': parameter '{}' = '{v}' is not {what}", p.name);
    match p.kind.as_str() {
        "net" => {
            if !word(&v) || v.contains(['(', ')', '"', '\'', ',', '=', '$', '{', '}']) {
                return Err(bad("a net name"));
            }
        }
        "number" => {
            if calc::parse_spice_number(&v).is_none() {
                return Err(bad("a number (SPICE suffixes such as 1k, 5p, 1meg are fine)"));
            }
        }
        "int" => {
            if v.parse::<i64>().is_err() {
                return Err(bad("an integer"));
            }
        }
        "bool" => {
            return match v.to_lowercase().as_str() {
                "true" | "yes" | "on" | "1" => Ok("true".into()),
                "false" | "no" | "off" | "0" => Ok("false".into()),
                _ => Err(bad("true or false")),
            };
        }
        "choice" if !p.choices.contains(&v) => {
            return Err(bad(&format!("one of {}", p.choices.join(", "))));
        }
        _ => {}
    }
    Ok(v)
}

/// A number as SPICE and the calculator read it.
fn fmt_spice(x: f64) -> String {
    if x.fract() == 0.0 && x.abs() < 1e15 {
        format!("{}", x as i64)
    } else if x.abs() >= 1e-3 && x.abs() < 1e9 {
        format!("{x}")
    } else {
        format!("{x:e}")
    }
}

/// Replace `${name}` in `s`.
fn substitute(s: &str, bound: &Bound, template: &str) -> Result<String> {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find("${") {
        out.push_str(&rest[..i]);
        let Some(j) = rest[i..].find('}') else {
            bail!("template '{template}': unclosed '${{' in \"{s}\"");
        };
        let key = &rest[i + 2..i + j];
        match bound.get(key) {
            Some(Some(v)) => out.push_str(v),
            Some(None) => return Err(eyre!("template '{template}': the parameter '{key}' is not set but \"{s}\" uses it")),
            None => return Err(eyre!("template '{template}': \"{s}\" uses ${{{key}}}, which is not a parameter")),
        }
        rest = &rest[i + j + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// The text of `${name}` when the whole string is one reference.
fn exact_ref(s: &str) -> Option<&str> {
    let t = s.trim();
    let inner = t.strip_prefix("${")?.strip_suffix('}')?;
    (!inner.contains(['$', '{', '}'])).then_some(inner)
}

/// Fill a fragment value. `None` drops it (an unset optional parameter alone in a value).
fn fill(v: &toml::Value, bound: &Bound, template: &str, key: &str) -> Result<Option<toml::Value>> {
    Ok(Some(match v {
        toml::Value::String(s) => {
            if let Some(name) = exact_ref(s)
                && let Some(value) = bound.get(name)
            {
                let Some(value) = value else { return Ok(None) };
                // Numbers keep their type where the setup wants a number.
                if matches!(key, "min" | "max" | "points") {
                    let x = calc::parse_spice_number(value)
                        .ok_or_else(|| eyre!("template '{template}': '{value}' (for {key}) is not a number"))?;
                    return Ok(Some(if key == "points" { toml::Value::Integer(x as i64) } else { toml::Value::Float(x) }));
                }
            }
            toml::Value::String(substitute(s, bound, template)?)
        }
        toml::Value::Array(items) => {
            let mut out = vec![];
            for item in items {
                let mut item = item.clone();
                if let toml::Value::Table(t) = &mut item
                    && let Some(cond) = t.remove("when")
                {
                    let cond = cond.as_str().ok_or_else(|| eyre!("template '{template}': `when` must be a string"))?;
                    if !condition(cond, bound, template)? {
                        continue;
                    }
                }
                if let Some(v) = fill(&item, bound, template, key)? {
                    out.push(v);
                }
            }
            toml::Value::Array(out)
        }
        toml::Value::Table(t) => {
            let mut out = toml::Table::new();
            for (k, i) in t {
                if let Some(i) = fill(i, bound, template, k)? {
                    out.insert(k.clone(), i);
                }
            }
            toml::Value::Table(out)
        }
        other => other.clone(),
    }))
}

/// `when` of a fragment entry: `name` (set and not false), `!name`, `name=value`,
/// `name!=value`.
fn condition(cond: &str, bound: &Bound, template: &str) -> Result<bool> {
    let cond = cond.trim();
    let get = |n: &str| -> Result<Option<&String>> {
        bound
            .get(n.trim())
            .map(|v| v.as_ref())
            .ok_or_else(|| eyre!("template '{template}': `when = \"{cond}\"` names '{n}', which is not a parameter"))
    };
    if let Some((n, v)) = cond.split_once("!=") {
        return Ok(get(n)?.map(String::as_str) != Some(v.trim()));
    }
    if let Some((n, v)) = cond.split_once('=') {
        return Ok(get(n)?.map(String::as_str) == Some(v.trim()));
    }
    let (negate, n) = match cond.strip_prefix('!') {
        Some(n) => (true, n),
        None => (false, cond),
    };
    let on = get(n)?.is_some_and(|v| !matches!(v.as_str(), "false" | "0" | "no" | "off"));
    Ok(on != negate)
}

/// The edit distance of two short names.
fn distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i];
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            cur.push((prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost));
        }
        prev = cur;
    }
    prev[b.len()]
}

fn closest<'a>(name: &str, names: &[&'a str]) -> Option<&'a str> {
    names.iter().map(|n| (distance(name, n), *n)).filter(|(d, _)| *d <= 2).min().map(|(_, n)| n)
}

/// The templates that can be picked.
#[derive(Debug, Clone, Default)]
pub struct TemplateSet {
    templates: Vec<Template>,
    /// Shadowed names and files that did not load.
    pub notes: Vec<String>,
}

impl TemplateSet {
    /// Only the built-in ones (cached).
    pub fn builtin() -> &'static TemplateSet {
        static SET: OnceLock<TemplateSet> = OnceLock::new();
        SET.get_or_init(|| {
            let mut set = TemplateSet::default();
            for (file, text) in BUILTIN {
                match Template::parse(text, Origin::Builtin) {
                    Ok(t) => set.add(t),
                    Err(e) => set.notes.push(format!("built-in template {file}: {e:#}")),
                }
            }
            set
        })
    }

    /// Built-in, then the technology's, then the project's.
    pub fn for_project(project: &Project) -> TemplateSet {
        let mut set = Self::builtin().clone();
        if let Some(tech) = project.technology.as_deref() {
            for dir in pdk_data_dirs() {
                set.load_dir(&dir.join(tech).join("templates"), Origin::Tech);
            }
        }
        set.load_dir(&project.root.join("templates"), Origin::Project);
        set
    }

    fn load_dir(&mut self, dir: &Path, origin: fn(PathBuf) -> Origin) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        let mut files: Vec<PathBuf> =
            rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "toml")).collect();
        files.sort();
        for f in files {
            let loaded = std::fs::read_to_string(&f)
                .map_err(eyre::Report::from)
                .and_then(|text| Template::parse(&text, origin(f.clone())));
            match loaded {
                Ok(t) => self.add(t),
                Err(e) => self.notes.push(format!("{}: {e:#}", f.display())),
            }
        }
    }

    fn add(&mut self, t: Template) {
        if let Some(old) = self.templates.iter().position(|o| o.file.name == t.file.name) {
            self.notes.push(format!(
                "template '{}' ({}) shadows the one from {}",
                t.file.name,
                t.origin.label(),
                self.templates[old].origin.label()
            ));
            self.templates[old] = t;
        } else {
            self.templates.push(t);
        }
    }

    /// Alphabetical.
    pub fn list(&self) -> Vec<&Template> {
        let mut v: Vec<&Template> = self.templates.iter().collect();
        v.sort_by(|a, b| a.file.name.cmp(&b.file.name));
        v
    }

    pub fn get(&self, name: &str) -> Option<&Template> {
        self.templates.iter().find(|t| t.file.name == name)
    }

    pub fn find(&self, name: &str) -> Result<&Template> {
        self.get(name).ok_or_else(|| {
            let names: Vec<&str> = self.templates.iter().map(|t| t.file.name.as_str()).collect();
            let hint = closest(name, &names).map(|c| format!(" (did you mean '{c}'?)")).unwrap_or_default();
            let mut s = format!("no test template '{name}'{hint}; there are: {}", {
                let mut n = names.clone();
                n.sort();
                n.join(", ")
            });
            for note in self.notes.iter().filter(|n| n.contains(name) && n.contains(".toml")) {
                s.push_str(&format!("\n  note: {note}"));
            }
            eyre!(s)
        })
    }
}

/// Folders that may hold `<tech>/templates`: the same ones as the device maps.
fn pdk_data_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
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
    dirs
}

/// The result of [`resolve`].
#[derive(Debug, Clone, Default)]
pub struct Resolved {
    /// The plain setup text (the input unchanged without a `template`).
    pub text: String,
    pub template: Option<String>,
    pub requires_nets: Vec<String>,
    pub requires_elements: Vec<String>,
    pub plot: Vec<String>,
}

/// The keys of `setup.toml` that belong to the template mechanism.
const OWN_KEYS: [&str; 2] = ["template", "template_args"];

/// If `text` names a template, make the plain setup text out of it.
pub fn resolve(text: &str, set: &TemplateSet) -> Result<Resolved> {
    let plain = || Resolved { text: text.to_string(), ..Default::default() };
    if !text.contains("template") {
        return Ok(plain());
    }
    // A syntax error is reported by the setup parser, with its own message.
    let Ok(user) = toml::from_str::<toml::Table>(text) else { return Ok(plain()) };
    let Some(name) = user.get("template") else { return Ok(plain()) };
    let name = name.as_str().ok_or_else(|| eyre!("`template` must be a name, e.g. template = \"ac_response\""))?;
    let template = set.find(name)?;
    let mut args = BTreeMap::new();
    match user.get("template_args") {
        None => {}
        Some(toml::Value::Table(t)) => {
            for (k, v) in t {
                let s = match v {
                    toml::Value::String(s) => s.clone(),
                    toml::Value::Integer(i) => i.to_string(),
                    toml::Value::Float(f) => crate::sim::setup::fmt_num(*f),
                    toml::Value::Boolean(b) => b.to_string(),
                    other => return Err(eyre!("template_args.{k}: expected a string, number or boolean, found {}", other.type_str())),
                };
                args.insert(k.clone(), s);
            }
        }
        Some(_) => return Err(eyre!("`template_args` must be a table: [template_args] out = \"out\"")),
    }
    let instance = template.instantiate(&args)?;
    let table = overlay(instance.table.clone(), &user);
    Ok(Resolved {
        text: toml::to_string(&table).wrap_err("cannot write the merged setup")?,
        template: Some(name.to_string()),
        requires_nets: instance.requires_nets,
        requires_elements: instance.requires_elements,
        plot: instance.plot,
    })
}

/// Lay the explicit fields of `user` over the template's fragment.
fn overlay(mut base: toml::Table, user: &toml::Table) -> toml::Table {
    use toml::Value;
    // Arrays of tables merged by a key; the user's entry replaces the template's.
    let by_key = |key: &'static str| -> Box<dyn Fn(&Value) -> String> {
        Box::new(move |v: &Value| v.get(key).and_then(|k| k.as_str()).unwrap_or("").to_lowercase())
    };
    for (k, v) in user {
        if k == "template" {
            continue;
        }
        let merged = match (k.as_str(), base.get(k), v) {
            ("analysis" | "output" | "spec" | "sweep", Some(Value::Array(old)), Value::Array(new)) => {
                let key = match k.as_str() {
                    "analysis" => by_key("kind"),
                    "sweep" => by_key("variable"),
                    _ => by_key("name"),
                };
                let mut out = old.clone();
                for n in new {
                    match out.iter().position(|o| key(o) == key(n)) {
                        Some(i) => out[i] = n.clone(),
                        None => out.push(n.clone()),
                    }
                }
                Value::Array(out)
            }
            ("variables" | "options", Some(Value::Table(old)), Value::Table(new)) => {
                let mut out = old.clone();
                out.extend(new.clone());
                Value::Table(out)
            }
            _ => v.clone(),
        };
        base.insert(k.clone(), merged);
    }
    for k in OWN_KEYS {
        if let Some(v) = user.get(k) {
            base.insert(k.into(), v.clone());
        }
    }
    base
}

/// The nets and elements at the top level of a SPICE netlist (lower case).
pub fn netlist_names(netlist: &str) -> (Vec<String>, Vec<String>) {
    let mut lines: Vec<String> = vec![];
    for raw in netlist.lines() {
        let l = raw.trim();
        if let Some(rest) = l.strip_prefix('+') {
            if let Some(last) = lines.last_mut() {
                last.push(' ');
                last.push_str(rest);
            }
        } else {
            lines.push(l.to_string());
        }
    }
    let (mut nets, mut elements) = (std::collections::BTreeSet::new(), std::collections::BTreeSet::new());
    let mut depth = 0usize;
    for l in &lines {
        let low = l.to_lowercase();
        if low.starts_with(".subckt") {
            depth += 1;
            continue;
        }
        if low.starts_with(".ends") {
            depth = depth.saturating_sub(1);
            continue;
        }
        if depth > 0 || low.is_empty() || low.starts_with(['*', '.', ';', '$']) {
            continue;
        }
        let toks: Vec<&str> = low.split_whitespace().collect();
        let Some(first) = toks.first() else { continue };
        elements.insert(first.to_string());
        let args: Vec<&str> = toks[1..].iter().copied().take_while(|t| !t.contains('=')).collect();
        let nodes: &[&str] = match first.chars().next() {
            Some('x') => &args[..args.len().saturating_sub(1)],
            Some('r' | 'c' | 'l' | 'v' | 'i' | 'd') => &args[..args.len().min(2)],
            Some('m') => &args[..args.len().min(4)],
            Some('q') => &args[..args.len().min(4)],
            _ => &args,
        };
        nets.extend(nodes.iter().map(|s| s.to_string()));
    }
    (nets.into_iter().collect(), elements.into_iter().collect())
}

/// Fail with a clear message when the netlist lacks what the template needs.
pub fn check_netlist(setup: &super::Setup, netlist: &str) -> Result<()> {
    if setup.requires_nets.is_empty() && setup.requires_elements.is_empty() {
        return Ok(());
    }
    let (nets, elements) = netlist_names(netlist);
    let name = if setup.template.is_empty() { "the testbench" } else { setup.template.as_str() };
    let mut problems = vec![];
    for n in &setup.requires_nets {
        let l = n.to_lowercase();
        if l != "0" && !nets.contains(&l) {
            problems.push(format!(
                "net '{n}' does not exist in the netlist (nets: {})",
                nets.iter().take(30).cloned().collect::<Vec<_>>().join(", ")
            ));
        }
    }
    for e in &setup.requires_elements {
        if !elements.contains(&e.to_lowercase()) {
            problems.push(format!(
                "element '{e}' does not exist in the netlist (elements: {})",
                elements.iter().take(30).cloned().collect::<Vec<_>>().join(", ")
            ));
        }
    }
    if problems.is_empty() {
        return Ok(());
    }
    Err(eyre!(
        "template '{name}' needs more than this schematic has: {}. Name the right ones in [template_args] of setup.toml.",
        problems.join("; ")
    ))
}

/// A `setup.toml` for a new testbench from a template.
pub fn starter_setup(template: &Template, args: &BTreeMap<String, String>, schematic: &str) -> Result<String> {
    // Check the arguments now, so a mistake is an error at `sim new`, not at the first run.
    template.bind(args)?;
    let mut out = format!(
        "# Testbench from the test template '{}' (docs/TESTS.md): {}\n# `cdept sim templates show {}` lists its parameters and outputs;\n# fields written here override the template's.\n",
        template.name, template.description, template.name
    );
    if !schematic.is_empty() {
        out.push_str(&format!("schematic = \"{schematic}\"\n"));
    }
    out.push_str(&format!("template = \"{}\"\n", template.name));
    if !args.is_empty() {
        out.push_str("\n[template_args]\n");
        for (k, v) in args {
            out.push_str(&format!("{k} = {}\n", toml::Value::String(v.clone())));
        }
    }
    Ok(out)
}

/// Text for `cdept sim templates`.
pub fn format_list(set: &TemplateSet) -> String {
    let mut out = String::new();
    let width = set.list().iter().map(|t| t.name.len()).max().unwrap_or(4);
    for t in set.list() {
        out.push_str(&format!("{:<width$}  {:<9}  {}\n", t.name, t.analyses().join("+"), t.description));
    }
    for n in &set.notes {
        out.push_str(&format!("note: {n}\n"));
    }
    out
}

/// Text for `cdept sim templates show NAME`.
pub fn format_show(t: &Template) -> Result<String> {
    let mut out = format!("{}  ({})\n{}\n", t.name, t.origin.label(), t.description);
    out.push_str("\nparameters\n");
    for p in &t.params {
        let kind = if p.kind.is_empty() { "string" } else { &p.kind };
        let default = match (&p.default, p.optional) {
            (Some(d), _) => format!("default {d}"),
            (None, true) => "optional".into(),
            (None, false) => "required".into(),
        };
        let unit = if p.unit.is_empty() { String::new() } else { format!(" [{}]", p.unit) };
        let choices = if p.choices.is_empty() { String::new() } else { format!(" ({})", p.choices.join("|")) };
        out.push_str(&format!("  {:<14} {kind}{choices}{unit}, {default}\n      {}\n", p.name, p.doc));
    }
    for d in &t.derived {
        out.push_str(&format!(
            "  {:<14} derived: {}\n      {}\n",
            d.name,
            if d.expr.is_empty() { format!("{} -> map", d.from) } else { d.expr.clone() },
            d.doc
        ));
    }
    let inst = t.instantiate(&t.sample_args())?;
    let setup = super::Setup::parse_with(&toml::to_string(&inst.table)?, &TemplateSet::default())?;
    out.push_str(&format!("\nanalyses: {}\n", if setup.analyses.is_empty() { "-".into() } else { t.analyses().join(", ") }));
    out.push_str("\noutputs (with the default parameters)\n");
    for o in &setup.outputs {
        let plot = if o.plot.is_empty() { String::new() } else { format!("   [plot {}]", o.plot) };
        out.push_str(&format!("  {:<14} {}{plot}\n", o.name, o.expr));
    }
    if !setup.measures.is_empty() {
        out.push_str("\nmeasures\n");
        for m in &setup.measures {
            out.push_str(&format!("  {m}\n"));
        }
    }
    out.push_str("\ndefault specs\n");
    if setup.specs.is_empty() {
        out.push_str("  none (set the *_min / *_max parameters, or add [[spec]] to setup.toml)\n");
    }
    for s in &setup.specs {
        out.push_str(&format!("  {:<14} min {:?} max {:?}\n", s.name, s.min, s.max));
    }
    if !inst.requires_nets.is_empty() || !inst.requires_elements.is_empty() {
        out.push_str(&format!(
            "\nthe schematic must have: nets {}; elements {}\n",
            inst.requires_nets.join(", "),
            inst.requires_elements.join(", ")
        ));
    }
    if !inst.plot.is_empty() {
        out.push_str(&format!("\nplot: {}\n", inst.plot.join("  ")));
    }
    let _ = Value::Num(0.0);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(texts: &[&str]) -> TemplateSet {
        let mut s = TemplateSet::default();
        for t in texts {
            s.add(Template::parse(t, Origin::Builtin).unwrap());
        }
        s
    }

    const TOY: &str = r#"
name = "toy"
description = "a toy"
requires_nets = ["${out}"]
requires_elements = ["${src}"]
plot = ['v("${out}")']
[[param]]
name = "out"
type = "net"
default = "out"
doc = "output net"
[[param]]
name = "src"
type = "string"
default = "vin"
[[param]]
name = "f"
type = "number"
default = "1k"
[[param]]
name = "gmin"
type = "number"
optional = true
[[param]]
name = "mode"
type = "choice"
choices = ["a", "b"]
default = "a"
[[derived]]
name = "f2"
expr = "f * 2"
[[derived]]
name = "edge"
from = "mode"
map = { a = "rise", b = "fall" }
[setup]
temperatures = [27]
[setup.variables]
fa = "${f}"
[[setup.analysis]]
kind = "ac"
points = 10
start = "1"
stop = "${f2}"
[[setup.output]]
name = "gain"
expr = 'db20(value_at(v("${out}"), ${f}))'
[[setup.output]]
name = "e"
expr = 'cross(v("${out}"), 0.5, "${edge}")'
[[setup.spec]]
name = "gain"
min = "${gmin}"
"#;

    #[test]
    fn substitutes_parameters_and_derived_values() {
        let s = set(&[TOY]);
        let r = resolve("template = \"toy\"\n[template_args]\nout = \"y\"\nf = 2000\nmode = \"b\"\n", &s).unwrap();
        let setup = super::super::Setup::parse_with("template = \"toy\"\n[template_args]\nout = \"y\"\nf = 2000\nmode = \"b\"\n", &s).unwrap();
        assert_eq!(setup.outputs[0].expr, "db20(value_at(v(\"y\"), 2000))");
        assert_eq!(setup.outputs[1].expr, "cross(v(\"y\"), 0.5, \"fall\")");
        assert!(matches!(&setup.analyses[0], crate::sim::Analysis::Ac { stop, points: 10, .. } if stop == "4000"));
        assert_eq!(setup.variables["fa"], "2000");
        // the optional bound is unset, so the spec is dropped
        assert!(setup.specs.is_empty());
        assert_eq!(r.requires_nets, ["y"]);
        assert_eq!(r.requires_elements, ["vin"]);
        assert_eq!(r.plot, ["v(\"y\")"]);
        assert_eq!(setup.template, "toy");
        assert_eq!(setup.requires_nets, ["y"]);
    }

    #[test]
    fn optional_spec_bound_becomes_a_number() {
        let s = set(&[TOY]);
        let setup =
            super::super::Setup::parse_with("template = \"toy\"\n[template_args]\ngmin = \"10\"\n", &s).unwrap();
        assert_eq!(setup.specs[0].min, Some(10.0));
    }

    #[test]
    fn explicit_fields_override_the_template() {
        let s = set(&[TOY]);
        let text = r#"
template = "toy"
temperatures = [-40, 125]
[variables]
extra = "1"
fa = "7"
[[output]]
name = "gain"
expr = "1"
[[output]]
name = "mine"
expr = "2"
[[analysis]]
kind = "ac"
points = 5
start = "1"
stop = "10"
[[analysis]]
kind = "op"
"#;
        let setup = super::super::Setup::parse_with(text, &s).unwrap();
        assert_eq!(setup.temperatures, [-40.0, 125.0]);
        assert_eq!(setup.variables["fa"], "7");
        assert_eq!(setup.variables["extra"], "1");
        let names: Vec<&str> = setup.outputs.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(names, ["gain", "e", "mine"]);
        assert_eq!(setup.outputs[0].expr, "1");
        assert_eq!(setup.analyses.len(), 2);
        assert!(matches!(&setup.analyses[0], crate::sim::Analysis::Ac { points: 5, .. }));
    }

    #[test]
    fn clear_errors() {
        let s = set(&[TOY]);
        let e = |t: &str| format!("{:#}", super::super::Setup::parse_with(t, &s).unwrap_err());
        let m = e("template = \"toi\"");
        assert!(m.contains("no test template 'toi'") && m.contains("did you mean 'toy'"), "{m}");
        let m = e("template = \"toy\"\n[template_args]\nouput = \"x\"");
        assert!(m.contains("no parameter 'ouput'") && m.contains("did you mean 'out'"), "{m}");
        let m = e("template = \"toy\"\n[template_args]\nf = \"fast\"");
        assert!(m.contains("not a number"), "{m}");
        let m = e("template = \"toy\"\n[template_args]\nmode = \"c\"");
        assert!(m.contains("one of a, b"), "{m}");
        let m = e("template = \"toy\"\n[template_args]\nout = \"v(x)\"");
        assert!(m.contains("not a net name"), "{m}");
        let m = e("template = 3");
        assert!(m.contains("must be a name"), "{m}");
        // an unknown field of the setup still fails
        assert!(e("template = \"toy\"\nbogus = 1").contains("bogus"));
        // a required parameter
        let r = set(&["name=\"r\"\n[[param]]\nname=\"a\"\ntype=\"net\"\n[setup]\nsave=[\"v(${a})\"]"]);
        let m = format!("{:#}", super::super::Setup::parse_with("template=\"r\"", &r).unwrap_err());
        assert!(m.contains("needs the parameter 'a'"), "{m}");
    }

    #[test]
    fn bad_templates_are_refused() {
        let t = |s: &str| format!("{:#}", Template::parse(s, Origin::Builtin).unwrap_err());
        assert!(t("name=\"a\"\n[setup]\nbogus=1").contains("not valid"));
        assert!(t("name=\"a\"\n[setup]\nsave=[\"${nope}\"]").contains("not a parameter"));
        assert!(t("name=\"a b\"").contains("valid template name"));
        assert!(t("name=\"a\"\n[[param]]\nname=\"x\"\ntype=\"weird\"").contains("unknown type"));
        assert!(t("name=\"a\"\n[[derived]]\nname=\"x\"").contains("needs `expr`"));
        assert!(t("name=\"a\"\n[[derived]]\nname=\"x\"\nexpr=\"max(\"").contains("derived 'x'"));
    }

    #[test]
    fn netlist_requirements() {
        let nl = "* t\nvdd vdd 0 1.2\nvin in 0 dc 0\nx1 in out vdd 0 inv\nC1 out 0 10f m=1\n.subckt inv a y vdd vss\nm1 y a vdd vdd p w=1\n.ends\n.end\n";
        let (nets, els) = netlist_names(nl);
        assert!(nets.contains(&"out".to_string()) && nets.contains(&"in".to_string()), "{nets:?}");
        assert!(!nets.contains(&"inv".to_string()) && !nets.contains(&"y".to_string()), "{nets:?}");
        assert!(els.contains(&"vin".to_string()));
        let s = set(&[TOY]);
        let setup = super::super::Setup::parse_with("template=\"toy\"", &s).unwrap();
        check_netlist(&setup, nl).unwrap();
        let setup = super::super::Setup::parse_with("template=\"toy\"\n[template_args]\nout=\"vout\"\nsrc=\"vx\"", &s).unwrap();
        let m = format!("{:#}", check_netlist(&setup, nl).unwrap_err());
        assert!(m.contains("net 'vout'") && m.contains("element 'vx'"), "{m}");
    }

    #[test]
    fn coherent_tone() {
        let mut env = Env::empty();
        env.set("x", Value::Num(1.0));
        let v = calc::evaluate("coherent(10.1meg, 100meg, 1024)", &env).unwrap().as_f64().unwrap();
        let m = v * 1024.0 / 100e6;
        assert!((m - m.round()).abs() < 1e-9 && (m.round() as u64) % 2 == 1, "{m}");
        assert!((v - 10e6).abs() <= 100e6 / 1024.0);
        assert!(calc::evaluate("coherent(60meg, 100meg, 1024)", &env).is_err());
    }

    #[test]
    fn project_templates_shadow_builtins() {
        let mut s = TemplateSet::builtin().clone();
        let t = Template::parse("name = \"dc_op\"\ndescription = \"mine\"", Origin::Project("x".into())).unwrap();
        s.add(t);
        assert_eq!(s.get("dc_op").unwrap().description, "mine");
        assert!(s.notes.iter().any(|n| n.contains("shadows")));
    }

    #[test]
    fn every_builtin_loads_with_its_defaults() {
        let set = TemplateSet::builtin();
        assert!(set.notes.is_empty(), "{:?}", set.notes);
        assert_eq!(set.list().len(), BUILTIN.len());
        for t in set.list() {
            format_show(t).unwrap_or_else(|e| panic!("{}: {e:#}", t.name));
            assert!(!t.description.is_empty(), "{}", t.name);
        }
    }
}
