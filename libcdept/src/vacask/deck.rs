//! The VACASK netlist of one run point: loads, the technology's converted models, the
//! translated testbench netlist and a `control` block with the analyses.
//!
//! The model lines come from [`crate::tech::models::ModelsTech::lines`] (`.lib file
//! section`, `.include file`) plus the pseudo lines [`model_lines`] adds for the set's
//! OSDI files and VACASK lines: `.osdi <file>` and `.vcline <text>`.

use std::collections::BTreeSet;

use eyre::{Result, bail};

use super::translate::{self, Translation, expr, name};
use crate::sim::deck::{RunPoint, merged_variables};
use crate::sim::setup::{Analysis, Setup};
use crate::tech::models::ModelSet;

/// The name of the first analysis of each kind in the deck, `tran1`, `ac1`, … (one
/// per analysis of the setup, numbered by kind).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalysisName {
    pub name: String,
    /// `op`, `dc`, `ac`, `tran`, `noise`.
    pub kind: &'static str,
}

/// A built deck.
#[derive(Debug, Clone, PartialEq)]
pub struct Deck {
    pub text: String,
    /// The analyses in order; each writes `<name>.raw`.
    pub analyses: Vec<AnalysisName>,
    pub warnings: Vec<String>,
}

/// The model lines of a run point for VACASK: the OSDI libraries and raw lines of the
/// set, then the corner's `.lib`/`.include` lines.
pub fn model_lines(
    set: &ModelSet,
    corner_lines: Vec<String>,
    var: impl Fn(&str) -> Option<String> + Copy,
) -> Result<Vec<String>, String> {
    let mut out = vec![];
    for o in &set.osdi {
        let p = o.resolve(var)?;
        out.push(format!(".osdi {}", p.display()));
    }
    for line in &set.options {
        out.push(format!(".vcline {line}"));
    }
    out.extend(corner_lines);
    Ok(out)
}

fn quoted(path: &str) -> String {
    format!("\"{}\"", path.trim().trim_matches('"'))
}

/// One model line as VACASK text.
fn model_line(line: &str) -> Result<String> {
    let t = line.trim();
    if let Some(p) = t.strip_prefix(".osdi ") {
        return Ok(format!("load {}", quoted(p)));
    }
    if let Some(p) = t.strip_prefix(".vcline ") {
        return Ok(p.to_string());
    }
    let (card, rest) = t.split_once(char::is_whitespace).unwrap_or((t, ""));
    match card.to_lowercase().as_str() {
        ".include" | ".inc" => Ok(format!("include {}", quoted(rest))),
        ".lib" => {
            // `.lib "file with spaces" section` or `.lib file section`.
            let rest = rest.trim();
            let (file, section) = if let Some(r) = rest.strip_prefix('"') {
                let (f, s) = r.split_once('"').ok_or_else(|| eyre::eyre!("unclosed quote in '{t}'"))?;
                (f.to_string(), s.trim().to_string())
            } else {
                let (f, s) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
                (f.to_string(), s.trim().to_string())
            };
            if section.is_empty() {
                bail!("'{t}': a library line needs a section");
            }
            Ok(format!("include {} section={section}", quoted(&file)))
        }
        _ => Err(eyre::eyre!("cannot use the model line '{t}' with VACASK")),
    }
}

/// `v(out)` to `out`; a plain name stays.
fn node_of(vector: &str) -> Result<String> {
    let v = vector.trim();
    let lower = v.to_lowercase();
    if let Some(inner) = lower.strip_prefix("v(").and_then(|r| r.strip_suffix(')')) {
        if inner.contains(',') {
            bail!("'{v}': a differential output is not supported with VACASK");
        }
        // Keep the case of the original.
        return Ok(name(&v[2..2 + inner.len()]));
    }
    Ok(name(v))
}

