# CACE datasheets

CDePT reads and writes the datasheets of **CACE**, the Circuit Automatic Characterisation
Engine ([fossi-foundation/cace](https://github.com/fossi-foundation/cace), formerly
efabless/cace), instead of having a second spec format. A datasheet becomes testbench
views ([setup.toml](setup-toml.md)) and a suite ([suite.toml](suite-toml.md)) of a cell;
a suite or testbench goes back out as a datasheet with its measured results. Commands:
`cdept cace import` and `cdept cace export` ([CLI](../cli.md)); in the GUI, **Import
CACE Datasheet…** and **Export CACE Datasheet…** in the context menu of a cell, and
**Export CACE…** in the Simulation tab. Recipes: [CACE datasheets](../../howto/cace.md).
Code: `libcdept/src/cace/`.

## Version

The format followed is the YAML datasheet of **CACE 2.x, `cace_format: 5.2`**, as read
from CACE 2.9.0: `docs/source/reference_manual/datasheet_format.md` (the format),
`cace/common/cace_read.py` (what is required: `name`, `description`, `PDK`, `paths`, a
`tool` per parameter), `cace/parameter/parameter.py` and `parameter_ngspice.py` (how
conditions are merged and enumerated, how spec limits are checked and scaled by their
unit) and `cace/common/spiceunits.py` (the units). Other `cace_format` values are read as
5.2 with a warning. The CACE 4.0 text format (and its JSON form with
`electrical_parameters`) is refused: convert it with CACE 2.x first. An export writes
`cace_format: 5.2`.

## Import: CACE field to CDePT field

A parameter whose `tool` is `ngspice` becomes the testbench view `<prefix><parameter>` of
the cell; the suite view `<prefix>datasheet` lists them. Conditions are first merged the
way CACE merges them: the datasheet's `default_conditions`, with the parameter's
`description`, `display` and `unit` over them and the parameter's values instead of the
default's when it has any. Their values are then enumerated as CACE does: the
`enumerate` list, then with `step` the `typical` value and the range from `minimum` to
`maximum` (`linear` by `stepsize`, default 1; `logarithmic` by the factor `stepsize`,
default 2; the last value clipped to the maximum), else `minimum`, `typical`, `maximum`,
those present. A condition with a `unit` is scaled to SI like CACE scales it before it
substitutes the template (`ib: 10` in `uA` is `1e-5`).

| CACE | CDePT | Notes |
|---|---|---|
| `name`, `description` | suite `description` | The header as a whole is kept in the suite's `[cace]`. |
| `PDK` | (checked) | A warning when it is not the project's technology. |
| `cace_format`, `authorship`, `paths`, `pins` | suite `[cace] datasheet` | Kept as CACE YAML, written back by an export. |
| `default_conditions` | merged into every testbench | CACE uses only the conditions its template names; CDePT does not read the template, so every default condition is applied (as a design variable when it is not a corner or temperature). |
| parameter key | testbench view name | With the prefix; characters other than letters, digits, `_` and `-` become `_`. |
| `display` | suite `[[test]] name` and `[cace] display` | |
| `status: skip` | not a `[[test]]` of the suite | The testbench is made all the same. |
| `tool: ngspice` | a testbench | `template` is recorded in `[cace] template`; CACE's other `ngspice` arguments (`format`, `suffix`, `variables`, `script`, `jobs`, `spiceinit_path`) stay in `[cace] yaml`. |
| any other `tool` (`magic_drc`, `magic_area`, `netgen_lvs`, `klayout_drc`, `klayout_lvs`, `magic_antenna_check`) | suite `[cace.other]` | Not a simulation: no testbench, kept as CACE YAML. CDePT's own DRC and LVS are separate ([LVS and DRC](../../howto/lvs-drc.md)). |
| condition `temperature` | `temperatures` | Numbers in deg C. |
| condition `corner` | `corners` | Each name must be a corner set of the technology's default model set ([tech.toml](tech-toml.md)): the same name, the same in another case (`TT`), or `typical`/`nom` (`tt`), `fast` (`ff`), `slow` (`ss`); otherwise it is kept as written, with a warning. |
| condition `corner` = `mc`, with `collate: <c>` | `[montecarlo]` `runs` = the number of values of `<c>`, `seed = 1` | CACE's Monte Carlo: the runs of one condition set collated over an iterations condition. `corner = <name>` when the technology calls its statistical corner otherwise. `mc` among other corners is left out (warning). |
| any other condition, one value | `[variables] name = "value"` | SI value as text. The schematic uses it as `{name}`. |
| any other condition, several values | `[[sweep]] variable = name`, `values = [...]` | |
| condition named like a bit vector (`b[7:0]`) | (kept) | CDePT has no vector variables. |
| `spec.<result>.minimum.value` (number, `fail` true by default) | `[[spec]] name = "<result>"`, `min` | Scaled by the result's `unit` (else the parameter's), as CACE does: `5` in `mV` is `0.005`. |
| `spec.<result>.maximum.value` | `[[spec]] max` | Likewise. |
| `value: any` | no bound | Measured only, kept. |
| `fail: false` | no bound | Kept; CACE does not check it either. |
| `typical` with `fail: true`, or a `calculation`/`limit` other than the defaults (`minimum`/`above`, `median`/`exact`, `maximum`/`below`) | no bound (warning) | CDePT checks every run against `min`/`max`, which is CACE's default `minimum`-`above` and `maximum`-`below`. |
| `spec.<result>.unit`, parameter `unit` | `[cace.units]` | Per result; an export writes them back. |
| result names | `[[spec]] name` | The testbench must produce these measurements (`measures` or `[[output]]`): CACE's template computed them, CDePT does not read the template (warning). |
| `plot`, `variables` (of a parameter) | `[cace] yaml` | Plots are CACE's; not drawn by CDePT. |
| `results` (CACE 4.0, or a CDePT export) | dropped on export | Replaced by the results of the latest runs. |

