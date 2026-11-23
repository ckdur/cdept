# CDePT: embedded KLayout — plan

Goal: opening a layout view shows KLayout's layout editor **inside a CDePT tab**
(dockable, detachable, CDePT menus and panels), driven by KLayout's own C++ code, so
drawing, editing, undo and file I/O are exactly KLayout's. Layout views reference each
other the way Virtuoso cellviews do: a GDS holds only its own cell; the cells it
places live in other views and are found through the project's libraries.

Status (2026-11-20): steps 0–4 built and tested (helper protocol tests, Rust
integration tests, GUI scripts `layout-*.script`): editing, references, edit in place,
properties and PCell forms, OASIS references, renames that follow, DRC/LVS (§8).
Open items: §7 and docs/NOTES.md.

Decided:
- **Helper process.** KLayout is GPL-3.0-or-later; CDePT must stay MIT (NLnet). The
  GPL code runs in `cdept-klayout` (a Python program executed by KLayout itself) and
  talks to CDePT over a socket, like `cdept-xschem`. `libcdept`, `cdept-gui` and
  `cdept` never link KLayout.
- **Submodule** `cdept-klayout/klayout`, pinned to **v0.30.12** (`e71272c3b`, what the
  asic-all image builds). Zero patches to KLayout unless unavoidable.
- **Full build** of KLayout (Qt, Ruby, Python) from the submodule with
  `cdept-klayout/build.sh` into `cdept-klayout/.bin/` (not in git). The Qt build is
  needed anyway for DRC/LVS (Ruby DSL), for PDK autorun macros (PCell libraries) and
  for "Open in KLayout" as an external fallback. Qt5 with QtMultimedia; Qt6 also
  works when `qt6-5compat-dev` and `qt6-multimedia-dev` are installed.

---

## 1. What KLayout offers (checked on 0.30.12)

| Fact | Consequence |
|------|-------------|
| `pya.LayoutView(editable, manager)` exists **without a widget** (no Qt window) in the Qt build and in the Qt-less pip module | The editor core runs headless in the helper |
| All edit modes are there: `select move ruler polygon box text path instance partial` (`switch_mode`) | Editing needs no Qt dialogs for the basic modes |
| `send_mouse_press/release/move/double_clicked_event(DPoint, buttons)`, `send_wheel_event`, `send_key_press_event`, `send_enter/leave_event` | Input is fed from egui, as with xschem |
| A box drawn by two synthetic clicks lands in the layout; errors (e.g. no current layer) come back as Python exceptions | Errors become notices in CDePT |
| `get_pixels_with_options(w, h)`: 1600×1000 in ~4 ms, 4K in 50–85 ms incl. `to_bytes` | Pixel streaming is fast enough; no damage tracking needed at first |
| `PixelBuffer.to_bytes()`: 8-byte header (width, height as u32 LE) + pixels as u32 LE `0xAARRGGBB`, i.e. bytes **B G R A**, opaque | Frame format `bgra`; the GUI swizzles while uploading |
| Undo/redo through `pya.Manager` given to the view | Undo works without the main window |
| Ghost cells (`Cell.ghost_cell = True`): referenced, not defined. Written to GDS as references only, read back as ghost cells | The basis of references between views (§3) |
| `pya.Application.instance().main_window()` is `nil` in `-zz`; the helper runs `-z` (hidden, offscreen Qt) instead, see §2 | No menus, no dialogs, no layer toolbox: CDePT draws those (§4) |
| Qt widget dialogs (properties, PCell parameters, editor options, layer toolbox) cannot be mirrored the way `faketk.tcl` mirrors Tk | CDePT re-makes the few that matter with egui, from the API |

## 2. Architecture

