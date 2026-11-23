# VACASK and OpenVAF in CDePT

Status 2026-11-23. `simulator = "vacask"` in a testbench's `setup.toml` runs it with
VACASK; `cdept openvaf compile` compiles Verilog-A to OSDI. Code: `libcdept/src/vacask/`
(translator, models, deck, raw reader, measurements), `libcdept/src/batch/vacask.rs`,
`libcdept/src/batch/openvaf.rs`; CLI `cdept sim run`, `cdept vacask models`,
`cdept openvaf compile` (`cdept/src/cmd_vacask.rs`). Tests: `libcdept/tests/vacask.rs`.

## 1. What VACASK is, as found here

`/opt/ext/OpenLane/bin/vacask` (0.3.2, a debug build, fast enough for these circuits).

- **Own netlist format**, Spectre-like, not SPICE: `name (n1 n2) master param=value`,
  `subckt … ends`, `model name module (params)`, `load "x.osdi"`, `include "file"
  [section=name]`, `parameters`, `ground 0`, `global`, and a `control … endc` block with
  `options`, `save`, `sweep` and `analysis <name> <type> …`. Devices are OSDI libraries
  (`model resistor sp_resistor` after `load "spice/resistor.osdi"`); only the sources
  (`vsource`, `isource`, `vcvs`, …) are builtin (`include "builtins.inc"`).
- **Analyses:** `op`, `tran`, `ac`, `noise`, `acxf`, `hb`. There is no `dc` analysis: a
  DC sweep is `sweep s instance="V1" parameter="dc" from= to= step=` in front of an `op`
  analysis. `ac`/`noise` take `from to mode="dec|oct|lin" points`.
- **Output:** one ngspice-style binary raw file per analysis, `<analysis name>.raw`, in the
  working folder (`Flags: real|complex`, variables `time`, `out`, `V1:flow(br)`,
  `onoise`, …). No `.meas`: measurements are done on the raw file by the user.
- **Runs as a process:** `vacask [-qp] deck.sim`. Most errors print a message and a
  source position but the exit status stays 0 (`Analysis 'x' aborted.`), so CDePT also
  checks that every analysis left its raw file.
- **Needs** `libboost_process`/`libboost_filesystem`, in `/opt/ext/OpenLane/lib`, which the
  dynamic loader does not know: the adapter adds `<prefix>/lib` of the program to
  `LD_LIBRARY_PATH` (`vacask::tool_env_for`), also for OpenVAF.
- **VACASK ships its own converters** in `<prefix>/lib/vacask/python` (`ng2vc.py`:
  ngspice netlist and model files to VACASK; `sg13g2tovc.py`: the IHP PDK; `xschem2vc.py`
  and `sg13g2xschem.tcl`: patches of xschem symbols so xschem writes VACASK netlists).
  `ng2vc` does not read sources (`V`, `I`) or `.tran` cards, only models and subcircuits.
- **xschem cannot netlist for VACASK here:** xschem has a Spectre/VACASK netlister, but it
  needs `spectre_format=` in each symbol; the IHP PDK symbols have none (VACASK's
  `sg13g2tovc.py` patches them *in the PDK folder*). CDePT does not change PDK files, so
  the testbench is netlisted as SPICE and translated (section 3).

## 2. Models: the PDKs have no VACASK ones

No installed PDK brings `libs.tech/vacask` (ihp-sg13g2, sky130A/B have `ngspice`, `xyce`,
`verilog-a` or only `ngspice`). So CDePT makes them, once per technology:

`cdept sim run … ` (or `cdept vacask models [--force]`) does, for a technology with
`[models.ngspice]`:

1. Reads the library files of the ngspice set and the files its corner files include
   (only for the sections some corner uses), hashes them with the Verilog-A sources.
2. Converts each with VACASK's `ng2vc` (a separate `python3 -I convert.py` process,
   `libcdept/src/vacask/convert.py`) into `<project>/.cdept/osdi/<pdk>-<hash>/models/`.
   PDK-specific settings (a profile for `ihp-sg13g2`) are the ones of VACASK's own
   `sg13g2tovc.py`, plus: `agauss(mean, …)` parameters become `mean` (nominal, no
   mismatch), a `!=` that `ng2vc` cannot split is rewritten, a second definition of `swsoa`
   is dropped. Corner files keep their `.lib` sections as `section` blocks.
3. Compiles the Verilog-A models behind the set's `osdi` entries (`psp103`,
   `psp103_nqs`, `r3_cmc`, `mosvar`, `cap_cmomi`, `cap_cmomf`) with OpenVAF-r into the same
   folder, found as `<verilog_a>/<stem>/<stem>.va`; a compiled library that exists is not
   compiled again (the folder name has the hash).
