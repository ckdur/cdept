//! The tutorial file (`cdept-tutorial`, version 1) and its text as Markdown.

use std::path::Path;

use serde::Deserialize;

use super::circuit::Spec;

const SCHEMA: &str = "cdept-tutorial";
const VERSION: u32 = 1;

// The tutorials, starting projects and pictures that come with the program (build.rs).
include!(concat!(env!("OUT_DIR"), "/tutorials.rs"));

/// The series, in the order they are listed: the name in the files and its title.
pub const SERIES: &[(&str, &str)] = &[
    ("setup", "Setting up"),
    ("inverter", "Inverter: the whole flow once"),
    ("ota", "Five-transistor OTA: analog design"),
];

/// A picture of the written pages by its name.
pub(crate) fn picture(name: &str) -> Option<&'static [u8]> {
    PICTURES.iter().find(|(n, _)| *n == name).map(|(_, b)| *b)
}

/// The files of one layer of a starting project: path and contents.
pub(crate) fn start_layer(layer: &str) -> impl Iterator<Item = (&'static str, &'static [u8])> {
    let layer = layer.to_string();
    START_FILES
        .iter()
        .filter(move |(l, _, _)| *l == layer)
        .map(|(_, path, bytes)| (*path, *bytes))
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tutorial {
    schema: String,
    version: u32,
    pub id: String,
    pub title: String,
    /// One of [`SERIES`].
    pub series: String,
    /// The place in the series, from 1.
    pub order: u32,
    /// The layers of the starting project (`tutorials/start/<layer>`), laid over each
    /// other; none for a tutorial that starts from nothing.
    #[serde(default)]
    pub start: Vec<String>,
    /// The PDK the starting project is made for.
    #[serde(default)]
    pub pdk: Option<String>,
    /// The circuits the steps ask about (`circuit:<id>`).
    #[serde(default, rename = "circuit")]
    pub circuits: Vec<Spec>,
    /// What the user has at the end.
    pub goal: String,
    pub minutes: u32,
    /// What to have before starting.
    #[serde(default)]
    pub before: String,
    #[serde(rename = "step")]
    pub steps: Vec<Step>,
}

/// One thing to point at: the first of a step's list that is on screen wins, so a
/// menu entry comes before the menu that holds it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Point {
    pub at: String,
    pub say: String,
    /// Not pointed at while this fact holds: what is there is done (a "Delete" for
    /// something that has since been made right).
    #[serde(default)]
    pub unless: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub id: String,
    pub title: String,
    /// Paragraphs; lines indented by four spaces are commands.
    pub text: String,
    #[serde(default)]
    pub point: Vec<Point>,
    /// The step is done when any of these facts holds.
    #[serde(default)]
    pub done_when: Vec<String>,
    /// The step has a Continue button (it is read, not only done).
    #[serde(default)]
    pub continue_button: bool,
    /// Continue waits for all of these facts.
    #[serde(default)]
    pub continue_needs: Vec<String>,
    /// Why Continue is greyed out.
    #[serde(default)]
    pub needs_hint: String,
    /// When none of these facts holds any more, the user has left the place the step
    /// happens in, and the guide goes back to `back_to`.
    #[serde(default)]
    pub lost_without: Vec<String>,
    #[serde(default)]
    pub back_to: Option<String>,
    /// What the guide says when it goes back.
    #[serde(default)]
    pub lost_says: String,
    /// A picture to show with the step: `docs/tutorials/pictures/<name>.png`.
    #[serde(default)]
    pub picture: Option<String>,
    /// The step offers to make the starting project; making it finishes the step.
    #[serde(default)]
    pub start_button: bool,
    /// Lines about this machine to show under the text: `pdk_root`, `klayout`,
    /// `xschem`.
    #[serde(default)]
    pub status: Vec<String>,
}

impl Step {
    /// Every fact the step asks about.
    pub fn facts(&self) -> impl Iterator<Item = &String> {
        self.done_when
            .iter()
            .chain(&self.continue_needs)
            .chain(&self.lost_without)
            .chain(self.point.iter().filter_map(|p| p.unless.as_ref()))
    }
}

