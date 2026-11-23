//! `cdept cace export`: a suite or a testbench as a CACE datasheet, with what its runs
//! measured.
//!
//! A testbench imported from a datasheet starts from the parameter kept in its `[cace]`
//! table: what CDePT has no field for (plots, tool arguments, result variables, status)
//! comes back as it was, and the conditions and spec limits are put in from the
//! `setup.toml` as it is now. A condition whose values did not change is written as it
//! was read (`minimum`/`typical`/`maximum`, `step`), one that changed as an `enumerate`
//! list (a `typical` for one value). A testbench without `[cace]` becomes a new
//! parameter. The results of the latest run of every point (the results store) are added
//! to each parameter as `results`, see [`results_of`].

use std::collections::BTreeMap;

use eyre::{Result, WrapErr, eyre};

use super::import::{
    SpecLine, bound_of, known_corners, map_conditions, merged_conditions, result_unit, setup_values,
};
use super::{Mapping, Value, mapping, num_text, scalar_text, to_yaml, tool_of, unit_scale, yaml_number, yaml_scalar};
use crate::Project;
use crate::sim::results::{Overall, RunRecord, Status, Store};
use crate::sim::setup::SETUP_FILE;
use crate::sim::suite::{self, SUITE_FILE};

/// What an export made.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Exported {
    /// The datasheet, YAML.
    pub text: String,
    /// The view it was made of and whether that is a suite.
    pub view: String,
    pub suite: bool,
    /// The parameters, in order.
    pub parameters: Vec<String>,
    /// What changed against the imported datasheet, what is missing.
    pub notes: Vec<String>,
}

/// How to export.
#[derive(Debug, Clone, PartialEq)]
pub struct ExportOptions {
    /// The suite or testbench view. None: the cell's only suite, else its only testbench.
    pub view: Option<String>,
    /// Add the measured results of the latest runs.
    pub results: bool,
}

impl Default for ExportOptions {
    fn default() -> Self {
        ExportOptions { view: None, results: true }
    }
}

/// The suite and testbench views of a cell.
fn views_of(project: &Project, library: &str, cell: &str) -> Result<(Vec<String>, Vec<String>)> {
    let c = project
        .library(library)
        .and_then(|l| l.cell(cell))
        .ok_or_else(|| eyre!("no cell {library}/{cell}"))?;
    let suites = c.entries.iter().filter(|e| e.path.join(SUITE_FILE).is_file()).map(|e| e.name.clone()).collect();
    let benches = c.entries.iter().filter(|e| e.path.join(SETUP_FILE).is_file()).map(|e| e.name.clone()).collect();
    Ok((suites, benches))
}

/// The view to export and whether it is a suite.
pub fn pick_view(project: &Project, library: &str, cell: &str, view: Option<&str>) -> Result<(String, bool)> {
    let (suites, benches) = views_of(project, library, cell)?;
    match view {
        Some(v) if suites.iter().any(|s| s == v) => Ok((v.to_string(), true)),
        Some(v) if benches.iter().any(|s| s == v) => Ok((v.to_string(), false)),
        Some(v) => Err(eyre!("{library}/{cell}/{v} is neither a suite nor a testbench")),
        None => match (suites.as_slice(), benches.as_slice()) {
            ([one], _) => Ok((one.clone(), true)),
            ([], [one]) => Ok((one.clone(), false)),
            ([], []) => Err(eyre!("{library}/{cell} has no suite and no testbench")),
            ([], many) => Err(eyre!("{library}/{cell} has several testbenches ({}): name one with --view", many.join(", "))),
            (many, _) => Err(eyre!("{library}/{cell} has several suites ({}): name one with --view", many.join(", "))),
        },
    }
}

/// A header for a datasheet that was not imported.
fn default_header(project: &Project, cell: &str, description: &str) -> Mapping {
    let pdk = project
        .technology
        .clone()
        .or_else(|| project.tech.as_ref().map(|t| t.pdk.name.clone()))
        .unwrap_or_default();
    let mut h = Mapping::new();
    h.insert("name".into(), cell.into());
    h.insert("description".into(), description.into());
    h.insert("PDK".into(), pdk.into());
    h.insert("cace_format".into(), Value::from(super::FORMAT));
    let mut paths = Mapping::new();
    for (k, v) in [("root", ".."), ("schematic", "xschem"), ("layout", "gds"), ("netlist", "netlist"), ("documentation", "docs")] {
        paths.insert(k.into(), v.into());
    }
    h.insert("paths".into(), Value::Mapping(paths));
    h
}

