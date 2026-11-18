//! One schematic open in the GUI: an xschem helper of its own, kept running.
//!
//! A [`Session`] never blocks its caller. Input, sizes, widget events and Tcl commands
//! are queued and written to the helper by a supervisor thread; frames, widget updates
//! and replies are read by a second thread and kept until the GUI asks for them
//! ([`Session::frame`], [`Session::take_updates`], [`Session::take_panes`],
//! [`Session::take_replies`]). The
//! `wake` callback of the options is called whenever there is something new to show.
//!
//! The supervisor starts the helper, loads the file, and starts the helper again when
//! it stops unexpectedly (see [`Status`]).

use std::collections::HashMap;
use std::io::BufReader;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::engine::{Connection, EngineOptions, absolute, word};
use super::frame::{FrameReader, Rect};
use super::input::Input;
use super::protocol::{self, Message, Request, UiEvent, UiUpdate};

/// Called from the session's threads when there is something new to show. In the GUI:
/// request a repaint.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

/// A helper that stops within this time of starting counts as a failed start.
const SHORT_RUN: Duration = Duration::from_secs(5);
/// Failed starts in a row before giving up (the user can still ask for a restart).
const MAX_FAILED_STARTS: u32 = 3;
/// Ids from here on belong to the session's own requests (loading the file).
const INTERNAL_ID: u64 = 1 << 62;

pub struct SessionOptions {
    /// How to start the helper; `display` is switched on regardless.
    pub engine: EngineOptions,
    /// The schematic or symbol to load, also after every restart.
    pub file: Option<PathBuf>,
    pub wake: Wake,
}

/// What the engine behind a session is doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// The helper is starting for the first time.
    Starting,
    Running,
    /// The helper stopped unexpectedly (the text says how) and is being started again.
    /// The last frame stays on screen meanwhile.
    Restarting(String),
    /// The helper could not be started, or keeps stopping. [`Session::restart`] tries
    /// again.
    Failed(String),
    /// xschem ended on its own accord (its Exit command): the document is closed.
    Closed,
}

/// The answer to a [`Session::tcl`] call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub id: u64,
    /// The script's result, or the Tcl error message.
    pub result: Result<String, String>,
}

/// The drawing window as last reported by the helper, handed to [`Session::frame`].
pub struct FrameView<'a> {
    reader: &'a FrameReader,
    /// Changes when the frame was replaced (another size, or a restarted helper): the
    /// whole picture has to be taken again.
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

    /// Copy the pixels of `rect` into `out`: `rect.width * 4` bytes per row, R G B A
    /// with premultiplied alpha. Returns the rectangle copied (`rect` clipped to the
    /// frame).
    pub fn copy(&self, rect: Rect, out: &mut Vec<u8>) -> Rect {
        self.reader.copy_rect(rect, out)
    }

    /// The whole frame.
    pub fn full(&self) -> Rect {
        Rect::new(0, 0, self.width(), self.height())
    }
}

/// A picture pane of a dialog changed (see the protocol documentation): xschem draws a
/// schematic or symbol into a frame of the dialog `widget` names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaneUpdate {
    /// New pixels, all of them: `width * 4` bytes per row, R G B A premultiplied.
    Changed {
        widget: String,
        width: u32,
        height: u32,
        pixels: Vec<u8>,
    },
    Gone {
        widget: String,
    },
}

impl PaneUpdate {
    pub fn widget(&self) -> &str {
        match self {
            PaneUpdate::Changed { widget, .. } | PaneUpdate::Gone { widget } => widget,
        }
    }
}

