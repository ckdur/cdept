//! A small OASIS (SEMI P39) reader and rewriter: cell names, the cells each one
//! places, and renaming cells. Geometry is skipped, not decoded.
//!
//! Understood: `START`, `END`, `CELLNAME` (implicit and explicit numbers, the table may
//! come after its users), `CELL` by number or by name, both `PLACEMENT` records with
//! the modal placement cell, `PROPNAME` and `PROPERTY` (for `S_CELL_OFFSET` and
//! `S_TOP_CELL`), and `CBLOCK` (deflate). Every other record is parsed only as far as
//! needed to skip it.
//!
//! The result is the same [`GdsSummary`] the GDSII reader gives: a name that is placed
//! but has no `CELL` record is a ghost cell (KLayout writes ghost cells that way: a
//! `CELLNAME`, no `CELL`).
//!
//! Renaming writes the file again record by record. `CBLOCK`s are written out
//! uncompressed, the table offsets of `START`/`END` become "not given" (so a strict
//! file is no longer strict), `S_CELL_OFFSET` properties are recomputed, and the
//! validation signature of `END` (CRC32 or checksum) is recomputed.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::Path;

use eyre::{Context, Result, bail, eyre};

use super::gds::{GdsSummary, Structure};

/// First bytes of every OASIS file.
pub const MAGIC: &[u8] = b"%SEMI-OASIS\r\n";

const S_CELL_OFFSET: &[u8] = b"S_CELL_OFFSET";
const S_TOP_CELL: &[u8] = b"S_TOP_CELL";

/// Size of the `END` record, fixed by the standard.
const END_SIZE: usize = 256;

struct Rd<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Rd<'a> {
    fn new(b: &'a [u8]) -> Self {
        Rd { b, p: 0 }
    }

    fn done(&self) -> bool {
        self.p >= self.b.len()
    }

    fn byte(&mut self) -> Result<u8> {
        let v = *self
            .b
            .get(self.p)
            .ok_or_else(|| eyre!("truncated at byte {}", self.p))?;
        self.p += 1;
        Ok(v)
    }

    fn bytes(&mut self, n: u64) -> Result<&'a [u8]> {
        let n = usize::try_from(n).map_err(|_| eyre!("bad length {n}"))?;
        let end = self
            .p
            .checked_add(n)
            .filter(|&e| e <= self.b.len())
            .ok_or_else(|| eyre!("truncated at byte {}", self.p))?;
        let v = &self.b[self.p..end];
        self.p = end;
        Ok(v)
    }

    /// unsigned-integer: 7 bits per byte, low first; redundant zero bytes are allowed.
    fn uint(&mut self) -> Result<u64> {
        let start = self.p;
        let mut value = 0u64;
        let mut shift = 0u32;
        loop {
            let b = self.byte()?;
            let bits = u64::from(b & 0x7f);
            if shift < 64 {
                if shift > 57 && bits >> (64 - shift) != 0 {
                    bail!("integer too large at byte {start}");
                }
                value |= bits << shift;
            } else if bits != 0 {
                bail!("integer too large at byte {start}");
            }
            if b & 0x80 == 0 {
                return Ok(value);
            }
            shift += 7;
        }
    }

    fn real_body(&mut self, kind: u64) -> Result<f64> {
        Ok(match kind {
            0 => self.uint()? as f64,
            1 => -(self.uint()? as f64),
            2 => 1.0 / self.uint()? as f64,
            3 => -1.0 / self.uint()? as f64,
            4 => self.uint()? as f64 / self.uint()? as f64,
            5 => -(self.uint()? as f64 / self.uint()? as f64),
            6 => f64::from(f32::from_le_bytes(self.bytes(4)?.try_into()?)),
            7 => f64::from_le_bytes(self.bytes(8)?.try_into()?),
            _ => return Err(eyre!("bad real type {kind} at byte {}", self.p)),
        })
    }

    fn real(&mut self) -> Result<f64> {
        let kind = self.uint()?;
        self.real_body(kind)
    }

    fn string(&mut self) -> Result<&'a [u8]> {
        let n = self.uint()?;
        self.bytes(n)
    }

    fn skip(&mut self, n: usize) -> Result<()> {
        for _ in 0..n {
            self.uint()?;
        }
        Ok(())
    }

    fn g_delta(&mut self) -> Result<()> {
        if self.uint()? & 1 == 1 {
            self.uint()?;
        }
        Ok(())
    }

    fn count(&mut self, extra: u64) -> Result<u64> {
        let n = self.uint()?.saturating_add(extra);
        if n > self.b.len() as u64 {
            bail!("count {n} is larger than the data at byte {}", self.p);
        }
        Ok(n)
    }

    fn point_list(&mut self) -> Result<()> {
        let kind = self.uint()?;
        let n = self.count(0)?;
        for _ in 0..n {
            match kind {
                0..=3 => {
                    self.uint()?;
                }
                4 | 5 => self.g_delta()?,
                _ => return Err(eyre!("bad point list type {kind}")),
            }
        }
        Ok(())
    }

    fn repetition(&mut self) -> Result<()> {
        let kind = self.uint()?;
        match kind {
            0 => {}
            1 => self.skip(4)?,
            2 | 3 => self.skip(2)?,
            // Arbitrary spacings: dimension + 2 elements, so dimension + 1 spaces.
            4 | 6 => {
                let n = self.count(1)?;
                self.skip(n as usize)?;
            }
            5 | 7 => {
                let n = self.count(1)?;
                self.uint()?;
                self.skip(n as usize)?;
            }
            8 => {
                self.skip(2)?;
                self.g_delta()?;
                self.g_delta()?;
            }
            9 => {
                self.uint()?;
                self.g_delta()?;
            }
            10 | 11 => {
                let n = self.count(1)?;
                if kind == 11 {
                    self.uint()?;
                }
                for _ in 0..n {
                    self.g_delta()?;
                }
            }
            _ => return Err(eyre!("bad repetition type {kind}")),
        }
        Ok(())
    }

    fn interval(&mut self) -> Result<()> {
        match self.uint()? {
            0 => Ok(()),
            1..=3 => self.skip(1),
            4 => self.skip(2),
            k => Err(eyre!("bad interval type {k}")),
        }
    }

    /// `[x] [y] [repetition]` as flagged by `X`, `Y`, `R` bits of an info byte.
    fn xyr(&mut self, info: u8, x: u8, y: u8, r: u8) -> Result<()> {
        if info & x != 0 {
            self.uint()?;
        }
        if info & y != 0 {
            self.uint()?;
        }
        if info & r != 0 {
            self.repetition()?;
        }
        Ok(())
    }

    /// Layer and datatype as flagged by the low bits `D` (0x02) and `L` (0x01).
    fn layer(&mut self, info: u8) -> Result<()> {
        if info & 0x01 != 0 {
            self.uint()?;
        }
        if info & 0x02 != 0 {
            self.uint()?;
        }
        Ok(())
    }
}

