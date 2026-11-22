//! Open documents and the files on disk.
//!
//! A tab shows a file (a layout also the files of the cells it places). When such a
//! file changes on disk and it was not the tab's own save, the tab is out of date:
//!
//! - without changes of its own it opens the file again (unless the setting
//!   "Reload Unchanged Documents" is off);
//! - with changes of its own it shows a notice, and the user chooses: Reload (the
//!   tab's changes are lost) or Keep Mine (the next save writes over the file).
//!
//! Telling a tab's own save from a change behind its back:
//! - a layout tab reports the files each save wrote (`LayoutView::disk_written`);
//! - xschem saves by itself; the tab going from changed to unchanged is its save.
//!
//! What CDePT last knew of each file is its modification time and size.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::TARGET;
use crate::state::AppCtx;
use crate::views::{Document, Views};

/// Modification time and size; `None`: no such file.
type Stamp = Option<(SystemTime, u64)>;

fn stamp(path: &Path) -> Stamp {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

/// The files a document shows.
fn shown(doc: &Document) -> Vec<PathBuf> {
    match doc {
        Document::Schematic(view) => vec![view.file().to_path_buf()],
        Document::Layout(view) => view.disk_files(),
    }
}

/// Open the document's file again as it is on disk.
pub(super) fn reload(doc: &mut Document) {
    match doc {
        Document::Schematic(view) => {
            // Only when xschem shows that very file (not a cell it descended into).
            let file = view.file().display().to_string();
            view.tcl(&format!(
                "if {{[xschem get schname] eq {{{file}}}}} {{xschem reload}}"
            ));
        }
        Document::Layout(view) => view.disk_reload(),
    }
}

/// A document is out of date: files it shows changed on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Stale {
    pub files: Vec<PathBuf>,
    /// The document's own file is gone.
    pub gone: bool,
}

struct DocState {
    /// What CDePT last knew of the files the document shows.
    known: HashMap<PathBuf, Stamp>,
    was_modified: bool,
    /// When the document was last seen without changes of its own.
    clean_at: SystemTime,
    /// Changed files not looked at yet (the document was saving).
    pending: BTreeSet<PathBuf>,
    stale: Option<Stale>,
}

/// The open documents, by the file each was opened for.
#[derive(Default)]
pub(super) struct Documents {
    docs: HashMap<PathBuf, DocState>,
    /// Files the watcher reported since the last frame.
    suspects: Vec<PathBuf>,
}

impl Documents {
    /// Files changed on disk (from the watcher).
    pub fn changed(&mut self, files: Vec<PathBuf>) {
        self.suspects.extend(files);
    }

    pub fn stale(&self, file: &Path) -> Option<&Stale> {
        self.docs.get(file)?.stale.as_ref()
    }

    /// The user answered the notice of the document of `file`.
    pub fn answered(&mut self, file: &Path) {
        if let Some(state) = self.docs.get_mut(file) {
            state.stale = None;
            state.was_modified = false;
        }
    }
}

/// Once per frame, before the layout tabs are told about each other's saves.
pub(super) fn check(app: &mut AppCtx, views: &mut Views) {
    let auto = app.settings.auto_reload;
    let documents = &mut app.background.docs;
    let suspects = std::mem::take(&mut documents.suspects);
    let ids: Vec<_> = views.documents().map(|(id, _)| id).collect();
    let mut open = BTreeSet::new();
    for id in ids {
        let Some(doc) = views.document_mut(id) else {
            continue;
        };
        let file = doc.file().to_path_buf();
        open.insert(file.clone());
        let modified = doc.is_modified();
        let files = shown(doc);
        let state = documents
            .docs
            .entry(file.clone())
            .or_insert_with(|| DocState {
                known: HashMap::new(),
                was_modified: modified,
                clean_at: SystemTime::now(),
                pending: BTreeSet::new(),
                stale: None,
            });
        // Files it shows since this frame (the file itself, a cell just placed).
        for f in &files {
            state.known.entry(f.clone()).or_insert_with(|| stamp(f));
        }
        // Its own saves.
        match doc {
            Document::Layout(view) => {
                for f in view.disk_written() {
                    state.known.insert(f.clone(), stamp(f));
                }
            }
            Document::Schematic(_) => {
                if state.was_modified && !modified {
                    state.known.insert(file.clone(), stamp(&file));
                    // The save made the file what the tab shows.
                    state.stale = None;
                }
            }
        }
        if !modified {
            state.clean_at = SystemTime::now();
        }
        state.was_modified = modified;

        state
            .pending
            .extend(suspects.iter().filter(|f| files.contains(f)).cloned());
        let busy = match doc {
            Document::Layout(view) => view.disk_busy(),
            Document::Schematic(_) => false,
        };
        if state.pending.is_empty() || busy || !doc.is_running() {
            continue;
        }
        let mut changed = Vec::new();
        let mut newest: Option<SystemTime> = None;
        for f in std::mem::take(&mut state.pending) {
            let now = stamp(&f);
            if state.known.get(&f) != Some(&now) {
                if let Some((time, _)) = now {
                    newest = Some(newest.map_or(time, |n| n.max(time)));
                }
                state.known.insert(f.clone(), now);
                changed.push(f);
            }
        }
        if changed.is_empty() {
            continue;
        }
        let gone = stamp(&file).is_none();
        // xschem marks a schematic as changed when it notices the file changed on
        // disk: a tab that was unchanged when the file was written has nothing of its
        // own to lose.
        let own_changes = modified
            && !(matches!(doc, Document::Schematic(_))
                && newest.is_some_and(|time| state.clean_at >= time));
        let names: Vec<String> = changed.iter().map(|f| name(f)).collect();
        if !own_changes && auto && !gone {
            tracing::info!(
                target: TARGET,
                "{} changed on disk: {} opened again",
                names.join(", "),
                name(&file)
            );
            reload(doc);
            state.stale = None;
            state.was_modified = false;
        } else {
            tracing::warn!(
                target: TARGET,
                "{} {} on disk; {} still shows what it had",
                names.join(", "),
                if gone { "was deleted" } else { "changed" },
                name(&file)
            );
            let stale = state.stale.get_or_insert(Stale {
                files: Vec::new(),
                gone,
            });
            stale.gone = gone;
            for f in changed {
                if !stale.files.contains(&f) {
                    stale.files.push(f);
                }
            }
        }
    }
    documents.docs.retain(|file, _| open.contains(file));
}

pub(super) fn name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}
