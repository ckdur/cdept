# Projects, libraries, cells and views

CDePT keeps a design as plain files in folders and adds one small TOML file that says
which folders are libraries. There is no database and no binary design format: the files
are the ones the tools read and write (xschem `.sch`, GDSII, SPICE, Verilog), and
everything CDePT knows about a project is read from them, so a project works with git
and with a text editor.

## The model

```text
project
  library          a named folder of cells
    cell           a design unit: an inverter, an OTA, a testbench
      view         one representation of the cell: schematic, symbol, layout, ...
        files
```

- A **project** is a folder with a [cdept.toml](../reference/files/cdept-toml.md). The file
  names the project and its libraries and says which PDK (technology) it uses.
- A **library** is a named folder of cells. The name is the one in `cdept.toml`, not the
  folder's name. A project has any number of libraries, in or outside its own folder.
- A **cell** is a design unit inside a library, with a name that is unique in the library.
- A **view** is one representation of a cell with a single purpose: its schematic, its
  symbol, its layout, a netlist, a testbench. A cell has at most one view of each name.

A view is addressed as `library/cell/view`, for example `logic/inv/layout`. This
library, cell and view idea is the one of other chip-design environments, but the files
and the project file are CDePT's own.

The project file itself is shown in `examples/buffer`:

```toml
name = "buffer"

[libraries]
logic = "libs/logic"
bench = "libs/bench"
```

```text
$ cdept ls
logic  .../libs/logic  [cells]
  buf                              layout schematic symbol
  inv                              layout schematic symbol
  rdiv                             layout
  rpoly                            pcell
bench  .../libs/bench  [cells]
  tb_buf                           schematic
```

## Views

A view is a folder inside the cell folder (or, in a flat library, a single file). Its
**kind** says what it is for, independent of the tool that edits it. Its **format** is
the format of its main file and decides which tool opens it.

### View kinds

| Kind | Typical content |
|---|---|
| `schematic` | xschem `.sch`, Qucs-S `.sch` |
| `symbol` | xschem `.sym` |
| `layout` | GDSII (`.gds`, `.gds.gz`), OASIS (`.oas`), Magic (`.mag`) |
| `netlist` | SPICE (`.spice`, `.spi`, `.sp`, `.cir`, `.ckt`), CDL (`.cdl`) |
| `testbench` | a folder with a `setup.toml`: a simulation set-up, see [Simulation](simulation.md) |
| `hdl` | Verilog (`.v`), SystemVerilog (`.sv`), VHDL (`.vhd`, `.vhdl`), Verilog-A (`.va`) |
| `abstract` | LEF (`.lef`); opens read-only in the embedded KLayout, see [KLAYOUT.md](../KLAYOUT.md) |
| `liberty` | Liberty (`.lib`) |
| `pcell` | a KLayout parametrised cell written in Python (`<cell>.pcell.py`), see [KLAYOUT.md](../KLAYOUT.md) |
| `config` | reserved name for configuration views |
| `other` | anything else |

### How a view is recognised

In a library with the `cells` layout every folder inside a cell folder is a view. The
view's name is the folder name. CDePT works out the rest from the files in it:

1. **Main file.** The design file named after the cell (`inv.sch` in `inv/schematic/`);
   when none is, the first design file in name order. The main file is what a tool
   opens.
2. **Format.** From the suffix of the main file, longest suffix first (`ota.gds.gz` is
   GDSII). A `.sch` file is a Qucs-S schematic when it starts with `<Qucs`, otherwise
   xschem. A project can change what a suffix means with
   [`[suffixes]`](../reference/files/cdept-toml.md#suffixes).
3. **Kind.** A folder with a `setup.toml` is a `testbench`. Otherwise the kind of the main
   file's format. A folder with no design file gets a kind from its name (`schematic`,
   `symbol`, `layout`, `netlist`, `config`, `hdl`, `abstract`, `liberty`, `pcell`, and
   names starting with `tb_` or `testbench`), else `other`.

