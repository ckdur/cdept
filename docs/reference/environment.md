# Environment variables

This page lists every environment variable CDePT reads, and the ones it sets for the
tools it starts. Variables that name a program take a path to the executable; when one is
set to a non-empty value, it wins over any search. When it is unset, CDePT searches as
described in [External tools](../tools.md).

## Where tools are found

| Variable | Meaning | Default | Used by |
|---|---|---|---|
| `CDEPT_XSCHEM` | The `cdept-xschem` helper program (the embedded xschem engine). | `cdept-xschem` next to the `cdept` executable, else on `PATH` | schematics, netlisting, hierarchy, GUI schematic tabs |
| `CDEPT_XSCHEM_SHAREDIR` | Folder with xschem's run-time files (`xschem.tcl`, `xschem_library/devices`), read by the helper. | `<prefix>/share/cdept-xschem` of the installed helper, else the build tree | `cdept-xschem` |
| `CDEPT_KLAYOUT` | The `klayout` program. | `cdept-klayout/.bin/klayout` of a development tree, then `klayout` next to `cdept`, then `PATH` | embedded KLayout, layout checks, technology scan |
| `CDEPT_KLAYOUT_HELPER` | Folder that holds the `cdept_klayout/` Python helper run by KLayout. | `<prefix>/share/cdept-klayout`, else the source tree | embedded KLayout |
| `CDEPT_KLAYOUT_HOME` | KLayout home folder given to the helper (`KLAYOUT_HOME`), so the user's `~/.klayout` does not run in it. | `$XDG_CACHE_HOME/cdept/klayout-home`, else `~/.cache/cdept/klayout-home` | embedded KLayout |
| `CDEPT_NGSPICE` | The `ngspice` program for testbench runs. | `ngspice` on `PATH` | `cdept sim`, GUI Simulation tab |
| `CDEPT_LIBNGSPICE` | The `libngspice` shared library loaded in-process. When set it is the only candidate. | `libngspice.so.0`, `libngspice.so`, then `../lib` of the `ngspice` on `PATH` | `cdept ngspice`, in-process runs |
| `CDEPT_XYCE` | The Xyce program. | `Xyce` or `xyce` on `PATH` | `simulator = "xyce"` |
| `CDEPT_VACASK` | The VACASK program. | `vacask` on `PATH` | `simulator = "vacask"` |
| `CDEPT_VACASK_PYTHON` | VACASK's Python folder (the one holding `ng2vclib`), used to convert ngspice models. | `<prefix>/lib/vacask/python` of the VACASK program, else what `vacask -dp` prints | `cdept vacask models` |
| `CDEPT_OPENVAF` | The OpenVAF compiler. | `openvaf-r` or `openvaf` on `PATH` | `cdept openvaf`, VACASK models |
| `CDEPT_MAGIC` | The Magic program. | `magic` on `PATH` | `cdept magic`, technology scan |
| `CDEPT_NETGEN` | The netgen program. | `netgen-lvs` or `netgen` on `PATH` | `cdept netgen` |
| `CDEPT_YOSYS` | The Yosys program. | `yosys` on `PATH` | `cdept yosys` |
| `CDEPT_OPENROAD` | The OpenROAD program. | `openroad` on `PATH` | `cdept openroad` |
| `CDEPT_QUCS` | The Qucs-S program. | `qucs-s` on `PATH` | `cdept qucs` |
| `CDEPT_QUCS_SPAR` | The Qucs-S S-parameter viewer. | `qucs-sspar-viewer` on `PATH` | `cdept qucs spar` |
| `CDEPT_UBERWAVE` | The UberWave waveform viewer (`UberWaveViewer`). | `UberWaveViewer` or `uberwave` on `PATH`, then `../uberwave/build*/wp5-viewer/UberWaveViewer` next to the project or the CDePT repository | `cdept wave`, Open in UberWave |
| `CDEPT_PYTHON` | The Python interpreter used to detect and run Python tools. | `python3`, else `python`, on `PATH` | `cdept tools`, KLayout Python probe |
| `KLAYOUT_HOME`, `KLAYOUT_PATH` | KLayout's own folders; read to find KLayout salt packages (`klive`, ...). `KLAYOUT_PATH` is also extended with the PDK's `libs.tech/klayout` for the helper. | KLayout's usual places | `cdept tools`, embedded KLayout |
| `PATH` | Searched for every program that has no override. | | everywhere |

## Technology and PDK

