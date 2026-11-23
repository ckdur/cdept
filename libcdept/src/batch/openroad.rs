//! OpenROAD: `openroad -no_splash -exit script.tcl`, the log captured.

use std::ffi::OsString;
use std::path::Path;

use eyre::Result;

use super::{Adapter, Job, Output};

/// Environment variable that names the OpenROAD program.
pub const OPENROAD_ENV: &str = "CDEPT_OPENROAD";

pub const ADAPTER: Adapter = Adapter { name: "OpenROAD", env_var: OPENROAD_ENV, tool: "OpenROAD" };

/// The job for a Tcl script. `-exit` ends the program when the script does; a Tcl
/// error then gives a failure status. The log is `openroad.log` in `dir`.
pub fn script_job(program: &Path, script: &Path, dir: &Path, env: &[(OsString, OsString)]) -> Job {
    Job::new("OpenROAD", program, dir, "openroad.log")
        .args([OsString::from("-no_splash"), "-exit".into(), script.as_os_str().to_owned()])
        .env(env)
}

/// What a script run printed.
#[derive(Debug, Clone)]
pub struct Report {
    pub output: Output,
    /// `[ERROR …]` lines.
    pub errors: Vec<String>,
    /// Number of `[WARNING …]` lines.
    pub warnings: usize,
}

/// Run a Tcl script in `dir`. An error when OpenROAD fails; the end of the log is in it.
pub fn run_script(program: &Path, script: &Path, dir: &Path, env: &[(OsString, OsString)]) -> Result<Report> {
    let script = std::path::absolute(script)?;
    let output = script_job(program, &script, dir, env).run()?;
    let (errors, warnings) = scan(&output.text);
    Ok(Report { output, errors, warnings })
}

/// The `[ERROR …]` lines of a log and the number of `[WARNING …]` lines.
pub fn scan(log: &str) -> (Vec<String>, usize) {
    let errors = log.lines().filter(|l| l.trim_start().starts_with("[ERROR")).map(|l| l.trim().to_string()).collect();
    let warnings = log.lines().filter(|l| l.trim_start().starts_with("[WARNING")).count();
    (errors, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_the_log() {
        let (e, w) = scan("[INFO ODB-1] x\n[WARNING STA-2] y\n [ERROR GRT-3] bad\n[WARNING A-1] z\n");
        assert_eq!(e, ["[ERROR GRT-3] bad"]);
        assert_eq!(w, 2);
    }

    #[test]
    fn the_command_line_is_openroads() {
        let j = script_job(Path::new("openroad"), Path::new("/s/run.tcl"), Path::new("/w"), &[]);
        let args: Vec<_> = j.args.iter().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(args, ["-no_splash", "-exit", "/s/run.tcl"]);
    }

    #[cfg(unix)]
    #[test]
    fn runs_a_fake_openroad() {
        let dir = tempfile::tempdir().unwrap();
        let fake = super::super::testutil::fake(dir.path(), "openroad", "echo '[WARNING X-1] w'; echo \"ran $2\"");
        let r = run_script(&fake, Path::new("s.tcl"), &dir.path().join("w"), &[]).unwrap();
        assert_eq!(r.warnings, 1);
        assert!(r.output.text.contains("ran -exit"), "{}", r.output.text);
        let bad = super::super::testutil::fake(dir.path(), "bad", "echo '[ERROR X-2] no'; exit 1");
        assert!(format!("{}", run_script(&bad, Path::new("s.tcl"), dir.path(), &[]).unwrap_err()).contains("[ERROR X-2]"));
    }

    /// Against the real program, when there is one.
    #[test]
    fn real_openroad_runs_a_script() {
        let Some(program) = ADAPTER.locate() else { return };
        let dir = tempfile::tempdir().unwrap();
        let ok = dir.path().join("ok.tcl");
        std::fs::write(&ok, "puts \"hello from tcl\"\n").unwrap();
        let r = run_script(&program, &ok, &dir.path().join("a"), &[]).unwrap();
        assert!(r.output.text.contains("hello from tcl"), "{}", r.output.text);
        let bad = dir.path().join("bad.tcl");
        std::fs::write(&bad, "no_such_command\n").unwrap();
        let err = run_script(&program, &bad, &dir.path().join("b"), &[]).unwrap_err();
        assert!(format!("{err}").contains("no_such_command"), "{err}");
    }
}
