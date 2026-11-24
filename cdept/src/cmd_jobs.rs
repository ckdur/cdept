//! `cdept jobs …`: the remotes run points can be sent to (docs/howto/remote-runs.md).

use clap::Subcommand;
use eyre::{Result, bail};
use libcdept::Project;

#[derive(Subcommand)]
pub enum JobsCommand {
    /// List the remotes of [jobs.remote.*] in cdept.toml (ssh and SLURM).
    ///
    /// With --check each one is asked for a sign of life: `ssh host true`, or
    /// `sbatch --test-only` for SLURM. Exits non-zero when one does not answer.
    Remotes {
        /// Run the trivial command on every remote (or only on NAME).
        #[arg(long)]
        check: bool,
        /// Only this remote.
        name: Option<String>,
    },
}

pub fn run(command: JobsCommand, project: &Project) -> Result<()> {
    match command {
        JobsCommand::Remotes { check, name } => {
            let remotes: Vec<_> = match &name {
                Some(n) => vec![project.jobs.get(n)?],
                None => project.jobs.remotes.values().collect(),
            };
            if remotes.is_empty() {
                println!(
                    "no remotes: add [jobs.remote.NAME] to cdept.toml (docs/howto/remote-runs.md)"
                );
                return Ok(());
            }
            let mut bad = 0;
            for r in remotes {
                let default = project.jobs.default.as_deref() == Some(&r.name);
                println!(
                    "{}{}: {}",
                    r.name,
                    if default { " (default)" } else { "" },
                    r.describe()
                );
                if check {
                    match r.check() {
                        Ok(msg) => println!("  ok: {msg}"),
                        Err(e) => {
                            bad += 1;
                            println!("  FAILED: {e:#}");
                        }
                    }
                }
            }
            if bad > 0 {
                bail!("{bad} remote(s) did not answer");
            }
            Ok(())
        }
    }
}
