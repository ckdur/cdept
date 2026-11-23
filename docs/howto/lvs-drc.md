# Run DRC, LVS and extraction

CDePT runs layout checks with two sets of tools, both configured from the project's
[technology snapshot](../concepts/technology.md):

| Check | Tool | Command | Where it is available |
|---|---|---|---|
| DRC with the PDK's deck | KLayout (batch) | `cdept layout drc` | CLI and GUI (Layout menu) |
| LVS with the PDK's deck | KLayout (batch) | `cdept layout lvs` | CLI and GUI (Layout menu) |
| Net comparison with the schematic, pins | CDePT, with KLayout | `cdept layout check-nets`, `pins` | CLI and GUI, see [LINKING.md](../LINKING.md) |
| Extraction to SPICE | Magic | `cdept magic extract` | CLI only |
| DRC with Magic's rules | Magic | `cdept magic drc` | CLI only |
| LEF abstract | Magic | `cdept magic lef` | CLI only |
| LVS between two SPICE netlists | netgen | `cdept netgen lvs` | CLI only |

The examples run on a copy of `examples/ihp-inv` with `PDK_ROOT=/opt/ext/OpenPDKs` and
the tools of `/opt/ext/OpenLane/bin` on `PATH`. The inverter of that example is only
partly wired (VDD and VSS are not connected) on purpose, so its results show failures.
How each tool works is not repeated here: see [KLAYOUT.md](../KLAYOUT.md) section 8,
[MAGIC.md](../MAGIC.md) and [BATCH.md](../BATCH.md).

Exit status of every check is non-zero when it finds problems, so the commands can gate
a script or CI job.

## What is taken from the technology

| Needed | Taken from |
|---|---|
| KLayout DRC deck | `[klayout] drc` of `cdept.toml`, else the first `klayout.drc` entry of `tech.toml`, else the first `*.drc` in the `KLAYOUT_PATH` folders |
| KLayout LVS deck | `[klayout] lvs`, else the first `klayout.lvs` entry, else the first `*.lvs` |
| KLayout technology | `[klayout] tech`, else `klayout.tech` |
| Magic start-up file | `magic.rc` (else `$PDK_ROOT/$PDK/libs.tech/magic/$PDK.magicrc`) |
| Magic extraction and DRC style | the first entry of `magic.extract` and `magic.drc`, unless `--style` is given |
| netgen set-up | `netgen.setup`, else `--setup`, else an empty one |

`cdept tech show klayout` shows the decks the project will use. To use another deck
permanently, set `[klayout] drc` or `lvs` in `cdept.toml`; for a single run use
`--deck FILE`.

Program locations: `klayout` through the embedded helper; Magic is `$CDEPT_MAGIC` else
`magic` on `PATH`; netgen is `$CDEPT_NETGEN` else `netgen` on `PATH`. See
[Environment variables](../reference/environment.md).

## DRC with a KLayout deck

```sh
cdept layout drc cells inv
```

CDePT writes a self-contained GDS of the cell (every referenced cell filled in), runs the
deck with `klayout -b`, and reads the report:

```text
deck    /opt/ext/OpenPDKs/ihp-sg13g2/libs.tech/klayout/tech/drc/ihp-sg13g2.drc
layout  ~/.cache/cdept/verify/ihp-inv/cells/inv/drc/inv.gds (top cell inv)
     1  LU.b  7.2. LU.b : Max. space from any portion of N+Activ inside PWell to an pSD-PWell tie is 20.0 um
     5  Pin.e  Pin.e : 7.4. Pin.e: Min. metal1 enclosure of metal1:pin = 0um
report  ~/.cache/cdept/verify/ihp-inv/cells/inv/drc/inv.lyrdb
Error: 6 DRC violation(s) in 2 rule(s)
```

Each line is the number of violations, the rule and its description. The exit status is 1
when there are violations.

Options: `--view VIEW` picks the layout view when a cell has several, `--deck FILE` runs
another deck, `-D NAME=VALUE` passes further variables to the deck (`-rd` of KLayout; the
deck always gets `input`, `topcell` and `report`), `--dir DIR` chooses the output folder.

Results land in `~/.cache/cdept/verify/<project>/<library>/<cell>/drc/`:

| File | Contents |
|---|---|
| `<cell>.gds` | the stream that was checked |
| `<cell>.lyrdb` | KLayout's marker database (open it in KLayout) |
| `drc.log` | KLayout's output |

### In the GUI

Open the cell's layout and choose **Layout > Run DRC**. The check runs in the
background on the layout as it is now, unsaved edits included. The violations appear in a
results panel; clicking one highlights the shapes and zooms to them. The highlights are
markers and never part of the layout. There is no cancel button.

## LVS with a KLayout deck

```sh
cdept layout lvs cells inv
```

CDePT netlists the cell's schematic with xschem in LVS form, extracts the layout with the
PDK's LVS deck and compares them:

```text
layout  ~/.cache/cdept/verify/ihp-inv/cells/inv/lvs/inv.gds (top cell inv)
netlist ~/.cache/cdept/verify/ihp-inv/cells/inv/lvs/inv.spice
NoMatch          layout inv / schematic INV
  Mismatch         net        - / A
  Mismatch         net        - / VDD
  ...
  Mismatch         device     $1 / M1
  Mismatch         device     $2 / M2
report  ~/.cache/cdept/verify/ihp-inv/cells/inv/lvs/inv.lvsdb
extracted ~/.cache/cdept/verify/ihp-inv/cells/inv/lvs/inv_extracted.cir
Error: the layout does not match the schematic
```

