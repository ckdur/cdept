//! SPICE to VACASK netlist: the subset of SPICE that xschem writes for a testbench.
//!
//! Supported: `R C L` (value, optional model, `m`), `V I` (dc, `ac`, `pulse` `sin`
//! `exp` `pwl`), `E G` (controlled sources), `D M Q` with `.model` cards (SPICE
//! levels 1 to 3 for MOS, diode, Gummel-Poon BJT through VACASK's `spice/*.osdi`),
//! `X` subcircuit calls, `N` OSDI instances, `.subckt` / `.ends` with parameters,
//! `.param`, `.model`, `.global`, continuation lines, comments, `{expr}` / `'expr'`
//! and SPICE number suffixes. Analyses, `.meas`, `.save`, `.options`, `.temp` and
//! `.control` blocks of the netlist are dropped with a warning (a testbench's
//! `setup.toml` says what to run). Anything else is an error that names the line,
//! never a silent skip: `B` sources, `K`, `F`/`H`, `.include`/`.lib` of ngspice model
//! files (the model set of the technology brings the converted ones), `.ic`,
//! `.nodeset`, `^` in expressions.
//!
//! Names keep their case (SPICE is case blind, VACASK is not); characters VACASK does
//! not take in a name become `_xNN`. Nodes `0` is ground.

use std::collections::{BTreeMap, BTreeSet};

use eyre::{Result, bail};

/// The name of the default models for plain `R`, `C`, `L`: `cdept_r`, `cdept_c`, `cdept_l`.
pub const DEFAULT_PREFIX: &str = "cdept_";

/// What a translation produced.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Translation {
    /// VACASK text: parameters, models, subcircuits, instances (no `load`, no control).
    pub text: String,
    /// VACASK's own OSDI files (`spice/resistor.osdi`, …) the text needs loaded.
    pub loads: BTreeSet<String>,
    /// What was dropped or changed on purpose.
    pub warnings: Vec<String>,
    /// Subcircuits the netlist defines.
    pub subckts_defined: BTreeSet<String>,
    /// Subcircuit names called by `X` instances, and where (first instance).
    pub subckts_used: BTreeMap<String, String>,
    /// Model names the instances name (`D M Q R C L N`) and `.model` does not define, and where.
    pub models_used: BTreeMap<String, String>,
    pub models_defined: BTreeSet<String>,
    /// Names given to `.global`.
    pub globals: Vec<String>,
}

/// A node or instance name VACASK takes.
pub fn name(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
        } else {
            out.push_str(&format!("_x{:02x}", c as u32));
        }
    }
    out
}

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// A SPICE expression as a VACASK one: braces and quotes dropped, numbers written
/// without suffix (`10f` is `10e-15`, `1meg` `1e6`), `temper` is `$temp`.
pub fn expr(s: &str) -> Result<String, String> {
    let s = s.trim();
    let mut text = String::new();
    for c in s.chars() {
        match c {
            '{' | '}' | '\'' => {}
            c => text.push(c),
        }
    }
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let prev_ident = i > 0 && (is_ident_char(chars[i - 1]) || chars[i - 1] == '$');
        let starts_number = c.is_ascii_digit() || (c == '.' && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit()));
        if starts_number && !prev_ident {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            let mant: String = chars[start..i].iter().collect();
            let mut exp: i32 = 0;
            if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
                let mut j = i + 1;
                if j < chars.len() && (chars[j] == '+' || chars[j] == '-') {
                    j += 1;
                }
                if j < chars.len() && chars[j].is_ascii_digit() {
                    let k = j;
                    while j < chars.len() && chars[j].is_ascii_digit() {
                        j += 1;
                    }
                    let digits: String = chars[k..j].iter().collect();
                    let sign = if chars[i + 1] == '-' { -1 } else { 1 };
                    exp = sign * digits.parse::<i32>().map_err(|e| format!("bad exponent in '{s}': {e}"))?;
                    i = j;
                }
            }
            let suffix_start = i;
            while i < chars.len() && chars[i].is_ascii_alphabetic() {
                i += 1;
            }
            let suffix: String = chars[suffix_start..i].iter().collect::<String>().to_lowercase();
            if suffix.starts_with("mil") {
                out.push_str(&format!("({mant}e{exp}*25.4e-6)"));
                continue;
            }
            let scale = if suffix.starts_with("meg") {
                6
            } else {
                match suffix.chars().next() {
                    Some('t') => 12,
                    Some('g') => 9,
                    Some('k') => 3,
                    Some('m') => -3,
                    Some('u') => -6,
                    Some('n') => -9,
                    Some('p') => -12,
                    Some('f') => -15,
                    Some('a') => -18,
                    _ => 0,
                }
            };
            let total = exp + scale;
            if total == 0 {
                out.push_str(&mant);
            } else {
                out.push_str(&format!("{mant}e{total}"));
            }
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && is_ident_char(chars[i]) {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            if word.eq_ignore_ascii_case("temper") {
                out.push_str("$temp");
            } else {
                out.push_str(&word);
            }
            continue;
        }
        if c == '^' {
            return Err(format!("'^' in '{s}': write pow(a, b)"));
        }
        out.push(c);
        i += 1;
    }
    Ok(out.trim().to_string())
}

