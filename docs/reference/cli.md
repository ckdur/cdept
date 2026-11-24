# Command reference

This page is generated from the program's own help texts by `scripts/gen-cli-docs.sh`;
do not edit it by hand. Every command has the options `-p, --project <PROJECT>`,
`-v, --verbose` and `-h, --help` (see the first section); they are left out of the
sections below. How the commands are organised is in [the command line guide](../guide/cli.md).

## cdept

```text
Chip Design Project Tool

Usage: cdept [OPTIONS] [COMMAND]

Commands:
  gui           Open the main window (default)
  init          Create a project (a cdept.toml file) in a folder
  info          Show the project: its file, the PDK in use ($PDK, $PDK_ROOT) and where each value comes from, the libraries
  ls            List the libraries, cells and views of the project
  lib           Change the library definitions in cdept.toml
  netlist       Write the netlist of a cell's schematic, with its hierarchy (uses xschem)
  xschem        Use the embedded xschem engine directly
  layout        Layout views: references between them, export, pictures (uses KLayout)
  klayout       Use the embedded KLayout directly
  pdk           The PDKs installed on this machine and their versions (ciel)
  ngspice       ngspice in-process (libngspice, loaded at run time)
  magic         Magic in batch mode: extraction, DRC, LEF export
  sim           Testbenches: decks and ngspice runs with corners and specs
  jobs          Remotes for run points: list them, check that they answer (ssh, SLURM)
  cace          CACE datasheets: import into testbenches and a suite, export with the results
  netgen        LVS with netgen (batch)
  yosys         Synthesis with Yosys (batch)
  openroad      Run OpenROAD scripts (batch)
  qucs          Qucs-S: schematic to SPICE netlist (headless), open in Qucs-S, S-parameter viewer
  wave          Open the waveforms of a run point (or a raw file) in UberWave, the waveform viewer
  vacask        VACASK models of the technology (made from its ngspice models and Verilog-A)
  openvaf       OpenVAF: compile Verilog-A to OSDI, cached per technology
  tech          The project's technology snapshot (tech.toml): scan the PDK, show, refresh
  calc          Evaluate a calculator expression on a SPICE raw file (`cdept calc --functions` lists them)
  find          Search the libraries: cells and views by name, kind, format and category, optionally inside the files (see `cdept find --help` for the query words)
  category      Categories: named groups of the cells of a library
  import        Propose a project (cdept.toml) for an existing design folder: its libraries from the xschemrc, the folders with design files or a LibMan project file
  import-files  Import files into a library
  libman        IHP LibMan: write the libraries as a LibMan project file, start LibMan
  cell          Cells: create, copy, rename, move, delete. What uses a cell follows
  view          Views of a cell: create, copy, rename, delete
  hier          The hierarchy below a cell, read from the files (schematic, layout, netlist, HDL)
  used-by       Which cells use a cell, through which views, how often
  status        The git status of the project, by library, cell and view
  tools         List the external tools found on PATH
  version       Print version and build information
  help          Print this message or the help of the given subcommand(s)

Options:
  -p, --project <PROJECT>  Project folder (the one holding cdept.toml). Defaults to the project the current folder belongs to
  -v, --verbose...         More log output (-v debug, -vv trace). `RUST_LOG` overrides this
  -h, --help               Print help
  -V, --version            Print version
```

### cdept gui

```text
Open the main window (default)

Usage: cdept gui [OPTIONS]

Options:
      --script <SCRIPT>    Run a script of GUI commands, for testing (see docs/GUI-TESTING.md)
```

### cdept init

```text
Create a project (a cdept.toml file) in a folder

Usage: cdept init [OPTIONS] [DIR]

Arguments:
  [DIR]  Folder of the new project [default: .]

Options:
      --name <NAME>              Project name. Defaults to the folder name
      --technology <TECHNOLOGY>  PDK name (`technology` in cdept.toml, `$PDK` for tools), e.g. ihp-sg13g2, or the path of a PDK folder. The PDK is scanned and what was found is written next to cdept.toml (tech.toml, see `cdept tech`)
      --pdk-root <PDK_ROOT>      Folder PDKs are installed in (`$PDK_ROOT` for tools); kept in cdept.local.toml, as it differs per machine. Default: where the PDK is found
      --no-scan                  Do not scan the PDK: only name it in cdept.toml
      --files-only               Scan files only: do not start KLayout (PCells) or xschem (xschemrc)
```

### cdept info

```text
Show the project: its file, the PDK in use ($PDK, $PDK_ROOT) and where each value comes from, the libraries

Usage: cdept info [OPTIONS]
```

### cdept ls

```text
List the libraries, cells and views of the project

Usage: cdept ls [OPTIONS] [LIBRARY] [CELL]

Arguments:
  [LIBRARY]  Only this library
  [CELL]     Only this cell of the library: its views, their files and what the files say about it (LEF class and size, Liberty area and pins, …)

Options:
  -l, --libraries          Only list libraries, not their cells
  -f, --files              One line per view, with its file and, when the file holds many cells (a standard-cell GDS, LEF, Liberty file), the cell inside it
```

### cdept lib

```text
Change the library definitions in cdept.toml

Usage: cdept lib [OPTIONS] <COMMAND>

Commands:
  add     Define a library. The folder is created if it does not exist
  remove  Remove a library definition. Its folder is left untouched
  rename  Rename a library (and, with --folder, its folder). What names it follows
  set     Change a library definition: its folder, read-only, layout, description
  show    Show a library definition: where it is defined, what is written there, what CDePT makes of it
  help    Print this message or the help of the given subcommand(s)
```

#### cdept lib add

```text
Define a library. The folder is created if it does not exist

Usage: cdept lib add [OPTIONS] <NAME> <PATH>

Arguments:
  <NAME>
          

  <PATH>
          Library folder, written to cdept.toml as given ($VAR is kept)

Options:
      --read-only
          Reference library: CDePT will not modify it

      --layout <LAYOUT>
          Folder layout. Detected from the folder when left out

          Possible values:
          - cells: <library>/<cell>/<view>/<files>
          - flat:  <library>/<cell>.<ext>, one view per file
          - ref:   <library>/<format>/<files>: a PDK reference library (libs.ref), read only

      --force
          Define it also when its folder gets in the way of another library (the same folder, or a folder of the same name, which xschem cannot tell apart)
```

#### cdept lib remove

```text
Remove a library definition. Its folder is left untouched

Usage: cdept lib remove [OPTIONS] <NAME>

Arguments:
  <NAME>  
```

#### cdept lib rename

```text
Rename a library (and, with --folder, its folder). What names it follows

Usage: cdept lib rename [OPTIONS] <LIBRARY> <NEW>

Arguments:
  <LIBRARY>  
  <NEW>      

Options:
      --folder             Also rename the library folder. xschem names a library by its folder, so the schematics that use its cells are rewritten
  -n, --dry-run            Say what would be done (files moved, files rewritten) and change nothing
      --keep-references    Move or copy the files only: leave every reference to them as it is
```

#### cdept lib set

```text
Change a library definition: its folder, read-only, layout, description.

The change is written to the file that defines the library (the project file or one it includes), keeping that file's comments. The description goes into the lib.toml of the library folder.

Usage: cdept lib set [OPTIONS] <NAME>

Arguments:
  <NAME>
          

Options:
      --path <PATH>
          Another folder, written as given (relative to the defining file; $VAR is kept). It must exist

      --read-only
          Reference library: CDePT will not modify it

      --writable
          Not a reference library any more

      --layout <LAYOUT>
          Folder layout: `cells`, `flat`, …, or `detect` to find it out from the folder each time the project is loaded

      --description <DESCRIPTION>
          

      --force
          Go on when the new folder gets in the way of another library
```

#### cdept lib show

```text
Show a library definition: where it is defined, what is written there, what CDePT makes of it

Usage: cdept lib show [OPTIONS] <NAME>

Arguments:
  <NAME>  
```

### cdept netlist

