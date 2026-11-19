# Testing the GUI with a script

`cdept gui --script <file>` runs the real program and, while it is on screen, executes
the commands of the file in order. It is how the embedded tools are tested end to end:
open a view, move the mouse, press keys, look at xschem's or KLayout's answers, take
screenshots.

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
a scratch copy of the project:

| Script | Checks | Writes |
|---|---|---|
| `cdept-gui/scripts/buffer-smoke.script` | the picture, input, dialogs, the Inspector, the Hierarchy panel, closing a changed document | nothing |
| `cdept-gui/scripts/buffer-edit.script` | the library picker (open, insert a symbol, save as), "open in new window", netlist to a view, saving on close and on closing the project | into the project: run it on a copy |
| `cdept-gui/scripts/buffer-sim.script` | the simulator configuration, Netlist and Simulate as a background job (a copy command stands in for the simulator, so no ngspice is needed), failed jobs, a waveform graph with its dialog, cursors and zoom | a netlist, a raw file and a graph into the project |
| `cdept-gui/scripts/buffer-preview.script` | pictures xschem draws inside its own dialogs (key cheat sheet, its file browser), pasting the desktop's clipboard into a dialog, Tcl's `clipboard get` / `selection get` (run it by name: `run.sh cdept-gui/scripts/buffer-preview.script`) | nothing |
| `cdept-gui/scripts/layout-smoke.script` | the embedded KLayout: the picture, layers and cells (external cells), modes, notices, drawing, undo/redo, save (sidecar), Place Cell, self-contained export, opening an external cell, closing a changed layout (needs `cdept-klayout/.bin/klayout`, see cdept-klayout/README.md) | into the project: run it on a copy (`run.sh cdept-gui/scripts/layout-smoke.script`) |


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
| `tcl_send <script>` | Evaluate without waiting for the result (for scripts that block, or end xschem) |
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
and path tools take **clicks**, not a drag.

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
| `expect_selection <n>` | Fail unless `n` objects are selected |
| `expect_modified <yes\|no>` | Fail unless the layout does (not) have unsaved changes (the tab's star) |
| `expect_mode <name>` | Fail unless the editor is in this mode |

KLayout reports a changed modified flag with the next request it handles, so the
`expect_*` commands above ask it for a sign of life while they wait.

## Notes

- `tcl` can make the helper report widgets by hand, which is useful to test how a
  dialog is drawn: `tcl_send cdept::ui {<json of one UiUpdate>}` (see
  `libcdept/src/xschem/protocol.rs`).
- `RUST_LOG=info,xschem::ui=trace` logs the widget tree of every dialog xschem opens,
  as the helper reports it.
- `Ctrl+C`, `Ctrl+X` and `Ctrl+V` reach xschem as the keys they are, as with a real
  keyboard.
- Dialogs that use the desktop's file chooser cannot be answered by a script.
- Checks wait up to 8 seconds for the program to get there (a dialog to open, a
  background job to finish) before they fail.
- A script is fast. Two clicks on the same spot within about half a second are a double
  click for xschem (which opens the attribute dialog of an instance): put a `wait 0.7`
  between them when two single clicks are meant.