/// Split a line at white space outside `{}`, `()` and `'…'`; `a = b` becomes `a=b`.
pub fn words(line: &str) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    let mut cur = String::new();
    let (mut brace, mut paren, mut quote) = (0i32, 0i32, false);
    for c in line.chars() {
        match c {
            '{' => brace += 1,
            '}' => brace -= 1,
            '(' => paren += 1,
            ')' => paren -= 1,
            '\'' => quote = !quote,
            _ => {}
        }
        if c.is_whitespace() && brace <= 0 && paren <= 0 && !quote {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    // `w = 1u`, `w =1u`, `w= 1u`
    let mut merged: Vec<String> = vec![];
    for w in out {
        match merged.last_mut() {
            Some(last) if last.ends_with('=') || w.starts_with('=') => last.push_str(&w),
            _ => merged.push(w),
        }
    }
    merged
}

/// `name=value` of a word: the name is an identifier.
fn assign(w: &str) -> Option<(&str, &str)> {
    let (n, v) = w.split_once('=')?;
    let ok = !n.is_empty()
        && n.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && n.chars().all(|c| is_ident_char(c) || c == '.' || c == '$');
    ok.then_some((n, v))
}

fn value_like(w: &str) -> bool {
    w.chars().next().is_some_and(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | '{' | '\'' | '('))
}

/// Logical lines of a netlist: continuation lines joined, comments removed. The first
/// line is the title and is dropped. Each comes with its 1-based line number.
fn logical_lines(netlist: &str) -> Vec<(usize, String)> {
    let mut out: Vec<(usize, String)> = vec![];
    for (i, raw) in netlist.lines().enumerate() {
        if i == 0 {
            continue;
        }
        let line = raw.trim_end();
        let t = line.trim_start();
        if t.is_empty() || t.starts_with('*') {
            continue;
        }
        // Inline comments: `;` and ` $`.
        let mut cut = t.len();
        if let Some(p) = t.find(';') {
            cut = cut.min(p);
        }
        if let Some(p) = t.find(" $") {
            cut = cut.min(p);
        }
        let t = t[..cut].trim_end();
        if let Some(rest) = t.strip_prefix('+') {
            if let Some((_, last)) = out.last_mut() {
                last.push(' ');
                last.push_str(rest.trim());
            }
            continue;
        }
        out.push((i + 1, t.to_string()));
    }
    out
}

struct Ctx {
    t: Translation,
    /// Lines of each section.
    params: Vec<String>,
    models: Vec<String>,
    subckts: Vec<String>,
    insts: Vec<String>,
    /// The subcircuit being defined.
    sub: Option<String>,
    default_models: BTreeSet<char>,
    /// The line being translated (for messages).
    line: usize,
}

impl Ctx {
    fn emit(&mut self, line: String) {
        if self.sub.is_some() {
            self.subckts.push(format!("  {line}"));
        } else {
            self.insts.push(line);
        }
    }

    fn fail<T>(&self, msg: impl std::fmt::Display) -> Result<T> {
        Err(eyre::eyre!("netlist line {}: {msg}", self.line))
    }

    fn e(&self, s: &str) -> Result<String> {
        expr(s).map_err(|m| eyre::eyre!("netlist line {}: {m}", self.line))
    }

    fn default_model(&mut self, letter: char) -> String {
        self.default_models.insert(letter);
        format!("{DEFAULT_PREFIX}{letter}")
    }

    fn use_model(&mut self, model: &str, inst: &str) {
        self.t.models_used.entry(model.to_string()).or_insert_with(|| inst.to_string());
    }
}

/// Params of an instance as `name=value` text: `m` is `$mfactor`, names lower case.
fn device_params(cx: &Ctx, kv: &[(String, String)], drop: &[&str], warn: &mut Vec<String>, inst: &str) -> Result<String> {
    let mut out = String::new();
    for (k, v) in kv {
        let k = k.to_lowercase();
        if drop.contains(&k.as_str()) {
            warn.push(format!("{inst}: '{k}' dropped (VACASK sets initial conditions another way)"));
            continue;
        }
        let key = if k == "m" || k == "_mfactor" { "$mfactor".to_string() } else { k };
        out.push_str(&format!(" {key}={}", cx.e(v)?));
    }
    Ok(out)
}

fn split_pos(rest: &[String]) -> (Vec<String>, Vec<(String, String)>) {
    let mut pos = vec![];
    let mut kv = vec![];
    for w in rest {
        match assign(w) {
            Some((n, v)) => kv.push((n.to_string(), v.to_string())),
            None => pos.push(w.clone()),
        }
    }
    (pos, kv)
}