/// A cell named by `CELLNAME` number or by string.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum CellRef {
    Num(u64),
    Name(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PropRef {
    Num(u64),
    Name(Vec<u8>),
}

#[derive(Debug)]
struct Value<'a> {
    kind: u64,
    /// The whole value, type included.
    raw: &'a [u8],
    uint: Option<u64>,
    string: Option<&'a [u8]>,
}

/// One record, with what the reader and the renamer need of it.
#[derive(Debug)]
enum Ev<'a> {
    Start {
        version: &'a [u8],
        /// The unit, as written (type included).
        unit_raw: &'a [u8],
        unit: f64,
        tables_in_end: bool,
    },
    End {
        scheme: u64,
    },
    CellName {
        name: &'a [u8],
        num: Option<u64>,
    },
    PropName {
        name: &'a [u8],
        num: Option<u64>,
    },
    PropString {
        string: &'a [u8],
        num: Option<u64>,
    },
    Cell(CellRef),
    Placement {
        /// Record id and info byte.
        head: &'a [u8],
        /// `None`: the modal placement cell.
        cell: Option<CellRef>,
        /// Everything after the cell reference.
        tail: &'a [u8],
    },
    Property {
        /// Record id, info byte, name reference and value count.
        head: &'a [u8],
        name: Option<PropRef>,
        /// `None`: the values of the previous property (V bit).
        values: Option<Vec<Value<'a>>>,
    },
    /// `PROPERTY` repeating the previous one (record 29).
    PropertyRepeat,
    Other,
}