```
cdept (MIT)                                  cdept-klayout (GPL-3.0-or-later, helper)
┌───────────────────────────────┐  socket   ┌─────────────────────────────────────────┐
│ LayoutView tab (cdept-gui)    │◄────────► │ klayout -z -nc -e -r cdept_klayout/main.py│
│  texture ◄── frame file ──────┼───────────┼─ pya.LayoutView (editable, Manager)     │
│  input → Input                │  frames   │  get_pixels_with_options → frame (bgra) │
│  toolbar: modes, zoom, undo   │           │  references: ghost cells ↔ other views  │
│  layers / cells / selection   │◄─ events ─│  layers, cells, selection, status       │
│ libcdept::klayout (Session)   │           │  save: own cell only, report references │
│ libcdept::layout (refs, meta, │           └─────────────────────────────────────────┘
│   GDS reader) ─ resolves names│
└───────────────────────────────┘
```

- The helper is **one Python program run by KLayout's own interpreter**:
  `klayout -z -nc -e -r <helper>/cdept_klayout/main.py -rd connect=<socket>
  [-rd frame_dir=<dir> -rd display=1]` with `QT_QPA_PLATFORM=offscreen` and a private
  `KLAYOUT_HOME`. `-z`: a hidden GUI application (not `-zz`: some editing code builds a
  Qt dialog even when it never shows it, e.g. copy, and aborts without a
  QApplication; a dialog that does open is closed by a timer and reported as a
  warning); `-e`: editable mode; `-nc`: the user's `klayoutrc` is not read. Autorun
  macros on `KLAYOUT_PATH` still run, so PDK PCell libraries register themselves; the
  user's own `~/.klayout` does not (salt packages such as `klive` would otherwise
  start in every helper). `-rd` values arrive as Python globals.
- One helper per open layout tab (like xschem). A separate headless helper serves
  previews and exports (`Service`).
- **All knowledge of the project stays in Rust** (MIT): which library a name belongs
  to, where its file is, searching for a cell. The helper gets a ready-made reference
  table (§3.3) and only does KLayout work. The sidecar files (§3.2) are written only
  by Rust.

### Crates and folders, and who owns them

| Part | Licence | Owner |
|------|---------|-------|
| `cdept-klayout/klayout` (submodule), `cdept-klayout/build.sh`, `README.md` | GPL-3.0 | helper |
| `cdept-klayout/helper/cdept_klayout/*.py` (the helper), `cdept-klayout/tests/` | GPL-3.0-or-later | helper |
| `libcdept/src/klayout/` (protocol, engine, session, service, setup) | MIT | client |
| `libcdept/src/layout/` (GDS reader/rewriter, sidecar, resolver) | MIT | layout |
| `cdept/src/main.rs` (`cdept klayout …`, `cdept layout …`) | MIT | client |
| `cdept-gui/src/views/layout/`, document generalisation | MIT | gui |

## 3. Layout views and references

### 3.1 The model
A layout view file (`<lib>/<cell>/layout/<cell>.gds` in `cells` libraries,
`<lib>/<cell>.gds` in `flat` ones) contains **its own top cell and cells local to it**
(e.g. generated sub-cells). A cell that lives in another view is placed as an
instance of a **ghost cell** (referenced, not defined). Stock KLayout opens such a file
and shows empty instances; CDePT fills them.

On open, every ghost name is resolved:
1. the file's sidecar says which library/cell/view it is → that view's file;
2. no sidecar entry → **search** the project's libraries (in `cdept.toml` order) for a
   cell of that name with a layout view (GDS, then OASIS); found → used (and recorded
   in the sidecar on the next save or by `cdept layout refs --fix`);
3. not found → the ghost cell is drawn as a **red box with an X and the text
   "<lib>/<cell>/<view> cannot be found"** (or "<name> cannot be found"), with the
   bounding box last recorded in the sidecar, else 2 × 2 µm at the origin. The marker
   is never written to the file: the cell stays a ghost.

Resolution is recursive (a placed cell's own ghosts are resolved through its own
sidecar), with cycle detection. Cells loaded from other views are **external**: shown,
selectable, not saved. Editing inside them is not supported yet (open that view).

