//! One layout open in the GUI: a KLayout helper of its own, kept running.
//!
//! A [`Session`] never blocks its caller. Calls, sizes and input are queued and written
//! to the helper by a supervisor thread; frames, events and replies are read by a
//! second thread and kept until the GUI asks for them ([`Session::frame`],
//! [`Session::take_events`], [`Session::take_replies`]). `wake` is called whenever
//! there is something new to show.
//!
//! The supervisor starts the helper, sends `open` with the reference table, and starts
//! it again (opening the file again, with the latest table) when it stops
//! unexpectedly (see [`Status`]).

use std::io::BufReader;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::engine::{Connection, EngineOptions};
use super::protocol::{self, Event, Input, Message, NoticeLevel, PixelFormat, Request, method};
use super::setup;
use crate::Project;
use crate::layout::RefTable;
use crate::xschem::frame::{FrameReader, Rect};

/// Called from the session's threads when there is something new to show.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

/// A helper that stops within this time of starting counts as a failed start.
const SHORT_RUN: Duration = Duration::from_secs(5);
/// Failed starts in a row before giving up (the user can still ask for a restart).
const MAX_FAILED_STARTS: u32 = 3;
/// Ids from here on belong to the session's own calls (opening the file).
const INTERNAL_ID: u64 = 1 << 62;

pub struct SessionOptions {
    /// The `klayout` binary; found automatically when `None`.
    pub klayout: Option<PathBuf>,
    /// Working folder of the helper (the project folder).
    pub cwd: Option<PathBuf>,
    /// Extra environment, e.g. `KLAYOUT_PATH`, `PDK_ROOT` ([`setup::project_env`]).
    pub env: Vec<(std::ffi::OsString, std::ffi::OsString)>,
    /// The file and everything it references; sent with `open`, also after a restart.
    pub refs: RefTable,
    /// Open for editing (false for read-only libraries).
    pub editable: bool,
    pub wake: Wake,
}

impl SessionOptions {
    /// Options for a layout of `project`: its PDK, `KLAYOUT_PATH` and `[klayout]`
    /// settings, the project folder as working folder.
    pub fn for_project(project: &Project, refs: RefTable, editable: bool, wake: Wake) -> Self {
        SessionOptions {
            klayout: None,
            cwd: Some(project.root.clone()),
            env: setup::project_env(project),
            refs,
            editable,
            wake,
        }
    }
}

/// What the helper behind a session is doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Starting,
    Running,
    /// Stopped unexpectedly and being started again; the last frame stays on screen.
    Restarting(String),
    /// Could not be started, or keeps stopping. [`Session::restart`] tries again.
    Failed(String),
    /// The helper ended on its own accord.
    Closed,
}

/// The answer to a [`Session::call`].
#[derive(Debug, Clone, PartialEq)]
pub struct Reply {
    pub id: u64,
    pub result: Result<Value, String>,
}

/// The drawing area as last reported, handed to [`Session::frame`].
pub struct FrameView<'a> {
    reader: &'a FrameReader,
    format: PixelFormat,
    /// Changes when the frame was replaced: the whole picture has to be taken again.
    pub generation: u64,
    /// The rectangle that changed since the previous [`Session::frame`] call, if any.
    pub damage: Option<Rect>,
}

impl FrameView<'_> {
    pub fn width(&self) -> u32 {
        self.reader.width()
    }

    pub fn height(&self) -> u32 {
        self.reader.height()
    }

    /// Copy `rect` into `out` as R G B A (converted from KLayout's B G R A), `rect.width
    /// * 4` bytes per row. Returns the rectangle copied (clipped to the frame).
    pub fn copy(&self, rect: Rect, out: &mut Vec<u8>) -> Rect {
        let copied = self.reader.copy_rect(rect, out);
        if self.format == PixelFormat::Bgra {
            bgra_to_rgba(out);
        }
        copied
    }

    pub fn full(&self) -> Rect {
        Rect::new(0, 0, self.width(), self.height())
    }
}

