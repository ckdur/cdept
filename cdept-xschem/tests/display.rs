//! Drawing: xschem with a display, on the stand-in X of `shim/xshim.c`.
//!
//! Driven through `cdx-tcl`, with the minimal Tk stand-in (`shim/tcl/tkmin.tcl`), so
//! that these tests check the C side alone.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// Run `script` in `cdx-tcl` with a 900x600 drawing window and the example project's
/// libraries on the search path. Returns what it printed and the window as RGB rows.
fn run(script: &str) -> (String, Vec<[u8; 3]>, usize) {
    let dir = tempfile::tempdir().unwrap();
    let rc = dir.path().join("rc");
    let libs = repo().join("examples/buffer/libs");
    std::fs::write(
        &rc,
        format!("append XSCHEM_LIBRARY_PATH :{}\n", libs.display()),
    )
    .unwrap();
    let ppm = dir.path().join("out.ppm");
    let output = Command::new(env!("CARGO_BIN_EXE_cdx-tcl"))
        .env("CDEPT_XSCHEM_TK", "tkmin")
        .args(["--display", "900x600", "--rcfile"])
        .arg(&rc)
        .arg("--ppm")
        .arg(&ppm)
        .args(["-e", script])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "cdx-tcl failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    // "P6\n<width> <height>\n255\n" and then the pixels.
    let data = std::fs::read(&ppm).unwrap();
    let mut header_end = 0;
    for _ in 0..3 {
        header_end += data[header_end..].iter().position(|&b| b == b'\n').unwrap() + 1;
    }
    let header = String::from_utf8_lossy(&data[..header_end]).into_owned();
    let mut fields = header.split_whitespace();
    assert_eq!(fields.next(), Some("P6"));
    let width: usize = fields.next().unwrap().parse().unwrap();
    let pixels = data[header_end..].as_chunks::<3>().0.to_vec();
    (stdout, pixels, width)
}

fn testbench() -> String {
    repo()
        .join("examples/buffer/libs/bench/tb_buf/schematic/tb_buf.sch")
        .display()
        .to_string()
}

/// Wires are drawn in a light blue (layer 1 of the dark colour scheme).
fn is_wire(p: &[u8; 3]) -> bool {
    p[0] < 60 && p[1] > 150 && p[2] > 200
}

#[test]
fn a_schematic_is_drawn_in_the_window() {
    let (stdout, pixels, width) = run(&format!(
        "xschem load {{{}}}; xschem zoom_full; puts \"size [winfo width .drw] [winfo height .drw]\"",
        testbench()
    ));
    assert!(stdout.contains("size 900 600"), "{stdout}");
    assert_eq!(width, 900);
    assert_eq!(pixels.len(), 900 * 600);
    let drawn = pixels.iter().filter(|p| **p != [0, 0, 0]).count();
    assert!(drawn > 3000, "only {drawn} pixels are not background");
    // The two wires of the testbench are horizontal: a row with a long run of wire colour.
    let longest_run = pixels
        .chunks_exact(width)
        .map(|row| row.iter().filter(|p| is_wire(p)).count())
        .max()
        .unwrap();
    assert!(
        longest_run > 200,
        "no wire found (longest run {longest_run})"
    );
}

#[test]
fn input_selects_zooms_and_reports_what_changed() {
    // X11 event numbers: 2 key press, 4 button press, 5 button release, 6 motion,
    // 7 enter.
    let script = format!(
        r#"
        xschem load {{{}}}
        xschem zoom_full
        cdept::event 7 450 300 0 0 0
        set zoom0 [xschem get zoom]
        cdept::damage
        # a rubber band around the whole window selects everything
        cdept::event 4 2 2 0 1 0
        cdept::event 6 300 200 0 0 256
        cdept::event 6 400 300 0 0 256
        puts "band damage: [cdept::damage]"
        cdept::event 6 897 597 0 0 256
        cdept::event 5 897 597 0 1 256
        puts "selected: [xschem get lastsel]"
        # Escape drops the selection
        cdept::event 2 450 300 65307 0 0
        puts "after escape: [xschem get lastsel]"
        # wheel up zooms in (a smaller zoom value: fewer units per pixel)
        cdept::event 4 450 300 0 4 0
        cdept::event 5 450 300 0 4 0
        puts "zoomed in: [expr {{[xschem get zoom] < $zoom0}}]"
        puts "zoom damage: [cdept::damage]"
        # a resize gives a window of the new size, drawn again
        cdept::resize 640 480
        puts "resized: [winfo width .drw] [winfo height .drw] damage [cdept::damage]"
        "#,
        testbench()
    );
    let (stdout, pixels, width) = run(&script);
    let line = |start: &str| {
        stdout
            .lines()
            .find_map(|l| l.strip_prefix(start))
            .unwrap_or_else(|| panic!("no line '{start}' in:\n{stdout}"))
            .trim()
            .to_string()
    };
    // The rubber band is two pixels wide at most: the damage is its box, not the window.
    let band: Vec<i32> = line("band damage:")
        .split_whitespace()
        .map(|n| n.parse().unwrap())
        .collect();
    assert_eq!(band.len(), 4, "{stdout}");
    assert!(
        band[2] <= 420 && band[3] <= 320,
        "band damage too large: {band:?}"
    );
    let selected: u32 = line("selected:").parse().unwrap();
    assert!(
        selected >= 10,
        "the rubber band selected {selected} objects"
    );
    assert_eq!(line("after escape:"), "0");
    assert_eq!(line("zoomed in:"), "1");
    assert_eq!(line("zoom damage:"), "0 0 900 600");
    assert_eq!(line("resized:"), "640 480 damage 0 0 640 480");
    assert_eq!(width, 640);
    assert_eq!(pixels.len(), 640 * 480);
    assert!(pixels.iter().any(is_wire));
}