On save, the helper writes the top cell and its local cells, with every external and
missing cell as a ghost (references only), and reports each ghost with the file it came
from and its bounding box; Rust updates the sidecar from that report.

`cdept layout export <lib> <cell> -o out.gds` writes the **self-contained** stream
(every reference filled in), for DRC, LVS and tape-out.

### 3.2 Sidecar: `<file>.json` next to the layout file
`inv.gds` → `inv.gds.json`, `inv.gds.gz` → `inv.gds.gz.json`. Human-editable, own
schema (not a Virtuoso format):

```json
{
  "format": "cdept-layout",
  "version": 1,
  "library": "analog",
  "cell": "buf",
  "view": "layout",
  "top": "buf",
  "references": {
    "inv": {
      "library": "analog",
      "cell": "inv",
      "view": "layout",
      "bbox": [0.0, 0.0, 1.2, 3.4]
    }
  }
}
```

- `library` / `cell` / `view`: what this file is. A copy made behind CDePT's back keeps
  the old values, which `cdept layout refs` reports (like `cdept xschem link`).
- `top`: name of the top cell in the file.
- `references`: key = structure name used in this GDS; `library` is the name in
  `cdept.toml`; `bbox` = `[left, bottom, right, top]` in µm, in the referenced cell's
  own coordinates, last known (used for the "cannot be found" marker).
- The sidecar is optional. Missing → every ghost is searched by name.
- The library scanner ignores `*.json` next to layout files (they are not views).
- Copy/rename of a cell rewrites the sidecar's `library`/`cell`/`top`, the top
  structure name inside the GDS, and in the copied/renamed cell nothing else. (Other
  views that reference the renamed cell: later, with the hierarchy item of NOTES.md.)

### 3.3 Reference table (Rust → helper)
Sent with `open`, `place`, `render` and `export`. Every file reachable from the root,
already resolved:

```json
{
  "root": "/abs/lib/buf/layout/buf.gds",
  "files": {
    "/abs/lib/buf/layout/buf.gds": {
      "label": "analog/buf/layout",
      "refs": {
        "inv": { "file": "/abs/lib/inv/layout/inv.gds", "label": "analog/inv/layout" },
        "res": { "file": null, "label": "analog/res/layout",
                 "bbox": [0, 0, 2, 2], "reason": "cell res has no layout view" }
      }
    },
    "/abs/lib/inv/layout/inv.gds": { "label": "analog/inv/layout", "refs": {} }
  }
}
```

