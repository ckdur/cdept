//! Starting content for new views, so that a freshly created view opens in its tool.

use crate::{Format, ViewKind};

/// A kind of view CDePT can create from scratch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Template {
    XschemSchematic,
    XschemSymbol,
    GdsLayout,
    SpiceNetlist,
    Verilog,
}

const XSCHEM_HEADER: &str = "v {xschem version=3.4.5 file_version=1.2\n}\n";

impl Template {
    pub const ALL: [Template; 5] = [
        Self::XschemSchematic,
        Self::XschemSymbol,
        Self::GdsLayout,
        Self::SpiceNetlist,
        Self::Verilog,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::XschemSchematic => "Schematic (xschem)",
            Self::XschemSymbol => "Symbol (xschem)",
            Self::GdsLayout => "Layout (GDSII)",
            Self::SpiceNetlist => "Netlist (SPICE)",
            Self::Verilog => "HDL (Verilog)",
        }
    }

    pub fn format(self) -> Format {
        match self {
            Self::XschemSchematic => Format::XschemSchematic,
            Self::XschemSymbol => Format::XschemSymbol,
            Self::GdsLayout => Format::Gds,
            Self::SpiceNetlist => Format::Spice,
            Self::Verilog => Format::Verilog,
        }
    }

    pub fn kind(self) -> ViewKind {
        self.format().kind()
    }

    /// Default view name, e.g. `schematic`.
    pub fn view_name(self) -> &'static str {
        self.kind().name()
    }

    /// File extension, without the dot.
    pub fn extension(self) -> &'static str {
        match self {
            Self::XschemSchematic => "sch",
            Self::XschemSymbol => "sym",
            Self::GdsLayout => "gds",
            Self::SpiceNetlist => "spice",
            Self::Verilog => "v",
        }
    }

    /// File name of the view's main file for `cell`.
    pub fn file_name(self, cell: &str) -> String {
        format!("{cell}.{}", self.extension())
    }

    pub fn contents(self, cell: &str) -> Vec<u8> {
        match self {
            Self::XschemSchematic => {
                format!("{XSCHEM_HEADER}G {{}}\nK {{}}\nV {{}}\nS {{}}\nE {{}}\n").into_bytes()
            }
            Self::XschemSymbol => format!(
                "{XSCHEM_HEADER}G {{}}\nK {{type=subcircuit\nformat=\"@name @pinlist @symname\"\n\
                 template=\"name=x1\"\n}}\nV {{}}\nS {{}}\nE {{}}\n"
            )
            .into_bytes(),
            Self::GdsLayout => empty_gds(cell),
            Self::SpiceNetlist => format!("* {cell}\n.subckt {cell}\n.ends {cell}\n").into_bytes(),
            Self::Verilog => format!("module {cell} ();\n\nendmodule\n").into_bytes(),
        }
    }
}

/// A GDSII stream with one empty structure named `cell`.
/// Database unit 1 nm, user unit 1 µm.
fn empty_gds(cell: &str) -> Vec<u8> {
    // GDSII record types (record type byte, data type byte).
    const HEADER: u16 = 0x0002;
    const BGNLIB: u16 = 0x0102;
    const LIBNAME: u16 = 0x0206;
    const UNITS: u16 = 0x0305;
    const ENDLIB: u16 = 0x0400;
    const BGNSTR: u16 = 0x0502;
    const STRNAME: u16 = 0x0606;
    const ENDSTR: u16 = 0x0700;
    // 0.001 (user units per database unit) and 1e-9 (metres per database unit) as
    // GDSII 8-byte excess-64 reals, rounded to nearest so they read back exactly.
    const UNITS_DATA: [u8; 16] = [
        0x3E, 0x41, 0x89, 0x37, 0x4B, 0xC6, 0xA7, 0xF0, 0x39, 0x44, 0xB8, 0x2F, 0xA0, 0x9B, 0x5A,
        0x53,
    ];

    fn record(out: &mut Vec<u8>, kind: u16, data: &[u8]) {
        out.extend_from_slice(&((data.len() + 4) as u16).to_be_bytes());
        out.extend_from_slice(&kind.to_be_bytes());
        out.extend_from_slice(data);
    }
    fn string(text: &str) -> Vec<u8> {
        let mut bytes = text.as_bytes().to_vec();
        if bytes.len() % 2 == 1 {
            bytes.push(0);
        }
        bytes
    }
    // Modification and access time: year, month, day, hour, minute, second, twice.
    let dates: Vec<u8> = [1970i16, 1, 1, 0, 0, 0]
        .iter()
        .cycle()
        .take(12)
        .flat_map(|v| v.to_be_bytes())
        .collect();

    let mut out = Vec::new();
    record(&mut out, HEADER, &600i16.to_be_bytes());
    record(&mut out, BGNLIB, &dates);
    record(&mut out, LIBNAME, &string("LIB"));
    record(&mut out, UNITS, &UNITS_DATA);
    record(&mut out, BGNSTR, &dates);
    record(&mut out, STRNAME, &string(cell));
    record(&mut out, ENDSTR, &[]);
    record(&mut out, ENDLIB, &[]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gds_records_are_well_formed() {
        let gds = empty_gds("ota");
        // Walk the records: each starts with its own length.
        let mut pos = 0;
        let mut kinds = Vec::new();
        while pos < gds.len() {
            let len = u16::from_be_bytes([gds[pos], gds[pos + 1]]) as usize;
            assert!(len >= 4 && len.is_multiple_of(2));
            kinds.push(u16::from_be_bytes([gds[pos + 2], gds[pos + 3]]));
            pos += len;
        }
        assert_eq!(pos, gds.len());
        assert_eq!(
            kinds,
            [
                0x0002, 0x0102, 0x0206, 0x0305, 0x0502, 0x0606, 0x0700, 0x0400
            ]
        );
        // The odd-length name is padded.
        assert!(gds.windows(4).any(|w| w == b"ota\0"));
    }

    #[test]
    fn templates_match_their_extension() {
        for t in Template::ALL {
            let name = t.file_name("ota");
            assert_eq!(Format::split_name(&name), Some(("ota", t.format())));
        }
    }
}
