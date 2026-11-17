# CDePT: embedded xschem — plan

Goal: opening a schematic or symbol view shows xschem's editor **inside a CDePT tab**
(dockable, detachable, with CDePT menus and dialogs), driven by xschem's own C code, so
drawing, file handling, netlisting and attribute evaluation are exactly xschem's.

Status (2026-11-17): **step 0 done** — the helper builds from the submodule with xschem's
sources unmodified and runs headless (load, netlist, SVG, instance list). Steps 1–5 are
open. How to build and update it: [`cdept-xschem/README.md`](../cdept-xschem/README.md).

Decided: helper process (so `cdept` stays MIT), submodule pinned to the commit the
asic-all image builds (`8952fb28`, xschem 3.4.8RC), no patches to xschem unless
unavoidable.

The measurements in §1 were taken on xschem 3.4.6; the pinned commit differs slightly
(one more source file, `spectre_netlist.c`).

---

## 1. What xschem looks like from the inside

| Fact | Consequence |
|------|-------------|
| ~85k lines; C core ~60k, `xschem.tcl` 9.7k | Too big to rewrite, small enough to compile ourselves |
| Xlib: ~50 distinct functions, 240 calls, in `draw.c` (135), `xinit.c` (57), `callback.c` (30) | The drawing surface is narrow and can be replaced |
| The Windows port has no X server: Tk's Xlib emulation supplies the same calls | Precedent that an Xlib substitute works |
| Input enters through one C function: `callback(win_path, event, mx, my, key, button, aux, state)`, also exposed as the Tcl command `xschem callback …` | We can feed egui events straight in |
| Tcl: ~2000 `tcleval`/`Tcl_*` calls in 25 files; `xschemrc` and PDK setups are Tcl; netlisting uses Tcl + awk scripts | **Tcl stays.** libtcl is a hard dependency |
| Tk C API: 24 functions, ~60 calls, nearly all in `xinit.c` | Tk can be stubbed |
| Tk GUI in `xschem.tcl`: 31 toplevel dialogs, ~400 widget commands; C code calls them by name (`alert_`, `tk_messageBox`, `input_line`, `text_line`, `load_file_dialog`, …) and waits for the answer | Dialogs and menus must be re-made in egui; they are **modal and synchronous** |
| `has_x = 0` mode (`xschem -x`): Tcl only, no Tk, no drawing. Load, save, netlist and SVG export work; PNG export does not (`print_image` returns without X) | A headless engine is an existing, supported mode |
| Cairo is optional (`HAS_CAIRO`); without it text uses xschem's built-in vector font | Text rendering can go either way |
| State is global: `xctx`, one Tcl interpreter, `has_x`, colours. Several schematics = context switching, max 20 (`MAX_NEW_WINDOWS`) | One engine instance per process, used from one thread only |
| 11 `tcleval("exit")` sites; plain C with manual memory | A fault inside xschem ends its process |
| `parselabel.c`, `expandlabel.c`, `eval_expr.c` are generated (flex, bison) | Build needs flex + bison, or pre-generated files |
| Licence: GPL-2.0-or-later. CDePT: MIT | Linking it into `cdept` makes that binary GPL (see §3) |

## 2. Ways to do it

| | Approach | Verdict |
|---|----------|---------|
| A | Reparent xschem's Tk window into ours (XEmbed) | No. Impossible on native Wayland (the dev host runs Wayland); on X11 an alien child window fights egui's popups, focus and detached windows |
| B | Separate xschem window, Tcl over TCP (old ROADMAP Phase 3) | Keep as **fallback** ("Open in external xschem"), ~15 h |
| C | Unmodified xschem under a private Xvfb, framebuffer streamed into a tab | Works but it is a remote desktop: Tk menus and dialogs inside the picture, no integration with our library model. Emergency option only |
| D | **Compile xschem's C as a library with a replacement for Xlib and Tk** | **Recommended.** Described below |
| E | Own Rust editor reading `.sch`/`.sym` | No for editing (re-implements 20k lines of behaviour and needs Tcl anyway for attribute evaluation). A read-only Rust parser is still wanted for the hierarchy browser |

