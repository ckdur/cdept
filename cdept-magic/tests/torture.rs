//! Torture tests of embedded Magic (real Magic and sky130A; skipped without them):
//! floods of input and sizes, absurd sizes, a large layout, file names with spaces and
//! non-ASCII letters, Tcl errors, Magic's own `quit`, two helpers at once.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use libcdept::Project;
use libcdept::magic::embed;
use libcdept::xschem::input::{Input, button};
use libcdept::xschem::{Session, Status};

const MAGIC: &str = "/opt/ext/OpenLane/bin/magic";

fn project(
    dir: &Path,
    lib_dir: &str,
    cell: &str,
    body: &str,
) -> Option<(Project, PathBuf, libcdept::testutil::RealPdk)> {
    if libcdept::magic::Magic::find().is_none() && !Path::new(MAGIC).is_file() {
        eprintln!("skipped: no magic");
        return None;
    }
    let pdk = libcdept::testutil::real_pdk("sky130A")?;
    if std::env::var_os("CDEPT_MAGIC").is_none() {
        // SAFETY: the PDK guard serialises environment changes.
        unsafe { std::env::set_var("CDEPT_MAGIC", MAGIC) };
    }
    std::fs::write(
        dir.join("cdept.toml"),
        format!(
            "name = \"t\"\ntechnology = \"sky130A\"\npdk_root = \"{}\"\n[libraries]\nlib = {{ path = \"{lib_dir}\", layout = \"cells\" }}\n",
            pdk.root.display()
        ),
    )
    .unwrap();
    let file = dir
        .join(lib_dir)
        .join(cell)
        .join("layout")
        .join(format!("{cell}.mag"));
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, body).unwrap();
    Some((Project::load(dir).unwrap(), file, pdk))
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
    wait_for(script, 120, || {
        for r in session.take_replies() {
            if r.id == id {
                got = Some(r.result);
            }
        }
        got.take()
    })
}

fn frame_size(session: &Session) -> Option<(u32, u32)> {
    session.frame(|f| (f.width(), f.height()))
}

const SMALL: &str = "magic\ntech sky130A\ntimestamp 1\n<< metal1 >>\nrect 0 0 100 100\n<< end >>\n";

#[test]
fn floods_of_input_and_sizes_and_absurd_sizes() {
    let tmp = tempfile::tempdir().unwrap();
    let Some((p, file, _pdk)) = project(tmp.path(), "lib", "a", SMALL) else {
        return;
    };
    let s = open(&p, &file);
    s.resize(300, 200);
    wait_for("running", 60, || {
        (s.status() == Status::Running).then_some(())
    });
    // 3000 motions, presses and keys, and 60 sizes, as fast as they can be queued.
    for i in 0..3000i32 {
        let (x, y) = (i % 290, (i * 7) % 190);
        s.input(Input::Motion { x, y, state: 0 });
        if i % 50 == 0 {
            s.input(Input::ButtonPress {
                button: button::LEFT,
                x,
                y,
                state: 0,
            });
            s.input(Input::ButtonRelease {
                button: button::LEFT,
                x,
                y,
                state: 0,
            });
            s.input(Input::KeyPress {
                keysym: 'z' as u32,
                x,
                y,
                state: 0,
            });
            s.input(Input::KeyRelease {
                keysym: 'z' as u32,
                x,
                y,
                state: 0,
            });
        }
        if i % 50 == 25 {
            s.resize(200 + (i as u32 % 400), 150 + (i as u32 % 300));
        }
    }
    s.resize(640, 480);
    assert_eq!(tcl(&s, "expr 1+1").unwrap(), "2");
    wait_for("the last size", 60, || {
        (frame_size(&s) == Some((640, 480))).then_some(())
    });
    assert_eq!(s.status(), Status::Running);
    // Sizes beyond the screen are clamped; zero is ignored.
    s.resize(100_000, 100_000);
    wait_for("a clamped frame", 60, || {
        frame_size(&s)
            .filter(|&(w, h)| w <= 3840 && h <= 2160 && w > 640)
            .map(|_| ())
    });
    s.resize(0, 0);
    s.resize(1, 1);
    // The smallest window is 64x64 (Magic keeps a minimum size of its own).
    wait_for("a 64x64 frame", 60, || {
        (frame_size(&s) == Some((64, 64))).then_some(())
    });
    assert_eq!(tcl(&s, "tech name").unwrap(), "sky130A");
    // Tcl errors come back as errors; the helper goes on.
    assert!(tcl(&s, "this_is_not_a_command").is_err());
    assert!(tcl(&s, "error boom").unwrap_err().contains("boom"));
    assert!(tcl(&s, "load {}").is_ok() || tcl(&s, "expr 1").is_ok());
    assert_eq!(tcl(&s, "expr 2*3").unwrap(), "6");
}