```text
Write the netlist of a cell's schematic, with its hierarchy (uses xschem)

Usage: cdept netlist [OPTIONS] <LIBRARY> <CELL>

Arguments:
  <LIBRARY>  
  <CELL>     

Options:
  -o, --output <OUTPUT>    Output file. Defaults to <cell>.<suffix of the format> in the current folder
      --into-view          Write the netlist into the cell itself, as its `netlist` view
  -f, --format <FORMAT>    [default: spice] [possible values: spice, spectre, verilog, vhdl, tedax]
      --view <VIEW>        The view to netlist, when the cell has several schematics
```

### cdept xschem

```text
Use the embedded xschem engine directly

Usage: cdept xschem [OPTIONS] <COMMAND>

Commands:
  svg             Draw a schematic or symbol view into an SVG file
  instances       List the instances placed in a schematic: name, symbol, type
  hierarchy       Show the hierarchy below a schematic: which cells it uses, through which instances
  make-symbol     Make the symbol of a cell from the pins of its schematic
  make-schematic  Make the schematic of a cell from the pins of its symbol
  link            Check that xschem will find the schematic behind every symbol
  tcl             Evaluate a Tcl script in the engine and print the result (for debugging)
  version         Print the version of the engine
  help            Print this message or the help of the given subcommand(s)
```

#### cdept xschem svg

```text
Draw a schematic or symbol view into an SVG file

Usage: cdept xschem svg [OPTIONS] <LIBRARY> <CELL>

Arguments:
  <LIBRARY>  
  <CELL>     

Options:
  -o, --output <OUTPUT>    Output file. Defaults to <cell>.svg in the current folder
      --view <VIEW>        The view to draw. Defaults to the schematic, else the symbol
```

#### cdept xschem instances

```text
List the instances placed in a schematic: name, symbol, type

Usage: cdept xschem instances [OPTIONS] <LIBRARY> <CELL>

Arguments:
  <LIBRARY>  
  <CELL>     

Options:
      --view <VIEW>        
```

#### cdept xschem hierarchy

```text
Show the hierarchy below a schematic: which cells it uses, through which instances

Usage: cdept xschem hierarchy [OPTIONS] <LIBRARY> <CELL>

Arguments:
  <LIBRARY>  
  <CELL>     

Options:
      --view <VIEW>        
  -a, --all                Also list the instances that are not subcircuits (transistors, pins, …)
```

#### cdept xschem make-symbol

```text
Make the symbol of a cell from the pins of its schematic.

The same as xschem's "Make symbol from schematic". In a `cells` library the symbol goes into the cell's `symbol` view and names its schematic.

Usage: cdept xschem make-symbol [OPTIONS] <LIBRARY> <CELL>

Arguments:
  <LIBRARY>
          

  <CELL>
          

Options:
  -f, --force
          Replace the symbol if the cell has one. Its drawing is lost
```

#### cdept xschem make-schematic

```text
Make the schematic of a cell from the pins of its symbol.

The same as xschem's "Make schematic from symbol": a pin of the schematic for each pin of the symbol, nothing else. In a `cells` library the schematic goes where the symbol's schematic= attribute says, normally the `schematic` view.

Usage: cdept xschem make-schematic [OPTIONS] <LIBRARY> <CELL>

Arguments:
  <LIBRARY>
          

  <CELL>
          

Options:
  -f, --force
          Replace the schematic if the cell has one. Its contents are lost
```

#### cdept xschem link

```text
Check that xschem will find the schematic behind every symbol.

In a `cells` library the schematic is not next to its symbol, so the symbol names it with a `schematic=` attribute. CDePT writes the attribute and keeps it right; this finds the symbols where it is missing or wrong, e.g. after files were copied by hand. Without it xschem netlists the cell empty.

Usage: cdept xschem link [OPTIONS] [LIBRARY] [CELL]

Arguments:
  [LIBRARY]
          Only this library

  [CELL]
          Only this cell

Options:
      --fix
          Write the attribute where it is missing or names a file that does not exist

      --foreign
          With --fix: also rewrite attributes that name another cell's schematic
```

#### cdept xschem tcl

```text
Evaluate a Tcl script in the engine and print the result (for debugging)

Usage: cdept xschem tcl [OPTIONS] <SCRIPT>

Arguments:
  <SCRIPT>  
```

#### cdept xschem version

```text
Print the version of the engine

Usage: cdept xschem version [OPTIONS]
```

### cdept layout

```text
Layout views: references between them, export, pictures (uses KLayout)

Usage: cdept layout [OPTIONS] <COMMAND>

Commands:
  refs          Check the references of a cell's layout to cells of other views
  export        Write a self-contained GDS (or OASIS) of a cell's layout, with every referenced cell filled in, e.g. for DRC, LVS and tape-out
  png           Draw a cell's layout into a PNG file. `--view` may also name a LEF abstract view (drawn read-only, with the cell's macro as top)
  drc           Run the PDK's DRC deck on a cell's layout (KLayout in batch mode)
  lvs           Compare a cell's layout with its schematic, with the PDK's LVS deck
  link          Generate or update a cell's layout from its schematic (Layout XL style)
  check-source  Compare a cell's layout with its schematic; fails if they are not in sync
  pins          List the pins of a cell's layout and check them (docs/LINKING.md §9): against the schematic's pins (when the cell has a schematic), their labels, and the layout file (pins on the technology's pin layers). Fails on a finding
  check-nets    Compare the nets of a cell's layout with its schematic's: opens, shorts, unconnected terminals, pins without a label (docs/LINKING.md §8)
  parallel      Devices in parallel (an array of transistors): list the groups found in the schematic, and record them in the layout's sidecar with your choices
  help          Print this message or the help of the given subcommand(s)
```

#### cdept layout refs

```text
Check the references of a cell's layout to cells of other views.

A layout holds its own cell; the cells it places are found through its sidecar (`<file>.json`) or by searching the libraries for a cell of that name. Lists every reference and how it was resolved; fails if one cannot be found.

Usage: cdept layout refs [OPTIONS] <LIBRARY> <CELL>

Arguments:
  <LIBRARY>
          

  <CELL>
          

Options:
      --fix
          Record what was found by searching, and fix a sidecar that names another cell, in the sidecar
```

#### cdept layout export

```text
Write a self-contained GDS (or OASIS) of a cell's layout, with every referenced cell filled in, e.g. for DRC, LVS and tape-out

Usage: cdept layout export [OPTIONS] --output <OUTPUT> <LIBRARY> <CELL>

Arguments:
  <LIBRARY>  
  <CELL>     

Options:
  -o, --output <OUTPUT>    Output file; `.oas` writes OASIS
      --view <VIEW>        The layout view, when the cell has several
```

#### cdept layout png

```text
Draw a cell's layout into a PNG file. `--view` may also name a LEF abstract view (drawn read-only, with the cell's macro as top)

Usage: cdept layout png [OPTIONS] <LIBRARY> <CELL>

Arguments:
  <LIBRARY>  
  <CELL>     

Options:
  -o, --output <OUTPUT>    Output file. Defaults to <cell>.png in the current folder
      --size <SIZE>        Picture size in pixels, as WIDTHxHEIGHT [default: 800x600]
      --view <VIEW>        
```

#### cdept layout drc

```text
Run the PDK's DRC deck on a cell's layout (KLayout in batch mode).

The deck is `[klayout] drc` in cdept.toml, else found in the PDK's libs.tech/klayout. Prints the violations per rule; fails if there are any.

Usage: cdept layout drc [OPTIONS] <LIBRARY> <CELL>

Arguments:
  <LIBRARY>
          

  <CELL>
          

Options:
      --deck <DECK>
          The deck to run instead of the project's

      --view <VIEW>
          The layout view, when the cell has several

  -D, --define <DEFINES>
          Further variables for the deck, as NAME=VALUE (`-rd` of KLayout)

      --dir <DIR>
          Folder for the stream, the report and KLayout's output. Defaults to ~/.cache/cdept/verify/<project>/<library>/<cell>/<drc|lvs>
```

#### cdept layout lvs

