//! Starting the simulator on a deck and reading what it printed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use eyre::{Result, WrapErr, bail};

/// Environment variable that names the ngspice program (else `PATH` is searched).
pub const NGSPICE_ENV: &str = "CDEPT_NGSPICE";

/// Environment entry a Xyce run reads extra command-line arguments from (one per line;
/// [`super::deck::xyce_args`]): the model plugins and the Monte Carlo seed.
pub const XYCE_ARGS_ENV: &str = "CDEPT_XYCE_ARGS";

/// Which simulator a program is. The deck and the command line differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SimKind {
    #[default]
    Ngspice,
    Xyce,
}

impl SimKind {
    /// `setup.toml`'s `simulator` (empty: ngspice).
    pub fn of(name: &str) -> Option<SimKind> {
        match name {
            "" | "ngspice" => Some(SimKind::Ngspice),
            "xyce" => Some(SimKind::Xyce),
            _ => None,
        }
    }
}

/// A simulator program, run in batch mode.
#[derive(Debug, Clone, PartialEq)]
pub struct Simulator {
    pub kind: SimKind,
    pub program: PathBuf,
}

impl Simulator {
    /// ngspice: `$CDEPT_NGSPICE`, else `PATH`.
    pub fn find() -> Option<Simulator> {
        Self::find_kind(SimKind::Ngspice)
    }

    /// The program of a simulator: `$CDEPT_NGSPICE` / `$CDEPT_XYCE`, else the tools
    /// table (`PATH`).
    pub fn find_kind(kind: SimKind) -> Option<Simulator> {
        let program = match kind {
            SimKind::Ngspice => match std::env::var_os(NGSPICE_ENV) {
                Some(p) if !p.is_empty() => PathBuf::from(p),
                _ => crate::tools::find_in_path("ngspice")?,
            },
            SimKind::Xyce => crate::batch::xyce::ADAPTER.locate()?,
        };
        Some(Simulator { kind, program })
    }

    /// The simulator a testbench asks for (`simulator = "xyce"`; ngspice when empty).
    pub fn for_setup(setup: &super::setup::Setup) -> Option<Simulator> {
        Self::find_kind(SimKind::of(&setup.simulator)?)
    }

