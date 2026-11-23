//! `cdept cace import`: a CACE datasheet becomes testbench views and a suite of a cell.
//!
//! Every parameter that CACE simulates (tool `ngspice`) becomes a testbench view (a
//! `setup.toml`); its conditions, merged with the datasheet's `default_conditions` the
//! way CACE merges them, become `corners`, `temperatures`, `[variables]`, `[[sweep]]`
//! or `[montecarlo]`, its spec limits `[[spec]]` bounds (scaled by their unit, as CACE
//! does). A suite view ties the testbenches together and keeps the datasheet header and
//! the parameters CDePT cannot run (DRC, LVS, area). What is not mapped is in the
//! notes of the plan and kept in the `[cace]` tables. Nothing is written until
//! [`apply`].

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use eyre::{Result, WrapErr, bail, eyre};

use super::{
    Mapping, Value, brief, condition_values, mapping, num_text, scalar_number, scalar_text,
    to_yaml, tool_of, unit_scale,
};
use crate::Project;
use crate::sim::setup::{SETUP_FILE, Setup};
use crate::sim::suite::SUITE_FILE;

/// What to import and where.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImportOptions {
    /// The library and cell the views go into (a `cells` library; the cell is made when
    /// it does not exist).
    pub library: String,
    pub cell: String,
    /// Put in front of every view name.
    pub prefix: String,
    /// `library/cell` of the schematic every testbench netlists. Empty: the cell's own.
    pub schematic: String,
    /// Replace views that exist (their `setup.toml` / `suite.toml`).
    pub overwrite: bool,
    /// How the datasheet is named in the files' comments (its path).
    pub source: String,
}

/// What a note of the plan is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// A field went into a CDePT field.
    Mapped,
    /// A field CDePT does not use: kept in a `[cace]` table, given back by an export.
    Kept,
    /// Something to look at: a guess, a name the technology does not have, a field
    /// whose meaning CDePT cannot reproduce.
    Warning,
}

impl Level {
    pub fn word(self) -> &'static str {
        match self {
            Level::Mapped => "mapped",
            Level::Kept => "kept",
            Level::Warning => "warning",
        }
    }
}

/// One line of the import report.
#[derive(Debug, Clone, PartialEq)]
pub struct Note {
    pub level: Level,
    /// The parameter, empty for the datasheet.
    pub parameter: String,
    pub text: String,
}

/// A file the import writes.
#[derive(Debug, Clone, PartialEq)]
pub struct PlannedFile {
    pub view: String,
    pub file: PathBuf,
    pub text: String,
    /// The view is there already (it is replaced with `overwrite`).
    pub exists: bool,
    /// What it is, for the plan.
    pub what: String,
}

/// The plan of an import.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImportPlan {
    pub library: String,
    pub cell: String,
    pub suite_view: String,
    /// `name (PDK), format, parameters`, for the first line.
    pub summary: String,
    pub files: Vec<PlannedFile>,
    pub notes: Vec<Note>,
    /// Why the import cannot be written.
    pub blockers: Vec<String>,
}

impl ImportPlan {
    pub fn ok(&self) -> bool {
        self.blockers.is_empty()
    }

    /// The plan as text: the files, then the report.
    pub fn lines(&self, root: &Path) -> Vec<String> {
        let mut out = vec![self.summary.clone()];
        for f in &self.files {
            let file = f
                .file
                .strip_prefix(root)
                .unwrap_or(&f.file)
                .display()
                .to_string();
            let verb = if f.exists { "replace" } else { "create" };
            out.push(format!("  {verb} {file}: {}", f.what));
        }
        for b in &self.blockers {
            out.push(format!("  blocked: {b}"));
        }
        for level in [Level::Mapped, Level::Kept, Level::Warning] {
            let notes: Vec<&Note> = self.notes.iter().filter(|n| n.level == level).collect();
            if notes.is_empty() {
                continue;
            }
            out.push(match level {
                Level::Mapped => "mapped:".into(),
                Level::Kept => {
                    "kept in [cace] (not used by CDePT, given back by an export):".into()
                }
                Level::Warning => "warnings:".into(),
            });
            for n in notes {
                let who = if n.parameter.is_empty() {
                    String::new()
                } else {
                    format!("{}: ", n.parameter)
                };
                out.push(format!("  {who}{}", n.text));
            }
        }
        out
    }
}

/// The values of the conditions of a parameter, as CDePT runs them, and where they went.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Mapped {
    pub corners: Vec<String>,
    pub temperatures: Vec<f64>,
    pub variables: BTreeMap<String, String>,
    pub sweeps: Vec<(String, Vec<String>)>,
    /// Monte Carlo: the number of runs and the statistical corner (empty: `mc`).
    pub mc_runs: Option<u32>,
    pub mc_corner: String,
    /// The condition the Monte Carlo runs are collated over (`iterations`).
    pub collate: String,
    /// Every mapped condition with its values as CDePT has them ([`setup_values`]).
    pub values: BTreeMap<String, Vec<String>>,
}

