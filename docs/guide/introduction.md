# Introduction

CDePT, the Chip Design Project Tool, is a project manager for analog and mixed-signal
chip design on open PDKs. It keeps the schematics, layouts, netlists, HDL files and
testbenches of a design together as libraries of cells with views, and starts the
open-source tools that work on them.

## What CDePT is

- A **project model**: a project is a folder with a [`cdept.toml`](../reference/files/cdept-toml.md)
  file that defines libraries. A library holds cells, and a cell holds views
  (schematic, symbol, layout, netlist, testbench, ...). Everything is plain files; there is
  no database to corrupt and git works on it directly. See [Projects](../concepts/projects.md).
- A **technology description**: CDePT reads a PDK once and writes a snapshot,
  [`tech.toml`](../reference/files/tech-toml.md), that names the files each tool needs
  (decks, model corners, device lists, libraries). See [Technology](../concepts/technology.md).
- A **simulation environment**: testbenches with corners, temperatures, sweeps, Monte Carlo,
  measurements and pass/fail specs, run in parallel, with the results stored.
  See [Simulation](../concepts/simulation.md).
- One **window** and one **command line** over all of it: schematics and layouts open in
  tabs of the CDePT window, and every operation that does not need a window is also a
  `cdept` command, so a design can be built, simulated and checked in a script or in CI.

## What CDePT is not

CDePT is not an EDA engine. It does not draw schematics, edit layout, extract netlists,
solve circuits or check design rules. The tools do: xschem, KLayout, Magic, netgen,
ngspice, Xyce, VACASK, Qucs-S, Yosys and OpenROAD. CDePT's work is to give them one project
model, one technology set-up and one place to run from, and to read their results back.
It does not replace any of them, and a design made with CDePT stays usable without it: the
files are the tools' own files.

The list of tools, with what CDePT uses each for, is in [External tools](../tools.md).

## The programs

| Program | What it is |
|---|---|
| `cdept` | The one program users run. Without a command it opens the GUI; with a command it runs headless. See [The graphical interface](gui.md) and [The command line](cli.md). |
| `cdept-xschem` | A helper process that contains xschem's engine. It draws schematics, edits them, netlists and answers hierarchy queries for `cdept`. |
| `cdept-klayout` | A Python helper run by KLayout (`klayout -z`) that drives a layout view without a window and sends its pixels to `cdept`. |

Inside the `cdept` program, `libcdept` holds the whole logic with no user interface, and
`cdept-gui` is the window. The command line and the GUI call the same library, so they
agree.

## Licences

CDePT is MIT licensed. xschem and KLayout are GPL programs. CDePT never links them: each runs as a separate
process and talks to `cdept` over a pipe or a socket, so the GPL applies to the helper
programs only. `cdept-xschem` carries its own licence file and is built and installed
apart from `cdept`; the KLayout helper is a script that runs inside KLayout. Other tools
(Magic, netgen, Xyce, Yosys, ...) are separate programs started in batch. `libngspice` is
loaded at run time when it is present and is not linked at build time.

## What works today

Working, and used in the examples of this manual:

- Projects and libraries, in the `cells` layout and in the `flat` layout; cells and
  views created, copied, renamed, moved and deleted from the GUI and the command line, with
  the references in other files rewritten; hierarchy and "used by" read from the files;
  categories and search; git status; import of existing xschem and LibMan folders.
- Technology snapshots made by scanning a PDK (IHP SG13G2, which the examples use, and
  sky130A scan); PDK discovery; a New Project wizard.
- Schematics in tabs through the embedded xschem; layouts in tabs through the embedded
  KLayout, with edit in place, PCells, DRC and LVS with the PDK's decks.
- Linking schematics with layouts: generate a layout from a schematic, check it, and
  cross-probe instances and nets.
- Testbenches: ngspice, Xyce and VACASK; corners, temperatures, sweeps, Monte Carlo;
  measurements with a calculator on the raw file; specs; test templates and suites;
  results stored per run.
- Batch runs of Magic, netgen, Yosys and OpenROAD from the command line.

Not done yet:

- Waveform viewer in the GUI (raw files can be evaluated with `cdept calc`).
- Signoff dashboard (the Problems panel is a placeholder) and post-layout simulation.
- Mixed-signal flows (Liberty and LEF generation, co-simulation).
- Magic as an embedded editor; Magic layout generators.
- SLURM or ssh job submission, importing and exporting CACE datasheets.
- PDKs other than IHP SG13G2 are less tested; sky130's models do not convert to VACASK.

The plan and the running list of open items are kept in [ROADMAP.md](../ROADMAP.md) and
[NOTES.md](../NOTES.md).

## About this manual

The **User guide** takes you from installation to a simulated testbench. The **Concepts**
pages explain how CDePT thinks about projects, technology and simulation. The **How-to**
pages are short recipes. The **Reference** defines the file formats, the commands and the
environment variables. Design notes and the pages of each tool integration are in
[Design notes and tool pages](../design.md).

## See also

- [Installation](installation.md)
- [Quickstart](quickstart.md)
- [External tools](../tools.md)
