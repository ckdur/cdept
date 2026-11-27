//! `wv …` commands of a test script (docs/GUI-TESTING.md): open waveform tabs, drive
//! them with the mouse and check what they draw.
//!
//! | Command | Does |
//! |---|---|
//! | `wv open <file>` | A new waveform tab on the file (the script then acts on the newest tab) |
//! | `wv open_point <lib> <cell> <view> <point>` | The same for a run point's raw file |
//! | `wv select <n>\|newest` | Act on the n-th waveform tab (oldest is 1) |
//! | `wv expect_tabs <n>` | n waveform tabs are open |
//! | `wv wait_loaded` | Wait until the tab's file is open (fails when it failed) |
//! | `wv expect_failed <text>` | The tab failed to open, with this text in the message |
//! | `wv expect_loading` | The tab is still loading |
//! | `wv cancel`, `wv close`, `wv detach` | Cancel the load; close the tab (its X); move it to a window of its own |
//! | `wv show <signal>…`, `wv hide <signal>…` | Show or hide signals |
//! | `wv fit`, `wv zoom <x0> <x1>` | The whole x range; a given one |
//! | `wv expect_range <x0> <x1> [tolerance]` | The x range on screen (relative tolerance, default 1e-6) |
//! | `wv cursor <a\|b> <x\|none>` | Put or remove a cursor |
//! | `wv expect_cursor_x <a\|b> <x> [tolerance]` | Where a cursor is |
//! | `wv expect_cursor <a\|b> <signal> <value> [tolerance]` | The readout of a signal at a cursor |
//! | `wv expect_delta <signal> <value> [tolerance]` | Its B−A readout |
//! | `wv expect_drawn <min\|max> <signal> <value> [tolerance]` | The smallest or largest value drawn in the x range on screen |
//! | `wv check_extremes <signal>` | What is drawn has the exact extremes of the samples in the x range (every sample read) |
//! | `wv wheel <fx> <up\|down> [count] [ctrl\|shift]` | Turn the wheel over the plot at a fraction `fx` of its width (real input) |
//! | `wv drag <fx1> <fx2>` | Drag across the plot (real input) |
//! | `wv click <fx> [left\|right]` | Click on the plot (real input) |
//! | `wv expect_cache <text>` | The `.uwraw` being read has this text in its path |
//! | `wv expect_changed <yes\|no>` | Whether the file changed on disk since it was read |
//! | `wv reload` | Open the file again (what Reload does) |
//! | `wv expect_files <dir> <text> <n>` | The folder has n files whose name contains the text (caches, temporary files) |
//! | `wv stats` | Log frame times, query times and memory since the last `wv stats` |
//! | `wv open_files <file>…` | One tab on several files overlaid (tags = file names) |
//! | `wv open_family <lib> <cell> <view> [point…]` | What "Waveform Tab (family)" does: every run point (or these) in one tab |
//! | `wv add_file <file> [tag]` | Add a file to the tab |
//! | `wv expect_points <loading\|ready\|failed\|memory\|all> <n>` | How many sources of the tab are in that state |
//! | `wv expect_point <tag> <state> [text]` | A source's state (and its message) |
//! | `wv point <tag> on\|off`, `wv points all\|none` | Show or hide a point (source) |
//! | `wv show_in <n\|new> <signal>…` | Show signals in pane n or a new pane |
//! | `wv pane new\|remove\|up\|down\|select\|fit_y [n]` | Add, remove, move, activate a pane, fit its y |
//! | `wv expect_panes <n>`, `wv expect_pane <n> <signal>…` | Pane count; the signals of pane n, in order |
//! | `wv expect_x_unit <n> <unit>` | The x unit of pane n |
//! | `wv expect_lines <n\|>n>` | Lines drawn now |
//! | `wv y <n> <y0> <y1>\|fit`, `wv expect_y <n> <y0> <y1> [tol]` | Pane n's y range |
//! | `wv list <flat\|kind\|tree> [search…]`, `wv expect_rows <n>`, `wv expect_row <text>`, `wv fold <header>` | The signal list: grouping, search, rows |
//! | `wv pick <signal>…`, `wv pick_add <signal>…`, `wv picked show\|new\|hide` | Select in the list; what its buttons do with the selection |
//! | `wv drag_signal <signal> <n>` | Drag a signal from the list onto pane n (real input) |
//! | `wv style <signal> <color #rrggbb\|dash solid\|dashed\|dotted\|dashdot\|width w> [tag]` | Change a line's style |
//! | `wv expect_style <n> <label> <color\|dash\|width> <value>` | The style of a line of pane n (label contains the text) |
//! | `wv expect_distinct <n>` | No two lines of pane n look alike |
//! | `wv expect_changed_files <n>` | Files of the tab that changed on disk |
//! | `wv expect_markers <n>` | The calculator's markers in the tab |
//! | `wv expect_cursor_at <a\|b> <signal> <tag> <value> [tol]` | The readout of one point's line at a cursor |
//! | `wv test_trace <name> <empty\|nan\|inf\|nonmono\|ramp\|sine\|big> [n] [x unit] [tag]` | Hand a series to the tab as the calculator does (hostile ones too) |
//! | `wv analyses`, `wv analysis <n\|kind\|name>`, `wv expect_analysis <text>` | Log the analyses of the first file; open another (every file); the open one's name or kind has the text |
//! | `wv form <signal> <dB20\|mag\|phase\|uphase\|re\|im\|dB10>`, `wv ac_form <form>` | The form one shown complex signal is drawn in; the form of all (and of those shown later) |
//! | `wv bode <signal>` | Bode plot of a complex signal (gain and unwrapped phase panes, margin markers) |
//! | `wv expect_marker <text> [x] [tol]` | A marker whose label has the text (at x, relative tolerance default 1e-3) |
//! | `wv eye <signal> <period> [offset] [lines\|density]`, `wv expect_note <n> <text>` | An eye diagram in a pane of its own; a line note of pane n, 0 for any (traces drawn, or why not) |
//! | `wv integrate <signal>` | The integrated noise of a noise density in a new pane (`integ(<signal>)`) |
//! | `wv log <n> <x\|y> on\|off`, `wv expect_log <n> <x\|y> on\|off` | Log axes of pane n |
//! | `wv expect_value <signal> <value> [tol]` | The table of values (an operating point) has this value for the first file |
//! | `wv expect_parts <n> [order]`, `wv expect_cursor_part <a\|b> <signal> <part> <value> [tol]` | Parts of x of the first file (a parametric sweep), its order; one part's readout |
//!
//! Numbers take SI suffixes: `2.5u`, `10n`, `1meg`. Mouse commands use window
//! coordinates: no schematic or layout should be the script's document meanwhile.

