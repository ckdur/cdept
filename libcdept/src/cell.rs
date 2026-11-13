//! A [`Cell`] is a design unit inside a library. It owns a number of views
//! ([`Entry`]), e.g. schematic, symbol, layout and testbenches.

use std::path::PathBuf;

use crate::Entry;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cell {
    pub name: String,
    /// The cell folder. `None` in a flat library, where a cell is only a set of files
    /// sharing a name.
    pub path: Option<PathBuf>,
    /// Views, sorted by name.
    pub entries: Vec<Entry>,
}

impl Cell {
    pub fn entry(&self, name: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.name == name)
    }
}