```text
Compare a cell's layout with its schematic, with the PDK's LVS deck.

The schematic is netlisted by xschem for LVS. The deck is `[klayout] lvs` in cdept.toml, else found in the PDK. Fails if the netlists do not match.

Usage: cdept layout lvs [OPTIONS] <LIBRARY> <CELL>

Arguments:
  <LIBRARY>
          

  <CELL>
          

Options:
      --schematic <SCHEMATIC>
          The schematic view, when the cell has several

      --netlist <NETLIST>
          Compare with this SPICE netlist instead of netlisting the schematic

      --deck <DECK>
          The deck to run instead of the project's

      --view <VIEW>
          The layout view, when the cell has several

  -D, --define <DEFINES>
          Further variables for the deck, as NAME=VALUE (`-rd` of KLayout)

      --dir <DIR>
          Folder for the stream, the report and KLayout's output. Defaults to ~/.cache/cdept/verify/<project>/<library>/<cell>/<drc|lvs>
```

#### cdept layout link

```text
Generate or update a cell's layout from its schematic (Layout XL style).

Every device of the schematic becomes a PCell of the PDK's KLayout library (see the device map, pdk/<PDK>/devices.toml and `[klayout] devices`), every instance of a project cell a reference to that cell's layout. New instances are placed in rows next to what the layout has; changed parameters are updated; the link is recorded in the layout's sidecar. Creates the layout view if there is none.

Usage: cdept layout link [OPTIONS] <LIBRARY> <CELL>

Arguments:
  <LIBRARY>
          

  <CELL>
          

Options:
      --schematic <SCHEMATIC>
          The schematic view, when the cell has several

      --view <VIEW>
          The layout view, when the cell has several

      --remove
          Also delete layout instances whose schematic instance is gone

      --dry-run
          Only print what would change
```

#### cdept layout check-source

```text
Compare a cell's layout with its schematic; fails if they are not in sync

Usage: cdept layout check-source [OPTIONS] <LIBRARY> <CELL>

Arguments:
  <LIBRARY>  
  <CELL>     

Options:
      --schematic <SCHEMATIC>  
      --view <VIEW>            
```

#### cdept layout pins

```text
List the pins of a cell's layout and check them (docs/LINKING.md §9): against the schematic's pins (when the cell has a schematic), their labels, and the layout file (pins on the technology's pin layers). Fails on a finding

Usage: cdept layout pins [OPTIONS] <LIBRARY> <CELL>

Arguments:
  <LIBRARY>  
  <CELL>     

Options:
      --schematic <SCHEMATIC>  
      --view <VIEW>            
      --json                   Print the pins and the findings as JSON
```

#### cdept layout check-nets

```text
Compare the nets of a cell's layout with its schematic's: opens, shorts, unconnected terminals, pins without a label (docs/LINKING.md §8).

The layout's nets are extracted with the PDK's LVS deck; the schematic's come from xschem, through the hierarchy. Layout instances are matched to schematic instances by the link (`cdept layout link`). Fails if the nets do not agree.

Usage: cdept layout check-nets [OPTIONS] <LIBRARY> <CELL>

Arguments:
  <LIBRARY>
          

  <CELL>
          

Options:
      --schematic <SCHEMATIC>
          

      --view <VIEW>
          

      --json
          Print the whole comparison as JSON

      --dir <DIR>
          Folder for the stream, the database and KLayout's output. Defaults to ~/.cache/cdept/verify/<project>/<library>/<cell>/nets
```

#### cdept layout parallel

```text
Devices in parallel (an array of transistors): list the groups found in the schematic, and record them in the layout's sidecar with your choices.

A joined group gets one flyline per open net, and one layout device may stand for all its members (an extraction that combines them). Without options the groups are listed and recorded.

Usage: cdept layout parallel [OPTIONS] <LIBRARY> <CELL>

Arguments:
  <LIBRARY>
          

  <CELL>
          

Options:
      --schematic <SCHEMATIC>
          

      --view <VIEW>
          

      --off <INSTANCE>
          Do not join the group holding this instance (repeat for more)

      --on <INSTANCE>
          Join the group holding this instance again

      --group <A,B,…>
          Make a group of your own: instance paths separated by commas

      --ungroup <INSTANCE>
          Remove a group of your own (any of its instances)
```

### cdept klayout

```text
Use the embedded KLayout directly

Usage: cdept klayout [OPTIONS] <COMMAND>

Commands:
  version  Print the version of KLayout the helper runs in
  python   Run Python in the helper and print the value of the last expression (for debugging)
  help     Print this message or the help of the given subcommand(s)
```

#### cdept klayout version

```text
Print the version of KLayout the helper runs in

Usage: cdept klayout version [OPTIONS]
```

#### cdept klayout python

```text
Run Python in the helper and print the value of the last expression (for debugging)

Usage: cdept klayout python [OPTIONS] <CODE>

Arguments:
  <CODE>  
```

### cdept pdk

```text
The PDKs installed on this machine and their versions (ciel)

Usage: cdept pdk [OPTIONS] <COMMAND>

Commands:
  ls        List the PDKs found on this machine
  info      Show what a PDK says about itself: version, where it comes from, its tools
  versions  List the PDK versions ciel (or volare) has installed
  enable    Put an installed PDK version in use (what `ciel enable` does)
  help      Print this message or the help of the given subcommand(s)
```

#### cdept pdk ls

```text
List the PDKs found on this machine.

Looked at, in this order: the project's PDK folder, $PDK_ROOT, the folders in $CDEPT_PDK_PATH, ~/.ciel, ~/.volare, /usr/share/pdk, /usr/local/share/pdk, ~/share/pdk. A PDK is a folder with libs.tech in it.

Usage: cdept pdk ls [OPTIONS]

Options:
      --root <DIR>
          Look in this folder too (may be given several times)

      --only
          Look only in the folders given with --root

      --json
          Print JSON
```

#### cdept pdk info

```text
Show what a PDK says about itself: version, where it comes from, its tools

Usage: cdept pdk info [OPTIONS] [PDK]

Arguments:
  [PDK]  A PDK name (`ihp-sg13g2`) or the path of a PDK folder. Default: the project's PDK

Options:
      --root <DIR>         Look in this folder too (may be given several times)
      --json               Print JSON
```

#### cdept pdk versions

```text
List the PDK versions ciel (or volare) has installed

Usage: cdept pdk versions [OPTIONS] [FAMILY]

Arguments:
  [FAMILY]  The PDK family (`sky130`, `gf180mcu`, `ihp-sg13g2`). Default: all

Options:
      --root <DIR>         The PDK folder ciel manages. Default: every folder `cdept pdk ls` looks at
      --remote             Also list the versions that can be fetched (needs the ciel program)
      --json               Print JSON
```

#### cdept pdk enable

```text
Put an installed PDK version in use (what `ciel enable` does).

The PDKs of that version are linked into the PDK folder. A version that is not installed is fetched with the ciel program when it is on PATH.

Usage: cdept pdk enable [OPTIONS] <FAMILY> <VERSION>

Arguments:
  <FAMILY>
          The PDK family (`sky130`, `gf180mcu`, `ihp-sg13g2`)

  <VERSION>
          The version (an open_pdks commit), or the start of it

Options:
      --root <DIR>
          The PDK folder ciel manages. Default: the first folder that has versions of this family
```

### cdept ngspice

```text
ngspice in-process (libngspice, loaded at run time)

Usage: cdept ngspice [OPTIONS] <COMMAND>

Commands:
  info  Say whether libngspice can be loaded and where it is looked for
  live  Run a deck in-process on ngspice's background thread and print the vectors as they arrive. A `.spiceinit` next to the deck is applied first; the deck's `.control` commands run, the analysis commands (`run`, `tran`, `ac`, `dc`, `op`) starting the background run
  help  Print this message or the help of the given subcommand(s)
```

#### cdept ngspice info

```text
Say whether libngspice can be loaded and where it is looked for

Usage: cdept ngspice info [OPTIONS]
```

#### cdept ngspice live