A ghost name of a file that is not in its `refs` is treated as missing with the name
as label. Rust types: `libcdept::layout::RefTable`, `FileRefs`, `Ref`. A file's entry
also carries its sidecar's `pins`, and the table the technology's `pin_rules` (the
device map's `[pins]`, docs/LINKING.md §9).

OASIS views are read the same way (`libcdept::layout::oasis`: CELLNAME tables, CELL,
PLACEMENT, CBLOCK); KLayout writes a ghost cell as a CELLNAME without a CELL record.
Compressed files (`.gds.gz`, `.oas.gz`) are not supported: CDePT writes its own views.

Version 2 adds the link with a schematic (`source`, `instances`; docs/LINKING.md §1); a
sidecar without a link is still written as version 1.

### 3.4 Editing in place
Descending into an instance (`descend`) makes its cell the one edited; edits go to that
cell's file. The next `save` writes the root as before and, for every external file
changed in place, that file with its own cells (original names) and the cells of other
files as ghosts under the names it uses for them. The SaveReport lists those files in
`also` (each a SaveReport); CDePT writes one sidecar per file. A file may not be changed
when its `FileRefs.read_only` is set (read-only library) or when CDePT lists it in
`locks` (unsaved changes in another tab); descending is refused, and a change detected
in such a file is undone at once. After a save, tabs showing or placing a written file
reload unless they have unsaved changes.

### 3.5 Renames
Renaming a cell (`Library::rename_cell_following`, `layout::follow`) rewrites the
layout views that place it: the sidecar entry points at the new name; the structure in
the file is renamed when it carried the old cell name and the new one is free there
(GDS, or OASIS written back uncompressed and non-strict); the sidecar is re-keyed.
Read-only libraries are reported, not changed. Deleting a cell or view lists the views
that will show "cannot be found".
### 3.6 PCells
- KLayout libraries (PDK on KLAYOUT_PATH, Basic) and the project's PCell views are
  placed as library cells; KLayout writes them with geometry plus a context record
  (`$$$CONTEXT_INFO$$$` in GDS, properties in OASIS) and reads them back live. The
  Rust readers ignore the context structure. `export` writes plain geometry
  (`write_context_info = false`).
- PCell view: `<cell>/pcell/<cell>.pcell.py` defines a `pya.PCellDeclarationHelper`
  subclass (named by `PCELL = <class>`, else the only one). The PCell is named after
  the cell, in a KLayout library named after the CDePT library. The reference table
  carries `pcells: {library: {cell: file}}`; the helper registers them before reading
  (technology set first, so PDK libraries bound to it link too), and again when a file
  changes (checked every second; `refresh_pcells {pcells?}`).
- Shape parameters are JSON objects in µm: `{"point": [x, y]}`, `{"box": [l, b, r, t]}`,
  `{"polygon": [[x, y], …]}`, `{"path": [[x, y], …], "width": w}`, `{"edge": [p1, p2]}`.
  Changes are coerced (current values first, then the change, then again); C++ Basic
  PCells through the port in `basic.py`.

## 4. Protocol (`libcdept::klayout::protocol`, version 1)

Transport as for xschem: Unix socket, UTF-8, one JSON object per line. CDePT listens,
the helper connects (`-rd connect=<path>`) and sends `ready` or `failed`.

CDePT → helper (`"type"` tag):
- `call {id, method, params}` → exactly one `reply {id, ok, value | error}`.
- `resize {width, height}`, `input {input}` → no reply.
- `shutdown {id}` → reply, then exit.

Helper → CDePT:
- `ready {protocol, klayout}` / `failed {error}`
- `reply {id, ok, value, error}` (`value` any JSON)
- `frame {serial, path, width, height, stride, format}` (`format`: `bgra` here; xschem's
  frames are `rgba`): new frame file, laid out as in `xschem::frame` (no header).
- `damage {serial, x, y, width, height}`
- `event {event}`, with `event.kind` one of:
  - `layers {layers: [Layer]}`: `Layer {id, name, source, color, frame_color, dither,
    visible, current, depth, valid}`; `id` is the index in KLayout's flattened layer
    list, colours `#rrggbb`.
  - `cells {top, path, cells: [CellNode]}`: `CellNode {name, children: [name],
    external: label|null, missing: bool, file: path|null}`; `path` is the shown cell
    path from the top; `context` = `{file, label, writable, reason}` while editing in
    place, else null.
  - `selection {count, bbox, items: [SelItem]}`: `SelItem {kind, layer, cell, text}`,
    at most 200 items.
  - `status {x, y, mode, message}`: pointer position in µm (throttled), current mode,
    KLayout's one-line hint if any.
  - `modified {dirty, can_undo, can_redo, edited, revision}`: `edited` = external files
    with unsaved in-place changes; `revision` = changes seen so far (edits, undo,
    redo), only growing while the helper runs (what was made from the layout at
    another revision, e.g. the nets, is out of date).
  - `notice {level, text}`: errors from editing (e.g. "Please select a layer first").
  - `nets_live {revision, ms, nets, devices, moved, labels, added, gone, untraced,
    stopped}`: the last extraction's terminals traced again after a change
    (docs/LINKING.md §8.1).

Input (`input`): `{kind, x, y, button, buttons, modifiers, delta, horizontal, key,
text}` with `kind` ∈ `press release double_click move wheel key enter leave`; `x`, `y`
in physical pixels; `button` ∈ `left middle right`; `modifiers {shift, ctrl, alt}`;
`key` a name (`Escape Return Backspace Delete Tab Left Right Up Down PageUp PageDown
Home End F1…F12`) or one character in `text`.

Methods (`call`):

| method | params | value |
|--------|--------|-------|
| `open` | `{refs: RefTable, editable, technology?, layer_properties?, locked?}` | `{top, dbu, missing: [name], external: [name], notes}` |
| `save` | `{file?}` (default: the root) | `{file, top, references: {name: {file, missing, bbox}}, pins: {name: Pin}, also: [SaveReport]}` (`pins`: the file's pins for its sidecar, docs/LINKING.md §9) |
| `descend` | `{}` (the selected instance) | `{cell, context}`: edit that cell in place (§3.4) |
| `ascend` | `{all?}` | `{cell, context}` |
| `locks` | `{files}` | files with unsaved changes elsewhere (not to be changed) |
| `mode` | `{name}` | |
| `zoom` | `{what: fit|in|out|box|selection, box?: [l,b,r,t] µm}` | |
| `undo` / `redo` / `delete` / `select_all` / `clear_selection` | | |
| `copy` / `cut` / `paste` | | if KLayout's standalone view allows it |
| `layer_visible` | `{id, visible}` | |
| `current_layer` | `{id}` | |
| `add_layer` | `{layer, datatype, name?}` | `{id}` |
| `show_cell` | `{path: [name]}` | descend / ascend / another top |
| `levels` | `{min, max}` | hierarchy levels shown |
| `place` | `{name?, refs: RefTable}` | loads the cell (external) and switches to `instance` mode |
| `config` / `get_config` | `{name, value}` / `{name}` | KLayout's `set_config` / `get_config` |
| `render` | `{refs?, width, height, out}` | writes a PNG (previews; works without display) |
| `export` | `{refs?, out}` (no `refs`: the open layout) | writes a self-contained GDS/OASIS (all references filled); `{file, missing, pins}` (the pins as they are now, for the net check) |
| `info` | | `{klayout, technology, file, top, cells, dbu}` |
| `properties` | `{}` | `{objects: [Object], count, layers}`: the selection as the properties dialog shows it (µm) |
| `set_properties` | `{index, key, object, pcell}` | as `properties`; one undo step; refused when the selection changed or the object is inside another view's cell |
| `libraries` | `{}` | `[{name, description, technologies, cells: [{name, pcell}]}]`: KLayout's libraries (also those bound to a technology; `project` true for the project's PCell views) |
| `pcell_form` | `{library, cell}` | `{library, name, params, values}`: the parameter declarations and defaults |
| `place_library` | `{library, cell, values}` | `{name, pcell}`: instance mode with a library cell or PCell |
| `replace_cell` | `{index, key, library, cell, values}` or `{index, key, refs, name}` | as `properties`: the instance places another cell, transformation kept; one undo step |
| `refresh_pcells` | `{pcells?}` | `{changed, libraries}`: PCell files that changed registered again |
| `link_apply` / `link_instances` / `link_select` / `link_selected` / `link_highlight` | see docs/LINKING.md §5 | linking with the schematic; select/selected take instance paths (`x1/M1`, §10) |
| `pins_list` / `pins_add` / `pins_update` / `pins_rename` / `pins_remove` / `pins_select` / `pins_tool` / `pins_style` | see docs/LINKING.md §9 | pins, CDePT objects kept in the sidecar |
| `nets_load` / `nets_highlight` / `nets_flylines` / `nets_probe` / `nets_clear` | see docs/LINKING.md §8 | nets: the LVS database of the last extraction, highlights, flylines, probing |
| `nets_live` | `{connectivity?, rules?, on?, delay_ms?, limit_ms?, update?}` | live flylines (LINKING.md §8.1): set up from the extraction's JSON, switch, or trace now; then `nets_live` events |
| `markers` | `{shapes: [{kind: polygon\|box\|edge\|text, …}], color?, zoom?}` | `{markers}`; empty clears (DRC/LVS highlights, never part of the layout) |
| `python` | `{code}` | `repr` of the last expression (tests, power users) |

Unknown method → `ok: false`, error "unknown method …".

## 5. CDePT side

- `libcdept::klayout::Engine` (blocking, for CLI/tests) and `Session` (non-blocking,
  per tab, threads, restart, like `xschem::Session`); `Service` for previews.
- Finding KLayout: `$CDEPT_KLAYOUT` (the `klayout` binary), then
  `cdept-klayout/.bin/klayout` of a development tree, then `klayout` next to `cdept`,
  then `PATH`. The helper script: `$CDEPT_KLAYOUT_HELPER`, `<prefix>/share/cdept-klayout`,
  or the development tree.
- Setup from the project: `KLAYOUT_PATH` = `[klayout] path` (list) or the PDK's
  `libs.tech/klayout` folder; `[klayout] tech = "<name or .lyt>"`, `[klayout]
  layer_properties = "<.lyp>"`; `PDK_ROOT`/`PDK` from `Project::tool_env`.
- CLI: `cdept klayout version|python <code>`, `cdept layout refs <lib> <cell> [--fix]`,
  `cdept layout export <lib> <cell> -o <file>`, `cdept layout png <lib> <cell> -o
  <file> [--size WxH]`.
- GUI: a layout document tab: the picture, a tool bar (modes, zoom, undo/redo, save,
  hierarchy levels), panels inside the tab (layers with visibility and current layer;
  cell tree with external cells marked and missing ones red; selection), status line
  (µm coordinates, mode, hint). "Place cell…" uses CDePT's library picker filtered to
  layout views. Inspector shows a PNG preview of a selected layout view.

## 6. Steps

0. Build KLayout from the submodule; helper skeleton answers `info`, `python`. ✔ probe
1. Reference loading + saving with ghost cells, missing markers; sidecar; resolver;
   CLI `refs`, `export`, `png`.
2. Display: frames, input, modes, layers, cells, status; `Session`.
3. GUI tab, panels, place cell, Inspector preview, double-click opens a layout view.
4. Properties dialog, PCell parameters, copy/paste, edit in place of external cells,
   OASIS references, renames that follow, DRC/LVS (§8), KLayout in CI. ✔

## 7. Known gaps and risks
- Properties and PCell forms are CDePT's (egui). PCell handles are dragged by the
  helper (select mode), not by KLayout's partial mode. A PCell view renamed breaks the
  layouts placing it (they name LIB/PCELL).
- Full-frame transfer on every change; 4K costs 50–85 ms. Damage rectangles or
  tile diffing later if it feels slow.
- External cells are edited in place after a descent (§3.4); without one they stay
  read-only. Locks are exchanged once per GUI frame; a reload loses zoom and selection.
- Renaming an OASIS file's cells writes it back without CBLOCKs and non-strict (table
  offsets "not given"); KLayout reads it the same.
