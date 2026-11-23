//! DRC and LVS through KLayout (docs/KLAYOUT.md §8).
//!
//! The deck of the PDK runs in a KLayout of its own, in batch mode
//! (`klayout -b -r <deck> -rd input=… -rd topcell=… -rd report=…`), on a
//! self-contained stream of the layout (every referenced cell filled in, see
//! [`export_stream`]). KLayout is GPL: it is only ever started, never linked.
//!
//! - DRC: the report database (`.lyrdb`, XML) is read here ([`Rdb`]).
//! - LVS: the schematic is netlisted by xschem for LVS ([`schematic_netlist`]), the
//!   deck compares, and the LVS database (`.lvsdb`, KLayout's own text format) is
//!   summarised by the helper script `cdept_klayout/lvsdb.py`, run by KLayout too
//!   ([`LvsSummary`]).

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use eyre::{Result, WrapErr, bail, eyre};
use quick_xml::events::Event;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::engine::{Engine, EngineOptions};
use super::protocol::method;
use super::setup;
use crate::{Format, Project};

/// The helper script that summarises an LVS database, relative to the helper folder.
pub const LVSDB_SCRIPT: &str = "cdept_klayout/lvsdb.py";

/// Which check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Check {
    Drc,
    Lvs,
}

impl Check {
    /// `drc` or `lvs` (also the key in `[klayout]`).
    pub fn name(self) -> &'static str {
        match self {
            Check::Drc => "drc",
            Check::Lvs => "lvs",
        }
    }

    /// `DRC` or `LVS`, for people.
    pub fn label(self) -> &'static str {
        match self {
            Check::Drc => "DRC",
            Check::Lvs => "LVS",
        }
    }

    /// File suffixes of a deck: the plain script and KLayout's macro form.
    fn suffixes(self) -> [&'static str; 2] {
        match self {
            Check::Drc => ["drc", "lydrc"],
            Check::Lvs => ["lvs", "lylvs"],
        }
    }
}

/// The deck for `check`: `[klayout] drc`/`lvs`, else the one found in the
/// `KLAYOUT_PATH` folders ([`Project::klayout_path`]): `tech/<check>/*.<suffix>`,
/// `<check>/*.<suffix>` and `tech/*.<suffix>` in that order. Decks whose name has the
/// PDK's name in it come first; partial decks (`antenna`, `density`, `maximal`, …)
/// last.
pub fn find_deck(project: &Project, check: Check) -> Option<PathBuf> {
    let chosen = match check {
        Check::Drc => &project.klayout.drc,
        Check::Lvs => &project.klayout.lvs,
    };
    if let Some(deck) = chosen {
        return Some(deck.clone());
    }
    let pdk = project.pdk.var("PDK").unwrap_or_default().to_lowercase();
    for dir in project.klayout_path() {
        for sub in [
            dir.join("tech").join(check.name()),
            dir.join(check.name()),
            dir.join("tech"),
        ] {
            if let Some(deck) = best_deck(&sub, check, &pdk) {
                return Some(deck);
            }
        }
    }
    None
}

fn best_deck(dir: &Path, check: Check, pdk: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut decks: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| check.suffixes().contains(&e))
        })
        .collect();
    const PARTIAL: [&str; 6] = ["antenna", "density", "maximal", "fill", "options", "test"];
    let rank = |p: &PathBuf| {
        let stem = p
            .file_stem()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let partial = PARTIAL.iter().any(|w| stem.contains(w));
        let named = !pdk.is_empty()
            && pdk
                .split(['-', '_'])
                .filter(|w| w.len() > 2)
                .any(|w| stem.contains(w));
        // Plain scripts before macros, then by name.
        let macro_form = p
            .extension()
            .is_some_and(|e| e.to_string_lossy().starts_with("ly"));
        (partial, !named, macro_form, stem)
    };
    decks.sort_by_key(rank);
    decks.into_iter().next()
}

// -- report database (.lyrdb) ---------------------------------------------------

/// A DRC report: categories and the places that violate them, in µm, in the
/// coordinates of the top cell.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Rdb {
    pub description: String,
    pub top_cell: String,
    pub categories: Vec<Category>,
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Category {
    /// Names from the outermost category in.
    pub path: Vec<String>,
    pub description: String,
}

impl Category {
    /// `outer.inner`.
    pub fn name(&self) -> String {
        self.path.join(".")
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Item {
    /// The category, as [`Category::path`].
    pub category: Vec<String>,
    /// The cell the item was found in (the shapes are already in top coordinates).
    pub cell: String,
    pub shapes: Vec<Shape>,
    /// Values that are not shapes (numbers, texts).
    pub notes: Vec<String>,
}

impl Item {
    /// The box around the shapes.
    pub fn bbox(&self) -> Option<[f64; 4]> {
        self.shapes.iter().map(Shape::bbox).reduce(|a, b| {
            [
                a[0].min(b[0]),
                a[1].min(b[1]),
                a[2].max(b[2]),
                a[3].max(b[3]),
            ]
        })
    }
}

/// One shape of an item, in µm.
#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    /// The hull; holes are left out.
    Polygon(Vec<[f64; 2]>),
    Box([f64; 4]),
    Edge([[f64; 2]; 2]),
    Text([f64; 2], String),
}

impl Shape {
    /// `[left, bottom, right, top]`.
    pub fn bbox(&self) -> [f64; 4] {
        let points: &[[f64; 2]] = match self {
            Shape::Polygon(points) => points,
            Shape::Box(b) => return *b,
            Shape::Edge(points) => points,
            Shape::Text(at, _) => return [at[0], at[1], at[0], at[1]],
        };
        points.iter().fold(
            [f64::MAX, f64::MAX, f64::MIN, f64::MIN],
            |[l, b, r, t], [x, y]| [l.min(*x), b.min(*y), r.max(*x), t.max(*y)],
        )
    }