/// The name the netlist gives an instance that the setup names in any case (SPICE has
/// none, VACASK is case sensitive): `Vin` for `vin`.
fn instance_name(netlist: &str, source: &str) -> String {
    let want = name(source);
    netlist
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .find(|n| n.eq_ignore_ascii_case(&want))
        .map_or(want, str::to_string)
}

/// The parameter a DC sweep of `source` has to move. A source with a waveform has its
/// DC value from the waveform (the pulse's `val0`, the sine's offset), not from `dc`,
/// so that is what ngspice's `dc` sweep replaces and what is swept here.
fn sweep_parameter(netlist: &str, source: &str) -> Result<&'static str> {
    let want = name(source);
    let line = netlist.lines().find(|l| l.split_whitespace().next().is_some_and(|n| n.eq_ignore_ascii_case(&want)));
    let Some(line) = line else {
        return Err(eyre::eyre!("the DC sweep names '{source}', which is not a source of the testbench netlist"));
    };
    Ok(match line.split("type=\"").nth(1).and_then(|r| r.split('"').next()) {
        None => "dc",
        Some("pulse" | "exp") => "val0",
        Some("sine") => "sinedc",
        Some(other) => return Err(eyre::eyre!("a DC sweep of '{source}', a '{other}' source, is not supported with VACASK")),
    })
}

/// A sweep end: `to` plus a millionth of a step, so that a float sum that falls just
/// short of it (1.15 + 0.05) still reaches it (ngspice includes the end point).
fn sweep_end(stop: &str, step: &str) -> Result<String> {
    let (s, d) = (expr(stop).map_err(|m| eyre::eyre!(m))?, expr(step).map_err(|m| eyre::eyre!(m))?);
    match (s.parse::<f64>(), d.parse::<f64>()) {
        (Ok(s), Ok(d)) => Ok(format!("{:e}", s + d.abs() * 1e-6)),
        _ => Ok(s),
    }
}

fn analysis(a: &Analysis, n: usize, netlist: &str, out: &mut Vec<String>) -> Result<AnalysisName> {
    let e = |s: &str| expr(s).map_err(|m| eyre::eyre!(m));
    let mode = |s: &str| -> Result<&'static str> {
        match s.to_lowercase().as_str() {
            "dec" => Ok("dec"),
            "oct" => Ok("oct"),
            "lin" => Ok("lin"),
            other => Err(eyre::eyre!("sweep '{other}' is not supported with VACASK (dec, oct, lin)")),
        }
    };
    Ok(match a {
        Analysis::Op => {
            let name = format!("op{n}");
            out.push(format!("  analysis {name} op"));
            AnalysisName { name, kind: "op" }
        }
        Analysis::Dc { source, start, stop, step } => {
            let name = format!("dc{n}");
            out.push(format!(
                "  sweep {name}_sweep instance=\"{}\" parameter=\"{}\" from={} to={} step={}",
                instance_name(netlist, source),
                sweep_parameter(netlist, source)?,
                e(start)?,
                sweep_end(stop, step)?,
                e(step)?
            ));
            out.push(format!("  analysis {name} op"));
            AnalysisName { name, kind: "dc" }
        }
        Analysis::Ac { sweep, points, start, stop } => {
            let name = format!("ac{n}");
            out.push(format!(
                "  analysis {name} ac from={} to={} mode=\"{}\" points={points}",
                e(start)?,
                e(stop)?,
                mode(sweep)?
            ));
            AnalysisName { name, kind: "ac" }
        }
        Analysis::Tran { step, stop, start } => {
            if !start.is_empty() && e(start)?.parse::<f64>() != Ok(0.0) {
                bail!("a transient that saves from a start time ({start}) is not supported with VACASK; use start 0");
            }
            let name = format!("tran{n}");
            out.push(format!("  analysis {name} tran step={} stop={}", e(step)?, e(stop)?));
            AnalysisName { name, kind: "tran" }
        }
        Analysis::Noise { output, source, sweep, points, start, stop } => {
            let name = format!("noise{n}");
            out.push(format!(
                "  analysis {name} noise out=\"{}\" in=\"{}\" from={} to={} mode=\"{}\" points={points}",
                node_of(output)?,
                instance_name(netlist, source),
                e(start)?,
                e(stop)?,
                mode(sweep)?
            ));
            AnalysisName { name, kind: "noise" }
        }
    })
}