/// Walk the records of `bytes`, calling `f` with each one and its bytes. `CBLOCK`s are
/// opened and their records passed on instead. Stops after `END` (top level).
fn walk<'a>(
    bytes: &'a [u8],
    in_cblock: bool,
    f: &mut dyn FnMut(Ev<'_>, &[u8]) -> Result<()>,
) -> Result<()> {
    let mut rd = Rd::new(bytes);
    if !in_cblock {
        if !bytes.starts_with(MAGIC) {
            bail!("not an OASIS file (no %SEMI-OASIS header)");
        }
        rd.p = MAGIC.len();
    }
    let mut started = in_cblock;
    let mut tables_in_end = false;
    loop {
        if rd.done() {
            if in_cblock {
                return Ok(());
            }
            bail!("no END record (truncated file?)");
        }
        let at = rd.p;
        let id = rd.uint()?;
        if !started && id != 1 && id != 0 {
            bail!("the first record is not START");
        }
        let ev = match id {
            0 => Ev::Other,
            1 => {
                if started {
                    bail!("second START record at byte {at}");
                }
                started = true;
                let version = rd.string()?;
                let u0 = rd.p;
                let unit = rd.real()?;
                let unit_raw = &bytes[u0..rd.p];
                let flag = rd.uint()?;
                if flag == 0 {
                    rd.skip(12)?;
                }
                tables_in_end = flag != 0;
                Ev::Start {
                    version,
                    unit_raw,
                    unit,
                    tables_in_end: flag != 0,
                }
            }
            2 => {
                if in_cblock {
                    bail!("END inside a CBLOCK");
                }
                if bytes.len() - at < END_SIZE {
                    bail!("END record is shorter than {END_SIZE} bytes");
                }
                let scheme = end_scheme(&bytes[at..at + END_SIZE], tables_in_end)?;
                f(Ev::End { scheme }, &bytes[at..at + END_SIZE])?;
                return Ok(());
            }
            3 | 4 => {
                let name = rd.string()?;
                let num = if id == 4 { Some(rd.uint()?) } else { None };
                Ev::CellName { name, num }
            }
            5 => {
                rd.string()?;
                Ev::Other
            }
            6 => {
                rd.string()?;
                rd.uint()?;
                Ev::Other
            }
            9 | 10 => {
                let string = rd.string()?;
                let num = if id == 10 { Some(rd.uint()?) } else { None };
                Ev::PropString { string, num }
            }
            7 | 8 => {
                let name = rd.string()?;
                let num = if id == 8 { Some(rd.uint()?) } else { None };
                Ev::PropName { name, num }
            }
            11 | 12 => {
                rd.string()?;
                rd.interval()?;
                rd.interval()?;
                Ev::Other
            }
            13 => Ev::Cell(CellRef::Num(rd.uint()?)),
            14 => Ev::Cell(CellRef::Name(rd.string()?.to_vec())),
            15 | 16 => Ev::Other,
            17 | 18 => {
                let info = rd.byte()?;
                let head = &bytes[at..rd.p];
                let cell = if info & 0x80 != 0 {
                    Some(if info & 0x40 != 0 {
                        CellRef::Num(rd.uint()?)
                    } else {
                        CellRef::Name(rd.string()?.to_vec())
                    })
                } else {
                    None
                };
                let t0 = rd.p;
                if id == 18 {
                    if info & 0x04 != 0 {
                        rd.real()?;
                    }
                    if info & 0x02 != 0 {
                        rd.real()?;
                    }
                }
                rd.xyr(info, 0x20, 0x10, 0x08)?;
                Ev::Placement {
                    head,
                    cell,
                    tail: &bytes[t0..rd.p],
                }
            }
            19 => {
                let info = rd.byte()?;
                if info & 0x40 != 0 {
                    if info & 0x20 != 0 {
                        rd.uint()?;
                    } else {
                        rd.string()?;
                    }
                }
                rd.layer(info)?;
                rd.xyr(info, 0x10, 0x08, 0x04)?;
                Ev::Other
            }
            20 => {
                let info = rd.byte()?;
                rd.layer(info)?;
                if info & 0x40 != 0 {
                    rd.uint()?;
                }
                if info & 0x20 != 0 {
                    rd.uint()?;
                }
                rd.xyr(info, 0x10, 0x08, 0x04)?;
                Ev::Other
            }
            21 => {
                let info = rd.byte()?;
                rd.layer(info)?;
                if info & 0x20 != 0 {
                    rd.point_list()?;
                }
                rd.xyr(info, 0x10, 0x08, 0x04)?;
                Ev::Other
            }
            22 => {
                let info = rd.byte()?;
                rd.layer(info)?;
                if info & 0x40 != 0 {
                    rd.uint()?;
                }
                if info & 0x80 != 0 {
                    let scheme = rd.uint()?;
                    if (scheme >> 2) & 3 == 3 {
                        rd.uint()?;
                    }
                    if scheme & 3 == 3 {
                        rd.uint()?;
                    }
                }
                if info & 0x20 != 0 {
                    rd.point_list()?;
                }
                rd.xyr(info, 0x10, 0x08, 0x04)?;
                Ev::Other
            }
            23..=25 => {
                let info = rd.byte()?;
                rd.layer(info)?;
                if info & 0x40 != 0 {
                    rd.uint()?;
                }
                if info & 0x20 != 0 {
                    rd.uint()?;
                }
                rd.skip(if id == 23 { 2 } else { 1 })?;
                rd.xyr(info, 0x10, 0x08, 0x04)?;
                Ev::Other
            }
            26 => {
                let info = rd.byte()?;
                rd.layer(info)?;
                for bit in [0x80, 0x40, 0x20] {
                    if info & bit != 0 {
                        rd.uint()?;
                    }
                }
                rd.xyr(info, 0x10, 0x08, 0x04)?;
                Ev::Other
            }
            27 => {
                let info = rd.byte()?;
                rd.layer(info)?;
                if info & 0x20 != 0 {
                    rd.uint()?;
                }
                rd.xyr(info, 0x10, 0x08, 0x04)?;
                Ev::Other
            }
            28 => {
                let info = rd.byte()?;
                let name = if info & 0x04 != 0 {
                    Some(if info & 0x02 != 0 {
                        PropRef::Num(rd.uint()?)
                    } else {
                        PropRef::Name(rd.string()?.to_vec())
                    })
                } else {
                    None
                };
                let values = if info & 0x08 == 0 {
                    let n = match info >> 4 {
                        15 => rd.count(0)?,
                        n => u64::from(n),
                    };
                    let head = &bytes[at..rd.p];
                    let mut values = Vec::new();
                    for _ in 0..n {
                        let v0 = rd.p;
                        let kind = rd.uint()?;
                        let (mut uint, mut string) = (None, None);
                        match kind {
                            0..=7 => {
                                rd.real_body(kind)?;
                            }
                            8 | 9 | 13..=15 => uint = Some(rd.uint()?),
                            10..=12 => string = Some(rd.string()?),
                            _ => return Err(eyre!("bad property value type {kind} at byte {v0}")),
                        }
                        values.push(Value {
                            kind,
                            raw: &bytes[v0..rd.p],
                            uint,
                            string,
                        });
                    }
                    let raw = &bytes[at..rd.p];
                    f(
                        Ev::Property {
                            head,
                            name,
                            values: Some(values),
                        },
                        raw,
                    )?;
                    continue;
                } else {
                    None
                };
                Ev::Property {
                    head: &bytes[at..rd.p],
                    name,
                    values,
                }
            }
            29 => Ev::PropertyRepeat,
            30 | 32 => {
                rd.uint()?;
                rd.string()?;
                Ev::Other
            }
            31 => {
                rd.uint()?;
                rd.string()?;
                rd.uint()?;
                Ev::Other
            }
            33 => {
                let info = rd.byte()?;
                rd.uint()?;
                rd.layer(info)?;
                rd.string()?;
                rd.xyr(info, 0x10, 0x08, 0x04)?;
                Ev::Other
            }
            34 => {
                if in_cblock {
                    bail!("CBLOCK inside a CBLOCK");
                }
                let method = rd.uint()?;
                let size = rd.uint()?;
                let packed = rd.string()?;
                if method != 0 {
                    bail!("CBLOCK compression type {method} is not supported");
                }
                let size = usize::try_from(size)?;
                let data = miniz_oxide::inflate::decompress_to_vec_with_limit(packed, size)
                    .map_err(|e| eyre!("cannot unpack the CBLOCK at byte {at}: {e:?}"))?;
                if data.len() != size {
                    bail!("the CBLOCK at byte {at} has the wrong size");
                }
                walk(&data, true, f).wrap_err_with(|| format!("in the CBLOCK at byte {at}"))?;
                continue;
            }
            _ => return Err(eyre!("unknown record type {id} at byte {at}")),
        };
        f(ev, &bytes[at..rd.p])?;
    }
}

