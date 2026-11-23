//! Monte Carlo: which model corner sets (and switches) a mode draws from.
//!
//! The PDKs do it two ways (docs/SIMULATION.md section 5):
//!
//! - IHP SG13G2: process variation is the `*_stat` sections (`mc` corner set:
//!   `gauss()` parameters, drawn once per circuit), per-device mismatch the
//!   `*_mismatch` sections (`tt_mm`: `agauss()` on every instance whose `mm_ok` is 1).
//!   The PDK has no corner with both; a technology snapshot may add `mc_mm`.
//! - sky130A: `mc` is the process corner (`mc_pr_switch=1`) and `tt_mm` the mismatch
//!   one (`mc_mm_switch=1`); both are switches, so "both" is `mc` with
//!   `.param mc_mm_switch=1` after it.

use eyre::{Result, bail};

use super::setup::{McMode, MonteCarlo};

/// The model lines of a Monte Carlo run: the lines of these corner sets in order, then
/// these extra lines (`.param` switches).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// What the run's folder and record call its corner (`mc`, `tt_mm`, `mc_mm`).
    pub label: String,
    pub corners: Vec<String>,
    pub extra: Vec<String>,
}

/// The mismatch switch of sky130-style models.
pub const MISMATCH_SWITCH: &str = "mc_mm_switch";

/// Choose the corner sets for a mode. `has` tells whether the model set has a corner
/// set, `mentions` whether the files of a corner set mention a word, `xyce` that the
/// models are the Xyce ones.
pub fn plan(
    mc: &MonteCarlo,
    set: &str,
    xyce: bool,
    has: impl Fn(&str) -> bool,
    mentions: impl Fn(&str, &str) -> bool,
) -> Result<Plan> {
    let process = if mc.corner.is_empty() { "mc" } else { mc.corner.as_str() };
    let need = |corner: &str, what: &str| -> Result<()> {
        if has(corner) {
            return Ok(());
        }
        bail!("Monte Carlo ({what}) needs the corner set '{corner}' in the models '{set}'; name another in [montecarlo]");
    };
    let single = |corner: &str| Plan { label: corner.to_string(), corners: vec![corner.to_string()], extra: vec![] };
    match mc.mode {
        McMode::Process => {
            need(process, "process")?;
            Ok(single(process))
        }
        McMode::Mismatch => {
            if xyce && mc.mismatch_corner.is_empty() {
                bail!(
                    "mismatch Monte Carlo with Xyce needs `mismatch_corner` in [montecarlo]: the PDK's Xyce models \
                     have no mismatch sections for the transistors (their `tt_mm` only covers the MOS capacitor)"
                );
            }
            let corner = if mc.mismatch_corner.is_empty() { "tt_mm" } else { mc.mismatch_corner.as_str() };
            need(corner, "mismatch")?;
            Ok(single(corner))
        }
        McMode::Both => {
            if !mc.both_corner.is_empty() {
                need(&mc.both_corner, "process and mismatch")?;
                return Ok(single(&mc.both_corner));
            }
            if xyce {
                bail!("process + mismatch Monte Carlo with Xyce needs `both_corner` in [montecarlo] (the PDK's Xyce models have no transistor mismatch)");
            }
            if has("mc_mm") {
                return Ok(single("mc_mm"));
            }
            if has(process) && mentions(process, MISMATCH_SWITCH) {
                return Ok(Plan {
                    label: "mc_mm".into(),
                    corners: vec![process.to_string()],
                    extra: vec![format!(".param {MISMATCH_SWITCH}=1")],
                });
            }
            Err(eyre::eyre!(
                "the models '{set}' have no corner set with process variation and mismatch together: add one \
                 (the corner list of tech.toml) and name it as `both_corner` in [montecarlo]"
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mc(mode: McMode) -> MonteCarlo {
        MonteCarlo { mode, ..Default::default() }
    }
    const IHP: &[&str] = &["mc", "tt", "tt_mm"];

    #[test]
    fn modes_pick_their_corners() {
        let has = |c: &str| IHP.contains(&c);
        let no = |_: &str, _: &str| false;
        assert_eq!(plan(&mc(McMode::Process), "ngspice", false, has, no).unwrap(), Plan { label: "mc".into(), corners: vec!["mc".into()], extra: vec![] });
        assert_eq!(plan(&mc(McMode::Mismatch), "ngspice", false, has, no).unwrap().corners, ["tt_mm"]);
        // IHP: no corner with both, unless the snapshot adds mc_mm.
        assert!(plan(&mc(McMode::Both), "ngspice", false, has, no).is_err());
        let with = |c: &str| has(c) || c == "mc_mm";
        assert_eq!(plan(&mc(McMode::Both), "ngspice", false, with, no).unwrap().label, "mc_mm");
        let m = MonteCarlo { mode: McMode::Both, both_corner: "x".into(), ..Default::default() };
        assert!(plan(&m, "ngspice", false, has, no).is_err());
    }

    #[test]
    fn switch_style_models_get_the_mismatch_switch() {
        let has = |c: &str| IHP.contains(&c);
        let yes = |_: &str, w: &str| w == MISMATCH_SWITCH;
        let p = plan(&mc(McMode::Both), "ngspice", false, has, yes).unwrap();
        assert_eq!((p.corners, p.extra), (vec!["mc".to_string()], vec![".param mc_mm_switch=1".to_string()]));
    }

    #[test]
    fn xyce_mismatch_needs_an_explicit_corner() {
        let has = |c: &str| IHP.contains(&c);
        let no = |_: &str, _: &str| false;
        assert!(plan(&mc(McMode::Mismatch), "xyce", true, has, no).is_err());
        let m = MonteCarlo { mode: McMode::Mismatch, mismatch_corner: "tt_mm".into(), ..Default::default() };
        assert!(plan(&m, "xyce", true, has, no).is_ok());
        assert!(plan(&mc(McMode::Process), "xyce", true, has, no).is_ok());
    }
}
