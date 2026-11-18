//! `cdept-xschem`: xschem's schematic engine as a helper process for CDePT.
//!
//! CDePT starts this program and talks to it over a Unix socket
//! (`libcdept::xschem::protocol`). xschem's own code does the work; this program is
//! therefore GPL-2.0-or-later, while CDePT itself stays MIT.
//!
//! The program is one thread running the Tcl event loop. The socket is a file the loop
//! watches, so requests are also served while xschem waits inside a nested loop (a
//! dialog it wants answered). After each batch of requests, and after timers, the part
//! of the drawing window that changed is copied to the shared frame.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::ExitCode;
use std::rc::Rc;

use cdept_xschem::start::{self, StartOptions};
use cdept_xschem::xschem::{self, Pane, Xschem};
use libcdept::xschem::frame::{FrameWriter, Rect};
use libcdept::xschem::protocol::{self, Message, PROTOCOL_VERSION, Request, UiEvent, UiUpdate};
use libcdept::xschem::tcl;

const USAGE: &str = "\
cdept-xschem: xschem engine for CDePT (started by cdept, not by hand)

Usage: cdept-xschem --connect <socket> [options]
       cdept-xschem --version
       cdept-xschem --print-sharedir [--sharedir <folder>]

  --connect <socket>   Unix socket CDePT listens on
  --rcfile <file>      start-up file (Tcl) to read instead of the user's xschemrc
  --sharedir <folder>  xschem's runtime files: xschem.tcl, the awk scripts and
                       xschem_library/devices (default: $CDEPT_XSCHEM_SHAREDIR, then
                       ../share/cdept-xschem next to this program, then the build tree)
  --display            draw: xschem gets a window, whose pixels go to frame files
  --frame-dir <folder> where to create the frame files (needed with --display)
";

struct Args {
    connect: PathBuf,
    rcfile: Option<PathBuf>,
    sharedir: Option<PathBuf>,
    display: bool,
    frame_dir: Option<PathBuf>,
}

fn main() -> ExitCode {
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(Some(args)) => args,
        Ok(None) => return ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("cdept-xschem: {error}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("cdept-xschem: {error}");
            ExitCode::FAILURE
        }
    }
}

fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Option<Args>, String> {
    let mut connect = None;
    let mut rcfile = None;
    let mut sharedir = None;
    let mut display = false;
    let mut frame_dir = None;
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or(format!("{name} needs a value"));
        match arg.as_str() {
            "--connect" => connect = Some(PathBuf::from(value("--connect")?)),
            "--rcfile" => rcfile = Some(PathBuf::from(value("--rcfile")?)),
            "--sharedir" => sharedir = Some(PathBuf::from(value("--sharedir")?)),
            "--frame-dir" => frame_dir = Some(PathBuf::from(value("--frame-dir")?)),
            "--display" => display = true,
            "--version" => {
                println!(
                    "cdept-xschem {} (xschem {}, protocol {PROTOCOL_VERSION})",
                    env!("CARGO_PKG_VERSION"),
                    xschem::version()
                );
                return Ok(None);
            }
            "--print-sharedir" => {
                // Where the runtime files are found (used by the install script).
                return match start::find_sharedir(sharedir.as_deref()) {
                    Ok(dir) => {
                        println!("{}", dir.display());
                        Ok(None)
                    }
                    Err(error) => Err(error),
                };
            }
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(None);
            }
            other => return Err(format!("unknown argument '{other}'")),
        }
    }
    let connect = connect.ok_or("--connect is required")?;
    if display && frame_dir.is_none() {
        return Err("--display needs --frame-dir".into());
    }
    Ok(Some(Args {
        connect,
        rcfile,
        sharedir,
        display,
        frame_dir,
    }))
}

