# VACASK and OpenVAF in CDePT

Status 2026-11-24 (SYMBI-78: VACASK at the level of ngspice). `simulator = "vacask"` in a
testbench's `setup.toml`, or `--simulator vacask` on any testbench, runs it with VACASK;
`cdept openvaf compile` compiles Verilog-A to OSDI. Code: `libcdept/src/vacask/`
(translator, models, deck, raw reader, measurements, Monte Carlo draws),
`libcdept/src/batch/vacask.rs`, `libcdept/src/batch/openvaf.rs`,
`libcdept/src/sim/caps.rs` (what each simulator supports); CLI `cdept sim run|deck|suite
--simulator`, `cdept vacask models`, `cdept openvaf compile` (`cdept/src/cmd_vacask.rs`).
Tests: `libcdept/tests/vacask.rs`, `libcdept/tests/real_vacask.rs`, the unit tests of
`vacask::*` and `sim::caps`, and the GUI script `cdept-gui/scripts/vacask.script`.

## 1. What VACASK is, as found here

`/opt/ext/OpenLane/bin/vacask` (0.3.2, a debug build, fast enough for these circuits).

- **Own netlist format**, Spectre-like, not SPICE: `name (n1 n2) master param=value`,
  `subckt … ends`, `model name module (params)`, `load "x.osdi"`, `include "file"
  [section=name]`, `parameters`, `ground 0`, `global`, and a `control … endc` block with
  `options`, `save`, `sweep` and `analysis <name> <type> …`. Devices are OSDI libraries
  (`model resistor sp_resistor` after `load "spice/resistor.osdi"`); only the sources
  (`vsource`, `isource`, `vcvs`, …) are builtin (`include "builtins.inc"`). Names are case
  sensitive, a parameter or subcircuit defined twice is an error, an undefined name is an
  error when the line is read (even in a branch that is not taken), `?:` is lazy, and
  vectors (`[1, 2]`, `v[i]`) exist.
- **Analyses:** `op`, `tran`, `ac`, `noise`, `acxf`, `hb`. There is no `dc` analysis: a
  DC sweep is `sweep s instance="V1" parameter="dc" from= to= step=` in front of an `op`
  analysis. `ac`/`noise` take `from to mode="dec|oct|lin" points`.
- **Output:** one ngspice-style binary raw file per analysis, `<analysis name>.raw`, in the
  working folder (`Flags: real|complex`, variables `time`, `out`, `V1:flow(br)`,
  `onoise` (a power spectral density, V^2/Hz) and `gain` (a power gain) for noise). No
  `.meas`: CDePT evaluates the setup's `measures` on the raw file. **No random numbers.**
- **Runs as a process:** `vacask [-qp] deck.sim`. Most errors print a message and a
  source position but the exit status stays 0 (`Analysis 'x' aborted.`), so CDePT also
  checks that every analysis left its raw file.
- **Needs** `libboost_process`/`libboost_filesystem`, in `/opt/ext/OpenLane/lib`, which the
  dynamic loader does not know: the adapter adds `<prefix>/lib` of the program to
  `LD_LIBRARY_PATH` (`vacask::tool_env_for`), also for OpenVAF.
- **VACASK ships its own converters** in `<prefix>/lib/vacask/python` (`ng2vc.py`:
  ngspice netlist and model files to VACASK; `sg13g2tovc.py`: the IHP PDK; `xschem2vc.py`
  and `sg13g2xschem.tcl`: patches of xschem symbols so xschem writes VACASK netlists).
  `ng2vc` does not read sources (`V`, `I`) or `.tran` cards, only models and subcircuits;
  it reads no `M` cards, no BSIM4 and no binned models either (section 2, sky130).
- **xschem cannot netlist for VACASK here:** xschem has a Spectre/VACASK netlister, but it
  needs `spectre_format=` in each symbol; the IHP PDK symbols have none (VACASK's
  `sg13g2tovc.py` patches them *in the PDK folder*). CDePT does not change PDK files, so
  the testbench is netlisted as SPICE and translated (section 3).
