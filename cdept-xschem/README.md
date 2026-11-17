# cdept-xschem

Helper process that gives CDePT an embedded [xschem](https://github.com/StefanSchippers/xschem):
xschem's own C code, built from the git submodule `xschem/`, behind a socket protocol.
CDePT starts it when needed; it is not meant to be run by hand.

Plan and status: [`docs/XSCHEM.md`](../docs/XSCHEM.md).

## Licence

xschem is GPL-2.0-or-later, and this program contains it, so **this crate and the
`cdept-xschem` program are GPL-2.0-or-later** (see `LICENSE`). The rest of the workspace
(`libcdept`, `cdept-gui`, `cdept`) is MIT. It does not link this crate: it starts the
program and exchanges messages with it (`libcdept::xschem::protocol`).

## Build

```
git submodule update --init
cargo build -p cdept-xschem
```

Needs a C compiler, flex, bison and the development files of Tcl and cairo
(Debian/Ubuntu: `sudo apt install build-essential flex bison tcl-dev libcairo2-dev`).
The crate is not a default workspace member, so a plain `cargo build` does not need
them.

At run time xschem needs `awk` for netlisting.

## Layout

| Path | What |
|------|------|
| `xschem/` | xschem sources, unmodified (git submodule, pinned to the commit the asic-all image builds) |
| `shim/include/` | Stand-ins for `<X11/*.h>`, `<tk.h>`, `<cairo-xlib.h>` and xschem's `config.h` |
| `shim/xshim.c` | The Xlib and Tk calls xschem makes. Placeholders until drawing is implemented |
| `shim/engine.c` | Start-up and Tcl access; replaces xschem's `main.c` |
| `src/` | The program: socket protocol loop around the engine |
| `tests/` | End-to-end tests through `libcdept::xschem::Engine` |

`cdept` finds the helper through `$CDEPT_XSCHEM`, next to its own executable, or on
`PATH`. The helper finds xschem's runtime files (`xschem.tcl`, awk scripts,
`xschem_library/devices`) through `--sharedir`, `$CDEPT_XSCHEM_SHAREDIR`,
`../share/cdept-xschem` next to its executable, or the build tree.

## Updating xschem

```
git -C cdept-xschem/xschem fetch && git -C cdept-xschem/xschem checkout <commit>
cargo test -p cdept-xschem
```

If xschem started to use an Xlib or Tk function that the shim lacks, the build stops
with an undeclared or undefined name: add it to `shim/include/xshim.h` (or `tk.h`) and
`shim/xshim.c`. New source files must be added to `XSCHEM_SOURCES` in `build.rs`
(compare with `xschem/src/Makefile.in`).
