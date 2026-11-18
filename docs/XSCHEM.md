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
| 1 ✔ | GUI plumbing: `Tab::Document(DocId)` (dynamic tabs, layout persistence), double-click opens a view, a session per tab that starts and restarts the engine (`libcdept::xschem::Session`), frames through a memory-mapped file → texture (first with a test pattern). The Inspector preview moved to step 2: it shows pixels from the engine, not an SVG | Double-click opens a tab fed by the helper | 12–15 |
| 2 ✔ | xshim (Xlib on cairo) + Tk stand-in (`faketk.tcl`, §8), `has_x=1`. Draw, resize, pan, zoom, descend/ascend, select, highlight | **Read-only schematic viewer in a tab.** Gate, see below | 30–40 |
| 3 ✔ | Editing: full input mapping, menus, toolbar, message/prompt/property dialogs, save, undo, modified flag, close-with-unsaved | Draw and save a small schematic without leaving CDePT | 25–30 |
| 4 ✔ (mostly) | CDePT integration: insert instance through the library picker, new cell/symbol flows, netlist to a view, hierarchy data for the Hierarchy panel, selection → Inspector | Create an OTA from PDK devices and netlist it | 20–25 |
| 5 (partly) | Remaining dialogs (search, layers, simulation setup, graphs), several schematics at once, packaging in asic-all, upstream patches | Parity list checked against stock xschem | 30+ |

Total ≈ 130–160 h, more than ROADMAP gave all of Phase 3 (110 h). Steps 0–1 are useful
whatever happens: the headless engine replaces the planned TCP adapter for netlisting
and hierarchy.

**Gate after step 2.** Continue only if (a) a real PDK schematic renders the same as
stock xschem, (b) pan/zoom is smooth on a 4K tab, (c) the patch series on xschem is
short. Otherwise fall back to B for editing and keep the engine for netlist, hierarchy
and previews.

**Gate result (2026-11-18): passed.** (a) Compared with stock xschem of the same
commit on a headless X server (Xvnc): 39 of 44 pictures are identical pixel for pixel
(the example project, nine of xschem's examples, four IHP SG13G2 testbenches, and
states such as selection, highlight, rubber band, light colours); the other five
differ by one step of one colour value inside embedded bitmaps. (b) A full redraw at
3840x2160 takes 9 to 15 ms, a pointer move 20 to 65 µs. (c) No patch on xschem.

