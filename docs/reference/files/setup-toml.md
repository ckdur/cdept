# setup.toml (testbench)

A view of a cell that contains a `setup.toml` is a testbench. The file is TOML with its own
schema; **unknown keys are errors**. Every key is optional. Numbers that go into a deck
(`step`, `stop`, `start`, variable and option values) are strings so that SPICE suffixes
(`5p`, `1meg`) are kept as written; `temperatures` and the spec bounds are TOML numbers.

```toml
schematic = "cells/tb_inv"
corners = ["tt", "ff", "ss"]
temperatures = [27]
save = ["v(in)", "v(out)"]
measures = ["tran tpd trig v(in) val=0.6 rise=1 targ v(out) val=0.6 fall=1"]

[variables]
vdd = "1.2"

[[analysis]]
kind = "tran"
step = "5p"
stop = "1.2n"

[[spec]]
name = "tpd"
max = 3e-10
```

## Top-level keys

| key | type | default | meaning |
|---|---|---|---|
| `schematic` | string `library/cell` | the cell the view is in | The schematic to netlist. |
| `schematic_view` | string | first xschem schematic of the cell | Which schematic view when the cell has several. |
| `schematic_kind` | `"xschem"` or `"qucs"` | the named view's own kind, else the cell's xschem schematic, else its Qucs-S one | Which tool's schematic is netlisted ([QUCS.md](../../QUCS.md)). |
| `simulator` | `"ngspice"`, `"xyce"`, `"vacask"` | `ngspice` | The engine. Any other value is an error. See [Other simulators](../../howto/other-simulators.md). |
| `simulator_backend` | `"process"` or `"inproc"` | `process` | How ngspice runs: `process` starts `ngspice -b` for every run point; `inproc` runs the points in CDePT through libngspice, one at a time, with live progress and cancel ([NGSPICE.md](../../NGSPICE.md#the-in-process-backend)). The files and numbers are the same. `inproc` is for ngspice only and is an error when libngspice cannot be loaded (no fallback). `cdept sim run --backend` overrides it. |
| `models` | string | the technology's default set; `xyce` for `simulator = "xyce"`, `vacask` for `"vacask"` | The model set of `[models]` in [tech.toml](tech-toml.md). |
| `corners` | array of strings | the technology's default corner | Corner sets of the model set; one run per corner. |
| `temperatures` | array of numbers (deg C) | the simulator's own (27) | One run per temperature. |
| `variables` | table of string to string | none | Design variables, written as `.param name=value` in the deck. A schematic uses them as `{name}`. |
| `options` | table of string to string | none | `.options name=value`. With Xyce a name is `package.name` (`timeint.reltol`); a bare name goes to `device`. With VACASK the names are VACASK's. |
| `save` | array of strings | all vectors | Vectors to keep (`.save`). |
| `measures` | array of strings | none | `.meas` statements without the `.meas`. The first word is the analysis kind that the measure belongs to. |
| `template` | string | none | Name of a [test template](test-template.md) this setup starts from. |
| `template_args` | table | none | The template's parameters. Strings, numbers and booleans are accepted and kept as text. |
| `analysis` | array of tables | none | The analyses, see below. Leave out when the schematic runs its own analyses in code blocks. |
| `spec` | array of tables | none | Bounds on measurements. |
| `output` | array of tables | none | Calculator expressions. |
| `sweep` | array of tables | none | Design-variable sweeps. |
| `montecarlo` | table | off | Monte Carlo runs. |

Variable and option names must be a single word (no whitespace).

## `[[analysis]]`

`kind` selects the analysis and the keys it takes. Unknown keys are errors.

| kind | key | type | default | meaning |
|---|---|---|---|---|
| `op` | | | | Operating point. No keys. |
| `dc` | `source` | string | required | The swept source. |
| | `start`, `stop`, `step` | string | required | Sweep range. |
| `ac` | `sweep` | `dec`, `oct`, `lin` | `dec` | Point spacing. |
| | `points` | integer | required | Points per decade/octave, or in total for `lin`. |
| | `start`, `stop` | string | required | Frequency range. |
| `tran` | `step` | string | required, non-empty | Maximum time step. |
| | `stop` | string | required, non-empty | End time. |
| | `start` | string | empty | Time from which results are saved. |
| `noise` | `output` | string | required | Output node, e.g. `v(out)`. |
| | `source` | string | required | The input source referred to. |
| | `sweep`, `points`, `start`, `stop` | as `ac` | | Frequency range. |

With `simulator = "xyce"` at most one analysis other than `op` is allowed per setup; split
the testbench otherwise. Several analyses with ngspice share one `out.raw`.

## `[[spec]]`

| key | type | meaning |
|---|---|---|
| `name` | string | A measurement: a `.meas` name or an `[[output]]` name (compared in lower case). |
| `min` | number | Lowest allowed value. |
| `max` | number | Highest allowed value. |

At least one of `min` and `max` is required and `min` must not exceed `max`. A measurement
that is not produced gives the verdict `missing`, which fails the run.

## `[[output]]`

| key | type | default | meaning |
|---|---|---|---|
| `name` | string | required | A single word, unique (case-insensitive). Specs refer to it. |
| `expr` | string | required | A calculator expression ([CALC.md](../../CALC.md)); it must give a number. It can use earlier outputs and `[variables]` by name. It is parsed when the setup loads. |
| `plot` | string | any | Look only in this plot of the raw file (`tran`, `ac`, ... or part of its name). |

```toml
[[output]]
name = "gain_db"
expr = 'db20(value_at(v("out") / v("in"), 1k))'
plot = "ac"
```

## `[[sweep]]`

Sweeps of design variables, crossed with each other, the corners and the temperatures.
A variable is swept at most once.

| key | type | meaning |
|---|---|---|
| `variable` | string | The design variable (a word). |
| `values` (alias `list`) | array of strings or numbers | The values, written as given. |
| `start`, `stop`, `step` | numbers | Inclusive linear range, used when `values` is empty. At most 10000 values; `step` must lead from `start` to `stop`. |

## `[montecarlo]`

When the table is present, a run is `runs` seeded points of the statistical corner instead
of the corner grid. Not available with `simulator = "vacask"`.

| key | type | default | meaning |
|---|---|---|---|
| `runs` | integer 1..100000 | 10 | Number of independent runs. |
| `seed` | integer | 1 | Base seed; run `i` gets its own derived seed, stored in `result.json`. |
| `mode` | `process`, `mismatch`, `both` | `process` | What is drawn. |
| `corner` | string | `mc` | Corner set for process variation. |
| `mismatch_corner` | string | `tt_mm` | Corner set for `mismatch`. Required with Xyce. |
| `both_corner` | string | `mc_mm`, else `corner` with the model's `mc_mm_switch` set to 1 | Corner set for `both`. Required with Xyce. |

## Examples

`examples/ihp-inv/libs/cells/tb_inv/` has `testbench` (corners and one spec), `sweep`,
`montecarlo`, `mismatch`, `both`, `xyce_mc`, `vacask`, and template-based `tran_delay`,
`dc_transfer`, `power`; `tb_inv_ac/ac_response` is an AC template testbench.

## See also

- [Simulation concepts](../../concepts/simulation.md), [SIMULATION.md](../../SIMULATION.md)
- [Test template](test-template.md), [suite.toml](suite-toml.md), [Results](results.md)
- [tech.toml](tech-toml.md) for model sets and corner names
