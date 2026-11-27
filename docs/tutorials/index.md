# Tutorials

CDePT teaches itself: **a tutorial runs inside the program**. A floating card lists the
steps, an orange outline shows what to use next, and a step is ticked off when the
program shows you have done it. When you draw a schematic, the card checks the circuit
itself and says what is still missing, such as *"the PMOS: pin G should be on A"*,
however you arranged the drawing. You do all the clicking.

Every tutorial is also a page of this manual, made from the same file.

## Start one

In CDePT: **Help > Tutorials**. From a terminal, in the source tree:

```sh
target/debug/cdept tutorial                 # the list
target/debug/cdept tutorial pdk-setup       # open CDePT and follow one
```

You need:

- CDePT with the xschem helper: `cargo build && cargo build -p cdept-xschem`;
- IHP SG13G2. Set `PDK_ROOT` to the folder that holds `ihp-sg13g2`, or do
  [the first tutorial](pdk-setup.md), which shows how to point CDePT at it;
- ngspice (on `PATH`, or named by `CDEPT_NGSPICE`) for every simulation;
- KLayout for the layout tutorials: `cdept-klayout/build.sh`
  ([installation](../guide/installation.md)).

Except for the first one, each tutorial offers to **make its starting project**, in
which the earlier steps are already done. So you can start anywhere, repeat one, or
recover from a schematic that went wrong. The projects are made in `~/cdept-tutorial/`
(`CDEPT_TUTORIAL_DIR` names another folder) and nothing is written over: a second try
gets a new folder.

## Setting up

| # | Tutorial | You learn | Minutes |
|---|---|---|---|
| S1 | [Set up a project for a PDK](pdk-setup.md) | make a project for a PDK and check what CDePT read from it | 10 |

## Inverter: the whole flow once

A CMOS inverter from an empty cell to a layout that passes DRC and LVS. Do this series
first to learn the program. About 150 minutes.

| # | Tutorial | You learn | Minutes |
|---|---|---|---|
| I1 | [Inverter 1: your library and your first cell](inv-library.md) | libraries, cells and views in the Library Manager | 5 |
| I2 | [Inverter 2: draw the schematic](inv-schematic.md) | draw, size and wire an inverter in xschem; read its netlist | 20 |
| I3 | [Inverter 3: the symbol and a testbench](inv-symbol.md) | make its symbol and draw a testbench around it | 20 |
| I4 | [Inverter 4: the first simulation](inv-transient.md) | a first transient simulation, waveforms, delay and edge times | 15 |
| I5 | [Inverter 5: limits, transfer curve and corners](inv-corners.md) | transfer curve, limits, pass and fail over corners and temperatures | 25 |
| I6 | [Inverter 6: draw the layout](inv-layout.md) | generate the layout, place, add contacts, route, pins, check the nets | 45 |
| I7 | [Inverter 7: DRC and LVS](inv-verify.md) | DRC and LVS: read what they report, fix a mistake | 20 |

## Five-transistor OTA: analog design

The OTA of `examples/ihp-inv`, with the weight on what the results mean. It takes the
use of the program for granted. About 210 minutes, without simulation time.

| # | Tutorial | You learn | Minutes |
|---|---|---|---|
| O1 | [OTA 1: draw the five-transistor OTA](ota-schematic.md) | the five-transistor OTA, device by device, and its symbol | 30 |
| O2 | [OTA 2: the operating point](ota-bias.md) | operating point: currents, node voltages, saturation | 20 |
| O3 | [OTA 3: gain, unity-gain frequency, phase margin](ota-ac.md) | gain, unity-gain frequency and phase margin, against hand estimates | 20 |
| O4 | [OTA 4: the step response as a buffer](ota-step.md) | step response as a unity-gain buffer: settling and slewing | 15 |
| O5 | [OTA 5: sizing against a spec](ota-sizing.md) | sweep the load, change a size against a spec, compare | 30 |
| O6 | [OTA 6: supply and common-mode rejection, and power](ota-rejection.md) | PSRR, CMRR and power | 25 |
| O7 | [OTA 7: every test over every corner](ota-suite.md) | all tests over 15 corner points, and a weakness to find and fix | 30 |
| O8 | [OTA 8: input offset and mismatch](ota-mismatch.md) | input offset from Monte Carlo mismatch | 20 |
| O9 | [OTA 9: noise](ota-noise.md) | noise: flicker, thermal, the corner between them | 20 |