/// Swap the first and third byte of every pixel.
fn bgra_to_rgba(pixels: &mut [u8]) {
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
}

enum Command {
    Resize(u32, u32),
    Input(Input),
    Call {
        id: u64,
        method: String,
        params: Value,
    },
    Restart,
    Close,
    /// From the reader thread of connection `epoch`: the helper is gone.
    Gone {
        epoch: u64,
        error: std::io::Error,
    },
}

#[derive(Default)]
struct State {
    status: Option<Status>,
    klayout_version: Option<String>,
    /// Counts the helpers started; messages of an older one are ignored.
    epoch: u64,
    frame: Option<FrameReader>,
    format: PixelFormat,
    frame_serial: u64,
    generation: u64,
    damage: Rect,
    events: Vec<Event>,
    replies: Vec<Reply>,
    /// Last size asked for, sent again to a restarted helper.
    size: Option<(u32, u32)>,
    /// Sent with `open` on every start.
    refs: RefTable,
    next_id: u64,
}

struct Shared {
    state: Mutex<State>,
    wake: Wake,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn set_status(&self, status: Status) {
        self.lock().status = Some(status);
        (self.wake)();
    }
}

/// One open layout and the KLayout process behind it. Dropping it ends the process.
pub struct Session {
    commands: Sender<Command>,
    shared: Arc<Shared>,
    supervisor: Option<JoinHandle<()>>,
}

impl Session {
    /// Start the helper in the background and open `options.refs.root` in it.
    pub fn open(options: SessionOptions) -> Session {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                status: Some(Status::Starting),
                refs: options.refs,
                next_id: 1,
                ..Default::default()
            }),
            wake: options.wake.clone(),
        });
        let (commands, receiver) = mpsc::channel();
        let supervisor = Supervisor {
            options: EngineOptions {
                klayout: options.klayout,
                cwd: options.cwd,
                env: options.env,
                display: true,
                ..Default::default()
            },
            editable: options.editable,
            shared: shared.clone(),
            commands: commands.clone(),
            receiver,
        };
        let handle = thread::Builder::new()
            .name("klayout-session".into())
            .spawn(move || supervisor.run())
            .expect("cannot start a thread");
        Session {
            commands,
            shared,
            supervisor: Some(handle),
        }
    }

    pub fn status(&self) -> Status {
        self.shared
            .lock()
            .status
            .clone()
            .unwrap_or(Status::Starting)
    }

    /// Counts helper starts; changes after a restart (state shown must be re-read).
    pub fn epoch(&self) -> u64 {
        self.shared.lock().epoch
    }

    /// `KLayout 0.30.12`, once the helper is ready.
    pub fn klayout_version(&self) -> Option<String> {
        self.shared.lock().klayout_version.clone()
    }

    /// The drawing area has this size now, in pixels. A new frame follows.
    pub fn resize(&self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.shared.lock().size = Some((width, height));
        self.send(Command::Resize(width, height));
    }

    /// Pointer or keyboard input in the drawing area (positions in frame pixels).
    pub fn input(&self, input: Input) {
        self.send(Command::Input(input));
    }

    /// Call a helper method (docs/KLAYOUT.md §4); the answer comes back through
    /// [`Session::take_replies`] with the returned id. Calls made while the helper is
    /// not running get no answer.
    pub fn call(&self, method: &str, params: Value) -> u64 {
        let id = {
            let mut state = self.shared.lock();
            let id = state.next_id;
            state.next_id += 1;
            id
        };
        self.send(Command::Call {
            id,
            method: method.to_string(),
            params,
        });
        id
    }

    /// Replace the reference table used on restarts (after `place` added a file).
    pub fn set_refs(&self, refs: RefTable) {
        self.shared.lock().refs = refs;
    }

    /// Stop the helper, if it runs, and start a fresh one with the file opened again.
    /// Unsaved changes are lost.
    pub fn restart(&self) {
        self.send(Command::Restart);
    }

    /// Look at the drawing area. `None` while no frame has arrived. The damage is
    /// handed out once: a caller that sees `damage` must copy it.
    pub fn frame<R>(&self, look: impl FnOnce(FrameView<'_>) -> R) -> Option<R> {
        let mut state = self.shared.lock();
        let damage = std::mem::take(&mut state.damage);
        let generation = state.generation;
        let format = state.format;
        let reader = state.frame.as_ref()?;
        Some(look(FrameView {
            reader,
            format,
            generation,
            damage: (!damage.is_empty()).then_some(damage),
        }))
    }

    /// Events since the last call, oldest first. Of the kinds that describe a state
    /// (all but notices) only the latest is kept. After a restart the helper sends the
    /// full state again.
    pub fn take_events(&self) -> Vec<Event> {
        std::mem::take(&mut self.shared.lock().events)
    }

    pub fn take_replies(&self) -> Vec<Reply> {
        std::mem::take(&mut self.shared.lock().replies)
    }

    fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.send(Command::Close);
        if let Some(handle) = self.supervisor.take() {
            let _ = handle.join();
        }
    }
}

