# The command line

Everything that does not need a window is a `cdept` command. With no command, `cdept`
opens the [GUI](gui.md). The commands are the same operations the GUI performs, on the
same library, so they can be used in scripts and in continuous integration.

```console
$ cdept --help
Usage: cdept [OPTIONS] [COMMAND]
...
Options:
  -p, --project <PROJECT>  Project folder (the one holding cdept.toml). Defaults to the project the current folder belongs to
  -v, --verbose...         More log output (-v debug, -vv trace). `RUST_LOG` overrides this
  -h, --help               Print help
  -V, --version            Print version
```

Every command and subcommand has `--help`. The complete text of all of them is in the
[Command reference](../reference/cli.md).

## Finding the project

Most commands work on a project. `cdept` finds it like `cargo` finds a workspace: it takes
the closest folder at or above the current one that holds a `cdept.toml`. `-p <folder>`
names the project folder explicitly and works with any command, before or after the
command name:

```console
$ cdept -p ~/chips/sar-adc ls
$ cd ~/chips/sar-adc/libs && cdept ls          # same project
$ cd /tmp && cdept ls
Error: not inside a project (no cdept.toml); try `cdept init`
```

`init`, `pdk`, `tools`, `version`, `calc` on a raw file, and `tech scan --print` need no
project. A project that cannot be fully read still opens: a library whose folder is missing
is reported as a warning and the rest works.

The PDK a command uses is the project's `technology` with the folder `$PDK_ROOT`; see
[Technology](../concepts/technology.md) and `cdept info`. The tools are found as described
in [External tools](../tools.md).

## Cells are named `<library> <cell> [--view <view>]`

Commands that act on a cell take the library and the cell as two arguments
(`cdept netlist cells inv`). When a cell has several views of the kind the command needs,
`--view` chooses one; without it the command stops and lists them:

```console
$ cdept sim run cells tb_inv
Error: cells/tb_inv has several testbenches (both, dc_transfer, ...); pick one with --view
```

## Output, logging and exit status

- Results go to standard output; log lines (warnings, `-v` debug output) go to standard
  error, so a pipe or a redirect gets the results only.
- `-v` shows debug messages and `-vv` trace messages, including the commands sent to the
  helpers. `RUST_LOG` takes over when it is set.
- A command that works exits with status 0. An error exits with status 1 and a message
  starting with `Error:`. A mistake in the command line exits with status 2.
- Checks exit with status 1 when they find a problem: `layout drc` and `layout lvs`
  when not clean, `layout check-source` and `layout check-nets` when out of sync,
  `netgen lvs` when the circuits differ, `sim run` and `sim suite` when a run fails or a
  spec is not met. That makes them usable as steps of a script.
- `--json` is available where the output is data: `find`, `hier`, `used-by`, `status`,
  `pdk ls|info|versions`, `tech show`, `sim suite` and `layout pins|check-nets`.

## Command groups

| Group | Commands | What for |
|---|---|---|
| Project | `init`, `info`, `import`, `lib`, `tools`, `version` | Create and describe a project; define libraries; list tools and version. |
| Cells and views | `ls`, `cell`, `view`, `category`, `find`, `hier`, `used-by`, `status` | Browse and change what is in the libraries. Renames and moves rewrite references. |
| Technology | `tech`, `pdk` | Scan a PDK into `tech.toml`; list installed PDKs and versions. |
| Schematics | `netlist`, `xschem`, `qucs` | Netlists in several formats, pictures, symbols and instances; Qucs-S conversion. |
| Layout | `layout`, `klayout` | References between views, export, pictures, DRC, LVS, linking with schematics. |
| Simulation | `sim`, `calc`, `ngspice`, `vacask`, `openvaf` | Testbenches, test templates and suites; the calculator on raw files; simulator support. |
| Verification and flow | `magic`, `netgen`, `yosys`, `openroad` | Extraction, DRC and LEF with Magic; LVS; synthesis; OpenROAD scripts. |
| Interop | `libman` | Export the libraries to a LibMan project and open it. |
| Interface | `gui` | Open the window (the default). `--script` runs a GUI test script ([GUI-TESTING.md](../GUI-TESTING.md)). |

The commands, with their subcommands:

| Command | Subcommands | Reference |
|---|---|---|
| `cdept init` | | [init](../reference/cli.md#cdept-init) |
| `cdept lib` | `add`, `remove`, `rename`, `set`, `show` | [lib](../reference/cli.md#cdept-lib) |
| `cdept cell` | `new`, `copy`, `rename`, `move`, `rm` | [cell](../reference/cli.md#cdept-cell) |
| `cdept view` | `new`, `copy`, `rename`, `rm` | [view](../reference/cli.md#cdept-view) |
| `cdept category` | `ls`, `add`, `rm`, `rename`, `of`, `suggest`, `adopt` | [category](../reference/cli.md#cdept-category) |
| `cdept tech` | `scan`, `show`, `refresh`, `notes`, `devices` | [tech](../reference/cli.md#cdept-tech) |
| `cdept pdk` | `ls`, `info`, `versions`, `enable` | [pdk](../reference/cli.md#cdept-pdk) |
| `cdept xschem` | `svg`, `instances`, `hierarchy`, `make-symbol`, `make-schematic`, `link`, `tcl`, `version` | [xschem](../reference/cli.md#cdept-xschem) |
| `cdept layout` | `refs`, `export`, `png`, `drc`, `lvs`, `link`, `check-source`, `pins`, `check-nets`, `parallel` | [layout](../reference/cli.md#cdept-layout) |
| `cdept sim` | `run`, `results`, `templates`, `new`, `suite`, `deck` | [sim](../reference/cli.md#cdept-sim) |
| `cdept magic` | `extract`, `drc`, `lef` | [magic](../reference/cli.md#cdept-magic) |

Subcommands of the other groups are in the [Command reference](../reference/cli.md).

## Scripting

```sh
set -e
cdept -p "$PROJ" netlist cells inv -o build/inv.spice
cdept -p "$PROJ" layout drc cells inv          # exit 1 unless clean
cdept -p "$PROJ" sim suite cells tb_inv --json > results.json
```

Working in a copy of a project is safe: a project is a folder. Generated files
(`simulations/`, caches) can be deleted at any time; what to commit is described in
[Project folders](../reference/files/project-folders.md).

## See also

- [Command reference](../reference/cli.md)
- [Environment variables](../reference/environment.md)
- [The graphical interface](gui.md)
- [Quickstart](quickstart.md)
