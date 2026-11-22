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
    KlayoutPcell,
    MagicLayout,
}

/// The xschem version and file version new schematics and symbols are written for:
/// those of the embedded engine (the `cdept-xschem/xschem` submodule; a test compares
/// them with its `xschem.h`). xschem reads older headers and rewrites them on save.
pub const XSCHEM_VERSION: &str = "3.4.8RC";
pub const XSCHEM_FILE_VERSION: &str = "1.3";

/// What a new view starts from besides the cell name: the technology's part. A project
/// gives its own with [`crate::Project::template_setup`]; the default is for a
/// project without a technology snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct Setup {
    /// Database unit of a new layout, in micron.
    pub dbu: f64,
    /// The technology name magic knows the PDK by (the `tech` line of a `.mag` file).
    /// `None`: `scmos`, magic's built-in technology.
    pub magic_tech: Option<String>,
    /// xschem's version and file version for the header of new schematics and symbols:
    /// those of the engine in use when it reports others than the built-in ones.
    pub xschem: Option<(String, String)>,
}

impl Default for Setup {
    fn default() -> Self {
        Setup {
            dbu: 0.001,
            magic_tech: None,
            xschem: None,
        }
    }
}

impl Setup {
    /// The first lines of a new xschem file.
    fn xschem_header(&self) -> String {
        let (version, file_version) = match &self.xschem {
            Some((version, file_version)) => (version.as_str(), file_version.as_str()),
            None => (XSCHEM_VERSION, XSCHEM_FILE_VERSION),
        };
        format!("v {{xschem version={version} file_version={file_version}\n}}\n")
    }
}

impl Template {
    pub const ALL: [Template; 7] = [
        Self::XschemSchematic,
        Self::XschemSymbol,
        Self::GdsLayout,
        Self::SpiceNetlist,
        Self::Verilog,
        Self::KlayoutPcell,
        Self::MagicLayout,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::XschemSchematic => "Schematic (xschem)",
            Self::XschemSymbol => "Symbol (xschem)",
            Self::GdsLayout => "Layout (GDSII)",
            Self::SpiceNetlist => "Netlist (SPICE)",
            Self::Verilog => "HDL (Verilog)",
            Self::KlayoutPcell => "PCell (KLayout, Python)",
            Self::MagicLayout => "Layout (Magic)",
        }
    }

    pub fn format(self) -> Format {
        match self {
            Self::XschemSchematic => Format::XschemSchematic,
            Self::XschemSymbol => Format::XschemSymbol,
            Self::GdsLayout => Format::Gds,
            Self::SpiceNetlist => Format::Spice,
            Self::Verilog => Format::Verilog,
            Self::KlayoutPcell => Format::KlayoutPcell,
            Self::MagicLayout => Format::Magic,
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
            Self::KlayoutPcell => "pcell.py",
            Self::MagicLayout => "mag",
        }
    }

    /// File name of the view's main file for `cell`.
    pub fn file_name(self, cell: &str) -> String {
        format!("{cell}.{}", self.extension())
    }

    /// The starting content of a view of `cell`, without a technology.
    pub fn contents(self, cell: &str) -> Vec<u8> {
        self.contents_with(cell, &Setup::default())
    }

    /// The starting content of a view of `cell` for a project's technology.
    pub fn contents_with(self, cell: &str, setup: &Setup) -> Vec<u8> {
        let header = setup.xschem_header();
        match self {
            Self::XschemSchematic => {
                format!("{header}G {{}}\nK {{}}\nV {{}}\nS {{}}\nE {{}}\n").into_bytes()
            }
            Self::XschemSymbol => format!(
                "{header}G {{}}\nK {{type=subcircuit\nformat=\"@name @pinlist @symname\"\n\
                 template=\"name=x1\"\n}}\nV {{}}\nS {{}}\nE {{}}\n"
            )
            .into_bytes(),
            Self::GdsLayout => empty_gds(cell, setup.dbu),
            Self::MagicLayout => empty_mag(setup.magic_tech.as_deref()).into_bytes(),
            Self::SpiceNetlist => format!("* {cell}\n.subckt {cell}\n.ends {cell}\n").into_bytes(),
            Self::Verilog => format!("module {cell} ();\n\nendmodule\n").into_bytes(),
            Self::KlayoutPcell => pcell_python(cell).into_bytes(),
        }
    }
}

/// A working PCell: a resistor-like bar of `l` × `w` µm with a contact square at each
/// end, on layers that are parameters (docs/KLAYOUT.md §3.6 for the conventions).
fn pcell_python(cell: &str) -> String {
    let class: String = cell
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|p| !p.is_empty())
        .map(|p| {
            let mut chars = p.chars();
            chars.next().map_or(String::new(), |c| {
                c.to_ascii_uppercase().to_string() + chars.as_str()
            })
        })
        .collect();
    let class = if class.is_empty() || class.starts_with(|c: char| c.is_ascii_digit()) {
        format!("Pcell{class}")
    } else {
        class
    };
    PCELL_TEMPLATE
        .replace("@CELL@", cell)
        .replace("@CLASS@", &class)
}

