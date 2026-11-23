//! Starting the simulator on a deck and reading what it printed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use eyre::{Result, WrapErr, bail};

/// Environment variable that names the ngspice program (else `PATH` is searched).
pub const NGSPICE_ENV: &str = "CDEPT_NGSPICE";

/// A simulator program, run in batch mode.
#[derive(Debug, Clone, PartialEq)]
pub struct Simulator {
    pub program: PathBuf,
}

impl Simulator {
    pub fn find() -> Option<Simulator> {
        let program = match std::env::var_os(NGSPICE_ENV) {
            Some(p) if !p.is_empty() => PathBuf::from(p),
            _ => crate::tools::find_in_path("ngspice")?,
        };
        Some(Simulator { program })
    }
}

/// What a run left behind.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub deck: PathBuf,
    pub raw: PathBuf,
    pub log: PathBuf,
    /// `.meas` results, by name.
    pub measures: BTreeMap<String, f64>,
}

/// Write `deck` as `deck.sp` into `dir` and run the simulator on it:
/// `<program> -b -o out.log deck.sp` (the deck's control block writes `out.raw`), with `env` on top of the environment.
/// An error when the program fails or leaves no raw file; the end of the log is in it.
pub fn run(
    simulator: &Simulator,
    deck: &str,
    dir: &Path,
    env: &[(std::ffi::OsString, std::ffi::OsString)],
) -> Result<Outcome> {
    std::fs::create_dir_all(dir).wrap_err_with(|| format!("cannot create {}", dir.display()))?;
    let deck_file = dir.join("deck.sp");
    let raw = dir.join("out.raw");
    let log = dir.join("out.log");
    std::fs::write(&deck_file, deck).wrap_err_with(|| format!("cannot write {}", deck_file.display()))?;
    // A stale result must not pass for a new one.
    for old in [&raw, &log] {
        let _ = std::fs::remove_file(old);
    }
    let output = Command::new(&simulator.program)
        .current_dir(dir)
        .envs(env.iter().map(|(k, v)| (k, v)))
        .args(["-b", "-o", "out.log", "deck.sp"])
        .output()
        .wrap_err_with(|| format!("cannot start {}", simulator.program.display()))?;
    let text = std::fs::read_to_string(&log).unwrap_or_default();
    if !output.status.success() || !raw.is_file() {
        let tail: Vec<&str> = text.lines().rev().take(12).collect();
        let tail: Vec<&str> = tail.into_iter().rev().collect();
        bail!(
            "the simulator failed ({}){}\n{}\n{}",
            output.status,
            if raw.is_file() { "" } else { ", no raw file" },
            tail.join("\n"),
            String::from_utf8_lossy(&output.stderr).trim_end()
        );
    }
    Ok(Outcome { deck: deck_file, raw, log, measures: parse_measures(&text) })
}

/// The `.meas` results of an ngspice log: lines `name = value …`. A failed
/// measurement (`failed`, `nan`) is left out.
pub fn parse_measures(log: &str) -> BTreeMap<String, f64> {
    let mut out = BTreeMap::new();
    for line in log.lines() {
        let Some((name, rest)) = line.split_once('=') else { continue };
        let name = name.trim();
        // ngspice prints measurement names in lower case; that leaves out its
        // statistics (`Stack = 0 bytes.`).
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
            continue;
        }
        let Some(first) = rest.split_whitespace().next() else { continue };
        if let Ok(value) = first.parse::<f64>()
            && value.is_finite()
        {
            out.entry(name.to_lowercase()).or_insert(value);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_measures() {
        let log = "Doing analysis\nno. of data rows : 3\ndelay               =  1.250000e-10 targ=  1.2e-09 trig=  1.0e-09\n\
                   Delay2 = failed\nvmax = -1.5\nnoise: x = y\nStack = 0 bytes.\n";
        let m = parse_measures(log);
        assert_eq!(m["delay"], 1.25e-10);
        assert_eq!(m["vmax"], -1.5);
        assert_eq!(m.len(), 2, "{m:?}");
    }

    #[cfg(unix)]
    #[test]
    fn runs_a_program_and_reports_failures() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("fake-spice");
        std::fs::write(&fake, "#!/bin/sh\necho 'gain = 2.5' > out.log\necho raw > out.raw\n").unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let sim = Simulator { program: fake };
        let out = run(&sim, ".end\n", &dir.path().join("run"), &[]).unwrap();
        assert_eq!(out.measures["gain"], 2.5);
        assert!(out.raw.is_file() && out.deck.is_file());
        let broken = dir.path().join("broken");
        std::fs::write(&broken, "#!/bin/sh\necho 'boom' > out.log\nexit 1\n").unwrap();
        std::fs::set_permissions(&broken, std::fs::Permissions::from_mode(0o755)).unwrap();
        let err = run(&Simulator { program: broken }, ".end\n", &dir.path().join("run2"), &[]).unwrap_err();
        assert!(format!("{err}").contains("boom"), "{err}");
    }
}
