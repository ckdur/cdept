# CDePT: working notes

What is still to do and what may cause trouble. [ROADMAP.md](ROADMAP.md) is the plan; this
file is the running list behind it. Update it when an item is done or a new problem shows
up. Items marked **(verify)** are suspicions, not confirmed facts.

Last updated: 2026-11-23 (Phase 1 and 2 built and merged into the docs; tests re-run green).

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
- [ ] Dead code: old New Project folder flow (`PickFor::NewProject`); `Library::rename_cell`,
      `copy_cell`, `rename_cell_following`.
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

### Phase 4 (first slice, 2026-11-23, [SIMULATION.md](SIMULATION.md))
`cdept sim run|deck`, `setup.toml` testbench view, deck builder, ngspice batch, specs. Open: see
SIMULATION.md §4 (GUI, job queue, sweeps, results DB). `libcdept/tests/sim.rs` needs
`CDEPT_XSCHEM` and `CDEPT_NGSPICE` set to run (else it skips).
Job queue / process supervisor: `libcdept::jobs` ([JOBS.md](JOBS.md)); `sim::run_testbench` runs points in parallel, `cdept sim run --jobs N`. Open: SLURM/ssh, results DB, process-group kill.

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
- [ ] No library rename in CDePT yet (`layout::follow::library_renamed` is ready for it);
      xschem instances are not rewritten when a cell is renamed.
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
      `libcdept/tests/nets.rs`, helper `test_nets.py` (generic deck, no PDK),
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

### Smaller gaps left in what exists
- [x] Edit project fields (name, description, technology, PDK folder) from the GUI.
- [ ] Edit or remove libraries that come from an `include` file; today the GUI refuses.
- [ ] Change a library definition (path, read-only, layout) from the GUI; today remove
      and add again, or edit `cdept.toml`.
- [ ] Rename and copy a single view; rename a library.
- [ ] Magic layout template (needs the technology name, so Phase 2).
- [ ] CLI for cells and views (`cdept cell new|copy|rename|rm`); only `init`, `ls` and
      `lib add|remove` exist.
- [ ] A text field as fallback for the folder dialog (see §2.1).

### Carried to later phases
- Phase 2: GDS template units and the xschem header version should come from the
  technology, not constants. `technology` in `cdept.toml` is read but unused.
- Phase 3: double-click on a view only logs a line. `Format::tool()` already names the tool.
- Phase 4: a testbench view is recognised by a `setup.toml` or a `tb_` name. That is a
  guess until the testbench format exists.

---

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
  first folder on the path wins. Not detected yet.
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
- **Rename and copy leave stale references.** Until item 3 above, renaming a cell that
  others instantiate breaks those schematics, and a renamed layout keeps its old
  structure name inside the GDS.
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
- **Delete is permanent.** No trash, no undo, git is not consulted. There is a
  confirmation dialog, and Enter does not confirm it.
- **Operations are not atomic.** Rename moves the cell folder, then renames files one by
  one; copy works file by file. A failure in the middle (permissions, disk full) leaves a
  half-renamed cell.
- **Copying a cell copies everything in its folder**, including simulation results once
  testbenches exist (possibly gigabytes). Needs an exclusion rule in Phase 4.
- **Saving `cdept.toml`** goes through a temporary file and a rename: a symlinked project
  file is replaced by a regular file, and file permissions may change.
- **Symlinked paths.** The project root is canonicalised but picked library folders are
  not, so a library inside the project reached through a symlink may be written as an
  absolute path.
