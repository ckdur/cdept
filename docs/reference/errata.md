# Known issues and surprises

Behaviour that a user is likely not to expect, and the gaps between what a page of the
design notes says and what the program does. Each entry says what happens, why it
matters and what to do. Where the program ought to change, the entry says so; the
manual then describes the behaviour that exists today.

## Configuration files

Unknown keys are not errors in `cdept.toml` and `tech.toml`
: A misspelt key in the project file, in `[xschem]`, `[klayout]` or in `tech.toml` is
  ignored without a message. A misspelt key inside a library table gives a warning and
  the whole library is dropped from the project. `cdept.local.toml` warns. Only
  `setup.toml` rejects unknown keys. Check a new file with `cdept info` and
  `cdept lib show <name>`. See [cdept.toml](files/cdept-toml.md).

Library names are checked on creation, not on load
: `cdept lib add`, renames and new cells refuse bad names; a name written by hand in
  `cdept.toml` is accepted as it is.

Generated folders are not ignored by git
: CDePT adds only `cdept.local.toml` to `.gitignore`. `simulations/`, `magic/` and
  `.cdept/` are left for you to decide ([project folders](files/project-folders.md)).

`tech refresh` replaces hand edits
: `tech.toml` is read as it is, so hand edits apply, but a refresh (and `tech scan` over
  an existing file, and Rescan in the GUI) replaces the whole file. It says so, shows the
  differences and asks first (`--yes` skips the question; without a terminal it is
  required). Corner sets added by hand (the `mc_mm` set of the `ihp-inv` example) are
  lost when you agree; the old file is kept as `tech.toml.bak` to copy them back from.
  The differences list the PDK's changes and your edits together: the file does not
  record which is which.

`tech.toml` library `enabled = false` has no counterpart in `cdept.toml`
: An error message about a PDK library suggests `enabled = false` in the project file,
  but `cdept.toml` has no such key. Leave a library out in the Technology tab or in
  `tech.toml`.

Layouts: three exist, the template names two
: `layout = "ref"` is valid and detected automatically; the `cdept.toml` template and
  older notes mention only `cells` and `flat`.

## Simulation

Numbers in `[variables]` and `[options]` must be strings
: `vdd = "1.2"` works, `vdd = 1.2` is a parse error. `[template_args]` takes numbers.
  See [setup.toml](files/setup-toml.md).

`sim results --reindex` forgets history
: It rebuilds `results.db` from the `result.json` files, one per run point, so runs that
  were overwritten on disk are gone from the database. See [results](files/results.md).

Where the results go
: Runs write to `<project>/simulations/<lib>/<cell>/<view>/<point>/`. Older notes
  (`ROADMAP.md`) say `results/<run-id>/` inside the view; that is no longer so.

Monte Carlo and sweeps from a template
: A template cannot declare them, but a `setup.toml` that uses a template can add
  `[montecarlo]` and `[[sweep]]` itself, and they are merged in. `TESTS.md` says they
  cannot.

Xyce, Monte Carlo and mismatch
: Xyce does process variation only (the PDK's Xyce models have no mismatch sections),
  and a Xyce run cannot be cancelled once started. Its random draws are not the same
  as ngspice's for the same seed: the two simulators give different, equally valid
  samples. See [other simulators](../howto/other-simulators.md).

VACASK
: Monte Carlo is refused. The PDK's models convert for IHP SG13G2 but not for sky130A;
  devices such as `bondpad` and the varicap are not converted. Needs OpenVAF on first
  use.

Fractional temperatures in folder names
: `27.5` becomes the folder suffix `27p5`.

## Tools and environment

`cdept tools` shows an error as a version
: A tool whose probe fails (VACASK without its shared library, LibreLane's Python
  traceback) shows the error text in the version column. The tool is then not usable.

`ciel`, git and LibMan have no override variable
: They are taken from `PATH` only. The other tools have a `CDEPT_*` variable
  ([environment](environment.md)).

`CDEPT_PROJECT` is not read from the process environment
: It is expanded only inside paths of `tech.toml`.

`cdept --version` and `cdept version` differ
: The first prints the crate version; the second adds the commit and a `*` for a
  modified tree.

DRC and LVS results are not stored in the project
: `cdept layout drc|lvs` write to `~/.cache/cdept/verify/`, and the index cache is in
  `~/.cache/cdept/index`: both are per user. The Magic outputs go to `<project>/magic/`.
  DRC results from Magic do not reach the GUI. See [DRC and LVS](../howto/lvs-drc.md).

netgen on a KLayout-extracted netlist of sky130
: The IHP netlist of KLayout is accepted by netgen (CDePT writes the devices as calls, as
  the PDK does). The sky130 deck adds a substrate pin of its own to a cell without a tap
  (`sky130_gnd`) and scales sizes by 1e6, so netgen reports `PinMismatch` or property
  errors: use the netlist extracted by Magic. `cdept netgen lvs` says so.

`.cdept/` holds only compiled OSDI models
: Nothing else of the project's state lives there.

## GUI

The Simulation tab is one scroll area
: On a window of 900 px the Jobs list and the results sit below a long setup form.
  Make the window taller or scroll.

The Console shows raw Tcl text after an xschem selection
: Lines such as `properties: name=x1 pinlist: ...` come from xschem's Tcl and are not
  an error.

Open files changed outside CDePT
: Edits made within about 0.3 to 2 seconds of a change on disk by another program can
  be lost when the tab reloads; use Keep Mine when the prompt appears.

Quick-open shortcuts only work in the main window
: Ctrl+P is xschem's add-pin; quick open is Ctrl+Shift+F.

There is no calculator panel or waveform plot yet
: Results are tables and statistics; waveforms are read with the
  [calculator](../CALC.md) on the command line or in an external viewer.

## Documentation of the design notes

The notes in the top-level `docs/*.md` pages were written while the code was being
built and a few parts are out of date:

- `SIMULATION.md` has repeated status paragraphs at the top and two sections numbered 8;
  its list of simulators omits `vacask`, and `schematic_kind` accepts `xschem` and
  `qucs`.
- `NOTES.md` lists `cdept cell` and `cdept view` as missing; both exist.
- `LINKING.md` §3 does not describe the `um` unit of `@{x|um}` or the `terminals` key
  of `[[device]]`; see [tech.toml](files/tech-toml.md).
- `TECH.md` says an unreadable `tech.toml` is a warning; a wrong `schema` or a higher
  `version` warns and the project opens without the snapshot.

## See also

- [Environment variables](environment.md)
- [Project roadmap and notes](../ROADMAP.md)
