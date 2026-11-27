# CDePT: working notes

What is still to do and what may cause trouble. [ROADMAP.md](ROADMAP.md) is the plan; this
file is the running list behind it. Update it when an item is done or a new problem shows
up. Items marked **(verify)** are suspicions, not confirmed facts.

Last updated: 2026-11-26 (old items checked against the code and the CLI; Phase 3, 4 and 5
completion round started).

---

## Tutorials (2026-11-27)

Built: the in-GUI guide (`cdept-gui/src/guide/`, tutorial files in
`cdept-gui/tutorials/`), the first tutorial `pdk-setup`, its replay
(`cdept-gui/scripts/tutorial.sh`) and the task list `docs/tutorials/index.md`.

- [ ] A person follows `pdk-setup` (`cdept tutorial pdk-setup`); only the replay has.
      The next tutorial waits for that.
- [ ] The PDK list and "Other folder" are chosen by `tech` script commands in the replay,
      not by clicks.
- [ ] Highlights in a detached window are drawn but not replayed (`guide click` works in
      the main window only).
- [ ] Schematic and layout tutorials need anchors inside the pictures of xschem and
      KLayout: the helpers would have to report where things are.
- [ ] The marks in the task list come from docs, `--help` and the roadmap; each is
      proven only when its tutorial exists.

---

## 1. To do

### Before building further on Phase 1
- [ ] Try the GUI by hand. Only headless drawing tests and a 6 s launch were done. Not
      exercised: the native folder dialog (New/Open Project, Add Existing Library),
      right-click menus, every dialog, detached windows with a dialog open.
- [x] Let CI run once: `rfd` builds on the runner and with the MSRV toolchain (CI run
      of fd11b7b, 2026-11-18).

### Phase 1: done (2026-11-22), what is left
All of the original list is built: xschem and GDS readers, hierarchy browser and "used by",
reference rewriting on rename/copy/move (with rollback, `refactor.rs`), file watching,
git status, categories and search, import of existing projects, new-project wizard,
trash. Open:
- [ ] LibMan: import its `define(name, path)` list, read `Category.group`, "open in
      LibMan" (not exercised).
- [ ] Hierarchy gaps: OASIS PCells, instance-level `schematic=`, read-only layouts are not
      indexed, multi-cell GDS is not used by the index.
- [ ] Auto-reload can drop edits made within 0.3-2 s of an outside change.
- [ ] Quick-open shortcuts only work in the main window (Ctrl+P is xschem's add-pin; use
      Ctrl+Shift+F).
- [ ] Old New Project folder flow (`PickFor::NewProject`) is still referenced (state, app,
      library view); `Library::rename_cell_following` has no caller left. Check
      `rename_cell`/`copy_cell` before removing.
- [ ] Member (collection) cells are refused by refactor on purpose; categories follow
      rename/delete/move.
- [ ] Not exercised: native-dialog fallback without a portal.

### Phase 2: done (2026-11-22), what is left
Technology snapshot (`tech.toml` + `tech.catalog.json`, design in [TECH.md](TECH.md)),
`cdept tech|pdk`, per-user `cdept.local.toml`, Technology tab. Open:
- [ ] Decide whether to commit `examples/ihp-inv/tech.catalog.json` (1 MB). The PDK commit
      pinned in `tech.toml` gives "out of date" warnings; NDA PDK snapshots are not
      checked before commit.
- [ ] `tech refresh` overwrites hand edits to `tech.toml`; enabling a PDK library has no
      `cdept.toml` key.
- [ ] Library is one folder: `libs.ref/sg13g2_pr` and `sky130_fd_pr` are left out beside
      their xschem namesakes.
- [ ] Deck switches: `klayout::verify` passes only the IHP variable names; the catalog has
      the roles.
- [ ] Default-deck order is implemented twice (`tech::klayout::decks` and
      `klayout::verify::best_deck`).
- [ ] `tech::scan` has no progress hook (the GUI pdkscan mirrors it).
- [ ] Not exercised: the `ciel` program, gf180mcu and ihp-sg13cmos5l (not installed), a CI
      KLayout job.
- Never read `/opt/ext/OpenPDKs/XFAB` (NDA); discovery only stats its subfolders.

### Import files (SYMBI-64, 2026-11-23, [howto](howto/import-files.md))
`libcdept::importer`, `cdept import-files`, Library Manager "Import Files…". Open:
- [ ] Not split: OASIS (copied whole), VHDL, Verilog-A, Magic, Qucs, xschem (one file, one
      cell). A schematic that places symbols of a library the project lacks is not
      followed.
- [ ] Verilog dependencies are found by words of the module body, not by parsing
      instances; Liberty/LEF splitting is textual (no `.lib` `include` or `ifdef`).
- [ ] An imported GDS gets no `.gds.json` sidecar; the GUI file field adds one file at a
      time (the native chooser picks one).

### Phase 4 (first slice, 2026-11-23, [SIMULATION.md](SIMULATION.md))
`cdept sim run|deck`, `setup.toml` testbench view, deck builder, ngspice batch, specs. Open: see
SIMULATION.md §4 (GUI, job queue, sweeps, results DB). `libcdept/tests/real_sim.rs` needs
`CDEPT_XSCHEM` and `CDEPT_NGSPICE` set to run (else it skips).
Job queue / process supervisor: `libcdept::jobs` ([JOBS.md](JOBS.md)); `sim::run_testbench` runs points in parallel, `cdept sim run --jobs N`. Programs die with CDePT since 2026-11-27 (SYMBI-90, JOBS.md); a process-group kill on cancel is still open.
- [x] SLURM/ssh remote runs (SYMBI-82): `[jobs.remote.NAME]` in cdept.toml, `remote` in setup.toml, `cdept sim run|suite --remote`, `cdept jobs remotes [--check]`; `libcdept::jobs::remote`, `sim::remote`. SMOKE-TESTED ONLY against fake ssh/rsync/sbatch/squeue/sacct/scancel (and real ngspice behind them); never a cluster. Shared folder is the mode to trust; rsync mode only against a fake rsync. The run point's time limit is a `Watch.timeout` (wired: `sim::run_testbench_with` passes the per-point limit to `sim::remote::run_in_job`; checked 2026-11-27). See howto/remote-runs.md.
- [x] In-process ngspice backend (SYMBI-71): `simulator_backend = "inproc"` / `cdept sim run --backend inproc`, one worker thread owns libngspice, points are queue jobs, progress from the status callback, cancel = `bg_halt`; same `tpd` as the process to 1e-12 (tt and Monte Carlo). See NGSPICE.md.
- [x] Cancelling a Xyce point kills Xyce (`batch::Job::run_watched`); Magic batch runs have a time limit (`[magic] timeout`, `--timeout`, default 15m) and `Magic::in_job` for the queue (SYMBI-71).

Milestone (SYMBI-72, 2026-11-23, [TESTS.md](TESTS.md) §6): corner sweep of an OTA from the GUI.
- [x] `examples/ihp-inv/libs/cells/ota5t`: 5-transistor OTA (PMOS pair, NMOS mirror, PMOS
      tail, 20 uA), testbench schematics `tb_ota5t` (open loop through 1 TH / 1 F) and
      `tb_ota5t_step` (unity gain); views `ac_open`, `tran_step`, `dc_op` (all six
      saturated, from PSP's `vdss`), `power`, `psrr`, `cmrr`, `ac_cload`, `suite` (90 runs
      over tt/ff/ss/fs/sf x -40/27/125 C, all pass). tt 27 C: 35.8 dB, UGBW 145 MHz, PM 59 deg.
- [x] Real examples for `psrr_cmrr` (the OTA) and `inl_dnl` (`cells/flash3`, a behavioural
      3-bit flash ADC: DNL 0.082 / INL 0.049 LSB, as computed from the ladder).
- [x] Templates declare `[[sweep]]` and `[montecarlo]` from their parameters (setup.toml's
      tables win); project template `examples/ihp-inv/templates/ac_load.toml`.
