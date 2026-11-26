# Magic

Magic is CDePT's second layout editor, next to KLayout: `.mag` views are design files of
the project's libraries, open and edit in a CDePT tab, place each other across
libraries, are made from xschem schematics with the PDK's device generators, and go
through DRC, extraction, LVS (netgen), LEF and GDS. The how-to is
[howto/magic-layout.md](howto/magic-layout.md).

Code:

| Part | Where | Licence |
|---|---|---|
| Helper program (Magic in a tab) | `cdept-magic/` (`shim/cdm.c` the Tk and X stand-ins, `tcl/cdept-magic.tcl`, `src/`) | GPL-2.0-or-later (it runs the Tk stand-in `faketk.tcl` of `cdept-xschem`) |
| `.mag` files, references, sidecars | `libcdept/src/magic/{mag,refs}.rs` | MIT |
| Tab session, start-up script | `libcdept/src/magic/embed.rs` (uses `libcdept::xschem::Session`) | MIT |
| Layout from schematic | `libcdept/src/magic/link.rs`, `pdk/<PDK>/magic-devices.toml` | MIT |
| Batch runs and flows | `libcdept/src/magic/{mod,tcl,parse,flows}.rs` | MIT |
| GUI tab | `cdept-gui/src/views/magic.rs` | MIT |
| CLI | `cdept magic …` (`cdept/src/cmd_magic.rs`), `cdept open` | MIT |

## Licence of Magic

Checked in Magic's own sources at tag 8.3.623 (the version of the asic-all image and of
`/opt/ext/OpenLane/bin/magic`): the `LICENSE` file and the file headers are the
University of California's permission notice ("Permission to use, copy, modify, and
distribute this software … is hereby granted, provided that the above copyright notice
appear in all copies"; SPDX `HPND-UC-export-US`), a permissive licence. It is **not** the
GPL, as this page said before. Exceptions in the tree: `readline/` (a copy of GNU
readline 4.3, GPL; Magic's configure leaves it out of the Tcl build, which is the one
CDePT uses), and two headers (`utils/magic_stdbool.h`, `utils/magic_alloca.h`) that quote
autoconf documentation (GFDL/GPL notes). The RPM spec template says `License: GPL`,
which does not match the sources.

CDePT still runs Magic only as a process of its own, never linked: the helper program
`cdept-magic` is GPL-2.0-or-later because it executes the xschem helper's Tk stand-in
(`cdept-xschem/shim/tcl/faketk.tcl`, part of a GPL crate). `libcdept`, `cdept-gui` and
`cdept` stay MIT and talk to it over a socket.

## Embedded Magic

### How it works

Magic's Tcl build is a shared library, `tclmagic.so`, that `magicexec` (a `wish`) loads.
It calls Tk only through Tk's stub table (`tkStubsPtr`, from `package require Tk`) and
Xlib directly. The helper is a Tcl interpreter of its own that provides what `wish` would:

