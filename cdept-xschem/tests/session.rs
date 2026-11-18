//! End-to-end tests of `libcdept::xschem::Session`: a helper with a drawing window,
//! frames through shared memory, restart after the helper stops.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use libcdept::Project;
use libcdept::xschem::frame::Rect;
use libcdept::xschem::input::{Input, button};
use libcdept::xschem::protocol::{MenuItem, MenuKind, UiEvent, UiUpdate};
use libcdept::xschem::session::Reply;
use libcdept::xschem::{EngineOptions, Session, SessionOptions, Status};

fn example_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/buffer")
}

fn open(file: Option<&str>) -> Session {
    let root = example_root();
    let project = Project::load(&root).unwrap();
    Session::open(SessionOptions {
        engine: EngineOptions {
            helper: Some(PathBuf::from(env!("CARGO_BIN_EXE_cdept-xschem"))),
            ..EngineOptions::for_project(&project)
        },
        file: file.map(|f| root.join(f)),
        wake: Arc::new(|| {}),
    })
}

const TB_BUF: &str = "libs/bench/tb_buf/schematic/tb_buf.sch";

/// Poll until `check` gives a value.
fn wait_for<T>(what: &str, mut check: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(value) = check() {
            return value;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn wait_status(session: &Session, wanted: Status) {
    wait_for(&format!("{wanted:?}"), || {
        (session.status() == wanted).then_some(())
    });
}

/// Wait for a frame of this size and return its generation and all its pixels.
fn wait_frame(session: &Session, width: u32, height: u32) -> (u64, Vec<u8>) {
    wait_for(&format!("a {width}x{height} frame"), || {
        session
            .frame(|frame| {
                (frame.width() == width && frame.height() == height).then(|| {
                    let mut pixels = Vec::new();
                    frame.copy(frame.full(), &mut pixels);
                    (frame.generation, pixels)
                })
            })
            .flatten()
    })
}

fn wait_reply(session: &Session, id: u64) -> Reply {
    wait_for("a reply", || {
        session.take_replies().into_iter().find(|r| r.id == id)
    })
}

#[test]
fn frames_follow_the_size_and_tcl_is_answered() {
    let session = open(Some(TB_BUF));
    session.resize(640, 480);
    wait_status(&session, Status::Running);
    assert!(session.xschem_version().is_some());

    let (first, pixels) = wait_frame(&session, 640, 480);
    assert_eq!(pixels.len(), 640 * 480 * 4);
    // Something was drawn, and the frame is opaque.
    wait_for("a drawn frame", || {
        let (_, pixels) = wait_frame(&session, 640, 480);
        let drawn = pixels
            .as_chunks::<4>()
            .0
            .iter()
            .any(|p| p[..3] != [0, 0, 0]);
        let opaque = pixels.as_chunks::<4>().0.iter().all(|p| p[3] == 255);
        (drawn && opaque).then_some(())
    });

    let id = session.tcl("expr {6 * 7}");
    assert_eq!(wait_reply(&session, id).result, Ok("42".into()));
    let id = session.tcl("xschem get schname");
    let name = wait_reply(&session, id).result.unwrap();
    assert!(name.ends_with("tb_buf.sch"), "{name}");
    let id = session.tcl("no_such_command");
    assert!(wait_reply(&session, id).result.is_err());

    // A new size is a new frame.
    session.resize(800, 300);
    let (second, pixels) = wait_frame(&session, 800, 300);
    assert!(second > first);
    assert_eq!(pixels.len(), 800 * 300 * 4);

    // A rectangle outside the frame is clipped, not an error.
    let clipped = session
        .frame(|frame| frame.copy(Rect::new(790, 290, 100, 100), &mut Vec::new()))
        .unwrap();
    assert_eq!(clipped, Rect::new(790, 290, 10, 10));
}

#[test]
fn input_reaches_the_helper() {
    let session = open(Some(TB_BUF));
    session.resize(400, 300);
    wait_frame(&session, 400, 300);
    // Whatever xschem makes of a click, the helper must stay up and keep answering.
    for (x, y) in [(100, 100), (200, 150)] {
        session.input(Input::Motion { x, y, state: 0 });
        session.input(Input::ButtonPress {
            button: button::LEFT,
            x,
            y,
            state: 0,
        });
        session.input(Input::ButtonRelease {
            button: button::LEFT,
            x,
            y,
            state: 0x100,
        });
    }
    let id = session.tcl("expr {1 + 1}");
    assert_eq!(wait_reply(&session, id).result, Ok("2".into()));
    assert_eq!(session.status(), Status::Running);
}

#[test]
fn a_stopped_helper_is_started_again() {
    let session = open(Some(TB_BUF));
    session.resize(320, 200);
    let (before, _) = wait_frame(&session, 320, 200);

    // The helper dies (exit code 3: not a regular exit).
    session.tcl("exit 3");
    // It comes back, with the file loaded and the same window size.
    let (after, _) = wait_for("a frame from the new helper", || {
        let (generation, pixels) = wait_frame(&session, 320, 200);
        (generation > before).then_some((generation, pixels))
    });
    assert!(after > before);
    wait_status(&session, Status::Running);
    let id = session.tcl("xschem get schname");
    let name = wait_reply(&session, id).result.unwrap();
    assert!(name.ends_with("tb_buf.sch"), "{name}");
}

#[test]
fn a_regular_exit_closes_and_restart_reopens() {
    let session = open(Some(TB_BUF));
    wait_status(&session, Status::Running);
    session.tcl("exit 0");
    wait_status(&session, Status::Closed);

    session.restart();
    wait_status(&session, Status::Running);
    let id = session.tcl("expr {2 * 2}");
    assert_eq!(wait_reply(&session, id).result, Ok("4".into()));
}

#[test]
fn a_missing_helper_fails_with_an_explanation() {
    let session = Session::open(SessionOptions {
        engine: EngineOptions {
            helper: Some(PathBuf::from("/nonexistent/cdept-xschem")),
            ..Default::default()
        },
        file: None,
        wake: Arc::new(|| {}),
    });
    let message = wait_for("the failure", || match session.status() {
        Status::Failed(message) => Some(message),
        _ => None,
    });
    assert!(message.contains("/nonexistent/cdept-xschem"), "{message}");
    // Dropping a failed session does not hang.
}

/// Collects the widget updates of a session and keeps the latest of each kind.
#[derive(Default)]
struct Mirror {
    menu: Vec<MenuItem>,
    menus_seen: usize,
    toolbar: usize,
    status: usize,
    title: String,
}

impl Mirror {
    fn take(&mut self, session: &Session) {
        for update in session.take_updates() {
            match update {
                UiUpdate::Menu { items } => {
                    self.menu = items;
                    self.menus_seen += 1;
                }
                UiUpdate::Toolbar { items } => self.toolbar = items.len(),
                UiUpdate::Status { fields } => self.status = fields.len(),
                UiUpdate::Title { text } => self.title = text,
                _ => {}
            }
        }
    }

    /// The first entry of this kind anywhere in the menu bar.
    fn find(&self, wanted: impl Fn(&MenuItem) -> bool + Copy) -> Option<MenuItem> {
        fn walk(items: &[MenuItem], wanted: impl Fn(&MenuItem) -> bool + Copy) -> Option<MenuItem> {
            items.iter().find_map(|item| {
                if wanted(item) {
                    Some(item.clone())
                } else {
                    walk(&item.items, wanted)
                }
            })
        }
        walk(&self.menu, wanted)
    }
}

#[test]
fn xschems_menus_are_mirrored_and_can_be_used() {
    let session = open(Some(TB_BUF));
    session.resize(640, 480);
    let mut mirror = Mirror::default();
    wait_for("menu bar, tool bar, status bar and title", || {
        mirror.take(&session);
        (!mirror.menu.is_empty()
            && mirror.toolbar > 0
            && mirror.status > 0
            && mirror.title.contains("tb_buf"))
        .then_some(())
    });
    let top: Vec<&str> = mirror.menu.iter().map(|m| m.label.as_str()).collect();
    for label in ["File", "Edit", "View", "Properties"] {
        assert!(top.contains(&label), "no {label} menu in {top:?}");
    }

    // Choosing a check entry flips it, and the menu comes back with the new state.
    let entry = mirror
        .find(|item| item.kind == MenuKind::Check && item.enabled)
        .expect("a check entry");
    let seen = mirror.menus_seen;
    session.ui(UiEvent::Menu {
        menu: entry.menu.clone(),
        index: entry.index,
    });
    wait_for("the menu with the entry flipped", || {
        mirror.take(&session);
        let now = mirror.find(|item| item.menu == entry.menu && item.index == entry.index)?;
        (mirror.menus_seen > seen && now.checked != entry.checked).then_some(())
    });
    assert_eq!(session.status(), Status::Running);
}

/// xschem waiting inside a request (a dialog it wants answered) must still get the
/// requests that were already read from the socket along with that one.
#[test]
fn requests_queued_behind_a_waiting_one_are_served() {
    let session = open(None);
    wait_status(&session, Status::Running);
    // Sent together: the first waits for what only the second does.
    let waiting = session.tcl("vwait ::cdept_test_go; set ::cdept_test_go");
    let release = session.tcl("set ::cdept_test_go released");
    let mut replies = Vec::new();
    wait_for("both replies", || {
        replies.extend(session.take_replies());
        (replies.len() == 2).then_some(())
    });
    for id in [waiting, release] {
        let reply = replies.iter().find(|r| r.id == id).unwrap();
        assert_eq!(reply.result, Ok("released".into()));
    }
}