/// The arguments of `pulse(...)` and friends: the keyword in lower case and the words.
fn source_function(rest: &str) -> Option<(String, Vec<String>, String)> {
    let lower = rest.to_lowercase();
    for kw in ["pulse", "sin", "exp", "pwl", "sffm", "am"] {
        let mut from = 0;
        while let Some(p) = lower[from..].find(kw) {
            let at = from + p;
            let before_ok = at == 0 || !lower[..at].chars().next_back().is_some_and(is_ident_char);
            let after = lower[at + kw.len()..].trim_start();
            if before_ok && after.starts_with('(') {
                let open = at + kw.len() + (lower[at + kw.len()..].len() - after.len());
                let mut depth = 0;
                for (j, c) in rest[open..].char_indices() {
                    match c {
                        '(' => depth += 1,
                        ')' => {
                            depth -= 1;
                            if depth == 0 {
                                let inner = &rest[open + 1..open + j];
                                let args: Vec<String> = words(&inner.replace(',', " "));
                                let others = format!("{} {}", &rest[..at], &rest[open + j + 1..]);
                                return Some((kw.to_string(), args, others));
                            }
                        }
                        _ => {}
                    }
                }
            }
            from = at + kw.len();
        }
    }
    None
}

fn source(cx: &mut Ctx, w: &[String], line: &str) -> Result<String> {
    let inst = name(&w[0]);
    if w.len() < 3 {
        return cx.fail(format!("source '{}' needs two nodes", w[0]));
    }
    let master = if w[0].to_lowercase().starts_with('v') { "vsource" } else { "isource" };
    let nodes = format!("{} {}", name(&w[1]), name(&w[2]));
    // Everything after the nodes.
    let mut rest = line.trim_start().to_string();
    for _ in 0..3 {
        rest = rest.trim_start().split_once(char::is_whitespace).map(|x| x.1.to_string()).unwrap_or_default();
    }
    let mut params: Vec<(String, String)> = vec![];
    let (func, others) = match source_function(&rest) {
        Some((kw, args, others)) => (Some((kw, args)), others),
        None => (None, rest.clone()),
    };
    let tokens = words(&others);
    let mut i = 0;
    let mut dc: Option<String> = None;
    while i < tokens.len() {
        let t = tokens[i].to_lowercase();
        if t == "dc" {
            if let Some(v) = tokens.get(i + 1) {
                dc = Some(cx.e(v)?);
                i += 2;
                continue;
            }
            return cx.fail("'dc' without a value");
        } else if t == "ac" {
            let mut j = i + 1;
            if let Some(v) = tokens.get(j).filter(|v| value_like(v)) {
                params.push(("mag".into(), cx.e(v)?));
                j += 1;
                if let Some(p) = tokens.get(j).filter(|v| value_like(v)) {
                    params.push(("phase".into(), cx.e(p)?));
                    j += 1;
                }
            } else {
                params.push(("mag".into(), "1".into()));
            }
            i = j;
            continue;
        } else if let Some((n, v)) = assign(&tokens[i]) {
            if n.eq_ignore_ascii_case("dc") {
                dc = Some(cx.e(v)?);
            } else if n.eq_ignore_ascii_case("m") {
                params.push(("$mfactor".into(), cx.e(v)?));
            } else {
                return cx.fail(format!("source parameter '{n}' is not supported"));
            }
        } else if value_like(&tokens[i]) && dc.is_none() {
            dc = Some(cx.e(&tokens[i])?);
        } else {
            return cx.fail(format!("cannot read '{}' in the source '{}'", tokens[i], w[0]));
        }
        i += 1;
    }
    let mut wave: Vec<(String, String)> = vec![];
    if let Some((kw, args)) = func {
        let a: Vec<String> = args.iter().map(|x| cx.e(x)).collect::<Result<_>>()?;
        let get = |i: usize, default: Option<&str>| -> Result<String> {
            a.get(i).cloned().or_else(|| default.map(String::from)).ok_or_else(|| {
                eyre::eyre!("netlist line {}: {kw}(...) needs at least {} values", cx.line, i + 1)
            })
        };
        match kw.as_str() {
            "pulse" => {
                if a.len() < 7 {
                    return cx.fail("pulse(...) needs v1 v2 td tr tf pw per (VACASK has no default rise, fall, width or period)");
                }
                wave.push(("type".into(), "\"pulse\"".into()));
                wave.push(("val0".into(), get(0, None)?));
                wave.push(("val1".into(), get(1, None)?));
                wave.push(("delay".into(), get(2, None)?));
                wave.push(("rise".into(), get(3, None)?));
                wave.push(("fall".into(), get(4, None)?));
                wave.push(("width".into(), get(5, None)?));
                wave.push(("period".into(), get(6, None)?));
                if dc.is_none() {
                    dc = Some(get(0, None)?);
                }
            }
            "sin" => {
                wave.push(("type".into(), "\"sine\"".into()));
                wave.push(("sinedc".into(), get(0, None)?));
                wave.push(("ampl".into(), get(1, None)?));
                wave.push(("freq".into(), get(2, None)?));
                if a.len() > 3 {
                    wave.push(("delay".into(), get(3, None)?));
                }
                if a.len() > 4 {
                    wave.push(("theta".into(), get(4, None)?));
                }
                if a.len() > 5 {
                    wave.push(("sinephase".into(), get(5, None)?));
                }
                if dc.is_none() {
                    dc = Some(get(0, None)?);
                }
            }
            "exp" => {
                if a.len() < 6 {
                    return cx.fail("exp(...) needs v1 v2 td1 tau1 td2 tau2");
                }
                wave.push(("type".into(), "\"exp\"".into()));
                wave.push(("val0".into(), get(0, None)?));
                wave.push(("val1".into(), get(1, None)?));
                wave.push(("delay".into(), get(2, None)?));
                wave.push(("tau1".into(), get(3, None)?));
                wave.push(("td2".into(), get(4, None)?));
                wave.push(("tau2".into(), get(5, None)?));
                if dc.is_none() {
                    dc = Some(get(0, None)?);
                }
            }
            "pwl" => {
                if a.len() < 2 || !a.len().is_multiple_of(2) {
                    return cx.fail("pwl(...) needs time/value pairs");
                }
                wave.push(("type".into(), "\"pwl\"".into()));
                wave.push(("wave".into(), format!("[{}]", a.join(" "))));
                if dc.is_none() {
                    dc = Some(a[1].clone());
                }
            }
            other => return cx.fail(format!("source function '{other}' is not supported")),
        }
    }
    let mut text = format!("{inst} ({nodes}) {master}");
    if let Some(d) = dc {
        text.push_str(&format!(" dc={d}"));
    }
    for (k, v) in params.iter().chain(&wave) {
        text.push_str(&format!(" {k}={v}"));
    }
    Ok(text)
}