fn table_str<'a>(t: &'a toml::Table, key: &str) -> Option<&'a str> {
    t.get(key).and_then(toml::Value::as_str)
}

fn parse_yaml(text: &str, what: &str) -> Result<Mapping> {
    let v: Value = serde_yaml_ng::from_str(text).wrap_err_with(|| format!("{what}: the CACE YAML in [cace] does not read"))?;
    Ok(v.as_mapping().cloned().unwrap_or_default())
}

const VALUE_KEYS: &[&str] = &["minimum", "typical", "maximum", "enumerate", "step", "stepsize"];

/// A CDePT value (SI) for a condition with this unit.
fn condition_value(text: &str, unit: &str) -> Value {
    let scale = unit_scale(unit);
    match text.parse::<f64>() {
        Ok(v) if scale != 1.0 => yaml_number(v / scale),
        _ => yaml_scalar(text),
    }
}

/// The parameter of a testbench view.
fn parameter_of(
    project: &Project,
    (library, cell, view): (&str, &str, &str),
    defaults: Option<&Mapping>,
    results: bool,
    notes: &mut Vec<String>,
) -> Result<(String, Mapping)> {
    let entry = project
        .library(library)
        .and_then(|l| l.cell(cell))
        .and_then(|c| c.entry(view))
        .ok_or_else(|| eyre!("no view '{view}' of {library}/{cell}"))?;
    let setup = crate::sim::Setup::load_with(
        &entry.path.join(SETUP_FILE),
        &crate::sim::template::TemplateSet::for_project(project),
    )?;
    let setup = &setup;
    let cace = setup.cace.clone().unwrap_or_default();
    let id = format!("{library}/{cell}/{view}");
    let raw = match table_str(&cace, "yaml") {
        Some(text) => parse_yaml(text, &id)?,
        None => Mapping::new(),
    };
    let key = table_str(&cace, "parameter").unwrap_or(view).to_string();
    let mut param = raw.clone();
    if raw.is_empty() {
        param.insert("display".into(), view.into());
        param.insert("description".into(), format!("CDePT testbench {id}").into());
        let mut args = Mapping::new();
        // The schematic the testbench netlists, by its file name (the cell's when it has none yet).
        let template = crate::sim::open(project, library, cell, view)
            .ok()
            .and_then(|tb| tb.schematic.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| format!("{cell}.sch"));
        args.insert("template".into(), template.into());
        let mut tool = Mapping::new();
        tool.insert("ngspice".into(), Value::Mapping(args));
        param.insert("tool".into(), Value::Mapping(tool));
    }
    let (tool, mut args) = tool_of(&param);
    let collate = table_str(&cace, "collate")
        .map(String::from)
        .or_else(|| args.get("collate").and_then(scalar_text))
        .unwrap_or_default();

    // Conditions: as they were read where CDePT runs the same values.
    let own = mapping(&raw, "conditions").cloned();
    let merged = merged_conditions(defaults, own.as_ref());
    let known = known_corners(project);
    let expected = map_conditions(&merged, &args, known.as_deref(), &mut vec![], &mut vec![]).values;
    let current = setup_values(setup, &collate);
    let mut conds = own.clone().unwrap_or_default();
    let mut names: Vec<String> = merged.iter().map(|(n, _, _)| n.clone()).collect();
    let more: Vec<String> = current.keys().filter(|k| !names.contains(k)).cloned().collect();
    names.extend(more);
    let mc_collate = if collate.is_empty() { "iterations".to_string() } else { collate.clone() };
    for name in &names {
        let (exp, cur) = (expected.get(name), current.get(name));
        if exp == cur {
            continue;
        }
        let unit = merged
            .iter()
            .find(|(n, _, _)| n == name)
            .and_then(|(_, c, _)| c.get("unit").and_then(scalar_text))
            .unwrap_or_default();
        match cur {
            Some(values) => {
                let mut entry = conds.get(name.as_str()).and_then(Value::as_mapping).cloned().unwrap_or_default();
                entry.retain(|k, _| !VALUE_KEYS.contains(&k.as_str().unwrap_or("")));
                if setup.montecarlo.is_some() && *name == mc_collate {
                    entry.insert("minimum".into(), Value::from(1));
                    entry.insert("maximum".into(), Value::from(values.len() as i64));
                    entry.insert("step".into(), "linear".into());
                    entry.insert("stepsize".into(), Value::from(1));
                } else if values.len() == 1 {
                    entry.insert("typical".into(), condition_value(&values[0], &unit));
                } else {
                    let list: Vec<Value> = values.iter().map(|v| condition_value(v, &unit)).collect();
                    entry.insert("enumerate".into(), Value::Sequence(list));
                }
                conds.insert(name.as_str().into(), Value::Mapping(entry));
                notes.push(format!("{key}: condition {name} is {} in {id}", values.join(", ")));
            }
            None => {
                if conds.shift_remove(name.as_str()).is_some() {
                    notes.push(format!("{key}: condition {name} is not in {id} any more: left out"));
                } else {
                    notes.push(format!(
                        "{key}: {id} does not set {name}; CACE applies its default_conditions entry"
                    ));
                }
            }
        }
    }
    // Monte Carlo collates its runs.
    if setup.montecarlo.is_some() {
        args.insert("collate".into(), mc_collate.as_str().into());
    } else if !collate.is_empty() && expected.contains_key(&collate) && !current.contains_key(&collate) {
        args.shift_remove("collate");
    }
    if let Some(Value::Mapping(tools)) = param.get_mut("tool")
        && let Some(slot) = tools.get_mut(tool.as_str())
        && (!args.is_empty() || !slot.is_null())
    {
        *slot = Value::Mapping(args.clone());
    }
    if !conds.is_empty() || own.is_some() {
        param.insert("conditions".into(), Value::Mapping(conds));
    }

    // Spec limits: CDePT's bounds are the numeric limits that fail.
    let units = cace.get("units").and_then(toml::Value::as_table).cloned().unwrap_or_default();
    let mut spec = mapping(&raw, "spec").cloned().unwrap_or_default();
    let mut lines: Vec<SpecLine> = vec![];
    for (k, v) in spec.iter_mut() {
        let Some(name) = scalar_text(k) else { continue };
        let Some(result) = v.as_mapping_mut() else { continue };
        let read_unit = result_unit(&raw, result);
        let unit = units.get(&name).and_then(toml::Value::as_str).map(String::from).unwrap_or(read_unit.clone());
        if unit != read_unit {
            result.insert("unit".into(), unit.as_str().into());
        }
        let scale = unit_scale(&unit);
        let s = setup.specs.iter().find(|s| s.name.eq_ignore_ascii_case(&name));
        for (entry, bound) in [("minimum", s.and_then(|s| s.min)), ("maximum", s.and_then(|s| s.max))] {
            let before = bound_of(entry, result.get(entry), scale).ok().flatten();
            if before.map(num_text) == bound.map(num_text) {
                continue;
            }
            let mut e = result.get(entry).and_then(Value::as_mapping).cloned().unwrap_or_default();
            match bound {
                Some(b) => {
                    for k in ["fail", "calculation", "limit"] {
                        e.shift_remove(k);
                    }
                    e.insert("value".into(), yaml_number(b / scale));
                    notes.push(format!("{key}: spec {name} {entry} is {} in {id}", num_text(b / scale)));
                }
                None => {
                    e.insert("value".into(), "any".into());
                    notes.push(format!("{key}: spec {name} has no {entry} bound in {id}: any"));
                }
            }
            result.insert(entry.into(), Value::Mapping(e));
        }
        lines.push(SpecLine { name: name.clone(), min: s.and_then(|s| s.min), max: s.and_then(|s| s.max), unit });
    }
    for s in &setup.specs {
        if lines.iter().any(|l| l.name.eq_ignore_ascii_case(&s.name)) {
            continue;
        }
        let unit = units.get(&s.name).and_then(toml::Value::as_str).unwrap_or("").to_string();
        let scale = unit_scale(&unit);
        let mut result = Mapping::new();
        if !unit.is_empty() {
            result.insert("unit".into(), unit.as_str().into());
        }
        for (entry, bound) in [("minimum", s.min), ("maximum", s.max)] {
            if let Some(b) = bound {
                let mut e = Mapping::new();
                e.insert("value".into(), yaml_number(b / scale));
                result.insert(entry.into(), Value::Mapping(e));
            }
        }
        spec.insert(s.name.as_str().into(), Value::Mapping(result));
        lines.push(SpecLine { name: s.name.clone(), min: s.min, max: s.max, unit });
    }
    if !spec.is_empty() || raw.contains_key("spec") {
        param.insert("spec".into(), Value::Mapping(spec));
    }
    param.shift_remove("results");
    if results {
        let records = Store::open(&project.root)?.latest(library, cell, view)?;
        if records.is_empty() {
            notes.push(format!("{key}: {id} has not been run: no results"));
        } else {
            param.insert("results".into(), results_of(&records, &lines));
        }
    }
    Ok((key, param))
}

