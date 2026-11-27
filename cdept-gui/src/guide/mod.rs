//! The tutorial guide: a step list over the running program that points at what to
//! use next and ticks a step off when the program shows it was done
//! (`docs/tutorials/writing-tutorials.md`).
//!
//! The rest of the GUI tells the guide two things while it draws:
//!
//! - [`anchor`]: where a widget is, under a stable name, so the guide can point at it;
//! - [`fact`]: that something is true right now ("the wizard is open").
//!
//! Both are no-ops while no tutorial runs. A tutorial file names only anchors and facts
//! listed in [`ANCHORS`] and [`FACTS`] (a test checks it).

mod card;
mod circuit;
mod facts;
mod run;
mod start;
mod tutorial;

use std::collections::HashMap;

use egui::{Context, Id, Rect, Response, Ui, ViewportId};

pub(crate) use card::{highlight, pointing, show as show_card};
pub(crate) use circuit::{Instance, check, passes};
pub(crate) use facts::{Open, World, view_file};
pub(crate) use run::{Guide, Start};
pub(crate) use start::{base_dir, make as make_start};
pub use tutorial::{SERIES, Tutorial, builtin, builtins};

/// The names widgets are marked with. A name ending in `.` or `:` is a family: the
/// replay of a tutorial is what finds a wrong name in one.
#[cfg(test)]
pub(crate) const ANCHORS: &[&str] = &[
    "menu.",
    "wizard.",
    "technology.page.",
    "libraries.",
    "manager.",
    "tree:",
    "cell.",
    "dialog.",
    "toolbar.",
    "xschem.",
    "schematic.",
    "sim.",
    "wave.",
    "layout.",
];

/// The facts a tutorial may ask about.
#[cfg(test)]
pub(crate) const FACTS: &[&str] = &[
    // Reported by widgets.
    "wizard.open",
    "wizard.page.project",
    "wizard.page.technology",
    "wizard.folder_ok",
    "wizard.pdk_chosen",
    "wizard.scanned",
    "technology.tab",
    "technology.page.",
    "dialog.",
    "sim.",
    "wave.",
    "layout.",
    "manager.",
    // Worked out by the guide (facts.rs).
    "project.created",
    "project.started",
    "library.pdk_selected",
    "library:",
    "view:",
    "selected:",
    "open:",
    "saved:",
    "pins:",
    "setup:",
    "file:",
    "sim.ran:",
    "sim.passed:",
    // The tutorial's own circuits (`[[circuit]]`).
    "circuit:",
];

/// What the widgets said, kept in egui's memory so that any of them can reach it.
#[derive(Debug, Clone, Default)]
struct Marks {
    /// A tutorial is running.
    active: bool,
    /// Counts the frames of the application; what is older than the last is gone.
    frame: u64,
    anchors: HashMap<String, (ViewportId, Rect, u64)>,
    facts: HashMap<String, u64>,
}

fn id() -> Id {
    Id::new("cdept-guide-marks")
}

fn active(ctx: &Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(id().with("active")))
        .unwrap_or(false)
}

fn with_marks<T>(ctx: &Context, f: impl FnOnce(&mut Marks) -> T) -> T {
    ctx.data_mut(|d| f(d.get_temp_mut_or_default::<Marks>(id())))
}

/// Say where the widget called `name` is. Nothing when it is scrolled out of sight.
pub(crate) fn anchor(ui: &Ui, name: &str, response: &Response) {
    anchor_rect(ui, name, response.rect);
}

/// [`anchor`] for a part of the window that is not one widget.
pub(crate) fn anchor_rect(ui: &Ui, name: &str, rect: Rect) {
    if !active(ui.ctx()) {
        return;
    }
    let rect = rect.intersect(ui.clip_rect());
    if !rect.is_positive() {
        return;
    }
    let viewport = ui.ctx().viewport_id();
    with_marks(ui.ctx(), |marks| {
        let frame = marks.frame;
        marks
            .anchors
            .insert(name.to_string(), (viewport, rect, frame));
    });
}

/// Say that `name` is true in this frame.
pub(crate) fn fact(ctx: &Context, name: &str) {
    if !active(ctx) {
        return;
    }
    with_marks(ctx, |marks| {
        let frame = marks.frame;
        marks.facts.insert(name.to_string(), frame);
    });
}

/// Where `name` was drawn in this frame, and in which window.
pub(crate) fn find_anchor(ctx: &Context, name: &str) -> Option<(ViewportId, Rect)> {
    with_marks(ctx, |marks| {
        let (viewport, rect, frame) = marks.anchors.get(name)?;
        (*frame == marks.frame).then_some((*viewport, *rect))
    })
}

/// Whether a widget said `name` in the frame that was drawn last. To be asked before
/// [`next_frame`].
pub(crate) fn has_fact(ctx: &Context, name: &str) -> bool {
    with_marks(ctx, |marks| {
        marks.facts.get(name).is_some_and(|f| *f == marks.frame)
    })
}

/// What the current step asks about and points at, for views that work a fact out only
/// when it is asked for. Empty while no tutorial runs.
pub(crate) fn wanted(ctx: &Context) -> Vec<String> {
    ctx.data(|d| d.get_temp::<Vec<String>>(id().with("wanted")))
        .unwrap_or_default()
}

pub(crate) fn set_wanted(ctx: &Context, names: Vec<String>) {
    ctx.data_mut(|d| d.insert_temp(id().with("wanted"), names));
}

/// Start a frame: what was marked before is old from now on.
pub(crate) fn next_frame(ctx: &Context, running: bool) {
    ctx.data_mut(|d| d.insert_temp(id().with("active"), running));
    with_marks(ctx, |marks| {
        marks.active = running;
        marks.frame += 1;
    });
}
