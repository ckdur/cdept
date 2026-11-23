//! A test driver for the real GUI: runs a script of commands, one per line, while the
//! program is on screen (`cdept gui --script <file>`). See `docs/GUI-TESTING.md`.
//!
//! Mouse and keyboard commands are fed to egui as input events, so they take the same
//! path as the user's. While a script runs, the real mouse and keyboard are ignored.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use egui::{
    Context, Event, Key, Modifiers, MouseWheelUnit, PointerButton, Pos2, RawInput, TouchPhase,
    Vec2, ViewportCommand,
};
use libcdept::xschem::NetlistFormat;

use crate::app::{CdeptApp, LeaveChoice};
use crate::state::{Action, Selection};
use crate::views::{CloseChoice, NETLIST_FORMATS};

/// How long a command may wait for the program before the script fails.
const TIMEOUT: Duration = Duration::from_secs(20);
/// Frames to let pass after the helper answered, for its picture to arrive.
const SETTLE_FRAMES: u32 = 3;
/// How long a check or a dialog command waits for the program to get there: what
/// xschem reports (a title, a dialog) arrives a moment after its answer, and a
/// background job (hierarchy, netlist, preview) has to start an xschem first.
const PATIENCE: Duration = Duration::from_secs(8);
/// `sim wait`: a simulation netlists with xschem and runs every corner.
const SIM_PATIENCE: Duration = Duration::from_secs(120);
/// `sim suite_wait`: a suite runs many testbenches (the OTA's: 90 runs).
const SUITE_PATIENCE: Duration = Duration::from_secs(600);
/// Steps of a drag between its two ends.
const DRAG_STEPS: u32 = 8;
/// While waiting for an answer of xschem, how often to send it another request. A
/// request that reached the helper together with the input that made xschem open a
/// dialog is only looked at when more arrives.
const NUDGE: Duration = Duration::from_millis(300);

#[derive(Debug, Clone, PartialEq)]
enum Command {
    Open(Selection),
    Close,
    CloseTab,
    CloseAnswer(CloseChoice),
    ExpectTitle(String),
    Detach,
    Wait(f32),
    WaitFrame,
    Sync,
    Move(f32, f32),
    MoveBy(f32, f32),
    Click(f32, f32, PointerButton, Modifiers),
    DoubleClick(f32, f32),
    ClickHere(PointerButton),
    ClickInstance(String, PointerButton),
    Press(f32, f32, PointerButton),
    Release(f32, f32, PointerButton),
    Drag([f32; 4], PointerButton),
    Wheel(f32, f32, f32, u32),
    Key(Key, Modifiers),
    Text(String),
    Tcl(String),
    TclSend(String),
    TclExpect(String, String),
    Menu(Vec<String>),
    Menus,
    DialogClick(String),
    DialogType(String, String),
    ExpectDialog(String),
    ExpectPane(String),
    Clipboard(String),
    Primary(String),
    Frame,
    Pixel(u32, u32),
    ExpectPixel(u32, u32, [u8; 3]),
    ExpectDocuments(usize),
    Panel(String),
    Select(Selection),
    ExpectHierarchy(String),
    HierarchyOpen(String),
    ExpectPreview,
    ExpectSelected(Option<String>),
    /// The nets the active schematic last exchanged with a layout (cross-probing).
    ExpectSchematicNets(Vec<String>),
    Netlist(String, String, NetlistFormat),
    ExpectView(Selection),
    CloseProject,
    QuitAsk,
    LeaveAnswer(LeaveChoice),
    ExpectProject(Option<String>),
    /// Library definitions and path prompts (library_dialog.rs).
    Definitions(crate::library_dialog::ScriptCommand),
    /// Renames and moves with references following, the Hierarchy panel (refactor.rs).
    Refactor(crate::refactor::ScriptCommand),
    ExpectPicker(String),
    Picker(Selection),
    PickerFile(Option<PathBuf>),
    Window(f32, f32),
    Zoom(f32),
    Screenshot(PathBuf),
    CheckPicture,
    Log(String),
    Layout(LayoutCommand),
    /// `tech <verb> …`: the New Project wizard and the technology (newproject.rs).
    Tech(Vec<String>),
    /// `sim <verb> …`: the Simulation tab (views/simulation.rs).
    Sim(Vec<String>),
    /// `organise <verb> …`: Go to Cell, Search, categories, Import Project (import.rs).
    Organise(Vec<String>),
    /// Changes on disk, git marks, the trash (background/script.rs).
    Disk(crate::background::script::DiskCommand),
    Quit,
}

/// Commands for a layout document (KLayout).
#[derive(Debug, Clone, PartialEq)]
enum LayoutCommand {
    Mode(String),
    Zoom(String),
    /// `undo`, `redo`, `delete`, `select_all`, `clear_selection`, `copy`, `cut`, `paste`.
    Edit(String),
    Save,
    Place(Selection),
    Export(PathBuf),
    AddLayer(i32, i32, String),
    CurrentLayer(String),
    LayerVisible(String, bool),
    ShowCell(Vec<String>),
    /// A double click on a cell of the Cells panel.
    CellOpen(String),
    Levels(u32),
    Call(String, serde_json::Value),
    ExpectNotice(String),
    ExpectLayer(String),
    /// A cell of the tree, and what kind it is (`local`, `external`, `missing`).
    ExpectCell(String, Option<String>),
    /// The cell the tab shows (one cell of a file that holds several).
    ExpectShown(String),
    ExpectSelection(usize),
    ExpectModified(bool),
    ExpectMode(String),
    // --- properties and library cells ---
    /// Open the properties of the selection.
    Properties,
    /// Apply the form (`true`: and close, as OK).
    PropertiesApply(bool),
    /// A field of the form (`x`, `bbox.2`, `pcell.radius`) and its JSON value.
    SetProperty(String, String),
    ExpectProperty(String, String),
    /// Instance mode with a cell of a KLayout library, and PCell values.
    PlaceLibrary(String, String, serde_json::Value),
    // --- end properties ---
    // --- PCells: another cell for the instance shown in the properties ---
    /// A cell of a KLayout library, with PCell values.
    ReplaceLibrary(String, String, serde_json::Value),
    /// A layout view of the project: library, cell, view.
    ReplaceView(String, String, String),
    // --- end PCells ---
    /// The external cell edited in place (`library/cell/view`), or `None`.
    ExpectInPlace(Option<String>),
    /// Run DRC or LVS (`drc`, `lvs`).
    Check(String),
    /// The last check is done and its summary contains the text.
    ExpectCheck(String),
    /// Highlight DRC result n (from 1), as a click in the results panel does.
    CheckShow(usize),
    // --- linking with the schematic (docs/LINKING.md) ---
    /// Generate / Update from Schematic (`true`: also remove extra instances).
    LinkGenerate(bool),
    /// Select instances by schematic name, as a click in the Link panel does.
    LinkSelect(Vec<String>),
    /// The Link panel shows this status for a schematic instance.
    ExpectLink(String, String),
    /// The names bound to the layout's selection, as cross-probing last saw them.
    ExpectProbed(Vec<String>),
    // --- nets (docs/LINKING.md §8) ---
    /// Update Connectivity.
    NetsUpdate,
    /// The nets are compared and the summary contains the text.
    ExpectNets(String),
    /// Show a schematic net, as a click in the Nets panel does.
    NetsShow(String),
    /// Probe the net at a frame pixel (`None`: the middle of the picture), as a click in
    /// probe mode does.
    NetsProbe(Option<(f64, f64)>),
    /// The schematic nets last probed or shown in the layout.
    ExpectNetProbed(Vec<String>),
    /// Flylines on or off.
    Flylines(bool),
    /// The helper drew this many flylines.
    ExpectFlylines(usize),
    /// Change a flyline setting (as the Flyline Style window does): key, value.
    FlylineStyle(String, String),
    /// The helper drew the flylines with this setting: key, value.
    ExpectFlylineStyle(String, String),
    /// Join a parallel group (index from 1) or not.
    Parallel(usize, bool),
    /// The comparison found these parallel groups (members joined by commas, `!`
    /// after a group that is not joined), or `none`.
    ExpectParallel(Vec<String>),
    // --- pins (docs/LINKING.md §9) ---
    /// Add a pin with a box on a conductor: name, direction, layer, box (µm).
    PinAdd(String, String, String, [f64; 4]),
    /// Add a pin from the selected shapes: name, direction.
    PinFromSelection(String, String),
    /// Draw Pin: the next box drawn on the current layer becomes the pin.
    PinDraw(String, String),
    PinRemove(String),
    PinRename(String, String),
    /// Select and show a pin, as a click in the Pins panel does.
    PinShow(String),
    /// The Pins panel shows this status for a pin (`absent`: no such pin).
    ExpectPin(String, String),
    /// The Pins panel on or off.
    PinsPanel(bool),
    // --- end linking ---
}

impl Command {
    /// Commands that are tried again for a while before they fail. They do nothing
    /// when they fail, so trying again is safe.
    fn patient(&self) -> bool {
        if let Command::Definitions(command) = self {
            return command.patient();
        }
        if let Command::Sim(args) = self {
            return crate::views::simulation::patient(args);
        }
        matches!(
            self,
            Command::Menu(_)
                | Command::DialogClick(_)
                | Command::DialogType(..)
                | Command::CloseAnswer(_)
                | Command::ExpectDialog(_)
                | Command::ExpectPane(_)
                | Command::ExpectPixel(..)
                | Command::ExpectDocuments(_)
                | Command::ExpectTitle(_)
                | Command::ExpectHierarchy(_)
                | Command::HierarchyOpen(_)
                | Command::ExpectPreview
                | Command::ExpectSelected(_)
                | Command::ExpectSchematicNets(_)
                | Command::ExpectView(_)
                | Command::LeaveAnswer(_)
                | Command::ExpectProject(_)
                | Command::ExpectPicker(_)
                | Command::Picker(_)
                | Command::PickerFile(_)
                | Command::Tech(_)
                | Command::Organise(_)
                | Command::Layout(
                    LayoutCommand::Place(_)
                        | LayoutCommand::CurrentLayer(_)
                        | LayoutCommand::LayerVisible(..)
                        | LayoutCommand::ShowCell(_)
                        | LayoutCommand::ExpectNotice(_)
                        | LayoutCommand::ExpectLayer(_)
                        | LayoutCommand::ExpectCell(..)
                        | LayoutCommand::ExpectShown(_)
                        | LayoutCommand::ExpectSelection(_)
                        | LayoutCommand::ExpectModified(_)
                        | LayoutCommand::ExpectMode(_)
                        | LayoutCommand::SetProperty(..)
                        | LayoutCommand::ExpectProperty(..)
                        | LayoutCommand::ExpectInPlace(_)
                        | LayoutCommand::ExpectCheck(_)
                        | LayoutCommand::ExpectLink(..)
                        | LayoutCommand::ExpectProbed(_)
                        | LayoutCommand::ExpectNets(_)
                        | LayoutCommand::ExpectNetProbed(_)
                        | LayoutCommand::ExpectFlylines(_)
                        | LayoutCommand::ExpectFlylineStyle(..)
                        | LayoutCommand::ExpectParallel(_)
                        | LayoutCommand::ExpectPin(..),
                )
        ) || matches!(self, Command::Disk(command) if command.patient())
    }

