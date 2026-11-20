# CDePT: linking schematics with layouts

Goal: a layout knows which schematic it implements and which layout instance stands
for which schematic instance, as Virtuoso Layout XL does, in CDePT's own formats.
From that: generate the layout's devices from the schematic, keep them in sync when the
schematic changes, check the two against each other, and cross-probe (select in one,
see it in the other).

Status (2026-11-20): first round built and tested: device map, schematic reader, plan,
generate/update (`cdept layout link`, Generate / Update in the GUI), check
(`cdept layout check-source`), Link panel, cross-probing between schematic and layout
tabs. Not yet: nets (flylines, net highlighting, connectivity checks), see §7.

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
  parameters; `cell`: the layout view of a project cell). Rust types:
  `layout::{Source, Linked}`.

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

- **Nets.** No connectivity yet: flylines (rats nest) between pins of bound
  instances, net highlighting across schematic and layout, net-aware placement, and a
  connectivity check short of LVS. The netlist is cheap to get (xschem's), the layout
  side needs the PDK's connectivity (KLayout's `LayoutToNetlist` with the LVS deck's
  layer stack).
- Hierarchical cross-probing (instance paths below the top cell).
- Generating from a schematic with unsaved changes (it is read from disk).
- Pins become text labels only; no pin shapes, no placement next to the device.
- `check-source` compares with the record, not with the PCells' actual parameters.
- Bus instances (`M1[3:0]`) are treated as one instance with that name.
- Examples: `examples/ihp-inv` (inv: two IHP PCells; buf: two references to inv), and
  `examples/buffer` buf (links its existing inv instances). `examples/buffer` inv uses
  xschem's generic devices: nothing to map.