impl Mapped {
    /// How many runs the testbench makes.
    fn runs(&self) -> usize {
        let base = match self.mc_runs {
            Some(n) => n as usize,
            None => self.corners.len().max(1),
        };
        base * self.temperatures.len().max(1)
            * self
                .sweeps
                .iter()
                .map(|(_, v)| v.len().max(1))
                .product::<usize>()
    }
}

/// The values of a setup per CACE condition name (`corner`, `temperature`, a variable,
/// the collate condition of Monte Carlo), comparable with [`Mapped::values`].
pub(crate) fn setup_values(setup: &Setup, collate: &str) -> BTreeMap<String, Vec<String>> {
    let mut out = BTreeMap::new();
    if !setup.temperatures.is_empty() {
        out.insert(
            "temperature".to_string(),
            setup.temperatures.iter().map(|t| num_text(*t)).collect(),
        );
    }
    if let Some(mc) = &setup.montecarlo {
        let corner = if mc.corner.is_empty() {
            "mc".to_string()
        } else {
            mc.corner.clone()
        };
        out.insert("corner".to_string(), vec![corner]);
        let name = if collate.is_empty() {
            "iterations"
        } else {
            collate
        };
        out.insert(
            name.to_string(),
            (1..=mc.runs).map(|i| i.to_string()).collect(),
        );
    } else if !setup.corners.is_empty() {
        out.insert("corner".to_string(), setup.corners.clone());
    }
    for (k, v) in &setup.variables {
        out.insert(k.clone(), vec![v.clone()]);
    }
    for s in &setup.sweeps {
        if let Ok(values) = s.expand() {
            out.insert(s.variable.clone(), values);
        }
    }
    out
}

/// The corner sets of the project's technology (its default model set), if it has any.
pub(crate) fn known_corners(project: &Project) -> Option<Vec<String>> {
    let tech = project.tech.as_ref()?;
    let set = tech
        .models
        .default
        .clone()
        .unwrap_or_else(|| "ngspice".into());
    let corners = tech.models.corners(&set);
    (!corners.is_empty()).then(|| corners.into_iter().map(String::from).collect())
}

/// A CACE corner name as a corner set of the technology: the same name, the same in
/// another case, or the usual synonym (`typical` is `tt`). A warning when there is
/// none (the name is kept as written).
pub(crate) fn map_corner(name: &str, known: Option<&[String]>) -> (String, Option<String>) {
    let Some(known) = known else {
        return (name.to_string(), None);
    };
    if known.iter().any(|k| k == name) {
        return (name.to_string(), None);
    }
    if let Some(k) = known.iter().find(|k| k.eq_ignore_ascii_case(name)) {
        return (
            k.clone(),
            Some(format!("corner '{name}' is the technology's '{k}'")),
        );
    }
    let synonym = match name.to_ascii_lowercase().as_str() {
        "typical" | "typ" | "nom" | "nominal" => "tt",
        "fast" => "ff",
        "slow" => "ss",
        _ => "",
    };
    if !synonym.is_empty()
        && let Some(k) = known.iter().find(|k| k.as_str() == synonym)
    {
        return (
            k.clone(),
            Some(format!("corner '{name}' taken as the technology's '{k}'")),
        );
    }
    (
        name.to_string(),
        Some(format!(
            "corner '{name}' is not a corner set of the technology (it has {}); kept as written",
            known.join(", ")
        )),
    )
}

/// A condition of a parameter after merging: its name, its fields, whether the
/// parameter has it itself (else it comes from `default_conditions` only).
pub(crate) type Condition = (String, Mapping, bool);

const COND_INFO: &[&str] = &["description", "display", "unit"];

/// The conditions of a parameter as CACE merges them (`parameter_ngspice.py`): the
/// datasheet's default, with the parameter's description, display and unit over it and
/// the parameter's values instead of the default's when it has any. CACE only takes the
/// conditions its template uses; without the template every one is taken.
pub(crate) fn merged_conditions(
    defaults: Option<&Mapping>,
    own: Option<&Mapping>,
) -> Vec<Condition> {
    let mut out: Vec<Condition> = vec![];
    for (k, v) in defaults.into_iter().flatten() {
        if let (Some(name), Some(m)) = (scalar_text(k), v.as_mapping()) {
            out.push((name, m.clone(), false));
        }
    }
    for (k, v) in own.into_iter().flatten() {
        let (Some(name), Some(m)) = (scalar_text(k), v.as_mapping()) else {
            continue;
        };
        match out.iter_mut().find(|(n, _, _)| *n == name) {
            Some((_, merged, is_own)) => {
                *is_own = true;
                for key in COND_INFO {
                    if let Some(v) = m.get(*key) {
                        merged.insert(Value::from(*key), v.clone());
                    }
                }
                let has_values = m
                    .keys()
                    .any(|k| !COND_INFO.contains(&k.as_str().unwrap_or("")));
                if has_values {
                    merged.retain(|k, _| COND_INFO.contains(&k.as_str().unwrap_or("")));
                    for (k, v) in m {
                        if !COND_INFO.contains(&k.as_str().unwrap_or("")) {
                            merged.insert(k.clone(), v.clone());
                        }
                    }
                }
            }
            None => out.push((name, m.clone(), true)),
        }
    }
    out
}

