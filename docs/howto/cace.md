# Import and export CACE datasheets

A [CACE](https://github.com/fossi-foundation/cace) datasheet (YAML, `cace_format: 5.2`)
holds the spec table of a block: parameters with limits, the conditions to measure them
under, and the tool that does it. CDePT reads it into testbench views and a suite, and
writes the suite back out as a datasheet with the measured results, so the same spec table
goes between CDePT and CACE. The mapping, field by field: [CACE
datasheets](../reference/files/cace.md).

The outputs below are from a copy of `examples/ihp-inv` (IHP SG13G2, `PDK_ROOT` set).

## A datasheet

`inv.yaml`, the delay of the example's inverter at three corners and three temperatures,
and a DRC that CACE would run:

```yaml
name: inv
description: Inverter of examples/ihp-inv
PDK: ihp-sg13g2
cace_format: 5.2
paths:
  root: ..
default_conditions:
  vdd:
    description: Supply
    unit: V
    typical: 1.2
parameters:
  delay:
    display: Propagation delay
    unit: ps
    spec:
      tpd:
        maximum:
          value: 300
    tool:
      ngspice:
        template: tb_inv.sch
    plot:
      tpd_vs_corner:
        xaxis: corner
        yaxis: tpd
    conditions:
      corner:
        enumerate: [tt, ff, ss]
      temperature:
        minimum: -40
        typical: 27
        maximum: 125
  drc:
    display: Magic DRC
    spec:
      drc_errors:
        maximum:
          value: 0
    tool:
      magic_drc:
        gds_flatten: true
```

## Import it

The views go into a cell; `tb_inv` already has the testbench schematic, so the new
testbenches netlist it (`--schematic library/cell` names another one). Without `--write`
the plan and the report are printed:

```bash
cdept cace import inv.yaml --into cells/tb_inv --view-prefix cace_
```

```text
CACE datasheet inv.yaml: inv (ihp-sg13g2), cace_format 5.2, 2 parameter(s)
  create libs/cells/tb_inv/cace_delay/setup.toml: testbench of delay: 9 run(s), 1 spec(s)
  create libs/cells/tb_inv/cace_datasheet/suite.toml: suite of the datasheet: 1 test(s)
mapped:
  default_conditions: merged into every testbench's conditions (CACE takes only those its template uses)
  delay: vdd = 1.2 -> [variables]
  delay: corner tt, ff, ss -> corners
  delay: temperature -40, 27, 125 -> temperatures
  delay: spec tpd [ps] -> [[spec]] <= 3e-10
kept in [cace] (not used by CDePT, given back by an export):
  PDK: datasheet header, in the suite's [cace] datasheet
  cace_format: datasheet header, in the suite's [cace] datasheet
  paths: datasheet header, in the suite's [cace] datasheet
  delay: plot: {tpd_vs_corner}
  delay: ngspice arguments template
  drc: tool magic_drc: not a simulation, no testbench (in the suite's [cace.other])
warnings:
  delay: the testbench must measure tpd (CACE's template tb_inv.sch did): add measures or [[output]]

Nothing written. --write makes the views; --view-prefix, --schematic and --overwrite change them.
```

The corners are checked against the technology's corner sets (a name it lacks is a
warning); the limit is scaled by its unit (300 ps is `3e-10`). Add `--write` to make the
views; `--overwrite` replaces views that exist. `cace_delay/setup.toml`:

```toml
corners = ["tt", "ff", "ss"]
temperatures = [-40.0, 27.0, 125.0]

[variables]
vdd = "1.2"

[[spec]]
name = "tpd"
max = 3e-10

[cace]
display = "Propagation delay"
parameter = "delay"
template = "tb_inv.sch"
tool = "ngspice"
yaml = """
display: Propagation delay
unit: ps
...
"""

[cace.units]
tpd = "ps"
```

## Say how the result is measured

CACE's template computed `tpd`; CDePT does not read CACE templates, so the testbench gets
the measurement (and, unless its schematic runs its own analyses, the analysis). Put the
lines above the first table of `setup.toml`, or use the Simulation tab:

```toml
save = ["v(in)", "v(out)"]
measures = ["tran tpd trig v(in) val=0.6 rise=1 targ v(out) val=0.6 fall=1"]

[[analysis]]
kind = "tran"
step = "5p"
stop = "1.2n"
```

## Run the suite

```bash
cdept sim suite cells tb_inv --view cace_datasheet
```

```text
test               point   result           tpd
Propagation delay  tt_m40  PASS      4.4977e-11
Propagation delay  tt_27   PASS      4.5138e-11
Propagation delay  tt_125  PASS      4.7007e-11
Propagation delay  ff_m40  PASS      3.8471e-11
...
Propagation delay  ss_125  PASS      5.6411e-11

suite cells/tb_inv/cace_datasheet: 1 test(s), 9 run(s): 9 pass, 0 fail, 0 error; specs: 0 failing, 0 missing
SUITE PASS
```

## Export it

```bash
cdept cace export cells/tb_inv --view cace_datasheet -o inv-out.yaml
```

The datasheet comes back as it was read, with the results of the latest run of every
point added (minimum, median and maximum over the runs, in the result's unit, and the
verdict); the DRC parameter is there again:

```yaml
parameters:
  delay:
    display: Propagation delay
    unit: ps
    spec:
      tpd:
        maximum:
          value: 300
    ...
    results:
    - name: schematic
      tool: cdept
      runs: 9
      status: pass
      tpd:
        minimum:
          value: 38.4711
        typical:
          value: 45.1382
        maximum:
          value: 56.4111
          status: pass
  drc:
    display: Magic DRC
    ...
```

If the testbench was changed in CDePT (other corners, another limit), the export writes
the new values: a changed condition as an `enumerate` list, a changed limit as its new
`value`. `--no-results` leaves the results out; without `-o` the datasheet goes to the
standard output. A testbench that was never imported (`--view testbench`) becomes a new
parameter with its corners, temperatures, variables and sweeps as conditions.

## In the GUI

In the Library Manager, right-click a cell: **Import CACE Datasheet…** asks for the file,
the view prefix and the schematic, and shows the plan and the report above before
anything is written. **Export CACE Datasheet…** (also **Export CACE…** in the Simulation
tab) shows the parameters, the notes and the start of the datasheet before it writes the
file. Test: `cdept-gui/scripts/cace.script`.

## See also

- [CACE datasheets](../reference/files/cace.md) (the mapping, what is kept, what is not supported)
- [Run a suite of tests](suites.md), [setup.toml](../reference/files/setup-toml.md)