/// The validation scheme of an `END` record (its 256 bytes): id, [tables], padding,
/// scheme, [signature].
fn end_scheme(end: &[u8], tables: bool) -> Result<u64> {
    let mut rd = Rd::new(end);
    rd.uint()?;
    if tables {
        rd.skip(12)?;
    }
    rd.string()?;
    let scheme = rd.uint()?;
    let sig = if matches!(scheme, 1 | 2) { 4 } else { 0 };
    if scheme > 2 || rd.p + sig != END_SIZE {
        bail!("cannot read the END record");
    }
    Ok(scheme)
}

/// Read the cells of an OASIS file and the cells each one places.
pub fn read_structure(path: &Path) -> Result<GdsSummary> {
    let bytes = load(path)?;
    parse(&bytes).wrap_err_with(|| format!("{} is not a valid OASIS file", path.display()))
}

fn load(path: &Path) -> Result<Vec<u8>> {
    let bytes = fs::read(path).wrap_err_with(|| format!("cannot read {}", path.display()))?;
    if bytes.starts_with(&[0x1f, 0x8b]) {
        bail!(
            "{} is compressed (gzip); uncompressed OASIS only",
            path.display()
        );
    }
    Ok(bytes)
}

fn utf8(name: &[u8]) -> String {
    String::from_utf8_lossy(name).into_owned()
}

/// The name tables of a file: `CELLNAME` and `PROPNAME` numbers, and the `PROPSTRING`
/// numbers that `S_TOP_CELL` values point at.
#[derive(Default)]
struct Tables {
    cells: HashMap<u64, Vec<u8>>,
    props: HashMap<u64, Vec<u8>>,
    top_strings: BTreeSet<u64>,
}

/// Hands out implicit numbers and checks a table does not mix them with explicit ones.
#[derive(Default)]
struct Numbers {
    next: u64,
    modes: BTreeSet<bool>,
}

impl Numbers {
    fn take(&mut self, explicit: Option<u64>) -> u64 {
        self.modes.insert(explicit.is_some());
        explicit.unwrap_or_else(|| {
            self.next += 1;
            self.next - 1
        })
    }

    fn check(&self, what: &str) -> Result<()> {
        if self.modes.len() > 1 {
            bail!("{what} records mix implicit and explicit numbers");
        }
        Ok(())
    }
}