enum Command {
    Resize(u32, u32),
    Input(Input),
    Ui(UiEvent),
    Tcl {
        id: u64,
        script: String,
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
    xschem_version: Option<String>,
    /// Counts the helpers started; messages of an older one are ignored.
    epoch: u64,
    frame: Option<FrameReader>,
    frame_serial: u64,
    generation: u64,
    damage: Rect,
    updates: Vec<UiUpdate>,
    /// Files of the picture panes, by widget, with their serial numbers.
    panes: HashMap<String, (u64, FrameReader)>,
    pane_updates: Vec<PaneUpdate>,
    replies: Vec<Reply>,
    /// Last size asked for, sent again to a restarted helper.
    size: Option<(u32, u32)>,
    next_id: u64,
}

struct Shared {
    state: Mutex<State>,
    wake: Wake,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        // A panic while holding the lock leaves plain data behind: keep going.
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn set_status(&self, status: Status) {
        self.lock().status = Some(status);
        (self.wake)();
    }
}

/// One open schematic and the xschem process behind it. Dropping it ends the process.
pub struct Session {
    commands: Sender<Command>,
    shared: Arc<Shared>,
    supervisor: Option<JoinHandle<()>>,
}

impl Session {
    /// Start the helper in the background and load `options.file` in it.
    pub fn open(options: SessionOptions) -> Session {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                status: Some(Status::Starting),
                next_id: 1,
                ..Default::default()
            }),
            wake: options.wake.clone(),
        });
        let (commands, receiver) = mpsc::channel();
        let supervisor = Supervisor {
            options: EngineOptions {
                display: true,
                ..options.engine
            },
            file: options.file,
            shared: shared.clone(),
            commands: commands.clone(),
            receiver,
        };
        let handle = thread::Builder::new()
            .name("xschem-session".into())
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

    /// Counts the helpers started for this session. When it changes, everything
    /// mirrored from the previous helper (menus, dialogs) is gone.
    pub fn epoch(&self) -> u64 {
        self.shared.lock().epoch
    }

    /// Version of xschem in the helper, once it has started.
    pub fn xschem_version(&self) -> Option<String> {
        self.shared.lock().xschem_version.clone()
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

    /// Something the user did in a mirrored widget (menu, tool bar, dialog).
    pub fn ui(&self, event: UiEvent) {
        self.send(Command::Ui(event));
    }

    /// Evaluate a Tcl script in xschem. The answer arrives later, through
    /// [`take_replies`](Self::take_replies), under the id returned here. Requests made
    /// while the helper is not running get no answer.
    pub fn tcl(&self, script: &str) -> u64 {
        let id = {
            let mut state = self.shared.lock();
            let id = state.next_id;
            state.next_id += 1;
            id
        };
        self.send(Command::Tcl {
            id,
            script: script.to_string(),
        });
        id
    }

    /// Stop the helper, if it runs, and start a fresh one with the file loaded again.
    /// Unsaved changes in xschem are lost.
    pub fn restart(&self) {
        self.send(Command::Restart);
    }

    /// Look at the drawing window. `None` while no frame has arrived. The damage is
    /// handed out once: a caller that sees `damage` must copy it.
    pub fn frame<R>(&self, look: impl FnOnce(FrameView<'_>) -> R) -> Option<R> {
        let mut state = self.shared.lock();
        let damage = std::mem::take(&mut state.damage);
        let generation = state.generation;
        let reader = state.frame.as_ref()?;
        Some(look(FrameView {
            reader,
            generation,
            damage: (!damage.is_empty()).then_some(damage),
        }))
    }

    /// Widget updates received since the last call, oldest first.
    pub fn take_updates(&self) -> Vec<UiUpdate> {
        std::mem::take(&mut self.shared.lock().updates)
    }

    /// Changes of the picture panes since the last call, oldest first; only the last
    /// one of each pane.
    pub fn take_panes(&self) -> Vec<PaneUpdate> {
        std::mem::take(&mut self.shared.lock().pane_updates)
    }

    /// Answers to [`tcl`](Self::tcl) calls received since the last call.
    pub fn take_replies(&self) -> Vec<Reply> {
        std::mem::take(&mut self.shared.lock().replies)
    }

    fn send(&self, command: Command) {
        // The supervisor only goes away in `drop`.
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
    file: Option<PathBuf>,
    shared: Arc<Shared>,
    commands: Sender<Command>,
    receiver: Receiver<Command>,
}

/// How a run of the helper ended.
enum End {
    /// The session was dropped.
    Close,
    /// The user asked for a fresh helper.
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
                    tracing::warn!(target: "xschem", "{reason}; starting it again");
                    self.shared.set_status(Status::Restarting(reason));
                    continue;
                }
                tracing::error!(target: "xschem", "{reason}");
                self.shared.set_status(Status::Failed(reason));
            }
            // Stopped for good: wait until the user asks for another try.
            failed_starts = 0;
            loop {
                match self.receiver.recv() {
                    Ok(Command::Restart) => break,
                    Ok(Command::Close) | Err(_) => return,
                    Ok(_) => {} // nothing to send it to
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
            xschem_version,
        } = connection;

        let (epoch, size) = {
            let mut state = self.shared.lock();
            state.epoch += 1;
            // Widgets and answers of the previous helper mean nothing to this one.
            state.updates.clear();
            state.replies.clear();
            state.panes.clear();
            // The panes of the old helper are gone with its windows.
            let old: Vec<String> = state
                .pane_updates
                .drain(..)
                .map(|u| u.widget().into())
                .collect();
            state.pane_updates = old
                .into_iter()
                .map(|widget| PaneUpdate::Gone { widget })
                .collect();
            state.xschem_version = Some(xschem_version);
            (state.epoch, state.size)
        };
        self.spawn_reader(reader, epoch);

        // A window first, so that loading the file already draws it.
        let mut first = Vec::new();
        if let Some((width, height)) = size {
            first.push(Request::Resize { width, height });
        }
        if let Some(file) = &self.file {
            first.push(Request::Tcl {
                id: INTERNAL_ID,
                script: load_script(file),
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
            // Take everything that is waiting, so that a run of pointer movements or
            // of sizes is sent as its last one only.
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
                    Command::Gone { .. } => continue, // an earlier helper
                    Command::Resize(width, height) => Request::Resize { width, height },
                    Command::Input(input) => Request::Input { input },
                    Command::Ui(event) => Request::Ui { event },
                    Command::Tcl { id, script } => Request::Tcl { id, script },
                };
                let supersedes = match (requests.last(), &request) {
                    (Some(Request::Input { input: last }), Request::Input { input: next }) => {
                        last.is_motion() && next.is_motion()
                    }
                    (Some(Request::Resize { .. }), Request::Resize { .. }) => true,
                    _ => false,
                };
                if supersedes {
                    requests.pop();
                }
                requests.push(request);
            }
            for request in &requests {
                // A failed write means the helper is gone; the reader reports that.
                let _ = protocol::write_message(&mut writer, request);
            }
        }
    }

    fn spawn_reader(&self, mut reader: BufReader<UnixStream>, epoch: u64) {
        let shared = self.shared.clone();
        let commands = self.commands.clone();
        let spawned = thread::Builder::new()
            .name("xschem-reader".into())
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

/// The Tcl script that loads `file` as the current design.
fn load_script(file: &Path) -> String {
    let file = absolute(file).unwrap_or_else(|_| file.to_path_buf());
    format!("xschem load {}", word(&file))
}

/// Queue a pane update; an older one of the same pane is replaced.
fn push_pane(queue: &mut Vec<PaneUpdate>, update: PaneUpdate) {
    queue.retain(|u| u.widget() != update.widget());
    queue.push(update);
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
        } => match FrameReader::open(Path::new(&path), width, height, stride) {
            Ok(reader) => {
                state.frame = Some(reader);
                state.frame_serial = serial;
                state.generation += 1;
                state.damage = Rect::new(0, 0, width, height);
            }
            // Usually a frame that was replaced before we got to it: the next one
            // is on its way.
            Err(error) => tracing::debug!(target: "xschem", "frame {serial}: {error}"),
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
        Message::Ui { update } => state.updates.push(update),
        Message::Preview {
            widget,
            serial,
            path,
            width,
            height,
            stride,
        } => {
            let known = state.panes.get(&widget).is_some_and(|(s, _)| *s == serial);
            if !known {
                match FrameReader::open(Path::new(&path), width, height, stride) {
                    Ok(reader) => {
                        state.panes.insert(widget.clone(), (serial, reader));
                    }
                    Err(error) => {
                        tracing::debug!(target: "xschem", "pane {widget}: {error}");
                        return;
                    }
                }
            }
            let (_, reader) = &state.panes[&widget];
            let mut pixels = Vec::new();
            reader.copy_rect(Rect::new(0, 0, width, height), &mut pixels);
            let update = PaneUpdate::Changed {
                widget,
                width: reader.width(),
                height: reader.height(),
                pixels,
            };
            push_pane(&mut state.pane_updates, update);
        }
        Message::PreviewGone { widget } => {
            state.panes.remove(&widget);
            push_pane(&mut state.pane_updates, PaneUpdate::Gone { widget });
        }
        Message::Reply { id, ok, value } => {
            if id >= INTERNAL_ID {
                if !ok {
                    tracing::error!(target: "xschem", "{}", value.trim_end());
                    state.updates.push(UiUpdate::Log {
                        text: value.trim_end().to_string(),
                    });
                }
            } else {
                let result = if ok {
                    Ok(value)
                } else {
                    Err(value.trim_end().to_string())
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

    #[test]
    fn only_the_last_change_of_a_pane_is_kept() {
        let changed = |widget: &str, width| PaneUpdate::Changed {
            widget: widget.into(),
            width,
            height: 1,
            pixels: vec![0; width as usize * 4],
        };
        let mut queue = Vec::new();
        push_pane(&mut queue, changed(".a", 1));
        push_pane(&mut queue, changed(".b", 1));
        push_pane(&mut queue, changed(".a", 2));
        assert_eq!(queue, vec![changed(".b", 1), changed(".a", 2)]);
        push_pane(
            &mut queue,
            PaneUpdate::Gone {
                widget: ".b".into(),
            },
        );
        assert_eq!(queue[0].widget(), ".a");
        assert_eq!(
            queue[1],
            PaneUpdate::Gone {
                widget: ".b".into()
            }
        );
    }

    #[test]
    fn a_pane_arrives_with_its_pixels() {
        let dir = tempfile::tempdir().unwrap();
        let mut file =
            super::super::frame::FrameWriter::create_named(dir.path(), "preview-1", 2, 2).unwrap();
        file.pixels_mut()[..4].copy_from_slice(&[1, 2, 3, 255]);
        let shared = Shared {
            state: Mutex::new(State::default()),
            wake: Arc::new(|| {}),
        };
        let path = file.path().to_string_lossy().into_owned();
        let preview = |serial| Message::Preview {
            widget: ".bk".into(),
            serial,
            path: path.clone(),
            width: 2,
            height: 2,
            stride: 8,
        };
        receive(&shared, 0, preview(1));
        let updates = std::mem::take(&mut shared.lock().pane_updates);
        let [PaneUpdate::Changed { widget, pixels, .. }] = &updates[..] else {
            panic!("{updates:?}");
        };
        assert_eq!(widget, ".bk");
        assert_eq!(&pixels[..4], &[1, 2, 3, 255]);
        // Same file, new pixels.
        file.pixels_mut()[..4].copy_from_slice(&[9, 9, 9, 255]);
        receive(&shared, 0, preview(1));
        let updates = std::mem::take(&mut shared.lock().pane_updates);
        let [PaneUpdate::Changed { pixels, .. }] = &updates[..] else {
            panic!("{updates:?}");
        };
        assert_eq!(&pixels[..4], &[9, 9, 9, 255]);
        receive(
            &shared,
            0,
            Message::PreviewGone {
                widget: ".bk".into(),
            },
        );
        assert!(shared.lock().panes.is_empty());
    }
}
