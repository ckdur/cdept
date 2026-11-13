//! An [`Entry`] is one view of a cell (Virtuoso: "cellview").
//!
//! A view has a single function: a schematic, a symbol, a layout, a netlist, a
//! simulation setup. On disk it is either a folder inside the cell folder or a single
//! file. The [`ViewKind`] says what the view is for; the [`Format`] of its main file says
//! which tool opens it.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

/// What a view is for, independent of the tool that edits it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ViewKind {
    Schematic,
    Symbol,
    Layout,
    Netlist,
    Testbench,
    Config,
    Hdl,
    Abstract,
    Liberty,
    Other,
}

impl ViewKind {
    pub const ALL: [ViewKind; 10] = [
        Self::Schematic,
        Self::Symbol,
        Self::Layout,
        Self::Netlist,
        Self::Testbench,
        Self::Config,
        Self::Hdl,
        Self::Abstract,
        Self::Liberty,
        Self::Other,
    ];

    /// Lower-case name, also the default name of a view of this kind.
    pub fn name(self) -> &'static str {
        match self {
            Self::Schematic => "schematic",
            Self::Symbol => "symbol",
            Self::Layout => "layout",
            Self::Netlist => "netlist",
            Self::Testbench => "testbench",
            Self::Config => "config",
            Self::Hdl => "hdl",
            Self::Abstract => "abstract",
            Self::Liberty => "liberty",
            Self::Other => "other",
        }
    }

    /// The kind a view folder name suggests, e.g. `schematic` or `tb_ac`.
    pub fn from_view_name(name: &str) -> Option<ViewKind> {
        let name = name.to_ascii_lowercase();
        if name.starts_with("tb_") || name.starts_with("testbench") {
            return Some(Self::Testbench);
        }
        Self::ALL
            .into_iter()
            .find(|k| *k != Self::Other && k.name() == name)
    }
}

impl std::fmt::Display for ViewKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// File format of a view's main file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    XschemSchematic,
    XschemSymbol,
    QucsSchematic,
    Gds,
    Oasis,
    Magic,
    Spice,
    Cdl,
    Verilog,
    SystemVerilog,
    Vhdl,
    VerilogA,
    Lef,
    Liberty,
}

/// File name suffixes (lower case, with the dot) and their format. Longer suffixes of
/// the same family come first so that `.gds.gz` wins over a bare `.gz`.
const SUFFIXES: &[(&str, Format)] = &[
    (".sch", Format::XschemSchematic),
    (".sym", Format::XschemSymbol),
    (".gds.gz", Format::Gds),
    (".gds", Format::Gds),
    (".gds2", Format::Gds),
    (".gdsii", Format::Gds),
    (".oas.gz", Format::Oasis),
    (".oas", Format::Oasis),
    (".oasis", Format::Oasis),
    (".mag", Format::Magic),
    (".spice", Format::Spice),
    (".spi", Format::Spice),
    (".sp", Format::Spice),
    (".cir", Format::Spice),
    (".ckt", Format::Spice),
    (".cdl", Format::Cdl),
    (".sv", Format::SystemVerilog),
    (".v", Format::Verilog),
    (".vhd", Format::Vhdl),
    (".vhdl", Format::Vhdl),
    (".va", Format::VerilogA),
    (".lef", Format::Lef),
    (".lib", Format::Liberty),
];

impl Format {
    /// Split a file name into its stem and format, by suffix only (no file access).
    /// `ota.gds.gz` gives `("ota", Gds)`.
    pub fn split_name(file_name: &str) -> Option<(&str, Format)> {
        let lower = file_name.to_ascii_lowercase();
        SUFFIXES
            .iter()
            .filter(|(suffix, _)| lower.ends_with(suffix) && lower.len() > suffix.len())
            .max_by_key(|(suffix, _)| suffix.len())
            .map(|(suffix, format)| (&file_name[..file_name.len() - suffix.len()], *format))
    }

