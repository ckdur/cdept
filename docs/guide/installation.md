# Installation

CDePT is built from source with Rust. It needs a PDK and the external tools it drives, but
none of them is needed to build it.

## Build

You need a Rust toolchain of version 1.95 or newer (edition 2024) and a C linker. Then:

```sh
git clone https://github.com/ckdur/cdept
cd cdept
cargo build --release -p cdept
```

This builds `target/release/cdept`, the program with the GUI and every command. It runs
as it is. It does not need xschem or KLayout to build; the tools are looked up when a
command needs them.

To run the window under X11, the system also needs `libxkbcommon-x11` (Debian and Ubuntu:
`libxkbcommon-x11-0`; desktops usually have it, minimal installs and containers may not).
CDePT stops at start-up with "Library libxkbcommon-x11.so could not be loaded" without it.

The workspace has four crates:

| Crate | Content | Licence |
|---|---|---|
| `libcdept` | Core library: project, technology, simulation, tool adapters | MIT |
| `cdept` | The `cdept` program (command line, and the GUI by default) | MIT |
| `cdept-gui` | The window | MIT |
| `cdept-xschem` | The xschem helper process | GPL-2.0-or-later |

`cdept-xschem` is not a default member of the workspace, so a plain `cargo build` does not
build it.

### The xschem helper

Schematics open in CDePT through `cdept-xschem`, which contains xschem built from a git
submodule. Build it with:

```sh
git submodule update --init cdept-xschem/xschem
cargo build --release -p cdept-xschem
```

It needs a C compiler, flex, bison and the development files of Tcl and cairo. On
Debian or Ubuntu:

```sh
sudo apt install build-essential flex bison tcl-dev libcairo2-dev
# optional: pictures embedded in schematics
sudo apt install libjpeg-dev
```

Without the helper, everything that does not touch a schematic still works; commands
that need it say so. See [XSCHEM.md](../XSCHEM.md) for what the helper is.

### UberWave, the waveform viewer (optional, built in)

`cdept wave` and **Open in UberWave** use UberWave, Symbiotic EDA's waveform viewer. It can be
built **into the cdept executable**: check out its submodule (or point
`UBERWAVE_SRC` at another checkout of the `uberwave` repository) before `cargo build` (the crate `cdept-uberwave` runs CMake on it; it needs `cmake`, a C++20 compiler
and the OpenGL and GTK 3 development files, and the network once to fetch GLFW, Dear ImGui and
ImPlot).

```sh
sudo apt install build-essential cmake libgl1-mesa-dev libgtk-3-dev
git submodule update --init cdept-uberwave/uberwave
cargo build --release
cdept tools | grep UberWave        # built in (embedded)
```

Without the source (or when that build fails: the build prints a warning and goes on) the
executable has a stub instead and `cdept wave` starts the separate `UberWaveViewer` program, if
there is one. [View waveforms](../howto/view-waveforms.md) has the details, the offline variables
and the fallback.

### The KLayout helper

Layouts open through KLayout itself, which runs a Python helper that ships with CDePT
(`cdept-klayout/helper`). The helper needs no build. KLayout can be a system installation:
CDePT uses `$CDEPT_KLAYOUT`, then `cdept-klayout/.bin/klayout`, then `klayout` next to
`cdept`, then `klayout` on `PATH`. The embedded layout view needs a KLayout built with Qt
and Python (the usual distribution packages are). To build KLayout from the submodule
instead, see `cdept-klayout/README.md` and [KLAYOUT.md](../KLAYOUT.md).

### Installing

`scripts/install.sh` builds and installs CDePT, the xschem helper, the KLayout helper
script and the device maps of the PDKs:

```sh
scripts/install.sh --prefix /usr/local
scripts/install.sh --prefix /opt/cdept --no-helper    # cdept only
scripts/install.sh --destdir /tmp/stage               # for packagers
```

The installed tree is:

