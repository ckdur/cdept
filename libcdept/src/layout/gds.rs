//! A small GDSII reader and rewriter: structure names, the names they reference, and
//! renaming structures. Geometry is skipped.
//!
//! Only what the reference machinery needs is understood: `HEADER`, `BGNLIB`,
//! `LIBNAME`, `UNITS`, `BGNSTR`, `STRNAME`, `ENDSTR`, `SREF`, `AREF`, `SNAME` and
//! `ENDLIB`. Every other record is skipped by its length. Compressed files (`.gds.gz`)
//! are refused with an error.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use eyre::{Context, Result, bail};

// Record types (record byte, data type byte).
const BGNLIB: u16 = 0x0102;
const LIBNAME: u16 = 0x0206;
const UNITS: u16 = 0x0305;
const ENDLIB: u16 = 0x0400;
const BGNSTR: u16 = 0x0502;
const STRNAME: u16 = 0x0606;
const ENDSTR: u16 = 0x0700;
const SNAME: u16 = 0x1206;
/// The structure in which KLayout records where library cells and PCell variants come
/// from (one SREF per proxy cell, with properties).
pub const CONTEXT_INFO: &str = "$$$CONTEXT_INFO$$$";

/// What [`read_structure`] finds in a GDS file.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GdsSummary {
    pub library: String,
    /// Size of a database unit in user units (e.g. 0.001 for 1 nm in µm).
    pub user_per_dbu: f64,
    /// Size of a database unit in metres (e.g. 1e-9).
    pub dbu_in_meters: f64,
    /// In file order.
    pub structures: Vec<Structure>,
}

/// One structure (cell) and the names it places with `SREF` and `AREF`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Structure {
    pub name: String,
    pub refs: BTreeSet<String>,
}

impl GdsSummary {
    /// Names placed somewhere but not defined in the file (ghost cells), sorted.
    pub fn ghosts(&self) -> Vec<String> {
        let defined: BTreeSet<&str> = self.structures.iter().map(|s| s.name.as_str()).collect();
        let referenced: BTreeSet<&String> = self.structures.iter().flat_map(|s| &s.refs).collect();
        referenced
            .into_iter()
            .filter(|name| !defined.contains(name.as_str()))
            .cloned()
            .collect()
    }

    /// Structures defined and not placed by any structure of the file, in file order.
    pub fn top_cells(&self) -> Vec<String> {
        let referenced: BTreeSet<&String> = self.structures.iter().flat_map(|s| &s.refs).collect();
        self.structures
            .iter()
            .filter(|s| !referenced.contains(&s.name))
            .map(|s| s.name.clone())
            .collect()
    }

    pub fn defines(&self, name: &str) -> bool {
        self.structures.iter().any(|s| s.name == name)
    }
}

/// Walks the records of a stream: `(type, data)` pairs. A zero length after `ENDLIB`
/// (tape padding) ends it quietly.
fn records(bytes: &[u8]) -> impl Iterator<Item = Result<(u16, &[u8])>> {
    let mut pos = 0;
    let mut done = false;
    std::iter::from_fn(move || {
        if done || pos >= bytes.len() {
            return None;
        }
        let item = (|| {
            if pos + 4 > bytes.len() {
                bail!("truncated record header at byte {pos}");
            }
            let len = u16::from_be_bytes([bytes[pos], bytes[pos + 1]]) as usize;
            if len < 4 || pos + len > bytes.len() {
                bail!("bad record length {len} at byte {pos}");
            }
            let kind = u16::from_be_bytes([bytes[pos + 2], bytes[pos + 3]]);
            let data = &bytes[pos + 4..pos + len];
            pos += len;
            Ok((kind, data))
        })();
        if item.is_err() {
            done = true;
        }
        Some(item)
    })
}

fn text(data: &[u8]) -> String {
    let end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
    String::from_utf8_lossy(&data[..end]).into_owned()
}

/// A GDSII 8-byte excess-64 real.
fn real(data: &[u8]) -> f64 {
    if data.len() < 8 {
        return 0.0;
    }
    let sign = if data[0] & 0x80 != 0 { -1.0 } else { 1.0 };
    let exponent = i32::from(data[0] & 0x7f) - 64;
    let mantissa = data[1..8]
        .iter()
        .fold(0u64, |acc, &b| (acc << 8) | u64::from(b)) as f64
        / (1u64 << 56) as f64;
    sign * mantissa * 16f64.powi(exponent)
}