impl Tables {
    fn read(bytes: &[u8]) -> Result<Tables> {
        let mut t = Tables::default();
        let (mut cells, mut props) = (Numbers::default(), Numbers::default());
        let mut top_refs = Vec::new();
        walk(bytes, false, &mut |ev, _| {
            match ev {
                Ev::CellName { name, num } => {
                    let n = cells.take(num);
                    if t.cells.insert(n, name.to_vec()).is_some() {
                        bail!("cell name number {n} is used twice");
                    }
                }
                Ev::PropName { name, num } => {
                    t.props.insert(props.take(num), name.to_vec());
                }
                Ev::Property {
                    name: Some(name),
                    values: Some(values),
                    ..
                } => {
                    for v in values.iter().filter(|v| matches!(v.kind, 13..=15)) {
                        top_refs.push((name.clone(), v.uint.unwrap_or(0)));
                    }
                }
                _ => {}
            }
            Ok(())
        })?;
        cells.check("CELLNAME")?;
        props.check("PROPNAME")?;
        for (name, n) in top_refs {
            if t.prop(&name) == Some(S_TOP_CELL) {
                t.top_strings.insert(n);
            }
        }
        Ok(t)
    }

    fn cell(&self, r: &CellRef) -> Result<Vec<u8>> {
        match r {
            CellRef::Name(n) => Ok(n.clone()),
            CellRef::Num(n) => self
                .cells
                .get(n)
                .cloned()
                .ok_or_else(|| eyre!("cell number {n} has no CELLNAME")),
        }
    }

    fn prop<'s>(&'s self, r: &'s PropRef) -> Option<&'s [u8]> {
        match r {
            PropRef::Name(n) => Some(n),
            PropRef::Num(n) => self.props.get(n).map(Vec::as_slice),
        }
    }
}

/// [`read_structure`] on bytes in memory.
pub fn parse(bytes: &[u8]) -> Result<GdsSummary> {
    let tables = Tables::read(bytes)?;
    let mut summary = GdsSummary::default();
    let mut cells: Vec<(CellRef, BTreeSet<CellRef>)> = Vec::new();
    let mut modal: Option<CellRef> = None;
    walk(bytes, false, &mut |ev, _| {
        match ev {
            Ev::Start { unit, .. } => {
                if !(unit.is_finite() && unit > 0.0) {
                    bail!("bad unit {unit}");
                }
                summary.user_per_dbu = 1.0 / unit;
                summary.dbu_in_meters = 1e-6 / unit;
            }
            Ev::Cell(r) => {
                modal = None;
                cells.push((r, BTreeSet::new()));
            }
            Ev::Placement { cell, .. } => {
                let Some((_, refs)) = cells.last_mut() else {
                    bail!("PLACEMENT outside a cell");
                };
                if let Some(c) = cell {
                    modal = Some(c);
                }
                let c = modal
                    .clone()
                    .ok_or_else(|| eyre!("PLACEMENT without a cell"))?;
                refs.insert(c);
            }
            _ => {}
        }
        Ok(())
    })?;
    let mut seen = BTreeSet::new();
    for (r, refs) in cells {
        let name = utf8(&tables.cell(&r)?);
        if !seen.insert(name.clone()) {
            bail!("cell '{name}' is defined twice");
        }
        let refs = refs
            .iter()
            .map(|r| tables.cell(r).map(|n| utf8(&n)))
            .collect::<Result<_>>()?;
        summary.structures.push(Structure { name, refs });
    }
    Ok(summary)
}

fn put_uint(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            return;
        }
        out.push(b | 0x80);
    }
}

fn put_string(out: &mut Vec<u8>, s: &[u8]) {
    put_uint(out, s.len() as u64);
    out.extend_from_slice(s);
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (i, slot) in table.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xedb8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
        *slot = c;
    }
    !bytes.iter().fold(!0u32, |c, &b| {
        table[((c ^ u32::from(b)) & 0xff) as usize] ^ (c >> 8)
    })
}

/// Rename cells: every `CELLNAME` and `CELL` name and every `PLACEMENT` by name equal
/// to a key of `map` gets the value, and so does an `S_TOP_CELL` value. The file is
/// replaced through a temporary file; it is not touched when nothing changes. Fails if
/// two cells would share a name.
pub fn rename_structures(path: &Path, map: &BTreeMap<String, String>) -> Result<()> {
    let bytes = load(path)?;
    if let Some(out) = rename_bytes(&bytes, map)
        .wrap_err_with(|| format!("cannot rename cells in {}", path.display()))?
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
    let map: HashMap<Vec<u8>, Vec<u8>> = map
        .iter()
        .map(|(k, v)| (k.as_bytes().to_vec(), v.as_bytes().to_vec()))
        .collect();
    if map.values().any(Vec::is_empty) {
        bail!("an empty cell name is not usable");
    }
    let tables = Tables::read(bytes)?;
    let new_name = |n: &[u8]| map.get(n).cloned().unwrap_or_else(|| n.to_vec());
    // Names after renaming must stay distinct: in the name table and among the cells
    // named by string.
    let mut names = BTreeSet::new();
    let mut changed = false;
    for name in tables.cells.values() {
        changed |= map.contains_key(name);
        if !names.insert(new_name(name)) {
            bail!("two cells would be named '{}'", utf8(&new_name(name)));
        }
    }
    let mut defined = BTreeSet::new();
    walk(bytes, false, &mut |ev, _| {
        match ev {
            Ev::Cell(r) => {
                let name = tables.cell(&r)?;
                changed |= map.contains_key(&name);
                if !defined.insert(new_name(&name)) {
                    bail!("two cells would be named '{}'", utf8(&new_name(&name)));
                }
            }
            Ev::Placement {
                cell: Some(CellRef::Name(n)),
                ..
            } => changed |= map.contains_key(&n),
            _ => {}
        }
        Ok(())
    })?;
    if !changed {
        return Ok(None);
    }
    // S_CELL_OFFSET values depend on where the cells end up: write until stable.
    let mut offsets: HashMap<Vec<u8>, u64> = HashMap::new();
    for _ in 0..8 {
        let (out, found) = emit(bytes, &tables, &new_name, &offsets)?;
        if found == offsets {
            return Ok(Some(out));
        }
        offsets = found;
    }
    Err(eyre!("the cell offsets do not settle"))
}