/// The values of a condition as text, scaled by its unit as CACE does (`ib: 10` in
/// `uA` is `1e-5`).
pub(crate) fn scaled_values(cond: &Mapping) -> Vec<String> {
    let unit = cond.get("unit").and_then(scalar_text).unwrap_or_default();
    let scale = unit_scale(&unit);
    condition_values(cond)
        .iter()
        .filter_map(|v| {
            if scale != 1.0
                && let Some(n) = scalar_number(v)
            {
                return Some(num_text(n * scale));
            }
            match scalar_number(v) {
                Some(n) if !matches!(v, Value::String(_)) => Some(num_text(n)),
                _ => scalar_text(v),
            }
        })
        .collect()
}

fn word(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_alphanumeric() || c == '_')
}

/// Where the conditions of a parameter go in a setup.
pub(crate) fn map_conditions(
    conditions: &[Condition],
    tool_args: &Mapping,
    known: Option<&[String]>,
    notes: &mut Vec<String>,
    kept: &mut Vec<String>,
) -> Mapped {
    let mut m = Mapped::default();
    let collate = tool_args
        .get("collate")
        .and_then(scalar_text)
        .unwrap_or_default();
    let corner_values: Vec<String> = conditions
        .iter()
        .find(|(n, _, _)| n == "corner")
        .map(|(_, c, _)| scaled_values(c))
        .unwrap_or_default();
    let monte_carlo = corner_values.len() == 1 && corner_values[0].eq_ignore_ascii_case("mc");
    for (name, cond, _) in conditions {
        let values = scaled_values(cond);
        let unit = cond.get("unit").and_then(scalar_text).unwrap_or_default();
        let shown = |vals: &[String]| vals.join(", ");
        if values.is_empty() {
            kept.push(format!(
                "condition {name}: no value (only {})",
                brief(&Value::Mapping(cond.clone()))
            ));
            continue;
        }
        if name.contains('[') {
            kept.push(format!(
                "condition {name}: a bit vector, CDePT has no vector variables"
            ));
            continue;
        }
        match name.as_str() {
            "temperature" => {
                let temps: Vec<Option<f64>> = values.iter().map(|v| v.parse().ok()).collect();
                if temps.iter().any(Option::is_none) {
                    kept.push(format!(
                        "condition temperature: not numbers ({})",
                        shown(&values)
                    ));
                    continue;
                }
                m.temperatures = temps.into_iter().flatten().collect();
                m.values.insert(
                    name.clone(),
                    m.temperatures.iter().map(|t| num_text(*t)).collect(),
                );
                notes.push(format!("temperature {} -> temperatures", shown(&values)));
            }
            "corner" if monte_carlo => {
                let (corner, warning) = map_corner("mc", known);
                notes.extend(warning);
                m.mc_corner = if corner == "mc" {
                    String::new()
                } else {
                    corner.clone()
                };
                m.values.insert(name.clone(), vec![corner]);
            }
            "corner" => {
                let mut corners = vec![];
                for c in &values {
                    if c.eq_ignore_ascii_case("mc") {
                        notes.push(
                            "corner mc among other corners: left out (CDePT runs Monte Carlo as [montecarlo], not as a corner)"
                                .into(),
                        );
                        continue;
                    }
                    let (mapped, warning) = map_corner(c, known);
                    notes.extend(warning);
                    corners.push(mapped);
                }
                notes.push(format!("corner {} -> corners", shown(&values)));
                m.values.insert(name.clone(), corners.clone());
                m.corners = corners;
            }
            _ if monte_carlo && *name == collate => {
                m.mc_runs = Some(values.len() as u32);
                m.collate = collate.clone();
                notes.push(format!(
                    "corner mc collated over {name} ({} values) -> [montecarlo] runs = {}",
                    values.len(),
                    values.len()
                ));
                m.values.insert(
                    name.clone(),
                    (1..=values.len()).map(|i| i.to_string()).collect(),
                );
            }
            _ if !word(name) => {
                kept.push(format!(
                    "condition {name}: not a valid design-variable name"
                ));
            }
            _ => {
                let scaled = if unit_scale(&unit) != 1.0 {
                    format!(" ({unit} scaled to SI, as CACE does)")
                } else {
                    String::new()
                };
                if values.len() == 1 {
                    notes.push(format!("{name} = {} -> [variables]{scaled}", values[0]));
                    m.variables.insert(name.clone(), values[0].clone());
                } else {
                    notes.push(format!("{name} {} -> [[sweep]]{scaled}", shown(&values)));
                    m.sweeps.push((name.clone(), values.clone()));
                }
                m.values.insert(name.clone(), values);
            }
        }
    }
    if monte_carlo && m.mc_runs.is_none() {
        m.mc_runs = Some(1);
        notes.push("corner mc -> [montecarlo] runs = 1".into());
    }
    if !collate.is_empty() && !monte_carlo {
        notes.push(format!(
            "collate {collate} without corner mc: the values are separate runs (CACE merges them for its plots)"
        ));
    }
    m
}

/// A spec bound of a setup (SI) with the unit of its CACE result.
#[derive(Debug, Clone, PartialEq)]
pub struct SpecLine {
    pub name: String,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub unit: String,
}