```text
Run a deck in-process on ngspice's background thread and print the vectors as they arrive. A `.spiceinit` next to the deck is applied first; the deck's `.control` commands run, the analysis commands (`run`, `tran`, `ac`, `dc`, `op`) starting the background run

Usage: cdept ngspice live [OPTIONS] <DECK>

Arguments:
  <DECK>  

Options:
      --every <EVERY>            Print one progress line for every N data points [default: 100]
      --halt-after <HALT_AFTER>  Halt the run after this many data points
```

### cdept magic

```text
Magic in batch mode: extraction, DRC, LEF export

Usage: cdept magic [OPTIONS] <COMMAND>

Commands:
  extract  Extract a layout to SPICE with the PDK's extraction style
  drc      Check a layout against the PDK's DRC rules; prints the rules violated
  lef      Write the LEF abstract of a layout
  help     Print this message or the help of the given subcommand(s)
```

#### cdept magic extract

```text
Extract a layout to SPICE with the PDK's extraction style.

Magic is `$CDEPT_MAGIC` or the one on PATH. Files go to <project>/magic/<library>/<cell>/ (<cell>.spice and Magic's .ext files).

Usage: cdept magic extract [OPTIONS] <LIBRARY> <CELL>

Arguments:
  <LIBRARY>
          

  <CELL>
          

Options:
      --style <STYLE>
          Extraction style (default: the technology's)

      --pex <PEX>
          Parasitics to extract

          Possible values:
          - none: Devices and connectivity only
          - c:    With capacitances
          
          [default: none]

      --timeout <TIMEOUT>
          Stop Magic after this long: 90s, 10m, 1h; 0 for no limit (default: [magic] timeout in cdept.toml, else 15m)
```

#### cdept magic drc

```text
Check a layout against the PDK's DRC rules; prints the rules violated.

Writes <cell>.drc.json next to the other files. Exit status 1 when not clean.

Usage: cdept magic drc [OPTIONS] <LIBRARY> <CELL>

Arguments:
  <LIBRARY>
          

  <CELL>
          

Options:
      --style <STYLE>
          DRC style, e.g. `drc(full)` (default: the technology's)

      --timeout <TIMEOUT>
          Stop Magic after this long: 90s, 10m, 1h; 0 for no limit (default: [magic] timeout in cdept.toml, else 15m)
```

#### cdept magic lef

```text
Write the LEF abstract of a layout

Usage: cdept magic lef [OPTIONS] <LIBRARY> <CELL>

Arguments:
  <LIBRARY>  
  <CELL>     

Options:
      --hide               Hide the inside of the cell (`lef write -hide`)
      --timeout <TIMEOUT>  Stop Magic after this long: 90s, 10m, 1h; 0 for no limit (default: [magic] timeout in cdept.toml, else 15m)
```

### cdept sim

```text
Testbenches: decks and ngspice runs with corners and specs

Usage: cdept sim [OPTIONS] <COMMAND>

Commands:
  run        Netlist a testbench, run every corner and temperature, print the spec table
  results    Show the stored results of a testbench: the latest run of every point and, over them, pass/fail counts and min/max/mean/std of every measurement
  templates  List the test templates (built-in, the technology's and the project's), or show one
  new        Create a testbench view from a template: a setup.toml, and with --dut a starter schematic that instantiates the DUT with the template's sources
  suite      Run a suite (a view with a suite.toml): its testbenches together, one table
  deck       Print the deck of the first run without starting the simulator
  help       Print this message or the help of the given subcommand(s)
```

#### cdept sim run

```text
Netlist a testbench, run every corner and temperature, print the spec table.

The testbench is a view with a setup.toml. ngspice is `$CDEPT_NGSPICE` or the one on PATH. Files go to <project>/simulations/<library>/<cell>/<view>/<run>/.

Usage: cdept sim run [OPTIONS] <LIBRARY> <CELL>

Arguments:
  <LIBRARY>
          

  <CELL>
          

Options:
      --view <VIEW>
          The testbench view. Default: the only one of the cell

      --corner <CORNERS>
          Run only these corners (repeat), instead of the setup's

  -j, --jobs <JOBS>
          Simulators running at once. Default: the smaller of the points and the cores

      --runs <RUNS>
          Monte Carlo: this many seeded runs of the statistical corner (overrides the setup's [montecarlo] runs, or turns Monte Carlo on)

      --mode <MODE>
          What the Monte Carlo runs draw: process, mismatch or both (default: the setup's)

      --seed <SEED>
          Base seed of the Monte Carlo runs

      --no-mc
          Run the corner/temperature/sweep grid although the setup has [montecarlo]

      --no-store
          Do not write result.json / results.db

      --backend <BACKEND>
          ngspice as a process per point (`ngspice -b`) or in-process through libngspice ($CDEPT_LIBNGSPICE). Default: the setup's simulator_backend, else process
          
          [possible values: inproc, process]

      --simulator <SIMULATOR>
          Run with this simulator instead of the setup's (`simulator` of setup.toml)
          
          [possible values: ngspice, xyce, vacask]

      --remote <REMOTE>
          Send the points to this remote ([jobs.remote.NAME] of cdept.toml: ssh or SLURM) instead of running them here. Default: the setup's `remote`, else [jobs] default

      --timeout <TIMEOUT>
          Time limit of each run point, like `10m`, `90s`, `1h30m` or a number of seconds; `0` is none. A point that runs longer is killed and ends as an error. Default: the setup's `timeout`, else `[sim] timeout` of cdept.toml, else 30m
```

#### cdept sim results

```text
Show the stored results of a testbench: the latest run of every point and, over them, pass/fail counts and min/max/mean/std of every measurement

Usage: cdept sim results [OPTIONS] <LIBRARY> <CELL>

Arguments:
  <LIBRARY>  
  <CELL>     

Options:
      --view <VIEW>        
      --all                Every stored run, not only the latest of each point
      --reindex            Rebuild results.db from the result.json files first
      --stats              Statistics of the latest runs: percentiles (p5/p50/p95), text histograms and the yield per spec (measured, and estimated from mean and sigma)
      --bins <BINS>        Histogram bins for --stats [default: 10]
      --sigma <SIGMA>      Sigmas of the mean +- k sigma yield estimate for --stats [default: 3]
```

#### cdept sim templates

```text
List the test templates (built-in, the technology's and the project's), or show one.

A `setup.toml` picks a template with `template = "ac_response"` and `[template_args]` (docs/TESTS.md).

Usage: cdept sim templates [OPTIONS] [COMMAND]

Commands:
  list  List the templates
  show  Document one: parameters, outputs, default specs, requirements
  help  Print this message or the help of the given subcommand(s)
```

##### cdept sim templates list

```text
List the templates

Usage: cdept sim templates list [OPTIONS]
```

##### cdept sim templates show

```text
Document one: parameters, outputs, default specs, requirements

Usage: cdept sim templates show [OPTIONS] <NAME>

Arguments:
  <NAME>  
```

#### cdept sim new

```text
Create a testbench view from a template: a setup.toml, and with --dut a starter schematic that instantiates the DUT with the template's sources

Usage: cdept sim new [OPTIONS] --template <TEMPLATE> <LIBRARY> <CELL> <VIEW>

Arguments:
  <LIBRARY>  
  <CELL>     
  <VIEW>     

Options:
      --template <TEMPLATE>  
      --arg <NAME=VALUE>     A template parameter, `name=value` (repeat)
      --dut <DUT>            The DUT, `library/cell` (needs its xschem symbol): write the cell's schematic
```

#### cdept sim suite

```text
Run a suite (a view with a suite.toml): its testbenches together, one table.

Exits non-zero when a test cannot run, a run fails or a spec fails or is missing.

Usage: cdept sim suite [OPTIONS] <LIBRARY> <CELL>

Arguments:
  <LIBRARY>
          

  <CELL>
          

Options:
      --view <VIEW>
          The suite view. Default: the only one of the cell

  -j, --jobs <JOBS>
          Simulators running at once. Default: the cores

      --all-columns
          A column for every measurement, not only the ones with a spec

      --no-store
          Do not write result.json / results.db / suite.json

      --json
          Print the report as JSON

      --simulator <SIMULATOR>
          Run every test with this simulator instead of the tests' (and the suite's `simulator`)
          
          [possible values: ngspice, xyce, vacask]

      --remote <REMOTE>
          Send every test's points to this remote ([jobs.remote.NAME] of cdept.toml)

      --timeout <TIMEOUT>
          Time limit of each run point (see `sim run --timeout`)
```

