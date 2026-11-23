# CDePT: technology (PDK) in a project

Status 2026-11-22: built (ROADMAP Phase 2). Open points: §11.

A project is designed in one PDK. Each tool wants to be told about it in its own
way: KLayout a technology and decks, xschem an `xschemrc`, magic a `magicrc`, netgen a
set-up file, the simulator model files and corner sections. CDePT reads all of that
out of the PDK **once**, when the project is set up for it, and keeps what it found
next to the project file. From then on every tool CDePT starts is configured from
that one place.

## 1. The files

```
myproject/
  cdept.toml           # project: name, technology = "ihp-sg13g2", libraries   (committed)
  tech.toml            # technology snapshot: what the scan found, small        (committed)
  tech.catalog.json    # the long lists behind it: layers, PCells, symbols …    (committed or not)
  cdept.local.toml     # this user, this machine: pdk_root, [env]               (not committed)
```

| File | Written by | Edited by hand | Schema |
|---|---|---|---|
| `cdept.toml` | `cdept init`, the GUI | yes, comments are kept | (project file) |
| `tech.toml` | the scan (`cdept init --technology`, `cdept tech scan/refresh`, the New Project wizard) | yes, and the edit applies; a refresh replaces the whole file (after a warning, with `tech.toml.bak` kept) | `cdept-tech`, version 1 |
| `tech.catalog.json` | the scan | no; it can always be made again | `cdept-tech-catalog`, version 1 |
| `cdept.local.toml` | `cdept init --pdk-root`, Project Settings, the scan | yes, comments are kept | (two keys) |

Both schemas are CDePT's own. A file written by a newer CDePT (higher `version`) loads
with a warning; what this version does not know is ignored. A `tech.toml` that cannot
be read is a warning, not an error: the project opens without a snapshot.

## 2. Where the PDK is: `$PDK`, `$PDK_ROOT`, `cdept.local.toml`

A PDK is the folder `$PDK_ROOT/$PDK` (it has `libs.tech` in it). The name is the
project's; the folder differs per machine.

| | first | then | then | last |
|---|---|---|---|---|
| `$PDK` (the name) | `technology` in `cdept.toml` | the PDK `tech.toml` was made for | | `$PDK` of the environment, when it names one PDK |
| `$PDK_ROOT` (the folder) | `pdk_root` in `cdept.local.toml` | `pdk_root` in `cdept.toml` (still read; not written by CDePT any more) | | `$PDK_ROOT` of the environment |

`cdept info` shows both values and where each comes from.

- **Several PDKs in `$PDK`.** The asic-all image sets `PDK=sky130A:gf180mcu:ihp-sg13g2`.
  That is what is *available* (`Pdk::available`); the project chooses one with
  `technology`. With several on offer and none chosen there is no PDK and a warning
  says so. Tools started by CDePT always get one name in `PDK`.
- **`cdept.local.toml`** holds what must not travel with the project:

  ```toml
  # Where PDKs are installed on this machine ($PDK_ROOT):
  pdk_root = "/opt/ext/OpenPDKs"

  # Variables for $VAR in the project's paths, also given to every tool CDePT starts:
  [env]
  MY_LIBS = "/work/libs"
  ```

  `pdk_root` may be relative to the project folder and may use `$VAR` and `~`. `[env]`
  values may use `~` and the variables of the environment. The file wins over
  `cdept.toml` and over the environment. When CDePT creates it inside a git
  repository it adds the name to the project's `.gitignore`. CDePT writes `pdk_root`
  there only when it is needed: when the environment's `PDK_ROOT` already points at
  the PDK's folder, nothing is written.
- Tools started for a project get ([`Project::tool_env`]): `[env]` of the local file,
  `PDK_ROOT`, `PDK` and `PDKPATH` (`$PDK_ROOT/$PDK`, which open_pdks scripts expect).

## 3. The scan

`tech::scan(&ScanContext) -> (Tech, Catalog)` runs one scan per section, side by side
(`libcdept/src/tech/`, one module per section). A section returns its part of
`tech.toml`, its part of the catalog, and notes: what it could not find or had to
guess. The notes are kept in `tech.toml` (`notes = […]`, `cdept tech notes`).

Two kinds of scan:

- **files only** (`--files-only`): nothing is started; everything comes from reading
  the PDK's files;
