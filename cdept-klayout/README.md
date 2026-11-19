# cdept-klayout: KLayout's layout editor for CDePT

A helper program that runs KLayout's editor without a window and talks to CDePT over a
socket, so a layout view opens inside a CDePT tab. Plan and protocol:
[`docs/KLAYOUT.md`](../docs/KLAYOUT.md).

**Licence.** KLayout is GPL-3.0-or-later, and so is everything in this folder (the
helper, its tests, the build script). CDePT itself (`libcdept`, `cdept-gui`, `cdept`)
is MIT: it never links KLayout, it only starts this program.

```
klayout/                  submodule, KLayout v0.30.12 (unmodified)
build.sh                  builds it into .build/ and .bin/ (not in git)
helper/cdept_klayout/     the helper, a Python package run by KLayout's interpreter
  main.py                 start-up script (what -r runs)
  server.py               socket, requests, the main loop
  editor.py               LayoutView, frames, input, layer/cell/selection events
  document.py             references between views: ghost cells filled, saved back
tests/                    protocol tests (plain python3 drives the helper)
```

## Build KLayout

```sh
git submodule update --init cdept-klayout/klayout
cdept-klayout/build.sh            # 20–40 min the first time; incremental afterwards
cdept-klayout/.bin/klayout -v     # KLayout 0.30.12
```

That runs, in `klayout/`:
`./build.sh -qmake qmake -python python3 -ruby ruby -build ../.build -bin ../.bin -option -j<cores>`.
Qt5 is the default. For Qt6 set `QMAKE=qmake6` (needs `qt6-5compat-dev` and
`qt6-multimedia-dev`); without QtMultimedia add `EXTRA=-without-qt-multimedia`
(KLayout's base application does not use it). The binary finds its libraries through
its rpath (`$ORIGIN/../.bin`): no `LD_LIBRARY_PATH` needed.

The full Qt build is used on purpose: the helper needs a `QApplication` (below), PDK
autorun macros register their PCell libraries only in the application, and the same
binary runs DRC/LVS decks (Ruby) and serves as "Open in KLayout".

## Run

CDePT starts the helper; by hand it is:

```sh
QT_QPA_PLATFORM=offscreen KLAYOUT_HOME=<private folder> \
  cdept-klayout/.bin/klayout -z -nc -e -r cdept-klayout/helper/cdept_klayout/main.py \
    -rd connect=<unix socket> [-rd frame_dir=<folder> -rd display=1]
```

- The helper **connects** to `connect` (CDePT listens), sends `ready`, then serves
  JSON lines until `shutdown` or until the socket closes. `-rd name=value` arrive in
  the script as global variables.
- `display=1`: frames are written into `frame_dir` (one file per size, B G R A) and
  announced with `frame`/`damage`. Without it only calls work (open, save, export,
  render, info, python): that is how previews and `cdept layout export` run.
- `-z` (hidden main window, so there is a `QApplication`) and not `-zz`: KLayout's edit
  code builds Qt dialogs in some places (copy asks "shallow or deep?"), which aborts
  the program without a `QApplication`. `QT_QPA_PLATFORM=offscreen` keeps it off the
  screen; a timer closes any modal dialog that still opens, and reports a notice.
  Copying is set to "shallow" (`edit-hier-copy-mode = 0`) so it never asks.
- `-e`: editable mode (without it `cm_copy` refuses). `-nc`: the user's `klayoutrc` is
  not read. Autorun macros still run, so set `KLAYOUT_PATH` to the PDK's
  `libs.tech/klayout` for its technology and PCells.
- `KLAYOUT_HOME`: recommended to point at a private folder (e.g.
  `~/.cache/cdept/klayout`), otherwise the user's salt packages autorun inside every
  helper — `klive`, for one, opens port 8082 and clashes with the user's own KLayout.
- Start-up takes about 1 s. Frame time from input to `damage`: about 17–20 ms at
  1920×1080, 75–85 ms at 3840×2160 (whole frame redrawn and copied every time).
- Clicks select top-level objects only (`edit-top-level-selection`), so a click on a
  placed cell picks the instance. Objects inside cells of other views are unselected
  again (with a warning) if that is switched off; delete and cut refuse them.
- "modified": every change KLayout flags counts one step, undo/redo step back and
  forth, so undoing back to the saved state reports clean. KLayout's flag is reset
  by letting the view save nothing to a scratch file (0.2 ms); a `python` call making
  several undo steps at once counts as one.
- `CDEPT_KLAYOUT_DEBUG=1` prints Python tracebacks of failed calls to stderr.

The `klayout` Python module from PyPI (no Qt) should also run the helper (it has
`LayoutView.timer()`, which the loop calls), but that is untested.

## Test

```sh
cdept-klayout/tests/run.sh             # all; or: tests/run.sh Headless / Display
```

Uses `.bin/klayout`, else `klayout` on `PATH`, else the asic-all docker image
(`CDEPT_TEST_DOCKER=<image>` to choose, `CDEPT_TEST_KLAYOUT=<binary>` to force one).
The display tests print the frame latency at two sizes and the start-up time.

## Update the submodule

```sh
cd cdept-klayout/klayout && git fetch --tags && git checkout v0.30.x && cd ../..
cdept-klayout/build.sh && cdept-klayout/tests/run.sh
```

Pin it to the version the asic-all image builds (`klayout:` in its `asic-all.yaml`)
so both write the same files.
