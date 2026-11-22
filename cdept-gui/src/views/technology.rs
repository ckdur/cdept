//! Technology tab: what CDePT knows about the project's PDK.
//!
//! It shows the technology snapshot of the project (`tech.toml` and
//! `tech.catalog.json`, see `libcdept::tech`): a summary, then what a designer looks
//! things up in (layers, decks, model corners, devices, the PDK's libraries) and, as
//! "Everything", both files as they are.
//!
//! The sections of the snapshot are read through their serialised form (JSON values),
//! not through the Rust types: a section that gains fields shows them without a change
//! here, and a section that is empty shows as empty.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use egui::{CollapsingHeader, Color32, Grid, RichText, ScrollArea, Sense, Stroke, Ui, vec2};
use libcdept::Format;
use libcdept::tech::{self, Catalog, Tech, TechLibrary, TechPath};
use serde_json::Value;

use super::{View, empty_state};
use crate::pdkscan;
use crate::state::{Action, AppCtx};

/// A value of the snapshot as JSON (`Null` if it cannot be serialised).
pub(crate) fn to_json<T: serde::Serialize>(value: &T) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

/// `a.b.c` in a JSON value; array elements by number.
pub(crate) fn lookup<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.')
        .filter(|key| !key.is_empty())
        .try_fold(value, |value, key| match value {
            Value::Object(map) => map.get(key),
            Value::Array(items) => items.get(key.parse::<usize>().ok()?),
            _ => None,
        })
}

/// A scalar as text; lists of scalars joined; anything else summarised.
pub(crate) fn text_of(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Bool(b) => if *b { "yes" } else { "no" }.into(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        Value::Array(items) if items.iter().all(is_scalar) => {
            items.iter().map(text_of).collect::<Vec<_>>().join(", ")
        }
        // A list of named things: their names.
        Value::Array(items) if items.iter().all(|item| item.get("name").is_some()) => items
            .iter()
            .filter_map(|item| item.get("name"))
            .map(text_of)
            .collect::<Vec<_>>()
            .join(", "),
        Value::Array(items) => format!("{} entries", items.len()),
        Value::Object(map) => {
            // A reference to something: its name, with `?` when it is a guess.
            let name = match (map.get("name"), map.get("library"), map.get("cell")) {
                (Some(name), _, _) => Some(text_of(name)),
                (None, Some(library), Some(cell)) => {
                    Some(format!("{}/{}", text_of(library), text_of(cell)))
                }
                _ => None,
            };
            match name {
                Some(name) if map.get("guess") == Some(&Value::Bool(true)) => format!("{name} ?"),
                Some(name) => name,
                // Few plain entries: `key=value`; else how many there are.
                None if map.len() <= 4 && map.values().all(is_scalar) => map
                    .iter()
                    .map(|(key, value)| format!("{key}={}", text_of(value)))
                    .collect::<Vec<_>>()
                    .join(", "),
                None => format!("{} entries", map.len()),
            }
        }
    }
}

/// The model sets of the snapshot (`[models.<set>]`, or `[models.sets.<set>]`): every
/// table in the models section that has corners.
pub(crate) fn model_sets(tech: &Value) -> Vec<(&String, &Value)> {
    let Some(Value::Object(models)) = tech.get("models") else {
        return Vec::new();
    };
    let nested = match models.get("sets") {
        Some(Value::Object(sets)) => Some(sets),
        _ => None,
    };
    models
        .iter()
        .chain(nested.into_iter().flatten())
        .filter(|(_, set)| set.get("corners").is_some())
        .collect()
}

fn is_scalar(value: &Value) -> bool {
    !matches!(value, Value::Array(_) | Value::Object(_))
}

fn is_empty(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(s) => s.is_empty(),
        Value::Array(items) => items.is_empty(),
        Value::Object(map) => map.values().all(is_empty),
        _ => false,
    }
}

/// Text that names a file or folder of the PDK.
fn looks_like_path(text: &str) -> bool {
    (text.starts_with("$PDK_ROOT") || text.starts_with('/') || text.starts_with("~/"))
        && !text.contains('\n')
}

/// The names of the objects of an array (`name` of each), joined.
fn names(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("name").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default()
}

fn count(value: Option<&Value>) -> usize {
    match value {
        Some(Value::Array(items)) => items.len(),
        Some(Value::Object(map)) => map.len(),
        _ => 0,
    }
}