- **with the helpers** (the default): KLayout is started to list the PCell libraries
  and their parameters, xschem to evaluate the `xschemrc`. Slower by a few seconds,
  and it needs the helper programs (docs/KLAYOUT.md, docs/XSCHEM.md).

Paths are stored as `$PDK_ROOT/<pdk>/…` (`TechPath`), so the snapshot is the same on
every machine and can be committed.

### 3.1 Finding the PDK (`tech::discover`, `tech::ciel`) → `[pdk]`

A PDK is a folder with `libs.tech` in it; a *root* (what `$PDK_ROOT` means) holds one
subfolder per PDK. Discovery is cheap and careful: it lists each root once, asks only
whether `<child>/libs.tech` exists and never lists a subfolder that is not a PDK (this is
why NDA folders next to real PDKs are only stat'ed). A checkout that holds PDKs one level
down (IHP-Open-PDK holds `ihp-sg13g2` and `ihp-sg13cmos5l`) is recognised by asking for
the well-known names (`NESTED_NAMES`), not by walking it. ciel (formerly volare) keeps
`<root>/ciel/<family>/versions/<commit>/<pdk>` and links the one in use into the root;
CDePT reads that layout itself, so listing and switching to a version already there
(`cdept pdk versions|enable`) work without the program. Fetching needs `ciel` (not
exercised yet). A PDK is looked for in the
project's PDK folder, `$PDK_ROOT`, `$CDEPT_PDK_PATH`, `~/.ciel`, `~/.volare`,
`/usr/share/pdk`, `/usr/local/share/pdk`, `~/share/pdk` (`cdept pdk ls`). `[pdk]` has
what the PDK says about itself: name, family (`sky130`, `gf180mcu`, `ihp-sg13g2`,
`ihp-sg13cmos5l`, `other`), version, commit, how it was installed (`open_pdks`, `ciel`,
`git`, `folder`), the tools it has a `libs.tech/<tool>` folder for.

### 3.2 KLayout (`tech::klayout`) → `[klayout]`

The scan reads the files; no program is needed for the first part. From files: the technology
(`.lyt`: name, database unit, layer properties file, reader options, connectivity),
the layers (`.lyp`), the LEF/DEF layer maps, the DRC and LVS decks with the switches
they read, macros, 2.5D and cross-section scripts. With the helper: the PCell
libraries the technology registers and every PCell's parameters. Fixed keys used by
the project: `path`, `tech`, `lyt`, `layer_properties`, `dbu`, `drc`, `lvs` (the first
deck of each list is the default).

### 3.3 Magic (`tech::magic`) → `[magic]`

Read from the files; Magic is never started. The start-up file (`.magicrc`: technology it
loads, Tcl it sources, supply names, cell paths), the technology file (`.tech` and what it includes: name, version, planes, CIF/GDS,
extraction and DRC styles, GDS numbers, devices the extraction knows), the device
generators (`<name>_defaults` procedures) and the scripts next to them. Fixed keys: `rc`, `tech`,
`tech_file`.

### 3.4 Netgen (`tech::netgen`) → `[netgen]`

The LVS set-up file is Tcl; it is read, not run. Kept: the device groups it sets rules for
(pin swaps, series/parallel merging, compared properties with tolerances, dropped ones),
the cell name patterns it treats specially and the environment variables it looks at.
Fixed key: `setup`.

### 3.5 xschem (`tech::xschem`) → `[xschem]`

The `xschemrc`, evaluated in the engine when the helper may be
started (else read as text, with a note), the symbol libraries on its search path,
the variables that point into the PDK; the catalog has every symbol. Fixed keys:
`rc`, `libraries`.

### 3.6 Simulation models (`tech::models`) → `[models]`

One model set per folder (`ngspice`, `xyce`, sky130's
`combined`): library files, start-up files, OSDI, and the named corner sets (`tt`,
`ff`, `ss`, `sf`, `fs`, `mc`, `tt_mm`, …), each a list of `"<file> <section>"`.

### 3.7 Devices (`tech::devices`) → `[devices]`

The PDK's devices joined from the other sections: symbol →
model → KLayout PCell and Magic generator. The PCell comes from the device map
shipped with CDePT when there is one (IHP SG13G2), else it is guessed and marked so;
`cdept tech devices` writes the guesses as a draft map (docs/LINKING.md §3).

### 3.8 Reference libraries (`tech::refs`) → `[refs]`

`libs.ref/<library>/<format>/…`: standard cells, IO, memories, primitives, and the digital
flow set-up next to them (`libs.tech/librelane` or `openlane`). Each library is read as a
CDePT library of the `ref` layout, so what the snapshot lists is what the project shows.
`tech.toml` gets, per library: purpose, formats, cell count, technology LEFs and Liberty
corners (process, voltage, temperature); the catalog gets the cells with their views and
facts (LEF class and size, Liberty area), every Liberty file, LEF layers and sites, and
every digital-flow setting. Which file is a library's GDS, SPICE, CDL, LEF, Verilog and
Liberty file comes from the flow's configuration (`tech::librelane`, a tolerant reader of
the Tcl, never run), else from the folders: `[refs.library.files]`.

