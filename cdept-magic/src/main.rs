//! `cdept-magic`: Magic's layout editor as a helper process for CDePT.
//!
//! CDePT starts this program and talks to it over a Unix socket, with the same
//! protocol and frame files as the xschem helper (`libcdept::xschem::protocol`). The
//! program loads Magic's own `tclmagic.so` into its Tcl interpreter with stand-ins for
//! Tk and X (`shim/cdm.c`, faketk.tcl), so Magic draws with its Cairo driver into
//! memory that is copied to the frame file.
//!
//! One thread runs the Tcl event loop; the socket is a file the loop watches.

mod engine;

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::ExitCode;
use std::rc::Rc;

use engine::{Magic, StartOptions};
use libcdept::xschem::frame::{FrameWriter, Rect};
use libcdept::xschem::protocol::{self, Message, PROTOCOL_VERSION, Request, UiEvent, UiUpdate};
use libcdept::xschem::tcl;

const USAGE: &str = "\
cdept-magic: Magic for CDePT (started by cdept, not by hand)

Usage: cdept-magic --connect <socket> [options]
       cdept-magic --run [options] [-e <tcl>]... [--ppm <file>]
       cdept-magic --version

  --connect <socket>   Unix socket CDePT listens on
  --rcfile <file>      CDePT's Tcl, read after Magic started
  --magic-rc <file>    Magic's start-up file (the PDK's magicrc); also $CDEPT_MAGIC_RC
  --magic-tcl <dir>    Magic's Tcl folder (magic.tcl, tclmagic.so); default from
                       $CDEPT_MAGIC_TCLDIR or `magic --prefix` ($CDEPT_MAGIC)
  --display            draw: Magic gets a window, whose pixels go to frame files
  --frame-dir <folder> where to create the frame files (needed with --display)
  --size <w>x<h>       first size of the window (default 800x600)
  --run                test mode: start, run the -e scripts, print their results,
                       write the window as a PPM picture with --ppm
";

struct Args {
    connect: Option<PathBuf>,
    rcfile: Option<PathBuf>,
    magic_rc: Option<PathBuf>,
    magic_tcl: Option<PathBuf>,
    display: bool,
    frame_dir: Option<PathBuf>,
    size: (u32, u32),
    run: bool,
    scripts: Vec<String>,
    ppm: Option<PathBuf>,
}

fn main() -> ExitCode {
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(Some(args)) => args,
        Ok(None) => return ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("cdept-magic: {error}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let result = if args.run { run_test(args) } else { run(args) };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("cdept-magic: {error}");
            ExitCode::FAILURE
        }
    }
}

fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Option<Args>, String> {
    let mut out = Args {
        connect: None,
        rcfile: None,
        magic_rc: std::env::var_os("CDEPT_MAGIC_RC")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from),
        magic_tcl: None,
        display: false,
        frame_dir: None,
        size: (800, 600),
        run: false,
        scripts: Vec::new(),
        ppm: None,
    };
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or(format!("{name} needs a value"));
        match arg.as_str() {
            "--connect" => out.connect = Some(PathBuf::from(value("--connect")?)),
            "--rcfile" => out.rcfile = Some(PathBuf::from(value("--rcfile")?)),
            "--magic-rc" => out.magic_rc = Some(PathBuf::from(value("--magic-rc")?)),
            "--magic-tcl" => out.magic_tcl = Some(PathBuf::from(value("--magic-tcl")?)),
            "--frame-dir" => out.frame_dir = Some(PathBuf::from(value("--frame-dir")?)),
            "--display" => out.display = true,
            "--run" => out.run = true,
            "-e" => out.scripts.push(value("-e")?),
            "--ppm" => out.ppm = Some(PathBuf::from(value("--ppm")?)),
            "--size" => {
                let text = value("--size")?;
                let (w, h) = text
                    .split_once('x')
                    .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
                    .ok_or(format!("--size {text}: expected <width>x<height>"))?;
                out.size = (w, h);
            }
            "--version" => {
                println!(
                    "cdept-magic {} (protocol {PROTOCOL_VERSION})",
                    env!("CARGO_PKG_VERSION")
                );
                return Ok(None);
            }
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(None);
            }
            other => return Err(format!("unknown argument '{other}'")),
        }
    }
    if !out.run && out.connect.is_none() {
        return Err("--connect is required".into());
    }
    if out.run {
        out.display = true;
    } else if out.display && out.frame_dir.is_none() {
        return Err("--display needs --frame-dir".into());
    }
    Ok(Some(out))
}

/// Largest window, from `$CDEPT_MAGIC_SCREEN` (`<w>x<h>`), else 3840x2160.
fn screen_size() -> (u32, u32) {
    std::env::var("CDEPT_MAGIC_SCREEN")
        .ok()
        .and_then(|s| {
            let (w, h) = s.split_once('x')?;
            Some((w.parse().ok()?, h.parse().ok()?))
        })
        .filter(|&(w, h): &(u32, u32)| (64..=16384).contains(&w) && (64..=16384).contains(&h))
        .unwrap_or((3840, 2160))
}

