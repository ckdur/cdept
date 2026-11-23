# Qucs-S

Qucs-S (25.2.0 in `/opt/ext/OpenLane/bin`) is the second schematic tool of the roadmap.
CDePT runs it as a separate process only (GPL; nothing is linked). Code:
`libcdept/src/batch/qucs.rs`; CLI: `cdept qucs`.

## What Qucs-S can do headless (checked here)

- `qucs-s -n -i in.sch -o out.cir --ngspice|--xyce|--cdl` converts a schematic to a
  SPICE netlist and exits. It works with no display (`env -u DISPLAY`, also with
  `QT_QPA_PLATFORM=offscreen`; no xvfb needed). It prints Qt font warnings on stderr.
- The exit status is **0 even on failure**. A schematic it cannot convert gives
  `Error: Could not go throughAllComps` / `Error giving NodeNames` and a netlist that is
  only the header, an `.INCLUDE` of `ngspice_mathfunc.inc` and an empty `.control`
  block; a missing input gives `Error: Could not load schematic`. CDePT therefore
  judges success from the `Error` lines and from the netlist having devices.
- The netlist carries the analysis of the schematic's simulation component: for
  ngspice a `.control` block (`ac …`, `write spice4qucs.ac1.plot …`, `exit`), for Xyce
  `.ac`/`.tran`… and `.PRINT … format=raw file=spice4qucs.ac.plot` cards.
- `--run` executes the simulator at once (results in `~/.cache/qucs-s`, Qucs-S's own
  data format); CDePT does not use it, a testbench runs the netlist itself.
- `-p` prints a schematic to pdf/png/svg (not used yet).
- Settings are read from `~/.config/qucs/qucs_s.conf` (`QucsHomeDir`, simulator paths).
  There is no command-line or environment way to add library folders.

## Detection

`.sch` is shared with xschem. `Format::of_file` reads the first bytes: `<Qucs` makes it
`Format::QucsSchematic` (view kind Schematic); libraries may also keep them in a
`sch/qucs-s` folder (`library.rs`). This was already in place; nothing changed.

## Testbenches

`setup.toml` of a testbench gets `schematic_kind = "qucs"` (`"xschem"`, or empty:
the named view's own kind, else the cell's xschem schematic, else its Qucs-S one):

```toml
schematic = "cells/rc_lowpass"   # default: the cell the testbench is in
schematic_kind = "qucs"
simulator = "ngspice"            # or "xyce": picks the --ngspice / --xyce netlist
[[analysis]]
kind = "ac" …
```

`sim::run_testbench` calls `sim::netlist_for`, which sends a Qucs-S schematic through
`batch::qucs::netlist_for_sim`: `qucs-s -n` in a temporary folder, then
`sim_netlist` removes the schematic's own analysis (the `.control` block, or the
`.ac`/`.tran`/`.print`… cards for Xyce) and `.end`. The result is `netlist.spice`;
the deck is built as for xschem, so `[[analysis]]`, `measures`, `corners`, specs,
sweeps and Monte Carlo all apply. The analysis component of the Qucs-S schematic is
ignored (it can stay for use in the Qucs-S window).

Example: `examples/ihp-inv/libs/cells/rc_lowpass` (schematic: Vac 1 V, R 1k, C 159n;
testbench: AC 10 Hz to 100 kHz, `find vm(out) at=1000`, spec 0.69 to 0.72).
`cdept sim run cells rc_lowpass` gives 0.7075 with ngspice and with Xyce (needs
`CDEPT_QUCS=/opt/ext/OpenLane/bin/qucs-s` unless `qucs-s` is on `PATH`).

## CLI

```
cdept qucs export <library> <cell> [--view v] [--flavour ngspice|xyce|cdl] [--for-sim] [--out f]
cdept qucs open <library> <cell> [--view v]       # the Qucs-S window
cdept qucs spar <file.s2p>                        # the S-parameter viewer
```
`export` writes `<project>/batch/qucs/<library>/<cell>.cir` unless `--out`; without
`--for-sim` the schematic's analysis stays in. The programs are `$CDEPT_QUCS` and
`$CDEPT_QUCS_SPAR`, else `qucs-s` / `qucs-sspar-viewer` on `PATH` (both are in the
tool table, `tools.rs`).

## GUI

Library Manager, view context menu: **Open in Qucs-S** on a Qucs-S schematic view starts
the window on the file (`batch::qucs::launch`, detached, working folder = the
schematic's). Testbenches on a Qucs-S schematic run from the Simulation tab as any
other (the same `run_testbench`).

## Not done / blocked

- **PDK parts.** The IHP PDK ships Qucs-S libraries and examples
  (`ihp-sg13g2/libs.tech/qucs-s`: `user_lib/*.lib`, `symbols/`, `examples/*.sch`) whose
  schematics use `Lib` components with the path `$HOME/<qucs_workspace>/user_lib/…`.
  Its `install.py` copies them into `~/QucsWorkspace` / `~/.qucs`. Headless export of
  `dc_lv_nmos.sch` failed here (`Could not go throughAllComps`) with or without the
  libraries copied into a private `QucsHomeDir` (private `XDG_CONFIG_HOME`) or
  `HOME`; the cause (a `Lib` component, `INCLSCR`, or the `.SW` sweeps) was not
  found. So CDePT does not set up library paths: run the PDK's `install.py` once, as
  for the GUI. The snapshot of the technology has nothing for Qucs-S.
- No "open with" table exists, so the S-parameter viewer is only reachable by
  `cdept qucs spar` and `launch_spar_viewer` (`is_touchstone` tells `.s1p`…`.snp`);
  the `.s2p` files are not a `Format` yet, so the Library Manager does not list them.
  Not tried here: the viewer's GUI (no display).
- The other helpers (`qucs-sfilter`, `-sactivefilter`, `-sattenuator`,
  `-spowercombining`, `-strans`, `qucs_run_hdl`, `qucs_run_verilog`) are Qucs-S's
  design tools and HDL runners; they are in `/opt/ext/OpenLane/bin` but not wired.
- Qucsator (the native RF simulator) analyses and `.dpl` plots are not read; the SPICE
  export is the only path (RF-only parts were not tried).
- The GUI action starts the window without the project's environment (the button has
  no project at hand); the CLI `cdept qucs open` passes `PDK_ROOT` and friends.
- Hierarchical Qucs-S subcircuits (`.sch` with `<Symbol>` used in another schematic)
  were not tried.