struct Supervisor {
    options: EngineOptions,
    editable: bool,
    shared: Arc<Shared>,
    commands: Sender<Command>,
    receiver: Receiver<Command>,
}

/// How a run of the helper ended.
enum End {
    Close,
    Restart,
    /// The helper is gone: the explanation, and its exit code if it ended by itself.
    Gone(String, Option<i32>),
}

impl Supervisor {
    fn run(self) {
        let mut failed_starts = 0;
        loop {
            let started = Instant::now();
            let end = match Connection::start(&self.options) {
                Ok(connection) => self.serve(connection),
                Err(error) => End::Gone(format!("{error:#}"), None),
            };
            let (reason, code) = match end {
                End::Close => return,
                End::Restart => {
                    failed_starts = 0;
                    self.shared.set_status(Status::Starting);
                    continue;
                }
                End::Gone(reason, code) => (reason, code),
            };
            if code == Some(0) {
                self.shared.set_status(Status::Closed);
            } else {
                if started.elapsed() >= SHORT_RUN {
                    failed_starts = 0;
                }
                failed_starts += 1;
                if failed_starts < MAX_FAILED_STARTS {
                    tracing::warn!(target: "klayout", "{reason}; starting it again");
                    self.shared.set_status(Status::Restarting(reason));
                    continue;
                }
                tracing::error!(target: "klayout", "{reason}");
                self.shared.set_status(Status::Failed(reason));
            }
            failed_starts = 0;
            loop {
                match self.receiver.recv() {
                    Ok(Command::Restart) => break,
                    Ok(Command::Close) | Err(_) => return,
                    Ok(_) => {}
                }
            }
            self.shared.set_status(Status::Starting);
        }
    }

    /// Run one helper until it ends or is no longer wanted.
    fn serve(&self, connection: Connection) -> End {
        let Connection {
            reader,
            mut writer,
            mut process,
            klayout_version,
        } = connection;

        let (epoch, size, refs) = {
            let mut state = self.shared.lock();
            state.epoch += 1;
            state.events.clear();
            state.replies.clear();
            state.klayout_version = Some(klayout_version);
            (state.epoch, state.size, state.refs.clone())
        };
        self.spawn_reader(reader, epoch);

        // The size first, so that opening the file already draws it.
        let mut first = Vec::new();
        if let Some((width, height)) = size {
            first.push(Request::Resize { width, height });
        }
        if !refs.root.as_os_str().is_empty() {
            first.push(Request::Call {
                id: INTERNAL_ID,
                method: method::OPEN.into(),
                params: self.options.open_params(&refs, self.editable),
            });
        }
        for request in &first {
            let _ = protocol::write_message(&mut writer, request);
        }
        self.shared.set_status(Status::Running);

        loop {
            let Ok(command) = self.receiver.recv() else {
                return End::Close;
            };
            // Take everything waiting, so that a run of pointer movements or sizes is
            // sent as its last one only.
            let mut batch = vec![command];
            batch.extend(self.receiver.try_iter());
            let mut requests: Vec<Request> = Vec::new();
            for command in batch {
                let request = match command {
                    Command::Close => return End::Close,
                    Command::Restart => return End::Restart,
                    Command::Gone { epoch: gone, error } if gone == epoch => {
                        let (reason, code) = process.explain(&error);
                        return End::Gone(format!("{reason:#}"), code);
                    }
                    Command::Gone { .. } => continue,
                    Command::Resize(width, height) => Request::Resize { width, height },
                    Command::Input(input) => Request::Input { input },
                    Command::Call { id, method, params } => Request::Call { id, method, params },
                };
                if supersedes(requests.last(), &request) {
                    requests.pop();
                }
                requests.push(request);
            }
            for request in &requests {
                let _ = protocol::write_message(&mut writer, request);
            }
        }
    }

