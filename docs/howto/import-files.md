# Import design files into a library

`cdept import` looks at a design *folder* and proposes a project for it. This page is
about the other job: bringing **files** from outside the project (a vendor's GDS and
netlists, a PDK's standard cells, an xschem schematic a colleague sent) into a library
as cells and views, with `cdept import-files` or **Import Files…** in the Library
Manager.

The command is a dry run until `--write` is given: it reads the files, prints which cells
and views would be written and where, and says what it could not bring in. The files
outside the project are never changed.

## What a file gives

| File | Cells | Notes |
|---|---|---|
| GDSII | one per top structure (a structure no other one places), with the structures it places | the structures keep their names; names placed and not defined (ghost cells) are reported |
| SPICE, CDL | one per `.subckt`, with the subcircuits it instantiates that the file defines | the lines of the file outside the subcircuits (`.include`, `.model`, `.param`, comments) are kept in every netlist; a relative `.include` is made absolute so the netlist still works from its new folder; circuit cards and analyses outside any subcircuit are left out |
| LEF | one per `MACRO` | the version, units and sites of the file go with each macro; layers and vias (a technology part) become a cell `<file>_tech` with the view `techlef` |
| Liberty | one per `cell` group | the `library` attributes and tables are kept in each |
| Verilog, SystemVerilog | one per `module`, with the modules it instantiates that the file defines | `` `timescale `` and the other lines outside the modules are kept |
| xschem `.sch`/`.sym`, Qucs-S, Magic `.mag`, VHDL, Verilog-A, OASIS, KLayout PCell | the file as it is, the cell named after the file | a `.sch` and a `.sym` of the same name are two views of one cell |

Compressed files (`.gds.gz`) are refused with a message: decompress them first. Files of
a format CDePT does not know are reported and left out, never guessed.

Cells of the same name from several files are **one cell with several views**: the GDS and
the SPICE file of `sg13g2_inv_1` give the views `layout` and `netlist` of one cell. The
view a format goes into:

| Format | View | Format | View |
|---|---|---|---|
| GDSII | `layout` | LEF | `abstract` (`techlef` for the technology part) |
| OASIS | `oasis` | Liberty | `liberty` |
| Magic | `magic` | Verilog, SystemVerilog | `verilog` |
| SPICE | `netlist` | VHDL | `vhdl` |
| CDL | `cdl` | Verilog-A | `veriloga` |
| xschem | `schematic`, `symbol` | Qucs-S | `qucs` |
| KLayout PCell | `pcell` | | |

## Import the standard cells of a PDK

The IHP SG13G2 standard cells are one GDS, one SPICE and one CDL file of 84 cells each in
the PDK's read-only reference library. Import them into a library of your own (here a
copy of `examples/buffer`):

```sh
cp -r examples/buffer /tmp/demo && cd /tmp/demo
cdept lib add std libs/std
P=$PDK_ROOT/ihp-sg13g2/libs.ref/sg13g2_stdcell
cdept import-files $P/gds/sg13g2_stdcell.gds $P/spice/sg13g2_stdcell.spice \
      $P/cdl/sg13g2_stdcell.cdl --to std
```

```text
import into std: 3 file(s) give 252 view(s) of 84 cell(s)
  note: .../spice/sg13g2_stdcell.spice: 'dantenna' is used and not defined in the file, by 'sg13g2_antennanp'
  note: .../spice/sg13g2_stdcell.spice: 'sg13_lv_nmos' is used and not defined in the file, by 'sg13g2_a21o_1', 'sg13g2_a21o_2', 'sg13g2_a21oi_1' and 76 more
  note: .../spice/sg13g2_stdcell.spice: 'sg13_lv_pmos' is used and not defined in the file, by 'sg13g2_a21o_1', 'sg13g2_a21o_2', 'sg13g2_a21oi_1' and 76 more

252 view(s) of 84 cell(s); 0 skipped, 4 problem(s)

Nothing written. --write imports the files; -l lists every view; --on-clash, --prefix and --no-split change what comes in.
```

The notes are the report of what the files do not hold: the SPICE file calls the PDK's
transistor models `sg13_lv_nmos` and `sg13_lv_pmos` as subcircuits and does not define
them, so the netlists of the cells need the PDK's model files when they are simulated.
`-l` lists every view with what is taken from the file:

```text
  copy .../gds/sg13g2_stdcell.gds -> libs/std/sg13g2_a21o_1/layout/sg13g2_a21o_1.gds  [std/sg13g2_a21o_1/layout]
      structure 'sg13g2_a21o_1'
  copy .../spice/sg13g2_stdcell.spice -> libs/std/sg13g2_a21o_1/netlist/sg13g2_a21o_1.spice  [std/sg13g2_a21o_1/netlist]
      subcircuit 'sg13g2_a21o_1'
```

Write it, then list the library:

```sh
cdept import-files $P/gds/sg13g2_stdcell.gds $P/spice/sg13g2_stdcell.spice \
      $P/cdl/sg13g2_stdcell.cdl --to std --write
cdept ls std | head -3
```

```text
Imported 252 view(s) of 84 cell(s) into std.
std  /tmp/demo/libs/std  [cells]
  sg13g2_a21o_1                    cdl layout netlist
  sg13g2_a21o_2                    cdl layout netlist
```

Each cell has a GDS with only that cell (and what it places) and a netlist with only its
`.subckt`, so every one is a normal cell: it can be renamed, copied and deleted, and
`cdept layout drc std sg13g2_inv_1` runs the PDK's DRC deck on it (see
[Run DRC, LVS and extraction](lvs-drc.md)).

The LEF and the Liberty file of the same cells are added to the same cells the same way. The
technology LEF has no macros, so it is one cell with the view `techlef`; the cell LEF of
the PDK carries layers too, which go to `sg13g2_stdcell_tech`:

```sh
cdept import-files $P/lef/sg13g2_stdcell.lef $P/lef/sg13g2_tech.lef \
      $P/lib/sg13g2_stdcell_typ_1p20V_25C.lib --to std --write
cdept ls std | grep -e a21o_1 -e tech
```

```text
  sg13g2_a21o_1                    abstract cdl layout liberty netlist
  sg13g2_stdcell_tech              techlef
  sg13g2_tech                      techlef
```

## Import a vendor's files

A folder from a vendor holds a netlist of two subcircuits, a layout, a note and a
compressed layout:

```sh
cdept import-files ../vendor/amp.spice ../vendor/inv.gds ../vendor/README.txt \
      ../vendor/old.gds.gz --to logic -l
```

```text
import into logic: 4 file(s) give 2 view(s) of 2 cell(s)
  copy ../vendor/amp.spice -> libs/logic/amp/netlist/amp.spice  [logic/amp/netlist]
      subcircuit 'amp' and the 1 it places
  copy ../vendor/amp.spice -> libs/logic/stage/netlist/stage.spice  [logic/stage/netlist]
      subcircuit 'stage'
  note: skipped: inv/layout: the library has it already (.../libs/logic/inv/layout/inv.gds)
  note: ../vendor/README.txt: unknown format, not imported
  note: ../vendor/old.gds.gz: compressed (GDSII): decompress it first (gunzip) and import the result

2 view(s) of 2 cell(s); 1 skipped, 2 problem(s)
```

- `amp.spice` gives two cells. The netlist of `amp` also holds `stage`, which it places,
  so it can be simulated on its own; the file's `.include "models.spice"` line is in
  both, with the path made absolute.
- `inv.gds` has the same cell `inv` as the library: that view exists already, so it is
  skipped (the default) and the existing layout is not touched.
- The note and the compressed layout are reported, not imported.

## When the library has the cell already

`--on-clash` says what to do with a view that the library has:

| `--on-clash` | |
|---|---|
| `skip` (default) | the existing view stays; the other views of the cell still come in |
| `rename` | the whole cell comes in under another name: `inv_1`, `inv_2`, … The structure or subcircuit inside the file is renamed to match |
| `overwrite` | the file of the existing view is replaced. Only on request |

```sh
cdept import-files ../vendor/inv.gds --to logic --on-clash rename -l
```

```text
import into logic: 1 file(s) give 1 view(s) of 1 cell(s)
  copy ../vendor/inv.gds -> libs/logic/inv_1/layout/inv_1.gds  [logic/inv_1/layout]
      structure 'inv', as 'inv_1'
  note: skipped: inv: the library has it already, imported as 'inv_1'
```

Everything is written or nothing: if a file cannot be written, the ones already written
are put back and the error says so.

## Names, prefixes and keeping a file whole

- `--prefix vendor_` puts a prefix in front of every cell name (`vendor_amp`).
- `--name OLD=NEW` renames one cell of a file (before the prefix); it can be repeated.
  The structure, subcircuit, macro, module or cell inside the file gets the new name too.
- `--no-split` takes every file as one cell named after the file, whatever is inside:
  `amp.spice` is the cell `amp` with a view `netlist` holding both subcircuits.
- `--views gds,spice` takes only files of these formats (the names of the
  [`[suffixes]`](../reference/files/cdept-toml.md#suffixes) table).

## In the Library Manager

**Import Files…** is in the context menu of a library (and in the Library menu while a
library is selected). The dialog has the library, a prefix, what to do when a view
exists, the "One cell per structure, subcircuit, macro, module or cell" switch and the
list of files: type a path or use **Browse…**, then **Add**. Below the fields it shows the
plan the way the rename and copy dialogs do, including the notes about what cannot come
in. **Import** writes the files, selects the first new view and opens a summary of what
came in and what could not. The test `cdept-gui/scripts/import-files.script` does this on
`examples/buffer` (see [GUI-TESTING.md](../GUI-TESTING.md)).

## What it does not do

- It does not follow references between files: a schematic that places a symbol of a
  library the project does not have keeps naming it. Use the problems list of the project
  after the import.
- Only top structures become cells: a structure that another one places is part of that
  cell's file, not a cell of its own. `--no-split` keeps the whole stream as one cell.
- Libraries in the `ref` layout (a PDK's) are read-only and cannot take files.
