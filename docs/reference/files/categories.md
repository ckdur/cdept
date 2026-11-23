# Category files

A category is a named group of the cells of one library, for browsing and searching
(`cat:amplifiers` in [find](../cli.md), the category tree of the Library Manager). A cell
can be in several categories, and categories nest: `analog/amplifiers` is the category
`amplifiers` inside `analog`. A category holds the cells listed for it; when browsing,
it also shows the cells of the categories below it.

Categories are CDePT's own plain TOML. They change nothing on disk about the cells
themselves. A cell name in a category file that does not exist (any more) stays in the
file and is not shown.

## Where categories live

| Store | File | Used for |
|---|---|---|
| The library | `[categories]` in the library's `lib.toml` | Libraries CDePT may write that have view folders (`cells` layout). The categories travel with the library. |
| The project | `categories.toml` next to `cdept.toml`, one table per library | Read-only libraries (a PDK's) and `flat` libraries. A `lib.toml` dropped into a folder of design files would make it look like a `cells` library. |
| IHP LibMan | `<library>/<name>.group` | Read, never written. |

Both of CDePT's files are read for every library and the results are joined. Changes
go to one of them: `lib.toml` when the library is not read-only, has no error, and either
has a `lib.toml` already or uses the `cells` layout; `categories.toml` otherwise.
`cdept category ls` says which file a library uses.

### In `lib.toml`

```toml
[categories]
"analog/amplifiers" = ["ota5t", "ota_folded"]
digital = ["buf", "inv"]
```

The table is `[categories]`; each key is a category path and its value is a list of cell
names. The rest of `lib.toml` is described in
[Project folders](project-folders.md#lib-toml).

### In `categories.toml`

One table per library, named like the library in `cdept.toml`:

```toml
# CDePT: categories of cells, for the libraries that cannot keep them in
# their own lib.toml (read-only and flat libraries). One table per library:
#   [library]
#   "analog/amplifiers" = ["ota5t"]

[sg13g2_pr]
mos = ["sg13_lv_nmos", "sg13_lv_pmos"]
```

The file does not exist until a first category is written for such a library. When the
last category of a library is removed, its table is removed.

### LibMan `.group` files

IHP LibMan keeps a category as a file `<library>/<name>.group` in the library folder:
the cell names, separated by white space. The file name without `.group` is the
category name. CDePT reads these files for browsing and
searching, but never changes them. A category that exists only in a `.group` file cannot
be changed by CDePT until `cdept category adopt LIBRARY` copies the LibMan categories into
CDePT's own files.

## Names

A category name is a path of parts separated by `/`. White space around a part is
removed (`analog / amplifiers` is `analog/amplifiers`). A part cannot be empty, and
control characters and `"` are not allowed. Names are case sensitive. A name that
cannot be used is a warning when read and is ignored. A category whose value is not a
list of strings is a warning too.

Long lists (more than six cells) are written one cell per line, so that edits diff
well. Writes go through a temporary file renamed into place.

## Following cell changes

Renaming a cell with `cdept cell rename` or in the GUI renames it in the categories
too, and deleting a cell removes it from them. A cell moved to another library leaves its categories
behind (it starts in the new library with none); a copy does not bring them either.

## Commands

| Command | Does |
|---|---|
| `cdept category ls [LIBRARY]` | Shows the category tree with cell counts and where each library keeps them. |
| `cdept category add LIBRARY CATEGORY [CELL...]` | Puts cells into a category (made when new). Without cells it makes an empty category. |
| `cdept category rm LIBRARY CATEGORY [CELL...]` | Takes cells out; without cells removes the category and the ones below it. The cells stay in the library. |
| `cdept category rename LIBRARY CATEGORY NEW` | Renames or moves a category with the ones below it. |
| `cdept category of LIBRARY CELL` | Shows the categories of a cell. |
| `cdept category suggest LIBRARY [--write]` | Proposes categories: by the `type` attribute of the xschem symbols (`nmos`, `resistor`, `subcircuit`, ...) or, for standard-cell libraries, by function (`and`, `nand`, `dff`, ...). |
| `cdept category adopt LIBRARY` | Copies LibMan's `.group` categories into CDePT's files. |

The full option lists are in the [command reference](../cli.md).

## See also

- [Project folders](project-folders.md): `lib.toml` and the rest of the on-disk layout
- [Projects, libraries, cells and views](../../concepts/projects.md#categories)
- [cdept.toml](cdept-toml.md)
