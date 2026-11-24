//! Whether VACASK can run a project's technology, decided cheaply: no model is
//! converted and no program but a path lookup is started. The Simulation tab, `cdept sim
//! run|suite` and `cdept tools` ask it before offering or starting a VACASK run.
//!
//! A project is [`Availability::Ready`] for VACASK when
//!
//! 1. the technology has VACASK models, hand-written (`[models.vacask]` without
//!    `generated`) or makeable from its `[models.ngspice]` (docs/VACASK.md section 2): the
//!    set must not hold **binned BSIM3/BSIM4 SPICE models** (`.model … level=14|54|8|49`
//!    with `lmin`/`lmax`/`wmin`/`wmax`, how sky130 writes its transistors), which VACASK's
//!    converter (`ng2vc`) cannot convert (no binning, no `M` cards), so sky130A and
//!    sky130B are unavailable. A technology whose transistors are OSDI models (the PSP103
//!    of ihp-sg13g2) is not affected;
//! 2. the VACASK program exists (`$CDEPT_VACASK` or `PATH`);
//! 3. when Verilog-A models have to be compiled and the compiled libraries are not in the
//!    project's cache yet, OpenVAF exists (and `python3`, the converter's interpreter).
//!
//! What costs time (reading the model files for rule 1) is done once per project and
//! technology and kept in memory; the rest is looking up programs and one small file.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use eyre::{Result, eyre};

use crate::Project;
use crate::batch::{openvaf, vacask as batch_vacask};

use super::models;

/// Whether VACASK can run the technology of a project, and if not, why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    Ready,
    /// The reason, one line a user can act on.
    Unavailable(String),
}

impl Availability {
    pub fn is_ready(&self) -> bool {
        matches!(self, Availability::Ready)
    }

    /// The reason when it is not available.
    pub fn reason(&self) -> Option<&str> {
        match self {
            Availability::Ready => None,
            Availability::Unavailable(why) => Some(why),
        }
    }

    /// An error carrying the reason, for a run that must not start.
    pub fn check(&self) -> Result<()> {
        match self {
            Availability::Ready => Ok(()),
            Availability::Unavailable(why) => Err(eyre!("{why}")),
        }
    }

    /// One line for `cdept tools` and `cdept vacask models`.
    pub fn line(&self) -> String {
        match self {
            Availability::Ready => "VACASK: available for this technology".to_string(),
            Availability::Unavailable(why) => format!("VACASK: not available: {why}"),
        }
    }
}

/// What reading the technology's model files says (kept per project and technology).
#[derive(Debug, Clone)]
struct TechRule {
    /// Why no VACASK models can be made.
    problem: Option<String>,
    /// The Verilog-A models to compile, and where the compiled ones go.
    stems: Vec<String>,
    cache: Option<PathBuf>,
    /// The technology's own VACASK models: nothing is made.
    own_set: bool,
}

type Cache = Mutex<HashMap<(PathBuf, String), TechRule>>;

fn cache() -> &'static Cache {
    static CACHE: std::sync::OnceLock<Cache> = std::sync::OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// Whether a program the adapters found can be started: a path must be a file, a bare
/// name a hit on `PATH`.
fn program_exists(p: &Path) -> bool {
    if p.components().count() > 1 || p.is_absolute() {
        p.is_file()
    } else {
        crate::tools::find_in_path(&p.to_string_lossy()).is_some()
    }
}

/// Can VACASK run `project`'s technology? See the module's rules. Cheap: call it
/// whenever a UI needs the answer (the model files are read once per project and
/// technology).
pub fn availability(project: &Project) -> Availability {
    match check(project) {
        Ok(()) => Availability::Ready,
        Err(why) => Availability::Unavailable(why),
    }
}

fn check(project: &Project) -> std::result::Result<(), String> {
    let Some(tech) = project.tech.as_ref() else {
        return Err(
            "the project has no technology snapshot (cdept tech scan), so there are no models for VACASK"
                .to_string(),
        );
    };
    // The technology first: installing VACASK does not help when its models cannot be made.
    let rule = rule(project, tech);
    if let Some(why) = &rule.problem {
        return Err(why.clone());
    }
    match batch_vacask::ADAPTER.locate() {
        Some(p) if program_exists(&p) => {}
        Some(p) => {
            return Err(format!(
                "VACASK not found: {} (from {}) is not a program; put vacask on PATH or fix {}",
                p.display(),
                batch_vacask::VACASK_ENV,
                batch_vacask::VACASK_ENV
            ));
        }
        None => {
            return Err(format!(
                "VACASK not found: put vacask on PATH or set {}",
                batch_vacask::VACASK_ENV
            ));
        }
    }
    if rule.own_set || rule.stems.is_empty() {
        return Ok(());
    }
    // Verilog-A models to compile: the cache of this project may have them already.
    if let Some(dir) = &rule.cache
        && let Some(g) = std::fs::read_to_string(dir.join("cdept-vacask.json"))
            .ok()
            .and_then(|t| serde_json::from_str::<models::Generated>(&t).ok())
        && g.compiled.iter().all(|c| dir.join(c).is_file())
    {
        return models::check_libraries(&g).map_err(|e| format!("{e}"));
    }
    let stems: Vec<&str> = rule.stems.iter().map(String::as_str).collect();
    match openvaf::ADAPTER.locate() {
        Some(p) if program_exists(&p) => {}
        _ => return Err(models::openvaf_missing_message(&stems)),
    }
    if crate::tools::find_in_path("python3")
        .or_else(|| crate::tools::find_in_path("python"))
        .is_none()
    {
        return Err(
            "python3 not found: VACASK's model converter is a Python program (put python3 on PATH)"
                .to_string(),
        );
    }
    Ok(())
}