/// Everything the request handlers share. xschem calls back into this program while a
/// request is being handled (a script opens a dialog and waits: the event loop runs
/// nested and serves further requests), so nothing here is borrowed across a call into
/// xschem.
struct Helper {
    xschem: Xschem,
    writer: RefCell<UnixStream>,
    reader: RefCell<UnixStream>,
    /// Bytes read from the socket that do not yet end in a newline.
    partial: RefCell<Vec<u8>>,
    /// Requests read but not handled yet.
    queue: RefCell<VecDeque<Request>>,
    /// Where frame files go; `None` without `--display`.
    frame_dir: Option<PathBuf>,
    frame: RefCell<Option<FrameWriter>>,
    serial: Cell<u64>,
    /// Files of the picture panes of dialogs, by Tk path, with their serial numbers.
    previews: RefCell<HashMap<String, (u64, FrameWriter)>>,
    preview_serial: Cell<u64>,
}

impl Helper {
    fn send(&self, message: &Message) {
        // A failed write means CDePT is gone; the read side notices and exits.
        let _ = protocol::write_message(&mut *self.writer.borrow_mut(), message);
    }

    /// Leave through Tcl's `exit`: it runs xschem's exit handler (which removes its
    /// undo folder) and ends the process.
    fn exit(&self) -> ! {
        if let Some(frame) = self.frame.borrow().as_ref() {
            frame.unlink();
        }
        for (_, file) in self.previews.borrow().values() {
            file.unlink();
        }
        let _ = self.xschem.eval("exit 0");
        std::process::exit(0)
    }

    /// The socket has data: read what is there and handle the complete requests.
    fn readable(&self) {
        let mut buffer = [0u8; 16384];
        // One read per call: the event loop said it will not block, and calls again
        // while more is waiting.
        let count = match self.reader.borrow_mut().read(&mut buffer) {
            Ok(0) => self.exit(), // CDePT closed the connection, or died
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => return,
            Err(_) => self.exit(),
        };
        {
            let mut partial = self.partial.borrow_mut();
            let mut queue = self.queue.borrow_mut();
            partial.extend_from_slice(&buffer[..count]);
            while let Some(end) = partial.iter().position(|&byte| byte == b'\n') {
                let line: Vec<u8> = partial.drain(..=end).collect();
                match serde_json::from_slice::<Request>(&line) {
                    Ok(request) => queue.push_back(request),
                    Err(error) => eprintln!("cdept-xschem: bad request: {error}"),
                }
            }
        }
        self.drain();
        self.present();
    }

    /// Handle the queued requests, oldest first.
    fn drain(&self) {
        loop {
            let request = {
                let mut queue = self.queue.borrow_mut();
                let Some(request) = queue.pop_front() else {
                    return;
                };
                // Of a run of pointer movements, or of sizes, only the last counts.
                let superseded = match (&request, queue.front()) {
                    (Request::Input { input }, Some(Request::Input { input: next })) => {
                        input.is_motion() && next.is_motion()
                    }
                    (Request::Resize { .. }, _) => {
                        queue.iter().any(|r| matches!(r, Request::Resize { .. }))
                    }
                    _ => false,
                };
                if superseded {
                    continue;
                }
                if !queue.is_empty() {
                    // Handling this request may not return for a long time (xschem
                    // opens a dialog and waits for it): the requests behind it are
                    // then served from the event loop running inside that wait.
                    self.xschem.when_idle();
                }
                request
            };
            self.handle(request);
        }
    }

    fn handle(&self, request: Request) {
        match request {
            Request::Tcl { id, script } => {
                let (ok, value) = match self.xschem.eval(&script) {
                    Ok(value) => (true, value),
                    Err(error) => (false, error),
                };
                self.send(&Message::Reply { id, ok, value });
            }
            Request::Shutdown { id } => {
                self.send(&Message::Reply {
                    id,
                    ok: true,
                    value: String::new(),
                });
                let _ = self.writer.borrow_mut().flush();
                self.exit();
            }
            Request::Resize { width, height } => {
                if self.frame_dir.is_some() {
                    self.xschem.resize(width, height);
                    self.new_frame();
                }
            }
            Request::Input { input } => {
                if self.frame_dir.is_some() {
                    self.xschem.input(input);
                }
            }
            Request::Ui { event } => {
                if let Err(error) = self.xschem.eval(&ui_event_script(&event)) {
                    eprintln!("cdept-xschem: {event:?}: {error}");
                }
            }
        }
    }