    fn spawn_reader(&self, mut reader: BufReader<UnixStream>, epoch: u64) {
        let shared = self.shared.clone();
        let commands = self.commands.clone();
        let spawned = thread::Builder::new()
            .name("klayout-reader".into())
            .spawn(move || {
                let error = loop {
                    match protocol::read_message::<Message>(&mut reader) {
                        Ok(Some(message)) => receive(&shared, epoch, message),
                        Ok(None) => break std::io::ErrorKind::UnexpectedEof.into(),
                        Err(error) => break error,
                    }
                };
                let _ = commands.send(Command::Gone { epoch, error });
            });
        if let Err(error) = spawned {
            let _ = self.commands.send(Command::Gone { epoch, error });
        }
    }
}

/// True when `next` makes `last` pointless: two pointer movements with the same
/// buttons held, or two sizes.
fn supersedes(last: Option<&Request>, next: &Request) -> bool {
    match (last, next) {
        (
            Some(Request::Input {
                input: Input::Move { buttons: a, .. },
            }),
            Request::Input {
                input: Input::Move { buttons: b, .. },
            },
        ) => a == b,
        (Some(Request::Resize { .. }), Request::Resize { .. }) => true,
        _ => false,
    }
}

/// Queue an event; an older one of the same kind is dropped unless it is a notice.
fn push_event(queue: &mut Vec<Event>, event: Event) {
    if !matches!(event, Event::Notice { .. }) {
        let kind = std::mem::discriminant(&event);
        queue.retain(|e| std::mem::discriminant(e) != kind);
    }
    queue.push(event);
}

