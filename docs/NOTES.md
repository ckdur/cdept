# CDePT: working notes

What is still to do and what may cause trouble. [ROADMAP.md](ROADMAP.md) is the plan; this
file is the running list behind it. Update it when an item is done or a new problem shows
up. Items marked **(verify)** are suspicions, not confirmed facts.

Last updated: 2026-11-13 (after the first step of Phase 1).

---

## 1. To do

### Before building further on Phase 1
- [ ] Try the GUI by hand. Only headless drawing tests and a 6 s launch were done. Not
      exercised: the native folder dialog (New/Open Project, Add Existing Library),
      right-click menus, every dialog, detached windows with a dialog open.
- [ ] Let CI run once: the new dependency `rfd` has not been built on the CI runner or
      with the MSRV toolchain (its declared minimum, 1.92, is below ours).

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

### Smaller gaps left in what exists
- [ ] Edit project fields (name, description, technology) from the GUI; today by hand.
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
- **xschem expects a symbol and its schematic side by side (verify).** To descend into
  an instance, xschem takes the symbol path and swaps `.sym` for `.sch`, unless the
  symbol carries a `schematic=` attribute. Our `cells` layout puts them in
  `cell/symbol/cell.sym` and `cell/schematic/cell.sch`, so descend may fail. Options:
  keep both in one folder, write `schematic=` into the symbol template, or generate the
  search path in the xschemrc (Phase 2). Decide before people create real designs in the
  `cells` layout, because changing the layout later means moving files.
- **How instances name symbols (verify).** xschem stores instance references as paths
  resolved through `XSCHEM_LIBRARY_PATH` (e.g. `sg13g2_pr/nmos.sym`). In the `cells`
  layout the reference becomes `lib/cell/symbol/cell.sym` or needs one search-path entry
  per cell. This decides what "rewrite references" has to rewrite.
- **Folder dialog inside containers.** `rfd` uses the XDG desktop portal. The asic-all
  image and bare X sessions usually have no portal, so the dialog may never appear and
  New/Open Project silently does nothing. The CLI (`cdept init`, `cdept -p`) still works.
- **Rename and copy leave stale references.** Until item 3 above, renaming a cell that
  others instantiate breaks those schematics, and a renamed layout keeps its old
  structure name inside the GDS.
- **Variables missing in the GUI's environment.** `$PDK_ROOT` set in a shell rc is not
  set when the app starts from a desktop launcher; such libraries show an error. Phase 2
  should resolve PDK paths from the technology instead of the environment.

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
- xschem and KLayout are not installed on the development host. Templates were checked
  in the asic-all image (`docker run --entrypoint bash asic-all:dev-amd64-new`); the
  host-built `cdept` binary cannot run in that image (glibc too old there).

### 2.6 Standing risks from the roadmap
See ROADMAP §4: per-tool IPC (Phase 3), PDK heterogeneity, large waveforms, scope creep.
