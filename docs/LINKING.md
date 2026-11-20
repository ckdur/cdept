# CDePT: linking schematics with layouts

Goal: a layout knows which schematic it implements and which layout instance stands
for which schematic instance, as Virtuoso Layout XL does, in CDePT's own formats.
From that: generate the layout's devices from the schematic, keep them in sync when the
schematic changes, check the two against each other, and cross-probe (select in one,
see it in the other).

Status (2026-11-20): first round built and tested: device map, schematic reader, plan,
generate/update (`cdept layout link`, Generate / Update in the GUI), check
(`cdept layout check-source`), Link panel, cross-probing between schematic and layout
tabs. Second round (nets, §8): connectivity from the PDK's LVS deck against xschem's,
opens and shorts, flylines, net highlighting and probing, cross-probing of nets
(`cdept layout check-nets`, Layout › Update Connectivity, the Nets panel).

## 1. The link

- **Instance identity.** A layout instance belongs to a schematic instance through a
  GDS property on the instance: attribute **61** (`PROPATTR` 61, `PROPVALUE` = the
  schematic instance name, e.g. `M1`, `x2`). KLayout keeps integer property keys on
  instances, writes them to GDS as `PROPATTR`/`PROPVALUE` and to OASIS as
  `S_GDS_PROPERTY` (checked: both survive save and reload; a moved instance keeps it;
  a copied instance carries the same name, which is reported as a duplicate). Rust
  reads them without KLayout (`layout::read_placements`, GDS and OASIS).
- **The sidecar** `<layout file>.json` (`cdept-layout`) gets version **2** when a link
  is recorded; files without a link are still written as version 1, and version 1 is
  read as before:

  ```json
  {
    "format": "cdept-layout",
    "version": 2,
    "library": "cells", "cell": "inv", "view": "layout", "top": "inv",
    "references": {},
    "source": { "library": "cells", "cell": "inv", "view": "schematic" },
    "instances": {
      "M1": { "symbol": "sg13g2_pr/sg13_lv_nmos.sym", "kind": "pcell",
              "library": "SG13_dev", "cell": "nmos",
              "params": { "l": "0.13u", "m": "1", "model": "sg13_lv_nmos",
                          "ng": "1", "w": "0.74u" } },
      "x1": { "symbol": "cells/inv/symbol/inv.sym", "kind": "cell",
              "library": "cells", "cell": "inv" }
    }
  }
  ```

  `source` is the schematic view; `instances` what each schematic instance was made
  into at the last sync (`kind` `pcell`: a PCell of a KLayout library with its
  parameters; `cell`: the layout view of a project cell). An optional `parallel`
  lists groups of devices in parallel (§8.3). Rust types:
  `layout::{Source, Linked, Parallel}`.

## 2. Reading the schematic (`libcdept::link::schematic`)

The `C` records of the `.sch` (symbol reference, position, attributes), read in Rust;
wires and texts are skipped, and so are embedded symbols (`[ … ]`). Symbols are looked
up like xschem does: the parents of the project's libraries, the PDK's
`libs.tech/xschem`, xschem's own `xschem_library/*`. From a symbol: `type=`, whether it
has a `format=`/`lvs_format=` (a netlist line), and the defaults of `template=`.

Each instance is one of:
- a **pin** (`type=ipin|opin|iopin`, or a symbol named so): its `lab` becomes a pin
  label (§3);
- a **device** with an entry in the device map (§3): a PCell with parameters;
- an instance of a **project cell** (its symbol file is in a library of the project): a
  reference to that cell's layout view (`layout`, else the first GDS/OASIS view), as
  "Place Cell" makes; no layout view → a problem;
- skipped: labels, titles, probes, code blocks and other symbols without a netlist line,
  and instances with `spice_ignore` / `lvs_ignore` set;
- else a **problem**: no device map entry, symbol not found, no name, a name used twice,
  a parameter that is not a number where one is needed.

The schematic is read as saved: unsaved edits in an open schematic tab are not seen.

## 3. The device map (`libcdept::link::devices`)

Our own TOML, `format = "cdept-devices"`, version 1:

```toml
format = "cdept-devices"
version = 1

[pins]               # optional: labels for the schematic's pins
layer = "8/25"       # layer/datatype of the text (IHP: Metal1.text)
size = 0.1           # µm

[[device]]
symbol = "sg13g2_pr/sg13_lv_nmos.sym"   # as the schematic refers to it
library = "SG13_dev"                    # KLayout library
cell = "nmos"                           # its PCell (or a fixed cell)
params = { model = "@model", w = "@{w|u}", l = "@{l|u}", ng = "@ng", m = "@m" }
```