    pub fn ngspice(program: PathBuf) -> Simulator {
        Simulator { kind: SimKind::Ngspice, program }
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
    run_with(simulator, deck, dir, env, |cmd| {
        let output = cmd.output().wrap_err_with(|| format!("cannot start {}", simulator.program.display()))?;
        Ok((output.status, String::from_utf8_lossy(&output.stderr).into_owned()))
    })
}

/// [`run`] as part of a job: the simulator's stdout/stderr go to the job log and
/// cancelling the job kills it.
pub fn run_in_job(
    simulator: &Simulator,
    deck: &str,
    dir: &Path,
    env: &[(std::ffi::OsString, std::ffi::OsString)],
    ctx: &crate::jobs::JobCtx,
) -> Result<Outcome> {
    run_with(simulator, deck, dir, env, |cmd| Ok((ctx.run_command(cmd)?, String::new())))
}

fn run_with(
    simulator: &Simulator,
    deck: &str,
    dir: &Path,
    env: &[(std::ffi::OsString, std::ffi::OsString)],
    exec: impl FnOnce(&mut Command) -> Result<(std::process::ExitStatus, String)>,
) -> Result<Outcome> {
    // Xyce goes through the batch layer; it is not killed when a job is cancelled yet.
    if simulator.kind == SimKind::Xyce {
        return run_xyce(simulator, deck, dir, env);
    }
    std::fs::create_dir_all(dir).wrap_err_with(|| format!("cannot create {}", dir.display()))?;
    let deck_file = dir.join("deck.sp");
    let raw = dir.join("out.raw");
    let log = dir.join("out.log");
    std::fs::write(&deck_file, deck).wrap_err_with(|| format!("cannot write {}", deck_file.display()))?;
    // A stale result must not pass for a new one.
    for old in [&raw, &log] {
        let _ = std::fs::remove_file(old);
    }
    let mut cmd = Command::new(&simulator.program);
    cmd.current_dir(dir).envs(env.iter().map(|(k, v)| (k, v))).args(["-b", "-o", "out.log", "deck.sp"]);
    let (status, stderr) = exec(&mut cmd)?;
    let text = std::fs::read_to_string(&log).unwrap_or_default();
    if !status.success() || !raw.is_file() {
        let tail: Vec<&str> = text.lines().rev().take(12).collect();
        let tail: Vec<&str> = tail.into_iter().rev().collect();
        bail!(
            "the simulator failed ({}){}\n{}\n{}",
            status,
            if raw.is_file() { "" } else { ", no raw file" },
            tail.join("\n"),
            stderr.trim_end()
        );
    }
    Ok(Outcome { deck: deck_file, raw, log, measures: parse_measures(&text) })
}

/// Xyce: `Xyce -r out.raw -l out.log deck.cir` ([`crate::batch::xyce`]).
fn run_xyce(
    simulator: &Simulator,
    deck: &str,
    dir: &Path,
    env: &[(std::ffi::OsString, std::ffi::OsString)],
) -> Result<Outcome> {
    std::fs::create_dir_all(dir).wrap_err_with(|| format!("cannot create {}", dir.display()))?;
    let name = "deck.cir";
    let deck_file = dir.join(name);
    let raw = dir.join("out.raw");
    let log = dir.join("out.log");
    std::fs::write(&deck_file, deck).wrap_err_with(|| format!("cannot write {}", deck_file.display()))?;
    for old in [&raw, &log, &dir.join(format!("{name}.mt0"))] {
        let _ = std::fs::remove_file(old);
    }
    let mut job = crate::batch::xyce::job(&simulator.program, name, dir, env);
    if let Some((_, extra)) = env.iter().rev().find(|(k, _)| *k == XYCE_ARGS_ENV) {
        // Before the deck: Xyce wants its options first.
        let extra: Vec<std::ffi::OsString> =
            extra.to_string_lossy().lines().filter(|l| !l.is_empty()).map(Into::into).collect();
        let at = job.args.len().saturating_sub(1);
        job.args.splice(at..at, extra);
    }
    let out = job.run_unchecked()?;
    let text = std::fs::read_to_string(&log).unwrap_or_default();
    if !out.status.success() || !raw.is_file() {
        bail!(
            "the simulator failed ({}){}\n{}\n{}",
            out.status,
            if raw.is_file() { "" } else { ", no raw file" },
            crate::batch::tail(&text, 12),
            crate::batch::tail(&out.text, 6)
        );
    }
    Ok(Outcome { deck: deck_file, raw, log, measures: crate::batch::xyce::measures(dir, name, &text) })
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
    fn runs_points_in_parallel_through_the_queue() {
        use crate::jobs::{JobSpec, JobState, Queue};
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("fake-spice");
        std::fs::write(&fake, "#!/bin/sh\nsleep 0.3\necho 'gain = 2.5' > out.log\necho raw > out.raw\n").unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let sim = Simulator::ngspice(fake);
        let q = Queue::new(4);
        let t = std::time::Instant::now();
        for i in 0..4 {
            let (sim, run_dir) = (sim.clone(), dir.path().join(format!("p{i}")));
            q.submit(JobSpec::func(format!("p{i}"), move |ctx| run_in_job(&sim, ".end\n", &run_dir, &[], ctx).map(|_| ())));
        }
        q.wait_all();
        assert!(q.snapshot().iter().all(|j| j.state == JobState::Done), "{:?}", q.snapshot());
        assert!(t.elapsed() < std::time::Duration::from_millis(1100), "4 x 0.3 s ran serially: {:?}", t.elapsed());
    }

    #[cfg(unix)]
    #[test]
    fn runs_a_program_and_reports_failures() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("fake-spice");
        std::fs::write(&fake, "#!/bin/sh\necho 'gain = 2.5' > out.log\necho raw > out.raw\n").unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let sim = Simulator::ngspice(fake);
        let out = run(&sim, ".end\n", &dir.path().join("run"), &[]).unwrap();
        assert_eq!(out.measures["gain"], 2.5);
        assert!(out.raw.is_file() && out.deck.is_file());
        let broken = dir.path().join("broken");
        std::fs::write(&broken, "#!/bin/sh\necho 'boom' > out.log\nexit 1\n").unwrap();
        std::fs::set_permissions(&broken, std::fs::Permissions::from_mode(0o755)).unwrap();
        let err = run(&Simulator::ngspice(broken), ".end\n", &dir.path().join("run2"), &[]).unwrap_err();
        assert!(format!("{err}").contains("boom"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn runs_xyce_like_programs() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("fake-xyce");
        std::fs::write(
            &fake,
            "#!/bin/sh\n[ \"$1 $2 $3 $4 $5\" = '-r out.raw -l out.log deck.cir' ] || exit 2\n\
             echo raw > out.raw\necho 'GAIN = 2.5' > out.log\necho 'GAIN = 2.5' > deck.cir.mt0\n",
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let sim = Simulator { kind: SimKind::Xyce, program: fake };
        let out = run(&sim, ".end\n", &dir.path().join("run"), &[]).unwrap();
        assert_eq!(out.measures["gain"], 2.5);
        assert!(out.deck.ends_with("deck.cir") && out.raw.is_file());
        let broken = dir.path().join("broken-xyce");
        std::fs::write(&broken, "#!/bin/sh\necho 'Netlist error: X has invalid MEASURE line' > out.log\nexit 1\n").unwrap();
        std::fs::set_permissions(&broken, std::fs::Permissions::from_mode(0o755)).unwrap();
        let err = run(&Simulator { kind: SimKind::Xyce, program: broken }, ".end\n", &dir.path().join("r2"), &[]).unwrap_err();
        assert!(format!("{err}").contains("invalid MEASURE"), "{err}");
    }

    #[test]
    fn the_kind_comes_from_the_setup_name() {
        assert_eq!(SimKind::of(""), Some(SimKind::Ngspice));
        assert_eq!(SimKind::of("xyce"), Some(SimKind::Xyce));
        assert_eq!(SimKind::of("spectre"), None);
    }

    /// A deck for the real Xyce, when there is one: a transient and a measurement.
    #[test]
    fn real_xyce_runs_a_deck() {
        let Some(sim) = Simulator::find_kind(SimKind::Xyce) else { return };
        let dir = tempfile::tempdir().unwrap();
        let setup = super::super::setup::Setup::parse(
            "simulator=\"xyce\"\nmeasures=[\"tran dly trig v(in) val=0.5 rise=1 targ v(out) val=0.5 rise=1\"]\n\
             [[analysis]]\nkind=\"tran\"\nstep=\"0.1n\"\nstop=\"20n\"",
        )
        .unwrap();
        let point = super::super::deck::RunPoint { corner: "tt".into(), ..Default::default() };
        let net = "* rc\nV1 in 0 PULSE(0 1 1n 1n 1n 5n 20n)\nR1 in out 1k\nC1 out 0 1p\n.end\n";
        let deck = super::super::deck::build(&setup, &point, "rc", &[], net);
        let out = run(&sim, &deck, dir.path(), &[]).unwrap();
        assert!((out.measures["dly"] - 7.35e-10).abs() < 5e-11, "{:?}", out.measures);
        assert!(std::fs::metadata(&out.raw).unwrap().len() > 100);
    }
}