fn start_options(args: &Args) -> Result<StartOptions, String> {
    let own = std::env::var_os("CDEPT_MAGIC_SHAREDIR")
        .map(PathBuf::from)
        .or_else(|| {
            let exe = std::env::current_exe().ok()?;
            Some(exe.parent()?.parent()?.join("share/cdept-magic"))
        })
        .filter(|dir| dir.join("cdept-magic.tcl").is_file())
        .unwrap_or_else(|| PathBuf::from(env!("CDEPT_MAGIC_DEFAULT_TCLDIR")));
    let faketk = [
        own.join("faketk.tcl"),
        PathBuf::from(env!("CDEPT_MAGIC_DEFAULT_FAKETK")),
    ]
    .into_iter()
    .find(|f| f.is_file())
    .ok_or("the Tk stand-in faketk.tcl was not found; set CDEPT_MAGIC_SHAREDIR")?;
    Ok(StartOptions {
        magic_tcl_dir: engine::find_magic_tcl_dir(args.magic_tcl.as_deref())?,
        magic_rc: args.magic_rc.clone(),
        rcfile: args.rcfile.clone(),
        faketk,
        own_tcl_dir: own,
        display: args.display,
        screen: screen_size(),
    })
}

/// Everything the request handlers share. Magic calls back into this program while a
/// request is handled (a script waits in a nested event loop), so nothing here is
/// borrowed across a call into Magic.
struct Helper {
    magic: Magic,
    writer: RefCell<UnixStream>,
    reader: RefCell<UnixStream>,
    partial: RefCell<Vec<u8>>,
    queue: RefCell<VecDeque<Request>>,
    frame_dir: Option<PathBuf>,
    frame: RefCell<Option<FrameWriter>>,
    serial: Cell<u64>,
    title: RefCell<String>,
}

impl Helper {
    fn send(&self, message: &Message) {
        let _ = protocol::write_message(&mut *self.writer.borrow_mut(), message);
    }

    fn exit(&self) -> ! {
        if let Some(frame) = self.frame.borrow().as_ref() {
            frame.unlink();
        }
        std::process::exit(0)
    }

    fn readable(&self) {
        let mut buffer = [0u8; 16384];
        let count = match self.reader.borrow_mut().read(&mut buffer) {
            Ok(0) => self.exit(),
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
                    Err(error) => eprintln!("cdept-magic: bad request: {error}"),
                }
            }
        }
        self.drain();
        self.present();
    }

    fn drain(&self) {
        loop {
            let request = {
                let mut queue = self.queue.borrow_mut();
                let Some(request) = queue.pop_front() else {
                    return;
                };
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
                    self.magic.when_idle();
                }
                request
            };
            self.handle(request);
        }
    }

    fn handle(&self, request: Request) {
        trace(&format!("request {request:?}"));
        match request {
            Request::Tcl { id, script } => {
                let (ok, value) = match self.magic.eval(&script) {
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
                    self.magic.resize(width, height);
                    self.magic.run_pending_events();
                    self.new_frame();
                }
            }
            Request::Input { input } => {
                if self.frame_dir.is_some() {
                    self.magic.input(input);
                }
            }
            Request::Ui { event } => {
                if let Err(error) = self.magic.eval(&ui_event_script(&event)) {
                    eprintln!("cdept-magic: {event:?}: {error}");
                }
            }
        }
    }

    fn new_frame(&self) {
        let (Some(dir), Some((width, height))) = (&self.frame_dir, self.magic.display_size())
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
                let _ = self.magic.take_damage();
                self.copy_out(Rect::new(0, 0, width, height));
            }
            Err(error) => eprintln!("cdept-magic: cannot create a frame file: {error}"),
        }
    }

    fn present(&self) {
        if self.frame_dir.is_none() {
            return;
        }
        let title = self.magic.title();
        if *self.title.borrow() != title {
            self.send(&Message::Ui {
                update: UiUpdate::Title {
                    text: title.clone(),
                },
            });
            *self.title.borrow_mut() = title;
        }
        let Some((width, height)) = self.magic.display_size() else {
            return;
        };
        let fits = self
            .frame
            .borrow()
            .as_ref()
            .is_some_and(|f| f.width() == width && f.height() == height);
        if !fits {
            return self.new_frame();
        }
        if let Some(rect) = self.magic.take_damage() {
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
        self.magic.read_pixels(frame.pixels_mut(), stride, rect);
        self.send(&Message::Damage {
            serial: self.serial.get(),
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: rect.height,
        });
    }

    fn ui_update(&self, json: &str) {
        match serde_json::from_str::<UiUpdate>(json) {
            Ok(update) => self.send(&Message::Ui { update }),
            Err(error) => {
                let start: String = json.chars().take(200).collect();
                eprintln!("cdept-magic: bad widget update ({error}): {start}");
            }
        }
    }
}