- **VACASK also ships OSDI models** in `<prefix>/lib/vacask/mod` (`spice/*.osdi`:
  `bsim3v3`, `bsim4v8`, `mos1`..`mos9`, `bjt`, `diode`, `jfet`, ...; `psp103v4`,
  `bsimbulk106`, `vbic_1p3*`), and the sources in `<prefix>/src/vacask/devices`.

## 2. Models: the PDKs have no VACASK ones

No installed PDK brings `libs.tech/vacask` (ihp-sg13g2, sky130A/B have `ngspice`, `xyce`,
`verilog-a` or only `ngspice`). So CDePT makes them, once per technology:

`cdept sim run … ` (or `cdept vacask models [--force]`) does, for a technology with
`[models.ngspice]`:

1. Reads the library files of the ngspice set and the files its corner files include
   (only for the sections some corner uses, the statistical ones too), hashes them with
   the Verilog-A sources.
2. Converts each with VACASK's `ng2vc` (a separate `python3 -I convert.py` process,
   `libcdept/src/vacask/convert.py`) into `<project>/.cdept/osdi/<pdk>-<hash>/models/`.
   PDK-specific settings (a profile for `ihp-sg13g2`) are the ones of VACASK's own
   `sg13g2tovc.py`, plus: a `!=` that `ng2vc` cannot split is rewritten (`x != 1` is
   `abs(x-1)>0.5`), a second definition of `swsoa` is dropped, `level=54.0` is read as 54,
   BSIM4 (levels 14 and 54) is mapped to `spice/bsim4v8.osdi`. Corner files keep their
   `.lib` sections as `section` blocks. **`gauss()` and `agauss()` become reads of the
   Monte Carlo draws** (section 5). A library parameter that several files define alike
   (`num_sigmas` of every `_stat` file) moves into `[models.vacask] options` as one
   `parameters` line, because VACASK refuses a parameter defined twice.
3. Compiles the Verilog-A models behind the set's `osdi` entries (`psp103`,
   `psp103_nqs`, `r3_cmc`, `mosvar`, `cap_cmomi`, `cap_cmomf`) with OpenVAF-r into the same
   folder, found as `<verilog_a>/<stem>/<stem>.va`; a compiled library that exists is not
   compiled again (the folder name has the hash).
4. Writes the `ModelSet` into the snapshot: `[models.vacask]` in `tech.toml`, `generated =
   "from [models.ngspice] with … hash …"`, `dir`/`osdi` as `$CDEPT_PROJECT/.cdept/osdi/…`
   (`$CDEPT_PROJECT` is the project folder), the corner sets whose files converted (all
   of them on IHP, `mc`, `*_mm` and the hand-written `mc_mm` included), `options` with raw
   VACASK lines (`parameters swsoa=0`, the shared parameters, the default models), and
   notes `vacask: …` for everything that could not be done. What the statistical models
   draw is kept next to the converted files (`cdept-vacask.json`). A changed PDK, CDePT
   converter or Verilog-A source gives another hash and a new set replaces the generated
   one; a `[models.vacask]` without `generated` (written by hand, or one day by a PDK) is
   never touched. `cdept tech refresh` rewrites tech.toml and drops the set; the next run
   makes it again from the cache. Several tests of a suite ask for the set at once: one
   makes it, the others wait.

A testbench whose netlist calls a subcircuit or model that no converted file defines is
refused before VACASK starts, with the technology and the device named:

```
the technology 'ihp-sg13g2' has no VACASK model for 'sg13_hv_svaricap' (instance x1): no converted model file defines it
  what the conversion could not do:
    vacask: sg13_hv_svaricap (in sg13g2_svaricaphv_mod.lib) cannot be simulated: it uses a node voltage v() in an expression, a behavioural element VACASK does not have
```

### PDK status

