//! `setup.toml` of a testbench view.

use std::collections::BTreeMap;
use std::path::Path;

use eyre::{Result, WrapErr, bail};
use serde::{Deserialize, Serialize};

/// The file that makes a view a testbench.
pub const SETUP_FILE: &str = "setup.toml";

/// A testbench.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Setup {
    /// The schematic to netlist, `library/cell`. Empty: the cell this view is in.
    pub schematic: String,
    /// The schematic view when the cell has several. Empty: the first xschem one.
    pub schematic_view: String,
    /// `xschem` or `qucs`: which tool's schematic is netlisted (Qucs-S: `qucs-s -n`, see
    /// docs/QUCS.md). Empty: the named view's own kind, else the cell's xschem
    /// schematic, else its Qucs-S one.
    pub schematic_kind: String,
    /// The simulator: `ngspice` (also when empty) or `xyce`. With Xyce one analysis is
    /// run (plus `op`), options are written `package.name` (`timeint.reltol`; a bare
    /// name goes to `device`) and the model set is the technology's `xyce` one unless
    /// `models` names another.
    pub simulator: String,
    /// How ngspice runs: `process` (also when empty: `ngspice -b`, one process per run
    /// point) or `inproc` (libngspice loaded in CDePT, docs/NGSPICE.md).
    /// `cdept sim run --backend` overrides it.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub simulator_backend: String,
    /// The remote (`[jobs.remote.NAME]` of cdept.toml) the run points go to, over ssh or as
    /// SLURM jobs (docs/howto/remote-runs.md). Empty: this machine, unless the project names a
    /// default. `cdept sim run --remote` overrides it.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub remote: String,
    /// The time limit of one run point: `"10m"`, `"90s"`, `"1h30m"` or a number of
    /// seconds; `"0"` is no limit. Empty: the project's `[sim] timeout` (default 30
    /// minutes). `cdept sim run --timeout` comes first.
    #[serde(
        skip_serializing_if = "String::is_empty",
        deserialize_with = "text_value"
    )]
    pub timeout: String,
    /// The model set of `[models]` in tech.toml. Empty: the technology's default.
    pub models: String,
    /// The corner sets to run. Empty: the technology's default corner.
    pub corners: Vec<String>,
    /// Temperatures in °C, one run each. Empty: the simulator's own (27).
    pub temperatures: Vec<f64>,
    /// Design variables, written as `.param name=value`.
    pub variables: BTreeMap<String, String>,
    /// `.options name=value`.
    pub options: BTreeMap<String, String>,
    /// Vectors to keep (`.save`). Empty: everything.
    pub save: Vec<String>,
    #[serde(rename = "analysis")]
    pub analyses: Vec<Analysis>,
    /// `.meas` statements without the `.meas`, e.g. `tran delay trig v(in) …`.
    pub measures: Vec<String>,
    #[serde(rename = "spec")]
    pub specs: Vec<Spec>,
    /// Calculator expressions evaluated on each run's raw file ([`crate::calc`]); their
    /// numbers are measurements like `.meas` ones (specs may name them).
    #[serde(rename = "output")]
    pub outputs: Vec<Output>,
    /// Design-variable sweeps, crossed with each other, the corners and the temperatures.
    #[serde(rename = "sweep")]
    pub sweeps: Vec<Sweep>,
    /// Monte Carlo: when present, a run is `runs` independent seeded points of the
    /// technology's statistical corner (instead of the corner grid).
    pub montecarlo: Option<MonteCarlo>,
    /// A test template (docs/TESTS.md) this setup starts from: its analyses, outputs and
    /// specs are laid under the fields written here. Built-in or `<project>/templates/`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub template: String,
    /// The template's parameters (`out = "out"`, `vdd = 1.2`), kept as text.
    #[serde(
        skip_serializing_if = "BTreeMap::is_empty",
        deserialize_with = "text_map"
    )]
    pub template_args: BTreeMap<String, String>,
    /// CACE data this testbench came from (`cdept cace import`): the parameter as CACE
    /// YAML and what CDePT has no field for. Not used to run; `cdept cace export` reads
    /// it (docs/reference/files/cace.md).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cace: Option<toml::Table>,
    /// Nets and elements the netlist must have (from the template; not a file field).
    #[serde(skip)]
    pub requires_nets: Vec<String>,
    #[serde(skip)]
    pub requires_elements: Vec<String>,
    /// What the template suggests to plot (not a file field).
    #[serde(skip)]
    pub plot_hints: Vec<String>,
}

