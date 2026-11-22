//! What a layout tab has to do with the files on disk: which files it shows, which
//! ones it wrote itself, and opening them again after they changed behind its back
//! (see `crate::background`).

use std::path::PathBuf;

use libcdept::layout::refs;

use super::{Closing, LayoutView, Pending};

impl LayoutView {
    /// The files the tab shows: its own and the ones it places.
    pub(crate) fn disk_files(&self) -> Vec<PathBuf> {
        let mut files = vec![self.file.clone()];
        files.extend(self.refs.files.keys().filter(|f| **f != self.file).cloned());
        files
    }

    /// The files the last save of this tab wrote, until the tabs have been told
    /// about them (`inplace::coordinate`).
    pub(crate) fn disk_written(&self) -> &[PathBuf] {
        &self.written
    }

    /// A save is under way: the files change, and it is the tab's own doing.
    pub(crate) fn disk_busy(&self) -> bool {
        self.pending.values().any(|p| matches!(p, Pending::Save))
            || matches!(self.closing, Some(Closing::Saving(_)))
    }

    /// Open the file again, as it is on disk now, with the references read anew.
    /// Changes that were not saved are gone.
    pub(crate) fn disk_reload(&mut self) {
        self.refs = refs::resolve(&self.project, &self.file);
        self.session.set_refs(self.refs.clone());
        self.session.restart();
        self.locks_sent = None;
    }
}
