//! Messages between CDePT and the `cdept-klayout` helper (docs/KLAYOUT.md §4).
//!
//! Transport: one Unix socket, UTF-8, one JSON object per line. CDePT listens and the
//! helper connects. After connecting the helper sends [`Message::Ready`] or
//! [`Message::Failed`].
//!
//! - [`Request::Call`] and [`Request::Shutdown`] get exactly one [`Message::Reply`] with
//!   the same `id`. Methods and their parameters: docs/KLAYOUT.md §4; [`method`] names
//!   them.
//! - [`Request::Resize`] and [`Request::Input`] get no reply. What they change comes back
//!   as [`Message::Frame`], [`Message::Damage`] and [`Message::Event`].
//!
//! **Pixels.** As for xschem (see [`crate::xschem::frame`]): a file mapped by both
//! sides, `height` rows of `stride` bytes, four bytes per pixel, in the byte order
//! [`Message::Frame::format`] names. KLayout's is [`PixelFormat::Bgra`].

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use crate::xschem::protocol::{read_message, write_message};

/// Bumped on incompatible changes. Both sides must agree.
pub const PROTOCOL_VERSION: u32 = 1;

/// Names of the methods of [`Request::Call`].
pub mod method {
    pub const OPEN: &str = "open";
    pub const SAVE: &str = "save";
    pub const MODE: &str = "mode";
    pub const ZOOM: &str = "zoom";
    pub const UNDO: &str = "undo";
    pub const REDO: &str = "redo";
    pub const DELETE: &str = "delete";
    pub const SELECT_ALL: &str = "select_all";
    pub const CLEAR_SELECTION: &str = "clear_selection";
    pub const COPY: &str = "copy";
    pub const CUT: &str = "cut";
    pub const PASTE: &str = "paste";
    pub const LAYER_VISIBLE: &str = "layer_visible";
    pub const CURRENT_LAYER: &str = "current_layer";
    pub const ADD_LAYER: &str = "add_layer";
    pub const SHOW_CELL: &str = "show_cell";
    pub const LEVELS: &str = "levels";
    pub const PLACE: &str = "place";
    pub const CONFIG: &str = "config";
    pub const GET_CONFIG: &str = "get_config";
    pub const RENDER: &str = "render";
    pub const EXPORT: &str = "export";
    pub const INFO: &str = "info";
    pub const PYTHON: &str = "python";
    // --- Editing in place (docs/KLAYOUT.md §3.4) ---
    /// Edit the cell of the selected instance in place: `{}` → `{cell, context}`.
    pub const DESCEND: &str = "descend";
    /// Back up one level (`{"all": true}`: to the top) → `{cell, context}`.
    pub const ASCEND: &str = "ascend";
    /// Files with unsaved changes in other tabs, not to be changed: `{"files": [..]}`.
    pub const LOCKS: &str = "locks";
    // --- end editing in place ---
    // --- Properties and KLayout's libraries (helper: properties.py) ---
    /// The selected objects: `{}` → [`super::Properties`].
    pub const PROPERTIES: &str = "properties";
    /// Change one: `{index, key, object, pcell}` → [`super::Properties`] (one undo step).
    pub const SET_PROPERTIES: &str = "set_properties";
    /// KLayout's libraries (PDK, "Basic"): `{}` → `[`[`super::KLibrary`]`]`.
    pub const LIBRARIES: &str = "libraries";
    /// The form of a PCell: `{library, cell}` → [`super::PcellForm`].
    pub const PCELL_FORM: &str = "pcell_form";
    /// Instance mode with a library cell: `{library, cell, values}` → `{name, pcell}`.
    pub const PLACE_LIBRARY: &str = "place_library";
    // --- end properties ---
    // --- DRC and LVS results (helper: markers.py) ---
    /// Highlight shapes over the view, not part of the layout: `{shapes: [{kind:
    /// "polygon"|"box"|"edge"|"text", …}], color, zoom}` (µm, top cell; empty
    /// `shapes` clears) → `{markers}`. See [`crate::klayout::verify::Shape::to_marker`].
    pub const MARKERS: &str = "markers";
    // --- end DRC and LVS ---
}