    /// The shape as the helper's `markers` method takes it.
    pub fn to_marker(&self) -> Value {
        match self {
            Shape::Polygon(points) => json!({"kind": "polygon", "points": points}),
            Shape::Box(b) => json!({"kind": "box", "box": b}),
            Shape::Edge(points) => json!({"kind": "edge", "points": points}),
            Shape::Text(at, text) => json!({"kind": "text", "at": at, "text": text}),
        }
    }

    fn transformed(&self, t: &Trans) -> Shape {
        match self {
            Shape::Polygon(points) => Shape::Polygon(points.iter().map(|p| t.apply(*p)).collect()),
            Shape::Box(b) => {
                let (p, q) = (t.apply([b[0], b[1]]), t.apply([b[2], b[3]]));
                Shape::Box([
                    p[0].min(q[0]),
                    p[1].min(q[1]),
                    p[0].max(q[0]),
                    p[1].max(q[1]),
                ])
            }
            Shape::Edge([p, q]) => Shape::Edge([t.apply(*p), t.apply(*q)]),
            Shape::Text(at, text) => Shape::Text(t.apply(*at), text.clone()),
        }
    }
}

impl Rdb {
    pub fn read(file: &Path) -> Result<Rdb> {
        let text = std::fs::read_to_string(file)
            .wrap_err_with(|| format!("cannot read {}", file.display()))?;
        Rdb::parse(&text).wrap_err_with(|| format!("{} is not a report database", file.display()))
    }

    /// Read the XML of a `.lyrdb` file.
    pub fn parse(xml: &str) -> Result<Rdb> {
        let raw = parse_xml(xml)?;
        let mut rdb = Rdb {
            description: raw.description,
            top_cell: raw.top_cell,
            categories: raw.categories,
            items: Vec::new(),
        };
        // Where each cell sits in the top cell: follow the first reference up.
        let cells: HashMap<String, RawCell> = raw.cells.into_iter().map(|c| (c.key(), c)).collect();
        let to_top = |cell: &str| -> Trans {
            let mut trans = Trans::IDENTITY;
            let mut at = cell.to_string();
            for _ in 0..64 {
                let Some((parent, t)) = cells.get(&at).and_then(|c| c.parent.clone()) else {
                    break;
                };
                trans = trans.then(&t);
                at = parent;
            }
            trans
        };
        for raw in raw.items {
            let trans = to_top(&raw.cell);
            let mut item = Item {
                category: split_path(&raw.category),
                cell: raw.cell.split(':').next().unwrap_or_default().to_string(),
                ..Default::default()
            };
            for value in &raw.values {
                match parse_value(value) {
                    Parsed::Shapes(shapes) => item
                        .shapes
                        .extend(shapes.iter().map(|s| s.transformed(&trans))),
                    Parsed::Note(note) => item.notes.push(note),
                }
            }
            rdb.items.push(item);
        }
        Ok(rdb)
    }

    /// The categories with the number of items in each (items of a category's
    /// subcategories not included), in the order of the report; categories without
    /// items left out.
    pub fn counts(&self) -> Vec<(&Category, usize)> {
        let mut counts: HashMap<&[String], usize> = HashMap::new();
        for item in &self.items {
            *counts.entry(item.category.as_slice()).or_default() += 1;
        }
        self.categories
            .iter()
            .filter_map(|c| counts.get(c.path.as_slice()).map(|n| (c, *n)))
            .collect()
    }

    /// The items of the category `path`.
    pub fn items_of<'a>(&'a self, path: &'a [String]) -> impl Iterator<Item = &'a Item> + 'a {
        self.items.iter().filter(move |i| i.category == path)
    }
}

/// What the XML holds, before cells are resolved.
#[derive(Default)]
struct RawRdb {
    description: String,
    top_cell: String,
    categories: Vec<Category>,
    cells: Vec<RawCell>,
    items: Vec<RawItem>,
}

#[derive(Default)]
struct RawCell {
    name: String,
    variant: String,
    /// The first reference: the parent (`name` or `name:variant`) and the placement.
    parent: Option<(String, Trans)>,
}

impl RawCell {
    fn key(&self) -> String {
        if self.variant.is_empty() {
            self.name.clone()
        } else {
            format!("{}:{}", self.name, self.variant)
        }
    }
}

#[derive(Default)]
struct RawItem {
    category: String,
    cell: String,
    values: Vec<String>,
}

fn parse_xml(xml: &str) -> Result<RawRdb> {
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut stack: Vec<String> = Vec::new();
    let mut text = String::new();
    let mut rdb = RawRdb::default();
    // Names of the categories being read, outermost first.
    let mut category_path: Vec<String> = Vec::new();
    let mut cell = RawCell::default();
    let mut reference: (String, String) = Default::default();
    let mut item = RawItem::default();
    let mut root_seen = false;
    loop {
        let event = reader
            .read_event()
            .map_err(|e| eyre!("XML error at {}: {e}", reader.buffer_position()))?;
        match event {
            Event::Start(e) => {
                let name = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                if stack.is_empty() {
                    if name != "report-database" {
                        bail!("the root element is <{name}>, not <report-database>");
                    }
                    root_seen = true;
                }
                let parent = stack.last().map(String::as_str);
                match (parent, name.as_str()) {
                    (Some("categories"), "category") => {
                        category_path.push(String::new());
                        rdb.categories.push(Category::default());
                    }
                    (Some("cells"), "cell") => cell = RawCell::default(),
                    (Some("references"), "ref") => reference = Default::default(),
                    (Some("items"), "item") => item = RawItem::default(),
                    _ => {}
                }
                stack.push(name);
                text.clear();
            }
            Event::Empty(e) => {
                let name = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                if stack.is_empty() {
                    bail!("the report database is empty");
                }
                stack.push(name);
                text.clear();
                end_element(
                    &stack,
                    "",
                    &mut rdb,
                    &mut category_path,
                    &mut cell,
                    &mut reference,
                    &mut item,
                );
                stack.pop();
            }
            Event::Text(e) => text.push_str(&e.decode().map_err(|e| eyre!("{e}"))?),
            Event::CData(e) => text.push_str(&e.decode().map_err(|e| eyre!("{e}"))?),
            Event::GeneralRef(e) => {
                if let Some(c) = e.resolve_char_ref().map_err(|e| eyre!("{e}"))? {
                    text.push(c);
                } else {
                    let name = e.decode().map_err(|e| eyre!("{e}"))?;
                    text.push_str(match name.as_ref() {
                        "lt" => "<",
                        "gt" => ">",
                        "amp" => "&",
                        "quot" => "\"",
                        "apos" => "'",
                        other => return Err(eyre!("unknown entity &{other};")),
                    });
                }
            }
            Event::End(_) => {
                let value = std::mem::take(&mut text);
                end_element(
                    &stack,
                    value.trim(),
                    &mut rdb,
                    &mut category_path,
                    &mut cell,
                    &mut reference,
                    &mut item,
                );
                stack.pop();
            }
            Event::Eof if stack.is_empty() => break,
            Event::Eof => {
                bail!("the report database ends inside <{}>", stack.join("><"));
            }
            _ => {}
        }
    }
    if !root_seen {
        bail!("no <report-database>");
    }
    Ok(rdb)
}