/// SPICE model types to VACASK modules.
fn model_module(kind: &str, level: Option<f64>) -> Option<(&'static str, &'static str, Option<&'static str>)> {
    // (osdi file, module, type parameter)
    Some(match (kind, level.map(|l| l as i64)) {
        ("d", _) => ("spice/diode.osdi", "sp_diode", None),
        ("npn", None | Some(1)) => ("spice/bjt.osdi", "sp_bjt", Some("1")),
        ("pnp", None | Some(1)) => ("spice/bjt.osdi", "sp_bjt", Some("-1")),
        ("nmos", None | Some(1)) => ("spice/mos1.osdi", "sp_mos1", Some("1")),
        ("pmos", None | Some(1)) => ("spice/mos1.osdi", "sp_mos1", Some("-1")),
        ("nmos", Some(2)) => ("spice/mos2.osdi", "sp_mos2", Some("1")),
        ("pmos", Some(2)) => ("spice/mos2.osdi", "sp_mos2", Some("-1")),
        ("nmos", Some(3)) => ("spice/mos3.osdi", "sp_mos3", Some("1")),
        ("pmos", Some(3)) => ("spice/mos3.osdi", "sp_mos3", Some("-1")),
        ("nmos", Some(8 | 49)) => ("spice/bsim3v3.osdi", "sp_bsim3v3", Some("1")),
        ("pmos", Some(8 | 49)) => ("spice/bsim3v3.osdi", "sp_bsim3v3", Some("-1")),
        ("nmos", Some(14 | 54)) => ("spice/bsim4v8.osdi", "sp_bsim4v8", Some("1")),
        ("pmos", Some(14 | 54)) => ("spice/bsim4v8.osdi", "sp_bsim4v8", Some("-1")),
        _ => return None,
    })
}

fn model_card(cx: &mut Ctx, line: &str) -> Result<()> {
    let body = line.trim_start()[".model".len()..].replace(['(', ')'], " ");
    let w = words(&body);
    if w.len() < 2 {
        return cx.fail(".model needs a name and a type");
    }
    let mname = w[0].clone();
    let kind = w[1].to_lowercase();
    let (_, kv) = split_pos(&w[2..]);
    let level = kv
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("level"))
        .and_then(|(_, v)| expr(v).ok().and_then(|v| v.parse::<f64>().ok()));
    let Some((file, module, ty)) = model_module(&kind, level) else {
        return cx.fail(format!(
            ".model {mname} {kind}{}: this model type/level has no VACASK device here",
            level.map(|l| format!(" level={l}")).unwrap_or_default()
        ));
    };
    cx.t.loads.insert(file.to_string());
    let mut params = String::new();
    if let Some(t) = ty {
        params.push_str(&format!(" type={t}"));
    }
    for (k, v) in &kv {
        let k = k.to_lowercase();
        if k == "level" || k == "version" {
            continue;
        }
        params.push_str(&format!(" {k}={}", cx.e(v)?));
    }
    cx.t.models_defined.insert(mname.clone());
    cx.models.push(format!("model {} {module}{}", name(&mname), if params.is_empty() { String::new() } else { format!(" ( {} )", params.trim_start()) }));
    Ok(())
}