/// CACE's defaults for the entries of a spec: `fail`, `calculation`, `limit`.
pub(crate) const SPEC_DEFAULTS: [(&str, bool, &str, &str); 3] = [
    ("minimum", true, "minimum", "above"),
    ("typical", false, "median", "exact"),
    ("maximum", true, "maximum", "below"),
];

/// The unit of a named result: its own, else the parameter's.
pub(crate) fn result_unit(param: &Mapping, result: &Mapping) -> String {
    result
        .get("unit")
        .and_then(scalar_text)
        .or_else(|| param.get("unit").and_then(scalar_text))
        .unwrap_or_default()
}

/// Whether an entry of a spec (`minimum`, ...) is a bound CDePT checks, and its value
/// in SI. `None`: not a bound (`any`, `fail: false`, absent, or another calculation).
pub(crate) fn bound_of(
    entry_name: &str,
    entry: Option<&Value>,
    scale: f64,
) -> Result<Option<f64>, String> {
    let Some(entry) = entry else { return Ok(None) };
    let Some(e) = entry.as_mapping() else {
        return Err(format!("{entry_name} is not a mapping"));
    };
    let (_, fail_default, calc_default, limit_default) = SPEC_DEFAULTS
        .iter()
        .find(|(n, ..)| *n == entry_name)
        .copied()
        .unwrap_or(("", false, "", ""));
    let value = e.get("value");
    if value.and_then(Value::as_str) == Some("any") || value.is_none() {
        return Ok(None);
    }
    let Some(v) = value.and_then(scalar_number) else {
        return Err(format!(
            "{entry_name} value {} is not a number",
            value.map(brief).unwrap_or_default()
        ));
    };
    let fail = e
        .get("fail")
        .and_then(Value::as_bool)
        .unwrap_or(fail_default);
    let calc = e
        .get("calculation")
        .and_then(scalar_text)
        .unwrap_or_else(|| calc_default.into());
    let limit = e
        .get("limit")
        .and_then(scalar_text)
        .unwrap_or_else(|| limit_default.into());
    if !fail {
        return Err(format!(
            "{entry_name} {} is not checked (fail: false)",
            num_text(v)
        ));
    }
    if calc != calc_default || limit != limit_default {
        return Err(format!(
            "{entry_name} {} with calculation {calc} / limit {limit}: CDePT checks every run against min/max",
            num_text(v)
        ));
    }
    if entry_name == "typical" {
        return Err(format!(
            "{entry_name} {} with fail: CDePT has no typical check",
            num_text(v)
        ));
    }
    Ok(Some(v * scale))
}

/// The `[[spec]]` lines of a parameter's spec.
pub(crate) fn map_specs(
    param: &Mapping,
    notes: &mut Vec<String>,
    kept: &mut Vec<String>,
    warnings: &mut Vec<String>,
) -> Vec<SpecLine> {
    let mut out = vec![];
    for (k, v) in mapping(param, "spec").into_iter().flatten() {
        let Some(name) = scalar_text(k) else { continue };
        let Some(result) = v.as_mapping() else {
            warnings.push(format!("spec {name} is not a mapping"));
            continue;
        };
        let unit = result_unit(param, result);
        let scale = unit_scale(&unit);
        let mut line = SpecLine {
            name: name.clone(),
            min: None,
            max: None,
            unit: unit.clone(),
        };
        for (entry, ..) in SPEC_DEFAULTS {
            match bound_of(entry, result.get(entry), scale) {
                Ok(Some(b)) if entry == "minimum" => line.min = Some(b),
                Ok(Some(b)) => line.max = Some(b),
                Ok(None) => {}
                Err(e) if e.contains("not checked") || e.contains("typical") => {
                    kept.push(format!("spec {name}: {e}"))
                }
                Err(e) => warnings.push(format!("spec {name}: {e}; not a CDePT bound (kept)")),
            }
        }
        let u = if unit.is_empty() {
            String::new()
        } else {
            format!(" [{unit}]")
        };
        if line.min.is_none() && line.max.is_none() {
            kept.push(format!(
                "spec {name}{u}: no limit CDePT checks (measured only)"
            ));
            continue;
        }
        let range = match (line.min, line.max) {
            (Some(a), Some(b)) => format!("{} .. {}", num_text(a), num_text(b)),
            (Some(a), None) => format!(">= {}", num_text(a)),
            (None, Some(b)) => format!("<= {}", num_text(b)),
            _ => String::new(),
        };
        notes.push(format!("spec {name}{u} -> [[spec]] {range}"));
        out.push(line);
    }
    out
}

/// A view name made of a parameter name.
fn view_name(prefix: &str, name: &str) -> String {
    let clean: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{prefix}{clean}")
}

fn toml_str(s: &str) -> String {
    toml::Value::String(s.to_string()).to_string()
}