- [x] Sweeps and Monte Carlo in the Simulation tab's form (SYMBI-77): own rows, inherited template ones read-only (Override copies), point count with a warning above 500, Monte Carlo corners from the model set with the CLI's error text, Xyce modes disabled until a corner is named, VACASK refused inline; Save merges into the file with toml_edit (`views/simulation/keep.rs`: comments and untouched keys stay). Code in `cdept-gui/src/views/simulation/sweeps.rs`, script `sweeps.script`, screenshot `sim-sweeps.png`.
- [x] Xyce Monte Carlo `mismatch` and `both` (SYMBI-81): root cause is that the PDK's Xyce `sg13g2_mos{lv,hv}_mod.lib` have no `mm_ok`/`agauss()` and no `*_mm` sigmas (only the MOS capacitor has them). Xyce draws `agauss()` per instance under `.SAMPLING useExpr=true`, so `libcdept/src/sim/xyce_mm.rs` writes those two files with the ngspice mismatch expressions (sigmas read from the ngspice mismatch files) into `.cdept/xyce-mm/<pdk>/`; `mc::Plan::mismatch_models` makes the run read the models there. No second sampler (the VACASK `vacask::stat` draws are for a simulator without random numbers). Statistics tb_inv tpd, 150 runs, seed 3 (mean +- std): mismatch ngspice 5.431e-11 +- 1.222e-12, Xyce 5.401e-11 +- 1.270e-12; process ngspice 5.457e-11 +- 4.05e-12, Xyce 5.428e-11 +- 5.37e-12 (the PDK's Xyce `_stat` sigmas are larger, e.g. vfbo 0.01 vs 0.005: not CDePT's); both ngspice 5.468e-11 +- 4.28e-12, Xyce 5.430e-11 +- 5.61e-12 (each = process and mismatch in quadrature). The `_vfbo_mm` aliases of the Xyce `mc` corner are now added by `mc::plan` (no `[variables]` in the view). Real test `xyce_mismatch_and_both_agree_with_ngspice`; the Simulation form no longer disables the modes for Xyce.
- [x] `startup_settle`: `settle_time` subtracted `t_step` twice (negative times); fixed.
- [x] Tests: `libcdept/tests/real_ota.rs`, GUI `cdept-gui/scripts/ota.script`, screenshot
      `docs/_static/screenshots/ota-suite.png` (`docs/screenshots/ota-suite.script`).
- [x] `psrr_cmrr` takes the differential gain as a constant (`adm_db`): in a suite,
      `[[combine]]` now uses the gain of the same run point (2026-11-27, TESTS.md §4).
- [ ] ngspice runs slowly when many run at once on a loaded machine (the PSP OSDI models:
      a 1 us transient took 3 s alone, 67 s under a load average of 300).
- [ ] No layout of the OTA.

### Batch adapters (2026-11-23, [BATCH.md](BATCH.md))
`libcdept/src/batch/`: Job/Adapter layer, Xyce (also `simulator = "xyce"` in testbenches),
netgen LVS, Yosys synth, OpenROAD scripts; `cdept netgen|yosys|openroad`. Tested against the real
programs in `/opt/ext/OpenLane/bin`. Open: VACASK, Qucs-S, Xyce AC/noise/plugins, Yosys with
a real Liberty file, GUI. The OpenLane Xyce hangs without the Open MPI single-process settings
(the adapter sets them).

### Embedded xschem ([XSCHEM.md](XSCHEM.md))
- [x] Decisions: helper process (GPL helper, MIT rest), pinned to the asic-all commit,
      no patches unless unavoidable.
- [x] Step 0: submodule, headless engine (load, netlist, SVG, instances), CLI
      `cdept netlist` and `cdept xschem …`. xschem's sources are unmodified.
- [x] `cells` layout for xschem views (XSCHEM.md §7): decided B, the symbol names its
      schematic with a `schematic=` attribute. Written on create, rewritten on copy and
      rename, checked by `cdept xschem link [--fix]`.
- [x] Example project `examples/buffer` (inverter, buffer, testbench in two `cells`
      libraries; generic xschem devices, no PDK). A test netlists it; checked by hand
      in ngspice and in stock xschem 3.4.8RC (asic-all image).
- [x] Link check in the GUI (2026-11-18): warning marks on library, cell and symbol
      view (tree and Library Manager), tooltip with the expected value, "Fix schematic
      link" on right-click and in the Inspector, "Fix all schematic links…" in the
      Library menu (links to another cell only when ticked, like `--foreign`).
      Checked on load, Reload and after library operations; a symbol saved from an
      xschem tab is not re-checked until then.
- [x] CI with the new job steps (submodule checkout, flex, bison, tcl-dev,
      libcairo2-dev): builds. The first run failed in the tests: helpers starting
      together fought over creating `~/.xschem` (XSCHEM.md §6); fixed in the helper.
- [x] The helper shares `~/.xschem` with the user's own xschem (decided 2026-11-18):
      that is what makes copy and paste work between CDePT tabs and stock xschem
      windows. The price: "Open recent" lists files of both.
- [x] Steps 1 to 4 (2026-11-18): xschem draws and edits inside a CDePT tab. Stand-in
      Xlib on cairo, stand-in Tk in Tcl (xschem's own menus and dialogs, mirrored),
      library picker, close flow, Hierarchy panel, Inspector preview and selection,
      "Netlist to View". Compared with stock xschem: pixel-identical (XSCHEM.md §4, §9).
      Test driver for the real GUI: `cdept gui --script` (GUI-TESTING.md).
- [x] `[xschem] rc` in `cdept.toml`: the PDK's `xschemrc`. Checked with IHP SG13G2.
- [x] `PDK_ROOT` and `PDK` from the project (2026-11-18): `technology` → `PDK`, new
      optional `pdk_root` → `PDK_ROOT`, environment as fallback; used for `$VAR`
      expansion and passed to every tool (`Project::tool_env`). `cdept info`, `cdept
      init --technology --pdk-root`, Project → Project Settings…, PDK in the status bar.
- [ ] `pdk_root` is machine-specific: a project shared through git needs a per-user
      override (e.g. a local, uncommitted file) or must leave it to the environment.
- [ ] Changing the PDK in Project Settings does not restart open xschem tabs (a Console
      warning says to reopen them). No folder-browse button for the PDK folder.
- [x] "Make symbol from schematic" writes `<cell>/symbol/<cell>.sym` with its
      `schematic=` link in `cells` libraries (2026-11-18): the helper replaces xschem's
      Tcl procedure `make_symbol`, xschem itself is unchanged. Also `cdept xschem
      make-symbol <library> <cell> [--force]`. "Open symbol of this schematic" already
      looked in the view folders.
- [x] "Make schematic from symbol" (Ctrl+L, Symbol menu) in `cells` libraries
      (2026-11-18): redone in Tcl in `cdept.tcl`, byte-identical to xschem's
      `create_sch_from_sym` (a test compares them; keep in step when the submodule
      moves). Also works on an open symbol with nothing selected. CLI `cdept xschem
      make-schematic <library> <cell> [--force]`. The new schematic has unwired pins,
      so netlisting reports "undriven node" until it is drawn, as in stock xschem.
- [x] Preview panes inside xschem's own dialogs (file browser, symbol chooser, key
      cheat sheet), and a Picture column in CDePT's library picker (2026-11-18).
- [x] Clipboard from the system into xschem: Ctrl+V and middle-click (primary) in
      dialog fields, Tcl `clipboard get` / `selection get` see the system clipboard.
      Not tested on Wayland without Xwayland.
- [ ] Scripted input in detached windows; run the native file chooser once by hand
      (also Waves → Tran/Op and the graph's "Raw file:" button use it).
- [x] Let CI run: with `fonts-dejavu-core` the helper's drawing tests pass (CI run of
      fd11b7b, 2026-11-18).
- [x] Simulation and graphs checked with ngspice 47 and the IHP PDK (2026-11-18):
      simulator setup, netlist, simulate (background and foreground), jobs, waveforms,
      graph dialog, cursors, zoom. Fixed four bugs; a failed job now shows "Job failed"
      and a Console line. Tests without ngspice (`t_sim.tcl`, `buffer-sim.script`).
- [x] Dialogs size to their content, button rows stay visible; the helper no longer
      passes its made-up `DISPLAY=:0` to programs it starts.
- [ ] Graph dialog "Del" removes the signal under the text cursor, which CDePT does not
      report to xschem; edit the list text instead.
- [ ] With the IHP benches use "Ngspice Control mode": batch mode (`-r $n.raw`) deletes
      the raw file their `.control` block writes (xschem/ngspice, not CDePT). The IHP
      `xschemrc` sets `netlist_dir` to `$PWD/simulations`, so it depends on where CDePT
      was started.
- [x] Install layout (2026-11-18): `scripts/install.sh --prefix P [--destdir D]` →
      `bin/cdept`, `bin/cdept-xschem`, `share/cdept-xschem/`; CI runs it as a smoke
      test. `libjpeg-dev` installed in CI (JPEG stays optional).
- [ ] asic-all: entry drafted in `bouquet_assembler/tool_library.yaml` and
      `asic-all.yaml` (centerstage repo, uncommitted), not built: the image has no Rust
      toolchain and the cdept repo is private (needs credentials; the gitlab mirror
      path in the entry is a guess).
- [ ] Try it by hand: first session by the user on 2026-11-18 found four problems, all
      fixed the same day (below). Keep trying: graph/simulation dialogs, detached
      windows, the native file chooser.