## 4. `tech.toml`

What a designer may want to look at or correct, section by section; real excerpt
(`examples/ihp-inv/tech.toml`, IHP SG13G2, shortened with `…`):

```toml
# CDePT technology snapshot of the PDK "ihp-sg13g2": what CDePT read out of the PDK's
# files when this project was set up for it. …
schema = "cdept-tech"
version = 1
notes = [
    "devices: 2 device(s) netlist as a model no model library defines (inductor, inductor3)",
    "libraries: the PDK has two libraries named 'sg13g2_pr'; a project lists … and not …",
]

[pdk]
name = "ihp-sg13g2"
family = "ihp-sg13g2"
version = "unreleased (2026-09-01)"
commit = "cb743f75aabc96e30450e9e53afc8d3c4867e5e2"
source = "git"

[klayout]
path = ["$PDK_ROOT/ihp-sg13g2/libs.tech/klayout"]
tech = "sg13g2"
lyt = "$PDK_ROOT/ihp-sg13g2/libs.tech/klayout/tech/sg13g2.lyt"
layer_properties = "$PDK_ROOT/ihp-sg13g2/libs.tech/klayout/tech/sg13g2.lyp"
dbu = 0.001
pcell_libraries = ["SG13_dev", "SG13_native_pcell_lib"]

[[klayout.drc]]                 # the first deck of a list is the default
name = "ihp-sg13g2"
file = "$PDK_ROOT/ihp-sg13g2/libs.tech/klayout/tech/drc/ihp-sg13g2.drc"

[[klayout.lvs]]
name = "sg13g2"
file = "$PDK_ROOT/ihp-sg13g2/libs.tech/klayout/tech/lvs/sg13g2.lvs"

[magic]
rc = "$PDK_ROOT/ihp-sg13g2/libs.tech/magic/ihp-sg13g2.magicrc"
tech = "ihp-sg13g2"
tech_file = "$PDK_ROOT/ihp-sg13g2/libs.tech/magic/ihp-sg13g2.tech"

[netgen]
setup = "$PDK_ROOT/ihp-sg13g2/libs.tech/netgen/ihp-sg13g2_setup.tcl"

[xschem]
rc = "$PDK_ROOT/ihp-sg13g2/libs.tech/xschem/xschemrc"
evaluated = true

[[xschem.libraries]]
name = "sg13g2_pr"
path = "$PDK_ROOT/ihp-sg13g2/libs.tech/xschem/sg13g2_pr"
layout = "flat"
role = "primitives"
description = "xschem: device symbols (47 symbols, 1 schematics)"
enabled = true

[[xschem.libraries]]
name = "sg13g2_tests"           # found, not listed in the project
…
enabled = false

[models]
default = "ngspice"
corner = "tt"

[models.ngspice]
simulator = "ngspice"
dir = "$PDK_ROOT/ihp-sg13g2/libs.tech/ngspice/models"
init = ["$PDK_ROOT/ihp-sg13g2/libs.tech/ngspice/.spiceinit"]

[models.ngspice.corners]
tt = [
    "cornerCAP.lib cap_typ",
    "cornerMOSlv.lib mos_tt",
    …
]

[devices]
pcells = "map"
count = 43
with_pcell = 15

[refs]
path = "$PDK_ROOT/ihp-sg13g2/libs.ref"

[[refs.library]]
name = "sg13g2_stdcell"
path = "$PDK_ROOT/ihp-sg13g2/libs.ref/sg13g2_stdcell"
layout = "ref"
role = "stdcells"
enabled = true
```

