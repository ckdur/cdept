//! Xyce: `Xyce -r out.raw -l out.log deck.cir`.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use super::{Adapter, Job};

/// Environment variable that names the Xyce program.
pub const XYCE_ENV: &str = "CDEPT_XYCE";

pub const ADAPTER: Adapter = Adapter { name: "Xyce", env_var: XYCE_ENV, tool: "Xyce" };

/// The job for a deck that is already written in `dir`. The raw file is `out.raw`,
/// Xyce's own log `out.log`, what the program printed `xyce.stdout`.
///
/// Xyce is an MPI program. Where Open MPI cannot find a launcher or network (a
/// container, a sandbox), a plain `Xyce` hangs before it reads the deck, so the
/// single-process settings are given as defaults (a variable that is set wins).
pub fn job(program: &Path, deck: &str, dir: &Path, env: &[(OsString, OsString)]) -> Job {
    let mut job = Job::new("Xyce", program, dir, "xyce.stdout")
        .args(["-r", "out.raw", "-l", "out.log", deck])
        .env(env);
    job.env_defaults = vec![
        ("OMPI_MCA_plm".into(), "isolated".into()),
        ("OMPI_MCA_btl".into(), "self".into()),
        ("PMIX_MCA_gds".into(), "hash".into()),
    ];
    job
}

/// The `.meas` results of a run. Xyce writes them to `<deck>.mt0` (`NAME = value`) and
/// prints them in its log; the first is used, else the second. Names are lower-cased
/// (Xyce prints them in upper case) and a failed measurement is left out.
pub fn measures(dir: &Path, deck: &str, log: &str) -> BTreeMap<String, f64> {
    let mt0: PathBuf = dir.join(format!("{deck}.mt0"));
    if let Ok(text) = std::fs::read_to_string(&mt0) {
        let found = parse_measures(&text);
        if !found.is_empty() {
            return found;
        }
    }
    parse_measures(log)
}

/// Lines `NAME = value …` with a single-word name.
pub fn parse_measures(text: &str) -> BTreeMap<String, f64> {
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let Some((name, rest)) = line.split_once('=') else { continue };
        let name = name.trim();
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            continue;
        }
        let Some(first) = rest.split_whitespace().next() else { continue };
        if let Ok(v) = first.parse::<f64>()
            && v.is_finite()
        {
            out.entry(name.to_lowercase()).or_insert(v);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_measures_of_a_log() {
        let log = "***** Number of Unknowns = 3\nDLY = 7.351065e-10 with targ = 2.2e-09 and trig = 1.5e-09\n\
                   VMAX = 9.96e-01 at time = 7e-09\nBAD = FAILED\nNAN = nan\n";
        let m = parse_measures(log);
        assert_eq!(m["dly"], 7.351065e-10);
        assert_eq!(m["vmax"], 0.996);
        assert_eq!(m.len(), 2, "{m:?}");
    }

    #[test]
    fn the_command_line_is_xyces() {
        let j = job(Path::new("/x/Xyce"), "deck.sp", Path::new("/w"), &[]);
        let args: Vec<_> = j.args.iter().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(args, ["-r", "out.raw", "-l", "out.log", "deck.sp"]);
    }

    #[test]
    fn prefers_the_mt0_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("deck.sp.mt0"), "GAIN = 2.5\n").unwrap();
        assert_eq!(measures(dir.path(), "deck.sp", "GAIN = 1\n")["gain"], 2.5);
        assert_eq!(measures(dir.path(), "other.sp", "GAIN = 1\n")["gain"], 1.0);
    }
}
