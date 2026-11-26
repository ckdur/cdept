# Handoff: 2026-11-26

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
commits on top of its `main`, tip 45356b6). **It is only local**: pushing it was refused
on 2026-11-26 ("You are not allowed to push code to this project"). It is checked out at
`../uberwave-ipc`; build with `UBERWAVE_SRC=../uberwave-ipc` (absolute path). The sibling
`../uberwave` is on a branch without the shim. CI builds the stub.

## Tool and test environment
Tools in `/opt/ext/OpenLane/bin`, PDKs in `/opt/ext/OpenPDKs`. Real-tool tests skip unless
`PDK_ROOT`, `CDEPT_NGSPICE`, `CDEPT_XSCHEM`, `CDEPT_XYCE`, `CDEPT_VACASK`,
`CDEPT_OPENVAF`, `CDEPT_MAGIC` or `CDEPT_QUCS` are set. GUI scripts:
`cdept-gui/scripts/run.sh` with `DISPLAY=:0`. Never read `/opt/ext/OpenPDKs/XFAB` (NDA).

## In progress
Completion of Phases 3 (Magic first), 4 and 5, started 2026-11-26.
