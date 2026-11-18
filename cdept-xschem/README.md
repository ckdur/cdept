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
**Optional:** `libjpeg-dev` (found with pkg-config `libjpeg`; libjpeg-turbo's works).
With it, JPEG pictures embedded in schematics are drawn; without it they are left out
and the build prints a warning. Release, CI and package builds should install it.
The crate is not a default workspace member, so a plain `cargo build` does not need
them.

At run time xschem needs `awk` for netlisting.

## Install

```
scripts/install.sh [--prefix /usr/local] [--destdir D] [--no-build] [--no-helper]
```

builds `cdept` and the helper in release mode and installs, under `D/prefix`:

| Path | What |
|------|------|
| `bin/cdept` | the program (MIT) |
| `bin/cdept-xschem` | the helper (GPL-2.0-or-later). In `bin/`, not `libexec/`: `cdept` already looks next to its own executable, and it can also be started by `--version` and found on `PATH` for diagnosis |
| `share/cdept-xschem/` | what the helper reads at run time: xschem's Tcl and awk files, `systemlib`, `xschem_library/devices`, and `cdept/` (the Tk stand-in and CDePT's Tcl) |
| `share/doc/cdept/` | the licences |

The files do not depend on the prefix: the helper finds `../share/cdept-xschem` relative
to its executable, so an installed tree can be moved. `--destdir` stages for packagers
(nothing is recorded with it). `--no-helper` installs `cdept` alone. At run time the
helper needs `libtcl8.6`, `libcairo2` (and `libjpeg`), and `awk` (netlisting).
`cdept-xschem --print-sharedir` shows which share folder it uses.

Lookup order. `cdept` finds the helper through `$CDEPT_XSCHEM`, then next to its own
executable, then `PATH`. The helper finds the share folder through `--sharedir`
(an explicit one that is wrong is an error), `$CDEPT_XSCHEM_SHAREDIR`,
`<exe>/../share/cdept-xschem`, then the build tree (a symlink farm in cargo's `OUT_DIR`,
for development). The install script copies from that farm with the links resolved.

For the asic-all image the entry is `cdept` in `bouquet_assembler/tool_library.yaml`
(private repository; it has to be built inside the image, because a host build needs a
newer glibc).

## How it works

xschem normally draws through Xlib and builds its menus and dialogs with Tk. Here it
gets neither:

- **Drawing.** `shim/xshim.c` implements the Xlib calls xschem makes on cairo image
  surfaces. The drawing window is such a surface; after each batch of requests the
  rectangles that changed are copied into a file that CDePT maps into memory.
- **Widgets.** `shim/tcl/faketk.tcl` implements the Tk commands in plain Tcl. It
  records the widgets xschem's own Tcl code creates and reports them; CDePT draws them
  with its own widgets and reports back what the user does. `shim/tcl/cdept.tcl`
  replaces a few things with CDePT's own (file browser → library picker; a symbol
  made from a schematic goes into the cell's `symbol` view).
- **Loop.** The program is one thread running the Tcl event loop, with the socket as a
  file the loop watches. Requests are therefore also served while xschem waits for a
  dialog.

xschem's sources are not modified.

## Layout

| Path | What |
|------|------|
| `xschem/` | xschem sources, unmodified (git submodule, pinned to the commit the asic-all image builds) |
| `shim/include/` | Stand-ins for `<X11/*.h>`, `<tk.h>`, `<cairo-xlib.h>` and xschem's `config.h` |
| `shim/cdx.h` | The C functions the Rust side calls |
| `shim/engine.c` | Start-up, Tcl access and event loop; replaces xschem's `main.c` |
| `shim/xshim.c`, `shim/display.c` | Xlib on cairo, the Tk C functions, the drawing window |
| `shim/mi/`, `shim/xs_mi.h` | The X server's own line, arc and polygon code (X.Org, MIT-style licences), so that these come out as on a real X server; see `shim/mi/README.md` |
| `shim/tcl/faketk.tcl` | The Tk stand-in (`tkmin.tcl`: a minimal one, for debugging the C side) |
| `shim/tcl/cdept.tcl` | CDePT's adjustments to xschem's Tcl side |
| `shim/tcl/tests/` | Tests of the Tcl side (`run.sh`) |
| `src/main.rs` | The helper program: requests, frames, widget updates |
| `src/bin/cdx-tcl.rs` | Test tool: run Tcl in the engine without CDePT |
| `tests/` | End-to-end tests through `libcdept::xschem` (`Engine`, `Session`) |

How `cdept` finds the helper and the helper its runtime files: see Install above.

## Testing

```
cargo test -p cdept-xschem                                   # Rust, end to end
sh cdept-xschem/shim/tcl/tests/run.sh                        # Tk stand-in, no drawing
CDX_DISPLAY=900x600 sh cdept-xschem/shim/tcl/tests/run.sh    # with the drawing window
```

`cdx-tcl` runs Tcl in the engine and can write the drawing window to a PNG:

```
target/debug/cdx-tcl --display 1200x800 \
    -e 'xschem load /path/to/file.sch' -e 'xschem zoom_full' -e 'cdept::png out.png'
```

Widget updates are printed as `ui: <json>` lines. `CDEPT_XSCHEM_TK=tkmin` selects the
minimal Tk stand-in; `CDEPT_FAKETK_DEBUG=1` logs Tk features the stand-in lacks.

## Updating xschem

```
git -C cdept-xschem/xschem fetch && git -C cdept-xschem/xschem checkout <commit>
cargo test -p cdept-xschem
```

Then run the three test commands above.

- If xschem started to use an Xlib or Tk C function that the shim lacks, the build
  stops with an undeclared or undefined name: add it to `shim/include/xshim.h` (or
  `tk.h`) and `shim/xshim.c`.
