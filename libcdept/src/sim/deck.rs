//! The SPICE deck of one run: model lines of the corner, parameters, the netlist of
//! the testbench schematic, analyses.

use std::collections::BTreeMap;

use super::setup::{Analysis, Setup};

/// The raw file a run writes, next to its deck.
pub const RAW_FILE: &str = "out.raw";

/// One run of a testbench: a corner, a temperature, design variables that override
/// the setup's `[variables]`, and for Monte Carlo the run's index and seed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RunPoint {
    pub corner: String,
    pub temperature: Option<f64>,
    /// Swept design variables of this point (they win over `[variables]`).
    pub variables: BTreeMap<String, String>,
    /// The random seed of a Monte Carlo run.
    pub seed: Option<u64>,
    /// The index of a Monte Carlo run.
    pub run: Option<u32>,
}

fn safe(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '.' => 'p',
            '-' => 'm',
            '+' => '_',
            c if c.is_ascii_alphanumeric() => c,
            _ => '_',
        })
        .collect()
}

impl RunPoint {
    /// A folder-safe name: `tt`, `ff_125`, `ss_m40`, `tt_27_vdd1p2`, `mc_27_r003`.
    pub fn name(&self) -> String {
        let mut name = match self.temperature {
            None => self.corner.clone(),
            Some(t) => {
                let t = if t.fract() == 0.0 {
                    format!("{}", t as i64)
                } else {
                    format!("{t}").replace('.', "p")
                };
                format!("{}_{}", self.corner, t.replace('-', "m"))
            }
        };
        for (k, v) in &self.variables {
            name.push_str(&format!("_{}{}", safe(k), safe(v)));
        }
        if let Some(r) = self.run {
            name.push_str(&format!("_r{r:03}"));
        }
        name
    }
}

/// The values of a point's design variables: the setup's, the point's on top.
pub fn merged_variables(setup: &Setup, point: &RunPoint) -> BTreeMap<String, String> {
    let mut v = setup.variables.clone();
    v.extend(point.variables.clone());
    v
}

/// The sweep combinations of `setup` (the first sweep varies slowest); one empty map
/// when there are no sweeps.
pub fn sweep_combinations(setup: &Setup) -> Vec<BTreeMap<String, String>> {
    let mut combos = vec![BTreeMap::new()];
    for sweep in &setup.sweeps {
        let values = sweep.expand().unwrap_or_default();
        combos = combos
            .into_iter()
            .flat_map(|c| {
                values.iter().map(move |v| {
                    let mut c = c.clone();
                    c.insert(sweep.variable.clone(), v.clone());
                    c
                })
            })
            .collect();
    }
    combos
}

fn temperatures(setup: &Setup) -> Vec<Option<f64>> {
    if setup.temperatures.is_empty() { vec![None] } else { setup.temperatures.iter().map(|t| Some(*t)).collect() }
}

/// Every run of `setup`: corners × temperatures × sweeps. `default_corner` stands in
/// when the setup names none. (Monte Carlo points: [`mc_points`].)
pub fn points(setup: &Setup, default_corner: &str) -> Vec<RunPoint> {
    let corners: Vec<String> = if setup.corners.is_empty() {
        vec![default_corner.to_string()]
    } else {
        setup.corners.clone()
    };
    let combos = sweep_combinations(setup);
    let mut out = vec![];
    for c in &corners {
        for t in temperatures(setup) {
            for v in &combos {
                out.push(RunPoint { corner: c.clone(), temperature: t, variables: v.clone(), ..Default::default() });
            }
        }
    }
    out
}

