//! The Simulation tab and remote runs (SYMBI-82): a read-only line that says where the points
//! run when the testbench (or the project's `[jobs] default`) names a remote, and the
//! simulator of such a run (the remote's own program, not one found on this machine).

use libcdept::Project;
use libcdept::sim::{Setup, Simulator};

/// `Remote: cluster (slurm via login1, ...)` when the run goes to a remote, else None.
pub(super) fn line(project: &Project, setup: &Setup) -> Option<String> {
    match project.jobs.pick(None, &setup.remote) {
        Ok(Some(r)) => Some(format!("Remote: {} ({})", r.name, r.describe())),
        Ok(None) => None,
        Err(e) => Some(format!("Remote: {e}")),
    }
}

/// The simulator to run `setup` with: found here, or just named when the run is remote.
pub(super) fn simulator(project: &Project, setup: &Setup) -> Option<Simulator> {
    if matches!(project.jobs.pick(None, &setup.remote), Ok(Some(_))) {
        Simulator::for_remote(setup)
    } else {
        Simulator::for_setup(setup)
    }
}
