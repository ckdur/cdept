# View waveforms

CDePT has no waveform viewer of its own. The viewer is **UberWave**, a separate program
of Symbiotic EDA (C++ with Dear ImGui and ImPlot, built for very large ngspice raw files
and Spectre PSF results). CDePT starts it on the raw file of a run point, as a separate
process. UberWave belongs to the same owner as CDePT, so its licence does not limit how it
is integrated; embedding it in the CDePT window is possible later if starting it as a
program proves too limited (`docs/ROADMAP.md`, Phase 5).

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

`cdept tools` shows the result in the UberWave row. The version column is the first line of
`UberWaveViewer --version` (for example `UberWaveViewer 0.1.0 ipc 1`); `ipc 1` means the viewer
has the control channel described below. CDePT asks without a display and gives the viewer 2
seconds; an older viewer that does not know the flag does not answer in time, and the column
says `found, file of <date>`.

### A viewer with IPC

Families, the reuse of a running viewer and the calculator traces need a viewer that prints
`ipc 1` in `--version`. It comes from the branch `ipc-multifile` of the `uberwave` repository
(build as above after `git checkout ipc-multifile`; once it is merged, any current build
has it). With an older viewer CDePT keeps working as before: it opens the first file, says
so, and lists the other files (**File > Import** adds them); calculator traces cannot be sent
to it, scalar results are still printed.

## Open a run point

From the GUI: run a testbench in the Simulation tab, and press **Open in UberWave** on the
point you want, in the Results list. A problem (no viewer found, no raw file) goes to the log
panel with how to fix it.

From the shell:

```bash
cdept sim run cells tb_inv --view testbench
cdept wave cells tb_inv --point tt_27           # the raw file of one run point (the viewer of the run is reused)
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
- Several points need `--point`, `--points` or `--family`; the error lists them.

## Sweeps, corners and Monte Carlo: families

A sweep, a corner grid or a Monte Carlo run has one raw file per point. With a viewer that has
IPC they open together in one window, each file tagged with its point name (`tt_27`,
`mc_27_r003`):

```bash
cdept wave cells tb_inv --view testbench --family           # every point of the view
cdept wave cells tb_inv --view testbench --points tt_27,ss_27
```

In the GUI, **Open family in UberWave** in the Results area of the Simulation tab does the
same. CDePT starts `UberWaveViewer --ipc <socket> --tag ... FILE...` with the socket named after
the project and the run (`$XDG_RUNTIME_DIR/cdept-uw-<hash>.sock`, else the temporary folder).
Opening the same run again (a point, a family, the button) finds that viewer and sends `open`
and `import` to it instead of starting another window; the session is replaced by the files
asked for. Another run, or another project, gets its own viewer. A socket left by a viewer that
died is replaced.

## Calculator traces

The [calculator](../CALC.md) can feed the viewer: an expression is evaluated by CDePT on the raw
file of a point and the waveform it gives is sent as a new trace (`add_trace`), next to the
signals of the files.

```bash
cdept wave cells tb_inv --view testbench --point tt_27 --calc 'db20(v("out"))' --name out_db
cdept wave cells tb_inv --view testbench --family --calc 'v("out") / v("in")'
cdept wave cells tb_inv --view testbench --family --calc 'max(v("out"))'   # scalars: a table
```

- With one point and `--name`, the trace is called as given; otherwise `<name or expression> @
  <point>`, one trace per point. Sending the same name again replaces the trace.
- A complex waveform (an AC result) is sent as two traces, `<name> (mag dB)` (20 log10 of the
  magnitude) and `<name> (phase)` (degrees, unwrapped).
- Points whose value is not finite (a `db20` of zero) are left out of a trace.
- A scalar result is printed, one line per point (`point<TAB>value`), and nothing is sent. A
  calculator error is reported per point with its position in the expression.
- The viewer is started on the chosen points when none of the run is open yet; a running one is
  left as it is.
- `--analysis` limits the expression to one analysis of the raw file, as `cdept calc --plot`.

In the GUI, the Results area of the Simulation tab has an expression line with a choice of
points (all, or one) and **Send**. Scalars and errors show under the line; typing the start of
a function name offers matching names as buttons. The evaluation runs in the background.

The protocol (UberWave's own, version 1) is one JSON object per line over the Unix socket.
CDePT implements the client in `libcdept::wave::ipc` (`hello`, `open`, `import`,
`list_files`, `list_signals`, `show`, `add_trace`, `get_cursors`, `set_cursors`, `quit`, ...);
`cdept-gui/scripts/fake-uberwave-ipc.py` is a stand-in server for tests.

## See also

- [Run a testbench](run-testbench.md)
- [Calculator](../CALC.md) for the numbers: `cdept calc`, `[[output]]`
- [Tools](../tools.md)
