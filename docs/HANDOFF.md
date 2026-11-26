# Handoff: 2026-11-27

State at the start of the day: `main` at a45461f, equal to `origin/main`, CI green.
[ROADMAP.md](ROADMAP.md) is the plan, [NOTES.md](NOTES.md) the running list of open items.
Tickets are in Plane, project SYMBI, titles starting with `[CDEPT]`. SYMBI-60 to 83 were
brought up to date on this day; SYMBI-79 (VACASK on sky130) stays in the backlog.

## Built since the previous handoff (2026-11-22)
- Phase 4: testbench view, sweeps, Monte Carlo, results store, templates and suites, job
  queue, ngspice (process and in-process), Xyce, VACASK, CACE, remote runs (smoke-tested
  against fake ssh/SLURM only), 5T-OTA milestone.
- Phase 5: raw reader and calculator (CLI); UberWave built into the executable.
- Phase 3 leftovers: Magic batch, netgen, yosys, OpenROAD, Qucs-S export.
- Import of design files, LEF abstract views, `tech.toml` as source of truth.

## UberWave
The built-in viewer needs the branch `ipc-multifile` of the `uberwave` repository (4
commits on top of its `main` on 2026-11-26, 9 on 2026-11-27, tip 0108e4b). **It is only local**: pushing it was refused
on 2026-11-26 ("You are not allowed to push code to this project"). It is checked out at
`../uberwave-ipc`; build with `UBERWAVE_SRC=../uberwave-ipc` (absolute path). The sibling
`../uberwave` is on a branch without the shim. CI builds the stub.

## Tool and test environment
Tools in `/opt/ext/OpenLane/bin`, PDKs in `/opt/ext/OpenPDKs`. Real-tool tests skip unless
`PDK_ROOT`, `CDEPT_NGSPICE`, `CDEPT_XSCHEM`, `CDEPT_XYCE`, `CDEPT_VACASK`,
`CDEPT_OPENVAF`, `CDEPT_MAGIC` or `CDEPT_QUCS` are set. GUI scripts:
`cdept-gui/scripts/run.sh` with `DISPLAY=:0`. Never read `/opt/ext/OpenPDKs/XFAB` (NDA).

## Phases 3, 4 and 5 completion round (2026-11-27, merged into main)
- Phase 3: Magic edits in a CDePT tab (`cdept-magic` helper around the installed
  `tclmagic.so`, [MAGIC.md](MAGIC.md)), `.mag` references and `.mag.json` sidecars, layout
  from the schematic, LVS, Open With, supervised tool windows, per-project Python.
  Magic's licence is permissive (HPND-UC); the helper is GPL only because it runs
  xschem's Tk stand-in.
- Phases 4 and 5: calculator panel, CSV/VCD/FST/GHW readers, jitter/PSD, `dut`, suite
  `[[combine]]`, SAR ADC ENOB milestone, typed UberWave shim (no JSON when embedded).
  UberWave must be at `ipc-multifile` 1a162cf or later.
- Regression after the merges: 891 tests with real tools (892 in a clean clone without
  tools or UberWave, after the locale fix), xschem Tcl and KLayout helper suites, GUI
  scripts (`reflib` and `uberwave-calc` need the setup written in their headers).

## Memory
On 2026-11-26 an UberWave test used 51 GB (a raw header that lies about its counts, fixed
in UberWave 50a38c4) and the session died. Run builds, tests and torture inputs under
`systemd-run --user --scope -q -p MemoryMax=12G -p MemorySwapMax=0`.

## Open from this round
Per phase in [NOTES.md](NOTES.md). The larger ones: no nets or flylines for Magic;
ihp-sg13g2 Magic device map not tried with a real generate; UberWave not pushed.
