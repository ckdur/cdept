# External tools

CDePT does not simulate, draw or check anything itself. It organises a design and starts
the open-source tools that work on it. This page says which tool is used for what, how it
is started, and how to point CDePT at a particular copy. What a tool does and how CDePT
talks to it is documented once, in the page linked in the last column; the manual does not
repeat it.

`cdept tools` lists the tools found on this machine (see [Installation](guide/installation.md)).

## How a tool is run

| Mode | Meaning |
|---|---|
| Embedded helper | A helper program of its own, started once and kept running. The tool's window or picture is shown inside a CDePT tab. The helper is a separate process, so GPL code never links with CDePT. |
| Batch | The tool runs to completion as a process of its own, in a working folder, with a log file. CDePT reads what it wrote. |
| In-process | A shared library loaded at run time with `dlopen`. There is no link-time dependency. |
| Launched | The tool starts with its own window, on a file of the project. |

## The tools

| Tool | Used for | Run as | Override | Documentation |
|---|---|---|---|---|
| xschem | Schematic and symbol editing, netlisting, hierarchy, SVG pictures | Embedded helper `cdept-xschem`, which contains xschem's engine built from a submodule. A stock `xschem` on `PATH` is only detected. | `CDEPT_XSCHEM` (the helper), `CDEPT_XSCHEM_SHAREDIR` | [XSCHEM.md](XSCHEM.md) |
| KLayout | Layout editing, DRC and LVS with the PDK's decks, PCells, pictures, technology scan | Embedded helper: `klayout -z` runs the `cdept_klayout` Python helper. DRC, LVS and scans run as `klayout -b`. | `CDEPT_KLAYOUT`, `CDEPT_KLAYOUT_HELPER`, `CDEPT_KLAYOUT_HOME` | [KLAYOUT.md](KLAYOUT.md), [LINKING.md](LINKING.md) |
| Magic | Layout editing in a tab (`.mag` views, GDS read-only), layout from schematic with the PDK's device generators, DRC, extraction, LVS with netgen, LEF, GDS import/export | Embedded helper `cdept-magic`, which loads the installed Magic's `tclmagic.so` with Tk and X stand-ins; batch: `magic -dnull -noconsole` with a Tcl script; launched (Open With › Magic window) | `CDEPT_MAGIC`, `CDEPT_MAGIC_HELPER`, `CDEPT_MAGIC_TCLDIR` | [MAGIC.md](MAGIC.md) |
| netgen | LVS | Batch | `CDEPT_NETGEN` | [BATCH.md](BATCH.md) |
| ngspice | Default simulator of testbenches; live runs | Batch (`ngspice -b`) for testbenches; in-process (`libngspice`) for `cdept ngspice live` | `CDEPT_NGSPICE`, `CDEPT_LIBNGSPICE` | [NGSPICE.md](NGSPICE.md), [SIMULATION.md](SIMULATION.md) |
| Xyce | Testbench simulator (`simulator = "xyce"`) | Batch | `CDEPT_XYCE` | [BATCH.md](BATCH.md) |
| VACASK | Testbench simulator (`simulator = "vacask"`) | Batch | `CDEPT_VACASK`, `CDEPT_VACASK_PYTHON` | [VACASK.md](VACASK.md) |
| OpenVAF | Compiles Verilog-A models to OSDI for ngspice and VACASK, cached per technology | Batch | `CDEPT_OPENVAF` | [VACASK.md](VACASK.md) |
| Qucs-S | Second schematic tool: schematic to SPICE netlist, S-parameter viewer | Batch for conversion; launched for the editor and viewer | `CDEPT_QUCS`, `CDEPT_QUCS_SPAR` | [QUCS.md](QUCS.md) |
| UberWave | The waveform viewer: CDePT has none of its own | Built into the cdept executable when its source was there at build time (the UberWave submodule or `UBERWAVE_SRC`) (the window runs on a thread of the process; `cdept tools` says `built in (embedded)`), else launched as the separate program `UberWaveViewer` (`process: ...`): one raw file for an older build, families and calculator traces over IPC for a build that prints `ipc 1` in `--version`. Symbiotic EDA's own program | `CDEPT_UBERWAVE`, `CDEPT_UBERWAVE_BACKEND`, `UBERWAVE_SRC` (build) | [view-waveforms](howto/view-waveforms.md) |
| Yosys | Synthesis | Batch | `CDEPT_YOSYS` | [BATCH.md](BATCH.md) |
| OpenROAD | Digital place and route scripts | Batch | `CDEPT_OPENROAD` | [BATCH.md](BATCH.md) |
| ciel (or volare) | Installs and switches PDK versions under `$PDK_ROOT` | Batch, on request (`cdept pdk enable`) | none: `ciel`, else `volare`, on `PATH` | [TECH.md](TECH.md) |
| git | Per-view status in the Library Manager and `cdept status` | Batch (`git status`) | none: `git` on `PATH` | [ROADMAP.md](ROADMAP.md) (Phase 1) |
| LibMan | Optional: opens the project's libraries in IHP's library manager | Launched (`cdept libman open`) | none: `libman`, `LibMan` or `door` on `PATH` | [ROADMAP.md](ROADMAP.md) (Phase 1) |

