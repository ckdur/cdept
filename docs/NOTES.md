# CDePT: working notes

What is still to do and what may cause trouble. [ROADMAP.md](ROADMAP.md) is the plan; this
file is the running list behind it. Update it when an item is done or a new problem shows
up. Items marked **(verify)** are suspicions, not confirmed facts.

Last updated: 2026-11-20 (embedded KLayout: steps 0–3 built).

---

## 1. To do

### Before building further on Phase 1
- [ ] Try the GUI by hand. Only headless drawing tests and a 6 s launch were done. Not
      exercised: the native folder dialog (New/Open Project, Add Existing Library),
      right-click menus, every dialog, detached windows with a dialog open.
- [x] Let CI run once: `rfd` builds on the runner and with the MSRV toolchain (CI run
      of fd11b7b, 2026-11-18).

### Phase 1, remaining (suggested order)
1. [ ] **xschem and GDS readers** (instances in `.sch`, structures and references in GDS).
       Everything below in this group depends on them.
2. [ ] **Hierarchy browser**: tree of a cell, "used by" query. List the structures inside
       a multi-cell GDS (a standard-cell GDS is one cell today).
3. [ ] **Rewrite references on rename/copy**: instances in other `.sch` files, the
       structure name inside the GDS, the `.subckt`/`module` name in netlist and HDL views.
4. [ ] **File watching** (`notify`) so external edits show up without Reload.
5. [ ] **Git status** per cell (and decide what Delete does in a git repo, see §2.3).
6. [ ] **Categories** and search across libraries.
7. [ ] **Import** an existing xschem/KLayout project (scan → `[libraries]` table).
8. [ ] **LibMan**: import its `define(name, path)` list, read `Category.group`, "open in
       LibMan".

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
      cells, selection, status, notices; 15 protocol tests (`cdept-klayout/tests/run.sh`).
      1920×1080: ~17 ms per change; 4K: ~75–85 ms (whole frame each time).
- [x] GUI: layout document tab (toolbar, Layers/Cells/Selection panels, Place Cell…,
      export, close flow, Inspector preview, script commands; `layout-smoke.script`).
- [ ] Not done: properties dialog, PCell parameter dialog (KLayout's are Qt), editing
      inside external cells (open the view instead), DRC/LVS, OASIS references (no
      reader: OASIS views are leaves), `.gds.gz` (no reader), renames do not yet
      rewrite *other* views that reference the renamed cell.
- [ ] Press-drag-release does not draw a box in box mode; two clicks do (as in KLayout?).
- [ ] KLayout is not built in CI; its tests skip there. `install.sh` installs the helper
      script only, KLayout comes from the system or `cdept-klayout/.bin`.
- [ ] Try it by hand.

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
- [ ] Hierarchy panel is still a placeholder.

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
