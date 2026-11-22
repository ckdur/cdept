//! Git badges for the library views: a letter at the right edge of a library, cell or
//! view row (`M` modified, `?` untracked, `A` added, `S` staged, `!` in conflict, `I`
//! ignored; nothing when the files are as committed or not in a repository).
//!
//! The status is read in the background (`crate::background`) and published to the
//! egui context every frame, so a view only needs its row: `response =
//! vcs::cell_badge(ui, response, cell)`.

use std::sync::Arc;

use egui::{Align2, Color32, Context, FontId, Id, Response, Ui};
use libcdept::vcs::{State, Status, Summary};
use libcdept::{Cell, Entry, Library};

fn key() -> Id {
    Id::new("cdept-vcs-status")
}

/// Make the status available to the views (called once per frame).
pub fn publish(egui: &Context, status: &Arc<Status>) {
    egui.data_mut(|data| data.insert_temp(key(), status.clone()));
}

/// The git status as last read; `None` before the first frame.
pub fn status(egui: &Context) -> Option<Arc<Status>> {
    egui.data(|data| data.get_temp::<Arc<Status>>(key()))
}

/// The colour of a state's badge.
pub fn color(ui: &Ui, state: State) -> Color32 {
    let dark = ui.visuals().dark_mode;
    match state {
        State::Conflicted => ui.visuals().error_fg_color,
        State::Modified => ui.visuals().warn_fg_color,
        State::Untracked | State::Added => {
            if dark {
                Color32::from_rgb(110, 200, 120)
            } else {
                Color32::from_rgb(30, 130, 50)
            }
        }
        State::Staged => {
            if dark {
                Color32::from_rgb(110, 170, 240)
            } else {
                Color32::from_rgb(30, 90, 190)
            }
        }
        State::Ignored | State::Clean => ui.visuals().weak_text_color(),
    }
}

/// Draw the badge of `summary` at the right end of the row of `response`, and say in
/// the row's tooltip what it stands for.
pub fn badge(ui: &Ui, response: Response, summary: Option<Summary>) -> Response {
    let Some(summary) = summary else {
        return response;
    };
    let state = summary.state();
    if state == State::Clean {
        return response;
    }
    if ui.is_rect_visible(response.rect) {
        let right = ui
            .clip_rect()
            .right()
            .min(ui.max_rect().right().max(response.rect.right()));
        let pos = egui::pos2(right - 4.0, response.rect.center().y);
        ui.painter().text(
            pos,
            Align2::RIGHT_CENTER,
            state.mark(),
            FontId::monospace(ui.text_style_height(&egui::TextStyle::Small)),
            color(ui, state),
        );
    }
    response.on_hover_text(format!("git: {}", summary.describe()))
}

pub fn library_badge(ui: &Ui, response: Response, library: &Library) -> Response {
    let summary = status(ui.ctx()).and_then(|s| s.library(library));
    badge(ui, response, summary)
}

pub fn cell_badge(ui: &Ui, response: Response, cell: &Cell) -> Response {
    let summary = status(ui.ctx()).and_then(|s| s.cell(cell));
    badge(ui, response, summary)
}

pub fn view_badge(ui: &Ui, response: Response, entry: &Entry) -> Response {
    let summary = status(ui.ctx()).and_then(|s| s.entry(entry));
    badge(ui, response, summary)
}

/// Whether the row of a collapsed cell can be left out because it is not on screen:
/// space is taken for it and `true` returned. For trees with thousands of cells.
///
/// `open` says whether the cell's node is unfolded (then it has to be drawn: its
/// height is not known).
pub fn skip_hidden_row(ui: &mut Ui, open: bool) -> bool {
    if open {
        return false;
    }
    let height = ui.spacing().interact_size.y;
    let top = ui.cursor().top();
    let clip = ui.clip_rect();
    if top + height < clip.top() - height || top > clip.bottom() + height {
        ui.allocate_space(egui::vec2(0.0, height));
        true
    } else {
        false
    }
}

/// Whether the tree node with this id salt (as given to `CollapsingHeader::id_salt`)
/// is unfolded.
pub fn node_open(ui: &Ui, id_salt: impl std::hash::Hash + std::fmt::Debug) -> bool {
    let id = ui.make_persistent_id(Id::new(id_salt));
    egui::collapsing_header::CollapsingState::load(ui.ctx(), id).is_some_and(|s| s.is_open())
}