/// Take in one message from the helper (reader thread).
fn receive(shared: &Shared, epoch: u64, message: Message) {
    let mut state = shared.lock();
    if state.epoch != epoch {
        return;
    }
    match message {
        Message::Frame {
            serial,
            path,
            width,
            height,
            stride,
            format,
        } => match FrameReader::open(Path::new(&path), width, height, stride) {
            Ok(reader) => {
                state.frame = Some(reader);
                state.format = format;
                state.frame_serial = serial;
                state.generation += 1;
                state.damage = Rect::new(0, 0, width, height);
            }
            Err(error) => tracing::debug!(target: "klayout", "frame {serial}: {error}"),
        },
        Message::Damage {
            serial,
            x,
            y,
            width,
            height,
        } => {
            if serial != state.frame_serial {
                return;
            }
            state.damage = state.damage.union(Rect::new(x, y, width, height));
        }
        Message::Event { event } => push_event(&mut state.events, event),
        Message::Reply {
            id,
            ok,
            value,
            error,
        } => {
            if id >= INTERNAL_ID {
                if !ok {
                    let text = format!("Cannot open the layout: {}", error.trim_end());
                    tracing::error!(target: "klayout", "{text}");
                    push_event(
                        &mut state.events,
                        Event::Notice {
                            level: NoticeLevel::Error,
                            text,
                        },
                    );
                }
            } else {
                let result = if ok {
                    Ok(value)
                } else {
                    Err(error.trim_end().to_string())
                };
                state.replies.push(Reply { id, result });
            }
        }
        Message::Ready { .. } | Message::Failed { .. } => return,
    }
    drop(state);
    (shared.wake)();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::klayout::protocol::{Button, Modifiers};
    use crate::xschem::frame::FrameWriter;

    fn shared() -> Shared {
        Shared {
            state: Mutex::new(State::default()),
            wake: Arc::new(|| {}),
        }
    }

    #[test]
    fn frames_come_out_as_rgba() {
        let dir = tempfile::tempdir().unwrap();
        let mut file = FrameWriter::create(dir.path(), 1, 2, 1).unwrap();
        file.pixels_mut()
            .copy_from_slice(&[10, 20, 30, 255, 1, 2, 3, 255]);
        let shared = shared();
        receive(
            &shared,
            0,
            Message::Frame {
                serial: 1,
                path: file.path().to_string_lossy().into(),
                width: 2,
                height: 1,
                stride: file.stride(),
                format: PixelFormat::Bgra,
            },
        );
        let state = shared.lock();
        let view = FrameView {
            reader: state.frame.as_ref().unwrap(),
            format: state.format,
            generation: 1,
            damage: None,
        };
        let mut out = Vec::new();
        view.copy(view.full(), &mut out);
        assert_eq!(out, [30, 20, 10, 255, 3, 2, 1, 255]);
        assert_eq!(state.damage, Rect::new(0, 0, 2, 1));
    }

    #[test]
    fn only_the_latest_state_is_kept_but_every_notice() {
        let shared = shared();
        let modified = |dirty| Message::Event {
            event: Event::Modified {
                dirty,
                can_undo: dirty,
                can_redo: false,
                edited: Vec::new(),
            },
        };
        let notice = |text: &str| Message::Event {
            event: Event::Notice {
                level: NoticeLevel::Error,
                text: text.into(),
            },
        };
        receive(&shared, 0, modified(false));
        receive(&shared, 0, notice("a"));
        receive(&shared, 0, modified(true));
        receive(&shared, 0, notice("b"));
        let events = std::mem::take(&mut shared.lock().events);
        assert_eq!(events.len(), 3);
        assert!(matches!(events[1], Event::Modified { dirty: true, .. }));
        // A message of an earlier helper is ignored.
        receive(&shared, 7, modified(true));
        assert!(shared.lock().events.is_empty());
    }

    #[test]
    fn a_failed_open_becomes_a_notice_and_replies_are_kept() {
        let shared = shared();
        receive(
            &shared,
            0,
            Message::Reply {
                id: INTERNAL_ID,
                ok: false,
                value: Value::Null,
                error: "no such file\n".into(),
            },
        );
        receive(
            &shared,
            0,
            Message::Reply {
                id: 4,
                ok: true,
                value: serde_json::json!({"top": "a"}),
                error: String::new(),
            },
        );
        let state = shared.lock();
        assert!(
            matches!(&state.events[..], [Event::Notice { text, .. }] if text.contains("no such file"))
        );
        assert_eq!(state.replies[0].id, 4);
        assert_eq!(state.replies[0].result.as_ref().unwrap()["top"], "a");
    }

    #[test]
    fn movements_with_the_same_buttons_are_merged() {
        let moved = |x, buttons: Vec<Button>| Request::Input {
            input: Input::Move {
                x,
                y: 0.0,
                buttons,
                modifiers: Modifiers::default(),
            },
        };
        assert!(supersedes(Some(&moved(1.0, vec![])), &moved(2.0, vec![])));
        assert!(!supersedes(
            Some(&moved(1.0, vec![])),
            &moved(2.0, vec![Button::Left])
        ));
        assert!(supersedes(
            Some(&Request::Resize {
                width: 1,
                height: 1
            }),
            &Request::Resize {
                width: 2,
                height: 2
            }
        ));
    }
}