Nothing is dropped silently: the plan of `cdept cace import` lists each field under
**mapped**, **kept in [cace]** or **warnings**.

### The `[cace]` tables

Of a testbench ([setup.toml](setup-toml.md#cace)):

| key | meaning |
|---|---|
| `parameter` | The parameter's key in the datasheet. |
| `display` | Its `display`. |
| `tool` | `ngspice`. |
| `template` | CACE's template testbench (`ac.sch`), for reference. |
| `collate` | The condition the Monte Carlo runs are collated over (`iterations`). |
| `units` | Table: result name to unit. |
| `yaml` | The parameter as CACE YAML, as read. |

Of a suite ([suite.toml](suite-toml.md#cace)):

| key | meaning |
|---|---|
| `source` | The datasheet file it was read from. |
| `parameters` | The parameter keys, in the datasheet's order. |
| `views` | Table: parameter key to testbench view. |
| `other` | Table: parameter key to CACE YAML, for the parameters that are not testbenches. |
| `datasheet` | The header (everything but `parameters`) as CACE YAML. |

CDePT does not run with these values; they only carry what the CDePT fields cannot.
They survive the Simulation tab's Save (keys the form does not show stay as loaded).

## Export: CDePT field to CACE field

`cdept cace export <library>/<cell> [--view V] [-o FILE] [--no-results]` writes the
suite (or testbench) `V`, by default the cell's only suite, else its only testbench.

- **Header**: the suite's `[cace] datasheet`; for a testbench, that of a suite of the
  cell that lists it; else a new one (`name` = cell, `PDK` = the project's technology,
  `cace_format: 5.2`, `paths` with the usual folders). A suite `description` that differs
  replaces the datasheet's.
- **Parameters**, in the order of `[cace] parameters`, then the suite's other tests
  (keyed by view name). A testbench's parameter starts from `[cace] yaml`, a testbench
  without it from a new parameter (`tool: ngspice` with the schematic's file as
  `template`).
- **Conditions**: the import mapping is applied again to the parameter as read and
  compared with the setup as it is now, condition by condition. Where the setup runs the
  same values the condition is written as it was read (`minimum`/`typical`/`maximum`,
  `step`, or inherited from `default_conditions`); where it changed it is written as
  `enumerate: [...]` (`typical:` for one value), unscaled back to its unit. A Monte Carlo
  setup writes `corner: {typical: mc}` and the collate condition `{minimum: 1, maximum:
  <runs>, step: linear, stepsize: 1}` with `collate` in the tool arguments. A condition
  the setup no longer has is left out (one that came from `default_conditions` only
  cannot be: a note says so).
- **Spec**: each `[[spec]]` `min`/`max` that differs from the limit as read becomes the
  `minimum`/`maximum` `value` (unscaled to the unit; `fail`, `calculation`, `limit`
  removed); a limit CDePT no longer has becomes `any`; limits that were not bounds stay
  as they were. A `[[spec]]` without a CACE result is added.
- **Results** (unless `--no-results`): from the latest run of every point of the
  testbench in the results store ([Results](results.md)), a `results` list with one
  entry, modelled on the `results` of the CACE 4.0 format:

```yaml
    results:
    - name: schematic      # the netlist source
      tool: cdept
      runs: 9              # run points
      status: pass         # fail: a spec failed or was missing in a run; error: a run did not finish
      tpd:
        minimum: {value: 38.4711}               # over the runs, in the result's unit
        typical: {value: 45.1382}               # the median (CACE's default for typical)
        maximum: {value: 56.4111, status: pass} # pass/fail against the bound
        # missing: 2 when some runs did not measure it
```

CACE 2.x does not read `results` (it keeps unknown keys and writes its own summary), so
the datasheet can be given to CACE as it is.

## Not supported, and what is lossy

- CACE's **templates** are not converted: they are xschem schematics with `CACE{name}`
  and `CACE[expression]` placeholders and their own `.control` code that writes the
  result columns. A CDePT testbench netlists the cell's own schematic (or
  `--schematic`), with design variables as `{name}`, and needs `measures` or
  `[[output]]` for the results. `CACE{N}`, `CACE{simpath}`, ... have no counterpart.
- **Physical-verification parameters** (DRC, LVS, area, antenna) are carried, not run.
- **Plots**, the `variables` of a parameter, `script`/`script_variables` of `ngspice`:
  carried, not used.
- **Collate** without the `mc` corner: the values are separate runs (CACE merges them for
  plotting only).
- **Conditions CACE would ignore** (not named in its template) are applied by CDePT as
  design variables; harmless for the netlist, but they are part of the run point.
- **Number types**: PyYAML (which CACE uses) reads `1e6` as a string, CDePT as a number;
  a value read that way comes back as `1000000.0`. CACE converts both with `float()`.
  YAML comments and key quoting are not kept (the data are).
- **Units**: scaling follows CACE's table (`spiceunits.py`), including its quirks (`m`
  alone is metres, `k` alone kelvin, both 1). An unknown unit (`dB`, `°`) scales by 1.
- The **netlist source** of the results is always `schematic` (post-layout is Phase 7).

## See also

- [CACE datasheets how-to](../../howto/cace.md)
- [setup.toml](setup-toml.md), [suite.toml](suite-toml.md), [Results](results.md)
