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

## Use by simulation

`sim::run_testbench` makes one job per run point and runs them in parallel
(`RunOptions::jobs`, default the smaller of points and cores; `run_testbench_with` also takes
an event listener). Results come back in point order. Each point directory gets a
`job.log` (the simulator's stdout/stderr) next to ngspice's own `out.log`.
`cdept sim run --jobs N` (`-j`) sets it and prints a line per point on stderr
as it starts and ends. With the in-process backend the points share one libngspice and
run one at a time; their progress is the simulator's own percentage.

## Not done yet

SLURM/ssh back ends, a persistent results DB (Phase 4), restart policy and one-instance-per-tool
reuse for the long-lived helpers (xschem, KLayout), process-group kill, CPU/memory limits.