A design file directly in the cell folder, without a view folder, is also a view: its
name is the kind (`schematic`, `layout`, ...); when that name is taken it is the kind with
the suffix (`layout_oas`). Files that are not design files (notes, images) are
not views. The `.json` sidecar of a layout is part of the layout view and is not listed
as a file of it; see [Project folders](../reference/files/project-folders.md#the-library-folders).
Hidden files and folders (names starting with `.`) are ignored.

Two view names have a meaning in the tools: the schematic view of a cell has a symbol
that names it through a `schematic=` attribute, which CDePT keeps right when cells are
copied or renamed ([XSCHEM.md](../XSCHEM.md) section 7), and a layout view has an optional
sidecar `<file>.json` with its references ([KLAYOUT.md](../KLAYOUT.md) section 3.2).
The programs that open each format are described in [External tools](../tools.md).

### Views that are one cell of a larger file

In flat and reference libraries a single file can hold many cells: the GDS, LEF and
Liberty files of a standard-cell library, a Verilog or SPICE library. CDePT reads what
is inside such a file (and keeps an index cache so that a large file is read once) and
lists each inner cell as a cell of the library, with the file as its view. These
**member views** cannot be renamed, copied or deleted on their own because the file
belongs to all the cells in it.

## Library layouts

A library definition has a `layout` that says how the folder maps to cells and views. It
is detected from the folder when left out.

| Layout | Folder shape | Used for |
|---|---|---|
| `cells` | `<library>/<cell>/<view>/<files>` | CDePT's own libraries. Every folder of the library is a cell, every folder of a cell a view. This is what new libraries get. |
| `flat` | `<library>/<cell>.<ext>` | Folders of design files: xschem symbol libraries, GDS folders, many PDK libraries. Files that share a name form a cell and each file is a view. No import is needed to use them in place. |
| `ref` | `<library>/<format>/<files>` | The reference libraries of a PDK (`libs.ref/<lib>/{gds,lef,lib,verilog,spice,cdl,mag,...}`): one folder per file format. CDePT only reads such a library. |

Detection: a `lib.toml` in the folder means `cells`; design files directly in the folder
mean `flat`; a folder that holds only format folders (`gds`, `lef`, `lib`, `verilog`,
`spice`, `cdl`, `mag`, `sym`, `sch`, ...) with design files in them means `ref`;
anything else is `cells`.

How the views are named in the other two layouts:

- In a **flat** library the view name is the kind of the file (`schematic`, `symbol`,
  `layout`, ...); a second view of the same kind gets the kind plus the suffix
  (`layout_oas`). `inv.sch`, `inv.sym` and `inv.gds` form the cell `inv` with the
  views `schematic`, `symbol` and `layout`.
- In a **reference** library the folder gives the name: the usual folder of a kind gives
  the kind (`gds` is `layout`, `lef` is `abstract`, `lib` is `liberty`, `verilog` is
  `hdl`, `spice` is `netlist`, `sym` is `symbol`, `sch` is `schematic`); another folder
  adds its name (`mag` is `layout_mag`, `cdl` is `netlist_cdl`).

CDePT creates, renames and deletes cells in `cells` and `flat` libraries. A flat library
has no empty cells: a cell exists once it has a view, and only files named after the cell
move with it.

The xschem side of the `cells` layout (`cell/schematic/cell.sch`, `cell/symbol/cell.sym`)
is explained in [XSCHEM.md](../XSCHEM.md) section 7.

### Read-only libraries

A library with `read_only = true`, and every `ref` library, is never changed by CDePT:
no new cells or views, no rename, copy-into, move or delete, no description edit, and
refactors do not rewrite files in it (see [Refactors](#refactors)). Use it for a PDK's
primitives. The libraries a PDK brings through
[tech.toml](../reference/files/tech-toml.md) are read-only and are added to the
project after its own libraries; a library of the project with the same name hides
them. Read-only libraries outside the project folder are watched only at their top
folder, since a PDK is large and does not change.

### Libraries that cannot be read

A library whose folder is missing or whose path uses an unset `$VAR` stays in the list
with an error, and the rest of the project loads. `cdept info` and the GUI show the
errors and warnings collected while loading.

## Names and case

CDePT creates library, cell and view names from letters, digits, `_`, `-` and `.`. A name
starts with a letter, a digit or `_`. A library name has no `.`. Existing folders with
other names are still listed.

Names are case sensitive inside CDePT, but not every file system is. CDePT warns when two
libraries or two cells of a writable library differ only by case, because they would be
one file on macOS and Windows. It also warns about two libraries whose folders have the
same last name, because xschem names a library by its folder and can then find only the
first, and about file names that are not valid UTF-8 (they are not shown).

Files named after the cell follow its name: renaming `inv` to `inv2` renames `inv.sch`
to `inv2.sch` in every view.

## Categories

A category is a named group of cells of one library, for browsing and searching.
Categories nest (`analog/amplifiers`) and a cell can be in several. They are stored in
the `lib.toml` of the library or, for read-only and flat libraries, in `categories.toml`
next to `cdept.toml`; IHP LibMan's `.group` files are read as well. The formats are
in [Category files](../reference/files/categories.md). A search finds cells by category
with `cat:NAME`, for example `cdept find cat:amplifiers kind:layout`.

## The reference index

CDePT knows what refers to what by reading the files, without starting a tool. For every
view of the project it reads:

| View | References found |
|---|---|
| xschem schematic and symbol | the symbols a schematic places; the schematic a symbol stands for |
| layout (GDSII, OASIS) | the cells of other views it places (resolved through the sidecar, else by name), the PCells it places, and the schematic it is linked with |
| SPICE and CDL netlist | the subcircuits it instantiates |
| Verilog and SystemVerilog | the modules it instantiates |

Each reference resolves to a view of the project, to a file outside it (xschem's own
devices) or stays unresolved. Read-only libraries are read too, but only text files up to
a size limit and none of their layouts.

Two questions are answered from the index:

- **Hierarchy**: what a cell is built from, per kind of view (`schematic`, `layout`,
  `netlist`, `hdl`). `cdept hier LIBRARY CELL [--view schematic|layout|netlist|hdl]`
  prints the tree; devices, pins and PCells appear with `--all`.
- **Used by**: which cells use a cell, through which views and how often. `cdept used-by
  LIBRARY CELL [--view V]`.

```text
$ cdept hier logic buf --view layout
buf
  inv ×2

$ cdept used-by logic inv
logic/buf/layout  placement ×2
logic/buf/schematic  instance ×2 (x1, x2)
```

Both accept `--json`. The GUI has the same two trees in the Library Manager.

## Refactors

Renaming, moving and copying are not just file operations: other views refer to the thing
that moves. CDePT plans each operation first and then applies it. The plan lists which
folders and files move or are copied, and which files are rewritten and how. A **dry
run** (`-n/--dry-run`) prints the plan and changes nothing.

| Operation | Command |
|---|---|
| Rename a cell | `cdept cell rename LIBRARY CELL NEW` |
| Move a cell to another library | `cdept cell move LIBRARY CELL TO_LIBRARY [--name NEW]` |
| Copy a cell | `cdept cell copy LIBRARY CELL NEW [--to LIBRARY] [--with-hierarchy]` |
| Delete a cell | `cdept cell rm LIBRARY CELL` |
| Create, copy, rename, delete a view | `cdept view new`, `copy`, `rename`, `rm` |
| Rename a library, also its folder | `cdept lib rename OLD NEW [--folder]` |

What follows a rename or a move:

- the symbol references in the schematics that place the cell, and the `schematic=`
  attribute of its symbols;
- in layouts: the sidecar entries of the views that place it, the structure name inside
  the GDSII file, the layout's own sidecar identity and top structure, the linked
  schematic and instances, and the `LIB=` and `PCELL=` records of placed PCells;
- in netlists and HDL: the `.subckt`, `.ends` and `module` names of the renamed cell and
  the instances of it in other views;
- the categories of the cell.

A **copy** rewrites only the copies: references between the copied files point at the
copies, everything else is left alone. `--with-hierarchy` also copies the cells of the
same library that the cell is built from. `--keep-references` moves or copies the files
only, and leaves every reference as it is.

Safety:

- **Rollback.** Applying keeps a journal of every step. If one fails, what was done is
  put back, in reverse order; the error says so, and lists what could not be put back
  if something could not.
- **Read-only libraries are never changed.** Their views that use the moved thing are
  listed as "not followed" in the plan: they will be wrong afterwards.
- **Unsaved editors block.** A file open in an editor with unsaved changes stops the
  operation that would move or rewrite it. Save or close it first.
- **Deleting lists its users first.** Deleting a cell or view prints what still uses it.

## Trash

`cdept cell rm` and `cdept view rm`, and delete in the Library Manager, move the folder to
the desktop's trash by default. The trash is the one of the freedesktop.org Trash
specification: `$XDG_DATA_HOME/Trash` (default `~/.local/share/Trash`), or, for a file on
another disk, the trash folder at the top of that disk. A `.trashinfo` record keeps
where the item was and when it went, so the file manager can put it back. `--permanent`
deletes for good. Removing a library definition (`cdept lib remove`) only edits
`cdept.toml`; the folder stays.

## Git status

When the libraries are in a git repository (the project's own, or a repository of the
library), CDePT shows what git says about each file, view, cell and library. It runs the
`git` program (`git status --porcelain=v2`) and links no git library. Outside a
repository, or without `git` on the path, there is no status and no error.

| Badge | State | Meaning |
|---|---|---|
| (none) | clean | tracked, as committed |
| `M` | modified | tracked, with changes not in the index (also deleted) |
| `S` | staged | tracked, all changes in the index |
| `A` | added | new, and added to the index |
| `?` | untracked | new, not known to git |
| `I` | ignored | left out by a `.gitignore` |
| `!` | conflicted | a merge left it in conflict |

A view, a cell and a library show the most urgent state of the files under them. In the
terminal: `cdept status [LIBRARY] [--all] [--json]`, which lists the cells that differ
from the commit (all of them with `--all`). Reference libraries are not asked. Which
project files to commit is in [Project folders](../reference/files/project-folders.md).

## Watching files

The GUI watches the project's files so that what changes on disk behind its back shows
up without Reload: the library folders with everything below them, `cdept.toml`, the files
it includes, `tech.toml`, `tech.catalog.json`, `cdept.local.toml`, and the index and
HEAD of the git repositories the libraries are in (a commit, a checkout or `git add`
refreshes the badges). A change is reported once the files have been quiet for 300 ms
(and no later than 2 s after the first event), so a tool that writes a view in several
steps gives one change. A changed library folder is scanned again; a changed project
file loads the project again. A file with unsaved changes in an editor is not
overwritten: the editor asks whether to reload or keep the edit.

Editor backups, swap and lock files (`*~`, `.#*`, `*.swp`, `*.tmp`, `*.lock`, ...) are
ignored. The command line has no watcher: each command reads the project afresh.

## See also

- [cdept.toml](../reference/files/cdept-toml.md): library definitions
- [Project folders](../reference/files/project-folders.md): the on-disk tree
- [Category files](../reference/files/categories.md)
- [Technology](technology.md) and [Simulation](simulation.md)
- [XSCHEM.md](../XSCHEM.md), [KLAYOUT.md](../KLAYOUT.md), [LINKING.md](../LINKING.md): the view formats
- [NOTES.md](../NOTES.md), [ROADMAP.md](../ROADMAP.md): design notes
