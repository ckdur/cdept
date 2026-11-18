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
use libcdept::xschem::{NetlistFormat, Status};

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
    Click(f32, f32, PointerButton),
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
    Netlist(String, String, NetlistFormat),
    ExpectView(Selection),
    CloseProject,
    QuitAsk,
    LeaveAnswer(LeaveChoice),
    ExpectProject(Option<String>),
    ExpectPicker(String),
    Picker(Selection),
    PickerFile(Option<PathBuf>),
    Window(f32, f32),
    Zoom(f32),
    Screenshot(PathBuf),
    CheckPicture,
    Log(String),
    Quit,
}

impl Command {
    /// Commands that are tried again for a while before they fail. They do nothing
    /// when they fail, so trying again is safe.
    fn patient(&self) -> bool {
        matches!(
            self,
            Command::Menu(_)
                | Command::DialogClick(_)
                | Command::DialogType(..)
                | Command::CloseAnswer(_)
                | Command::ExpectDialog(_)
                | Command::ExpectPixel(..)
                | Command::ExpectDocuments(_)
                | Command::ExpectTitle(_)
                | Command::ExpectHierarchy(_)
                | Command::HierarchyOpen(_)
                | Command::ExpectPreview
                | Command::ExpectSelected(_)
                | Command::ExpectView(_)
                | Command::LeaveAnswer(_)
                | Command::ExpectProject(_)
                | Command::ExpectPicker(_)
                | Command::Picker(_)
                | Command::PickerFile(_)
        )
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
        "click" => Command::Click(number(0)?, number(1)?, button(args.get(2).copied())?),
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
        "quit" => Command::Quit,
        other => return Err(format!("unknown command '{other}'")),
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
        if std::mem::take(&mut self.settle) && app.script_schematic().is_some() {
            return self.ask(app, "expr 1", Answer::Settle);
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
                let give_up = *self.give_up.get_or_insert(Instant::now() + PATIENCE);
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
                let view = app.script_schematic().ok_or("no schematic is open")?;
                if let Status::Failed(reason) = view.status() {
                    return Err(format!("the xschem engine stopped: {reason}"));
                }
                view.has_frame() || late(deadline, "a frame")?
            }
            Waiting::Reply(id, deadline, answer) => {
                let settling = matches!(answer, Answer::Settle);
                let Some(view) = app.script_schematic() else {
                    // The step closed the document: nothing left to wait for.
                    if settling {
                        self.waiting = Waiting::Frames(SETTLE_FRAMES);
                        return Ok(false);
                    }
                    return Err("no schematic is open".into());
                };
                let gone = settling && view.status() != Status::Running;
                if Instant::now() >= self.nudge {
                    self.nudge = Instant::now() + NUDGE;
                    view.tcl("expr 1");
                }
                match view.take_reply(*id) {
                    None if gone => true,
                    None => late(deadline, "xschem's answer")?,
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
                        let view = app.script_schematic().ok_or("no schematic is open")?;
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
        let press = |pressed| Event::PointerButton {
            pos: Pos2::new(x, y),
            button,
            pressed,
            modifiers: Modifiers::NONE,
        };
        self.events(vec![Event::PointerMoved(Pos2::new(x, y))], Modifiers::NONE);
        self.events(vec![press(true)], Modifiers::NONE);
        self.events(vec![press(false)], Modifiers::NONE);
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
                let asking = app.script_close_tab().ok_or("no schematic is open")?;
                if asking {
                    tracing::info!(target: "cdept::script", "asking about unsaved changes");
                }
                self.settle = app.script_schematic().is_some();
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
                app.script_schematic().ok_or("no schematic is open")?;
                self.settle = true;
            }
            Command::Move(x, y) => self.events(vec![Event::PointerMoved(Pos2::new(x, y))], none),
            Command::MoveBy(dx, dy) => {
                let at = Pos2::new(self.pointer.0 + dx, self.pointer.1 + dy);
                self.events(vec![Event::PointerMoved(at)], none);
            }
            Command::Click(x, y, button) => self.click(x, y, button),
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
                    (true, Key::V) => vec![Event::Paste(String::new())],
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
            Command::ExpectDialog(title) => {
                let view = app.script_schematic().ok_or("no schematic is open")?;
                let titles = view.dialog_titles();
                if !titles.iter().any(|t| t.contains(&title)) {
                    return Err(format!("no dialog '{title}' is open (open: {titles:?})"));
                }
            }
            Command::Frame => {
                let view = app.script_schematic().ok_or("no schematic is open")?;
                let [width, height] = view.frame_size().ok_or("there is no frame yet")?;
                tracing::info!(target: "cdept::script", "frame: {width} x {height}");
            }
            Command::Pixel(x, y) => {
                let view = app.script_schematic().ok_or("no schematic is open")?;
                let [r, g, b, _] = view.pixel(x, y).ok_or("no such pixel")?;
                tracing::info!(target: "cdept::script", "pixel {x} {y}: {r:02x}{g:02x}{b:02x}");
            }
            Command::ExpectPixel(x, y, wanted) => {
                let view = app.script_schematic().ok_or("no schematic is open")?;
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
                app.script_schematic().ok_or("no schematic is open")?;
                ctx.send_viewport_cmd(ViewportCommand::Screenshot(egui::UserData::default()));
                self.waiting = Waiting::Screenshot(None, Instant::now() + TIMEOUT);
            }
            Command::Log(text) => tracing::info!(target: "cdept::script", "{text}"),
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
            Ok(Some(Command::Click(10.0, 20.5, PointerButton::Secondary)))
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
