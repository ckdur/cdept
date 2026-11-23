//! CACE datasheets: the spec table of a design in the format of CACE (the Circuit
//! Automatic Characterisation Engine, <https://github.com/fossi-foundation/cace>), read
//! into CDePT testbenches and a suite ([`import`]) and written back from them, with the
//! measured results ([`export`]).
//!
//! The format followed is the YAML datasheet of CACE 2.x, `cace_format: 5.2` (CACE
//! 2.9.0, `docs/source/reference_manual/datasheet_format.md` and `cace/common/cace_read.py`,
//! `cace/parameter/parameter.py`): a header (`name`, `description`, `PDK`,
//! `cace_format`, `authorship`, `paths`, `pins`, `default_conditions`) and
//! `parameters`, each with a `spec` (named results with `minimum` / `typical` /
//! `maximum` entries of `value`, `fail`, `calculation`, `limit`), a `tool` (`ngspice`
//! with a `template`, or a physical-verification tool), `conditions`, `variables` and
//! `plot`. The CACE 4.0 text format and its JSON form (`electrical_parameters`) are not
//! read. The mapping, field by field: docs/reference/files/cace.md.
//!
//! What CDePT does not use is kept: every testbench made from a parameter has a
//! `[cace]` table in its `setup.toml` with the parameter as CACE YAML, and the suite of
//! the datasheet keeps the header and the parameters that are not simulations. An
//! export starts from that YAML and puts in what the CDePT files say now.

pub mod export;
pub mod import;

use eyre::{Result, WrapErr, bail, eyre};
pub use serde_yaml_ng::{Mapping, Value};

/// The datasheet format version written by an export.
pub const FORMAT: f64 = 5.2;

/// The CACE tools that are simulations, imported as testbenches.
pub const SIM_TOOLS: &[&str] = &["ngspice"];

/// Read the text of a datasheet: a YAML mapping with `parameters`.
pub fn parse(text: &str) -> Result<Mapping> {
    let value: Value = serde_yaml_ng::from_str(text).wrap_err("not a CACE datasheet (YAML)")?;
    let Value::Mapping(map) = value else {
        bail!(
            "not a CACE datasheet: the YAML is not a mapping (the CACE 4.0 text format is not read; convert it with CACE 2.x)"
        );
    };
    if map.contains_key("electrical_parameters") {
        bail!(
            "a CACE 4.x datasheet (`electrical_parameters`): convert it to the YAML format 5.x with CACE 2.x first"
        );
    }
    match map.get("parameters") {
        Some(Value::Mapping(_)) => {}
        Some(_) => {
            return Err(eyre!(
                "not a CACE datasheet: `parameters` is not a mapping of parameters"
            ));
        }
        None => return Err(eyre!("not a CACE datasheet: no `parameters`")),
    }
    Ok(map)
}

/// `map[key]` as a mapping.
pub(crate) fn mapping<'a>(map: &'a Mapping, key: &str) -> Option<&'a Mapping> {
    map.get(key).and_then(Value::as_mapping)
}

/// A scalar as text: strings as they are, numbers without float noise.
pub(crate) fn scalar_text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Number(n) => Some(match n.as_i64() {
            Some(i) => i.to_string(),
            None => num_text(n.as_f64()?),
        }),
        _ => None,
    }
}

