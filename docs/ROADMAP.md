# CDePT: Roadmap

Goal: one open-source environment that does for open PDKs what Cadence Virtuoso does for
commercial ones. It does **not** replace xschem/KLayout/Magic/ngspice. It **orchestrates**
them: one project and library model, one technology setup, one simulation environment, one
waveform viewer and calculator, one signoff dashboard, with schematic, layout and simulation
kept in sync.

Source of scope: NLnet proposal "Chip Design Project Tool (CDePT)". Budget: ~750 h
(15 h/week × 50 weeks).

---

## 0. Virtuoso feature map → what CDePT provides

| Virtuoso component              | CDePT equivalent                         | Backed by (external)                 |
|---------------------------------|----------------------------------------|--------------------------------------|
| cds.lib / Library Manager       | `cdept.lib` + Library Manager (lib/cell/view) | filesystem + git                 |
| Technology file / PDK setup     | Technology Manager (open_pdks aware)   | sky130, gf180mcu, ihp-sg13g2         |
| Virtuoso Schematic Editor       | Launch + control                       | xschem (primary), Qucs-S             |
| Virtuoso Layout Suite L/XL      | Launch + control, cross-probe, SDL     | KLayout (primary), Magic             |
| ADE L/XL/Assembler/Explorer     | Simulation Environment                 | ngspice (shared lib), Xyce           |
| ViVA waveform + Calculator      | Waveform Viewer + Calculator           | own egui plot + Surfer/wellen        |
| Assura/PVS/Calibre DRC/LVS/PEX  | Signoff dashboard                      | KLayout DRC/LVS, Magic, netgen       |
| Abstract Generator / Liberate   | Mixed-signal utilities                 | Magic LEF, lctime/CharLib, OpenSTA   |
| AMS Designer                    | Co-simulation                          | ngspice XSPICE d_cosim + Verilator/Icarus |
| Innovus handoff                 | Digital flow hook                      | OpenROAD / LibreLane, yosys          |

---

## 1. Architecture (decide in Phase 0, keep stable)

```
cdept (GUI binary, egui/eframe)
 └── libcdept (core, no GUI deps where possible)
      ├── project/     lib-cell-view model, cdept.lib, cell metadata (TOML, serde)
      ├── tech/        PDK discovery + technology description
      ├── tools/       ToolAdapter trait, process supervisor, IPC clients
      │    ├── xschem   (Tcl over TCP: xschem_listen_port)
      │    ├── klayout  (Python plugin, JSON-RPC over TCP)
      │    ├── magic    (Tcl over stdin/pipe or Tcl socket script)
      │    ├── ngspice  (libngspice shared API, in-process)
      │    └── xyce / qucs-s / netgen / yosys / openroad (batch)
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

### On-disk model (the Virtuoso lib/cell/view idea, in plain files)
```
myproject/
  cdept.toml                 # project: tech ref, tool paths, default corners
  cdept.lib                  # DEFINE mylib ./libs/mylib ; DEFINE sg13g2_pr $PDK/... (read-only)
  libs/mylib/
    lib.toml               # library metadata, tech binding
    ota5t/                 # cell
      cell.toml            # views, pins, symbol↔layout mapping, status
      schematic/ota5t.sch  # view: xschem
      symbol/ota5t.sym
      layout/ota5t.gds     # or .mag
      tb_ac/               # view: testbench (sim setup + results)
        setup.toml
        results/<run-id>/
