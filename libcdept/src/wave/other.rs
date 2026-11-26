//! Files UberWave cannot open itself: CSV tables and digital waveforms (VCD, FST, GHW).
//!
//! * A **table** is read by CDePT ([`crate::raw`]) and its columns go to the built-in
//!   UberWave as traces (plain calls, `f64` arrays, nothing written anywhere).
//! * A **digital** file opens in a digital viewer, Surfer or GTKWave (separate programs,
//!   started detached). With signals named (`--show clk,cnt`) they go to the built-in
//!   UberWave instead, as step traces next to analog ones.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use eyre::{Result, bail, eyre};

use super::backend::{self, Backend, ViewerOps};
use super::ipc::{PaneSel, Trace};
use crate::Project;
use crate::raw::RawFile;
use crate::tools::find_in_path;

/// What kind of waveform file a path is, by CDePT's reader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A SPICE raw file or a PSF folder: UberWave opens it itself.
    Spice,
    /// A CSV/TSV table.
    Table,
    /// VCD, FST or GHW.
    Digital,
}

/// The kind of `path` (a folder or an unreadable file counts as SPICE: UberWave decides).
pub fn kind(path: &Path) -> Kind {
    if !path.is_file() {
        return Kind::Spice;
    }
    match RawFile::open(path) {
        Ok(raw) if raw.plots().first().is_some_and(|p| p.is_digital()) => Kind::Digital,
        Ok(raw) if raw.plots().first().is_some_and(|p| p.title == "CSV table") => Kind::Table,
        _ => Kind::Spice,
    }
}

/// The digital viewers CDePT starts, in order of preference: `$CDEPT_DIGITAL_VIEWER`,
/// then Surfer, then GTKWave.
pub fn digital_viewer() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("CDEPT_DIGITAL_VIEWER").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(p));
    }
    find_in_path("surfer").or_else(|| find_in_path("gtkwave"))
}

/// Open a digital file in Surfer or GTKWave (detached). Returns the command line.
pub fn open_digital(project: Option<&Project>, path: &Path) -> Result<Vec<OsString>> {
    let program = digital_viewer().ok_or_else(|| {
        eyre!(
            "no digital waveform viewer: install Surfer or GTKWave (or set CDEPT_DIGITAL_VIEWER), \
             or send signals to UberWave with --show"
        )
    })?;
    let line = vec![
        program.into_os_string(),
        std::path::absolute(path)?.into_os_string(),
    ];
    super::spawn_detached(&line, path.parent().filter(|d| d.is_dir()), project)?;
    Ok(line)
}

/// The signals of a table or digital file that are sent when none are named: every
/// column of a table (at most 16), the first 8 signals of a digital file.
fn default_names(raw: &RawFile) -> Vec<String> {
    let p = &raw.plots()[0];
    let limit = if p.is_digital() { 8 } else { 16 };
    p.vars
        .iter()
        .skip(1)
        .take(limit)
        .map(|v| v.name.clone())
        .collect()
}

/// What [`send_file`] did.
#[derive(Debug, Default)]
pub struct SentFile {
    /// Traces the viewer took: name and number of points.
    pub pushed: Vec<(String, usize)>,
    /// Signals that were asked for and not found, and other notes.
    pub notes: Vec<String>,
}

/// Send the signals of a table or digital file to UberWave as traces (`names` empty: the
/// default ones), in one new pane. `--show` names are matched as the calculator does
/// (`out` finds `v(out)`, `clk` finds `tb.clk`).
pub fn send_file(viewer: &mut dyn ViewerOps, path: &Path, names: &[String]) -> Result<SentFile> {
    let raw = RawFile::open(path)?;
    let p = &raw.plots()[0];
    let names = if names.is_empty() {
        default_names(&raw)
    } else {
        names.to_vec()
    };
    let mut done = SentFile::default();
    let mut pane = PaneSel::New;
    let x_unit = match p.vars[0].unit.as_str() {
        "time" => Some("s".to_string()),
        u if u.contains("freq") => Some("Hz".to_string()),
        _ => None,
    };
    for n in names {
        let col = p.var_index(&n).or_else(|| p.var_index(&format!("v({n})")));
        let Some(col) = col.filter(|c| *c > 0) else {
            done.notes
                .push(format!("{} has no signal '{n}'", path.display()));
            continue;
        };
        let w = match raw.vector(0, &p.vars[col].name) {
            Ok(w) => w,
            Err(e) => {
                done.notes.push(format!("{}: {e:#}", p.vars[col].name));
                continue;
            }
        };
        let mut t = Trace::new(p.vars[col].name.clone(), w.x.to_vec(), w.y.re());
        t.x_unit = x_unit.clone();
        t.pane = pane;
        t.x_log = x_unit.as_deref() == Some("Hz");
        let _ = viewer.remove_trace(&t.name);
        match viewer.add_trace(&t) {
            Ok(()) => {
                done.pushed.push((t.name.clone(), t.x.len()));
                // the next ones in the same pane: the last one's
                pane = PaneSel::Default;
            }
            Err(e) => done.notes.push(format!("{}: {e:#}", t.name)),
        }
    }
    if done.pushed.is_empty() {
        bail!(
            "nothing was sent{}",
            if done.notes.is_empty() {
                String::new()
            } else {
                format!(": {}", done.notes.join("; "))
            }
        );
    }
    Ok(done)
}

/// Open a table or the named signals of a digital file in the built-in UberWave.
pub fn open_in_uberwave(
    project: Option<&Project>,
    path: &Path,
    names: &[String],
) -> Result<SentFile> {
    if backend::choose(project)? != Backend::Embedded {
        bail!(
            "tables and digital signals go to UberWave as traces, which needs the built-in UberWave \
             (this cdept uses the UberWaveViewer process); open a digital file without --show to use Surfer or GTKWave"
        );
    }
    let (viewer, _) = super::embedded::acquire()?;
    let mut conn = backend::Connection::Embedded(viewer);
    send_file(&mut conn, path, names)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds() {
        let dir = tempfile::tempdir().unwrap();
        let csv = dir.path().join("a.csv");
        std::fs::write(&csv, "time,v\n0,1\n1,2\n").unwrap();
        assert_eq!(kind(&csv), Kind::Table);
        let vcd = dir.path().join("a.vcd");
        std::fs::write(
            &vcd,
            "$timescale 1ns $end\n$var wire 1 ! a $end\n$enddefinitions $end\n#0\n0!\n#1\n1!\n",
        )
        .unwrap();
        assert_eq!(kind(&vcd), Kind::Digital);
        assert_eq!(kind(dir.path()), Kind::Spice);
        let raw = RawFile::open(&csv).unwrap();
        assert_eq!(default_names(&raw), vec!["v".to_string()]);
    }
}