- `symbol` with a folder matches that reference (also at the end of a longer path);
  without a folder it matches the file name anywhere.
- Parameter templates: `@name` is the instance's attribute, else the symbol's default
  (`template=`); `@{name|u}` reads it as a SPICE number (`0.5e-6`, `2u`, `1.5meg`) and
  writes it in µ with the suffix (`0.5u`), which is what IHP's PCells take. Units:
  `f p n u m k`, and `num` (plain number). Values are converted to the PCell
  parameter's type by the helper (an `int` parameter gets `1`, not `"1"`).
- The PDK's map ships with CDePT: `pdk/<PDK>/devices.toml` in the source tree,
  `<prefix>/share/cdept/pdk/<PDK>/devices.toml` installed (`$CDEPT_PDK_DATA` overrides),
  chosen by the project's `technology` (`$PDK`). IHP SG13G2 is mapped: LV/HV MOS,
  rsil/rppd/rhigh, cmim/rfcmim, ntap1/ptap1, dantenna/dpantenna, pnpMPA, npn13G2.
- The project's own map, `[klayout] devices = "devices.toml"` in cdept.toml, is read
  after the PDK's: an entry for the same symbol replaces the PDK's.
- PDK PCells are bound to a KLayout technology: the project needs `[klayout] tech`
  (e.g. `sg13g2`), or KLayout reads the saved PCells back as plain (defunct) cells.

## 4. Plan and check (`libcdept::link::Plan`)

Without KLayout: the schematic (§2, §3), the layout's bound instances (GDS/OASIS
properties of the top cell) and the sidecar's record give:

| | |
|---|---|
| add | a schematic instance with no layout instance |
| update | a parameter differs from the record (or the instance was never synced) |
| replace | the instance is another device or cell now (e.g. nmos → pmos) |
| extra | a layout instance bound to a name the schematic does not have |
| twice | a name bound to several layout instances (a copy) |
| problem | a schematic instance that cannot be made into layout (§2) |
| note | layout instances bound to nothing (allowed: guard rings, fill, …) |

`cdept layout check-source <lib> <cell> [--schematic V] [--view V]` prints it and exits 1
when anything but notes is listed. It compares with the record, not with the PCells'
actual parameters (those need KLayout); the Link panel compares with the actual ones.

## 5. Generate / update

`cdept layout link <lib> <cell> [--schematic V] [--view V] [--remove] [--dry-run]`, and
in the GUI "Generate / Update from Schematic" (layout tab: Layout menu and Link panel;
schematic tab: Cell › Generate / Update Layout, which makes the layout view from the
template when there is none and opens it). The helper's `link_apply` makes the top
cell match, as **one undoable step** on the open layout (in the GUI it leaves the
layout modified; the CLI saves):

- a bound instance of the right device: parameters that differ are changed
  (`change_pcell_parameters`); otherwise unchanged;
- of another device or cell: replaced in place (same transformation);
- not in the layout: an instance of the right cell that is bound to nothing yet is
  taken over (left to right, bottom to top: a layout drawn before the link); else a
  new instance is placed, in rows to the right of what the cell has, one row per kind
  of device, wrapped at 60 µm;
- bound to a name the schematic does not have: left in place and reported, deleted with
  `--remove` ("Remove Extra" in the panel);
- pin labels (`[pins] layer`) for pins that have no text of that name on that layer
  yet, in a column left of the cell.

After the save the sidecar records `source` and `instances` (§1).

Helper methods (docs/KLAYOUT.md §4 conventions; `link.py`):

| method | params | value |
|--------|--------|-------|
| `link_apply` | `{items: [{name, symbol, kind: pcell, library, cell, params} \| {name, symbol, kind: cell, library, cell, view, file, refs}], pins: [{name, direction}], pin_layer, pin_size, remove}` | `{added, bound, updated, replaced, unchanged, removed, extra, duplicates, pins, failed: [{name, error}]}` |
| `link_instances` | `{}` | `{instances: {name: {cell, library, pcell, params, bbox, count, file?, label?}}, unbound}` |
| `link_select` | `{names, zoom?}` | `{found}`: selects those instances of the top cell |
| `link_selected` | `{}` | `{names}`: names bound to the selected instances |
| `link_highlight` | `{names, color?, zoom?}` | `{markers}`: markers around them (empty: clear) |