#### cdept sim deck

```text
Print the deck of the first run without starting the simulator

Usage: cdept sim deck [OPTIONS] <LIBRARY> <CELL>

Arguments:
  <LIBRARY>  
  <CELL>     

Options:
      --view <VIEW>            
      --simulator <SIMULATOR>  The deck for this simulator instead of the setup's [possible values: ngspice, xyce, vacask]
```

### cdept jobs

```text
Remotes for run points: list them, check that they answer (ssh, SLURM)

Usage: cdept jobs [OPTIONS] <COMMAND>

Commands:
  remotes  List the remotes of [jobs.remote.*] in cdept.toml (ssh and SLURM)
  help     Print this message or the help of the given subcommand(s)
```

#### cdept jobs remotes

```text
List the remotes of [jobs.remote.*] in cdept.toml (ssh and SLURM).

With --check each one is asked for a sign of life: `ssh host true`, or `sbatch --test-only` for SLURM. Exits non-zero when one does not answer.

Usage: cdept jobs remotes [OPTIONS] [NAME]

Arguments:
  [NAME]
          Only this remote

Options:
      --check
          Run the trivial command on every remote (or only on NAME)
```

### cdept cace

```text
CACE datasheets: import into testbenches and a suite, export with the results

Usage: cdept cace [OPTIONS] <COMMAND>

Commands:
  import  Read a CACE datasheet (YAML, cace_format 5.x) into testbench views and a suite
  export  Write a suite or testbench as a CACE datasheet, with the results of its latest runs
  help    Print this message or the help of the given subcommand(s)
```

#### cdept cace import

```text
Read a CACE datasheet (YAML, cace_format 5.x) into testbench views and a suite.

Every parameter simulated with ngspice becomes a testbench view of the cell (its conditions are corners, temperatures, [variables], [[sweep]] or [montecarlo]; its spec limits [[spec]] bounds), and a suite view `datasheet` lists them and keeps the rest of the datasheet. Prints the plan and the report of what was mapped, what is only kept and what to look at; nothing is written without `--write`.

Usage: cdept cace import [OPTIONS] --into <LIBRARY/CELL> <DATASHEET>

Arguments:
  <DATASHEET>
          The datasheet

Options:
      --into <LIBRARY/CELL>
          The library/cell the views go into (made when it does not exist)

      --view-prefix <VIEW_PREFIX>
          Put this in front of every view name
          
          [default: ""]

      --schematic <LIBRARY/CELL>
          The library/cell of the testbench schematic. Default: the cell's own
          
          [default: ""]

      --overwrite
          Replace views that exist

      --write
          Write the views. Without it the plan is only printed
```

#### cdept cace export

```text
Write a suite or testbench as a CACE datasheet, with the results of its latest runs.

The spec limits, conditions and results (minimum, typical and maximum over the run points, pass/fail) come from the CDePT files and the results store; what CDePT does not use comes back from the [cace] tables of an import.

Usage: cdept cace export [OPTIONS] <LIBRARY/CELL>

Arguments:
  <LIBRARY/CELL>
          The library/cell

Options:
      --view <VIEW>
          The suite or testbench view. Default: the cell's only suite, else its only testbench

  -o, --output <OUTPUT>
          The file to write. Default: standard output

      --no-results
          Leave the measured results out
```

### cdept netgen

```text
LVS with netgen (batch)

Usage: cdept netgen [OPTIONS] <COMMAND>

Commands:
  lvs   Compare two netlists with netgen (`netgen -batch lvs`) and print the result
  help  Print this message or the help of the given subcommand(s)
```

#### cdept netgen lvs

```text
Compare two netlists with netgen (`netgen -batch lvs`) and print the result.

The set-up file is `[netgen] setup` of the project's technology snapshot, else `--setup`, else an empty one. netgen is `$CDEPT_NETGEN` or the one on PATH. Exits with an error unless the circuits match.

Usage: cdept netgen lvs [OPTIONS] <LAYOUT> <LAYOUT_CELL> <SCHEMATIC> <SCHEMATIC_CELL>

Arguments:
  <LAYOUT>
          SPICE netlist of the layout (extracted) and the cell in it

  <LAYOUT_CELL>
          

  <SCHEMATIC>
          SPICE netlist of the schematic and the cell in it

  <SCHEMATIC_CELL>
          

Options:
      --setup <SETUP>
          

      --dir <DIR>
          Where the log goes. Default: <project>/batch/netgen
```

### cdept yosys

```text
Synthesis with Yosys (batch)

Usage: cdept yosys [OPTIONS] <COMMAND>

Commands:
  synth   Synthesise Verilog to a netlist, mapped to a Liberty file of the technology
  script  Run a Yosys script and print the design size it reports
  help    Print this message or the help of the given subcommand(s)
```

#### cdept yosys synth

```text
Synthesise Verilog to a netlist, mapped to a Liberty file of the technology.

The Liberty file comes from `--liberty`, else from the technology snapshot (`--library`, `--corner`; the digital defaults when left out). `--generic` maps nothing. Yosys is `$CDEPT_YOSYS` or the one on PATH.

Usage: cdept yosys synth [OPTIONS] <TOP> <SOURCES>...

Arguments:
  <TOP>
          

  <SOURCES>...
          

Options:
      --liberty <LIBERTY>
          

      --library <LIBRARY>
          

      --corner <CORNER>
          

      --generic
          

      --out <OUT>
          Netlist to write. Default: <top>.v in the run folder

      --dir <DIR>
          Run folder. Default: <project>/batch/yosys
```

#### cdept yosys script

```text
Run a Yosys script and print the design size it reports

Usage: cdept yosys script [OPTIONS] <SCRIPT>

Arguments:
  <SCRIPT>  

Options:
      --dir <DIR>          
```

### cdept openroad

```text
Run OpenROAD scripts (batch)

Usage: cdept openroad [OPTIONS] <COMMAND>

Commands:
  run   Run a Tcl script with OpenROAD; the log is kept. OpenROAD is `$CDEPT_OPENROAD` or the one on PATH
  help  Print this message or the help of the given subcommand(s)
```

#### cdept openroad run

```text
Run a Tcl script with OpenROAD; the log is kept. OpenROAD is `$CDEPT_OPENROAD` or the one on PATH

Usage: cdept openroad run [OPTIONS] <SCRIPT>

Arguments:
  <SCRIPT>  

Options:
      --dir <DIR>          Run folder. Default: <project>/batch/openroad
```

### cdept qucs

```text
Qucs-S: schematic to SPICE netlist (headless), open in Qucs-S, S-parameter viewer

Usage: cdept qucs [OPTIONS] <COMMAND>

Commands:
  export  Convert a cell's Qucs-S schematic to a SPICE netlist, without a window (`qucs-s -n`). Qucs-S is `$CDEPT_QUCS` or the one on PATH. The netlist holds the analysis of the schematic's own simulation component unless `--for-sim`
  open    Open a cell's Qucs-S schematic in the Qucs-S window
  spar    Open a Touchstone file (.s2p, .snp) in the Qucs-S S-parameter viewer
  help    Print this message or the help of the given subcommand(s)
```

#### cdept qucs export

```text
Convert a cell's Qucs-S schematic to a SPICE netlist, without a window (`qucs-s -n`). Qucs-S is `$CDEPT_QUCS` or the one on PATH. The netlist holds the analysis of the schematic's own simulation component unless `--for-sim`

Usage: cdept qucs export [OPTIONS] <LIBRARY> <CELL>

Arguments:
  <LIBRARY>  
  <CELL>     

Options:
      --view <VIEW>        The schematic view when the cell has several. Default: its first Qucs-S one
      --flavour <FLAVOUR>  [default: ngspice] [possible values: ngspice, xyce, cdl]
      --for-sim            Drop the schematic's own analysis and `.end`, as a testbench uses it
      --out <OUT>          Netlist to write. Default: <project>/batch/qucs/<library>/<cell>.cir
```