/// A scalar as a number: a YAML number or a string that is one (PyYAML, which CACE
/// uses, reads `1e6` as a string).
pub(crate) fn scalar_number(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// A number as short text: integers without a point, 12 significant digits otherwise,
/// an exponent for very small and very large values (`1e-12`, `2.5e6`).
pub fn num_text(v: f64) -> String {
    if !v.is_finite() {
        return v.to_string();
    }
    let v: f64 = format!("{v:.11e}").parse().unwrap_or(v);
    if v.fract() == 0.0 && v.abs() < 1e15 {
        return format!("{}", v as i64);
    }
    if v != 0.0 && (v.abs() < 1e-3 || v.abs() >= 1e6) {
        format!("{v:e}")
    } else {
        format!("{v}")
    }
}

/// A YAML scalar for a value CDePT keeps as text: a number when it is one, else a string.
pub(crate) fn yaml_scalar(text: &str) -> Value {
    if let Ok(i) = text.parse::<i64>() {
        return Value::from(i);
    }
    let letters = text
        .chars()
        .any(|c| c.is_alphabetic() && c != 'e' && c != 'E');
    match text.parse::<f64>() {
        Ok(f) if f.is_finite() && !letters => Value::from(f),
        _ => Value::from(text),
    }
}

/// A number for YAML: an integer when it is one.
pub(crate) fn yaml_number(v: f64) -> Value {
    let v: f64 = format!("{v:.11e}").parse().unwrap_or(v);
    if v.fract() == 0.0 && v.abs() < 1e15 {
        Value::from(v as i64)
    } else {
        Value::from(v)
    }
}

/// The tool of a parameter and its arguments: `tool: magic_area` or
/// `tool: {ngspice: {template: ac.sch, ...}}`.
pub fn tool_of(param: &Mapping) -> (String, Mapping) {
    match param.get("tool") {
        Some(Value::String(name)) => (name.clone(), Mapping::new()),
        Some(Value::Mapping(m)) => match m.iter().next() {
            Some((k, v)) => (
                scalar_text(k).unwrap_or_default(),
                v.as_mapping().cloned().unwrap_or_default(),
            ),
            None => (String::new(), Mapping::new()),
        },
        _ => (String::new(), Mapping::new()),
    }
}

/// The values CACE runs for a condition, in its order (`Condition.condition_gen` in
/// CACE's `parameter.py`): the `enumerate` list, then with `step` the typical value and
/// the linear or logarithmic range from minimum to maximum, else minimum, typical,
/// maximum, those present.
pub fn condition_values(cond: &Mapping) -> Vec<Value> {
    let mut out = vec![];
    if let Some(Value::Sequence(list)) = cond.get("enumerate") {
        out.extend(list.iter().cloned());
    }
    if let Some(step) = cond.get("step") {
        if let Some(t) = cond.get("typical") {
            out.push(t.clone());
        }
        let (Some(min), Some(max)) = (
            cond.get("minimum").and_then(scalar_number),
            cond.get("maximum").and_then(scalar_number),
        ) else {
            return out;
        };
        let logarithmic = step.as_str() == Some("logarithmic");
        let stepsize = cond
            .get("stepsize")
            .and_then(scalar_number)
            .unwrap_or(if logarithmic { 2.0 } else { 1.0 });
        let mut a = min;
        let mut n = 0;
        // linseq / logseq in CACE's common.py: the last value is clipped to the maximum.
        while n < 10_000
            && if logarithmic {
                a < max * stepsize
            } else {
                a < max + stepsize
            }
        {
            out.push(yaml_number(if a > max { max } else { a }));
            a = if logarithmic {
                a * stepsize
            } else {
                a + stepsize
            };
            if logarithmic && stepsize <= 1.0 || !logarithmic && stepsize <= 0.0 {
                break;
            }
            n += 1;
        }
    } else {
        for key in ["minimum", "typical", "maximum"] {
            if let Some(v) = cond.get(key) {
                out.push(v.clone());
            }
        }
    }
    out
}

/// The factor CACE multiplies a value with for its unit (`spice_unit_convert` in CACE's
/// `spiceunits.py`): `mV` is 1e-3, `uA` 1e-6, `V/V` 1, `µm²` 1e-12, `°C` 1. Units
/// CACE does not know (`dB`, `°`) are 1.
pub fn unit_scale(unit: &str) -> f64 {
    if let Some((a, b)) = unit.split_once('/') {
        return unit_scale(a) / unit_scale(b);
    }
    if unit.contains('\u{22c5}') {
        let mut parts = unit.split('\u{22c5}');
        let a = parts.next().unwrap_or("");
        let b = parts.next().unwrap_or("");
        return unit_scale(a) * unit_scale(b);
    }
    if let Some(i) = unit.find('\u{b2}') {
        let s = unit_scale(&unit[..i]);
        return s * s;
    }
    if unit.is_empty() || is_unit(unit) {
        return 1.0;
    }
    for (prefix, factor) in PREFIXES {
        if let Some(rest) = unit.strip_prefix(prefix)
            && is_unit(rest)
        {
            return *factor;
        }
    }
    if unit.starts_with('%') { 0.01 } else { 1.0 }
}

/// CACE's metric prefixes, in its order (the first that matches wins).
const PREFIXES: &[(&str, f64)] = &[
    ("T", 1e12),
    ("tera", 1e12),
    ("G", 1e9),
    ("giga", 1e9),
    ("M", 1e6),
    ("mega", 1e6),
    ("MEG", 1e6),
    ("meg", 1e6),
    ("K", 1e3),
    ("kilo", 1e3),
    ("k", 1e3),
    ("D", 1e1),
    ("deca", 1e1),
    ("d", 1e-1),
    ("deci", 1e-1),
    ("c", 1e-2),
    ("centi", 1e-2),
    ("%", 1e-2),
    ("m", 1e-3),
    ("milli", 1e-3),
    ("u", 1e-6),
    ("micro", 1e-6),
    ("\u{b5}", 1e-6),
    ("ppm", 1e-6),
    ("n", 1e-9),
    ("nano", 1e-9),
    ("ppb", 1e-9),
    ("p", 1e-12),
    ("pico", 1e-12),
    ("ppt", 1e-12),
    ("f", 1e-15),
    ("femto", 1e-15),
    ("a", 1e-18),
    ("atto", 1e-15),
];

/// One of CACE's unit names without a prefix (`unittypes` in `spiceunits.py`).
fn is_unit(s: &str) -> bool {
    // `[Xx]rest` with `[s]*` after it when `plural`.
    fn pat(s: &str, first: char, rest: &str, plural: bool) -> bool {
        let mut chars = s.chars();
        let Some(c) = chars.next() else { return false };
        if !c.eq_ignore_ascii_case(&first) {
            return false;
        }
        let tail = chars.as_str();
        match tail.strip_prefix(rest) {
            Some(more) if plural => more.chars().all(|c| c == 's'),
            Some(more) => more.is_empty(),
            None => false,
        }
    }
    let words: &[(char, &str, bool)] = &[
        ('f', "", false),
        ('f', "arad", true),
        ('o', "hm", true),
        ('v', "", false),
        ('v', "olt", true),
        ('a', "", false),
        ('a', "mp", true),
        ('a', "mpere", true),
        ('s', "", false),
        ('s', "econd", true),
        ('h', "", false),
        ('h', "enry", true),
        ('h', "enries", false),
        ('h', "z", false),
        ('h', "ertz", false),
        ('m', "", false),
        ('m', "eter", true),
        ('w', "", false),
        ('w', "att", true),
        ('v', "-rms", false),
    ];
    if s == "\u{3a9}" || words.iter().any(|(f, r, p)| pat(s, *f, r, *p)) {
        return true;
    }
    // [Vv]olt[s]*-rms
    if let Some(rest) = s.strip_suffix("-rms")
        && pat(rest, 'v', "olt", true)
    {
        return true;
    }
    // [°]*[Cc], [°]*[Cc]elsius, [°]*[Kk], [°]*[Kk]elvin
    let t = s.trim_start_matches('\u{b0}');
    if [('c', ""), ('c', "elsius"), ('k', ""), ('k', "elvin")]
        .iter()
        .any(|(f, r)| pat(t, *f, r, false))
    {
        return true;
    }
    // '[bohd]
    let mut chars = s.chars();
    chars.next() == Some('\'')
        && matches!(chars.next(), Some('b' | 'o' | 'h' | 'd'))
        && chars.next().is_none()
}

/// A YAML text of a value (a mapping), as CACE would read it.
pub(crate) fn to_yaml(v: &Value) -> String {
    serde_yaml_ng::to_string(v).unwrap_or_default()
}

/// A short form of a value for the report.
pub(crate) fn brief(v: &Value) -> String {
    match v {
        Value::Sequence(items) => {
            format!(
                "[{}]",
                items.iter().map(brief).collect::<Vec<_>>().join(", ")
            )
        }
        Value::Mapping(m) => format!(
            "{{{}}}",
            m.keys()
                .filter_map(scalar_text)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        other => scalar_text(other).unwrap_or_else(|| "null".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cond(text: &str) -> Mapping {
        serde_yaml_ng::from_str(text).unwrap()
    }

    fn texts(values: &[Value]) -> Vec<String> {
        values.iter().map(|v| scalar_text(v).unwrap()).collect()
    }

    #[test]
    fn condition_values_follow_cace() {
        assert_eq!(
            texts(&condition_values(&cond("enumerate: [ss, tt, ff]"))),
            ["ss", "tt", "ff"]
        );
        assert_eq!(
            texts(&condition_values(&cond(
                "minimum: -40\ntypical: 27\nmaximum: 130"
            ))),
            ["-40", "27", "130"]
        );
        assert_eq!(texts(&condition_values(&cond("maximum: 1"))), ["1"]);
        assert_eq!(
            texts(&condition_values(&cond(
                "minimum: 1\nmaximum: 4\nstep: linear\nstepsize: 1"
            ))),
            ["1", "2", "3", "4"]
        );
        // The last step is clipped to the maximum, the typical value comes first.
        assert_eq!(
            texts(&condition_values(&cond(
                "typical: 0.5\nminimum: 0\nmaximum: 1\nstep: linear\nstepsize: 0.3"
            ))),
            ["0.5", "0", "0.3", "0.6", "0.9", "1"]
        );
        assert_eq!(
            texts(&condition_values(&cond(
                "minimum: 1\nmaximum: 8\nstep: logarithmic"
            ))),
            ["1", "2", "4", "8"]
        );
    }

    #[test]
    fn units_scale_like_cace() {
        for (unit, scale) in [
            ("", 1.0),
            ("V", 1.0),
            ("mV", 1e-3),
            ("uA", 1e-6),
            ("pF", 1e-12),
            ("ns", 1e-9),
            ("MHz", 1e6),
            ("kHz", 1e3),
            ("V/V", 1.0),
            ("°", 1.0),
            ("°C", 1.0),
            ("dB", 1.0),
            ("µm²", 1e-12),
            ("%", 1e-2),
            ("m", 1.0),
            ("Volts-rms", 1.0),
        ] {
            assert!(
                (unit_scale(unit) - scale).abs() <= scale * 1e-12,
                "{unit}: {}",
                unit_scale(unit)
            );
        }
    }

    #[test]
    fn numbers_print_short() {
        assert_eq!(num_text(1e-12), "1e-12");
        assert_eq!(num_text(10.0 * 1e-6), "1e-5");
        assert_eq!(num_text(0.005), "0.005");
        assert_eq!(num_text(1e6), "1000000");
        assert_eq!(num_text(2.5e-7), "2.5e-7");
        assert_eq!(num_text(1.5e20), "1.5e20");
        assert_eq!(num_text(1234567.5), "1.2345675e6");
        assert_eq!(num_text(-40.0), "-40");
        assert_eq!(yaml_scalar("1.8"), Value::from(1.8));
        assert_eq!(yaml_scalar("27"), Value::from(27));
        assert_eq!(yaml_scalar("tt"), Value::from("tt"));
        assert_eq!(yaml_scalar("1e-5"), Value::from(1e-5));
    }

    #[test]
    fn rejects_what_is_not_a_datasheet() {
        assert!(parse("name: x\nparameters: {}\n").is_ok());
        assert!(parse("- a\n- b\n").is_err());
        assert!(parse("name: x\n").is_err());
        let e = parse("name: x\nelectrical_parameters: []\nparameters: {}\n").unwrap_err();
        assert!(format!("{e:#}").contains("4.x"), "{e:#}");
    }
}