How the file is written (`tech::file`):

- a header comment says what the file is and how it is made again;
- top-level keys first (`schema`, `version`, `notes`), then one table per section in
  a fixed order: `pdk`, `klayout`, `magic`, `netgen`, `xschem`, `models`, `devices`,
  `refs`; inside a section the keys keep the order of the section's struct;
- a list that does not fit on a line is written one entry per line; a list of small
  tables inside an entry of another list is written as rows (`{ … }`, one per line);
- only what was found is written: an unset key is left out.

Sizes: IHP SG13G2 about 550 lines, sky130A about 630 (six reference libraries with
their Liberty corners). The file holds no path of the machine it was made on.

## 5. `tech.catalog.json`

JSON, pretty-printed (it diffs, should it be committed). One object per section, with
the lists the tools and the GUI need but nobody edits. For IHP SG13G2 (about 1 MB;
sky130A about 2.8 MB):

| Section | Lists (entries) |
|---|---|
| `klayout` | connections (8), decks (2), dither_patterns (54), layer_map (56), layers (378), line_styles (12), macros (10), pcell_libraries (2), pcell_source, program, reader_options (84), symbols (18), writer_options (21) |
| `magic` | aliases (60), devices (39), extracted (44), gds (233), layers (119), lef (27), planes (14), styles (15) |
| `netgen` | defaults (3), groups (9), patterns (2) |
| `xschem` | counts (5), settings (6), simulators (0), symbols (131) |
| `models` | sets (2), verilog_a (6) |
| `devices` | devices (43) |
| `refs` | digital (3), libraries (4) |

```json
{
  "schema": "cdept-tech-catalog",
  "version": 1,
  "pdk": "ihp-sg13g2",
  "klayout": {
    "layers": [
      { "name": "Substrate.drawing", "layer": 40, "datatype": 0, "source": "40/0", … },
      …
```

The catalog is read on demand (`Project::catalog()`), never when a project loads.
Whether to commit it is the team's choice: without it the project works the same
(every setting is in `tech.toml`); lists of layers, PCells and symbols in the GUI are
empty until `cdept tech refresh` makes it again.

## 6. How the project uses the snapshot

Everywhere the order is: **`cdept.toml` → `tech.toml` → what CDePT finds by itself**.

| Setting | `cdept.toml` | from the snapshot | without either |
|---|---|---|---|
| xschem start-up file | `[xschem] rc` | `xschem.rc` | none |
| `KLAYOUT_PATH` | `[klayout] path` | `klayout.path` | `$PDK_ROOT/$PDK/libs.tech/klayout` |
| KLayout technology | `[klayout] tech` | `klayout.tech` (else the `.lyt`) | none |
| layer properties | `[klayout] layer_properties` | `klayout.layer_properties` | the technology's |
| DRC deck | `[klayout] drc` | first of `klayout.drc` | searched in the `KLAYOUT_PATH` folders |
| LVS deck | `[klayout] lvs` | first of `klayout.lvs` | searched likewise |
| libraries | `[libraries]`, `include` | the PDK's libraries marked `enabled` | none |
| database unit of new layouts | | `klayout.dbu` | 1 nm |
| magic technology of new `.mag` | | `magic.tech` | `scmos` |

The snapshot's values are filled into the same fields the project file fills
(`Project::xschem_rc`, `Project::klayout`), so the code that starts the tools did not
change. A file the snapshot names but that is not on this machine is skipped.

The snapshot is **used** only when it was made for the project's PDK
(`Project::tech_in_use`). Otherwise it stays loaded (`Project::tech`, the GUI can
still show it) and a warning says to refresh. Other warnings at load:

- `$PDK_ROOT` is not set, or the PDK is not in that folder: set `pdk_root` in
  `cdept.local.toml`;
- the PDK on this machine is another version or commit than the one scanned, or its
  LibreLane/OpenLane `config.tcl` files differ from those read (`[refs.digital]
  config_hash`) (`Project::snapshot_stale`): run `cdept tech refresh`;
