# Write a custom test template

Write a template when the same measurement is needed on several circuits. The format is
defined in [Test template](../reference/files/test-template.md).

## 1. See how a built-in one looks

```bash
cdept sim templates                     # list: name, analysis, description
cdept sim templates show tran_delay     # parameters, outputs, default specs, requirements
```

## 2. Write the file

Put it in `<project>/templates/swing.toml` (or `pdk/<technology>/templates/` to share it
across projects). A project template shadows a technology one, which shadows a built-in
one of the same name.

```toml
name = "swing"
description = "Output levels of a DC transfer: low and high output voltage."
requires_nets = ["${out}"]
plot = ['v("${out}")']

[[param]]
name = "out"
type = "net"
default = "out"
doc = "The output net."

[[param]]
name = "src"
type = "string"
default = "vin"
doc = "The swept source."

[[param]]
name = "vdd"
type = "number"
default = "1.2"
unit = "V"
doc = "Supply voltage."

[[param]]
name = "vout_in_low_min"
type = "number"
optional = true
unit = "V"
doc = "Spec: smallest allowed output with the input at 0."

[setup]
[setup.variables]
vdd = "${vdd}"
[[setup.analysis]]
kind = "dc"
source = "${src}"
start = "0"
stop = "${vdd}"
step = "10m"
[[setup.output]]
name = "vout_in_low"
expr = 'value_at(v("${out}"), 0)'
[[setup.output]]
name = "vout_in_high"
expr = 'value_at(v("${out}"), ${vdd})'
[[setup.spec]]
name = "vout_in_low"
min = "${vout_in_low_min}"
```

The optional parameter has no default: when it is not given, the spec has no bound and is
dropped, so the template can be used with or without that spec.

## 3. Use it

```bash
cdept sim templates                                   # now lists swing
cdept sim new cells tb_inv sw --template swing --arg vdd=1.2
```

or write the view's `setup.toml` yourself:

```toml
template = "swing"
[template_args]
vout_in_low_min = 1.1
```

```bash
cdept sim run cells tb_inv --view sw
```

```text
point  result  vout_in_high*  vout_in_low*
tt     PASS       7.3540e-8      1.2000e0
```

(`*` marks calculator outputs.) Fields written in the view's `setup.toml` override the
template's; see the merge rules in the reference. Mistakes are reported with what is wrong
and what is possible: unknown template or parameter (with a suggestion), a value of the
wrong type, a missing required parameter, an expression that does not parse, and after
netlisting any net or element the template requires that the schematic lacks.

With `sim new --dut library/cell` a starter schematic with the DUT, a supply, an input
source (the template's `stimulus`) and a load is written as well.

## 4. Sweeps and Monte Carlo in the template

A template can also say what to sweep and how many Monte Carlo runs to make, from its
parameters. `examples/ihp-inv/templates/ac_load.toml` sweeps the load of an amplifier:

```toml
[[param]]
name = "loads"
default = "50f, 100f, 200f, 500f"
doc = "The load capacitances, separated by commas or spaces."

[[param]]
name = "mc_runs"
type = "int"
optional = true
doc = "Monte Carlo runs at every load; unset: the corner grid."

[[sweep]]
variable = "cl"                 # the testbench's load is value={cl}
values = "${loads}"             # exactly one ${param}: its words become the list

[montecarlo]
when = "mc_runs"                # only when the parameter is set
runs = "${mc_runs}"             # becomes a number
```

The view `cells/ota5t/ac_cload` uses it with `loads = "50f 100f 200f 500f 1p"`:

```text
point         result      gain_db*           pm*         ugbw*
tt_27_cl50f   PASS        3.5759e1      5.6378e1      2.3395e8
tt_27_cl100f  PASS        3.5759e1      5.9378e1      1.4513e8
...
tt_27_cl1p    PASS        3.5759e1      7.7759e1      2.6424e7
```

and with `mc_runs = 3` added it makes three seeded runs at every load (`mc_27_cl100f_r000`,
...). A `[[sweep]]` of the same variable in the view's `setup.toml` replaces the template's,
and a `[montecarlo]` there replaces the template's as a whole. `cdept sim templates show`
lists the sweeps and the Monte Carlo settings of the expansion.

## See also

- [Test template](../reference/files/test-template.md), [setup.toml](../reference/files/setup-toml.md)
- [CALC.md](../CALC.md) for the expression functions, [TESTS.md](../TESTS.md)
- [Suites](suites.md)
