# CDePT: Roadmap

Goal: one open-source environment that does for open PDKs what Cadence Virtuoso does for
commercial ones. It does **not** replace xschem/KLayout/Magic/ngspice. It **orchestrates**
them: one project and library model, one technology setup, one simulation environment, one
waveform viewer and calculator, one signoff dashboard, with schematic, layout and simulation
kept in sync.

Working notes (open to-do items and known trouble spots): [NOTES.md](NOTES.md).

Source of scope: NLnet proposal "Chip Design Project Tool (CDePT)". Budget: ~750 h
(15 h/week × 50 weeks).

---

## 0. Virtuoso feature map → what CDePT provides

| Virtuoso component              | CDePT equivalent                         | Backed by (external)                 |
|---------------------------------|----------------------------------------|--------------------------------------|
| cds.lib / Library Manager       | `[libraries]` in `cdept.toml` + Library Manager (lib/cell/view) | filesystem + git |
| Technology file / PDK setup     | Technology Manager (open_pdks aware)   | sky130, gf180mcu, ihp-sg13g2/sg13cmos5l, ciel |
| Virtuoso Schematic Editor       | Launch + control                       | xschem (primary), Qucs-S             |
| Virtuoso Layout Suite L/XL      | Launch + control, cross-probe, SDL     | KLayout (primary), Magic             |
| ADE L/XL/Assembler/Explorer     | Simulation Environment                 | ngspice (shared lib), Xyce, VACASK, OpenVAF |
| ViVA waveform + Calculator      | Waveform Viewer + Calculator           | UberWave (separate viewer, launched), Surfer; calculator in CDePT |
| Assura/PVS/Calibre DRC/LVS/PEX  | Signoff dashboard                      | KLayout DRC/LVS, Magic, netgen, CVC, KLayout-PEX |
| Abstract Generator / Liberate   | Mixed-signal utilities                 | Magic LEF, lctime/CharLib, OpenSTA   |
| AMS Designer                    | Co-simulation                          | ngspice XSPICE d_cosim + Verilator/Icarus, cocotb + spicebind |
| Innovus handoff                 | Digital flow hook                      | OpenROAD / LibreLane, yosys          |

---

## 1. Architecture (decide in Phase 0, keep stable)

```
cdept (GUI binary, egui/eframe)
 └── libcdept (core, no GUI deps where possible)
      ├── project/     lib-cell-view model, cdept.toml (project + library definitions, TOML)
      ├── tech/        PDK discovery + technology description
      ├── tools/       ToolAdapter trait, process supervisor, IPC clients
      │    ├── xschem   (helper process cdept-xschem, see XSCHEM.md)
      │    ├── klayout  (helper process cdept-klayout, see KLAYOUT.md)
      │    ├── magic    (Tcl over stdin/pipe or Tcl socket script)
      │    ├── ngspice  (libngspice shared API, in-process)
      │    ├── uberwave (launched with raw files, libcdept/src/wave/; IPC client, families and calculator traces need the viewer's `ipc-multifile` branch)
      │    └── xyce / vacask / qucs-s / netgen / cvc / yosys / openroad / … (batch)
      ├── sim/         testbench, design variables, corners, sweeps, job queue, results DB
      ├── wave/        SPICE raw reader, VCD/FST via wellen, signal model
      ├── calc/        waveform calculator (expression language + DSP/measurement library)
      ├── signoff/     DRC/LVS/PEX runners + report parsers
      └── ms/          LEF/abstract, Liberty characterisation, netlist conversion
```

Suggested split: `libcdept` = core with no GUI, `cdept-gui` = egui frontend, `cdept-cli` = headless
(`cdept sim run`, `cdept lvs`, useful for CI and regressions). Each external tool is one
`ToolAdapter` with methods like `capabilities()`, `open(view)`, `netlist(cell)`,
`highlight(net)` and `on_event()`, so new tools plug in without touching the core.

### On-disk model (the lib/cell/view idea, in plain files)
The model is library → cell → view, but the files are our own: there is no `cds.lib`
clone. One hand-editable TOML file, `cdept.toml`, names the project and defines its
libraries. CDePT edits it in place and keeps comments and formatting.