fn median(sorted: &[f64]) -> f64 {
    let n = sorted.len();
    if n % 2 == 1 { sorted[n / 2] } else { (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0 }
}

/// What the runs measured, per named result: minimum, typical (median, CACE's default
/// for `typical`) and maximum over the runs, in the result's unit, and `pass`/`fail` for
/// each bound; then the verdict of the parameter (`pass`, `fail` when a spec failed or
/// was not measured in some run, `error` when a run did not finish). One list entry,
/// named after the netlist source (`schematic`), like the `results` of CACE 4.0.
pub fn results_of(records: &[RunRecord], specs: &[SpecLine]) -> Value {
    let mut r = Mapping::new();
    r.insert("name".into(), "schematic".into());
    r.insert("tool".into(), "cdept".into());
    r.insert("runs".into(), Value::from(records.len() as i64));
    let overall = if records.iter().any(|x| x.overall() == Overall::Error) {
        "error"
    } else if records.iter().any(|x| x.overall() == Overall::Fail) {
        "fail"
    } else {
        "pass"
    };
    r.insert("status".into(), overall.into());
    for s in specs {
        let mut values: Vec<f64> = records
            .iter()
            .filter(|x| x.status == Status::Ok)
            .filter_map(|x| x.measures.get(&s.name.to_lowercase()).copied())
            .collect();
        values.sort_by(f64::total_cmp);
        let mut m = Mapping::new();
        if values.is_empty() {
            m.insert("status".into(), "missing".into());
            r.insert(s.name.as_str().into(), Value::Mapping(m));
            continue;
        }
        let scale = unit_scale(&s.unit);
        let (lo, mid, hi) = (values[0], median(&values), values[values.len() - 1]);
        let entry = |v: f64, check: Option<bool>| {
            let mut e = Mapping::new();
            e.insert("value".into(), yaml_number(v / scale));
            if let Some(ok) = check {
                e.insert("status".into(), if ok { "pass" } else { "fail" }.into());
            }
            Value::Mapping(e)
        };
        m.insert("minimum".into(), entry(lo, s.min.map(|b| lo >= b)));
        m.insert("typical".into(), entry(mid, None));
        m.insert("maximum".into(), entry(hi, s.max.map(|b| hi <= b)));
        if values.len() < records.len() {
            m.insert("missing".into(), Value::from((records.len() - values.len()) as i64));
        }
        r.insert(s.name.as_str().into(), Value::Mapping(m));
    }
    Value::Sequence(vec![Value::Mapping(r)])
}

/// The datasheet of a suite or testbench of `library/cell`.
pub fn export(project: &Project, library: &str, cell: &str, options: &ExportOptions) -> Result<Exported> {
    let (view, is_suite) = pick_view(project, library, cell, options.view.as_deref())?;
    let mut out = Exported { view: view.clone(), suite: is_suite, ..Default::default() };
    let mut params = Mapping::new();
    let add = |params: &mut Mapping, out: &mut Exported, key: String, p: Mapping| {
        let mut key = key;
        let mut n = 1;
        while params.contains_key(key.as_str()) {
            n += 1;
            key = format!("{key}_{n}");
        }
        out.parameters.push(key.clone());
        params.insert(key.into(), Value::Mapping(p));
    };
    let header = if is_suite {
        let s = suite::open(project, library, cell, &view)?;
        let cace = s.cace.clone().unwrap_or_default();
        let mut header = match table_str(&cace, "datasheet") {
            Some(text) => parse_yaml(text, &format!("{library}/{cell}/{view}"))?,
            None => default_header(project, cell, &s.description),
        };
        if !s.description.is_empty() && header.get("description").and_then(scalar_text).as_deref() != Some(&s.description) {
            header.insert("description".into(), s.description.as_str().into());
        }
        let defaults = mapping(&header, "default_conditions").cloned();
        let views: BTreeMap<String, String> = cace
            .get("views")
            .and_then(toml::Value::as_table)
            .map(|t| t.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect())
            .unwrap_or_default();
        let other = cace.get("other").and_then(toml::Value::as_table).cloned().unwrap_or_default();
        let order: Vec<String> = cace
            .get("parameters")
            .and_then(toml::Value::as_array)
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();
        let mut done: Vec<(String, String, String)> = vec![];
        for p in &order {
            if let Some(v) = views.get(p) {
                let at = (library.to_string(), cell.to_string(), v.clone());
                match parameter_of(project, (library, cell, v), defaults.as_ref(), options.results, &mut out.notes) {
                    Ok((key, param)) => add(&mut params, &mut out, key, param),
                    Err(e) => out.notes.push(format!("{p}: {library}/{cell}/{v} left out: {e:#}")),
                }
                done.push(at);
            } else if let Some(text) = other.get(p).and_then(toml::Value::as_str) {
                add(&mut params, &mut out, p.clone(), parse_yaml(text, p)?);
            } else {
                out.notes.push(format!("{p}: neither a testbench nor kept in [cace.other]: left out"));
            }
        }
        for t in &s.tests {
            let (l, c, v) = t.resolve(library, cell);
            if done.contains(&(l.clone(), c.clone(), v.clone())) {
                continue;
            }
            match parameter_of(project, (&l, &c, &v), defaults.as_ref(), options.results, &mut out.notes) {
                Ok((key, param)) => add(&mut params, &mut out, key, param),
                Err(e) => out.notes.push(format!("test {l}/{c}/{v} left out: {e:#}")),
            }
            done.push((l, c, v));
        }
        header
    } else {
        // A testbench of a suite that came from a datasheet takes that header.
        let (suites, _) = views_of(project, library, cell)?;
        let from_suite = suites.iter().find_map(|sv| {
            let s = suite::open(project, library, cell, sv).ok()?;
            let cace = s.cace?;
            let listed = cace.get("views")?.as_table()?.values().any(|v| v.as_str() == Some(view.as_str()));
            let text = cace.get("datasheet")?.as_str()?.to_string();
            listed.then_some(text)
        });
        let header = match from_suite {
            Some(text) => parse_yaml(&text, "datasheet")?,
            None => default_header(project, cell, &format!("CDePT testbench {library}/{cell}/{view}")),
        };
        let defaults = mapping(&header, "default_conditions").cloned();
        let (key, param) = parameter_of(project, (library, cell, &view), defaults.as_ref(), options.results, &mut out.notes)?;
        add(&mut params, &mut out, key, param);
        header
    };
    let mut doc = header;
    doc.shift_remove("parameters");
    doc.insert("parameters".into(), Value::Mapping(params));
    out.text = format!(
        "# CACE datasheet (cace_format 5.2) written by `cdept cace export` from {library}/{cell}/{view}.\n{}",
        to_yaml(&Value::Mapping(doc))
    );
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::results::PointRecord;

    fn record(point: &str, measures: &[(&str, f64)], fail: bool) -> RunRecord {
        RunRecord {
            schema: 1,
            id: None,
            library: "l".into(),
            cell: "c".into(),
            view: "v".into(),
            setup_hash: String::new(),
            deck_hash: String::new(),
            simulator: "ngspice".into(),
            point: PointRecord { name: point.into(), ..Default::default() },
            status: Status::Ok,
            error: None,
            measures: measures.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
            calc: vec![],
            calc_errors: Default::default(),
            specs: vec![crate::sim::results::SpecRecord {
                name: "a0".into(),
                value: None,
                min: None,
                max: None,
                verdict: if fail { crate::sim::spec::Verdict::Fail } else { crate::sim::spec::Verdict::Pass },
            }],
            started: String::new(),
            duration_ms: 0,
            dir: Default::default(),
            deck: None,
            raw: None,
            log: None,
        }
    }

    #[test]
    fn results_are_min_typical_max_with_verdicts() {
        let recs = vec![
            record("tt_27", &[("a0", 60.0), ("vos", 0.002)], false),
            record("ss_27", &[("a0", 45.0), ("vos", 0.004)], true),
            record("ff_27", &[("a0", 70.0)], false),
        ];
        let specs = vec![
            SpecLine { name: "a0".into(), min: Some(50.0), max: None, unit: "V/V".into() },
            SpecLine { name: "vos".into(), min: None, max: Some(0.005), unit: "mV".into() },
            SpecLine { name: "pm".into(), min: Some(60.0), max: None, unit: String::new() },
        ];
        let v = results_of(&recs, &specs);
        let r = v.as_sequence().unwrap()[0].as_mapping().unwrap();
        assert_eq!(r["status"], Value::from("fail"));
        assert_eq!(r["runs"], Value::from(3));
        let a0 = r["a0"].as_mapping().unwrap();
        assert_eq!(a0["minimum"]["value"], Value::from(45));
        assert_eq!(a0["minimum"]["status"], Value::from("fail"));
        assert_eq!(a0["typical"]["value"], Value::from(60));
        assert_eq!(a0["maximum"]["value"], Value::from(70));
        // In the result's unit, like the spec.
        let vos = r["vos"].as_mapping().unwrap();
        assert_eq!(vos["maximum"]["value"], Value::from(4));
        assert_eq!(vos["maximum"]["status"], Value::from("pass"));
        assert_eq!(vos["missing"], Value::from(1));
        assert_eq!(r["pm"]["status"], Value::from("missing"));
    }
}
