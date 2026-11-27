# Testing the GUI with a script

`cdept gui --script <file>` runs the real program and, while it is on screen, executes
the commands of the file in order. It is how the embedded tools are tested end to end:
open a view, move the mouse, press keys, look at xschem's or KLayout's answers, take
screenshots.

These scripts run the real program with the environment you give them (the ones that need a PDK
name it: `PDK_ROOT=/opt/ext/OpenPDKs ...`). The Rust tests (`cargo test`) pin their environment
instead, see TESTS.md section 8.

```sh
XDG_DATA_HOME=$(mktemp -d) target/debug/cdept -p examples/buffer gui --script my.script
```

- **Exit status.** 0 when the script ran through; not 0 when a command failed (the
  message names the line). A script that does not end with `quit` leaves the window
  open.
- **Settings.** Always give `XDG_DATA_HOME` a scratch folder: the window layout and the
  list of recent projects live there, and a fresh folder gives the same layout (and so
  the same frame size) on every run.
- **Input.** Mouse and keyboard commands are fed to egui as input events, so they take
  the same path as a user's. The real mouse and keyboard are ignored while a script
  runs. After every step the driver waits until xschem has handled it.
- **Display.** A display is needed (the window is really drawn; screenshots read the
  window back). The window does not have to be visible or focused: with a script the
  program does not wait for the screen's refresh.
- **Messages.** Results appear in the log (stderr and the Console panel) with the
  target `cdept::script`. `-v` also shows each command as it starts, and what the
  xschem helper prints.

Scripts check the embedded xschem and KLayout on `examples/buffer`, and
`cdept-gui/scripts/run.sh` runs them with scratch settings, a scratch home folder and
a scratch copy of the project (examples/buffer; `CDEPT_EXAMPLE=<name>` takes another
folder of examples/):

