# Example project: buffer

A small CDePT project to try the library model and the embedded xschem engine. It uses
only the generic devices that come with xschem, so no PDK is needed.

```
cdept.toml                      the project: two libraries
libs/logic/                     library "logic" (cells layout)
  inv/symbol/inv.sym            inverter: symbol ...
  inv/schematic/inv.sch         ... and schematic (one NMOS, one PMOS)
  buf/symbol/buf.sym            buffer: symbol ...
  buf/schematic/buf.sch         ... and schematic (two inverters)
  inv/layout/inv.gds            inverter layout (generic layers 1/0, 2/0, 3/0)
  buf/layout/buf.gds            buffer layout: places inv twice as a reference only
  */layout/*.gds.json           sidecars: what each file is and where its references live
  rpoly/pcell/rpoly.pcell.py    a PCell view: Python for KLayout (a bar with two contacts)
  rdiv/layout/rdiv.gds          places rpoly twice (live PCells of the KLayout library "logic")
libs/bench/                     library "bench"
  tb_buf/schematic/tb_buf.sch   testbench: buffer, sources, load, simulation commands
```

## Try it

From the repository root, build once (the xschem helper is not part of the default
build; it needs flex, bison, Tcl and cairo, see `cdept-xschem/README.md`):

```sh
cargo build
cargo build -p cdept-xschem
```

Then, in this folder (or anywhere with `-p examples/buffer`):

```sh
cargo run -q -- ls                               # libraries, cells and views
cargo run -q -- netlist bench tb_buf             # tb_buf.spice, three levels deep
cargo run -q -- netlist logic buf -f verilog     # other formats: spectre, vhdl, tedax
cargo run -q -- xschem instances logic buf       # what is placed in a schematic
cargo run -q -- xschem svg bench tb_buf          # drawing as tb_buf.svg
cargo run -q -- xschem svg logic inv --view symbol
cargo run -q -- xschem link                      # are all symbols linked to a schematic?
cargo run -q --                                  # the main window, on this project
```

In the main window, double-click a schematic or symbol view in the Libraries panel:
it opens in a tab with xschem inside (xschem's keys and menus work as usual; `e`
descends into the selected instance, Ctrl+`e` comes back). The Hierarchy panel shows
`tb_buf → x1 buf → x1 inv, x2 inv`, the Inspector shows a picture of the selected view,
and the "Cell" menu has "Netlist to View".

The netlist of `tb_buf` holds its own transistor models (level 1), a `.tran` command
and a delay measurement, so `ngspice -b tb_buf.spice` runs it as it is and prints the
delay of the buffer.

## The `schematic=` attribute

xschem looks for a schematic next to its symbol. Here they are in different view
folders, so each symbol says where its schematic is (the `K {...}` block of
`inv.sym`):

```
schematic=logic/@symname/schematic/@symname.sch
```

CDePT writes this line when it creates a symbol view and rewrites it when a cell is
copied or renamed. To see what happens without it, delete the line from
`libs/logic/inv/symbol/inv.sym` and run:

```sh
cargo run -q -- netlist logic buf       # warns; `.subckt inv` comes out empty
cargo run -q -- xschem link             # says which symbol and what is expected
cargo run -q -- xschem link --fix       # writes the line back
```

The same files work in stock xschem when the folder `libs` is on its library path, e.g.
with this line in your `xschemrc`:

```tcl
append XSCHEM_LIBRARY_PATH :/path/to/examples/buffer/libs
```