`cdept tools` also detects tools that CDePT does not drive yet (Verilator, Icarus,
OpenSTA, gdsfactory, CACE, and others). They are listed with the phase of the plan that
integrates them; detection is all CDePT does with them. (CACE itself is not run either, but
CDePT imports and exports its datasheets: [CACE datasheets](reference/files/cace.md).)

## Opening views

A double click on a view opens it in the tool for its format: a CDePT tab for
schematics and symbols (xschem), layouts (KLayout for GDS/OASIS/LEF, Magic for `.mag`),
the tool's own window for the others (Qucs-S schematics, the desktop's text editor for
netlists and HDL). When several tabs could show a view (a GDS in KLayout or in Magic)
CDePT asks, and "Always open this kind of view this way" keeps the answer (per format,
in the user's settings). Right-click › Open With lists every tool for the view, the
tools' own windows included (KLayout, Magic, xschem windows). `cdept open <lib> <cell>
[view] [--tool T] [--list]` does the same from the command line (windows only)
(`libcdept::launch`).

Tool windows CDePT starts (Open With, the ▶ buttons of the tool bar, Tools › Launch)
are supervised: each writes its output into `<project>/.cdept/logs/<tool>-<n>.log`, a
file already open in a window of that tool is not opened twice, at most 8 windows of one
tool run at a time, and the Tools tab lists them with Stop and Restart. A GDS for "KLayout
window" goes to a KLayout that already runs the klive package (JSON on port 8082) before
a new window is started.

Python tools run with the project's interpreter: `[python] venv` or `interpreter` in
`cdept.toml` (or `python` in `cdept.local.toml`), else `$CDEPT_PYTHON`, else `python3`
([cdept.toml](reference/files/cdept-toml.md)).

## Licences

CDePT is MIT licensed and never links GPL code. xschem (GPL-2.0-or-later) and KLayout
(GPL-3.0) are used only through helper processes: `cdept-xschem` is a separate program
under the GPL, and the KLayout helper is a Python script run by KLayout. Magic's licence
is permissive (the University of California's notice, HPND-UC; see MAGIC.md); its helper
`cdept-magic` is a separate GPL-2.0-or-later program because it runs the xschem helper's
Tk stand-in. Every other tool
is a separate program that CDePT starts; `libngspice` is loaded at run time. Each tool keeps
its own licence; see the project of the tool. The split is explained in
[Introduction](guide/introduction.md).

## See also

- [Installation](guide/installation.md): where CDePT looks for the programs
- [Environment variables](reference/environment.md)
- [Design notes and tool pages](design.md)
