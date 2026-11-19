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
    },
    /// An error or message from an editing step, e.g. "Please select a layer first".
    Notice { level: NoticeLevel, text: String },
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
}