/// A string, or a number written without quotes (read as its text).
fn text_value<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum V {
        S(String),
        I(i64),
        F(f64),
    }
    Ok(match V::deserialize(d)? {
        V::S(s) => s,
        V::I(i) => i.to_string(),
        V::F(f) => fmt_num(f),
    })
}

fn text_map<'de, D: serde::Deserializer<'de>>(d: D) -> Result<BTreeMap<String, String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum V {
        S(String),
        I(i64),
        F(f64),
        B(bool),
    }
    Ok(BTreeMap::<String, V>::deserialize(d)?
        .into_iter()
        .map(|(k, v)| {
            let v = match v {
                V::S(s) => s,
                V::I(i) => i.to_string(),
                V::F(f) => fmt_num(f),
                V::B(b) => b.to_string(),
            };
            (k, v)
        })
        .collect())
}

/// A sweep of one design variable (`.param`). The values are written as given:
/// `values = ["1.0", "1.2"]` (or numbers, or `list`), or `start`/`stop`/`step`
/// (inclusive, linear).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Sweep {
    pub variable: String,
    #[serde(alias = "list", deserialize_with = "strings_or_numbers")]
    pub values: Vec<String>,
    pub start: Option<f64>,
    pub stop: Option<f64>,
    pub step: Option<f64>,
}

fn strings_or_numbers<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum V {
        S(String),
        I(i64),
        F(f64),
    }
    Ok(Vec::<V>::deserialize(d)?
        .into_iter()
        .map(|v| match v {
            V::S(s) => s,
            V::I(i) => i.to_string(),
            V::F(f) => fmt_num(f),
        })
        .collect())
}

/// A number the way a deck and a folder name want it: no float noise (`1.2`, not
/// `1.2000000000000002`).
pub fn fmt_num(v: f64) -> String {
    let r = (v * 1e9).round() / 1e9;
    if r.fract() == 0.0 && r.abs() < 1e15 {
        format!("{}", r as i64)
    } else {
        format!("{r}")
    }
}

impl Sweep {
    /// The values to run, in order.
    pub fn expand(&self) -> Result<Vec<String>> {
        if !self.values.is_empty() {
            return Ok(self.values.clone());
        }
        let (Some(start), Some(stop), Some(step)) = (self.start, self.stop, self.step) else {
            bail!(
                "sweep '{}' needs `values` or start, stop and step",
                self.variable
            );
        };
        if !(step.is_finite() && step != 0.0) || !start.is_finite() || !stop.is_finite() {
            bail!("sweep '{}': bad start/stop/step", self.variable);
        }
        if (stop - start) * step < 0.0 {
            bail!(
                "sweep '{}': step {step} does not lead from {start} to {stop}",
                self.variable
            );
        }
        let n = ((stop - start) / step + 1e-9).floor() as usize + 1;
        if n > 10_000 {
            bail!("sweep '{}' has {n} values", self.variable);
        }
        Ok((0..n).map(|i| fmt_num(start + step * i as f64)).collect())
    }
}

/// What a Monte Carlo run draws.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McMode {
    /// Global process variation (the technology's statistical corner, `mc`).
    #[default]
    Process,
    /// Local per-device mismatch (the technology's `tt_mm` corner).
    Mismatch,
    /// Both (the technology's `mc_mm` corner, or `mc` with the mismatch switch on).
    Both,
}

impl McMode {
    pub fn name(self) -> &'static str {
        match self {
            McMode::Process => "process",
            McMode::Mismatch => "mismatch",
            McMode::Both => "both",
        }
    }
}

impl std::str::FromStr for McMode {
    type Err = String;
    fn from_str(s: &str) -> Result<McMode, String> {
        match s {
            "process" => Ok(McMode::Process),
            "mismatch" => Ok(McMode::Mismatch),
            "both" => Ok(McMode::Both),
            other => Err(format!(
                "Monte Carlo mode '{other}' is not process, mismatch or both"
            )),
        }
    }
}

fn is_process(m: &McMode) -> bool {
    *m == McMode::Process
}

