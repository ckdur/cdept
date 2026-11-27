//! A tutorial being followed: which step it is at, and when it moves.

use std::path::{Path, PathBuf};

use super::circuit::Line;
use super::facts::Cache;
use super::tutorial::{Step, Tutorial};
use crate::pdkscan::ScanJob;

/// How many frames in a row a step's place must be gone before the guide goes back: a
/// dialog that turns a page is gone for a frame.
const LOST_FRAMES: u32 = 4;

/// How far the starting project of the tutorial is.
pub(crate) enum Start {
    None,
    /// The card's button was pressed.
    Asked,
    /// The PDK is being scanned.
    Scanning(Box<ScanJob>),
    Made(PathBuf),
    Failed(String),
}

pub(crate) struct Guide {
    pub tutorial: Tutorial,
    /// The step being done; the number of steps when the tutorial is finished.
    at: usize,
    /// The steps that were finished.
    done: Vec<bool>,
    /// The user went back to read a step again: it does not move on by itself while
    /// what finishes it still holds.
    reviewing: bool,
    /// Why the guide went back, until the next step is finished.
    pub notice: Option<String>,
    lost: u32,
    /// The project that was open when the tutorial began: "a project was made" means
    /// another one.
    pub baseline: Option<PathBuf>,
    /// The facts of the last frame, for the card.
    pub facts: Vec<String>,
    /// The card was closed.
    pub closed: bool,
    /// The checklists of the circuits the step asks about: the circuit and its lines
    /// (none while its schematic is not open).
    pub checks: Vec<(String, Option<Vec<Line>>)>,
    /// What was read from the disk for facts.
    pub cache: Cache,
    /// The starting project: asked for, being made, or made.
    pub start: Start,
    /// A picture shown large.
    pub picture: Option<String>,
}

impl Guide {
    pub fn new(tutorial: Tutorial, baseline: Option<PathBuf>) -> Guide {
        let steps = tutorial.steps.len();
        Guide {
            tutorial,
            at: 0,
            done: vec![false; steps],
            reviewing: false,
            notice: None,
            lost: 0,
            baseline,
            facts: Vec::new(),
            closed: false,
            checks: Vec::new(),
            cache: Cache::default(),
            start: Start::None,
            picture: None,
        }
    }

    pub fn started(&self) -> Option<&Path> {
        match &self.start {
            Start::Made(dir) => Some(dir),
            _ => None,
        }
    }

    /// The circuits the current step asks about.
    pub fn asked_circuits(&self) -> Vec<String> {
        let mut ids: Vec<String> = Vec::new();
        for fact in self.step().into_iter().flat_map(Step::facts) {
            if let Some(id) = fact.strip_prefix("circuit:")
                && !ids.iter().any(|known| known == id)
            {
                ids.push(id.to_string());
            }
        }
        ids
    }

    pub fn at(&self) -> usize {
        self.at
    }

    pub fn step(&self) -> Option<&Step> {
        self.tutorial.steps.get(self.at)
    }

    pub fn finished(&self) -> bool {
        self.at == self.tutorial.steps.len()
    }

    pub fn is_done(&self, n: usize) -> bool {
        self.done[n]
    }

    pub fn holds(&self, fact: &str) -> bool {
        self.facts.iter().any(|f| f == fact)
    }

    pub fn can_continue(&self) -> bool {
        self.step().is_some_and(|step| {
            (step.continue_button || self.reviewing)
                && step.continue_needs.iter().all(|f| self.holds(f))
        })
    }

    pub fn shows_continue(&self) -> bool {
        self.step()
            .is_some_and(|step| step.continue_button || self.reviewing)
    }

    /// The step is finished: on to the next.
    pub fn advance(&mut self) {
        if self.finished() {
            return;
        }
        self.done[self.at] = true;
        self.at += 1;
        self.reviewing = false;
        self.notice = None;
        self.lost = 0;
    }

    /// Go to a step by hand (the list, Back).
    pub fn jump(&mut self, n: usize) {
        if n >= self.tutorial.steps.len() || n == self.at {
            return;
        }
        self.reviewing = self.done[n];
        self.at = n;
        self.notice = None;
        self.lost = 0;
    }

    /// Take the facts of a frame and move as they say. Returns whether it moved.
    pub fn update(&mut self, facts: Vec<String>) -> bool {
        self.facts = facts;
        let Some(step) = self.step() else {
            return false;
        };
        // Making the starting project finishes the step that offers it.
        let made = step.start_button && self.holds("project.started");
        let done = made || step.done_when.iter().any(|f| self.holds(f));
        if done && !self.reviewing {
            self.advance();
            return true;
        }
        if !done {
            // From here on, doing it again moves on again.
            self.reviewing &= step.done_when.is_empty();
        }
        let step = &self.tutorial.steps[self.at];
        if done || step.lost_without.is_empty() || step.lost_without.iter().any(|f| self.holds(f)) {
            self.lost = 0;
            return false;
        }
        self.lost += 1;
        if self.lost < LOST_FRAMES {
            return false;
        }
        let says = step.lost_says.clone();
        let back = step
            .back_to
            .as_deref()
            .and_then(|id| self.tutorial.index_of(id))
            .expect("checked when the file was read");
        for done in &mut self.done[back..] {
            *done = false;
        }
        self.at = back;
        self.reviewing = false;
        self.lost = 0;
        self.notice = (!says.is_empty()).then_some(says);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guide() -> Guide {
        let text = "schema='cdept-tutorial'\nversion=1\nid='x'\ntitle='X'\nseries='setup'\norder=1\ngoal='g'\nminutes=1\n\
            [[step]]\nid='open'\ntitle='Open'\ntext='t'\ndone_when=['wizard.open']\n\
            [[step]]\nid='type'\ntitle='Type'\ntext='t'\ncontinue_button=true\n\
            continue_needs=['wizard.folder_ok']\nlost_without=['wizard.open']\nback_to='open'\n\
            lost_says='closed'\n";
        Guide::new(Tutorial::parse(text).unwrap(), None)
    }

    fn facts(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn a_step_is_finished_by_what_the_program_shows() {
        let mut g = guide();
        assert!(!g.update(facts(&[])));
        assert!(g.update(facts(&["wizard.open"])));
        assert_eq!(g.at(), 1);
        assert!(g.is_done(0));
        g.update(facts(&["wizard.open"]));
        assert!(!g.can_continue());
        g.update(facts(&["wizard.open", "wizard.folder_ok"]));
        assert!(g.can_continue());
        g.advance();
        assert!(g.finished());
    }

    #[test]
    fn leaving_the_place_of_a_step_goes_back() {
        let mut g = guide();
        g.update(facts(&["wizard.open"]));
        for _ in 1..LOST_FRAMES {
            assert!(!g.update(facts(&[])));
        }
        assert!(g.update(facts(&[])));
        assert_eq!(g.at(), 0);
        assert!(!g.is_done(0));
        assert_eq!(g.notice.as_deref(), Some("closed"));
    }

    #[test]
    fn a_step_read_again_stays_until_it_is_done_again() {
        let mut g = guide();
        g.update(facts(&["wizard.open"]));
        g.jump(0);
        assert!(!g.update(facts(&["wizard.open"])));
        assert_eq!(g.at(), 0);
        assert!(g.can_continue(), "a finished step can be left again");
        assert!(!g.update(facts(&[])));
        assert!(g.update(facts(&["wizard.open"])));
        assert_eq!(g.at(), 1);
    }
}
