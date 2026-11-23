# Test template

A test template is a TOML file that describes a family of tests ("measure the delay",
"measure the AC response"). A testbench's [setup.toml](setup-toml.md) picks one with
`template = "name"` and fills its parameters in `[template_args]`. The template file has its
own schema; unknown keys are errors. A file that does not load is reported as a note in
`cdept sim templates`; the other templates still work.

## Where templates live

Later sources shadow earlier ones of the same name; `cdept sim templates` notes it.

1. Built in (ten, below).
2. The technology: `pdk/<technology>/templates/*.toml`, next to the PDK's `devices.toml`.
3. The project: `<project>/templates/*.toml`.

## Top-level keys

| key | type | meaning |
|---|---|---|
| `name` | string | Letters, digits, `_` and `-`. This is the name `template =` uses. |
| `description` | string | One line, shown by `sim templates`. |
| `plot` | array of strings | What to look at after a run (calculator expressions or vectors); `${param}` allowed. |
| `requires_nets` | array of strings | Nets the netlist must have (`0` always exists). Checked after netlisting. |
| `requires_elements` | array of strings | Sources or instances the netlist must have. |
| `stimulus` | string | Value of the input source of a starter schematic made by `cdept sim new --dut`; SPICE with `{variable}` for design variables. |
| `starter_variables` | table | Design variables that `stimulus` uses, written to the starter's `[variables]`. |
| `param` | array of tables | The parameters. |
| `derived` | array of tables | Values computed from the parameters. |
| `sweep` | array of tables | Design-variable sweeps, with `${param}`: see below. |
| `montecarlo` | table | Monte Carlo runs, with `${param}`: see below. |
| `setup` | table | The `setup.toml` fragment. |

## `[[param]]`

| key | type | default | meaning |
|---|---|---|---|
| `name` | string | required | Letters, digits and `_`; unique. |
| `type` | `net`, `string`, `number`, `int`, `bool`, `choice` | empty | `number` accepts SPICE suffixes (`5p`, `100meg`). |
| `default` | scalar | none | Absent: the parameter is required, unless `optional`. May use earlier parameters (`"${vdd}"`). |
| `optional` | bool | `false` | May be left unset. What uses it is then dropped. Used for spec bounds. |
| `choices` | array of strings | empty | Required (non-empty) for `type = "choice"`. |
| `unit` | string | empty | Shown in documentation. |
| `doc` | string | empty | Shown by `sim templates show`. |

## `[[derived]]`

A derived value needs exactly one of `expr` or `from` + `map`. Derived values are computed
in order, after the parameters, and can be used as `${name}`.