/// Two lines one pixel wide with one pixel between them stay two lines, upright and
/// lying. (cairo, stroking several lines in one go, fills that gap; the lines are
/// drawn by the X server's own code instead.)
#[test]
fn close_parallel_lines_stay_apart() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("lines.sch");
    std::fs::write(
        &file,
        "v {xschem version=3.4.5 file_version=1.2\n}\nG {}\nK {}\nV {}\nS {}\nE {}\n\
         L 4 0 0 0 1000 {}\nL 4 20 0 20 1000 {}\nL 4 500 2000 3000 2000 {}\nL 4 500 2020 3000 2020 {}\n",
    )
    .unwrap();
    // Ten units to the pixel: the lines are at x = 10 and 12, and at y = 210 and 212.
    let (_, pixels, width) = run(&format!(
        "set draw_grid 0; set draw_grid_axes 0; xschem load {{{}}}; \
         xschem origin 100 100 10",
        file.display()
    ));
    let lit = |x: usize, y: usize| pixels[y * width + x] != [0, 0, 0];
    for y in 20..100 {
        assert!(lit(10, y) && lit(12, y), "a line is missing in row {y}");
        assert!(
            !lit(9, y) && !lit(11, y) && !lit(13, y),
            "row {y} is filled"
        );
    }
    for x in 80..300 {
        assert!(
            lit(x, 210) && lit(x, 212),
            "a line is missing in column {x}"
        );
        assert!(
            !lit(x, 209) && !lit(x, 211) && !lit(x, 213),
            "column {x} is filled"
        );
    }
}

// ---- through a Session: what the GUI sees --------------------------------------------------

mod session {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use libcdept::Project;
    use libcdept::xschem::input::{Input, button, state};
    use libcdept::xschem::{EngineOptions, Session, SessionOptions, Status};

    use super::repo;

    pub const WIDTH: u32 = 900;
    pub const HEIGHT: u32 = 600;

    /// The example testbench in a session with a 900x600 drawing window, drawn.
    pub fn open() -> Session {
        open_sized(WIDTH, HEIGHT)
    }

    /// The same with another window size. The file is loaded before the window gets
    /// its size, as when the GUI opens a tab.
    pub fn open_sized(width: u32, height: u32) -> Session {
        let root = repo().join("examples/buffer");
        let project = Project::load(&root).unwrap();
        let session = Session::open(SessionOptions {
            engine: EngineOptions {
                helper: Some(PathBuf::from(env!("CARGO_BIN_EXE_cdept-xschem"))),
                ..EngineOptions::for_project(&project)
            },
            file: Some(root.join("libs/bench/tb_buf/schematic/tb_buf.sch")),
            wake: Arc::new(|| {}),
        });
        let deadline = Instant::now() + Duration::from_secs(20);
        while session.status() != Status::Running {
            assert!(Instant::now() < deadline, "the helper does not start");
            std::thread::sleep(Duration::from_millis(2));
        }
        sync(&session);
        session.resize(width, height);
        sync(&session);
        session
    }