/// What a scan found, in a few lines: for the wizard's summary, the top of the
/// Technology tab and test scripts. Lines of things that were not found are left out,
/// except the ones a designer expects (KLayout technology, xschem start-up file).
pub(crate) fn summary(tech: &Tech, catalog: Option<&Catalog>) -> Vec<(String, String)> {
    let t = to_json(tech);
    let c = catalog.map(to_json).unwrap_or(Value::Null);
    let text = |path: &str| lookup(&t, path).map(text_of).unwrap_or_default();
    let mut lines: Vec<(String, String)> = Vec::new();
    let mut add = |label: &str, value: String| {
        if !value.is_empty() {
            lines.push((label.to_string(), value));
        }
    };
    let or_none = |value: String| {
        if value.is_empty() {
            "not found".to_string()
        } else {
            value
        }
    };

    let version = match (&tech.pdk.version, &tech.pdk.commit) {
        (Some(v), Some(c)) => format!("{v} ({})", pdkscan::short(c)),
        (Some(v), None) => v.clone(),
        (None, Some(c)) => pdkscan::short(c).to_string(),
        (None, None) => String::new(),
    };
    add(
        "PDK",
        [tech.pdk.name.clone(), version]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(", "),
    );
    add("Description", text("pdk.description"));

    let mut klayout = or_none(text("klayout.tech"));
    if let Some(dbu) = tech.klayout.dbu {
        klayout.push_str(&format!(", database unit {dbu} µm"));
    }
    add("KLayout technology", klayout);
    let layers = count(lookup(&c, "klayout.layers"));
    if layers > 0 {
        add("Layers", layers.to_string());
    }
    add("DRC decks", names(lookup(&t, "klayout.drc")));
    add("LVS decks", names(lookup(&t, "klayout.lvs")));
    let libraries = lookup(&c, "klayout.pcell_libraries")
        .and_then(Value::as_array)
        .map(|libs| {
            libs.iter()
                .map(|lib| {
                    format!(
                        "{} ({} PCells)",
                        lib.get("name").map(text_of).unwrap_or_default(),
                        count(lib.get("pcells"))
                    )
                })
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    add("PCell libraries", libraries);

    add("xschem start-up file", or_none(text("xschem.rc")));
    let symbols = count(lookup(&c, "xschem.symbols"));
    if symbols > 0 {
        add("Symbols", symbols.to_string());
    }

    for (name, set) in model_sets(&t) {
        let corners = match set.get("corners") {
            Some(Value::Object(corners)) => corners.keys().cloned().collect::<Vec<_>>().join(", "),
            Some(other) => text_of(other),
            None => String::new(),
        };
        let corners = if corners.is_empty() {
            "none found".to_string()
        } else {
            corners
        };
        add(&format!("Model corners ({name})"), corners);
    }
    add("Magic technology", text("magic.tech"));
    add("Magic start-up file", text("magic.rc"));
    add("Netgen set-up", text("netgen.setup"));

    let devices = [lookup(&t, "devices"), lookup(&c, "devices")]
        .into_iter()
        .flatten()
        .filter_map(|section| section.get("devices"))
        .map(|list| count(Some(list)))
        .max()
        .unwrap_or(0);
    if devices > 0 {
        add("Devices", devices.to_string());
    }

    let all = tech.libraries();
    if !all.is_empty() {
        let listed: Vec<&str> = all
            .iter()
            .filter(|lib| lib.enabled)
            .map(|lib| lib.name.as_str())
            .collect();
        add(
            "Libraries",
            format!("{} of {}: {}", listed.len(), all.len(), listed.join(", ")),
        );
    }
    if !tech.notes.is_empty() {
        add("Notes", format!("{} (see Notes)", tech.notes.len()));
    }
    lines
}

/// List or leave out a library of the PDK in the snapshot. xschem's libraries are
/// set directly; a reference library is found in its section by name and path,
/// whatever shape the section has. Returns whether something changed.
pub(crate) fn set_library_enabled(tech: &mut Tech, library: &TechLibrary, on: bool) -> bool {
    for lib in &mut tech.xschem.libraries {
        if lib.name == library.name && lib.path == library.path {
            let changed = lib.enabled != on;
            lib.enabled = on;
            return changed;
        }
    }
    fn walk(value: &mut Value, library: &TechLibrary, on: bool) -> bool {
        match value {
            Value::Object(map) => {
                let is_it = map.get("name").and_then(Value::as_str) == Some(&library.name)
                    && map.get("path").and_then(Value::as_str) == Some(library.path.as_str())
                    && matches!(map.get("enabled"), Some(Value::Bool(_)));
                if is_it {
                    let changed = map.get("enabled") != Some(&Value::Bool(on));
                    map.insert("enabled".into(), Value::Bool(on));
                    return changed;
                }
                map.values_mut().any(|child| walk(child, library, on))
            }
            Value::Array(items) => items.iter_mut().any(|child| walk(child, library, on)),
            _ => false,
        }
    }
    let mut refs = to_json(&tech.refs);
    if !walk(&mut refs, library, on) {
        return false;
    }
    match serde_json::from_value(refs) {
        Ok(section) => {
            tech.refs = section;
            // The section may work the flag out again; say whether it took.
            tech.refs.libraries().iter().any(|lib| {
                lib.name == library.name && lib.path == library.path && lib.enabled == on
            })
        }
        Err(_) => false,
    }
}

/// Turns a path of the snapshot into the file on this machine, when it is there.
pub(crate) type Resolve<'a> = &'a dyn Fn(&str) -> Option<PathBuf>;

/// What a click in the tab asks for.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Ask {
    /// Open this file in a tab of CDePT.
    OpenInTab(PathBuf),
    Rescan {
        helpers: bool,
    },
    /// List or leave out a library of the PDK.
    Library(TechLibrary, bool),
    /// Put another installed version of the PDK in use (ciel), then rescan.
    Enable {
        root: PathBuf,
        family: String,
        version: String,
    },
}

/// Open a file or folder with the desktop's program for it.
fn open_outside(path: &Path) {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    if let Err(e) = std::process::Command::new(program).arg(path).spawn() {
        tracing::warn!(target: "cdept::tech", "cannot open {}: {e}", path.display());
    }
}

/// CDePT opens these in a tab of its own.
fn opens_in_tab(path: &Path) -> bool {
    matches!(
        Format::of_file(path),
        Some(Format::XschemSchematic | Format::XschemSymbol | Format::Gds | Format::Oasis)
    )
}

/// A path of the snapshot: a link when the file is on this machine (click opens it,
/// right-click offers more), plain weak text when it is not.
pub(crate) fn path_label(ui: &mut Ui, text: &str, resolve: Resolve, asks: &mut Vec<Ask>) {
    let Some(file) = resolve(text) else {
        ui.weak(text)
            .on_hover_text("Not found on this machine (is the PDK folder set?)");
        return;
    };
    let response = ui.link(text).on_hover_text(format!(
        "{}\nClick to open, right-click for more",
        file.display()
    ));
    let open = |file: &Path, asks: &mut Vec<Ask>| {
        if opens_in_tab(file) {
            asks.push(Ask::OpenInTab(file.to_path_buf()));
        } else {
            open_outside(file);
        }
    };
    if response.clicked() {
        open(&file, asks);
    }
    response.context_menu(|ui| {
        if ui.button("Open").clicked() {
            open(&file, asks);
            ui.close();
        }
        if ui.button("Open Folder").clicked() {
            open_outside(if file.is_dir() {
                &file
            } else {
                file.parent().unwrap_or(&file)
            });
            ui.close();
        }
        if ui.button("Copy Path").clicked() {
            ui.ctx().copy_text(file.display().to_string());
            ui.close();
        }
    });
}

/// A scalar of the snapshot: a path as a link, anything else as text.
fn value_label(ui: &mut Ui, value: &Value, resolve: Resolve, asks: &mut Vec<Ask>) {
    let text = text_of(value);
    if matches!(value, Value::String(_)) && looks_like_path(&text) {
        path_label(ui, &text, resolve, asks);
    } else if text.len() > 120 {
        let short: String = text.chars().take(117).collect();
        ui.label(format!("{short}…")).on_hover_text(text);
    } else {
        ui.label(text);
    }
}

/// The most rows a table draws: more are cut with a note (filter to narrow).
const MAX_ROWS: usize = 1500;
/// The most columns a table gets from the keys of its objects.
const MAX_COLUMNS: usize = 9;

/// Does any scalar of this value contain the filter (lower case)?
fn matches_filter(value: &Value, filter: &str) -> bool {
    if filter.is_empty() {
        return true;
    }
    match value {
        Value::Object(map) => map.values().any(|v| matches_filter(v, filter)),
        Value::Array(items) => items.iter().any(|v| matches_filter(v, filter)),
        other => text_of(other).to_lowercase().contains(filter),
    }
}

/// The columns of a table of objects: the preferred keys that occur, then the other
/// keys with plain values, then the keys with lists and tables.
fn columns(rows: &[&Value], preferred: &[&str]) -> Vec<String> {
    let mut scalar: Vec<String> = Vec::new();
    let mut nested: Vec<String> = Vec::new();
    for row in rows.iter().take(200) {
        let Value::Object(map) = row else { continue };
        for (key, value) in map {
            if is_empty(value) {
                continue;
            }
            let list = if is_scalar(value) {
                &mut scalar
            } else {
                &mut nested
            };
            if !list.contains(key) {
                list.push(key.clone());
            }
        }
    }
    // A key that is plain here and a list there shows as a list.
    scalar.retain(|key| !nested.contains(key));
    let occurs = |key: &str| scalar.iter().chain(&nested).any(|k| k == key);
    let mut ordered: Vec<String> = preferred
        .iter()
        .filter(|key| occurs(key))
        .map(|key| key.to_string())
        .collect();
    for key in scalar {
        if !ordered.contains(&key) && ordered.len() < MAX_COLUMNS {
            ordered.push(key);
        }
    }
    for key in nested {
        if !ordered.contains(&key) {
            ordered.push(key);
        }
    }
    ordered
}

/// A list of objects as a table: one row each, a column per key.
pub(crate) fn object_table(
    ui: &mut Ui,
    id: &str,
    rows: &[&Value],
    preferred: &[&str],
    resolve: Resolve,
    asks: &mut Vec<Ask>,
) {
    let keys = columns(rows, preferred);
    if keys.is_empty() {
        ui.weak("Nothing to show.");
        return;
    }
    Grid::new(id)
        .striped(true)
        .num_columns(keys.len())
        .spacing([14.0, 3.0])
        .show(ui, |ui| {
            for key in &keys {
                ui.label(RichText::new(key.replace('_', " ")).strong());
            }
            ui.end_row();
            for row in rows.iter().take(MAX_ROWS) {
                for key in &keys {
                    match row.get(key) {
                        Some(value) if is_empty(value) => {
                            ui.label("");
                        }
                        Some(value) if is_scalar(value) => value_label(ui, value, resolve, asks),
                        // A list or a table: its names in the cell, all of it on hover.
                        Some(value) => {
                            let short = text_of(value);
                            let shown: String = short.chars().take(48).collect();
                            ui.label(if shown.len() < short.len() {
                                format!("{shown}…")
                            } else {
                                shown
                            })
                            .on_hover_ui(|ui| {
                                let full = serde_json::to_string_pretty(value).unwrap_or_default();
                                let cut: String = full.chars().take(4000).collect();
                                ui.label(RichText::new(cut).monospace().small());
                            });
                        }
                        None => {
                            ui.label("");
                        }
                    }
                }
                ui.end_row();
            }
        });
    if rows.len() > MAX_ROWS {
        ui.weak(format!(
            "Showing {MAX_ROWS} of {}: type in the filter to narrow.",
            rows.len()
        ));
    }
}

/// Any value of the snapshot, as it is: tables fold, lists of tables become tables.
pub(crate) fn value_tree(
    ui: &mut Ui,
    id: &str,
    value: &Value,
    filter: &str,
    resolve: Resolve,
    asks: &mut Vec<Ask>,
) {
    match value {
        Value::Object(map) => {
            let (scalars, nested): (Vec<_>, Vec<_>) = map
                .iter()
                .filter(|(_, v)| !is_empty(v))
                .partition(|(_, v)| is_scalar(v) || is_scalar_list(v));
            if !scalars.is_empty() {
                Grid::new(format!("{id}/scalars"))
                    .num_columns(2)
                    .spacing([12.0, 3.0])
                    .show(ui, |ui| {
                        for (key, v) in scalars {
                            ui.label(RichText::new(key.replace('_', " ")).weak());
                            match v {
                                Value::Array(items) => {
                                    ui.vertical(|ui| {
                                        for item in items {
                                            value_label(ui, item, resolve, asks);
                                        }
                                    });
                                }
                                _ => value_label(ui, v, resolve, asks),
                            }
                            ui.end_row();
                        }
                    });
            }
            for (key, v) in nested {
                if !matches_filter(v, filter) {
                    continue;
                }
                let title = match v {
                    Value::Array(items) => format!("{} ({})", key.replace('_', " "), items.len()),
                    _ => key.replace('_', " "),
                };
                CollapsingHeader::new(title)
                    .id_salt(format!("{id}/{key}"))
                    .show(ui, |ui| {
                        value_tree(ui, &format!("{id}/{key}"), v, filter, resolve, asks)
                    });
            }
        }
        Value::Array(items) if items.iter().all(Value::is_object) => {
            let rows: Vec<&Value> = items
                .iter()
                .filter(|item| matches_filter(item, filter))
                .collect();
            object_table(ui, id, &rows, &["name"], resolve, asks);
        }
        Value::Array(items) => {
            for (n, item) in items.iter().enumerate().take(MAX_ROWS) {
                if is_scalar(item) {
                    value_label(ui, item, resolve, asks);
                } else {
                    value_tree(ui, &format!("{id}/{n}"), item, filter, resolve, asks);
                }
            }
        }
        other => value_label(ui, other, resolve, asks),
    }
}

fn is_scalar_list(value: &Value) -> bool {
    matches!(value, Value::Array(items) if items.len() <= 12 && items.iter().all(is_scalar))
}

/// `#rrggbb` (or `0xrrggbb`, `rrggbb`) as a colour.
fn colour(text: &str) -> Option<Color32> {
    let hex = text.trim().trim_start_matches('#').trim_start_matches("0x");
    let hex = if hex.len() == 8 { &hex[2..] } else { hex };
    if hex.len() != 6 {
        return None;
    }
    let n = u32::from_str_radix(hex, 16).ok()?;
    Some(Color32::from_rgb((n >> 16) as u8, (n >> 8) as u8, n as u8))
}

/// The first of these keys a layer has as a colour.
fn layer_colour(layer: &Value, keys: &[&str]) -> Option<Color32> {
    keys.iter()
        .find_map(|key| layer.get(key).and_then(Value::as_str).and_then(colour))
}

/// The custom stipples of the layer properties file, by their number: each a tile
/// of rows of `*` (set) and `.` (clear).
type Stipples = std::collections::HashMap<i64, Vec<Vec<bool>>>;

fn stipples(catalog: &Value) -> Stipples {
    lookup(catalog, "klayout.dither_patterns")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|pattern| {
            let order = pattern.get("order")?.as_i64()?;
            let rows: Vec<Vec<bool>> = pattern
                .get("lines")?
                .as_array()?
                .iter()
                .filter_map(Value::as_str)
                .map(|line| line.chars().map(|c| c == '*').collect())
                .collect();
            (!rows.is_empty() && rows.iter().all(|row| !row.is_empty())).then_some((order, rows))
        })
        .collect()
}