/// Check the `S_CELL_OFFSET` properties of a file without `CBLOCK`s (as
/// [`rename_bytes`] writes them): each nonzero one must give the byte offset of the
/// `CELL` record of its cell. Returns how many were checked.
pub fn verify_cell_offsets(bytes: &[u8]) -> Result<usize> {
    let tables = Tables::read(bytes)?;
    let mut cells = HashMap::new();
    let mut claims = Vec::new();
    let mut owner: Option<Vec<u8>> = None;
    let base = bytes.as_ptr() as usize;
    walk(bytes, false, &mut |ev, raw| {
        let at = raw.as_ptr() as usize - base;
        if at >= bytes.len() {
            bail!("a CBLOCK is present");
        }
        let mut keep = false;
        match ev {
            Ev::CellName { name, .. } => {
                owner = Some(name.to_vec());
                keep = true;
            }
            Ev::Cell(r) => {
                cells.insert(tables.cell(&r)?, at as u64);
            }
            Ev::Property {
                name: Some(name),
                values: Some(values),
                ..
            } => {
                keep = true;
                if tables.prop(&name) == Some(S_CELL_OFFSET)
                    && let (Some(cell), [v]) = (&owner, values.as_slice())
                    && let Some(offset) = v.uint.filter(|&u| u != 0)
                {
                    claims.push((cell.clone(), offset));
                }
            }
            Ev::Property { .. } | Ev::PropertyRepeat => keep = true,
            _ => {}
        }
        if !keep {
            owner = None;
        }
        Ok(())
    })?;
    for (cell, offset) in &claims {
        if cells.get(cell) != Some(offset) {
            bail!(
                "S_CELL_OFFSET of '{}' is {offset}, the CELL record is at {:?}",
                utf8(cell),
                cells.get(cell)
            );
        }
    }
    Ok(claims.len())
}

/// Where each `CELL` record is, by (new) cell name.
type Offsets = HashMap<Vec<u8>, u64>;

