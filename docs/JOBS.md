# Jobs: the process supervisor and job queue

`libcdept::jobs` (ROADMAP Phase 3 "process supervisor", Phase 4 "job queue"). Plain
`std` threads, no async runtime. Tool adapters (ngspice now; Magic, Xyce, GUI later) put
their runs on a `Queue` instead of spawning processes themselves.

## API

- `Queue::new(workers)`: a pool of worker threads (`Queue::default_workers()` = cores).
  Dropping it cancels what is left and joins the workers.
- `queue.submit(spec) -> JobId`. A spec is
  - `JobSpec::command(name, program, args).cwd(..).env(..).log(..)`: runs a process;
    stdout and stderr go to the log file; a non-zero exit is `Failed`.
  - `JobSpec::func(name, |ctx| -> Result<()>).log(..)`: runs a closure. Results leave the
    closure through whatever it captured (an `Arc<Mutex<..>>` slot, a channel).
- `JobCtx` (given to closures): `is_cancelled()`, `log(line)`, `log_path()`,
  `progress(0..1)`, and `run_command(&mut Command)`, which logs the child's output into the
  job log and kills it when the job is cancelled. Use it for any process a closure starts.
- States: `Queued -> Running -> Done | Failed | Cancelled`. `Failed` carries the error
  text; a panicking closure is `Failed`.
- `cancel(id)` drops a queued job or kills the running one's child (it ends `Cancelled`,
  whatever the closure returned); `cancel_all()`.
- `wait_all()`, `wait(id)`, `info(id)`, `snapshot() -> Vec<JobInfo>`: the snapshot is a
  cheap copy (name, state, progress, error, log path, elapsed) for a UI to poll.
- `on_event(|JobEvent|)`: called on each state or progress change, from worker threads.
  Keep it quick; do not submit or cancel from it.

Jobs start in submission order. Only the direct child is killed on cancel (a script that
forks leaves grandchildren; use `exec` or have the tool clean up).

`JobCtx::cancel_flag()` hands the flag to code that polls it itself:
`batch::Job::run_watched(&Watch { cancel, timeout })` kills a batch program when the
flag is set or the time is up (Xyce runs of `sim` use it, so cancelling a Xyce point kills
Xyce); `Magic::in_job(ctx)` does the same for Magic, which also has a time limit
(`[magic] timeout` in `cdept.toml`, default 15 minutes; the error has Magic's last lines);
the in-process ngspice backend halts the simulation with `bg_halt`
([NGSPICE.md](NGSPICE.md#the-in-process-backend)). `jobs::parse_duration` reads the
durations (`90s`, `10m`, `1h30m`).

### Time limits

Every simulator run point has a time limit (SYMBI-80), the same for ngspice (process and
in-process), Xyce and VACASK: `--timeout` of `cdept sim run|suite`, else `timeout` in the
testbench's `setup.toml`, else `[sim] timeout` of `cdept.toml`, else 30 minutes (`0` is no
limit). The clock starts when the point starts (not while it waits in the queue). On
expiry the program is killed (`JobCtx::run_command_limited` for ngspice, `Watch.timeout`
of `Job::run_watched` for Xyce and VACASK, a watchdog and `bg_halt` in-process: one
mechanism per kind of run, the same `Watch`/`JobCtx` path as cancel) and the point ends as
a failed job with the message `run point exceeded its time limit of 10m` and the last
lines of its log (`sim::run::is_time_limit` recognises it). The compilation of the
VACASK models has its own limit, `[sim] openvaf_timeout` (default 20 minutes), and is
cancelled with the run's cancel flag; a half-written library is removed, so a stopped
compilation never counts as cached.

## Use by simulation

`sim::run_testbench` makes one job per run point and runs them in parallel
(`RunOptions::jobs`, default the smaller of points and cores; `run_testbench_with` also takes
an event listener). Results come back in point order. Each point directory gets a
`job.log` (the simulator's stdout/stderr) next to ngspice's own `out.log`.
`cdept sim run --jobs N` (`-j`) sets it and prints a line per point on stderr
as it starts and ends. With the in-process backend the points share one libngspice and
run one at a time; their progress is the simulator's own percentage.

## Remote backend (SLURM and ssh; smoke-tested only)

`libcdept::jobs::remote` (SYMBI-82) starts the simulator of a run point on another machine
instead of as a local process. How to use it: [remote-runs how-to](howto/remote-runs.md).
It was tested against fake `ssh`, `rsync`, `sbatch`, `squeue`, `sacct` and `scancel` programs
(and the real ngspice behind them), never against a cluster or a second machine.

A run point is prepared locally as always (`deck.sp`, `.spiceinit` in its folder). A `Remote`
(`[jobs.remote.NAME]` of `cdept.toml`) then `launch`es the simulator's `batch::Job` and returns the
same `batch::Output` a local run gives, so `sim::remote::run` reads `out.raw`, `out.log` and the
measures from the point folder exactly as for a local run. The queue job of the point is an
ordinary `JobSpec::func`: it is Running while the remote works, its cancel flag stops the remote
run and the `job.log` of the point collects the progress lines (`slurm job 123: PENDING (Priority)`).
A `Watch` timeout (the run point's time limit) is honoured the same way.

| | `kind = "ssh"` | `kind = "slurm"` |
|---|---|---|
| start | `ssh -o BatchMode=yes host '<script>'`: environment, `cd`, `exec <simulator>` | `job.sbatch` written in the point folder, `sbatch --parsable job.sbatch` (through `ssh host` when `host` is set) |
| watched | the local `ssh` process | `squeue -j ID -h -o %T\|%r`, then `sacct -j ID -n -P -o State,ExitCode`, then the `exit_status` file |
| cancel / time limit | the local ssh is killed, then the remote process is killed by the pid it wrote (`.cdept-remote.pid`) over a second ssh (best effort, 10 s) | `scancel ID` |
| result | the simulator's own exit status | the simulator's exit status from `exit_status` (the script ends 0 itself) |

States of SLURM and what CDePT does with them: PENDING, CONFIGURING, RUNNING, COMPLETING are
progress (logged when they change; the queue job stays Running). COMPLETED gives the simulator's
exit status, so a failing simulator is the usual "the simulator failed" error with its log.
FAILED, CANCELLED, TIMEOUT, NODE_FAIL, OUT_OF_MEMORY, BOOT_FAIL, DEADLINE, PREEMPTED, REVOKED
fail the point with the state, the exit code, the last reason `squeue` gave (`TimeLimit`,
`NodeFail`, ...) and the end of the job log. `squeue` failing, or not knowing the job, is not an
error at once: `sacct` is asked, then the `exit_status` file (shared folder); six polls in a row that
learn nothing end the wait. The poll interval is `poll` (default 5 s).

Limits: `max_jobs` per remote (default 8) replaces the local core count as the number of points
in flight (`--jobs` still wins). The simulator is the remote's own `program` (default the bare
name `ngspice`, `Xyce`, `vacask`), never the local path. Only the process backends are
remote-capable: `simulator_backend = "inproc"` with a remote is refused. VACASK model
conversion, OpenVAF and xschem netlisting stay local (they happen before the points are
prepared). After a run the files are waited for up to `file_wait` (default 10 s) for a lagging
network filesystem.

## Not done yet

A cluster or second machine to test the remote backend on (it is smoke-tested only), job arrays
and accounting for SLURM, a persistent results DB (Phase 4), restart policy and one-instance-per-tool
reuse for the long-lived helpers (xschem, KLayout), process-group kill, CPU/memory limits (a SLURM
remote has `cpus` and `mem`, local runs do not).