fn element(cx: &mut Ctx, line: &str) -> Result<()> {
    let w = words(line);
    let first = w[0].chars().next().unwrap().to_ascii_lowercase();
    let inst = name(&w[0]);
    let need = |n: usize, cx: &Ctx| -> Result<()> {
        if w.len() < n {
            return cx.fail(format!("'{}' has too few fields", w[0]));
        }
        Ok(())
    };
    let mut warn = vec![];
    let text = match first {
        'r' | 'c' | 'l' => {
            need(3, cx)?;
            let (pos, mut kv) = split_pos(&w[3..]);
            let (n1, n2) = (name(&w[1]), name(&w[2]));
            let pname = first.to_string();
            let mut value = None;
            let mut model = None;
            for p in &pos {
                if value.is_none() && model.is_none() && value_like(p) {
                    value = Some(p.clone());
                } else if model.is_none() {
                    model = Some(p.clone());
                } else {
                    return cx.fail(format!("cannot read '{p}' in '{}'", w[0]));
                }
            }
            if let Some(i) = kv.iter().position(|(k, _)| k.eq_ignore_ascii_case(&pname)) {
                value = Some(kv.remove(i).1);
            }
            let model_name = match &model {
                Some(m) => {
                    cx.use_model(m, &w[0]);
                    name(m)
                }
                None => {
                    if value.is_none() {
                        return cx.fail(format!("'{}' has no value", w[0]));
                    }
                    cx.t.loads.insert(
                        match first {
                            'r' => "spice/resistor.osdi",
                            'c' => "spice/capacitor.osdi",
                            _ => "spice/inductor.osdi",
                        }
                        .into(),
                    );
                    cx.default_model(first)
                }
            };
            let mut text = format!("{inst} ({n1} {n2}) {model_name}");
            if let Some(v) = value {
                text.push_str(&format!(" {pname}={}", cx.e(&v)?));
            }
            text.push_str(&device_params(cx, &kv, &["ic"], &mut warn, &w[0])?);
            text
        }
        'v' | 'i' => {
            need(3, cx)?;
            source(cx, &w, line)?
        }
        'e' | 'g' => {
            need(6, cx)?;
            let (master, p) = if first == 'e' { ("vcvs", "gain") } else { ("vccs", "gm") };
            if w[5].contains('=') || !value_like(&w[5]) {
                return cx.fail(format!("'{}': only the linear form (four nodes and a gain) is supported", w[0]));
            }
            format!("{inst} ({} {} {} {}) {master} {p}={}", name(&w[1]), name(&w[2]), name(&w[3]), name(&w[4]), cx.e(&w[5])?)
        }
        'd' => {
            need(4, cx)?;
            let (pos, kv) = split_pos(&w[4..]);
            cx.use_model(&w[3], &w[0]);
            let mut kv = kv;
            if let Some(a) = pos.first() {
                kv.insert(0, ("area".into(), a.clone()));
            }
            let mut text = format!("{inst} ({} {}) {}", name(&w[1]), name(&w[2]), name(&w[3]));
            text.push_str(&device_params(cx, &kv, &["ic", "off"], &mut warn, &w[0])?);
            text
        }
        'm' => {
            need(6, cx)?;
            let (_, kv) = split_pos(&w[6..]);
            cx.use_model(&w[5], &w[0]);
            let mut text = format!(
                "{inst} ({} {} {} {}) {}",
                name(&w[1]),
                name(&w[2]),
                name(&w[3]),
                name(&w[4]),
                name(&w[5])
            );
            text.push_str(&device_params(cx, &kv, &["ic", "off"], &mut warn, &w[0])?);
            text
        }
        'q' => {
            need(5, cx)?;
            let (pos, kv) = split_pos(&w[1..]);
            // c b e [s] model [area]
            let (nodes, model, area): (Vec<String>, String, Option<String>) = match pos.len() {
                4 => (pos[..3].to_vec(), pos[3].clone(), None),
                5 if value_like(&pos[4]) => (pos[..3].to_vec(), pos[3].clone(), Some(pos[4].clone())),
                5 => (pos[..4].to_vec(), pos[4].clone(), None),
                6 => (pos[..4].to_vec(), pos[4].clone(), Some(pos[5].clone())),
                _ => return cx.fail(format!("cannot read the nodes of '{}'", w[0])),
            };
            let mut nodes: Vec<String> = nodes.iter().map(|n| name(n)).collect();
            if nodes.len() == 3 {
                nodes.push("0".into());
            }
            cx.use_model(&model, &w[0]);
            let mut kv = kv;
            if let Some(a) = area {
                kv.insert(0, ("area".into(), a));
            }
            let mut text = format!("{inst} ({}) {}", nodes.join(" "), name(&model));
            text.push_str(&device_params(cx, &kv, &["ic", "off"], &mut warn, &w[0])?);
            text
        }
        'x' | 'n' => {
            need(3, cx)?;
            let (pos, kv) = split_pos(&w[1..]);
            if pos.len() < 2 && first == 'x' {
                return cx.fail(format!("'{}' has no subcircuit name", w[0]));
            }
            let (nodes, sub) = pos.split_at(pos.len().saturating_sub(1));
            let sub = &sub[0];
            let nodes: Vec<String> = nodes.iter().map(|n| name(n)).collect();
            let mut text = format!("{inst} ({}) {}", nodes.join(" "), name(sub));
            if first == 'x' {
                cx.t.subckts_used.entry(sub.clone()).or_insert_with(|| w[0].clone());
                for (k, v) in &kv {
                    text.push_str(&format!(" {k}={}", cx.e(v)?));
                }
            } else {
                cx.use_model(sub, &w[0]);
                text.push_str(&device_params(cx, &kv, &[], &mut warn, &w[0])?);
            }
            text
        }
        other => {
            return cx.fail(format!(
                "element '{}' ({}) is not supported by the VACASK translator",
                w[0],
                other.to_ascii_uppercase()
            ));
        }
    };
    cx.t.warnings.extend(warn);
    cx.emit(text);
    Ok(())
}