- DRC/LVS pass only the deck variables `input`, `topcell`, `report`, `schematic`,
  `target_netlist` (IHP's names); others with `-D` on the CLI. No cancel in the GUI.
- Name clashes between structures of different views are resolved by renaming the
  external copies in memory; the saved file keeps the names of its sidecar.

## 8. DRC and LVS
- Deck: `[klayout] drc` / `lvs` in cdept.toml, else the first `*.drc` / `*.lvs` in
  `<KLAYOUT_PATH dir>/tech/<check>/`, `<dir>/<check>/`, `<dir>/tech/` (PDK-named first,
  partial decks such as antenna/density last).
- Run: export a self-contained GDS (helper `export`; in the GUI from the open document,
  unsaved edits included), then `klayout -b -r <deck> -rd input=<gds> -rd topcell=<top>
  -rd report=<file>` (LVS also `schematic=<spice>`, `target_netlist=<file>`), with the
  project's environment, in ~/.cache/cdept/verify/<project>/<lib>/<cell>/<check>/.
- DRC results: the `.lyrdb` XML read in Rust (`verify::Rdb`), shapes in µm in top-cell
  coordinates. LVS results: `cdept_klayout/lvsdb.py` (GPL, run by `klayout -b`) writes
  `{matched, circuits: [{layout, schematic, status, nets, devices, pins, subcircuits}]}`.
- Schematic netlist for LVS: xschem with `lvs_netlist` set (top cell is a `.subckt`,
  devices use `lvs_format`).
- CLI `cdept layout drc|lvs <lib> <cell> [--deck] [--view] [-D NAME=VALUE]`, exit 1 on
  violations / mismatch. GUI: Layout › Run DRC / Run LVS (background), results panel,
  a click highlights (`markers`) and zooms.
- Highlights: helper method `markers`, `pya.Marker` objects.

## 9. LEF abstracts (read-only)
A view of kind `abstract` whose main file is a `.lef` (a standard cell or macro library
LEF) opens in the same layout tab, read-only. The LEF stays the view's main file;
nothing is converted, and no sidecar or GDS is written.

- **Reference table.** `refs::resolve` on a `.lef` returns a table with one file
  (`read_only`), no references, no PCells, no pin rules and `lef: {tech_lef?}`
  (`libcdept::klayout::lef::LefInfo`). Everything that sends a table (open, preview,
  `layout png`) therefore works unchanged. The cell of a multi-macro file is the view's
  `Entry::member`: the tab shows it with `show_cell`, previews pass `cell`.
- **Technology LEF** (layers, units), first match wins (`lef::tech_lef`): the `techlef`
  files of the snapshot's `[[refs.library]]` whose folder holds the LEF (nominal corner
  preferred); else a `*tech*.lef` / `*.tlef` next to the file or in the library's
  `techlef/`, `lef/` or `tech/` folder (`find_tech_lef`: IHP `sg13g2_tech.lef`, sky130
  `techlef/*__nom.tlef`); else none (layers are numbered by the order the LEF names
  them). The helper reads it first (`LEFDEFReaderConfiguration.lef_files`).