use std::path::{Path, PathBuf};
use std::time::Duration;

use egui::{Modifiers, PointerButton, Pos2};

use crate::app::CdeptApp;
use crate::dock::Tab;
use crate::state::Action;

/// Mouse input the script driver feeds to egui.
pub enum Input {
    Wheel(Pos2, f32, u32, Modifiers),
    Drag(Pos2, Pos2),
    Click(Pos2, PointerButton),
}

/// Commands that are tried again until they pass (or their patience ends).
pub fn patient(args: &[String]) -> bool {
    matches!(
        args.first().map(String::as_str),
        Some(
            "wait_loaded"
                | "expect_failed"
                | "expect_tabs"
                | "expect_range"
                | "expect_cursor"
                | "expect_cursor_x"
                | "expect_delta"
                | "expect_drawn"
                | "expect_cache"
                | "expect_changed"
                | "expect_files"
                | "check_extremes"
                | "wheel"
                | "drag"
                | "click"
                | "expect_points"
                | "expect_point"
                | "expect_lines"
                | "expect_y"
                | "expect_cursor_at"
                | "drag_signal"
                | "expect_markers"
                | "expect_changed_files"
                | "expect_marker"
                | "expect_note"
                | "expect_analysis"
                | "expect_value"
                | "expect_parts"
                | "expect_cursor_part"
        )
    )
}

pub fn patience(args: &[String]) -> Duration {
    match args.first().map(String::as_str) {
        // A 1 GB raw file is converted once: that takes a while.
        Some("wait_loaded" | "expect_failed") => Duration::from_secs(600),
        Some("expect_points" | "expect_point") => Duration::from_secs(120),
        _ => Duration::from_secs(20),
    }
}

/// The raw file of a run point folder: `out.raw`, else the first `*.raw` (VACASK
/// names it after the analysis).
pub fn point_raw(dir: &Path) -> PathBuf {
    let out = dir.join("out.raw");
    if out.is_file() {
        return out;
    }
    let mut raws: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "raw"))
        .collect();
    raws.sort();
    raws.into_iter().next().unwrap_or(out)
}

/// A number with an optional SI suffix (`f p n u µ m k meg g t`).
pub fn number(text: &str) -> Result<f64, String> {
    let t = text.trim();
    let lower = t.to_ascii_lowercase();
    // The suffix becomes an exponent, so 2.5u is exactly 2.5e-6.
    let (digits, exponent) = [
        ("meg", "e6"),
        ("f", "e-15"),
        ("p", "e-12"),
        ("n", "e-9"),
        ("u", "e-6"),
        ("µ", "e-6"),
        ("m", "e-3"),
        ("k", "e3"),
        ("g", "e9"),
        ("t", "e12"),
    ]
    .iter()
    .find_map(|(s, e)| lower.strip_suffix(s).map(|d| (d.to_string(), *e)))
    .unwrap_or((lower.clone(), ""));
    format!("{digits}{exponent}")
        .parse::<f64>()
        .map_err(|_| format!("'{text}' is not a number"))
}