| Path under the prefix | Content |
|---|---|
| `bin/cdept` | The program. |
| `bin/cdept-xschem` | The helper. `cdept` looks for it next to its own executable. |
| `share/cdept-xschem/` | What the helper reads at run time (xschem's Tcl and awk files, symbol library). |
| `share/cdept-klayout/` | The KLayout helper. KLayout itself is not installed. |
| `share/cdept/pdk/` | Device maps of PDKs (schematic symbol to layout PCell). |
| `share/doc/cdept/` | Licences. |

`--no-build` installs what `cargo build --release` already made.

## External tools

CDePT finds a tool in this order, and stops at the first that exists:

1. The tool's environment variable (`CDEPT_NGSPICE`, `CDEPT_MAGIC`, ...). It names the
   executable and is used as given.
2. For the helpers, the program next to the `cdept` executable (`cdept-xschem`, `klayout`).
3. The `PATH`.

A tool installed in a folder that is not on `PATH` (for instance `/opt/ext/OpenLane/bin`)
is found by adding that folder to `PATH` or by setting the variable of the tool. The
complete list of variables is in [Environment variables](../reference/environment.md),
and what each tool is for is in [External tools](../tools.md).

Required to use CDePT for design:

| Tool | For |
|---|---|
| A PDK | Technology, libraries, models (see below) |
| `cdept-xschem` | Schematics, netlists |
| KLayout | Layouts, DRC, LVS, technology scan |
| ngspice | Simulation |

Optional, each enabling its own commands: Magic (extraction, DRC, LEF), netgen (LVS),
Xyce, VACASK and OpenVAF (other simulators), Qucs-S (second schematic tool), Yosys,
OpenROAD, ciel or volare (PDK versions), LibMan, git (status marks).
Container images such as the Symbiotic EDA `asic-all` image bring most of them.

Check what CDePT sees:

```console
$ cdept tools
xschem               Schematic        3  XSCHEM V3.4.6  (/opt/ext/OpenLane/bin/xschem)
KLayout              Layout           3  KLayout 0.30.10  (/opt/ext/OpenLane/bin/klayout)
Magic                Layout           3  8.3.623  (/opt/ext/OpenLane/bin/magic)
ngspice              Simulation       3  ngspice-47 : Circuit level simulation program  (...)
...
```

Each line is the tool, its role, the roadmap phase that integrates it, and the version and
path found, or `not found`. `cdept tools` lists many more tools than CDePT drives; only
the ones in [External tools](../tools.md) matter for a design. The `xschem` line is a stock
xschem on `PATH`; the embedded one is the helper, which the GUI shows in its Tools tab as
`cdept-xschem`.

## PDKs

A PDK is a folder with the tool set-ups (`libs.tech`) and the reference libraries
(`libs.ref`). CDePT works with open_pdks style trees: `ihp-sg13g2`, `sky130A`, `sky130B`,
`gf180mcu*` and `ihp-sg13cmos5l`.

Install a PDK with [ciel](https://github.com/fossi-foundation/ciel) (or its predecessor
volare), or clone the PDK's repository, so that there is a folder `$PDK_ROOT/<pdk>`:

```sh
ciel enable --pdk-family sky130 <version>
export PDK_ROOT=$HOME/.ciel
```

CDePT looks for PDKs in this order: the project's `pdk_root`, `$PDK_ROOT`, the folders
of `$CDEPT_PDK_PATH`, `~/.ciel`, `~/.volare`, `/usr/share/pdk`, `/usr/local/share/pdk`,
`~/share/pdk`.

```console
$ PDK_ROOT=/opt/ext/OpenPDKs cdept pdk ls
* ihp-sg13g2      unreleased (2026-09-01) (cb743f75aabc)  git   /opt/ext/OpenPDKs
  sky130A         1.0.530 (8afc8346a57f)                  ciel  /opt/ext/OpenPDKs
  sky130B         1.0.530 (8afc8346a57f)                  ciel  /opt/ext/OpenPDKs
(* the project's PDK)
```

`cdept pdk versions` and `cdept pdk enable` show and switch the versions that ciel
installed. The environment variables of a PDK are explained in
[Technology](../concepts/technology.md); a per-machine `pdk_root` goes in
[`cdept.local.toml`](../reference/files/cdept-local-toml.md).

## Check the installation

```console
$ cdept version
cdept 0.1.0 (70e9a42)
commit 70e9a42d655136f8fa04b16f7316dc8452d6b8de
```

`cdept version` prints the version, the short git commit the program was built from
(with a `*` after it for a build from a modified tree) and the full commit. `cdept --version`
prints the version number only. Then follow the
[Quickstart](quickstart.md).

## See also

- [Quickstart](quickstart.md)
- [External tools](../tools.md)
- [Environment variables](../reference/environment.md)
- [Technology](../concepts/technology.md)