    /// Start a new frame file matching the size of the drawing window, and announce it.
    fn new_frame(&self) {
        let (Some(dir), Some((width, height))) = (&self.frame_dir, self.xschem.display_size())
        else {
            return;
        };
        let serial = self.serial.get() + 1;
        self.serial.set(serial);
        match FrameWriter::create(dir, serial, width, height) {
            Ok(writer) => {
                self.send(&Message::Frame {
                    serial,
                    path: writer.path().to_string_lossy().into_owned(),
                    width,
                    height,
                    stride: writer.stride(),
                });
                if let Some(old) = self.frame.borrow_mut().replace(writer) {
                    old.unlink();
                }
                // Nothing is in the new file yet: copy the whole window, which covers
                // whatever is reported as changed.
                let _ = self.xschem.take_damage_rects();
                self.copy_out(Rect::new(0, 0, width, height));
            }
            Err(error) => eprintln!("cdept-xschem: cannot create a frame file: {error}"),
        }
    }

    /// Copy what changed in the drawing window and in the picture panes to their
    /// files and tell CDePT.
    fn present(&self) {
        if self.frame_dir.is_none() {
            return;
        }
        self.present_drawing();
        self.present_previews();
    }

    /// The picture panes of dialogs: a file for each new one (or new size), the pixels
    /// copied whenever they changed (all of them: panes are small).
    fn present_previews(&self) {
        let Some(dir) = &self.frame_dir else {
            return;
        };
        let panes = self.xschem.previews();
        let mut files = self.previews.borrow_mut();
        let gone: Vec<String> = files
            .keys()
            .filter(|path| !panes.iter().any(|pane| &pane.path == *path))
            .cloned()
            .collect();
        for widget in gone {
            if let Some((_, file)) = files.remove(&widget) {
                file.unlink();
            }
            self.send(&Message::PreviewGone { widget });
        }
        for pane in &panes {
            let fits = files
                .get(&pane.path)
                .is_some_and(|(_, f)| f.width() == pane.width && f.height() == pane.height);
            if fits && !pane.changed {
                continue;
            }
            if !fits {
                let serial = self.preview_serial.get() + 1;
                self.preview_serial.set(serial);
                let name = format!("preview-{serial}");
                match FrameWriter::create_named(dir, &name, pane.width, pane.height) {
                    Ok(file) => {
                        if let Some((_, old)) = files.insert(pane.path.clone(), (serial, file)) {
                            old.unlink();
                        }
                    }
                    Err(error) => {
                        eprintln!(
                            "cdept-xschem: cannot create a file for {}: {error}",
                            pane.path
                        );
                        continue;
                    }
                }
            }
            let (serial, file) = files.get_mut(&pane.path).expect("just made");
            self.copy_pane(pane, file);
            self.send(&Message::Preview {
                widget: pane.path.clone(),
                serial: *serial,
                path: file.path().to_string_lossy().into_owned(),
                width: pane.width,
                height: pane.height,
                stride: file.stride(),
            });
        }
    }

    fn copy_pane(&self, pane: &Pane, file: &mut FrameWriter) {
        let stride = file.stride();
        self.xschem.read_preview(pane, file.pixels_mut(), stride);
    }

    /// Copy what changed in the drawing window to the frame and tell CDePT.
    fn present_drawing(&self) {
        let Some((width, height)) = self.xschem.display_size() else {
            return;
        };
        let fits = self
            .frame
            .borrow()
            .as_ref()
            .is_some_and(|f| f.width() == width && f.height() == height);
        if !fits {
            // xschem changed the window size on its own.
            return self.new_frame();
        }
        // Far-apart changes (the strips of a cross-hair cursor) stay separate
        // rectangles instead of one around them all.
        for rect in self.xschem.take_damage_rects() {
            self.copy_out(rect);
        }
    }

    fn copy_out(&self, rect: Rect) {
        let mut frame = self.frame.borrow_mut();
        let Some(frame) = frame.as_mut() else {
            return;
        };
        let rect = rect.clamp(frame.width(), frame.height());
        if rect.is_empty() {
            return;
        }
        let stride = frame.stride();
        self.xschem.read_pixels(frame.pixels_mut(), stride, rect);
        self.send(&Message::Damage {
            serial: self.serial.get(),
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: rect.height,
        });
    }

