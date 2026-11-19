# CDePT: embedded KLayout — plan

Goal: opening a layout view shows KLayout's layout editor **inside a CDePT tab**
(dockable, detachable, CDePT menus and panels), driven by KLayout's own C++ code, so
drawing, editing, undo and file I/O are exactly KLayout's. Layout views reference each
other the way Virtuoso cellviews do: a GDS holds only its own cell; the cells it
places live in other views and are found through the project's libraries.

Status (2026-11-20): steps 0–3 built and tested (helper protocol tests, Rust
integration tests, `layout-smoke.script` in the real GUI). Open items: §7 and
docs/NOTES.md.

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
as label. Rust types: `libcdept::layout::RefTable`, `FileRefs`, `Ref`.

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
    path from the top.
  - `selection {count, bbox, items: [SelItem]}`: `SelItem {kind, layer, cell, text}`,
    at most 200 items.
  - `status {x, y, mode, message}`: pointer position in µm (throttled), current mode,
    KLayout's one-line hint if any.
  - `modified {dirty, can_undo, can_redo}`
  - `notice {level, text}`: errors from editing (e.g. "Please select a layer first").

Input (`input`): `{kind, x, y, button, buttons, modifiers, delta, horizontal, key,
text}` with `kind` ∈ `press release double_click move wheel key enter leave`; `x`, `y`
in physical pixels; `button` ∈ `left middle right`; `modifiers {shift, ctrl, alt}`;
`key` a name (`Escape Return Backspace Delete Tab Left Right Up Down PageUp PageDown
Home End F1…F12`) or one character in `text`.

Methods (`call`):

| method | params | value |
|--------|--------|-------|
| `open` | `{refs: RefTable, editable, technology?, layer_properties?}` | `{top, dbu, missing: [name], external: [name], notes}` |
| `save` | `{file?}` (default: the root) | `{file, top, references: {name: {file, missing, bbox}}}` |
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
| `export` | `{refs?, out}` (no `refs`: the open layout) | writes a self-contained GDS/OASIS (all references filled); `{file, missing}` |
| `info` | | `{klayout, technology, file, top, cells, dbu}` |
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
4. Gaps (§7): properties dialog, PCell parameters, copy/paste, edit-in-place of
   external cells, DRC.

## 7. Known gaps and risks
- No properties dialog or PCell parameter dialog yet (Qt in KLayout); to be made in
  egui from the API.
- Full-frame transfer on every change; 4K costs 50–85 ms. Damage rectangles or
  tile diffing later if it feels slow.
- External cells are read-only; edits inside them are lost on save (warned).
- Name clashes between structures of different views are resolved by renaming the
  external copies in memory; the saved file keeps the names of its sidecar.