/// Monte Carlo runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MonteCarlo {
    pub runs: u32,
    /// The base seed; run `i` gets [`crate::sim::deck::mc_seed`]`(seed, i)`.
    pub seed: u64,
    /// The corner set that samples process variation (the technology's statistical
    /// one). Empty: `mc`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub corner: String,
    /// What is drawn: `process` (default), `mismatch` or `both` ([`crate::sim::mc`]).
    #[serde(skip_serializing_if = "is_process")]
    pub mode: McMode,
    /// The corner set for `mismatch`. Empty: `tt_mm`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub mismatch_corner: String,
    /// The corner set for `both`. Empty: `mc_mm`, else `corner` with the model's
    /// mismatch switch (`mc_mm_switch`) set.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub both_corner: String,
}

impl Default for MonteCarlo {
    fn default() -> Self {
        MonteCarlo {
            runs: 10,
            seed: 1,
            corner: String::new(),
            mode: McMode::Process,
            mismatch_corner: String::new(),
            both_corner: String::new(),
        }
    }
}

/// One analysis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum Analysis {
    Op,
    Dc {
        source: String,
        start: String,
        stop: String,
        step: String,
    },
    Ac {
        /// `dec`, `oct` or `lin`.
        #[serde(default = "dec")]
        sweep: String,
        points: u32,
        start: String,
        stop: String,
    },
    Tran {
        step: String,
        stop: String,
        #[serde(default)]
        start: String,
    },
    Noise {
        output: String,
        source: String,
        #[serde(default = "dec")]
        sweep: String,
        points: u32,
        start: String,
        stop: String,
    },
}

fn dec() -> String {
    "dec".into()
}

/// A measurement computed by the calculator from the run's waveforms:
/// `[[output]] name = "gain" expr = "db20(value_at(v(\"out\")/v(\"in\"), 1k))"`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Output {
    pub name: String,
    /// The expression (docs/CALC.md); it must give a number.
    pub expr: String,
    /// Look only in this plot of the raw file: `tran`, `ac`, … or part of its name.
    pub plot: String,
}

/// A bound on a measurement.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Spec {
    /// The measurement (the name given to `.meas`).
    pub name: String,
    pub min: Option<f64>,
    pub max: Option<f64>,
}

impl Setup {
    pub fn parse(text: &str) -> Result<Setup> {
        Self::parse_with(text, super::template::TemplateSet::builtin())
    }

    /// [`Setup::parse`] with a set of test templates to pick from (a project's).
    pub fn parse_with(text: &str, templates: &super::template::TemplateSet) -> Result<Setup> {
        let resolved = super::template::resolve(text, templates)?;
        let mut setup: Setup =
            toml::from_str(&resolved.text).wrap_err("not a valid testbench setup")?;
        setup.requires_nets = resolved.requires_nets;
        setup.requires_elements = resolved.requires_elements;
        setup.plot_hints = resolved.plot;
        setup.check()?;
        // The technology's default models are for its default simulator (ngspice).
        if setup.simulator == "xyce" && setup.models.is_empty() {
            setup.models = "xyce".into();
        }
        if setup.simulator == "vacask" && setup.models.is_empty() {
            setup.models = "vacask".into();
        }
        Ok(setup)
    }

    /// Run this setup with another simulator (`--simulator`, a suite's `simulator`): the
    /// name is checked, the model set that belonged to the old simulator's default goes
    /// back to the new simulator's, and what the new simulator cannot run is refused.
    pub fn set_simulator(&mut self, name: &str) -> Result<()> {
        if super::run::SimKind::of(name).is_none() {
            bail!("simulator '{name}' is not supported (ngspice, xyce and vacask are)");
        }
        let name = if name == "ngspice" { "" } else { name };
        if super::run::SimKind::of(&self.simulator) == super::run::SimKind::of(name) {
            return Ok(());
        }
        if matches!(self.models.as_str(), "xyce" | "vacask") && self.models == self.simulator {
            self.models.clear();
        }
        self.simulator = name.to_string();
        if matches!(name, "xyce" | "vacask") && self.models.is_empty() {
            self.models = name.to_string();
        }
        if !name.is_empty() && self.simulator_backend == "inproc" {
            self.simulator_backend.clear();
        }
        super::caps::check(self)
    }

    /// The `timeout` of the testbench: `None` when it has none (the project's applies),
    /// zero for "0" (no limit).
    pub fn time_limit(&self) -> Result<Option<std::time::Duration>> {
        if self.timeout.trim().is_empty() {
            return Ok(None);
        }
        crate::jobs::parse_duration(&self.timeout)
            .map(Some)
            .map_err(|e| eyre::eyre!("timeout: {e}"))
    }