impl Tutorial {
    pub fn parse(text: &str) -> Result<Tutorial, String> {
        let tutorial: Tutorial = toml::from_str(text).map_err(|e| e.to_string())?;
        if tutorial.schema != SCHEMA || tutorial.version != VERSION {
            return Err(format!(
                "not a {SCHEMA} file of version {VERSION} (schema {}, version {})",
                tutorial.schema, tutorial.version
            ));
        }
        tutorial.check()?;
        Ok(tutorial)
    }

    /// A built-in tutorial by its id, else the tutorial in the file of that path.
    pub fn find(name: &str) -> Result<Tutorial, String> {
        if let Some(tutorial) = builtin(name) {
            return Ok(tutorial);
        }
        let file = Path::new(name);
        if !file.is_file() {
            let ids: Vec<&str> = TUTORIAL_FILES.iter().map(|(id, _)| *id).collect();
            return Err(format!(
                "no tutorial '{name}' (there is: {}), and no such file",
                ids.join(", ")
            ));
        }
        let text = std::fs::read_to_string(file).map_err(|e| format!("{name}: {e}"))?;
        Tutorial::parse(&text).map_err(|e| format!("{name}: {e}"))
    }

    fn check(&self) -> Result<(), String> {
        if self.steps.is_empty() {
            return Err("a tutorial needs a step".into());
        }
        for (n, step) in self.steps.iter().enumerate() {
            let id = &step.id;
            if self.steps[..n].iter().any(|s| s.id == *id) {
                return Err(format!("two steps are called '{id}'"));
            }
            if step.start_button && (self.start.is_empty() || self.pdk.is_none()) {
                return Err(format!("step '{id}': start_button needs `start` and `pdk`"));
            }
            for fact in step.facts() {
                if let Some(circuit) = fact.strip_prefix("circuit:")
                    && self.circuit(circuit).is_none()
                {
                    return Err(format!("step '{id}': no circuit '{circuit}'"));
                }
            }
            if step.done_when.is_empty() && !step.continue_button && !step.start_button {
                return Err(format!("step '{id}' can never be finished"));
            }
            if step.lost_without.is_empty() != step.back_to.is_none() {
                return Err(format!("step '{id}': lost_without and back_to go together"));
            }
            // Back only: going forward would skip steps.
            if let Some(back) = &step.back_to
                && !self.steps[..n].iter().any(|s| s.id == *back)
            {
                return Err(format!("step '{id}': no earlier step '{back}'"));
            }
        }
        Ok(())
    }

    pub fn circuit(&self, id: &str) -> Option<&Spec> {
        self.circuits.iter().find(|c| c.id == id)
    }

    pub fn index_of(&self, id: &str) -> Option<usize> {
        self.steps.iter().position(|s| s.id == id)
    }

    /// What the guide checks in a step, for the written page.
    fn checklist(&self, step: &Step) -> String {
        let mut out = String::new();
        let mut seen: Vec<&str> = Vec::new();
        for fact in step.done_when.iter().chain(&step.continue_needs) {
            let Some(circuit) = fact.strip_prefix("circuit:").and_then(|c| self.circuit(c)) else {
                continue;
            };
            if seen.contains(&circuit.id.as_str()) {
                continue;
            }
            seen.push(&circuit.id);
            out += &format!(
                "\nThe guide checks the schematic `{}`:\n\n\
                 | Part | Symbol | Values | Pins on nets |\n|---|---|---|---|\n",
                circuit.view
            );
            let list = |map: &std::collections::BTreeMap<String, String>| {
                let items: Vec<String> = map
                    .iter()
                    .map(|(k, v)| {
                        // `?name` is a net of the user's own, `0|GND` either of two.
                        let v = match v.strip_prefix('?') {
                            Some(name) => format!("(the {name} net)"),
                            None => v.split('|').next().unwrap_or(v).to_string(),
                        };
                        format!("`{k}={v}`")
                    })
                    .collect();
                items.join(" ")
            };
            for part in &circuit.parts {
                out += &format!(
                    "| {} | `{}` | {} | {} |\n",
                    part.what,
                    part.symbol,
                    list(&part.with),
                    list(&part.pins)
                );
            }
        }
        out
    }