/// Write the file with cells renamed; `offsets` gives the `S_CELL_OFFSET` values (by new
/// name). Returns the bytes and where each `CELL` record went.
fn emit(
    bytes: &[u8],
    tables: &Tables,
    new_name: &dyn Fn(&[u8]) -> Vec<u8>,
    offsets: &HashMap<Vec<u8>, u64>,
) -> Result<(Vec<u8>, Offsets)> {
    let mut out = Vec::with_capacity(bytes.len() + 64);
    out.extend_from_slice(MAGIC);
    let mut found = HashMap::new();
    let mut tables_in_end = false;
    let mut strings = Numbers::default();
    // The cell a following PROPERTY belongs to (after a CELLNAME).
    let mut owner: Option<Vec<u8>> = None;
    walk(bytes, false, &mut |ev, raw| {
        let keeps_owner = matches!(
            ev,
            Ev::Property { .. } | Ev::PropertyRepeat | Ev::CellName { .. }
        );
        match ev {
            Ev::Start {
                version,
                unit_raw,
                tables_in_end: at_end,
                ..
            } => {
                tables_in_end = at_end;
                put_uint(&mut out, 1);
                put_string(&mut out, version);
                out.extend_from_slice(unit_raw);
                put_uint(&mut out, u64::from(at_end));
                if !at_end {
                    out.extend_from_slice(&[0; 12]);
                }
            }
            Ev::End { scheme } => {
                let start = out.len();
                put_uint(&mut out, 2);
                if tables_in_end {
                    out.extend_from_slice(&[0; 12]);
                }
                let sig = if matches!(scheme, 1 | 2) { 4 } else { 0 };
                let room = END_SIZE - (out.len() - start) - 1 - sig;
                let pad = if room - 2 >= 128 { room - 2 } else { room - 1 };
                put_string(&mut out, &vec![0; pad]);
                put_uint(&mut out, scheme);
                match scheme {
                    1 => {
                        let c = crc32(&out);
                        out.extend_from_slice(&c.to_le_bytes());
                    }
                    2 => {
                        let c = out.iter().fold(0u32, |s, &b| s.wrapping_add(u32::from(b)));
                        out.extend_from_slice(&c.to_le_bytes());
                    }
                    _ => {}
                }
                debug_assert_eq!(out.len() - start, END_SIZE);
            }
            Ev::CellName { name, num } => {
                let renamed = new_name(name);
                put_uint(&mut out, if num.is_some() { 4 } else { 3 });
                put_string(&mut out, &renamed);
                if let Some(num) = num {
                    put_uint(&mut out, num);
                }
                owner = Some(renamed);
            }
            Ev::PropString { string, num } => {
                if tables.top_strings.contains(&strings.take(num)) {
                    put_uint(&mut out, if num.is_some() { 10 } else { 9 });
                    put_string(&mut out, &new_name(string));
                    if let Some(num) = num {
                        put_uint(&mut out, num);
                    }
                } else {
                    out.extend_from_slice(raw);
                }
            }
            Ev::Cell(r) => {
                let name = new_name(&tables.cell(&r)?);
                found.insert(name.clone(), out.len() as u64);
                match r {
                    CellRef::Num(_) => out.extend_from_slice(raw),
                    CellRef::Name(_) => {
                        put_uint(&mut out, 14);
                        put_string(&mut out, &name);
                    }
                }
            }
            Ev::Placement {
                head,
                cell: Some(CellRef::Name(n)),
                tail,
            } => {
                out.extend_from_slice(head);
                put_string(&mut out, &new_name(&n));
                out.extend_from_slice(tail);
            }
            Ev::Property {
                head,
                name: Some(name),
                values: Some(values),
            } => {
                let pname = tables.prop(&name);
                out.extend_from_slice(head);
                for v in values {
                    if pname == Some(S_CELL_OFFSET)
                        && v.kind == 8
                        && v.uint.is_some_and(|u| u != 0)
                        && let Some(cell) = &owner
                    {
                        put_uint(&mut out, 8);
                        put_uint(&mut out, offsets.get(cell).copied().unwrap_or(0));
                    } else if pname == Some(S_TOP_CELL)
                        && let Some(s) = v.string
                    {
                        put_uint(&mut out, v.kind);
                        put_string(&mut out, &new_name(s));
                    } else {
                        out.extend_from_slice(v.raw);
                    }
                }
            }
            _ => out.extend_from_slice(raw),
        }
        if !keeps_owner {
            owner = None;
        }
        Ok(())
    })?;
    Ok((out, found))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Build a small OASIS file: `cells` as `(name, placed names)`; names placed but not
    /// listed get a `CELLNAME` only (ghosts). Placements alternate between by-number,
    /// by-name and modal (repeat) forms; with `cblock`, the cells are packed into a
    /// CBLOCK; with `explicit`, CELLNAME numbers are explicit and the table comes at the
    /// end.
    pub(crate) fn build(cells: &[(&str, &[&str])], cblock: bool, explicit: bool) -> Vec<u8> {
        let mut names: Vec<&str> = Vec::new();
        for (c, refs) in cells {
            for n in std::iter::once(c).chain(refs.iter()) {
                if !names.contains(n) {
                    names.push(n);
                }
            }
        }
        let num = |n: &str| names.iter().position(|m| *m == n).unwrap() as u64;
        let mut out = MAGIC.to_vec();
        put_uint(&mut out, 1);
        put_string(&mut out, b"1.0");
        out.extend_from_slice(&[0, 0xe8, 0x07]); // real type 0: 1000
        put_uint(&mut out, 0);
        out.extend_from_slice(&[0; 12]);
        let mut table = Vec::new();
        for (i, n) in names.iter().enumerate() {
            if explicit {
                put_uint(&mut table, 4);
                put_string(&mut table, n.as_bytes());
                put_uint(&mut table, i as u64);
            } else {
                put_uint(&mut table, 3);
                put_string(&mut table, n.as_bytes());
            }
        }
        // A PROPNAME and an S_TOP_CELL property after the first CELLNAME... kept simple:
        // a property on the file.
        let mut body = Vec::new();
        for (c, refs) in cells {
            put_uint(&mut body, 13);
            put_uint(&mut body, num(c));
            // A rectangle on 1/0.
            body.extend_from_slice(&[20, 0x7b, 1, 0, 10, 20, 0, 0]);
            for (i, r) in refs.iter().enumerate() {
                match i % 2 {
                    0 => {
                        body.extend_from_slice(&[17, 0xf0]);
                        put_uint(&mut body, num(r));
                        body.extend_from_slice(&[4, 6]);
                    }
                    _ => {
                        body.extend_from_slice(&[17, 0xb8]);
                        put_string(&mut body, r.as_bytes());
                        body.extend_from_slice(&[8, 0]);
                        // repetition type 2: 3 columns spaced 100
                        body.extend_from_slice(&[2, 1, 100]);
                    }
                }
                // Modal repeats of the same cell, one with arbitrary spacings (type 4:
                // 2 elements), one with arbitrary displacements (type 10: 3 elements).
                body.extend_from_slice(&[17, 0x38, 2, 2, 4, 0, 5]);
                body.extend_from_slice(&[17, 0x38, 2, 2, 10, 1, 4, 6]);
            }
            // A path and a polygon for the skipper.
            body.extend_from_slice(&[22, 0xfb, 1, 0, 5, 0x0f, 3, 4, 0, 2, 20, 21, 0, 0]);
            body.extend_from_slice(&[21, 0x3b, 1, 0, 0, 3, 2, 4, 6, 0, 0]);
            // A text by string.
            body.extend_from_slice(&[19, 0x5b, 1, b'A', 2, 0, 0, 0]);
        }
        if !explicit {
            out.extend_from_slice(&table);
        }
        if cblock {
            let packed = miniz_oxide::deflate::compress_to_vec(&body, 6);
            put_uint(&mut out, 34);
            put_uint(&mut out, 0);
            put_uint(&mut out, body.len() as u64);
            put_string(&mut out, &packed);
        } else {
            out.extend_from_slice(&body);
        }
        if explicit {
            out.extend_from_slice(&table);
        }
        let start = out.len();
        put_uint(&mut out, 2);
        let room = END_SIZE - 1 - 1;
        put_string(&mut out, &vec![0x80; room - 2]);
        put_uint(&mut out, 0);
        assert_eq!(out.len() - start, END_SIZE);
        out
    }

    fn names(s: &GdsSummary) -> Vec<(String, Vec<String>)> {
        s.structures
            .iter()
            .map(|c| (c.name.clone(), c.refs.iter().cloned().collect()))
            .collect()
    }

    #[test]
    fn reads_cells_placements_and_ghosts() {
        for cblock in [false, true] {
            for explicit in [false, true] {
                let bytes = build(
                    &[("leaf", &[]), ("buf", &["leaf", "inv"])],
                    cblock,
                    explicit,
                );
                let s = parse(&bytes).unwrap();
                assert_eq!(
                    names(&s),
                    [
                        ("leaf".to_string(), vec![]),
                        (
                            "buf".to_string(),
                            vec!["inv".to_string(), "leaf".to_string()]
                        )
                    ]
                );
                assert_eq!(s.ghosts(), ["inv"]);
                assert_eq!(s.top_cells(), ["buf"]);
                assert!((s.user_per_dbu - 0.001).abs() < 1e-12);
            }
        }
    }

    #[test]
    fn rejects_bad_files() {
        assert!(parse(b"not oasis").is_err());
        let bytes = build(&[("a", &[])], false, false);
        assert!(parse(&bytes[..bytes.len() - 10]).is_err());
        let mut bad = bytes.clone();
        // Cut away the END record.
        bad.truncate(bytes.len() - END_SIZE);
        assert!(parse(&bad).is_err());
    }

    #[test]
    fn renames_cells_everywhere() {
        for cblock in [false, true] {
            for explicit in [false, true] {
                let bytes = build(
                    &[("leaf", &[]), ("buf", &["leaf", "inv", "inv"])],
                    cblock,
                    explicit,
                );
                let map = BTreeMap::from([
                    ("inv".to_string(), "inverter_long_name".to_string()),
                    ("buf".to_string(), "b".to_string()),
                ]);
                let out = rename_bytes(&bytes, &map).unwrap().unwrap();
                let s = parse(&out).unwrap();
                assert_eq!(s.ghosts(), ["inverter_long_name"]);
                assert_eq!(s.top_cells(), ["b"]);
                assert!(out.len() >= END_SIZE);
                let nothing = BTreeMap::from([("x".to_string(), "y".to_string())]);
                assert_eq!(rename_bytes(&bytes, &nothing).unwrap(), None);
                let clash = BTreeMap::from([("inv".to_string(), "leaf".to_string())]);
                assert!(rename_bytes(&bytes, &clash).is_err());
            }
        }
    }

    #[test]
    fn validation_signatures_are_recomputed() {
        let bytes = build(&[("a", &["g"])], false, false);
        for scheme in [1u8, 2] {
            // Same END, now with a signature (4 bytes less padding).
            let mut file = bytes[..bytes.len() - END_SIZE].to_vec();
            let start = file.len();
            put_uint(&mut file, 2);
            put_string(&mut file, &vec![0; END_SIZE - 1 - 2 - 1 - 4]);
            file.push(scheme);
            file.extend_from_slice(&[0; 4]);
            assert_eq!(file.len() - start, END_SIZE);
            parse(&file).unwrap();
            let map = BTreeMap::from([("g".to_string(), "h".to_string())]);
            let out = rename_bytes(&file, &map).unwrap().unwrap();
            let (body, sig) = out.split_at(out.len() - 4);
            let expected = match scheme {
                1 => crc32(body),
                _ => body.iter().fold(0u32, |s, &b| s.wrapping_add(u32::from(b))),
            };
            assert_eq!(sig, expected.to_le_bytes());
            assert_eq!(parse(&out).unwrap().ghosts(), ["h"]);
        }
    }

    #[test]
    fn checksums() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
    }
}
