# Project folders

This page lists what is on disk in a CDePT project: which file or folder CDePT writes,
which program or command writes it, and whether it belongs in git. The concepts behind
the library, cell and view folders are in
[Projects, libraries, cells and views](../../concepts/projects.md).

## The tree

```text
myproject/
  cdept.toml                  project file                              commit
  cdept.local.toml            this user's PDK folder and variables      do not commit
  tech.toml                   technology snapshot                       commit
  tech.catalog.json           the long lists behind the snapshot        commit or not
  categories.toml             categories of read-only and flat libs     commit
  templates/*.toml            the project's own test templates          commit
  devices.toml                schematic-to-layout device map (any name) commit
  libs/
    analog/                   a library (cells layout)
      lib.toml                description, categories, copy rule        commit
      ota5t/                  a cell
        schematic/ota5t.sch   a view folder with its files             commit
        symbol/ota5t.sym
        layout/ota5t.gds
        layout/ota5t.gds.json sidecar of the layout                     commit
        pcell/rpoly.pcell.py
        tb_ac/setup.toml      a testbench view                          commit
        suite/suite.toml      a suite view                              commit
    old/                      a library in the flat layout
      inv.sym  inv.sch  inv.gds
  simulations/                written by cdept sim                      do not commit
    results.db                SQLite index of all runs
    analog/ota5t/tb_ac/       <library>/<cell>/<view>
      netlist.spice           the schematic netlist
      suite.json              the last suite report
      tt_27/                  one run point
        deck.sp  .spiceinit  out.log  out.raw  job.log  result.json
  magic/<library>/<cell>/     written by cdept magic                    do not commit
  .cdept/osdi/<name>-<hash>/  compiled Verilog-A (OSDI)                 do not commit
```

