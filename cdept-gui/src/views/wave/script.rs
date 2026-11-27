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
        )
    )
}

pub fn patience(args: &[String]) -> Duration {
    match args.first().map(String::as_str) {
        // A 1 GB raw file is converted once: that takes a while.
        Some("wait_loaded" | "expect_failed") => Duration::from_secs(600),
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
        click <fx> [left|right] | expect_cache <text> | expect_changed <yes|no> | reload | expect_files <dir> <text> <n> | stats";
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
            let got = app.script_wave()?.script_cursor_value(i, signal)?;
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
            let a = w.script_cursor_value(0, signal)?;
            let b = w.script_cursor_value(1, signal)?;
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
    fn point_raw_prefers_out_raw() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("tran1.raw"), "x").unwrap();
        assert_eq!(point_raw(dir.path()), dir.path().join("tran1.raw"));
        std::fs::write(dir.path().join("out.raw"), "x").unwrap();
        assert_eq!(point_raw(dir.path()), dir.path().join("out.raw"));
    }
}
