//! What a tutorial may ask about: what widgets report while they are drawn
//! ([`super::fact`]), and what follows from the project and its files, the open
//! documents and the stored simulation results.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use libcdept::Project;
use libcdept::sim::results::{Overall, Store};

/// How long what was read from the disk is taken as still so.
const KEPT: Duration = Duration::from_millis(600);

/// An open document, as far as the facts care.
pub(crate) struct Open {
    /// `library/cell/view`; empty for a loose file.
    pub name: String,
    /// The document in front in the main window.
    pub front: bool,
    pub modified: bool,
}

/// What the application knows.
pub(crate) struct World<'a> {
    pub project: Option<&'a Project>,
    pub selected: (Option<&'a str>, Option<&'a str>),
    pub documents: Vec<Open>,
    /// The project that was open when the tutorial began.
    pub baseline: Option<&'a Path>,
    /// The starting project the tutorial made.
    pub started: Option<&'a Path>,
}

#[derive(Default)]
pub(crate) struct Cache(HashMap<(PathBuf, String), (Instant, bool)>);

/// The file of the view `library/cell/view`.
pub(crate) fn view_file(project: &Project, name: &str) -> Option<PathBuf> {
    let (library, cell, view) = three(name)?;
    let entry = project.library(library)?.cell(cell)?.entry(view)?;
    entry
        .main_file
        .clone()
        .or_else(|| entry.files.first().cloned())
}

fn three(name: &str) -> Option<(&str, &str, &str)> {
    let mut parts = name.splitn(3, '/');
    Some((parts.next()?, parts.next()?, parts.next()?))
}

/// The pins a symbol file draws: the `name=` of its pin boxes.
fn symbol_pins(text: &str) -> Vec<String> {
    let mut pins = Vec::new();
    for record in text.split("\nB 5 ").skip(1) {
        let Some(open) = record.find('{') else {
            continue;
        };
        let Some(close) = record[open..].find('}') else {
            continue;
        };
        let attributes = libcdept::link::schematic::attributes(&record[open + 1..open + close]);
        if let Some(name) = attributes.get("name") {
            pins.push(name.clone());
        }
    }
    pins
}

