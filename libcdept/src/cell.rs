//! A [`Cell`] is a design unit inside a library. It owns a number of views
//! ([`Entry`](crate::Entry)), e.g. schematic, symbol, layout and testbenches.

use serde::{Deserialize, Serialize};

use crate::Entry;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cell {
    pub name: String,
    pub entries: Vec<Entry>,
}