Checked with one instance of each (`libcdept/tests/real_vacask.rs`,
`the_devices_of_the_pdk_run_or_are_refused_by_name`), `op` analysis, typical corner (`tt`):

| PDK / device | VACASK | Notes |
|---|---|---|
| ihp-sg13g2 `sg13_lv_nmos/pmos`, `sg13_hv_nmos/pmos` | works | PSP103 (`psp103.osdi`, compiled here); the inverter, the OTA and the test templates agree with ngspice (section 6) |
| ihp-sg13g2 `rsil`, `rppd`, `rhigh` | works | r3_cmc; mismatch draws with `mm_ok=1` |
| ihp-sg13g2 `cap_cmim`, `cap_rfcmim`, `cap_cmomi/f` | works | |
| ihp-sg13g2 `npn13G2`, `npn13G2l`, `npn13G2v`, `pnpMPA` | works | VBIC (`vbic_1p3_5t.osdi`, VACASK's own), mismatch draws with `mm_ok=1` |
| ihp-sg13g2 `npn13G2_5t`, `npn13G2l_5t`, `npn13G2v_5t` | works | the same model with a fifth (thermal) terminal; run with `t` at ground |
| ihp-sg13g2 diodes, ESD, `dantenna`, `schottky_nbl1`, `sg13_moscap_n/p` | works | |
| ihp-sg13g2 `bondpad` | works | an empty subcircuit in `[models.ngspice] includes`, which no corner includes: the symbol's `.include .../sg13g2_bondpad.lib` in the netlist is replaced by the converted file of the set (any `.include` whose file name is a converted file of the technology is; other `.include`s are refused, section 3) |
| ihp-sg13g2 `sg13_hv_svaricap` | **refused, by name** | the model's well resistor is `r = f(v(2,4))`, a node voltage in a resistor expression; VACASK has no behavioural element and no `v()` in expressions. An OpenVAF route exists (a voltage-dependent resistor written in Verilog-A, the expression being linear in the voltage) but is not built; the device has to be rewritten, not faked |
| ihp-sg13g2 Monte Carlo / mismatch corners (`mc`, `*_mm`, `mc_mm`) | works | section 5 |
| sky130A / sky130B | **no** | see below |
| any PDK without `[models.ngspice]` | error | "has no ngspice models to make VACASK models from" |

Not tested: the `*_NX_vbic` variants, `sg13g2_stdcell` digital cells (their spice models
are not in the model set), the XFAB PDK (NDA, not read).

#### sky130

`sky130A` and `sky130B` use the same files. The reasons the conversion stops, found by
running `ng2vc` on them (`cdept vacask models` prints the first ones):

1. `ng2vc` has no BSIM4 (SPICE levels 14 and 54) in its `family_map`; CDePT now adds it
   (`spice/bsim4v8.osdi`, which VACASK ships: **an OSDI BSIM4 exists**), and it reads
   `level = 54.0` and `version = 4.5` (it wanted integers).
2. `ng2vc` does not convert **`M` cards** ("Dont' know how to process instances of type
   'm'"): sky130 writes its transistors as `m... d g s b model` inside subcircuits.
3. The MOS models are **binned** (`.model nfet.0 ... lmin lmax wmin wmax`, instances name
   the unbinned model): `ng2vc` does not make VACASK's binned models ("Model not found").
4. `.param x = {a} + expr` (an expression after the braces) in `parameters_*.spice` gives
   "not enough values to unpack"; the resistor subcircuits use a model that is defined
   after them ("Model not specified"); the Monte Carlo parameters use upper-case `AGAUSS`
   and `EXP(AGAUSS(...))` inside `.param` with the `MC_PR_SWITCH` correlation.

So `ng2vc` cannot convert sky130 as shipped; what would be needed is a converter for `M`
cards and binned BSIM4 models (or a sky130 profile in `ng2vc`), after which the models
could use `bsim4v8.osdi`. Nothing is faked: `sim run` says "has no usable VACASK models"
with the reasons.

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
- **Errors naming the line:** `B` sources (a behavioural source has no VACASK
  equivalent: `flash3/inl_dnl` runs with ngspice only), `K`, `F`/`H`, `S`/`W`, `.ic`,
  `.nodeset`, `.include`/`.lib` of model files (ngspice model files cannot be read by
  VACASK; the technology's `vacask` set brings the converted ones, and an `.include` of
  a file it converted, like the bond pad's, becomes that file), `^` in expressions, a
  `pulse` without all seven values (VACASK has no defaults for rise, fall, width,
  period), `ic=` on passives is dropped with a warning.
- Names keep their case (SPICE is case blind, VACASK is not): a testbench that writes
  `VDD` in one place and `vdd` in another works in ngspice only. The names the setup
  writes (a noise or DC sweep source, `save @n.xdut.xm1.nsg13_lv_pmos[vds]`) are looked up
  in the netlist and the model files without regard to case, and an instance's parameter
  names follow the case of the converted subcircuit (`Nx=1` of a `npn13G2` is `nx=1`:
  ng2vc lower-cases parameters). Characters VACASK does not take in names become `_xNN`.

## 4. The deck and the results

`cdept sim deck [--simulator vacask]` prints it. For each run point: `ground 0`,
`include "builtins.inc"`, the `load`s of the model set and of the builtin devices the
netlist uses, the model set's raw lines, the Monte Carlo draw vectors when the models
read them, the `include "…" section=…` of the corner (a section that includes a
subcircuit an earlier line defined is written out without that include: "the first
definition wins", as in ngspice), design variables as `parameters`, the translated
netlist, and `control` with `options temp=… <setup options>`, `save`, and the analyses:
`op`, `dc` (`sweep` + `op`, end point included), `ac`, `tran`, `noise`
(`out="node" in="source"`). A transient that saves from a start time is refused when the
setup loads. Setup options are passed as written (VACASK's names, not ngspice's).
`save = ["all"]` is VACASK's `save default`; `@n.<path>[var]` is `p('<path>', var)`.

Raw files: `<analysis>.raw` per analysis; `out.raw` is a copy of the first analysis that is
not `op`; `out.log` is VACASK's output. Several analyses of one kind in a setup are named
`tran1`, `tran2`, …. For a **noise** analysis the copy has ngspice's two spectra added:
`onoise_spectrum = sqrt(onoise)` and `inoise_spectrum = sqrt(onoise / gain)` (V/sqrt(Hz)),
which the `noise` template reads; `noise1.raw` is VACASK's own. The calculator finds a
source current as `vdd:flow(br)` and a hierarchical name `xdut.tail` as `xdut:tail`.

**Measurements.** VACASK has no `.meas`. The setup's `measures` (ngspice syntax) are
evaluated by `vacask::meas` on the raw file: `trig`/`targ` delays (`val`, `rise`/`fall`/
`cross`, `last`, `td`), `max`, `min`, `pp`, `avg`, `rms`, `integ` (`from`/`to`), `find
<vec> at=` and `find <vec> when <vec2>=<val> [rise|fall|cross=N]`. `deriv`, `param` and
anything else is an error naming the measure (the calculator, `[[output]]`, does what the
templates need). The raw reader (`vacask::raw`) also reads ngspice/Xyce files (real,
complex, binary, ASCII).

**Cancel.** A cancelled run point kills the VACASK process (the job's cancel flag goes to
`Job::run_watched`, as for Xyce); tested with a fake slow program
(`a_cancelled_job_kills_vacask`). There is no time limit for any simulator. OpenVAF
compiles are not cancellable.

## 5. Monte Carlo

VACASK has no random numbers and no `.control`. The IHP statistical models call `gauss()`
(process variation, one draw per parameter per circuit) and `agauss()` (mismatch, one draw
per instance parameter, only for instances with `mm_ok=1`). So CDePT draws:

- **The converter** (`convert.py`) rewrites each call in the model files. A call outside a
  subcircuit becomes `A + A*B/N*mcz[slot]` (`gauss(A, B, N)`) or `A + B/N*mcz[slot]`
  (`agauss`), `slot` numbering the technology's parameters (`mc_sg13g2_lv_nmos_vfbo`, ...;
  138 on IHP). A call inside a subcircuit reads `mmz[mmk*mmnz + slot]`: `mmk` is a
  parameter of the subcircuit (default -1: no draw, the nominal value), `slot` numbers
  the calls of that subcircuit (`w`, `l`, `delvto`, `factuo` of a MOS; the three
  `nsmm_*` of a resistor). A call whose third argument (the number of sigmas, here
  `mm_ok != 1 ? 0 : 1`) is 0 gives the nominal value, as in ngspice.
- **The deck** of a Monte Carlo point defines the vectors `mcz` (length: the technology's
  parameters) and `mmz` (length: devices times the slots of the most complex device) from
  `vacask::stat::normal(seed, stream, index)`: a counter-based generator (splitmix64,
  Box-Muller) of the run's seed and the index, so a draw depends on nothing but its
  index. The run's seed is the one every simulator gets (`deck::mc_seed(base seed,
  run)`): the same `--seed` and run give the same numbers on every machine, whatever the
  number of devices or runs.
- **`mmk` per device:** `vacask::stat::instrument` numbers the leaf devices of the
  flattened testbench depth first and gives every instance of a model subcircuit, and of
  a user subcircuit that holds some, an `mmk` (user subcircuits get an `mmk` offset
  parameter), so a cell instantiated twice draws twice.
- **Modes** are the corner sets of `sim::mc::plan`, as for ngspice: `process` is `mc`
  (the `_stat` sections; resistors with `mm_ok=1` also take their mismatch, because their
  one model file has both, as in ngspice), `mismatch` is `tt_mm` (the `*_mismatch`
  sections: `w`, `l`, `delvto`, `factuo` of every MOS with `mm_ok=1`), `both` is the
  hand-written `mc_mm` of the example's tech.toml. ngspice keeps the first definition of a
  subcircuit, so `mc_mm` lists the mismatch device models before the statistical corner;
  VACASK refuses a second definition, so the deck builder drops the later one.
- A corner run without Monte Carlo (`tt_mm`, `mc` as plain corners) has every draw at 0:
  the nominal circuit (ngspice would draw unseeded numbers).
- `unif`, `aunif` and `limit` are not converted (the file is left out with a note); the
  IHP models do not use them.

Statistics against ngspice (`examples/ihp-inv/libs/cells/tb_inv/montecarlo`, tpd at 0.5 V,
seed 1; the generators differ, so only the distribution agrees, never the runs):

| runs | mode | ngspice mean / std | VACASK mean / std | difference |
|---|---|---|---|---|
| 200 | process | 54.884 ps / 4.153 ps | 53.954 ps / 3.952 ps | mean 1.7 %, std 4.8 % |
| 100 | mismatch | 54.124 ps / 1.442 ps | 54.194 ps / 1.233 ps | mean 0.1 %, std -14.5 % |
| 100 | both | 54.949 ps / 3.914 ps | 53.873 ps / 4.278 ps | mean -2.0 %, std 9.3 % |

The seed 1 of VACASK is the low one: with seeds 2 to 5 (200 runs each) the VACASK means are
54.57, 54.70, 54.91, 54.87 ps and the stds 4.42, 4.45, 4.20, 4.51 ps, which brackets
ngspice's 54.88 ps / 4.15 ps; 100 or 200 runs measure a standard deviation to about 5 to 7
%, a mean to 0.7 %. `real_vacask.rs` asserts mean within 2 % and std within 25 % of the ngspice reference (the
200-run numbers above), that the same seed gives the same twelve numbers, another seed
other ones, and that mismatch spreads less than the process, and both more than mismatch.

## 6. Parity with ngspice

Every row was run for real (`libcdept/tests/real_vacask.rs`, plus the manual runs noted),
IHP SG13G2, `examples/ihp-inv`, ngspice (PSP103 through OSDI, `/opt/ext/OpenLane/bin/ngspice`)
against VACASK 0.3.2 with the PSP103 that OpenVAF compiled.

| area | ngspice | VACASK | status |
|---|---|---|---|
| `op` | yes | yes | same (`dc_op` of the OTA: all five outputs within 1e-8) |
| `dc`, source sweep | yes | yes (`sweep` + `op`, end point included; a `pulse` or `sin` source moves its `val0`/`sinedc`) | same (`dc_transfer`: identical to 5 digits) |
| `dc`, nested sweeps | not in `[[analysis]]` (use `[[sweep]]`) | the same | same |
| `ac` | yes | yes | same (`ac_response`, OTA `ac_open`/`psrr`/`cmrr`: 1e-6 or better) |
| `tran` | yes | yes (a start time is refused when the setup loads) | agrees within the step control (below) |
| `noise` | yes | yes (`out=`/`in=`; spectra added for the templates) | same (`noise` template: 0.005 %) |
| temperature | `.temp`/`options temp` | `options temp=` | same |
| corners | model sets | the generated set has every corner of ngspice's (`tt ss ff fs sf`, `*_mm`, `mc`, `mc_mm`) | same |
| `[variables]` | `.param` | `parameters` | same |
| `[options]` | ngspice names | VACASK's own names | different names, passed as written |
| `[[sweep]]` | yes | yes (one point per combination) | same |
| Monte Carlo process / mismatch / both | seeded by the deck | CDePT's own draws, section 5 | same distributions, other numbers |
| seeds, statistics, yield | yes | yes (the same results store) | same |
| `measures` | `.meas` in ngspice | evaluated on the raw file | `trig/targ`, `max min pp avg rms integ`, `find at`, `find when`; no `deriv`/`param` |
| calculator `[[output]]` | yes | yes, real and complex (AC), noise | same |
| specs, results database, stats, suites | yes | yes | same (`--simulator`, suite `simulator`) |
| templates | all | all eleven built in run; `inl_dnl` of the example needs `B` sources: refused when the deck is built, naming the line | see below |
| OTA suite (examples/ihp-inv/libs/cells/ota5t) | yes | all six tests, typical corner | see below |
| job queue: parallel | yes | yes (one process per point) | same |
| job queue: cancel | kills the process | kills the process | same |
| job queue: time limit | none | none | same |
| in-process (libngspice) | `simulator_backend = "inproc"` | not applicable (process only; `caps.in_process` says so) | n/a |
| PDKs | ihp-sg13g2, sky130A/B | ihp-sg13g2 only | sky130: section 2 |
| devices | all | PSP103 MOS, `bondpad`, `sg13_hv_*`, VBIC `npn13G2*` and `_5t`, resistors, capacitors, diodes: yes; `sg13_hv_svaricap`: refused by name | section 2 |

Differences of the tests, typical corner (VACASK against ngspice; the allowed tolerance of
`real_vacask.rs` per measurement in brackets):

| test | measurement (tolerance) | observed difference |
|---|---|---|
| `tb_inv/tran_delay` | `tpd`, `tpd_rise`, `tpd_fall` (0.5 %) | 0.09 %, 0.20 %, 0.01 % |
| | `t_rise`, `t_fall`, `slew_rise`, `slew_fall` (6 %) | 3.2 %, 2.7 %, 3.1 %, 2.6 % (5.2 % at `ss`) |
| | `overshoot` (12 %) | 9.2 % of 4 % (an absolute 0.4 point) |
| `tb_inv/dc_transfer` | all (0.5 %) | 0.000 % |
| `tb_inv/power` | `i_avg`, `p_avg`, `e_cycle` (2 %) | 1.3 % |
| | `i_peak`, `p_peak` (2 %) | 0.06 % |
| `tb_inv_ac/ac_response` | all (0.5 %) | 0.005 % |
| `ota5t/ac_open`, `psrr`, `cmrr`, `dc_op` | all (0.5 %) | below 1e-6 |
| `ota5t/ac_cload` (five loads) | all | identical to 5 digits |
| `ota5t/tran_step` | `settle_time`, `t_rise` (6 %) | 1.2 %, 3.5 % |
| | `overshoot_pct` (12 %) | 9.2 % (8.93 against 8.18) |
| `ota5t/power` | `i_avg`, `p_avg`, `e_cycle` (2 %) | 0.001 % |
| | `i_peak`, `p_peak` (2 %) | 0.5 % |
| `thd_fft` on an inverter | `thd_db`, `enob` (5 %) | 1.8 % |
| `noise` on an inverter | all (0.5 %) | 0.005 % |

Every difference above 5 % is in the transient edge measurements (`overshoot`, `t_fall`
at `ss`): both simulators integrate with default tolerances (`reltol` 1e-3) and choose
their own steps. With `reltol = 1e-5` in both, the unity-gain step of the OTA gives
`overshoot_pct` 8.233 (VACASK) against 8.183 (ngspice), 0.6 %, `settle_time` 9.205 ns
against 9.192 ns, `t_rise` 3.660 ns against 3.653 ns: the same circuit, the difference is
integration error, not a model difference. Both simulators run PSP103 from the same Verilog-A
(the PDK's own `.osdi` for ngspice, one compiled here for VACASK), and no model difference
showed in the DC, AC and noise results, which agree to 1e-6 or better.

Earlier results: a level-1 MOS inverter with a diode agrees with ngspice within the test's
5 %. RC: delay 7.35e-10 as with Xyce, `ac` follows `1/sqrt(1+(f/fc)^2)`, `noise` gives the
thermal noise of the resistor, a DC sweep reaches its end point.

Gaps that remain, and what they need:

- **sky130A/B**: a converter for `M` cards and binned BSIM4 models (section 2).
- **`sg13_hv_svaricap`**: a Verilog-A voltage-dependent resistor through OpenVAF.
- **`B` sources** (behavioural: `flash3`), `F`/`H`, `K`, `.ic`/`.nodeset`: no VACASK
  equivalent written; the translator names the line.
- **`measures`**: `deriv`, `param`.
- **Cancelling an OpenVAF compile**.
- **Time limit** for a run point (none for any simulator).

## 7. OpenVAF

`libcdept/src/batch/openvaf.rs`: `$CDEPT_OPENVAF`, else `openvaf-r` or `openvaf` on PATH;
`openvaf-r -o out.osdi [-D m] [-I dir] model.va`. `cdept openvaf compile model.va [-o
out.osdi] [-D …] [-I …]` caches into `<project>/.cdept/osdi/<pdk>-<hash>/<stem>.osdi`
(hash of the `.va`/`.include`/`.vams` files next to the source, the flags and the
compiler) and prints the path; without a project give `--out`. PSP103 compiles in about
3 s. The PDK's own `ngspice/osdi/*.osdi` files are the same models and are what ngspice
uses; they could be loaded by VACASK too (same OSDI), but the generated set compiles its
own so that the result follows the Verilog-A source and the hash.

## 8. Open

- A PDK that brings `libs.tech/vacask` (the generated set steps aside only if one writes
  `[models.vacask]` without `generated`; scanning for the PDK's own is not done).
- Corner/temperature points are one VACASK run each (no `sweep` in the deck for them);
  parallel runs share the generated cache read-only.
- `hb`/`acxf` analyses, `.ic`/`.nodeset`, `F`/`H`/`B`, `$mfactor` on sources.
- The Simulation tab form still has to follow the changes (Monte Carlo is no longer
  refused for VACASK; `SimKind::capabilities()` tells what each simulator supports).
