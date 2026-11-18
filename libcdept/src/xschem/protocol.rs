//! Messages between CDePT and the `cdept-xschem` helper.
//!
//! Transport: one Unix socket, UTF-8, one JSON object per line. CDePT listens and the
//! helper connects (its `--connect <path>` argument). After connecting the helper sends
//! [`Message::Ready`] or [`Message::Failed`].
//!
//! - [`Request::Tcl`] and [`Request::Shutdown`] get exactly one [`Message::Reply`] with
//!   the same `id`.
//! - [`Request::Resize`], [`Request::Input`] and [`Request::Ui`] get no reply. What they
//!   change comes back as [`Message::Frame`], [`Message::Damage`] and [`Message::Ui`],
//!   which the helper also sends on its own (a timer in xschem redraws, a dialog opens).
//!
//! **Pixels.** The drawing window is a file the helper creates in the folder given by
//! `--frame-dir` and both sides map into memory: `height` rows of `stride` bytes, four
//! bytes per pixel, R G B A with premultiplied alpha. [`Message::Frame`] announces a
//! new file (after a resize); a file is never resized. [`Message::Damage`] says which
//! rectangle of it changed. See [`super::frame`].
//!
//! **Widgets.** xschem's menus, tool bar, status bar and dialogs are written in Tcl/Tk.
//! The helper has no Tk: a stand-in written in Tcl (`shim/tcl/faketk.tcl`) records
//! what xschem's code creates and reports it as [`UiUpdate`]s; CDePT draws them with
//! its own widgets and reports what the user does as [`UiEvent`]s.

use std::collections::BTreeMap;
use std::io::{self, BufRead, Write};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::input::Input;

/// Bumped on incompatible changes. Both sides must agree.
pub const PROTOCOL_VERSION: u32 = 3;

/// CDePT → helper.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    /// Evaluate a Tcl script in xschem's interpreter, at global level.
    Tcl { id: u64, script: String },
    /// Reply, then exit.
    Shutdown { id: u64 },
    /// The drawing window has this size now, in pixels. A [`Message::Frame`] follows.
    Resize { width: u32, height: u32 },
    /// Pointer or keyboard input in the drawing window.
    Input { input: Input },
    /// Something the user did in a mirrored widget.
    Ui { event: UiEvent },
}

/// Helper → CDePT.
// Messages are read one at a time and handled at once: their size does not matter.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Message {
    /// xschem started. `xschem` is its version string.
    Ready { protocol: u32, xschem: String },
    /// xschem could not start; the helper exits after this.
    Failed { error: String },
    /// Answer to a request. `value` is the result, or the error text when `ok` is false.
    Reply { id: u64, ok: bool, value: String },
    /// The drawing window is now the file `path` (see the module documentation).
    /// `serial` counts up with every new file.
    Frame {
        serial: u64,
        path: String,
        width: u32,
        height: u32,
        stride: u32,
    },
    /// A rectangle of frame `serial` changed.
    Damage {
        serial: u64,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
    /// A menu, the tool bar, the status bar or a dialog changed.
    Ui { update: UiUpdate },
}

fn yes() -> bool {
    true
}