fn dot_card(cx: &mut Ctx, line: &str, control: &mut bool) -> Result<bool> {
    let w = words(line);
    let card = w[0].to_lowercase();
    if *control {
        if card == ".endc" {
            *control = false;
        }
        return Ok(false);
    }
    match card.as_str() {
        ".end" => return Ok(true),
        ".control" => {
            *control = true;
            cx.t.warnings.push(format!("line {}: a .control block is not run by VACASK and is dropped", cx.line));
        }
        ".subckt" => {
            if cx.sub.is_some() {
                return cx.fail("nested .subckt is not supported");
            }
            if w.len() < 2 {
                return cx.fail(".subckt needs a name");
            }
            let mut pins = vec![];
            let mut params = vec![];
            for t in &w[2..] {
                if t.eq_ignore_ascii_case("params:") {
                    continue;
                }
                match assign(t) {
                    Some((n, v)) => params.push((n.to_string(), v.to_string())),
                    None => pins.push(name(t)),
                }
            }
            cx.t.subckts_defined.insert(w[1].clone());
            cx.subckts.push(format!("subckt {} ({})", name(&w[1]), pins.join(" ")));
            for (n, v) in params {
                let v = cx.e(&v)?;
                cx.subckts.push(format!("  parameters {n}={v}"));
            }
            cx.sub = Some(w[1].clone());
        }
        ".ends" => {
            if cx.sub.take().is_none() {
                return cx.fail(".ends without .subckt");
            }
            cx.subckts.push("ends".into());
        }
        ".param" | ".params" => {
            for t in &w[1..] {
                let Some((n, v)) = assign(t) else {
                    return cx.fail(format!("cannot read '{t}' in .param"));
                };
                let v = cx.e(v)?;
                let text = format!("parameters {n}={v}");
                if cx.sub.is_some() {
                    cx.subckts.push(format!("  {text}"));
                } else {
                    cx.params.push(text);
                }
            }
        }
        ".model" => model_card(cx, line)?,
        ".global" => {
            for g in &w[1..] {
                cx.t.globals.push(g.clone());
            }
        }
        ".include" | ".inc" | ".lib" | ".hdl" => {
            return cx.fail(format!(
                "'{}' of a model file in the netlist: ngspice model files cannot be read by VACASK; \
                 leave the models to the technology's vacask model set (models = \"vacask\")",
                line.trim()
            ));
        }
        ".ic" | ".nodeset" => {
            return cx.fail(format!("{card} is not supported by the VACASK translator yet"));
        }
        ".tran" | ".ac" | ".dc" | ".op" | ".noise" | ".tf" | ".pz" | ".sens" | ".disto" | ".meas" | ".measure"
        | ".save" | ".print" | ".plot" | ".probe" | ".four" | ".options" | ".option" | ".temp" | ".title" | ".width"
        | ".csparam" | ".endc" => {
            cx.t.warnings.push(format!("line {}: {card} is not translated (the testbench setup says what to run)", cx.line));
        }
        other => return cx.fail(format!("'{other}' is not supported by the VACASK translator")),
    }
    Ok(false)
}