/// CDePT → helper.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    /// Call a method; one [`Message::Reply`] follows.
    Call {
        id: u64,
        method: String,
        #[serde(default)]
        params: Value,
    },
    /// Reply, then exit.
    Shutdown { id: u64 },
    /// The drawing area has this size now, in pixels. A [`Message::Frame`] follows.
    Resize { width: u32, height: u32 },
    /// Pointer or keyboard input in the drawing area.
    Input { input: Input },
}

/// Byte order of a frame's pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PixelFormat {
    /// R G B A, alpha premultiplied (xschem).
    #[default]
    Rgba,
    /// B G R A (KLayout's `0xAARRGGBB` words, little-endian).
    Bgra,
}

/// Helper → CDePT.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Message {
    /// KLayout started. `klayout` is its version string (`KLayout 0.30.12`).
    Ready { protocol: u32, klayout: String },
    /// The helper could not start; it exits after this.
    Failed { error: String },
    /// Answer to a request: `value` when `ok`, else `error`.
    Reply {
        id: u64,
        ok: bool,
        #[serde(default)]
        value: Value,
        #[serde(default)]
        error: String,
    },
    /// The drawing area is now the file `path`; `serial` counts up with every file.
    Frame {
        serial: u64,
        path: String,
        width: u32,
        height: u32,
        stride: u32,
        #[serde(default)]
        format: PixelFormat,
    },
    /// A rectangle of frame `serial` changed.
    Damage {
        serial: u64,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
    /// Something in the editor changed.
    Event { event: Event },
}

/// A change in the editor's state. Each replaces what was sent before of its kind.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    /// The layer list (KLayout's layer properties, flattened).
    Layers { layers: Vec<Layer> },
    /// The cell hierarchy of the open layout.
    Cells {
        top: String,
        /// The cell shown, as a path of cell names from the top.
        path: Vec<String>,
        cells: Vec<CellNode>,
        /// The external cell edited in place; `None` at the file's own cells.
        #[serde(default)]
        context: Option<InPlace>,
    },
    /// What is selected.
    Selection {
        count: usize,
        /// `[left, bottom, right, top]` in µm.
        bbox: Option<[f64; 4]>,
        items: Vec<SelItem>,
    },
    /// Pointer position in µm, current mode and KLayout's hint line.
    Status {
        x: Option<f64>,
        y: Option<f64>,
        mode: String,
        #[serde(default)]
        message: String,
    },
    /// Modified flag and undo state.
    Modified {
        dirty: bool,
        can_undo: bool,
        can_redo: bool,
        /// External files with unsaved changes made in place (written by the next save).
        #[serde(default)]
        edited: Vec<std::path::PathBuf>,
    },
    /// An error or message from an editing step, e.g. "Please select a layer first".
    Notice { level: NoticeLevel, text: String },
}