    /// How long a patient command is tried again. A simulation takes a while.
    fn patience(&self) -> Duration {
        match self {
            Command::Sim(args) if args.first().map(String::as_str) == Some("wait") => SIM_PATIENCE,
            Command::Sim(args) if args.first().map(String::as_str) == Some("suite_wait") => SUITE_PATIENCE,
            _ => PATIENCE,
        }
    }
}

/// Something to feed to egui in one frame.
#[derive(Debug, Clone)]
enum Step {
    /// Events, with the modifier keys held meanwhile. Positions are in frame pixels of
    /// the active schematic until they are sent.
    Events(Vec<Event>, Modifiers),
}

enum Waiting {
    Nothing,
    Until(Instant),
    Frames(u32),
    /// A frame of the active schematic on screen.
    Frame(Instant),
    /// The answer to a Tcl request, then what to do with it.
    Reply(u64, Instant, Answer),
    /// The outcome of a layout's save, place or export.
    Done(u64, Instant),
    /// The screenshot asked for: to save, or (`None`) to compare with the frame.
    Screenshot(Option<PathBuf>, Instant),
}

#[derive(Clone)]
enum Answer {
    /// Only wait for it: the helper has handled everything sent before.
    Settle,
    Log,
    Expect(String),
    /// The frame position to click at with this button.
    Click(PointerButton),
}

pub struct Script {
    commands: VecDeque<(usize, Command)>,
    steps: VecDeque<Step>,
    waiting: Waiting,
    /// Line of the command being run.
    line: usize,
    /// Ask the helper for a sign of life once the steps are sent.
    settle: bool,
    /// Where the scripted pointer is, in frame pixels.
    pointer: (f32, f32),
    /// When the patience with the command being tried runs out.
    give_up: Option<Instant>,
    /// When to send xschem another request while waiting for an answer (see [`NUDGE`]).
    nudge: Instant,
    failed: Arc<AtomicBool>,
    finished: bool,
    /// What the desktop's clipboard and primary selection hold, as far as the script
    /// says (`clipboard`, `primary`): Ctrl+V pastes the first.
    clipboard: (String, String),
}

/// The first word of `text` (or a `{braced}` group) and the rest.
fn word(text: &str) -> (&str, &str) {
    let text = text.trim_start();
    if let Some(rest) = text.strip_prefix('{') {
        let mut depth = 1;
        for (at, c) in rest.char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return (&rest[..at], rest[at + 1..].trim_start());
                    }
                }
                _ => {}
            }
        }
    }
    match text.split_once(char::is_whitespace) {
        Some((first, rest)) => (first, rest.trim_start()),
        None => (text, ""),
    }
}

fn button(name: Option<&str>) -> Result<PointerButton, String> {
    match name {
        None | Some("left") => Ok(PointerButton::Primary),
        Some("middle") => Ok(PointerButton::Middle),
        Some("right") => Ok(PointerButton::Secondary),
        Some(other) => Err(format!("unknown mouse button '{other}'")),
    }
}

fn key(name: &str) -> Result<Key, String> {
    let named = Key::from_name(name).or_else(|| {
        // "a" for the A key, "return" for Enter, as people write them.
        let mut chars = name.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) => Key::from_name(&c.to_uppercase().to_string()),
            _ => match name.to_ascii_lowercase().as_str() {
                "return" => Some(Key::Enter),
                "esc" => Some(Key::Escape),
                "pageup" => Some(Key::PageUp),
                "pagedown" => Some(Key::PageDown),
                "del" => Some(Key::Delete),
                _ => None,
            },
        }
    });
    named.ok_or_else(|| format!("unknown key '{name}'"))
}