fn without_space(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

/// `<library/cell/view>:<dotted key>~<text>`: the view's saved `setup.toml` (or
/// `suite.toml`) has the key, and its value, written as TOML, contains the text. Spaces
/// are ignored.
fn setup_says(project: &Project, rest: &str) -> bool {
    let Some((view, query)) = rest.split_once(':') else {
        return false;
    };
    let Some((key, wanted)) = query.split_once('~') else {
        return false;
    };
    let Some(file) = view_file(project, view) else {
        return false;
    };
    let Ok(text) = std::fs::read_to_string(file) else {
        return false;
    };
    let Ok(mut value) = text.parse::<toml::Table>().map(toml::Value::Table) else {
        return false;
    };
    for part in key.split('.') {
        match value.get(part) {
            Some(inner) => value = inner.clone(),
            None => return false,
        }
    }
    without_space(&value.to_string()).contains(&without_space(wanted))
}

/// The latest run of every point of a testbench: there is one, and all pass when asked.
fn simulated(project: &Project, name: &str, passed: bool) -> bool {
    let Some((library, cell, view)) = three(name) else {
        return false;
    };
    if !Store::path(&project.root).is_file() {
        return false;
    }
    let Ok(runs) = Store::open(&project.root).and_then(|s| s.latest(library, cell, view)) else {
        return false;
    };
    !runs.is_empty()
        && runs.iter().all(|r| match r.overall() {
            Overall::Pass => true,
            Overall::Fail => !passed,
            Overall::Error => false,
        })
}

fn from_disk(project: &Project, kind: &str, rest: &str) -> Option<bool> {
    Some(match kind {
        "pins" => {
            let (name, wanted) = rest.split_once('=')?;
            let file = view_file(project, &format!("{name}/symbol"))?;
            let pins = symbol_pins(&std::fs::read_to_string(file).unwrap_or_default());
            wanted.split(',').all(|w| pins.iter().any(|p| p == w))
        }
        "setup" => setup_says(project, rest),
        "file" => project.root.join(rest).is_file(),
        "sim.ran" => simulated(project, rest, false),
        "sim.passed" => simulated(project, rest, true),
        _ => return None,
    })
}

impl Cache {
    /// Whether `fact` follows from the world; `None` for a fact widgets report.
    pub fn derived(&mut self, world: &World<'_>, fact: &str) -> Option<bool> {
        let project = world.project;
        let (kind, rest) = fact.split_once(':').unwrap_or((fact, ""));
        let open = || world.documents.iter().filter(|d| d.name == rest);
        Some(match kind {
            // A project with a technology, other than the one the tutorial began with.
            "project.created" => {
                project.is_some_and(|p| p.tech_in_use && Some(p.root.as_path()) != world.baseline)
            }
            "project.started" => project.is_some_and(|p| Some(p.root.as_path()) == world.started),
            // A cell of a library of the PDK is selected.
            "library.pdk_selected" => project.is_some_and(|p| {
                let (library, cell) = world.selected;
                cell.is_some()
                    && library.is_some_and(|name| {
                        let tech = libcdept::tech::file::tech_file(&p.root);
                        p.libraries
                            .iter()
                            .any(|l| l.name == name && l.defined_in == tech)
                    })
            }),
            // A library of the project's own.
            "library" => project.is_some_and(|p| p.library(rest).is_some_and(|l| !l.read_only)),
            "view" => project.is_some_and(|p| {
                three(rest).is_some_and(|(l, c, v)| {
                    p.library(l)
                        .and_then(|l| l.cell(c))
                        .is_some_and(|c| c.entry(v).is_some())
                })
            }),
            // `selected:library` or `selected:library/cell`.
            "selected" => {
                let (library, cell) = world.selected;
                match rest.split_once('/') {
                    Some((l, c)) => library == Some(l) && cell == Some(c),
                    None => library == Some(rest),
                }
            }
            "open" => open().any(|d| d.front),
            "saved" => open().next().is_some_and(|d| !d.modified),
            "pins" | "setup" | "file" | "sim.ran" | "sim.passed" => {
                let Some(project) = project else {
                    return Some(false);
                };
                let key = (project.root.clone(), fact.to_string());
                if let Some((at, holds)) = self.0.get(&key)
                    && at.elapsed() < KEPT
                {
                    return Some(*holds);
                }
                let holds = from_disk(project, kind, rest)?;
                self.0.insert(key, (Instant::now(), holds));
                holds
            }
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pins_of_a_symbol_are_read() {
        let text = "v {xschem}\nB 5 -62.5 -2.5 -57.5 2.5 {name=A dir=in}\n\
                    B 5 57.5 -2.5 62.5 2.5 {name=Y dir=out }\nT {@name} 0 0 0 0 0.2 0.2 {}\n";
        assert_eq!(symbol_pins(text), ["A", "Y"]);
    }

    #[test]
    fn a_saved_setup_is_asked_about() {
        let dir = tempfile::tempdir().unwrap();
        let view = dir.path().join("libs/work/tb/tran");
        std::fs::create_dir_all(&view).unwrap();
        std::fs::write(
            view.join("setup.toml"),
            "corners = [\"tt\", \"ff\"]\n[variables]\nvdd = \"1.2\"\n\
             [[spec]]\nname = \"tpd\"\nmax = 3e-10\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join(libcdept::PROJECT_FILE),
            "name = \"p\"\n[libraries]\nwork = \"libs/work\"\n",
        )
        .unwrap();
        let project = Project::load(dir.path()).unwrap();
        let says = |query: &str| setup_says(&project, &format!("work/tb/tran:{query}"));
        assert!(says("corners~\"ff\""));
        assert!(says("variables.vdd~1.2"));
        assert!(says("spec~name=\"tpd\""));
        assert!(!says("corners~ss"));
        assert!(!says("temperatures~27"));
    }
}