/// The seed of Monte Carlo run `index` for a base `seed`: a well-mixed positive
/// number below 2^31 (what ngspice's `rndseed` takes), the same on every machine.
pub fn mc_seed(seed: u64, index: u32) -> u64 {
    let mut z = seed.wrapping_add((index as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    1 + z % 0x7FFF_FFFE
}

/// The Monte Carlo runs: `runs` per temperature and sweep combination, each with its
/// own seed, all at the statistical corner `corner`.
pub fn mc_points(setup: &Setup, corner: &str, runs: u32, seed: u64) -> Vec<RunPoint> {
    let combos = sweep_combinations(setup);
    let mut out = vec![];
    for t in temperatures(setup) {
        for v in &combos {
            for i in 0..runs {
                out.push(RunPoint {
                    corner: corner.to_string(),
                    temperature: t,
                    variables: v.clone(),
                    seed: Some(mc_seed(seed, i)),
                    run: Some(i),
                });
            }
        }
    }
    out
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
    if let Some(seed) = point.seed {
        out.push_str(&format!("* Monte Carlo seed {seed} (setseed + reset in the control block)\n"));
    }
    out.push_str("* models\n");
    for line in models {
        out.push_str(line);
        out.push('\n');
    }
    if let Some(t) = point.temperature {
        out.push_str(&format!(".temp {t}\n"));
    }
    let variables = merged_variables(setup, point);
    if !variables.is_empty() {
        out.push_str("* design variables\n");
        for (name, value) in &variables {
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
    out.push_str(".control\n");
    if let Some(seed) = point.seed {
        // `.param x = agauss(..)` in the models is drawn when the circuit is read,
        // before any seed can be set (neither `set rndseed` in .spiceinit nor anything
        // else outside a control block reaches it), so seed the generator, then have
        // ngspice read the circuit again, which draws the parameters again.
        out.push_str(&format!("setseed {seed}\nreset\n"));
    }
    out.push_str("set filetype=binary\nset appendwrite\n");
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
    let variables = merged_variables(setup, point);
    if !variables.is_empty() {
        out.push_str("* design variables\n");
        for (name, value) in &variables {
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
    fn sweeps_cross_and_override() {
        let s = Setup::parse(
            "corners=[\"tt\",\"ff\"]\ntemperatures=[27]\n[variables]\nvdd=\"1.8\"\nc=\"1p\"\n\
             [[sweep]]\nvariable=\"vdd\"\nvalues=[\"1.0\",\"1.2\"]\n[[sweep]]\nvariable=\"r\"\nvalues=[\"-1\",\"2.5\"]",
        )
        .unwrap();
        let p = points(&s, "tt");
        let names: Vec<String> = p.iter().map(RunPoint::name).collect();
        assert_eq!(p.len(), 8);
        assert_eq!(names[0], "tt_27_rm1_vdd1p0");
        assert_eq!(names[1], "tt_27_r2p5_vdd1p0");
        assert_eq!(names[2], "tt_27_rm1_vdd1p2");
        assert_eq!(names[4], "ff_27_rm1_vdd1p0");
        let deck = build(&s, &p[2], "t", &[], "R1 a b 1k\n");
        assert!(deck.contains(".param vdd=1.2\n") && !deck.contains("vdd=1.8"), "{deck}");
        assert!(deck.contains(".param c=1p\n") && deck.contains(".param r=-1\n"));
    }

    #[test]
    fn montecarlo_points_have_distinct_stable_seeds() {
        let s = Setup::parse("temperatures=[27, 85]").unwrap();
        let p = mc_points(&s, "mc", 3, 42);
        let names: Vec<String> = p.iter().map(RunPoint::name).collect();
        assert_eq!(names, ["mc_27_r000", "mc_27_r001", "mc_27_r002", "mc_85_r000", "mc_85_r001", "mc_85_r002"]);
        assert_eq!(p[0].seed, Some(mc_seed(42, 0)));
        assert_eq!(p[0].seed, p[3].seed);
        let seeds: std::collections::BTreeSet<_> = p[..3].iter().map(|x| x.seed).collect();
        assert_eq!(seeds.len(), 3);
        assert!(p.iter().all(|x| (1..0x7FFF_FFFF).contains(&x.seed.unwrap())));
        assert_ne!(mc_seed(1, 0), mc_seed(2, 0));
        let deck = build(&s, &p[1], "t", &[], "R1 a b 1k\n");
        assert!(deck.contains(&format!(".control\nsetseed {}\nreset\nset filetype", p[1].seed.unwrap())), "{deck}");
        assert!(!build(&s, &points(&s, "tt")[0], "t", &[], "R1 a b 1k\n").contains("setseed"));
    }

    #[test]
    fn deck_has_every_part_in_order() {
        let s = Setup::parse(
            "save=[\"v(out)\"]\nmeasures=[\"tran d trig v(in) val=1 rise=1 targ v(out) val=1 rise=1\"]\n\
             [variables]\nvdd=\"1.8\"\n[options]\nreltol=\"1e-4\"\n[[analysis]]\nkind=\"tran\"\nstep=\"1n\"\nstop=\"1u\"",
        )
        .unwrap();
        let p = RunPoint { corner: "tt".into(), temperature: Some(125.0), ..Default::default() };
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
        let p = RunPoint { corner: "tt".into(), ..Default::default() };
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
