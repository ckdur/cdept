//! Which simulators the Simulation tab offers (SYMBI-80): VACASK is shown disabled, with
//! the reason as hover text, for a technology it cannot run (sky130's binned BSIM4
//! models, OpenVAF missing, no VACASK program: `libcdept::vacask::availability`,
//! docs/VACASK.md). A testbench file that already names VACASK there gets a problem
//! line and its run is not offered.
//!
//! The answer is cheap but not free (programs are looked up on `PATH`): it is asked when a
//! testbench opens and again every few seconds while the tab is drawn.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use egui::{ComboBox, Ui};
use libcdept::Project;
use libcdept::vacask::{Availability, availability};

use super::SimulationView;

/// How long an answer is kept while the tab is drawn.
const KEEP: Duration = Duration::from_secs(3);

/// The last answer, for the project it was asked of.
#[derive(Default)]
pub(super) struct Avail {
    last: Option<(Instant, PathBuf, Availability)>,
}

impl Avail {
    /// The answer for `project`, asked again when it is older than a few seconds or
    /// `fresh`.
    pub(super) fn get(&mut self, project: &Project, fresh: bool) -> Availability {
        if let Some((at, root, a)) = &self.last
            && !fresh
            && *root == project.root
            && at.elapsed() < KEEP
        {
            return a.clone();
        }
        let a = availability(project);
        self.last = Some((Instant::now(), project.root.clone(), a.clone()));
        a
    }

    /// The last answer (`Ready` before the first one).
    pub(super) fn last(&self) -> Availability {
        self.last
            .as_ref()
            .map_or(Availability::Ready, |(_, _, a)| a.clone())
    }
}

/// The simulator combo of the form: VACASK is a disabled entry with the reason as hover
/// text when it is not available.
pub(super) fn simulator_combo(ui: &mut Ui, current: &mut String, avail: &Availability) {
    ComboBox::from_id_salt("sim-simulator")
        .selected_text(current.as_str())
        .show_ui(ui, |ui| {
            for name in ["ngspice", "xyce", "vacask"] {
                match (name, avail.reason()) {
                    ("vacask", Some(why)) => {
                        ui.add_enabled(false, egui::Button::selectable(current == name, "vacask"))
                            .on_disabled_hover_text(why);
                    }
                    _ => {
                        ui.selectable_value(current, name.to_string(), name);
                    }
                }
            }
        });
}

/// The problem a form that names `simulator` has: VACASK where it cannot run.
pub(super) fn problem(simulator: &str, avail: &Availability) -> Option<String> {
    (simulator == "vacask")
        .then(|| {
            avail
                .reason()
                .map(|why| format!("VACASK is not available: {why}"))
        })
        .flatten()
}

/// What the combo does when `name` is picked: the simulator changes, unless it is
/// disabled (then the reason is the error).
fn select(current: &mut String, name: &str, avail: &Availability) -> Result<(), String> {
    match (name, avail.reason()) {
        ("vacask", Some(why)) => Err(format!("vacask is disabled in the combo: {why}")),
        _ => {
            *current = name.to_string();
            Ok(())
        }
    }
}

/// The verbs of this file: `sim expect_simulator_disabled <name> [reason words]`,
/// `sim expect_simulator_enabled <name>`, `sim select_simulator <name>` (what picking the
/// entry of the combo does: an error for a disabled one, the form is not changed),
/// `sim expect_select_refused <name>` (the combo refuses it and the form keeps its
/// simulator) and `sim expect_run_refused` (Run is not started: a problem, no run).
pub(super) fn script(
    view: &mut SimulationView,
    project: &Project,
    args: &[String],
) -> Option<Result<(), String>> {
    let verb = args.first()?.as_str();
    if !matches!(
        verb,
        "expect_simulator_disabled"
            | "expect_simulator_enabled"
            | "select_simulator"
            | "expect_select_refused"
            | "expect_run_refused"
    ) {
        return None;
    }
    if verb == "expect_run_refused" {
        return Some(match view.start(project, Vec::new()) {
            Err(why) if why.contains("not available") => Ok(()),
            Err(other) => Err(format!("the run was refused for another reason: {other}")),
            Ok(()) => Err("the run was started".into()),
        });
    }
    let avail = view.avail.get(project, true);
    let name = args.get(1).map(String::as_str).unwrap_or_default();
    let disabled = match name {
        "vacask" => avail.reason().map(str::to_string),
        "ngspice" | "xyce" => None,
        other => return Some(Err(format!("unknown simulator '{other}'"))),
    };
    Some(match verb {
        "expect_simulator_disabled" => match disabled {
            Some(why) if why.contains(&args[2..].join(" ")) => Ok(()),
            Some(why) => Err(format!("{name} is disabled for another reason: {why}")),
            None => Err(format!("{name} is enabled")),
        },
        "expect_simulator_enabled" => match disabled {
            None => Ok(()),
            Some(why) => Err(format!("{name} is disabled: {why}")),
        },
        "expect_select_refused" => match view.form.as_mut() {
            Some(form) => {
                let before = form.simulator.clone();
                match select(&mut form.simulator, name, &avail) {
                    Err(_) if form.simulator == before => Ok(()),
                    Err(e) => Err(format!("refused, but the form changed: {e}")),
                    Ok(()) => Err(format!("the combo took {name}")),
                }
            }
            None => Err("no testbench is open".into()),
        },
        _ => match view.form.as_mut() {
            Some(form) => select(&mut form.simulator, name, &avail),
            None => Err("no testbench is open".into()),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vacask_is_refused_with_the_reason_and_the_others_are_not() {
        let no = Availability::Unavailable("OpenVAF not found".into());
        let mut sim = "ngspice".to_string();
        let err = select(&mut sim, "vacask", &no).unwrap_err();
        assert!(err.contains("OpenVAF not found"), "{err}");
        assert_eq!(sim, "ngspice", "the form keeps its simulator");
        assert!(select(&mut sim, "xyce", &no).is_ok());
        assert_eq!(sim, "xyce");
        assert!(select(&mut sim, "vacask", &Availability::Ready).is_ok());
        assert_eq!(sim, "vacask");
    }

    #[test]
    fn only_a_form_that_names_vacask_has_a_problem() {
        let no = Availability::Unavailable("sky130A has no VACASK models".into());
        assert_eq!(
            problem("vacask", &no).as_deref(),
            Some("VACASK is not available: sky130A has no VACASK models")
        );
        assert_eq!(problem("ngspice", &no), None);
        assert_eq!(problem("vacask", &Availability::Ready), None);
    }
}
