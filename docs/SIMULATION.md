# CDePT: simulation (Phase 4, first slice)

Status 2026-11-23: a testbench view, the deck CDePT builds, batch ngspice runs over
corners × temperatures, `.meas` results against specs. Code: `libcdept/src/sim/`, CLI
`cdept sim`. Since 2026-11-23 also design-variable sweeps, Monte Carlo and a results
store (sections 5 to 7). Not built yet: VACASK, waveform viewer (Phase 5).

## 1. The testbench view

A view of a cell with a `setup.toml` in it (`ViewKind::Testbench`). The schematic that
is netlisted is an ordinary xschem schematic (DUT, sources, load, its own code blocks);
`setup.toml` says how to run it. Own schema, unknown keys are errors:

```toml
schematic = "lib/cell"        # default: the cell the view is in
schematic_view = "schematic"  # default: the first xschem schematic
simulator = "ngspice"         # the only one run today
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
seed = 1          # base seed
corner = "mc"     # default: the model set's `mc` corner
```

`cdept sim run <lib> <cell> --view V [--runs N] [--seed S] [--no-mc]` (`--runs` also
turns it on for a setup without `[montecarlo]`). The run points are `runs` x
temperatures x sweeps instead of the corner grid; each is independent, in a folder like
`mc_27_r003`, with its own seed `deck::mc_seed(base, i)` (SplitMix64, 1..2^31, the same
on every machine) kept in the point and the record. They run in parallel through the job
queue. `examples/ihp-inv/libs/cells/tb_inv/montecarlo` is one.

What the IHP SG13G2 models do (read in `libs.tech/ngspice/models`): the `mc` corner of
tech.toml (`cornerMOSlv.lib mos_tt_stat`, `cornerRES.lib res_stat`, ...) selects the
`*_stat.lib` files, which define the process parameters as `.param p = 'gauss(nom, rel,
num_sigmas)'` or `agauss(0, 1, 1)`: global draws, no switch. Per-device mismatch is
separate (`*_mismatch.lib` in the `_mm` corners, `tt_mm` ...); `corner = "tt_mm"` in
`[montecarlo]` would sample those, `mc` alone does not.

How the seed gets in: the draws happen when ngspice reads the circuit. Neither
`set rndseed` nor `setseed` in `.spiceinit` reaches them (tried: every run differs and
nothing repeats). What works is a `.control` block that starts with `setseed N` and
`reset` (re-reads the circuit and draws the parameters again), so the deck of a Monte
Carlo point starts its control block that way. The real test shows the same seed repeats
a run and another seed changes it. Another PDK whose statistical corner uses
`gauss`/`agauss` parameters (sky130's `mc`) works the same; one that selects a mismatch
section needs a corner set for it in tech.toml. Xyce: Monte Carlo is refused (it has
`.sampling`, not generated yet); sweeps work.

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

- GUI: testbench editor, run button, results panel. Job queue (parallel points, cancel).
- Xyce Monte Carlo (`.sampling`), mismatch (`_mm`) Monte Carlo with per-device seeds, sweeps of a
  model corner parameter, resuming a half-done sweep.
- Only the first measurement value per name is kept (`.meas` inside sweeps repeats it).
- Raw file reader (Phase 5); `out.raw` holds several plots when there are several analyses.
- Post-layout: swap the DUT for the extracted netlist (Phase 7).
- Xyce/VACASK through the same deck builder; OpenVAF for Verilog-A.