fn toml_list<T: AsRef<str>>(items: &[T]) -> String {
    format!(
        "[{}]",
        items
            .iter()
            .map(|s| toml_str(s.as_ref()))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// A TOML float, short: `3e-10`, `50.0`, `-40.0`.
fn toml_float(v: f64) -> String {
    let t = num_text(v);
    if !v.is_finite() {
        toml::Value::Float(v).to_string()
    } else if t.contains(['.', 'e']) {
        t
    } else {
        format!("{t}.0")
    }
}

/// `[cace]` and what is under it, as TOML text.
fn cace_table(table: toml::Table) -> String {
    let mut doc = toml::Table::new();
    doc.insert("cace".into(), toml::Value::Table(table));
    toml::to_string_pretty(&doc).unwrap_or_default()
}

/// The `setup.toml` of a parameter.
#[allow(clippy::too_many_arguments)]
fn setup_text(
    source: &str,
    pname: &str,
    param: &Mapping,
    tool: &str,
    tool_args: &Mapping,
    m: &Mapped,
    specs: &[SpecLine],
    schematic: &str,
) -> String {
    let template = tool_args
        .get("template")
        .and_then(scalar_text)
        .unwrap_or_default();
    let measured: Vec<String> = mapping(param, "spec")
        .into_iter()
        .flatten()
        .filter_map(|(k, _)| scalar_text(k))
        .collect();
    let mut t = format!(
        "# Imported by `cdept cace import` from the CACE datasheet {source}, parameter `{pname}`.\n"
    );
    t.push_str(&format!(
        "# CACE simulates it with {tool}{}. This testbench netlists {}; its code blocks may run the\n",
        if template.is_empty() { String::new() } else { format!(" on the template {template}") },
        if schematic.is_empty() { "the schematic of this cell".to_string() } else { schematic.to_string() },
    ));
    if measured.is_empty() {
        t.push_str("# analyses.\n");
    } else {
        t.push_str(&format!(
            "# analyses; it must make the measurements {} (measures or [[output]]).\n",
            measured.join(", ")
        ));
    }
    t.push_str("# [cace] keeps the parameter for `cdept cace export`.\n");
    if !schematic.is_empty() {
        t.push_str(&format!("schematic = {}\n", toml_str(schematic)));
    }
    if !m.corners.is_empty() {
        t.push_str(&format!("corners = {}\n", toml_list(&m.corners)));
    }
    if !m.temperatures.is_empty() {
        let temps: Vec<String> = m.temperatures.iter().map(|v| toml_float(*v)).collect();
        t.push_str(&format!("temperatures = [{}]\n", temps.join(", ")));
    }
    if !m.variables.is_empty() {
        t.push_str("\n[variables]\n");
        for (k, v) in &m.variables {
            t.push_str(&format!("{k} = {}\n", toml_str(v)));
        }
    }
    for (name, values) in &m.sweeps {
        t.push_str(&format!(
            "\n[[sweep]]\nvariable = {}\nvalues = {}\n",
            toml_str(name),
            toml_list(values)
        ));
    }
    if let Some(runs) = m.mc_runs {
        t.push_str(&format!("\n[montecarlo]\nruns = {runs}\nseed = 1\n"));
        if !m.mc_corner.is_empty() {
            t.push_str(&format!("corner = {}\n", toml_str(&m.mc_corner)));
        }
    }
    for s in specs {
        t.push_str(&format!("\n[[spec]]\nname = {}\n", toml_str(&s.name)));
        if let Some(v) = s.min {
            t.push_str(&format!("min = {}\n", toml_float(v)));
        }
        if let Some(v) = s.max {
            t.push_str(&format!("max = {}\n", toml_float(v)));
        }
    }
    let mut cace = toml::Table::new();
    cace.insert("parameter".into(), toml::Value::String(pname.into()));
    if let Some(d) = param.get("display").and_then(scalar_text) {
        cace.insert("display".into(), toml::Value::String(d));
    }
    cace.insert("tool".into(), toml::Value::String(tool.into()));
    if !template.is_empty() {
        cace.insert("template".into(), toml::Value::String(template));
    }
    if !m.collate.is_empty() {
        cace.insert("collate".into(), toml::Value::String(m.collate.clone()));
    }
    let units: toml::Table = specs
        .iter()
        .filter(|s| !s.unit.is_empty())
        .map(|s| (s.name.clone(), toml::Value::String(s.unit.clone())))
        .collect();
    if !units.is_empty() {
        cace.insert("units".into(), toml::Value::Table(units));
    }
    cace.insert(
        "yaml".into(),
        toml::Value::String(to_yaml(&Value::Mapping(param.clone()))),
    );
    t.push('\n');
    t.push_str(&cace_table(cace));
    t
}

/// Plan the import of the datasheet `text` into `options.library` / `options.cell`.
pub fn plan(project: &Project, text: &str, options: &ImportOptions) -> Result<ImportPlan> {
    let sheet = super::parse(text)?;
    let lib = project
        .library(&options.library)
        .ok_or_else(|| eyre!("no library '{}'", options.library))?;
    if lib.read_only {
        bail!("library '{}' is read-only", options.library);
    }
    if lib.layout != crate::library::LibraryLayout::Cells {
        bail!(
            "library '{}' is not a cells library (views live in <cell>/<view>/)",
            options.library
        );
    }
    if !word(&options.cell.replace('-', "_")) {
        bail!("'{}' is not a valid cell name", options.cell);
    }
    let source = if options.source.is_empty() {
        "(text)".to_string()
    } else {
        options.source.clone()
    };
    let cell_dir = lib.path.join(&options.cell);
    let mut plan = ImportPlan {
        library: options.library.clone(),
        cell: options.cell.clone(),
        suite_view: view_name(&options.prefix, "datasheet"),
        ..Default::default()
    };
    let note = |plan: &mut ImportPlan, level, parameter: &str, text: String| {
        plan.notes.push(Note {
            level,
            parameter: parameter.into(),
            text,
        });
    };
    let name = sheet.get("name").and_then(scalar_text).unwrap_or_default();
    let pdk = sheet.get("PDK").and_then(scalar_text).unwrap_or_default();
    let format = sheet
        .get("cace_format")
        .and_then(scalar_text)
        .unwrap_or_default();
    let params = mapping(&sheet, "parameters").cloned().unwrap_or_default();
    plan.summary = format!(
        "CACE datasheet {source}: {} ({}), cace_format {}, {} parameter(s)",
        if name.is_empty() { "unnamed" } else { &name },
        if pdk.is_empty() { "no PDK" } else { &pdk },
        if format.is_empty() {
            "not given"
        } else {
            &format
        },
        params.len()
    );
    if !format.is_empty() && format != "5.2" {
        note(
            &mut plan,
            Level::Warning,
            "",
            format!("cace_format {format}: read as 5.2 (CACE 2.9's format)"),
        );
    }
    let tech_pdk = project
        .technology
        .clone()
        .or_else(|| project.tech.as_ref().map(|t| t.pdk.name.clone()));
    if let Some(t) = &tech_pdk
        && !pdk.is_empty()
        && !t.eq_ignore_ascii_case(&pdk)
    {
        note(
            &mut plan,
            Level::Warning,
            "",
            format!("the datasheet is for {pdk}, the project's technology is {t}"),
        );
    }
    for key in sheet.keys().filter_map(scalar_text) {
        let text = match key.as_str() {
            "parameters" => continue,
            "name" | "description" => continue,
            "default_conditions" => "default_conditions: merged into every testbench's conditions (CACE takes only those its template uses)".to_string(),
            other => format!("{other}: datasheet header, in the suite's [cace] datasheet"),
        };
        let level = if key == "default_conditions" {
            Level::Mapped
        } else {
            Level::Kept
        };
        note(&mut plan, level, "", text);
    }
    let known = known_corners(project);
    if known.is_none() {
        note(
            &mut plan,
            Level::Warning,
            "",
            "the project has no technology snapshot with corners: corner names are kept as written"
                .into(),
        );
    }
    let defaults = mapping(&sheet, "default_conditions");
    let has_schematic = lib.cell(&options.cell).is_some_and(|c| {
        c.entries
            .iter()
            .any(|e| e.format == Some(crate::entry::Format::XschemSchematic))
    });
    let mut tests: Vec<(String, String)> = vec![];
    let mut views = toml::Table::new();
    let mut other = toml::Table::new();
    let mut order: Vec<toml::Value> = vec![];
    let templates = crate::sim::template::TemplateSet::for_project(project);
    for (k, v) in &params {
        let Some(pname) = scalar_text(k) else {
            continue;
        };
        order.push(toml::Value::String(pname.clone()));
        let Some(param) = v.as_mapping() else {
            plan.blockers
                .push(format!("parameter {pname} is not a mapping"));
            continue;
        };
        let (tool, tool_args) = tool_of(param);
        if !super::SIM_TOOLS.contains(&tool.as_str()) {
            note(
                &mut plan,
                Level::Kept,
                &pname,
                format!(
                    "tool {}: not a simulation, no testbench (in the suite's [cace.other])",
                    if tool.is_empty() { "(none)" } else { &tool }
                ),
            );
            other.insert(pname.clone(), toml::Value::String(to_yaml(v)));
            continue;
        }
        let view = view_name(&options.prefix, &pname);
        let (mut mapped_notes, mut kept, mut warnings) = (vec![], vec![], vec![]);
        let conditions = merged_conditions(defaults, mapping(param, "conditions"));
        let m = map_conditions(
            &conditions,
            &tool_args,
            known.as_deref(),
            &mut mapped_notes,
            &mut kept,
        );
        let specs = map_specs(param, &mut mapped_notes, &mut kept, &mut warnings);
        for key in param.keys().filter_map(scalar_text) {
            match key.as_str() {
                "spec" | "conditions" | "tool" | "display" | "description" | "unit" => {}
                "status" => {
                    let s = param
                        .get("status")
                        .and_then(scalar_text)
                        .unwrap_or_default();
                    kept.push(format!(
                        "status {s}{}",
                        if s == "skip" {
                            ": not a test of the suite"
                        } else {
                            ""
                        }
                    ));
                }
                other => kept.push(format!(
                    "{other}: {}",
                    param.get(other).map(brief).unwrap_or_default()
                )),
            }
        }
        let args: Vec<String> = tool_args
            .keys()
            .filter_map(scalar_text)
            .filter(|k| k != "collate" || m.collate.is_empty())
            .collect();
        if !args.is_empty() {
            kept.push(format!("{tool} arguments {}", args.join(", ")));
        }
        if !specs.is_empty() || mapping(param, "spec").is_some_and(|s| !s.is_empty()) {
            let names: Vec<String> = mapping(param, "spec")
                .into_iter()
                .flatten()
                .filter_map(|(k, _)| scalar_text(k))
                .collect();
            warnings.push(format!(
                "the testbench must measure {} (CACE's template {} did): add measures or [[output]]",
                names.join(", "),
                tool_args.get("template").and_then(scalar_text).unwrap_or_else(|| "?".into())
            ));
        }
        let text = setup_text(
            &source,
            &pname,
            param,
            &tool,
            &tool_args,
            &m,
            &specs,
            &options.schematic,
        );
        if let Err(e) = Setup::parse_with(&text, &templates) {
            plan.blockers.push(format!(
                "{pname}: the setup.toml made of it does not load: {e:#}"
            ));
        }
        for t in mapped_notes {
            note(&mut plan, Level::Mapped, &pname, t);
        }
        for t in kept {
            note(&mut plan, Level::Kept, &pname, t);
        }
        for t in warnings {
            note(&mut plan, Level::Warning, &pname, t);
        }
        let dir = cell_dir.join(&view);
        let exists = dir.exists();
        if exists && !options.overwrite {
            plan.blockers.push(format!(
                "{}/{}/{view} exists (overwrite replaces it)",
                options.library, options.cell
            ));
        }
        plan.files.push(PlannedFile {
            view: view.clone(),
            file: dir.join(SETUP_FILE),
            text,
            exists,
            what: format!(
                "testbench of {pname}: {} run(s), {} spec(s){}",
                m.runs(),
                specs.len(),
                if m.mc_runs.is_some() {
                    ", Monte Carlo"
                } else {
                    ""
                }
            ),
        });
        views.insert(pname.clone(), toml::Value::String(view.clone()));
        let skip = param.get("status").and_then(Value::as_str) == Some("skip");
        if !skip {
            let label = param
                .get("display")
                .and_then(scalar_text)
                .unwrap_or_else(|| pname.clone());
            tests.push((view, label));
        }
    }
    if views.is_empty() {
        plan.blockers
            .push("no parameter is simulated with ngspice: there is no testbench to make".into());
    } else if tests.is_empty() {
        // Every simulated parameter is skipped; the suite lists them anyway.
        for (p, v) in &views {
            tests.push((v.as_str().unwrap_or(p).to_string(), p.clone()));
        }
    }
    if plan.files.iter().any(|f| f.view == plan.suite_view) {
        plan.blockers.push(format!(
            "a parameter has the name of the suite view '{}': use a prefix",
            plan.suite_view
        ));
    }
    if !options.schematic.is_empty() {
        if options.schematic.split('/').count() != 2 {
            plan.blockers.push(format!(
                "schematic '{}' is not library/cell",
                options.schematic
            ));
        }
    } else if !has_schematic && !views.is_empty() {
        note(
            &mut plan,
            Level::Warning,
            "",
            format!(
                "{}/{} has no xschem schematic: draw the testbench (CACE's templates are xschem files with CACE{{name}} placeholders; CDePT uses {{name}}) or give a schematic",
                options.library, options.cell
            ),
        );
    }
    // The suite.
    let mut header = sheet.clone();
    header.shift_remove("parameters");
    let description = sheet
        .get("description")
        .and_then(scalar_text)
        .unwrap_or_default();
    let mut s = format!(
        "# Imported by `cdept cace import` from the CACE datasheet {source}: a test per simulated\n# parameter. [cace] keeps the datasheet header and the other parameters for `cdept cace export`.\n"
    );
    s.push_str(&format!("description = {}\n", toml_str(&description)));
    for (view, label) in &tests {
        s.push_str(&format!(
            "\n[[test]]\nview = {}\nname = {}\n",
            toml_str(view),
            toml_str(label)
        ));
    }
    let mut cace = toml::Table::new();
    cace.insert("source".into(), toml::Value::String(source.clone()));
    cace.insert("parameters".into(), toml::Value::Array(order));
    cace.insert("views".into(), toml::Value::Table(views));
    if !other.is_empty() {
        cace.insert("other".into(), toml::Value::Table(other));
    }
    cace.insert(
        "datasheet".into(),
        toml::Value::String(to_yaml(&Value::Mapping(header))),
    );
    s.push('\n');
    s.push_str(&cace_table(cace));
    if !tests.is_empty()
        && let Err(e) = crate::sim::suite::Suite::parse(&s)
    {
        plan.blockers
            .push(format!("the suite.toml does not load: {e:#}"));
    }
    let dir = cell_dir.join(&plan.suite_view);
    let exists = dir.exists();
    if exists && !options.overwrite {
        plan.blockers.push(format!(
            "{}/{}/{} exists (overwrite replaces it)",
            options.library, options.cell, plan.suite_view
        ));
    }
    if !tests.is_empty() {
        plan.files.push(PlannedFile {
            view: plan.suite_view.clone(),
            file: dir.join(SUITE_FILE),
            text: s,
            exists,
            what: format!("suite of the datasheet: {} test(s)", tests.len()),
        });
    }
    Ok(plan)
}

/// [`plan`] of a datasheet file.
pub fn plan_file(project: &Project, file: &Path, options: &ImportOptions) -> Result<ImportPlan> {
    let text = std::fs::read_to_string(file)
        .wrap_err_with(|| format!("cannot read {}", file.display()))?;
    let mut options = options.clone();
    if options.source.is_empty() {
        options.source = file.display().to_string();
    }
    plan(project, &text, &options).wrap_err_with(|| file.display().to_string())
}

/// Write the files of a plan. The project must be scanned again to see the views.
pub fn apply(plan: &ImportPlan) -> Result<Vec<PathBuf>> {
    if !plan.ok() {
        bail!("cannot import: {}", plan.blockers.join("; "));
    }
    let mut written = vec![];
    for f in &plan.files {
        if let Some(dir) = f.file.parent() {
            std::fs::create_dir_all(dir)
                .wrap_err_with(|| format!("cannot create {}", dir.display()))?;
        }
        std::fs::write(&f.file, &f.text)
            .wrap_err_with(|| format!("cannot write {}", f.file.display()))?;
        written.push(f.file.clone());
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn yaml(text: &str) -> Mapping {
        serde_yaml_ng::from_str(text).unwrap()
    }

    #[test]
    fn conditions_merge_like_cace() {
        let defaults = yaml(
            "vdd:\n  display: Vdd\n  unit: V\n  typical: 1.8\ncorner:\n  typical: tt\nib:\n  unit: uA\n  typical: 10\n",
        );
        let own = yaml("vdd:\n  minimum: 1.7\n  maximum: 1.9\nTmax:\n  unit: ns\n  typical: 500\n");
        let merged = merged_conditions(Some(&defaults), Some(&own));
        let names: Vec<&str> = merged.iter().map(|(n, _, _)| n.as_str()).collect();
        assert_eq!(names, ["vdd", "corner", "ib", "Tmax"]);
        // The parameter's values replace the default's (typical is gone), the display stays.
        assert_eq!(scaled_values(&merged[0].1), ["1.7", "1.9"]);
        assert_eq!(
            merged[0].1.get("display").and_then(Value::as_str),
            Some("Vdd")
        );
        assert!(merged[0].2 && !merged[1].2);
        // Units scale the values.
        assert_eq!(scaled_values(&merged[2].1), ["1e-5"]);
        assert_eq!(scaled_values(&merged[3].1), ["5e-7"]);
    }

    #[test]
    fn corners_follow_the_technology() {
        let known = vec!["tt".to_string(), "ff".into(), "ss".into(), "mc".into()];
        assert_eq!(map_corner("tt", Some(&known)), ("tt".into(), None));
        assert_eq!(map_corner("TT", Some(&known)).0, "tt");
        assert_eq!(map_corner("typical", Some(&known)).0, "tt");
        let (c, w) = map_corner("fs", Some(&known));
        assert_eq!(c, "fs");
        assert!(w.unwrap().contains("not a corner set"));
        assert_eq!(map_corner("fs", None), ("fs".into(), None));
    }

    #[test]
    fn monte_carlo_is_collated_iterations() {
        let conds = merged_conditions(
            None,
            Some(&yaml(
                "iterations:\n  minimum: 1\n  maximum: 10\n  step: linear\n  stepsize: 1\ncorner:\n  typical: mc\ntemperature:\n  minimum: -40\n  typical: 27\n  maximum: 130\n",
            )),
        );
        let (mut n, mut k) = (vec![], vec![]);
        let m = map_conditions(
            &conds,
            &yaml("collate: iterations"),
            Some(&["tt".into(), "mc".into()]),
            &mut n,
            &mut k,
        );
        assert_eq!(m.mc_runs, Some(10));
        assert_eq!(m.mc_corner, "");
        assert_eq!(m.temperatures, [-40.0, 27.0, 130.0]);
        assert!(m.corners.is_empty() && m.sweeps.is_empty());
        assert_eq!(m.values["corner"], ["mc"]);
        assert_eq!(m.values["iterations"].len(), 10);
        assert_eq!(m.runs(), 30);
    }

    #[test]
    fn specs_scale_and_report() {
        let param = yaml(
            "unit: mV\nspec:\n  vos:\n    minimum: {value: -5}\n    maximum: {value: 5}\n  gain:\n    unit: V/V\n    minimum: {value: 50}\n    typical: {value: any}\n  noise:\n    maximum: {value: 2, fail: false}\n  odd:\n    maximum: {value: 3, calculation: average}\n",
        );
        let (mut n, mut k, mut w) = (vec![], vec![], vec![]);
        let specs = map_specs(&param, &mut n, &mut k, &mut w);
        assert_eq!(specs.len(), 2);
        assert_eq!((specs[0].min, specs[0].max), (Some(-0.005), Some(0.005)));
        assert_eq!(
            (specs[1].min, specs[1].max, specs[1].unit.as_str()),
            (Some(50.0), None, "V/V")
        );
        assert!(
            k.iter()
                .any(|t| t.contains("noise") && t.contains("fail: false")),
            "{k:?}"
        );
        assert!(
            w.iter().any(|t| t.contains("odd") && t.contains("average")),
            "{w:?}"
        );
    }
}