- **Layer mapping** (`cdept_klayout/lef.py`): when the project's KLayout technology has
  a LEF/DEF layer map (`<lefdef><map-file>`; IHP `sg13g2.map`), its reader settings are
  used: layers get the PDK's GDS numbers (pins `8/2`, obstructions `8/4`, boundary
  `189/4` for IHP), and the technology's `.lyp` is loaded, so the colours are the PDK's.
  A technology without one (sky130A's `.lyt` has none) gets KLayout's defaults: layers
  numbered in order of appearance, pins / labels / obstructions on datatype 2 / 1 / 3,
  the macro boundary on layer 1; colours are taken by layer name from the `.lyp` where it
  has them (`met1.drawing`), else from a fixed palette.
- **Layers panel.** One layer per layer and purpose, named `<LEF layer> pin`, `… label`,
  `… obstruction`, plus `macro boundary`; layers with nothing drawn are not listed.
  Obstructions are hatched (hollow when the technology gives pins a hatch), the
  boundary is a thick light outline.
- **Read-only.** The helper opens the view non-editable whatever `editable` says and
  refuses `save` and `export` ("LEF abstract: read-only"). The tab (`views/layout/readonly.rs`)
  disables the edit modes, Place Cell, Delete, Undo/Redo and Save, shows the notice on
  open and refuses `mode`, `save`, `place` and `place_library` from keys or scripts.
  Navigation, zoom, layers and cells panels, rulers, selection and properties (look only)
  work. DEF is not read yet.
- **CLI.** `cdept layout png <lib> <cell> --view <lef view>`.