const PCELL_TEMPLATE: &str = r#"# PCell "@CELL@" for KLayout, registered by CDePT in the KLayout library named after
# this CDePT library. Conventions (docs/KLAYOUT.md §3.6):
# - the file is <cell>.pcell.py in the cell's pcell view; the PCell is named after the cell;
# - it defines one subclass of pya.PCellDeclarationHelper (or names it: PCELL = ...);
# - saving this file updates every layout open in CDePT that places the PCell.
import pya


class @CLASS@(pya.PCellDeclarationHelper):
    def __init__(self):
        super().__init__()
        self.param("body", self.TypeLayer, "Body layer", default=pya.LayerInfo(1, 0))
        self.param("contact", self.TypeLayer, "Contact layer", default=pya.LayerInfo(2, 0))
        self.param("l", self.TypeDouble, "Length", default=2.0, unit="µm")
        self.param("w", self.TypeDouble, "Width", default=0.5, unit="µm")
        self.param("c", self.TypeDouble, "Contact size", default=0.3, unit="µm")

    def display_text_impl(self):
        return "@CELL@(l=%g, w=%g)" % (self.l, self.w)

    def coerce_parameters_impl(self):
        self.w = max(self.w, self.c)
        self.l = max(self.l, 2 * self.c)

    def produce_impl(self):
        l, w, c = self.l, self.w, self.c
        self.cell.shapes(self.body_layer).insert(pya.DBox(0, 0, l, w))
        y = (w - c) / 2
        for x in (0, l - c):
            self.cell.shapes(self.contact_layer).insert(pya.DBox(x, y, x + c, y + c))


PCELL = @CLASS@
"#;

/// An empty magic layout for the technology `tech` (`scmos`, magic's built-in one,
/// when there is none). The cell name is the file name; magic fills in the rest
/// (`magscale`, the time stamp) when it saves.
fn empty_mag(tech: Option<&str>) -> String {
    let tech = tech.filter(|t| !t.trim().is_empty()).unwrap_or("scmos");
    format!("magic\ntech {tech}\ntimestamp 0\n<< end >>\n")
}

/// `digits / 10^pow10` as a GDSII 8-byte real: sign, excess-64 exponent of 16, 56-bit
/// mantissa. Worked out in whole numbers and rounded to nearest, so that units such as
/// 0.001 and 1e-9 come out as every other GDS writer has them (a `f64` has fewer bits
/// than the mantissa). `None` when the number has too many digits for that.
fn gds_real_decimal(digits: u64, pow10: u32) -> Option<[u8; 8]> {
    if digits == 0 {
        return Some([0; 8]);
    }
    if digits >= 1_000_000_000 || pow10 > 18 {
        return None;
    }
    let mut n = digits as u128;
    let mut d = 10u128.pow(pow10);
    let mut exponent: i32 = 64;
    while n >= d {
        d *= 16;
        exponent += 1;
    }
    while n * 16 < d {
        n *= 16;
        exponent -= 1;
    }
    // n/d is in [1/16, 1).
    let mut mantissa = ((n << 56) + d / 2) / d;
    if mantissa >= 1 << 56 {
        mantissa >>= 4;
        exponent += 1;
    }
    let mut out = (mantissa as u64).to_be_bytes();
    out[0] = u8::try_from(exponent).ok().filter(|e| *e < 128)?;
    Some(out)
}

/// A positive number as written in decimal: its digits and how many of them follow the
/// decimal point (`0.005` is 5 and 3).
fn decimal(value: f64) -> Option<(u64, u32)> {
    let text = format!("{value}");
    let (whole, fraction) = text.split_once('.').unwrap_or((&text, ""));
    let digits: u64 = format!("{whole}{fraction}").parse().ok()?;
    Some((digits, fraction.len() as u32))
}

/// The UNITS record of a layout with database unit `dbu` micron: user units (micron)
/// per database unit, then metres per database unit.
fn gds_units(dbu: f64) -> [u8; 16] {
    let reals = decimal(dbu).and_then(|(digits, places)| {
        Some((
            gds_real_decimal(digits, places)?,
            gds_real_decimal(digits, places + 6)?,
        ))
    });
    // A unit with very many digits: 1 nm.
    let (user, metres) = reals.unwrap_or((
        [0x3E, 0x41, 0x89, 0x37, 0x4B, 0xC6, 0xA7, 0xF0],
        [0x39, 0x44, 0xB8, 0x2F, 0xA0, 0x9B, 0x5A, 0x53],
    ));
    let mut units = [0u8; 16];
    units[..8].copy_from_slice(&user);
    units[8..].copy_from_slice(&metres);
    units
}