/// A change in xschem's widgets, as recorded by the Tk stand-in.
///
/// Every update replaces what was sent before for the same thing (the whole menu bar,
/// the whole window): there are no partial updates.
#[allow(clippy::large_enum_variant)] // a handful are alive at any time
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UiUpdate {
    /// The menu bar of the main window.
    Menu { items: Vec<MenuItem> },
    /// Pictures used by tool bar buttons and other widgets, sent before their first use.
    Images { images: Vec<Image> },
    /// The tool bar of the main window.
    Toolbar { items: Vec<ToolItem> },
    /// The status bar of the main window.
    Status { fields: Vec<StatusField> },
    /// Title of the main window (`wm title .`): file name and modified mark.
    Title { text: String },
    /// Pointer shape over the drawing window, as a Tk cursor name (`arrow`,
    /// `crosshair`, `watch`, …).
    Cursor { name: String },
    /// A top-level window (a dialog) was created or changed.
    Window { window: Window },
    /// A top-level window is gone.
    WindowClosed { path: String },
    /// A context menu to show at the pointer (`tk_popup`).
    Popup { items: Vec<MenuItem> },
    /// Text for the user that is not a dialog: xschem's info window, ERC messages.
    Log { text: String },
    /// xschem put text on the clipboard.
    Clipboard { text: String },
    /// xschem wants this file shown in a window of its own ("open in new tab/window"):
    /// CDePT opens it as another document.
    Open { path: String },
    /// xschem wrote a design file that is not the open document (a symbol made from
    /// the schematic): the libraries may hold a new view.
    Written { path: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MenuKind {
    Command,
    Check,
    Radio,
    Separator,
    /// A sub-menu; its entries are in `items`.
    Cascade,
}

/// One entry of a menu. Choosing it is reported as [`UiEvent::Menu`] with the same
/// `menu` and `index`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MenuItem {
    pub kind: MenuKind,
    #[serde(default)]
    pub label: String,
    /// Key shown next to the label, e.g. `Ctrl+S`. Display only: keys pressed in the
    /// drawing window go to xschem as [`Request::Input`].
    #[serde(default)]
    pub accelerator: String,
    /// Tk path of the menu the entry is in, and its index there.
    #[serde(default)]
    pub menu: String,
    #[serde(default)]
    pub index: u32,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Current state of a check or radio entry.
    #[serde(default)]
    pub checked: bool,
    /// Name of an [`Image`] shown with the entry; empty for none.
    #[serde(default)]
    pub image: String,
    /// Colour of the label when xschem sets one (it marks the state of Netlist,
    /// Simulate and Waves this way), as `#rrggbb` or a Tk colour name; empty for the
    /// usual colour.
    #[serde(default)]
    pub color: String,
    #[serde(default)]
    pub items: Vec<MenuItem>,
}

/// A picture, e.g. a tool bar icon.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Image {
    /// Tk image name; widgets refer to it.
    pub name: String,
    /// `gif`, `png`, `xpm`, … as given to `image create`.
    pub format: String,
    /// The file contents, base64.
    pub data: String,
}

/// A tool bar button, or a gap between groups. Pressing it is reported as
/// [`UiEvent::Invoke`] with `path`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolItem {
    #[serde(default)]
    pub separator: bool,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub tooltip: String,
    /// Name of an [`Image`]; empty for a text button.
    #[serde(default)]
    pub image: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Shown pressed in (a mode that is on).
    #[serde(default)]
    pub checked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusField {
    /// Tk path of the label, stable across updates.
    pub name: String,
    pub text: String,
    /// An entry the user can change (xschem's SNAP and GRID): the new text is
    /// reported as [`UiEvent::Text`] followed by [`UiEvent::Key`] `Return`, both with
    /// `name` as the path.
    #[serde(default)]
    pub editable: bool,
}

/// A top-level window with its widgets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Window {
    /// Tk path, e.g. `.dialog`.
    pub path: String,
    #[serde(default)]
    pub title: String,
    /// xschem waits for this window (`tkwait`, `grab`): nothing else reacts meanwhile.
    #[serde(default)]
    pub modal: bool,
    pub root: Widget,
}

/// A widget and its children, as Tk sees them.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Widget {
    /// Tk path, e.g. `.dialog.f1.entry`.
    pub path: String,
    /// Tk class: `Frame`, `Labelframe`, `Label`, `Button`, `Checkbutton`,
    /// `Radiobutton`, `Entry`, `Text`, `Listbox`, `Spinbox`, `Scale`, `TCombobox`,
    /// `Scrollbar`, `Canvas`, `Panedwindow`, `Menubutton`, … Classes the other side
    /// does not know are drawn as their children.
    pub class: String,
    /// Label or button text; for `Entry`, `Text`, `Spinbox` and `TCombobox` the
    /// content.
    #[serde(default)]
    pub text: String,
    /// `Checkbutton` and `Radiobutton`: whether it is on.
    #[serde(default)]
    pub checked: bool,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// The widget with the keyboard focus.
    #[serde(default)]
    pub focus: bool,
    /// `Listbox` lines, `TCombobox` and `Spinbox` choices.
    #[serde(default)]
    pub items: Vec<String>,
    /// `Listbox`: selected lines.
    #[serde(default)]
    pub selection: Vec<u32>,
    /// Other Tk options worth knowing, by option name without the dash: `width`,
    /// `height`, `wrap`, `image`, `justify`, `show`, `from`, `to`, …
    #[serde(default)]
    pub options: BTreeMap<String, String>,
    /// How the widget is placed in its parent: `manager` is `pack`, `grid` or `place`,
    /// the other keys are that manager's options without the dash (`side`, `fill`,
    /// `expand`, `row`, `column`, `columnspan`, `sticky`, …). Empty: not placed, do not
    /// show.
    #[serde(default)]
    pub layout: BTreeMap<String, String>,
    #[serde(default)]
    pub children: Vec<Widget>,
}

