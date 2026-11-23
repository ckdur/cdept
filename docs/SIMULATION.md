# CDePT: simulation (Phase 4, first slice)

Status 2026-11-23: a testbench view, the deck CDePT builds, batch ngspice runs over
corners × temperatures, `.meas` results against specs. Code: `libcdept/src/sim/`, CLI
`cdept sim`. Not built yet: GUI, job queue, sweeps of design variables, Monte Carlo,
Xyce/VACASK, results database, waveform viewer (Phase 5).

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

## 4. Open

- GUI: testbench editor, run button, results panel. Job queue (parallel points, cancel).
- Sweeps of `[variables]`, Monte Carlo (`mc` corner sets exist in the snapshot).
- Only the first measurement value per name is kept (`.meas` inside sweeps repeats it).
- Raw file reader (Phase 5); `out.raw` holds several plots when there are several analyses.
- Post-layout: swap the DUT for the extracted netlist (Phase 7).
- Xyce/VACASK through the same deck builder; OpenVAF for Verilog-A.