    /// The written form of the tutorial: `docs/tutorials/<id>.md`.
    pub fn markdown(&self) -> String {
        let mut out = format!(
            "# {}\n\n<!-- Made from cdept-gui/tutorials/{id}.toml by `cdept tutorial {id} \
             --print`. Edit that file, not this one. -->\n\n{}\n\nAbout {} minutes. To be \
             guided through it inside CDePT:\n\n```sh\ncdept tutorial {id}\n```\n",
            self.title,
            self.goal.trim(),
            self.minutes,
            id = self.id,
        );
        if !self.before.trim().is_empty() {
            out += &format!("\n## Before you start\n\n{}\n", self.before.trim_end());
        }
        for (n, step) in self.steps.iter().enumerate() {
            // `{project}` is filled in by the running guide.
            let text = step
                .text
                .trim_end()
                .replace("{project}", "<your project folder>");
            out += &format!("\n## {}. {}\n\n{text}\n", n + 1, step.title);
            if let Some(name) = &step.picture {
                out += &format!("\n![{}](pictures/{name}.png)\n", step.title);
            }
            out += &self.checklist(step);
        }
        out
    }
}

pub fn builtin(id: &str) -> Option<Tutorial> {
    let (_, text) = TUTORIAL_FILES.iter().find(|(known, _)| *known == id)?;
    Some(Tutorial::parse(text).expect("the built-in tutorials are tested"))
}

/// The built-in tutorials, series by series.
pub fn builtins() -> Vec<Tutorial> {
    let mut all: Vec<Tutorial> = TUTORIAL_FILES
        .iter()
        .filter_map(|(id, _)| builtin(id))
        .collect();
    let series = |t: &Tutorial| SERIES.iter().position(|(s, _)| *s == t.series);
    all.sort_by_key(|t| (series(t), t.order));
    all
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guide::{ANCHORS, FACTS};

    fn known(list: &[&str], name: &str) -> bool {
        list.iter()
            .any(|k| *k == name || (k.ends_with(['.', ':']) && name.starts_with(k)))
    }

    #[test]
    fn the_builtin_tutorials_name_what_the_program_marks() {
        let mut places = Vec::new();
        for (id, text) in TUTORIAL_FILES {
            let tutorial = Tutorial::parse(text).unwrap_or_else(|e| panic!("{id}: {e}"));
            assert_eq!(tutorial.id, *id);
            assert!(SERIES.iter().any(|(s, _)| *s == tutorial.series), "{id}");
            places.push((tutorial.series.clone(), tutorial.order));
            for layer in &tutorial.start {
                assert!(
                    start_layer(layer).next().is_some(),
                    "{id}: no layer {layer}"
                );
            }
            for step in &tutorial.steps {
                for point in &step.point {
                    assert!(known(ANCHORS, &point.at), "{id}/{}: {}", step.id, point.at);
                }
                for fact in step.facts() {
                    assert!(known(FACTS, fact), "{id}/{}: {fact}", step.id);
                }
                for status in &step.status {
                    assert!(["pdk_root", "klayout", "xschem"].contains(&status.as_str()));
                }
                if let Some(name) = &step.picture {
                    assert!(
                        picture(name).is_some(),
                        "{id}/{}: no picture {name}",
                        step.id
                    );
                }
            }
        }
        let count = places.len();
        places.sort();
        places.dedup();
        assert_eq!(
            places.len(),
            count,
            "two tutorials at one place of a series"
        );
    }

    #[test]
    fn the_written_tutorials_are_up_to_date() {
        let docs = Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/tutorials");
        for tutorial in builtins() {
            let file = docs.join(format!("{}.md", tutorial.id));
            let written = std::fs::read_to_string(&file).unwrap_or_default();
            assert!(
                written == tutorial.markdown(),
                "{} is out of date: cdept tutorial {} --print > docs/tutorials/{}.md",
                file.display(),
                tutorial.id,
                tutorial.id
            );
        }
    }

    #[test]
    fn mistakes_are_refused() {
        let head = "schema='cdept-tutorial'\nversion=1\nid='x'\ntitle='X'\nseries='setup'\norder=1\ngoal='g'\nminutes=1\n";
        let step = |rest: &str| format!("{head}[[step]]\nid='a'\ntitle='A'\ntext='t'\n{rest}");
        assert!(Tutorial::parse(&step("continue_button=true")).is_ok());
        assert!(Tutorial::parse(&step("")).unwrap_err().contains("never"));
        let back = step("continue_button=true\nlost_without=['wizard.open']\nback_to='b'");
        assert!(
            Tutorial::parse(&back)
                .unwrap_err()
                .contains("no earlier step")
        );
        assert!(Tutorial::parse(&step("continue_button=true\ntypo=1")).is_err());
    }
}