    /// The result of a Tcl script.
    pub fn eval(session: &Session, script: &str) -> String {
        let id = session.tcl(script);
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(reply) = session.take_replies().into_iter().find(|r| r.id == id) {
                return reply.result.unwrap();
            }
            assert!(Instant::now() < deadline, "the helper does not answer");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// Wait until the helper has handled everything sent so far and its frames have
    /// arrived. A reply is sent before the damage of its batch, so ask twice.
    pub fn sync(session: &Session) {
        for _ in 0..2 {
            let id = session.tcl("expr {1}");
            let deadline = Instant::now() + Duration::from_secs(20);
            while !session.take_replies().iter().any(|r| r.id == id) {
                assert!(Instant::now() < deadline, "the helper does not answer");
                std::thread::sleep(Duration::from_millis(2));
            }
        }
    }

    /// The whole frame as RGB pixels, row after row.
    pub fn picture(session: &Session) -> Vec<[u8; 3]> {
        session
            .frame(|frame| {
                assert_eq!((frame.width(), frame.height()), (WIDTH, HEIGHT));
                let mut rgba = Vec::new();
                frame.copy(frame.full(), &mut rgba);
                rgba.as_chunks::<4>()
                    .0
                    .iter()
                    .map(|p| [p[0], p[1], p[2]])
                    .collect()
            })
            .expect("no frame")
    }

    pub fn motion(session: &Session, x: i32, y: i32, state: u32) {
        session.input(Input::Motion { x, y, state });
        // One at a time: a run of movements is sent as its last one only.
        sync(session);
    }

    pub fn press(session: &Session, x: i32, y: i32) {
        session.input(Input::ButtonPress {
            button: button::LEFT,
            x,
            y,
            state: 0,
        });
        sync(session);
    }

    pub fn release(session: &Session, x: i32, y: i32) {
        session.input(Input::ButtonRelease {
            button: button::LEFT,
            x,
            y,
            state: state::BUTTON1,
        });
        sync(session);
    }
}

/// Pixels of `now` that differ from `before` inside the box.
fn changed(
    before: &[[u8; 3]],
    now: &[[u8; 3]],
    (x1, y1, x2, y2): (usize, usize, usize, usize),
) -> usize {
    let width = session::WIDTH as usize;
    (y1..y2)
        .flat_map(|y| (x1..x2).map(move |x| y * width + x))
        .filter(|&i| before[i] != now[i])
        .count()
}

/// A drag over empty space shows a rubber band in the frames while it lasts, and the
/// band is gone afterwards.
#[test]
fn the_rubber_band_shows_in_the_frames_during_a_drag() {
    use libcdept::xschem::input::state::BUTTON1;
    let s = session::open();
    session::motion(&s, 40, 30, 0);
    let before = session::picture(&s);

    // Top-left corner of the window: empty in the testbench at full zoom.
    session::press(&s, 40, 30);
    session::motion(&s, 120, 90, BUTTON1);
    session::motion(&s, 200, 140, BUTTON1);
    session::motion(&s, 260, 180, BUTTON1);
    let during = session::picture(&s);
    // The band is the outline of the box from (40, 30) to (260, 180).
    let top = changed(&before, &during, (60, 28, 240, 33));
    let left = changed(&before, &during, (38, 50, 43, 160));
    let bottom = changed(&before, &during, (60, 178, 240, 183));
    let right = changed(&before, &during, (258, 50, 263, 160));
    assert!(
        top > 100 && left > 60 && bottom > 100 && right > 60,
        "no rubber band in the frame: top {top}, left {left}, bottom {bottom}, right {right}"
    );
    // Nothing of an earlier, smaller band is left inside.
    assert_eq!(changed(&before, &during, (125, 95, 195, 135)), 0);

    session::release(&s, 260, 180);
    let after = session::picture(&s);
    let left_over = changed(&before, &after, (0, 0, 300, 200));
    assert_eq!(left_over, 0, "the band was not erased");
}

/// A quick drag can reach xschem as a press and a single movement. The band shows
/// all the same.
#[test]
fn the_rubber_band_shows_after_a_single_movement() {
    use libcdept::xschem::input::state::BUTTON1;
    let s = session::open();
    session::motion(&s, 40, 30, 0);
    let before = session::picture(&s);
    session::press(&s, 40, 30);
    session::motion(&s, 260, 180, BUTTON1);
    let during = session::picture(&s);
    let top = changed(&before, &during, (60, 28, 240, 33));
    let right = changed(&before, &during, (258, 50, 263, 160));
    assert!(
        top > 100 && right > 60,
        "no rubber band: top {top}, right {right}"
    );
    session::release(&s, 260, 180);
    assert_eq!(changed(&before, &session::picture(&s), (0, 0, 300, 200)), 0);
}

/// The first view fits the whole schematic, also its longest text: the block of
/// simulation commands, whose width xschem measured before the window had its size.
#[test]
fn the_first_view_fits_the_long_text() {
    // A tab gets its size in steps. The last one is the size at which the text used
    // to run out of the window.
    let (width, height) = (947.0, 614.0);
    let s = session::open_sized(640, 400);
    s.resize(width as u32, height as u32);
    session::sync(&s);
    let numbers = |script: &str| -> Vec<f64> {
        session::eval(&s, script)
            .split_whitespace()
            .filter_map(|word| word.parse().ok())
            .collect()
    };
    // "Instance: x1 y1 x2 y2\nSymbol: …": the box of the instance with its texts,
    // measured again at the zoom now in use.
    session::eval(&s, "xschem update_all_sym_bboxes");
    let bbox = numbers("xschem instance_bbox SIM");
    let view = numbers("list [xschem get xorigin] [xschem get yorigin] [xschem get zoom]");
    let (xorigin, yorigin, zoom) = (view[0], view[1], view[2]);
    let right = (bbox[2] + xorigin) / zoom;
    let bottom = (bbox[3] + yorigin) / zoom;
    assert!(
        right <= width && bottom <= height,
        "the text reaches ({right:.0}, {bottom:.0}) in a {width}x{height} window"
    );
    // ... and it is a fit, not a view from far away.
    assert!(
        right > 0.85 * width,
        "the text ends at {right:.0} of {width}"
    );

    // Once the user has chosen a view, a new size leaves it alone, as in xschem.
    session::eval(&s, "xschem zoom_box -100 -100 100 100");
    let chosen = numbers("xschem get zoom");
    s.resize(800, 500);
    session::sync(&s);
    assert_eq!(numbers("xschem get zoom"), chosen);
}
