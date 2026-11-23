# tech.toml and tech.catalog.json

The technology snapshot is two files next to `cdept.toml`. Both are CDePT's own formats.
The idea behind them is in [Technology and the PDK](../../concepts/technology.md).

| File | Schema | Version | Written by |
|---|---|---|---|
| `tech.toml` | `cdept-tech` | 1 | `cdept init --technology`, `cdept tech scan`, `cdept tech refresh`, the New Project wizard |
| `tech.catalog.json` | `cdept-tech-catalog` | 1 | the same commands |

## Conventions of `tech.toml`

- Every key is optional. A key without a value is left out when the file is written, so a
  missing key means "not found" or "not applicable". Defaults below are what CDePT assumes
  when the key is absent.
- Path values (type *path*) are strings, either `$PDK_ROOT/<pdk>/...` or absolute. `$VAR`,
  `${VAR}` and a leading `~` are expanded as in `cdept.toml`; `$PDK_ROOT` and `$PDK`
  follow the project's [PDK settings](../../concepts/technology.md#where-the-pdk-is-found).
- The file starts with a comment header, then the top-level keys, then one table per
  section in a fixed order: `pdk`, `klayout`, `magic`, `netgen`, `xschem`, `models`,
  `devices`, `refs`. A list that does not fit on a line is written one entry per line.
- A key CDePT does not know is ignored without a warning (a section that gains keys in a
  newer CDePT still loads). A file whose `version` is higher than 1 loads with a
  warning (`tech.toml is version 2 and this CDePT knows version 1: what it does not know
  is ignored`). A `schema` other than `cdept-tech` is refused: the project opens without a
  snapshot and says why. A file that is not valid TOML is refused the same way.
- A table under `[models]` is read as a model set (see below), so a stray table there
  becomes a model set; a stray non-table key is ignored and reported.