| Variable | Meaning | Default | Used by |
|---|---|---|---|
| `PDK_ROOT` | The folder PDKs are installed in. A `pdk_root` in the project (`cdept.local.toml` or `cdept.toml`) replaces it. | unset | project, technology scan, every tool started |
| `PDK` | The PDK name. Several names separated by colons mean "available PDKs"; the project's `technology` chooses one. | unset | project, technology scan |
| `PDKPATH` | Not read as a setting. Set for every started tool to `$PDK_ROOT/$PDK` when both are known. | set by CDePT | tools started by CDePT |
| `CDEPT_PDK_PATH` | More folders that hold PDKs, separated like `PATH`. | unset | `cdept pdk ls`, PDK discovery, New Project wizard |
| `CDEPT_PDK_DATA` | Folder with CDePT's shipped device maps and templates: `<pdk>/devices.toml`. | `<prefix>/share/cdept/pdk` next to the executable, else the `pdk/` folder of the source tree | schematic-to-layout linking, test templates |
| `CDEPT_PROJECT` | Not read from the process environment. `$CDEPT_PROJECT` can be written in paths of `tech.toml`; it expands to the project folder. | | `tech.toml` paths |
| `CDEPT_TECH_SCAN_TIMEOUT` | Seconds KLayout gets to list the PDK's PCell libraries during a scan. | 60 | `cdept tech scan`, `cdept init --technology` |

PDK discovery also looks in `~/.ciel`, `~/.volare`, `/usr/share/pdk`, `/usr/local/share/pdk`
and `~/share/pdk` (see [`cdept pdk ls`](cli.md)). Any variable named in a path of
`cdept.toml` or `tech.toml` (`$NAME`, `${NAME}`) is read from `[env]` of
[`cdept.local.toml`](files/cdept-local-toml.md) first, then from the process.

## Simulation

| Variable | Meaning | Default | Used by |
|---|---|---|---|
| `CDEPT_XYCE_ARGS` | Extra Xyce command-line arguments, one per line (model plugins, the Monte Carlo seed). CDePT sets it for the run it starts; setting it outside has no lasting effect. | set per run | Xyce runs |

For Xyce runs CDePT also sets `OMPI_MCA_plm=isolated`, `OMPI_MCA_btl=self` and
`PMIX_MCA_gds=hash` unless they are already set ([BATCH.md](../BATCH.md)).

## Files, state and behaviour

| Variable | Meaning | Default | Used by |
|---|---|---|---|
| `CDEPT_INDEX_CACHE` | Folder of the hierarchy index cache. `off` or `0` turns the cache off. | `$XDG_CACHE_HOME/cdept/index`, else `~/.cache/cdept/index` | `cdept hier`, `used-by`, `find`, GUI |
| `CDEPT_NATIVE_DIALOG` | `off`, `0` or `no`: never use the system file dialog; CDePT's own path prompt is used instead. | on | GUI |
| `RUST_LOG` | Log filter in the `tracing` syntax (`RUST_LOG=cdept=debug`). When set it replaces the level chosen with `-v`. | `info`; `-v` gives debug, `-vv` trace | every program |
| `XDG_DATA_HOME` | Where the GUI keeps its window layout and recent projects, and where the trash lives (`Trash/`). | `~/.local/share` | GUI, delete to trash |
| `XDG_CACHE_HOME` | Base of CDePT's caches (index, LibMan project files, KLayout home). | `~/.cache` | cache users |
| `HOME` | Base for the defaults above and for `~/.ciel`, `~/.volare`. | | everywhere |
| `DISPLAY`, `WAYLAND_DISPLAY` | Needed by the GUI. The headless commands do not need a display. | | GUI |

## Set by CDePT for the tools it starts

| Variable | Value |
|---|---|
| `PDK_ROOT`, `PDK`, `PDKPATH` | The project's PDK, resolved to one name. |
| `[env]` entries of `cdept.local.toml` | Added to the environment of every tool. |
| `KLAYOUT_PATH` | The project's KLayout folders, ahead of an inherited value. |
| `CDEPT_KLAYOUT_TECH`, `CDEPT_KLAYOUT_LAYER_PROPERTIES` | The KLayout technology and layer properties file of `[klayout]`, for the helper. |
| `QT_QPA_PLATFORM=offscreen`, `KLAYOUT_HOME`, `LD_LIBRARY_PATH` | For the KLayout helper (`LD_LIBRARY_PATH` gets the folder of KLayout's libraries first). |

## Used by the test suites only

`CDEPT_TEST_PDK_ROOT`, `CDEPT_TEST_LOCAL_HOME`, `CDEPT_TEST_PRINT` and `CDEPT_EXAMPLE`
are read by tests and by `cdept-gui/scripts/run.sh`; see [TESTS.md](../TESTS.md) and
[GUI-TESTING.md](../GUI-TESTING.md).

## See also

- [External tools](../tools.md)
- [Installation](../guide/installation.md)
- [Technology](../concepts/technology.md)
- [cdept.local.toml](files/cdept-local-toml.md)