    /// Format of a file on disk. `.sch` is shared by xschem and Qucs-S, so the first
    /// bytes of the file decide between them.
    pub fn of_file(path: &Path) -> Option<Format> {
        let name = path.file_name()?.to_str()?;
        let (_, format) = Self::split_name(name)?;
        if format == Format::XschemSchematic && is_qucs(path) {
            return Some(Format::QucsSchematic);
        }
        Some(format)
    }

    pub fn kind(self) -> ViewKind {
        match self {
            Self::XschemSchematic | Self::QucsSchematic => ViewKind::Schematic,
            Self::XschemSymbol => ViewKind::Symbol,
            Self::Gds | Self::Oasis | Self::Magic => ViewKind::Layout,
            Self::Spice | Self::Cdl => ViewKind::Netlist,
            Self::Verilog | Self::SystemVerilog | Self::Vhdl | Self::VerilogA => ViewKind::Hdl,
            Self::Lef => ViewKind::Abstract,
            Self::Liberty => ViewKind::Liberty,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::XschemSchematic => "xschem schematic",
            Self::XschemSymbol => "xschem symbol",
            Self::QucsSchematic => "Qucs-S schematic",
            Self::Gds => "GDSII",
            Self::Oasis => "OASIS",
            Self::Magic => "Magic layout",
            Self::Spice => "SPICE netlist",
            Self::Cdl => "CDL netlist",
            Self::Verilog => "Verilog",
            Self::SystemVerilog => "SystemVerilog",
            Self::Vhdl => "VHDL",
            Self::VerilogA => "Verilog-A",
            Self::Lef => "LEF",
            Self::Liberty => "Liberty",
        }
    }

    /// The tool that edits this format, by its name in
    /// [`KNOWN_TOOLS`](crate::tools::KNOWN_TOOLS). Text formats have none.
    pub fn tool(self) -> Option<&'static str> {
        match self {
            Self::XschemSchematic | Self::XschemSymbol => Some("xschem"),
            Self::QucsSchematic => Some("Qucs-S"),
            Self::Gds | Self::Oasis => Some("KLayout"),
            Self::Magic => Some("Magic"),
            _ => None,
        }
    }
}

fn is_qucs(path: &Path) -> bool {
    let mut head = [0u8; 16];
    let Ok(mut file) = File::open(path) else {
        return false;
    };
    let n = file.read(&mut head).unwrap_or(0);
    head[..n].starts_with(b"<Qucs")
}

/// One view of a cell, as found on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// View name, e.g. `schematic`, `layout`, `tb_ac`.
    pub name: String,
    pub kind: ViewKind,
    /// The view folder, or the file itself when the view is a single loose file.
    pub path: PathBuf,
    /// The file a tool opens, if the view has one.
    pub main_file: Option<PathBuf>,
    pub format: Option<Format>,
    /// All files of the view (not recursive), sorted.
    pub files: Vec<PathBuf>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_names_by_longest_suffix() {
        assert_eq!(
            Format::split_name("ota.sch"),
            Some(("ota", Format::XschemSchematic))
        );
        assert_eq!(Format::split_name("ota.GDS"), Some(("ota", Format::Gds)));
        assert_eq!(Format::split_name("ota.gds.gz"), Some(("ota", Format::Gds)));
        assert_eq!(
            Format::split_name("a.b.sv"),
            Some(("a.b", Format::SystemVerilog))
        );
        assert_eq!(Format::split_name("top.v"), Some(("top", Format::Verilog)));
        assert_eq!(Format::split_name("README.md"), None);
        // A bare suffix is not a cell.
        assert_eq!(Format::split_name(".sch"), None);
    }

    #[test]
    fn view_names_suggest_kinds() {
        assert_eq!(
            ViewKind::from_view_name("schematic"),
            Some(ViewKind::Schematic)
        );
        assert_eq!(ViewKind::from_view_name("tb_ac"), Some(ViewKind::Testbench));
        assert_eq!(ViewKind::from_view_name("other"), None);
        assert_eq!(ViewKind::from_view_name("notes"), None);
    }
}