Everything outside `cdept.toml`, the libraries and the files named "commit" is generated
and can be made again. Libraries need not sit inside the project folder: a definition
can point anywhere, and a read-only library (a PDK's) usually lives outside it.

## Files in the project root

| File | Written by | Commit | Format |
|---|---|---|---|
| `cdept.toml` | `cdept init`, `cdept lib ...`, the GUI, by hand | yes | [cdept.toml](cdept-toml.md) |
| `cdept.local.toml` | `cdept init --pdk-root`, the PDK scan, Project Settings, by hand | no (CDePT adds it to `.gitignore` inside a git repository) | [cdept.local.toml](cdept-local-toml.md) |
| `tech.toml` | `cdept init --technology`, `cdept tech scan` and `refresh`, the New Project wizard | yes | [tech.toml](tech-toml.md) |
| `tech.catalog.json` | the same scan | optional; it can always be made again with `cdept tech refresh` | [TECH.md](../../TECH.md) section 5 |
| `categories.toml` | `cdept category ...`, the Library Manager | yes | [Category files](categories.md) |
| `templates/*.toml` | by hand | yes | [test-template.md](test-template.md) |
| the device map | by hand; named by `[klayout] devices` | yes | [LINKING.md](../../LINKING.md) section 3 |

## `lib.toml`

`lib.toml` sits in a library folder and holds data about the library as a whole. It is
optional. It is plain TOML; CDePT creates it when a library is created with
`cdept lib add`, and edits it in place, keeping comments.

```toml
# CDePT library 'logic'
format = 1
description = "Inverter and buffer, drawn with the generic devices of xschem"

[categories]
digital = ["buf", "inv"]

[copy]
exclude = ["results", "*.raw"]
```

| Key | Type | Default | Meaning |
|---|---|---|---|
| `format` | integer | `1` | Format of this file; a newer number loads with a warning. |
| `description` | string | none | One line shown in the Library Manager and by `cdept lib show`. Written by `cdept lib set --description` or the library dialog; a read-only library cannot get one. |
| `[categories]` | table | none | Categories of the library's cells: [Category files](categories.md). |
| `[copy]` `exclude` | list of strings | `["results"]` | Names (with `*` wildcards) of files and folders inside a view folder that copying a cell leaves out. The default skips result folders of testbench views. |

A `lib.toml` makes a folder count as a `cells` library when the layout is detected. A
library with design files directly in its folder (a flat library) should not have one.

## The library folders

The layouts of a library folder (`cells`, `flat`, `ref`), the view folders and how a
view is recognised are in
[Library layouts](../../concepts/projects.md#library-layouts) and
[Views](../../concepts/projects.md#views). The files CDePT adds next to the design files:

| File | Where | Written by | Commit |
|---|---|---|---|
| `<layout file>.json`, for example `inv.gds.json` | next to a GDS or OASIS file in a view folder (or beside it in a flat library) | CDePT, when it creates or edits a layout, links it to a schematic, or places cells; never by KLayout | yes |
| `setup.toml` | in a testbench view folder | `cdept sim new`, the Simulation tab, by hand | yes |
| `suite.toml` | in a suite view folder | by hand; run by `cdept sim suite` | yes |

The sidecar records which library, cell and view a layout file is, the structure names
it uses for cells of other views, and the link to its schematic, so that references
can follow renames. It is optional: a layout without one has its references searched by
name. The scanner does not list `*.json` sidecars as files of a view. A sidecar is
copied and renamed together with its layout. Its schema is in
[KLAYOUT.md](../../KLAYOUT.md) section 3.2 and [LINKING.md](../../LINKING.md).

Hidden files and folders (names starting with `.`) in a library are ignored. Files
whose names are not valid UTF-8 are not shown, and a warning says so.

## `simulations/`

The simulator runs of `cdept sim` and the GUI's Simulation tab go to
`<project>/simulations/<library>/<cell>/<view>/` for the testbench view
`library/cell/view`: the netlist of the schematic once as `netlist.spice`, then a
folder per run point (corner and temperature, for example `tt_27`) with the deck, the
simulator's log and raw file, the job log and a `result.json` record. `results.db` in
`simulations/` is an SQLite index over all `result.json` files; the folders are the source of truth and
`cdept sim results --reindex` rebuilds the index. The record format and the database
are described in [results.md](results.md) and [SIMULATION.md](../../SIMULATION.md).

The folder is generated. A `.gitignore` entry such as `/simulations/` keeps it out of
git. CDePT does not write that entry itself.

## `magic/`

`cdept magic extract`, `drc` and `lef` write their files (extracted SPICE, a
`<cell>.drc.json` report, LEF) to `<project>/magic/<library>/<cell>/`. Generated; do not commit.
Magic itself: [MAGIC.md](../../MAGIC.md).

## `.cdept/`

`.cdept/` is a hidden folder for generated data CDePT keeps per project. Only one part is
used so far:

| Path | Content | Written by |
|---|---|---|
| `.cdept/osdi/<name>-<hash>/` | OSDI libraries compiled from Verilog-A with OpenVAF, and the VACASK model files made from a PDK's ngspice models. The hash covers the sources, the flags and the compiler, so a changed model compiles into a new folder; the old folder stays until someone deletes `.cdept/osdi`. | `cdept openvaf compile`, `cdept vacask models`, and simulations with `simulator = "vacask"` |

Do not commit it. See [VACASK.md](../../VACASK.md).

## Outside the project folder

Some state is per user and not part of the project:

| Path | Content |
|---|---|
| `$CDEPT_INDEX_CACHE`, else `$XDG_CACHE_HOME/cdept/index/` (default `~/.cache/cdept/index/`) | Index cache: what a large GDS, LEF, Liberty, Verilog or SPICE file holds, so it is read once. Files under 64 KiB are not cached. `CDEPT_INDEX_CACHE=off` turns it off. |
| `$XDG_CACHE_HOME/cdept/verify/<project>/<library>/<cell>/<check>/` (default under `~/.cache`) | Working files of DRC, LVS and net-extraction runs. |
| `$XDG_DATA_HOME/Trash/` (default `~/.local/share/Trash/`) | Cells and views deleted through CDePT, see [Trash](../../concepts/projects.md#trash). |
| `$XDG_RUNTIME_DIR/cdept-xschem-<pid>-<n>/` (else the temporary folder) | Socket and scratch files of a running embedded xschem and KLayout helper; removed when it exits. |

## What to put in `.gitignore`

A project in git usually ignores:

```text
/cdept.local.toml
/simulations/
/magic/
/.cdept/
```

CDePT adds only `cdept.local.toml` by itself. The GUI shows git status per library, cell
and view (see [Git status](../../concepts/projects.md#git-status)); generated folders
that are not ignored show up there as untracked.

## See also

- [Projects, libraries, cells and views](../../concepts/projects.md)
- [cdept.toml](cdept-toml.md), [cdept.local.toml](cdept-local-toml.md), [tech.toml](tech-toml.md)
- [Category files](categories.md)
- [Simulation results](results.md)
- [KLAYOUT.md](../../KLAYOUT.md), [XSCHEM.md](../../XSCHEM.md), [SIMULATION.md](../../SIMULATION.md)