    /// A widget update from the Tcl side: check it and pass it on.
    fn ui_update(&self, json: &str) {
        match serde_json::from_str::<UiUpdate>(json) {
            Ok(update) => self.send(&Message::Ui { update }),
            Err(error) => {
                let start: String = json.chars().take(200).collect();
                eprintln!("cdept-xschem: bad widget update ({error}): {start}");
            }
        }
    }
}

/// The Tcl call that tells the Tk stand-in (`shim/tcl/faketk.tcl`) what the user did.
fn ui_event_script(event: &UiEvent) -> String {
    let q = tcl::quote;
    match event {
        UiEvent::Menu { menu, index } => format!("faketk::event menu {} {index}", q(menu)),
        UiEvent::PopupClosed => "faketk::event popup_closed".into(),
        UiEvent::Invoke { path } => format!("faketk::event invoke {}", q(path)),
        UiEvent::Text { path, text } => format!("faketk::event text {} {}", q(path), q(text)),
        UiEvent::Select { path, selection } => {
            let list: Vec<String> = selection.iter().map(u32::to_string).collect();
            format!("faketk::event select {} {}", q(path), q(&list.join(" ")))
        }
        UiEvent::Activate { path, index } => {
            format!("faketk::event activate {} {index}", q(path))
        }
        UiEvent::Key { path, key } => format!("faketk::event key {} {}", q(path), q(key)),
        UiEvent::Focus { path } => format!("faketk::event focus {}", q(path)),
        UiEvent::Close { path } => format!("faketk::event close {}", q(path)),
        UiEvent::Configure {
            path,
            width,
            height,
        } => format!("faketk::event configure {} {width} {height}", q(path)),
        UiEvent::Clipboard { selection, text } => {
            format!("faketk::event clipboard {} {}", q(selection), q(text))
        }
    }
}

fn run(args: Args) -> Result<(), String> {
    let stream = UnixStream::connect(&args.connect)
        .map_err(|e| format!("cannot connect to {}: {e}", args.connect.display()))?;
    let mut writer = stream.try_clone().map_err(|e| e.to_string())?;
    let mut send = |message: Message| {
        protocol::write_message(&mut writer, &message).map_err(|e| format!("socket: {e}"))
    };

    // Widget updates reported while xschem starts (it builds its menus then) are kept
    // until the connection is ready for them.
    let early: Rc<RefCell<Vec<String>>> = Rc::default();
    {
        let early = early.clone();
        xschem::on_ui(move |json| early.borrow_mut().push(json.to_string()));
    }

    let options = StartOptions {
        rcfile: args.rcfile.clone(),
        sharedir: args.sharedir.clone(),
        display: args.display,
    };
    let xschem = match start::start(&options) {
        Ok(xschem) => xschem,
        Err(error) => {
            send(Message::Failed {
                error: error.clone(),
            })?;
            return Err(error);
        }
    };
    send(Message::Ready {
        protocol: PROTOCOL_VERSION,
        xschem: xschem::version(),
    })?;

    let fd = stream.as_raw_fd();
    let helper = Rc::new(Helper {
        xschem,
        writer: RefCell::new(writer),
        reader: RefCell::new(stream),
        partial: RefCell::new(Vec::new()),
        queue: RefCell::new(VecDeque::new()),
        frame_dir: args.display.then_some(args.frame_dir).flatten(),
        frame: RefCell::new(None),
        serial: Cell::new(0),
        previews: RefCell::new(HashMap::new()),
        preview_serial: Cell::new(0),
    });
    {
        let helper = helper.clone();
        helper.clone().xschem.on_idle(move || {
            helper.drain();
            helper.present();
        });
    }
    for json in early.take() {
        helper.ui_update(&json);
    }
    {
        let helper = helper.clone();
        xschem::on_ui(move |json| helper.ui_update(json));
    }
    {
        let helper = helper.clone();
        helper.clone().xschem.watch(fd, move || helper.readable());
    }

    // The process ends in `Helper::exit`, on Shutdown or when the socket closes.
    loop {
        helper.xschem.do_one_event(true);
        helper.present();
    }
}
