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
  the process working directory and environment for the run. Not yet wired into `sim`.

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
Wiring `run_deck` into `sim` as `simulator_backend = "inproc"`; GUI live plotting from
`Event::Data`; sweeps over corners reusing one session (each run reloads models today);
crash isolation (a libngspice abort takes the process down, so a helper process may still
be wanted for long batches).