- a file `[[refs.library]] files` names is not on disk: the folders decide that format.
  `[refs.library.files]` is what the views' main files follow; the PDK's configuration
  is read by the scan only (docs/concepts/technology.md, "Hand edits and refresh").

### Libraries the PDK brings

The snapshot lists them (`Tech::libraries()`: the xschem symbol libraries first, then
the reference libraries of `libs.ref`). Those marked `enabled` appear in the project
after its own libraries: read-only, `defined_in` = `tech.toml`. Test and example
libraries are found but not enabled; set `enabled = true` in `tech.toml`, or define
the library in `cdept.toml`.

- A name the project defines itself wins; the PDK's library of that name is not
  listed.
- **Two PDK libraries of one name** (`Tech::project_libraries`):
  - one folder holds the other (`libs.ref/sg13g2_stdcell` and its
    `sym/xschem` folder, which xschem has on its search path): the outer one is
    listed; the `ref` layout shows the symbols as views of the same cells;
  - neither holds the other (`libs.tech/xschem/sg13g2_pr` with the symbols and
    `libs.ref/sg13g2_pr` with fixed GDS layouts): a CDePT library is one folder, so
    the first, the xschem one (which schematics refer to by folder name), is listed
    and the other left out. The scan says so in its notes; loading does not warn
    again. A library made of several folders is a later change of the model (§11).

## 7. New views

`Project::template_setup()` gives what a new view takes from the technology
(`template::Setup`), used by `Library::create_view_with`:

- **GDS**: the database unit of the snapshot (the UNITS record is worked out in whole
  numbers, so 0.001 and 1e-9 come out bit for bit as other writers have them);
- **Magic** (`Template::MagicLayout`, `<cell>.mag`): `tech <magic.tech>`;
- **xschem** schematic and symbol: the header of the embedded engine
  (`template::XSCHEM_VERSION`, `XSCHEM_FILE_VERSION`: 3.4.8RC, file version 1.3; a
  test compares them with the submodule's `xschem.h`).

## 8. Command line

```
cdept init [DIR] --technology <pdk|folder> [--pdk-root DIR] [--no-scan] [--files-only]
cdept tech scan [PDK] [--pdk-root DIR] [--files-only] [--print] [--yes]
cdept tech show [SECTION] [--catalog] [--json]
cdept tech refresh [--files-only] [--dry-run] [--yes]
cdept tech notes
cdept tech devices [-o FILE]    # draft device map (symbol → PCell) from the scan
cdept info                      # PDK, PDK_ROOT and their source, the snapshot, [env]
cdept pdk ls | info | versions | enable     # PDKs on this machine (§3, discovery)
```

- `init --technology ihp-sg13g2` looks for the PDK (in `--pdk-root`, else `$PDK_ROOT`
  and the usual places, see `cdept pdk ls`), scans it, writes the four files and
  prints what was found per tool, then the notes. A PDK that is not on this machine
  still gives a project (with `technology` set and a warning); `--no-scan` asks for
  exactly that.
- `tech scan` does the same for an existing project, also to change its PDK.
  `--print` only prints `tech.toml` and needs no project.
- `tech refresh` scans the project's PDK again and lists what changes before writing
  (`~ klayout.tech: a → b`, `+ magic.rc: …`, `~ catalog klayout.layers: 205 → 207
  entries`); `--dry-run` stops there. The file is replaced as a whole, so it warns that
  hand edits are lost, copies the old file to `tech.toml.bak` and asks `Replace
  tech.toml? [y/N]`; `--yes` skips the question, and without a terminal it is required.
  `tech scan` over an existing `tech.toml` does the same.
- `tech show` prints the summary; `tech show klayout` that section as it is in
  `tech.toml`; `--catalog` the section's lists as JSON.

What `cdept init --technology ihp-sg13g2` prints (shortened):

```
scanning /opt/ext/OpenPDKs/ihp-sg13g2 …
created /work/adc/cdept.toml
PDK ihp-sg13g2 in /opt/ext/OpenPDKs (cdept.local.toml)
pdk      commit            cb743f75aabc96e30450e9e53afc8d3c4867e5e2
         source            git
         …
klayout  dbu               0.001
         drc               ihp-sg13g2
         lvs               sg13g2
         tech              sg13g2
         catalog           … 378 layers, … 2 pcell_libraries, …
magic    rc                $PDK_ROOT/ihp-sg13g2/libs.tech/magic/ihp-sg13g2.magicrc
         tech              ihp-sg13g2
         …
models   corner            tt
         default           ngspice
         …
devices  count             43
         pcells            map
         …
notes:
  - devices: 2 device(s) netlist as a model no model library defines (inductor, inductor3)
  - libraries: the PDK has two libraries named 'sg13g2_pr'; a project lists … and not …
```

Measured here (debug build): IHP SG13G2 1.3 s, sky130A 1.4 s with the helpers; 0.1 s
and 1.2 s reading files only. Loading a project with a snapshot costs the scan of
the PDK's libraries (sky130A: 0.2 s with the index cache of the reference
libraries warm).