#[test]
fn a_large_layout_and_names_with_spaces_and_letters() {
    let tmp = tempfile::tempdir().unwrap();
    // 40000 rectangles on two layers.
    let mut body = String::from("magic\ntech sky130A\ntimestamp 1\n<< metal1 >>\n");
    for i in 0..20_000 {
        let (x, y) = ((i % 200) * 60, (i / 200) * 60);
        body += &format!("rect {x} {y} {} {}\n", x + 40, y + 40);
    }
    body += "<< metal2 >>\n";
    for i in 0..20_000 {
        let (x, y) = ((i % 200) * 60 + 20, (i / 200) * 60 + 20);
        body += &format!("rect {x} {y} {} {}\n", x + 40, y + 40);
    }
    body += "<< end >>\n";
    let Some((p, file, _pdk)) = project(tmp.path(), "my lib é", "big", &body) else {
        return;
    };
    let s = open(&p, &file);
    s.resize(500, 400);
    let started = Instant::now();
    wait_for("running", 120, || {
        (s.status() == Status::Running).then_some(())
    });
    assert_eq!(tcl(&s, "lindex [cellname list window] 0").unwrap(), "big");
    // Magic cannot take such a path: it works in the folder (MAGIC.md).
    let path = tcl(&s, "pwd").unwrap();
    assert!(path.contains("my lib é"), "{path}");
    // The whole cell drawn in a reasonable time.
    tcl(&s, "view").unwrap();
    wait_for("a frame", 120, || frame_size(&s));
    assert!(started.elapsed() < Duration::from_secs(120));
    // An edit and a save in the folder with a space and an accent.
    tcl(&s, "box 0 0 50 50; paint m3").unwrap();
    tcl(&s, embed::SAVE_SCRIPT).unwrap();
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("<< metal3 >>"));
    assert!(text.matches("rect ").count() > 40_000);
}

#[test]
fn magic_quit_closes_the_session_and_two_helpers_run_side_by_side() {
    let tmp = tempfile::tempdir().unwrap();
    let Some((_, file, _pdk)) = project(tmp.path(), "lib", "a", SMALL) else {
        return;
    };
    let other = tmp.path().join("lib/b/layout/b.mag");
    std::fs::create_dir_all(other.parent().unwrap()).unwrap();
    std::fs::write(&other, SMALL.replace("metal1", "metal2")).unwrap();
    let p = Project::load(tmp.path()).unwrap();
    let a = open(&p, &file);
    let b = open(&p, &other);
    a.resize(300, 200);
    b.resize(320, 240);
    for s in [&a, &b] {
        wait_for("running", 60, || {
            (s.status() == Status::Running).then_some(())
        });
    }
    assert_eq!(tcl(&a, "lindex [cellname list window] 0").unwrap(), "a");
    assert_eq!(tcl(&b, "lindex [cellname list window] 0").unwrap(), "b");
    wait_for("a's frame", 60, || {
        (frame_size(&a) == Some((300, 200))).then_some(())
    });
    wait_for("b's frame", 60, || {
        (frame_size(&b) == Some((320, 240))).then_some(())
    });
    // Magic's own quit (no unsaved changes): the helper ends, the session is closed,
    // the other one runs on.
    let _ = a.tcl("quit -noprompt");
    wait_for("closed", 60, || {
        (a.status() == Status::Closed).then_some(())
    });
    assert_eq!(tcl(&b, "expr 7").unwrap(), "7");
}