fn load(path: &Path) -> Result<Vec<u8>> {
    let bytes = fs::read(path).wrap_err_with(|| format!("cannot read {}", path.display()))?;
    if bytes.starts_with(&[0x1f, 0x8b]) {
        bail!(
            "{} is compressed (gzip); uncompressed GDSII only",
            path.display()
        );
    }
    Ok(bytes)
}

/// Read the structures of a GDSII file and the names each one places.
pub fn read_structure(path: &Path) -> Result<GdsSummary> {
    let bytes = load(path)?;
    parse(&bytes).wrap_err_with(|| format!("{} is not a valid GDSII stream", path.display()))
}

/// [`read_structure`] on bytes in memory.
pub fn parse(bytes: &[u8]) -> Result<GdsSummary> {
    let mut summary = GdsSummary::default();
    let mut current: Option<Structure> = None;
    let mut first = true;
    let mut ended = false;
    for record in records(bytes) {
        let (kind, data) = record?;
        if first {
            if kind != 0x0002 {
                bail!("no HEADER record");
            }
            first = false;
        }
        match kind {
            BGNLIB => {}
            LIBNAME => summary.library = text(data),
            UNITS if data.len() >= 16 => {
                summary.user_per_dbu = real(&data[..8]);
                summary.dbu_in_meters = real(&data[8..16]);
            }
            BGNSTR => current = Some(Structure::default()),
            STRNAME => {
                if let Some(s) = &mut current {
                    s.name = text(data);
                }
            }
            SNAME => {
                if let Some(s) = &mut current {
                    s.refs.insert(text(data));
                }
            }
            // KLayout's context structure (library and PCell links of proxy cells,
            // docs/KLAYOUT.md §3.6) is not a cell: it would show as a second top.
            ENDSTR => summary
                .structures
                .extend(current.take().filter(|s| s.name != CONTEXT_INFO)),
            ENDLIB => {
                ended = true;
                break;
            }
            _ => {}
        }
    }
    if first {
        bail!("empty file");
    }
    if !ended {
        bail!("no ENDLIB record (truncated file?)");
    }
    Ok(summary)
}

/// One cell placement (`SREF` or `AREF`) and the value of one of its properties.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    /// The structure the placement is in.
    pub structure: String,
    /// The structure placed.
    pub cell: String,
    /// The value of the property asked for, when the placement has it.
    pub value: Option<String>,
}

/// Every `SREF` and `AREF` of the stream with the value of its property `attr`
/// (`PROPATTR` / `PROPVALUE`), in file order.
pub fn placements(bytes: &[u8], attr: u16) -> Result<Vec<Placement>> {
    const SREF: u16 = 0x0a00;
    const AREF: u16 = 0x0b00;
    const ENDEL: u16 = 0x1100;
    const PROPATTR: u16 = 0x2b02;
    const PROPVALUE: u16 = 0x2c06;
    let mut out = Vec::new();
    let mut structure = String::new();
    let mut current: Option<Placement> = None;
    let mut wanted = false;
    for record in records(bytes) {
        let (kind, data) = record?;
        match kind {
            STRNAME => structure = text(data),
            SREF | AREF => {
                current = Some(Placement {
                    structure: structure.clone(),
                    cell: String::new(),
                    value: None,
                })
            }
            SNAME => {
                if let Some(p) = &mut current {
                    p.cell = text(data);
                }
            }
            PROPATTR => wanted = data.len() >= 2 && u16::from_be_bytes([data[0], data[1]]) == attr,
            PROPVALUE if wanted => {
                if let Some(p) = &mut current {
                    p.value = Some(text(data));
                }
                wanted = false;
            }
            ENDEL => out.extend(current.take()),
            ENDLIB => break,
            _ => {}
        }
    }
    Ok(out)
}

/// A shape (`BOUNDARY`, `BOX`, `PATH`, `TEXT`) that carries a property.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tagged {
    /// The structure it is in.
    pub structure: String,
    pub layer: i16,
    /// The datatype, or the texttype of a text.
    pub datatype: i16,
    pub text: bool,
    pub value: String,
}

