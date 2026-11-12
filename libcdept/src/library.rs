//! A [`Library`] is a named folder of cells, bound to a technology.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::Cell;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Library {
    pub name: String,
    pub path: PathBuf,
    /// Reference libraries (e.g. PDK primitives) cannot be modified.
    pub read_only: bool,
    pub cells: Vec<Cell>,
}