/// A GDSII stream with one empty structure named `cell`.
/// Database unit `dbu` micron (1 nm when that is no usable number), user unit 1 µm.
fn empty_gds(cell: &str, dbu: f64) -> Vec<u8> {
    let dbu = if dbu.is_finite() && dbu > 0.0 {
        dbu
    } else {
        0.001
    };
    // GDSII record types (record type byte, data type byte).
    const HEADER: u16 = 0x0002;
    const BGNLIB: u16 = 0x0102;
    const LIBNAME: u16 = 0x0206;
    const UNITS: u16 = 0x0305;
    const ENDLIB: u16 = 0x0400;
    const BGNSTR: u16 = 0x0502;
    const STRNAME: u16 = 0x0606;
    const ENDSTR: u16 = 0x0700;
    let units = gds_units(dbu);

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
    record(&mut out, UNITS, &units);
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
        let gds = empty_gds("ota", 0.001);
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
    fn gds_units_follow_the_database_unit() {
        // 0.001 and 1e-9, as every GDS writer has them for 1 nm.
        assert_eq!(
            gds_real_decimal(1, 3),
            Some([0x3E, 0x41, 0x89, 0x37, 0x4B, 0xC6, 0xA7, 0xF0])
        );
        assert_eq!(
            gds_real_decimal(1, 9),
            Some([0x39, 0x44, 0xB8, 0x2F, 0xA0, 0x9B, 0x5A, 0x53])
        );
        assert_eq!(gds_real_decimal(1, 0), Some([0x41, 0x10, 0, 0, 0, 0, 0, 0]));
        assert_eq!(gds_real_decimal(32, 0).unwrap()[..2], [0x42, 0x20]);
        assert_eq!(gds_real_decimal(0, 0), Some([0; 8]));
        assert_eq!(decimal(0.005), Some((5, 3)));
        assert_eq!(decimal(2.0), Some((2, 0)));
        // The reader of the layout module gets the unit back.
        for dbu in [0.001, 0.005, 0.0005, 0.01] {
            let setup = Setup {
                dbu,
                ..Setup::default()
            };
            let gds = Template::GdsLayout.contents_with("ota", &setup);
            let summary = crate::layout::gds::parse(&gds).unwrap();
            assert_eq!(summary.top_cells(), ["ota"]);
            let at = gds.windows(4).position(|w| w == [0, 20, 3, 5]).unwrap();
            assert_eq!(gds[at + 4..at + 20], gds_units(dbu));
            // Micron per database unit, read back as a number.
            let real = &gds[at + 4..at + 12];
            let mantissa = real[1..].iter().fold(0u64, |m, b| (m << 8) | *b as u64);
            let back = mantissa as f64 / (1u64 << 56) as f64 * 16f64.powi(real[0] as i32 - 64);
            assert!((back - dbu).abs() < 1e-15, "{dbu} read back as {back}");
        }
        // A unit that is no number falls back to 1 nm.
        assert_eq!(empty_gds("a", f64::NAN), empty_gds("a", 0.001));
    }

    #[test]
    fn magic_and_xschem_follow_the_setup() {
        let text = String::from_utf8(Template::MagicLayout.contents("ota")).unwrap();
        assert_eq!(text, "magic\ntech scmos\ntimestamp 0\n<< end >>\n");
        let setup = Setup {
            magic_tech: Some("sky130A".into()),
            xschem: Some(("3.4.9".into(), "1.4".into())),
            ..Setup::default()
        };
        let text = String::from_utf8(Template::MagicLayout.contents_with("ota", &setup)).unwrap();
        assert!(text.contains("\ntech sky130A\n"), "{text}");
        let text =
            String::from_utf8(Template::XschemSchematic.contents_with("ota", &setup)).unwrap();
        assert!(text.starts_with("v {xschem version=3.4.9 file_version=1.4\n}\n"));
        let text = String::from_utf8(Template::XschemSymbol.contents("ota")).unwrap();
        assert!(text.starts_with(&format!(
            "v {{xschem version={XSCHEM_VERSION} file_version={XSCHEM_FILE_VERSION}\n}}\n"
        )));
    }

    #[test]
    fn the_xschem_header_is_the_one_of_the_engine() {
        // The embedded engine's source, when the submodule is checked out.
        let header = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../cdept-xschem/xschem/src/xschem.h");
        let Ok(text) = std::fs::read_to_string(header) else {
            return;
        };
        let define = |name: &str| {
            text.lines()
                .find_map(|line| line.strip_prefix(&format!("#define {name} \"")))
                .and_then(|rest| rest.split('"').next())
                .map(str::to_string)
        };
        assert_eq!(define("XSCHEM_VERSION").as_deref(), Some(XSCHEM_VERSION));
        assert_eq!(
            define("XSCHEM_FILE_VERSION").as_deref(),
            Some(XSCHEM_FILE_VERSION)
        );
    }

    #[test]
    fn pcell_template_names_its_class() {
        let text = String::from_utf8(Template::KlayoutPcell.contents("r_poly-2")).unwrap();
        assert!(
            text.contains("class RPoly2(pya.PCellDeclarationHelper):"),
            "{text}"
        );
        assert!(text.contains("PCELL = RPoly2"));
        let text = String::from_utf8(Template::KlayoutPcell.contents("2r")).unwrap();
        assert!(text.contains("class Pcell2r("));
    }

    #[test]
    fn templates_match_their_extension() {
        for t in Template::ALL {
            let name = t.file_name("ota");
            assert_eq!(Format::split_name(&name), Some(("ota", t.format())));
        }
    }
}
