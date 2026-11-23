//! VACASK: `vacask -qp deck.sim` in the run folder; it writes one ngspice-style raw
//! file per analysis (`<analysis name>.raw`) and prints its progress.
//!
//! The deck is VACASK's own netlist format (`crate::vacask`). Nothing is linked.

use std::ffi::OsString;
use std::path::Path;

use super::{Adapter, Job};

/// Environment variable that names the VACASK program.
pub const VACASK_ENV: &str = "CDEPT_VACASK";

pub const ADAPTER: Adapter = Adapter { name: "VACASK", env_var: VACASK_ENV, tool: "VACASK" };

/// The job for a deck already written in `dir`. Output goes to `vacask.stdout`.
///
/// VACASK finds its OpenVAF, Python and device libraries from its own install folder
/// (`<prefix>/bin/vacask`, `<prefix>/lib/vacask`) but its shared libraries (Boost) may
/// be in `<prefix>/lib`, outside the loader's path; that folder is appended to
/// `LD_LIBRARY_PATH` when it exists ([`crate::vacask::tool_env_for`]).
pub fn job(program: &Path, deck: &str, dir: &Path, env: &[(OsString, OsString)]) -> Job {
    let mut all = crate::vacask::tool_env_for(program);
    all.extend(env.iter().cloned());
    Job::new("VACASK", program, dir, "vacask.stdout").args(["-qp", deck]).env(&all)
}

/// Lines of the log that report a failure: VACASK exits with 0 after most of them.
pub fn problems(log: &str) -> Vec<String> {
    let mut out = vec![];
    let lines: Vec<&str> = log.lines().collect();
    for (i, l) in lines.iter().enumerate() {
        let t = l.trim();
        let bad = t.contains("aborted")
            || t.starts_with("Parser syntax error")
            || t.contains(" not found")
            || t.contains("not defined")
            || t.contains("failed")
            || t.starts_with("Failed")
            || t.starts_with("Error");
        if bad {
            // The line, and the source line and position VACASK prints after it.
            out.push(lines[i..lines.len().min(i + 4)].join("\n"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_line_is_vacasks() {
        let j = job(Path::new("/x/vacask"), "deck.sim", Path::new("/w"), &[]);
        let args: Vec<_> = j.args.iter().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(args, ["-qp", "deck.sim"]);
    }

    #[test]
    fn finds_the_failures_in_a_log() {
        let log = "Simulating: x\nRunning analysis 'ac1'.\n  Elapsed time: 1e-4\nSweep needs to specify values, mode, or step.\nAnalysis 'ac1' aborted.\n";
        let p = problems(log);
        assert_eq!(p.len(), 1, "{p:?}");
        assert!(p[0].contains("aborted"));
        assert!(problems("Simulating: x\nRunning analysis 'tran1'.\n  Elapsed time: 0.01\n").is_empty());
    }
}