/// What a deck needs to know.
pub struct Inputs<'a> {
    pub setup: &'a Setup,
    pub point: &'a RunPoint,
    pub title: &'a str,
    /// [`model_lines`] of the point's corner.
    pub models: &'a [String],
    /// The testbench's SPICE netlist as xschem wrote it.
    pub netlist: &'a str,
    /// The `subckt` and `model` names the technology's converted files define, to
    /// check the netlist's devices against; `None` skips the check.
    pub defined: Option<&'a BTreeSet<String>>,
    /// What the technology's statistical models draw ([`super::stat`]); `None`: nothing.
    pub mm: Option<&'a super::stat::MmInfo>,
    /// The technology (for messages) and what it says about models VACASK cannot have.
    pub pdk: &'a str,
    pub notes: &'a [String],
}

/// Check that every subcircuit and model the netlist calls exists: in the netlist, in
/// the technology's converted files (`defined`) or in VACASK's builtins. The error
/// names the technology and the device and says why the technology has no such model.
pub fn check_devices(t: &Translation, i: &Inputs) -> Result<()> {
    let Some(defined) = i.defined else { return Ok(()) };
    let why = |dev: &str, inst: &str| {
        let notes: Vec<&str> = i.notes.iter().filter(|n| n.starts_with("vacask:")).map(String::as_str).collect();
        let mut m = format!(
            "the technology '{}' has no VACASK model for '{dev}' (instance {inst}): no converted model file defines it",
            i.pdk
        );
        if !notes.is_empty() {
            m.push_str("\n  what the conversion could not do:\n    ");
            m.push_str(&notes.join("\n    "));
        }
        m
    };
    for (sub, inst) in &t.subckts_used {
        if !t.subckts_defined.contains(sub) && !defined.contains(sub) {
            bail!("{}", why(sub, inst));
        }
    }
    for (model, inst) in &t.models_used {
        if !defined.contains(model) {
            bail!("{}", why(model, inst));
        }
    }
    Ok(())
}

/// The netlist without the `.include` lines of PDK model files that the technology's
/// converted set has (by file name, in the folder of its converted corner files), and the
/// model lines that bring the converted files instead.
fn adopt_includes(netlist: &str, mm: Option<&super::stat::MmInfo>, models: &[String]) -> (String, Vec<String>) {
    let Some(mm) = mm else { return (netlist.to_string(), vec![]) };
    let dir = models.iter().find_map(|l| {
        let path = l.split_whitespace().nth(1)?.trim_matches('"');
        let p = std::path::Path::new(path);
        mm.files.contains_key(p.file_name()?.to_str()?).then(|| p.parent().map(|d| d.to_path_buf())).flatten()
    });
    let Some(dir) = dir else { return (netlist.to_string(), vec![]) };
    let mut out = String::new();
    let mut brought: Vec<String> = vec![];
    for l in netlist.lines() {
        let mut w = l.split_whitespace();
        let adopted = (w.next().is_some_and(|c| c.eq_ignore_ascii_case(".include") || c.eq_ignore_ascii_case(".inc")))
            .then(|| w.next())
            .flatten()
            .map(|f| f.trim_matches(['"', '\'']))
            .and_then(|f| std::path::Path::new(f).file_name()?.to_str().map(str::to_string))
            .filter(|name| mm.files.contains_key(name) && dir.join(name).is_file());
        match adopted {
            Some(name) => {
                let line = format!(".include {}", dir.join(&name).display());
                if !brought.contains(&line) && !models.contains(&line) {
                    brought.push(line);
                }
                out.push_str(&format!("* {l}\n"));
            }
            None => {
                out.push_str(l);
                out.push('\n');
            }
        }
    }
    (out, brought)
}

