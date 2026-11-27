//! The tutorial file (`cdept-tutorial`, version 1) and its text as Markdown.

use std::path::Path;

use serde::Deserialize;

const SCHEMA: &str = "cdept-tutorial";
const VERSION: u32 = 1;

/// The tutorials that come with the program: the id and the file.
const BUILTIN: &[(&str, &str)] = &[("pdk-setup", include_str!("../../tutorials/pdk-setup.toml"))];

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tutorial {
    schema: String,
    version: u32,
    pub id: String,
    pub title: String,
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
    /// Lines about this machine to show under the text: `pdk_root`, `klayout`,
    /// `xschem`.
    #[serde(default)]
    pub status: Vec<String>,
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
            let ids: Vec<&str> = BUILTIN.iter().map(|(id, _)| *id).collect();
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
            if step.done_when.is_empty() && !step.continue_button {
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

    pub fn index_of(&self, id: &str) -> Option<usize> {
        self.steps.iter().position(|s| s.id == id)
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
        }
        out
    }
}

pub fn builtin(id: &str) -> Option<Tutorial> {
    let (_, text) = BUILTIN.iter().find(|(known, _)| *known == id)?;
    Some(Tutorial::parse(text).expect("the built-in tutorials are tested"))
}

pub fn builtins() -> Vec<Tutorial> {
    BUILTIN.iter().filter_map(|(id, _)| builtin(id)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guide::{ANCHORS, FACTS};

    fn known(list: &[&str], name: &str) -> bool {
        list.iter()
            .any(|k| *k == name || (k.ends_with('.') && name.starts_with(k)))
    }

    #[test]
    fn the_builtin_tutorials_name_what_the_program_marks() {
        for (id, text) in BUILTIN {
            let tutorial = Tutorial::parse(text).unwrap();
            assert_eq!(tutorial.id, *id);
            for step in &tutorial.steps {
                for point in &step.point {
                    assert!(known(ANCHORS, &point.at), "{}: {}", step.id, point.at);
                }
                for fact in step
                    .done_when
                    .iter()
                    .chain(&step.continue_needs)
                    .chain(&step.lost_without)
                {
                    assert!(known(FACTS, fact), "{}: {fact}", step.id);
                }
                for status in &step.status {
                    assert!(["pdk_root", "klayout", "xschem"].contains(&status.as_str()));
                }
            }
        }
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
        let head = "schema='cdept-tutorial'\nversion=1\nid='x'\ntitle='X'\ngoal='g'\nminutes=1\n";
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