```toml
# myproject/cdept.toml
name = "sar_adc"
description = "10-bit SAR ADC"
technology = "ihp-sg13g2"                      # the PDK name ($PDK); optional pdk_root = "…" is $PDK_ROOT

include = ["../shared/libraries.toml"]         # more definitions (same keys), optional

[libraries]
analog = "libs/analog"                         # short form: name = "folder"
sg13g2_pr = { path = "$PDK_ROOT/ihp-sg13g2/libs.tech/xschem/sg13g2_pr", read_only = true }
```

- Paths are relative to the file that contains them. `$VAR`, `${VAR}` and a leading `~`
  are expanded when loading and kept as written when saving.
- `read_only = true` marks reference libraries (PDK primitives): CDePT never modifies them.
- `layout` says how a library folder maps to cells and views. It is detected from the
  folder when left out.
  - `"cells"` (default for new libraries): `<library>/<cell>/<view>/<files>`. A design
    file directly in the cell folder is a view too.
    An xschem symbol names the schematic view of its cell with a `schematic=`
    attribute, which CDePT writes and keeps right (XSCHEM.md §7).
  - `"flat"`: `<library>/<cell>.<ext>`, files sharing a name form a cell and each file is
    a view. xschem symbol libraries, GDS folders and PDK libraries work in place, with
    no import step.
- A library that cannot be resolved (unset variable, missing folder) stays listed with
  its error instead of failing the whole project.

```
myproject/
  cdept.toml                 # project + library definitions
  libs/analog/
    lib.toml                 # optional library metadata (description; tech binding later)
    ota5t/                   # cell
      schematic/ota5t.sch    # view: xschem
      symbol/ota5t.sym
      layout/ota5t.gds       # or .mag
      tb_ac/                 # view: testbench (sim setup + results), Phase 4
        setup.toml
        results/<run-id>/
```
The kind of a view (schematic, symbol, layout, netlist, testbench, config, hdl, abstract,
liberty) comes from its main file's format, which also selects the tool that opens it.
Plain text, diff-able, git-friendly. That is a real advantage over Virtuoso's binary OA.

---

## 2. Phases

Effort is in hours, out of ~750. Each phase ends with a demo-able milestone (NLnet pays per
milestone).

### Phase 0: Foundation (~30 h)
- Fix the current skeleton: `entry.rs` is invalid Rust (default field values,
  `entry_type.Undefined`), modules are not declared in `lib.rs`, and
  `cdept/src/library_handler_gui.rs` is a duplicate. Use idiomatic names (`EntryType`, `Library`).
- Name decided: CDePT. License: MIT (LICENSE file added).
- Workspace split (core / gui / cli), `tracing` logging, `clap` CLI, CI (fmt, clippy, test).
- Dockable multi-panel GUI shell (`egui_dock` or `egui_tiles`), settings persistence.
- Tool detection (`libcdept::tools::KNOWN_TOOLS`): binaries on `PATH`, Python packages
  (`importlib` lookup, version from package metadata, interpreter `$CDEPT_PYTHON` or
  `python3`) and KLayout salt packages. Every entry records the phase that integrates it.
  The list covers the asic-all image (see §3, "External tools").
- **Milestone:** empty app with docked panels, CLI stub, CI green.

### Phase 1: Project & Library Manager (~80 h) — done 2026-11-22
Done:
- Data model: Project → Library → Cell → View (`Entry`), with typed views (`ViewKind`) and
  file formats (`Format`, which names the tool).
- `cdept.toml`: project fields, `[libraries]` (short and long form), `include`, variable
  expansion, read-only reference libraries, `cells` and `flat` layouts (see §1).
- Operations: create project/library/cell/view (templates: xschem schematic and symbol,
  empty GDS, SPICE, Verilog), copy (also between layouts), rename and delete cell, delete
  view, remove a library definition. Read-only libraries refuse changes.
- CLI: `cdept init`, `cdept ls`, `cdept lib add|remove`. The project is found from the
  current folder upwards, like cargo.
- GUI: New/Open/Recent/Close Project, library tree (side panel), 3-column Library Manager
  with cell filter, context menus and dialogs for all operations, Inspector.

Also done in the 2026-11-22 round: hierarchy browser, reference rewriting, file watching, git
status, categories, search, import, new-project wizard. Leftovers: NOTES.md §1.
Original to-do (kept for the record):
- Rewrite references when a cell is renamed or copied (instances in other `.sch` files,
  the structure name inside GDS). Needs the parsers of the hierarchy browser.
- Hierarchy browser: parse xschem instances and GDS references → tree, "used by" query.
  A GDS file with many cells (e.g. a standard-cell library) is one cell today; the
  hierarchy work should list the structures inside it.