- [x] Selection rectangle stuck after a double-click on an instance: the GUI dropped
      the button release while xschem's property dialog was modal, so later motions
      carried Button1 and xschem started a rubber band. Releases now go through while a
      dialog is open (`schematic/mod.rs`); check in `scripts/buffer-smoke.script`.
- [x] Console "vibrated" on multi-line messages (e.g. a failing simulation): rows of
      unequal height in `ScrollArea::show_rows`. One fixed-height row per text line now.
      Not seen yet: xschem's own job-error window, if a failure shows up there instead.
- [x] Ctrl+Z / Ctrl+Shift+Z / Ctrl+Y undo and redo in a schematic tab, and Edit → Undo
      / Redo work (in xschem Ctrl+Z is "zoom out", undo is `u`; both kept for `u`).
- [x] File → Export (PNG, SVG, PDF) for the active schematic. xschem's own "Image
      export" was there all along, but under the "Schematic" menu where xschem's File
      menu is merged, so nobody found it.

### Embedded KLayout ([KLAYOUT.md](KLAYOUT.md))
- [x] Decisions (2026-11-19): helper process (KLayout is GPL-3.0), submodule
      `cdept-klayout/klayout` pinned to v0.30.12 (asic-all), no patches; full Qt5 build
      with `cdept-klayout/build.sh` into `cdept-klayout/.bin` (~20 min at -j30).
- [x] References between layout views: ghost cells + `<file>.json` sidecar (own schema
      `cdept-layout` v1); resolved through the sidecar, else searched by name in the
      libraries; not found → red box with X and "<label> cannot be found". Save writes
      only the file's own cells. `libcdept::layout` (GDS reader/renamer, sidecar,
      resolver); create/copy/rename keep sidecars and the top structure name right.
- [x] CLI: `cdept klayout version|python`, `cdept layout refs [--fix]`, `export`
      (self-contained stream), `png`.
- [x] Helper: frames (bgra), input, all edit modes, undo/redo, copy/paste, layers,
      cells, selection, status, notices; 30 protocol tests (`cdept-klayout/tests/run.sh`).
      1920×1080: ~17 ms per change; 4K: ~75–85 ms (whole frame each time).
- [x] GUI: layout document tab (toolbar, Layers/Cells/Selection panels, Place Cell…,
      export, close flow, Inspector preview, script commands; `layout-smoke.script`).
- [x] Edit in place (2026-11-20): Shift+X / Layout › Edit in Place descends into the selected
      instance, Shift+B returns (Ctrl+Shift+B to the top; Ctrl+Shift+X is taken: any Ctrl+X is
      a cut). Save writes the root and every external file changed in place (own cells
      under their own names, references kept under the file's names; SaveReport `also`,
      one sidecar each). Read-only libraries and files with unsaved changes in another tab
      are locked (`locks`); a change to a locked file is undone at once. Other tabs showing
      a written file reload (a warning instead when they have unsaved changes).
      `layout-inplace.script`, helper tests `InPlace`, `edit_inv_in_place_from_buf`.
- [x] Properties dialog in egui (`views/layout/properties.rs`, helper `properties.py`):
      box, polygon (move by offset), path (width, ends, points), text, instance
      (transform, array); one undo step per Apply; instances of external cells change
      place only. Opens with Q, a double click in select mode, the tool bar, Layout menu.
- [x] PCells: Layout › Place Library Cell… lists KLayout's libraries (PDK on KLAYOUT_PATH,
      also those bound to a technology such as IHP's SG13_dev; Basic); the parameter form
      is built from the PCell declaration (all types, choices, groups, hidden/readonly);
      PCell instances change parameters in the properties dialog. Helper test places an
      IHP nmos when the PDK is on this machine.
- [x] Box mode: press-drag-release draws a box (the helper turns it into KLayout's two
      clicks); two clicks still work.
- [x] OASIS views are followed like GDS (`libcdept::layout::oasis`: CELLNAME tables, CELL,
      PLACEMENT, CBLOCK; rename rewrites the file uncompressed, non-strict, S_CELL_OFFSET
      and the END signature recomputed). Checked against KLayout's writer in all modes
      (`libcdept/tests/oasis.rs`). `.gds.gz`/`.oas.gz` are not supported (not needed:
      CDePT is the editor and writes its own views).