fn parse_line(line: &str) -> Result<Option<Command>, String> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return Ok(None);
    }
    let (name, rest) = word(line);
    let args: Vec<&str> = rest.split_whitespace().collect();
    let number = |at: usize| -> Result<f32, String> {
        let text = args
            .get(at)
            .ok_or(format!("'{name}' needs more arguments"))?;
        text.parse()
            .map_err(|_| format!("'{text}' is not a number"))
    };
    let whole = |at: usize| -> Result<u32, String> {
        let text = args
            .get(at)
            .ok_or(format!("'{name}' needs more arguments"))?;
        text.parse()
            .map_err(|_| format!("'{text}' is not a whole number"))
    };
    let text = || -> Result<String, String> {
        if rest.is_empty() {
            Err(format!("'{name}' needs an argument"))
        } else {
            Ok(rest.to_string())
        }
    };
    let command = match name {
        "open" => match args.as_slice() {
            [library, cell, view] => Command::Open(Selection::view(library, cell, view)),
            _ => return Err("usage: open <library> <cell> <view>".into()),
        },
        "close" => Command::Close,
        "close_tab" => Command::CloseTab,
        "close_answer" => Command::CloseAnswer(match args.first().copied() {
            Some("save") => CloseChoice::Save,
            Some("discard") => CloseChoice::Discard,
            Some("cancel") => CloseChoice::Cancel,
            _ => return Err("usage: close_answer <save|discard|cancel>".into()),
        }),
        "expect_title" => Command::ExpectTitle(text()?),
        "detach" => Command::Detach,
        "wait" => Command::Wait(number(0)?),
        "wait_frame" => Command::WaitFrame,
        "sync" => Command::Sync,
        "move" => Command::Move(number(0)?, number(1)?),
        "move_by" => Command::MoveBy(number(0)?, number(1)?),
        "click_here" => Command::ClickHere(button(args.first().copied())?),
        "click_instance" => {
            let name = args
                .first()
                .ok_or("usage: click_instance <name> [button]")?;
            Command::ClickInstance(name.to_string(), button(args.get(1).copied())?)
        }
        "click" => {
            // click <x> <y> [button] [ctrl] [shift] [alt]
            let mut pressed = PointerButton::Primary;
            let mut modifiers = Modifiers::NONE;
            for flag in args.iter().skip(2) {
                match *flag {
                    "ctrl" => modifiers |= Modifiers::CTRL | Modifiers::COMMAND,
                    "shift" => modifiers |= Modifiers::SHIFT,
                    "alt" => modifiers |= Modifiers::ALT,
                    other => pressed = button(Some(other))?,
                }
            }
            Command::Click(number(0)?, number(1)?, pressed, modifiers)
        }
        "double_click" => Command::DoubleClick(number(0)?, number(1)?),
        "press" => Command::Press(number(0)?, number(1)?, button(args.get(2).copied())?),
        "release" => Command::Release(number(0)?, number(1)?, button(args.get(2).copied())?),
        "drag" => Command::Drag(
            [number(0)?, number(1)?, number(2)?, number(3)?],
            button(args.get(4).copied())?,
        ),
        "wheel" => {
            let notch = match args.get(2).copied() {
                Some("up") => 1.0,
                Some("down") => -1.0,
                _ => return Err("usage: wheel <x> <y> <up|down> [count]".into()),
            };
            let count = if args.len() > 3 { whole(3)? } else { 1 };
            Command::Wheel(number(0)?, number(1)?, notch, count)
        }
        "key" => {
            let (name, flags) = args
                .split_first()
                .ok_or("usage: key <name> [ctrl] [shift] [alt]")?;
            let mut modifiers = Modifiers::NONE;
            for flag in flags {
                modifiers |= match *flag {
                    "ctrl" => Modifiers::CTRL | Modifiers::COMMAND,
                    "shift" => Modifiers::SHIFT,
                    "alt" => Modifiers::ALT,
                    other => return Err(format!("unknown modifier '{other}'")),
                };
            }
            Command::Key(key(name)?, modifiers)
        }
        "text" => Command::Text(text()?),
        "tcl" => Command::Tcl(text()?),
        "tcl_send" => Command::TclSend(text()?),
        "tcl_expect" => {
            let (expected, script) = word(rest);
            if script.is_empty() {
                return Err("usage: tcl_expect <result or {result}> <script>".into());
            }
            Command::TclExpect(expected.to_string(), script.to_string())
        }
        "menu" => Command::Menu(text()?.split('/').map(|l| l.trim().to_string()).collect()),
        "menus" => Command::Menus,
        "dialog_click" => Command::DialogClick(text()?),
        "dialog_type" => {
            let (target, typed) = word(rest);
            if target.is_empty() {
                return Err("usage: dialog_type <path or {label}> <text>".into());
            }
            Command::DialogType(target.to_string(), typed.to_string())
        }
        "expect_dialog" => Command::ExpectDialog(text()?),
        "expect_pane" => Command::ExpectPane(text()?),
        // {text} keeps spaces at the ends
        "clipboard" | "primary" => {
            let inner = rest.strip_prefix('{').and_then(|r| r.strip_suffix('}'));
            let text = inner.unwrap_or(rest).to_string();
            match name {
                "clipboard" => Command::Clipboard(text),
                _ => Command::Primary(text),
            }
        }
        "frame" => Command::Frame,
        "pixel" => Command::Pixel(whole(0)?, whole(1)?),
        "expect_pixel" => {
            let colour = args.get(2).ok_or("usage: expect_pixel <x> <y> <rrggbb>")?;
            let value = u32::from_str_radix(colour.trim_start_matches('#'), 16)
                .ok()
                .filter(|_| colour.trim_start_matches('#').len() == 6)
                .ok_or(format!("'{colour}' is not a colour like ff8000"))?;
            let [_, r, g, b] = value.to_be_bytes();
            Command::ExpectPixel(whole(0)?, whole(1)?, [r, g, b])
        }
        "expect_documents" => Command::ExpectDocuments(whole(0)? as usize),
        "panel" => Command::Panel(text()?),
        "select" => Command::Select(match args.as_slice() {
            [library] => Selection::library(library),
            [library, cell] => Selection::cell(library, cell),
            [library, cell, view] => Selection::view(library, cell, view),
            _ => return Err("usage: select <library> [cell] [view]".into()),
        }),
        "expect_hierarchy" => Command::ExpectHierarchy(text()?),
        "hierarchy_open" => Command::HierarchyOpen(text()?),
        "expect_preview" => Command::ExpectPreview,
        "expect_selected" => Command::ExpectSelected(match args.as_slice() {
            ["none"] => None,
            [name] => Some(name.to_string()),
            _ => return Err("usage: expect_selected <instance|none>".into()),
        }),
        "netlist" => {
            let format = match args.get(2) {
                None => NetlistFormat::Spice,
                Some(name) => NETLIST_FORMATS
                    .iter()
                    .find(|(label, format)| {
                        label.eq_ignore_ascii_case(name) || format.name() == *name
                    })
                    .map(|(_, format)| *format)
                    .ok_or(format!("unknown netlist format '{name}'"))?,
            };
            match args.as_slice() {
                [library, cell] | [library, cell, _] => {
                    Command::Netlist(library.to_string(), cell.to_string(), format)
                }
                _ => return Err("usage: netlist <library> <cell> [format]".into()),
            }
        }
        "expect_view" => match args.as_slice() {
            [library, cell, view] => Command::ExpectView(Selection::view(library, cell, view)),
            _ => return Err("usage: expect_view <library> <cell> <view>".into()),
        },
        "close_project" => Command::CloseProject,
        "quit_ask" => Command::QuitAsk,
        "leave_answer" => Command::LeaveAnswer(match args.first().copied() {
            Some("save") => LeaveChoice::SaveAll,
            Some("discard") => LeaveChoice::DiscardAll,
            Some("cancel") => LeaveChoice::Cancel,
            _ => return Err("usage: leave_answer <save|discard|cancel>".into()),
        }),
        "expect_project" => Command::ExpectProject(match text()?.as_str() {
            "none" => None,
            name => Some(name.to_string()),
        }),
        "expect_picker" => Command::ExpectPicker(text()?),
        "picker" => match args.as_slice() {
            [library, cell, view] => Command::Picker(Selection::view(library, cell, view)),
            _ => return Err("usage: picker <library> <cell> <view>".into()),
        },
        "picker_file" => Command::PickerFile(Some(PathBuf::from(text()?))),
        "picker_cancel" => Command::PickerFile(None),
        "window" => Command::Window(number(0)?, number(1)?),
        "zoom" => Command::Zoom(number(0)?),
        "screenshot" => Command::Screenshot(PathBuf::from(text()?)),
        "check_picture" => Command::CheckPicture,
        "log" => Command::Log(rest.to_string()),
        "tech" => Command::Tech(args.iter().map(|word| word.to_string()).collect()),
        "sim" => Command::Sim(args.iter().map(|word| word.to_string()).collect()),
        "organise" => Command::Organise(args.iter().map(|word| word.to_string()).collect()),
        "quit" => Command::Quit,
        "mode" => Command::Layout(LayoutCommand::Mode(text()?)),
        "layout_zoom" => match args.first().copied() {
            Some(what @ ("fit" | "in" | "out")) => {
                Command::Layout(LayoutCommand::Zoom(what.to_string()))
            }
            _ => return Err("usage: layout_zoom <fit|in|out>".into()),
        },
        "layout_edit" => match args.first().copied() {
            Some(
                what @ ("undo" | "redo" | "delete" | "select_all" | "clear_selection" | "copy"
                | "cut" | "paste"),
            ) => Command::Layout(LayoutCommand::Edit(what.to_string())),
            _ => {
                return Err(
                    "usage: layout_edit <undo|redo|delete|select_all|clear_selection|\
                            copy|cut|paste>"
                        .into(),
                );
            }
        },
        "save" => Command::Layout(LayoutCommand::Save),
        "place" => match args.as_slice() {
            [library, cell, view] => {
                Command::Layout(LayoutCommand::Place(Selection::view(library, cell, view)))
            }
            _ => return Err("usage: place <library> <cell> <view>".into()),
        },
        "export" => Command::Layout(LayoutCommand::Export(PathBuf::from(text()?))),
        "add_layer" => {
            let name = args.get(2..).map(|rest| rest.join(" ")).unwrap_or_default();
            let whole = |at: usize| -> Result<i32, String> {
                let text = args
                    .get(at)
                    .ok_or("usage: add_layer <layer> <datatype> [name]")?;
                text.parse()
                    .map_err(|_| format!("'{text}' is not a whole number"))
            };
            Command::Layout(LayoutCommand::AddLayer(whole(0)?, whole(1)?, name))
        }
        "current_layer" => Command::Layout(LayoutCommand::CurrentLayer(text()?)),
        "layer_visible" => match args.as_slice() {
            [layer, on @ ("on" | "off")] => {
                Command::Layout(LayoutCommand::LayerVisible(layer.to_string(), *on == "on"))
            }
            _ => return Err("usage: layer_visible <layer name or source> <on|off>".into()),
        },
        "show_cell" => {
            if args.is_empty() {
                return Err("usage: show_cell <cell> [<cell> ...] (the path from the top)".into());
            }
            Command::Layout(LayoutCommand::ShowCell(
                args.iter().map(|a| a.to_string()).collect(),
            ))
        }
        "cell_open" => Command::Layout(LayoutCommand::CellOpen(text()?)),
        "levels" => Command::Layout(LayoutCommand::Levels(whole(0)?)),
        "layout_call" => {
            let (method, params) = word(rest);
            if method.is_empty() {
                return Err("usage: layout_call <method> [<json params>]".into());
            }
            let params = if params.is_empty() {
                serde_json::json!({})
            } else {
                serde_json::from_str(params).map_err(|e| format!("the parameters: {e}"))?
            };
            Command::Layout(LayoutCommand::Call(method.to_string(), params))
        }
        "expect_notice" => Command::Layout(LayoutCommand::ExpectNotice(text()?)),
        "expect_layer" => Command::Layout(LayoutCommand::ExpectLayer(text()?)),
        "expect_cell" => match args.as_slice() {
            [name] => Command::Layout(LayoutCommand::ExpectCell(name.to_string(), None)),
            [name, kind @ ("local" | "external" | "missing")] => Command::Layout(
                LayoutCommand::ExpectCell(name.to_string(), Some(kind.to_string())),
            ),
            _ => return Err("usage: expect_cell <name> [local|external|missing]".into()),
        },
        "expect_shown" => Command::Layout(LayoutCommand::ExpectShown(text()?)),
        "properties" => Command::Layout(LayoutCommand::Properties),
        "properties_apply" => Command::Layout(LayoutCommand::PropertiesApply(false)),
        "properties_ok" => Command::Layout(LayoutCommand::PropertiesApply(true)),
        "set_property" | "expect_property" => match args.as_slice() {
            [field, _, ..] => {
                let value = rest[field.len()..].trim().to_string();
                Command::Layout(if name == "set_property" {
                    LayoutCommand::SetProperty(field.to_string(), value)
                } else {
                    LayoutCommand::ExpectProperty(field.to_string(), value)
                })
            }
            _ => return Err(format!("usage: {name} <field> <value>")),
        },
        "place_library" => match args.as_slice() {
            [library, cell, ..] => {
                let after = rest[rest.find(cell).unwrap_or(0) + cell.len()..].trim();
                let values = if after.is_empty() {
                    serde_json::json!({})
                } else {
                    serde_json::from_str(after).map_err(|e| format!("the values: {e}"))?
                };
                Command::Layout(LayoutCommand::PlaceLibrary(
                    library.to_string(),
                    cell.to_string(),
                    values,
                ))
            }
            _ => return Err("usage: place_library <library> <cell> [<json values>]".into()),
        },
        // --- PCells ---
        "replace_library" => match args.as_slice() {
            [library, cell, ..] => {
                let after = rest[rest.find(cell).unwrap_or(0) + cell.len()..].trim();
                let values = if after.is_empty() {
                    serde_json::json!({})
                } else {
                    serde_json::from_str(after).map_err(|e| format!("the values: {e}"))?
                };
                Command::Layout(LayoutCommand::ReplaceLibrary(
                    library.to_string(),
                    cell.to_string(),
                    values,
                ))
            }
            _ => return Err("usage: replace_library <library> <cell> [<json values>]".into()),
        },
        "replace_view" => match args.as_slice() {
            [library, cell, view] => Command::Layout(LayoutCommand::ReplaceView(
                library.to_string(),
                cell.to_string(),
                view.to_string(),
            )),
            _ => return Err("usage: replace_view <library> <cell> <view>".into()),
        },
        // --- end PCells ---
        "expect_selection" => Command::Layout(LayoutCommand::ExpectSelection(whole(0)? as usize)),
        "expect_in_place" => {
            Command::Layout(LayoutCommand::ExpectInPlace(match text()?.as_str() {
                "none" => None,
                label => Some(label.to_string()),
            }))
        }
        "expect_modified" => match args.first().copied() {
            Some("yes") => Command::Layout(LayoutCommand::ExpectModified(true)),
            Some("no") => Command::Layout(LayoutCommand::ExpectModified(false)),
            _ => return Err("usage: expect_modified <yes|no>".into()),
        },
        "expect_mode" => Command::Layout(LayoutCommand::ExpectMode(text()?)),
        "layout_check" => match args.as_slice() {
            [check @ ("drc" | "lvs")] => Command::Layout(LayoutCommand::Check(check.to_string())),
            _ => return Err("usage: layout_check <drc|lvs>".into()),
        },
        "expect_check" => Command::Layout(LayoutCommand::ExpectCheck(text()?)),
        "check_show" => Command::Layout(LayoutCommand::CheckShow(whole(0)? as usize)),
        // --- linking with the schematic ---
        "link_generate" => match args.as_slice() {
            [] => Command::Layout(LayoutCommand::LinkGenerate(false)),
            ["remove"] => Command::Layout(LayoutCommand::LinkGenerate(true)),
            _ => return Err("usage: link_generate [remove]".into()),
        },
        "link_select" => Command::Layout(LayoutCommand::LinkSelect(
            args.iter().map(|a| a.to_string()).collect(),
        )),
        "expect_link" => match args.as_slice() {
            [name, status] => Command::Layout(LayoutCommand::ExpectLink(
                name.to_string(),
                status.to_string(),
            )),
            _ => return Err("usage: expect_link <instance> <status>".into()),
        },
        "expect_probed" => Command::Layout(LayoutCommand::ExpectProbed(match args.as_slice() {
            ["none"] => Vec::new(),
            names => names.iter().map(|a| a.to_string()).collect(),
        })),
        // --- pins ---
        "pin_add" => match args.as_slice() {
            [name, direction, layer, l, b, r, t] => {
                let n = |v: &str| {
                    v.parse::<f64>()
                        .map_err(|_| format!("pin_add: {v} is not a number"))
                };
                Command::Layout(LayoutCommand::PinAdd(
                    name.to_string(),
                    direction.to_string(),
                    layer.to_string(),
                    [n(l)?, n(b)?, n(r)?, n(t)?],
                ))
            }
            _ => return Err("usage: pin_add <name> <direction> <layer> <l> <b> <r> <t>".into()),
        },
        "pin_from_selection" => match args.as_slice() {
            [name, direction] => Command::Layout(LayoutCommand::PinFromSelection(
                name.to_string(),
                direction.to_string(),
            )),
            _ => return Err("usage: pin_from_selection <name> <direction>".into()),
        },
        "pin_draw" => match args.as_slice() {
            [name, direction] => Command::Layout(LayoutCommand::PinDraw(
                name.to_string(),
                direction.to_string(),
            )),
            _ => return Err("usage: pin_draw <name> <direction>".into()),
        },
        "pin_remove" => Command::Layout(LayoutCommand::PinRemove(text()?)),
        "pin_rename" => match args.as_slice() {
            [name, new] => {
                Command::Layout(LayoutCommand::PinRename(name.to_string(), new.to_string()))
            }
            _ => return Err("usage: pin_rename <name> <new name>".into()),
        },
        "pin_show" => Command::Layout(LayoutCommand::PinShow(text()?)),
        "expect_pin" => match args.split_first() {
            Some((name, status)) if !status.is_empty() => {
                Command::Layout(LayoutCommand::ExpectPin(name.to_string(), status.join(" ")))
            }
            _ => return Err("usage: expect_pin <name> <status>".into()),
        },
        "pins_panel" => match args.as_slice() {
            ["on"] => Command::Layout(LayoutCommand::PinsPanel(true)),
            ["off"] => Command::Layout(LayoutCommand::PinsPanel(false)),
            _ => return Err("usage: pins_panel <on|off>".into()),
        },
        // --- nets ---
        "nets_update" => Command::Layout(LayoutCommand::NetsUpdate),
        "expect_nets" => Command::Layout(LayoutCommand::ExpectNets(text()?)),
        "nets_show" => Command::Layout(LayoutCommand::NetsShow(text()?)),
        "nets_probe" => match args.as_slice() {
            ["center"] => Command::Layout(LayoutCommand::NetsProbe(None)),
            [x, y] => Command::Layout(LayoutCommand::NetsProbe(Some((
                x.parse()
                    .map_err(|_| "usage: nets_probe <x> <y> | center")?,
                y.parse()
                    .map_err(|_| "usage: nets_probe <x> <y> | center")?,
            )))),
            _ => return Err("usage: nets_probe <x> <y> | center".into()),
        },
        "expect_net_probed" => {
            Command::Layout(LayoutCommand::ExpectNetProbed(match args.as_slice() {
                ["none"] => Vec::new(),
                names => names.iter().map(|a| a.to_string()).collect(),
            }))
        }
        "flylines" => match args.as_slice() {
            ["on"] => Command::Layout(LayoutCommand::Flylines(true)),
            ["off"] => Command::Layout(LayoutCommand::Flylines(false)),
            _ => return Err("usage: flylines <on|off>".into()),
        },
        "expect_flylines" => Command::Layout(LayoutCommand::ExpectFlylines(whole(0)? as usize)),
        "flyline_style" => match args.as_slice() {
            [key, value] => Command::Layout(LayoutCommand::FlylineStyle(
                key.to_string(),
                value.to_string(),
            )),
            _ => return Err("usage: flyline_style <key> <value>".into()),
        },
        "expect_flyline_style" => match args.as_slice() {
            [key, value] => Command::Layout(LayoutCommand::ExpectFlylineStyle(
                key.to_string(),
                value.to_string(),
            )),
            _ => return Err("usage: expect_flyline_style <key> <value>".into()),
        },
        "parallel" => match args.as_slice() {
            [n, on @ ("on" | "off")] => Command::Layout(LayoutCommand::Parallel(
                n.parse().map_err(|_| "usage: parallel <n> <on|off>")?,
                *on == "on",
            )),
            _ => return Err("usage: parallel <n> <on|off>".into()),
        },
        "expect_parallel" => {
            Command::Layout(LayoutCommand::ExpectParallel(match args.as_slice() {
                ["none"] => Vec::new(),
                groups => groups.iter().map(|a| a.to_string()).collect(),
            }))
        }
        "expect_schematic_nets" => Command::ExpectSchematicNets(match args.as_slice() {
            ["none"] => Vec::new(),
            names => names.iter().map(|a| a.to_string()).collect(),
        }),
        other => match crate::library_dialog::script_parse(other, &args, rest) {
            Some(command) => Command::Definitions(command?),
            None if crate::refactor::script_parse(other, &args, rest).is_some() => {
                let parsed = crate::refactor::script_parse(other, &args, rest);
                Command::Refactor(parsed.expect("checked above")?)
            }
            None => match crate::background::script::parse(other, rest)? {
                Some(command) => Command::Disk(command),
                None => return Err(format!("unknown command '{other}'")),
            },
        },
    };
    Ok(Some(command))
}