/// The element on top of `stack` ends with the text `value`.
#[allow(clippy::too_many_arguments)]
fn end_element(
    stack: &[String],
    value: &str,
    rdb: &mut RawRdb,
    category_path: &mut Vec<String>,
    cell: &mut RawCell,
    reference: &mut (String, String),
    item: &mut RawItem,
) {
    let n = stack.len();
    let name = stack[n - 1].as_str();
    let parent = if n >= 2 { stack[n - 2].as_str() } else { "" };
    match (parent, name) {
        ("report-database", "description") => rdb.description = value.to_string(),
        ("report-database", "top-cell") => rdb.top_cell = value.to_string(),
        ("category", "name") if n >= 3 && stack[n - 3] == "categories" => {
            if let Some(last) = category_path.last_mut() {
                *last = value.to_string();
            }
            // The category was pushed when it started; its subcategories come after
            // the name, so the name is known before they are.
            if let Some(c) = rdb.categories.last_mut()
                && c.path.is_empty()
            {
                c.path = category_path.clone();
            }
        }
        ("category", "description") if n >= 3 && stack[n - 3] == "categories" => {
            let path = category_path.clone();
            if let Some(c) = rdb.categories.iter_mut().rev().find(|c| c.path == path) {
                c.description = value.to_string();
            }
        }
        ("categories", "category") => {
            category_path.pop();
        }
        ("cell", "name") if n >= 3 && stack[n - 3] == "cells" => cell.name = value.to_string(),
        ("cell", "variant") if n >= 3 && stack[n - 3] == "cells" => {
            cell.variant = value.to_string()
        }
        ("ref", "parent") => reference.0 = value.to_string(),
        ("ref", "trans") => reference.1 = value.to_string(),
        ("references", "ref") => {
            if cell.parent.is_none()
                && !reference.0.is_empty()
                && let Some(t) = Trans::parse(&reference.1)
            {
                cell.parent = Some((reference.0.clone(), t));
            }
        }
        ("cells", "cell") => rdb.cells.push(std::mem::take(cell)),
        ("item", "category") => item.category = value.to_string(),
        ("item", "cell") => item.cell = value.to_string(),
        ("values", "value") if n >= 3 && stack[n - 3] == "item" => {
            item.values.push(value.to_string())
        }
        ("items", "item") => rdb.items.push(std::mem::take(item)),
        _ => {}
    }
}

/// `'a.b'.c` → `["a.b", "c"]`: names separated by dots, quoted when they hold one.
fn split_path(text: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut chars = text.trim().chars();
    while let Some(c) = chars.next() {
        match c {
            '\'' | '"' => {
                let quote = c;
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => current.extend(chars.next()),
                        c if c == quote => break,
                        c => current.push(c),
                    }
                }
            }
            '.' => parts.push(std::mem::take(&mut current)),
            c => current.push(c),
        }
    }
    parts.push(current);
    parts
}

enum Parsed {
    Shapes(Vec<Shape>),
    Note(String),
}

/// One `<value>`: `polygon: (x,y;…[/hole])`, `box: (l,b;r,t)`, `edge: (…)`,
/// `edge-pair: (…)/(…)`, `path: (…) w=…`, `label: ('text',r0 x,y)`; anything else
/// is a note.
fn parse_value(text: &str) -> Parsed {
    let mut text = text.trim();
    // A tagged value: `[tag] kind: …`.
    if text.starts_with('[')
        && let Some(end) = text.find(']')
    {
        text = text[end + 1..].trim_start();
    }
    let Some((kind, body)) = text.split_once(':') else {
        return Parsed::Note(text.to_string());
    };
    let body = body.trim();
    let shapes = match kind.trim() {
        "polygon" => outer(body)
            .split('/')
            .next()
            .map(points)
            .filter(|p| p.len() >= 3)
            .map(|p| vec![Shape::Polygon(p)]),
        "box" => match points(outer(body)).as_slice() {
            [p, q] => Some(vec![Shape::Box([
                p[0].min(q[0]),
                p[1].min(q[1]),
                p[0].max(q[0]),
                p[1].max(q[1]),
            ])]),
            _ => None,
        },
        "edge" => edge(outer(body)).map(|e| vec![e]),
        "edge-pair" => {
            let edges: Vec<Shape> = body
                .split(['/', '|'])
                .filter_map(|e| edge(outer(e)))
                .collect();
            (!edges.is_empty()).then_some(edges)
        }
        "path" => {
            let spine = points(outer(body.split(" w=").next().unwrap_or(body)));
            let edges: Vec<Shape> = spine
                .windows(2)
                .map(|w| Shape::Edge([w[0], w[1]]))
                .collect();
            (!edges.is_empty()).then_some(edges)
        }
        "label" => label(outer(body)).map(|t| vec![t]),
        _ => None,
    };
    match shapes {
        Some(shapes) => Parsed::Shapes(shapes),
        None => Parsed::Note(text.to_string()),
    }
}