4. Writes the `ModelSet` into the snapshot: `[models.vacask]` in `tech.toml`, `generated =
   "from [models.ngspice] with … hash …"`, `dir`/`osdi` as `$CDEPT_PROJECT/.cdept/osdi/…`
   (`$CDEPT_PROJECT` is the project folder), the corner sets whose files converted,
   `options` with raw VACASK lines (`parameters swsoa=0`, the default models), and notes
   `vacask: …` for everything that could not be done. A changed PDK, CDePT converter or
   Verilog-A source gives another hash and a new set replaces the generated one; a
   `[models.vacask]` without `generated` (written by hand, or one day by a PDK) is never
   touched. `cdept tech refresh` rewrites tech.toml and drops the set; the next run makes
   it again from the cache.

A testbench whose netlist calls a subcircuit or model that no converted file defines is
refused before VACASK starts, with the technology and the device named:

```
the technology 'ihp-sg13g2' has no VACASK model for 'sg13_hv_svaricap' (instance x1): no converted model file defines it
  what the conversion could not do:
    vacask: sg13_hv_svaricap (in sg13g2_svaricaphv_mod.lib) cannot be simulated: it uses a node voltage v() in an expression, a behavioural element VACASK does not have
```

### PDK status

Checked with one instance of each, `op` analysis, typical corner (`tt`):

