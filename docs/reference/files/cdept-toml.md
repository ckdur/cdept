# cdept.toml

`cdept.toml` is the project file. It sits in the project's root folder, names the project
and defines its libraries. A folder is a CDePT project when it holds this file; every
command and the GUI find the project by looking for `cdept.toml` in the current folder
and its parents (`-p/--project` names the folder explicitly).

The file is plain TOML, meant to be edited by hand and by CDePT. When CDePT changes it
(`cdept lib add`, Project Settings, the Library Manager) it edits the file in place and
keeps comments, ordering and the short or long form of each library definition. A write
goes through a temporary file, so a failure never truncates the project file, and a
symbolic link stays a link.

```toml
# examples/ihp-inv/cdept.toml
name = "ihp-inv"
technology = "ihp-sg13g2"
# pdk_root = "/opt/ext/OpenPDKs"

[libraries]
cells = "libs/cells"
```

```toml
# examples/buffer/cdept.toml
name = "buffer"

[libraries]
logic = "libs/logic"
bench = "libs/bench"
```

## Top-level keys

| Key | Type | Default | Meaning |
|---|---|---|---|
| `format` | integer | `1` | Format of this file. A file without the key is format 1. A higher number loads with a warning (parts may be ignored, and lost when CDePT changes the file). |
| `name` | string | the folder name | Project name, shown in the GUI and by `cdept info`. |
| `description` | string | none | One line about the project. |
| `technology` | string | none | The PDK name, for example `ihp-sg13g2`. It is `$PDK` in library paths and for every tool CDePT starts. See [Where the PDK comes from](#where-the-pdk-comes-from). |
| `pdk_root` | string | none | The folder PDKs are installed in (`$PDK_ROOT`). It differs per machine, so it normally belongs in [cdept.local.toml](cdept-local-toml.md); a value written here is still read. It may use `$VAR` and `~` but not `$PDK_ROOT`, and a relative path is relative to the project folder. |
| `include` | list of strings | none | More definition files, for example one shared by a team. See [Included files](#include). |
| `[libraries]` | table | empty | The library definitions. |
| `[suffixes]` | table | empty | What a file suffix means in this project. |
| `[xschem]` | table | none | Start-up file of the embedded xschem. |
| `[klayout]` | table | none | Settings of the embedded KLayout. |
| `[magic]` | table | none | Batch runs of Magic. |

An empty string for `name`, `description` or `technology` counts as not set. A value of
the wrong type for `name`, `description`, `technology` or `pdk_root` makes the project
fail to load.

:::{note}
Keys that CDePT does not know are ignored without a warning, in the top level and in
`[xschem]` and `[klayout]`. Only the local file and the library definitions report
unknown keys (see below). A typo in a key name therefore shows up as a setting that has
no effect, not as an error.
:::

## `[libraries]`

Each entry defines one library: a name and a folder.

```toml
[libraries]
# Short form: name = "folder"
analog = "libs/analog"

# Long form, a table with these keys:
sg13g2_pr = { path = "$PDK_ROOT/ihp-sg13g2/libs.tech/xschem/sg13g2_pr", read_only = true }
cells_flat = { path = "libs/old", layout = "flat" }
```

| Key | Type | Default | Meaning |
|---|---|---|---|
| `path` | string | required | The library folder. Relative paths are relative to the file that contains the definition. `$VAR`, `${VAR}` and a leading `~` are expanded when the project loads and stay as written in the file. |
| `layout` | `"cells"`, `"flat"` or `"ref"` | detected from the folder | How the folder maps to cells and views, see [Library layouts](../../concepts/projects.md#library-layouts). `ref` is for the reference libraries of a PDK (`<library>/<format>/<files>`); CDePT only reads such a library. |
| `read_only` | boolean | `false` | CDePT never changes the library: no new cells, no renames, no category or description edits. |

Rules for definitions:

- CDePT creates library names with letters, digits, `_` and `-`, starting with a letter,
  digit or `_`, and without `.` (the rules of cell names, see
  [Names and case](../../concepts/projects.md#names-and-case)). Loading accepts any
  TOML key.
- A library whose folder cannot be found, or whose path uses an unset variable, stays
  in the list with an error instead of failing the project.
- A table with an unknown key (anything other than `path`, `layout`, `read_only`), a
  missing `path`, or a `layout` that is not `cells`, `flat` or `ref` is a warning, and
  that library is left out.
- A name defined twice (also across included files) keeps the first definition; the
  later one is a warning.
- Two libraries must not have the same folder name (the last path component): xschem
  names a symbol by its library folder, so it cannot tell them apart and finds only the
  first. Loading warns about such a pair, and `cdept lib add` and the library dialogs
  check for it.
- `cdept lib add NAME PATH` always writes the long form with `layout`, because the
  layout detected for a folder can change once files are added to it. A definition
  edited by `cdept lib set` stays in the short form while only the folder is set.
- The description of a library is not in `cdept.toml`; it is in the library's
  [lib.toml](project-folders.md#lib-toml).

Libraries appear in this order: the project file first, then included files (depth
first), then the libraries the PDK brings through [tech.toml](tech-toml.md), which are
read-only.

## `include`

```toml
include = ["../shared/libraries.toml", "$TEAM/libs.toml"]
```

An included file has the same `include` and `[libraries]` keys and nothing else (its
other keys are ignored). Paths in it are relative to that file. A file that is missing,
or that includes a file already loaded (a cycle), is skipped; a missing file is a
warning. `cdept lib set` and `cdept lib remove` change a definition in the file that
holds it.

## `[suffixes]`

CDePT tells the format of a design file by its suffix (`.sch`, `.gds`, `.lib`, ...).
`[suffixes]` changes or extends that table for this project.

```toml
[suffixes]
".lib" = "spice"      # model libraries here, not Liberty
".net" = "spice"      # a suffix CDePT does not know
".v"   = "none"       # not a design file in this project
```

The key is a suffix: it starts with a dot and has no `/`. The value is one of:
`xschem-schematic`, `xschem-symbol`, `qucs-schematic`, `gds`, `oasis`, `magic`, `spice`,
`cdl`, `verilog`, `systemverilog`, `vhdl`, `verilog-a`, `lef`, `liberty`,
`klayout-pcell`, or `none` (the file is not a design file and is not shown). The
longest matching suffix wins; at equal length the project's entry wins. An entry that
cannot be understood is a warning. Only the table of the project file is read, not the
ones of included files.

## `[xschem]`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `rc` | string or list of strings | the file `tech.toml` names | Start-up file(s) (`xschemrc`, Tcl) the embedded xschem reads before the project's libraries are added to its search path. Typically the PDK's. A file that is not found is left out with a warning. |

```toml
[xschem]
rc = "$PDK_ROOT/ihp-sg13g2/libs.tech/xschem/xschemrc"
```

How xschem uses it: [XSCHEM.md](../../XSCHEM.md).

## `[klayout]`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `path` | string or list | the PDK's `libs.tech/klayout` (from `tech.toml`, else `$PDK_ROOT/$PDK/libs.tech/klayout` when it exists) | Folders for `KLAYOUT_PATH`: technologies, macros, PCell libraries. |
| `tech` | string | `tech.toml` | The KLayout technology: a name KLayout knows, or a `.lyt` file (then resolved like a path). |
| `layer_properties` | string | `tech.toml` | A `.lyp` layer-properties file. |
| `drc` | string | `tech.toml`, else looked for in the `KLAYOUT_PATH` folders | The DRC deck (`.drc`, `.lydrc`). |
| `lvs` | string | like `drc` | The LVS deck (`.lvs`, `.lylvs`). |
| `lvs_variables` | table | `tech.toml` (`[klayout.lvs_variables]`) | Variables passed to the LVS deck (`-rd name=value`), for example `lvs_sub = "VNB"`; a value is a text, a number or `true`/`false`. They win over `tech.toml` by name, and `-D` wins over both. |
| `devices` | string | the PDK's device map only | The project's device map (schematic symbol to layout PCell) that adds to the PDK's. See [LINKING.md](../../LINKING.md). |
| `[klayout.flylines]` | table | the user's own settings | How flylines look in this project. |

```toml
[klayout]
path = ["$PDK_ROOT/ihp-sg13g2/libs.tech/klayout"]
tech = "sg13g2"
drc = "$PDK_ROOT/ihp-sg13g2/libs.tech/klayout/tech/drc/ihp-sg13g2.drc"
devices = "devices.toml"

[klayout.flylines]
color = '#ff40ff'
width = 2
line = "dashed"
opacity = 80
live = true
```

A folder or file that does not exist is left out with a warning, and then the value of
`tech.toml` applies.

`[klayout.flylines]` keys:

| Key | Type | Meaning |
|---|---|---|
| `color`, `highlight`, `short`, `pin` | string `"#rrggbb"` | Colour of a flyline, of a highlighted net, of a short, of a pin. |
| `width` | integer 1 to 16 | Line width in pixels. |
| `line` | `"solid"`, `"dashed"`, `"dotted"`, `"dash-dotted"` | Line style. |
| `opacity` | integer 0 to 100 | Percent. |
| `live` | boolean | Follow edits as they happen. |
| `delay_ms` | integer 0 to 10000 | Delay before flylines are redrawn. |

An unknown key or a value of the wrong kind in this table is a warning. What the
settings do: [LINKING.md](../../LINKING.md) section 8.

## `[magic]`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `timeout` | string or number | `"15m"` | How long a batch run of Magic (`cdept magic extract`, `drc`, `lef`) may take before it is stopped. A duration such as `"90s"`, `"10m"`, `"1h30m"`, or a number of seconds; `0` is no limit. `--timeout` on the command overrides it. |

```toml
[magic]
timeout = "30m"
```

A Magic that is stopped (out of time, or its job cancelled) is killed, and the error
carries the last lines it printed. A value that is not a duration is a warning, and the
default applies. See [MAGIC.md](../../MAGIC.md).

## Where the PDK comes from

`$PDK` and `$PDK_ROOT` in library paths and in the environment of every tool are worked
out per project:

| | first | then | then | last |
|---|---|---|---|---|
| `$PDK` (name) | `technology` in `cdept.toml` | the PDK name in `tech.toml` | | `$PDK` of the environment, when it names exactly one PDK |
| `$PDK_ROOT` (folder) | `pdk_root` in [cdept.local.toml](cdept-local-toml.md) | `pdk_root` in `cdept.toml` | | `$PDK_ROOT` of the environment |

`cdept info` prints both values and where each comes from. Tools started by CDePT get
`PDK`, `PDK_ROOT` and, with both known, `PDKPATH` (`$PDK_ROOT/$PDK`). Details of the
resolution and of the snapshot: [TECH.md](../../TECH.md).

## Precedence

Several files can say the same thing. The order, strongest first:

1. `cdept.local.toml` (this user, this machine): `pdk_root`, `[env]`.
2. `cdept.toml` (the project): any key above.
3. `tech.toml` (the technology snapshot, [tech-toml.md](tech-toml.md)): defaults for
   `[xschem] rc` and the `[klayout]` keys, and the PDK's own libraries. A library
   defined in `cdept.toml` hides a PDK library of the same name.
4. What CDePT finds by itself: the PDK's `libs.tech/klayout` folder, decks found in the
   `KLAYOUT_PATH` folders, the environment's `PDK_ROOT` and `PDK`.

The snapshot is used only when it was made for the project's PDK. A `tech.toml` made
for another PDK stays on disk, is not used, and produces a warning that suggests
`cdept tech refresh`.

## Variables in paths

Paths in `[libraries]`, `include`, `[xschem]`, `[klayout]` and `pdk_root` are expanded
as follows: `$VAR` and `${VAR}` are looked up first among `$PDK_ROOT` and `$PDK` as
resolved above, then in the `[env]` table of the local file, then in the environment.
A leading `~` is `$HOME`. An unset variable makes that one path fail (a warning, or the
library's error) and does not stop the project from loading. Inside `tech.toml`, the
additional variable `$CDEPT_PROJECT` is the project folder.

## Creating the file

`cdept init [DIR]` writes `cdept.toml` from a commented template; `--name`,
`--technology` and `--pdk-root` fill in the first keys (the last one goes to the local
file). The GUI's New Project wizard does the same. The commented template documents
`[libraries]`, `[suffixes]`, `[xschem]`, `[klayout]` and `[magic]` as above.

## See also

- [cdept.local.toml](cdept-local-toml.md): per-user overrides
- [tech.toml](tech-toml.md): the technology snapshot
- [Projects, libraries, cells and views](../../concepts/projects.md)
- [Project folders](project-folders.md): what else is in a project folder
- [TECH.md](../../TECH.md): PDK resolution and the scan
- [XSCHEM.md](../../XSCHEM.md), [KLAYOUT.md](../../KLAYOUT.md), [LINKING.md](../../LINKING.md)
