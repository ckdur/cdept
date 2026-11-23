# Batch adapters

Tools that run to completion and print a result: Xyce, netgen, Yosys, OpenROAD. Each runs
as a separate process (no linking), in a folder of its own, with its output in a log file.
Code: `libcdept/src/batch/`; CLI: `cdept netgen|yosys|openroad` (`cdept/src/cmd_batch.rs`).

## The layer

- `Adapter { name, env_var, tool }`: the program is `$env_var`, else the binary names of
  the tool's entry in `tools::KNOWN_TOOLS` on `PATH`. Variables: `CDEPT_XYCE`,
  `CDEPT_NETGEN`, `CDEPT_YOSYS`, `CDEPT_OPENROAD` (ngspice keeps `CDEPT_NGSPICE`).
- `Job`: program, arguments, working folder, environment (`Project::tool_env()` on top of
  the process's), environment defaults (set only when nobody sets them), log file. Stdout
  and stderr go to the log; stdin is closed.
  `run()` fails on a bad exit status with the status, the log path and the last 12 lines;
  `run_unchecked()` returns the status for tools that report in the text.
- Each module adds the command line and a reader for the output.

## Adapters

| Tool | Command line | Result |
|---|---|---|
| Xyce | `Xyce -r out.raw -l out.log deck.cir` | raw file, `.meas` from `deck.cir.mt0` |
| netgen | `netgen -batch lvs "<layout> <cell>" "<schematic> <cell>" setup.tcl lvs.log` | `Verdict`, device and net counts, cells with property errors |
| Yosys | `yosys -s synth.ys` (script written by `yosys::synth`, or yours) | netlist, `Stat` (cells, area, cells by type) |
| OpenROAD | `openroad -no_splash -exit script.tcl` | log, `[ERROR]` lines, warning count |
| VACASK | `vacask -qp deck.sim` | one raw file per analysis; `CDEPT_VACASK` (details: VACASK.md) |
| OpenVAF | `openvaf-r -o x.osdi model.va` | OSDI library, cached per technology; `CDEPT_OPENVAF` |

**Xyce in testbenches.** `simulator = "xyce"` in a testbench's `setup.toml` makes
`sim::Simulator::for_setup` pick Xyce and `sim::deck::build` write the Xyce deck: no
`.control` block (one in the netlist is dropped), `.tran`/`.ac`/… cards, `.print <kind>`
for `save` (`v(*)` when empty), `.meas` in upper case (Xyce rejects `trig`/`val` in lower
case), options as `package.name` (`timeint.reltol`; a bare name goes to `device`). The
model set defaults to `xyce` (`[models.xyce]` of the technology). One analysis per deck
(plus `op`); the setup is refused otherwise. The Xyce adapter sets `OMPI_MCA_plm=isolated`,
`OMPI_MCA_btl=self`, `PMIX_MCA_gds=hash` unless set: the build in `/opt/ext/OpenLane/bin`
hangs before reading anything without them, here at least.

**netgen.** The set-up file is `[netgen] setup` of the snapshot (`netgen::project_setup`),
else `--setup`, else an empty one written to the run folder. Verdicts: `Match` (clean),
`PropertyErrors` (also when netgen says "match uniquely" but lists cells with property
errors), `PinMismatch`, `Mismatch`, `Unknown`. netgen exits 0 on all of them, so the
verdict is read from `Final result:`. Netlist paths with spaces are refused.

**Yosys.** `synth` writes `read_verilog`, `hierarchy`, `synth -flatten`, `dfflibmap` and
`abc` with the Liberty file, `stat`, `write_verilog`. The Liberty file is
`yosys::liberty_of(project, library, corner)` from `[refs]` of the snapshot (the digital
defaults when left out); a library with a file per cell cannot be used and says so.

## CLI

```
cdept netgen lvs <layout.spice> <cell> <schematic.spice> <cell> [--setup f] [--dir d]
cdept yosys synth <top> <sources>... [--liberty f | --library L --corner C | --generic] [--out f]
cdept yosys script <script.ys>
cdept openroad run <script.tcl>
```
Runs go to `<project>/batch/<tool>/` unless `--dir` is given. `netgen lvs` fails unless the
circuits match; `openroad run` fails on `[ERROR` lines.

## Tested

Canned output and fake scripts for all four; against the real programs (netgen 1.5.287,
Yosys 0.69, OpenROAD 26Q4, Xyce 7.10, from `/opt/ext/OpenLane/bin`) when found: netlist
comparison, a generic synthesis, a Tcl script and an RC transient with a delay `.meas`.
Not tested: Yosys with a real Liberty file, OpenROAD with a design.

## Open

- Qucs-S adapter (detected, not run). VACASK and OpenVAF: [VACASK.md](VACASK.md).
- Xyce: AC, noise and DC decks are written but only a transient was run; plugins
  (`-plugin` from `[models.xyce] plugins`) and the OSDI options of the model set are not
  passed; `.step` for several temperatures in one run is not used (one run per point).
- Nothing yet starts these from the GUI; jobs run on the calling thread (a job queue is
  being added to `sim`).
