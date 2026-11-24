# Run on a cluster or over ssh

A testbench can send its run points to another machine: each point's simulator is started over
`ssh`, or submitted as a SLURM batch job, while the netlist, the deck, the results table and
the GUI stay on your machine.

```{admonition} Smoke-tested only
:class: warning
This was tested against fake `ssh`, `rsync`, `sbatch`, `squeue`, `sacct` and `scancel`
programs (shell scripts), and with the real ngspice behind them, on one machine. It has not
been run against a real cluster, a real SLURM or a second machine. Expect to adjust the
`[jobs.remote.*]` table to your site, and read [what it does not do](#what-it-does-not-do).
```

## 1. Name the remote

In `cdept.toml` (the table is described in [cdept.toml](../reference/files/cdept-toml.md#jobs)):

```toml
[jobs.remote.cluster]
kind = "slurm"
host = "login1"            # where sbatch is; leave out when you are on the login node
partition = "short"
time = "00:30:00"
cpus = 2
mem = "4G"
modules = ["ngspice/45"]   # `module load` lines on the compute node
max_jobs = 16              # points in flight at once (default 8)

[jobs.remote.workstation]
kind = "ssh"
host = "me@big-box"
command_prefix = "source ~/.cdept-env"
```

The simulator on the remote is the remote's own: `ngspice`, `Xyce` or `vacask` found on its
`PATH` (after the prefix and the modules), or `program = "/opt/ngspice/bin/ngspice"` /
`programs = { xyce = "..." }`. The path CDePT found on your machine is never used.

Check that the remotes answer:

```
cdept jobs remotes            # list them
cdept jobs remotes --check    # ssh host true, or sbatch --test-only
```

## 2. Use it

```
cdept sim run cells tb_inv --remote cluster
cdept sim suite cells tb_inv --remote cluster
```

or put `remote = "cluster"` in the `setup.toml` of the testbench, or `default = "cluster"` under
`[jobs]` for the whole project. The Simulation tab shows a `Remote: cluster (...)` line for a
testbench that goes to a remote; it has no control of its own for it yet (the form keeps the
`remote` key when you save).

What happens for a run point:

1. The deck and `.spiceinit` are written into the point folder here, as for a local run.
2. ssh: `ssh -o BatchMode=yes host '<environment, cd, exec ngspice -b -o out.log deck.sp>'`.
   SLURM: a `job.sbatch` is written in the point folder (the `#SBATCH` lines from the remote's
   table, the prefix, the modules, the environment, `cd`, the simulator, `echo $? > exit_status`),
   submitted with `sbatch --parsable`, and `squeue` / `sacct` are polled (every `poll`, 5 s) until
   the job ends.
3. `out.raw`, `out.log` and the measures are read from the point folder, as for a local run. The
   `job.log` of the point says what happened on the way (`slurm job 123: PENDING (Priority)`,
   `RUNNING`, `COMPLETED`).

Cancelling the run (the Cancel button, Ctrl-C of the command) kills the local ssh and
tries to kill the remote process, or runs `scancel`. A time limit does the same.

## Shared folder or rsync

The default, `workdir_mode = "shared"`, expects the project folder at the same path on the
remote (an NFS home, a cluster filesystem). This is the mode the feature is meant for. After a
run CDePT waits up to `file_wait` (10 s) for the files to show up here, for a lagging
filesystem.

`workdir_mode = "rsync"` with `remote_dir = "/scratch/me/cdept"` copies the point folder to
`remote_dir/<the last four folder names>` before the run and back after it (also after a
failure, for the logs). It needs `host`. It was tried only against a fake `rsync`.

## States and errors

For SLURM the job states are mapped like this: PENDING and RUNNING are progress lines in the
job log; COMPLETED gives the simulator's own exit status (so a simulator that fails is the
usual "the simulator failed" error with the end of `out.log`); FAILED, CANCELLED, TIMEOUT,
NODE_FAIL, OUT_OF_MEMORY and the others fail the point, with the state, the exit code, SLURM's
last reason (`TimeLimit`, `NodeFail`, ...) and the end of the job log. A `squeue` that fails now and then is
asked again; if the job left the queue, `sacct` and then the `exit_status` file say how it
ended; six polls in a row that learn nothing end the wait with an error that says the job may
still be running (`scancel` it yourself).

ssh exit status 255 is reported as a connection or login problem (`BatchMode` means no password
prompt: set up a key or the agent first).

## What it does not do

- No file staging beyond `rsync` mode: libraries, PDK and models must be at the same paths
  on the remote (`PDK_ROOT` and the project's tool environment are exported to the job).
- Netlisting (xschem), the VACASK model conversion and OpenVAF run on your machine, before the
  points go out.
- `simulator_backend = "inproc"` cannot go to a remote and is refused with a message.
- One SLURM job per run point: no job arrays, no dependencies, no accounting, no resubmission
  of a failed node.
- A run point that waits in the SLURM queue shows as running in the Simulation tab.