/// `(…)` → `…` (the text itself when it is not in brackets).
fn outer(text: &str) -> &str {
    let text = text.trim();
    match (text.find('('), text.rfind(')')) {
        (Some(a), Some(b)) if a < b => &text[a + 1..b],
        _ => text,
    }
}

/// `x,y;x,y;…`.
fn points(text: &str) -> Vec<[f64; 2]> {
    text.split(';')
        .filter_map(|p| {
            let (x, y) = p.split_once(',')?;
            Some([x.trim().parse().ok()?, y.trim().parse().ok()?])
        })
        .collect()
}

fn edge(text: &str) -> Option<Shape> {
    match points(text).as_slice() {
        [p, q] => Some(Shape::Edge([*p, *q])),
        _ => None,
    }
}

/// `'text',r0 x,y`.
fn label(text: &str) -> Option<Shape> {
    let (string, place) = text.rsplit_once(',').and_then(|(head, y)| {
        let (string, rest) = head.rsplit_once(',')?;
        let x = rest.split_whitespace().last()?;
        Some((string, [x.parse().ok()?, y.trim().parse().ok()?]))
    })?;
    Some(Shape::Text(place, split_path(string).join(".")))
}

/// A placement: `p' = d + mag · R(angle) · (mirror ? (x, −y) : (x, y))`.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Trans {
    angle: f64,
    mirror: bool,
    mag: f64,
    d: [f64; 2],
}

impl Trans {
    const IDENTITY: Trans = Trans {
        angle: 0.0,
        mirror: false,
        mag: 1.0,
        d: [0.0, 0.0],
    };

    /// KLayout's notation: `r90 *2 1.5,0`, `m45 10,0` (`mθ`: mirrored at the axis at
    /// θ degrees, i.e. mirrored at x and turned by 2θ).
    fn parse(text: &str) -> Option<Trans> {
        let mut t = Trans::IDENTITY;
        for word in text.split_whitespace() {
            if let Some(rest) = word.strip_prefix('r') {
                t.angle = rest.parse().ok()?;
            } else if let Some(rest) = word.strip_prefix('m') {
                t.mirror = true;
                t.angle = 2.0 * rest.parse::<f64>().ok()?;
            } else if let Some(rest) = word.strip_prefix('*') {
                t.mag = rest.parse().ok()?;
            } else if let Some((x, y)) = word.split_once(',') {
                t.d = [x.parse().ok()?, y.parse().ok()?];
            } else {
                return None;
            }
        }
        Some(t)
    }

    fn apply(&self, [x, y]: [f64; 2]) -> [f64; 2] {
        let y = if self.mirror { -y } else { y };
        let (s, c) = self.angle.to_radians().sin_cos();
        let round = |v: f64| (v * 1e9).round() / 1e9;
        [
            round(self.d[0] + self.mag * (c * x - s * y)),
            round(self.d[1] + self.mag * (s * x + c * y)),
        ]
    }

    /// `self` first, then `outer`, as points of `outer`'s space are computed:
    /// `outer.apply(self.apply(p))`, kept as points (exact enough for markers).
    fn then(&self, outer: &Trans) -> Trans {
        // Compose through three points: origin and the two unit vectors.
        let o = outer.apply(self.apply([0.0, 0.0]));
        let ex = outer.apply(self.apply([1.0, 0.0]));
        let ey = outer.apply(self.apply([0.0, 1.0]));
        let ux = [ex[0] - o[0], ex[1] - o[1]];
        let uy = [ey[0] - o[0], ey[1] - o[1]];
        let mag = ux[0].hypot(ux[1]);
        let angle = ux[1].atan2(ux[0]).to_degrees();
        // Mirrored when the frame turns clockwise.
        let mirror = ux[0] * uy[1] - ux[1] * uy[0] < 0.0;
        Trans {
            angle,
            mirror,
            mag,
            d: o,
        }
    }
}

// -- running decks ----------------------------------------------------------------

/// A self-contained stream of a layout, as the decks read it.
#[derive(Debug, Clone, PartialEq)]
pub struct Stream {
    pub file: PathBuf,
    /// Its top cell.
    pub top: String,
    /// Referenced cells that could not be found (left empty in the stream).
    pub missing: Vec<String>,
}

/// Write the layout file `layout` of the cell `cell` with every reference filled in to
/// `out` (a `.gds`), through a KLayout helper of its own. The top cell of the stream is
/// [`expected_top`] of `layout` and `cell`.
pub fn export_stream(project: &Project, layout: &Path, cell: &str, out: &Path) -> Result<Stream> {
    let refs = crate::layout::refs::resolve(project, layout);
    let options = EngineOptions::for_project(project);
    let mut params = options.open_params(&refs, false);
    let mut engine = Engine::start(options)?;
    if let Value::Object(map) = &mut params {
        map.insert("out".into(), json!(absolute(out)?));
    }
    let value = engine.call(method::EXPORT, params);
    let _ = engine.shutdown();
    let value = value?;
    Ok(stream_of(out, &value, &expected_top(layout, cell)))
}

/// What the helper's answer to `export` (`{file, missing}`) says about `out`.
pub fn stream_of(out: &Path, value: &Value, fallback_top: &str) -> Stream {
    let missing = value
        .get("missing")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    Stream {
        file: out.to_path_buf(),
        top: stream_top(out, fallback_top),
        missing,
    }
}

/// The top cell a layout file is expected to have: its sidecar's `top`, else the file
/// name without suffixes.
pub fn fallback_top(layout: &Path) -> String {
    if let Ok(Some(meta)) = crate::layout::Meta::read_for(layout) {
        return meta.top;
    }
    let name = layout
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    name.split('.').next().unwrap_or_default().to_string()
}

/// The top cell a check of the cell `cell` whose layout is `layout` works on: the
/// sidecar's `top`, else `cell` when the GDS file defines a structure of that name (a
/// cell of a library whose one GDS holds many cells, as the PDK's standard cells do),
/// else [`fallback_top`].
pub fn expected_top(layout: &Path, cell: &str) -> String {
    if let Ok(Some(meta)) = crate::layout::Meta::read_for(layout) {
        return meta.top;
    }
    if !cell.is_empty() && crate::layout::gds::read_structure(layout).is_ok_and(|s| s.defines(cell))
    {
        return cell.to_string();
    }
    fallback_top(layout)
}

