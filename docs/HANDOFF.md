# Handoff: end of 2026-11-22 (Phase 1 + Phase 2 round)

State: `main`, everything committed. Build, clippy and all tests green; all 18 GUI scripts
pass (`cdept-gui/scripts/run.sh`, with `PDK_ROOT=/opt/ext/OpenPDKs`; the ihp-inv ones need
`CDEPT_EXAMPLE=ihp-inv`). Helper suites (KLayout, xschem Tcl) pass.

## What landed (nine parallel forks, all merged)
- **Technology snapshot** (`libcdept/src/tech/`, design in `docs/TECH.md`): a PDK scan writes
  `tech.toml` (small, editable) + `tech.catalog.json` (bulky lists) next to `cdept.toml`.
  Sections: klayout, magic, netgen, xschem, models (corner sets), devices, refs.
  `cdept init --technology <pdk>`, `cdept tech scan|show|refresh|notes|devices`,
  `cdept pdk ls|info|versions|enable`. Per-user `cdept.local.toml` (git-ignored).
  Precedence: cdept.toml > tech.toml > auto-detect. ihp-sg13g2 and sky130A scan in ~1.4 s.
- **GUI**: New Project wizard (PDK pick, scan, summary), Technology tab, categories, search,
  import, hierarchy/used-by, rename/move/copy plans, git badges, file watching, trash.
- **Libraries**: `ref` layout for libs.ref, multi-cell files as cells, index cache.
- **Core**: reference index + refactor with rollback (`hierarchy.rs`, `refactor.rs`),
  library editing, path field without portal, watch/vcs/trash.

## Last-minute fixes by me (after the forks)
Member (collection) cells are refused by refactor; categories follow rename/delete/move;
shadowed-folder and case warnings skip read-only PDK libraries (no more sky130_fd_io warning).

## Open (see docs/NOTES.md for older items; NOT yet merged into NOTES/ROADMAP/TECH.md)
- Merge the forks' NOTES blocks into docs/NOTES.md, tick ROADMAP Phase 1/2; fill docs/TECH.md
  §3.1–3.8 (stubs) from section fields (read `libcdept/src/tech/*.rs`).
- Decide: commit `examples/ihp-inv/tech.catalog.json` (1 MB)? PDK commit pinned in tech.toml
  gives "out of date" warnings; NDA PDK snapshots not checked before commit.
- `tech refresh` overwrites hand edits to tech.toml; enabling a PDK library has no cdept.toml key.
- Library is one folder: libs.ref/sg13g2_pr and sky130_fd_pr left out beside xschem namesakes.
- Deck switches: `klayout::verify` still passes IHP variable names only; catalog has roles.
- Default-deck order implemented twice (tech::klayout::decks vs klayout::verify::best_deck).
- Dead code to remove: old New Project folder flow (`PickFor::NewProject`), `Library::rename_cell/
  copy_cell/rename_cell_following`; `tech::scan` has no progress hook (pdkscan mirrors it).
- Not exercised: `ciel` program, Open in LibMan, native-dialog fallback without portal,
  gf180mcu/ihp-sg13cmos5l (not installed), CI klayout job.
- Hierarchy gaps: OASIS PCells, instance-level schematic=, read-only layouts not indexed,
  multi-cell GDS not used by the index.
- Watch: auto-reload can drop edits made within 0.3–2 s of an outside change.
- Quick-open shortcuts only in the main window (Ctrl+P is xschem's add-pin; Ctrl+Shift+F).
- Never read /opt/ext/OpenPDKs/XFAB (NDA); discovery only stats its subfolders.

## Next
Finish the doc merge, then Phase 3 leftovers (Magic, ngspice in-process) / Phase 4 (testbench,
corners from `models`, job queue) — the models API (`ModelsTech::lines`) is ready for it.
