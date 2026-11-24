# CDePT

[![CI](https://github.com/ckdur/cdept/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/ckdur/cdept/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

**CDePT** (Chip Design Project Tool) is a project manager and editor shell for analog and
mixed-signal chip design on open PDKs. It keeps schematics, layouts, netlists, HDL and
testbenches together as *libraries of cells with views*, and starts the open-source tools
that work on them: xschem, KLayout, Magic, netgen, ngspice, Xyce, VACASK, Qucs-S, Yosys,
OpenROAD, and the UberWave waveform viewer. The embedded editors run as helper processes
inside the CDePT window; the simulators and checkers run in batch.

The CI badge above shows whether the `main` branch builds, passes `cargo fmt`, `clippy`
and the test suite, and runs from an installed layout. Tests that need a PDK or a tool the
CI runner does not have (ngspice, KLayout, the IHP and sky130 PDKs) skip themselves there;
they are run locally before a push (`docs/TESTS.md`).

## What it does

- **Projects and libraries:** a project is a folder with a `cdept.toml`; libraries hold
  cells, cells hold views (schematic, symbol, layout, netlist, LEF abstract, testbench, ...).
  Rename, move, copy and delete follow the references between views, with rollback.
- **Technology:** a scan of the PDK (ihp-sg13g2 and sky130 work) writes a small, editable
  `tech.toml`: tool set-ups, DRC/LVS decks, model corners, and the PDK's libraries.
- **Embedded xschem and KLayout:** open, edit and cross-probe schematics and layouts in the
  CDePT window; generate and update a layout from its schematic; DRC and LVS with the PDK's
  decks.
- **Simulation:** testbench views with corners, temperatures, sweeps and Monte Carlo on
  ngspice (also in-process), Xyce and VACASK; measures, specs, a calculator, test templates
  and suites, a results store with statistics; CACE datasheet import and export.
- **Import:** bring GDS, SPICE, CDL, LEF, Liberty, Verilog and other files into a library as
  cells and views.
- **Waveforms:** open results in UberWave, including whole families and calculator results.

What is not there yet is listed in [docs/ROADMAP.md](docs/ROADMAP.md) and the open points
of [docs/NOTES.md](docs/NOTES.md).

## Quick start

```sh
git clone --recurse-submodules git@github.com:ckdur/cdept.git
cd cdept
cargo build && cargo build -p cdept-xschem     # the xschem helper needs flex, bison, Tcl and cairo
export PDK_ROOT=/path/to/pdks                  # the folder that holds ihp-sg13g2 (or sky130A)

demo/run.sh buffer      # the made-up technology (examples/buffer): no PDK needed
demo/run.sh ihp         # IHP SG13G2 (examples/ihp-inv): needs the PDK, KLayout and ngspice
target/debug/cdept --help
```

KLayout is built from a submodule (`cdept-klayout/build.sh`); ngspice, Xyce, VACASK, Magic
and the other tools are found on `PATH` or through `CDEPT_*` variables
(`docs/reference/environment.md`, `cdept tools`).

## Documentation

The manual is in `docs/` (Sphinx, Read the Docs style):

```sh
pip install -r docs/requirements.txt
make -C docs html          # docs/_build/html/index.html
```

It has a user guide, concepts, how-to recipes, and the reference for the file formats,
the command line and the environment variables. The pages about each tool's integration
(`docs/XSCHEM.md`, `KLAYOUT.md`, `MAGIC.md`, `NGSPICE.md`, `VACASK.md`, ...) are linked from
the manual.

## Repository layout

| Folder | What |
|---|---|
| `libcdept/` | core library: projects, libraries, technology, simulation, calculator, importers |
| `cdept/` | the command line program (`cdept`) |
| `cdept-gui/` | the GUI (egui) |
| `cdept-xschem/`, `cdept-klayout/` | helper programs that embed xschem and KLayout |
| `examples/` | `buffer` (no PDK) and `ihp-inv` (IHP SG13G2) projects |
| `demo/` | replayable GUI demos |
| `docs/` | the manual and the design notes |

## Licence

CDePT is released under the [MIT licence](LICENSE). xschem, KLayout, Magic, ngspice and the
other tools are separate programs with their own licences; the GPL ones are only ever
started as processes of their own and never linked into CDePT. UberWave is Symbiotic EDA's
own software and is integrated by the owner's choice.