- Categories and search across libraries.
- File watching (`notify`) so external edits show up live (today: Reload). Git status per cell.
- Import an existing xschem/KLayout project (scan → generate the `[libraries]` table).
- IHP **LibMan** (`github.com/IHP-GmbH/LibMan`, in the asic-all image). Findings so far:
  its project file (`*.projects` / `*.lib`) is a list of `define(name, path)` calls, with
  wildcard paths; a library folder is marked by a `.room` file; views are chosen by file
  suffix (`.sch .sym .gds .oas .lstr`); categories live in `Category.group` files. Our
  `flat` layout reads the same kind of folder. To do: import its `define()` list into
  `[libraries]`, read its categories, and offer "open in LibMan".
- **Milestone:** manage a real project (e.g. ASICONE SAR-ADC) entirely from CDePT.

### Phase 2: Technology / PDK Manager (~60 h) — done 2026-11-22 (details: [TECH.md](TECH.md); leftovers in NOTES.md)
- Discover PDKs: `$PDK_ROOT` / open_pdks tree (`libs.tech/{klayout,magic,xschem,ngspice,netgen,qflow}`,
  `libs.ref/*`), with per-PDK quirks for sky130, gf180mcu and ihp-sg13g2.
- Technology description (`tech.toml`): model files plus corner sections (tt/ff/ss/fs/sf,
  mos/res/cap/bjt), layer map (.lyp/.lyt), DRC/LVS decks, xschemrc, magicrc, netgen setup,
  device list with PCell names.
- Generate per-tool config from that one source (xschemrc, .magicrc, KLayout tech), so the
  user configures once.
- Support NDA PDKs through user-written `tech.toml` (no PDK content shipped).
- `ciel` integration: list/enable PDK versions under `$PDK_ROOT` (LibreLane's
  `pdk_hashes.yaml` pins them). PDKs to support: sky130A, gf180mcu, ihp-sg13g2 and
  ihp-sg13cmos5l.
- `$PDK` may list several PDKs separated by colons (the asic-all image sets
  `PDK=sky130A:gf180mcu:ihp-sg13g2`). Treat it as "available PDKs", not "the PDK": the
  project chooses one. The Phase 0 toolbar still shows the raw value.
- **Milestone:** create a project for sg13g2 or sky130 in 3 clicks. All tools open with the
  correct tech.

### Phase 3: Tool Hub & IPC (~110 h) ← highest technical risk
- Process supervisor: launch, track, restart, collect logs per tool, one instance per tool
  reused across cells.
- **xschem:** embedded (decided 2026-11-17, working since 2026-11-18). xschem's C code
  is built from a submodule into a GPL helper process, `cdept-xschem`, with stand-ins
  for Xlib and Tk; its editor, menus and dialogs show in a CDePT tab, and the same
  engine does netlists, hierarchy and previews in the background. It draws the same as
  stock xschem, so the fallback (a separate xschem window driven over its TCP
  listener) is not needed. Plan, results and known gaps: [XSCHEM.md](XSCHEM.md).
- **KLayout:** embedded (started 2026-11-19). KLayout v0.30.12 is built from a
  submodule; a GPL helper (`cdept-klayout`, a Python program run by `klayout -z`)
  drives a widget-less editable `LayoutView` whose pixels show in a CDePT tab; layers,
  cells and selection are CDePT panels. Layout views reference each other through
  ghost cells and `<file>.json` sidecars; missing cells show as a red X. Edit in
  place, properties and PCell forms, OASIS references, DRC/LVS with the PDK decks. Plan,
  results and gaps: [KLAYOUT.md](KLAYOUT.md). PCell views in CDePT libraries.
  Schematic ↔ layout linking (Layout XL style): device map, generate/update, check,
  cross-probing, nets (opens, shorts, flylines, net probing) ([LINKING.md](LINKING.md)).
- *Done 2026-11-23:* process supervisor/job queue ([JOBS.md](JOBS.md)); Magic batch ([MAGIC.md](MAGIC.md));
  in-process libngspice ([NGSPICE.md](NGSPICE.md)); Xyce, netgen, yosys, OpenROAD ([BATCH.md](BATCH.md)).
  Not done: VACASK, Qucs-S, Magic GUI socket server, supervisor restart/limits.