/// Tags for files opened together: the file name; files of one name (`out.raw` of every
/// run point) are told apart by their folder (`p000`); still equal ones get a number.
pub fn tag_files(paths: Vec<PathBuf>) -> Vec<(String, PathBuf)> {
    let name = |p: &PathBuf| {
        p.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| p.display().to_string())
    };
    let dir = |p: &PathBuf| {
        p.parent()
            .and_then(|d| d.file_name())
            .map(|d| d.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    for p in &paths {
        let n = name(p);
        let twins = paths.iter().filter(|q| name(q) == n).count() > 1;
        let tag = if twins { dir(p) } else { n };
        let mut unique = tag.clone();
        let mut k = 2;
        while out.iter().any(|(t, _)| *t == unique) {
            unique = format!("{tag} ({k})");
            k += 1;
        }
        out.push((unique, p.clone()));
    }
    out
}

fn on_off(text: &str) -> Result<bool, String> {
    match text {
        "on" | "yes" | "show" => Ok(true),
        "off" | "no" | "hide" => Ok(false),
        _ => Err(format!("'{text}' is not on or off")),
    }
}

/// `#rrggbb`.
fn hex_color(text: &str) -> Result<[u8; 3], String> {
    let h = text.trim_start_matches('#');
    let bad = || format!("'{text}' is not a colour (#rrggbb)");
    if h.len() != 6 {
        return Err(bad());
    }
    let v = u32::from_str_radix(h, 16).map_err(|_| bad())?;
    Ok([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

/// A series for `wv test_trace`: what the calculator could hand over, hostile ones too.
fn test_series(
    name: &str,
    kind: &str,
    n: usize,
    x_unit: &str,
) -> Result<cdept_wavedata::Series, String> {
    let x: Vec<f64> = (0..n).map(|i| i as f64 * 1e-9).collect();
    let (x, y): (Vec<f64>, Vec<f64>) = match kind {
        "empty" => (vec![], vec![]),
        "ramp" => (x, (0..n).map(|i| i as f64 / n.max(1) as f64).collect()),
        "sine" => (x, (0..n).map(|i| (i as f64 * 0.01).sin()).collect()),
        "big" => (
            x,
            (0..n).map(|i| ((i % 1000) as f64 * 1e-3) - 0.5).collect(),
        ),
        "nan" => (
            x,
            (0..n)
                .map(|i| if i % 3 == 0 { f64::NAN } else { i as f64 })
                .collect(),
        ),
        "inf" => (
            x.iter()
                .enumerate()
                .map(|(i, v)| if i % 7 == 3 { f64::INFINITY } else { *v })
                .collect(),
            (0..n)
                .map(|i| match i % 5 {
                    1 => f64::INFINITY,
                    2 => f64::NEG_INFINITY,
                    _ => (i as f64).sqrt(),
                })
                .collect(),
        ),
        "nonmono" => (
            (0..n).map(|i| (i as f64 * 0.05).sin()).collect(),
            (0..n).map(|i| (i as f64 * 0.07).cos()).collect(),
        ),
        _ => {
            return Err(format!(
                "'{kind}' is not empty, nan, inf, nonmono, ramp, sine or big"
            ));
        }
    };
    Ok(cdept_wavedata::Series::new(name, "V", x_unit, x, y))
}

fn close_to(got: f64, want: f64, tol: f64) -> bool {
    got == want || (got - want).abs() <= tol * want.abs().max(1e-300) || (got - want).abs() <= tol
}

fn cursor_index(text: &str) -> Result<usize, String> {
    match text {
        "a" | "A" => Ok(0),
        "b" | "B" => Ok(1),
        _ => Err(format!("'{text}' is not a cursor (a or b)")),
    }
}

fn tolerance(args: &[String], at: usize, default: f64) -> Result<f64, String> {
    args.get(at).map_or(Ok(default), |t| number(t))
}

/// Run one `wv` command. Mouse commands return the input to feed.
pub fn run(
    app: &mut CdeptApp,
    ctx: &egui::Context,
    args: &[String],
) -> Result<Option<Input>, String> {
    const USAGE: &str = "usage: wv open <file> | open_point <lib> <cell> <view> <point> | select <n|newest> | \
        expect_tabs <n> | wait_loaded | expect_failed <text> | expect_loading | cancel | close | detach | \
        show <signal>… | hide <signal>… | fit | zoom <x0> <x1> | expect_range <x0> <x1> [tol] | \
        cursor <a|b> <x|none> | expect_cursor_x <a|b> <x> [tol] | expect_cursor <a|b> <signal> <value> [tol] | \
        expect_delta <signal> <value> [tol] | expect_drawn <min|max> <signal> <value> [tol] | \
        check_extremes <signal> | wheel <fx> <up|down> [count] [ctrl|shift] | drag <fx1> <fx2> | \
        click <fx> [left|right] | expect_cache <text> | expect_changed <yes|no> | reload | expect_files <dir> <text> <n> | stats | \
        open_files <file>... | open_family <lib> <cell> <view> [point...] | add_file <file> [tag] | expect_points <state> <n> | \
        expect_point <tag> <state> [text] | point <tag> on|off | points all|none | show_in <n|new> <signal>... | \
        pane new|remove|up|down|select|fit_y [n] | expect_panes <n> | expect_pane <n> <signal>... | expect_x_unit <n> <unit> | \
        expect_lines <n|>n> | y <n> <y0> <y1>|fit | expect_y <n> <y0> <y1> [tol] | list <flat|kind|tree> [search] | \
        expect_rows <n> | expect_row <text> | fold <header> | pick <signal>... | pick_add <signal>... | expect_picked <signal>... | \
        picked show|new|hide | drag_signal <signal> <n> | style <signal> color|dash|width <value> [tag] | \
        expect_style <n> <label> color|dash|width <value> | expect_distinct <n> | expect_markers <n> | \
        expect_cursor_at <a|b> <signal> <tag> <value> [tol] | test_trace <name> <kind> [n] [x unit] [tag] | \
        analyses | analysis <n|kind|name> | expect_analysis <text> | form <signal> <form> | ac_form <form> | bode <signal> | \
        expect_marker <text> [x] [tol] | eye <signal> <period> [offset] [lines|density] | expect_note <n> <text> | \
        integrate <signal> | log <n> <x|y> on|off | expect_log <n> <x|y> on|off | expect_value <signal> <value> [tol] | \
        expect_parts <n> [order] | expect_cursor_part <a|b> <signal> <part> <value> [tol]";
    let verb = args.first().map(String::as_str).unwrap_or("");
    let rest = args.get(1..).unwrap_or_default();
    match (verb, rest) {
        ("open", file) if !file.is_empty() => {
            let path = std::path::absolute(file.join(" ")).map_err(|e| e.to_string())?;
            app.script_select_wave(None);
            app.script_request(Action::OpenWave(path, None));
        }
        ("open_point", [lib, cell, view, point]) => {
            let project = app.script_project().ok_or("no project is open")?;
            let dir = libcdept::sim::runs_dir(project, lib, cell, view).join(point);
            app.script_select_wave(None);
            app.script_request(Action::OpenWave(
                point_raw(&dir),
                Some(format!("{cell} {point}")),
            ));
        }
        ("select", [which]) => {
            let (ids, _) = app.script_wave_ids();
            if which == "newest" {
                app.script_select_wave(None);
            } else {
                let n: usize = which
                    .parse()
                    .map_err(|_| format!("'{which}' is not a tab number"))?;
                let id = *ids
                    .get(n.wrapping_sub(1))
                    .ok_or_else(|| format!("there are {} waveform tabs", ids.len()))?;
                app.script_select_wave(Some(id));
            }
        }
        ("expect_tabs", [n]) => {
            let want: usize = n.parse().map_err(|_| format!("'{n}' is not a number"))?;
            let (ids, _) = app.script_wave_ids();
            if ids.len() != want {
                return Err(format!("{} waveform tabs, expected {want}", ids.len()));
            }
        }
        ("wait_loaded", []) => {
            let w = app.script_wave()?;
            if let Some(e) = w.script_failed() {
                // Not worth waiting for: fail now.
                return Err(format!("the file did not open: {e}"));
            }
            w.script_state()?;
        }
        ("expect_failed", text) => {
            let w = app.script_wave()?;
            match w.script_failed() {
                Some(e) if e.contains(&text.join(" ")) => {
                    tracing::info!(target: "cdept::script", "wv: failed as expected: {e}");
                }
                Some(e) => return Err(format!("failed with another message: {e}")),
                None => return Err("the tab did not fail (yet)".into()),
            }
        }
        ("expect_loading", []) => {
            if !app.script_wave()?.script_loading() {
                return Err("the tab is not loading".into());
            }
        }
        ("cancel", []) => app.script_wave()?.script_cancel(),
        ("close", []) => {
            let (_, id) = app.script_wave_ids();
            let id = id.ok_or("no waveform tab is open")?;
            app.script_request(Action::CloseTab(Tab::Wave(id)));
        }
        ("detach", []) => {
            let (_, id) = app.script_wave_ids();
            let id = id.ok_or("no waveform tab is open")?;
            app.script_request(Action::DetachTab(Tab::Wave(id)));
        }
        ("show" | "hide", names) if !names.is_empty() => {
            app.script_wave()?.script_show(names, verb == "show")?;
        }
        ("fit", []) => app.script_wave()?.script_fit()?,
        ("zoom", [x0, x1]) => app.script_wave()?.script_zoom(number(x0)?, number(x1)?)?,
        ("expect_range", [x0, x1, tol @ ..]) if tol.len() <= 1 => {
            let tol = tolerance(tol, 0, 1e-6)?;
            let (a, b) = app.script_wave()?.script_range()?;
            let (w0, w1) = (number(x0)?, number(x1)?);
            let span = (w1 - w0).abs().max(f64::MIN_POSITIVE);
            if (a - w0).abs() > tol * span || (b - w1).abs() > tol * span {
                return Err(format!(
                    "the x range is {a:e} .. {b:e}, expected {w0:e} .. {w1:e}"
                ));
            }
        }
        ("cursor", [which, x]) => {
            let i = cursor_index(which)?;
            let x = if x == "none" { None } else { Some(number(x)?) };
            app.script_wave()?.script_cursor(i, x)?;
        }
        ("expect_cursor_x", [which, x, tol @ ..]) if tol.len() <= 1 => {
            let i = cursor_index(which)?;
            let want = number(x)?;
            let tol = tolerance(tol, 0, 1e-6)?;
            let w = app.script_wave()?;
            let (a, b) = w.script_range()?;
            match w.script_cursor_x(i)? {
                Some(got) if (got - want).abs() <= tol * (b - a).abs() => {}
                got => return Err(format!("cursor {which} is at {got:?}, expected {want:e}")),
            }
        }
        ("expect_cursor", [which, signal, value, tol @ ..]) if tol.len() <= 1 => {
            let i = cursor_index(which)?;
            let want = number(value)?;
            let tol = tolerance(tol, 0, 1e-9)?;
            let got = app.script_wave()?.script_cursor_value(i, signal, None)?;
            if !close_to(got, want, tol) {
                return Err(format!(
                    "{signal} at cursor {which} is {got:e}, expected {want:e}"
                ));
            }
            tracing::info!(target: "cdept::script", "wv: {signal} at {which} = {got:e}");
        }
        ("expect_delta", [signal, value, tol @ ..]) if tol.len() <= 1 => {
            let want = number(value)?;
            let tol = tolerance(tol, 0, 1e-9)?;
            let w = app.script_wave()?;
            let a = w.script_cursor_value(0, signal, None)?;
            let b = w.script_cursor_value(1, signal, None)?;
            if !close_to(b - a, want, tol) {
                return Err(format!("{signal}: B−A is {:e}, expected {want:e}", b - a));
            }
        }
        ("expect_drawn", [which, signal, value, tol @ ..]) if tol.len() <= 1 => {
            let want = number(value)?;
            let tol = tolerance(tol, 0, 1e-9)?;
            let (lo, hi, n) = app.script_wave()?.script_drawn_extremes(signal)?;
            let got = match which.as_str() {
                "min" => lo,
                "max" => hi,
                _ => return Err(USAGE.into()),
            };
            if !close_to(got, want, tol) {
                return Err(format!(
                    "drawn {which} of {signal} is {got:e} ({n} points), expected {want:e}"
                ));
            }
            tracing::info!(target: "cdept::script", "wv: drawn {which} of {signal} = {got:e} ({n} points)");
        }
        ("check_extremes", [signal]) => {
            let w = app.script_wave()?;
            let (lo, hi, n) = w.script_drawn_extremes(signal)?;
            let started = std::time::Instant::now();
            let (elo, ehi, count) = w.script_exact_extremes(signal)?;
            if lo != elo || hi != ehi {
                return Err(format!(
                    "{signal}: drawn min/max {lo:e}/{hi:e} ({n} points), every sample {elo:e}/{ehi:e} ({count} samples)"
                ));
            }
            tracing::info!(
                target: "cdept::script",
                "wv: {signal}: drawn min/max {lo:e}/{hi:e} from {n} points = exact over {count} samples ({:.0} ms to read them)",
                started.elapsed().as_secs_f64() * 1e3
            );
        }
        ("wheel", [fx, dir, more @ ..]) if more.len() <= 2 => {
            let rect = app.script_wave()?.script_plot_rect()?;
            let fx = number(fx)?;
            let notch = match dir.as_str() {
                "up" => 1.0,
                "down" => -1.0,
                _ => return Err(USAGE.into()),
            };
            let mut count = 1;
            let mut modifiers = Modifiers::NONE;
            for m in more {
                match m.as_str() {
                    "ctrl" => modifiers = Modifiers::CTRL,
                    "shift" => modifiers = Modifiers::SHIFT,
                    n => count = n.parse().map_err(|_| format!("'{n}' is not a count"))?,
                }
            }
            let at = Pos2::new(rect.left() + rect.width() * fx as f32, rect.center().y);
            return Ok(Some(Input::Wheel(at, notch, count, modifiers)));
        }
        ("drag", [fx1, fx2]) => {
            let rect = app.script_wave()?.script_plot_rect()?;
            let y = rect.center().y;
            let at = |f: f64| Pos2::new(rect.left() + rect.width() * f as f32, y);
            return Ok(Some(Input::Drag(at(number(fx1)?), at(number(fx2)?))));
        }
        ("click", [fx, button @ ..]) if button.len() <= 1 => {
            let rect = app.script_wave()?.script_plot_rect()?;
            let button = match button.first().map(String::as_str) {
                None | Some("left") => PointerButton::Primary,
                Some("right") => PointerButton::Secondary,
                Some(b) => return Err(format!("'{b}' is not a button (left, right)")),
            };
            let at = Pos2::new(
                rect.left() + rect.width() * number(fx)? as f32,
                rect.center().y,
            );
            return Ok(Some(Input::Click(at, button)));
        }
        ("expect_cache", text) if !text.is_empty() => {
            let (cache, _) = app.script_wave()?.script_cache()?;
            if !cache.contains(&text.join(" ")) {
                return Err(format!("the cache is {cache}"));
            }
            tracing::info!(target: "cdept::script", "wv: cache {cache}");
        }
        ("expect_changed", [yes]) => {
            let want = match yes.as_str() {
                "yes" => true,
                "no" => false,
                _ => return Err(USAGE.into()),
            };
            let (_, changed) = app.script_wave()?.script_cache()?;
            if changed != want {
                return Err(format!("the file changed: {changed}, expected {want}"));
            }
        }
        ("expect_files", [dir, text, n]) => {
            let want: usize = n.parse().map_err(|_| format!("'{n}' is not a number"))?;
            let names: Vec<String> = std::fs::read_dir(dir)
                .map_err(|e| format!("{dir}: {e}"))?
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.contains(text.as_str()))
                .collect();
            if names.len() != want {
                return Err(format!(
                    "{} files with '{text}' in {dir}: {names:?}",
                    names.len()
                ));
            }
        }
        ("reload", []) => {
            app.script_wave()?.script_reload(ctx);
        }
        ("stats", []) => {
            let line = app.script_wave()?.script_stats()?;
            tracing::info!(target: "cdept::script", "wv stats: {line}");
        }
        ("open_files", files) if !files.is_empty() => {
            let paths: Vec<PathBuf> = files
                .iter()
                .map(std::path::absolute)
                .collect::<Result<_, _>>()
                .map_err(|e| e.to_string())?;
            let tagged = tag_files(paths);
            let title = format!("{} files", tagged.len());
            app.script_select_wave(None);
            app.script_request(Action::OpenWaveFiles(tagged, title));
        }
        ("open_family", [lib, cell, view, points @ ..]) => {
            app.script_select_wave(None);
            app.script_request(Action::OpenWaveFamily(
                lib.clone(),
                cell.clone(),
                view.clone(),
                points.to_vec(),
            ));
        }
        ("add_file", [file, tag @ ..]) if tag.len() <= 1 => {
            let path = std::path::absolute(file).map_err(|e| e.to_string())?;
            let tag = tag.first().cloned().unwrap_or_else(|| {
                path.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            });
            app.script_wave()?.script_add_file(tag, path);
        }
        ("expect_points", [state, n]) => {
            let want: usize = n.parse().map_err(|_| format!("'{n}' is not a number"))?;
            let w = app.script_wave()?;
            let sources = w.script_sources();
            let got = sources
                .iter()
                .filter(|(_, s, _, _)| state == "all" || s == state)
                .count();
            if got != want {
                let summary: Vec<String> = sources
                    .iter()
                    .take(12)
                    .map(|(t, s, _, _)| format!("{t}:{s}"))
                    .collect();
                return Err(format!(
                    "{got} sources {state}, expected {want} ({} of {}: {})",
                    summary.len(),
                    sources.len(),
                    summary.join(" ")
                ));
            }
        }
        ("expect_point", [tag, state, text @ ..]) => {
            let w = app.script_wave()?;
            let sources = w.script_sources();
            let (_, got, message, _) = sources
                .iter()
                .find(|(t, _, _, _)| t == tag)
                .ok_or_else(|| format!("no point '{tag}'"))?;
            let text = text.join(" ");
            if got != state || !message.contains(&text) {
                return Err(format!(
                    "point {tag} is {got} ({message}), expected {state} with '{text}'"
                ));
            }
            tracing::info!(target: "cdept::script", "wv: point {tag} {got}: {message}");
        }
        ("point", [tag, on]) => {
            let on = on_off(on)?;
            app.script_wave()?.script_point(tag, on)?;
        }
        ("points", [all]) => match all.as_str() {
            "all" => app.script_wave()?.script_points(true),
            "none" => app.script_wave()?.script_points(false),
            _ => return Err(USAGE.into()),
        },
        ("show_in", [pane, names @ ..]) if !names.is_empty() => {
            app.script_wave()?.script_show_in(pane, names)?;
        }
        ("pane", [verb, n @ ..]) if n.len() <= 1 => {
            app.script_wave()?
                .script_pane(verb, n.first().map(String::as_str))?;
        }
        ("expect_panes", [n]) => {
            let want: usize = n.parse().map_err(|_| format!("'{n}' is not a number"))?;
            let panes = app.script_wave()?.script_panes();
            if panes.len() != want {
                return Err(format!("{} panes, expected {want}: {panes:?}", panes.len()));
            }
        }
        ("expect_pane", [n, names @ ..]) => {
            let n: usize = n.parse().map_err(|_| format!("'{n}' is not a number"))?;
            let panes = app.script_wave()?.script_panes();
            let (signals, _) = panes
                .get(n.wrapping_sub(1))
                .ok_or_else(|| format!("there are {} panes", panes.len()))?;
            if signals != names {
                return Err(format!("pane {n} has {signals:?}, expected {names:?}"));
            }
        }
        ("expect_x_unit", [n, unit]) => {
            let n: usize = n.parse().map_err(|_| format!("'{n}' is not a number"))?;
            let got = app.script_wave()?.script_x_unit(n)?;
            if &got != unit {
                return Err(format!("pane {n} has x in '{got}', expected '{unit}'"));
            }
        }
        ("expect_lines", [n]) => {
            let (more, digits) = n
                .strip_prefix('>')
                .map_or((false, n.as_str()), |d| (true, d));
            let want: usize = digits
                .parse()
                .map_err(|_| format!("'{n}' is not a number"))?;
            let got = app.script_wave()?.script_lines()?;
            if (more && got <= want) || (!more && got != want) {
                return Err(format!("{got} lines drawn, expected {n}"));
            }
            tracing::info!(target: "cdept::script", "wv: {got} lines drawn");
        }
        ("y", [n, rest @ ..]) => {
            let n: usize = n.parse().map_err(|_| format!("'{n}' is not a number"))?;
            let y = match rest {
                [fit] if fit == "fit" => None,
                [a, b] => Some((number(a)?, number(b)?)),
                _ => return Err(USAGE.into()),
            };
            app.script_wave()?.script_set_y(n, y)?;
        }
        ("expect_y", [n, y0, y1, tol @ ..]) if tol.len() <= 1 => {
            let n: usize = n.parse().map_err(|_| format!("'{n}' is not a number"))?;
            let tol = tolerance(tol, 0, 1e-6)?;
            let (a, b) = app.script_wave()?.script_y(n)?;
            let (w0, w1) = (number(y0)?, number(y1)?);
            let span = (w1 - w0).abs().max(f64::MIN_POSITIVE);
            if (a - w0).abs() > tol * span || (b - w1).abs() > tol * span {
                return Err(format!(
                    "pane {n}: y is {a:e} .. {b:e}, expected {w0:e} .. {w1:e}"
                ));
            }
        }
        ("list", [grouping, filter @ ..]) => {
            let g = super::model::Grouping::parse(grouping)
                .ok_or_else(|| format!("'{grouping}' is not flat, kind or tree"))?;
            let rows = app
                .script_wave()?
                .script_list(Some(&filter.join(" ")), Some(g));
            tracing::info!(target: "cdept::script", "wv list: {} rows: {}", rows.len(), rows.iter().take(30).cloned().collect::<Vec<_>>().join(" | "));
        }
        ("expect_rows", [n]) => {
            let want: usize = n.parse().map_err(|_| format!("'{n}' is not a number"))?;
            let rows = app.script_wave()?.script_list(None, None);
            if rows.len() != want {
                return Err(format!(
                    "{} rows, expected {want}: {:?}",
                    rows.len(),
                    rows.iter().take(20).collect::<Vec<_>>()
                ));
            }
        }
        ("expect_row", text) if !text.is_empty() => {
            let text = text.join(" ");
            let rows = app.script_wave()?.script_list(None, None);
            if !rows.iter().any(|r| r.contains(&text)) {
                return Err(format!(
                    "no row has '{text}': {:?}",
                    rows.iter().take(20).collect::<Vec<_>>()
                ));
            }
        }
        ("fold", key) if !key.is_empty() => {
            app.script_wave()?.script_fold(&key.join(" "));
        }
        ("pick" | "pick_add", names) if !names.is_empty() => {
            app.script_wave()?
                .script_select(names, verb == "pick_add")?;
        }
        ("expect_picked", names) => {
            let got = app.script_wave()?.script_selected();
            let mut want: Vec<String> = names.to_vec();
            want.sort();
            if got != want {
                return Err(format!("selected {got:?}, expected {want:?}"));
            }
        }
        ("picked", [how]) => {
            app.script_wave()?.script_send_selected(how)?;
        }
        ("drag_signal", [signal, pane]) => {
            let n: usize = pane
                .parse()
                .map_err(|_| format!("'{pane}' is not a number"))?;
            let w = app.script_wave()?;
            let from = w.script_row_rect(signal)?.center();
            let to = w.script_pane_rect(n)?.center();
            return Ok(Some(Input::Drag(from, to)));
        }
        ("style", [signal, key, value, tag @ ..]) if tag.len() <= 1 => {
            let (mut color, mut dash, mut width) = (None, None, None);
            match key.as_str() {
                "color" | "colour" => color = Some(hex_color(value)?),
                "dash" => {
                    dash = Some(super::raster::Dash::parse(value).ok_or_else(|| {
                        format!("'{value}' is not solid, dashed, dotted or dashdot")
                    })?)
                }
                "width" => width = Some(number(value)? as f32),
                _ => return Err(USAGE.into()),
            }
            app.script_wave()?.script_style(
                signal,
                tag.first().map(String::as_str),
                color,
                dash,
                width,
            )?;
        }
        ("expect_style", [n, label, key, value]) => {
            let n: usize = n.parse().map_err(|_| format!("'{n}' is not a number"))?;
            let styles = app.script_wave()?.script_styles(n)?;
            let (l, st) = styles
                .iter()
                .find(|(l, _)| l.contains(label.as_str()))
                .ok_or_else(|| format!("no line '{label}' in pane {n}"))?;
            let ok = match key.as_str() {
                "color" | "colour" => st.color == hex_color(value)?,
                "dash" => Some(st.dash) == super::raster::Dash::parse(value),
                "width" => (st.width - number(value)? as f32).abs() < 1e-3,
                _ => return Err(USAGE.into()),
            };
            if !ok {
                return Err(format!("{l}: {st:?}, expected {key} {value}"));
            }
        }
        ("expect_distinct", [n]) => {
            let n: usize = n.parse().map_err(|_| format!("'{n}' is not a number"))?;
            let styles = app.script_wave()?.script_styles(n)?;
            for (i, (a, sa)) in styles.iter().enumerate() {
                if let Some((b, _)) = styles[i + 1..].iter().find(|(_, sb)| sb == sa) {
                    return Err(format!("'{a}' and '{b}' look alike: {sa:?}"));
                }
            }
            tracing::info!(target: "cdept::script", "wv: the {} lines of pane {n} all look different", styles.len());
        }
        ("expect_changed_files", [n]) => {
            let want: usize = n.parse().map_err(|_| format!("'{n}' is not a number"))?;
            let got = app.script_wave()?.script_changed_files();
            if got != want {
                return Err(format!("{got} files changed on disk, expected {want}"));
            }
        }
        ("expect_markers", [n]) => {
            let want: usize = n.parse().map_err(|_| format!("'{n}' is not a number"))?;
            let w = app.script_wave()?;
            let markers = w.script_markers();
            if markers.len() != want {
                return Err(format!(
                    "{} markers, expected {want}: {markers:?}",
                    markers.len()
                ));
            }
            for m in markers.iter().take(5) {
                tracing::info!(target: "cdept::script", "wv: marker {} at {:e} {}", m.label, m.x, m.x_unit);
            }
        }
        ("expect_cursor_at", [which, signal, tag, value, tol @ ..]) if tol.len() <= 1 => {
            let i = cursor_index(which)?;
            let want = number(value)?;
            let tol = tolerance(tol, 0, 1e-9)?;
            let got = app
                .script_wave()?
                .script_cursor_value(i, signal, Some(tag))?;
            if !close_to(got, want, tol) {
                return Err(format!(
                    "{signal} @ {tag} at cursor {which} is {got:e}, expected {want:e}"
                ));
            }
        }
        ("test_trace", [name, kind, rest @ ..]) if rest.len() <= 3 => {
            let n: usize = match rest.first() {
                Some(t) => number(t)? as usize,
                None => 1000,
            };
            let x_unit = rest.get(1).cloned().unwrap_or_else(|| "s".into());
            let tag = rest.get(2).cloned().unwrap_or_else(|| "calc".into());
            let series = test_series(name, kind, n.min(200_000_000), &x_unit)?;
            app.script_wave()?.add_series(&tag, series);
        }
        ("analyses", []) => {
            let (list, open) = app.script_wave()?.script_analyses()?;
            for a in &list {
                tracing::info!(
                    target: "cdept::script",
                    "wv analysis {}{}: {} ({}) {} points, {} signals{}{}",
                    a.index + 1,
                    if a.index == open { " (open)" } else { "" },
                    a.name,
                    a.kind,
                    a.points,
                    a.signals,
                    if a.complex { ", complex" } else { "" },
                    if a.plottable { String::new() } else { format!(", not plottable: {}", a.reason) }
                );
            }
        }
        ("analysis", [which]) => app.script_wave()?.script_set_analysis(which)?,
        ("expect_analysis", text) if !text.is_empty() => {
            let text = text.join(" ");
            let (list, open) = app.script_wave()?.script_analyses()?;
            let a = list.get(open as usize).ok_or("no analysis is open")?;
            if !a.name.contains(&text) && a.kind != text {
                return Err(format!(
                    "the open analysis is {} ({}), expected '{text}'",
                    a.name, a.kind
                ));
            }
        }
        ("form", [signal, form]) => app.script_wave()?.script_form(signal, form)?,
        ("ac_form", [form]) => app.script_wave()?.script_ac_form(form)?,
        ("bode", [signal]) => {
            if let Some(note) = app.script_wave()?.script_bode(signal)? {
                tracing::info!(target: "cdept::script", "wv bode: {note}");
            }
        }
        ("expect_marker", [text, rest @ ..]) if rest.len() <= 2 => {
            let markers = app.script_wave()?.script_markers();
            let found: Vec<_> = markers
                .iter()
                .filter(|m| m.label.contains(text.as_str()))
                .collect();
            let Some(m) = found.first() else {
                return Err(format!(
                    "no marker '{text}': {:?}",
                    markers.iter().map(|m| m.label.clone()).collect::<Vec<_>>()
                ));
            };
            if let Some(x) = rest.first() {
                let want = number(x)?;
                let tol = tolerance(rest, 1, 1e-3)?;
                if !close_to(m.x, want, tol) {
                    return Err(format!(
                        "marker '{}' at {:e}, expected {want:e}",
                        m.label, m.x
                    ));
                }
            }
            tracing::info!(target: "cdept::script", "wv: marker '{}' at {:e} {}", m.label, m.x, m.x_unit);
        }
        ("eye", [signal, period, rest @ ..]) if rest.len() <= 2 => {
            let offset = match rest.first() {
                Some(o) if o != "lines" && o != "density" => number(o)?,
                _ => 0.0,
            };
            let density = rest.iter().any(|r| r == "density");
            app.script_wave()?.script_eye(
                signal,
                super::model::Eye {
                    period: number(period)?,
                    offset,
                    density,
                },
            )?;
        }
        ("expect_note", [n, text @ ..]) if !text.is_empty() => {
            let n: usize = n.parse().map_err(|_| format!("'{n}' is not a number"))?;
            let text = text.join(" ");
            let notes = app.script_wave()?.script_notes(n)?;
            if !notes.iter().any(|t| t.contains(&text)) {
                return Err(format!("pane {n}: notes {notes:?}, expected '{text}'"));
            }
            tracing::info!(target: "cdept::script", "wv: pane {n}: {}", notes.join("; "));
        }
        ("integrate", [signal]) => app.script_wave()?.script_integrate(signal)?,
        ("log", [n, axis, on]) => {
            let n: usize = n.parse().map_err(|_| format!("'{n}' is not a number"))?;
            let on = on_off(on)?;
            app.script_wave()?.script_log(n, axis == "y", on)?;
        }
        ("expect_log", [n, axis, on]) => {
            let n: usize = n.parse().map_err(|_| format!("'{n}' is not a number"))?;
            let on = on_off(on)?;
            let (x, y) = app.script_wave()?.script_is_log(n)?;
            let got = if axis == "y" { y } else { x };
            if got != on {
                return Err(format!("pane {n}: log {axis} is {got}, expected {on}"));
            }
        }
        ("expect_value", [signal, value, tol @ ..]) if tol.len() <= 1 => {
            let want = number(value)?;
            let tol = tolerance(tol, 0, 1e-9)?;
            let got = app.script_wave()?.script_table_value(signal)?;
            if !close_to(got, want, tol) {
                return Err(format!(
                    "{signal} is {got:e} in the table, expected {want:e}"
                ));
            }
            tracing::info!(target: "cdept::script", "wv: table {signal} = {got:e}");
        }
        ("expect_parts", [n, order @ ..]) if order.len() <= 1 => {
            let want: usize = n.parse().map_err(|_| format!("'{n}' is not a number"))?;
            let (got, o) = app.script_wave()?.script_parts()?;
            if got != want || order.first().is_some_and(|w| !o.eq_ignore_ascii_case(w)) {
                return Err(format!("{got} parts of x ({o}), expected {want} {order:?}"));
            }
        }
        ("expect_cursor_part", [which, signal, part, value, tol @ ..]) if tol.len() <= 1 => {
            let i = cursor_index(which)?;
            let part: usize = part
                .parse()
                .map_err(|_| format!("'{part}' is not a number"))?;
            let want = number(value)?;
            let tol = tolerance(tol, 0, 1e-9)?;
            let got = app
                .script_wave()?
                .script_cursor_part(i, signal, part.wrapping_sub(1))?;
            if !close_to(got, want, tol) {
                return Err(format!(
                    "{signal} part {part} at cursor {which} is {got:e}, expected {want:e}"
                ));
            }
        }
        _ => return Err(USAGE.into()),
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_with_suffixes() {
        assert_eq!(number("2.5u").unwrap(), 2.5e-6);
        assert_eq!(number("10n").unwrap(), 1e-8);
        assert_eq!(number("1meg").unwrap(), 1e6);
        assert_eq!(number("-3").unwrap(), -3.0);
        assert_eq!(number("1e-9").unwrap(), 1e-9);
        assert!(number("abc").is_err());
    }

    #[test]
    fn files_of_one_name_are_tagged_by_folder() {
        let t = tag_files(vec![
            "/a/p000/out.raw".into(),
            "/a/p001/out.raw".into(),
            "/b/x.raw".into(),
            "/c/p000/out.raw".into(),
        ]);
        let tags: Vec<&str> = t.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(tags, ["p000", "p001", "x.raw", "p000 (2)"]);
    }

    #[test]
    fn point_raw_prefers_out_raw() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("tran1.raw"), "x").unwrap();
        assert_eq!(point_raw(dir.path()), dir.path().join("tran1.raw"));
        std::fs::write(dir.path().join("out.raw"), "x").unwrap();
        assert_eq!(point_raw(dir.path()), dir.path().join("out.raw"));
    }
}