/// The technology-level part, from memory or from reading the model files.
fn rule(project: &Project, tech: &crate::tech::Tech) -> TechRule {
    let key = {
        let var = |n: &str| project.var(n);
        let ng = tech.models.set("ngspice").map(|(_, s)| s);
        let json = serde_json::to_string(&ng).unwrap_or_default();
        let dir = ng
            .and_then(|s| s.dir.resolve(var).ok())
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let own = tech
            .models
            .sets
            .get(models::SET)
            .map(|s| s.generated.is_none());
        openvaf::hash_parts([
            tech.pdk.name.as_bytes(),
            json.as_bytes(),
            dir.as_bytes(),
            format!("{own:?}").as_bytes(),
        ])
    };
    let id = (project.root.clone(), key);
    if let Some(r) = cache().lock().unwrap_or_else(|e| e.into_inner()).get(&id) {
        return r.clone();
    }
    let r = compute(project, tech);
    cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(id, r.clone());
    r
}

fn compute(project: &Project, tech: &crate::tech::Tech) -> TechRule {
    let mut r = TechRule {
        problem: None,
        stems: vec![],
        cache: None,
        own_set: false,
    };
    if let Some(set) = tech.models.sets.get(models::SET)
        && set.generated.is_none()
    {
        r.own_set = true;
        return r;
    }
    let plan = match models::Plan::new(project, tech) {
        Ok(p) => p,
        Err(e) => {
            r.problem = Some(format!("{e}"));
            return r;
        }
    };
    for s in &plan.sources {
        let Ok(bytes) = std::fs::read(&s.path) else {
            continue;
        };
        if let Some(level) = binned_bsim(&String::from_utf8_lossy(&bytes)) {
            r.problem = Some(format!(
                "the technology '{}' models its transistors as binned BSIM{} SPICE models ({}), \
                 which VACASK's converter cannot convert (no binning, no M cards): use ngspice \
                 or Xyce (docs/VACASK.md section 2)",
                plan.pdk,
                if level == 8 || level == 49 { 3 } else { 4 },
                s.name
            ));
            return r;
        }
    }
    r.stems = plan.osdi.iter().map(|(stem, _)| stem.clone()).collect();
    r.cache = Some(plan.dir(project));
    r
}

/// The BSIM level (8, 49, 14 or 54) of the first binned `.model` card of a SPICE
/// library: a `level` of a BSIM3/BSIM4 model together with `lmin` or `wmin`, which is
/// how a technology says that the model is one bin of many.
pub(super) fn binned_bsim(text: &str) -> Option<u32> {
    let mut card = String::new();
    for line in text.lines() {
        let t = line.trim();
        if let Some(more) = t.strip_prefix('+') {
            card.push(' ');
            card.push_str(more);
        } else {
            if let Some(level) = card_level(&card) {
                return Some(level);
            }
            card.clear();
            card.push_str(t);
        }
    }
    card_level(&card)
}

/// The BSIM level of a `.model` card that is one bin of a binned model.
fn card_level(card: &str) -> Option<u32> {
    let lower = card.to_ascii_lowercase();
    if !lower.starts_with(".model") {
        return None;
    }
    let words: Vec<&str> = lower
        .split(|c: char| c.is_whitespace() || matches!(c, '=' | '(' | ')' | ','))
        .filter(|w| !w.is_empty())
        .collect();
    let level = words
        .iter()
        .position(|w| *w == "level")
        .and_then(|i| words.get(i + 1))
        .and_then(|v| v.parse::<f64>().ok())
        .map(|v| v as u32)?;
    (matches!(level, 8 | 14 | 49 | 54) && words.iter().any(|w| matches!(*w, "lmin" | "wmin")))
        .then_some(level)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binned_bsim_models_are_found() {
        let sky = "* x\n.model nfet_01v8__model.0 nmos\n+ level = 54\n+ lmin = 1.0e-07 lmax = 1.0e-06 wmin = 4.2e-07\n+ wmax = 1e-6\n";
        assert_eq!(binned_bsim(sky), Some(54));
        assert_eq!(
            binned_bsim(".MODEL n nmos (LEVEL=49.0 WMIN=1u)\n"),
            Some(49)
        );
        // Not binned, or not BSIM: the models of ihp-sg13g2 and a plain level 1.
        assert_eq!(binned_bsim(".model n nmos level=54 vth0=0.4\n"), None);
        assert_eq!(binned_bsim(".model n nmos level=1 lmin=1u wmin=1u\n"), None);
        assert_eq!(binned_bsim(".model d d is=1e-14\nmodel psp103va\n"), None);
        assert_eq!(binned_bsim(""), None);
    }

    #[test]
    fn the_lines_say_what_is_wrong() {
        assert!(Availability::Ready.is_ready());
        let no = Availability::Unavailable("OpenVAF not found".into());
        assert!(!no.is_ready() && no.reason() == Some("OpenVAF not found"));
        assert!(format!("{}", no.check().unwrap_err()).contains("OpenVAF not found"));
        assert!(no.line().starts_with("VACASK: not available: "));
        assert!(Availability::Ready.line().contains("available"));
    }
}
