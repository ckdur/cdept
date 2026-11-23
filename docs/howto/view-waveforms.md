# View waveforms

CDePT has no waveform viewer of its own. The viewer is **UberWave**, a separate program
(GPL-2.0, C++ with Dear ImGui and ImPlot, built for very large ngspice raw files and Spectre
PSF results). CDePT starts it on the raw file of a run point, as a separate process: it
never links to it, so CDePT itself stays MIT.

What stays in CDePT are the numbers: measurements, the [calculator](../CALC.md)
(`cdept calc`, `[[output]]` in `setup.toml`) and the spec table of the Simulation tab. UberWave
is for looking at the curves.

## Build UberWave (Linux)

UberWave lives in its own repository (`uberwave`). Its `Docs/Building.md` has the details;
the short form:

```bash
sudo apt install build-essential cmake git libgtk-3-dev   # GTK 3 development files, a C++20 compiler
cd uberwave
cmake -B build -DCMAKE_BUILD_TYPE=Release
cmake --build build --target UberWaveViewer
```

CMake fetches the libraries it needs (ImGui, ImPlot, GLFW, ...) from the network, so the
first configuration needs it. The program is `build/wp5-viewer/UberWaveViewer`.

CDePT finds it, in this order:

1. `$CDEPT_UBERWAVE`, the path of the program;
2. `UberWaveViewer` (or `uberwave`) on `PATH`;
3. a build next to the project or next to the CDePT repository, as a convenience for
   developers: `../uberwave/build/wp5-viewer/UberWaveViewer`, or in another `build*` folder.

`cdept tools` shows the result in the UberWave row. The viewer has no `--version` and
starting it opens a window, so the version column says `found` and the date of the file.

## Open a run point

From the GUI: run a testbench in the Simulation tab, and press **Open in UberWave** on the
point you want, in the Results list. A problem (no viewer found, no raw file) goes to the log
panel with how to fix it.

From the shell:

```bash
cdept sim run cells tb_inv --view testbench
cdept wave cells tb_inv --point tt_27           # the raw file of one run point
cdept wave cells tb_inv --list                  # the points and their raw files
cdept wave simulations/cells/tb_inv/testbench/tt_27/out.raw      # any raw file
cdept wave out.raw --analysis ac                # preselect an analysis
cdept wave --list out.raw                       # analyses and signal counts, no viewer
```

- `--view` names the testbench view; it can be left out when only one view of the cell has
  results. `--point` can be left out when there is only one run point; with several, the error
  lists them.
- When a raw file has several analyses (an operating point, then a transient, for example),
  CDePT preselects the transient one with `--analysis tran`. `--analysis` takes `tran`, `ac`,
  `dc`, `noise` or `op`, or the analysis name as the simulator wrote it. For a Spectre PSF
  folder the name goes to UberWave unchanged.
- The viewer is started detached, with the run point's folder as its working folder, and
  keeps running after `cdept` exits.

## Sweeps and Monte Carlo

A sweep, a corner grid or a Monte Carlo run has one raw file per point. UberWave opens one
file at a time from the command line; further files are added inside it with
**File > Import**. `cdept wave <lib> <cell> --list` prints the raw file of every point, so
the paths can be pasted into the file dialog. Overlaying the points of a family from CDePT
needs a multi-file command line or a connection to the running viewer, which UberWave does
not have yet (see Phase 5 of the [roadmap](../ROADMAP.md)).

## See also

- [Run a testbench](run-testbench.md)
- [Calculator](../CALC.md) for the numbers: `cdept calc`, `[[output]]`
- [Tools](../tools.md)
