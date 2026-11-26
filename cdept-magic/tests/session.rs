//! End-to-end tests of embedded Magic: the helper started by a
//! `libcdept::xschem::Session` (as a CDePT tab does), with a real Magic and sky130A.
//! Skipped when Magic or the PDK is not installed.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use libcdept::Project;
use libcdept::magic::embed;
use libcdept::xschem::input::{Input, button};
use libcdept::xschem::protocol::UiUpdate;
use libcdept::xschem::{Session, Status};

const MAGIC: &str = "/opt/ext/OpenLane/bin/magic";

/// A project with a `cells` library `logic` (cell `inv`, cell `top` placing it) in
/// sky130A, or `None` when Magic or the PDK is missing.
fn project(dir: &Path) -> Option<(Project, libcdept::testutil::RealPdk)> {
    if libcdept::magic::Magic::find().is_none() && !Path::new(MAGIC).is_file() {
        eprintln!("skipped: no magic");
        return None;
    }
    let pdk = libcdept::testutil::real_pdk("sky130A")?;
    if std::env::var_os("CDEPT_MAGIC").is_none() {
        // SAFETY: the PDK guard serialises the tests that touch the environment.
        unsafe { std::env::set_var("CDEPT_MAGIC", MAGIC) };
    }
    std::fs::write(
        dir.join("cdept.toml"),
        format!(
            "name = \"t\"\ntechnology = \"sky130A\"\npdk_root = \"{}\"\n[libraries]\nlogic = {{ path = \"logic\", layout = \"cells\" }}\n",
            pdk.root.display()
        ),
    )
    .unwrap();
    let inv = dir.join("logic/inv/layout");
    let top = dir.join("logic/top/layout");
    std::fs::create_dir_all(&inv).unwrap();
    std::fs::create_dir_all(&top).unwrap();
    std::fs::write(
        inv.join("inv.mag"),
        "magic\ntech sky130A\ntimestamp 1\n<< metal1 >>\nrect 0 0 100 100\n<< end >>\n",
    )
    .unwrap();
    std::fs::write(
        top.join("top.mag"),
        "magic\ntech sky130A\ntimestamp 2\n<< metal2 >>\nrect 300 0 400 100\n\
         use inv  inv_0 ../../inv/layout\ntimestamp 1\ntransform 1 0 0 0 1 0\nbox 0 0 100 100\n<< end >>\n",
    )
    .unwrap();
    Some((Project::load(dir).unwrap(), pdk))
}

fn open(project: &Project, file: &Path) -> Session {
    let mut options = embed::session_options(project, file, Arc::new(|| {}));
    options.engine.helper = Some(PathBuf::from(env!("CARGO_BIN_EXE_cdept-magic")));
    Session::open(options)
}

fn wait_for<T>(what: &str, secs: u64, mut check: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        if let Some(value) = check() {
            return value;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn tcl(session: &Session, script: &str) -> Result<String, String> {
    let id = session.tcl(script);
    let mut got = None;
    wait_for(script, 60, || {
        for r in session.take_replies() {
            if r.id == id {
                got = Some(r.result);
            }
        }
        got.take()
    })
}

fn frame_pixels(session: &Session, w: u32, h: u32) -> Vec<u8> {
    wait_for("a frame", 60, || {
        session
            .frame(|f| {
                (f.width() == w && f.height() == h).then(|| {
                    let mut px = Vec::new();
                    f.copy(f.full(), &mut px);
                    px
                })
            })
            .flatten()
    })
}

fn helper_pid(session: &Session) -> u32 {
    tcl(session, "pid").unwrap().trim().parse().unwrap()
}

#[test]
fn magic_draws_edits_and_survives_a_kill() {
    let tmp = tempfile::tempdir().unwrap();
    let Some((project, _pdk)) = project(tmp.path()) else {
        return;
    };
    let file = tmp.path().join("logic/top/layout/top.mag");
    let session = open(&project, &file);
    session.resize(400, 300);
    wait_for("running", 60, || (session.status() == Status::Running).then_some(()));
    assert_eq!(tcl(&session, "tech name").unwrap(), "sky130A");
    assert_eq!(tcl(&session, "cellname list self").unwrap(), "top");
    // The placed cell came from the other cell's folder through the relative path.
    let children = tcl(&session, "cellname list children top").unwrap();
    assert!(children.contains("inv"), "{children}");
    let path = tcl(&session, "cellname list filepath inv").unwrap();
    assert!(path.contains("logic/top/layout/../../inv/layout"), "{path}");

    // Pixels: a 400x300 frame that is not all one colour (once Magic has drawn: the
    // damage of later drawing arrives after the first, blank, copy).
    wait_for("a drawn frame", 30, || {
        let px = frame_pixels(&session, 400, 300);
        assert_eq!(px.len(), 400 * 300 * 4);
        let first = px[..4].to_vec();
        px.chunks(4).any(|p| p != first).then_some(())
    });

    // A resize gives a new frame of the new size.
    session.resize(320, 200);
    frame_pixels(&session, 320, 200);

    // Input: the `b` key macro is `box`, a left click moves the box corner there.
    session.input(Input::Motion { x: 100, y: 100, state: 0 });
    session.input(Input::ButtonPress { button: button::LEFT, x: 100, y: 100, state: 0 });
    session.input(Input::ButtonRelease { button: button::LEFT, x: 100, y: 100, state: 0 });
    let box1 = tcl(&session, "box values").unwrap();
    session.input(Input::ButtonPress { button: button::RIGHT, x: 200, y: 50, state: 0 });
    session.input(Input::ButtonRelease { button: button::RIGHT, x: 200, y: 50, state: 0 });
    let box2 = tcl(&session, "box values").unwrap();
    assert_ne!(box1, box2, "the right button should move the box corner");

    // An edit, then the helper is killed before saving: it starts again with the
    // file as it was on disk (the edit is lost, the file is intact).
    tcl(&session, "box 0 0 50 50; paint m3").unwrap();
    assert_eq!(tcl(&session, "cdept::magic::modified").unwrap(), "top");
    let before = std::fs::read(&file).unwrap();
    let pid = helper_pid(&session);
    let epoch = session.epoch();
    // SAFETY: plain libc call on our own child.
    unsafe { libc_kill(pid as i32, 9) };
    wait_for("restarted", 60, || {
        (session.epoch() > epoch && session.status() == Status::Running).then_some(())
    });
    assert_ne!(helper_pid(&session), pid);
    assert_eq!(std::fs::read(&file).unwrap(), before, "a kill must not touch the file");
    assert_eq!(tcl(&session, "cellname list self").unwrap(), "top");
    assert_eq!(tcl(&session, "cdept::magic::modified").unwrap(), "");

    // Save writes the file; the log reports Magic's text.
    tcl(&session, "box 0 0 50 50; paint m3").unwrap();
    tcl(&session, embed::SAVE_SCRIPT).unwrap();
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("<< metal3 >>"), "{text}");
    let logs: Vec<String> = session
        .take_updates()
        .into_iter()
        .filter_map(|u| match u {
            UiUpdate::Log { text } => Some(text),
            _ => None,
        })
        .collect();
    assert!(!logs.is_empty());
}

unsafe extern "C" {
    #[link_name = "kill"]
    fn libc_kill(pid: i32, sig: i32) -> i32;
}