impl Script {
    /// Read a script. `failed` is set when a command fails, so that the program can
    /// end with an error.
    pub fn parse(text: &str, failed: Arc<AtomicBool>) -> Result<Script, String> {
        let mut commands = VecDeque::new();
        for (n, line) in text.lines().enumerate() {
            match parse_line(line) {
                Ok(Some(command)) => commands.push_back((n + 1, command)),
                Ok(None) => {}
                Err(error) => return Err(format!("line {}: {error}", n + 1)),
            }
        }
        Ok(Script {
            commands,
            steps: VecDeque::new(),
            waiting: Waiting::Nothing,
            line: 0,
            settle: false,
            pointer: (0.0, 0.0),
            give_up: None,
            nudge: Instant::now(),
            failed,
            finished: false,
            clipboard: Default::default(),
        })
    }

    /// Before egui sees the input of a frame: drop what the real mouse and keyboard
    /// did, and add the next scripted events.
    pub fn hook(&mut self, app: &CdeptApp, input: &mut RawInput) {
        if self.finished {
            return;
        }
        input.events.retain(|event| {
            !matches!(
                event,
                Event::PointerMoved(_)
                    | Event::MouseMoved(_)
                    | Event::PointerButton { .. }
                    | Event::PointerGone
                    | Event::MouseWheel { .. }
                    | Event::Key { .. }
                    | Event::ModifiersChanged(_)
                    | Event::Text(_)
                    | Event::Copy
                    | Event::Cut
                    | Event::Paste(_)
            )
        });
        // Keys reach widgets only in a focused window, whatever the desktop thinks.
        input.focused = true;
        let step = match self.waiting {
            Waiting::Nothing => self.steps.pop_front(),
            _ => None,
        };
        let Some(Step::Events(events, modifiers)) = step else {
            input.events.push(Event::ModifiersChanged(Modifiers::NONE));
            return;
        };
        // One step at a time, as slowly as a hand: xschem answers before the next.
        self.settle = true;
        {
            input.events.push(Event::ModifiersChanged(modifiers));
            for event in events {
                input.events.push(match event {
                    Event::PointerMoved(pos) => Event::PointerMoved(app.script_screen(pos)),
                    Event::PointerButton {
                        pos,
                        button,
                        pressed,
                        modifiers,
                    } => Event::PointerButton {
                        pos: app.script_screen(pos),
                        button,
                        pressed,
                        modifiers,
                    },
                    other => other,
                });
            }
        }
    }

    fn fail(&mut self, app: &mut CdeptApp, message: &str) {
        tracing::error!(target: "cdept::script", "line {}: {message}", self.line);
        self.failed.store(true, Ordering::SeqCst);
        self.finished = true;
        app.script_fail();
    }

