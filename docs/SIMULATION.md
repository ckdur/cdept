# CDePT: simulation (Phase 4, first slice)

Status 2026-11-23: a testbench view, the deck CDePT builds, batch ngspice runs over
corners × temperatures, `.meas` results against specs. Code: `libcdept/src/sim/`, CLI
`cdept sim`; the Simulation tab (section 8). Also design-variable sweeps, Monte Carlo
(process, mismatch, both; ngspice and Xyce), a results store and statistics (sections
4 to 7). Not built yet: VACASK, waveform viewer (Phase 5).
`cdept sim`. Since 2026-11-23 also design-variable sweeps, Monte Carlo and a results
store (sections 5 to 7). VACASK runs as a testbench simulator too ([VACASK.md](VACASK.md)). Not built yet: waveform viewer (Phase 5).
`cdept sim`; the Simulation tab (§5). Not built yet: sweeps of design variables, Monte Carlo,
Xyce/VACASK, results database, waveform viewer (Phase 5).

## 1. The testbench view

A view of a cell with a `setup.toml` in it (`ViewKind::Testbench`). The schematic that
is netlisted is an ordinary xschem schematic (DUT, sources, load, its own code blocks);
`setup.toml` says how to run it. Own schema, unknown keys are errors:

```toml
schematic = "lib/cell"        # default: the cell the view is in
schematic_view = "schematic"  # default: the first xschem schematic
simulator = "ngspice"         # or "xyce", "vacask" (docs/VACASK.md)
models = "ngspice"            # model set of [models] in tech.toml; default: the snapshot's
corners = ["tt", "ff", "ss"]  # corner sets of that set; default: the snapshot's corner
temperatures = [-40, 27, 125] # °C, one run each; default: the simulator's 27
save = ["v(in)", "v(out)"]    # .save
measures = ["tran tpd trig v(in) val=0.6 rise=1 targ v(out) val=0.6 fall=1"]
[variables]                   # .param name=value
[options]                     # .options name=value
[[analysis]]                  # kind = op | dc | ac | tran | noise
kind = "tran"
step = "5p"
stop = "1.2n"
[[spec]]                      # a bound on a measurement
name = "tpd"
max = 3e-10
```

`examples/ihp-inv/libs/cells/tb_inv` is a working one (IHP SG13G2 inverter, three corners).

## 2. What a run does

`cdept sim run <library> <cell> [--view V] [--corner C]…`

1. xschem (the embedded engine) netlists the schematic once, plain SPICE, into
   `<project>/simulations/<lib>/<cell>/<view>/netlist.spice`.
2. For each run point (corner × temperature) a folder `<corner>_<temp>` (`tt_27`,
   `ss_m40`) gets:
   - `deck.sp`: title, the corner's model lines (`ModelsTech::lines`), `.temp`,
     `.param`, `.options`, the netlist (its `.end` dropped, code blocks kept), `.save`,
     then a `.control` block that runs each analysis, appends its plot to `out.raw`
     (`set appendwrite`) and runs the measures that belong to it (first word =
     analysis kind). A control block, because ngspice does no `.meas` in batch mode
     when `-r` is given;
   - `.spiceinit`: the model search path, the OSDI libraries and the settings of the
     PDK's own start-up file, from the snapshot (`[models.<set>]` `search_path`,
     `osdi`, `options`), so the PDK's file and `$PDK_ROOT` are not needed;
   - then `ngspice -b -o out.log deck.sp` runs there with the project's tool
     environment; `out.log` is parsed for `name = value` measurements.
3. Specs: min/max per measurement; a measurement that did not come out is `MISSING`
   and counts as a failure. `cdept sim run` exits non-zero if any run or spec fails.

`cdept sim deck` prints the first run's deck without simulating.

ngspice is `$CDEPT_NGSPICE`, else `ngspice` on `PATH`. A testbench that already runs its
own analyses in code blocks is fine: leave `[[analysis]]` out.

## 3. Test

`libcdept/tests/sim.rs` runs `tb_inv` for real (skipped without ngspice, the xschem
helper or the PDK):

    CDEPT_XSCHEM=$PWD/target/debug/cdept-xschem CDEPT_NGSPICE=/opt/ext/OpenLane/bin/ngspice \
      PDK_ROOT=/opt/ext/OpenPDKs cargo test -p libcdept --test sim

Unit tests cover the schema, the deck order, the log parser and the verdicts; the runner
is tested with stand-in scripts.

## 4. Design-variable sweeps