## 3. Architecture (option D)

```
cdept (MIT)                               cdept-xschem (GPL-2.0-or-later, helper process)
┌──────────────────────────┐   socket    ┌──────────────────────────────────────────┐
│ SchematicView (egui tab) │◄──────────► │ engine loop = Tcl event loop             │
│  texture ◄── shared mem ─┼─────────────┼─ window surface (cairo image)            │
│  input → events          │   frames    │ xshim: Xlib calls → cairo image surfaces │
│  menus → Tcl commands    │             │ tkstub: Tk C API + Tk Tcl commands       │
│  dialogs (egui)          │◄─ requests ─│ dialog procs replaced by "ask the GUI"   │
│  library picker          │             │ libxschem = xschem/src/*.c − main.c      │
└──────────────────────────┘             │ libtcl, cairo  (no X11, no Tk)           │
                                         └──────────────────────────────────────────┘
```

**Why a helper process, not a linked library.**
1. Licence: the GPL code stays in its own binary and talks over a documented protocol, so
   `libcdept`/`cdept-gui`/`cdept` stay MIT. (Not legal advice; this is the usual
   arm's-length reading.)
2. Modal dialogs: xschem's C code blocks until a dialog answers. In a separate process it
   simply blocks on the socket while the egui loop keeps running.
3. Faults, `exit` calls and global state are contained; a crashed engine is restarted
   and the tab reloads its file.
4. No X connection at all, so it behaves the same on Wayland, X11 and in containers.

The engine is written against a message interface, so it can also run on a thread
inside the GUI for debugging.

**Crates**
- `cdept-xschem` (GPL-2.0-or-later, not a default workspace member): git submodule
  `cdept-xschem/xschem`, `build.rs` with the `cc` crate (flex/bison step, sources minus
  `main.c`), `shim/include` shadowing `<X11/*.h>`, `<tk.h>` and `<cairo-xlib.h>`,
  `shim/xshim.c` (the Xlib/Tk calls), `shim/engine.c` (start-up), and the program
  itself. Links libtcl and cairo.
- `libcdept::xschem` (MIT): `protocol` (JSON lines over a Unix socket), `Engine`
  (starts the helper, typed calls), `setup` (generated start-up file), `tcl` (quoting
  and list parsing).
- `cdept-gui::views::schematic` (MIT): the tab. Not started.

**Drawing: Xlib on cairo, pixels to a texture.** xschem draws to a window and a backing
pixmap, erases rubber bands by tiling from the pixmap, and copies areas between them.
Implementing those ~50 calls literally on cairo image surfaces (GC = colour, width,
dash, fill, clip, tile, stipple) keeps xschem's logic untouched; xschem's own cairo text
and image code then draws on the same surfaces (5 `cairo_xlib_surface_create` sites to
redirect). The engine reports damaged rectangles; the GUI uploads them into one egui
texture. A vector display list would be sharper to composite but needs xschem's
erase-by-tile logic re-interpreted; revisit only if texture upload is too slow.

**Input.** egui pointer/key/scroll/resize events → X event numbers, keysyms and state
masks → `callback()`. xschem's key bindings therefore work unchanged. Coordinates are in
physical pixels (points × `pixels_per_point`).

**Menus and toolbar.** `View::menus`/`View::toolbar` built from a table of
(label, Tcl command, optional Tcl variable for check items). Variable values come from
the engine's state updates.

**Dialogs.** After `xschem.tcl` is sourced, the dialog procs are redefined to send a
request and wait for the reply. Order: message/confirm → one-line prompt → property
editor (`edit_prop`/`text_line`) → file/symbol chooser, replaced by **CDePT's library
picker** → the rest. A dialog not yet ported answers "cancel" and logs it.

**Smaller hooks.** Status bar labels, cursor shape, info/ERC window → Console, title
and modified flag → tab title, colour scheme follows the CDePT theme.

**Technology and files.**
- The engine gets its setup as Tcl at start, generated from the project: library search
  path from `[libraries]`, then the PDK's own `xschemrc`. The user's `~/.xschem` is not
  read unless enabled. This is the start of ROADMAP Phase 2's "generate per-tool config".
- xschem's share directory (`xschem.tcl`, awk scripts, `devices` library) ships with the
  helper, taken from the submodule.
- **Constraint:** files stay plain xschem files that stock xschem opens with the
  generated `xschemrc`. No CDePT-only reference syntax.
- Pin the submodule to the commit the asic-all image builds (`xschem:8952fb28`,
  3.4.8RC), so both write the same files.

**Changes to xschem itself.** Target: none (header shadowing, `-D` redirection, Tcl proc
redefinition). Result so far: **none needed for step 0**. xschem.h already skips its
generated `config.h` when `NO_SCCONFIG` is defined, and the whole source compiles
against the stand-in headers (the first attempt lacked four macros, nothing else).
Step 2 is where patches may still become necessary (Tk with drawing on). If so, keep a
short series on a fork branch and offer it upstream. Upstream also has a
`DRAW_ALL_CAIRO` switch marked "work in progress" (all drawing through cairo); if it
matures it shrinks the Xlib surface we have to stand in for.

## 4. Steps

Each step ends with something visible. Hours are estimates.

| Step | Work | Shows | Hours |
|------|------|-------|-------|
| 0 ✔ | Submodule, helper builds headless (`has_x=0`, Tcl only), protocol, client, generated start-up file. CLI: `cdept netlist <lib> <cell>`, `cdept xschem svg|instances|tcl|version` | Done: an IHP PDK testbench netlists and draws to SVG | 15–20 |
| 1 | SVG preview in the Inspector (moved here from step 0: it needs the engine off the UI thread). GUI plumbing: `Tab::Document(DocId)` (dynamic tabs, layout persistence), double-click opens a view, supervisor starts/restarts the engine, shared-memory frame → texture (first with a test pattern) | Double-click opens a tab fed by the helper | 12–15 |
| 2 | xshim + tkstub, `has_x=1`. Draw, resize, pan, zoom, descend/ascend, select, highlight | **Read-only schematic viewer in a tab.** Gate, see below | 30–40 |
| 3 | Editing: full input mapping, menus, toolbar, message/prompt/property dialogs, save, undo, modified flag, close-with-unsaved | Draw and save a small schematic without leaving CDePT | 25–30 |
| 4 | CDePT integration: insert instance through the library picker, new cell/symbol flows, netlist to a view, hierarchy data for the Hierarchy panel, selection → Inspector | Create an OTA from PDK devices and netlist it | 20–25 |
| 5 | Remaining dialogs (search, layers, simulation setup, graphs), several schematics at once, packaging in asic-all, upstream patches | Parity list checked against stock xschem | 30+ |

Total ≈ 130–160 h, more than ROADMAP gave all of Phase 3 (110 h). Steps 0–1 are useful
whatever happens: the headless engine replaces the planned TCP adapter for netlisting
and hierarchy.

**Gate after step 2.** Continue only if (a) a real PDK schematic renders the same as
stock xschem, (b) pan/zoom is smooth on a 4K tab, (c) the patch series on xschem is
short. Otherwise fall back to B for editing and keep the engine for netlist, hierarchy
and previews.

## 5. Risks

1. **Tk assumptions deeper than the count suggests.** `xschem.tcl` with `has_x=1` wants
   to build its whole widget tree. Either stub widget commands wholesale or add a
   "drawing without Tk" switch (a patch). Step 2 finds out.
2. **Dialog tail.** 31 dialogs; the first five cover most editing, the rest is a long
   tail (graphs and the simulation setup are the heavy ones, and Phase 4/5 replace them).
3. **One engine, global state.** One project per engine; 20 open schematics per engine.
   Start a second engine if that ever limits.
4. **Version drift.** Every submodule bump can break the shim. CI builds the helper and
   runs render comparisons (SVG from stock mode vs our frame) on a few PDK schematics.
5. **Build dependencies.** libtcl, cairo, flex, bison on every build host and in CI; the
   helper is Linux-first.
6. **Scope.** This moves CDePT from "orchestrates xschem" towards "contains xschem".
   KLayout and Magic stay external windows; say so in the roadmap.

## 6. What step 0 showed

- **Zero patches** to xschem for the headless engine; see §3.
- **libcairo itself links libX11** on this system, so the helper process still loads
  libX11 although nothing calls it. This is why every stand-in function carries an
  `xs_` prefix: without it our definitions would replace the real ones inside libcairo.
- **Symbol search path.** PDK schematics refer to xschem's devices both as `res.sym`
  and as `devices/res.sym`, and PDK start-up files expect
  `$XSCHEM_SHAREDIR/xschem_library/devices`. The helper therefore uses an install-style
  share folder (assembled from links at build time) and puts both
  `xschem_library/devices` and `xschem_library` on the path.
- **Library folders.** A library is reached through the parent of its folder, so
  references read `<library folder>/<file>`. xschem also has library *aliases*
  (`alias_lib` in `xschem.tcl`); they could map a CDePT library name that differs from
  its folder name. Not used yet.
- **`cells` layout does not descend.** With `cell/symbol/cell.sym` and
  `cell/schematic/cell.sch`, xschem looks for `cell/symbol/cell.sch`, does not find it
  and writes an empty `.subckt` without an error. The `flat` layout works. Solved with
  a `schematic=` attribute in the symbol, see §7.
- **The user's `~/.xschem` is still touched**: xschem creates it if missing and reads
  `recent_files` from it (the user's `xschemrc` is not read). Harmless so far.
- In headless mode xschem prints `tk scaling failed` during SVG export. Harmless; it
  goes to the log at debug level (`cdept -v`).

## 7. xschem views in the `cells` layout

xschem finds a symbol's schematic by swapping `.sym` for `.sch` in the same folder,
unless the symbol (or the instance) has a `schematic=` attribute.

| | Option | Effect |
|---|--------|--------|
| A | Keep both files directly in the cell folder: `lib/cell/cell.sch`, `lib/cell/cell.sym`. The layout already accepts loose files in a cell folder as views | Works in stock xschem with no attribute. References read `lib/cell/cell.sym` |
| B | Keep `cell/schematic/` and `cell/symbol/`, and write `schematic=<lib>/@symname/schematic/@symname.sch` into the symbol | Layout unchanged, but the attribute names the library folder, so copying a cell to another library must rewrite it |

**Decided 2026-11-17: B.** The layout stays `cell/<view>/<file>`. Implemented in
`libcdept::xschem::symbol` (MIT; reads and rewrites only the attribute block of the
`.sym` file):

- **Value.** `<library folder>/@symname/<view>/@symname.sch`. xschem looks it up on the
  library path like a symbol reference and replaces `@symname` with the symbol's file
  name. So the files work in stock xschem too, given the same library path. When the
  symbol file is not named after its cell (a second symbol of the cell), the cell name
  is written out instead of `@symname`.
- **Written** when a symbol view is created in a `cells` library (not in a `flat` one,
  where the schematic is next to the symbol).
- **Rewritten** when a cell is copied or renamed through CDePT (GUI or library API): a
  value pointing into the old cell points into the new one; copied into a `flat`
  library it is removed; copied from `flat` into `cells` it is added. Values that
  point elsewhere, Tcl (`tcleval(...)`) and generator scripts are left alone.
- **Checked.** `cdept xschem link [library] [cell]` lists the symbols whose schematic
  xschem would not find (attribute missing, or naming a file that does not exist) or
  would find in another cell (a cell folder copied by hand). `--fix` writes the
  expected value; `--fix --foreign` also rewrites links to another cell. `cdept
  netlist` warns when the project has such symbols.

What CDePT cannot see, and `cdept xschem link` is for: a library folder renamed or
moved by hand, cell folders copied with `cp`, symbols brought in from another project.

Left for step 4 (inside the editor): xschem's own "make symbol from schematic" writes
the symbol next to the schematic, and "open the symbol of this schematic" looks there
too. Both need to go through CDePT's view folders.