/// Translate a SPICE netlist (first line: title).
pub fn translate(netlist: &str) -> Result<Translation> {
    let mut cx = Ctx {
        t: Translation::default(),
        params: vec![],
        models: vec![],
        subckts: vec![],
        insts: vec![],
        sub: None,
        default_models: BTreeSet::new(),
        line: 0,
    };
    let mut control = false;
    for (n, line) in logical_lines(netlist) {
        cx.line = n;
        let t = line.trim_start();
        if control {
            dot_card(&mut cx, t, &mut control)?;
            continue;
        }
        if t.starts_with('.') {
            if dot_card(&mut cx, t, &mut control)? {
                break;
            }
        } else if t.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
            element(&mut cx, t)?;
        } else {
            return cx.fail(format!("cannot read '{t}'"));
        }
    }
    if let Some(s) = &cx.sub {
        bail!("subcircuit '{s}' has no .ends");
    }
    let mut out = String::new();
    for g in &cx.t.globals {
        out.push_str(&format!("global {}\n", name(g)));
    }
    for l in &cx.params {
        out.push_str(l);
        out.push('\n');
    }
    for l in &cx.default_models {
        let module = match l {
            'r' => "sp_resistor",
            'c' => "sp_capacitor",
            _ => "sp_inductor",
        };
        out.push_str(&format!("model {DEFAULT_PREFIX}{l} {module}\n"));
    }
    for l in cx.models.iter().chain(&cx.subckts).chain(&cx.insts) {
        out.push_str(l);
        out.push('\n');
    }
    cx.t.text = out;
    let defined = cx.t.models_defined.clone();
    cx.t.models_used.retain(|m, _| !defined.contains(m));
    Ok(cx.t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_lose_their_suffixes() {
        assert_eq!(expr("10f").unwrap(), "10e-15");
        assert_eq!(expr("0.13u").unwrap(), "0.13e-6");
        assert_eq!(expr("1meg").unwrap(), "1e6");
        assert_eq!(expr("1M").unwrap(), "1e-3");
        assert_eq!(expr("2.2e-3").unwrap(), "2.2e-3");
        assert_eq!(expr("2e3k").unwrap(), "2e6");
        assert_eq!(expr("5").unwrap(), "5");
        assert_eq!(expr("{vdd/2 + 10p}").unwrap(), "vdd/2 + 10e-12");
        assert_eq!(expr("'a1u*2'").unwrap(), "a1u*2");
        assert_eq!(expr("temper+1").unwrap(), "$temp+1");
        assert_eq!(expr("1kohm").unwrap(), "1e3");
        assert_eq!(expr("1mil").unwrap(), "(1e0*25.4e-6)");
        assert!(expr("a^2").is_err());
    }

    #[test]
    fn words_keep_groups_and_join_equals() {
        assert_eq!(words("m1 a b w = 1u l= 0.13u x={a + b}"), ["m1", "a", "b", "w=1u", "l=0.13u", "x={a + b}"]);
        assert_eq!(words("v1 a 0 pulse(0 1 1n)"), ["v1", "a", "0", "pulse(0 1 1n)"]);
    }

    const TB: &str = "** sch_path: /x/tb.sch\n**.subckt tb_inv\nx1 in out VDD 0 inv\nVdd VDD 0 {vdd}\n\
Vin in 0 pulse(0 {vdd} 200p 50p 50p 400p 1n)\nC1 out 0 10f\n**.ends\n\n.subckt inv A Y VDD VSS\n*.ipin A\n\
XM2 VDD A Y VDD sg13_lv_pmos w=1.5u l=0.13u ng=1 m=1 mm_ok=1\nXM1 Y A VSS VSS sg13_lv_nmos w=0.74u l=0.13u ng=1 m=1 mm_ok=1\n.ends\n\n.GLOBAL VDD\n.end\n";

    #[test]
    fn translates_the_xschem_inverter_testbench() {
        let t = translate(TB).unwrap();
        let text = &t.text;
        assert!(text.contains("global VDD\n"), "{text}");
        assert!(text.contains("subckt inv (A Y VDD VSS)\n"), "{text}");
        assert!(text.contains("  XM2 (VDD A Y VDD) sg13_lv_pmos w=1.5e-6 l=0.13e-6 ng=1 m=1 mm_ok=1\n"), "{text}");
        assert!(text.contains("\nx1 (in out VDD 0) inv\n"), "{text}");
        assert!(text.contains("Vdd (VDD 0) vsource dc=vdd\n"), "{text}");
        assert!(
            text.contains("Vin (in 0) vsource dc=0 type=\"pulse\" val0=0 val1=vdd delay=200e-12 rise=50e-12 fall=50e-12 width=400e-12 period=1e-9\n"),
            "{text}"
        );
        assert!(text.contains("C1 (out 0) cdept_c c=10e-15\n"), "{text}");
        assert!(text.contains("model cdept_c sp_capacitor\n"), "{text}");
        assert!(t.loads.contains("spice/capacitor.osdi"));
        assert_eq!(t.subckts_used.keys().cloned().collect::<Vec<_>>(), ["inv", "sg13_lv_nmos", "sg13_lv_pmos"]);
        assert!(t.subckts_defined.contains("inv"));
        // The subcircuit comes before the instance that uses it.
        assert!(text.find("subckt inv").unwrap() < text.find("x1 (").unwrap());
    }

    #[test]
    fn translates_passives_sources_and_models() {
        let net = "title\nR1 a b 1k\nR2 a b rmod w=1u l=2u\nL1 a b 10n ic=0\nC2 a 0 {c0} m=2\n\
V1 a 0 dc 1 ac 1 sin(0 1 1meg)\nI1 a 0 pwl(0 0 1n 1m, 2n 0)\nE1 o 0 a 0 2\n\
.model dm d (is=1e-14 rs=10)\n.model nm nmos (level=1 vto=0.5 kp=1e-4)\nD1 a b dm 2\nM1 d g s b nm w=10u l=1u\n\
Q1 c b e npn1\n.param w1=1u w2={w1*2}\n.model npn1 npn (bf=100)\n.end\n";
        let t = translate(net).unwrap();
        let x = &t.text;
        assert!(x.contains("R1 (a b) cdept_r r=1e3\n"), "{x}");
        assert!(x.contains("R2 (a b) rmod w=1e-6 l=2e-6\n"), "{x}");
        assert!(x.contains("L1 (a b) cdept_l l=10e-9\n"), "{x}");
        assert!(t.warnings.iter().any(|w| w.contains("'ic' dropped")), "{:?}", t.warnings);
        assert!(x.contains("C2 (a 0) cdept_c c=c0 $mfactor=2\n"), "{x}");
        assert!(x.contains("V1 (a 0) vsource dc=1 mag=1 type=\"sine\" sinedc=0 ampl=1 freq=1e6\n"), "{x}");
        assert!(x.contains("I1 (a 0) isource dc=0 type=\"pwl\" wave=[0 0 1e-9 1e-3 2e-9 0]\n"), "{x}");
        assert!(x.contains("E1 (o 0 a 0) vcvs gain=2\n"), "{x}");
        assert!(x.contains("model dm sp_diode ( is=1e-14 rs=10 )\n"), "{x}");
        assert!(x.contains("model nm sp_mos1 ( type=1 vto=0.5 kp=1e-4 )\n"), "{x}");
        assert!(x.contains("D1 (a b) dm area=2\n"), "{x}");
        assert!(x.contains("M1 (d g s b) nm w=10e-6 l=1e-6\n"), "{x}");
        assert!(x.contains("Q1 (c b e 0) npn1\n"), "{x}");
        assert!(x.contains("parameters w2=w1*2\n"), "{x}");
        // npn1 is defined (after its use); rmod is not.
        assert_eq!(t.models_used.keys().cloned().collect::<Vec<_>>(), ["rmod"]);
        for f in ["spice/diode.osdi", "spice/mos1.osdi", "spice/bjt.osdi", "spice/inductor.osdi"] {
            assert!(t.loads.contains(f), "{f}");
        }
    }

    #[test]
    fn continuation_lines_and_comments() {
        let t = translate("t\n* c\nR1 a b ; hi\n+ 1k $ x\n.end\n").unwrap();
        assert!(t.text.contains("R1 (a b) cdept_r r=1e3"), "{}", t.text);
    }

    #[test]
    fn analyses_and_control_blocks_are_dropped_with_a_warning() {
        let t = translate("t\nR1 a 0 1\n.tran 1n 1u\n.control\nrun\n.endc\n.end\n").unwrap();
        assert_eq!(t.warnings.len(), 2, "{:?}", t.warnings);
        assert!(!t.text.contains("run"));
    }

    #[test]
    fn unsupported_things_are_errors_with_the_line() {
        for (net, what) in [
            ("t\nB1 a 0 v=sin(1)\n", "element 'B1'"),
            ("t\nK1 l1 l2 0.9\n", "element 'K1'"),
            ("t\n.include /x/models.lib\n", "ngspice model files"),
            ("t\n.lib /x/c.lib tt\n", "ngspice model files"),
            ("t\n.ic v(a)=1\n", ".ic"),
            ("t\nR1 a b {x^2}\n", "pow"),
            ("t\nV1 a 0 pulse(0 1 0)\n", "pulse"),
            ("t\n.subckt a x\nR1 x 0 1\n", "no .ends"),
            ("t\n.model m nmos (level=7)\n", "no VACASK device"),
            ("t\nF1 a 0 v1 2\n", "element 'F1'"),
            ("t\nR1 a b\n", "no value"),
            ("t\nR1 a\n", "too few"),
        ] {
            let e = translate(net).unwrap_err();
            assert!(format!("{e}").contains(what), "{net:?}: {e}");
            if !net.contains(".subckt") {
                assert!(format!("{e}").contains("line 2"), "{e}");
            }
        }
    }

    #[test]
    fn odd_names_are_made_safe() {
        assert_eq!(name("a[0]"), "a_x5b0_x5d");
        assert_eq!(name("net1"), "net1");
        let t = translate("t\nR1 a<1> b 1\n").unwrap();
        assert!(t.text.contains("(a_x3c1_x3e b)"), "{}", t.text);
    }
}
