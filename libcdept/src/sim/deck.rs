//! The SPICE deck of one run: model lines of the corner, parameters, the netlist of
//! the testbench schematic, analyses.

use super::setup::{Analysis, Setup};

/// The raw file a run writes, next to its deck.
pub const RAW_FILE: &str = "out.raw";

/// One run of a testbench: a corner and a temperature.
#[derive(Debug, Clone, PartialEq)]
pub struct RunPoint {
    pub corner: String,
    pub temperature: Option<f64>,
}

impl RunPoint {
    /// A folder-safe name: `tt`, `ff_125`, `ss_m40`.
    pub fn name(&self) -> String {
        match self.temperature {
            None => self.corner.clone(),
            Some(t) => {
                let t = if t.fract() == 0.0 {
                    format!("{}", t as i64)
                } else {
                    format!("{t}").replace('.', "p")
                };
                format!("{}_{}", self.corner, t.replace('-', "m"))
            }
        }
    }
}

/// Every run of `setup`: corners × temperatures. `default_corner` stands in when the
/// setup names none.
pub fn points(setup: &Setup, default_corner: &str) -> Vec<RunPoint> {
    let corners: Vec<String> = if setup.corners.is_empty() {
        vec![default_corner.to_string()]
    } else {
        setup.corners.clone()
    };
    let temps: Vec<Option<f64>> = if setup.temperatures.is_empty() {
        vec![None]
    } else {
        setup.temperatures.iter().map(|t| Some(*t)).collect()
    };
    corners
        .iter()
        .flat_map(|c| {
            temps.iter().map(|t| RunPoint {
                corner: c.clone(),
                temperature: *t,
            })
        })
        .collect()
}

/// The command that runs an analysis from a `.control` block, and the word `.meas`
/// uses for its results.
fn analysis_command(a: &Analysis) -> (String, &'static str) {
    match a {
        Analysis::Op => ("op".into(), "op"),
        Analysis::Dc { source, start, stop, step } => (format!("dc {source} {start} {stop} {step}"), "dc"),
        Analysis::Ac { sweep, points, start, stop } => (format!("ac {sweep} {points} {start} {stop}"), "ac"),
        Analysis::Tran { step, stop, start } => {
            if start.is_empty() {
                (format!("tran {step} {stop}"), "tran")
            } else {
                (format!("tran {step} {stop} {start}"), "tran")
            }
        }
        Analysis::Noise { output, source, sweep, points, start, stop } => {
            (format!("noise {output} {source} {sweep} {points} {start} {stop}"), "noise")
        }
    }
}

