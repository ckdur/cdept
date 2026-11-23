# Magic in batch mode

Code: `libcdept/src/magic/` (`mod.rs` the runner, `tcl.rs` the scripts, `parse.rs` the
readers), CLI: `cdept magic …` (`cdept/src/cmd_magic.rs`). Magic is GPL: it only runs
as a process of its own.

## How it runs

`magic -dnull -noconsole -rcfile <rc>` with the Tcl on stdin, in the folder of the job,
with `Project::tool_env()` (`PDK_ROOT`, `PDK`, `PDKPATH`, local `[env]`).

- Program: `$CDEPT_MAGIC`, else `magic` on `PATH` (`Magic::find`).
- Start-up file: `[magic] rc` of the technology snapshot, else
  `$PDK_ROOT/$PDK/libs.tech/magic/$PDK.magicrc` (`magic::rc_file`).
- Every script is wrapped in `catch`: an error prints `CDEPT: error <msg>` and quits;
  the end prints `CDEPT: done`. Results are other `CDEPT:` lines. A run without
  `done`, a failing exit status or an error is a Rust error that carries the last 25
  lines of Magic's output.
- Time limit: a run that takes longer than `[magic] timeout` of `cdept.toml` (default
  `15m`; `0` is none; `--timeout` on `extract`, `drc` and `lef` overrides it) is killed,
  and the error says so and has the last 25 lines of output (`Magic::timeout`,
  `Magic::with_timeout`). From a job of the queue, `Magic::in_job(ctx)` kills Magic when
  the job is cancelled (`Magic::cancel`), with the same kind of error. The output is read
  while Magic runs, so the lines printed before a hang are kept.
- Input (`magic::Source`): the `.mag` view of the cell (its folder joins the search
  path), else a cell of a GDS view (`gds read`, then `load`). A cell that is not in
  the GDS is an error.

## Operations

| CLI | API | Output in `<project>/magic/<lib>/<cell>/` |
| --- | --- | --- |
| `cdept magic extract <lib> <cell> [--style S] [--pex none\|c]` | `Magic::extract` | `<cell>.spice`, `.ext` files; parsed: subcircuits with ports, devices by model |
| `cdept magic drc <lib> <cell> [--style drc(full)]` | `Magic::drc` | `<cell>.drc.json`; violations by rule id (`met1.2`), boxes in µm of the top cell; exit status 1 when not clean |
| `cdept magic lef <lib> <cell> [--hide]` | `Magic::lef` | `<cell>.lef`; parsed: macro, size, pins |
| (API only) | `Magic::import_gds` | every cell of a GDS as `<cell>.mag` |

Extraction uses `ext2spice lvs` (devices and connectivity); `--pex c` adds
capacitances. DRC runs `drc catchup` and `drc listall why`; without `--style` the
technology's default style (usually `drc(fast)`) is used.

## Tests

Unit tests cover the Tcl and the parsers with canned text. The `real_*` tests run
Magic on the sky130A and ihp-sg13g2 inverters when Magic (`$CDEPT_MAGIC`, `PATH`,
`/opt/ext/OpenLane/bin/magic`) and the PDK in `/opt/ext/OpenPDKs` exist, and skip
otherwise.

## Open

- Resistive extraction (`extresist`, RC), hierarchical or per-subcell options.
- LVS (netgen) and the Magic-based `--style` choice from the snapshot's `drc` list.
- DRC results do not yet go to the GUI or link back to layout markers.
- `import_gds` has no CLI command.
