//! A layout view that is one cell of a file holding several (a standard-cell GDS):
//! the tab shows that cell, and only shows it.
//!
//! The file is opened as it is; once the helper has it, it is told to show the cell
//! (`show_cell`). Such a view never opens for editing: the file belongs to every cell
//! in it (see `libcdept::Entry::member`).

use libcdept::Project;

use super::LayoutView;
use crate::dock::DocRef;

/// The cell inside its file that the view `doc` shows, when the file holds several.
pub fn member_of(project: &Project, doc: &DocRef) -> Option<String> {
    project
        .library(&doc.library)?
        .cell(&doc.cell)?
        .entry(&doc.view)?
        .member
        .clone()
}

impl LayoutView {
    /// Show this cell of the file instead of its first top cell.
    pub fn set_member(&mut self, member: Option<String>) {
        self.member = member;
    }

    /// The cell the helper shows now (the last of its path from the top).
    pub fn shown_cell(&self) -> Option<&str> {
        self.cells.as_ref()?.path.last().map(String::as_str)
    }

    /// Tell the helper to show the view's cell. Called when the layout is first seen
    /// (also after the helper was started again).
    pub(super) fn show_member(&mut self) {
        if let Some(member) = self.member.clone() {
            self.show_cell(&[member]);
        }
    }
}