#### cdept qucs open

```text
Open a cell's Qucs-S schematic in the Qucs-S window

Usage: cdept qucs open [OPTIONS] <LIBRARY> <CELL>

Arguments:
  <LIBRARY>  
  <CELL>     

Options:
      --view <VIEW>        
```

#### cdept qucs spar

```text
Open a Touchstone file (.s2p, .snp) in the Qucs-S S-parameter viewer

Usage: cdept qucs spar [OPTIONS] <FILE>

Arguments:
  <FILE>  
```

### cdept wave

```text
Open the waveforms of a run point (or a raw file) in UberWave, the waveform viewer

Usage: cdept wave [OPTIONS] <FIRST> [CELL]

Arguments:
  <FIRST>  A raw file (ngspice or Xyce) or a Spectre PSF folder; or a library, followed by the cell of a testbench whose run point is opened
  [CELL]   The cell, to open the raw file of a run point of its testbench

Options:
      --analysis <ANALYSIS>  The analysis to preselect (`tran`, `ac`, `dc`, `noise`, `op`; a PSF analysis name). Default: the transient one when the file has several
      --view <VIEW>          The testbench view. Default: the only view of the cell that has results
      --point <POINT>        The run point (a folder of simulations/<lib>/<cell>/<view>/). Default: the only one
      --family               Open all the run points of the view in one viewer (tags = point names). Needs a viewer with IPC for more than the first; an older one opens the first and the other files are listed
      --points <POINTS>      Open these run points (comma separated) in one viewer, as `--family` does for all
      --calc <CALC>          Evaluate a calculator expression on the point (or on every point with `--family` / `--points`) and send waveform results to the viewer as traces (started when it is not running; needs a viewer with IPC). Scalar results are printed, as a table for several points. A complex waveform is sent as `<name> (mag dB)` and `<name> (phase)`
      --name <NAME>          The name of the trace sent by `--calc` (default: the expression); with several points `<name> @ <point>`
      --show <SHOW>          Signals to plot (comma separated, `v(out)` or just `out`; the tag of each file is added). Default: the nodes the testbench `save`s, else the ones its measures use, else the first four voltages that are not ground or a supply
      --no-show              Plot nothing: the viewer opens as it is (File traces are added in the viewer)
      --no-decimate          Send calculator traces whole, however many points they have (by default a trace with more than `[wave] max_points` of cdept.toml, 200000, goes as a min/max envelope of half of them, named `... (decimated)`)
      --list                 Do not start the viewer: for a file, print its analyses and signal counts (CDePT's own reader); for a cell, the raw files of all its run points
```

### cdept vacask

```text
VACASK models of the technology (made from its ngspice models and Verilog-A)

Usage: cdept vacask [OPTIONS] <COMMAND>

Commands:
  models  Make the technology's VACASK models from its ngspice models and Verilog-A sources (VACASK's ng2vc and OpenVAF), record them as `[models.vacask]` in tech.toml and print what could and could not be done
  help    Print this message or the help of the given subcommand(s)
```

#### cdept vacask models

```text
Make the technology's VACASK models from its ngspice models and Verilog-A sources (VACASK's ng2vc and OpenVAF), record them as `[models.vacask]` in tech.toml and print what could and could not be done.

Done on the first VACASK run anyway; this shows the result and `--force` makes it again. Files are cached in <project>/.cdept/osdi/<pdk>-<hash>/.

Usage: cdept vacask models [OPTIONS]

Options:
      --force
          Make them again even when the cache has them
```

### cdept openvaf

```text
OpenVAF: compile Verilog-A to OSDI, cached per technology

Usage: cdept openvaf [OPTIONS] <COMMAND>

Commands:
  compile  Compile a Verilog-A file to an OSDI library with OpenVAF (`$CDEPT_OPENVAF`, else openvaf-r / openvaf on PATH)
  help     Print this message or the help of the given subcommand(s)
```

#### cdept openvaf compile

```text
Compile a Verilog-A file to an OSDI library with OpenVAF (`$CDEPT_OPENVAF`, else openvaf-r / openvaf on PATH).

Without `--out` the result is cached in <project>/.cdept/osdi/<pdk>-<hash>/ (the hash covers the sources and flags): an unchanged model is not compiled twice. The path of the OSDI file is printed.

Usage: cdept openvaf compile [OPTIONS] <SOURCE>

Arguments:
  <SOURCE>
          

Options:
  -o, --out <OUT>
          Write the OSDI file here instead of the cache (always compiles)

  -D, --define <DEFINES>
          Macro for the preprocessor: `NAME` or `NAME=VALUE`

  -I, --include <INCLUDES>
          Folder searched for `include files
```

### cdept tech

```text
The project's technology snapshot (tech.toml): scan the PDK, show, refresh

Usage: cdept tech [OPTIONS] <COMMAND>

Commands:
  scan     Scan a PDK and write the project's technology snapshot
  show     Show the snapshot: a summary, or one section
  refresh  Scan the project's PDK again, say what changed, then replace the snapshot
  notes    What the scan could not find or had to guess, and whether the snapshot is still the PDK's
  devices  Print a draft device map made from the scan: which KLayout PCell each xschem symbol of the PDK becomes when a layout is generated from a schematic (docs/LINKING.md). For PDKs CDePT ships no map for. The matches are guesses: check them, save the file in the project and name it in `[klayout] devices`
  help     Print this message or the help of the given subcommand(s)
```

#### cdept tech scan

```text
Scan a PDK and write the project's technology snapshot.

Reads the PDK's files (KLayout technology, layers and decks, xschemrc and symbols, magic and netgen set-up, model files and their corners, the libraries in libs.ref) and writes tech.toml and tech.catalog.json next to cdept.toml. The PDK becomes the project's (`technology`); where it is on this machine goes into cdept.local.toml.

Usage: cdept tech scan [OPTIONS] [PDK]

Arguments:
  [PDK]
          A PDK name (`ihp-sg13g2`) or the path of a PDK folder. Default: the project's PDK

Options:
      --pdk-root <DIR>
          The folder PDKs are installed in. Default: the project's, $PDK_ROOT and the usual places (see `cdept pdk ls`)

      --files-only
          Read files only: do not start KLayout (PCells) or xschem (xschemrc)

      --print
          Print tech.toml instead of writing anything (needs no project)

  -y, --yes
          Replace an existing tech.toml without asking (scripts, no terminal)
```

#### cdept tech show

```text
Show the snapshot: a summary, or one section

Usage: cdept tech show [OPTIONS] [SECTION]

Arguments:
  [SECTION]  pdk, klayout, magic, netgen, xschem, models, devices or refs

Options:
      --catalog            The section's part of the catalog (the long lists) instead
      --json               Print JSON
```

#### cdept tech refresh

```text
Scan the project's PDK again, say what changed, then replace the snapshot.

tech.toml is replaced as a whole: what was edited by hand in it is lost (the old file is kept as tech.toml.bak). The differences are shown and, on a terminal, confirmed; without a terminal give --yes.

Usage: cdept tech refresh [OPTIONS]

Options:
      --files-only
          Read files only: do not start KLayout (PCells) or xschem (xschemrc)

      --dry-run
          Only say what would change

  -y, --yes
          Replace tech.toml without asking (scripts, no terminal)
```

#### cdept tech notes

```text
What the scan could not find or had to guess, and whether the snapshot is still the PDK's

Usage: cdept tech notes [OPTIONS]
```

#### cdept tech devices

```text
Print a draft device map made from the scan: which KLayout PCell each xschem symbol of the PDK becomes when a layout is generated from a schematic (docs/LINKING.md). For PDKs CDePT ships no map for. The matches are guesses: check them, save the file in the project and name it in `[klayout] devices`