/// Build the deck. Errors say what could not be expressed for VACASK.
pub fn build(i: &Inputs) -> Result<Deck> {
    // A `.include` of a PDK model file in the netlist (the symbol of `bondpad` brings one)
    // is the converted file of the technology's set, when there is one.
    let (netlist, brought) = adopt_includes(i.netlist, i.mm, i.models);
    let models: Vec<String> = i.models.iter().cloned().chain(brought).collect();
    let mut t = translate::translate(&netlist)?;
    check_devices(&t, i)?;
    let resolved = i.mm.map(|mm| super::stat::resolve(&models, mm));
    let model_lines: &[String] = resolved.as_ref().map_or(&models, |r| &r.lines);
    if resolved.is_some() {
        // The converted subcircuits have lower-case parameters (`nx`), SPICE writes `Nx=1`.
        let texts: Vec<String> = model_lines.iter().flat_map(|l| model_texts(l)).collect();
        let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        t.text = super::stat::fix_param_case(&t.text, &refs);
    }
    // Monte Carlo point: every device of the circuit gets a key for its own draws.
    let mut leaves = 0;
    if let (Some(r), Some(_)) = (&resolved, i.point.seed)
        && !r.masters.is_empty()
    {
        (t.text, leaves) = super::stat::instrument(&t.text, &r.masters);
    }
    let mut out = String::new();
    out.push_str(&format!("// CDePT testbench {}, run {} (VACASK)\n", i.title, i.point.name()));
    out.push_str("ground 0\ninclude \"builtins.inc\"\n");
    let mut loaded = BTreeSet::new();
    let mut body = vec![];
    for line in model_lines {
        let text = model_line(line)?;
        if text.starts_with("load ") && !loaded.insert(text.clone()) {
            continue;
        }
        body.push(text);
    }
    // Loads and raw lines first, then the includes (they need the OSDI files).
    let (pre, inc): (Vec<_>, Vec<_>) = body.into_iter().partition(|l| !l.starts_with("include "));
    out.push_str("// models\n");
    for l in pre {
        out.push_str(&l);
        out.push('\n');
    }
    for f in &t.loads {
        let l = format!("load {}", quoted(f));
        if loaded.insert(l.clone()) {
            out.push_str(&l);
            out.push('\n');
        }
    }
    if let (Some(r), Some(mm)) = (&resolved, i.mm)
        && r.uses_draws
    {
        let nz = r.masters.values().copied().max().unwrap_or(1);
        for l in super::stat::draw_lines(i.point.seed, nz, leaves, mm.globals.len()) {
            out.push_str(&l);
            out.push('\n');
        }
    }
    for l in inc {
        out.push_str(&l);
        out.push('\n');
    }
    let variables = merged_variables(i.setup, i.point);
    if !variables.is_empty() {
        out.push_str("// design variables\n");
        for (n, v) in &variables {
            out.push_str(&format!("parameters {n}={}\n", expr(v).map_err(|m| eyre::eyre!(m))?));
        }
    }
    out.push_str("// netlist\n");
    out.push_str(&t.text);
    out.push_str("control\n");
    let mut options: Vec<String> = vec![];
    if let Some(temp) = i.point.temperature {
        options.push(format!("temp={temp}"));
    }
    for (k, v) in &i.setup.options {
        options.push(format!("{k}={}", expr(v).map_err(|m| eyre::eyre!(m))?));
    }
    if !options.is_empty() {
        out.push_str(&format!("  options {}\n", options.join(" ")));
    }
    // `@n.xdut.xm1.nsg13_lv_pmos[vds]` names devices inside the models: look them up.
    let structure = i.setup.save.iter().any(|s| s.trim_start().starts_with('@')).then(|| {
        let mut texts = vec![t.text.clone()];
        for l in model_lines {
            texts.extend(model_texts(l));
        }
        let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        super::stat::structure(&refs)
    });
    for s in &i.setup.save {
        match (&structure, s.trim_start().starts_with('@')) {
            (Some(st), true) => out.push_str(&format!("  save {}\n", super::stat::device_save(s, st).map_err(|e| eyre::eyre!(e))?)),
            _ => out.push_str(&format!("  save {}\n", save_item(s)?)),
        }
    }
    let mut lines = vec![];
    let mut analyses = vec![];
    let mut counts = std::collections::BTreeMap::<&str, usize>::new();
    for a in &i.setup.analyses {
        let kind = match a {
            Analysis::Op => "op",
            Analysis::Dc { .. } => "dc",
            Analysis::Ac { .. } => "ac",
            Analysis::Tran { .. } => "tran",
            Analysis::Noise { .. } => "noise",
        };
        let n = counts.entry(kind).or_insert(0);
        *n += 1;
        analyses.push(analysis(a, *n, &t.text, &mut lines)?);
    }
    if analyses.is_empty() {
        bail!("the testbench has no analysis");
    }
    for l in lines {
        out.push_str(&l);
        out.push('\n');
    }
    out.push_str("endc\n");
    // Read back by `run_vacask`: which raw file is which analysis, what to measure.
    for a in &analyses {
        out.push_str(&format!("{}{} {}\n", super::MARK_ANALYSIS, a.name, a.kind));
    }
    for m in &i.setup.measures {
        out.push_str(&format!("{}{}\n", super::MARK_MEASURE, m.split_whitespace().collect::<Vec<_>>().join(" ")));
    }
    Ok(Deck { text: out, analyses, warnings: t.warnings })
}

