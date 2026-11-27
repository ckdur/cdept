# View waveforms

```{note}
This is changing (2026-11-27): CDePT is getting its own viewer tabs on UberWave's data engine
(see the roadmap, Phase 5). UberWave's own viewer, which this page describes, stays as an
option that opens in a separate window.
```

CDePT has no finished waveform viewer of its own yet. The viewer in use is **UberWave**, Symbiotic EDA's program
(C++ with Dear ImGui and ImPlot, built for very large ngspice raw files and Spectre PSF
results). It is **part of the `cdept` executable**: its window runs on a thread of the cdept
process, next to the main window, and CDePT talks to it with plain function calls with typed
values (no helper process, no socket, no JSON, no files in between: the calculator hands over
its `f64` arrays as they are, lists come back as C structs, cursor moves arrive through a
callback).
When UberWave was not built into your `cdept`, CDePT falls back to starting `UberWaveViewer`
as a separate process (see [the fallback](#the-fallback-uberwaveviewer-as-a-process)).

What stays in CDePT are the numbers: measurements, the [calculator](../CALC.md)
(`cdept calc`, `[[output]]` in `setup.toml`) and the spec table of the Simulation tab. UberWave
is for looking at the curves.

![A family of three corners and the calculator trace v(out) / 2, in the UberWave window of cdept](../_static/screenshots/uberwave-embedded.png)

*Made with* `cdept wave cells tb_inv --view testbench --family --calc 'v("out") * 0.5' --name 'half out' --screenshot out.png`.

## Which UberWave you have

`cdept tools` shows it in the UberWave row:

| row says | meaning |
| --- | --- |
| `built in (embedded)  UberWaveViewer 0.1.0 ipc 1` | UberWave is in this executable; `cdept wave` opens its window in this process |
| `process: <version>  (<path>)` | not built in; the separate program `UberWaveViewer` was found (the fallback) |
| `not available` | neither: build UberWave in, or build the program, as below |

The backend is chosen in this order, the first that is set wins:

1. `cdept wave --backend embedded|process`;
2. the environment variable `CDEPT_UBERWAVE_BACKEND` (`embedded` or `process`);
3. `[wave] backend = "embedded" | "process"` in `cdept.toml`;
4. the default: `embedded` when it is built in, else `process`.

Asking for `embedded` in a cdept that does not have it is an error that says how to build it in
(it does not silently use the process).

## Build UberWave into cdept (Linux)

UberWave lives in its own repository. `cdept-uberwave`, a crate of this workspace, builds
its library `uberwave_embed` with CMake and links it into `cdept`. CDePT has it as the
submodule `cdept-uberwave/uberwave`: branch `main` of our public copy
`gitlab.symbioticeda.com/ckdur.iso/uberwave` (the changes CDePT needs are not in the
original repository). It is offered under MIT or GPL-2.0, at your option; third-party parts
keep their own licenses (see its `LICENSE`).

```bash
sudo apt install build-essential cmake libgl1-mesa-dev libgtk-3-dev   # compiler, CMake, OpenGL and GTK 3 development files
git submodule update --init cdept-uberwave/uberwave                   # done already by clone --recurse-submodules
cargo build                              # or: cargo install --path cdept
cargo run -- tools | grep UberWave       # built in (embedded)
```

No account is needed (HTTPS). The submodule is fetched shallow; the download is still large
because of data files in the repository. After a `git pull` that moves the submodule, run the
`git submodule update` line again. To push to the copy, set an SSH push address:
`git -C cdept-uberwave/uberwave remote set-url --push origin ssh://git@gitlab.symbioticeda.com:20022/ckdur.iso/uberwave.git`.

- To work on UberWave, work in the submodule: `git -C cdept-uberwave/uberwave switch main`
  (`git submodule update` leaves it on a commit, not on a branch), commit, push to `origin`
  (our copy), then commit the new submodule pointer in CDePT. The copy publishes no GitLab
  Pages: its pipeline file is renamed to `.gitlab-ci.yml.disabled` (merging from the original
  may bring `.gitlab-ci.yml` back). UberWave commits made with AI carry an `AI:` footer (its
  README, "Use of Generative AI"). The original repository can be added as a fetch-only remote
  `upstream`.
- `UBERWAVE_SRC` names another checkout (the repository root; the `wp5-viewer` folder is
  accepted too) and wins over the submodule. Without both, the build also looks for a sibling
  `../uberwave` of the CDePT repository. A source is used only when it has
  `wp5-viewer/shim/uberwave_shim.h`.
- The first build fetches GLFW, Dear ImGui, ImPlot, NFD-Extended and nlohmann/json with CMake
  (it needs the network and `git`) and compiles them under `target/`; it takes about a minute.
  Later builds only redo what changed in the UberWave source.
- No network, or the libraries are fetched already: `UBERWAVE_OFFLINE=1` stops CMake from
  fetching and `UBERWAVE_DEPS_DIR` names a FetchContent folder that has the sources (the
  `_deps` of an earlier build). `UBERWAVE_LIB_DIR=<cmake build dir>` links an existing CMake
  build of the source (one that built the target `uberwave_embed_probe`) and builds nothing.
- Linux only, X11 or Wayland. UberWave's own `Docs/Embedding.md` describes the C API and the
  threading.

If anything is missing (no source, no `cmake`, no OpenGL or GTK 3 files, no network) the build does
**not** fail: it prints a `cargo:warning` that says why and builds a stub; `cdept` then uses the
process fallback. That is also what a build without the source gives, for example the public CI.

### How the window lives next to cdept

The viewer thread owns its own window, its own GLFW and display connection and its own OpenGL
context; cdept's window (egui on winit) keeps running meanwhile. Checked with the GUI on Wayland
and on X11, with the viewer on either backend. Without `libdecor` a Wayland viewer window has
no title bar, which is cosmetic. To force a backend for the viewer window set
`UBERWAVE_GLFW_PLATFORM=x11` (or `wayland`).

- Only one viewer window exists per process. The GUI reuses it for every open of every project
  run (the files are replaced by the ones asked for); after you close it, the next open starts a
  new one.
- `cdept wave` opens the window and **waits until you close it**, then exits (the viewer's thread
  belongs to the process). The GUI never waits: opens run in a background thread.
- Closing the viewer window does not close cdept.
- `cdept wave ... --screenshot out.png` loads everything into a hidden 1280x720 window, saves it
  and exits (no window appears): for documentation and checks.

## Open a run point

From the GUI: run a testbench in the Simulation tab, and press **Open in UberWave** on the
point you want, in the Results list. A problem (no viewer, no raw file) goes to the log panel with
how to fix it.

From the shell:

```bash
cdept sim run cells tb_inv --view testbench
cdept wave cells tb_inv --point tt_27           # the raw file of one run point (the window of the run is reused)
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
- With the embedded UberWave `cdept wave` stays until the window is closed. The process fallback
  starts the viewer detached, with the run point's folder as its working folder, and it keeps
  running after `cdept` exits.
- Several points need `--point`, `--points` or `--family`; the error lists them.

## Sweeps, corners and Monte Carlo: families

A sweep, a corner grid or a Monte Carlo run has one raw file per point. They open together in one
window (the embedded UberWave, or a process viewer that has IPC), each file tagged with its point
name (`tt_27`, `mc_27_r003`):

```bash
cdept wave cells tb_inv --view testbench --family           # every point of the view
cdept wave cells tb_inv --view testbench --points tt_27,ss_27
```

In the GUI, **Open family in UberWave** in the Results area of the Simulation tab does the
same. Opening again (a point, a family, the button) reuses the open window and replaces its
session by the files asked for.

### What is plotted when it opens

A file that is opened this way plots nothing until it is told to, so after opening CDePT shows
signals for each file (tag = point name), all in one pane. By default the signals are:

1. the nodes the testbench `save`s in its `setup.toml`, if the raw file has them;
2. else the signals its `measures` and `[[output]]` expressions use;
3. else the first four voltages that are not the time or frequency axis, ground (`0`, `gnd`)
   or a supply (`vdd`, `vss`, ...).

```bash
cdept wave cells tb_inv --view testbench --family --show out,in   # these signals of every point
cdept wave cells tb_inv --view testbench --family --no-show       # open, plot nothing
```

`--show` takes a comma separated list (`v(out)` or just `out`, `i(vdd)`; no `tag:`, CDePT adds
the point name). A name a file does not have is a note. A freshly started viewer is waited for
while it loads the files (up to a minute); **Open family in UberWave** and **Open in UberWave**
in the GUI use the default and do not block the window.

![A family of three corners in UberWave](../_static/screenshots/uberwave-family.png)

## Calculator traces

The [calculator](../CALC.md) can feed the viewer: an expression is evaluated by CDePT on the raw
file of a point and the waveform it gives is sent as a new trace, next to the signals of the
files.

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
- The viewer is started on the chosen points when none is open yet; a running one is left as it
  is.
- `--analysis` limits the expression to one analysis of the raw file, as `cdept calc --plot`.
- Names are written in quotes, `v("out")`. `v(out)` is accepted here too (tried when the
  expression as written fails); when an expression still fails the error says
  `hint: write node names in quotes: v("out")` (see [the calculator](../CALC.md)).
- A viewer that is started for the push plots the default signals first (`--show`,
  `--no-show` as above); a running one is left as it is.
- The embedded UberWave gets the points as they are, however many (no thinning). The process
  fallback sends a trace with more than 200000 points as a min/max envelope of at most half of
  them (first and last point and the extrema are kept), named `<name> (decimated)`, with a note;
  `--no-decimate` sends it whole and `[wave] max_points` in `cdept.toml` changes the limit (`0`:
  never). A socket carries the data as JSON: the viewer parses 10 million points in about ten
  seconds, with the window stopped meanwhile.

![The calculator trace vout pushed next to the three corners](../_static/screenshots/uberwave-calc.png)

In the GUI, the **Calculator** section of the Simulation tab does this and more: a function
browser with help, a signal browser, a results table, Bode and eye layouts, markers where
scalars were measured, "Add as output", and the viewer's cursors followed live (see
[the calculator](../CALC.md#6-the-calculator-panel-gui-and-the-viewer)).

### Bode plots, eye diagrams, markers

```bash
cdept wave cells ota5t --view ac_open --points tt_27,ss_125 --calc 'v("out")/v("in")' --name gain --bode
cdept wave cells ota5t --view tran_step --point tt_27 --calc 'v("out")' --eye 20n,0
```

- `--bode`: a complex (AC) result as magnitude (dB) in one new pane and phase (degrees) in
  another, both on a log frequency axis; several points overlay in the same two panes.
- `--eye PERIOD[,OFFSET]`: the waveform folded at PERIOD from OFFSET, every period drawn on
  top of the others, in a new pane with its own X axis (0 to PERIOD). UberWave folds it
  (`fold_period` of a trace).
- A trace whose X unit the active pane cannot show (a spectrum next to a transient) goes to a
  pane of that unit, or a new one.
- Markers: a scalar result marks where it was measured (a crossing, the trigger and target of
  a delay, the edge points of a rise time, an extreme, the settling instant) as a dashed line
  or a point with a label, only in panes of the same X unit; each send replaces them;
  `--no-markers` leaves them out. They need the built-in UberWave and start no viewer by
  themselves.

![A crossing marked in UberWave, the code spectrum in its own pane](../_static/screenshots/sar-markers.png)

## Tables and digital waveforms

UberWave reads SPICE raw files and Spectre PSF, not CSV or VCD. CDePT reads them
([the calculator](../CALC.md#csv-tables-and-digital-waveforms)) and:

```bash
cdept wave scope.csv                        # every column (up to 16) to UberWave as traces
cdept wave scope.csv --show vout,iload      # these columns
cdept wave dump.vcd                         # opens in Surfer, else GTKWave ($CDEPT_DIGITAL_VIEWER first)
cdept wave dump.fst --show clk,cnt          # these signals to UberWave as step traces
cdept wave --list dump.vcd                  # its signal count
```

A table's columns and named digital signals go to the built-in UberWave as traces in a new
pane (nothing is written anywhere: CDePT hands the arrays over); without the built-in viewer
this is refused (the UberWaveViewer process cannot take them without a file). A digital file
without `--show` opens in a digital viewer, a separate program (GTKWave is GPL: started, never
linked). Digital signals are also available to the calculator (`frequency(v("clk"),
level=0.5)`).

## The fallback: UberWaveViewer as a process

Without the built-in UberWave CDePT starts the separate program `UberWaveViewer`. Build it from
the `uberwave` repository: `cmake -B build -DCMAKE_BUILD_TYPE=Release && cmake --build build
--target UberWaveViewer`; the program is `build/wp5-viewer/UberWaveViewer`. The process is also
used when you ask for it (`--backend process`, `CDEPT_UBERWAVE_BACKEND=process`, `[wave] backend =
"process"`).

CDePT finds the program, in this order:

1. `$CDEPT_UBERWAVE`, the path of the program;
2. `UberWaveViewer` (or `uberwave`) on `PATH`;
3. a build next to the project or next to the CDePT repository, as a convenience for
   developers: `../uberwave/build/wp5-viewer/UberWaveViewer`, or in another `build*` folder.

The version shown by `cdept tools` is the first line of `UberWaveViewer --version` (for example
`UberWaveViewer 0.1.0 ipc 1`); `ipc 1` means the viewer has the control channel. Families, the
reuse of a running viewer and the calculator traces need it (the `main` branch of our mirror of the
`uberwave` repository, until it is merged). With an older viewer CDePT keeps working as before: it
opens the first file, says so and lists the other files (**File > Import** adds them);
calculator traces cannot be sent to it, scalar results are still printed.

What differs from the built-in viewer:

- The viewer is a process of its own, started detached; `cdept wave` returns at once and the
  window stays after `cdept` exits.
- CDePT starts `UberWaveViewer --ipc <socket> --tag ... FILE...` with the socket named after the
  project and the run (`$XDG_RUNTIME_DIR/cdept-uw-<hash>.sock`, else the temporary folder).
  Opening the same run again finds that viewer and sends `open` and `import` to it; another run,
  or another project, gets its own viewer. A socket left by a viewer that died is replaced.
- Calculator traces travel as JSON and are thinned above `[wave] max_points` (see above).
- `--screenshot` is not available (it needs the embedded UberWave).

The protocol (UberWave's own, version 1) is one JSON object per line over the Unix socket.
CDePT implements the client in `libcdept::wave::ipc` (`hello`, `open`, `import`,
`list_files`, `list_signals`, `show`, `add_trace`, `get_cursors`, `set_cursors`, `quit`, ...);
`cdept-gui/scripts/fake-uberwave-ipc.py` is a stand-in server for tests. Both backends go
through one interface (`libcdept::wave::backend::ViewerOps`), so `cdept wave` and the GUI do not
care which one runs.

## See also

- [Run a testbench](run-testbench.md)
- [Calculator](../CALC.md) for the numbers: `cdept calc`, `[[output]]`
- [Tools](../tools.md)