| key | type | meaning |
|---|---|---|
| `name` | string | A new name (not a parameter's). |
| `expr` | string | Calculator expression over the parameters and earlier derived values. |
| `from` | string | A parameter whose value is looked up in `map`. |
| `map` | table | Value to result. |
| `doc` | string | Documentation. |

## `[setup]`

Any field of [setup.toml](setup-toml.md) except `template` and `template_args`.
`${param}` is replaced inside every string; a spec bound that is exactly `${param}` becomes a
number, and an unset optional parameter removes the bound (a spec left with no bound is
dropped). Using an unset parameter inside a longer string is an error.

An entry of an array in `[setup]` (`[[setup.output]]`, `[[setup.spec]]`, ...) may carry
`when = "<condition>"` and is then kept only if it holds. Conditions: `name` (set and not
false), `!name`, `name=value`, `name!=value`.

```toml
name = "ac_response"
description = "AC response of out over in."
requires_nets = ["${in}", "${out}"]

[[param]]
name = "out"
type = "net"
default = "out"
doc = "The output net."

[[param]]
name = "gain_db_min"
type = "number"
optional = true
unit = "dB"
doc = "Spec: smallest DC gain."

[[derived]]
name = "sign"
from = "inverting"
map = { no = "1", yes = "-1" }

[setup]
[[setup.analysis]]
kind = "ac"
points = "${points}"
start = "${fstart}"
stop = "${fstop}"
[[setup.output]]
name = "gain_db"
expr = 'db20(value_at(${sign} * v("${out}") / v("${in}"), ${fstart}))'
plot = "ac"
[[setup.spec]]
name = "gain_db"
min = "${gain_db_min}"
```

## `[[sweep]]` and `[montecarlo]`

The tables of [setup.toml](setup-toml.md#sweep) with the same keys, filled in from the
parameters, so a template can say what to sweep and how many Monte Carlo runs to make.
They may also be written inside `[setup]` (`[[setup.sweep]]`, `[setup.montecarlo]`), with
the same rules.

- `${param}` is replaced in every string, as in `[setup]`.
- In a sweep, `start`, `stop` and `step` given as strings become numbers (SPICE suffixes
  allowed: `"1p"`, `"${vlo}"`).
- `values` (or `list`) that is exactly `"${param}"` becomes the list of the parameter's
  words, separated by commas or spaces: with `loads = "50f, 100f 200f"` it is
  `["50f", "100f", "200f"]`. A list written as an array is filled entry by entry.
- A sweep whose values come from an optional parameter that is not set is dropped, so a
  parameter can switch it off. `when` works on a sweep entry and on `[montecarlo]` as a
  whole.
- In `[montecarlo]`, `runs` and `seed` given as strings become whole numbers; an unset
  optional parameter leaves the key out (the default applies).
- The template's top-level sweeps come after those of its `[setup]`; its top-level
  `[montecarlo]` replaces the one of `[setup]`.

```toml
[[param]]
name = "loads"
default = "50f, 100f, 200f"
[[param]]
name = "mc_runs"
type = "int"
optional = true

[[sweep]]
variable = "cl"
values = "${loads}"

[montecarlo]
when = "mc_runs"
runs = "${mc_runs}"
seed = 1
```

`examples/ihp-inv/templates/ac_load.toml` is a complete one (a project template), used by
the view `cells/ota5t/ac_cload`.

## How a template and a setup merge

The template fragment is filled in, then the fields written in `setup.toml` go on top:
`analysis` merges by `kind`, `output` and `spec` by `name`, `sweep` by `variable` (an entry
with the same key replaces the template's, others are added), `variables` and `options` key
by key; everything else (`corners`, `temperatures`, `measures`, `save`, `montecarlo`, ...)
is replaced as a whole. So a setup's own `[[sweep]]` of a variable the template sweeps
replaces that sweep, and its own `[montecarlo]` replaces the template's. Output expressions
see earlier outputs and `[variables]` by name.

## Built-in templates

Defaults suit a 1.2 V circuit with nets `in` and `out` and supply source `vdd`. Optional
`*_min` / `*_max` parameters switch on a spec on the output of the same name. Use
`cdept sim templates show <name>` for parameters, expanded outputs and requirements.

| template | analysis | what it measures |
|---|---|---|
| `dc_op` | op | Output voltage, supply current, power. |
| `dc_transfer` | dc | End levels, trip point, switching threshold, peak gain, linearity error. |
| `tran_delay` | tran | Propagation delay (rise, fall, mean), edge times, slew, overshoot. |
| `ac_response` | ac | DC gain, -3 dB bandwidth, unity-gain frequency, phase and gain margin, peaking. |
| `noise` | noise | Spot and integrated noise, output and input referred. |
| `thd_fft` | tran | THD, SNDR, SFDR and ENOB of a coherently sampled sine. |
| `psrr_cmrr` | ac | Supply or common-mode transfer, PSRR/CMRR. |
| `power` | tran | Average and peak current and power, energy per cycle. |
| `inl_dnl` | dc or tran | INL and DNL in LSB of a converter code. |
| `startup_settle` | tran | Settling time, overshoot, undershoot, rise time, final value. |

## See also

- [Custom test template how-to](../../howto/custom-test-template.md), [TESTS.md](../../TESTS.md)
- [CALC.md](../../CALC.md) for the expressions
- [setup.toml](setup-toml.md), [suite.toml](suite-toml.md)
