//! A run point on a remote (SYMBI-82, [`crate::jobs::remote`]): the deck is written into the
//! point folder as for a local run, the simulator is started over ssh or as a SLURM job, and the
//! files it wrote are read back from the folder.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use eyre::{Result, WrapErr, bail};

use super::run::{Outcome, SimKind, Simulator, XYCE_ARGS_ENV, parse_measures};
use crate::batch::{Job, Watch};
use crate::jobs::JobCtx;
use crate::jobs::remote::Remote;

/// The name a remote knows the simulator of `kind` by (`programs` of the remote).
pub fn sim_name(kind: SimKind) -> &'static str {
    match kind {
        SimKind::Ngspice => "ngspice",
        SimKind::Xyce => "xyce",
        SimKind::Vacask => "vacask",
    }
}

impl Simulator {
    /// The simulator of a setup when the program is on the remote and need not be found
    /// here: the bare name (the remote's [`Remote::program_for`] decides what is started).
    pub fn for_remote(setup: &super::setup::Setup) -> Option<Simulator> {
        let kind = SimKind::of(&setup.simulator)?;
        Some(Simulator {
            kind,
            program: PathBuf::from(sim_name(kind)),
        })
    }
}

/// [`super::run::run_in_job`] on `remote`: the job's cancel flag stops it (`scancel`, or a kill
/// over ssh), `timeout` too, and the job log gets the progress lines.
pub fn run_in_job(
    remote: &Remote,
    simulator: &Simulator,
    deck: &str,
    dir: &Path,
    env: &[(OsString, OsString)],
    ctx: &JobCtx,
    timeout: Option<std::time::Duration>,
) -> Result<Outcome> {
    let watch = Watch {
        cancel: Some(ctx.cancel_flag()),
        timeout,
    };
    run(remote, simulator.kind, deck, dir, env, &watch, &|l| {
        ctx.log(l)
    })
}

/// Run a point on `remote`; see [`run_in_job`].
pub fn run(
    remote: &Remote,
    kind: SimKind,
    deck: &str,
    dir: &Path,
    env: &[(OsString, OsString)],
    watch: &Watch,
    log: &dyn Fn(&str),
) -> Result<Outcome> {
    let program = remote.program_for(sim_name(kind));
    let program = Path::new(&program);
    std::fs::create_dir_all(dir).wrap_err_with(|| format!("cannot create {}", dir.display()))?;
    match kind {
        SimKind::Vacask => crate::vacask::run_vacask_launch(program, deck, dir, env, &|job| {
            remote.launch(job, watch, log)
        }),
        SimKind::Ngspice => {
            let deck_file = dir.join("deck.sp");
            let (raw, log_file) = (dir.join("out.raw"), dir.join("out.log"));
            std::fs::write(&deck_file, deck)
                .wrap_err_with(|| format!("cannot write {}", deck_file.display()))?;
            for old in [&raw, &log_file] {
                let _ = std::fs::remove_file(old);
            }
            let job = Job::new("ngspice", program, dir, "ngspice.stdout")
                .args(["-b", "-o", "out.log", "deck.sp"])
                .env(env);
            let out = remote.launch(&job, watch, log)?;
            if out.status.success() {
                remote.wait_for_file(&raw);
            }
            let text = std::fs::read_to_string(&log_file).unwrap_or_default();
            if !out.status.success() || !raw.is_file() {
                bail!(
                    "the simulator failed ({}){}\n{}\n{}",
                    out.status,
                    if raw.is_file() { "" } else { ", no raw file" },
                    crate::batch::tail(&text, 12),
                    crate::batch::tail(&out.text, 6)
                );
            }
            Ok(Outcome {
                deck: deck_file,
                raw,
                log: log_file,
                measures: parse_measures(&text),
            })
        }
        SimKind::Xyce => {
            let name = "deck.cir";
            let deck_file = dir.join(name);
            let (raw, log_file) = (dir.join("out.raw"), dir.join("out.log"));
            std::fs::write(&deck_file, deck)
                .wrap_err_with(|| format!("cannot write {}", deck_file.display()))?;
            for old in [&raw, &log_file, &dir.join(format!("{name}.mt0"))] {
                let _ = std::fs::remove_file(old);
            }
            let mut job = crate::batch::xyce::job(program, name, dir, env);
            if let Some((_, extra)) = env.iter().rev().find(|(k, _)| *k == XYCE_ARGS_ENV) {
                let extra: Vec<OsString> = extra
                    .to_string_lossy()
                    .lines()
                    .filter(|l| !l.is_empty())
                    .map(Into::into)
                    .collect();
                let at = job.args.len().saturating_sub(1);
                job.args.splice(at..at, extra);
            }
            let out = remote.launch(&job, watch, log)?;
            if out.status.success() {
                remote.wait_for_file(&raw);
            }
            let text = std::fs::read_to_string(&log_file).unwrap_or_default();
            if !out.status.success() || !raw.is_file() {
                bail!(
                    "the simulator failed ({}){}\n{}\n{}",
                    out.status,
                    if raw.is_file() { "" } else { ", no raw file" },
                    crate::batch::tail(&text, 12),
                    crate::batch::tail(&out.text, 6)
                );
            }
            Ok(Outcome {
                deck: deck_file,
                raw,
                log: log_file,
                measures: crate::batch::xyce::measures(dir, name, &text),
            })
        }
    }
}