/// What the user did in a mirrored widget.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UiEvent {
    /// A menu entry was chosen (menu bar or context menu).
    Menu { menu: String, index: u32 },
    /// A context menu was dismissed without a choice.
    PopupClosed,
    /// A button, check button, radio button or tool bar button was pressed.
    Invoke { path: String },
    /// The content of an entry, text, spin box or combo box, after an edit.
    Text { path: String, text: String },
    /// Lines selected in a list box.
    Select { path: String, selection: Vec<u32> },
    /// A line of a list box was double-clicked.
    Activate { path: String, index: u32 },
    /// A key pressed while `path` had the focus, as a Tk keysym name: `Return`,
    /// `Escape`, `Tab`, `Up`, `a`, … Sent for keys that may have a binding; ordinary
    /// typing is reported with [`UiEvent::Text`].
    Key { path: String, key: String },
    /// The widget got the keyboard focus.
    Focus { path: String },
    /// The close button of a top-level window.
    Close { path: String },
}

/// Write one message as a line and flush.
pub fn write_message<T: Serialize>(writer: &mut impl Write, message: &T) -> io::Result<()> {
    let mut line = serde_json::to_string(message)?;
    line.push('\n');
    writer.write_all(line.as_bytes())?;
    writer.flush()
}

/// Read one message. `Ok(None)` when the other side closed the connection.
pub fn read_message<T: DeserializeOwned>(reader: &mut impl BufRead) -> io::Result<Option<T>> {
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    serde_json::from_str(&line)
        .map(Some)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_are_single_lines() {
        let request = Request::Tcl {
            id: 7,
            script: "puts {a\nb}".into(),
        };
        let mut wire = Vec::new();
        write_message(&mut wire, &request).unwrap();
        let text = String::from_utf8(wire.clone()).unwrap();
        assert_eq!(text.matches('\n').count(), 1, "newlines must be escaped");
        assert!(text.contains(r#""type":"tcl""#));

        let back: Option<Request> = read_message(&mut wire.as_slice()).unwrap();
        assert_eq!(back, Some(request));
    }

    #[test]
    fn end_of_stream_is_none() {
        let back: Option<Message> = read_message(&mut &b""[..]).unwrap();
        assert_eq!(back, None);
    }

    #[test]
    fn garbage_is_an_error() {
        let result: io::Result<Option<Message>> = read_message(&mut &b"not json\n"[..]);
        assert!(result.is_err());
    }

    #[test]
    fn requests_without_reply_read_back() {
        for request in [
            Request::Resize {
                width: 800,
                height: 600,
            },
            Request::Input {
                input: Input::ButtonPress {
                    button: 1,
                    x: 10,
                    y: 20,
                    state: 0,
                },
            },
            Request::Ui {
                event: UiEvent::Menu {
                    menu: ".menubar.file".into(),
                    index: 3,
                },
            },
        ] {
            let mut wire = Vec::new();
            write_message(&mut wire, &request).unwrap();
            let back: Option<Request> = read_message(&mut wire.as_slice()).unwrap();
            assert_eq!(back, Some(request));
        }
    }

    #[test]
    fn widget_updates_need_only_the_fields_that_matter() {
        // What the Tcl side writes by hand: short, most fields left out.
        let line = r#"{"type":"ui","update":{"kind":"window","window":{"path":".dialog",
            "title":"Save?","modal":true,"root":{"path":".dialog","class":"Toplevel",
            "children":[{"path":".dialog.yes","class":"Button","text":"Yes",
            "layout":{"manager":"pack","side":"left"}}]}}}}"#
            .replace('\n', " ");
        let message: Message = serde_json::from_str(&line).unwrap();
        let Message::Ui {
            update: UiUpdate::Window { window },
        } = message
        else {
            panic!("not a window update");
        };
        assert!(window.modal);
        let button = &window.root.children[0];
        assert!(button.enabled, "enabled is the default");
        assert_eq!(button.layout["side"], "left");

        let menu: UiUpdate = serde_json::from_str(
            r#"{"kind":"menu","items":[{"kind":"cascade","label":"File","items":[
                {"kind":"command","label":"Open","accelerator":"Ctrl+O",
                 "menu":".menubar.file","index":1},{"kind":"separator"}]}]}"#,
        )
        .unwrap();
        let UiUpdate::Menu { items } = menu else {
            panic!("not a menu");
        };
        assert_eq!(items[0].items[0].index, 1);
        assert_eq!(items[0].items[1].kind, MenuKind::Separator);
    }
}