Usage: cdept tech devices [OPTIONS]

Options:
  -o, --output <FILE>      Write the map to this file instead of printing it
```

### cdept calc

```text
Evaluate a calculator expression on a SPICE raw file (`cdept calc --functions` lists them)

Usage: cdept calc [OPTIONS] [EXPR]

Arguments:
  [EXPR]  The expression, e.g. `ugbw(v("out") / v("in"))` or `delay(v("in"), v("out"), 0.6)`

Options:
      --raw <RAW>          The raw file (ngspice or Xyce, binary or ascii; a run's `out.raw`)
      --plot <PLOT>        Look only in this plot: its number, its kind (tran, ac, dc, noise, op) or part of its name
      --var <VARS>         A name for the expression, `name=value` (SPICE suffixes work: `vdd=1.2`, `cload=10f`). May be repeated
      --list               List the plots and vectors of the raw file and stop
      --functions          List the functions and stop
      --dump               For a waveform result, print every point (x y), not the summary
```

### cdept find

```text
Search the libraries: cells and views by name, kind, format and category, optionally inside the files (see `cdept find --help` for the query words)

Usage: cdept find [OPTIONS] [QUERY]...

Arguments:
  [QUERY]...  What to look for: words that must be in the cell name (a few letters in order also match), patterns with * ? [a-z], `library/cell`, and filters written `lib:NAME kind:KIND view:NAME format:SUFFIX cat:CATEGORY in:content`

Options:
  -l, --lib <LIBRARY>        Only in this library (a pattern is fine). May be repeated
  -k, --kind <KIND>          Only views of this kind (schematic, symbol, layout, netlist, …). The hits are then views, not cells
  -f, --format <FORMAT>      Only views of this file format, by suffix (gds, sch, v, …)
  -c, --category <CATEGORY>  Only cells of this category (or of one below it)
      --content              Also look inside schematics, symbols, netlists and HDL files: pins, instances and text lines that have every word
  -n, --limit <LIMIT>        The most hits to print [default: 50]
      --json                 Print the hits as JSON
```

### cdept category

```text
Categories: named groups of the cells of a library

Usage: cdept category [OPTIONS] <COMMAND>

Commands:
  ls       List the categories of a library (of every library when none is given) and their cells
  add      Put cells into a category (made if it is new). Without cells: make an empty category. Nested categories are written `analog/amplifiers`
  rm       Take cells out of a category. Without cells: remove the category (and those below it); its cells stay in the library
  rename   Rename or move a category, with the categories below it
  of       Show the categories a cell is in
  suggest  Propose categories for a library and, with --write, add them: by the `type` of the xschem symbols (nmos, resistor, subcircuit, …) or, for standard cells, by function (and, nand, dff, …)
  adopt    Copy the categories IHP LibMan keeps in the library folder (*.group files) into CDePT's own files, so they can be changed here
  help     Print this message or the help of the given subcommand(s)
```

#### cdept category ls

```text
List the categories of a library (of every library when none is given) and their cells

Usage: cdept category ls [OPTIONS] [LIBRARY]

Arguments:
  [LIBRARY]  

Options:
      --names              Only the category names, not their cells
```

#### cdept category add

```text
Put cells into a category (made if it is new). Without cells: make an empty category. Nested categories are written `analog/amplifiers`

Usage: cdept category add [OPTIONS] <LIBRARY> <CATEGORY> [CELLS]...

Arguments:
  <LIBRARY>   
  <CATEGORY>  
  [CELLS]...  
```

#### cdept category rm

```text
Take cells out of a category. Without cells: remove the category (and those below it); its cells stay in the library

Usage: cdept category rm [OPTIONS] <LIBRARY> <CATEGORY> [CELLS]...

Arguments:
  <LIBRARY>   
  <CATEGORY>  
  [CELLS]...  
```

#### cdept category rename

```text
Rename or move a category, with the categories below it

Usage: cdept category rename [OPTIONS] <LIBRARY> <CATEGORY> <NEW>

Arguments:
  <LIBRARY>   
  <CATEGORY>  
  <NEW>       
```

#### cdept category of

```text
Show the categories a cell is in

Usage: cdept category of [OPTIONS] <LIBRARY> <CELL>

Arguments:
  <LIBRARY>  
  <CELL>     
```

#### cdept category suggest

```text
Propose categories for a library and, with --write, add them: by the `type` of the xschem symbols (nmos, resistor, subcircuit, …) or, for standard cells, by function (and, nand, dff, …)

Usage: cdept category suggest [OPTIONS] <LIBRARY>

Arguments:
  <LIBRARY>  

Options:
      --write              
```

#### cdept category adopt

```text
Copy the categories IHP LibMan keeps in the library folder (*.group files) into CDePT's own files, so they can be changed here

Usage: cdept category adopt [OPTIONS] <LIBRARY>

Arguments:
  <LIBRARY>  
```

### cdept import

```text
Propose a project (cdept.toml) for an existing design folder: its libraries from the xschemrc, the folders with design files or a LibMan project file

Usage: cdept import [OPTIONS] [FOLDER]

Arguments:
  [FOLDER]  The folder to look at: one with an xschemrc, schematic and layout folders, or an IHP LibMan project file. Default: the current folder

Options:
      --write              Write the project file. Without it the plan is only printed
      --name <NAME>        The project name (default: the folder name)
      --into <INTO>        Put cdept.toml into this folder instead of the scanned one (created if missing); the libraries stay where they are
      --libman <LIBMAN>    Import from this LibMan project file (*.projects). Those in the folder are found by themselves
      --skip <SKIP>        Leave these libraries out (names as the plan prints them). May be repeated
      --with <WITH>        List these libraries although the plan leaves them unticked (a PDK's tests and examples). May be repeated; `all` ticks every library
      --add                Add the libraries to the project the folder already has (or the one given with -p) instead of making a new one
      --no-helper          Do not run the xschemrc with the xschem helper: read its text only
      --toml               Print the project file the plan stands for instead of the plan
```

### cdept import-files

```text
Import files into a library.

Reads the files and prints what would come in: which cells, in which views, written where. Nothing is written without `--write`.

A file that holds several cells is taken apart: a GDSII file gives one cell per top structure (with the structures it places), a SPICE or CDL file one per `.subckt` (with the subcircuits it uses and the header of the file: `.include`, `.model`, `.param`), a LEF file one per `MACRO` (its layers and vias become the cell `<file>_tech`), a Liberty file one per `cell` group, a Verilog file one per module. Use `--no-split` to take each file as one cell named after it. xschem, Qucs-S, Magic, VHDL, Verilog-A and OASIS files are copied as they are.

Cells of the same name from several files (an `inv` in a GDS and in a SPICE file) become one cell with several views: `layout`, `netlist`, `cdl`, `abstract`, `liberty`, `verilog`, `schematic`, `symbol`, ...

Compressed files (`.gds.gz`) are refused: decompress them first. Files of unknown format are reported, not guessed.

Usage: cdept import-files [OPTIONS] --to <LIBRARY> <FILES>...

Arguments:
  <FILES>...
          The files to import

Options:
      --to <LIBRARY>
          The library they go into (a `cells` or `flat` library of the project)

      --split
          Take the cells out of files that hold several (the default)

      --no-split
          Take every file as one cell named after it, whatever is inside

      --prefix <PREFIX>
          Put this in front of every cell name
          
          [default: ""]

      --name <OLD=NEW>
          Give a cell of a file another name: OLD=NEW (before the prefix). May be repeated

      --on-clash <ON_CLASH>
          What to do with a view the library has already

          Possible values:
          - skip:      Keep what the library has; the other views of the cell still come in
          - rename:    Import the cell under another name (`inv_1`)
          - overwrite: Replace the existing file of the view
          
          [default: skip]

      --views <FORMAT>
          Take only files of these formats (names as in `[suffixes]`: gds, spice, cdl, lef, liberty, verilog, xschem-schematic, ...). Comma separated or repeated

      --write
          Write the files. Without it the plan is only printed

  -l, --long
          List every view that would come in, not only the summary
```

### cdept libman

```text
IHP LibMan: write the libraries as a LibMan project file, start LibMan

Usage: cdept libman [OPTIONS] <COMMAND>

Commands:
  export  Print the project's libraries as a LibMan project file (define(...) lines)
  open    Start LibMan (libman or door on PATH) on the project's libraries
  show    Show what a LibMan project file defines: libraries, their folders, the PDK
  help    Print this message or the help of the given subcommand(s)
```

#### cdept libman export

```text
Print the project's libraries as a LibMan project file (define(...) lines)

Usage: cdept libman export [OPTIONS]

Options:
  -o, --output <OUTPUT>    Write it to this file instead of printing it
```

#### cdept libman open

```text
Start LibMan (libman or door on PATH) on the project's libraries

Usage: cdept libman open [OPTIONS]
```

#### cdept libman show

```text
Show what a LibMan project file defines: libraries, their folders, the PDK

Usage: cdept libman show [OPTIONS] <FILE>

Arguments:
  <FILE>  
```

### cdept cell

```text
Cells: create, copy, rename, move, delete. What uses a cell follows

Usage: cdept cell [OPTIONS] <COMMAND>

Commands:
  new     Create a cell, empty or with a first view
  copy    Copy a cell, into the same library or another one
  rename  Rename a cell. The schematics, layouts, netlists and HDL views that use it follow
  move    Move a cell into another library. What uses it follows
  rm      Delete a cell with all its views (to the trash). Lists what uses it first
  help    Print this message or the help of the given subcommand(s)
```

#### cdept cell new

```text
Create a cell, empty or with a first view

Usage: cdept cell new [OPTIONS] <LIBRARY> <CELL>

Arguments:
  <LIBRARY>
          

  <CELL>
          

Options:
      --view <VIEW>
          The first view of the cell. A flat library has no empty cells: give one

          Possible values:
          - schematic: xschem schematic
          - symbol:    xschem symbol
          - layout:    GDSII layout
          - netlist:   SPICE netlist
          - hdl:       Verilog
          - pcell:     KLayout PCell in Python
          - magic:     Magic layout
```

#### cdept cell copy

```text
Copy a cell, into the same library or another one.

The copy's own files point at each other (its symbol at its schematic, its sidecars name it); what it places stays the original, unless the cells below are copied too.

Usage: cdept cell copy [OPTIONS] <LIBRARY> <CELL> <NEW>

Arguments:
  <LIBRARY>
          

  <CELL>
          

  <NEW>
          Name of the copy

Options:
      --to <LIBRARY>
          The library the copy goes into. Default: the same library

      --with-hierarchy
          Also copy the cells of the same library it is built from, under their names (needs another library), and make the copies use each other

  -n, --dry-run
          Say what would be done (files moved, files rewritten) and change nothing

      --keep-references
          Move or copy the files only: leave every reference to them as it is
```

#### cdept cell rename

```text
Rename a cell. The schematics, layouts, netlists and HDL views that use it follow

Usage: cdept cell rename [OPTIONS] <LIBRARY> <CELL> <NEW>

Arguments:
  <LIBRARY>  
  <CELL>     
  <NEW>      

Options:
  -n, --dry-run            Say what would be done (files moved, files rewritten) and change nothing
      --keep-references    Move or copy the files only: leave every reference to them as it is
```

#### cdept cell move

```text
Move a cell into another library. What uses it follows

Usage: cdept cell move [OPTIONS] <LIBRARY> <CELL> <TO_LIBRARY>

Arguments:
  <LIBRARY>     
  <CELL>        
  <TO_LIBRARY>  

Options:
      --name <NAME>        Another name in that library. Default: the same name
  -n, --dry-run            Say what would be done (files moved, files rewritten) and change nothing
      --keep-references    Move or copy the files only: leave every reference to them as it is
```

#### cdept cell rm

```text
Delete a cell with all its views (to the trash). Lists what uses it first

Usage: cdept cell rm [OPTIONS] <LIBRARY> <CELL>

Arguments:
  <LIBRARY>  
  <CELL>     

Options:
      --permanent          Delete for good instead of moving to the trash
  -n, --dry-run            Say what would be deleted and who uses it, and change nothing
```

### cdept view

```text
Views of a cell: create, copy, rename, delete

Usage: cdept view [OPTIONS] <COMMAND>

Commands:
  new     Create a view of a cell from a template (the cell is created if needed)
  copy    Copy a view, into the same cell under another name or into another cell
  rename  Rename a view (a view folder of a `cells` library). What names it follows: the symbol's schematic= attribute, the layouts that place or are linked with it
  rm      Delete a view (to the trash). Lists what uses it first
  help    Print this message or the help of the given subcommand(s)
```

#### cdept view new

```text
Create a view of a cell from a template (the cell is created if needed)

Usage: cdept view new [OPTIONS] <LIBRARY> <CELL> <KIND>

Arguments:
  <LIBRARY>
          

  <CELL>
          

  <KIND>
          Possible values:
          - schematic: xschem schematic
          - symbol:    xschem symbol
          - layout:    GDSII layout
          - netlist:   SPICE netlist
          - hdl:       Verilog
          - pcell:     KLayout PCell in Python
          - magic:     Magic layout
```

#### cdept view copy

```text
Copy a view, into the same cell under another name or into another cell

Usage: cdept view copy [OPTIONS] <LIBRARY> <CELL> <VIEW>

Arguments:
  <LIBRARY>  
  <CELL>     
  <VIEW>     

Options:
      --name <NAME>           Name of the copy. Default: the same name
      --to-library <LIBRARY>  The library the copy goes into. Default: the same library
      --to-cell <CELL>        The cell the copy goes into. Default: the same cell
  -n, --dry-run               Say what would be done (files moved, files rewritten) and change nothing
      --keep-references       Move or copy the files only: leave every reference to them as it is
```

#### cdept view rename

```text
Rename a view (a view folder of a `cells` library). What names it follows: the symbol's schematic= attribute, the layouts that place or are linked with it

Usage: cdept view rename [OPTIONS] <LIBRARY> <CELL> <VIEW> <NEW>

Arguments:
  <LIBRARY>  
  <CELL>     
  <VIEW>     
  <NEW>      

Options:
  -n, --dry-run            Say what would be done (files moved, files rewritten) and change nothing
      --keep-references    Move or copy the files only: leave every reference to them as it is
```

#### cdept view rm

```text
Delete a view (to the trash). Lists what uses it first

Usage: cdept view rm [OPTIONS] <LIBRARY> <CELL> <VIEW>

Arguments:
  <LIBRARY>  
  <CELL>     
  <VIEW>     

Options:
      --permanent          Delete for good instead of moving to the trash
  -n, --dry-run            Say what would be deleted and who uses it, and change nothing
```

### cdept hier

```text
The hierarchy below a cell, read from the files (schematic, layout, netlist, HDL)

Usage: cdept hier [OPTIONS] <LIBRARY> <CELL>

Arguments:
  <LIBRARY>  
  <CELL>     

Options:
      --view <VIEW>        Which hierarchy: through the schematics, the layouts, the netlists or the HDL views. Default: the first of these the cell has a view of [possible values: schematic, layout, netlist, hdl]
      --all                Also list what has nothing below: devices, pins, PCells
      --json               Print JSON
```

### cdept used-by

```text
Which cells use a cell, through which views, how often

Usage: cdept used-by [OPTIONS] <LIBRARY> <CELL>

Arguments:
  <LIBRARY>  
  <CELL>     

Options:
      --view <VIEW>        Only what uses this view of the cell
      --json               Print JSON
```

### cdept status

```text
The git status of the project, by library, cell and view

Usage: cdept status [OPTIONS] [LIBRARY]

Arguments:
  [LIBRARY]  Only this library

Options:
      --all                Also list the cells that are as committed
      --json               Print as JSON
```

### cdept tools

```text
List the external tools found on PATH

Usage: cdept tools [OPTIONS]
```

### cdept version

```text
Print version and build information

Usage: cdept version [OPTIONS]
```