```toml
[variables]
vdd = "1.2"                   # default
[[sweep]]
variable = "vdd"
values = ["1.0", 1.2, "1.4"]  # `list` is the same; strings or numbers, written as given
[[sweep]]
variable = "cload"
start = 1e-15                 # or start/stop/step: inclusive, linear
stop = 5e-15
step = 1e-15
```

Sweeps cross with each other, the corners and the temperatures (the first sweep varies
slowest): 3 x 5 x corners x temperatures runs. A run point carries a `variables` map that
wins over `[variables]` in the deck's `.param` lines and is part of its folder name
(`tt_27_cload2em15_vdd1p2`: names sorted, `.` becomes `p`, `-` becomes `m`). The schematic
uses the variable as `{vdd}` (in an xschem property `value="\{vdd\}"`; unescaped braces
break the property parser). Works with ngspice and Xyce (both read `.param`). Each point
is a separate simulator run, not a `.step`, so every point has its own measurements,
specs and result record. `examples/ihp-inv/libs/cells/tb_inv/sweep` is one (supply x 2
corners x 2 temperatures = 20 runs).
## 5. Monte Carlo

```toml
[montecarlo]
runs = 50
seed = 1                 # base seed
mode = "process"         # process (default) | mismatch | both
corner = "mc"            # process corner set; default: the model set's `mc`
# mismatch_corner = "tt_mm"   # mode mismatch; default tt_mm
# both_corner = "mc_mm"       # mode both; default mc_mm, else `corner` + the mismatch switch
```

`cdept sim run <lib> <cell> --view V [--runs N] [--seed S] [--mode M] [--no-mc]`
(`--runs` also turns it on for a setup without `[montecarlo]`). The run points are
`runs` x temperatures x sweeps instead of the corner grid; each is independent, in a
folder like `mc_27_r003` (`tt_mm_27_r003` for mismatch, `mc_mm_27_r003` for both: the
corner set the mode drew from, then temperature, sweep values, run), with its own seed
`deck::mc_seed(base, i)` (SplitMix64, 1..2^31, the same on every machine) kept in the
point and the record. They run in parallel through the job queue. Views of
`examples/ihp-inv/libs/cells/tb_inv`: `montecarlo` (process), `mismatch`, `both`,
`xyce_mc`.

### What the PDKs do (read in the model files)

- **IHP SG13G2, ngspice** (`libs.tech/ngspice/models`). *Process*: the `mc` corner set
  (`cornerMOSlv.lib mos_tt_stat`, `cornerRES.lib res_stat`, ...) selects the `*_stat.lib`
  files: `.param p = 'gauss(nom, rel, num_sigmas)'`, one global draw per parameter per
  circuit. *Mismatch*: the `*_mm` corner sets (`tt_mm`, `ss_mm`, ...) select
  `sg13g2_mos*_mismatch.lib` and `*_mod_mismatch.lib`, whose device models draw
  `agauss(0, sigma/sqrt(m*l*w), (mm_ok != 1 ? 0 : 1))` on every instance; the symbols
  set `mm_ok=1`. The PDK has no corner with both, and ngspice keeps the *first*
  definition of a subcircuit (`Warning: redefinition of .subckt ..., ignored`), so a
  hand-written `mc_mm` corner set in `examples/ihp-inv/tech.toml` lists the mismatch
  files before the statistical corner (whose mismatch-free device models are then
  ignored). `cdept tech refresh` does not make it: add it again (or name your own with
  `both_corner`). Only the MOS transistors have mismatch there.
- **sky130A, ngspice** (`libs.tech/ngspice/sky130.lib.spice`, models in `libs.ref`). The
  models multiply `AGAUSS(0, 1, 1)` terms by `MC_PR_SWITCH` (process: global) and
  `MC_MM_SWITCH` (mismatch: per instance, scaled by area). The corner sets are
  `mc` (`mc_pr_switch=1`; it includes every model itself, load it alone, not after `tt`),
  and `tt_mm`, `ss_mm`, ... (`mc_mm_switch=1`). So `process` is `mc`, `mismatch` is
  `tt_mm`, and `both` is `mc` followed by `.param mc_mm_switch=1` (a later `.param`
  wins; CDePT does this itself when the corner's library mentions `mc_mm_switch` and the
  set has no `mc_mm`). Checked on a scratch inverter (nfet_01v8/pfet_01v8): all three
  modes differ run to run, repeat for a seed and change with another one; the example
  project is IHP only.
