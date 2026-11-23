# Simulation

CDePT runs simulations from a **testbench view**: a view of a cell that holds a
`setup.toml`. The schematic that is simulated is an ordinary schematic (the device under
test, sources, load, its own code blocks); the setup says how to run it and what counts as
a pass. This page describes the model. The file formats are defined in
[setup.toml](../reference/files/setup-toml.md), [suite.toml](../reference/files/suite-toml.md),
[test templates](../reference/files/test-template.md) and [results](../reference/files/results.md).

## The run flow

`cdept sim run <library> <cell> [--view V]` does the following.

1. **Netlist once.** The embedded xschem engine writes the schematic as plain SPICE into
   `simulations/<library>/<cell>/<view>/netlist.spice`. A Qucs-S schematic
   (`schematic_kind = "qucs"`) goes through `qucs-s -n` instead ([QUCS.md](../QUCS.md)).
2. **Expand the run points.** The setup is turned into a list of run points (below).
3. **Build a deck per point.** The deck holds the model lines of the point's corner, the
   temperature, the design variables, the netlist, the analyses and the measures.
4. **Run the simulator through the job queue.** One job per point, in parallel
   (`-j N`, default the smaller of the points and the cores); see [JOBS.md](../JOBS.md).
5. **Collect measurements.** `.meas` results come from the simulator's log; calculator
   `[[output]]` expressions are evaluated on the raw file ([CALC.md](../CALC.md)). Both are
   measurements by name.
6. **Check the specs.** Each `[[spec]]` is a min and/or max on a measurement. A
   measurement that did not come out is `missing` and counts as a failure.
7. **Store.** Every point writes `result.json`; `simulations/results.db` indexes them.

`cdept sim run` exits non-zero when a run errors or a spec fails or is missing.
`cdept sim deck` prints the first point's deck without simulating.

## Run points

A run point is one simulator run. The points are the cross product of

- the **corners** (corner sets of the technology's model set, see
  [Technology](technology.md)),
- the **temperatures**,
- the **sweeps** (design variables, the first sweep varies slowest).

With a `[montecarlo]` table the corner grid is replaced by `runs` independent seeded
points of the statistical corner, crossed with temperatures and sweeps. A point has a
name (`tt_27`, `tt_85_vdd1p2`, `mc_27_r003`) that is also its folder, and its own
`result.json`. Every point is a separate simulator run, not a `.step`, so each has its own
measurements, specs and record.

## Specs, templates and suites

- A **spec** bounds a measurement. The verdict of a run is `pass`, `fail` (a spec is
  violated or missing) or `error` (the simulator did not produce a result).
- A **test template** is a ready-made family of tests ("delay", "AC response", "noise"):
  analyses, calculator outputs, default specs and the nets the schematic must have, with
  typed parameters. A setup starts from one with `template = "..."` and fields written in
  the setup override it. Ten templates are built in; a technology and a project add their
  own. See [Test templates](../reference/files/test-template.md) and [TESTS.md](../TESTS.md).
- A **suite** is a view with a `suite.toml` that lists testbenches and runs them together
  on one job queue with one combined table and one verdict
  ([suite.toml](../reference/files/suite-toml.md), [Suites](../howto/suites.md)).

## The calculator

Besides `.meas`, a setup can name expressions of the measurement calculator
(`[[output]]`): functions such as delay, rise time, bandwidth, phase margin, THD or INL on
the waveforms of the raw file. The language and the function library are defined in
[CALC.md](../CALC.md); the same engine is available from the command line as `cdept calc`.

## Choosing the simulator

`simulator` in the setup selects the engine. ngspice is the default; Xyce runs one
analysis per deck and supports process Monte Carlo; VACASK runs OSDI models converted from
the technology's ngspice models. The limits of each are in
[Other simulators](../howto/other-simulators.md) and the tool pages
[NGSPICE.md](../NGSPICE.md), [VACASK.md](../VACASK.md) and [QUCS.md](../QUCS.md).

## Results and statistics

Runs are kept: a re-run adds history. `cdept sim results` shows the latest run of each
point, min/max/mean/std per measurement, and with `--stats` percentiles, histograms and the
yield per spec. The storage is defined in [Results](../reference/files/results.md).

## See also

- [Run a testbench](../howto/run-testbench.md), [Corners and sweeps](../howto/corners-and-sweeps.md),
  [Monte Carlo](../howto/monte-carlo.md)
- [SIMULATION.md](../SIMULATION.md), [JOBS.md](../JOBS.md), [TESTS.md](../TESTS.md), [CALC.md](../CALC.md)