/// The external cell being edited in place ([`Event::Cells`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InPlace {
    /// Its file; `None` for a cell that cannot be found.
    pub file: Option<std::path::PathBuf>,
    /// `library/cell/view`.
    pub label: String,
    pub writable: bool,
    /// Why it is not writable (a sentence), else empty.
    #[serde(default)]
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeLevel {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    /// Index in the flattened layer list; what [`method::LAYER_VISIBLE`] and
    /// [`method::CURRENT_LAYER`] take.
    pub id: u32,
    /// Display name (the layer properties name, else the source).
    pub name: String,
    /// KLayout's source specification, e.g. `1/0@1`.
    pub source: String,
    /// `#rrggbb`.
    pub color: String,
    pub frame_color: String,
    /// KLayout dither pattern index.
    pub dither: i32,
    pub visible: bool,
    pub current: bool,
    /// Nesting depth in layer groups (0 = top level).
    #[serde(default)]
    pub depth: u32,
    /// False when the layer has no counterpart in the layout.
    #[serde(default = "yes")]
    pub valid: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CellNode {
    pub name: String,
    /// Names of the cells instantiated in this one (each once).
    pub children: Vec<String>,
    /// Set for cells loaded from another view: its label, e.g. `analog/inv/layout`.
    pub external: Option<String>,
    /// A reference that could not be resolved (drawn as a red X).
    #[serde(default)]
    pub missing: bool,
    /// The file the cell was loaded from (external cells).
    pub file: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelItem {
    /// `box`, `polygon`, `path`, `text`, `edge`, `point` or `instance`.
    pub kind: String,
    /// Layer source (`1/0`) for shapes, empty for instances.
    #[serde(default)]
    pub layer: String,
    /// The cell holding the object.
    pub cell: String,
    /// One-line description (instance: placed cell and transformation; text: string).
    #[serde(default)]
    pub text: String,
}

/// Pointer or keyboard input, coordinates in physical pixels of the drawing area.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Input {
    Press {
        x: f64,
        y: f64,
        button: Button,
        modifiers: Modifiers,
    },
    Release {
        x: f64,
        y: f64,
        button: Button,
        modifiers: Modifiers,
    },
    DoubleClick {
        x: f64,
        y: f64,
        button: Button,
        modifiers: Modifiers,
    },
    Move {
        x: f64,
        y: f64,
        /// Buttons held down.
        buttons: Vec<Button>,
        modifiers: Modifiers,
    },
    /// `delta` in KLayout/Qt units (120 = one wheel notch, positive = away from user).
    Wheel {
        x: f64,
        y: f64,
        delta: i32,
        horizontal: bool,
        modifiers: Modifiers,
    },
    /// `key` is a name (`Escape`, `Return`, `Backspace`, `Delete`, `Tab`, `Left`,
    /// `Right`, `Up`, `Down`, `PageUp`, `PageDown`, `Home`, `End`, `F1`…`F12`), or empty
    /// with one character in `text`.
    Key {
        key: String,
        #[serde(default)]
        text: String,
        modifiers: Modifiers,
    },
    Enter,
    Leave,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Button {
    Left,
    Middle,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Modifiers {
    #[serde(default)]
    pub shift: bool,
    #[serde(default)]
    pub ctrl: bool,
    #[serde(default)]
    pub alt: bool,
}

// --- Properties and KLayout's libraries (helper: properties.py) -----------------------

/// The answer to `properties` and `set_properties`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Properties {
    /// The selected objects (the first 200).
    pub objects: Vec<Object>,
    /// How many objects are selected.
    pub count: usize,
    /// The layers of the layout (`1/0`, `name (1/0)`), for layer choices.
    #[serde(default)]
    pub layers: Vec<String>,
}

/// One selected object as the properties dialog shows it. Lengths are µm.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Object {
    /// Position in the selection and what identifies the object; both go back with
    /// `set_properties`.
    pub index: usize,
    pub key: String,
    /// The cell holding the object.
    pub parent: String,
    pub editable: bool,
    /// Why it cannot be changed, when it cannot.
    #[serde(default)]
    pub reason: String,
    /// The layer of a shape.
    #[serde(default)]
    pub layer: Option<String>,
    #[serde(flatten)]
    pub shape: Shape,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Shape {
    Box {
        /// `[left, bottom, right, top]`.
        bbox: [f64; 4],
    },
    Polygon {
        /// The hull (read-only).
        points: Vec<[f64; 2]>,
        holes: usize,
        /// Moves the polygon when sent back.
        #[serde(default)]
        offset: [f64; 2],
    },
    Path {
        width: f64,
        points: Vec<[f64; 2]>,
        begin_ext: f64,
        end_ext: f64,
        round: bool,
    },
    Text {
        string: String,
        size: f64,
        x: f64,
        y: f64,
        /// Quarter turns, 0 to 3.
        rotation: u32,
        mirror: bool,
        /// KLayout's alignment codes (-1 = default, 0 left/bottom, 1 centre, 2 right/top).
        halign: i32,
        valign: i32,
    },
    Instance(Instance),
    /// A shape the dialog does not edit (edges, points): its description.
    Other {
        text: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Instance {
    /// The placed cell (the PCell's name for a PCell).
    pub cell: String,
    /// KLayout library of the cell, if it comes from one.
    #[serde(default)]
    pub library: Option<String>,
    /// `library/cell/view` when the cell comes from another view of the project.
    #[serde(default)]
    pub external: Option<String>,
    #[serde(default)]
    pub missing: bool,
    pub x: f64,
    pub y: f64,
    /// Degrees.
    pub angle: f64,
    pub mirror: bool,
    pub mag: f64,
    #[serde(default)]
    pub array: Option<InstArray>,
    #[serde(default)]
    pub pcell: Option<PcellForm>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstArray {
    pub a: [f64; 2],
    pub b: [f64; 2],
    pub na: u32,
    pub nb: u32,
}

/// A PCell's parameters and their values (JSON: numbers, strings, booleans, lists of
/// strings; layers as `1/0`; shapes as text).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PcellForm {
    pub library: String,
    pub name: String,
    pub params: Vec<PcellParam>,
    pub values: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PcellParam {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: ParamType,
    pub description: String,
    /// KLayout's group ("Group<TAB>Label" in the description), or empty.
    #[serde(default)]
    pub group: String,
    #[serde(default)]
    pub unit: String,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub readonly: bool,
    #[serde(default)]
    pub default: Value,
    #[serde(default)]
    pub choices: Vec<Choice>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamType {
    Int,
    Double,
    String,
    Bool,
    List,
    Layer,
    Shape,
    Callback,
    None,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Choice {
    pub label: String,
    pub value: Value,
}

/// One of KLayout's libraries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KLibrary {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub technologies: Vec<String>,
    pub cells: Vec<KLibraryCell>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KLibraryCell {
    pub name: String,
    pub pcell: bool,
}

// --- end properties ---

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_and_messages_have_the_documented_shape() {
        let call = Request::Call {
            id: 3,
            method: method::MODE.into(),
            params: serde_json::json!({"name": "box"}),
        };
        assert_eq!(
            serde_json::to_string(&call).unwrap(),
            r#"{"type":"call","id":3,"method":"mode","params":{"name":"box"}}"#
        );
        let input = Request::Input {
            input: Input::Press {
                x: 1.0,
                y: 2.0,
                button: Button::Left,
                modifiers: Modifiers::default(),
            },
        };
        let text = serde_json::to_string(&input).unwrap();
        assert!(text.contains(r#""kind":"press""#) && text.contains(r#""button":"left""#));
        let frame: Message = serde_json::from_str(
            r#"{"type":"frame","serial":1,"path":"/f","width":2,"height":3,"stride":8,"format":"bgra"}"#,
        )
        .unwrap();
        assert!(matches!(
            frame,
            Message::Frame {
                format: PixelFormat::Bgra,
                ..
            }
        ));
        let reply: Message =
            serde_json::from_str(r#"{"type":"reply","id":1,"ok":false,"error":"no"}"#).unwrap();
        assert!(matches!(
            reply,
            Message::Reply {
                ok: false,
                value: Value::Null,
                ..
            }
        ));
        let event: Message = serde_json::from_str(
            r#"{"type":"event","event":{"kind":"notice","level":"error","text":"x"}}"#,
        )
        .unwrap();
        assert!(matches!(
            event,
            Message::Event {
                event: Event::Notice { .. }
            }
        ));
    }

    #[test]
    fn properties_as_the_helper_sends_them() {
        let text = r#"{"objects": [
            {"index": 0, "key": "shape 0 0 box", "parent": "buf", "editable": true,
             "reason": "", "layer": "3/0", "kind": "box", "bbox": [-0.5, -0.5, 12.0, -0.1]},
            {"index": 1, "key": "inst 0 CIRCLE", "parent": "buf", "editable": true,
             "reason": "", "kind": "instance", "layer": null, "cell": "CIRCLE",
             "library": "Basic", "external": null, "missing": false, "x": 7.0, "y": 0.96,
             "angle": 0.0, "mirror": false, "mag": 1.0, "array": null,
             "pcell": {"library": "Basic", "name": "CIRCLE", "values": {"layer": "3/0"},
                "params": [{"name": "layer", "type": "layer", "description": "Layer",
                  "group": "", "unit": "", "hidden": false, "readonly": false,
                  "default": null, "choices": []}]}}],
            "count": 2, "layers": ["3/0"]}"#;
        let props: Properties = serde_json::from_str(text).unwrap();
        assert_eq!(props.objects[0].layer.as_deref(), Some("3/0"));
        assert!(matches!(props.objects[0].shape, Shape::Box { bbox } if bbox[2] == 12.0));
        let Shape::Instance(inst) = &props.objects[1].shape else {
            panic!("not an instance");
        };
        assert_eq!(inst.cell, "CIRCLE");
        let form = inst.pcell.as_ref().unwrap();
        assert_eq!(form.params[0].kind, ParamType::Layer);
        // Back to the helper: the same fields, the kind as a tag.
        let back = serde_json::to_value(&props.objects[1]).unwrap();
        assert_eq!(back["kind"], "instance");
        assert_eq!(back["parent"], "buf");
        assert_eq!(back["x"], 7.0);
    }
}