- **IHP SG13G2, Xyce** (`libs.tech/xyce/models`). The `mc` corner set has the same
  `gauss()` parameters (sigma scaled by `mc_ok`), but there are no transistor mismatch
  sections (`tt_mm` only covers the MOS capacitor), so `mismatch` and `both` are
  refused with a message naming `mismatch_corner`/`both_corner`.

### How the seed gets in

- **ngspice.** The draws happen when ngspice reads the circuit. Neither `set rndseed`
  nor `setseed` in `.spiceinit` reaches them (tried: every run differs and nothing
  repeats). What works is a `.control` block that starts with `setseed N` and `reset`
  (re-reads the circuit and draws again), so the deck of a Monte Carlo point starts its
  control block that way. Verified for process, mismatch and both on IHP: the same seed
  repeats a run, another seed changes it (real tests in `libcdept/tests/sim.rs`).
- **Xyce 7.10.** `gauss()`/`agauss()` in a `.param` are *not* drawn in a plain run (the
  value is the nominal one, every time). They are drawn under `.SAMPLING
  useExpr=true` (the PDK's own `mc_*_cs_loop` test benches do this) with `.options
  SAMPLES numsamples=N SAMPLE_TYPE=MC`, and `-randseed N` on the command line seeds the
  draws. A Monte Carlo point is therefore one ordinary Xyce run with `numsamples=1`
  (`.SAMPLING useExpr=true`, `.param mc_ok=1` unless the setup sets it, `-randseed
  <seed>`), the same shape as the ngspice points: own folder, seed, record, job. One
  `numsamples=N` run would need `.mt<k>` files read back per sample; per-point runs keep
  results, store and queue as they are.
- Xyce on the IHP models needs its plugins (PSP103, R3): every Xyce run now passes
  `-plugin <the model set's plugins from tech.toml>`. (Before, a Xyce run of an IHP
  testbench stopped at `Unrecognized parameter D for device YPSP103_VA`.) The run reads
  its extra arguments from the `CDEPT_XYCE_ARGS` entry of the environment it is given
  (one per line, built by `deck::xyce_args`).
- **IHP Xyce statistical files are incomplete.** `mos_tt_stat` defines `sg13g2_*_vfbo`
  but the device files read `sg13g2_lv_pmos_vfbo_mm` (and the other three): Xyce stops
  with `Parameter VFBO ... contains unrecognized symbols`. The `xyce_mc` view ties them
  in `[variables]` (`sg13g2_lv_pmos_vfbo_mm = "sg13g2_lv_pmos_vfbo"`, ...). Check the
  PDK's tests (`sg13g2_tests_xyce/mc_*`) when its files change.

## 5b. Statistics

`libcdept::sim::stats` works on stored records (`analyze(&records, bins, k)`):

- per measurement: n, min, p5, p50, p95 (linear interpolation), max, mean, sample std,
  a histogram (equal bins between min and max);
- per spec: runs that met it of all runs (a failed or missing run does not), the yield
  the measurement would have if it were normal with the sample's mean and sigma
  (`gaussian`, from an erf approximation), and whether mean - k sigma and mean + k sigma
  are both inside the spec (`within_k_sigma`, k = 3 by default);
- all specs: runs that passed every spec, of all runs.

`cdept sim results <lib> <cell> --view V --stats [--bins 10] [--sigma 3]` prints the
latest run of every point, the min/max/mean/std table and then this report (percentile
table, one text histogram per measurement, the yield table); `cdept sim run` prints the
same report after a Monte Carlo run.

## 6. Results store

Every run of `cdept sim run` / `run_testbench[_with]` (unless `no_store`) writes
`<run folder>/result.json` (`sim::results::RunRecord`, schema 1): library, cell, view,
`setup_hash` (hash of the setup as JSON), `deck_hash`, simulator, point (name, corner,
temperature, variables, seed, run), status `ok`/`error`, error text, measures, specs
(value, min, max, verdict `pass`/`fail`/`missing`), start time, duration, paths of deck,
raw file and log. `PointResult` carries the same as `run` (plus `record`, the path
written, `started`, `duration_ms`); `RunOptions` got `mc_runs`, `mc_seed`, `no_mc`,
`no_store`.