- a **Tk package** whose stub table holds the helper's own functions (`shim/cdm.c`):
  windows (`Tk_CreateWindowFromPath`, `Tk_MapWindow`, `Tk_FakeWin` layout), event
  handlers, GCs, pixmaps, fonts (Magic's Cairo driver draws text with cairo), colours,
  cursors. A function Magic might call that is not there aborts with the slot number;
- an **X display**: the Xlib functions `tclmagic.so` uses are defined in the program,
  which is linked with `--export-dynamic`, so the dynamic linker binds Magic to them
  before libX11's. Windows and pixmaps are cairo image surfaces; `cairo_xlib_surface_create`
  is defined too and returns them. Magic's Cairo driver (`-d XR`) draws unchanged;
- the **Tcl side of Tk**: `faketk.tcl` of the xschem helper (records widgets, reports
  them as `UiUpdate`s), so Tk dialogs of Magic's or the PDK's Tcl show as CDePT dialogs.

Magic is started as `magic.tcl` would: `-nowrapper -noconsole -nowindow -d XR -rcfile
<the PDK's magicrc>`, then one layout window is opened without Magic's caption and scroll
bars (CDePT has its own tool bar, command line and messages). Nothing of Magic is
compiled and nothing is patched: the helper uses the installed `tclmagic.so` and
`magic.tcl` (found with `magic --prefix`, `$CDEPT_MAGIC`, or `$CDEPT_MAGIC_TCLDIR`), so the
tab and the batch runs are the same Magic. No submodule is needed (CDePT does not build
Magic); the protocol and the frame files are the xschem helper's
(`libcdept::xschem::protocol`, `frame`), and the tab uses `libcdept::xschem::Session`
(restart on a crash, three tries).

Pixels: the window is a screen-sized cairo surface (`$CDEPT_MAGIC_SCREEN`, default
3840×2160; a window keeps its surface because Magic keeps its cairo context). What
changed is found by comparing with the last copy (bounding box of the changed rows),
after every batch of requests and every 40 ms, and copied to the frame file.

Input: X events go to Magic's event handler (`TCairoEventProc`): keys are Magic's key
macros (`v` view, `s` select, `a`, `b`, `z`, …), buttons its box tool. Magic sends a
command to the window under its "current point", which the helper keeps at the pointer
(`*bypass setpoint`, as Magic's wrapper does on every motion).

### What had to be worked around (Magic 8.3.623)

| Problem | Why | Fix in the helper |
|---|---|---|
| Startup files fail ("Error parsing system startup file", empty message) | with `-nowrapper`, the first window gets a frame from a caption height that is garbage (below), commands miss the window | `-nowindow`; the window is opened after `magic::initialize` (trace on it) with caption and scroll bars off |
| Windows drawn cut off at the bottom | `windCaptionPixels` is set in `WindInit()` before any window exists; the Cairo driver's `GrTextSize()` fails then and leaves the rectangle unset, so the value is stack garbage (seen 460 … 3000), and `WIND_MIN_HEIGHT` adds it even when windows have no caption | the variable (not exported) is found in `tclmagic.so`'s symbol table and set to 18 (`cdept::fixcaption`). Without a symbol table (a stripped build) a warning is printed |
| Nothing drawn without a window border | a top-level window with `windowborder off` draws nothing | the 2-pixel border stays |
| Loaded cells drawn black | `magic.tcl` turns the background DRC off while starting and on again only when `drc status` returned 1, which it does not (it prints); areas marked `<< checkpaint >>` then stay drawn as "not checked" | `drc on` after start-up |
| `save <cell>` | writes the *edit* cell under that name | the save writes the cells with `writeall force <cells>` |

Real Magic in `-nowrapper` mode shows the first two problems too (checked under Xvnc:
"At minimum scale!", the startup errors, then an X error).

### The tab

- Tool bar: Fit, Zoom In/Out, Box, Wire, Select, Delete, Expand, Grid, DRC, Layers.
- Bottom: Magic's command line (`:` in the drawing jumps there; Up recalls) and its
  messages (what Magic prints, and the answers of commands).
- Right: the layer list (paint/erase in the box) and the DRC results: the cell as it is
  now (unsaved edits included) is checked (`drc check` over the cell, `drc listall
  why`), a click boxes and zooms to an error.
- Menus: Magic (Save, Edit Selected Cell, back to the top, expand, layer list, DRC,
  Restart Magic) and Cell (Open Schematic, Generate / Update from Schematic, LVS against
  Schematic, Select in Libraries). Edit › Undo/Redo and Ctrl+Z/Ctrl+Y are Magic's
  `undo`/`redo`. File › Export SVG.
- Save writes the modified cells of the view's folder, then CDePT makes the `use`
  paths relative and writes the sidecar (below). The "changed on disk" watch knows the
  tab's own saves.
- Placed cells that cannot be found are listed in red over the drawing.
- Closing with unsaved changes asks Save / Discard / Cancel. A killed or crashed helper
  is started again with the file as saved (unsaved changes are lost; the file is not
  touched).
- A GDS view can be opened in a Magic tab too ("Open With › Magic (tab)"): read with
  `gds read`, looked at, not saved (`cdept magic import-gds` makes Magic views of it).
- Cross-probing with the schematic tab the layout is linked with: instances selected in
  Magic are selected in xschem and back (Magic's instance names are the schematic's).

Test driver commands (`magic <verb>`, GUI-TESTING.md): `command`, `save`, `drc`,
`drc_show`, `generate`, `layers`, and the checks `expect_saved`, `expect_modified`,
`expect_drc`, `expect_generated`, `expect_log`, `expect_cell`, `expect_selected`,
`expect_missing`. `tcl`, `tcl_send`, `tcl_expect` run Tcl in Magic when the active
document is a Magic tab.

Debugging: `cdept-magic --run [--magic-rc F] [--size WxH] -e <tcl>… --ppm out.ppm`
starts Magic in the helper, runs Tcl and writes the window as a picture;
`CDEPT_MAGIC_TRACE=1` prints the stand-ins' window and event traffic, `CDEPT_MAGIC_LOG=<file>`
the requests, `CDEPT_MAGIC_DEBUG=1` the Tcl stack of a failed start.

## Libraries: views placing views

A `.mag` file places cells with `use <cell> <instance> [<folder>]`. Magic reads a
relative folder from the placing file's folder, and writes absolute ones (with `~`, or
`$PDKPATH` for the PDK's cells) when it saves. CDePT keeps two things right:

- **Paths**: a placed cell of the project gets the folder of its view, **relative** to
  the placing file (`use inv  inv_0 ../../../logic/inv/layout`), so the project can be
  moved or cloned. After every save from a tab, and with `cdept magic refs --fix`
  (`magic::refs::normalize`).
- **Sidecar** `<cell>.mag.json` next to the file: the `cdept-layout` schema of GDS
  views (docs/KLAYOUT.md §3.2), our own: what this file is (library/cell/view/top),
  and for every placed cell of the project which library/cell/view it is
  (`references`, with the placed box), and, once linked with a schematic, `source` and
  `instances`.

A placed cell is found by: the sidecar entry, the `use` path (none: the placing file's
folder, where generated devices are), a project cell of that name with a Magic view
(own library first). PDK cells (`$PDKPATH/…`, on the PDK's `addpath`) are external. A
cell found by none is missing: a red line in the tab, a problem in `cdept magic refs`,
an unresolved reference in the Hierarchy panel.

Rename, copy and move of cells (`Library::rename_cell_following`, `copy_cell`, the
refactor plans of the GUI and `cdept cell rename|copy|move`) rewrite the `use` lines
(cell name and folder) of the views placing the cell, the sidecars (re-keyed), and a
copied or moved view's own relative paths for its new folder. Read-only libraries are
reported, not changed; open files with unsaved changes block a plan. The reference
index (Hierarchy panel, Used By) reads `use` lines (`hierarchy::Builder::mag_file`).
New Magic views come from the template with the technology of the snapshot (`[magic]
tech`), with a sidecar. The tab's helper gets the folders of the project's own Magic
views as search path (`addpath`), the batch runs too (`Magic::search`).

## Layout from schematic

`cdept magic link <lib> <cell>` (GUI: Cell › Generate / Update from Schematic) makes or
updates the cell's Magic view from its xschem schematic (the sidecar's `source`, else
the cell's `schematic` view):

- the schematic is read like for KLayout (`link::read_schematic`), with the Magic
  device map `pdk/<PDK>/magic-devices.toml` (`cdept-devices` format: `library` = the
  generators' Tcl namespace, `cell` = the device, `params` templates). Shipped for
  sky130A (nfet/pfet 01v8, lvt, hvt, g5v0d10v5, xhigh/high poly resistors, MIM caps)
  and ihp-sg13g2 (lv/hv MOS, rsil, rppd, rhigh, cmim, rfcmim);
- devices are made with the PDK's generators, `magic::gencell <lib>::<device> <name>
  <params>`, named as the schematic instance (Magic's instance name *is* the link);
  subcircuits of project cells are instances of their Magic views;
- missing instances are added in a row right of what is there; an instance with other
  parameters is regenerated in place (another device type: replaced at its place);
  `--remove` deletes layout instances not in the schematic; the device cells written
  are the ones used (old versions are removed);
- the sidecar records `source` and what each instance was made into.

In the GUI the edits happen in the tab's own Magic (save first, apply, save), so they
are undoable. `cdept magic check-source` prints the differences (exit 1 when out of
sync).

## Batch runs

`magic -dnull -noconsole -rcfile <rc>` with the Tcl on stdin, in the folder of the job,
with `Project::tool_env()`; every script is wrapped in `catch`, prints `CDEPT:` lines,
has a time limit (`[magic] timeout`, default 15m, `--timeout`) and is killed when its job
is cancelled. Input: the `.mag` view (the project's view folders on the search path),
else a cell of a GDS view.

| CLI | API | Output |
|---|---|---|
| `cdept magic extract <lib> <cell> [--style S] [--pex none\|c]` | `Magic::extract` | `<project>/magic/<lib>/<cell>/<cell>.spice` |
| `cdept magic drc <lib> <cell> [--style S]` | `Magic::drc` | `<cell>.drc.json`; exit 1 when not clean |
| `cdept magic lef <lib> <cell> [--hide]` | `Magic::lef` | `<cell>.lef` |
| `cdept magic lvs <lib> <cell>` | `flows::lvs` | extraction, schematic netlisted by xschem for LVS, netgen with `[netgen] setup`; exit 1 on a mismatch |
| `cdept magic link <lib> <cell> [--schematic V] [--remove]` | `flows::generate` | the Magic view |
| `cdept magic check-source <lib> <cell>` | `flows::generate` (dry run) | exit 1 when out of sync |
| `cdept magic export-gds <lib> <cell> [-o F]` | `flows::export_gds` | one GDS with the hierarchy |
| `cdept magic import-gds <file> <lib> [--cell C]… [--replace]` | `flows::import_gds` | one Magic view per cell, relative `use` paths, sidecars |
| `cdept magic refs [--fix]` | `refs::check` | problems of the project's Magic views |
| `cdept open <lib> <cell> [view] --tool magic` | `launch::Launcher` | a Magic window (not a tab) |

## Tests

- `libcdept`: `magic::mag` (reader, broken files, rewrites, paths), `magic_refs.rs`
  (resolve, normalize, rename/copy/delete/move with refactor, read-only libraries,
  cycles, unicode/spaces/dots in names, broken sidecars, 200-cell hierarchy),
  `real_magic_flows.rs` (sky130A: generate, update, check-source, GDS export/import,
  refs, LVS; skipped without Magic/PDK; LVS needs `CDEPT_XSCHEM` and netgen), and the
  batch tests in `magic/mod.rs`.
- `cdept-magic/tests/session.rs`: the helper through a `Session` with sky130A: tech,
  cross-library `use` path, frames (not blank, resize), mouse on the box tool, an edit,
  kill -9 and restart (file untouched, edit lost), save.
- GUI: `cdept-gui/scripts/magic.script` (examples/sky130-inv) and `open-with.script`
  (examples/ihp-inv).

## Open

- One window per helper: Magic's own commands that open more windows (`openwindow`)
  are drawn in memory but not shown.
- Magic's GUI wrapper (its menus and layer tool bar) is not used; the layer list has no
  colours; Tk dialogs of PDK scripts go through faketk (tested: none of the gencell
  dialogs; the link uses the generators without dialogs).
- Nets (opens/shorts/flylines like KLayout's) are not done for Magic; cross-probing of
  nets neither (instances only).
- Unsaved changes are lost when the helper crashes (Magic's `crashbackups` is not used).
- The DRC panel lists boxes as Magic reports them (also those inside device cells).
- Resistive extraction, `extresist`, per-subcell options.