(Method names use `_`, not `.`: the helper dispatches on Python identifiers.)

## 6. GUI

- **Layout tab**: Layout › Generate / Update from Schematic, Open Schematic, Link Panel,
  Cross-probe with the Schematic. The Link panel (bottom) lists every schematic
  instance with what it becomes and its status in the layout (`ok`, `missing`,
  `parameters`, `other cell`, `twice`), the layout instances not in the schematic, and
  the problems; a click selects and shows the instance.
- **Schematic tab**: Cell › Open Layout (the layout linked with this schematic: a
  layout view of the same cell whose sidecar has no other source), Cell › Generate /
  Update Layout.
- **Cross-probing** (on by default, per layout tab): instances selected in a layout tab
  are selected in the open tabs of its schematic (`xschem select instance`), and the
  reverse. The names last exchanged are remembered so a selection does not bounce.
  Top level only: no hierarchy paths yet.
- Script commands for tests: `link_generate [remove]`, `link_select <names…>`,
  `expect_link <instance> <status>`, `expect_probed <names…|none>`
  (docs/GUI-TESTING.md); `cdept-gui/scripts/layout-link.script`.

## 7. Not yet

- **Nets**, further: net-aware placement; flylines that follow edits live (today they
  are as old as the last Update Connectivity); a connectivity check without a PDK
  LVS deck (e.g. from the technology's `connectivity` section and KLayout's net
  tracer); unsaved schematic edits (nets are read from the schematic as saved).
- Hierarchical cross-probing (instance paths below the top cell).
- Generating from a schematic with unsaved changes (it is read from disk).
- Pins become text labels only; no pin shapes, no placement next to the device.
- `check-source` compares with the record, not with the PCells' actual parameters.
- Bus instances (`M1[3:0]`) are treated as one instance with that name.
- Examples: `examples/ihp-inv` (inv: two IHP PCells; buf: two references to inv), and
  `examples/buffer` buf (links its existing inv instances). `examples/buffer` inv uses
  xschem's generic devices: nothing to map.

## 8. Nets

The schematic's connectivity against the layout's, short of LVS: which schematic net
is open in the layout (in pieces), which layout net shorts schematic nets, where the
pieces are (flylines), and the nets highlighted across both editors. Rust owns the
comparison (`libcdept::link::nets`); KLayout extracts, the helper draws.