- **Magic:** `magic -dnull -noconsole` with Tcl over a pipe for batch (ext, LEF, DRC). For the
  GUI, source a Tcl socket-server script.
- **ngspice:** in-process via `libngspice` (shared API: `ngSpice_Init`, `ngSpice_Command`,
  callbacks for live vectors). *Done:* hand-written bindings loaded with dlopen, and the
  testbench backend `simulator_backend = "inproc"` with progress and cancel
  ([NGSPICE.md](NGSPICE.md)); cancel of Xyce runs and a Magic time limit ([JOBS.md](JOBS.md)).
- **KLayout extras:** `klive` (salt package, JSON on port 8082) already reloads a GDS in a
  running KLayout. Use it as the first "open layout" path before our own bridge exists.
  The productivity suite and xsection are detected as salt packages too.
- **Xyce, VACASK, Qucs-S, netgen, yosys, OpenROAD:** batch adapters (command and parse output).
- **Python tools** (KLayout pip module, gdsfactory, CACE, pygmid…) run with the detected
  interpreter. Same supervisor, with the venv/interpreter recorded per project.
- **Milestone:** double-click a view to open it in the correct tool. Netlist any cell from CDePT.

### Phase 4: Simulation Environment, "ADE-lite" (~120 h)
Done 2026-11-23 ([SIMULATION.md](SIMULATION.md)): testbench view, deck, ngspice and Xyce batch runs,
corners × temperatures × variable sweeps, Monte Carlo, measures and specs, job queue
([JOBS.md](JOBS.md)), results store (`result.json` + `results.db`), Simulation tab in the GUI.
VACASK as a testbench simulator and OpenVAF on-demand compile (cached per technology) are
done ([VACASK.md](VACASK.md): ihp-sg13g2 works, sky130 does not convert yet).
Milestone reached 2026-11-23 ([TESTS.md](TESTS.md) §6): the 5-transistor OTA of
`examples/ihp-inv` (`cells/ota5t`, IHP SG13G2, 1.2 V, 20 uA bias, 100 fF) runs as a suite
from the Simulation tab over five corners (tt/ff/ss/fs/sf) and three temperatures
(-40/27/125 C): 6 tests, 90 runs, a spec table of gain, UGBW, phase margin, settling,
overshoot, saturation margin, current, power, PSRR and CMRR, all PASS. At tt, 27 C: open-loop
gain 35.8 dB, UGBW 145 MHz, phase margin 59 deg, 1 % settling 9.2 ns; over the corners gain
32.1 to 36.6 dB, UGBW 111 to 176 MHz, phase margin 57.9 to 62.4 deg (screenshot in the
[suites how-to](howto/suites.md)). Templates may now declare sweeps and Monte Carlo.
CACE datasheet import/export is done ([reference/files/cace.md](reference/files/cace.md), CACE 2.x `cace_format` 5.2).
Sweeps and Monte Carlo are editable in the GUI form (SYMBI-77).
Still to do: SLURM/ssh; the rest of the list below as written.
- Testbench view: DUT cell, analyses (op/dc/ac/tran/noise/pss if available), design
  variables, model corners (from tech.toml), temperature, saves/probes.
- Netlisting pipeline: xschem netlist → wrapper with includes, `.param` and `.control`.
  Post-layout option: swap the DUT for the extracted netlist (Phase 7).
- Sweeps (parametric, corners × variables × temperature) and Monte Carlo when the PDK has
  statistical models.
- Simulators: ngspice first, then Xyce and VACASK through the same netlist pipeline.
  OpenVAF(-reloaded) compiles Verilog-A models into OSDI for ngspice/VACASK (needed for PSP
  and other compact models in the IHP PDKs). CDePT compiles them on demand and caches them
  per tech.
- CACE (datasheet-driven characterisation) overlaps with the spec table: import and export
  CACE datasheets (done) rather than inventing a second spec format. pygmid can feed gm/Id sizing
  helpers later.
- Job queue: parallel local runs (tokio + process pool), progress, cancel. Optional SLURM/ssh
  later.
- Results DB: per-run directory + SQLite index (`rusqlite`). Outputs/specs table with
  pass/fail (the ADE "Outputs" panel).
- **Milestone:** corner sweep of an OTA from the GUI, with a spec table showing gain, PM
  and UGBW pass/fail.

### Phase 5: Waveform Viewer + Calculator (~150 h)
- Readers: SPICE raw (binary and ascii, ngspice and Xyce flavours; small custom parser),
  CSV, digital VCD/FST/GHW via `wellen`.
