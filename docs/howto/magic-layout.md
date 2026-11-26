# Draw a layout in Magic

This walks through a Magic layout in a CDePT project: open it in a tab, make it from the
schematic, check it, and keep its references right. The example is
`examples/sky130-inv` (an inverter in sky130A). Background: [MAGIC.md](../MAGIC.md).

## What you need

- Magic 8.3 with its Tcl build (`magic --prefix` names the folder of `tclmagic.so`), on
  `PATH` or in `$CDEPT_MAGIC`; netgen for LVS.
- The PDK (sky130A or ihp-sg13g2) and a project made for it (`cdept init --technology
  sky130A`), so that the technology snapshot names the PDK's magicrc.
- The helpers built next to `cdept`: `cargo build -p cdept-magic -p cdept-xschem`
  (`cdept-magic` needs the Tcl, Tk and cairo development headers).

## Open a Magic view

Double-click a `layout` view whose file is a `.mag`: it opens in a tab, drawn by Magic.
Magic's key macros work over the drawing (`v` fits the cell, `z`/`Z` zoom, `b` the box,
`s` selects, `a` selects what is under the box, `d` deletes, `x` expands); the left and
right buttons set the box corners. Type Magic commands in the line under the drawing
(`:` jumps there), e.g. `box 0 0 2um 1um` then `paint m1`. Magic's answers appear in the
messages below it.

A new Magic view: Library Manager › New View › "Layout (Magic)" (the file starts with
the technology of the project). A GDS view can be looked at in Magic too: right-click ›
Open With › Magic (tab); a double click on a GDS asks which tool (tick "Always" to stop
asking).

Save with Magic › Save (or Ctrl+S). CDePT then writes the paths of placed project cells
relative to the file and updates `<cell>.mag.json`.

## Make the layout from the schematic

With the schematic drawn (`cells/inv/schematic`), choose Cell › Generate / Update from
Schematic in the layout tab, or run

```sh
cdept magic link cells inv
```

The PDK's device generators place `M1` and `M2` with the schematic's W, L, nf, m, named
as in the schematic. Change a size in the schematic, save, and run it again: only that
device is made again. `cdept magic check-source cells inv` says whether the layout is in
sync (exit status 1 when not). Select a device in the layout: the schematic tab selects
it too, and the other way round.

The map from symbols to generators is `pdk/<PDK>/magic-devices.toml`; a PDK without one
gets no devices (the instances are listed as problems).

## Check it

- DRC in the tab: the DRC button checks the cell as it is now; click a result to box
  and zoom to it. In batch: `cdept magic drc cells inv` (exit 1 when not clean).
- LVS: Cell › LVS against Schematic, or `cdept magic lvs cells inv`: Magic extracts,
  xschem netlists the schematic, netgen compares with the PDK's set-up.
- `cdept magic lef cells inv`, `cdept magic export-gds cells inv`.

## Use cells of other libraries

Place a cell of another library from the command line (`getcell inv` finds every Magic
view of the project's libraries) and save. Rename or move the placed cell from the
Library Manager: the `use` lines and sidecars of every view that places it follow.
`cdept magic refs` lists views whose placed cells cannot be found or whose paths are
absolute; `--fix` rewrites what can be fixed.

## Bring in an existing GDS

```sh
cdept magic import-gds chip.gds mylib --cell top
```

makes one Magic view per cell (the asked one and what it places), with relative paths
between them.

## Open the tool's own window

`cdept open cells inv layout --tool magic` (or Open With › Magic window) starts a Magic
window with the project's set-up; its output goes to `<project>/.cdept/logs/`. The Tools
tab lists the windows CDePT started, with Stop and Restart.