| PDK / device | VACASK | Notes |
|---|---|---|
| ihp-sg13g2 `sg13_lv_nmos/pmos`, `sg13_hv_nmos/pmos` | works | PSP103 (`psp103.osdi`, compiled here), `tb_inv` tpd within 0.2 % of ngspice |
| ihp-sg13g2 `rsil`, `rppd`, `rhigh` | works | r3_cmc; mismatch parameters nominal |
| ihp-sg13g2 `cap_cmim`, `cap_rfcmim`, `cap_cmomi/f` | works | |
| ihp-sg13g2 `npn13G2`, `npn13G2l`, `npn13G2v`, `pnpMPA` | works | VBIC (`vbic_1p3_5t.osdi`, VACASK's own); `npn13G2*_5t` loaded, not run |
| ihp-sg13g2 diodes, ESD, `dantenna`, `schottky_nbl1`, `sg13_moscap_n/p` | works | |
| ihp-sg13g2 `sg13_hv_svaricap` | **no** | the model uses `v()` in a resistor expression; VACASK has no such function |
| ihp-sg13g2 `bondpad` | needs include | in `[models.ngspice] includes`, not in any corner, like with ngspice; the netlist's own `.include` is refused (below) |
| ihp-sg13g2 Monte Carlo / mismatch corners (`mc`, `*_mm`) | **no** | `agauss`; not converted, the corners are left out with a note |
| sky130A | **no** | tried: `ng2vc` converts the resistor, capacitor, diode and BJT files but stops on the MOS models (`level=54.0`, `4.5` where it expects an integer) and on the corner files that include them, so no corner is usable; `sim run` says "has no usable VACASK models" with the reasons. Needs a patch or a newer ng2vc; sky130 has no Verilog-A models, its BSIM4 would use VACASK's `spice/bsim4v8.osdi` |
| sky130B | not tried | same files as sky130A |
| any PDK without `[models.ngspice]` | error | "has no ngspice models to make VACASK models from" |

Not tested: the other `_5t` and `npn13G2*_NX_vbic` variants, `sg13g2_stdcell` digital
cells (their spice models are not in the model set), the XFAB PDK (NDA, not read).

## 3. The testbench netlist

xschem netlists the testbench as SPICE (as for ngspice). `vacask::translate` rewrites it
for VACASK:

- Supported: `R C L` (value, optional model, `m`), `V I` (`dc`, `ac`, `pulse`, `sin`,
  `exp`, `pwl`), `E G` (four nodes and a gain), `D M Q` with `.model` cards (SPICE
  levels 1 to 3 for MOS, diodes, Gummel-Poon BJT, BSIM3 levels 8/49, BSIM4 14/54, as
  VACASK's `spice/*.osdi`), `X` calls with parameters, `N` OSDI instances, `.subckt` with
  parameters, `.param`, `.global`, continuation lines, comments, `{expr}` and `'expr'`.
  Numbers lose their SPICE suffixes (`10f` becomes `10e-15`, `1M` is milli, `1meg`
  mega); `temper` becomes `$temp`; `m=` of passives becomes `$mfactor`.
- A source's DC value is its first waveform value (`pulse` `val0`, `sin` offset), as in
  ngspice's operating point; a DC sweep of such a source moves that parameter.
- Dropped with a warning (the setup says what to run): analysis cards, `.meas`, `.save`,
  `.options`, `.temp`, `.control` blocks.
- **Errors naming the line:** `B` sources, `K`, `F`/`H`, `S`/`W`, `.ic`, `.nodeset`,
  `.include`/`.lib` of model files (ngspice model files cannot be read by VACASK; the
  technology's `vacask` set brings the converted ones), `^` in expressions, a `pulse`
  without all seven values (VACASK has no defaults for rise, fall, width, period),
  `ic=` on passives is dropped with a warning.
- Names keep their case (SPICE is case blind, VACASK is not): a testbench that writes
  `VDD` in one place and `vdd` in another works in ngspice only. Characters VACASK does
  not take in names become `_xNN`.

## 4. The deck and the results

`cdept sim deck` prints it. For each run point: `ground 0`, `include "builtins.inc"`, the
`load`s of the model set and of the builtin devices the netlist uses, the model set's raw
lines, the `include "…" section=…` of the corner, design variables as `parameters`, the
translated netlist, and `control` with `options temp=… <setup options>`, `save`, and the
analyses: `op`, `dc` (`sweep` + `op`, end point included), `ac`, `tran`, `noise`
(`out="node" in="source"`). A transient that saves from a start time is refused.
Setup options are passed as written (VACASK's names, not ngspice's).

Raw files: `<analysis>.raw` per analysis; `out.raw` is a copy of the first analysis that is
not `op`; `out.log` is VACASK's output. Several analyses of one kind in a setup are named
`tran1`, `tran2`, …

**Measurements.** VACASK has no `.meas`. The setup's `measures` (ngspice syntax) are
evaluated by `vacask::meas` on the raw file: `trig`/`targ` delays (`val`, `rise`/`fall`/
`cross`, `last`, `td`), `max`, `min`, `pp`, `avg`, `rms`, `integ` (`from`/`to`), and `find
<vec> at=`. Anything else is an error (use the calculator when it lands). The raw reader
(`vacask::raw`) also reads ngspice/Xyce files (real, complex, binary, ASCII).

## 5. Results, compared with ngspice

`examples/ihp-inv` view `tb_inv/vacask` (same circuit as `testbench`), tpd of the
inverter, PSP103 through OSDI:

| corner | ngspice | VACASK | difference |
|---|---|---|---|
| tt | 45.138 ps | 45.135 ps | 0.01 % |
| ff | 38.640 ps | 38.561 ps | 0.2 % |
| ss | 53.923 ps | 53.981 ps | 0.1 % |

A DC sweep (`vout` at `vin` = 0.6 V, 27 and 125 degC) gives 0.94518 and 0.94793 V in
both. A level-1 MOS inverter with a diode agrees with ngspice within the test's 5 %.
RC: delay 7.35e-10 as with Xyce, `ac` follows `1/sqrt(1+(f/fc)^2)`, `noise` gives the
thermal noise of the resistor, a DC sweep reaches its end point.

## 6. OpenVAF

`libcdept/src/batch/openvaf.rs`: `$CDEPT_OPENVAF`, else `openvaf-r` or `openvaf` on PATH;
`openvaf-r -o out.osdi [-D m] [-I dir] model.va`. `cdept openvaf compile model.va [-o
out.osdi] [-D …] [-I …]` caches into `<project>/.cdept/osdi/<pdk>-<hash>/<stem>.osdi`
(hash of the `.va`/`.include`/`.vams` files next to the source, the flags and the
compiler) and prints the path; without a project give `--out`. PSP103 compiles in about
3 s. The PDK's own `ngspice/osdi/*.osdi` files are the same models and are what ngspice
uses; they could be loaded by VACASK too (same OSDI), but the generated set compiles its
own so that the result follows the Verilog-A source and the hash.

## 7. Open

- Include files with devices that no corner brings in (`bondpad`): the netlist cannot
  `.include` them yet; map them to the converted file.
- `sg13_hv_svaricap` (behavioural resistor), mismatch/Monte Carlo (no `agauss`).
- sky130 (above), a PDK that brings `libs.tech/vacask` (the
  generated set steps aside only if one writes `[models.vacask]` without `generated`;
  scanning for the PDK's own is not done).
- Corner/temperature points are one VACASK run each (no `sweep` in the deck for them);
  parallel runs share the generated cache read-only.
- `hb`/`acxf` analyses, `.ic`/`.nodeset`, `F`/`H`/`B`, `$mfactor` on sources.
- Waveform viewer/calculator on the raw files (Phase 5).