/// With `$CDEPT_MAGIC_LOG` set: a line about each request in that file (debugging).
fn trace(text: &str) {
    if let Some(file) = std::env::var_os("CDEPT_MAGIC_LOG") {
        use std::io::Write as _;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(file)
        {
            let short: String = text.chars().take(300).collect();
            let _ = writeln!(f, "{short}");
        }
    }
}

/// The Tcl call that tells the Tk stand-in what the user did in a mirrored widget.
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
    let connect = args.connect.clone().expect("checked");
    let stream = UnixStream::connect(&connect)
        .map_err(|e| format!("cannot connect to {}: {e}", connect.display()))?;
    let mut writer = stream.try_clone().map_err(|e| e.to_string())?;
    let mut send = |message: Message| {
        protocol::write_message(&mut writer, &message).map_err(|e| format!("socket: {e}"))
    };

    let early: Rc<RefCell<Vec<String>>> = Rc::default();
    {
        let early = early.clone();
        engine::on_ui(move |json| early.borrow_mut().push(json.to_string()));
    }
    let options = match start_options(&args) {
        Ok(options) => options,
        Err(error) => {
            send(Message::Failed {
                error: error.clone(),
            })?;
            return Err(error);
        }
    };
    let magic = match Magic::start(&options) {
        Ok(magic) => magic,
        Err(error) => {
            // What Magic printed before it failed is the useful part.
            let log: Vec<String> = early
                .borrow()
                .iter()
                .filter_map(|json| match serde_json::from_str::<UiUpdate>(json) {
                    Ok(UiUpdate::Log { text }) => Some(text),
                    _ => None,
                })
                .collect();
            let tail: Vec<&str> = log.iter().flat_map(|t| t.lines()).collect();
            let tail = tail[tail.len().saturating_sub(15)..].join("\n");
            let error = if tail.is_empty() {
                error
            } else {
                format!("{error}\n{tail}")
            };
            send(Message::Failed {
                error: error.clone(),
            })?;
            return Err(error);
        }
    };
    send(Message::Ready {
        protocol: PROTOCOL_VERSION,
        xschem: magic.version(),
    })?;

    let fd = stream.as_raw_fd();
    let helper = Rc::new(Helper {
        magic,
        writer: RefCell::new(writer),
        reader: RefCell::new(stream),
        partial: RefCell::new(Vec::new()),
        queue: RefCell::new(VecDeque::new()),
        frame_dir: args.display.then_some(args.frame_dir).flatten(),
        frame: RefCell::new(None),
        serial: Cell::new(0),
        title: RefCell::new(String::new()),
    });
    if helper.frame_dir.is_some() {
        helper.magic.resize(args.size.0, args.size.1);
    }
    {
        let helper = helper.clone();
        helper.clone().magic.on_idle(move || {
            helper.drain();
            helper.present();
        });
    }
    for json in early.take() {
        helper.ui_update(&json);
    }
    {
        let helper = helper.clone();
        engine::on_ui(move |json| helper.ui_update(json));
    }
    {
        let helper = helper.clone();
        helper.clone().magic.watch(fd, move || helper.readable());
    }
    {
        // Magic redraws from its own idle and timer callbacks (background DRC, the
        // highlights): look for changed pixels a few times a second as well.
        let helper = helper.clone();
        helper.clone().magic.every(40, move || helper.present());
    }
    loop {
        helper.magic.do_one_event(true);
        helper.present();
    }
}

/// `--run`: start Magic, run scripts, dump the window. For tests and debugging.
fn run_test(args: Args) -> Result<(), String> {
    engine::on_ui(|json| {
        if let Ok(UiUpdate::Log { text }) = serde_json::from_str::<UiUpdate>(json) {
            for line in text.lines() {
                eprintln!("magic| {line}");
            }
        } else {
            eprintln!("ui| {}", json.chars().take(300).collect::<String>());
        }
    });
    let options = start_options(&args)?;
    let magic = Magic::start(&options)?;
    magic.resize(args.size.0, args.size.1);
    magic.run_pending_events();
    for script in &args.scripts {
        match magic.eval(script) {
            Ok(value) => println!("{value}"),
            Err(error) => println!("ERROR: {error}"),
        }
        magic.run_pending_events();
    }
    magic.run_pending_events();
    if let Some(ppm) = &args.ppm {
        let (width, height) = magic.display_size().ok_or("Magic has no window")?;
        let stride = width * 4;
        let mut pixels = vec![0u8; (stride * height) as usize];
        magic.read_pixels(&mut pixels, stride, Rect::new(0, 0, width, height));
        let mut out = format!("P6\n{width} {height}\n255\n").into_bytes();
        for pixel in pixels.as_chunks::<4>().0 {
            out.extend_from_slice(&pixel[..3]);
        }
        std::fs::write(ppm, out).map_err(|e| format!("{}: {e}", ppm.display()))?;
    }
    Ok(())
}