Options are those of DRC plus `--schematic VIEW` (the schematic view, when the cell has
several) and `--netlist FILE`, which compares with a SPICE netlist you provide instead of
netlisting the schematic. A clean run exits 0.

Results land in `.../<cell>/lvs/`: the stream, `<cell>.spice` (schematic netlist in LVS
form), `<cell>.lvsdb` (KLayout's LVS database), `<cell>_extracted.cir` (the netlist
extracted from the layout), `lvs.log`, `summary.json` and `summary.log`. The `.lvsdb` is
what the net highlighting in the GUI reads.

In the GUI use **Layout > Run LVS**. Two related checks need no deck and are quicker for
day-to-day work: `cdept layout check-nets` (opens, shorts, unconnected terminals, pins
without labels) and `cdept layout pins`. They are described in
[LINKING.md](../LINKING.md) sections 8 and 9.

## Extraction and DRC with Magic

Magic is run as a separate program in batch mode, with the PDK's start-up file.
It reads the cell's `.mag` view, or a cell of a GDS view.

```sh
cdept magic extract cells inv            # layout to SPICE
cdept magic extract cells inv --pex c    # with capacitances
cdept magic drc cells inv                # PDK DRC rules
cdept magic drc cells inv --style 'drc(full)'
cdept magic lef cells inv [--hide]       # LEF abstract
```

```text
$ cdept magic extract cells inv
/work/ihp-inv/magic/cells/inv/inv.spice
  .subckt nmos (3 ports)
  .subckt pmos (2 ports)
  .subckt inv (4 ports)
      1 x nmos
      1 x pmos
$ cdept magic drc cells inv
cells/inv: DRC clean (drc(fast))
$ cdept magic lef cells inv
/work/ihp-inv/magic/cells/inv/inv.lef
  inv 5.68 x 5.22 um, 4 pins
```

`--style` takes a style name of the technology (`cdept tech show magic` lists
`extract` and `drc`); without it the first listed style is used. `magic drc` exits with
status 1 when the layout is not clean and prints the rules violated.

Results land in `<project>/magic/<library>/<cell>/`: `<cell>.spice` and Magic's `.ext`
files, `<cell>.drc.json` (violations by rule id with boxes in micron), `<cell>.lef`.
These commands have no GUI equivalent; Magic itself can be started from the GUI's tool
launchers.

Magic's DRC rules and KLayout's deck are different checks: a layout can pass one and
fail the other. A PDK's signoff deck is normally the KLayout one.

## LVS with netgen

`cdept netgen lvs` compares two SPICE netlists: the layout's extracted netlist and the
schematic's, each with the name of the cell to compare.

```sh
cdept netgen lvs <layout.spice> <cell> <schematic.spice> <cell> [--setup FILE] [--dir DIR]
```

A typical use compares the KLayout-extracted netlist with the schematic netlist of the
step above:

```sh
V=~/.cache/cdept/verify/ihp-inv/cells/inv/lvs
cdept netgen lvs $V/inv_extracted.cir inv $V/inv.spice inv
```

```text
Netlists do not match.
devices: layout 2, schematic 2
nets:    layout 5, schematic 4
log: /work/ihp-inv/batch/netgen/lvs.log
Error: LVS is not clean (Mismatch)
```

The verdict is one of `Match`, `PropertyErrors`, `PinMismatch`, `Mismatch` or `Unknown`;
the command exits with an error unless the circuits match. `Unknown` means netgen did not
print a result, usually because a cell name is missing from one of the files; the log
says which. The set-up file is `netgen.setup` of the snapshot unless `--setup` is given.
Output goes to `<project>/batch/netgen/` (the log, `lvs.log`) unless `--dir` is given.

The netlists must have a `.subckt` for the cell: the netlist that `cdept netlist` writes
by default has the subcircuit header commented out, and netgen reports that it cannot
find the cell. The LVS-form netlist written by `cdept layout lvs` (`inv.spice` above) is
suitable. Comparing Magic's extracted netlist with a schematic netlist also needs the
device models of both sides to have matching names; for the PDK devices of IHP the
Magic netlist refers to `sg13_lv_nmos` and `sg13_lv_pmos` as subcircuits that no file
defines, so netgen creates placeholder cells.

## Where the results are

| Check | Folder |
|---|---|
| `cdept layout drc` / `lvs` | `~/.cache/cdept/verify/<project>/<library>/<cell>/drc` and `/lvs` (or `--dir`) |
| `cdept magic ...` | `<project>/magic/<library>/<cell>/` |
| `cdept netgen lvs` | `<project>/batch/netgen/` (or `--dir`) |

The `verify` folder is a cache and can be deleted. The `magic` and `batch` folders are
inside the project; add them to `.gitignore` unless you want to keep results under
version control.

## See also

- [Technology and the PDK](../concepts/technology.md): where the decks and styles come from
- [tech.toml and tech.catalog.json](../reference/files/tech-toml.md)
- [KLAYOUT.md](../KLAYOUT.md): how DRC and LVS run in KLayout, deck variables, results format
- [MAGIC.md](../MAGIC.md): Magic in batch mode
- [BATCH.md](../BATCH.md): netgen and the other batch tools
- [LINKING.md](../LINKING.md): schematic-to-layout link, net and pin checks
- [Create a project for a PDK](new-project.md)
