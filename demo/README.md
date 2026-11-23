# Demos

Two replayable GUI scripts. They drive the real program (`cdept gui --script`, see
`docs/GUI-TESTING.md`) on scratch copies of the examples, so nothing in the repository
changes, and leave the window open at the end.

```sh
cargo build && cargo build -p cdept-xschem      # once
demo/run.sh buffer      # the made-up technology (examples/buffer): no PDK needed
demo/run.sh ihp         # IHP SG13G2 (examples/ihp-inv): needs PDK_ROOT, KLayout, ngspice
DEMO_PACE=0.5 demo/run.sh ihp     # shorter pauses (DEMO_PACE=0 runs it as a test and quits)
```

| Demo | Shows |
|---|---|
| `buffer.script` | libraries, cells and views; the schematic in xschem and the hierarchy read from the files; layouts in KLayout with references and a live PCell; what uses a cell; a testbench netlisted into a view |
| `ihp.script` | the technology snapshot (pages of the Technology tab); a PDK standard cell, layout and read-only LEF abstract; the project's own schematic and layout and the net check; a testbench over corners with specs; the suite in one table; Monte Carlo |

`pause N` lines are waits of N seconds scaled by `DEMO_PACE`. Edit the scripts freely:
the commands are the ones listed in `docs/GUI-TESTING.md` and at the top of
`cdept-gui/scripts/*.script`.

Notes for presenting: the Simulation tab is one scroll area, so the results table sits
below the setup form (scroll down, or make the window taller); DRC and LVS are not in the
demo because the example inverter is deliberately unwired (see `docs/howto/lvs-drc.md`
for the clean standard-cell result).
