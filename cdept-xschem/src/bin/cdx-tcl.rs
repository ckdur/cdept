//! `cdx-tcl`: run Tcl scripts in the xschem engine without CDePT, for developing and
//! testing the helper's C and Tcl sides.
//!
//! ```text
//! cdx-tcl [--display WIDTHxHEIGHT] [--rcfile FILE] [--ppm FILE] [-e SCRIPT | FILE]...
//! ```
//!
//! Scripts run in order; each result is printed. Widget updates (`cdept::ui <json>`)
//! are printed as lines starting with `ui: `. With `--display` xschem draws into a
//! window of that size; `--ppm` writes the window to a PPM image after the last script
//! (`convert out.ppm out.png` turns it into a PNG).

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use cdept_xschem::start::{self, StartOptions};
use libcdept::xschem::frame::Rect;

enum Script {
    Text(String),
    File(PathBuf),
}

fn main() -> ExitCode {
    let mut options = StartOptions::default();
    let mut size = None;
    let mut ppm = None;
    let mut scripts = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = |name: &str| {
            args.next().unwrap_or_else(|| {
                eprintln!("cdx-tcl: {name} needs a value");
                std::process::exit(2)
            })
        };
        match arg.as_str() {
            "--display" => {
                let text = value("--display");
                let parsed = text
                    .split_once('x')
                    .and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)));
                let Some(parsed) = parsed else {
                    eprintln!("cdx-tcl: --display wants WIDTHxHEIGHT, got '{text}'");
                    return ExitCode::from(2);
                };
                size = Some(parsed);
                options.display = true;
            }
            "--rcfile" => options.rcfile = Some(PathBuf::from(value("--rcfile"))),
            "--sharedir" => options.sharedir = Some(PathBuf::from(value("--sharedir"))),
            "--ppm" => ppm = Some(PathBuf::from(value("--ppm"))),
            "-e" => scripts.push(Script::Text(value("-e"))),
            file => scripts.push(Script::File(PathBuf::from(file))),
        }
    }

    cdept_xschem::xschem::on_ui(|json| println!("ui: {json}"));
    let xschem = match start::start(&options) {
        Ok(xschem) => xschem,
        Err(error) => {
            eprintln!("cdx-tcl: xschem did not start: {error}");
            return ExitCode::FAILURE;
        }
    };
    if let Some((width, height)) = size {
        xschem.resize(width, height);
        xschem.run_pending_events();
    }

    let mut failed = false;
    for script in scripts {
        let text = match script {
            Script::Text(text) => text,
            Script::File(file) => match std::fs::read_to_string(&file) {
                Ok(text) => text,
                Err(error) => {
                    eprintln!("cdx-tcl: {}: {error}", file.display());
                    return ExitCode::FAILURE;
                }
            },
        };
        match xschem.eval(&text) {
            Ok(result) => println!("{result}"),
            Err(error) => {
                eprintln!("error: {error}");
                failed = true;
            }
        }
        xschem.run_pending_events();
    }

    if let (Some(file), Some((width, height))) = (ppm, xschem.display_size()) {
        let stride = width * 4;
        let mut pixels = vec![0u8; (stride * height) as usize];
        xschem.read_pixels(&mut pixels, stride, Rect::new(0, 0, width, height));
        let mut out = format!("P6\n{width} {height}\n255\n").into_bytes();
        out.extend(
            pixels
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|p| [p[0], p[1], p[2]]),
        );
        let written = std::fs::File::create(&file).and_then(|mut f| f.write_all(&out));
        if let Err(error) = written {
            eprintln!("cdx-tcl: {}: {error}", file.display());
            failed = true;
        }
    }
    let _ = std::io::stdout().flush();
    // Leave through Tcl so that xschem removes its temporary folders.
    let _ = xschem.eval(if failed { "exit 1" } else { "exit 0" });
    ExitCode::from(u8::from(failed))
}