- The snapshot is *used* only for the PDK it was made for (`pdk.name` equals the
  project's `technology`). See
  [When the snapshot is not used](../../concepts/technology.md#when-the-snapshot-is-not-used).
- `tech.toml` is replaced as a whole by `cdept tech refresh`. Lasting changes belong in
  `cdept.toml`.

## Top level

| Key | Type | Default | Meaning |
|---|---|---|---|
| `schema` | string | | Always `"cdept-tech"`. |
| `version` | integer | | Format version, `1`. |
| `notes` | list of strings | `[]` | What the scan could not find or had to guess, one sentence each, prefixed with the section (`devices: ...`). Shown by `cdept tech notes`. |

## `[pdk]`

What the PDK says about itself.

| Key | Type | Default | Meaning |
|---|---|---|---|
| `name` | string | | `$PDK`: `ihp-sg13g2`, `sky130A`. |
| `family` | string | `other` | `sky130`, `gf180mcu`, `ihp-sg13g2`, `ihp-sg13cmos5l` or `other`. Chosen from the name. |
| `version` | string | | Version or tag the PDK states. |
| `commit` | string | | Commit of the PDK. With `version`, used to tell whether the PDK changed since the scan. |
| `source` | string | | How it was installed: `open_pdks`, `ciel`, `git`, `folder`. |
| `description` | string | | One line from the PDK. |
| `tools` | list of strings | `[]` | Tools the PDK has a `libs.tech/<tool>` folder for. |
| `branch` | string | | Branch of a PDK that is a git checkout. |
| `[pdk.tested_with]` | table of strings | `{}` | Tool versions the PDK says it was built with: `klayout = "0.30.5"`. |

```toml
[pdk]
name = "ihp-sg13g2"
family = "ihp-sg13g2"
version = "unreleased (2026-09-01)"
commit = "cb743f75aabc96e30450e9e53afc8d3c4867e5e2"
source = "git"
tools = ["klayout", "magic", "netgen", "ngspice", "xschem", "xyce"]

[pdk.tested_with]
klayout = "0.30.5"
magic = "8.3.589"
```

## `[klayout]`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `path` | list of path | `[]` | Folders for `KLAYOUT_PATH`. Without it CDePT uses `$PDK_ROOT/$PDK/libs.tech/klayout`. |
| `tech` | string | | The technology name KLayout knows (`sg13g2`, `sky130A`). PCells of the PDK are bound to it. |
| `description` | string | | Description from the `.lyt`. |
| `lyt` | path | | The technology file. |
| `base_path` | path | | The technology's folder: where its macros, decks and the files the `.lyt` names are looked for. |
| `layer_properties` | path | | The `.lyp` layer properties file. |
| `dbu` | float | 0.001 (1 nm) when absent | Database unit in micron. Used for new layouts. |
| `grids` | list of float | `[]` | Grids the technology offers, micron. |
| `default_grid` | float | | The default grid, micron. |
| `layer_maps` | list of path | `[]` | LEF/DEF layer map files (`.map`). The first is read into the catalog. |
| `lef_files` | list of path | `[]` | LEF files the technology's LEF/DEF reader loads. |
| `connectivity` | list of strings | `[]` | What the net tracer connects, `conductor,via,conductor`: `"Metal1,Via1,Metal2"`. The names stand for layer expressions, see the catalog's `symbols`. |
| `pcell_libraries` | list of strings | `[]` | PCell libraries the technology registers (`SG13_dev`). Their PCells are in the catalog. |
| `d25` | list of path | `[]` | 2.5D view scripts (`.lyd25`). |
| `xsection` | list of path | `[]` | Cross-section scripts (`.xs`). |
| `other_technologies` | list of path | `[]` | Other technology files found in the PDK; the one in `lyt` was chosen. |
| `[[klayout.drc]]` | list of tables | `[]` | DRC decks. The first is the default. |
| `[[klayout.lvs]]` | list of tables | `[]` | LVS decks. The first is the default. |

A deck table has these keys:

| Key | Type | Default | Meaning |
|---|---|---|---|
| `name` | string | | Display name. |
| `file` | path | | The deck file. |
| `form` | string | | `script` (`.drc`, `.lvs`) or `macro` (`.lydrc`, `.lylvs`). |
| `partial` | boolean | `false` | The deck checks one part only (antenna, density, ...). Written only when true. |

```toml
[klayout]
path = ["$PDK_ROOT/ihp-sg13g2/libs.tech/klayout"]
tech = "sg13g2"
lyt = "$PDK_ROOT/ihp-sg13g2/libs.tech/klayout/tech/sg13g2.lyt"
dbu = 0.001
connectivity = ["Metal1,Via1,Metal2", "Metal2,Via2,Metal3"]
pcell_libraries = ["SG13_dev", "SG13_native_pcell_lib"]

[[klayout.drc]]
name = "ihp-sg13g2"
file = "$PDK_ROOT/ihp-sg13g2/libs.tech/klayout/tech/drc/ihp-sg13g2.drc"
form = "script"
```

## `[magic]`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `rc` | path | | The start-up file (`<pdk>.magicrc`). |
| `tech` | string | `scmos` when absent | The technology name Magic knows (the `tech` line of a `.mag` file). Used for new `.mag` files. |
| `tech_file` | path | | The technology file (`.tech`). |
| `version` | string | | Version the technology file states. |
| `description` | string | | |
| `requires` | string | | The Magic version the technology asks for (`magic-8.3.617`). |
| `includes` | list of path | `[]` | Files the technology file includes. |
| `tcl` | path | | The Tcl the start-up file sources (device generators, menus). |
| `scale` | string | | Internal units per lambda, as the styles state it (`10 nanometers`). |
| `[magic.globals]` | table of strings | `{}` | Supply and substrate names the start-up file sets (`VDD`, `GND`, `SUB`). |
| `settings` | list of strings | `[]` | Settings the start-up file makes (`drc euclidean on`). |
| `cell_paths` | list of path | `[]` | Cell search paths the start-up file adds that exist. |
| `cifoutput` | list of strings | `[]` | GDS/CIF output styles, default first. |
| `cifinput` | list of strings | `[]` | GDS/CIF input styles, default first. |
| `extract` | list of strings | `[]` | Extraction styles, default first. |
| `drc` | list of strings | `[]` | DRC styles, default first (`drc(fast)`). |
| `[[magic.scripts]]` | list of tables | `[]` | Auxiliary files next to the technology: `role` (`antenna`, `density`, `fill`, `seal-ring`, `drc`, `bind-keys`, `bump-bond`, `gds-tech`) and `file` (path). |

```toml
[magic]
rc = "$PDK_ROOT/ihp-sg13g2/libs.tech/magic/ihp-sg13g2.magicrc"
tech = "ihp-sg13g2"
extract = ["ngspice", "ngspice(lvs)"]
drc = ["drc(fast)", "drc(full)"]

[magic.globals]
GND = "VSS"
VDD = "VDD"

[[magic.scripts]]
role = "density"
file = "$PDK_ROOT/ihp-sg13g2/libs.tech/magic/check_density.py"
```

## `[netgen]`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `setup` | path | | The LVS set-up file (`<pdk>_setup.tcl`). `cdept netgen lvs` uses it; without it an empty set-up is used. |
| `other_setups` | list of path | `[]` | Other set-up files in the folder that differ from `setup`. |
| `environment` | list of strings | `[]` | Environment variables the set-up file looks at (`NETGEN_COLUMNS`). |

## `[xschem]`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `rc` | path | | The PDK's `xschemrc`. |
| `evaluated` | boolean | `false` | The `xschemrc` was evaluated by the xschem helper; false means it was read as text. |
| `start_window` | path | | The schematic the PDK opens first. |
| `netlist_dir` | string | | Where the `xschemrc` sends netlists and results; `$PWD` is the folder xschem was started in. |
| `library_path` | list of path | `[]` | Folders the `xschemrc` puts on xschem's symbol search path, in order. |
| `tcl_files` | list of path | `[]` | Tcl files the `xschemrc` loads (DRC procedures, menus). |
| `model_symbols` | list of strings | `[]` | Symbols that put model libraries into the netlist (sky130's `corner.sym`). |
| `[xschem.variables]` | table of strings | `{}` | Global variables of the `xschemrc` that point into the PDK (`SG13G2_MODELS`). |
| `[xschem.references]` | table of strings | `{}` | Per library, what a schematic writes before a symbol's file name: `sg13g2_pr = "sg13g2_pr/"`, or `""`. |
| `[[xschem.libraries]]` | list of tables | `[]` | The symbol libraries the PDK brings (library table below). |

A library table (`[[xschem.libraries]]`, and the libraries of `[refs]` as far as they
are listed in a project) has these keys:

| Key | Type | Default | Meaning |
|---|---|---|---|
| `name` | string | | Library name as the project lists it. |
| `path` | path | | The library folder. |
| `layout` | string | | Folder layout: `flat`, `cells`, `ref`. See [Library layouts](project-folders.md). |
| `role` | string | | `primitives`, `stdcells`, `io`, `memory`, `tests`, `examples`, `other`. |
| `description` | string | | |
| `enabled` | boolean | `false` | Listed in a project of this PDK. Tests and examples are found but disabled. |

```toml
[xschem]
rc = "$PDK_ROOT/ihp-sg13g2/libs.tech/xschem/xschemrc"
evaluated = true

[xschem.references]
sg13g2_pr = "sg13g2_pr/"

[[xschem.libraries]]
name = "sg13g2_pr"
path = "$PDK_ROOT/ihp-sg13g2/libs.tech/xschem/sg13g2_pr"
layout = "flat"
role = "primitives"
enabled = true

[[xschem.libraries]]
name = "sg13g2_tests"
path = "$PDK_ROOT/ihp-sg13g2/libs.tech/xschem/sg13g2_tests"
layout = "flat"
role = "tests"
enabled = false
```

## `[models]`

`[models]` has three settings of its own, and one sub-table per model set. A model set is
a folder of simulator model files under the PDK's `libs.tech` (`ngspice`, `xyce`, sky130's
`combined`); CDePT may add generated sets (`[models.vacask]`).

| Key | Type | Default | Meaning |
|---|---|---|---|
| `default` | string | `ngspice` | The model set a new testbench uses: the one the PDK's `xschemrc` points at. |
| `corner` | string | | The corner set a new testbench uses (`tt`). |
| `verilog_a` | path | | The folder of Verilog-A sources (compiled with OpenVAF into OSDI). |
| `[models.<set>]` | table | | One model set. |

A model set `[models.<set>]`:

| Key | Type | Default | Meaning |
|---|---|---|---|
| `simulator` | string | | The simulator the files are written for: `ngspice`, `xyce`. |
| `dir` | path | | The folder of the library files. Entries of `libraries`, `includes` and `corners` are relative to it. |
| `libraries` | list of strings | `[]` | Files with corner sections (`.lib <section>` ... `.endl`). |
| `includes` | list of strings | `[]` | Files without sections that define devices and that no library brings in. A testbench using one of those devices includes the file itself. |
| `init` | list of path | `[]` | Start-up files the simulator must read (`.spiceinit`). |
| `options` | list of strings | `[]` | Settings found in the start-up files (`set ngbehavior=hsa`). |
| `search_path` | list of path | `[]` | Folders the start-up files put on the simulator's search path, so `.lib cornerMOSlv.lib mos_tt` works without a folder. |
| `osdi` | list of path | `[]` | Compiled Verilog-A models the simulator loads. |
| `plugins` | list of path | `[]` | Simulator plugins (Xyce `.so`). |
| `generated` | string | | Set when CDePT made the set from another one (VACASK from ngspice models): how, and the hash of the sources. A generated set is made again when the sources change. |
| `tnom` | float | | Nominal temperature of the models, degC. |
| `[models.<set>.corners]` | table of lists | `{}` | Corner sets: name to a list of `"<file> <section>"` entries (`"<file>"` alone is an include). |

```toml
[models]
default = "ngspice"
corner = "tt"

[models.ngspice]
simulator = "ngspice"
dir = "$PDK_ROOT/ihp-sg13g2/libs.tech/ngspice/models"
init = ["$PDK_ROOT/ihp-sg13g2/libs.tech/ngspice/.spiceinit"]

[models.ngspice.corners]
tt = ["cornerCAP.lib cap_typ", "cornerMOSlv.lib mos_tt"]
ss = ["cornerCAP.lib cap_wcs", "cornerDIO.lib dio_ss"]
```

How testbenches use these sets and corners is described in
[Simulation](../../concepts/simulation.md).

## `[devices]`

A summary of the device join. The devices themselves are in the catalog.

| Key | Type | Default | Meaning |
|---|---|---|---|
| `pcells` | string | | Where the KLayout PCell of a device comes from: `map` (the device map shipped with CDePT), `guess` (matched by the scan; check them), `none`. |
| `count` | integer | `0` | Devices found: symbols of the primitives libraries that netlist as a model. |
| `with_pcell` | integer | `0` | Devices with a KLayout PCell. |
| `guessed` | integer | `0` | Of those, the ones that were guessed. |
| `with_magic` | integer | `0` | Devices with a Magic generator. |
| `without_model` | integer | `0` | Devices whose model no model set defines. |
| `[devices.classes]` | table of integers | `{}` | Devices per class: `mos_lv`, `mos_hv`, `mos`, `res`, `cap`, `bjt`, `diode`, `varactor`, `esd`, `tap`, `ind`, `pad`, `other`. |

## `[refs]`

The reference libraries (`libs.ref`) and the digital flow set-up.

| Key | Type | Default | Meaning |
|---|---|---|---|
| `path` | path | | The folder of the reference libraries (`libs.ref`). |
| `[[refs.library]]` | list of tables | `[]` | One table per library, sorted by name (below). |
| `[refs.digital]` | table | | The digital flow set-up (below). |

A `[[refs.library]]` table has the library keys `name`, `path`, `layout`, `role`,
`description` and `enabled` of the [library table](#xschem), and:

| Key | Type | Default | Meaning |
|---|---|---|---|
| `cells` | integer | `0` | Number of cells. |
| `formats` | list of strings | `[]` | The format folders it has (`gds`, `lef`, `lib`, ...). |
| `techlef` | list of path | `[]` | Technology LEF files (layers, vias, sites; no macros). |
| `default_corner` | string | | The Liberty corner to start with (the typical one). |
| `sites` | table | `{}` | Placement sites: name to `[width, height]` in micron (`unithd = [0.46, 2.72]`). |
| `liberty` | table of path | `{}` | Liberty files per corner: the corner as the file names say it (`tt_025C_1v80`) to the file. A library with a file per cell gives the pattern of the names. Process, voltage and temperature of each corner are in the catalog. |

`[refs.digital]` says what LibreLane or OpenLane is told about the PDK:

| Key | Type | Default | Meaning |
|---|---|---|---|
| `flow` | string | | `librelane` or `openlane`: the folder under `libs.tech`. |
| `config` | path | | The PDK's `config.tcl`. |
| `default_library` | string | | `STD_CELL_LIBRARY`: the cells used when a design says nothing. |
| `pad_library` | string | | `PAD_CELL_LIBRARY`. |
| `default_corner` | string | | `DEFAULT_CORNER`. |
| `corners` | list of strings | `[]` | `STA_CORNERS`. |
| `rcx_rules` | list of path | `[]` | Parasitic extraction rules (OpenRCX). |
| `[refs.digital.libraries]` | table of path | `{}` | Libraries with a set-up of their own: name to their `config.tcl`. |

```toml
[refs]
path = "$PDK_ROOT/ihp-sg13g2/libs.ref"

[[refs.library]]
name = "sg13g2_stdcell"
path = "$PDK_ROOT/ihp-sg13g2/libs.ref/sg13g2_stdcell"
layout = "ref"
role = "stdcells"
enabled = true
cells = 84
default_corner = "typ_1p20V_25C"
```

## `tech.catalog.json`

JSON, pretty-printed (it diffs well if committed). The catalog holds the lists the tools
and the GUI need and nobody edits. It is read on demand, never when the project loads.
Without it the project works the same, since every setting is in `tech.toml`; the lists of
layers, PCells and symbols in the GUI stay empty until `cdept tech refresh` makes it
again. IHP SG13G2 gives about 1 MB, sky130A about 2.8 MB.

| Top-level key | Type | Meaning |
|---|---|---|
| `schema` | string | `"cdept-tech-catalog"`. Another value is refused. |
| `version` | integer | `1`. A higher version loads with a warning. |
| `pdk` | string | `$PDK` the catalog was made for. |
| `klayout` | object | See below. |
| `magic` | object | See below. |
| `netgen` | object | See below. |
| `xschem` | object | See below. |
| `models` | object | See below. |
| `devices` | object | See below. |
| `refs` | object | See below. |

Each section object holds the lists below (an absent list is empty). Entries are objects
whose keys are shown in parentheses for the main ones.

| Section | Key | Entries |
|---|---|---|
| `klayout` | `layers` | the `.lyp` layers (`name`, `layer`, `datatype`, `base`, `purpose`, `source`, colors, `dither_pattern`, `line_style`, `visible`, `group`) |
| | `pcell_libraries` | PCell libraries with their `pcells` and each PCell's `parameters` (`name`, `kind`, `default`, `unit`, `choices`, `group`, `hidden`) |
| | `pcell_source` | `klayout` (asked from the program), `files` (read from Python sources) or empty |
| | `program` | the KLayout that was asked |
| | `dither_patterns`, `line_styles` | stipples and line styles the `.lyp` defines |
| | `connections`, `symbols` | the net tracer's connectivity and the layer expressions it names |
| | `reader_options`, `writer_options` | technology options that are set (name to value) |
| | `layer_map` | the LEF/DEF layer map (first of `layer_maps`) |
| | `decks`, `macros` | every DRC and LVS deck in detail with the switches it reads; the PDK's macros |
| `magic` | `planes`, `layers`, `aliases` | planes, layer types and type aliases of the technology |
| | `styles` | every style with its section (`cifoutput`, `cifinput`, `extract`, `drc`) |
| | `gds`, `lef` | GDS numbers per CIF layer; the layers LEF names |
| | `extracted` | devices the extraction recognises and the model each is written as |
| | `devices` | device generators of the PDK's Tcl (`namespace`, `name`, `parameters`) |
| `netgen` | `defaults` | commands the set-up gives for everything |
| | `groups` | device groups: `devices`, `permute`, `series`, `parallel`, `combine`, `tolerance`, `delete`, `ignore` |
| | `patterns` | rules for cells by name pattern (`pattern`, `rule`) |
| `xschem` | `symbols` | every symbol: `library`, `name`, `kind`, `model`, `format`, `template`, `pins`, `directions`, `prefix` |
| | `counts` | symbols per library, disabled libraries included |
| | `settings`, `simulators` | other plain settings of the `xschemrc`; its `sim` commands |
| `models` | `sets` | per model set: `files` (with `role`, `class`, `sections`), `definitions` (subcircuits and models with pins and parameters), `files_read` |
| | `verilog_a` | the Verilog-A sources |
| `devices` | `devices` | one entry per device: `name`, `class`, `symbol`, `library`, `kind`, `model`, `prefix`, `terminals`, `parameters`, `models` (set to library file), `simulators`, `pcell` (`library`, `cell`, `params`, `guess`, `reason`), `magic` (`name`, `guess`, `reason`) |
| `refs` | `libraries` | per library: `formats`, `cells` (`name`, `views`, `class`, `size`, `area`, `sequential`), `liberty` files, `corners` (`name`, `process`, `voltage`, `temperature`, `files`), `techlef` |
| | `digital` | every `set ::env(NAME) value` of the digital flow's `config.tcl` files, per library (`""` is the PDK's) |

```json
{
  "schema": "cdept-tech-catalog",
  "version": 1,
  "pdk": "ihp-sg13g2",
  "klayout": {
    "layers": [
      { "name": "Metal1.drawing", "layer": 8, "datatype": 0, "base": "Metal1", "purpose": "drawing", "source": "8/0" }
    ]
  }
}
```

To look at either file without opening it, use `cdept tech show [SECTION]` for the
`tech.toml` part and `cdept tech show --catalog SECTION` for the JSON of the catalog.

## The device map

The device map says which KLayout PCell an xschem symbol becomes when a layout is
generated from a schematic. It is not part of the snapshot. CDePT reads two files, the
second on top of the first:

1. the PDK's map shipped with CDePT, `pdk/<PDK>/devices.toml` in the source tree and
   `<prefix>/share/cdept/pdk/<PDK>/devices.toml` when installed (`$CDEPT_PDK_DATA`
   overrides the folder). The map is chosen by the project's `technology`. IHP SG13G2 has
   one;
2. the project's own map, named by `devices` in the `[klayout]` table of `cdept.toml`
   (`devices = "devices.toml"`, relative to the project folder). An entry for the same
   symbol replaces the PDK's.

For a PDK without a shipped map, `cdept tech devices [-o FILE]` writes a draft made from
the scan. Its matches are guesses; check each entry, save the file in the project and name
it in `[klayout] devices`. What the map is used for, and how PCell parameters are checked
against the schematic, is in [LINKING.md](../../LINKING.md) (section 3 for the entries,
section 9 for pins).

### File schema

```toml
format = "cdept-devices"      # required
version = 1                   # required; 1 is the only version

[pins]                        # optional, see LINKING.md section 9
generate = "8/0"
box = 0.5
size = 0.1
layers = [
  { drawing = "8/0", pin = "8/2", label = "8/25" },
]

[[device]]
symbol = "sg13g2_pr/sg13_lv_nmos.sym"
library = "SG13_dev"
cell = "nmos"
params = { model = "@model", w = "@{w|u}", l = "@{l|u}", ng = "@ng", m = "@m" }
terminals = { D = "D", G = "G", S = "S", B = "B" }
```

Top level:

| Key | Type | Meaning |
|---|---|---|
| `format` | string | Must be `"cdept-devices"`; anything else is an error. |
| `version` | integer | Must be `1`. A higher version is refused. |
| `[pins]` | table | How pins are drawn in the stream (below). Optional. |
| `[[device]]` | list of tables | One entry per symbol. |

`[[device]]`:

| Key | Type | Required | Meaning |
|---|---|---|---|
| `symbol` | string | yes | The symbol as the schematic refers to it. With a folder (`sg13g2_pr/sg13_lv_nmos.sym`) it matches that reference, also at the end of a longer path; without a folder it matches the file name in any folder. A match with a folder wins over one without. |
| `library` | string | yes | The KLayout library of the PCell (or fixed cell). |
| `cell` | string | yes | The PCell, or fixed cell, in that library. |
| `params` | table of strings | no | PCell parameter to template (below). |
| `terminals` | table of strings | no | Symbol pin to the terminal of the device KLayout's extraction makes of the PCell, used by the net comparison. Without it a pin goes to the terminal of the same name (any case), else the pins go to the terminals in order. |

A parameter template is a string. `@name` is replaced by the instance's attribute `name`,
else by the symbol's default (its `template=`). `@{name|unit}` reads the value as a SPICE
number (`0.5e-6`, `2u`, `1.5meg`) and writes it in the unit: `f`, `p`, `n`, `u`, `m`, `k`
write the number with that suffix (`0.5u`), `num` writes a plain number, `um` writes a
plain number of micron (`0.5e-6` becomes `0.5`, for PCells that count in micron). Values
are converted to the PCell parameter's type by the KLayout helper, so an integer parameter
gets `1`, not `"1"`. Text without `@` is passed as it is.

`[pins]`:

| Key | Type | Meaning |
|---|---|---|
| `generate` | string | `layer/datatype` of the conductor generated pins go on. |
| `box` | number | Size of a generated pin, micron. |
| `size` | number | Text size of pin labels, micron. |
| `layers` | list | Per conductor: `drawing` (`layer/datatype`, `*` for every conductor), `pin` (where the pin shape goes; absent: the technology has no pin layer, the shape stays CDePT's own) and `label` (where its name goes as text). Also accepted as `[[pins.layers]]` tables. |
| `layer` | string | Version 1 form: one label layer for every conductor. `layers` replaces it. |

## See also

- [Technology and the PDK](../../concepts/technology.md)
- [Create a project for a PDK](../../howto/new-project.md)
- [cdept.toml](cdept-toml.md) and [cdept.local.toml](cdept-local-toml.md)
- [TECH.md](../../TECH.md): scan design, section by section
- [LINKING.md](../../LINKING.md): the device map, pins, linking