/// Every shape of the stream with a value for its property `attr`, in file order
/// (pins carry their name in property 62, docs/LINKING.md §9).
pub fn tagged_shapes(bytes: &[u8], attr: u16) -> Result<Vec<Tagged>> {
    const BOUNDARY: u16 = 0x0800;
    const PATH: u16 = 0x0900;
    const TEXT: u16 = 0x0c00;
    const BOX: u16 = 0x2d00;
    const LAYER: u16 = 0x0d02;
    const DATATYPE: u16 = 0x0e02;
    const TEXTTYPE: u16 = 0x1602;
    const BOXTYPE: u16 = 0x2e02;
    const ENDEL: u16 = 0x1100;
    const PROPATTR: u16 = 0x2b02;
    const PROPVALUE: u16 = 0x2c06;
    let int = |data: &[u8]| -> i16 {
        if data.len() >= 2 {
            i16::from_be_bytes([data[0], data[1]])
        } else {
            0
        }
    };
    let mut out = Vec::new();
    let mut structure = String::new();
    let mut current: Option<Tagged> = None;
    let mut wanted = false;
    for record in records(bytes) {
        let (kind, data) = record?;
        match kind {
            STRNAME => structure = text(data),
            BOUNDARY | PATH | TEXT | BOX => {
                current = Some(Tagged {
                    structure: structure.clone(),
                    layer: 0,
                    datatype: 0,
                    text: kind == TEXT,
                    value: String::new(),
                })
            }
            LAYER => {
                if let Some(t) = &mut current {
                    t.layer = int(data);
                }
            }
            DATATYPE | TEXTTYPE | BOXTYPE => {
                if let Some(t) = &mut current {
                    t.datatype = int(data);
                }
            }
            PROPATTR => wanted = data.len() >= 2 && u16::from_be_bytes([data[0], data[1]]) == attr,
            PROPVALUE if wanted => {
                if let Some(t) = &mut current {
                    t.value = text(data);
                }
                wanted = false;
            }
            ENDEL => {
                if let Some(t) = current.take()
                    && !t.value.is_empty()
                {
                    out.push(t);
                }
            }
            ENDLIB => break,
            _ => {}
        }
    }
    Ok(out)
}

/// Rename structures and the references to them: every `STRNAME` and `SNAME` equal to a
/// key of `map` gets the value as name. Record lengths are recomputed (names are padded
/// to an even length). The file is replaced through a temporary file; it is not touched
/// when nothing changes. Fails if a new name is longer than GDSII allows or if renaming
/// would make two structures share a name.
pub fn rename_structures(path: &Path, map: &BTreeMap<String, String>) -> Result<()> {
    let bytes = load(path)?;
    if let Some(out) = rename_bytes(&bytes, map)
        .wrap_err_with(|| format!("cannot rename structures in {}", path.display()))?
    {
        super::write_atomic(path, &out)?;
    }
    Ok(())
}

/// [`rename_structures`] on bytes in memory; `None` when nothing changed.
pub fn rename_bytes(bytes: &[u8], map: &BTreeMap<String, String>) -> Result<Option<Vec<u8>>> {
    if map.is_empty() {
        return Ok(None);
    }
    let mut out = Vec::with_capacity(bytes.len() + 16);
    let mut changed = false;
    let mut defined = BTreeSet::new();
    for record in records(bytes) {
        let (kind, data) = record?;
        let mut new_name = None;
        if kind == STRNAME || kind == SNAME {
            let name = text(data);
            if kind == STRNAME && !defined.insert(map.get(&name).unwrap_or(&name).clone()) {
                bail!(
                    "two structures would be named '{}'",
                    map.get(&name).unwrap_or(&name)
                );
            }
            new_name = map.get(&name);
        }
        if let Some(name) = new_name {
            let mut data = name.as_bytes().to_vec();
            if data.len() % 2 == 1 {
                data.push(0);
            }
            if data.len() + 4 > usize::from(u16::MAX) - 1 || name.is_empty() {
                bail!("'{name}' is not a usable structure name");
            }
            out.extend_from_slice(&((data.len() + 4) as u16).to_be_bytes());
            out.extend_from_slice(&kind.to_be_bytes());
            out.extend_from_slice(&data);
            changed = true;
        } else {
            out.extend_from_slice(&((data.len() + 4) as u16).to_be_bytes());
            out.extend_from_slice(&kind.to_be_bytes());
            out.extend_from_slice(data);
        }
        if kind == ENDLIB {
            // Tape padding after ENDLIB is dropped.
            break;
        }
    }
    Ok(changed.then_some(out))
}

