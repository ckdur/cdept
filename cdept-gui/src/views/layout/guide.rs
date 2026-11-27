//! What a tutorial may ask about this tab (docs/tutorials/writing-tutorials.md): the
//! facts of the layout as it is now. Where parts and shapes are is asked from the
//! helper (`cdept-klayout/helper/cdept_klayout/tutorial.py`); the results of the net
//! check, DRC and LVS count only while the layout is the one they were made from.

use std::time::{Duration, Instant};

use serde_json::json;

use super::LayoutView;

/// The facts the helper answers.
const PROBED: [&str; 3] = ["layout.at:", "layout.covers:", "layout.pin:"];
/// How often the helper is asked again although nothing is known to have changed.
const AGAIN: Duration = Duration::from_secs(2);

/// What was asked of the helper, and what it answered.
#[derive(Default)]
pub(super) struct Probe {
    /// A question on its way: the call, the queries and the layout asked about.
    asked: Option<(u64, Vec<String>, (u64, u64))>,
    /// The last answer: the queries that hold, and what they were about.
    holds: Vec<String>,
    about: Option<(Vec<String>, (u64, u64))>,
    at: Option<Instant>,
}

impl LayoutView {
    /// Tell the guide what holds. Called every frame the tab is drawn.
    pub(super) fn tell_guide(&mut self, ctx: &egui::Context) {
        let wanted = crate::guide::wanted(ctx);
        if wanted.is_empty() || !self.ready() {
            return;
        }
        crate::guide::fact(ctx, "layout.ready");
        if let Some(cells) = &self.cells
            && let Some(top) = cells.cells.iter().find(|c| c.name == cells.top)
        {
            for child in &top.children {
                // A PCell's variants are `via_stack`, `via_stack$1`, ...
                let name = child.split('$').next().unwrap_or(child);
                crate::guide::fact(ctx, &format!("layout.cell:{name}"));
            }
        }
        for fact in self.net_facts().into_iter().chain(self.check_facts()) {
            crate::guide::fact(ctx, &fact);
        }
        self.probe_for_guide(ctx, &wanted);
    }

    fn probe_for_guide(&mut self, ctx: &egui::Context, wanted: &[String]) {
        let mut queries: Vec<String> = wanted
            .iter()
            .filter(|f| PROBED.iter().any(|p| f.starts_with(p)))
            .cloned()
            .collect();
        queries.sort();
        queries.dedup();
        if queries.is_empty() {
            return;
        }
        let now = (self.epoch, self.revision);
        if let Some((id, asked, layout)) = self.guide.asked.take() {
            match self.take_reply(id) {
                None => self.guide.asked = Some((id, asked, layout)),
                Some(Ok(answer)) => {
                    let answers: Vec<bool> = serde_json::from_str(&answer).unwrap_or_default();
                    self.guide.holds = asked
                        .iter()
                        .zip(answers)
                        .filter(|(_, holds)| *holds)
                        .map(|(q, _)| q.clone())
                        .collect();
                    self.guide.about = Some((asked, layout));
                }
                // The next question may fare better (a helper starting again).
                Some(Err(_)) => {}
            }
        }
        // An answer counts while it is about this layout.
        let current = self
            .guide
            .about
            .as_ref()
            .is_some_and(|(_, layout)| *layout == now);
        if current {
            for query in self.guide.holds.iter().filter(|q| queries.contains(q)) {
                crate::guide::fact(ctx, query);
            }
        }
        let known = self
            .guide
            .about
            .as_ref()
            .is_some_and(|(asked, layout)| *asked == queries && *layout == now);
        let due = self.guide.at.is_none_or(|at| at.elapsed() >= AGAIN);
        if self.guide.asked.is_none() && (!known || due) {
            let id = self.raw_call("tutorial_probe", json!({ "queries": queries }));
            self.guide.asked = Some((id, queries, now));
            self.guide.at = Some(Instant::now());
        }
    }
}
