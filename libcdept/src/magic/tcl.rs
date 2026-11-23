//! The Tcl fed to Magic. Every operation prints lines that start with [`MARK`] so the
//! results can be told from Magic's own chatter; a Tcl error prints `CDEPT: error …`
//! and quits, the end of a good run prints `CDEPT: done`.

use std::path::Path;

use super::{ExtractOptions, Parasitics, Source};

/// Prefix of the lines written for CDePT.
pub const MARK: &str = "CDEPT:";

/// Quote `s` as one Tcl word (braces would break on unbalanced ones, so backslash-escape).
pub fn word(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        if matches!(c, '"' | '\\' | '$' | '[' | ']') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

fn path(p: &Path) -> String {
    word(&p.to_string_lossy())
}

/// Load the cell, checking that it exists.
fn load(src: &Source) -> String {
    let mut t = String::new();
    match src {
        Source::Mag { file } => {
            let dir = file.parent().filter(|d| !d.as_os_str().is_empty());
            if let Some(dir) = dir {
                t += &format!("addpath {}\n", path(dir));
            }
            let stem = file.file_stem().map(|s| s.to_string_lossy().into_owned());
            t += &format!("load {}\n", word(&stem.unwrap_or_default()));
        }
        Source::Gds { file, cell } => {
            t += &format!("gds read {}\n", path(file));
            t += &format!(
                "if {{[cellname list exists {}] eq \"0\"}} {{ error \"no such cell in the GDS file\" }}\n",
                word(cell)
            );
            t += &format!("load {}\n", word(cell));
        }
    }
    t
}

/// The cell's name as Magic sees it, for a source.
pub fn cell_name(src: &Source) -> String {
    match src {
        Source::Mag { file } => file
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default(),
        Source::Gds { cell, .. } => cell.clone(),
    }
}

/// Wrap `body` so that an error is reported and ends the run, and success is marked.
fn guard(body: &str) -> String {
    format!(
        "if {{[catch {{\n{body}\n}} cdept_err]}} {{\n\
         puts \"{MARK} error $cdept_err\"\n\
         quit -noprompt\n\
         }}\n\
         puts \"{MARK} done\"\n\
         quit -noprompt\n"
    )
}

/// Extraction to SPICE: `extract all`, `ext2spice` to `<out>`.
pub fn extract(src: &Source, dir: &Path, out: &Path, options: &ExtractOptions) -> String {
    let mut b = String::new();
    b += &format!("extract path {}\n", path(dir));
    if let Some(style) = &options.style {
        b += &format!("extract style {}\n", word(style));
    }
    b += "select top cell\n";
    match options.parasitics {
        Parasitics::None => {
            b += "extract no capacitance\nextract no coupling\nextract no resistance\n";
            b += "extract all\next2spice lvs\n";
        }
        Parasitics::Capacitance => {
            b += "extract do capacitance\nextract do coupling\nextract no resistance\n";
            b += "extract all\next2spice cthresh 0\next2spice rthresh infinite\n";
        }
    }
    b += &format!("ext2spice -p {} -o {}\n", path(dir), path(out));
    b += &format!("puts \"{MARK} spice {}\"\n", out.display());
    guard(&(load(src) + &b))
}

/// DRC of the top cell, one line per box: `CDEPT: drc\t<why>\t<llx lly urx ury>` in
/// microns, then the total.
pub fn drc(src: &Source, style: Option<&str>) -> String {
    let mut b = String::new();
    b += "units microns\ndrc on\n";
    if let Some(style) = style {
        b += &format!("drc style {}\n", word(style));
    }
    b += "select top cell\ndrc catchup\n";
    b += &format!(
        "foreach {{why boxes}} [drc listall why] {{\n\
         foreach box $boxes {{ puts \"{MARK} drc\\t$why\\t$box\" }}\n\
         }}\n"
    );
    b += &format!("puts \"{MARK} style [drc list style]\"\n");
    guard(&(load(src) + &b))
}

/// LEF of the top cell to `out`.
pub fn lef(src: &Source, out: &Path, hide: bool) -> String {
    let mut b = String::new();
    b += "select top cell\n";
    b += &format!(
        "lef write {}{}\n",
        path(out),
        if hide { " -hide" } else { "" }
    );
    b += &format!("puts \"{MARK} lef {}\"\n", out.display());
    guard(&(load(src) + &b))
}

/// Read a GDS file and write every cell as `.mag` into `dir` (Magic's working folder).
pub fn import_gds(file: &Path, dir: &Path) -> String {
    let b = format!(
        "cd {}\n\
         gds read {}\n\
         writeall force\n\
         foreach c [cellname list allcells] {{ puts \"{MARK} cell $c\" }}\n",
        path(dir),
        path(file)
    );
    guard(&b)
}
