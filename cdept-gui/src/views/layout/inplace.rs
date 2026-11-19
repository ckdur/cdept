//! Editing an external cell in place (docs/KLAYOUT.md §3.4): descending into an
//! instance, and keeping the layout tabs in step with each other.
//!
//! A tab edits the cells of other views in place; its next save writes them back. While
//! one tab has unsaved changes in a file, every other tab gets that file as *locked*
//! (the helper refuses changes to it). After a save, the tabs showing a written file, or
//! placing it, open it again unless they have unsaved changes of their own.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use libcdept::klayout::protocol::{InPlace, NoticeLevel, method};
use libcdept::layout::refs;
use serde_json::json;

use super::LayoutView;

impl LayoutView {
    /// Edit the cell of the selected instance in place.
    pub fn descend(&mut self) {
        self.quiet(method::DESCEND, json!({}));
    }

    /// Back up from a descent, one level or (`all`) to the file's own cells.
    pub fn ascend(&mut self, all: bool) {
        self.quiet(method::ASCEND, json!({ "all": all }));
    }

    /// The external cell edited in place, as last reported.
    pub fn in_place(&self) -> Option<&InPlace> {
        self.in_place.as_ref()
    }

    /// Menu items: Edit in Place, Return.
    pub(super) fn in_place_menu(&mut self, ui: &mut egui::Ui, editable: bool) {
        ui.separator();
        let selected = self.selection.items.iter().any(|i| i.kind == "instance");
        if ui
            .add_enabled(
                editable && selected,
                egui::Button::new("Edit in Place").shortcut_text("Shift+X"),
            )
            .on_hover_text(
                "Edit the cell of the selected instance here; saving writes its view too",
            )
            .clicked()
        {
            self.descend();
            ui.close();
        }
        let inside = self.cells.as_ref().is_some_and(|c| c.path.len() > 1);
        if ui
            .add_enabled(inside, egui::Button::new("Return").shortcut_text("Shift+B"))
            .on_hover_text("Back up one level from Edit in Place")
            .clicked()
        {
            self.ascend(false);
            ui.close();
        }
    }

    /// "editing in place: library/cell/view" on the status line.
    pub(super) fn in_place_status(&self, ui: &mut egui::Ui) {
        let Some(context) = &self.in_place else {
            return;
        };
        ui.separator();
        if context.writable {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                format!("editing in place: {}", context.label),
            );
        } else {
            ui.weak(format!("in {} (not editable)", context.label))
                .on_hover_text(&context.reason);
        }
    }

    /// Files this tab has unsaved changes in: its own when modified, and the external
    /// files changed in place.
    fn files_in_use(&self) -> Vec<PathBuf> {
        if !self.dirty {
            return Vec::new();
        }
        let mut files = vec![self.file.clone()];
        files.extend(self.edited.iter().cloned());
        files
    }

    /// Whether the layout shows `file`: it is the file, or one the file places.
    fn shows(&self, file: &Path) -> bool {
        self.file == file || self.refs.files.contains_key(file)
    }

    /// Tell the helper which of its files other tabs are changing.
    fn set_locks(&mut self, mut files: Vec<PathBuf>) {
        files.retain(|f| self.shows(f));
        files.sort();
        files.dedup();
        let epoch = self.session.epoch();
        if self.locks_sent.as_ref() == Some(&(epoch, files.clone())) || !self.is_running() {
            return;
        }
        self.quiet(method::LOCKS, json!({ "files": files }));
        self.locks_sent = Some((epoch, files));
    }

    /// Open the file again (after another tab wrote it or one it places), with the
    /// references read anew.
    fn reload(&mut self) {
        self.refs = refs::resolve(&self.project, &self.file);
        self.session.set_refs(self.refs.clone());
        self.session.restart();
        self.locks_sent = None;
    }
}

/// Keep the layout tabs in step: locks for files changed elsewhere, and reloads after
/// a save wrote files that other tabs show.
pub fn coordinate(mut views: Vec<&mut LayoutView>) {
    let written: Vec<(usize, PathBuf)> = views
        .iter_mut()
        .enumerate()
        .flat_map(|(i, v)| {
            std::mem::take(&mut v.written)
                .into_iter()
                .map(move |f| (i, f))
        })
        .collect();
    let mut reloaded = BTreeSet::new();
    for (writer, file) in &written {
        for (i, view) in views.iter_mut().enumerate() {
            if i == *writer || !view.shows(file) {
                continue;
            }
            if view.dirty {
                let text = format!(
                    "{} was saved from another tab; this tab has unsaved changes and still \
                     shows the old version",
                    file.display()
                );
                view.notify(NoticeLevel::Warning, text);
            } else if reloaded.insert(i) {
                view.reload();
            }
        }
    }

    let in_use: Vec<Vec<PathBuf>> = views.iter().map(|v| v.files_in_use()).collect();
    for (i, view) in views.iter_mut().enumerate() {
        let others: BTreeSet<PathBuf> = in_use
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .flat_map(|(_, files)| files.iter().cloned())
            .collect();
        view.set_locks(others.into_iter().collect());
    }
}