    /// After a frame: finish what was waited for, then start the next command.
    pub fn advance(&mut self, app: &mut CdeptApp, ctx: &Context) {
        if self.finished {
            return;
        }
        // The script moves on with the frames, not with the user's input.
        ctx.request_repaint();
        match self.wait(app, ctx) {
            Ok(true) => {}
            Ok(false) => return,
            Err(message) => return self.fail(app, &message),
        }
        if std::mem::take(&mut self.settle)
            && let Some(view) = app.script_view()
        {
            let id = view.sync_request();
            self.nudge = Instant::now() + NUDGE;
            self.waiting = Waiting::Reply(id, Instant::now() + TIMEOUT, Answer::Settle);
            return;
        }
        if !self.steps.is_empty() {
            return;
        }
        let Some((line, command)) = self.commands.pop_front() else {
            self.finished = true;
            tracing::info!(target: "cdept::script", "end of the script");
            return;
        };
        self.line = line;
        if self.give_up.is_none() {
            tracing::debug!(target: "cdept::script", "line {line}: {command:?}");
        }
        match self.start(app, ctx, command.clone()) {
            Ok(()) => self.give_up = None,
            Err(message) => {
                let give_up = *self.give_up.get_or_insert(Instant::now() + command.patience());
                if command.patient() && Instant::now() < give_up {
                    // Not yet: the same command again in the next frame.
                    self.commands.push_front((line, command));
                } else {
                    self.fail(app, &message);
                }
            }
        }
    }

    /// Whether what the script waits for has happened.
    fn wait(&mut self, app: &mut CdeptApp, ctx: &Context) -> Result<bool, String> {
        let late = |deadline: &Instant, what: &str| -> Result<bool, String> {
            if Instant::now() > *deadline {
                Err(format!("timed out waiting for {what}"))
            } else {
                Ok(false)
            }
        };
        let done = match &mut self.waiting {
            Waiting::Nothing => true,
            Waiting::Until(when) => Instant::now() >= *when,
            Waiting::Frames(left) => {
                *left = left.saturating_sub(1);
                *left == 0
            }
            Waiting::Frame(deadline) => {
                let view = app.script_view().ok_or("no document is open")?;
                if let Some(reason) = view.failure() {
                    return Err(format!("the engine stopped: {reason}"));
                }
                view.has_frame() || late(deadline, "a frame")?
            }
            Waiting::Reply(id, deadline, answer) => {
                let settling = matches!(answer, Answer::Settle);
                let Some(view) = app.script_view() else {
                    // The step closed the document: nothing left to wait for.
                    if settling {
                        self.waiting = Waiting::Frames(SETTLE_FRAMES);
                        return Ok(false);
                    }
                    return Err("no document is open".into());
                };
                let gone = settling && !view.is_running();
                if Instant::now() >= self.nudge {
                    self.nudge = Instant::now() + NUDGE;
                    if let Some(schematic) = view.as_schematic_mut() {
                        schematic.tcl("expr 1");
                    }
                }
                match view.take_reply(*id) {
                    None if gone => true,
                    None => late(deadline, "the helper's answer")?,
                    Some(result) => {
                        match (answer.clone(), result) {
                            (Answer::Settle, _) => {}
                            (Answer::Log, Ok(value)) => {
                                tracing::info!(target: "cdept::script", "tcl: {value}");
                            }
                            (Answer::Log, Err(error)) => {
                                tracing::warn!(target: "cdept::script", "tcl error: {error}");
                            }
                            (Answer::Expect(wanted), Ok(value)) if value == wanted => {}
                            (Answer::Expect(wanted), Ok(value)) => {
                                return Err(format!(
                                    "expected '{wanted}', xschem answered '{value}'"
                                ));
                            }
                            (Answer::Expect(_), Err(error)) | (Answer::Click(_), Err(error)) => {
                                return Err(format!("tcl error: {error}"));
                            }
                            (Answer::Click(button), Ok(value)) => {
                                let mut at = value.split_whitespace().map(str::parse::<f32>);
                                let (Some(Ok(x)), Some(Ok(y))) = (at.next(), at.next()) else {
                                    return Err(format!("no position in '{value}'"));
                                };
                                self.click(x.round(), y.round(), button);
                            }
                        }
                        self.waiting = Waiting::Frames(SETTLE_FRAMES);
                        return Ok(false);
                    }
                }
            }
            Waiting::Done(id, deadline) => {
                let view = app.script_layout().ok_or("no layout is open")?;
                match view.take_done(*id) {
                    None => late(deadline, "KLayout's answer")?,
                    Some(Ok(value)) => {
                        if !value.is_empty() {
                            tracing::info!(target: "cdept::script", "done: {value}");
                        }
                        true
                    }
                    Some(Err(error)) => return Err(format!("KLayout answered: {error}")),
                }
            }
            Waiting::Screenshot(path, deadline) => {
                let path = path.clone();
                let image = ctx.input(|i| {
                    i.events.iter().find_map(|event| match event {
                        Event::Screenshot { image, .. } => Some(image.clone()),
                        _ => None,
                    })
                });
                match (image, path) {
                    (None, _) => late(deadline, "the screenshot")?,
                    (Some(image), None) => {
                        let view = app.script_view().ok_or("no document is open")?;
                        view.compare_with_screen(&image)?;
                        tracing::info!(target: "cdept::script", "the picture on screen is the frame");
                        true
                    }
                    (Some(image), Some(path)) => {
                        let bytes: Vec<u8> =
                            image.pixels.iter().flat_map(|c| c.to_array()).collect();
                        let [width, height] = image.size;
                        image::save_buffer(
                            &path,
                            &bytes,
                            width as u32,
                            height as u32,
                            image::ColorType::Rgba8,
                        )
                        .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
                        tracing::info!(target: "cdept::script", "screenshot: {}", path.display());
                        true
                    }
                }
            }
        };
        if done {
            self.waiting = Waiting::Nothing;
        }
        Ok(done)
    }

    fn ask(&mut self, app: &mut CdeptApp, script: &str, answer: Answer) {
        if let Some(view) = app.script_schematic() {
            let id = view.tcl(script);
            self.nudge = Instant::now() + NUDGE;
            self.waiting = Waiting::Reply(id, Instant::now() + TIMEOUT, answer);
        }
    }

    fn events(&mut self, events: Vec<Event>, modifiers: Modifiers) {
        for event in &events {
            if let Event::PointerMoved(pos) | Event::PointerButton { pos, .. } = event {
                self.pointer = (pos.x, pos.y);
            }
        }
        self.steps.push_back(Step::Events(events, modifiers));
        self.settle = true;
    }

    /// Move to a frame position, press and release there.
    fn click(&mut self, x: f32, y: f32, button: PointerButton) {
        self.click_with(x, y, button, Modifiers::NONE);
    }

    /// A click with modifier keys held down meanwhile.
    fn click_with(&mut self, x: f32, y: f32, button: PointerButton, modifiers: Modifiers) {
        let press = |pressed| Event::PointerButton {
            pos: Pos2::new(x, y),
            button,
            pressed,
            modifiers,
        };
        self.events(vec![Event::PointerMoved(Pos2::new(x, y))], modifiers);
        self.events(vec![press(true)], modifiers);
        self.events(vec![press(false)], modifiers);
        if modifiers != Modifiers::NONE {
            self.events(Vec::new(), Modifiers::NONE);
        }
    }

