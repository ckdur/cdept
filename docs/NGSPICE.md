# CDePT: ngspice in-process

Code: `libcdept/src/ngspice/` (`sys.rs` has the hand-written C structs of `sharedspice.h`).
CLI: `cdept ngspice info`, `cdept ngspice live <deck>`.

## Loading
`libngspice` is opened with dlopen (the `libloading` crate) when first needed. There is no
link-time dependency and the build works without it; `ngspice::available()` says whether
it was found. Search order: `$CDEPT_LIBNGSPICE` (only that, when set), then
`libngspice.so.0`, `libngspice.so` (and the macOS/Windows names), then `../lib` of the
`ngspice` program on `PATH`. The library is never unloaded.

## API
- `Session::open()` (waits) / `try_open()`: initialises the library once, returns the
  session. **libngspice is a process-global singleton** (one circuit, one plot list, one
  set of callbacks), so a process-wide mutex lets one `Session` exist at a time; dropping
  it halts a background run and removes the circuit and plots.
- `load_deck(&str)`, `command(&str)`, `run()` (blocking), `run_background()` (`bg_run`),
  `halt()` (`bg_halt`), `is_running()`, `wait_background(|event| ..)`.
- `events()`: a channel of `Event`: `Stdout`/`Stderr` lines, `Status`, `InitData` (plot
  and vector names), `Data` (one point, name to value, complex ones too),
  `BackgroundRunning(bool)`, `Exit` (ngspice asked to quit; we do not exit the process).
- `plot_names()`, `current_plot()`, `vectors(plot)`, `vector(plot, name)` to
  `VectorData::Real | Complex`.
- `run_deck(deck, dir, env) -> sim::run::Outcome`: same contract as `sim::run::run`
  (writes `deck.sp`, leaves `out.log`/`out.raw`, parses `.meas`), so it can back a
  `simulator_backend = "inproc"` setting. It applies the folder's `.spiceinit` as commands
  (`osdi path`, `setcs sourcepath`, ...; the library reads no `.spiceinit`), and it changes
  the process working directory and environment for the run. The testbench backend
  below is built on the same steps.

## The in-process backend

`setup.toml`'s `simulator_backend = "inproc"`, or `cdept sim run --backend inproc|process`
(the flag wins; the default is `process`, one `ngspice -b` per run point). Code:
`ngspice/backend.rs`, chosen in `sim::run_testbench_with` (`run::Backend`).

- **Threading.** libngspice is one instance per process and not re-entrant across
  threads, so one dedicated worker thread (`cdept-ngspice`) owns it. Each run point is
  still a func job of the job queue (the CLI's or the GUI's); the job sends the point to
  the worker and waits. Points therefore run one after the other (a private queue gets
  one worker). The library is initialised once; each point opens a `Session` (which
  removes the previous circuit and plots: `destroy all`, `remcirc`) and drops it after.
- **A point.** `deck.sp` is written; the folder's `.spiceinit` lines are run as commands;
  the deck without its `.control` blocks is loaded; then the control lines run one by one
  (`setseed N` + `reset` for Monte Carlo, `write out.raw all`, `meas ...`), each analysis
  (`tran`, `ac`, `dc`, `op`, `noise`, `run`, ...) as `bg_<analysis>` on ngspice's
  background thread. A control block with control flow (`if`, `while`, `foreach`,
  `repeat`, `goto`, ...) cannot be run line by line; such a deck is loaded whole (its
  control block runs while loading), without progress, and a cancel takes effect when it
  ends. `quit`/`exit` end the control lines.
- **Outputs.** What ngspice prints (the `SendChar` callback) is `out.log`, and also goes
  to the job's `job.log`; `out.raw` is what the deck's `write` made; `.meas` results are
  read from the log as for the process. Results, specs, statistics and the calculator see
  the same files.
- **Progress and cancel.** Status lines (`tran: 45.2%`, the `SendStat` callback) are the
  job's progress, spread over the deck's analyses. Cancelling the job makes the worker
  send `bg_halt`; the point ends `Cancelled` and the instance is reused by the next.
- **Same numbers.** Corners and Monte Carlo seeds give the numbers of the process
  backend for the same deck: `real_sim::inproc_backend_matches_the_process` compares the
  `tpd` of tb_inv (tt, and three Monte Carlo runs) to 1e-12 relative.
- **No library.** `inproc` with libngspice not loadable (`$CDEPT_LIBNGSPICE`, then the
  search above) is an error that says so; it never falls back to the process. `inproc`
  with Xyce or VACASK is an error too. No `ngspice` program is needed.
- **Process-wide effects.** During a point the process's working directory is the point's
  folder (ngspice resolves `write out.raw` and relative includes against it) and the
  project's tool environment is set; both are put back after. ngspice settings made with
  `set` and loaded OSDI libraries stay for the next point (the process backend starts
  clean). A crash or `ControlledExit` of the library hits CDePT itself; after an exit
  request the point fails and the process backend is the safe choice.

## Things learned
- `ngSpice_Circ` **runs the deck's `.control` block while loading** (synchronously).
  `strip_control_blocks()` removes it for a streamed run; `cdept ngspice live` strips the
  block, then runs its commands, an analysis command being sent as `bg_<command>` so it
  streams from ngspice's thread.
- Callbacks run on ngspice's thread; they only push to the channel.

## Tested
With `/opt/ext/OpenLane/lib/libngspice.so.0` (ngspice 46+ shared): RC transient, background
run with data events, AC (complex vectors), `run_deck` with `.meas`, and the IHP SG13G2
`tb_inv` deck (PSP103 through OSDI): `tpd` 4.51382e-11, the same as the batch run.
Unit tests skip when the library is missing.

## Open
GUI live plotting from `Event::Data`; points reusing loaded models (each point reloads
them today); crash isolation (a libngspice abort takes the process down, so a helper
process may still be wanted for long batches); several points at once (one instance per
process).
