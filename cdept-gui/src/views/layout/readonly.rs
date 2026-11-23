//! A LEF abstract in a layout tab: read-only (docs/KLAYOUT.md §9).
//!
//! The tab looks at the file KLayout's LEF reader made of the LEF: navigation, zoom, the
//! layers and cells panels, rulers and the selection with its properties work; nothing
//! that changes the layout does. Menus and buttons are disabled by `editable`; what
//! scripts and keys could still start is refused here with a notice.

use std::sync::atomic::{AtomicU64, Ordering};

use libcdept::klayout::lef;
use libcdept::klayout::protocol::NoticeLevel;

use super::LayoutView;

/// The modes that change nothing (the others draw or move).
pub(super) fn mode_is_safe(name: &str) -> bool {
    matches!(name, "select" | "ruler")
}

/// Ids of calls that were refused, apart from the helper's (which count up from 1).
static REFUSED: AtomicU64 = AtomicU64::new(u64::MAX / 2);

impl LayoutView {
    /// Whether the tab shows a LEF abstract.
    pub fn is_abstract(&self) -> bool {
        lef::is_lef(&self.file)
    }

    /// What the tab says about why it cannot be edited, for the status line.
    pub(super) fn read_only_note(&self) -> &'static str {
        if self.is_abstract() {
            lef::READ_ONLY_NOTICE
        } else {
            "read-only"
        }
    }

    /// Say once that the tab is read-only; called as it opens.
    pub(super) fn announce_abstract(&mut self) {
        if self.is_abstract() {
            self.notify(NoticeLevel::Info, lef::READ_ONLY_NOTICE.into());
        }
    }

    /// Refuse a change of a LEF abstract: a notice, and for a call whose outcome is
    /// waited for (`save`, `place`), an id whose outcome is the error. `None`: not a
    /// LEF abstract, go on.
    pub(super) fn refuse_change(&mut self) -> Option<u64> {
        if !self.is_abstract() {
            return None;
        }
        let text = lef::READ_ONLY_NOTICE.to_string();
        self.notify(NoticeLevel::Warning, text.clone());
        let id = REFUSED.fetch_add(1, Ordering::Relaxed);
        self.done.insert(id, Err(text));
        Some(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_looking_modes_are_safe() {
        assert!(mode_is_safe("select"));
        assert!(mode_is_safe("ruler"));
        for mode in [
            "box", "polygon", "path", "text", "instance", "move", "partial",
        ] {
            assert!(!mode_is_safe(mode), "{mode}");
        }
    }
}