/// The text of the converted files a model line brings in (`.include`, `.lib` with the
/// files of its section), for looking up the devices inside them.
fn model_texts(line: &str) -> Vec<String> {
    let t = line.trim();
    let (card, rest) = t.split_once(char::is_whitespace).unwrap_or((t, ""));
    let rest = rest.trim();
    match card.to_lowercase().as_str() {
        ".include" | ".inc" => std::fs::read_to_string(rest.trim_matches('"')).into_iter().collect(),
        ".lib" => {
            let (file, section) = match rest.strip_prefix('"').and_then(|r| r.split_once('"')) {
                Some((f, s)) => (f, s.trim()),
                None => rest.split_once(char::is_whitespace).map_or((rest, ""), |(f, s)| (f, s.trim())),
            };
            let Ok(text) = std::fs::read_to_string(file) else { return vec![] };
            let dir = std::path::Path::new(file).parent().map(|p| p.to_path_buf()).unwrap_or_default();
            super::stat::section_includes(&text, section)
                .iter()
                .filter_map(|f| std::fs::read_to_string(dir.join(f)).ok())
                .collect()
        }
        _ => vec![],
    }
}

/// `v(out)` and `i(vdd)` as VACASK saves them; anything else as written.
fn save_item(s: &str) -> Result<String> {
    let lower = s.trim().to_lowercase();
    if lower.starts_with("v(") {
        return Ok(format!("v({})", node_of(s)?));
    }
    if let Some(inner) = lower.strip_prefix("i(").and_then(|r| r.strip_suffix(')')) {
        return Ok(format!("{}:flow(br)", name(&s.trim()[2..2 + inner.len()])));
    }
    // ngspice's "all" is what VACASK saves by default (node voltages and branch flows).
    if lower == "all" {
        return Ok("default".to_string());
    }
    Ok(s.trim().to_string())
}

