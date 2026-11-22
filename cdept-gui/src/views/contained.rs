//! What the files of a view say about its cell, for the Inspector: the class, size
//! and pins of a LEF macro, the area and pins of a Liberty cell, the pins of a
//! sub-circuit, … (`libcdept::container`).
//!
//! The facts come through the index cache, so a large file is read once; they are
//! kept for the view shown until its file changes.

use std::sync::Arc;
use std::time::SystemTime;

use egui::{Id, Ui};
use libcdept::Entry;
use libcdept::container::{self, Cache};

/// `(name, value)` lines.
pub type Facts = Arc<Vec<(String, String)>>;

/// The facts of the view last asked for, with what tells it is still the one.
#[derive(Clone)]
struct Kept {
    file: std::path::PathBuf,
    member: Option<String>,
    modified: Option<SystemTime>,
    facts: Facts,
}

/// The facts of `entry`, a view of the cell `cell`. Empty for views whose file says
/// nothing (schematics, symbols).
pub fn facts(ui: &Ui, entry: &Entry, cell: &str) -> Facts {
    let Some(file) = &entry.main_file else {
        return Facts::default();
    };
    if entry
        .format
        .and_then(container::Reader::of_format)
        .is_none()
    {
        return Facts::default();
    }
    let modified = file.metadata().and_then(|m| m.modified()).ok();
    let id = Id::new("inspector-contained-facts");
    let kept: Option<Kept> = ui.data(|data| data.get_temp(id));
    if let Some(kept) = kept
        && kept.file == *file
        && kept.member == entry.member
        && kept.modified == modified
    {
        return kept.facts;
    }
    let facts: Facts = Arc::new(container::entry_facts(entry, cell, &Cache::user()));
    let kept = Kept {
        file: file.clone(),
        member: entry.member.clone(),
        modified,
        facts: facts.clone(),
    };
    ui.data_mut(|data| data.insert_temp(id, kept));
    facts
}
