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
Our UberWave is the public copy (MIT or GPL-2.0 since 2026-11-27, commit ef2497f) `gitlab.symbioticeda.com/ckdur.iso/uberwave` (made
2026-11-27 because `Juan_25/uberwave` refuses our pushes). Its `main` is the original `main`
plus the 9 commits CDePT needs (0108e4b, then the license commit; the branch `ipc-multifile` is at the same
commit). The only working copy is the submodule `cdept-uberwave/uberwave`
(`git submodule update --init cdept-uberwave/uberwave`, then
`git switch main` inside it to work). Its GitLab Pages job is disabled for now
(`.gitlab-ci.yml.disabled`; keep it so after merging from the original). Commits there need an
`AI:` footer when AI wrote them. After pushing, move the pointer in CDePT. The mirror does not update by
itself: add the original as the remote `upstream`, fetch and merge. CI builds the stub.

## Own waveform viewer (SYMBI-89: first fixes and steps 1-4 done 2026-11-27)
`cdept-wavedata` binds UberWave's data engine (C interface `uwdata`); waveform tabs in
`cdept-gui/src/views/wave/` (File > Open Waveform File / Open PSF Results Folder, or
"Waveform Tab" / "(family)" in the Simulation tab). Tests: `cdept-gui/scripts/wave/run.sh`,
`cdept-gui/scripts/wave-tabs.script`; CI job `uberwave` (`scripts/ci/uberwave.sh`) builds the
real engine and the built-in viewer (it needs Symbiotic EDA's GitLab to be up).
UberWave's own viewer stays for good as a separate-window option (do not remove
`cdept-uberwave` or the process fallback). **Next is step 5** (tabs become the default:
Open buttons and `cdept wave`, calculator send target, restore tabs after restart); the user
has not tried the tabs by hand yet. Other open items: NOTES.md (CSV/digital in tabs,
families with mixed analyses, labels of parametric sweep parts, preview before the min/max
index, CJK names). Programs a job waits for die with CDePT
(`libcdept::orphan`, SYMBI-90 fixed).
`ota.script` times out on a loaded machine; `cdept sim suite cells ota5t` passed 90/90.

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
ihp-sg13g2 Magic device map not tried with a real generate; UberWave changes not in the original.