/// Size of a layer's box in points, and of its picture in pixels.
const SWATCH: [usize; 2] = [28, 14];

/// The pictures of the layer boxes drawn so far: one per stipple and fill colour.
type Swatches = std::collections::HashMap<(String, Color32), egui::TextureHandle>;

/// Which pixels of a layer's box its stipple sets. KLayout names a stipple `I<n>`
/// (built in: 0 solid, 1 hollow, then dots and lines) or `C<n>` (the layer
/// properties file's own, drawn as it is).
fn stipple_bits(stipple: &str, stipples: &Stipples) -> Vec<bool> {
    let [w, h] = SWATCH;
    let number = stipple
        .trim_start_matches(['I', 'C'])
        .parse::<i64>()
        .unwrap_or(0);
    let custom = stipple
        .starts_with('C')
        .then(|| stipples.get(&number))
        .flatten();
    let set = |x: usize, y: usize| match (custom, stipple) {
        // The file's rows go top down; a tile repeats in both directions.
        (Some(tile), _) => {
            let row = &tile[y % tile.len()];
            row[x % row.len()]
        }
        (None, "" | "I0") => true,
        (None, "I1") => false,
        // The other built-in ones: dots for the low numbers, a hatch for the rest.
        (None, _) if (2..=4).contains(&number) => {
            let step = number as usize;
            y.is_multiple_of(step) && (x + (y / step % 2) * (step / 2).max(1)).is_multiple_of(step)
        }
        (None, _) => (x + y).is_multiple_of(4),
    };
    (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .map(|(x, y)| set(x, y))
        .collect()
}

/// A layer's box as the layout shows it: the frame colour around, the fill colour
/// inside in the layer's stipple.
fn swatch(ui: &mut Ui, layer: &Value, stipples: &Stipples, pictures: &mut Swatches) {
    let size = vec2(SWATCH[0] as f32, SWATCH[1] as f32);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let fill = layer_colour(layer, &["fill_color", "fill", "color", "colour"]);
    let frame = layer_colour(layer, &["frame_color", "frame"]).or(fill);
    let Some(frame) = frame else {
        return;
    };
    let stipple = ["dither_pattern", "stipple", "pattern"]
        .iter()
        .find_map(|key| layer.get(key))
        .map(text_of)
        .unwrap_or_default();
    if let Some(fill) = fill {
        let picture = pictures.entry((stipple.clone(), fill)).or_insert_with(|| {
            let pixels = stipple_bits(&stipple, stipples)
                .into_iter()
                .map(|set| if set { fill } else { Color32::TRANSPARENT })
                .collect();
            ui.ctx().load_texture(
                format!("cdept-layer-{stipple}-{fill:?}"),
                egui::ColorImage::new(SWATCH, pixels),
                egui::TextureOptions::NEAREST,
            )
        });
        ui.painter().image(
            picture.id(),
            rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }
    ui.painter().rect_stroke(
        rect.shrink(0.5),
        0.0,
        Stroke::new(1.0, frame),
        egui::StrokeKind::Inside,
    );
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Page {
    #[default]
    Overview,
    Layers,
    Verification,
    Models,
    Devices,
    Libraries,
    Notes,
    Everything,
}

impl Page {
    const ALL: [Page; 8] = [
        Page::Overview,
        Page::Layers,
        Page::Verification,
        Page::Models,
        Page::Devices,
        Page::Libraries,
        Page::Notes,
        Page::Everything,
    ];

    fn label(self) -> &'static str {
        match self {
            Page::Overview => "Overview",
            Page::Layers => "Layers",
            Page::Verification => "Layout tools",
            Page::Models => "Models",
            Page::Devices => "Devices",
            Page::Libraries => "Libraries",
            Page::Notes => "Notes",
            Page::Everything => "Everything",
        }
    }
}

/// The snapshot of a project as the tab keeps it between frames.
struct Loaded {
    root: PathBuf,
    generation: u64,
    stamp: Option<SystemTime>,
    tech: Tech,
    tech_json: Value,
    catalog: Option<Catalog>,
    catalog_json: Value,
    summary: Vec<(String, String)>,
    /// Why the snapshot should be made again, if so.
    stale: Option<String>,
    /// The stipples of the layers.
    stipples: Stipples,
    /// The PDK folder (`$PDK_ROOT`) and the versions of the PDK's family ciel has
    /// installed there; empty when the PDK is not a ciel install.
    versions: (PathBuf, Vec<tech::ciel::Version>),
    /// Versions LibreLane pins for this project that are not the ones in use.
    pins: Vec<String>,
}

#[derive(Default)]
pub struct TechnologyView {
    page: Page,
    filter: String,
    loaded: Option<Loaded>,
    /// Scan with the helper programs (KLayout, xschem), not only by reading files.
    files_only: bool,
    /// The pictures of the layer boxes, made as they come into view.
    swatches: Swatches,
}

fn stamp(root: &Path) -> Option<SystemTime> {
    std::fs::metadata(tech::file::tech_file(root))
        .and_then(|m| m.modified())
        .ok()
}

fn when(time: SystemTime) -> String {
    chrono::DateTime::<chrono::Local>::from(time)
        .format("%Y-%m-%d %H:%M")
        .to_string()
}

impl TechnologyView {
    /// Read the snapshot again when the project, its technology or the file changed.
    fn refresh(&mut self, ctx: &AppCtx) {
        let Some(project) = &ctx.project else {
            self.loaded = None;
            return;
        };
        let Some(tech) = &project.tech else {
            self.loaded = None;
            return;
        };
        let now = stamp(&project.root);
        if let Some(loaded) = &self.loaded
            && loaded.root == project.root
            && loaded.generation == ctx.tech.generation
            && loaded.stamp == now
            && loaded.tech == *tech
        {
            return;
        }
        let catalog = match project.catalog() {
            Ok(catalog) => catalog,
            Err(e) => {
                tracing::warn!(target: "cdept::tech", "{e:#}");
                None
            }
        };
        // The versions ciel keeps of this PDK's family, when it installed the PDK.
        let pdk_dir = project.pdk.dir();
        let versions = pdk_dir
            .as_ref()
            .and_then(|pdk| {
                let real = std::fs::canonicalize(pdk.dir()).ok()?;
                let family = tech::ciel::version_of(&real)?.family;
                Some((
                    pdk.root.clone(),
                    tech::ciel::installed(&pdk.root, Some(&family)),
                ))
            })
            .unwrap_or_default();
        let pins = pdk_dir
            .iter()
            .flat_map(|pdk| tech::ciel::pin_mismatches(&project.root, &pdk.root))
            .map(|wrong| wrong.to_string())
            .collect();
        self.swatches.clear();
        self.loaded = Some(Loaded {
            versions,
            pins,
            root: project.root.clone(),
            generation: ctx.tech.generation,
            stamp: now,
            tech_json: to_json(tech),
            stipples: catalog
                .as_ref()
                .map(|catalog| stipples(&to_json(catalog)))
                .unwrap_or_default(),
            catalog_json: catalog.as_ref().map(to_json).unwrap_or(Value::Null),
            summary: summary(tech, catalog.as_ref()),
            stale: project.snapshot_stale(),
            tech: tech.clone(),
            catalog,
        });
    }

    fn head(&mut self, ui: &mut Ui, ctx: &AppCtx, asks: &mut Vec<Ask>) {
        let Some(project) = &ctx.project else { return };
        ui.horizontal_wrapped(|ui| {
            match ctx.pdk() {
                Some(pdk) => ui.heading(pdk),
                None => ui.heading("No PDK"),
            };
            if let Some(loaded) = &self.loaded {
                let pdk = &loaded.tech.pdk;
                if let Some(text) = &pdk.description {
                    ui.label(text);
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            if let Some(root) = ctx.pdk_root() {
                ui.weak("PDK folder");
                ui.label(root);
            }
            if let Some(loaded) = &self.loaded {
                let pdk = &loaded.tech.pdk;
                for (label, value) in [
                    ("version", &pdk.version),
                    ("branch", &pdk.branch),
                    ("commit", &pdk.commit),
                    ("installed by", &pdk.source),
                ] {
                    if let Some(value) = value {
                        ui.weak(label);
                        ui.label(pdkscan::short(value));
                    }
                }
                if let Some(time) = loaded.stamp {
                    ui.weak("scanned");
                    ui.label(when(time));
                }
            }
        });
        if let Some(loaded) = &self.loaded
            && let Some(why) = &loaded.stale
        {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                format!("The snapshot is out of date: {why}. Rescan to make it again."),
            );
        }
        ui.horizontal(|ui| {
            let running = ctx.tech.rescanning().is_some();
            let can = project.pdk.dir().is_some();
            if ui
                .add_enabled(!running && can, egui::Button::new("⟳ Rescan"))
                .on_hover_text(
                    "Read the PDK's files again and write tech.toml and tech.catalog.json",
                )
                .on_disabled_hover_text(if running {
                    "A scan is running"
                } else {
                    "The project has no PDK: choose one in Project > Project Settings…"
                })
                .clicked()
            {
                asks.push(Ask::Rescan {
                    helpers: !self.files_only,
                });
            }
            ui.checkbox(&mut self.files_only, "Files only")
                .on_hover_text(
                    "Do not start KLayout and xschem for the scan: faster, but PCells and what \
                     the xschemrc sets are not found",
                );
            if running {
                ui.spinner();
                ui.weak("scanning…");
            }
            if self.loaded.is_some() {
                ui.separator();
                let file = tech::file::tech_file(&project.root);
                if ui
                    .button("Open tech.toml")
                    .on_hover_text(file.display().to_string())
                    .clicked()
                {
                    open_outside(&file);
                }
            }
        });
        if let Some(job) = ctx.tech.rescanning() {
            pdkscan::progress_ui(ui, job);
        }
        if let Some(Err(e)) = &ctx.tech.last {
            ui.colored_label(ui.visuals().error_fg_color, e);
        }
    }

    fn layers(&mut self, ui: &mut Ui, loaded: &Loaded) {
        let Some(layers) = lookup(&loaded.catalog_json, "klayout.layers").and_then(Value::as_array)
        else {
            empty_state(
                ui,
                "No layers in the snapshot.",
                Some("The PDK's layer properties file (.lyp) was not found or not read."),
            );
            return;
        };
        let filter = self.filter.to_lowercase();
        let rows: Vec<&Value> = layers
            .iter()
            .filter(|layer| matches_filter(layer, &filter))
            .collect();
        ui.weak(format!("{} of {} layers", rows.len(), layers.len()));
        let has = |key: &str| {
            rows.iter()
                .any(|l| l.get(key).is_some_and(|v| !is_empty(v)))
        };
        // What else a designer looks for, where the layers say it.
        let extra: Vec<&str> = ["purpose", "source", "valid", "visible"]
            .into_iter()
            .filter(|key| has(key))
            .collect();
        let stipple = ["dither_pattern", "stipple", "pattern"]
            .into_iter()
            .find(|key| has(key));
        Grid::new("tech-layers")
            .striped(true)
            .spacing([14.0, 3.0])
            .show(ui, |ui| {
                ui.label("");
                for title in ["name", "layer", "datatype"] {
                    ui.label(RichText::new(title).strong());
                }
                if stipple.is_some() {
                    ui.label(RichText::new("stipple").strong());
                }
                for key in &extra {
                    ui.label(RichText::new(key.replace('_', " ")).strong());
                }
                ui.end_row();
                for layer in rows.iter().take(MAX_ROWS) {
                    swatch(ui, layer, &loaded.stipples, &mut self.swatches);
                    for key in ["name", "layer", "datatype"] {
                        ui.label(layer.get(key).map(text_of).unwrap_or_default());
                    }
                    if let Some(key) = stipple {
                        ui.label(layer.get(key).map(text_of).unwrap_or_default());
                    }
                    for key in &extra {
                        ui.label(layer.get(key).map(text_of).unwrap_or_default());
                    }
                    ui.end_row();
                }
            });
    }

    /// The corners of each model set as a table: a row per corner, a column per model
    /// library, the section of that library the corner takes.
    fn corners(ui: &mut Ui, tech: &Value) {
        let default_set = lookup(tech, "models.default")
            .map(text_of)
            .unwrap_or_default();
        let default_corner = lookup(tech, "models.corner")
            .map(text_of)
            .unwrap_or_default();
        for (name, set) in model_sets(tech) {
            let Some(Value::Object(corners)) = set.get("corners") else {
                continue;
            };
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.strong(format!("Corners of {name}"));
                if *name == default_set {
                    ui.weak("(the default set)");
                }
            });
            // An entry reads "<library file> <section>"; the files are the columns.
            let split = |entry: &Value| -> (String, String) {
                let text = text_of(entry);
                match text.split_once(char::is_whitespace) {
                    Some((file, section)) => (file.to_string(), section.trim().to_string()),
                    None => (String::new(), text),
                }
            };
            let mut files: Vec<String> = Vec::new();
            for entries in corners.values() {
                for entry in entries.as_array().into_iter().flatten() {
                    let (file, _) = split(entry);
                    if !files.contains(&file) {
                        files.push(file);
                    }
                }
            }
            Grid::new(format!("tech-corners-{name}"))
                .striped(true)
                .spacing([14.0, 3.0])
                .show(ui, |ui| {
                    ui.label(RichText::new("corner").strong());
                    for file in &files {
                        ui.label(RichText::new(file).strong());
                    }
                    ui.end_row();
                    for (corner, entries) in corners {
                        let is_default = *name == default_set && *corner == default_corner;
                        let text = RichText::new(corner);
                        ui.label(if is_default { text.strong() } else { text })
                            .on_hover_text(if is_default {
                                "The corner used unless a test bench says another"
                            } else {
                                ""
                            });
                        let entries: Vec<(String, String)> = entries
                            .as_array()
                            .into_iter()
                            .flatten()
                            .map(split)
                            .collect();
                        for file in &files {
                            let sections: Vec<&str> = entries
                                .iter()
                                .filter(|(f, _)| f == file)
                                .map(|(_, section)| section.as_str())
                                .collect();
                            ui.label(sections.join(", "));
                        }
                        ui.end_row();
                    }
                });
        }
    }

    /// The devices of the PDK as a table: symbol, model, PCell, Magic generator.
    fn devices(ui: &mut Ui, catalog: &Value, filter: &str, resolve: Resolve, asks: &mut Vec<Ask>) {
        let Some(devices) = lookup(catalog, "devices.devices").and_then(Value::as_array) else {
            return;
        };
        let rows: Vec<&Value> = devices
            .iter()
            .filter(|device| matches_filter(device, filter))
            .collect();
        ui.add_space(6.0);
        ui.strong(format!("Devices ({} of {})", rows.len(), devices.len()));
        ui.small("A ? after a PCell or generator marks a guess of the scan.");
        object_table(
            ui,
            "tech-devices",
            &rows,
            &[
                "name",
                "class",
                "symbol",
                "model",
                "pcell",
                "magic",
                "terminals",
                "parameters",
            ],
            resolve,
            asks,
        );
    }

    /// A section as a tree, or a line saying it is empty.
    fn section(
        ui: &mut Ui,
        title: &str,
        id: &str,
        value: Option<&Value>,
        filter: &str,
        resolve: Resolve,
        asks: &mut Vec<Ask>,
    ) {
        ui.add_space(6.0);
        ui.strong(title);
        match value {
            Some(value) if !is_empty(value) => value_tree(ui, id, value, filter, resolve, asks),
            _ => {
                ui.weak("Nothing found.");
            }
        }
    }

    fn libraries(
        ui: &mut Ui,
        ctx: &AppCtx,
        loaded: &Loaded,
        resolve: Resolve,
        asks: &mut Vec<Ask>,
    ) {
        let all = loaded.tech.libraries();
        if all.is_empty() {
            empty_state(ui, "The scan found no libraries in the PDK.", None);
            return;
        }
        ui.label("Ticked libraries are listed in the project, read-only.");
        let project = ctx.project.as_ref();
        Grid::new("tech-libraries")
            .striped(true)
            .spacing([14.0, 3.0])
            .show(ui, |ui| {
                for title in ["", "library", "role", "layout", "folder", ""] {
                    ui.label(RichText::new(title).strong());
                }
                ui.end_row();
                for lib in &all {
                    let mut on = lib.enabled;
                    if ui.checkbox(&mut on, "").changed() {
                        asks.push(Ask::Library(lib.clone(), on));
                    }
                    ui.label(&lib.name);
                    ui.label(&lib.role);
                    ui.label(&lib.layout);
                    path_label(ui, lib.path.as_str(), resolve, asks);
                    // Two libraries of one name, or one the project defines itself.
                    let listed = project.and_then(|p| p.library(&lib.name));
                    match listed {
                        Some(found) if lib.enabled => {
                            let here = resolve(lib.path.as_str());
                            if here.as_deref().is_some_and(|path| path != found.path) {
                                ui.weak("another library of this name is listed")
                                    .on_hover_text(found.path.display().to_string());
                            } else if let Some(error) = &found.error {
                                ui.colored_label(ui.visuals().warn_fg_color, error);
                            } else {
                                ui.weak(format!("{} cells", found.cells.len()));
                            }
                        }
                        _ => {
                            ui.weak(lib.description.as_deref().unwrap_or(""));
                        }
                    }
                    ui.end_row();
                }
            });
    }

    fn body(&mut self, ui: &mut Ui, ctx: &AppCtx, asks: &mut Vec<Ask>) {
        let Some(project) = &ctx.project else { return };
        let Some(loaded) = self.loaded.take() else {
            return;
        };
        let resolve_path = |text: &str| project.tech_path(&TechPath(text.to_string()));
        let resolve: Resolve = &resolve_path;
        let filter = self.filter.to_lowercase();
        let t = &loaded.tech_json;
        let c = &loaded.catalog_json;
        match self.page {
            Page::Overview => {
                Grid::new("tech-summary")
                    .num_columns(2)
                    .spacing([16.0, 4.0])
                    .show(ui, |ui| {
                        for (label, value) in &loaded.summary {
                            ui.label(RichText::new(label).weak());
                            if looks_like_path(value) {
                                path_label(ui, value, resolve, asks);
                            } else {
                                ui.label(value);
                            }
                            ui.end_row();
                        }
                    });
                if loaded.catalog.is_none() {
                    ui.add_space(6.0);
                    ui.weak(
                        "tech.catalog.json is missing: layers, PCells and symbols are not \
                         listed. Rescan to make it again.",
                    );
                }
                for wrong in &loaded.pins {
                    ui.add_space(6.0);
                    ui.colored_label(ui.visuals().warn_fg_color, wrong);
                }
                let (root, versions) = &loaded.versions;
                if !versions.is_empty() {
                    ui.add_space(8.0);
                    ui.strong("Installed versions (ciel)");
                    Grid::new("tech-versions")
                        .num_columns(3)
                        .spacing([14.0, 3.0])
                        .show(ui, |ui| {
                            for version in versions {
                                ui.label(&version.version);
                                ui.weak(version.pdks.join(", "));
                                if version.current {
                                    ui.label("in use");
                                } else if ui
                                    .button("Use")
                                    .on_hover_text(
                                        "Link this version into the PDK folder (as `ciel \
                                         enable` does) and scan it",
                                    )
                                    .clicked()
                                {
                                    asks.push(Ask::Enable {
                                        root: root.clone(),
                                        family: version.family.clone(),
                                        version: version.version.clone(),
                                    });
                                }
                                ui.end_row();
                            }
                        });
                    ui.small(format!(
                        "Other versions are fetched with the ciel program: ciel enable \
                         --pdk-root {} --pdk-family {} <version>",
                        root.display(),
                        versions[0].family
                    ));
                }
                if !loaded.tech.pdk.tested_with.is_empty() {
                    ui.add_space(8.0);
                    ui.strong("Tool versions the PDK names");
                    let tools: Vec<String> = loaded
                        .tech
                        .pdk
                        .tested_with
                        .iter()
                        .map(|(tool, version)| format!("{tool} {version}"))
                        .collect();
                    ui.label(tools.join(", "));
                }
            }
            Page::Layers => self.layers(ui, &loaded),
            Page::Verification => {
                // The decks first: what a designer runs.
                if let Some(decks) = lookup(c, "klayout.decks").and_then(Value::as_array) {
                    let rows: Vec<&Value> = decks
                        .iter()
                        .filter(|deck| matches_filter(deck, &filter))
                        .collect();
                    ui.add_space(6.0);
                    ui.strong("DRC and LVS decks");
                    object_table(
                        ui,
                        "tech-decks",
                        &rows,
                        &[
                            "check",
                            "name",
                            "description",
                            "default",
                            "switches",
                            "file",
                        ],
                        resolve,
                        asks,
                    );
                }
                Self::section(
                    ui,
                    "KLayout",
                    "t/klayout",
                    lookup(t, "klayout"),
                    &filter,
                    resolve,
                    asks,
                );
                let pcells = lookup(c, "klayout.pcell_libraries");
                Self::section(
                    ui,
                    "PCell libraries",
                    "c/pcells",
                    pcells,
                    &filter,
                    resolve,
                    asks,
                );
                Self::section(
                    ui,
                    "Magic",
                    "t/magic",
                    lookup(t, "magic"),
                    &filter,
                    resolve,
                    asks,
                );
                if lookup(c, "magic").is_some_and(|v| !is_empty(v)) {
                    Self::section(
                        ui,
                        "Magic, details",
                        "c/magic",
                        lookup(c, "magic"),
                        &filter,
                        resolve,
                        asks,
                    );
                }
                Self::section(
                    ui,
                    "Netgen",
                    "t/netgen",
                    lookup(t, "netgen"),
                    &filter,
                    resolve,
                    asks,
                );
                if lookup(c, "netgen").is_some_and(|v| !is_empty(v)) {
                    Self::section(
                        ui,
                        "Netgen, details",
                        "c/netgen",
                        lookup(c, "netgen"),
                        &filter,
                        resolve,
                        asks,
                    );
                }
            }
            Page::Models => {
                Self::corners(ui, t);
                Self::section(
                    ui,
                    "Model sets and corners",
                    "t/models",
                    lookup(t, "models"),
                    &filter,
                    resolve,
                    asks,
                );
                Self::section(
                    ui,
                    "Model files",
                    "c/models",
                    lookup(c, "models"),
                    &filter,
                    resolve,
                    asks,
                );
            }
            Page::Devices => {
                Self::devices(ui, c, &filter, resolve, asks);
                Self::section(
                    ui,
                    "Devices",
                    "t/devices",
                    lookup(t, "devices"),
                    &filter,
                    resolve,
                    asks,
                );
                Self::section(
                    ui,
                    "xschem",
                    "t/xschem",
                    lookup(t, "xschem"),
                    &filter,
                    resolve,
                    asks,
                );
                let symbols = lookup(c, "xschem.symbols");
                Self::section(ui, "Symbols", "c/symbols", symbols, &filter, resolve, asks);
            }
            Page::Libraries => {
                Self::libraries(ui, ctx, &loaded, resolve, asks);
                Self::section(
                    ui,
                    "Reference libraries",
                    "t/refs",
                    lookup(t, "refs"),
                    &filter,
                    resolve,
                    asks,
                );
                if lookup(c, "refs").is_some_and(|v| !is_empty(v)) {
                    Self::section(
                        ui,
                        "Cells",
                        "c/refs",
                        lookup(c, "refs"),
                        &filter,
                        resolve,
                        asks,
                    );
                }
            }
            Page::Notes => {
                if loaded.tech.notes.is_empty() {
                    empty_state(ui, "The scan had nothing to remark.", None);
                }
                for note in &loaded.tech.notes {
                    if filter.is_empty() || note.to_lowercase().contains(&filter) {
                        ui.label(format!("• {note}"));
                    }
                }
            }
            Page::Everything => {
                CollapsingHeader::new("tech.toml")
                    .default_open(true)
                    .show(ui, |ui| value_tree(ui, "all/t", t, &filter, resolve, asks));
                CollapsingHeader::new("tech.catalog.json")
                    .default_open(false)
                    .show(ui, |ui| value_tree(ui, "all/c", c, &filter, resolve, asks));
            }
        }
        self.loaded = Some(loaded);
    }
}

impl View for TechnologyView {
    fn title(&self, _ctx: &AppCtx) -> String {
        "Technology".into()
    }

    fn ui(&mut self, ui: &mut Ui, ctx: &mut AppCtx) {
        if ctx.project.is_none() {
            empty_state(
                ui,
                "No project is open.",
                Some("File > New Project… sets one up for a PDK."),
            );
            return;
        }
        self.refresh(ctx);
        ctx.tech.files_only = self.files_only;
        // A test script asked for a page.
        if let Some(wanted) = ctx.tech.page.take()
            && let Some(page) = Page::ALL
                .into_iter()
                .find(|page| page.label().eq_ignore_ascii_case(&wanted))
        {
            self.page = page;
        }
        let mut asks: Vec<Ask> = Vec::new();
        self.head(ui, ctx, &mut asks);
        ui.separator();
        if self.loaded.is_none() {
            let has_pdk = ctx.project.as_ref().is_some_and(|p| p.pdk.dir().is_some());
            empty_state(
                ui,
                "This project has no technology snapshot (tech.toml).",
                Some(if has_pdk {
                    "Rescan reads the PDK's files and writes it."
                } else {
                    "Choose a PDK in Project > Project Settings… first."
                }),
            );
        } else {
            ui.horizontal_wrapped(|ui| {
                for page in Page::ALL {
                    ui.selectable_value(&mut self.page, page, page.label());
                }
                ui.separator();
                ui.add(
                    egui::TextEdit::singleline(&mut self.filter)
                        .hint_text("Filter")
                        .desired_width(160.0),
                );
                if !self.filter.is_empty() && ui.small_button("✖").clicked() {
                    self.filter.clear();
                }
            });
            ui.separator();
            ScrollArea::both()
                .auto_shrink(false)
                .show(ui, |ui| self.body(ui, ctx, &mut asks));
        }
        for ask in asks {
            match ask {
                Ask::OpenInTab(file) => ctx.request(Action::OpenFile(file)),
                Ask::Rescan { helpers } => {
                    let egui_ctx = ui.ctx().clone();
                    if let Err(e) = pdkscan::start_rescan(ctx, &egui_ctx, helpers) {
                        ctx.error(e);
                    }
                }
                Ask::Library(library, on) => {
                    if let Err(e) = crate::newproject::list_library(ctx, &library, on) {
                        ctx.error(e);
                    }
                }
                Ask::Enable {
                    root,
                    family,
                    version,
                } => match tech::ciel::enable(&root, &family, &version) {
                    Ok(pdks) => {
                        tracing::info!(
                            target: "cdept::tech",
                            "{family} {version} is in use under {}: {}",
                            root.display(),
                            pdks.join(", ")
                        );
                        // The PDK's files changed under the project: scan them.
                        ctx.tech.generation += 1;
                        ctx.tech.pending = true;
                    }
                    Err(e) => ctx.error(format!("{e:#}")),
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_looked_up_and_shown() {
        let value = serde_json::json!({
            "klayout": {"tech": "sg13g2", "drc": [{"name": "full"}, {"name": "min"}]},
            "list": [1, 2, 3],
            "on": true,
        });
        assert_eq!(text_of(lookup(&value, "klayout.tech").unwrap()), "sg13g2");
        assert_eq!(
            text_of(lookup(&value, "klayout.drc.1.name").unwrap()),
            "min"
        );
        assert!(lookup(&value, "klayout.none").is_none());
        assert_eq!(names(lookup(&value, "klayout.drc")), "full, min");
        assert_eq!(text_of(lookup(&value, "list").unwrap()), "1, 2, 3");
        assert_eq!(text_of(lookup(&value, "on").unwrap()), "yes");
        assert!(matches_filter(&value, "sg13"));
        assert!(!matches_filter(&value, "sky"));
        assert!(looks_like_path("$PDK_ROOT/x/libs.tech"));
        assert!(!looks_like_path("sg13g2"));
    }

    #[test]
    fn the_summary_says_what_was_found_and_what_was_not() {
        let mut tech = Tech::default();
        tech.pdk.name = "ihp-sg13g2".into();
        tech.pdk.version = Some("1.0".into());
        tech.klayout.tech = Some("sg13g2".into());
        tech.klayout.dbu = Some(0.001);
        tech.xschem.libraries.push(TechLibrary {
            name: "sg13g2_pr".into(),
            path: TechPath("$PDK_ROOT/ihp-sg13g2/libs.tech/xschem/sg13g2_pr".into()),
            layout: "flat".into(),
            role: "primitives".into(),
            enabled: true,
            ..TechLibrary::default()
        });
        tech.notes.push("no netgen set-up".into());
        let lines = summary(&tech, Some(&Catalog::default()));
        let line = |label: &str| {
            lines
                .iter()
                .find(|(l, _)| l == label)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(line("PDK"), Some("ihp-sg13g2, 1.0"));
        assert_eq!(
            line("KLayout technology"),
            Some("sg13g2, database unit 0.001 µm")
        );
        assert_eq!(line("xschem start-up file"), Some("not found"));
        assert_eq!(line("Libraries"), Some("1 of 1: sg13g2_pr"));
        assert_eq!(line("Layers"), None);
        assert!(line("Notes").unwrap().starts_with('1'));
    }

    #[test]
    fn a_library_is_listed_or_left_out() {
        let mut tech = Tech::default();
        let lib = TechLibrary {
            name: "sg13g2_pr".into(),
            path: TechPath("$PDK_ROOT/p".into()),
            enabled: true,
            ..TechLibrary::default()
        };
        tech.xschem.libraries.push(lib.clone());
        assert!(set_library_enabled(&mut tech, &lib, false));
        assert!(!tech.libraries()[0].enabled);
        assert!(!set_library_enabled(&mut tech, &lib, false), "no change");
        let other = TechLibrary {
            name: "missing".into(),
            ..lib
        };
        assert!(!set_library_enabled(&mut tech, &other, true));
    }

    #[test]
    fn stipples_are_tiled_into_the_box() {
        let catalog = serde_json::json!({"klayout": {"dither_patterns": [
            {"order": 3, "name": "x", "lines": ["*.", ".*"]},
            {"order": 4, "name": "broken", "lines": []},
        ]}});
        let known = stipples(&catalog);
        assert_eq!(known.len(), 1);
        let [w, h] = SWATCH;
        let bits = stipple_bits("C3", &known);
        assert_eq!(bits.len(), w * h);
        assert!(bits[0] && !bits[1] && bits[2], "first row: *.*.");
        assert!(!bits[w] && bits[w + 1], "second row: .*");
        assert!(stipple_bits("I0", &known).iter().all(|b| *b), "solid");
        assert!(!stipple_bits("I1", &known).iter().any(|b| *b), "hollow");
        // A stipple the file does not define still shows something.
        assert!(stipple_bits("C99", &known).iter().any(|b| *b));
    }

    #[test]
    fn colours_are_read() {
        assert_eq!(colour("#ff8000"), Some(Color32::from_rgb(255, 128, 0)));
        assert_eq!(colour("0x0000ff"), Some(Color32::from_rgb(0, 0, 255)));
        assert_eq!(colour("ff0000ff"), Some(Color32::from_rgb(0, 0, 255)));
        assert_eq!(colour("red"), None);
    }

    #[test]
    fn tables_take_their_columns_from_the_rows() {
        let rows = [
            serde_json::json!({"name": "nmos", "kind": "mos", "pins": ["d", "g"], "w": 1.0}),
            serde_json::json!({"name": "rsil", "kind": "res", "note": ""}),
        ];
        let refs: Vec<&Value> = rows.iter().collect();
        assert_eq!(columns(&refs, &["name"]), ["name", "kind", "w", "pins"]);
        assert_eq!(
            columns(&refs, &["pins", "name", "absent"]),
            ["pins", "name", "kind", "w"]
        );
    }
}