/// A standalone stream with the structure `top` and every structure it places, at any
/// depth: the library header and units are kept, the structures keep their order and
/// content, the others are dropped. Fails when `top` is not defined in the stream.
/// Names placed but not defined (ghost cells) stay ghosts.
pub fn extract_structure(bytes: &[u8], top: &str) -> Result<Vec<u8>> {
    let summary = parse(bytes)?;
    if !summary.defines(top) {
        bail!("the stream has no structure '{top}'");
    }
    let mut wanted: BTreeSet<&str> = BTreeSet::new();
    let mut todo = vec![top];
    while let Some(name) = todo.pop() {
        if !wanted.insert(name) {
            continue;
        }
        if let Some(s) = summary.structures.iter().find(|s| s.name == name) {
            todo.extend(s.refs.iter().map(String::as_str));
        }
    }
    let mut out = Vec::new();
    let mut pending: Vec<u8> = Vec::new();
    let mut inside = false;
    let mut name = String::new();
    for record in records(bytes) {
        let (kind, data) = record?;
        let mut raw = Vec::with_capacity(data.len() + 4);
        raw.extend_from_slice(&((data.len() + 4) as u16).to_be_bytes());
        raw.extend_from_slice(&kind.to_be_bytes());
        raw.extend_from_slice(data);
        match kind {
            BGNSTR => {
                inside = true;
                name.clear();
                pending = raw;
            }
            ENDSTR => {
                pending.extend_from_slice(&raw);
                inside = false;
                if wanted.contains(name.as_str()) {
                    out.append(&mut pending);
                }
                pending.clear();
            }
            ENDLIB => {
                out.extend_from_slice(&raw);
                break;
            }
            _ if inside => {
                if kind == STRNAME {
                    name = text(data);
                }
                pending.extend_from_slice(&raw);
            }
            _ => out.extend_from_slice(&raw),
        }
    }
    Ok(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn extracts_a_structure_and_what_it_places() {
        let gds = build(&[
            ("inv", &[]),
            ("other", &["inv"]),
            ("buf", &["inv", "leaf", "inv"]),
            ("leaf", &[]),
            ("unused", &["leaf"]),
        ]);
        let out = extract_structure(&gds, "buf").unwrap();
        let s = parse(&out).unwrap();
        let names: Vec<&str> = s.structures.iter().map(|s| s.name.as_str()).collect();
        // Order of the source, the unreachable ones dropped.
        assert_eq!(names, ["inv", "buf", "leaf"]);
        assert_eq!(s.library, "LIB");
        assert!((s.user_per_dbu - 0.001).abs() < 1e-12);
        assert!((s.dbu_in_meters - 1e-9).abs() < 1e-18);
        assert!(s.ghosts().is_empty());
        assert_eq!(s.top_cells(), ["buf"]);
        // A leaf alone, and the whole stream when everything is reachable.
        let leaf = parse(&extract_structure(&gds, "leaf").unwrap()).unwrap();
        assert_eq!(leaf.structures.len(), 1);
        let single = build(&[("a", &[])]);
        assert_eq!(extract_structure(&single, "a").unwrap(), single);
        // Renaming the top afterwards is what the copy of a cell does.
        let map = BTreeMap::from([("buf".to_string(), "buf_copy".to_string())]);
        let renamed = rename_bytes(&out, &map).unwrap().unwrap();
        assert_eq!(parse(&renamed).unwrap().top_cells(), ["buf_copy"]);
    }

    #[test]
    fn extracting_a_missing_structure_fails() {
        let gds = build(&[("a", &[])]);
        let error = extract_structure(&gds, "zzz").unwrap_err().to_string();
        assert!(error.contains("zzz"), "{error}");
    }

    #[test]
    fn extraction_keeps_ghosts_as_ghosts() {
        let gds = build(&[("a", &["ext"]), ("b", &[])]);
        let s = parse(&extract_structure(&gds, "a").unwrap()).unwrap();
        assert_eq!(s.structures.len(), 1);
        assert_eq!(s.ghosts(), ["ext"]);
    }

    pub(crate) fn rec(out: &mut Vec<u8>, kind: u16, data: &[u8]) {
        out.extend_from_slice(&((data.len() + 4) as u16).to_be_bytes());
        out.extend_from_slice(&kind.to_be_bytes());
        out.extend_from_slice(data);
    }

    fn name(text: &str) -> Vec<u8> {
        let mut b = text.as_bytes().to_vec();
        if b.len() % 2 == 1 {
            b.push(0);
        }
        b
    }

    /// A stream with the given structures `(name, [referenced names])`; a polygon-ish
    /// record (BOUNDARY with XY) is added to each so skipping is exercised.
    pub(crate) fn build(structures: &[(&str, &[&str])]) -> Vec<u8> {
        const UNITS_DATA: [u8; 16] = [
            0x3E, 0x41, 0x89, 0x37, 0x4B, 0xC6, 0xA7, 0xF0, 0x39, 0x44, 0xB8, 0x2F, 0xA0, 0x9B,
            0x5A, 0x53,
        ];
        let dates = [0u8; 24];
        let mut out = Vec::new();
        rec(&mut out, 0x0002, &600i16.to_be_bytes());
        rec(&mut out, BGNLIB, &dates);
        rec(&mut out, LIBNAME, &name("LIB"));
        rec(&mut out, UNITS, &UNITS_DATA);
        for (cell, refs) in structures {
            rec(&mut out, BGNSTR, &dates);
            rec(&mut out, STRNAME, &name(cell));
            rec(&mut out, 0x0800, &[]); // BOUNDARY
            rec(&mut out, 0x0D02, &[0, 1]); // LAYER
            rec(&mut out, 0x0E02, &[0, 0]); // DATATYPE
            rec(&mut out, 0x1003, &[0u8; 40]); // XY
            rec(&mut out, 0x1100, &[]); // ENDEL
            for r in *refs {
                rec(&mut out, 0x0A00, &[]); // SREF
                rec(&mut out, SNAME, &name(r));
                rec(&mut out, 0x1003, &[0u8; 8]);
                rec(&mut out, 0x1100, &[]);
            }
            rec(&mut out, ENDSTR, &[]);
        }
        rec(&mut out, ENDLIB, &[]);
        out
    }

    #[test]
    fn placements_carry_their_property() {
        let dates = [0u8; 24];
        let mut out = Vec::new();
        rec(&mut out, 0x0002, &600i16.to_be_bytes());
        rec(&mut out, BGNLIB, &dates);
        rec(&mut out, LIBNAME, &name("LIB"));
        rec(&mut out, BGNSTR, &dates);
        rec(&mut out, STRNAME, &name("top"));
        for (cell, props) in [
            ("nmos", &[(61u16, "M1")][..]),
            ("pmos", &[(7, "x"), (61, "M2")][..]),
            ("inv", &[][..]),
        ] {
            rec(&mut out, 0x0A00, &[]); // SREF
            rec(&mut out, SNAME, &name(cell));
            rec(&mut out, 0x1003, &[0u8; 8]);
            for (attr, value) in props {
                rec(&mut out, 0x2B02, &attr.to_be_bytes());
                rec(&mut out, 0x2C06, &name(value));
            }
            rec(&mut out, 0x1100, &[]);
        }
        rec(&mut out, ENDSTR, &[]);
        rec(&mut out, ENDLIB, &[]);
        let found = placements(&out, 61).unwrap();
        let got: Vec<(&str, &str, Option<&str>)> = found
            .iter()
            .map(|p| (p.structure.as_str(), p.cell.as_str(), p.value.as_deref()))
            .collect();
        assert_eq!(
            got,
            [
                ("top", "nmos", Some("M1")),
                ("top", "pmos", Some("M2")),
                ("top", "inv", None)
            ]
        );
    }

    #[test]
    fn reads_structures_refs_and_units() {
        let gds = build(&[
            ("buf", &["inv", "inv", "res"]),
            ("local", &[]),
            ("odd", &["x"]),
        ]);
        let s = parse(&gds).unwrap();
        assert_eq!(s.library, "LIB");
        assert!((s.user_per_dbu - 0.001).abs() < 1e-12);
        assert!((s.dbu_in_meters - 1e-9).abs() < 1e-18);
        assert_eq!(s.structures.len(), 3);
        assert_eq!(s.structures[0].name, "buf");
        assert_eq!(s.structures[2].name, "odd");
        assert_eq!(s.ghosts(), ["inv", "res", "x"]);
        assert_eq!(s.top_cells(), ["buf", "local", "odd"]);
        assert!(s.defines("local"));
    }

    #[test]
    fn klayout_context_is_not_a_cell() {
        // As KLayout writes a placed library PCell: the context structure comes first
        // and places the proxy cell.
        let s = parse(&build(&[
            (CONTEXT_INFO, &["CIRCLE"]),
            ("CIRCLE", &[]),
            ("top", &["CIRCLE", "ghost"]),
        ]))
        .unwrap();
        assert_eq!(s.top_cells(), ["top"]);
        assert_eq!(s.ghosts(), ["ghost"]);
        assert!(!s.defines(CONTEXT_INFO));
    }

    #[test]
    fn local_cells_are_not_ghosts() {
        let s = parse(&build(&[("top", &["sub", "ext"]), ("sub", &[])])).unwrap();
        assert_eq!(s.ghosts(), ["ext"]);
        assert_eq!(s.top_cells(), ["top"]);
    }

    #[test]
    fn rejects_bad_streams() {
        assert!(parse(&[]).is_err());
        let gds = build(&[("a", &[])]);
        assert!(parse(&gds[..gds.len() - 4]).is_err());
        assert!(parse(&gds[..gds.len() - 1]).is_err());
        assert!(parse(b"\0\0\0\0").is_err());
        let mut bad = gds.clone();
        bad[0] = 0xff;
        assert!(parse(&bad).is_err());
    }

    #[test]
    fn tape_padding_after_endlib_is_fine() {
        let mut gds = build(&[("a", &[])]);
        gds.extend_from_slice(&[0; 100]);
        assert_eq!(parse(&gds).unwrap().structures.len(), 1);
    }

    #[test]
    fn compressed_files_are_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("a.gds.gz");
        fs::write(&file, [0x1f, 0x8b, 8, 0]).unwrap();
        let error = read_structure(&file).unwrap_err().to_string();
        assert!(error.contains("compressed"), "{error}");
    }

    #[test]
    fn renames_names_and_fixes_lengths() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("a.gds");
        fs::write(
            &file,
            build(&[("buf", &["inv", "res"]), ("sub", &["buf2"])]),
        )
        .unwrap();
        let map = BTreeMap::from([
            ("buf".to_string(), "buffer_x".to_string()),
            ("inv".to_string(), "i".to_string()),
        ]);
        rename_structures(&file, &map).unwrap();
        let s = read_structure(&file).unwrap();
        assert_eq!(s.structures[0].name, "buffer_x");
        assert_eq!(
            s.structures[0].refs.iter().collect::<Vec<_>>(),
            ["i", "res"]
        );
        assert_eq!(s.structures[1].name, "sub");
        assert!(s.structures[1].refs.contains("buf2"));
        // Still a well-formed stream (every record length matches).
        let bytes = fs::read(&file).unwrap();
        assert!(records(&bytes).all(|r| r.is_ok()));
        assert!(!tmp.path().join("a.gds.tmp").exists());
    }

    #[test]
    fn rename_without_match_leaves_file_alone() {
        let gds = build(&[("a", &[])]);
        let map = BTreeMap::from([("zzz".to_string(), "b".to_string())]);
        assert_eq!(rename_bytes(&gds, &map).unwrap(), None);
    }

    #[test]
    fn rename_refuses_duplicates() {
        let gds = build(&[("a", &[]), ("b", &[])]);
        let map = BTreeMap::from([("a".to_string(), "b".to_string())]);
        assert!(rename_bytes(&gds, &map).is_err());
    }
}
