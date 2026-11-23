# Results

Every simulation leaves files under `<project>/simulations/` and an index in a SQLite
database. The folder is generated; it can be deleted and is normally not kept in version
control. The project-wide folder layout is in [Project folders](project-folders.md).

## Folder layout

```text
simulations/
  results.db                          index of all runs (SQLite)
  <library>/<cell>/<view>/
    netlist.spice                     the schematic, netlisted once per run command
    suite.json                        only for a suite view
    <point>/                          one folder per run point
      deck.sp                         the deck that was simulated
      .spiceinit                      ngspice start-up settings (model search path, OSDI)
      out.raw                         raw waveforms (binary; several plots for several analyses)
      out.log                         the simulator's own output
      job.log                         stdout and stderr of the job
      result.json                     the record of the run
```

The point name is `<corner>_<temperature>` followed by the sweep values and the Monte
Carlo run: `tt_27`, `ss_m40` (temperature -40), `tt_85_vdd1p2`, `mc_27_r003`.

- The temperature is written as an integer, or with `.` replaced by `p`; `-` becomes `m`.
- Sweep values follow in the order of the variable names (sorted), each as
  `<variable><value>` with `.` as `p` and `-` as `m` (`tt_27_cload2em15_vdd1p2`; a value of
  `1.0` is written `1`).
- A Monte Carlo point has the corner set the mode drew from (`mc`, `tt_mm` for mismatch,
  `mc_mm` for both) and ends with `_r<index>` (three digits).

Re-running a point rewrites its folder. VACASK writes one raw file per analysis
(`<analysis>.raw`) and copies the first non-`op` one to `out.raw`
([VACASK.md](../../VACASK.md)).

## result.json

| key | type | meaning |
|---|---|---|
| `schema` | integer | Record version, currently 1. |
| `library`, `cell`, `view` | string | The testbench. |
| `setup_hash` | string | FNV-1a hash (16 hex digits) of the setup as JSON; equal hashes mean the same setup. |
| `deck_hash` | string | Hash of the deck text. |
| `simulator` | string | `ngspice`, `xyce` or `vacask`. |
| `point` | object | `name`, `corner`, `temperature` (number or null), `variables` (object of strings), `seed` and `run` (numbers, null outside Monte Carlo). |
| `status` | `"ok"` or `"error"` | `ok` means the simulator ran and left a raw file. |
| `error` | string or null | The message when `status` is `error`. |
| `measures` | object name to number | `.meas` results and calculator outputs. |
| `calc` | array of strings | Names in `measures` that come from `[[output]]`; absent when empty. |
| `calc_errors` | object name to string | Outputs that failed, with the message; absent when empty. |
| `specs` | array | `name`, `value` (number or null), `min`, `max` (number or null), `verdict`: `pass`, `fail` or `missing`. |
| `started` | string | RFC 3339 local time. |
| `duration_ms` | integer | Run time. |
| `dir`, `deck`, `raw`, `log` | path or null | Absolute paths of the folder and files. |

The overall result of a run is `error` when `status` is `error`, `fail` when any spec is
not `pass`, else `pass`.

```json
{
  "schema": 1, "library": "cells", "cell": "tb_inv", "view": "testbench",
  "simulator": "ngspice",
  "point": {"name": "tt_27", "corner": "tt", "temperature": 27.0,
            "variables": {}, "seed": null, "run": null},
  "status": "ok", "error": null,
  "measures": {"tpd": 4.51382e-11},
  "specs": [{"name": "tpd", "value": 4.51382e-11, "min": null, "max": 3e-10, "verdict": "pass"}]
}
```

(Hashes, times and paths are left out of the example.)

## results.db

`simulations/results.db` is SQLite. `result.json` is the source of truth; the database
indexes it. A re-run adds rows, so the database keeps history. `cdept sim results <lib>
<cell> --reindex` rebuilds it from the `result.json` files (history is then only what the
files hold).

Table `runs`, one row per run:

| column | type | meaning |
|---|---|---|
| `id` | integer, primary key | |
| `library`, `cell`, `view` | text | The testbench. |
| `name` | text | The point name. |
| `setup_hash`, `deck_hash`, `simulator` | text | As in `result.json`. |
| `corner` | text | |
| `temperature` | real | |
| `variables` | text | JSON object of the point's design variables. |
| `seed`, `run_index` | integer | Monte Carlo seed and run number. |
| `status` | text | `ok` or `error`. |
| `verdict` | text | `pass`, `fail` or `error`. |
| `error` | text | |
| `started` | text | |
| `duration_ms` | integer | |
| `dir` | text | |
| `record` | text | The whole record as JSON. |

Table `measures`, one row per measurement of a run (primary key `run_id`, `name`):
`run_id` (references `runs.id`, cascade delete), `name`, `value` (real; null for a missing
spec), `spec_min`, `spec_max` and `verdict` (the spec's, when the measurement has one).

Table `suite_runs`, created by the first suite run: `id`, `library`, `cell`, `view`,
`started`, `duration_ms`, `tests`, `runs`, `pass`, `fail`, `error`, `spec_fail`,
`spec_missing` and `record` (the whole suite report as JSON).

```sql
SELECT name, value FROM measures
  JOIN runs ON runs.id = measures.run_id
 WHERE view = 'montecarlo' AND measures.name = 'tpd';
```

## Statistics

`cdept sim results <lib> <cell> --view V` prints the latest run of every point and, over
them, per measurement the count, min, max, mean and sample standard deviation (n - 1).
`--stats` adds:

- percentiles p5, p50, p95 (linear interpolation), one text histogram per measurement
  (`--bins`, default 10);
- per spec: runs that met it of all runs, the yield a normal distribution with the sample's
  mean and sigma would give, and whether mean - k sigma and mean + k sigma are inside the
  spec (`--sigma`, default 3);
- the share of runs that pass every spec.

A failed or missing run does not count as meeting a spec. `cdept sim run` prints the same
report after a Monte Carlo run.

## See also

- [Simulation concepts](../../concepts/simulation.md), [SIMULATION.md](../../SIMULATION.md)
- [setup.toml](setup-toml.md), [suite.toml](suite-toml.md)
- [Project folders](project-folders.md)