**State after steps 1 to 4** (details in §9): a schematic or symbol opens in a tab by
double-click; drawing, editing, xschem's own menus, tool bar, status bar, context
menu and dialogs work inside CDePT; files are chosen with CDePT's library picker;
closing asks through xschem's own question; the Hierarchy panel, the Inspector
(preview and selection) and "Netlist to View" use the engine in the background.

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
- **The user's `~/.xschem` is still used**: xschem reads and writes `recent_files`
  there (the user's `xschemrc` is not read), so files opened through CDePT show up in
  the "recent" list of the user's own xschem. Copy and paste between xschem windows
  also goes through this folder, which is a reason to share it. To decide in step 3:
  share it, or give the helper a folder of its own (`USER_CONF_DIR` in
  `xschem_config.h`).
- **Start-up race in xschem.** When `~/.xschem` is missing, xschem checks, then creates
  it, and exits if the folder appeared in between. Several helpers starting together
  on a fresh machine hit this (it failed the first CI run). The helper now creates the
  folder itself before starting xschem.
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

Inside the editor (step 4) the same holds:
- **Make symbol from schematic** (Symbol menu, key A; also `cdept xschem make-symbol
  <library> <cell> [--force]`). xschem's C code leaves the writing to a Tcl procedure,
  `make_symbol`, which runs xschem's `make_sym.awk`; that script writes `x.sym` next to
  `x.sch`. The helper replaces the procedure (`cdept.tcl`, no change to xschem): for a
  schematic in a view folder of a `cells` library it runs the same script on a copy in
  a scratch folder, puts the result at `<cell>/symbol/<name>.sym` and adds the
  `schematic=` attribute. Anywhere else xschem's own procedure runs, as before. The
  start-up file tells the helper which libraries have view folders
  (`cdept_cells_libraries`). As in stock xschem, the generated symbol replaces a drawn
  one (after a question) and puts the pins where the generator wants them.
- **Open the symbol of this schematic** and back look in the neighbouring view folder
  (`cdept::counterpart`).
- **Make schematic from symbol** (Symbol menu, Ctrl+L; also `cdept xschem
  make-schematic <library> <cell> [--force]`). xschem's C code (`create_sch_from_sym`)
  writes the file itself, a pin (ipin, opin, iopin) for each pin of the selected
  instance's symbol, into the file the symbol's `schematic=` attribute names; it does
  not replace `@symname`, so in a `cells` library it would fail with "file opening for
  write failed". The helper takes the key (`cdept::key_filter`) and the command
  (`xschem make_sch`, which the menu entry runs) before xschem's C code: for a symbol
  in a view folder of a `cells` library it writes the same text in Tcl
  (`cdept::make_schematic`), into the file xschem looks for when descending (the
  attribute with `@symname` replaced, or `<cell>/schematic/<name>.sch` without
  attribute), creating the folder. As in xschem, an existing schematic is replaced only
  after xschem's question, and the result is not opened; CDePT is told
  (`UiUpdate::Written`) and shows the new view. With nothing selected and such a
  symbol open, Ctrl+L makes the schematic of that symbol (stock xschem does nothing
  then). Other symbols are left to xschem's C code. The pins are not connected yet, so
  netlisting the schematic reports xschem's check errors (undriven output) until it is
  drawn.

## 8. Interfaces between the parts (steps 1 to 5)

One helper process per open schematic tab: each tab is its own xschem, as separate
xschem windows are. Copy and paste between tabs then works the way it does between
xschem instances (through the clipboard file in `~/.xschem`).

```
cdept-gui  views/schematic.rs      draws the frame, sends input, draws mirrored widgets
   │  libcdept::xschem::Session    thread per tab: starts and restarts the engine
   │  libcdept::xschem::Engine     one helper process; requests, replies, events
   ▼  socket (protocol.rs, JSON lines) + frame file (frame.rs, mapped by both)
cdept-xschem  src/main.rs          Tcl event loop; socket watched as a Tcl file event
   │  shim/cdx.h                   C functions the Rust side calls
   ├─ shim/engine.c                start-up, Tcl, event loop, `cdept::ui` command
   ├─ shim/display.c, xshim.c      stand-in Xlib on cairo image surfaces; Tk C stubs
   ├─ shim/tcl/faketk.tcl          stand-in Tk in Tcl: records widgets, reports them
   └─ xschem/src/*.c, xschem.tcl   unmodified
```

**Widgets: a stand-in Tk instead of per-dialog replacements.** xschem's menus, tool
bar, status bar and dialogs are Tcl/Tk code. `faketk.tcl` implements the Tk commands
in plain Tcl: it records the widgets xschem's code creates and their state, and
reports them (`cdept::ui <json>`, one `UiUpdate` of `protocol.rs`). CDePT draws them
with egui widgets and reports what the user does (`UiEvent`), which the helper turns
into `faketk::event <kind> <args>` calls. xschem's own Tcl code then runs unchanged,
including after an update of the submodule. Waiting for a dialog (`tkwait`, `vwait`)
works because the socket is served by the Tcl event loop, also when nested. Dialogs
that deserve a native version (file chooser → library picker) are replaced one by one
on top of this.

Conventions on top of `protocol.rs`:
- `faketk::event` kinds and arguments: `menu <menu path> <index>`, `popup_closed`,
  `configure <path> <width> <height>` (a picture pane's size),
  `clipboard <CLIPBOARD|PRIMARY> <text>`,
  `invoke <path>`, `text <path> <text>`, `select <path> <list of indices>`,
  `activate <path> <index>`, `key <path> <Tk keysym name>`, `focus <path>`,
  `close <toplevel path>`.
- A window whose root widget has class `FileDialog` asks for a file name
  (`tk_getOpenFile`, `tk_getSaveFile`, `tk_chooseDirectory`): options `mode`
  (`open`, `save`, `directory`), `initialdir`, `initialfile`, `filetypes`. CDePT shows
  its own chooser and answers with `UiEvent::Text { path: <window path>, text: <chosen
  path, empty for cancel> }`.
- A window whose root widget has class `LibraryPicker` asks for a schematic or symbol
  the CDePT way (it replaces xschem's own file browser, `load_file_dialog` and
  `file_chooser`): options `mode` (`open`, `save`, `symbol`, `merge`), `title`,
  `extensions` (space separated, e.g. `sym tcl`), `initialdir`, `initialfile`. CDePT
  shows its library / cell / view picker and answers with `UiEvent::Text { path:
  <window path>, text: <absolute file, empty for cancel> }`.
- A window whose root widget has class `MessageBox` is `tk_messageBox`: options
  `icon`, `type`, `message`, `detail`; its children are the buttons to show.

**CDePT's adjustments to xschem's Tcl side** (`shim/tcl/cdept.tcl`, read after
`xschem.tcl` and the start-up file):
- xschem's file browsers (`load_file_dialog`, `file_chooser`) open a `LibraryPicker`
  window instead; the originals stay reachable (`set cdept::native_file_dialogs 0`).
  Destinations of exports (PNG, SVG, PDF) use the ordinary `FileDialog`.
- One drawing area per helper. Everything that would open another window or tab
  inside the same xschem ("open in new window", "create new tab", descending in a new
  window, the keys Alt+E, Alt+I, Ctrl+T, …) reports `UiUpdate::Open { path }` and leaves
  the current design alone; CDePT opens that file as another document. Not redirected:
  `xschem load a b c` with several files (the extra ones become xschem tabs on the same
  drawing area).
- A symbol made from a schematic goes into the cell's `symbol` view and names its
  schematic (section 7); the helper reports `UiUpdate::Written { path }` and CDePT
  looks at the libraries again. This part also applies without a display.
- A schematic made from a symbol (Ctrl+L) of such a library goes where the symbol's
  `schematic=` attribute says (section 7); reported the same way.
- For CDePT to call: `cdept::symbol_file <schematic>` (where that symbol is written);
  `cdept::schematic_file <symbol>` and `cdept::make_schematic <symbol> ?yes?` (the
  other way round);
  `cdept::modified` (0 or 1); `cdept::close` (asks xschem's own
  "save changes?" question when modified, then exits with status 0, or returns 0 when
  cancelled); `cdept::selection` (`count N instances {{name symbol attributes}…} wires
  N texts N`).

**Test tool.** `target/debug/cdx-tcl [--display WxH] [--rcfile F] [--ppm out.ppm]
[-e script | file]...` runs Tcl in the engine without CDePT, prints results and
widget updates, and can dump the drawing window.

## 9. What steps 1 to 4 showed

- **Zero patches still holds.** xschem's C and Tcl sources are unmodified. Two things
  are done by the preprocessor in `xschem_config.h` instead: the Xlib names get an
  `xs_` prefix, and `srand` is replaced (xschem seeds its temporary folder names with
  the time in seconds, so engines started in the same second collided).
- **Pixel-exact drawing needed the X server's own algorithms.** Lines, arcs, dashes
  and polygons go through code taken unmodified from xorg-server (`shim/mi/`, MIT-style
  licences, notices kept); cairo draws text, images and fills. cairo alone was close
  but differed by single pixels at line ends and merged lines two pixels apart.
- **The Tk stand-in covers all of xschem.tcl.** A sweep test calls every procedure of
  xschem.tcl that builds a window (31) and 124 menu commands. Not exercised: the graph
  dialogs (they need a graph object) and two minor ones.
- **xschem measures text once, at the zoom in use when a file is loaded**, so "zoom
  full" can cut long text; stock xschem does the same. The helper measures again when
  a window first gets its size. A later "zoom full" (key `f`) is still xschem's own.
- **A question during start cannot be answered** (nobody is connected yet). Tk's own
  dialogs then return their default and any other window xschem waits for is logged
  and closed. Seen with the IHP `xschemrc`, which loads a start page that asks
  "execute scripts embedded in schematics?".
- **PDK start-up files.** `[xschem] rc = "…/xschemrc"` in `cdept.toml` names the
  PDK's file; it is read before the project's libraries are added. With the IHP file
  the PDK's own menu ("IHP") shows up in CDePT's menu bar, because menus are mirrored.
- **Requests behind a waiting one.** When xschem opens a dialog inside a request, the
  requests already read from the socket must still be served; an idle callback of the
  Tcl event loop does that.
- **Simulation and graphs, checked by hand** (buffer example and IHP's
  `tran_logic_not` with the PDK's `xschemrc`, ngspice 47; then `t_sim.tcl` and
  `cdept-gui/scripts/buffer-sim.script`, which need no simulator). Configure
  simulators, Netlist, Simulate (background and Fg), the job list, "View last job
  data/errors", reading raw files, adding a graph, its dialog (signals, raw file, sim
  type, ranges, colours), the wave colour dialog, cursors a/b, wheel zoom, drag, `f`
  and "Send highlighted nets to viewer" (no gaw: a message, no hang) all work: jobs are
  `open |cmd` pipes read by Tcl file events, and the helper's event loop serves them,
  also while a dialog or an Fg job waits. Fixed: `grid column` (an abbreviation Tk
  accepts) broke the wave colour dialog; lists in grid rows were laid out sideways,
  fields in grid cells stayed narrow, and the colour radio buttons showed no colour.
  xschem says nothing when a job it was not asked to report on fails, so a
  `$terminal -e ngspice` whose terminal cannot open a window failed silently;
  `cdept.tcl` now logs the end of every job in the console and shows a "Job failed"
  box when such a job ends with error output (a missing terminal program already gave
  xschem's own message). Still open (the `DISPLAY` and dialog height points were fixed
  afterwards, see below): the graph dialog's Del button deletes the signal under the
  text cursor, which CDePT does not report (edit the list instead); Waves > Tran and the
  graph's "Raw file" button use the desktop's file chooser. Not CDePT's: ngspice's batch
  mode (`-b -r file`) deletes a raw file of that name written by a `.control` block, as
  the IHP test benches do; use "Ngspice Control mode" for those.

- **Pictures in dialogs and the desktop's clipboard.** xschem draws a schematic or
  symbol into a Tk frame of some dialogs (`xschem preview_window`: its own file
  browser, the `file_chooser` symbol browser, and Help > Show Keybindings, whose whole
  window is the picture). The stand-in X already gave such a frame a window of its own;
  now the helper sends it like the drawing window, a frame file per picture
  (`Message::Preview`, sent again when its pixels change; `PreviewGone`), and CDePT
  draws it where the frame is, pixel for pixel. CDePT sizes it (as wide as the dialog,
  as high as Tk's `-height`) and says so with `UiEvent::Configure`: the window gets
  that size and the frame's `<Configure>`/`<Expose>` bindings make xschem draw again.
  `Tk_DestroyWindow` now destroys the widget too, as in Tk. With the library picker
  (the default) these browsers are not reached; the picker got a "Picture" column
  instead, drawn by a second preview engine like the Inspector's. Clipboard: Ctrl+V in
  a mirrored text field always worked on X11 (egui pastes itself; checked under Xvnc
  with `xclip`). What was missing was the rest of what Tk does: the middle button now
  pastes the desktop's primary selection into a field (read with arboard, the crate
  egui already uses), and xschem's Tcl gets the desktop's clipboard and primary
  selection (`clipboard get`, `selection get`): CDePT sends them
  (`UiEvent::Clipboard`) when a dialog opens, when the window gets the focus back and
  on every paste. Not checked on a Wayland desktop (no nested compositor here):
  there egui pastes through smithay-clipboard and arboard reads through Xwayland.
  Also: dialogs size to their content up to the tab's height, and in a taller one the
  part packed to expand scrolls while the rows before and after it (buttons) stay;
  a `DISPLAY` the helper made up for xschem.tcl is unset after start-up, so programs
  xschem starts see the real one.
  Tests: `t_preview.tcl` (panes need `CDX_DISPLAY`), `buffer-preview.script`, and the
  IHP `xschemrc` by hand (IHP symbols in xschem's browser).

Known gaps:
- xschem's "make schematic from symbol" does not work in a `cells` library
  (section 7).
- The GUI test driver cannot inject mouse and keys into detached windows.
- The native file chooser (export destinations, "Other File…") is written but was not
  run, because it opens a dialog on the desktop.
- Step 5 packaging: `scripts/install.sh` (layout `bin/cdept`, `bin/cdept-xschem`,
  `share/cdept-xschem/`, see `cdept-xschem/README.md`), CI installs libjpeg-dev and runs an
  install smoke test, and a draft `cdept` entry exists in asic-all's `tool_library.yaml`
  (not built in the image: private repository, needs a toolchain and network).