- **External edits while the GUI is open.** Saving re-reads the file first, so hand edits
  are not lost, but the views are stale until Reload (no file watching yet).

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
`libcdept::ngspice` loads libngspice with dlopen (`$CDEPT_LIBNGSPICE`), one Session at a time; `cdept ngspice live`; `run_deck` is Outcome-compatible but not wired into sim yet. See [NGSPICE.md](NGSPICE.md).
- Magic batch adapter (`libcdept/src/magic/`, `cdept magic extract|drc|lef`, docs/MAGIC.md): extraction, DRC, LEF, GDS import through Tcl on stdin; real tests need Magic and a PDK in /opt/ext and skip otherwise.
- Sweeps, Monte Carlo and results store (`libcdept/src/sim/results.rs`, docs/SIMULATION.md 4-6): `[[sweep]]` variables cross with corners x temperatures, `[montecarlo]` makes seeded `mc` points (seed via `setseed`+`reset` in the control block; `.spiceinit` rndseed does not work), `result.json` per run and `simulations/results.db` (rusqlite bundled); `cdept sim results`.
- Simulation tab (`cdept-gui/src/views/simulation.rs`, 2026-11-23): setup.toml form + Run/Cancel on `sim::RunOptions { queue, cancel }` + jobs list + results table; one tab, one testbench at a time; Jobs panel still a placeholder. See SIMULATION.md §5, `scripts/simulation.script`.
- Qucs-S (`libcdept/src/batch/qucs.rs`, `cdept qucs export|open|spar`, docs/QUCS.md): `qucs-s -n` exports a SPICE netlist headless (exit status is 0 even on failure, so errors are read from the output); `schematic_kind = "qucs"` in a testbench's setup.toml runs it through the normal ngspice/Xyce path (`examples/ihp-inv` cell `rc_lowpass`); "Open in Qucs-S" in the Library Manager; IHP PDK `Lib` parts did not export headless (open).
- Calculator (`libcdept/src/calc/`, `libcdept/src/raw/`, `cdept calc`, docs/CALC.md, 2026-11-23): SPICE raw reader (ngspice/Xyce, binary/ascii, multi-plot, complex), waveform function library (cross/delay/rise/slew/overshoot/settling/frequency/duty, bandwidth/UGBW/margins, own FFT with SNDR/SNR/SFDR/THD/ENOB, noise, INL/DNL), own expression language; `[[output]]` in setup.toml evaluated after each run, stored as measures flagged in `RunRecord.calc`.
- Monte Carlo modes and Xyce (2026-11-23, `libcdept/src/sim/mc.rs`, `stats.rs`, SIMULATION.md 5/5b): `[montecarlo] mode = process|mismatch|both` (IHP `mc` / `tt_mm` / hand-written `mc_mm` in the example tech.toml; sky130 `mc` / `tt_mm` / `mc` + `.param mc_mm_switch=1`); Xyce MC = one `.SAMPLING useExpr=true` sample per run + `-randseed`, Xyce runs now pass the model set's `-plugin` list (via `CDEPT_XYCE_ARGS` in the run env); IHP Xyce stat files lack `*_vfbo_mm` params (worked around in the `xyce_mc` view); `cdept sim results --stats`, GUI sweeps/Monte Carlo form, Run N MC, statistics block.
- VACASK + OpenVAF (`libcdept/src/vacask/`, `batch/vacask.rs`, `batch/openvaf.rs`, `cdept vacask models`, `cdept openvaf compile`, docs/VACASK.md, 2026-11-23): `simulator = "vacask"`; the testbench stays SPICE (xschem) and is translated by our own `vacask::translate`; the PDKs have no VACASK models, so `[models.vacask]` (`generated = ...`) is made from the ngspice ones with VACASK's ng2vc (python, separate process) and the Verilog-A compiled by OpenVAF-r into `<project>/.cdept/osdi/<pdk>-<hash>/`; ihp-sg13g2 tb_inv tpd matches ngspice to 0.2 %; sky130 does not convert yet. vacask needs `<prefix>/lib` on LD_LIBRARY_PATH (added by the adapter). Measurements on the raw file: small own subset (`vacask::meas`).
- Test templates and suites (`libcdept/src/sim/template.rs` + `templates/*.toml`, `starter.rs`, `suite.rs`, `cdept sim templates|new|suite`, `cdept-gui/src/views/testlib.rs`, docs/TESTS.md, 2026-11-23): `template = "ac_response"` + `[template_args]` in setup.toml expands a built-in / `pdk/<tech>/templates` / `<project>/templates` template (typed params, derived values, `when`, specs from `*_min/_max` params, required nets checked against the netlist); `suite.toml` views run several testbenches on one queue into one table (`suite.json`, `suite_runs` in results.db, exit status). The noise deck now writes both noise plots. Example: `examples/ihp-inv` (tb_inv tran_delay/dc_transfer/power/suite, tb_inv_ac); `scripts/tests.script`.
