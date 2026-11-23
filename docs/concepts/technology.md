# Technology and the PDK

A project is designed in one process design kit (PDK). Every tool wants to be told about
it in its own way: KLayout a technology and DRC/LVS decks, xschem an `xschemrc`, Magic a
`magicrc`, netgen a set-up file, the simulators model files and corner sections. CDePT
reads all of that out of the PDK once, when a project is set up for it, and keeps what it
found next to the project file. From then on every tool CDePT starts is configured from
that one place.

This page explains the idea. The file formats are in
[tech.toml and tech.catalog.json](../reference/files/tech-toml.md); the design notes are
in [TECH.md](../TECH.md).

## The technology snapshot

The result of the scan is the *technology snapshot*, two files in the project folder:

| File | Contents | Edited by hand |
|---|---|---|
| `tech.toml` | Small and readable: where each tool's set-up is, which model files and corners exist, which libraries the PDK brings. | Possible, see [below](#hand-edits-and-refresh) |
| `tech.catalog.json` | The long lists behind it: layers, PCells and their parameters, symbols, devices, the cells of the reference libraries. | No; it can always be made again |

Both are CDePT's own formats. They are written by `cdept init --technology`,
`cdept tech scan`, `cdept tech refresh` and the New Project wizard of the GUI.

Paths in the snapshot are stored as `$PDK_ROOT/<pdk>/...`, so the files are the same on
every machine and can be committed. The snapshot holds no path of the machine it was made
on.

The snapshot is a copy of facts, not of the PDK: nothing is copied out of the PDK except
names, paths and numbers.

## Where the PDK is found

A PDK is a folder that contains `libs.tech`. Two names describe it, as in the PDK tools:

- `$PDK` is the PDK's name (`ihp-sg13g2`, `sky130A`). It is the `technology` key of
  `cdept.toml`.
- `$PDK_ROOT` is the folder that holds the PDK folders on this machine
  (`/opt/ext/OpenPDKs`). It differs per machine.

`$PDK_ROOT` is taken from the first of these that is set:

1. `pdk_root` in `cdept.local.toml` (this user, this machine, not committed);
2. `pdk_root` in `cdept.toml` (still read, no longer written);
3. the `PDK_ROOT` environment variable.

`$PDK` is taken from `technology` in `cdept.toml`, else from the PDK the snapshot was made
for, else from the `PDK` environment variable when it names exactly one PDK. When the
environment lists several (`PDK=sky130A:gf180mcu:ihp-sg13g2`), the project chooses one with
`technology`.

`cdept info` prints both values and where each comes from:

```text
PDK          ihp-sg13g2  (project)
PDK_ROOT     /opt/ext/OpenPDKs  (environment)
snapshot     tech.toml  (PDK ihp-sg13g2)
```

When the environment's `PDK_ROOT` already points at the PDK, CDePT writes no
`cdept.local.toml`. Otherwise it writes `pdk_root` there and adds the file to the
project's `.gitignore`. See [cdept.local.toml](../reference/files/cdept-local-toml.md).

CDePT looks for installed PDKs in `$PDK_ROOT`, `$CDEPT_PDK_PATH`, `~/.ciel`, `~/.volare`,
`/usr/share/pdk`, `/usr/local/share/pdk` and `~/share/pdk`. `cdept pdk ls` lists what it
finds, with version, commit and how each was installed (`ciel`/`volare`, `open_pdks`,
`git`, `folder`). `cdept pdk versions` and `cdept pdk enable` list and switch the versions
that ciel keeps under `<root>/ciel/<family>/versions/`; they read that layout themselves
and do not need the `ciel` program. Fetching a version does need it.

Every tool CDePT starts gets `PDK_ROOT`, `PDK` and `PDKPATH` (`$PDK_ROOT/$PDK`, which
open_pdks scripts expect) in its environment, plus the `[env]` table of
`cdept.local.toml`.

## What the scan captures

One scan per section, side by side. Sections that read files never start a program; with
the default scan, KLayout and xschem are started for the parts only they can answer. A
section also records *notes*: what it could not find or had to guess (`cdept tech notes`).

| Section | Captures | Source |
|---|---|---|
| `[pdk]` | name, family, version, commit, how it was installed, the tools it has a `libs.tech/<tool>` folder for, tool versions it was tested with | the PDK folder |
| `[klayout]` | technology (`.lyt`) and its database unit, layer properties, grids, LEF/DEF layer maps, connectivity, DRC and LVS decks, 2.5D and cross-section scripts, PCell libraries | files; PCells and their parameters from the KLayout helper |
| `[magic]` | start-up file, technology file and its styles (extraction, DRC, GDS in/out), supply names, auxiliary scripts | files (Magic is never started) |
| `[netgen]` | the LVS set-up file, the device groups it sets rules for, the environment variables it reads | the Tcl file, read not run |
| `[xschem]` | the `xschemrc`, the symbol libraries on its search path, the variables that point into the PDK, every symbol | the `xschemrc` evaluated by the xschem helper (else read as text, with a note) |
| `[models]` | model sets per simulator (`ngspice`, `xyce`, ...), their library files, start-up files, OSDI, and the named corner sets (`tt`, `ff`, `ss`, `sf`, `fs`, `mc`, ...) | the PDK's model folders |
| `[devices]` | the PDK's devices joined from the other sections: symbol, model, KLayout PCell, Magic generator | derived |
| `[refs]` | the reference libraries of `libs.ref` (standard cells, IO, memories): cells, formats, Liberty corners, technology LEFs; the digital flow's set-up | `libs.ref`, `libs.tech/librelane` or `openlane` |