- **Decision: CDePT builds no waveform viewer of its own.** The analog viewer (egui plot,
  strips, cursors, Bode, eye diagrams, overlay of sweep families) is UberWave; digital
  VCD/FST/GHW viewing is Surfer or GTKWave, started the same way. What CDePT keeps is the
  raw reader (`libcdept/src/raw`, used by the calculator, `[[output]]` and the listing of a
  file's analyses) and the numbers: measures, the calculator and the spec table.
- **UberWave** (a separate program of Symbiotic EDA, repository `uberwave`; same owner as CDePT, so its licence does not limit the integration: starting it as a process is the first step and embedding it is possible if that proves too limited; the uberWAVE project from
  the proposal): C++20/ImGui/ImPlot analog and mixed-signal viewer built for multi-GB ngspice
  raw files (memory-mapped parser, decimation, `.uwraw` cache), plus Spectre PSF. Binary
  `UberWaveViewer`; CLI `UberWaveViewer <file.raw|psf-dir> [--analysis <name>]` (one path in
  the older builds, several files and `--ipc` in the `ipc-multifile` branch); there is also a WebAssembly build. CDePT only starts it as a
  separate process (never linked: CDePT stays MIT). Integration steps:
  1. **Done**: "Open in UberWave" on a point of the Simulation tab, and `cdept wave`; found
     through `$CDEPT_UBERWAVE`, `PATH` or a sibling build (`cdept tools`).
  2. **Done**: `cdept wave <lib> <cell> [--view] [--point]` opens the `out.raw` of a run
     point of the results folder, with the transient analysis preselected when the file has
     several (`--analysis`).
  3. **Reached on the CDePT side** (needs the viewer branch `ipc-multifile` of the `uberwave`
     repository to be merged and built; an older viewer keeps the one-file behaviour):
     UberWave takes several files (`UberWaveViewer [--analysis A] [--tag T ...] FILE...`),
     `--version` prints `UberWaveViewer <version> ipc 1`, and `--ipc <socket>` starts a
     JSON-lines server on a Unix socket (protocol 1: hello, open, import, list_files,
     list_signals, show, hide, set_analysis, cursors, range, add_trace, remove_trace,
     subscribe, quit). CDePT has the client (`libcdept/src/wave/ipc.rs`): `cdept wave --family`
     and "Open family in UberWave" open all the points of a view in one viewer (tags = point
     names), and a second open of the same run reuses the running viewer. Tested against a
     fake server (`cdept-gui/scripts/fake-uberwave-ipc.py`) and once by hand against the
     viewer build with IPC.
  4. **Reached on the CDePT side**: the calculator feeds the viewer. `cdept wave ... --calc
     EXPR` and the expression line of the Simulation tab evaluate an expression on one point
     or on every point of a family and push waveform results as traces (`add_trace`); scalars
     are printed. Still open: following the viewer's cursors in the spec table
     (`subscribe` is in the client, nothing uses it yet), and showing CDePT's own measures as
     markers.
  5. Which side owns large-file viewing is decided: UberWave. The numbers stay in CDePT.
- Other viewers detected for "open with": GTKWave and gaw (xschem's viewer).
- Calculator, in two layers:
  1. Native Rust function library (`rustfft`, `ndarray`): FFT/windowing, SNR/SNDR/SFDR/THD/ENOB,
     INL/DNL (histogram and code-edge), noise integration and spot noise, PSD, jitter
     (period, cycle-to-cycle, TIE), rise/fall, slew, settling, overshoot, delay, frequency,
     duty, bandwidth, gain/phase margin, cross, value-at, min/max/avg/RMS, derivative/integral.
  2. Expression language (Virtuoso-calculator style, e.g. `sndr(v("/out"), 1e6, 8)`). Use a small
     parser (`winnow`/`chumsky`). Optional Python scripting via `pyo3` (numpy/scipy) for
     power users.
- Expressions can be saved as testbench outputs (feeds the Phase 4 spec table).
- Built 2026-11-23 (no GUI yet): raw reader, native function library, expression language, `[[output]]`, `cdept calc` ([CALC.md](CALC.md)). Open: jitter/PSD, CSV/VCD, panel in the GUI.
- **Milestone:** SAR-ADC transient → FFT → ENOB in the GUI with no Python written.

### Phase 6: Schematic ↔ Layout coherence, "XL-lite" (~100 h)
- Cross-probing: select an instance or net in xschem and it highlights in KLayout, and the
  reverse (through the Phase 3 bridges).
- Schematic-driven layout: from the schematic netlist, place PDK PCells (sg13g2 KLayout
  PCells, gf180 KLayout PCells, sky130 Magic generators) with schematic parameters, plus
  pins and flylines (rat's nest) in KLayout.
- Consistency check: instance/parameter diff between schematic and layout (pre-LVS
  "connectivity check").
- Built for KLayout with the PDK's decks (2026-11-20): cross-probing of instances and
  nets, generate/update from the schematic, flylines, `check-source` and `check-nets`
  ([LINKING.md](LINKING.md)); Magic generators not yet.
- **Milestone:** generate a layout skeleton of a 5T-OTA from its schematic, then cross-probe
  a net.

### Phase 7: Physical verification & signoff (~60 h)
- DRC: KLayout decks / Magic DRC, results to `.lyrdb` and an CDePT summary list that links back
  to KLayout's marker browser.
- LVS: netgen or KLayout LVS. CDePT assembles the schematic netlist correctly (subckt
  includes, CDL conversion, global nets), which is the pain point the proposal names.
- PEX: Magic `ext2spice` (C, RC) → extracted view → back-annotated simulation (Phase 4).
- ERC: CVC (circuit validity check) on the schematic/extracted netlist.
- PEX alternatives: KLayout-PEX (`kpex`), plus FastHenry2/FasterCap for inductance and
  capacitance extraction of critical structures. GDS3D gives a 3D view of a cell.
- Antenna, density and fill hooks where the PDK provides them (gdsfill for IHP). Signoff checklist per cell
  (status badges in the Library Manager).
- **Milestone:** one-click DRC → LVS → PEX → post-layout sim on a cell.

### Phase 8: Mixed-signal integration (~60 h)
- Abstracts: LEF via Magic `lef write` or a KLayout script, with pin/blockage options.
- Liberty: characterise with lctime (simple cells) and CharLib. Timing-model stub for hard
  macros (OpenSTA-readable).
- Netlist conversion: Verilog ↔ SPICE/CDL (yosys `write_spice`, OpenROAD `write_cdl`, custom
  converter). Black-box handling for analog macros.
- Co-simulation: ngspice XSPICE `d_cosim` with Verilator/Icarus for digital blocks in analog
  tests. Alternative: cocotb + spicebind (ngspice inside the HDL simulator). NVC for VHDL.
- Digital flow hook: launch LibreLane/OpenROAD (or SiliconCompiler) on an RTL view and import
  GDS/LEF/LIB as views. OpenSTA for timing reports.
- Generators that produce views: padring (pad ring GDS), OpenRAM (SRAM macros).
- **Milestone:** SAR-ADC macro (analog) + SPI (digital) integrated with LEF/LIB generated
  from CDePT.

### Phase 9: Release, docs, community (~30 h, plus continuous)
- Packaging: cargo install, AppImage/Flatpak, Nix, IIC-OSIC-TOOLS docker integration, and the
  Symbiotic EDA `asic-all` image (ship CDePT in it, with its desktop entry).
- Tutorials: 5T-OTA (sky130 / sg13g2), SAR-ADC flow. User manual and developer guide (how to
  write a ToolAdapter).
- Upstream patches where needed (xschem, KLayout macro, Surfer).

Phase order is mostly sequential. 4 depends on 2 and 3. 5 can start in parallel with 3–4,
because the raw reader and calculator have no dependencies. 6 and 7 depend on 3.

---

## 3. Libraries & tools

### Rust crates (core/GUI)
| Purpose                | Crate(s)                                                     |
|------------------------|--------------------------------------------------------------|
| GUI                    | `egui`, `eframe` (already used), `egui_dock`/`egui_tiles`, `egui_extras` (tables), `egui_plot`, `rfd` (file dialogs) |
| Config / serialisation | `serde`, `toml_edit` (keeps comments in hand-edited files), `serde_json` |
| Errors / logs / CLI    | `eyre` (in use), `thiserror`, `tracing`, `clap`              |
| Async / processes / IPC| `tokio` (process, net), JSON-RPC over TCP (`serde_json` line protocol or `jsonrpsee`) |
| File watching          | `notify`                                                     |
| Results DB             | `rusqlite`                                                   |
| DSP / math             | `rustfft`, `realfft`, `ndarray`, `statrs`, `num-complex`     |
| Parsers                | `winnow` or `nom` (SPICE raw, netlists, xschem .sch), `chumsky` (calc language) |
| Waveforms              | `wellen` (VCD/FST/GHW, used by Surfer), `libsurfer`          |
| Layout                 | `gds21` (GDS read/write, thumbnails/hierarchy), `libreda-*` (Rust EDA DB from the LibreCell author) |
| ngspice                | `bindgen` on `sharedspice.h` (or an existing `ngspice-sys`-style crate) |
| Python scripting       | `pyo3` (optional)                                            |
| Git integration        | `git2` or `gix`                                              |

### External tools orchestrated
Reference tool set: the `factory.symbioticeda.com/asic-all:dev` image
(`centerstage/bouquet_factory/bouquet_assembler/asic-all/asic-all.yaml`). Every tool from it
that matters to CDePT is in `KNOWN_TOOLS`, with the phase that integrates it; binary names
and version flags were checked in that image. Add new tools there first.

| Area | Tools (phase) |
|------|---------------|
| Schematic | xschem (3), Qucs-S (3), XCircuit (detect only) |
| Layout | KLayout (3) + pip `klayout` (3), Magic (3), klive (3), KLayout productivity suite (6), xsection (7), gdsfactory (6), GDS3D (7) |
| Simulation | ngspice (3/4), Xyce (4), VACASK (4), OpenVAF-reloaded (4), CACE (4), pygmid (4), spicelib (5), PySpice (detect only) |
| Waveforms | UberWave (5), Surfer (5), GTKWave (5), gaw (5) |
| Verification | netgen (7), CVC (7), KLayout-PEX (7), FastHenry2 (7), FasterCap (7), gdsfill (7) |
| Digital / mixed | yosys, OpenROAD, OpenSTA, LibreLane, SiliconCompiler, Verilator, Icarus, NVC, cocotb, spicebind, CharLib, lctime, padring, OpenRAM (all 8) |
| RF / EM | openEMS (+ IHP openEMS scripts), scikit-rf (detect only; RF flows are outside this grant) |
| Project / PDK / scripting | LibMan (1), ciel (2), Python 3 (3) |

Left out on purpose, because they don't touch the analog/mixed-signal design data CDePT
manages or are reached through another tool: gvim; abc (via yosys); graywolf, qflow,
IRSIM (superseded by LibreLane/ngspice); HDL front-ends and linters (amaranth, PyRTL,
pyverilog, veryl, sv2v, surelog, verible, svck, rggen, kactus2); build/package managers
(edalize, fusesoc, bender, pulp-tools); digital verification extras (cocotbext-*, pyuvm,
verilog-covered, fault, kepler-formal); geometry/netlist libraries (gdspy, gdstk, gmsh,
najaeda, vlsir/hdl21, schemdraw, ngspyce, spyci); RISC-V toolchain and spike.

- PDKs: open_pdks via ciel (sky130, gf180mcu), IHP-Open-PDK (sg13g2, sg13cmos5l)
- Distribution references: the asic-all image (primary) and the IIC-OSIC-TOOLS container
  (both pin all tools, good for CI)

---

## 4. Key risks
1. **IPC per tool** (Phase 3). Prototype the xschem and KLayout bridges first: they are the
   backbone of Phases 6 and 7.
2. **PDK heterogeneity.** Directory layouts and corner naming differ. Keep PDK quirks in data
   (`tech.toml` templates), not in code.
3. **Large waveforms.** Multi-GB transient raw files need memory-mapping and decimation from
   day one.
4. **Scope.** Schematic-driven layout and Monte Carlo can grow without limit. Keep them "lite"
   in this grant.

## 5. Session plan
| Day/session | Work |
|-------------|------|
| 1 | Phase 0: fix skeleton, workspace split, docked GUI shell, CI |
| 2–3 | Phase 1: data model + `cdept.toml` libraries + Library Manager browser (done), then hierarchy, reference rewriting, file watching |
| 4 | Phase 2: PDK discovery for sg13g2 + sky130, `tech.toml` |
| 5 | Phase 3 spike: xschem listen-port + KLayout bridge prototypes |
| 6 | Phase 5 spike: SPICE raw reader + basic plot (parallel track) |
| 7+ | Phase 4 ADE-lite, then calculator, then 6–8 |