| Script | Checks | Writes |
|---|---|---|
| `cdept-gui/scripts/buffer-smoke.script` | the picture, input, dialogs, the Inspector, the Hierarchy panel, closing a changed document | nothing |
| `cdept-gui/scripts/buffer-edit.script` | the library picker (open, insert a symbol, save as), "open in new window", netlist to a view, saving on close and on closing the project | into the project: run it on a copy |
| `cdept-gui/scripts/buffer-sim.script` | the simulator configuration, Netlist and Simulate as a background job (a copy command stands in for the simulator, so no ngspice is needed), failed jobs, a waveform graph with its dialog, cursors and zoom | a netlist, a raw file and a graph into the project |
| `cdept-gui/scripts/buffer-preview.script` | pictures xschem draws inside its own dialogs (key cheat sheet, its file browser), pasting the desktop's clipboard into a dialog, Tcl's `clipboard get` / `selection get` (run it by name: `run.sh cdept-gui/scripts/buffer-preview.script`) | nothing |
| `cdept-gui/scripts/layout-smoke.script` | the embedded KLayout: the picture, layers and cells (external cells), modes, notices, drawing, undo/redo, save (sidecar), Place Cell, self-contained export, opening an external cell, closing a changed layout (needs `cdept-klayout/.bin/klayout`, see cdept-klayout/README.md) | into the project: run it on a copy (`run.sh cdept-gui/scripts/layout-smoke.script`) |
| `cdept-gui/scripts/layout-inplace.script` | edit in place: descend into an external cell (Shift+X), draw, save writes both files, the other tab reloads | into the project: run it on a copy |
| `cdept-gui/scripts/layout-properties.script` | the properties dialog (box, instance, PCell parameters), Place Library Cell (Basic CIRCLE), box by dragging | into the project: run it on a copy |
| `cdept-gui/scripts/layout-pcells.script` | PCells: the project's PCell view `logic/rpoly` placed by `logic/rdiv` (its parameters), a shape parameter (Basic CIRCLE's handle), another cell for an instance (Change… › Library Cell / Layout View), placing a project PCell | into the project: run it on a copy |
| `cdept-gui/scripts/layout-checks.script` | Run DRC / Run LVS, the results panel, highlight and zoom (needs `PDK_ROOT=/opt/ext/OpenPDKs PDK=ihp-sg13g2` or similar; not in the default set) | nothing |
| `cdept-gui/scripts/lef-abstract.script` | a LEF abstract (IHP `sg13g2_inv_1`) in the embedded KLayout: read-only notice, one macro of a many-macro file shown, layers per purpose, drawing modes / Ctrl+S refused, another macro in another tab (needs the PDK: `PDK_ROOT=/opt/ext/OpenPDKs CDEPT_EXAMPLE=ihp-inv run.sh cdept-gui/scripts/lef-abstract.script`) | nothing |
| `cdept-gui/scripts/layout-nets.script` | nets on examples/ihp-inv: Update Connectivity, flylines, a net shown in the layout and highlighted in the schematic, a wire selected in the schematic highlighted in the layout, probing, the flyline style, a short traced live, out of date after an edit with live flylines off, the schematic saved before Update Connectivity (needs the PDK: `PDK_ROOT=/opt/ext/OpenPDKs CDEPT_EXAMPLE=ihp-inv run.sh cdept-gui/scripts/layout-nets.script`) | into the project: run it on a copy |
| `cdept-gui/scripts/layout-pins.script` | pins on examples/ihp-inv: the Pins panel, a pin removed and undone, added on Metal1, renamed, shown, deleted, the net check's pin statuses; cross-probing through the hierarchy: a device inside x1 selected in buf's layout and in buf's schematic, inside x1 after xschem descended, inv's own layout following the schematic shown inside x1, a net inside x1 highlighted in buf's layout (needs the PDK: `PDK_ROOT=/opt/ext/OpenPDKs CDEPT_EXAMPLE=ihp-inv run.sh cdept-gui/scripts/layout-pins.script`) | into the project: run it on a copy |
| `cdept-gui/scripts/hierarchy.script` | the Hierarchy panel read from the files (schematic and layout hierarchy, Used By), a cell renamed while its schematic is open (plan, files, tab opened again), a taken name, a cell moved to another library, a library renamed with its folder | into the project: run it on a copy |
| `cdept-gui/scripts/layout-link.script` | linking buf's schematic with its layout: Generate takes over the inv instances, the Link panel, cross-probing both ways, undo | into the project: run it on a copy |
| `cdept-gui/scripts/watch.script` | the project follows the disk and git: cells made, renamed and removed by other programs, `cdept.toml` edited by hand, git marks (untracked, added, modified, clean) per view, cell and library, Delete to the trash and for good, a schematic whose file changes behind its back (opened again, or Reload / Keep Mine), the settings (needs git; `run.sh cdept-gui/scripts/watch.script`) | into the project, which becomes a git repository: run it on a copy |
| `cdept-gui/scripts/watch-layout.script` | the same for layout tabs: a tab's own save is no change behind its back, a file written by another program is opened again in every tab showing it (also the tabs placing the cell), a tab with changes asks | into the project: run it on a copy |
| `cdept-gui/scripts/cace.script` | Import CACE Datasheet… and Export CACE Datasheet… on `ihp-inv` (plan and report, the testbench and suite written, the datasheet exported; writes into the project and `../incoming`: run it on a copy, `CDEPT_EXAMPLE=ihp-inv run.sh cdept-gui/scripts/cace.script`) | into the project and next to it |
| `cdept-gui/scripts/import-files.script` | Import Files…: a GDS and a SPICE file from outside the project become the views of two cells of `logic` (prefix, plan lines, unknown files and a missing subcircuit reported), the same files again (nothing left to import, then imported under another name), a file kept whole (writes into the project and `../incoming`: run it on a copy, `run.sh cdept-gui/scripts/import-files.script`) | into the project and next to it |
| `cdept-gui/scripts/libdefs.script` | library definitions: Library Properties (read-only, layout, description, folder), Add Existing Library with its folder field, Remove Library, the project's problems; CDePT's own path prompt for Open Project and New Project (a wrong folder refused, cancel) | into the project and next to it: run it on a copy (`run.sh cdept-gui/scripts/libdefs.script`) |
| `cdept-gui/scripts/technology.script` | the New Project wizard (PDKs found, scan, summary, libraries, Create), the Technology tab and its pages, a PDK symbol opening with the PDK's `xschemrc` in effect, Rescan with a tab open (the "Replace tech.toml?" question, then the offer to reopen it), listing and leaving out a PDK library (needs IHP SG13G2: `PDK_ROOT=/opt/ext/OpenPDKs run.sh cdept-gui/scripts/technology.script`; the `tech <verb>` commands are listed at the top of the script) | a new project next to the copy |
| `cdept-gui/scripts/simulation.script` | the Simulation tab on examples/ihp-inv: tb_inv opening in it (double click), the setup form (corners from tech.toml, validation, save keeping the file's comments), Run / Cancel through the job queue, 3 PASS rows, FAIL rows, Run from the cell's context menu (needs the PDK, ngspice and the helper: `PDK_ROOT=/opt/ext/OpenPDKs CDEPT_NGSPICE=/opt/ext/OpenLane/bin/ngspice CDEPT_XSCHEM=target/debug/cdept-xschem CDEPT_EXAMPLE=ihp-inv run.sh cdept-gui/scripts/simulation.script`; the `sim <verb>` commands are listed at the top of the script; `sim wait` is tried for 2 minutes) | `setup.toml` and `simulations/` of the copy |
| `cdept-gui/scripts/vacask.script` | VACASK from the Simulation tab on examples/ihp-inv: tb_inv/testbench switched to `simulator = "vacask"` in the form and saved (comments kept), three corners run (PASS rows, `deck.sim` and `tran1.raw` in the point folders; the first run converts the PDK's models, a minute), a failing spec, then 6 Monte Carlo runs each of process, mismatch and both with statistics, and back to ngspice (needs the PDK, VACASK, OpenVAF, python3 and the helper: `CDEPT_VACASK=/opt/ext/OpenLane/bin/vacask` besides what simulation.script needs; set `CDEPT_SCRIPT_TIMEOUT` to 600) | `setup.toml` and `simulations/` of the copy |
| `cdept-gui/scripts/magic.script` | Magic in a tab on examples/sky130-inv: the picture, Magic commands from the command line and their messages, DRC of the cell as it is (results, box and zoom), undo, save, a key macro over the drawing, Generate from Schematic, cross-probing of instances with the schematic tab (needs Magic, sky130A and both helpers: `PDK_ROOT=/opt/ext/OpenPDKs CDEPT_MAGIC=/opt/ext/OpenLane/bin/magic CDEPT_XSCHEM=target/debug/cdept-xschem CDEPT_EXAMPLE=sky130-inv run.sh cdept-gui/scripts/magic.script`) | into the copy |
| `cdept-gui/scripts/magic-torture.script` | harder Magic cases on examples/sky130-inv: a cell placing a missing cell (red line), two Magic tabs and a schematic at once while the window changes size, Ctrl+S, unsaved changes on close (Cancel, Discard), Restart (same environment as magic.script) | into the copy |
| `cdept-gui/scripts/open-with.script` | a double click on a GDS asks for the tool (four choices), Open With › Magic (tab) shows the GDS read-only with the PDK's set-up (needs Magic, IHP SG13G2 and the Magic helper: `CDEPT_EXAMPLE=ihp-inv`) | nothing |
| `cdept-gui/scripts/uberwave.script` | "Open in UberWave" on examples/ihp-inv: after a run, `wave open <lib> <cell> <view> <point>` (what the button does, a script cannot press a button by its text) starts a stand-in viewer, `cdept-gui/scripts/fake-uberwave.sh`, which records its arguments and working folder in the point's folder: the raw path, and `--analysis tran` for a file with two analyses (set `CDEPT_UBERWAVE` to the stand-in, plus what simulation.script needs: `CDEPT_UBERWAVE=$PWD/cdept-gui/scripts/fake-uberwave.sh CDEPT_EXAMPLE=ihp-inv run.sh cdept-gui/scripts/uberwave.script`) | `simulations/` of the copy |
| `cdept-gui/scripts/vacask-unavailable.script` | VACASK on a technology it cannot run (sky130A, a project made with `cdept init --technology sky130A` in a scratch folder): the simulator combo's VACASK entry is disabled (`sim expect_simulator_disabled vacask sky130A`, `sim select_simulator vacask` fails and the form keeps its simulator), a testbench file that names `simulator = "vacask"` shows the problem line (`sim expect_problem VACASK is not available`) and the run is refused; ngspice stays selectable. New verbs: `sim expect_simulator_disabled <name> [reason text]`, `sim expect_simulator_enabled <name>` and `sim select_simulator <name>` (what picking the combo entry does; an error for a disabled one). Needs the PDK sky130A (`PDK_ROOT=/opt/ext/OpenPDKs`) and the helper; VACASK itself is not needed | a scratch folder |
| `cdept-gui/scripts/sweeps.script` | the Sweeps and Monte Carlo sections of the Simulation tab on examples/ihp-inv: a vdd sweep and Monte Carlo added in the form (live validation, run plan count, VACASK and Xyce notes), saved with the file's comments kept, 15 PASS points, Grid, Revert (needs the PDK, ngspice and the helper like simulation.script; the `sim add_sweep` / `set_sweep` / `remove_sweep` / `mc` / `set_mc` / `expect_sweep` / `expect_plan` / `expect_mc_note` commands are listed at the top of the script) | `setup.toml` and `simulations/` of the copy |
| `cdept-gui/scripts/uberwave-calc.script` | Families and calculator traces on examples/ihp-inv with a stand-in that speaks IPC protocol 1, `cdept-gui/scripts/fake-uberwave-ipc.py` (Python 3 standard library; logs every request to `$FAKE_UBERWAVE_LOG`): `wave family`, the second open reusing the viewer, `wave calc` of `v("out")` on a point and `db20(v("out"))` on all, a scalar, `wave expect_*` checks (`FAKE_UBERWAVE_LOG=$(mktemp) CDEPT_UBERWAVE=$PWD/cdept-gui/scripts/fake-uberwave-ipc.py CDEPT_EXAMPLE=ihp-inv run.sh cdept-gui/scripts/uberwave-calc.script`) | `simulations/` of the copy |
| `cdept-gui/scripts/uberwave-embedded.script` | The EMBEDDED UberWave next to the GUI window (needs UberWave built in and a display): after a run, `wave family` opens the viewer window in the cdept process, a second open reuses it, calculator traces go in as raw data, `wave embedded_files` / `embedded_signal` check what the viewer holds through its list calls, `wave embedded_screenshot` saves the window (checked: a PNG with content), and after `wave embedded_close` the next open starts a new window. `uberwave.script` and `uberwave-calc.script` start with `wave backend process`: they test the fallback |
| `cdept-gui/scripts/sar-enob.script` | The Phase 5 milestone on examples/ihp-inv (needs UberWave built in, the PDK, ngspice, the helper): tb_sar8/enob run from the Simulation tab, ENOB and SNDR by the calculator panel (`sim calc expr/evaluate/expect_value`), the code spectrum and the sampled codes sent to the built-in UberWave, an SFDR output added with a spec from the panel, saved and run again (3 PASS), an output reading `cursor_a` evaluated when the viewer's cursors move (`sim calc cursors`). The `sim calc` verbs are listed at the top of the script (`views/simulation/calc.rs`); `sim setup_section open|fold` opens or folds the Setup section | `setup.toml` and `simulations/` of the copy |
| `cdept-gui/scripts/tests.script` | test templates and suites in the Simulation tab on examples/ihp-inv: a templated testbench (parameters, wrong values as the tab's problem, re-expansion, the saved file keeping only the template and own fields), switching template, the suite view running four tests into one table (needs the PDK, ngspice and the helper, like simulation.script; `sim template_*` / `sim suite_*` commands are listed at the top of the script) | `setup.toml` and `simulations/` of the copy |

| `cdept-gui/scripts/wave/run.sh` | CDePT's own waveform tabs (`wv …`, below), on generated inputs under `target/wave-test`, release build, each GUI run capped at 6 GB. `smoke` (small.py; what CI runs): open, show, zoom, a cursor. `menu`: File › Open in UberWave… with the built-in viewer and the process (fake-uberwave.sh), errors as a dialog. `features`: panes (new, remove, reorder, drag from the list), the list's groups, search and selection, a spectrum in Hz going to a pane of its own, hostile series (empty, NaN, Inf, x that goes back, 5 million points), styles. `family`: 12 files overlaid with different x grids, a garbage, a missing and a cut one, per-point show/hide, distinct styles, unicode and 200-character names. `big` (big.py): 600 run points with 15 broken, 1770 lines, 300 panes, files changed while open, a detached tab, a family closed while it loads, 15 opens and closes of 100 files. `analyses` (analyses.py, analyses-sim.sh with `$CDEPT_NGSPICE`, `$CDEPT_XYCE`, `$CDEPT_VACASK`; the parts whose simulator is missing are left out): AC files of the three simulators and ascii/binary, the complex forms, Bode with margins against ngspice's `.meas`, an eye as lines and density and absurd periods, ngspice's six plots of one file (parametric DC per part, the op table, noise log-log and integrated against `onoise_total`), sweeps there and back, the PSF fixtures, hostile complex files (zeros, NaN/Inf, 0 Hz and negative frequencies, unicode names, cut, lying, half pairs), a family of mixed analyses, 15 opens and closes of an AC file and a PSF folder. `huge` (huge.py, needs numpy): a binary file of `HUGE_POINTS` points (default 100 million) under `HUGE_DIR` (default `target/wave-test/huge`), a cancelled first open, the conversion, zoom and pan times, a one-sample spike at every zoom, the cache reused; deleted at the end unless `KEEP_HUGE=1`, the GUI capped at `HUGE_MEMORY` (12G). `torture`, `loop`, `sar`: hostile raw files, caches, a file that changes and goes, a read-only folder, a non-ASCII path under `LC_ALL=C`, a one-sample spike in 10 million samples, two tabs indexing one file, closing and cancelling loads, 60 opens and closes, and the 845 MB SAR ADC file (unzipped into `target/sar`, removed at the end unless `KEEP_SAR=1`). `cdept-gui/scripts/wave/run.sh [smoke\|menu\|features\|family\|analyses\|big\|torture\|loop\|sar\|huge]` | `target/wave-test`, `target/sar` |
| `cdept-gui/scripts/wave-tabs.script` | The waveform tabs with a real testbench (examples/ihp-inv; PDK, ngspice and the helper like simulation.script, `CDEPT_SCRIPT_TIMEOUT=600`): `wv open_family` (the button "Waveform Tab (family)"), `sim calc send_tab` ("Send to tab": a waveform of every point, a spectrum into a pane of Hz, markers of a scalar, -inf values), the spec table's cursor outputs following a tab (`sim calc follow`), the eye layout sent to a tab, a sweep with Monte Carlo opened as a family (the point folders of the first plan left out), the Bode layout of the OTA's open-loop AC (cells/ota5t ac_open) sent to a tab with its margin markers next to `phase_margin` of the calculator | `setup.toml` and `simulations/` of the copy |

`$PROJECT` in a script stands for the folder of the project being opened.

## Coordinates

Positions are in **pixels of the frame** of the schematic or layout the script acts on: (0, 0) is
the top-left corner of the drawing. Positions outside the drawing work too (negative
numbers reach the panels to the left and above), which is how a script clicks other
parts of the window.

Without a document on screen, positions are window coordinates in points.

Pixel positions depend on the size of the drawing. Prefer `click_instance`, which asks
xschem where the instance is.

The script acts on the active document (a schematic or a layout) of the main window (else of a detached window,
else the document opened last). Mouse and keyboard commands only work in the main
window; `tcl`, `wait_frame`, `frame` and `pixel` work in detached windows too.

## Commands

One command per line. Empty lines and lines starting with `#` are ignored. An argument
with spaces that is not the last one is written in braces: `{Instance name}`.

**Documents**

| Command | Does |
|---|---|
| `open <library> <cell> <view>` | Open the view in a document tab, or bring its tab to the front |
| `close` | Close the document, discarding changes |
| `close_tab` | Close it the way the X on its tab does. A changed document asks first: xschem's own "Ask Save" dialog (answer it with `dialog_click Yes`, `No` or `Cancel`) |
| `close_answer <save\|discard\|cancel>` | Answer CDePT's own question, which is only asked when the xschem engine cannot ask itself |
| `detach` | Move the document into a window of its own |
| `expect_documents <n>` | Fail unless `n` documents are open |
| `expect_title <text>` | Fail unless the document's tab has this title: `cell (view)`, or `buf (in tb_buf)` after descending; with `*` when changed |

**Waiting**

| Command | Does |
|---|---|
| `wait <seconds>` | Wait |
| `wait_frame` | Bring the document to the front and wait until xschem's picture is on screen |
| `sync` | Wait until xschem has handled everything sent so far |

**Mouse and keyboard**

| Command | Does |
|---|---|
| `move <x> <y>` | Move the pointer |
| `move_by <dx> <dy>` | Move it relative to where the script left it |
| `click <x> <y> [left\|middle\|right] [ctrl] [shift] [alt]` | Move, press and release (with the keys held: `click 10 20 left ctrl` is a Ctrl+click) |
| `click_here [button]` | Click where the pointer is |
| `click_instance <name> [button]` | Click the middle of an instance of the schematic |
| `double_click <x> <y>` | Two clicks |
| `press <x> <y> [button]`, `release <x> <y> [button]` | The two halves of a click, for looking at the picture in between |
| `drag <x1> <y1> <x2> <y2> [button]` | Press, move in steps, release |
| `wheel <x> <y> <up\|down> [count]` | Turn the wheel |
| `key <name> [ctrl] [shift] [alt]` | Press and release a key: a letter or digit, or `Escape`, `Enter`, `Tab`, `Space`, `Delete`, `Backspace`, `Insert`, `Home`, `End`, `PageUp`, `PageDown`, `Left`, `Right`, `Up`, `Down`, `F1`…`F12` |
| `text <string>` | Type characters |

**xschem**

| Command | Does |
|---|---|
| `tcl <script>` | Evaluate Tcl in xschem and log the result |
| `tcl_expect <result> <script>` | Fail unless the result is exactly `<result>` (use braces: `{{x1}}` is the Tcl list `{x1}`) |
| `tcl_send <script>` | Evaluate without waiting for the result (for scripts that block, or end xschem); the selection is read again afterwards |
| `menus` | Log every entry of xschem's menu bar as `Menu / Entry` |
| `menu <label> / <label> …` | Choose a menu entry by its labels (trailing dots can be left out) |
| `dialog_click <button text>` | Press a button of the topmost dialog |
| `dialog_type <path or {label}> <text>` | Replace the text of a field, named by its Tk path or by the label in front of it |
| `expect_dialog <title>` | Fail unless a dialog whose title contains `<title>` is open |
| `expect_pane <path>` | Fail unless a dialog shows a picture xschem drew (a preview) in the widget at this Tk path, at the size CDePT asked for |
| `clipboard <text>`, `primary <text>` | What the desktop's clipboard and primary selection hold from now on, for CDePT and xschem (`{text}` keeps spaces at the ends); `key v ctrl` pastes the clipboard |

**Library picker** (where xschem asks for a schematic or symbol)

| Command | Does |
|---|---|
| `expect_picker <open\|save\|symbol\|merge>` | Fail unless the picker is open for this purpose |
| `picker <library> <cell> <view>` | Answer it with the file of this view |
| `picker_file <file>` | Answer it with this file (for "save as"; the folder is created) |
| `picker_cancel` | Cancel it |

**Panels and project**

| Command | Does |
|---|---|
| `panel <title>` | Bring a panel to the front: `Libraries`, `Hierarchy`, `Inspector`, `Tools`, `Console`, … A panel only works while it is in front |
| `select <library> [cell] [view]` | Select in the library views, as a click there does |
| `wave open <library> <cell> <view> <point>` | What the Results list's "Open in UberWave" does: start `$CDEPT_UBERWAVE` on the point's `out.raw`. Fails when it cannot start or there is no raw file. Point `CDEPT_UBERWAVE` at `cdept-gui/scripts/fake-uberwave.sh` (an older viewer, no IPC) to check the arguments |
| `wave family <library> <cell> <view> [point]` | "Open family in UberWave": all the points (or the one) in one viewer; the viewer of the run is reused when it runs. With `fake-uberwave-ipc.py` |
| `wave calc <library> <cell> <view> <point\|all> <expr>` | What the expression line's Send does: evaluate `<expr>` (no spaces) on the point or every point and push the waveforms. Fails on a calculator error or a refused push |
| `wave backend <embedded\|process>` | Chooses the UberWave backend for the rest of the script (as `cdept wave --backend`); `embedded` fails in a cdept without UberWave built in. |
| `wave embedded_files <tag>...` / `wave embedded_signal <file> <name>` | The embedded viewer's session has files with these tags / a signal of that file (`(external)`: a trace pushed by the calculator). |
| `wave embedded_screenshot <png>` / `wave embedded_close` | Save the embedded window as PNG (absolute path or relative to the working folder; fails when it is empty/dark) / close it. |
| `sim calc <verb> ...` | The calculator panel: `expr <text>`, `point <name|all>`, `plot <kind|any>`, `layout plain|bode|eye <period> [offset]`, `markers on|off`, `evaluate`, `send` (both wait up to 5 minutes), `expect <point|*> <text>`, `expect_value <point> <min> <max>`, `expect_note <text>`, `add_output <name> [min] [max]`, `cursors <a> <b>` (sets the built-in viewer's cursors and waits for its report), `expect_cursor_output <name> <min> <max>`, `expect_cursor_signal <name>`, `functions <filter>`, `signals <filter>` |
| `wave expect_start <text>` / `wave expect_request <cmd> [text]` / `wave expect_trace <name> <n\|>n>` | Checks of the log of `fake-uberwave-ipc.py` (`$FAKE_UBERWAVE_LOG`, JSON lines): the viewer was started with `<text>` in its record, got a `<cmd>` request, got `add_trace` with that name (the name may hold spaces) and n points (`>n`: more than n) |
| `wave quit_viewer <library> <cell> <view>` | Send `quit` to the run's viewer |
| `expect_hierarchy <line>` | Fail unless the Hierarchy panel has this line: `x1 buf` is instance `x1` of cell `buf` |
| `hierarchy_open <cell>` | Open the schematic of a cell of the Hierarchy panel, as a double-click there does |
| `expect_preview` | Fail unless the Inspector shows the picture of the selected view |
| `expect_selected <instance\|none>` | Fail unless the Inspector shows this instance as selected in the schematic (or nothing) |
| `netlist <library> <cell> [format]` | Write the netlist of the cell into its `netlist` view (`spice`, `spectre`, `verilog`, `vhdl`, `tedax`) |
| `expect_view <library> <cell> <view>` | Fail unless the project has this view |
| `close_project` | Close the project, as File > Close Project does: with changed documents it asks first |
| `quit_ask` | Quit as File > Quit does: with changed documents it asks first |
| `leave_answer <save\|discard\|cancel>` | Answer the question about changed documents |
| `expect_project <name\|none>` | Fail unless this project is open |

**Looking**

| Command | Does |
|---|---|
| `screenshot <file.png>` | Save the main window as a PNG |
| `check_picture` | Fail unless the drawing on screen is the frame, pixel for pixel (nothing scaled or shifted) |
| `frame` | Log the size of the frame |
| `move_middle` / `click_middle` | The pointer to the middle of the active picture (with a click), whatever its size: zoom on a point first (`layout_call zoom`) |
| `pixel <x> <y>` | Log the colour of a frame pixel |
| `expect_pixel <x> <y> <rrggbb>` | Fail unless the pixel has this colour |

**Window**

| Command | Does |
|---|---|
| `window <width> <height>` | Resize the main window (points) |
| `zoom <factor>` | Scale the whole GUI, e.g. `1.5`: the way to test screens with more pixels per point |
| `log <text>` | Write a line to the log |
| `quit` | End the program, without asking about changed documents |

## Layouts (KLayout)

The commands of the sections above that do not mention xschem work on a layout
document too: `open <library> <cell> <layout view>`, `close`, `close_tab`,
`close_answer`, `detach`, `expect_documents`, `expect_title`, `wait_frame`, `sync`
(waits until KLayout has handled everything sent so far), `move`, `click`,
`double_click`, `press`, `release`, `drag`, `wheel`, `key` (`F2`, `Plus ctrl`, `Delete`,
`Escape`), `text`, `frame`, `pixel`, `expect_pixel`, `check_picture`, `screenshot`.
Pixel positions are those of the picture. The xschem commands (`tcl`, `menu`,
`click_instance`, dialogs, the picker) fail on a layout. Note that KLayout's box, polygon
and path tools take **clicks**; a box can also be dragged (press, move, release).

| Command | Does |
|---|---|
| `mode <name>` | Switch the editor: `select`, `move`, `box`, `polygon`, `path`, `text`, `instance`, `ruler`, `partial` |
| `layout_zoom <fit\|in\|out>` | Zoom |
| `layout_edit <action>` | `undo`, `redo`, `delete`, `select_all`, `clear_selection`, `copy`, `cut`, `paste` |
| `save` | Save and wait for the answer (the sidecar of the references is written too) |
| `place <library> <cell> <view>` | Place Cell: load that layout view as an external cell and switch to instance mode |
| `export <file>` | Write the self-contained stream (every reference filled in) |
| `add_layer <layer> <datatype> [name]` | Add a layer (the form of the Layers panel) |
| `current_layer <name or source>` | Choose the layer to draw on; `1/0` finds `1/0@1` |
| `layer_visible <name or source> <on\|off>` | Show or hide a layer |
| `show_cell <cell> [<cell> ...]` | Show a cell, given as the path from the top (double click on a local cell) |
| `cell_open <cell>` | What a double click on a cell of the Cells panel does: a local cell is shown, an external one opens its view in a tab of its own |
| `levels <n>` | Hierarchy levels shown (16 and more: all) |
| `layout_call <method> [<json>]` | Call a helper method (docs/KLAYOUT.md §4) and log the answer |
| `expect_notice <text>` | Fail unless a message of the editor (the transient line over the picture, also in the Console) contains the text |
| `expect_layer <name or source>` | Fail unless the layer list has it |
| `expect_cell <name> [local\|external\|missing]` | Fail unless the cell tree has this cell (of this kind) |
| `expect_shown <cell>` | Fail unless the tab shows this cell (a view that is one cell of a file holding several, e.g. a standard cell of a PDK GDS) |
| `expect_selection <n>` | Fail unless `n` objects are selected |
| `expect_modified <yes\|no>` | Fail unless the layout does (not) have unsaved changes (the tab's star) |
| `expect_mode <name>` | Fail unless the editor is in this mode |
| `expect_in_place <library/cell/view\|none>` | Fail unless that external cell is being edited in place (Shift+X) |
| `properties` / `properties_apply` / `properties_ok` | Open / apply / apply and close the properties of the selection |
| `set_property <field> <json>` | Change a field of the properties form (`x`, `bbox.2`, `array.na`, `pcell.<name>`) |
| `expect_property <field> <json>` | Fail unless a field of the properties form has this value |
| `place_library <library> <cell> [<json values>]` | Instance mode with a cell or PCell of a KLayout library (PDK, Basic) |
| `replace_library <library> <cell> [<json values>]` | The instance the properties show places that KLayout library cell or PCell instead (Change… › Library Cell…), keeping its place |
| `replace_view <library> <cell> <view>` | The same with a layout view of the project (Change… › Layout View…) |
| `nets_update` | Update Connectivity: extract the layout and compare its nets with the schematic's (needs the PDK) |
| `expect_nets <text>` | Wait until the nets are compared and fail unless the summary (`2 ok, 2 open, 0 shorted`, `, clean`, ` (out of date)`, ` (live)`) contains the text; while the schematic is saved first it is `saving the schematic` |
| `nets_show <net>` | Highlight a schematic net in the layout, as a click in the Nets panel does |
| `nets_probe <x> <y>` / `nets_probe center` | Probe the net at a frame pixel (or the middle of the picture), as a click in probe mode does |
| `expect_net_probed <nets…\|none>` | Fail unless these schematic nets were last probed or shown in the layout |
| `flylines <on\|off>` / `expect_flylines <n>` | Show or hide the flylines; fail unless the helper drew n |
| `flyline_style <key> <value>` | Change the user's flyline setting as the Style window does: `color`, `highlight`, `short` (`#rrggbb`), `width`, `opacity` (percent), `line` (`solid`, `dashed`, `dotted`, `dash-dotted`), `live` (`on`/`off`), `delay_ms` |
| `expect_flyline_style <key> <value>` | Fail unless the helper drew the flylines with this setting |
| `parallel <n> <on\|off>` | Join the n-th parallel group (from 1) or not, as its checkbox in the Nets panel does |
| `expect_parallel <groups…\|none>` | Fail unless the comparison found these parallel groups: members joined by commas, `!` after a group that is not joined |
| `expect_schematic_nets <nets…\|none>` | Fail unless the active schematic last exchanged these nets with a layout (cross-probing); names are net paths from the tab's own schematic (`x1/A` while xschem shows the inside of x1) |
| `layout_check <drc\|lvs>` | Run DRC or LVS on the open layout (needs the PDK in the environment) |
| `expect_check <text>` | Wait until the last check is done and fail unless its summary contains the text |
| `check_show <n>` | Highlight DRC result n (from 1), as a click in the results panel does |
| `link_generate [remove]` | Generate / Update from Schematic (docs/LINKING.md); `remove` also deletes instances whose schematic instance is gone |
| `link_select <instance> ...` | Select instances by schematic name, as a click in the Link panel does (the schematic tabs follow); a name may be a path through the hierarchy (`x1/M1`: the device inside the placed x1) |
| `expect_link <instance> <status>` | Fail unless the Link panel shows this status (`ok`, `missing`, `parameters`, `other cell`, `twice`) |
| `expect_probed <instance ...\|none>` | Fail unless these names (instance paths, `x1/M1`) are bound to the layout's selection, as cross-probing last saw them |
| `pins_panel <on\|off>` | Show or hide the Pins panel (docs/LINKING.md §9) |
| `pin_add <name> <direction> <layer> <l> <b> <r> <t>` | Add a pin with a box (µm) on a conductor (`8/0`); direction `input`, `output`, `inout` (or `i`, `o`, `io`) |
| `pin_from_selection <name> <direction>` | The selected shapes become pin `name`, as the panel's From Selection does |
| `pin_draw <name> <direction>` | Draw Pin: the next box drawn on the current layer becomes the pin |
| `pin_remove <name>` / `pin_rename <name> <new>` / `pin_show <name>` | Delete, rename, select and show a pin |
| `expect_pin <name> <status>` | Fail unless the Pins panel shows this status: `placed`, `not placed`, `not in the schematic`, `twice`, `label off the pin`, `other direction`, what the net check says (`the pin is on no wire`, …), or `absent` |

KLayout reports a changed modified flag with the next request it handles, so the
`expect_*` commands above ask it for a sign of life while they wait.

## Magic tabs and opening views

`magic <verb> …` acts on the active Magic tab (docs/MAGIC.md): `command <magic
command>`, `save`, `drc`, `drc_show <n>`, `generate`, `layers on|off`; the checks (tried
again for a while) `expect_saved`, `expect_modified 0|1`, `expect_drc <n>` or `>n`,
`expect_generated`, `expect_log <text>`, `expect_cell <name>`, `expect_selected
<names…>`, `expect_missing <n>`. `tcl`, `tcl_send` and `tcl_expect` run Tcl in Magic when
the active document is a Magic tab.

`open_view <lib> <cell> <view>` does what a double click does (it may ask which tool);
`expect_open_choice <n>` checks that the question offers n tools and closes it;
`open_with <lib> <cell> <view> <tool>` opens with one tool (`klayout`, `magic`,
`xschem`, or a window: `magic-window`, …). `open` opens in the first tab that can show
the view, without asking.

## Library definitions and path prompts

CDePT's own dialogs for libraries, and its path prompt (the window with a path field
that asks for a folder or file; `cdept-gui/src/pathfield.rs`). The system's file dialog
cannot be answered by a script, CDePT's prompt can. `CDEPT_NATIVE_DIALOG=off` in the
environment makes CDePT behave as where there is no system dialog: every question for
a path becomes a prompt.

| Command | Does |
|---|---|
| `lib_properties <library>` | Open Library Properties for a library |
| `pick_folder <open\|new\|library>` | What File › Open Project, New Project (the plain folder question) and Library › Add Existing Library do: the first two open the path prompt, the third the Add Existing Library dialog |
| `lib_set <field> <value>` | Set a field of the open library dialog: `path`, `read_only` (`yes`/`no`), `layout` (`cells`, `flat`, `ref`, `detect`), `description` in Library Properties; `name`, `path`, `read_only`, `layout` in Add Existing Library; `name`, `path` in New Library |
| `lib_save` | Confirm the open dialog; fails with the dialog's error |
| `lib_save_error <text>` | Confirm it and fail unless it is refused with an error containing the text (the dialog stays open) |
| `lib_cancel` | Close the open dialog |
| `lib_remove <library>` | Remove Library, confirmed |
| `expect_lib_error <text>` | Fail unless the open dialog shows an error containing the text |
| `expect_library <name> <key> <value>` | Fail unless the library has this: `present` (`yes`/`no`), `path` (as written), `folder` (on disk), `layout`, `layout_written` (`detect` when not written), `read_only`, `description`, `cells`, `origin` (`project`, `included`, `pdk`), `unrecognised` (files that are part of no view) |
| `expect_problem <text\|none>` | Fail unless the project has a problem containing the text (what the line over the library trees counts), or none |
| `expect_path_prompt <title\|none>` | Fail unless a path prompt with this title is open (or none) |
| `path_answer <text>` | Type the text into the prompt and press OK; fails when the prompt refuses it |
| `path_wrong <text>` | The same, failing unless the prompt refuses it (it stays open) |
| `path_cancel` | Cancel the prompt |
| `expect_path_note <text>` | Fail unless the prompt's note or error contains the text |

## Hierarchy, Used By, renames that follow references

`expect_hierarchy` and `hierarchy_open` (above) work on whatever the Hierarchy panel
shows. From the files a line reads `inv ×2 (x1, x2)` (a cell used, how often, through
which instances); in "Used by" mode `logic/buf/schematic  instance ×2 (x1, x2)`.

| Command | What it does |
|---|---|
| `hierarchy_mode <schematic\|layout\|netlist\|hdl\|used-by> [files\|instances]` | The panel's mode. For schematics `instances` is xschem's tree (one line per instance, the default), `files` the tree read from the files |
| `hierarchy_of <library> <cell>` / `hierarchy_of selection` | Show this cell whatever is selected / follow the document in front and the selection again |
| `used_by <library> <cell>` | What Used By… in the library views does: the Hierarchy panel shows the users of the cell |
| `refactor rename_cell <library> <cell> <name>` | Open the Rename Cell dialog. Also: `copy_cell <library> <cell> <to library> <name>`, `move_cell <library> <cell> <to library> [name]`, `copy_hierarchy <library> <cell> <to library> [name]`, `rename_view <library> <cell> <view> <name>`, `copy_view <library> <cell> <view> <to library> <to cell> <name>`, `rename_library <library> <name> [folder]` |
| `refactor import_files <library>` | Open the Import Files dialog (docs/howto/import-files.md) |
| `refactor cace_import <library> <cell>` / `cace_export <library> <cell> [view]` | Open Import / Export CACE Datasheet (docs/howto/cace.md); fields via `refactor_set`: `file`, `prefix`, `schematic`, `overwrite` (import), `file`, `view`, `results` (export); script `cdept-gui/scripts/cace.script` |
| `refactor_set <field> <value>` | A field of the open dialog: `name`, `to`, `to_cell`, `folder` (`yes`/`no`); of Import Files: `add <file>` (relative to the project), `clear`, `library`, `prefix`, `split` (`on`/`off`), `on_clash` (`skip`, `rename`, `overwrite`) |
| `expect_plan <text>` | A line of the open dialog's plan (what `cdept cell rename … --dry-run` prints) has this text; for the summary dialog after an import, a line of the summary |
| `refactor_ok` | Confirm the open dialog; fails if the operation does |
| `refactor_fails <text>` | Confirm the open dialog; it must fail with this text and stay open |
| `refactor_cancel` | Close the open dialog |

## Changes on disk, git, trash

Commands of `cdept-gui/src/background/script.rs`. The driver does not go on to the
next command while the project is being read or a library scanned.

| Command | Does |
|---|---|
| `shell <command>` | Run a shell command in the project folder, the way another program changes files (`$PROJECT` is set) |
| `expect_listed <library> <cell> [<view>]`, `expect_not_listed …` | Fail unless the library views list (do not list) the cell or view; tried for a while, so it also waits for the watcher |
| `expect_git <state> <library> [<cell> [<view>]]` | Fail unless git says `clean`, `modified`, `untracked`, `added`, `staged`, `ignored`, `conflict` or `none` (not in a repository) |
| `expect_branch <name>` | Fail unless the project's repository is on this branch |
| `delete_dialog <library> <cell> [<view>]` | Open the Delete Cell / Delete View dialog |
| `confirm_dialog [permanent]` | Confirm the open dialog as its button does: a delete goes to the trash, with `permanent` for good |
| `expect_trashed <path ending>` | Fail unless the last delete moved a file or folder whose path ends like this to the trash |
| `expect_stale <yes\|no>` | Fail unless the document shows (does not show) the "changed on disk" notice |
| `stale_answer <reload\|keep>` | Answer the notice |
| `expect_changed <yes\|no>` | Fail unless the document has (has no) changes of its own |
| `reload_project` | Project ▸ Reload |
| `load_project <folder>` | Open another project |
| `setting <watch\|auto_reload> <on\|off>` | Project ▸ Follow Changes on Disk / Reload Unchanged Documents |

## Waveform tabs (`wv`)

CDePT's own waveform viewer (`cdept-gui/src/views/wave`, docs/NOTES.md). The script
acts on the newest waveform tab unless `wv select` chose another. Mouse commands are
real input on the plot at a fraction `fx` of its width; they use window coordinates, so
no schematic or layout may be the script's document meanwhile. Numbers take SI
suffixes (`2.5u`, `10n`, `1meg`).

| Command | Does |
|---|---|
| `wv open <file>` | A new waveform tab on the file (one more tab each time, also for the same file) |
| `wv open_point <lib> <cell> <view> <point>` | The same for a run point's raw file (`out.raw`, else the first `*.raw`) |
| `wv select <n>\|newest` | Act on the n-th waveform tab (oldest is 1) |
| `wv expect_tabs <n>` | n waveform tabs are open |
| `wv wait_loaded` | Wait (up to 10 minutes) until the file is open; fails at once when it failed |
| `wv expect_failed <text>` / `wv expect_loading` | The tab failed with this text / still loads |
| `wv cancel`, `wv close`, `wv detach`, `wv reload` | Cancel the load; close the tab (its X); move it into a window of its own; open the file again |
| `wv show <signal>…`, `wv hide <signal>…` | Show or hide signals |
| `wv fit`, `wv zoom <x0> <x1>`, `wv expect_range <x0> <x1> [tol]` | The whole x range; a given one; check it |
| `wv cursor <a\|b> <x\|none>`, `wv expect_cursor_x <a\|b> <x> [tol]` | Put, remove, check a cursor |
| `wv expect_cursor <a\|b> <signal> <value> [tol]`, `wv expect_delta <signal> <value> [tol]` | The readouts (value at a cursor, B−A) |
| `wv expect_drawn <min\|max> <signal> <value> [tol]` | The extreme of what is drawn in the x range on screen |
| `wv check_extremes <signal>` | What is drawn has exactly the extremes of every sample in the range (read on the spot) |
| `wv wheel <fx> <up\|down> [count] [ctrl\|shift]`, `wv drag <fx1> <fx2>`, `wv click <fx> [left\|right]` | Mouse on the plot |
| `wv expect_cache <text>`, `wv expect_changed <yes\|no>`, `wv expect_files <dir> <text> <n>` | Where the `.uwraw` is; whether the file changed on disk; how many files of a folder have the text in their name |
| `wv stats` | Log frame intervals, the tab's UI time, query and drawing times, lines by query mode, RSS, peak RSS and the cgroup's memory since the last `wv stats` |
| `wv open_files <file>…` | One tab on several files overlaid; tags are the file names, or their folders' names when the file names are equal (`out.raw` of run points) |
| `wv open_family <lib> <cell> <view> [point…]` | What "Waveform Tab (family)" does: every run point folder of the view (or the ones named) in one tab, tags = point names; a folder without a raw file shows as a failed point |
| `wv add_file <file> [tag]` | Add a file to the tab |
| `wv expect_points <loading\|ready\|failed\|memory\|all> <n>`, `wv expect_point <tag> <state> [text]` | How many sources are in a state; one source's state and message |
| `wv point <tag> on\|off`, `wv points all\|none` | Per-point show and hide |
| `wv show_in <n\|new> <signal>…` | Show in pane n (1 is the top) or a new pane; a signal of another x unit goes to a pane of its unit |
| `wv pane new\|remove\|up\|down\|select\|fit_y [n]` | Add, remove, move, make active, fit the y of a pane (`show` and the cursor commands act on the active pane) |
| `wv expect_panes <n>`, `wv expect_pane <n> <signal>…`, `wv expect_x_unit <n> <unit>` | Panes; the signals of pane n in order; its x unit |
| `wv expect_lines <n\|>n>` | Lines drawn now (signals × visible sources that have them) |
| `wv y <n> <y0> <y1>\|fit`, `wv expect_y <n> <y0> <y1> [tol]` | Pane n's y range |
| `wv list <flat\|kind\|tree> [search…]`, `wv expect_rows <n>`, `wv expect_row <text>`, `wv fold <header>` | The signal list (headers read `# Voltages (2)`) |
| `wv pick <signal>…`, `wv pick_add <signal>…`, `wv expect_picked <signal>…`, `wv picked show\|new\|hide` | Select rows; what Show / New Pane / Hide do with the selection |
| `wv drag_signal <signal> <n>` | Drag the signal's row of the list onto pane n (real input; the row must be on screen) |
| `wv style <signal> <color #rrggbb\|dash solid\|dashed\|dotted\|dashdot\|width w> [tag]`, `wv expect_style <n> <label> <key> <value>`, `wv expect_distinct <n>` | Change a line's style; check one; no two lines of pane n look alike |
| `wv expect_markers <n>`, `wv expect_changed_files <n>` | The calculator's markers in the tab; files changed on disk |
| `wv expect_cursor_at <a\|b> <signal> <tag> <value> [tol]` | The readout of one point's line |
| `wv test_trace <name> <empty\|nan\|inf\|nonmono\|ramp\|sine\|big> [n] [x unit] [tag]` | Hand a series to the tab as "Send to tab" does (default 1000 points, x in s, tag `calc`) |
| `wv analyses`, `wv analysis <n\|kind\|name>`, `wv expect_analysis <text>` | Log the analyses of the first file (plots of a raw file, analyses of a PSF folder); open another one (n counts from 1; every file of the tab opens the analysis of that name); the open one's name has the text or its kind is it |
| `wv form <signal> <dB20\|mag\|phase\|uphase\|re\|im\|dB10>`, `wv ac_form <form>` | The form a shown complex signal is drawn in; the form of all of them (the toolbar's AC menu) |
| `wv bode <signal>`, `wv expect_marker <text> [x] [tol]` | A Bode plot of a complex signal (gain and unwrapped phase panes, margin markers); a marker (calculator's or Bode's) whose label has the text, at x (relative tolerance, default 1e-3). Text without spaces: `9.4°`, `PM`, `at` |
| `wv eye <signal> <period> [offset] [lines\|density]`, `wv expect_note <n> <text>` | An eye diagram in a pane of its own (the same signal alone in an eye pane changes its period); a note of a line of pane n (0: any pane): `3000 traces of 3000 periods`, or why there is no eye |
| `wv integrate <signal>` | The integrated noise of a noise density (`integ(<signal>)`, new pane) |
| `wv log <n> <x\|y> on\|off`, `wv expect_log <n> <x\|y> on\|off` | Log axes of pane n (x for its whole x group) |
| `wv expect_value <signal> <value> [tol]` | The table of an analysis of one point (op) shows this value for the first file |
| `wv expect_parts <n> [increasing\|decreasing\|segmented\|unordered]`, `wv expect_cursor_part <a\|b> <signal> <part> <value> [tol]` | The parts of x of the first file (a parametric sweep: one per sweep) and its order; the readout of one part (from 1) at a cursor |
| `wave menu_open <file>`, `wave wait_opens`, `wave open_file <file>` | File › Open in UberWave… with a picked file (in a thread), until those opens are done; the same at once, failing the script on an error |
| `wave expect_error <text>`, `wave dismiss` | The error dialog says this; close the dialog |
| `sim calc send_tab`, `sim calc follow` | "Send to tab"; what the Cursors section does when a cursor moved (UberWave's or a tab's) |
| `detach_panel <title>` | Move a panel (Console, Jobs, …) into a window of its own |

With `RUST_LOG=info,cdept::frame=debug` the app logs every frame slower than 250 ms
(`CDEPT_SLOW_FRAME_MS` changes the threshold) with the time of the main window, of each
detached window (with and without creating its viewport) and the CPU time of the frame
before without its buffer swap.

## Notes

- `tcl` can make the helper report widgets by hand, which is useful to test how a
  dialog is drawn: `tcl_send cdept::ui {<json of one UiUpdate>}` (see
  `libcdept/src/xschem/protocol.rs`).
- `RUST_LOG=info,xschem::ui=trace` logs the widget tree of every dialog xschem opens,
  as the helper reports it.
- `Ctrl+C`, `Ctrl+X` and `Ctrl+V` reach xschem as the keys they are, as with a real
  keyboard.
- The desktop's file chooser cannot be answered by a script; CDePT's own path prompt can
  (above). With `CDEPT_NATIVE_DIALOG=off` every question for a path is such a prompt.
- Checks wait up to 8 seconds for the program to get there (a dialog to open, a
  background job to finish) before they fail.
- A script is fast. Two clicks on the same spot within about half a second are a double
  click for xschem (which opens the attribute dialog of an instance): put a `wait 0.7`
  between them when two single clicks are meant.

## Tutorials

`cdept-gui/scripts/tutorial.sh [-o FOLDER] [ID ...]` replays tutorials (all without
names; the scripts are in `cdept-gui/scripts/tutorials/`) with scratch HOME and settings
and writes a folder per tutorial with its log, screenshots and projects. `pdk-setup`
runs with `PDK_ROOT` unset. It needs a display, the IHP PDK (`PDKS`, default
`/opt/ext/OpenPDKs`), the xschem helper, ngspice and KLayout.

| Command | Does |
|---|---|
| `guide start <tutorial>` | Start a tutorial (an id or a file) |
| `guide expect_step <id\|finished>` | The tutorial is at this step |
| `guide expect_pointing <anchor>` | The guide outlines this |
| `guide expect_marked` | The guide outlines something of the step's list |
| `guide click` | Click the middle of what is outlined, in the main window |
| `guide press <anchor>` | Click the middle of a named widget |
| `guide continue` | Click the card's Continue button |

## New testbenches and suites

`cdept-gui/scripts/new-testbench.script` (`CDEPT_EXAMPLE=ihp-inv`; needs the IHP PDK,
ngspice and the xschem helper) makes testbenches and a suite through the dialogs and runs
them.

| Command | Does |
|---|---|
| `sim new_testbench`, `sim new_suite` | Open the dialog for the selection |
| `sim new_set <field> <value>` | Fill a field of the open dialog (`cell`, `view`, `template`, `source dut\|other\|empty\|none`, `dut`, `other`, `param.<name>`; suite: `description`, `corners`, `temperatures`, `test <cell/view> on\|off`) |
| `sim new_expect <field> <value>` | What the dialog has (`tests`: how many are ticked) |
| `sim new_expect_error <text>` | The dialog stayed open with this error |

