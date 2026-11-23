//! What each simulator supports, in one place: the refusals at load time and the forms of
//! a UI read it instead of knowing the simulators themselves.

use eyre::{Result, bail};

use super::run::SimKind;
use super::setup::{Analysis, Setup};

/// What a simulator can do for a testbench.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    /// `setup.toml`'s name of the simulator.
    pub name: &'static str,
    /// The analysis kinds of `[[analysis]]` it runs.
    pub analyses: &'static [&'static str],
    /// The `sweep` words of `ac` and `noise` it knows.
    pub sweeps: &'static [&'static str],
    /// `[montecarlo]` modes (`process`, `mismatch`, `both`) it can draw. Whether a
    /// technology's models bring the statistical corners is another matter
    /// ([`super::mc::plan`] says so).
    pub mc_modes: &'static [&'static str],
    /// A transient that saves from a start time (`start` of `tran`).
    pub tran_start: bool,
    /// `.ic` and `.nodeset` cards of a testbench netlist.
    pub initial_conditions: bool,
    /// The simulator evaluates `.meas` cards itself (else CDePT does, on the raw file).
    pub native_measures: bool,
    /// It can run inside CDePT's process (`simulator_backend = "inproc"`).
    pub in_process: bool,
    /// A running point can be cancelled (the process is killed).
    pub cancellable: bool,
}

const ALL_SWEEPS: &[&str] = &["dec", "oct", "lin"];
const ALL_ANALYSES: &[&str] = &["op", "dc", "ac", "tran", "noise"];
const ALL_MC: &[&str] = &["process", "mismatch", "both"];

impl SimKind {
    /// What this simulator supports.
    pub fn capabilities(self) -> Capabilities {
        match self {
            SimKind::Ngspice => Capabilities {
                name: "ngspice",
                analyses: ALL_ANALYSES,
                sweeps: ALL_SWEEPS,
                mc_modes: ALL_MC,
                tran_start: true,
                initial_conditions: true,
                native_measures: true,
                in_process: true,
                cancellable: true,
            },
            SimKind::Xyce => Capabilities {
                name: "xyce",
                analyses: ALL_ANALYSES,
                sweeps: ALL_SWEEPS,
                mc_modes: ALL_MC,
                tran_start: true,
                initial_conditions: true,
                native_measures: true,
                in_process: false,
                cancellable: true,
            },
            SimKind::Vacask => Capabilities {
                name: "vacask",
                analyses: ALL_ANALYSES,
                sweeps: ALL_SWEEPS,
                mc_modes: ALL_MC,
                tran_start: false,
                initial_conditions: false,
                native_measures: false,
                in_process: false,
                cancellable: true,
            },
        }
    }
}

/// The analysis kind word of an `[[analysis]]`.
pub fn analysis_kind(a: &Analysis) -> &'static str {
    match a {
        Analysis::Op => "op",
        Analysis::Dc { .. } => "dc",
        Analysis::Ac { .. } => "ac",
        Analysis::Tran { .. } => "tran",
        Analysis::Noise { .. } => "noise",
    }
}

/// Refuse a setup the simulator it names cannot run, with the reason. (`simulator` that
/// is not a known name is the caller's error.)
pub fn check(setup: &Setup) -> Result<()> {
    let Some(kind) = SimKind::of(&setup.simulator) else {
        return Ok(());
    };
    let caps = kind.capabilities();
    for a in &setup.analyses {
        let k = analysis_kind(a);
        if !caps.analyses.contains(&k) {
            bail!("{} does not run the {k} analysis", caps.name);
        }
        match a {
            Analysis::Tran { start, .. }
                if !caps.tran_start
                    && !start.is_empty()
                    && start.trim().parse::<f64>() != Ok(0.0) =>
            {
                bail!(
                    "a transient that saves from a start time ({start}) is not supported with {}; use start 0",
                    caps.name.to_uppercase()
                );
            }
            Analysis::Ac { sweep, .. } | Analysis::Noise { sweep, .. }
                if !caps.sweeps.contains(&sweep.to_lowercase().as_str()) =>
            {
                bail!(
                    "sweep '{sweep}' is not supported with {} ({})",
                    caps.name.to_uppercase(),
                    caps.sweeps.join(", ")
                );
            }
            _ => {}
        }
    }
    if let Some(mc) = &setup.montecarlo {
        let mode = match mc.mode {
            super::setup::McMode::Process => "process",
            super::setup::McMode::Mismatch => "mismatch",
            super::setup::McMode::Both => "both",
        };
        if !caps.mc_modes.contains(&mode) {
            bail!("Monte Carlo ({mode}) is not run with {}", caps.name);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_simulator_says_what_it_runs() {
        for kind in [SimKind::Ngspice, SimKind::Xyce, SimKind::Vacask] {
            let c = kind.capabilities();
            assert_eq!(SimKind::of(c.name), Some(kind));
            assert!(c.analyses.contains(&"tran") && c.analyses.contains(&"op"));
            assert!(c.cancellable);
        }
        assert!(SimKind::Ngspice.capabilities().in_process);
        assert!(
            !SimKind::Vacask.capabilities().in_process
                && !SimKind::Vacask.capabilities().native_measures
        );
        assert!(SimKind::Vacask.capabilities().mc_modes.contains(&"both"));
    }

    #[test]
    fn another_simulator_can_be_chosen_without_editing_the_setup() {
        let mut s = Setup::parse("simulator=\"xyce\"\n[[analysis]]\nkind=\"op\"\n").unwrap();
        assert_eq!(s.models, "xyce");
        s.set_simulator("vacask").unwrap();
        assert_eq!(
            (s.simulator.as_str(), s.models.as_str()),
            ("vacask", "vacask")
        );
        s.set_simulator("ngspice").unwrap();
        assert_eq!((s.simulator.as_str(), s.models.as_str()), ("", ""));
        assert!(s.set_simulator("spectre").is_err());
        // What the new simulator cannot run is refused with the reason.
        let mut t =
            Setup::parse("[[analysis]]\nkind=\"tran\"\nstep=\"1n\"\nstop=\"1u\"\nstart=\"0.5u\"\n")
                .unwrap();
        assert!(format!("{:#}", t.set_simulator("vacask").unwrap_err()).contains("start time"));
        let suite =
            crate::sim::suite::Suite::parse("simulator = \"vacask\"\n[[test]]\nview = \"a\"\n")
                .unwrap();
        assert_eq!(suite.simulator, "vacask");
    }

    #[test]
    fn a_setup_the_simulator_cannot_run_is_refused_with_the_reason() {
        let tran = |sim: &str, start: &str| {
            Setup::parse(&format!(
                "simulator=\"{sim}\"\n[[analysis]]\nkind=\"tran\"\nstep=\"1n\"\nstop=\"1u\"\nstart=\"{start}\"\n"
            ))
        };
        assert!(tran("ngspice", "0.5u").is_ok());
        assert!(tran("vacask", "0").is_ok());
        let e = format!("{:#}", tran("vacask", "0.5u").unwrap_err());
        assert!(e.contains("start time") && e.contains("VACASK"), "{e}");
    }
}
