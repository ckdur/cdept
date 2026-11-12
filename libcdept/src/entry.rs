//! An [`Entry`] is one view of a cell (Virtuoso: "cellview").
//!
//! An entry is a folder holding a file or a group of files with a single function, such as
//! a layout, a schematic, a netlist or a simulation setup.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The kind of view an [`Entry`] holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum EntryType {
    #[default]
    Undefined,
    // Layout
    LayoutMag,
    LayoutGds,
    // Schematic
    SchematicQucs,
    SchematicXschem,
    SchematicXcircuit,
    // Netlist
    NetlistSpice,
    NetlistCdl,
    // Simulation
    Simulation,
}

/// One view of a cell.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// View name, e.g. `schematic`, `layout`, `tb_ac`.
    pub name: String,
    pub typ: EntryType,
    /// Folder of the view, relative to the cell folder.
    pub path: PathBuf,
}