/// The top cell of the GDS file `file`: `expected` when it is one of its top cells,
/// else the only one; `expected` when that cannot be told.
pub fn stream_top(file: &Path, expected: &str) -> String {
    match crate::layout::gds::read_structure(file).map(|s| s.top_cells()) {
        Ok(tops) if tops.iter().any(|t| t == expected) => expected.to_string(),
        Ok(tops) if tops.len() == 1 => tops[0].clone(),
        _ => expected.to_string(),
    }
}

/// Where the files of a check of `library/cell` go:
/// `$XDG_CACHE_HOME/cdept/verify/<project>/<library>/<cell>/<check>` (or under
/// `~/.cache`), created.
pub fn work_dir(project: &Project, library: &str, cell: &str, check: Check) -> Result<PathBuf> {
    work_dir_named(project, library, cell, check.name())
}

/// [`work_dir`] for a job other than DRC and LVS (`nets`).
pub fn work_dir_named(project: &Project, library: &str, cell: &str, job: &str) -> Result<PathBuf> {
    let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty());
    let cache = var("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| var("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(std::env::temp_dir);
    let clean = |s: &str| -> String {
        let s: String = s
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || "-_.".contains(c) {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        if s.is_empty() || s.starts_with('.') {
            format!("_{s}")
        } else {
            s
        }
    };
    let dir = cache
        .join("cdept")
        .join("verify")
        .join(clean(&project.name))
        .join(clean(library))
        .join(clean(cell))
        .join(clean(job));
    std::fs::create_dir_all(&dir).wrap_err_with(|| format!("cannot create {}", dir.display()))?;
    Ok(dir)
}

/// The outcome of a DRC run.
#[derive(Debug, Clone)]
pub struct DrcRun {
    pub deck: PathBuf,
    pub report: PathBuf,
    /// KLayout's output.
    pub log: PathBuf,
    pub rdb: Rdb,
}

/// Run the DRC deck `deck` on `stream`; the files go into `dir`. `defines` are passed
/// as further `-rd name=value`.
pub fn run_drc(
    project: &Project,
    deck: &Path,
    stream: &Stream,
    dir: &Path,
    defines: &[(String, String)],
) -> Result<DrcRun> {
    let report = dir.join(format!("{}.lyrdb", stream.top));
    let log = dir.join("drc.log");
    remove_stale(&report)?;
    let mut vars = vec![
        ("input".to_string(), path_text(&absolute(&stream.file)?)),
        ("topcell".to_string(), stream.top.clone()),
        ("report".to_string(), path_text(&report)),
    ];
    vars.extend(defines.iter().cloned());
    run_batch(project, deck, &vars, dir, &log)?;
    if !report.is_file() {
        bail!(
            "the DRC deck wrote no report {} (does it take input, topcell and report?); see {}",
            report.display(),
            log.display()
        );
    }
    let rdb = Rdb::read(&report)?;
    Ok(DrcRun {
        deck: deck.to_path_buf(),
        report,
        log,
        rdb,
    })
}

/// What `lvsdb.py` reports.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LvsSummary {
    pub matched: bool,
    #[serde(default)]
    pub circuits: Vec<LvsCircuit>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LvsCircuit {
    /// Names in the layout and the schematic ("" when that side has none).
    #[serde(default)]
    pub layout: String,
    #[serde(default)]
    pub schematic: String,
    /// `Match`, `NoMatch`, `Mismatch`, `MatchWithWarning`, `Skipped`.
    pub status: String,
    /// Pairs that do not match (or match with a warning).
    #[serde(default)]
    pub nets: Vec<LvsPair>,
    #[serde(default)]
    pub devices: Vec<LvsPair>,
    #[serde(default)]
    pub pins: Vec<LvsPair>,
    #[serde(default)]
    pub subcircuits: Vec<LvsPair>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LvsPair {
    #[serde(default)]
    pub layout: String,
    #[serde(default)]
    pub schematic: String,
    pub status: String,
    /// The shapes of a layout net, in µm.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bbox: Option<[f64; 4]>,
}

impl LvsCircuit {
    /// Every listed pair with what it is (`net`, `device`, `pin`, `subcircuit`).
    pub fn pairs(&self) -> impl Iterator<Item = (&'static str, &LvsPair)> {
        let kinds = [
            ("net", &self.nets),
            ("device", &self.devices),
            ("pin", &self.pins),
            ("subcircuit", &self.subcircuits),
        ];
        kinds
            .into_iter()
            .flat_map(|(kind, list)| list.iter().map(move |p| (kind, p)))
    }
}

/// The outcome of an LVS run.
#[derive(Debug, Clone)]
pub struct LvsRun {
    pub deck: PathBuf,
    /// The LVS database.
    pub report: PathBuf,
    pub log: PathBuf,
    /// The netlist extracted from the layout.
    pub extracted: PathBuf,
    /// The netlist the user gave.
    pub schematic: PathBuf,
    /// The netlist the deck compared: `schematic`, or the copy with the PDK's device
    /// calls written as devices ([`super::reference`]).
    pub reference: PathBuf,
    /// What was done for the run, one sentence each: the rewrite of the reference
    /// netlist, deck variables the technology supplied.
    pub notes: Vec<String>,
    pub summary: LvsSummary,
}

/// Run the LVS deck `deck` comparing `stream` with the netlist `schematic`.
pub fn run_lvs(
    project: &Project,
    deck: &Path,
    stream: &Stream,
    schematic: &Path,
    dir: &Path,
    defines: &[(String, String)],
) -> Result<LvsRun> {
    let summary_file = dir.join("summary.json");
    remove_stale(&summary_file)?;
    let prepared = lvs_deck(project, deck, stream, schematic, dir, defines)?;
    let (report, extracted, log) = (prepared.report, prepared.extracted, prepared.log);
    let helper = setup::find_helper_dir()
        .ok_or_else(|| eyre!("the cdept-klayout helper scripts are not installed"))?;
    let script = helper.join(LVSDB_SCRIPT);
    let summary_log = dir.join("summary.log");
    let vars = [
        ("lvsdb".to_string(), path_text(&report)),
        ("out".to_string(), path_text(&summary_file)),
    ];
    run_batch(project, &script, &vars, dir, &summary_log)?;
    let text = std::fs::read_to_string(&summary_file).wrap_err_with(|| {
        format!(
            "the LVS database could not be summarised; see {}",
            summary_log.display()
        )
    })?;
    let summary: LvsSummary =
        serde_json::from_str(&text).wrap_err("the LVS summary is not understood")?;
    Ok(LvsRun {
        deck: deck.to_path_buf(),
        report,
        log,
        extracted,
        schematic: schematic.to_path_buf(),
        reference: prepared.reference,
        notes: prepared.notes,
        summary,
    })
}

/// Run the LVS deck and give its database, without reading it: for the net
/// comparison (`link::nets`), which needs the layout's side of it.
pub fn run_lvs_database(
    project: &Project,
    deck: &Path,
    stream: &Stream,
    schematic: &Path,
    dir: &Path,
) -> Result<PathBuf> {
    lvs_deck(project, deck, stream, schematic, dir, &[]).map(|run| run.report)
}

/// What [`lvs_deck`] made.
struct DeckRun {
    report: PathBuf,
    extracted: PathBuf,
    log: PathBuf,
    reference: PathBuf,
    notes: Vec<String>,
}

/// The netlist the deck gets: `schematic` with the PDK's device calls written as
/// devices, in `dir`, when it has any; else `schematic` itself.
pub fn reference_netlist(
    project: &Project,
    schematic: &Path,
    dir: &Path,
    top: &str,
) -> Result<(PathBuf, Vec<String>)> {
    let text = std::fs::read_to_string(schematic)
        .wrap_err_with(|| format!("cannot read {}", schematic.display()))?;
    let catalog = project.catalog().ok().flatten();
    let models = super::reference::DeviceModels::from_catalog(catalog.as_ref());
    let rewrite = super::reference::device_calls(&text, &models);
    let note = rewrite.note();
    let notes = if note.is_empty() { Vec::new() } else { vec![note] };
    if rewrite.converted == 0 {
        return Ok((schematic.to_path_buf(), notes));
    }
    let file = dir.join(format!("{top}_reference.spice"));
    std::fs::write(&file, &rewrite.text)
        .wrap_err_with(|| format!("cannot write {}", file.display()))?;
    Ok((file, notes))
}

/// The variables for the deck: those of the project (`lvs_variables` of `cdept.toml`,
/// else of the technology), then `defines`, which win by name.
fn deck_variables(project: &Project, defines: &[(String, String)]) -> Vec<(String, String)> {
    let mut vars: Vec<(String, String)> = project
        .klayout
        .lvs_variables
        .iter()
        .filter(|(name, _)| !defines.iter().any(|(d, _)| d == *name))
        .map(|(n, v)| (n.clone(), v.clone()))
        .collect();
    vars.extend(defines.iter().cloned());
    vars
}

/// The deck run of [`run_lvs`].
fn lvs_deck(
    project: &Project,
    deck: &Path,
    stream: &Stream,
    schematic: &Path,
    dir: &Path,
    defines: &[(String, String)],
) -> Result<DeckRun> {
    let report = dir.join(format!("{}.lvsdb", stream.top));
    let extracted = dir.join(format!("{}_extracted.cir", stream.top));
    let log = dir.join("lvs.log");
    for stale in [&report, &extracted] {
        remove_stale(stale)?;
    }
    let (reference, mut notes) = reference_netlist(project, schematic, dir, &stream.top)?;
    let mut vars = vec![
        ("input".to_string(), path_text(&absolute(&stream.file)?)),
        ("topcell".to_string(), stream.top.clone()),
        ("report".to_string(), path_text(&report)),
        ("schematic".to_string(), path_text(&absolute(&reference)?)),
        ("target_netlist".to_string(), path_text(&extracted)),
    ];
    let extra = deck_variables(project, defines);
    for (name, value) in &extra {
        if !defines.iter().any(|(d, _)| d == name) {
            notes.push(format!("deck variable {name}={value} from the technology"));
        }
    }
    vars.extend(extra);
    // A deck may exit with an error when the netlists differ (the sky130 one does):
    // what counts is the database it wrote.
    if let Err(error) = run_batch(project, deck, &vars, dir, &log)
        && !report.is_file()
    {
        return Err(error);
    }
    if !report.is_file() {
        bail!(
            "the LVS deck wrote no database {} (does it take input, topcell, report and \
             schematic?); see {}",
            report.display(),
            log.display()
        );
    }
    Ok(DeckRun {
        report,
        extracted,
        log,
        reference,
        notes,
    })
}

/// The schematic file of `library/cell`: the view `view`, else the first xschem
/// schematic.
pub fn schematic_file(
    project: &Project,
    library: &str,
    cell: &str,
    view: Option<&str>,
) -> Result<PathBuf> {
    let lib = project
        .library(library)
        .ok_or_else(|| eyre!("no library '{library}'"))?;
    let found = lib
        .cell(cell)
        .ok_or_else(|| eyre!("no cell '{cell}' in library '{library}'"))?;
    let entry = match view {
        Some(name) => found
            .entry(name)
            .ok_or_else(|| eyre!("cell '{cell}' has no view '{name}'"))?,
        None => found
            .entries
            .iter()
            .find(|e| e.format == Some(Format::XschemSchematic))
            .ok_or_else(|| eyre!("cell '{cell}' has no xschem schematic"))?,
    };
    entry
        .main_file
        .clone()
        .ok_or_else(|| eyre!("view '{}' of '{cell}' has no file", entry.name))
}

/// Netlist the schematic `file` for LVS (SPICE, xschem's `lvs_netlist`: the top cell
/// is a `.subckt` with its pins, devices use their `lvs_format`) into `out`.
pub fn schematic_netlist(project: &Project, file: &Path, out: &Path) -> Result<PathBuf> {
    let mut engine = crate::xschem::Engine::for_project(project)?;
    let outcome = (|| {
        engine.load(file)?;
        engine.tcl("set lvs_netlist 1")?;
        engine.netlist(crate::xschem::NetlistFormat::Spice, out)
    })();
    let _ = engine.shutdown();
    outcome?;
    Ok(out.to_path_buf())
}

/// `klayout -b -r <script> -rd name=value…` with the project's environment, in `dir`,
/// its output into `log`. An error when KLayout fails, with the end of its output.
pub fn run_batch(
    project: &Project,
    script: &Path,
    vars: &[(String, String)],
    dir: &Path,
    log: &Path,
) -> Result<()> {
    let klayout = setup::find_klayout().ok_or_else(|| {
        eyre!(
            "KLayout is not installed (build it with cdept-klayout/build.sh, or set {})",
            setup::KLAYOUT_ENV
        )
    })?;
    let mut command = Command::new(&klayout);
    command.arg("-b").arg("-r").arg(script);
    for (name, value) in vars {
        let mut pair = OsString::from(format!("{name}="));
        pair.push(value);
        command.arg("-rd").arg(pair);
    }
    let out =
        std::fs::File::create(log).wrap_err_with(|| format!("cannot write {}", log.display()))?;
    command
        .current_dir(dir)
        .envs(setup::helper_env(&klayout))
        .envs(setup::project_env(project))
        .stdin(Stdio::null())
        .stdout(out.try_clone()?)
        .stderr(out);
    tracing::debug!("running {} on {}", klayout.display(), script.display());
    let status = command
        .status()
        .wrap_err_with(|| format!("cannot start {}", klayout.display()))?;
    if !status.success() {
        let text = std::fs::read_to_string(log).unwrap_or_default();
        let tail: Vec<&str> = text.lines().rev().take(12).collect();
        let tail: Vec<&str> = tail.into_iter().rev().collect();
        bail!(
            "KLayout failed ({status}) running {}:\n{}\n(all of it in {})",
            script.display(),
            tail.join("\n"),
            log.display()
        );
    }
    Ok(())
}

fn remove_stale(file: &Path) -> Result<()> {
    if file.exists() {
        std::fs::remove_file(file)
            .wrap_err_with(|| format!("cannot replace {}", file.display()))?;
    }
    Ok(())
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// `path`, made absolute against the current folder.
pub fn absolute(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    Ok(std::env::current_dir()?.join(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NESTED: &str = include_str!("testdata/nested.lyrdb");

    /// A GDS stream with the (empty) structures `names`.
    fn gds_with(names: &[&str]) -> Vec<u8> {
        fn rec(out: &mut Vec<u8>, kind: u16, data: &[u8]) {
            out.extend(((data.len() + 4) as u16).to_be_bytes());
            out.extend(kind.to_be_bytes());
            out.extend(data);
        }
        fn name(s: &str) -> Vec<u8> {
            let mut v = s.as_bytes().to_vec();
            if v.len() % 2 == 1 {
                v.push(0);
            }
            v
        }
        let mut out = Vec::new();
        rec(&mut out, 0x0002, &600i16.to_be_bytes());
        rec(&mut out, 0x0102, &[0u8; 24]);
        rec(&mut out, 0x0206, &name("LIB"));
        for n in names {
            rec(&mut out, 0x0502, &[0u8; 24]);
            rec(&mut out, 0x0606, &name(n));
            rec(&mut out, 0x0700, &[]);
        }
        rec(&mut out, 0x0400, &[]);
        out
    }

    #[test]
    fn the_top_is_the_cell_checked_in_a_file_of_many_cells() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("stdcells.gds");
        std::fs::write(&file, gds_with(&["a21o_1", "inv_1", "nand2_1"])).unwrap();
        assert_eq!(expected_top(&file, "inv_1"), "inv_1");
        assert_eq!(expected_top(&file, "nand2_1"), "nand2_1");
        // A name the file does not define: the file's own name, as before.
        assert_eq!(expected_top(&file, "other"), "stdcells");
    }

    #[test]
    fn categories_and_items_of_a_nested_report() {
        let rdb = Rdb::parse(NESTED).unwrap();
        let names: Vec<String> = rdb.categories.iter().map(Category::name).collect();
        assert_eq!(names, ["a.b", "a.b.sub"]);
        assert_eq!(rdb.categories[1].path, ["a.b", "sub"]);
        assert_eq!(rdb.items.len(), 2);
        let sub = &rdb.items[0];
        assert_eq!(sub.category, ["a.b", "sub"]);
        assert_eq!(sub.cell, "SUB");
        // SUB:1 sits in TOP at "m45 *1 10,0": (x, y) → (10 + y, x).
        assert_eq!(sub.shapes, [Shape::Box([10.0, 0.0, 12.0, 1.0])]);
        let counts: Vec<(String, usize)> =
            rdb.counts().iter().map(|(c, n)| (c.name(), *n)).collect();
        assert_eq!(counts, [("a.b".to_string(), 1), ("a.b.sub".to_string(), 1)]);
    }

    #[test]
    fn every_kind_of_value() {
        let rdb = Rdb::parse(NESTED).unwrap();
        let item = &rdb.items[1];
        assert_eq!(item.cell, "TOP");
        assert_eq!(
            item.shapes[..2],
            [
                Shape::Edge([[0.0, 0.0], [1.0, 0.0]]),
                Shape::Edge([[0.0, 1.0], [1.0, 1.0]])
            ]
        );
        assert_eq!(item.shapes[2], Shape::Text([1.0, 1.0], "hi &<".into()));
        assert_eq!(item.shapes[3], Shape::Edge([[0.0, 0.0], [1.0, 0.0]])); // the path
        assert_eq!(
            item.shapes[4],
            Shape::Polygon(vec![[0.0, 0.0], [0.0, 4.0], [4.0, 4.0], [4.0, 0.0]])
        );
        assert_eq!(
            item.shapes.last(),
            Some(&Shape::Edge([[0.0, 0.0], [3.0, 3.0]]))
        );
        assert_eq!(item.notes, ["float: 3.5", "text: 'str\\'x'"]);
        assert_eq!(item.bbox(), Some([0.0, 0.0, 4.0, 4.0]));
    }

    #[test]
    fn a_report_of_the_pdk_deck() {
        // As written by the IHP SG13G2 deck (trimmed).
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<report-database>
 <description>Main DRC Run Report at</description>
 <top-cell>inv</top-cell>
 <categories>
  <category><name>LU.b</name><description>5.1 LU.b: latch-up</description><categories></categories></category>
  <category><name>M1.a</name><description>Min. Metal1 width = 0.16</description><categories/></category>
 </categories>
 <cells><cell><name>inv</name><variant/><layout-name/><references></references></cell></cells>
 <items>
  <item><tags/><category>'LU.b'</category><cell>inv</cell><visited>false</visited>
   <multiplicity>1</multiplicity><comment/><image/>
   <values><value>polygon: (0.3,2.6;0.3,3.8;1.7,3.8;1.7,2.6)</value></values></item>
  <item><category>'LU.b'</category><cell>inv</cell>
   <values><value>polygon: (0.3,0.4;0.3,1.4;1.7,1.4;1.7,0.4)</value></values></item>
 </items>
</report-database>"#;
        let rdb = Rdb::parse(xml).unwrap();
        assert_eq!(rdb.top_cell, "inv");
        assert_eq!(rdb.description, "Main DRC Run Report at");
        assert_eq!(rdb.categories[0].description, "5.1 LU.b: latch-up");
        assert_eq!(rdb.categories[1].name(), "M1.a");
        let counts: Vec<(String, usize)> =
            rdb.counts().iter().map(|(c, n)| (c.name(), *n)).collect();
        assert_eq!(counts, [("LU.b".to_string(), 2)]);
        let path = vec!["LU.b".to_string()];
        assert_eq!(rdb.items_of(&path).count(), 2);
        assert_eq!(rdb.items[0].bbox(), Some([0.3, 2.6, 1.7, 3.8]));
    }

    #[test]
    fn not_a_report() {
        assert!(Rdb::parse("<html></html>").is_err());
        assert!(Rdb::parse("<report-database><items>").is_err());
        assert!(Rdb::parse("").is_err());
    }

    #[test]
    fn placements() {
        let t = Trans::parse("r90 *2 1,2").unwrap();
        assert_eq!(t.apply([1.0, 0.0]), [1.0, 4.0]);
        let m = Trans::parse("m0 0,0").unwrap();
        assert_eq!(m.apply([1.0, 1.0]), [1.0, -1.0]);
        let m45 = Trans::parse("m45 *1 1,2").unwrap();
        assert_eq!(m45.apply([1.0, 0.0]), [1.0, 3.0]);
        let both = t.then(&m45);
        for p in [[0.0, 0.0], [1.0, 0.0], [0.5, -2.0]] {
            let want = m45.apply(t.apply(p));
            let got = both.apply(p);
            assert!((want[0] - got[0]).abs() < 1e-6 && (want[1] - got[1]).abs() < 1e-6);
        }
        assert!(Trans::parse("x 1,2").is_none());
    }

    #[test]
    fn category_paths() {
        assert_eq!(split_path("'a.b'.c"), ["a.b", "c"]);
        assert_eq!(split_path("plain"), ["plain"]);
        assert_eq!(split_path(r"'it\'s'"), ["it's"]);
    }

    #[test]
    fn decks_of_a_pdk() {
        let _env = crate::testutil::hermetic_env();
        let tmp = tempfile::tempdir().unwrap();
        let tech = tmp.path().join("pdks/ihp-sg13g2/libs.tech/klayout");
        let drc = tech.join("tech/drc");
        std::fs::create_dir_all(drc.join("rule_decks")).unwrap();
        for name in [
            "antenna.drc",
            "ihp-sg13g2.drc",
            "other.drc",
            "rule_decks/x.drc",
        ] {
            std::fs::write(drc.join(name), "").unwrap();
        }
        std::fs::create_dir_all(tech.join("tech/lvs")).unwrap();
        std::fs::write(tech.join("tech/lvs/sg13g2.lvs"), "").unwrap();
        let pdks = tmp.path().join("pdks");
        let project = Project::create_with(
            &tmp.path().join("p"),
            Some("p"),
            Some("ihp-sg13g2"),
            Some(pdks.to_str().unwrap()),
        )
        .unwrap();
        assert_eq!(
            find_deck(&project, Check::Drc),
            Some(drc.join("ihp-sg13g2.drc"))
        );
        assert_eq!(
            find_deck(&project, Check::Lvs),
            Some(tech.join("tech/lvs/sg13g2.lvs"))
        );
        let mut project = project;
        project.klayout.drc = Some(drc.join("other.drc"));
        assert_eq!(find_deck(&project, Check::Drc), Some(drc.join("other.drc")));
    }

    #[test]
    fn lvs_summary() {
        let text = r#"{"matched": false, "circuits": [{"layout": "inv", "schematic": "INV",
            "status": "NoMatch", "nets": [{"layout": "A", "schematic": "A",
            "status": "Mismatch", "bbox": [0, 0, 1, 2]}],
            "devices": [{"layout": "$2", "schematic": "X0", "status": "MatchWithWarning"}],
            "pins": [], "subcircuits": []}]}"#;
        let summary: LvsSummary = serde_json::from_str(text).unwrap();
        assert!(!summary.matched);
        let pairs: Vec<(&str, &str)> = summary.circuits[0]
            .pairs()
            .map(|(kind, p)| (kind, p.layout.as_str()))
            .collect();
        assert_eq!(pairs, [("net", "A"), ("device", "$2")]);
        assert_eq!(summary.circuits[0].nets[0].bbox, Some([0.0, 0.0, 1.0, 2.0]));
    }
}
