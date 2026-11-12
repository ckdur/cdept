use egui::Ui;

use super::{View, empty_state};
use crate::state::AppCtx;

/// A panel whose content arrives in a later phase.
pub struct Placeholder {
    title: &'static str,
    text: &'static str,
    phase: &'static str,
}

impl Placeholder {
    pub fn new(title: &'static str, text: &'static str, phase: &'static str) -> Self {
        Self { title, text, phase }
    }
}

impl View for Placeholder {
    fn title(&self, _ctx: &AppCtx) -> String {
        self.title.to_string()
    }

    fn ui(&mut self, ui: &mut Ui, _ctx: &mut AppCtx) {
        empty_state(ui, self.text, Some(&format!("(arrives in {})", self.phase)));
    }
}