- [x] Renaming a cell rewrites the layout views that place it (`layout::follow`,
      `Library::rename_cell_following`, used by the GUI): sidecar entry, structure name in
      the file (when it was the cell name and the new name is free), sidecar re-keyed.
      Read-only libraries are reported, not changed. Deleting a cell/view logs the views
      that now show "cannot be found". `refs --fix` repairs a library renamed outside
      CDePT (search by the sidecar's cell name); a cell renamed outside stays missing.
- [x] DRC/LVS (`libcdept::klayout::verify`, KLAYOUT.md §8): the PDK's decks (`[klayout]
      drc`/`lvs`, else found in the KLAYOUT_PATH folders, e.g. IHP `ihp-sg13g2.drc`,
      `sg13g2.lvs`) run by `klayout -b` on an exported self-contained GDS; `.lyrdb` read in
      Rust (quick-xml), `.lvsdb` summarised by the helper script `lvsdb.py`; schematic
      netlisted by xschem with `lvs_netlist`. CLI `cdept layout drc|lvs`; GUI Layout › Run
      DRC/LVS (background, unsaved edits included), results panel, click → highlight +
      zoom. Tested on the PDK's sg13g2_inv_1 (DRC M1.a found; LVS match and mismatch).
- [x] CI job `klayout`: builds KLayout from the submodule (cached on commit + build.sh),
      runs the helper tests and the Rust KLayout tests. Not yet seen running on GitHub.
- [ ] Edit in place: locks are exchanged once per GUI frame; a reload loses zoom and
      selection; a different database unit in an external file is rewritten with the root's.
- [x] PCell shape parameters: edited as coordinates in the form (point, box, polygon,
      path, edge) and dragged in the picture (select mode: squares on the handles of the
      selected PCell instance; helper `handles.py`, one undo step). KLayout's partial
      mode cannot finish a drag without a window, hence our own. Basic's coercion is
      ported (`basic.py`: CIRCLE, ELLIPSE, DONUT, PIE, ARC): KLayout's Python binding of a
      C++ PCell's `coerce_parameters` runs the base class's (none).
- [x] Properties › Cell › Change…: another library cell/PCell or layout view for an
      instance, transformation kept (`replace_cell`, one undo step).
- [x] PCell views (2026-11-20): `<cell>/pcell/<cell>.pcell.py` (Format::KlayoutPcell,
      ViewKind::Pcell, template), registered by the helper as one KLayout library per
      CDePT library before layouts are read (`RefTable.pcells`, `pcells.py`); files
      looked at every second, instances rebuilt (not an edit); errors as notices with
      file:line. Example `logic/rpoly` placed by `logic/rdiv`. Library cells stay live
      in files written back by edit in place; export has no KLayout context. IHP nmos,
      rsil, cmim survive save and reopen live.
- [ ] Renaming a PCell view's cell or library breaks layouts placing it (they name
      LIB/PCELL); a PCell of a library not registered inside an external file is
      copied as plain geometry and loses its link if that file is written back.
- [ ] Basic TEXT's computed (read-only) parameters are not recomputed in the form.
- [ ] DRC/LVS: no cancel in the GUI; deck variables (-D) only from the CLI; only
      input/topcell/report/schematic/target_netlist are passed (IHP's names).
- [x] Library rename (`cdept lib rename`) and cell rename/move rewriting the schematics,
      layouts, netlists and HDL that use a cell (`refactor.rs`, 2026-11-22).
- [ ] In the properties script, a single click on a just-placed PCell sometimes selected
      nothing in the GUI (helper tests select it fine); not tracked down.
- [ ] `install.sh` installs the helper script only, KLayout comes from the system or
      `cdept-klayout/.bin`.
- [ ] Try it by hand.

### Linking schematics with layouts ([LINKING.md](LINKING.md))
- [x] Link record (2026-11-20): layout instance ↔ schematic instance by GDS property 61
      (name), survives GDS/OASIS save; sidecar `cdept-layout` v2 (`source`, `instances`),
      v1 still written without a link. Rust reads the names (`layout::read_placements`).
- [x] Device map `cdept-devices` v1 (own TOML): `pdk/<PDK>/devices.toml` shipped (IHP
      SG13G2: MOS LV/HV, rsil/rppd/rhigh, cmim/rfcmim, taps, antennas, pnpMPA, npn13G2),
      project's `[klayout] devices` on top; templates `@w`, `@{w|u}` (SPICE → "0.5u").
- [x] `cdept layout link` (generate/update: add, update parameters, replace, take over
      unbound instances, `--remove`, pins) and `cdept layout check-source` (plan,
      exit 1 when out of sync); helper `link.py` (`link_apply`, `link_instances`,
      `link_select`, `link_selected`, `link_highlight`), one undoable step.
- [x] GUI: Generate / Update from Schematic (layout tab, schematic tab Cell menu), Link
      panel, Open Schematic / Open Layout, cross-probing both ways (`layout-link.script`).
      Example `examples/ihp-inv`; tests `libcdept/tests/link.rs`, helper `Link`.
- [x] Nets (2026-11-20, LINKING.md §8): schematic nets from xschem
      (`instance_nodemap`, hierarchy flattened through project cells), layout nets from
      the PDK's LVS deck (`klayout -b`) read by the helper script `connectivity.py`
      (`l2n.py`: nets flattened by walking the hierarchy, devices with terminal
      places, bound instances, top-cell labels); devices matched to bound instances by
      place, pins to terminals by name/order/`terminals` (S/D swappable); comparison
      in Rust (`link::nets`): ok/open/short/not placed, flylines (MST over the pieces),
      unconnected terminals, pin labels. `cdept layout check-nets [--json]`. GUI:
      Update Connectivity (background, unsaved edits included, "out of date" by the
      helper's `revision`), Nets panel, flylines, Probe Nets (N), cross-probing of
      nets with schematic tabs (`cdept::selection` reports `nets`, `hilight_netname`).
      Example inverter wired at A/Y (`examples/ihp-inv/scripts/wire-inv.py`). Tests:
      `libcdept/tests/real_nets.rs`, helper `test_nets.py` (generic deck, no PDK),
      `layout-nets.script` (`CDEPT_EXAMPLE=ihp-inv`).
- [x] Nets, round 2 (2026-11-20, LINKING.md §8.1–8.4):
      - live flylines: after an extraction the helper traces the wiring between its
        terminals again after each change (`live.py`, `pya.NetTracer` with the
        technology's net tracer connectivity from the `.lyt`), debounced (300 ms),
        devices as extracted (moved with their instance; deleted instances drop out;
        added ones wait for Update); CDePT compares again
        (`Connectivity::with_live`). Measured: IHP inv 3 ms, buf 5–8 ms, 100 devices
        120–135 ms (setup once 285 ms). An update over 250 ms turns live off with a
        notice (manual Update stays);
      - flyline style: colour, width, line (solid/dashed/dotted/dash-dotted),
        opacity (blended with the background), colours of a shown and of a shorted
        net, live on/off and delay; the user's GUI settings (Nets › Style…, Layout ›
        Flyline Style…), `[klayout.flylines]` in cdept.toml on top;
      - parallel devices: groups found in the schematic (same symbol, same nets,
        S/D either way), recorded in the sidecar (`parallel`, joined on/off, own
        groups); a joined group gets one flyline per net, and one layout device the
        extraction combined stands for all its members. `cdept layout parallel`
        (`--off/--on/--group/--ungroup`), Nets panel checkboxes;
      - Update Connectivity saves an open schematic with unsaved changes first (the
        comparison reads it as saved); no LVS deck: "this PDK has no KLayout LVS deck:
        net checking is not available";
      - the ihp-inv inverter schematic showed `*` on open: xschem splits a wire at a
        T-junction on load (A and Y joined the gate and drain wires in their middle);
        the example now has the wires split.
      Tests: nets unit tests (parallel, live, style), `parallel_devices_are_found_in_the_schematic`,
      helper `test_nets.py` live and style, `layout-nets.script` (style, live short,
      live off, save before update).
- [ ] Nets: live tracing keeps connections the net tracer cannot see (a MOS bulk
      through its well, labels on no wire at the extraction until a wire is under
      them) as extracted; instances added since the extraction count after Update;
      `m=`/`ng` multiples are one instance (nothing to group); bus instances
      (`M1[3:0]`) are one name; parallel groups are recorded by the panel and
      `cdept layout parallel`, not by `cdept layout link`.
- [x] Pins as CDePT objects (2026-11-20, LINKING.md §9): the sidecar lists them
      (version 3: name → direction, net, shapes on conductors, label); the device map's
      `[pins]` says what the stream holds (`generate`, `box`, `size`, `layers` =
      conductor → pin layer, label layer; IHP: every metal with datatype 2 and 25).
      Without a pin layer a pin is CDePT's only (in-memory layer `CDEPT_PINS`, drawn in
      the pin colour, never written). In the editor a pin is every shape carrying GDS
      property 62 = its name (helper `pins.py`): moves, undo, copy like any shape; the
      save report carries the pins to the sidecar. Pins panel (Layout › Pins Panel:
      direction, rename, delete, label to pin, statuses), Draw Pin, Pin from Selection;
      Generate makes the schematic's pins (old labels taken over); check-source lists
      pin differences; the net check and live flylines use the pins (pin point on its
      conductor, never the substrate net); IHP LVS sees them (labels on the metals).
      `cdept layout pins [--json]`. Example `examples/ihp-inv` migrated (inv, buf: four
      pins each). Tests: `link::pins`, plan and nets unit tests, helper `test_pins.py`,
      `pins_are_in_the_sidecar_the_stream_and_the_checks` (IHP), `layout-pins.script`.
- [x] Cross-probing through the hierarchy (LINKING.md §10): instance paths (`x1/M1`)
      both ways; `cdept::selection` reports xschem's `path` and the schematic shown; a
      layout linked with the schematic shown inside x1 maps below it; every linked
      layout follows; devices inside other views' cells are selected for showing until
      the next press or key; nets by every name they have (`x1/A`). Generate saves an
      open, modified schematic first.
- [ ] Pins: a pin is not moved with the wire under it; OASIS pin shapes are not read by
      Rust (`cdept layout pins` checks the stream for GDS only); with CDePT's pin layer
      in the layer list, a click on overlapping shapes may pick another layer's than
      before (seen in `layout-properties.script` with a pin added; not looked into).
- [ ] Cross-probing: xschem selects only in the schematic it shows (x1 at the top for
      `x1/M1`); a layout tab descended (edit in place) maps from its top cell. PDK
      PCells need `[klayout] tech` to read back live; bus instances (`M1[3:0]`) count
      as one name.

### Phase 3: Magic and the tool hub (2026-11-27, [MAGIC.md](MAGIC.md), [tools.md](tools.md))
- [x] Magic in a tab: helper `cdept-magic` (GPL because it runs faketk.tcl) loads the
      installed `tclmagic.so` with a Tk stub table and Xlib of its own; no Magic build,
      no submodule, no patch. Four Magic 8.3.623 problems worked around (MAGIC.md, "What
      had to be worked around"), one of them by writing an unexported variable found in
      `tclmagic.so`'s symbol table (`windCaptionPixels`): a stripped `tclmagic.so` gets a
      warning and windows cut off at the bottom. **(verify)** on other Magic builds than
      8.3.623 (the stub slots and `Tk_FakeWin` are Tk 8.6's).
- [x] Magic's licence is permissive (HPND-UC); the old docs said GPL.
- [x] `.mag` references: relative `use` paths + `<cell>.mag.json` (cdept-layout schema);
      rename/copy/move (Library and refactor), Hierarchy index, `cdept magic refs`.
- [x] Layout from schematic (`pdk/{sky130A,ihp-sg13g2}/magic-devices.toml`), check-source,
      LVS (netgen), LEF, GDS import/export, cross-probing of instances, DRC panel.
- [x] Opening views: choices per format, Open With, the question on double click,
      `cdept open`, supervised tool windows (logs, limit, stop/restart), klive, `[python]`.
- [ ] Magic: nets/flylines, net cross-probing; more than one window per helper; Magic's
      wrapper (menus, coloured layer tool bar); crash backups (unsaved edits are lost
      when the helper dies); gencell dialogs through faketk not exercised.
- [ ] `cdept-magic` needs tk-dev (headers) to build; CI installs it, not yet seen on CI.
- [ ] Tool windows are not stopped when CDePT quits; xschem window needs a stock xschem
      (not on the development host); the Tools tab has no row for `cdept-magic` itself.
- [ ] ihp-sg13g2 Magic device map not checked against a real generate (sky130A was).
- [ ] The example's `tech.catalog.json` (2.8 MB) is not committed (`cdept tech refresh`).
- [x] Torture (cdept-magic/tests/torture.rs, libcdept/tests/magic_refs.rs,
      magic-torture.script) found and fixed: paths with non-ASCII letters or spaces
      (Magic refuses them: the folder becomes its working folder), the start-up script
      read as UTF-8, the current point on the window border after a resize (commands
      failed with an empty error), windows under 64×64, Ctrl+S passed to Magic instead of
      saving. Survived: 3000 queued inputs + 60 resizes, 100000×100000 (clamped), a
      40000-rectangle layout (~1 s), kill -9 mid-edit (file intact, clean restart), Magic's
      own `quit`, two helpers at once, a failed rename rolled back, a sparse 512 MB file
      (refused by size), CRLF files, 5000 `use` lines, broken sidecars, cycles.
- [ ] Hung `Xyce -v` probes (tool detection of GUI runs) piled up and used up inotify
      instances, which breaks `libcdept/tests/watch.rs` ("cannot watch files"); fixed by
      "Tool probes: Open MPI single-process settings" (Phase 4/5 round).

### Smaller gaps left in what exists
- [x] Edit project fields (name, description, technology, PDK folder) from the GUI.
- [ ] Edit or remove libraries that come from an `include` file; today the GUI refuses.
- [ ] Change a library definition (path, read-only, layout): done in the CLI (`cdept lib
      set`); the GUI side was not checked.
- [x] Rename and copy a single view (`cdept view`); rename a library (`cdept lib rename`).
- [x] Magic layout template: the snapshot's `[magic] tech` (Phase 2), with a sidecar (2026-11-27).
- [x] CLI for cells and views (`cdept cell`, `cdept view`).
- [ ] A text field as fallback for the folder dialog (see §2.1).

### Carried to later phases
- Phase 2: GDS template units and the xschem header version should come from the
  technology, not constants. `technology` in `cdept.toml` is read but unused.
- Phase 4: a testbench view is recognised by a `setup.toml` or a `tb_` name. That is a
  guess until the testbench format exists.

### Phases 4 and 5 completed (2026-11-27, worktree agent-a608591bd9f97a5f7)
Done: typed UberWave shim (no JSON on the embedded path; typed control `app/Control.cpp`
shared with IPC), eye/Bode/markers/cursor callback in UberWave, CSV + VCD/FST/GHW readers,
jitter/PSD/histogram/sample functions, the calculator panel, `dut`, `[[combine]]`, the
SAR ADC milestone. Found by torture and fixed: UberWave allocated 51 GB on a raw header
that lies about its counts (RawHeaderParser); CDePT's ascii raw reader overflowed its
capacity on the same; the evaluator overflowed the stack on a long sum and the parser on
deep nesting; `resample(n=1e12)`/`fft(n=2^40)` allocated terabytes; a non-ASCII unknown
function name panicked; `cdept wave` crashed (SIGSEGV) at exit after a refused file while
the built-in viewer's thread lived; two concurrent runs of one testbench corrupted each
other's raw files (now a run lock, `.run.lock`); a family with one garbage raw sent
nothing (now the readable points go); `Xyce -v` probes hung forever and, orphaned by GUI
runs and tests, used up the inotify instances (the watch tests then failed). Open:
- [ ] GHW not tested with a real file (no GHDL here); FST tested with one Icarus file.
- [ ] Digital signals in UberWave are step traces in one pane (no bus/hex view): use
      Surfer/GTKWave for real digital debugging. Surfer is not installed here (GTKWave is).
- [ ] Tables and digital `--show` need the built-in UberWave (the process viewer cannot take
      traces without a file); markers too (not in IPC protocol 1).
- [ ] Each calculator send replaces its markers (one group); a family's markers carry the
      point in their label but are not per-point groups.
- [ ] Cursor values for an eye (folded) trace are NaN by design.
- [ ] The calculator panel's cursor section needs the built-in UberWave; with the process
      backend `subscribe` exists in the client but the GUI does not use it.
- [ ] `IpcSocketRoundTrip` (UberWave ctest) failed once when run in parallel with
      `ShimEmbedRoundTrip` on the user's desktop (cursor B read back off), passed 4 times
      alone and paired afterwards; not reproduced.
- [x] `AsciiMultiPlotConvert`, `Wp2CompatTests` (UberWave ctest) failed only without the
  spicelib submodule's raw files: with `git submodule update --init wp2-parser/spicelib` they
  pass; without it CMake leaves them out (UberWave 6375c11, which also removes the
  `[Task Diagnostic]` message and adds the missing third-party notices; `scripts/install.sh`
  installs them under `share/doc/cdept/uberwave/`).
- [ ] `layout-checks.script` fails at a45461f too (not looked into, not Phase 4/5).
- [ ] Versions: the worktree build cannot run vergen's git probe (symlinked submodules), so
      `cdept --version` shows defaults there.
- [ ] No pss analysis: the installed ngspice 47 has none ("no such command").
- [ ] The probes of the other worktree (agent-ae8c...) left `Xyce -v` processes too (built
      before the probe fix); not touched by this agent.
- [ ] `the_project_files_are_watched` failed once under full-suite load (passes alone).

---

### Own waveform viewer, steps 1 and 2 (2026-11-27, ROADMAP Phase 5 decision)
What exists:
- UberWave repository (`cdept-uberwave/uberwave`, branch main): the data engine
  `wp3-data-engine` builds alone (`cmake -S wp3-data-engine -DWP3_BUILD_TOOLS=OFF`, or the
  top level with `-DUBERWAVE_BUILD_VIEWER=OFF`); a configure of either touches no OpenGL,
  GTK, GLFW, ImGui, ImPlot or FetchContent (checked in CMakeCache/configure log). Target
  `uwdata`: C interface `wp3-data-engine/capi/include/uwdata.h` (opaque `uwd_dataset`,
  a mutex per dataset, no process-wide state, no exceptions across it, flock on the source
  while its cache is written so concurrent openers wait and reuse it, cache beside the file
  or in a fallback folder, `uwd_source_changed`). Tests: `UwdataTest` (C smoke + torture),
  `NumberParseTest` (decimal-comma locale `en_DK.utf8` is installed here and used).
  No `setlocale` any more; numbers parsed by `wp3/util/NumberParse.hpp` (from_chars).
- `cdept-wavedata`: Rust binding (`build.rs` builds only `uwdata` with cmake + a C++20
  compiler; stub otherwise or with `CDEPT_WAVEDATA_STUB=1`); `examples/wavebench.rs`
  measures a file.
- `cdept-gui/src/views/wave/`: `Tab::Wave(id)`, any number of tabs (also on one file),
  dockable/detachable like the others, not restored after a restart; open thread +
  query thread per tab (the UI thread never calls the engine); egui_plot 0.37 for axes and
  grid (own 1-2-5 grid spacer); File > Open Waveform File…, "Waveform Tab" per point in
  the Simulation tab, `wv …` script verbs (docs/GUI-TESTING.md), `cdept-gui/scripts/wave/run.sh`.
- The embedded UberWave (`cdept-uberwave`) is untouched and still built. It stays: UberWave's viewer remains
  an option that opens in a separate window (decided 2026-11-27).

Open points / risks (mine):
- Engine x lookups assume an increasing x (binary search): a reversed DC sweep queries wrong. (Step 4: parts.)
- PSF result folders are refused by `uwd_open` (the engine can read them; not wired). (Step 4: wired.)
- ascii AC (complex) raw files are refused by the engine's ascii converter. (Step 4: read.)
- No checksum in `.uwraw`: flipped data bytes in a cache with fresh stamps are not detected
  (table corruption and truncation are, and the cache is rebuilt).
- A `.uwraw` truncated in place while mapped would SIGBUS; the engine's own writer only
  renames, so only another program doing that triggers it.
- Every pyramid build maps the cache again: RSS (not memory) jumps transiently (1.5-1.8 GB
  RSS for 58 pyramids of the SAR file, cgroup peak stays ~0.6 GB); malloc keeps ~60-130 MB
  anon after closing (arenas of the build threads).
- `UwRawWriter::finalize` sleeps 50 ms (a Windows workaround): every conversion >= 50 ms.
- Drawing: in a debug build 25 traces stutter (55 ms frames); release is fine (p50 1.8 ms,
  p95 10-12 ms, max 31-36 ms). A detached second window: frames 25 ms (vsync of the second
  viewport, I think) and one 3.4 s frame when the window is created (not investigated;
  maybe general to detaching).
- The legend of egui_plot covers the plot with many traces; CJK names show as boxes
  (egui's default fonts).
- `wv` mouse commands assume no schematic/layout is the script's document.

### Own waveform viewer, fixes and step 3 (2026-11-27, SYMBI-89)
What exists (docs/howto/view-waveforms.md, GUI-TESTING.md):
- The "3.4 s freeze" after `wv detach` was no frame: the tab measured the time between
  two of its own draws, and the tab behind the detached one had been hidden 3.4 s. A
  frame clock in the app (`cdept::frame` at debug level, `CDEPT_SLOW_FRAME_MS`) saw no
  frame over 42 ms in the whole SAR run, detach included; detaching the Console, Jobs or
  a small waveform tab costs nothing visible. The 25-30 ms frames after it were egui
  tessellating the 25 dense traces of the tab that came to the front (30 ms CPU per
  frame in release too), now gone: traces are drawn on the query thread into one image
  per pane (`raster.rs`).
- CI job `uberwave`: real data engine and real built-in UberWave from the public
  submodule (`scripts/ci/uberwave.sh`; fails on a stub: build-log warnings, `cdept tools`
  rows, `CDEPT_REQUIRE_ENGINES=1` tests). Emulated in a clean clone with `env -i`, an
  empty HOME and a fresh CARGO_HOME, the display tests on a headless `Xvnc` (no xvfb
  here): all green. Not run on GitHub yet.
- File › Open in UberWave… (both backends, errors as a dialog).
- Step 3: sources (files, families, memory series `cdept_wavedata::Series`), panes with
  shared x per unit, signal list (search, groups, selection, drag), per-point show/hide,
  styles, legend above the pane, Send to tab, Waveform Tab (family), markers, cursor
  outputs following a tab.

Measured (release, 1600x1000 window, this desktop):
- SAR file (845 MB), 26 lines: UI frame p50 0.3-0.9 ms, drawing on the worker p95
  14-23 ms per view change, query p95 1-9 ms with pyramids.
- Family of 600 points (2000 samples each, 15 broken): the first open wrote 585
  caches at about 50 ms each, 4 at a time, ~8 s (2000 points: 25 s), almost all of it
  the 50 ms sleep of `UwRawWriter::finalize` (a Windows workaround); without it outside
  Windows (UberWave 91e034d) 2000 points open in 0.3 s. With 590 /
  1180 / 1770 lines: frame p95 2.6 / 4.6 / 10 ms, drawing 100-180 ms per view change,
  cgroup memory 215-345 MiB. 2000 points: frame p95 9-15 ms, drawing 260-430 ms,
  cgroup peak 491 MiB. That is the limit I would state: up to ~2000 points stay usable;
  each view change then takes about half a second to redraw (the old image moves
  meanwhile).
- 60 opens and closes of a 200-file family: RSS 144-148 MiB, cgroup 75-80 MiB, flat.
- 300 panes (300 signals of 20000 samples): frame p50 2 ms (only visible panes drawn).

Open points / risks (mine):
- [x] Log x axes are wired (an AC file's `uwd_x_log`, `XGroup::log`, a decade grid) but
      untested: the engine refuses ascii AC files and no binary AC file was tried.
      Complex data comes from the calculator as two real traces only. (Step 4: tested.)
- [ ] Drawing cost grows with lines × pixels touched; 2000 dense lines take ~0.4 s per
      redraw. No level-of-detail for families (e.g. an envelope of all points).
- [x] A family lists every point folder of the view, also points left from an earlier
      run plan (the UberWave family does the same). (Step 4: the tab leaves them out
      when the results store knows the newest plan; UberWave's family unchanged.)
- [ ] Each open file keeps its dataset (and file descriptor/mapping) while the tab lives;
      the fd limit here is 524288, a desktop with 1024 would fail opens beyond ~1000.
- [ ] Selection order in the list is alphabetical (New Pane puts them in that order).
- [ ] CJK characters show as boxes (egui's default fonts), as before.
- [ ] `wv` mouse commands and `drag_signal` need the rows/panes on screen.
- [x] Sending to a tab gives the series to the source of the same tag; a new tab made by
      Send to tab shows only the calculator traces (not the testbench defaults). (Step 4.)
- [x] The Bode and eye layouts of the calculator panel still go to UberWave only. (Step 4.)
- [ ] `cdept wave` has no way to open a tab (not required; default unchanged until step 5).
- [x] Natural sorting of points reads `vdd1p2` after `vdd1p08`. (Step 4: by value.)

### Own waveform viewer, step 4 (2026-11-27, SYMBI-89)
What exists (docs/howto/view-waveforms.md "Analyses", GUI-TESTING.md `wv` verbs):
- UberWave cee2f7d, 141e76e, 222b749 (data engine, C interface API 2): ascii complex raw
  files (ngspice `re,im`, Xyce `re, im`; refused before), every complex form (re, im,
  mag, phase, uphase = unwrapped in sweep order, dB20, dB10) in a cache of its own
  (`<stem>.cx.uwraw`; UberWave keeps its four forms in `<stem>.uwraw`), the plots after
  the first of an ascii file of several (failed before: a parse ran into the next
  plot's header), DC plot names typed `tran` before ("DC transfer characteristic"),
  V/√Hz for ngspice's noise densities, PSF folders and files through `uwd_open`
  (`<folder>.uwcache/<analysis>[.all].uwraw`), `uwd_list_analyses`, monotonic parts of
  x (`uwd_segment*`), log x for noise. Converter versions raised: old caches are made
  again once (UberWave's own viewer too).
- Tabs: AC forms (toolbar AC menu, legend menu), Bode with margins (legend menu; and
  the calculator's Bode layout with Send to tab), eye diagrams as lines or density
  (legend menu; calculator's eye layout), log y and integrated noise, op tables, the
  Analysis menu, sweeps that go back drawn part by part with a cursor row per part,
  File › Open PSF Results Folder…, small items (Send-to-tab tabs get the testbench
  defaults too, points sorted by value, leftover point folders left out, a bigger
  cursor table).

Checked against numbers:
- RC low pass, ngspice binary/ascii, Xyce binary/ascii, VACASK: dB within 3e-8 dB,
  phase within 1e-7° of the closed form (Xyce's ascii has 9 digits), −3.0101 dB at fc
  (cdept-wavedata/tests/analyses.rs).
- Three-pole loop (A0 1e4, poles 1k/1M/20M Hz) through the tab's Bode markers and
  ngspice's `meas`: unity gain 3.0657 MHz both, PM 9.4° vs 9.372°, GM 6.5 dB vs 6.454 dB
  at 4.4746 MHz both; a unit test checks the markers against the closed form (PM within
  0.2°, GM within 0.1 dB).
- The OTA's open-loop AC (cells/ota5t ac_open, tt_27) sent to a tab with the Bode
  layout: the tab's marker PM 59.4°, the calculator's `phase_margin` 59.378°
  (wave-tabs.script).
- Noise: √∫S²df of ngspice's onoise_spectrum 2.0378 µV vs its onoise_total 2.0350 µV
  (0.14 %, trapezoids on 20 points/decade); VACASK's onoise at 1 Hz is 4kTR within 2 %.
- Parametric DC (3 sweeps, V1 1.8 → 0): v(out) = (2 V1 + V2)/3 per part at the cursor;
  the op table 0.93333 V.
- PSF fixtures (psf-parser binary/ascii/psfxl, inverter_test.raw, spectre-examples):
  every plottable analysis opens, |re + j im| equals mag, caches reused.

Measured (release, this desktop, page cache warm):
- 100 M points, 2 signals (2.4 GB raw): first open with the cache written 1.2-1.6 s,
  reopen 0.33 s (0.83 s before the x scan read the chunks), RSS after drawing 2.1 GB
  (file-backed), anon 40-53 MB, cgroup 2.4 GB; first full view before the pyramids
  0.8 s, then pyramid queries p95 7 ms, max 55 ms; drawing p95 2-5 ms; UI frames p95
  0.5-0.8 ms. The one-sample spike is drawn at every zoom, `check_extremes` exact.
- 500 M points (12 GB raw + 12 GB cache, both deleted afterwards): first open 9.5 s,
  reopen 2.0 s, first full view before the pyramids 5.6-6.7 s (on the query thread; UI
  frames stayed under 7 ms), zoomed queries p95 12-16 ms, max 252 ms; cgroup held at its
  12 GB cap (page cache), peak RSS reported 21-22 GB (mapped file pages), anon < 70 MB.
- Eye of 192 000 samples (3000 traces): 13-20 ms drawing p50, 145-184 ms p95.

Found and fixed under torture: the multi-plot ascii parse (above); a half complex pair
is a format error (it was read as two reals); DC typed tran; an eye period of 1e-20 s is
refused before any work (no allocation: periods counted from the span); uwd_source_changed
waited for an 800 ms query (UI stalls when a script asked); `expect_failed` with another
message waits its whole patience (600 s) instead of failing (script driver, not changed).

Open points / risks (mine):
- [ ] The first full view of a very large file waits for a min/max over all samples
      until the pyramids exist (0.8 s at 100 M, 6 s at 500 M points): no coarser preview
      meanwhile.
- [ ] The parts of a parametric sweep are drawn in one colour (the cursor table names
      them `sweep k`); the sweep values of the outer variable are not in a raw file, so
      the parts are numbered, not labelled `V2=1`.
- [ ] A family of files of different analyses shows the catalogue of whichever file
      opened first; a complex signal of one file and a real one of the same name in
      another merge as real (no AC forms). Lines go only to panes of their x unit.
- [ ] Ascii raw files cut short are refused ("Incomplete conversion"), binary ones open
      with the points that are there (header counts held to the file size): as before.
- [ ] Unwrapping cannot follow a phase that moves more than 180° between two points
      (a long delay sampled coarsely): it is wrapped there, as any unwrapper would.
- [ ] Bode margins use CDePT's measures as they are: the loop gain must start near 0°
      (pass `-v(out)/v(in)` for an inverting amplifier); markers are recomputed only
      when Bode is chosen again.
- [ ] PSF: sweep containers, Monte Carlo families and non-swept op data of a PSF folder
      are listed as not plottable; a non-monotonic PSF sweep is refused by the
      engine's PSF converter (raw files are split into parts instead). A PSF conversion
      cancel was not tested mid-way (the fixtures convert in milliseconds).
- [ ] VACASK's raw files say `notype`: their signals have no unit in the tab.
- [ ] CSV tables and digital files are not in tabs (UberWave's viewer, Surfer/GTKWave).
- [ ] `wv` text arguments have no quoting (no spaces in `expect_marker` texts).
- [ ] Not mine, seen while testing: `ota.script` failed twice at `sim suite_wait` (90
      runs not done in 600 s) while other work on this desktop held the load average at
      200-300; and when the GUI quits while a suite runs, its ngspice processes keep
      running as orphans (23 of them, in a deleted scratch folder; killed by hand).

Before the tabs can be the default (step 5): the Simulation tab's "Open in UberWave" and
"Open family" buttons and `cdept wave` to open tabs (with UberWave's window as the
option), the calculator panel's default send target, restoring tabs after a restart
(they are not restored now), the open points above that a user meets first (preview
before the pyramids, parametric sweep colours/labels, mixed families), the CJK font
(boxes), and a look by the user at the Bode/eye/noise screens.

## 2. Things that may cause trouble

### 2.1 Most likely to bite soon
- **xschem expects a symbol and its schematic side by side (confirmed 2026-11-17).** To
  descend into an instance, xschem takes the symbol path and swaps `.sym` for `.sch`,
  unless the symbol or the instance carries a `schematic=` attribute. Our `cells`
  layout puts them in `cell/symbol/cell.sym` and `cell/schematic/cell.sch`, so every
  symbol there needs the attribute (XSCHEM.md §7); without it xschem netlists an
  **empty** `.subckt`, with no error. CDePT keeps the attribute right for what goes
  through it. It breaks when files are moved behind its back:
  - a **library folder renamed or moved** to a folder with another name: every symbol
    of the library names the old folder;
  - a cell folder copied with `cp` into another library: the copy still descends into
    the **original** schematic, silently;
  - symbols imported from another project have no attribute.

  `cdept xschem link` finds all three and `--fix` repairs them. `cdept netlist` only
  prints a one-line warning.
- **Two libraries whose folders have the same name** (`a/analog`, `b/analog`) cannot be
  told apart by xschem: references and `schematic=` values read `analog/...` and the
  first folder on the path wins. CDePT warns (`hierarchy::shadowed_libraries`; read-only
  PDK libraries are left out of the warning).
- **How instances name symbols (confirmed).** A reference is a path below one of the
  folders on `XSCHEM_LIBRARY_PATH`. CDePT puts the parent of each library folder on
  that path, so references read `<library folder>/<cell>.sym` (flat) or
  `<library folder>/<cell>/<view>/<cell>.sym` (cells). The library *folder* name is
  used, not the name in `cdept.toml`; they can differ. "Rewrite references" has to
  rewrite exactly these strings. Two libraries whose folders have the same name under
  different parents would shadow each other.
- **Template header is behind the engine.** New files are written with
  `xschem version=3.4.5 file_version=1.2`; the engine is 3.4.8RC with file version 1.3.
  xschem reads the old header fine and rewrites it on save.
- **Folder dialog inside containers.** `rfd` uses the XDG desktop portal. The asic-all
  image and bare X sessions usually have no portal, so the dialog may never appear and
  New/Open Project silently does nothing. The CLI (`cdept init`, `cdept -p`) still works.
- **Renames outside CDePT** leave stale references; renames through CDePT rewrite them
  (`refactor.rs`). The hierarchy gaps of §1 apply.
- **Variables missing in the GUI's environment.** Solved for `PDK_ROOT`/`PDK` when the
  project sets `technology` and `pdk_root`; any other `$VAR` in `cdept.toml` still comes
  from the environment the app was started in.

- **Embedded xschem (see XSCHEM.md §5).** The helper is GPL-2.0-or-later; `libcdept`,
  `cdept-gui` and `cdept` must never depend on the `cdept-xschem` crate, only start the
  program. Steps 2–5 remain the risk: xschem's dialogs block the caller, its state is
  global, and `xschem.tcl` expects Tk when drawing is on. Estimated 130–160 h in total,
  more than the 110 h Phase 3 had for all tools.
- **The helper needs its build tools everywhere it is built**: flex, bison, Tcl and
  cairo development files, and `git submodule update --init` after cloning. A plain
  `cargo build` skips it (not a default member), `cargo build --workspace` does not.
- **xschem's messages are hidden by default.** They go to the log at debug level
  (`cdept -v`). A netlist with unresolved symbols fails with "netlisting errors" and
  the reason is only visible there.

### 2.2 Data model and format
- **Layout detection can flip.** With no `layout` key and no `lib.toml`, a library is
  `flat` if design files sit directly in its folder. Dropping a `.sch` into the root of a
  hand-made `cells` library turns it flat. Libraries created by CDePT have a `lib.toml`,
  which pins `cells`. Consider always writing `layout` explicitly.
- **Suffix guessing.** `.lib` is taken as Liberty, `.sch` is xschem unless the file
  starts with `<Qucs`, `.sp/.cir/.ckt` are SPICE. Unknown suffixes are ignored, so a cell
  may look empty although its folder has files. No way yet to override per project.
- **Names.** Dots in cell names are accepted; in a flat library `a.b.sch` is cell `a.b`,
  while `a.gds.gz` is cell `a`. Names differing only by case collide on case-insensitive
  filesystems. File names that are not UTF-8 are skipped.
- **Duplicate library names**: the first definition wins, the later one becomes a warning
  in the Console. Easy to miss.
- **`cdept.toml` is the only schema so far** and has no version field. Add one before
  the format changes incompatibly.
- **Multi-cell GDS** is one cell, so the layout view of a standard cell cannot be
  selected individually yet.
- **Ghost references in stock KLayout.** A layout view opened outside CDePT shows its
  placed cells empty (they are references only). For DRC, LVS and tape-out use
  `cdept layout export`. A copy made with `cp` keeps the old sidecar identity;
  `cdept layout refs` reports it.
- **Structure names are global per stream.** Two views whose cells share a name are
  told apart by the sidecar. When the cells of two files clash inside one layout, the
  helper renames the external copy in memory, so an exported stream can carry such
  renamed cells (not yet checked against a real clash).

### 2.3 File operations
- **Delete** goes to the trash since 2026-11-22. Rename and move have a rollback; copy
  works file by file, so a failure in the middle can leave a partial copy **(verify)**.
- **Copying a cell copies everything in its folder**, including simulation results once
  testbenches exist (possibly gigabytes). Needs an exclusion rule in Phase 4.
- **Saving `cdept.toml`** goes through a temporary file and a rename: a symlinked project
  file is replaced by a regular file, and file permissions may change.
- **Symlinked paths.** The project root is canonicalised but picked library folders are
  not, so a library inside the project reached through a symlink may be written as an
  absolute path.
- **External edits while the GUI is open.** File watching reloads them (2026-11-22); see
  the auto-reload item in §1.

### 2.4 GUI
- **Scanning runs on the UI thread**, and every operation rescans all libraries. Fine
  locally (47 cells in `sg13g2_pr` is instant); may stall with large PDKs, many
  libraries or network filesystems. Each `.sch` is opened to tell xschem from Qucs.
  Move scanning to a background thread, or rescan only the touched library.
- **Folder dialog thread.** It runs on a background thread, which Linux accepts; macOS
  and some Windows setups require the main thread.
- **Dialogs opened from the folder dialog** always appear on the main window, even if
  the request came from a detached one. One dialog at a time.
- **Selection is by name.** After Reload a renamed item is simply deselected.
- **Recent projects** are never pruned; a deleted project shows an error dialog when
  clicked.
- **Tree panel** draws every cell of an expanded library (no row virtualisation, unlike
  the Library Manager's cell column).

### 2.5 Dependencies
- `egui_dock` has no release for egui 0.37, so we stay on egui/eframe 0.36.
- `rfd` pulls the portal stack (`ashpd`, `zbus`); watch build time and the MSRV.
- xschem (as a program) is not installed on the development host; KLayout is built in `cdept-klayout/.bin`. Templates were checked
  in the asic-all image (`docker run --entrypoint bash asic-all:dev-amd64-new`); the
  host-built `cdept` binary cannot run in that image (glibc too old there).

### 2.6 Standing risks from the roadmap
See ROADMAP §4: per-tool IPC (Phase 3), PDK heterogeneity, large waveforms, scope creep.

### 2.7 ngspice in-process (2026-11-23)
`libcdept::ngspice` loads libngspice with dlopen (`$CDEPT_LIBNGSPICE`), one Session at a time; `cdept ngspice live`; the testbench backend `simulator_backend = "inproc"` (`ngspice::backend`, SYMBI-71) runs points on one worker thread that owns the library. It changes the process working directory during a point and keeps ngspice settings between points; a library crash takes CDePT down. See [NGSPICE.md](NGSPICE.md).
- Magic batch adapter (`libcdept/src/magic/`, `cdept magic extract|drc|lef`, docs/MAGIC.md): extraction, DRC, LEF, GDS import through Tcl on stdin; real tests need Magic and a PDK in /opt/ext and skip otherwise.
- Sweeps, Monte Carlo and results store (`libcdept/src/sim/results.rs`, docs/SIMULATION.md 4-6): `[[sweep]]` variables cross with corners x temperatures, `[montecarlo]` makes seeded `mc` points (seed via `setseed`+`reset` in the control block; `.spiceinit` rndseed does not work), `result.json` per run and `simulations/results.db` (rusqlite bundled); `cdept sim results`.
- Simulation tab (`cdept-gui/src/views/simulation.rs`, 2026-11-23): setup.toml form + Run/Cancel on `sim::RunOptions { queue, cancel }` + jobs list + results table; one tab, one testbench at a time; Jobs panel still a placeholder. See SIMULATION.md §5, `scripts/simulation.script`.
- Qucs-S (`libcdept/src/batch/qucs.rs`, `cdept qucs export|open|spar`, docs/QUCS.md): `qucs-s -n` exports a SPICE netlist headless (exit status is 0 even on failure, so errors are read from the output); `schematic_kind = "qucs"` in a testbench's setup.toml runs it through the normal ngspice/Xyce path (`examples/ihp-inv` cell `rc_lowpass`); "Open in Qucs-S" in the Library Manager; IHP PDK `Lib` parts did not export headless (open).
- Calculator (`libcdept/src/calc/`, `libcdept/src/raw/`, `cdept calc`, docs/CALC.md, 2026-11-23): SPICE raw reader (ngspice/Xyce, binary/ascii, multi-plot, complex), waveform function library (cross/delay/rise/slew/overshoot/settling/frequency/duty, bandwidth/UGBW/margins, own FFT with SNDR/SNR/SFDR/THD/ENOB, noise, INL/DNL), own expression language; `[[output]]` in setup.toml evaluated after each run, stored as measures flagged in `RunRecord.calc`.
- Monte Carlo modes and Xyce (2026-11-23, `libcdept/src/sim/mc.rs`, `stats.rs`, SIMULATION.md 5/5b): `[montecarlo] mode = process|mismatch|both` (IHP `mc` / `tt_mm` / hand-written `mc_mm` in the example tech.toml; sky130 `mc` / `tt_mm` / `mc` + `.param mc_mm_switch=1`); Xyce MC = one `.SAMPLING useExpr=true` sample per run + `-randseed`, Xyce runs now pass the model set's `-plugin` list (via `CDEPT_XYCE_ARGS` in the run env); IHP Xyce stat files lack `*_vfbo_mm` params (worked around in the `xyce_mc` view); `cdept sim results --stats`, GUI sweeps/Monte Carlo form, Run N MC, statistics block.
- VACASK + OpenVAF (`libcdept/src/vacask/`, `batch/vacask.rs`, `batch/openvaf.rs`, `cdept vacask models`, `cdept openvaf compile`, docs/VACASK.md, 2026-11-23): `simulator = "vacask"`; the testbench stays SPICE (xschem) and is translated by our own `vacask::translate`; the PDKs have no VACASK models, so `[models.vacask]` (`generated = ...`) is made from the ngspice ones with VACASK's ng2vc (python, separate process) and the Verilog-A compiled by OpenVAF-r into `<project>/.cdept/osdi/<pdk>-<hash>/`; ihp-sg13g2 tb_inv tpd matches ngspice to 0.2 %; sky130 does not convert yet. vacask needs `<prefix>/lib` on LD_LIBRARY_PATH (added by the adapter). Measurements on the raw file: small own subset (`vacask::meas`).
- VACASK at ngspice's level (SYMBI-78, 2026-11-24, docs/VACASK.md 5 and 6): Monte Carlo with CDePT-side draws (`vacask/stat.rs`: `convert.py` turns `gauss()`/`agauss()` into reads of `mcz[slot]` / `mmz[mmk*mmnz+slot]`; the deck defines the vectors from a counter-based generator of the run seed; `instrument` gives every device of the flattened testbench its `mmk`; "first definition of a subcircuit wins" for `mc_mm`), `--simulator` on `sim run|deck|suite` + suite `simulator` + `Setup::set_simulator`, `sim/caps.rs` (`SimKind::capabilities()`, refusals at load), cancel kills VACASK, noise spectra added to the raw copy, `@n.x.y.dev[var]` saves and calculator names mapped (`i()` also `:flow(br)`, `a.b` is `a:b`), instance names looked up case-blind, `models::ready` serialised (suites asked for the set from six threads at once). sky130: ng2vc has no `M` cards, binned models or BSIM4 (bsim4v8.osdi exists); not converted. Real tests: `libcdept/tests/real_vacask.rs`; GUI script `cdept-gui/scripts/vacask.script`. For the Simulation tab form: Monte Carlo is no longer refused with VACASK (`SimKind::capabilities().mc_modes`).
- UberWave built in (SYMBI-83, 2026-11-24, docs/howto/view-waveforms.md): crate `cdept-uberwave` (`build.rs`: `$UBERWAVE_SRC` or sibling `../uberwave` -> cmake -> `uberwave_embed`, link line read from the probe's `link.txt`; any failure = warning + stub, `cfg(uberwave_embedded)` otherwise); the viewer window runs on its own thread (checked with winit on Wayland and X11, GLFW on either); `libcdept::wave::{backend, embedded}` (`Backend::Embedded|Process`, `ViewerOps` trait implemented by the IPC `Client` and the shim; `--backend`, `CDEPT_UBERWAVE_BACKEND`, `[wave] backend`); `cdept wave` blocks until the window closes, `--screenshot PNG` is the hidden-window variant. Open points: one viewer window per process (GLFW global state) so two project runs share it; Wayland has no title bar when `libdecor` is missing; the UberWave side is the branch `ipc-multifile` of the `uberwave` repository (`wp5-viewer/shim`, `Docs/Embedding.md`) and must be merged for the sibling-source lookup; a dedicated CI job with the private source is not set up (CI builds the stub); the GUI never closes the viewer on exit (the process end does); cursor events/`subscribe` are not exposed by the shim.
- Time limits, OpenVAF and VACASK availability (SYMBI-80, 2026-11-24): `timeout` in `setup.toml` / `[sim] timeout` (30m) / `--timeout` for every simulator run point (`sim::run::run_in_job_limited`, `JobCtx::run_command_limited`, `Watch.timeout`, in-process watchdog + `bg_halt`; message `run point exceeded its time limit of 10m` + log tail), `batch::{Cancelled, TimedOut}` typed errors, `openvaf::compile_watched` + `[sim] openvaf_timeout` (20m), `models::{generate_watched, ready_watched, check_libraries}` (a missing OpenVAF is a clear error and is no longer cached as a finished generation), `vacask::availability` (`vacask/avail.rs`; binned BSIM3/BSIM4 `.model` cards = sky130 unavailable), used by `sim run|suite|deck`, `cdept tools`, `cdept vacask models` and the Simulation tab (`views/simulation/avail.rs`, script `vacask-unavailable.script`). Tests: `libcdept/tests/vacask_limits.rs` (hermetic fake programs), `real_vacask_avail.rs`.
- Test templates and suites (`libcdept/src/sim/template.rs` + `templates/*.toml`, `starter.rs`, `suite.rs`, `cdept sim templates|new|suite`, `cdept-gui/src/views/testlib.rs`, docs/TESTS.md, 2026-11-23): `template = "ac_response"` + `[template_args]` in setup.toml expands a built-in / `pdk/<tech>/templates` / `<project>/templates` template (typed params, derived values, `when`, specs from `*_min/_max` params, required nets checked against the netlist); `suite.toml` views run several testbenches on one queue into one table (`suite.json`, `suite_runs` in results.db, exit status). The noise deck now writes both noise plots. Example: `examples/ihp-inv` (tb_inv tran_delay/dc_transfer/power/suite, tb_inv_ac); `scripts/tests.script`.