- New source files must be added to `XSCHEM_SOURCES` in `build.rs` (compare with
  `xschem/src/Makefile.in`).
- If xschem's Tcl code started to use a Tk command or option the stand-in lacks, the
  sweep test (`t_sweep.tcl`) names the procedure that fails, and
  `CDEPT_FAKETK_DEBUG=1` names the feature.
- `shim/tcl/cdept.tcl` redefines a few of xschem's Tcl procedures
  (`load_file_dialog`, `file_chooser`, `open_sub_schematic`, `make_symbol`,
  `make_symbol_lcc`): check that their arguments did not change, and that
  `make_sym.awk` and `make_sym_lcc.awk` still write `x.sym` for the file `x.sch` they
  are given.
- It also redoes in Tcl what xschem's C function `create_sch_from_sym` (save.c; "Make
  schematic from symbol", Ctrl+L, `xschem make_sch`) writes, for symbols in `cells`
  libraries (`cdept::schematic_text`). If that function changed, change the Tcl copy
  to match: `t_cdept.tcl` compares the two outputs (`same-text-as-xschem`).

## Comparing with stock xschem

The drawing is meant to be the same as stock xschem's, pixel for pixel. To check, build
stock xschem from the same sources, run it on a private X server, and compare
pictures of the same schematic at the same window size. Nothing here touches your own
display or `~/.xschem`.

```sh
W=$(mktemp -d)                                  # a scratch folder
Xvnc :77 -geometry 2000x1400 -depth 24 -SecurityTypes None -localhost -nolisten tcp -ac &
rsync -a --exclude .git cdept-xschem/xschem/ $W/src/            # stock build, same commit
(cd $W/src && ./configure --prefix=$W/inst && make -j8 && make install)

cat > $W/cmp.sh <<'END'
#!/bin/bash
# cmp.sh NAME FILE WIDTH HEIGHT [TCL] [RCFILE]: stock and embedded picture of FILE,
# and the number of pixels that differ. TCL runs in both after loading.
W=$(cd "$(dirname "$0")" && pwd); name=$1; file=$2; w=$3; h=$4; tcl=${5:-}; rc=${6:-/dev/null}
mkdir -p $W/out $W/home1 $W/home2
cat > $W/out/$name.tcl <<TCL
update
for {set i 0} {\$i < 8} {incr i} {   ;# give the drawing area exactly w x h
  set dw [expr {$w - [winfo width .drw]}]; set dh [expr {$h - [winfo height .drw]}]
  if {\$dw == 0 && \$dh == 0} break
  wm geometry . [expr {[winfo width .] + \$dw}]x[expr {[winfo height .] + \$dh}]+0+0
  update; after 150; update
}
xschem load {$file}; update
$tcl
update; after 300; update
exec import -window [winfo id .drw] $W/out/$name.stock.png
exit
TCL
(cd $W/out && HOME=$W/home1 DISPLAY=:77 timeout 60 $W/inst/bin/xschem --rcfile $rc \
    --script $W/out/$name.tcl) >/dev/null 2>&1
(cd $W/out && HOME=$W/home2 timeout 60 $CDX --display ${w}x${h} --rcfile $rc \
    -e "xschem load {$file}" -e "$tcl" -e "cdept::png $W/out/$name.ours.png") >/dev/null 2>&1
echo "$name: $(compare -metric AE $W/out/$name.stock.png $W/out/$name.ours.png \
    $W/out/$name.diff.png 2>&1) pixels differ"
END
chmod +x $W/cmp.sh
export CDX=$PWD/target/debug/cdx-tcl
echo "append XSCHEM_LIBRARY_PATH :$PWD/examples/buffer/libs" > $W/rc
T=$PWD/examples/buffer/libs/bench/tb_buf/schematic/tb_buf.sch
$W/cmp.sh tb $T 947 614 "" $W/rc                               # as loaded
$W/cmp.sh thick $T 1000 700 'xschem zoom_box -20 -25 30 15' $W/rc
# a rubber band half-way through a drag; `xschem callback` is the same in both
$W/cmp.sh band $T 1000 700 'xschem callback .drw 4 50 40 0 1 0 0
  xschem callback .drw 6 150 100 0 0 0 256; xschem callback .drw 6 300 200 0 0 0 256' $W/rc
```

It needs `Xvnc` (TigerVNC), ImageMagick, and what stock xschem needs to build (Tk and
X11 development files). For xschem's own examples put every folder of
`xschem/xschem_library` on the path in the rc file and add
`set xschem_execute_scripts 0`, or loading stops at a question; for a PDK, `source` its
`xschemrc`.

Last result (xschem 3.4.8RC, cairo 1.18.4): 44 pictures, among them the example
project, nine of xschem's examples, four IHP SG13G2 testbenches, selections,
highlighted nets, the light colours, thick lines when zoomed in, a rubber band, a
wire and a move in progress. 39 are identical. The other five differ only inside
embedded bitmap pictures (photos, logos), by one step of a colour value in at most
0.06 % of the pixels: there the X server scales the picture, here cairo does.

Two things xschem does are easy to take for differences:

- **Full zoom can leave a long text partly outside the window.** xschem measures the
  texts of an instance when the file is loaded, at the zoom of that moment, and keeps
  the result; text does not scale evenly with the zoom (font hinting). Stock xschem
  does the same. `xschem update_all_sym_bboxes` followed by `xschem zoom_full` fits
  better. The helper measures again by itself for the first view of a window
  (`fit_view()` in `shim/display.c`); a full zoom asked for later (key `f`, loading
  another file in the same tab) is xschem's own.
- **Junction dots are smaller than in xschem's SVG export.** On screen they are the
  same as in stock xschem; the SVG export draws them larger.