    /// The commands for a layout document.
    fn layout_command(&mut self, app: &mut CdeptApp, command: LayoutCommand) -> Result<(), String> {
        let wrong = |what: &str| format!("{what}: no layout is open");
        if let LayoutCommand::Place(selection) = &command {
            let id = app.script_place(selection)?;
            self.waiting = Waiting::Done(id, Instant::now() + TIMEOUT);
            self.settle = true;
            return Ok(());
        }
        let view = app.script_layout().ok_or_else(|| wrong("layout command"))?;
        match command {
            LayoutCommand::Mode(name) => view.set_mode(&name),
            LayoutCommand::Zoom(what) => view.zoom(&what),
            LayoutCommand::Edit(method) => view.simple(&method),
            LayoutCommand::Save => {
                let id = view.save();
                self.waiting = Waiting::Done(id, Instant::now() + TIMEOUT);
            }
            LayoutCommand::Place(_) => {}
            LayoutCommand::Export(out) => {
                let id = view.export(&out);
                self.waiting = Waiting::Done(id, Instant::now() + TIMEOUT);
            }
            LayoutCommand::AddLayer(layer, datatype, name) => {
                view.add_layer(layer, datatype, &name);
            }
            LayoutCommand::CurrentLayer(name) => {
                let id = view
                    .find_layer(&name)
                    .map(|l| l.id)
                    .ok_or(format!("there is no layer '{name}'"))?;
                view.set_current_layer(id);
            }
            LayoutCommand::LayerVisible(name, visible) => {
                let id = view
                    .find_layer(&name)
                    .map(|l| l.id)
                    .ok_or(format!("there is no layer '{name}'"))?;
                view.set_layer_visible(id, visible);
            }
            LayoutCommand::ShowCell(path) => view.show_cell(&path),
            LayoutCommand::CellOpen(name) => view.open_cell(&name)?,
            LayoutCommand::Levels(max) => view.set_levels(max),
            LayoutCommand::Call(method, params) => {
                let id = view.raw_call(&method, params);
                self.nudge = Instant::now() + NUDGE;
                self.waiting = Waiting::Reply(id, Instant::now() + TIMEOUT, Answer::Log);
            }
            LayoutCommand::ExpectNotice(text) => {
                view.nudge();
                if !view.has_notice(&text) {
                    return Err(format!("no notice mentions '{text}'"));
                }
            }
            LayoutCommand::ExpectLayer(text) => {
                if view.find_layer(&text).is_none() {
                    return Err(format!("there is no layer '{text}'"));
                }
            }
            LayoutCommand::ExpectCell(name, kind) => match (view.cell_kind(&name), kind) {
                (None, _) => return Err(format!("there is no cell '{name}'")),
                (Some(found), Some(wanted)) if found != wanted => {
                    return Err(format!("cell '{name}' is {found}, expected {wanted}"));
                }
                _ => {}
            },
            LayoutCommand::ExpectShown(wanted) => match view.shown_cell() {
                Some(shown) if shown == wanted => {}
                shown => {
                    return Err(format!(
                        "the tab shows cell {}, expected '{wanted}'",
                        shown.map_or("(none)".to_string(), |s| format!("'{s}'"))
                    ));
                }
            },
            LayoutCommand::ExpectSelection(wanted) => {
                let count = view.selection_count();
                if count != wanted {
                    return Err(format!("{count} objects are selected, expected {wanted}"));
                }
            }
            LayoutCommand::ExpectModified(wanted) => {
                view.nudge();
                if view.is_modified() != wanted {
                    return Err(format!(
                        "the layout is {}modified",
                        if wanted { "not " } else { "" }
                    ));
                }
            }
            LayoutCommand::ExpectInPlace(wanted) => {
                let found = view.in_place().map(|c| c.label.clone());
                if found != wanted {
                    return Err(format!(
                        "editing in place: {}, expected {}",
                        found.as_deref().unwrap_or("none"),
                        wanted.as_deref().unwrap_or("none")
                    ));
                }
            }
            LayoutCommand::Properties => view.open_properties(),
            LayoutCommand::PropertiesApply(close) => view.apply_properties(close)?,
            LayoutCommand::SetProperty(field, value) => {
                view.set_properties_field(&field, &value)?
            }
            LayoutCommand::ExpectProperty(field, wanted) => {
                let form = view
                    .properties_form()
                    .ok_or("the properties are not shown")?;
                let mut at = &form;
                for part in field.split('.') {
                    at = match part.parse::<usize>() {
                        Ok(n) => at.get(n),
                        Err(_) => at.get(part),
                    }
                    .ok_or(format!("the form has no field '{field}'"))?;
                }
                let wanted_value: serde_json::Value =
                    serde_json::from_str(&wanted).unwrap_or_else(|_| serde_json::json!(wanted));
                let same = match (at.as_f64(), wanted_value.as_f64()) {
                    (Some(a), Some(b)) => (a - b).abs() < 1e-9,
                    _ => *at == wanted_value,
                };
                if !same {
                    return Err(format!("{field} is {at}, expected {wanted}"));
                }
            }
            LayoutCommand::PlaceLibrary(library, cell, values) => {
                let values = values.as_object().cloned().unwrap_or_default();
                view.place_library(&library, &cell, values);
            }
            LayoutCommand::ReplaceLibrary(library, cell, values) => {
                let values = values.as_object().cloned().unwrap_or_default();
                view.replace_shown(crate::views::Replacement::Library {
                    library,
                    cell,
                    values,
                })?;
            }
            LayoutCommand::ReplaceView(library, cell, name) => {
                let file = view
                    .project_file(&library, &cell, &name)
                    .ok_or(format!("there is no view {library}/{cell}/{name}"))?;
                view.replace_shown(crate::views::Replacement::View { cell, file })?;
            }
            LayoutCommand::Check(check) => view.run_check(if check == "lvs" {
                libcdept::klayout::verify::Check::Lvs
            } else {
                libcdept::klayout::verify::Check::Drc
            }),
            LayoutCommand::CheckShow(n) => view.show_check_item(n)?,
            LayoutCommand::LinkGenerate(remove) => view.request_generate(remove),
            LayoutCommand::LinkSelect(names) => view.link_select(&names),
            LayoutCommand::ExpectLink(name, wanted) => {
                let found = view.link_status(&name);
                if view.link_busy() || found != Some(wanted.as_str()) {
                    return Err(format!(
                        "the Link panel says {name} is {}, expected {wanted}",
                        found.unwrap_or("not in the schematic")
                    ));
                }
            }
            LayoutCommand::ExpectProbed(wanted) => {
                if view.link_probed() != wanted.as_slice() {
                    return Err(format!(
                        "the layout's selection is {:?}, expected {wanted:?}",
                        view.link_probed()
                    ));
                }
            }
            LayoutCommand::PinAdd(name, direction, layer, b) => {
                view.pin_add(&name, &direction, &layer, b)
            }
            LayoutCommand::PinFromSelection(name, direction) => {
                view.pin_from_selection(&name, &direction)
            }
            LayoutCommand::PinDraw(name, direction) => view.pin_draw(&name, &direction),
            LayoutCommand::PinRemove(name) => view.pin_remove(&name),
            LayoutCommand::PinRename(name, new) => view.pin_rename(&name, &new),
            LayoutCommand::PinShow(name) => view.pin_show(&name),
            LayoutCommand::ExpectPin(name, wanted) => {
                let found = view.pin_status(&name);
                if view.pins_busy() || found != wanted {
                    return Err(format!(
                        "the Pins panel says {name} is {found}, expected {wanted}"
                    ));
                }
            }
            LayoutCommand::PinsPanel(on) => view.show_pins_panel(on),
            LayoutCommand::NetsUpdate => view.update_connectivity(),
            LayoutCommand::ExpectNets(text) => {
                let status = view.nets_status();
                if status == "running" || !status.contains(&text) {
                    return Err(format!("the nets: '{status}', expected '{text}'"));
                }
            }
            LayoutCommand::NetsShow(name) => view.show_net(&name)?,
            LayoutCommand::NetsProbe(at) => {
                let (x, y) = match at {
                    Some(at) => at,
                    None => {
                        let [w, h] = view.frame_size().ok_or("the layout has no picture yet")?;
                        (f64::from(w / 2), f64::from(h / 2))
                    }
                };
                view.probe_net_at(x, y);
            }
            LayoutCommand::ExpectNetProbed(wanted) => {
                if view.nets_probed() != wanted.as_slice() {
                    return Err(format!(
                        "the nets probed in the layout are {:?}, expected {wanted:?}",
                        view.nets_probed()
                    ));
                }
            }
            LayoutCommand::Flylines(on) => view.set_flylines(on),
            LayoutCommand::ExpectFlylines(n) => {
                if view.flylines_drawn() != n {
                    return Err(format!(
                        "{} flylines are drawn, expected {n}",
                        view.flylines_drawn()
                    ));
                }
            }
            LayoutCommand::FlylineStyle(key, value) => view.set_flyline_style(&key, &value)?,
            LayoutCommand::ExpectFlylineStyle(key, value) => {
                let sent = view
                    .flyline_style_sent()
                    .ok_or("no flylines were drawn yet")?;
                let have = match key.as_str() {
                    "color" => sent.color.clone(),
                    "highlight" => sent.highlight.clone(),
                    "short" => sent.short.clone(),
                    "width" => sent.width.to_string(),
                    "opacity" => sent.opacity.to_string(),
                    "delay_ms" => sent.delay_ms.to_string(),
                    "line" => sent.line.name().to_string(),
                    "live" => (if sent.live { "on" } else { "off" }).to_string(),
                    other => return Err(format!("no flyline setting {other}")),
                };
                if have != value {
                    return Err(format!("the flylines' {key} is {have}, expected {value}"));
                }
            }
            LayoutCommand::Parallel(n, on) => {
                if n == 0 || n > view.parallel_groups().len() {
                    return Err(format!("there is no parallel group {n}"));
                }
                view.join_parallel(n - 1, on);
            }
            LayoutCommand::ExpectParallel(wanted) => {
                let have: Vec<String> = view
                    .parallel_groups()
                    .iter()
                    .map(|(members, joined)| {
                        format!("{}{}", members.join(","), if *joined { "" } else { "!" })
                    })
                    .collect();
                if have != wanted {
                    return Err(format!(
                        "the parallel groups are {have:?}, expected {wanted:?}"
                    ));
                }
            }
            LayoutCommand::ExpectCheck(text) => {
                let status = view.check_status();
                if status.starts_with("running") || !status.contains(&text) {
                    return Err(format!("the check: '{status}', expected '{text}'"));
                }
            }
            LayoutCommand::ExpectMode(wanted) => {
                if view.mode() != wanted {
                    return Err(format!(
                        "the mode is '{}', expected '{wanted}'",
                        view.mode()
                    ));
                }
            }
        }
        self.settle = true;
        Ok(())
    }