/// [`build`] for callers that return the deck as text: a failure is written into the
/// deck as comment lines (so `cdept sim deck` shows it) and the run refuses it before
/// starting anything ([`crate::sim::run_testbench_with`] builds with [`build`] first).
pub fn build_or_comment(i: &Inputs) -> String {
    match build(i) {
        Ok(d) => d.text,
        Err(e) => {
            let mut s = String::from("// CDePT: this testbench cannot be written for VACASK\n");
            for l in format!("{e:#}").lines() {
                s.push_str(&format!("// {l}\n"));
            }
            s
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NET: &str = "** t\nx1 in out VDD 0 inv\nVdd VDD 0 {vdd}\nVin in 0 pulse(0 {vdd} 200p 50p 50p 400p 1n)\nC1 out 0 10f\n\
.subckt inv A Y VDD VSS\nXM2 VDD A Y VDD pm w=1u\nXM1 Y A VSS VSS nm w=1u\n.ends\n.end\n";

    fn setup(extra: &str) -> Setup {
        Setup::parse(&format!(
            "simulator = \"vacask\"\ntemperatures=[27]\nsave=[\"v(in)\", \"v(out)\", \"i(Vdd)\"]\n[variables]\nvdd=\"1.2\"\n{extra}\n\
[[analysis]]\nkind=\"tran\"\nstep=\"5p\"\nstop=\"1.2n\"\n"
        ))
        .unwrap()
    }

    #[test]
    fn writes_loads_models_netlist_and_analyses() {
        let s = setup("");
        let point = RunPoint { corner: "tt".into(), temperature: Some(27.0), ..Default::default() };
        let models = vec![
            ".osdi /p/psp103.osdi".to_string(),
            ".vcline parameters swsoa=0".to_string(),
            ".lib /p/cornerMOSlv.lib mos_tt".to_string(),
            ".include /p/extra.lib".to_string(),
        ];
        let defined: BTreeSet<String> = ["pm", "nm"].iter().map(|s| s.to_string()).collect();
        let i = Inputs { setup: &s, point: &point, title: "t", models: &models, netlist: NET, defined: Some(&defined), mm: None, pdk: "x", notes: &[] };
        let d = build(&i).unwrap();
        let t = &d.text;
        assert!(t.contains("ground 0\ninclude \"builtins.inc\"\n"), "{t}");
        assert!(t.contains("load \"/p/psp103.osdi\"\nparameters swsoa=0\n"), "{t}");
        assert!(t.contains("load \"spice/capacitor.osdi\"\n"), "{t}");
        assert!(t.contains("include \"/p/cornerMOSlv.lib\" section=mos_tt\ninclude \"/p/extra.lib\"\n"), "{t}");
        assert!(t.find("load \"/p/psp103.osdi\"").unwrap() < t.find("include \"/p/corner").unwrap());
        assert!(t.contains("parameters vdd=1.2\n"), "{t}");
        assert!(t.contains("control\n  options temp=27\n  save v(in)\n  save v(out)\n  save Vdd:flow(br)\n  analysis tran1 tran step=5e-12 stop=1.2e-9\nendc\n// cdept-analysis: tran1 tran\n"), "{t}");
        assert_eq!(d.analyses, [AnalysisName { name: "tran1".into(), kind: "tran" }]);
    }

    #[test]
    fn all_analysis_kinds() {
        let s = Setup::parse(
            "simulator=\"vacask\"\n[[analysis]]\nkind=\"op\"\n[[analysis]]\nkind=\"dc\"\nsource=\"V1\"\nstart=\"0\"\nstop=\"1.2\"\nstep=\"0.1\"\n\
[[analysis]]\nkind=\"ac\"\nsweep=\"dec\"\npoints=10\nstart=\"1\"\nstop=\"1meg\"\n\
[[analysis]]\nkind=\"noise\"\noutput=\"v(out)\"\nsource=\"V1\"\nsweep=\"dec\"\npoints=5\nstart=\"1\"\nstop=\"1k\"\n",
        )
        .unwrap();
        let point = RunPoint { corner: "tt".into(), ..Default::default() };
        let i = Inputs { setup: &s, point: &point, title: "t", models: &[], netlist: "t\nV1 out 0 dc 1 ac 1\nR1 out 0 1k\n", defined: None, mm: None, pdk: "x", notes: &[] };
        let d = build(&i).unwrap();
        for want in [
            "analysis op1 op",
            "sweep dc1_sweep instance=\"V1\" parameter=\"dc\" from=0 to=1.2000001e0 step=0.1\n  analysis dc1 op",
            "analysis ac1 ac from=1 to=1e6 mode=\"dec\" points=10",
            "analysis noise1 noise out=\"out\" in=\"V1\" from=1 to=1e3 mode=\"dec\" points=5",
        ] {
            assert!(d.text.contains(want), "{want}\n{}", d.text);
        }
        assert_eq!(d.analyses.iter().map(|a| a.kind).collect::<Vec<_>>(), ["op", "dc", "ac", "noise"]);
    }

    #[test]
    fn a_device_the_technology_cannot_give_is_an_error_naming_it() {
        let s = setup("");
        let point = RunPoint { corner: "tt".into(), ..Default::default() };
        let defined: BTreeSet<String> = ["pm"].iter().map(|s| s.to_string()).collect();
        let notes = vec!["vacask: cornerHBT.lib could not be converted: vbic".to_string(), "other note".to_string()];
        let i = Inputs { setup: &s, point: &point, title: "t", models: &[], netlist: NET, defined: Some(&defined), mm: None, pdk: "ihp-sg13g2", notes: &notes };
        let e = format!("{:#}", build(&i).unwrap_err());
        assert!(e.contains("'ihp-sg13g2'") && e.contains("'nm'") && e.contains("XM1"), "{e}");
        assert!(e.contains("cornerHBT.lib could not be converted") && !e.contains("other note"), "{e}");
        let text = build_or_comment(&i);
        assert!(text.starts_with("// CDePT: this testbench cannot be written for VACASK\n// the technology"), "{text}");
    }

    #[test]
    fn an_include_of_a_converted_file_becomes_the_converted_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("pad.lib"), "subckt bondpad (PAD)\nends\n").unwrap();
        let mut mm = super::super::stat::MmInfo::default();
        mm.files.insert("pad.lib".into(), super::super::stat::FileInfo { defs: vec!["bondpad".into()], ..Default::default() });
        mm.files.insert("corner.lib".into(), Default::default());
        let models = vec![format!(".lib {}/corner.lib tt", dir.path().display())];
        let net = "t\n.include /elsewhere/models/pad.lib\n.include /elsewhere/other.lib\nXB1 a bondpad\n.end\n";
        let (out, brought) = adopt_includes(net, Some(&mm), &models);
        assert_eq!(brought, [format!(".include {}/pad.lib", dir.path().display())]);
        assert!(out.contains("* .include /elsewhere/models/pad.lib\n") && out.contains("\n.include /elsewhere/other.lib\n"), "{out}");
        // Without a converted set the netlist stays.
        assert_eq!(adopt_includes(net, None, &models), (net.to_string(), vec![]));
    }

    #[test]
    fn refuses_what_vacask_cannot_do() {
        // The setup is refused when it loads (sim::caps); a deck built from a setup that
        // got around that is refused too.
        let point = RunPoint { corner: "tt".into(), ..Default::default() };
        let text = "simulator=\"vacask\"\n[[analysis]]\nkind=\"tran\"\nstep=\"1n\"\nstop=\"1u\"\nstart=\"0.5u\"\n";
        assert!(format!("{:#}", Setup::parse(text).unwrap_err()).contains("start time"));
        let s = Setup::parse(&text.replace("vacask", "ngspice")).unwrap();
        let i = Inputs { setup: &s, point: &point, title: "t", models: &[], netlist: "t\nR1 a 0 1\n", defined: None, mm: None, pdk: "x", notes: &[] };
        assert!(format!("{:#}", build(&i).unwrap_err()).contains("start time"));
        let e = model_line(".lib /x/a.lib").unwrap_err();
        assert!(format!("{e}").contains("section"));
        assert_eq!(model_line(".lib \"/x y/a.lib\" tt").unwrap(), "include \"/x y/a.lib\" section=tt");
        assert!(model_line(".bogus x").is_err());
    }
}