**The schematic side.** xschem's own connectivity: `xschem instance_nodemap` gives,
for each instance, symbol pin → net, as the netlister sees wires and labels (one
engine, the schematic as saved). Instances of project cells are followed into their
schematics (the cell's schematic view), and everything is flattened: every device
terminal of the hierarchy (`x1/M1` pin `G`) on one flat net. A flat net is named by
its name nearest the top (`#net1` when xschem names it). This is the netlister's view
without parsing a netlist: instance names, pin names and nets come out as xschem has
them, and the device map (§3) already tells devices from cells.

**The layout side.** The layout is written as a self-contained stream (unsaved edits
included, as for DRC) and extracted with the **PDK's LVS deck** in a KLayout of its own
(`klayout -b`; the deck needs a schematic netlist, xschem's LVS netlist of the cell is
given and its comparison is not used). The helper script `connectivity.py`
(`cdept_klayout/l2n.py`) reads the LVS database and the stream and writes JSON:

- `nets`: every net, flattened into the top cell by walking the hierarchy (a net of a
  placed cell that leaves it through a pin is the parent's net; one that does not is
  a net of its own); `name` is KLayout's (from labels), `""` without one;
- `devices`: class, box, and per terminal its net and the middle of its shapes;
- `instances`: the instances bound to schematic instances (GDS property 61, §1), with
  their paths through bound cells (`x1/M1`) and boxes in top-cell coordinates;
- `labels`: the texts of the **top cell** with the net each one is on. A label inside
  a placed cell names a net of the top circuit too, so net names alone cannot tell
  a pin label from an inner one.

No device geometry is in CDePT's files: KLayout's extraction with the PDK's rules
decides what a device is.

**Matching.** A device belongs to the bound leaf instance (a PCell or library cell)
whose box holds it (the smallest one). Symbol pins go to device terminals by name (any
case: `D G S B` of a MOS), else in order (IHP: rsil `P M` → `rsil_1 rsil_2`, cmim
`c0 c1` → `mim_top mim_btm`), else as the device map's optional
`terminals = { pin = "terminal" }` says. A MOS's `S`/`D` and a resistor's two ends may
be swapped: each such device takes the orientation that agrees with the nets its
other terminals and the other devices already put on the layout nets.

**The comparison** (`nets::compare`), per schematic net, the layout nets its terminals
are on (*pieces*):

| | |
|---|---|
| ok | one piece, and no other schematic net on it |
| open | several pieces; **flylines** join them: a minimum spanning tree (Prim) over the pieces, each edge between their nearest terminals |
| short | a layout net with terminals of several schematic nets (listed once more under shorts) |
| not placed | its terminals belong to devices that are not in the layout |

and also: terminals on no net (*unconnected*), schematic devices with no layout
device (*not in the layout*), layout devices that belong to no bound instance (a note:
taps, fill), and each schematic pin: `ok` when a label of its name in the top cell is
on a wire of its net, else *no label in the layout*, *the label is on no wire*, or
*the label is on the layout net of …*. A pin label joins its net's pieces too.

**Running it.** `cdept layout check-nets <lib> <cell> [--schematic V] [--view V]
[--json] [--dir D]` prints the findings (`net VDD: open, 2 pieces: {M2.D} on #4,
{M2.B} on #5`) and exits 1 unless clean; work files in
`~/.cache/cdept/verify/<project>/<lib>/<cell>/nets/`. About 4 s for the example
inverter (mostly the deck).

**In the layout tab.** Layout › Update Connectivity (also the Nets panel's Update)
runs the same on the layout as it is now, in the background. Then:

- the helper loads the LVS database (`nets_load`) and draws the **flylines**
  (`nets_flylines`, magenta, Layout › Flylines);
- the **Nets panel** lists the nets (problems first), shorts, unconnected terminals,
  pins and notes; a click highlights the net's shapes (`nets_highlight`) and zooms
  there;
- **Probe Nets** (Layout menu, the panel, key `N`): a left click in the picture asks the
  helper for the net under it (`nets_probe`, the topmost layer's) and highlights it;
- **cross-probing** (with the Link panel's switch): a net shown or probed in the layout
  is highlighted in the schematic tabs (`xschem hilight_netname`); a wire, label or
  pin selected in a schematic highlights its net in the layout (the selection reports
  `nets`: `cdept::selection` in cdept-xschem/shim/tcl/cdept.tcl);
- what was extracted is as old as the layout was then: the helper counts changes
  (`revision` of the `modified` event) and the panel says "out of date" after an
  edit; "Update Connectivity after Saving" extracts again after each save.

Helper methods (`nets.py`):

| method | params | value |
|--------|--------|-------|
| `nets_load` | `{lvsdb, top?}` | `{nets, top}`: the database of the last extraction |
| `nets_highlight` | `{nets: [id], color?, zoom?}` | `{markers}`; empty clears |
| `nets_flylines` | `{lines: [[x1, y1, x2, y2]], color?}` | `{markers}`; empty clears |
| `nets_probe` | `{point: [x, y]}` (µm) or `{pixel: [x, y]}` | `{net, name, point}`; `net` null: nothing there |
| `nets_clear` | `{}` | |

Markers are not part of the layout and never saved.

**Example.** `examples/ihp-inv`: the inverter is wired at A and Y
(`scripts/wire-inv.py`: GatPoly between the gates with a contact to a Metal1 pad, a
Metal1 wire between the drains, the labels A and Y on them), not at VDD and VSS: two
nets ok, two open (each MOS's source against its bulk), VDD and VSS labels on no
wire. buf places it twice: the net between the two inverters is open from one cell to
the other. Tests: `libcdept/tests/nets.rs` (also a deliberate short, A to Y),
`cdept-klayout/tests/test_nets.py` (a deck of its own on generic layers, no PDK),
`cdept-gui/scripts/layout-nets.script`.

**Limits.** The layout's nets need the PDK's LVS deck (any PDK whose deck takes
`input`, `topcell`, `report`, `schematic`, as IHP's does); a PDK without one has no
net check ("this PDK has no KLayout LVS deck: net checking is not available"), and
there is no fallback: CDePT does not guess a PDK's devices. Bulk terminals of MOS
devices sit in the channel's middle, so a MOS's source–bulk flyline is short and
inside the device. Bus instances (`M1[3:0]`) are one name (§7).

### 8.1 Live flylines

Wiring changes far more often than devices. After an extraction the helper
(`cdept_klayout/live.py`) keeps the devices as extracted and traces the **wiring**
between their terminals on the live layout after each change, with KLayout's net
tracer (`pya.NetTracer`) and the technology's net tracer connectivity (the
`<connectivity>` of the PDK's `.lyt`: IHP lists GatPoly/Diff–Cont–Metal1, the vias and
metals):

- *seeds*: every device terminal and every top-cell label of the extraction, each a
  point and the first conductor layer with a shape there (a label's own layer number
  first);
- *calibration*, right after the extraction: seeds traced into one net must have been
  on one extracted net. A seed that disagrees with most of its net is kept as
  extracted instead: a MOS bulk (its terminal sits in the channel, so it traces as the
  gate) whose connection goes through the well, which the net tracer does not know;
- after each change, once the layout has been quiet for `delay_ms` (300 ms), the
  seeds are traced again (a seed on a shape of a net traced already joins it without a
  trace of its own). Devices of a top-level instance move with it; devices of deleted
  instances drop out (their schematic devices are "not in the layout"); instances
  placed since are listed and count after the next Update. A label that was on no
  shape is tried again (a wire may be under it now);
- the result is a `nets_live` event: per device its terminals' nets and places now,
  the nets (names from the extraction), moved/added/gone instances. CDePT patches the
  extraction with it (`Connectivity::with_live`) and compares again with the same
  schematic: flylines, opens and shorts follow the edit at once. Highlights and
  probing use the traced shapes.

Measured (IHP SG13G2, 1600×1000 view): the inverter 3 ms per update, buf 5–8 ms, 100
devices (50 inverters) 120–135 ms, set-up 285 ms once. The helper stops live updates
when one takes longer than 250 ms (`limit_ms`; a notice says so) and the flylines then
change with Update Connectivity only, as before. A technology without net tracer
connectivity has no live flylines either (the Nets panel says why).

Helper method: `nets_live {connectivity?, rules?, on?, delay_ms?, limit_ms?, update?}`
(set up from the extraction's JSON; `rules` = `{connections, symbols}` instead of the
technology's; `update`: trace now and answer the result).

### 8.2 How flylines look

`FlylineStyle` (`libcdept::link::nets`): colour, width (px), line (solid, dashed,
dotted, dash-dotted: KLayout's line styles), opacity (percent; markers have no alpha,
so the colour is blended with the background), the colours of a net shown and of a
shown net that is shorted, live on/off and the delay. The user's are part of the GUI
settings (Nets panel › Style…, Layout › Flyline Style…); a project may set any of
them in cdept.toml, on top:

```toml
[klayout.flylines]
color = "#00c0ff"
width = 3
line = "dashed"
opacity = 80
live = true
```

`nets_flylines` takes `color`, `width`, `style`, `opacity` (kept for later calls);
`nets_highlight` takes `color`.

### 8.3 Devices in parallel

An array of transistors drawn as several schematic instances with the same nets would
get one flyline per member. A **parallel group** is found in the flattened schematic:
the same symbol and the same nets on every pin (a MOS's S and D either way round, the
two pins of a two-pin device either way). Groups are recorded in the layout's sidecar
with the link (version 2):

```json
"parallel": [
  { "members": ["M1a", "M1b", "M1c"], "joined": true },
  { "members": ["x1/M3", "x2/M3"], "joined": false, "manual": true }
]
```

`joined` (default true) is the user's choice; `manual` marks a group the user made.
For a joined group: the pieces of a net that hold only members of the group count as
one (one flyline to the nearest member), and a layout device lying over a member
without one of its own stands for it (the IHP deck combines parallel devices into
one). Found again at each comparison: a group no longer found goes unless it is the
user's. Edited with the Nets panel's checkboxes or `cdept layout parallel <lib>
<cell> [--off I] [--on I] [--group A,B,…] [--ungroup I]`. `m=`/`ng` multiples are one
instance and need no group.

### 8.4 The schematic as saved

The comparison reads the schematic from its file. Update Connectivity in the GUI
first saves an open schematic of the link that has unsaved changes (through its
xschem session, waiting for the answer), then extracts; a cancelled save means no
update. `cdept layout check-nets` reads the files as they are.