    fn start(&mut self, app: &mut CdeptApp, ctx: &Context, command: Command) -> Result<(), String> {
        let press = |x: f32, y: f32, button, pressed| Event::PointerButton {
            pos: Pos2::new(x, y),
            button,
            pressed,
            modifiers: Modifiers::NONE,
        };
        let none = Modifiers::NONE;
        match command {
            Command::Open(selection) => app.script_request(Action::OpenView(selection)),
            Command::Close => {
                let tab = app.script_document().ok_or("no document is open")?;
                app.script_request(Action::CloseTab(tab));
            }
            Command::CloseTab => {
                let asking = app.script_close_tab().ok_or("no document is open")?;
                if asking {
                    tracing::info!(target: "cdept::script", "asking about unsaved changes");
                }
                self.settle = app.script_view().is_some();
            }
            Command::CloseAnswer(choice) => {
                if !app.script_close_with(choice) {
                    return Err("nothing is asking about unsaved changes".into());
                }
                self.settle = choice == CloseChoice::Save;
            }
            Command::ExpectTitle(wanted) => {
                let title = app.script_title().ok_or("no document is open")?;
                if title != wanted {
                    return Err(format!("the tab is titled '{title}', expected '{wanted}'"));
                }
            }
            Command::Detach => {
                let tab = app.script_document().ok_or("no document is open")?;
                app.script_request(Action::DetachTab(tab));
            }
            Command::Wait(seconds) => {
                self.waiting = Waiting::Until(Instant::now() + Duration::from_secs_f32(seconds));
            }
            Command::WaitFrame => {
                // A document is drawn, and asks for its frame, once its tab is in front.
                let tab = app.script_document().ok_or("no document is open")?;
                app.script_request(Action::OpenTab(tab));
                self.waiting = Waiting::Frame(Instant::now() + TIMEOUT);
            }
            Command::Sync => {
                app.script_view().ok_or("no document is open")?;
                self.settle = true;
            }
            Command::Move(x, y) => self.events(vec![Event::PointerMoved(Pos2::new(x, y))], none),
            Command::MoveBy(dx, dy) => {
                let at = Pos2::new(self.pointer.0 + dx, self.pointer.1 + dy);
                self.events(vec![Event::PointerMoved(at)], none);
            }
            Command::Click(x, y, button, modifiers) => self.click_with(x, y, button, modifiers),
            Command::DoubleClick(x, y) => {
                self.click(x, y, PointerButton::Primary);
                self.click(x, y, PointerButton::Primary);
            }
            Command::ClickHere(button) => self.click(self.pointer.0, self.pointer.1, button),
            Command::ClickInstance(name, button) => {
                app.script_schematic().ok_or("no schematic is open")?;
                // The middle of the instance, from xschem's own numbers.
                let script = format!(
                    "regexp {{Instance: (\\S+) (\\S+) (\\S+) (\\S+)}} \\
                     [xschem instance_bbox {{{name}}}] -> x1 y1 x2 y2; \\
                     set z [xschem get zoom]; \\
                     list [expr {{(($x1 + $x2) / 2.0 + [xschem get xorigin]) / $z}}] \\
                     [expr {{(($y1 + $y2) / 2.0 + [xschem get yorigin]) / $z}}]"
                );
                self.ask(app, &script, Answer::Click(button));
            }
            Command::Press(x, y, button) => {
                self.events(vec![Event::PointerMoved(Pos2::new(x, y))], none);
                self.events(vec![press(x, y, button, true)], none);
            }
            Command::Release(x, y, button) => {
                self.events(vec![Event::PointerMoved(Pos2::new(x, y))], none);
                self.events(vec![press(x, y, button, false)], none);
            }
            Command::Drag([x1, y1, x2, y2], button) => {
                self.events(vec![Event::PointerMoved(Pos2::new(x1, y1))], none);
                self.events(vec![press(x1, y1, button, true)], none);
                for n in 1..=DRAG_STEPS {
                    let t = n as f32 / DRAG_STEPS as f32;
                    let at = Pos2::new(x1 + (x2 - x1) * t, y1 + (y2 - y1) * t);
                    self.events(vec![Event::PointerMoved(at)], none);
                }
                self.events(vec![press(x2, y2, button, false)], none);
            }
            Command::Wheel(x, y, notch, count) => {
                self.events(vec![Event::PointerMoved(Pos2::new(x, y))], none);
                for _ in 0..count {
                    let wheel = Event::MouseWheel {
                        unit: MouseWheelUnit::Line,
                        delta: Vec2::new(0.0, notch),
                        phase: TouchPhase::Move,
                        modifiers: none,
                    };
                    self.events(vec![wheel], none);
                }
            }
            Command::Key(key, modifiers) => {
                let event = |pressed| Event::Key {
                    key,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers,
                };
                // What the window system layer makes of the key before egui sees it.
                let command = modifiers.command || modifiers.ctrl;
                let mut down = match (command, key) {
                    (true, Key::C) => vec![Event::Copy],
                    (true, Key::X) => vec![Event::Cut],
                    (true, Key::V) => vec![Event::Paste(self.clipboard.0.clone())],
                    _ => vec![event(true)],
                };
                let typed = key.name().chars().next().filter(|_| key.name().len() == 1);
                if let (Some(c), false) = (typed, command) {
                    let c = if modifiers.shift {
                        c.to_ascii_uppercase()
                    } else {
                        c.to_ascii_lowercase()
                    };
                    down.push(Event::Text(c.to_string()));
                } else if key == Key::Space && !command {
                    down.push(Event::Text(" ".into()));
                }
                self.events(down, modifiers);
                self.events(vec![event(false)], modifiers);
            }
            Command::Text(text) => {
                for c in text.chars() {
                    self.events(vec![Event::Text(c.to_string())], none);
                }
            }
            Command::Tcl(script) => {
                app.script_schematic().ok_or("no schematic is open")?;
                self.ask(app, &script, Answer::Log);
            }
            Command::TclSend(script) => {
                let view = app.script_schematic().ok_or("no schematic is open")?;
                view.tcl(&script);
                // The script may have changed the selection.
                view.reread_selection();
            }
            Command::TclExpect(expected, script) => {
                app.script_schematic().ok_or("no schematic is open")?;
                self.ask(app, &script, Answer::Expect(expected));
            }
            Command::Menu(labels) => {
                let view = app.script_schematic().ok_or("no schematic is open")?;
                let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
                if !view.choose_menu(&labels) {
                    return Err(format!("no menu entry '{}'", labels.join(" / ")));
                }
                self.settle = true;
            }
            Command::Menus => {
                let view = app.script_schematic().ok_or("no schematic is open")?;
                for path in view.menu_paths() {
                    tracing::info!(target: "cdept::script", "menu: {path}");
                }
            }
            Command::DialogClick(text) => {
                let view = app.script_schematic().ok_or("no schematic is open")?;
                if !view.press_dialog_button(&text) {
                    return Err(format!("no button '{text}' in an open dialog"));
                }
                self.settle = true;
            }
            Command::DialogType(target, text) => {
                let view = app.script_schematic().ok_or("no schematic is open")?;
                view.type_in_dialog(&target, &text)?;
                self.settle = true;
            }
            Command::Clipboard(text) => {
                self.clipboard.0 = text;
                crate::views::pretend_clipboard(
                    Some(self.clipboard.0.clone()),
                    Some(self.clipboard.1.clone()),
                );
            }
            Command::Primary(text) => {
                self.clipboard.1 = text;
                crate::views::pretend_clipboard(
                    Some(self.clipboard.0.clone()),
                    Some(self.clipboard.1.clone()),
                );
            }
            Command::ExpectPane(path) => {
                let view = app.script_schematic().ok_or("no schematic is open")?;
                let (size, asked, colours) = view
                    .dialog_pane(&path)
                    .ok_or(format!("no dialog shows a picture in '{path}'"))?;
                if asked.is_some_and(|asked| asked != size) {
                    return Err(format!(
                        "the picture in '{path}' is {size:?}, not {asked:?}"
                    ));
                }
                if colours <= 3 {
                    return Err(format!("the picture in '{path}' is empty"));
                }
                tracing::info!(target: "cdept::script", "pane {path}: {} x {}", size[0], size[1]);
            }
            Command::ExpectDialog(title) => {
                let view = app.script_schematic().ok_or("no schematic is open")?;
                let titles = view.dialog_titles();
                if !titles.iter().any(|t| t.contains(&title)) {
                    return Err(format!("no dialog '{title}' is open (open: {titles:?})"));
                }
            }
            Command::Frame => {
                let view = app.script_view().ok_or("no document is open")?;
                let [width, height] = view.frame_size().ok_or("there is no frame yet")?;
                tracing::info!(target: "cdept::script", "frame: {width} x {height}");
            }
            Command::Pixel(x, y) => {
                let view = app.script_view().ok_or("no document is open")?;
                let [r, g, b, _] = view.pixel(x, y).ok_or("no such pixel")?;
                tracing::info!(target: "cdept::script", "pixel {x} {y}: {r:02x}{g:02x}{b:02x}");
            }
            Command::ExpectPixel(x, y, wanted) => {
                let view = app.script_view().ok_or("no document is open")?;
                let [r, g, b, _] = view.pixel(x, y).ok_or("no such pixel")?;
                if [r, g, b] != wanted {
                    let [wr, wg, wb] = wanted;
                    return Err(format!(
                        "pixel {x} {y} is {r:02x}{g:02x}{b:02x}, expected {wr:02x}{wg:02x}{wb:02x}"
                    ));
                }
            }
            Command::ExpectDocuments(wanted) => {
                let open = app.script_document_count();
                if open != wanted {
                    return Err(format!("{open} documents are open, expected {wanted}"));
                }
            }
            Command::Panel(name) => {
                if !app.script_show_panel(&name) {
                    return Err(format!("there is no panel '{name}'"));
                }
            }
            Command::Select(selection) => app.script_select(selection),
            Command::ExpectHierarchy(wanted) => {
                let lines = app.script_hierarchy();
                if !lines.iter().any(|line| line.trim() == wanted.trim()) {
                    return Err(format!("no line '{wanted}' in the hierarchy: {lines:?}"));
                }
            }
            Command::HierarchyOpen(cell) => {
                let file = app
                    .script_hierarchy_file(&cell)
                    .ok_or(format!("no cell '{cell}' in the hierarchy"))?;
                app.script_request(Action::OpenFile(file));
            }
            Command::ExpectPreview => {
                if !app.script_preview() {
                    return Err("the Inspector shows no picture of the selected view".into());
                }
            }
            Command::ExpectSchematicNets(wanted) => {
                let view = app.script_schematic().ok_or("no schematic is open")?;
                if view.nets_probed() != wanted.as_slice() {
                    return Err(format!(
                        "the schematic's nets exchanged with the layout are {:?}, expected \
                         {wanted:?}",
                        view.nets_probed()
                    ));
                }
            }
            Command::ExpectSelected(wanted) => {
                let selected = app.script_selected();
                let right = match &wanted {
                    Some(name) => selected.contains(name),
                    None => selected.is_empty(),
                };
                if !right {
                    return Err(format!(
                        "the Inspector shows {selected:?} as selected, expected {}",
                        wanted.as_deref().unwrap_or("nothing")
                    ));
                }
            }
            Command::Netlist(library, cell, format) => {
                app.script_request(Action::NetlistToView {
                    library,
                    cell,
                    format,
                });
            }
            Command::ExpectView(selection) => {
                let found = app.script_project().is_some_and(|project| {
                    let names = (
                        selection.library.as_deref(),
                        selection.cell.as_deref(),
                        selection.view.as_deref(),
                    );
                    let (Some(library), Some(cell), Some(view)) = names else {
                        return false;
                    };
                    let cell = project.library(library).and_then(|l| l.cell(cell));
                    cell.is_some_and(|c| c.entry(view).is_some())
                });
                if !found {
                    let waiting = if app.script_netlisting() {
                        " (a netlist is still being written)"
                    } else {
                        ""
                    };
                    return Err(format!("there is no such view{waiting}"));
                }
            }
            Command::CloseProject => app.script_request(Action::CloseProject),
            Command::QuitAsk => app.script_request(Action::Quit),
            Command::LeaveAnswer(choice) => {
                if !app.script_leave_with(ctx, choice) {
                    return Err("nothing is asking about unsaved documents".into());
                }
            }
            Command::Definitions(command) => {
                crate::library_dialog::script_run(command, app.script_ctx(), ctx)?;
            }
            Command::Refactor(command) => app.script_refactor(command)?,
            Command::ExpectProject(wanted) => {
                let open = app.script_project().map(|p| p.name.clone());
                if open != wanted {
                    return Err(format!(
                        "the open project is {}, expected {}",
                        open.as_deref().unwrap_or("none"),
                        wanted.as_deref().unwrap_or("none")
                    ));
                }
            }
            Command::ExpectPicker(mode) => {
                let view = app.script_schematic().ok_or("no schematic is open")?;
                let open = view.picker_mode();
                if open.as_deref() != Some(mode.as_str()) {
                    return Err(format!(
                        "no library picker for '{mode}' is open (open: {open:?})"
                    ));
                }
            }
            Command::Picker(selection) => {
                let file = app
                    .script_project()
                    .and_then(|project| {
                        project
                            .library(selection.library.as_deref()?)?
                            .cell(selection.cell.as_deref()?)?
                            .entry(selection.view.as_deref()?)?
                            .main_file
                            .clone()
                    })
                    .ok_or("there is no such view")?;
                let view = app.script_schematic().ok_or("no schematic is open")?;
                if !view.answer_picker(Some(&file)) {
                    return Err("no library picker is open".into());
                }
                self.settle = true;
            }
            Command::PickerFile(file) => {
                if let Some(folder) = file.as_ref().and_then(|f| f.parent()) {
                    std::fs::create_dir_all(folder)
                        .map_err(|e| format!("cannot create {}: {e}", folder.display()))?;
                }
                let view = app.script_schematic().ok_or("no schematic is open")?;
                if !view.answer_picker(file.as_deref()) {
                    return Err("no library picker is open".into());
                }
                self.settle = true;
            }
            Command::Window(width, height) => {
                ctx.send_viewport_cmd(ViewportCommand::InnerSize(Vec2::new(width, height)));
                self.waiting = Waiting::Frames(SETTLE_FRAMES);
            }
            Command::Zoom(factor) => {
                ctx.set_zoom_factor(factor);
                self.waiting = Waiting::Frames(SETTLE_FRAMES);
            }
            Command::Screenshot(path) => {
                ctx.send_viewport_cmd(ViewportCommand::Screenshot(egui::UserData::default()));
                self.waiting = Waiting::Screenshot(Some(path), Instant::now() + TIMEOUT);
            }
            Command::CheckPicture => {
                app.script_view().ok_or("no document is open")?;
                ctx.send_viewport_cmd(ViewportCommand::Screenshot(egui::UserData::default()));
                self.waiting = Waiting::Screenshot(None, Instant::now() + TIMEOUT);
            }
            Command::Layout(command) => self.layout_command(app, command)?,
            Command::Log(text) => tracing::info!(target: "cdept::script", "{text}"),
            Command::Tech(args) => app.script_tech(ctx, &args)?,
            Command::Sim(args) => app.script_sim(&args)?,
            Command::Organise(args) => app.script_organise(ctx, &args)?,
            Command::Disk(command) => {
                let mut requests = Vec::new();
                let (shared, doc) = app.script_disk();
                crate::background::script::run(command, shared, doc, ctx, &mut requests)?;
                for action in requests {
                    app.script_request(action);
                }
            }
            Command::Quit => {
                self.finished = true;
                app.script_quit(ctx);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(line: &str) -> Result<Option<Command>, String> {
        parse_line(line)
    }

    #[test]
    fn commands_parse() {
        assert_eq!(parse("# a comment"), Ok(None));
        assert_eq!(parse("   "), Ok(None));
        assert_eq!(
            parse("open bench tb_buf schematic"),
            Ok(Some(Command::Open(Selection::view(
                "bench",
                "tb_buf",
                "schematic"
            ))))
        );
        assert_eq!(
            parse("click 10 20.5 right"),
            Ok(Some(Command::Click(
                10.0,
                20.5,
                PointerButton::Secondary,
                Modifiers::NONE
            )))
        );
        assert_eq!(
            parse("click 1 2 ctrl"),
            Ok(Some(Command::Click(
                1.0,
                2.0,
                PointerButton::Primary,
                Modifiers::CTRL | Modifiers::COMMAND
            )))
        );
        assert_eq!(
            parse("drag 1 2 3 4"),
            Ok(Some(Command::Drag(
                [1.0, 2.0, 3.0, 4.0],
                PointerButton::Primary
            )))
        );
        assert_eq!(
            parse("wheel 5 6 down 3"),
            Ok(Some(Command::Wheel(5.0, 6.0, -1.0, 3)))
        );
        assert_eq!(
            parse("key s ctrl shift"),
            Ok(Some(Command::Key(
                Key::S,
                Modifiers::CTRL | Modifiers::COMMAND | Modifiers::SHIFT
            )))
        );
        assert_eq!(
            parse("key Escape"),
            Ok(Some(Command::Key(Key::Escape, Modifiers::NONE)))
        );
        assert_eq!(
            parse("key F5"),
            Ok(Some(Command::Key(Key::F5, Modifiers::NONE)))
        );
        assert_eq!(
            parse("tcl xschem get schname"),
            Ok(Some(Command::Tcl("xschem get schname".into())))
        );
        assert_eq!(
            parse("tcl_expect {a b} set ::x"),
            Ok(Some(Command::TclExpect("a b".into(), "set ::x".into())))
        );
        assert_eq!(
            parse("menu File / Save as"),
            Ok(Some(Command::Menu(vec!["File".into(), "Save as".into()])))
        );
        assert_eq!(
            parse("dialog_type {Instance name} R5"),
            Ok(Some(Command::DialogType(
                "Instance name".into(),
                "R5".into()
            )))
        );
        assert_eq!(
            parse("expect_pixel 3 4 ffcc00"),
            Ok(Some(Command::ExpectPixel(3, 4, [0xff, 0xcc, 0x00])))
        );
        assert_eq!(
            parse("screenshot /tmp/a b.png"),
            Ok(Some(Command::Screenshot(PathBuf::from("/tmp/a b.png"))))
        );
    }

    #[test]
    fn properties_commands_parse() {
        let layout = |c| Ok(Some(Command::Layout(c)));
        assert_eq!(parse("properties"), layout(LayoutCommand::Properties));
        assert_eq!(
            parse("properties_ok"),
            layout(LayoutCommand::PropertiesApply(true))
        );
        assert_eq!(
            parse("set_property pcell.text hello world"),
            layout(LayoutCommand::SetProperty(
                "pcell.text".into(),
                "hello world".into()
            ))
        );
        assert_eq!(
            parse("expect_property bbox.2 1.5"),
            layout(LayoutCommand::ExpectProperty("bbox.2".into(), "1.5".into()))
        );
        assert_eq!(
            parse(r#"place_library Basic CIRCLE {"actual_radius": 2}"#),
            layout(LayoutCommand::PlaceLibrary(
                "Basic".into(),
                "CIRCLE".into(),
                serde_json::json!({"actual_radius": 2})
            ))
        );
        assert!(parse("set_property x").is_err());
        assert!(parse("place_library Basic").is_err());
        assert_eq!(
            parse(r#"replace_library Basic TEXT {"text": "a"}"#),
            layout(LayoutCommand::ReplaceLibrary(
                "Basic".into(),
                "TEXT".into(),
                serde_json::json!({"text": "a"})
            ))
        );
        assert_eq!(
            parse("replace_view logic inv layout"),
            layout(LayoutCommand::ReplaceView(
                "logic".into(),
                "inv".into(),
                "layout".into()
            ))
        );
        assert!(parse("replace_view logic inv").is_err());
    }

    #[test]
    fn layout_commands_parse() {
        let layout = |c| Ok(Some(Command::Layout(c)));
        assert_eq!(parse("mode box"), layout(LayoutCommand::Mode("box".into())));
        assert_eq!(
            parse("layout_zoom fit"),
            layout(LayoutCommand::Zoom("fit".into()))
        );
        assert_eq!(parse("save"), layout(LayoutCommand::Save));
        assert_eq!(
            parse("layout_check lvs"),
            layout(LayoutCommand::Check("lvs".into()))
        );
        assert_eq!(
            parse("expect_check DRC clean"),
            layout(LayoutCommand::ExpectCheck("DRC clean".into()))
        );
        assert!(parse("layout_check erc").is_err());
        assert_eq!(
            parse("place logic inv layout"),
            layout(LayoutCommand::Place(Selection::view(
                "logic", "inv", "layout"
            )))
        );
        assert_eq!(
            parse("add_layer 10 0 metal one"),
            layout(LayoutCommand::AddLayer(10, 0, "metal one".into()))
        );
        assert_eq!(
            parse("layer_visible 1/0 off"),
            layout(LayoutCommand::LayerVisible("1/0".into(), false))
        );
        assert_eq!(
            parse("show_cell top sub"),
            layout(LayoutCommand::ShowCell(vec!["top".into(), "sub".into()]))
        );
        assert_eq!(
            parse("expect_cell inv external"),
            layout(LayoutCommand::ExpectCell(
                "inv".into(),
                Some("external".into())
            ))
        );
        assert_eq!(
            parse("expect_notice select a layer"),
            layout(LayoutCommand::ExpectNotice("select a layer".into()))
        );
        assert_eq!(
            parse(r#"layout_call zoom {"what": "fit"}"#),
            layout(LayoutCommand::Call(
                "zoom".into(),
                serde_json::json!({"what": "fit"})
            ))
        );
        for bad in [
            "layout_zoom sideways",
            "layout_edit explode",
            "layer_visible 1/0 maybe",
            "expect_modified perhaps",
            "layout_call",
            "layout_call zoom {broken",
            "add_layer 1",
            "show_cell",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn mistakes_are_reported_with_the_line() {
        for bad in [
            "fly 1 2",
            "click 1",
            "click a b",
            "click 1 2 thumb",
            "key",
            "key nosuchkey",
            "key a hyper",
            "wheel 1 2 sideways",
            "expect_pixel 1 2 red",
            "open onlylibrary",
            "tcl",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
        let failed = Arc::new(AtomicBool::new(false));
        let error = Script::parse("wait 1\n\nbogus\n", failed).err().unwrap();
        assert!(error.starts_with("line 3:"), "{error}");
    }
}