The PCell of a device comes from the *device map* shipped with CDePT when there is one
(IHP SG13G2), else it is guessed and marked as a guess. See
[the device map](../reference/files/tech-toml.md#the-device-map) and
[LINKING.md](../LINKING.md) for what the map is used for.

The scan understands the PDK families `sky130`, `gf180mcu`, `ihp-sg13g2` and
`ihp-sg13cmos5l`; any other PDK is family `other` and works as far as it follows the
`libs.tech` / `libs.ref` layout.

## How the project uses the snapshot

Everywhere the order of precedence is:

1. what `cdept.toml` says;
2. what `tech.toml` says;
3. what CDePT finds by itself.

For example, the DRC deck is `[klayout] drc` of `cdept.toml`; without it, the first
entry of `klayout.drc` in `tech.toml`; without that, the first `*.drc` file found in the
`KLAYOUT_PATH` folders.

| Setting | `cdept.toml` | From the snapshot | Without either |
|---|---|---|---|
| xschem start-up file | `[xschem] rc` | `xschem.rc` | none |
| `KLAYOUT_PATH` | `[klayout] path` | `klayout.path` | `$PDK_ROOT/$PDK/libs.tech/klayout` |
| KLayout technology | `[klayout] tech` | `klayout.tech` (else the `.lyt`) | none |
| Layer properties | `[klayout] layer_properties` | `klayout.layer_properties` | the technology's |
| DRC / LVS deck | `[klayout] drc` / `lvs` | first of `klayout.drc` / `klayout.lvs` | searched in the `KLAYOUT_PATH` folders |
| Libraries | `[libraries]`, `include` | the PDK's libraries marked `enabled` | none |
| Database unit of new layouts | | `klayout.dbu` | 1 nm |
| Magic technology of new `.mag` files | | `magic.tech` | `scmos` |
| Model sets and corners of testbenches | | `[models]` | none |
| netgen set-up | | `netgen.setup` | an empty set-up |

A file that the snapshot names but that does not exist on this machine is skipped.

### Libraries the PDK brings

The snapshot lists the PDK's symbol libraries (xschem) and reference libraries
(`libs.ref`). Those marked `enabled = true` appear in the project after the project's own
libraries, read-only. Test benches and examples of the PDK are found but not enabled. To
list one, set `enabled = true` in `tech.toml`, or define the library in `cdept.toml`. A
library the project defines with the same name wins over the PDK's.

When a PDK has two libraries of one name that are not nested (IHP's `sg13g2_pr` holds
xschem symbols under `libs.tech` and fixed layouts under `libs.ref`), the project lists the
xschem one and the scan records a note. A CDePT library is one folder.

### New views

A new view takes its defaults from the snapshot: a new GDS gets the database unit
`klayout.dbu`, a new Magic layout (`<cell>.mag`) gets `tech <magic.tech>`.

### When the snapshot is not used

The snapshot is *in use* only when it was made for the project's PDK. Otherwise it stays
loaded (the GUI can still show it) and `cdept info` says so:

```text
WARN tech.toml was made for the PDK "ihp-sg13g2" but the project uses "sky130A": it is not
used; run `cdept tech refresh` to make it again
snapshot     tech.toml  (not used: made for PDK ihp-sg13g2)
```

Other warnings at load: `$PDK_ROOT` is not set; the PDK is not in that folder (set
`pdk_root` in `cdept.local.toml`); the PDK on this machine is another version or commit
than the one scanned (`tech.toml is out of date`: run `cdept tech refresh`). A `tech.toml`
that cannot be read is a warning, not an error: the project opens without a snapshot.

## Hand edits and refresh

`cdept tech refresh` scans the PDK again, lists what changes (`~ klayout.tech: a -> b`,
`+ magic.rc: ...`) and then **replaces the whole file**. `--dry-run` stops after the list.
Hand edits of `tech.toml` are therefore lost:

```text
$ cdept tech refresh --dry-run
scanning /opt/ext/OpenPDKs/ihp-sg13g2 ...
~ models.corner: ff -> tt
```

Where a lasting change is needed, use `cdept.toml`: it wins over the snapshot and a
refresh never touches it. It has keys for the xschem start-up file, the `[klayout]`
settings (path, technology, layer properties, decks, device map) and the libraries. For
a PDK whose layout the scan does not understand, `tech.toml` may be written by hand: every
key is optional and paths may be absolute.

Not everything has a place in `cdept.toml` yet. Choosing another model corner or
disabling a PDK library is an edit of `tech.toml`, which a refresh resets.

## Hand-made and NDA PDKs

A PDK that is not open source can be used the same way: the scan only reads the files on
the machine and the snapshot holds names, paths and numbers. Read the snapshot before
committing it: layer names, device names and model corner names of an NDA PDK are in it.
Nothing checks this for you. CDePT ships no PDK data other than the device map of IHP
SG13G2.

A PDK that is not on a machine still gives a project: `cdept init --technology NAME
--no-scan` names the PDK in `cdept.toml` and warns until a scan is possible.

## See also

- [tech.toml and tech.catalog.json](../reference/files/tech-toml.md): the file formats
- [cdept.toml](../reference/files/cdept-toml.md) and
  [cdept.local.toml](../reference/files/cdept-local-toml.md)
- [Create a project for a PDK](../howto/new-project.md)
- [Run DRC, LVS and extraction](../howto/lvs-drc.md)
- [TECH.md](../TECH.md): design of the scan, per-section details and open points
- [LINKING.md](../LINKING.md): the device map and linking schematics with layouts
- [KLAYOUT.md](../KLAYOUT.md), [MAGIC.md](../MAGIC.md), [XSCHEM.md](../XSCHEM.md),
  [BATCH.md](../BATCH.md): the tools the snapshot configures