```
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
- **Milestone:** empty app with docked panels, CLI stub, CI green.

### Phase 1: Project & Library Manager (~80 h)
- Data model: Project → Library → Cell → View (entry), with typed views (schematic, symbol,
  layout, netlist, testbench, config, verilog, abstract, liberty).
- `cdept.lib` parser (DEFINE / INCLUDE / env vars), read-only reference libraries (PDK).
- Library Manager GUI: 3-column lib/cell/view browser, create/copy/rename/delete cell
  (rewriting references inside .sch/.gds), categories, search.
- Hierarchy browser: parse xschem instances and GDS references → tree, "used by" query.
- File watching (`notify`) so external edits show up live. Git status per cell.
- Import an existing xschem/KLayout project (scan → generate `cdept.lib`).
- **Milestone:** manage a real project (e.g. ASICONE SAR-ADC) entirely from CDePT.

### Phase 2: Technology / PDK Manager (~60 h)
- Discover PDKs: `$PDK_ROOT` / open_pdks tree (`libs.tech/{klayout,magic,xschem,ngspice,netgen,qflow}`,
  `libs.ref/*`), with per-PDK quirks for sky130, gf180mcu and ihp-sg13g2.
- Technology description (`tech.toml`): model files plus corner sections (tt/ff/ss/fs/sf,
  mos/res/cap/bjt), layer map (.lyp/.lyt), DRC/LVS decks, xschemrc, magicrc, netgen setup,
  device list with PCell names.
- Generate per-tool config from that one source (xschemrc, .magicrc, KLayout tech), so the
  user configures once.
- Support NDA PDKs through user-written `tech.toml` (no PDK content shipped).
- **Milestone:** create a project for sg13g2 or sky130 in 3 clicks. All tools open with the
  correct tech.

### Phase 3: Tool Hub & IPC (~110 h) ← highest technical risk
- Process supervisor: launch, track, restart, collect logs per tool, one instance per tool
  reused across cells.
- **xschem:** use the built-in TCP listener (`xschem_listen_port`) to send Tcl (open, netlist,
  select instance, highlight net). Get callbacks by injecting a small Tcl script. *Verify
  first*: this probably avoids the source modifications the proposal assumed.
- **KLayout:** ship an CDePT macro/plugin (`klayout -rm cdept_bridge.py`) that opens a TCP/JSON-RPC
  server via `pya.QTcpServer` or a polling `QTimer`. Commands: load cell, select/zoom, highlight
  net, run DRC, report selection back. Risk: some distro builds lack Qt bindings, so detect this
  and fall back to batch mode.
- **Magic:** `magic -dnull -noconsole` with Tcl over a pipe for batch (ext, LEF, DRC). For the
  GUI, source a Tcl socket-server script.
- **ngspice:** in-process via `libngspice` (shared API: `ngSpice_Init`, `ngSpice_Command`,
  callbacks for live vectors). Bind through `bindgen` or an existing crate.
- **Xyce, Qucs-S, netgen, yosys, OpenROAD:** batch adapters (command and parse output).
- **Milestone:** double-click a view to open it in the correct tool. Netlist any cell from CDePT.

### Phase 4: Simulation Environment, "ADE-lite" (~120 h)
- Testbench view: DUT cell, analyses (op/dc/ac/tran/noise/pss if available), design
  variables, model corners (from tech.toml), temperature, saves/probes.
- Netlisting pipeline: xschem netlist → wrapper with includes, `.param` and `.control`.
  Post-layout option: swap the DUT for the extracted netlist (Phase 7).
- Sweeps (parametric, corners × variables × temperature) and Monte Carlo when the PDK has
  statistical models.
- Job queue: parallel local runs (tokio + process pool), progress, cancel. Optional SLURM/ssh
  later.
- Results DB: per-run directory + SQLite index (`rusqlite`). Outputs/specs table with
  pass/fail (the ADE "Outputs" panel).
- **Milestone:** corner sweep of an OTA from the GUI, with a spec table showing gain, PM
  and UGBW pass/fail.

### Phase 5: Waveform Viewer + Calculator (~150 h)
- Readers: SPICE raw (binary and ascii, ngspice and Xyce flavours; small custom parser),
  CSV, digital VCD/FST/GHW via `wellen`.
- Analog viewer: `egui_plot` (or a custom plot widget for large data with min/max decimation),
  multiple strips, cursors and deltas, log axes, Bode/magnitude-phase, eye diagram, overlay of
  sweep families, mixed analog and digital in one window.
- Digital: embed or interoperate with Surfer (egui, Rust, `libsurfer`, Waveform Control
  Protocol). Coordinate with the uberWAVE team mentioned in the proposal.
- Calculator, in two layers:
  1. Native Rust function library (`rustfft`, `ndarray`): FFT/windowing, SNR/SNDR/SFDR/THD/ENOB,
     INL/DNL (histogram and code-edge), noise integration and spot noise, PSD, jitter
     (period, cycle-to-cycle, TIE), rise/fall, slew, settling, overshoot, delay, frequency,
     duty, bandwidth, gain/phase margin, cross, value-at, min/max/avg/RMS, derivative/integral.
  2. Expression language (Virtuoso-calculator style, e.g. `sndr(v("/out"), 1e6, 8)`). Use a small
     parser (`winnow`/`chumsky`). Optional Python scripting via `pyo3` (numpy/scipy) for
     power users.
- Expressions can be saved as testbench outputs (feeds the Phase 4 spec table).
- **Milestone:** SAR-ADC transient → FFT → ENOB in the GUI with no Python written.

### Phase 6: Schematic ↔ Layout coherence, "XL-lite" (~100 h)
- Cross-probing: select an instance or net in xschem and it highlights in KLayout, and the
  reverse (through the Phase 3 bridges).
- Schematic-driven layout: from the schematic netlist, place PDK PCells (sg13g2 KLayout
  PCells, gf180 KLayout PCells, sky130 Magic generators) with schematic parameters, plus
  pins and flylines (rat's nest) in KLayout.
- Consistency check: instance/parameter diff between schematic and layout (pre-LVS
  "connectivity check").
- **Milestone:** generate a layout skeleton of a 5T-OTA from its schematic, then cross-probe
  a net.

### Phase 7: Physical verification & signoff (~60 h)
- DRC: KLayout decks / Magic DRC, results to `.lyrdb` and an CDePT summary list that links back
  to KLayout's marker browser.
- LVS: netgen or KLayout LVS. CDePT assembles the schematic netlist correctly (subckt
  includes, CDL conversion, global nets), which is the pain point the proposal names.
- PEX: Magic `ext2spice` (C, RC) → extracted view → back-annotated simulation (Phase 4).
- Antenna, density and fill hooks where the PDK provides them. Signoff checklist per cell
  (status badges in the Library Manager).
- **Milestone:** one-click DRC → LVS → PEX → post-layout sim on a cell.

### Phase 8: Mixed-signal integration (~60 h)
- Abstracts: LEF via Magic `lef write` or a KLayout script, with pin/blockage options.
- Liberty: characterise with lctime (simple cells) and CharLib. Timing-model stub for hard
  macros (OpenSTA-readable).
- Netlist conversion: Verilog ↔ SPICE/CDL (yosys `write_spice`, OpenROAD `write_cdl`, custom
  converter). Black-box handling for analog macros.
- Co-simulation: ngspice XSPICE `d_cosim` with Verilator/Icarus for digital blocks in analog
  tests.
- Digital flow hook: launch LibreLane/OpenROAD on an RTL view and import GDS/LEF/LIB as views.
- **Milestone:** SAR-ADC macro (analog) + SPI (digital) integrated with LEF/LIB generated
  from CDePT.

### Phase 9: Release, docs, community (~30 h, plus continuous)
- Packaging: cargo install, AppImage/Flatpak, Nix, IIC-OSIC-TOOLS docker integration.
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
| Config / serialisation | `serde`, `toml`, `serde_json`                                |
| Errors / logs / CLI    | `eyre` (in use), `thiserror`, `tracing`, `clap`              |
| Async / processes / IPC| `tokio` (process, net), JSON-RPC over TCP (`serde_json` line protocol or `jsonrpsee`) |
| File watching          | `notify`                                                     |
| Results DB             | `rusqlite`                                                   |
| DSP / math             | `rustfft`, `realfft`, `ndarray`, `statrs`, `num-complex`     |
| Parsers                | `winnow` or `nom` (SPICE raw, netlists, xschem .sch, cdept.lib), `chumsky` (calc language) |
| Waveforms              | `wellen` (VCD/FST/GHW, used by Surfer), `libsurfer`          |
| Layout                 | `gds21` (GDS read/write, thumbnails/hierarchy), `libreda-*` (Rust EDA DB from the LibreCell author) |
| ngspice                | `bindgen` on `sharedspice.h` (or an existing `ngspice-sys`-style crate) |
| Python scripting       | `pyo3` (optional)                                            |
| Git integration        | `git2` or `gix`                                              |

### External tools orchestrated
- Schematic: xschem (primary), Qucs-S
- Layout: KLayout (primary, Python API), Magic (Tcl)
- Simulation: ngspice (shared lib), Xyce
- Verification: KLayout DRC/LVS, Magic DRC/ext, netgen LVS
- Digital / mixed: yosys, OpenROAD/OpenSTA, LibreLane, Verilator, Icarus, lctime, CharLib
- Waveforms: Surfer, GTKWave (fallback)
- PDKs: open_pdks (sky130, gf180mcu), IHP-Open-PDK (sg13g2)
- Distribution reference: IIC-OSIC-TOOLS container (all tools pinned, good for CI)

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
| 2–3 | Phase 1: data model + `cdept.lib` + Library Manager browser |
| 4 | Phase 2: PDK discovery for sg13g2 + sky130, `tech.toml` |
| 5 | Phase 3 spike: xschem listen-port + KLayout bridge prototypes |
| 6 | Phase 5 spike: SPICE raw reader + basic plot (parallel track) |
| 7+ | Phase 4 ADE-lite, then calculator, then 6–8 |