`<project>/simulations/results.db` (SQLite through `rusqlite`, bundled) indexes them:
`runs` (a row per run with the whole record as JSON) and `measures` (run, name, value,
the spec's min/max and verdict). A re-run adds rows (history); `result.json` is the
source of truth and `Store::reindex` (`cdept sim results --reindex`) rebuilds the
database from the files.

API (`libcdept::sim::results`): `Store::open(project_root)`, `insert`, `runs(lib, cell,
view)` (oldest first), `latest` (the last run of each point), `latest_of_setup(.., hash)`,
`stats` (an `Aggregate`: runs, pass, fail, error and min/max/mean/sample std per
measurement over the latest runs), `series(.., measure)`, `reindex`; pure: `aggregate`,
`Stats::of`, `latest_per_point`, `format_table`, `format_aggregate`.

CLI: `cdept sim run` prints a table (point, result, a column per measurement, failed
specs) and the pass/fail count, plus the aggregates for sweeps and Monte Carlo, and exits
non-zero on any failed or errored run. `cdept sim results <lib> <cell> [--view V] [--all]
[--reindex]` prints the latest run of each point (or all) and the aggregates.

## 7. Open

- Mismatch Monte Carlo of resistors/capacitors on IHP (the hand-written `mc_mm` corner set covers the MOS transistors); `cdept tech refresh` does not generate `mc_mm` for IHP.
- Xyce mismatch (the PDK's Xyce models have none for transistors); Xyce `numsamples=N` in one run (each run is a separate sample-1 run today).
- Sweeps of a model corner parameter, resuming a half-done sweep.
- GUI: histograms are text bars (no plot widget yet); the Jobs panel is still a placeholder (the jobs are listed in the Simulation tab).
- Only the first measurement value per name is kept (`.meas` inside sweeps repeats it).
- Raw file reader (Phase 5); `out.raw` holds several plots when there are several analyses.
- Post-layout: swap the DUT for the extracted netlist (Phase 7).
- Xyce/VACASK through the same deck builder; OpenVAF for Verilog-A.

## 8. The Simulation tab (GUI)

`Tab::Simulation`, `cdept-gui/src/views/simulation.rs`. Double-clicking a testbench view
in the Library Manager (or Open / Simulation in its context menu) shows it there; **Run
Simulation** in the context menu of a view or of a cell opens it and runs at once.

- **Setup.** A form over `sim::Setup`: simulator, model set, schematic, corners (the
  boxes are the corners of the model set in the project's `tech.toml`; a corner the
  technology lacks is shown with `?`), temperatures, `[variables]`, `[options]`,
  analyses (per kind), measures, `.save`, specs. The form is validated live with
  `Setup::parse` (the message is under the buttons). Save writes `setup.toml` again
  (keys sorted, empty ones left out, comment lines before the first key kept; comments
  elsewhere are lost), Revert reads it again. Run saves first.
- **Run / Cancel.** A thread calls `sim::run_testbench_with` with the tab's own
  `jobs::Queue` and a cancel flag (`RunOptions::queue`, `RunOptions::cancel`); the
  queue's events wake the window. Cancel stops the netlisting from queuing points and
  cancels the queued and running jobs.
- **Jobs.** One row per run point from `Queue::snapshot`: state, progress, elapsed, the
  last line of the job log (hover for more), the error of a failed job.
- **Sweeps and Monte Carlo.** `[[sweep]]` rows (variable, a list of values or
  start/stop/step) and a `[montecarlo]` box (runs, seed, what is drawn: process /
  mismatch / both, and the corner sets) sit under the specs. Save writes them with
  everything else of the file (keys the form does not show stay as loaded; turning the
  box off removes the table). **N** + **Run N MC** runs N seeded Monte Carlo runs
  once without editing the file (it also turns Monte Carlo on for a setup without
  `[montecarlo]`); **Grid** runs the corner x temperature x sweep grid once although the
  file has `[montecarlo]` (`RunOptions::mc_runs` / `no_mc`).
- **Results.** A **Statistics** block for runs of more than one point (percentiles p5 /
  p50 / p95, min/max/mean/std, a text histogram per measurement, a yield table per
  spec and for all specs: measured, Gaussian estimate, mean +- 3 sigma inside or
  outside); then the points (collapsed above six): a table of the specs (value, bound,
  PASS green / FAIL red / MISSING amber) and the other measures; **Open folder** and
  **Open log** hand the point's folder or `out.log` to the desktop (`xdg-open`).
- Test: `cdept-gui/scripts/simulation.script` (docs/GUI-TESTING.md).

## 8. Calculator outputs

`[[output]] name = ".." expr = ".."` (optional `plot`) are expressions of the measurement calculator evaluated on each run's `out.raw`; their numbers are measures next to the `.meas` ones, specs may name them, `result.json` flags them (`calc`, `calc_errors`). Language and functions: [CALC.md](CALC.md).
