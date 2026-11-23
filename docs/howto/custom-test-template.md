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

## See also

- [Test template](../reference/files/test-template.md), [setup.toml](../reference/files/setup-toml.md)
- [CALC.md](../CALC.md) for the expression functions, [TESTS.md](../TESTS.md)
- [Suites](suites.md)