## 9. For the code that starts tools

| Question | Answer |
|---|---|
| the PDK folder | `Project::pdk_dir() -> Option<PdkDir>` |
| the snapshot, when in use | `Project::snapshot() -> Option<&Tech>` |
| the catalog | `Project::catalog() -> Result<Option<Catalog>>` (read from disk each time) |
| a path the snapshot names, on this machine | `Project::tech_path(&TechPath) -> Option<PathBuf>` |
| environment of every tool | `Project::tool_env()` |
| xschem start-up files | `Project::xschem_rc` |
| KLayout settings | `Project::klayout`, `Project::klayout_path()` |
| magic start-up file, technology name | `Project::magic_rc()`, `Project::magic_tech()` |
| netgen set-up | `Project::netgen_setup()` |
| model sets, corner sets | `Project::model_simulators()`, `Project::model_corners(simulator)` |
| `.lib`/`.include` lines of a testbench | `Project::model_lines(simulator, corner)` |
| database unit, template set-up | `Project::dbu()`, `Project::template_setup()` |
| scan again | `Project::scan_technology(helpers)` |
| change the PDK | `Project::set_technology(&PdkDir, &Tech, &Catalog)`, `Project::create_for_pdk(…)` |
| is the snapshot still right | `Project::snapshot_stale() -> Option<String>` |

Reading and writing the files: `tech::file` (`load_tech`, `load_catalog`, `save`,
`tech_text`, `section_text`). A summary and the difference between two scans:
`tech::report` (`summary`, `summary_text`, `changes`); both walk the files' own
shape, so a section that gains fields needs no change there.

## 10. NDA and hand-made PDKs

Nothing of a PDK is shipped with CDePT and nothing is copied out of it except names,
paths and numbers into the two snapshot files. For a PDK whose layout the scan does
not understand, `tech.toml` can be written by hand: every key is optional, and paths
may be absolute. Before committing the snapshot of an NDA PDK, read it: layer names,
device names and model corner names are in it.

## 11. Open points

- **A library is one folder.** `sg13g2_pr` (xschem symbols) and `libs.ref/sg13g2_pr`
  (GDS) are one library in the designer's mind; today the second is left out, with a
  note from the scan. Needs `Library` to hold several folders, each with its layout.
- **Disabling or enabling a PDK library** is an edit of `tech.toml` (`enabled`), which
  a refresh undoes. A place in `cdept.toml` for it (e.g. `[technology] libraries =
  […]`) would make it last.
- **A refresh replaces `tech.toml`.** Hand edits are lost; `cdept tech refresh` lists
  what differs and asks first, and keeps `tech.toml.bak`. Lasting overrides exist only for what `cdept.toml` has keys for
  (xschem rc, the `[klayout]` settings, libraries).
- **Which deck, which corner.** The first DRC and LVS deck and `models.corner` are the
  defaults; there is no project setting yet for "use the second deck" other than
  `[klayout] drc = "…"`.
- **xschem header.** New files carry the version of the pinned engine through two
  constants (a test compares them with the submodule); a running engine of another
  version is not asked (`template::Setup::xschem` is there for it).
- **`pdk_root` in `cdept.toml`** is still read, so old projects work; CDePT no longer
  writes it there.
- **NDA PDKs**: nothing checks what goes into the snapshot before it is committed.
- **The device map** is not part of the snapshot: the shipped map
  (`pdk/<PDK>/devices.toml`) and `[klayout] devices` are used as before; for other
  PDKs `cdept tech devices` writes a draft to check by hand.