```{toctree}
:maxdepth: 1
:caption: All tutorials
:hidden:

pdk-setup
inv-library
inv-schematic
inv-symbol
inv-transient
inv-corners
inv-layout
inv-verify
ota-schematic
ota-bias
ota-ac
ota-step
ota-sizing
ota-rejection
ota-suite
ota-mismatch
ota-noise
plan
writing-tutorials
```

## What an analog designer does

These are the tasks of a design, in its order. The last column names the tutorials
that cover each; an empty one has none yet.

**CDePT** says what the program can do for the task today:

| Mark | Meaning |
|---|---|
| GUI | in the window (often also on the command line) |
| CLI | on the command line only |
| Partial | part of the task works; the note says what does not |
| No | CDePT does not do it yet |

For a task with a tutorial, the mark has been tried from end to end by the tutorial's
automated replay. The other marks come from the manual, the command line help and the
roadmap, and are not proven in the same way.

### A. Setting up

| # | Task | CDePT | Tutorial |
|---|---|---|---|
| A1 | Install CDePT and its helpers; see which tools are found | GUI (Tools tab) | |
| A2 | Make a project for a PDK and check what was read from it | GUI | [`pdk-setup`](pdk-setup.md) |
| A3 | Switch between PDK versions; refresh the snapshot after an update | GUI rescan; versions CLI (`cdept pdk`) | |
| A4 | Bring an existing design folder or LibMan project in | GUI | |
| A5 | Make libraries and cells; copy, rename, move, delete with references following; categories; search | GUI | [`inv-library`](inv-library.md) |
| A6 | Import GDS, SPICE, CDL, LEF, Liberty and Verilog files as cells | GUI | |
| A7 | Version a project with git: what to commit, status marks | Partial: status marks only, commits are made with git | |

### B. Knowing the devices

| # | Task | CDePT | Tutorial |
|---|---|---|---|
| B1 | Find the PDK's devices, their symbols, models and layout cells | GUI (Libraries, Technology tab) | [`pdk-setup`](pdk-setup.md) |
| B2 | Characterise a transistor: I-V curves, gm, ro, capacitances | GUI testbench; you draw the bench | |
| B3 | gm/ID sizing: tables and look-ups | No (pygmid is not integrated) | |

### C. Schematic

| # | Task | CDePT | Tutorial |
|---|---|---|---|
| C1 | Draw a schematic: devices, wires, pins, device parameters | GUI (embedded xschem) | [`inv-schematic`](inv-schematic.md), [`ota-schematic`](ota-schematic.md) |
| C2 | Make the symbol; use the cell in a larger one; walk the hierarchy | GUI | [`inv-symbol`](inv-symbol.md), [`ota-schematic`](ota-schematic.md) |
| C3 | Write and check a netlist (SPICE, Spectre, Verilog, VHDL) | GUI | [`inv-schematic`](inv-schematic.md) |

### D. Pre-layout simulation