/// The `.spiceinit` a run needs in its folder, from the model set: the search path of
/// the model files, the OSDI libraries and the settings the PDK's own start-up file
/// makes. ngspice reads `.spiceinit` from the folder it runs in, so this stands in for
/// the PDK's file without `$PDK_ROOT` in it.
pub fn spiceinit(
    set: &crate::tech::models::ModelSet,
    var: impl Fn(&str) -> Option<String> + Copy,
) -> Result<String, String> {
    let mut out = String::from("* written by CDePT from the technology snapshot\n");
    let dirs = set
        .search_path
        .iter()
        .map(|d| d.resolve(var).map(|p| p.display().to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    if !dirs.is_empty() {
        out.push_str(&format!("setcs sourcepath = ( $sourcepath {} )\n", dirs.join(" ")));
    }
    for o in &set.osdi {
        out.push_str(&format!("osdi '{}'\n", o.resolve(var)?.display()));
    }
    for line in &set.options {
        out.push_str(line);
        out.push('\n');
    }
    Ok(out)
}

/// The deck. `models` are the `.lib`/`.include` lines of the corner, `netlist` the
/// text xschem wrote for the testbench schematic. The netlist's own `.end` is dropped
/// (the deck ends once, at the end); everything else of it is kept, code blocks too.
pub fn build(setup: &Setup, point: &RunPoint, title: &str, models: &[String], netlist: &str) -> String {
    if setup.simulator == "xyce" {
        return build_xyce(setup, point, title, models, netlist);
    }
    let mut out = String::new();
    out.push_str(&format!("* CDePT testbench {title}, run {}\n", point.name()));
    out.push_str("* models\n");
    for line in models {
        out.push_str(line);
        out.push('\n');
    }
    if let Some(t) = point.temperature {
        out.push_str(&format!(".temp {t}\n"));
    }
    if !setup.variables.is_empty() {
        out.push_str("* design variables\n");
        for (name, value) in &setup.variables {
            out.push_str(&format!(".param {name}={value}\n"));
        }
    }
    if !setup.options.is_empty() {
        let items: Vec<String> = setup.options.iter().map(|(k, v)| format!("{k}={v}")).collect();
        out.push_str(&format!(".options {}\n", items.join(" ")));
    }
    out.push_str("* netlist\n");
    for line in netlist.lines() {
        if line.trim().eq_ignore_ascii_case(".end") {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    if !setup.save.is_empty() {
        out.push_str(&format!(".save {}\n", setup.save.join(" ")));
    }
    // ngspice does no `.meas` in batch mode when a raw file is asked for on the command
    // line, so the analyses run from a control block, each plot appended to out.raw,
    // each measurement right after the analysis it belongs to.
    out.push_str(".control\nset filetype=binary\nset appendwrite\n");
    let mut taken = vec![false; setup.measures.len()];
    for a in &setup.analyses {
        let (command, kind) = analysis_command(a);
        out.push_str(&format!("{command}\nwrite {RAW_FILE} all\n"));
        for (i, m) in setup.measures.iter().enumerate() {
            if !taken[i] && m.split_whitespace().next().is_some_and(|w| w.eq_ignore_ascii_case(kind)) {
                taken[i] = true;
                out.push_str(&format!("meas {m}\n"));
            }
        }
    }
    for (i, m) in setup.measures.iter().enumerate() {
        if !taken[i] {
            out.push_str(&format!("meas {m}\n"));
        }
    }
    out.push_str(".endc\n");
    out.push_str(".end\n");
    out
}

/// Options for Xyce are `.options <package> name=value`: the setup writes
/// `package.name`, a bare name goes to `device`.
fn xyce_options(setup: &Setup) -> Vec<String> {
    let mut by_pkg: std::collections::BTreeMap<&str, Vec<String>> = Default::default();
    for (k, v) in &setup.options {
        let (pkg, name) = k.split_once('.').unwrap_or(("device", k));
        by_pkg.entry(pkg).or_default().push(format!("{name}={v}"));
    }
    by_pkg.into_iter().map(|(p, items)| format!(".options {p} {}", items.join(" "))).collect()
}

/// Xyce reads `TRIG`, `TARG`, `VAL`, `RISE`, … in upper case only; the rest of a
/// measurement is case-blind, so all of it is written that way.
fn xyce_measure(m: &str) -> String {
    m.to_uppercase()
}

/// The deck for Xyce. It differs from the ngspice one in having no `.control` block
/// (Xyce runs the analysis its card names and writes the `.print` variables to the
/// raw file given on its command line), in its `.options` packages, in `.print` for
/// `save` (every voltage with `v(*)` when `save` is empty) and in upper-case `.meas`.
/// A `.control` … `.endc` block of the netlist is dropped.
pub fn build_xyce(setup: &Setup, point: &RunPoint, title: &str, models: &[String], netlist: &str) -> String {
    let mut out = String::new();
    out.push_str(&format!("* CDePT testbench {title}, run {} (Xyce)\n* models\n", point.name()));
    for line in models {
        out.push_str(line);
        out.push('\n');
    }
    if let Some(t) = point.temperature {
        out.push_str(&format!(".temp {t}\n"));
    }
    if !setup.variables.is_empty() {
        out.push_str("* design variables\n");
        for (name, value) in &setup.variables {
            out.push_str(&format!(".param {name}={value}\n"));
        }
    }
    for line in xyce_options(setup) {
        out.push_str(&line);
        out.push('\n');
    }
    out.push_str("* netlist\n");
    let mut control = false;
    for line in netlist.lines() {
        let word = line.trim().to_ascii_lowercase();
        if word.starts_with(".control") {
            control = true;
        }
        if !control && word != ".end" {
            out.push_str(line);
            out.push('\n');
        }
        if word.starts_with(".endc") {
            control = false;
        }
    }
    let vars = if setup.save.is_empty() { "v(*)".to_string() } else { setup.save.join(" ") };
    let mut taken = vec![false; setup.measures.len()];
    for a in &setup.analyses {
        let (command, kind) = analysis_command(a);
        out.push_str(&format!(".{command}\n"));
        if kind != "op" {
            out.push_str(&format!(".print {kind} {vars}\n"));
        }
        for (i, m) in setup.measures.iter().enumerate() {
            if !taken[i] && m.split_whitespace().next().is_some_and(|w| w.eq_ignore_ascii_case(kind)) {
                taken[i] = true;
                out.push_str(&format!(".meas {}\n", xyce_measure(m)));
            }
        }
    }
    for (i, m) in setup.measures.iter().enumerate() {
        if !taken[i] {
            out.push_str(&format!(".meas {}\n", xyce_measure(m)));
        }
    }
    out.push_str(".end\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn points_cross_corners_and_temperatures() {
        let s = Setup {
            corners: vec!["tt".into(), "ss".into()],
            temperatures: vec![-40.0, 27.5],
            ..Default::default()
        };
        let names: Vec<String> = points(&s, "tt").iter().map(RunPoint::name).collect();
        assert_eq!(names, ["tt_m40", "tt_27p5", "ss_m40", "ss_27p5"]);
        let only = points(&Setup::default(), "tt");
        assert_eq!(only.len(), 1);
        assert_eq!(only[0].name(), "tt");
    }

    #[test]
    fn deck_has_every_part_in_order() {
        let s = Setup::parse(
            "save=[\"v(out)\"]\nmeasures=[\"tran d trig v(in) val=1 rise=1 targ v(out) val=1 rise=1\"]\n\
             [variables]\nvdd=\"1.8\"\n[options]\nreltol=\"1e-4\"\n[[analysis]]\nkind=\"tran\"\nstep=\"1n\"\nstop=\"1u\"",
        )
        .unwrap();
        let p = RunPoint { corner: "tt".into(), temperature: Some(125.0) };
        let deck = build(&s, &p, "bench/tb", &[".lib /m/c.lib mos_tt".into()], "** sch\nR1 a b 1k\n.end\n");
        let at = |needle: &str| deck.find(needle).unwrap_or_else(|| panic!("no {needle} in\n{deck}"));
        assert!(deck.starts_with("* CDePT testbench bench/tb, run tt_125\n"));
        assert!(at(".lib /m/c.lib mos_tt") < at(".temp 125"));
        assert!(at(".temp 125") < at(".param vdd=1.8"));
        assert!(at(".param vdd=1.8") < at("R1 a b 1k"));
        assert!(at("R1 a b 1k") < at(".save v(out)"));
        assert!(at(".save v(out)") < at("tran 1n 1u"));
        assert!(at("tran 1n 1u") < at("write out.raw all"));
        assert!(at("write out.raw all") < at("meas tran d"));
        assert!(at("meas tran d") < at(".endc"));
        assert!(deck.contains(".options reltol=1e-4"));
        assert_eq!(deck.lines().filter(|l| *l == ".end").count(), 1, "{deck}");
        assert!(deck.trim_end().ends_with(".end"));
    }
    #[test]
    fn xyce_deck_has_no_control_block() {
        let s = Setup::parse(
            "simulator=\"xyce\"\nmeasures=[\"tran d trig v(in) val=1 rise=1 targ v(out) val=1 rise=1\"]\n\
             [variables]\nvdd=\"1.8\"\n[options]\n\"timeint.reltol\"=\"1e-4\"\ntnom=\"27\"\n\
             [[analysis]]\nkind=\"tran\"\nstep=\"1n\"\nstop=\"1u\"",
        )
        .unwrap();
        let p = RunPoint { corner: "tt".into(), temperature: None };
        let net = "** sch\nR1 a b 1k\n.control\nrun\n.endc\n.end\n";
        let deck = build(&s, &p, "b/tb", &[".lib /m/x.lib tt".into()], net);
        let at = |needle: &str| deck.find(needle).unwrap_or_else(|| panic!("no {needle} in\n{deck}"));
        assert!(!deck.contains(".control") && !deck.contains("write out.raw") && !deck.contains("\nrun\n"), "{deck}");
        assert!(at(".lib /m/x.lib tt") < at(".param vdd=1.8"));
        assert!(at(".param vdd=1.8") < at("R1 a b 1k"));
        assert!(at("R1 a b 1k") < at(".tran 1n 1u"));
        assert!(at(".tran 1n 1u") < at(".print tran v(*)"));
        assert!(at(".print tran v(*)") < at(".meas TRAN D TRIG V(IN) VAL=1 RISE=1 TARG V(OUT) VAL=1 RISE=1"));
        assert!(deck.contains(".options device tnom=27") && deck.contains(".options timeint reltol=1e-4"), "{deck}");
        assert_eq!(deck.lines().filter(|l| *l == ".end").count(), 1);
        // With `save`, those are what is printed.
        let s2 = Setup { save: vec!["v(out)".into()], ..s };
        assert!(build(&s2, &p, "t", &[], "").contains(".print tran v(out)\n"));
    }
}
