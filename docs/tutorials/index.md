# Tutorials

A tutorial takes one task of an analog designer from start to end. Each is written here
and can also be followed **inside CDePT**: a card lists the steps, an orange outline
shows what to use next, and a step is ticked off when you have done it.

```sh
cdept tutorial               # list the tutorials
cdept tutorial pdk-setup     # open CDePT and follow one
cdept tutorial pdk-setup --print
```

In a running CDePT: **Help > Tutorials**.

```{toctree}
:maxdepth: 1

pdk-setup
writing-tutorials
```

## What an analog designer does

This is the list of tasks the tutorials are to cover, in the order of a design. It is
also the plan: tutorials are added one at a time, after the one before has been tried
by a person.

**CDePT** says what the program can do for the task today:

| Mark | Meaning |
|---|---|
| GUI | in the window (often also on the command line) |
| CLI | on the command line only |
| Partial | part of the task works; the note says what does not |
| No | CDePT does not do it yet |

These marks come from the manual, the command line help and the roadmap. A tutorial
tries its task from end to end, so a mark is only proven once its tutorial exists.

### A. Setting up

| # | Task | CDePT | Tutorial |
|---|---|---|---|
| A1 | Install CDePT and its helpers; see which tools are found | GUI (Tools tab) | |
| A2 | Make a project for a PDK and check what was read from it | GUI | [`pdk-setup`](pdk-setup.md) |
| A3 | Switch between PDK versions; refresh the snapshot after an update | GUI rescan; versions CLI (`cdept pdk`) | |
| A4 | Bring an existing design folder or LibMan project in | GUI | |
| A5 | Make libraries and cells; copy, rename, move, delete with references following; categories; search | GUI | |
| A6 | Import GDS, SPICE, CDL, LEF, Liberty and Verilog files as cells | GUI | |
| A7 | Version a project with git: what to commit, status marks | Partial: status marks only, commits are made with git | |

### B. Knowing the devices

| # | Task | CDePT | Tutorial |
|---|---|---|---|
| B1 | Find the PDK's devices, their symbols, models and layout cells | GUI (Libraries, Technology tab) | |
| B2 | Characterise a transistor: I-V curves, gm, ro, capacitances | GUI testbench; you draw the bench | |
| B3 | gm/ID sizing: tables and look-ups | No (pygmid is not integrated) | |

### C. Schematic

| # | Task | CDePT | Tutorial |
|---|---|---|---|
| C1 | Draw a schematic: devices, wires, pins, device parameters | GUI (embedded xschem) | |
| C2 | Make the symbol; use the cell in a larger one; walk the hierarchy | GUI | |
| C3 | Write and check a netlist (SPICE, Spectre, Verilog, VHDL) | GUI | |

### D. Pre-layout simulation

| # | Task | CDePT | Tutorial |
|---|---|---|---|
| D1 | Make a testbench (by hand or from a test template) | GUI; new from a template is CLI (`cdept sim new`) | |
| D2 | Operating point and DC sweeps | GUI | |
| D3 | Transient: delays, slew, settling | GUI | |
| D4 | AC: gain, bandwidth, phase margin, loop stability | GUI | |
| D5 | Noise | GUI | |
| D6 | View waveforms: tabs, markers, Bode and eye diagrams | GUI | |
| D7 | Calculator expressions and measures | GUI and CLI | |
| D8 | Specs with limits, pass and fail | GUI | |
| D9 | Corners, temperatures, supply and parameter sweeps | GUI | |
| D10 | Monte Carlo: process and mismatch, statistics, yield | GUI | |
| D11 | Test suites: one table over many testbenches | GUI | |
| D12 | CACE datasheets in and out | GUI and CLI | |
| D13 | Other simulators (Xyce, VACASK), Qucs-S, Verilog-A | Partial: see the simulator pages | |
| D14 | Run on remote machines or SLURM | Partial: SLURM only smoke-tested | |

### E. Layout

| # | Task | CDePT | Tutorial |
|---|---|---|---|
| E1 | Generate a layout from the schematic (device cells with its parameters) | GUI (KLayout); Magic too, without nets | |
| E2 | Place and edit: PCells, matched and parallel devices, hierarchy | GUI | |
| E3 | Route with net highlighting, flylines and cross-probing | GUI: you route by hand, there is no router | |
| E4 | Pins and labels | GUI | |
| E5 | Keep layout and schematic in step; check instances and nets | GUI | |
| E6 | The same in Magic | Partial | |
| E7 | Guard rings, common-centroid generators | No (only what the PDK's cells offer) | |

### F. Verification

| # | Task | CDePT | Tutorial |
|---|---|---|---|
| F1 | DRC with the PDK's KLayout deck; read and fix violations | GUI | |
| F2 | LVS with KLayout | GUI | |
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

## A suggested order

A2, then one small circuit carried all the way through: A5, C1, C2, D1, D3, D6, D9,
E1, E3, F1, F2. An inverter is enough for the first pass. A five-transistor OTA
(`examples/ihp-inv` has one) is a good second pass for D4, D5, D8, D10, D11 and E2.