| # | Task | CDePT | Tutorial |
|---|---|---|---|
| D1 | Make a testbench (by hand or from a test template) | GUI; new from a template is CLI (`cdept sim new`) | [`inv-symbol`](inv-symbol.md), [`inv-transient`](inv-transient.md) |
| D2 | Operating point and DC sweeps | GUI | [`inv-corners`](inv-corners.md), [`ota-bias`](ota-bias.md) |
| D3 | Transient: delays, slew, settling | GUI | [`inv-transient`](inv-transient.md), [`ota-step`](ota-step.md) |
| D4 | AC: gain, bandwidth, phase margin, loop stability | GUI | [`ota-ac`](ota-ac.md), [`ota-rejection`](ota-rejection.md) |
| D5 | Noise | GUI | [`ota-noise`](ota-noise.md) |
| D6 | View waveforms: tabs, markers, Bode and eye diagrams | GUI | [`inv-transient`](inv-transient.md), [`ota-ac`](ota-ac.md) |
| D7 | Calculator expressions and measures | GUI and CLI | [`inv-transient`](inv-transient.md), [`ota-sizing`](ota-sizing.md) |
| D8 | Specs with limits, pass and fail | GUI | [`inv-corners`](inv-corners.md), [`ota-rejection`](ota-rejection.md) |
| D9 | Corners, temperatures, supply and parameter sweeps | GUI | [`inv-corners`](inv-corners.md), [`ota-sizing`](ota-sizing.md), [`ota-suite`](ota-suite.md) |
| D10 | Monte Carlo: process and mismatch, statistics, yield | GUI | [`ota-mismatch`](ota-mismatch.md) |
| D11 | Test suites: one table over many testbenches | GUI | [`ota-suite`](ota-suite.md) |
| D12 | CACE datasheets in and out | GUI and CLI | |
| D13 | Other simulators (Xyce, VACASK), Qucs-S, Verilog-A | Partial: see the simulator pages | |
| D14 | Run on remote machines or SLURM | Partial: SLURM only smoke-tested | |

### E. Layout

| # | Task | CDePT | Tutorial |
|---|---|---|---|
| E1 | Generate a layout from the schematic (device cells with its parameters) | GUI (KLayout); Magic too, without nets | [`inv-layout`](inv-layout.md) |
| E2 | Place and edit: PCells, matched and parallel devices, hierarchy | GUI | [`inv-layout`](inv-layout.md) |
| E3 | Route with net highlighting, flylines and cross-probing | GUI: you route by hand, there is no router | [`inv-layout`](inv-layout.md) |
| E4 | Pins and labels | GUI | [`inv-layout`](inv-layout.md) |
| E5 | Keep layout and schematic in step; check instances and nets | GUI | [`inv-layout`](inv-layout.md) |
| E6 | The same in Magic | Partial | |
| E7 | Guard rings, common-centroid generators | No (only what the PDK's cells offer) | |

### F. Verification

| # | Task | CDePT | Tutorial |
|---|---|---|---|
| F1 | DRC with the PDK's KLayout deck; read and fix violations | GUI | [`inv-verify`](inv-verify.md) |
| F2 | LVS with KLayout | GUI | [`inv-verify`](inv-verify.md) |
| F3 | DRC with Magic, LVS with netgen | CLI | |
| F4 | Parasitic extraction | Partial, CLI: Magic, capacitance only | |
| F5 | Post-layout simulation against the schematic | No (the swap to the extracted netlist is not built) | |
| F6 | Electrical rule check | No | |
| F7 | Antenna, density, metal fill | No, beyond what a PDK's DRC deck checks | |

### G. Handing over

| # | Task | CDePT | Tutorial |
|---|---|---|---|
| G1 | Export a self-contained GDS or OASIS | CLI (`cdept layout export`) | |
| G2 | LEF abstract of a block | CLI, with Magic (`cdept magic lef`) | |
| G3 | Liberty model of a block | No | |
| G4 | Digital parts: synthesis and place-and-route for a mixed-signal block | Partial, CLI: starts Yosys and OpenROAD in batch | |
| G5 | Pad ring, chip assembly, final DRC and LVS | No pad ring generator; assemble by hand | |

## Not covered yet

- **OTA layout** (matched pair and mirror, routed, DRC and LVS clean). Generating the
  OTA's layout from its schematic works; routing it has not been tried.
- CACE datasheets, importing existing designs and files, PDK updates, git, export and
  LEF.
- What CDePT cannot do yet has no tutorial: see the tables above (post-layout
  simulation, gm/ID sizing, electrical rule checks, Liberty models).