    pub fn load(file: &Path) -> Result<Setup> {
        Self::load_with(file, super::template::TemplateSet::builtin())
    }

    pub fn load_with(file: &Path, templates: &super::template::TemplateSet) -> Result<Setup> {
        let text = std::fs::read_to_string(file)
            .wrap_err_with(|| format!("cannot read {}", file.display()))?;
        Self::parse_with(&text, templates).wrap_err_with(|| file.display().to_string())
    }

    fn check(&self) -> Result<()> {
        if !matches!(self.simulator.as_str(), "" | "ngspice" | "xyce" | "vacask") {
            bail!(
                "simulator '{}' is not supported yet (ngspice, xyce and vacask are)",
                self.simulator
            );
        }
        self.time_limit()?;
        if super::run::Backend::of(&self.simulator_backend).is_none() {
            bail!(
                "simulator_backend '{}' is not process or inproc",
                self.simulator_backend
            );
        }
        if self.simulator == "xyce"
            && self
                .analyses
                .iter()
                .filter(|a| !matches!(a, Analysis::Op))
                .count()
                > 1
        {
            bail!("Xyce runs one analysis per deck (op may come with it); split the testbench");
        }
        for sweep in &self.sweeps {
            sweep.expand()?;
            if sweep.variable.is_empty() || sweep.variable.contains(char::is_whitespace) {
                bail!("'{}' is not a valid sweep variable", sweep.variable);
            }
            if self
                .sweeps
                .iter()
                .filter(|o| o.variable == sweep.variable)
                .count()
                > 1
            {
                bail!("variable '{}' is swept twice", sweep.variable);
            }
        }
        if let Some(mc) = &self.montecarlo
            && (mc.runs == 0 || mc.runs > 100_000)
        {
            bail!("montecarlo runs must be 1..100000");
        }
        super::caps::check(self)?;
        let word = |s: &str| !s.is_empty() && !s.contains(char::is_whitespace);
        for name in self.variables.keys().chain(self.options.keys()) {
            if !word(name) {
                bail!("'{name}' is not a valid name");
            }
        }
        for spec in &self.specs {
            if spec.min.is_none() && spec.max.is_none() {
                bail!("spec '{}' has neither min nor max", spec.name);
            }
            if let (Some(min), Some(max)) = (spec.min, spec.max)
                && min > max
            {
                bail!("spec '{}': min {min} is above max {max}", spec.name);
            }
        }
        for (i, o) in self.outputs.iter().enumerate() {
            if !word(&o.name) {
                bail!("'{}' is not a valid output name", o.name);
            }
            if self.outputs[..i]
                .iter()
                .any(|p| p.name.eq_ignore_ascii_case(&o.name))
            {
                bail!("output '{}' is defined twice", o.name);
            }
            if let Err(e) = crate::calc::parse(&o.expr) {
                bail!("output '{}': {}", o.name, e.render(&o.expr));
            }
        }
        for a in &self.analyses {
            if let Analysis::Tran { step, stop, .. } = a
                && (step.is_empty() || stop.is_empty())
            {
                bail!("a tran analysis needs step and stop");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_testbench() {
        let s = Setup::parse(
            r#"
schematic = "bench/tb_buf"
corners = ["tt", "ff"]
temperatures = [-40, 27, 125]
save = ["v(out)"]
measures = ["tran delay trig v(in) val=0.9 rise=1 targ v(out) val=0.9 rise=1"]
[variables]
vdd = "1.8"
[[analysis]]
kind = "tran"
step = "20p"
stop = "12n"
[[analysis]]
kind = "ac"
points = 10
start = "1"
stop = "1G"
[[spec]]
name = "delay"
max = 2e-10
"#,
        )
        .unwrap();
        assert_eq!(s.corners, ["tt", "ff"]);
        assert_eq!(s.temperatures.len(), 3);
        assert_eq!(s.analyses.len(), 2);
        assert!(matches!(&s.analyses[1], Analysis::Ac { sweep, .. } if sweep == "dec"));
        assert_eq!(s.specs[0].max, Some(2e-10));
    }

    #[test]
    fn sweeps_and_montecarlo() {
        let s = Setup::parse(
            "[[sweep]]\nvariable=\"vdd\"\nvalues=[\"1.0\", 1.2, 2]\n[[sweep]]\nvariable=\"c\"\nstart=0.1\nstop=0.5\nstep=0.1\n\
             [montecarlo]\nruns=5\nseed=7",
        )
        .unwrap();
        assert_eq!(s.sweeps[0].expand().unwrap(), ["1.0", "1.2", "2"]);
        assert_eq!(
            s.sweeps[1].expand().unwrap(),
            ["0.1", "0.2", "0.3", "0.4", "0.5"]
        );
        assert_eq!(
            s.montecarlo,
            Some(MonteCarlo {
                runs: 5,
                seed: 7,
                ..Default::default()
            })
        );
        let l = Setup::parse("[[sweep]]\nvariable=\"a\"\nlist=[\"x\"]").unwrap();
        assert_eq!(l.sweeps[0].values, ["x"]);
        assert_eq!(
            Sweep {
                variable: "a".into(),
                start: Some(3.0),
                stop: Some(1.0),
                step: Some(-1.0),
                ..Default::default()
            }
            .expand()
            .unwrap(),
            ["3", "2", "1"]
        );
        assert!(Setup::parse("[[sweep]]\nvariable=\"a\"").is_err());
        assert!(Setup::parse("[[sweep]]\nvariable=\"a\"\nstart=1\nstop=2\nstep=-1").is_err());
        assert!(
            Setup::parse(
                "[[sweep]]\nvariable=\"a\"\nlist=[\"1\"]\n[[sweep]]\nvariable=\"a\"\nlist=[\"2\"]"
            )
            .is_err()
        );
        assert!(Setup::parse("[montecarlo]\nruns=0").is_err());
        // Xyce does Monte Carlo through `.sampling`.
        assert!(Setup::parse("simulator=\"xyce\"\n[montecarlo]\nruns=2").is_ok());
        let m = Setup::parse("[montecarlo]\nmode=\"both\"\nmismatch_corner=\"ss_mm\"")
            .unwrap()
            .montecarlo
            .unwrap();
        assert_eq!(
            (m.mode, m.mismatch_corner.as_str()),
            (McMode::Both, "ss_mm")
        );
        assert!(Setup::parse("[montecarlo]\nmode=\"local\"").is_err());
    }

    #[test]
    fn outputs_are_checked_when_the_setup_loads() {
        let s =
            Setup::parse("[[output]]\nname=\"gain\"\nexpr=\"db20(v(\\\"out\\\"))\"\nplot=\"ac\"")
                .unwrap();
        assert_eq!(
            (s.outputs[0].name.as_str(), s.outputs[0].plot.as_str()),
            ("gain", "ac")
        );
        let e = Setup::parse("[[output]]\nname=\"g\"\nexpr=\"max(\"").unwrap_err();
        assert!(format!("{e:#}").contains("output 'g'"), "{e:#}");
        assert!(
            Setup::parse("[[output]]\nname=\"g\"\nexpr=\"1\"\n[[output]]\nname=\"G\"\nexpr=\"2\"")
                .is_err()
        );
        assert!(Setup::parse("[[output]]\nname=\"a b\"\nexpr=\"1\"").is_err());
        assert!(Setup::parse("[[output]]\nname=\"a\"\nexpr=\"1\"\nbogus=1").is_err());
    }

    #[test]
    fn rejects_mistakes() {
        assert!(Setup::parse("typo = 1").is_err());
        assert!(Setup::parse("simulator = \"spectre\"").is_err());
        assert!(Setup::parse("simulator = \"xyce\"\n[[analysis]]\nkind=\"tran\"\nstep=\"1n\"\nstop=\"1u\"\n[[analysis]]\nkind=\"op\"").is_ok());
        assert!(Setup::parse("simulator = \"xyce\"\n[[analysis]]\nkind=\"tran\"\nstep=\"1n\"\nstop=\"1u\"\n[[analysis]]\nkind=\"ac\"\npoints=1\nstart=\"1\"\nstop=\"2\"").is_err());
        assert_eq!(Setup::parse("simulator = \"xyce\"").unwrap().models, "xyce");
        assert!(Setup::parse("[[spec]]\nname = \"a\"").is_err());
        assert!(Setup::parse("[[spec]]\nname = \"a\"\nmin = 2\nmax = 1").is_err());
        assert!(Setup::parse("[[analysis]]\nkind = \"tran\"\nstep = \"\"\nstop = \"1n\"").is_err());
    }
}
