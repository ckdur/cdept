# CDePT: test templates and suites

Status 2026-11-23: ten built-in test templates, project and technology templates,
`cdept sim templates | new | suite`, suite views, the template picker and the suite
table in the Simulation tab. Code: `libcdept/src/sim/template.rs` (+ `templates/*.toml`),
`sim/starter.rs`, `sim/suite.rs`, `cdept-gui/src/views/testlib.rs`.

The idea is the one of a circuit designer's family of tests ("measure the delay", "measure
the AC response", ...) with the calculator ([CALC.md](CALC.md)) doing the measuring. A
**template** says which analyses to run, which `[[output]]` expressions to evaluate, which
specs to apply and what the schematic must have; a testbench's `setup.toml` picks it and
fills its parameters. A **suite** runs several testbenches (the family) together.

## 1. Using a template

```toml
# <lib>/<cell>/<view>/setup.toml
template = "ac_response"
corners = ["tt", "ss"]            # explicit fields override the template
[template_args]
in = "in"
out = "out"
inverting = "yes"
gain_db_min = 15                  # a spec: the parameter turns it on
```

`cdept sim templates` lists them; `cdept sim templates show ac_response` documents the
parameters (type, default, unit, doc), the derived values, the outputs as they read with
the defaults, the default specs, what the schematic must have and what to plot.

`cdept sim new <lib> <cell> <view> --template T [--arg k=v ...] [--dut lib/cell]` creates
the view. With `--dut` (the DUT's xschem symbol must exist) it also writes the cell's
`schematic/<cell>.sch`: the DUT, a labelled wire on each pin (input and output pins are
matched by name, else by direction; `vdd*` pins go to `VDD`, `vss*`/`gnd*` to `0`), a
supply source `Vdd = {vdd}`, an input source `Vin` whose value is the template's
`stimulus` (a pulse, `dc {vbias} ac 1`, a sine at the coherent tone, ...) and a 10 fF load.
The design variables the sources use go to `[variables]` of the new `setup.toml`
(`vbias = 0.6`, ...). Without `--dut` only `setup.toml` is written, for the cell's
existing schematic. The starter is a starting point: bias, load and the stimulus levels
are yours to adjust. Example: `examples/ihp-inv/libs/cells/tb_inv_ac`.

### Merging

The template's fragment is filled in (`${param}` in every string; a spec bound that is
exactly `${param}` becomes a number), then the explicit fields of `setup.toml` go on top:
`analysis` by `kind`, `output` and `spec` by `name`, `sweep` by `variable` (an entry of
the same key replaces the template's, others are added), `variables` and `options` key by
key, everything else (`corners`, `temperatures`, `measures`, `save`, ...) replaced as a
whole.

A template can bring design-variable sweeps and Monte Carlo of its own (`[[sweep]]` and
`[montecarlo]` at the top of the template file, or inside `[setup]`), with `${param}` in
their values; the setup's own `[[sweep]]` (by `variable`) and `[montecarlo]` (as a whole)
win, as above. See section 2.

### Errors

All say what is wrong and what is possible: an unknown template or parameter (with a
did-you-mean and the list), a value that is not a number / net name / one of the choices,
a missing required parameter, an unknown field of the setup, an expression that does not
parse (with a caret). At run time, after netlisting, **the nets and elements the template
requires are checked** against the netlist: `template 'tran_delay' needs more than this
schematic has: net 'vout' does not exist in the netlist (nets: 0, in, out, vdd)`.

## 2. Template schema

A template is a TOML file with its own schema (unknown keys are errors):

```toml
name = "ac_response"                       # [A-Za-z0-9_-]
description = "AC response of out over in: ..."
requires_nets = ["${in}", "${out}"]        # must exist in the netlist ("0" always does)
requires_elements = ["${supply}"]          # sources / instances that must exist
plot = ['db20(v("${out}") / v("${in}"))']  # what to show (calculator expressions)
stimulus = "dc {vbias} ac 1"               # input source value of a `sim new --dut` schematic
starter_variables = { vbias = "0.6" }      # design variables that stimulus uses

[[param]]
name = "out"
type = "net"        # net | string | number (SPICE suffixes) | int | bool | choice
default = "out"     # absent: required, unless optional = true
optional = false    # unset: what uses it is dropped
choices = []        # for type = "choice"
unit = ""
doc = "The output net."

[[derived]]         # computed from the parameters, in order
name = "fin"
expr = "coherent(tone, fs, n)"             # calculator expression over the parameters
doc = ""
[[derived]]
name = "sign"
from = "inverting"                         # or: look a parameter up
map = { no = "1", yes = "-1" }

[[sweep]]           # design-variable sweeps from the parameters (optional)
variable = "${load}"
values = "${loads}"                        # exactly ${param}: its comma/space separated words
[[sweep]]
when = "supply_sweep"
variable = "vdd"
start = "${vlo}"                           # start/stop/step: strings become numbers
stop = "${vhi}"
step = "0.1"
[montecarlo]        # Monte Carlo from the parameters (optional)
when = "mc_runs"                           # off unless the parameter is set
runs = "${mc_runs}"                        # runs/seed: strings become integers
seed = "${mc_seed}"
mode = "process"

[setup]             # the setup.toml fragment: any field of docs/SIMULATION.md §1
[[setup.analysis]]
kind = "ac"
points = "${points}"
[[setup.output]]
name = "gain_db"
expr = 'db20(value_at(${sign} * v("${out}") / v("${in}"), ${fstart}))'
plot = "ac"
[[setup.output]]
when = "margins"                           # keep this entry only if the condition holds
name = "pm"
expr = '...'
[[setup.spec]]
name = "gain_db"
min = "${gain_db_min}"                     # unset optional parameter: the bound is left out,
                                           # a spec without a bound is dropped
```

- A parameter default may use earlier parameters (`default = "${vdd}"`).
- `when` (on an entry of an array in `[setup]`): `name` (set and not false), `!name`,
  `name=value`, `name!=value`.
- An optional parameter left empty is *unset*; using it inside a longer string is an error,
  alone as a value it removes the key.
- Output expressions see the earlier outputs and the setup's `[variables]` by name.
- `[[sweep]]` and `[montecarlo]` have the schema of `setup.toml`'s. In a sweep, `start`,
  `stop` and `step` written as strings (`"${vlo}"`, `"1p"`) become numbers, and `values`
  that is exactly `"${param}"` becomes the list of that parameter's comma or space
  separated words (`loads = "50f, 100f"` gives `["50f", "100f"]`); a sweep whose values
  come from an optional parameter left unset is dropped. In `[montecarlo]`, `runs` and
  `seed` become integers (unset: the default). Both take `when`. The template's sweeps
  come after the ones of its `[setup]`; its `[montecarlo]` replaces the one of its
  `[setup]`. Example: `examples/ihp-inv/templates/ac_load.toml` (a sweep of the load
  capacitance, Monte Carlo with `mc_runs`), used by `cells/ota5t/ac_cload`.

**Where templates come from**, later ones shadow earlier ones of the same name (a note
says so in `cdept sim templates`):

1. built in (`libcdept/src/sim/templates/*.toml`, embedded with `include_str!`);
2. the technology: `pdk/<technology>/templates/*.toml` (the folder of the PDK's
   `devices.toml`: `$CDEPT_PDK_DATA`, `<prefix>/share/cdept/pdk`, or the development tree);
3. the project: `<project>/templates/*.toml`.

A template file that does not load is listed as a note, the others still work.

## 3. Built-in templates

All parameters have defaults for a 1.2 V circuit with nets `in` / `out` and supply source
`vdd`. `vdd` (where present) also sets the design variable `vdd`. Optional `*_min` /
`*_max` parameters switch on specs on the output of the same name.

| template | analysis | outputs |
|---|---|---|
| `dc_op` | op | `vout` (and `v2` with `node2`), `isupply`, `power` |
| `dc_transfer` | dc sweep of `src` | `vout_start`, `vout_end`, `vtrip` (out crosses level x vdd), `vm` (out = in), `gain_max` (max abs d vout/d vin), `lin_err` (percent of swing off the end-point line) |
| `tran_delay` | tran | `tpd_rise`, `tpd_fall`, `tpd` (mean), `t_rise`, `t_fall` (10-90 %), `slew_rise`, `slew_fall`, `overshoot` (percent of vdd beyond the rails) |
| `ac_response` | ac | `gain_db` (at `fstart`), `bw_3db`, `ugbw`, `pm` (`margins`), `gm` (`gain_margin`), `peaking_db`; `inverting = yes` takes -out/in |
| `noise` | noise | `spot_out`, `spot_in` (V/sqrt(Hz) at `fspot`), `int_out`, `int_in` (V rms over `flo`..`fhi`); `src` needs an AC value |
| `thd_fft` | tran, N samples at fs | `fin_hz`, `thd_db`, `thd_pct`, `sndr_db`, `sfdr_db`, `enob`. `fin` = `coherent(tone, fs, n)` (the nearest odd number of cycles in N samples), `tstep` = 1/fs, the run is `t_skip + n/fs` long and analysed with a rectangular window; the schematic's sine uses `{fin}` |
| `psrr_cmrr` | ac | `xfer_db` (out / `stim` at `f`), `psrr_db` or `cm_gain_db` by `mode`, `rejection_db` = `adm_db` - `xfer_db` when the differential gain is given. The AC stimulus must be on the supply (psrr) or the common-mode node (cmrr) |
| `power` | tran | `i_avg`, `i_peak`, `p_avg`, `p_peak` over `t_from`..`t_to`, `e_cycle` = `p_avg` x `period` |
| `inl_dnl` | dc (`method = edge`) or tran ramp (`hist`) | `inl_lsb`, `dnl_lsb` (largest, LSB) of the expression `code`; `lsb` optional |
| `startup_settle` | tran | `final_value`, `settle_time` (to `tol` of the step, from `t_step`), `overshoot_pct`, `undershoot_pct`, `t_rise`, `final_err` (with `target`) |

`coherent(fin, fs, n)` is a calculator function too (`cdept calc --functions`).

The ngspice deck writes both plots of a noise analysis (`noise1` the spectra, `noise2` the
totals) to `out.raw`, so `sig("onoise_spectrum")` works.

## 4. Suites

A suite is a view with a `suite.toml` (any view name; `suite` by convention):

```toml
description = "inverter characterisation"
corners = ["tt", "ff", "ss"]          # replaces the corners of every test (optional)
temperatures = [27]                   # replaces their temperatures (optional)
[[test]]
view = "tran_delay"                   # a view of this cell; or cell/view; or lib/cell/view
[[test]]
view = "tb_inv_ac/ac_response"
name = "ac"                           # label in the table (default cell/view)
```

`cdept sim suite <lib> <cell> [--view V] [-j N] [--all-columns] [--json] [--no-store]`
runs every test (each on its own thread for netlisting, all points on one shared job
queue). Output: a combined table, a row per test and point (result PASS / FAIL / ERROR,
a column per measurement that some test has a spec on; `--all-columns` for every one), the
list of failing and missing specs and calculator errors, and a summary line with
`SUITE PASS` or `SUITE FAIL`. A test that cannot run (not a testbench, template or net
error, simulator missing) is an ERROR row and does not stop the others. **The exit status
is non-zero** when a test cannot run, a run fails or errors, or a spec fails or is missing.

Stored: each test's runs as usual (`result.json`, `results.db`), the suite as
`simulations/<lib>/<cell>/<suite view>/suite.json` and as a row of the `suite_runs` table in
`results.db` (counts and the whole report as JSON). API: `sim::suite::{open, run, latest,
format_report}`, `SuiteReport::summary`.

## 5. The Simulation tab

- **Test template** (above the setup form): a picker (built-in, technology and project
  templates; the description is the hover text) and one row per parameter: the name
  (doc on hover), a text box with the default as hint (a drop-down for choices and
  booleans), the unit, and the error in red when the value is not valid. A valid edit
  re-expands the template into the form at once. What the form adds on top (another
  corner set, a spec, a variable) stays; picking another template drops those additions
  except the general fields (schematic, simulator, models, corners, temperatures,
  options, variables); picking "(none)" keeps the expansion as plain fields.
- **Save** writes `template`, `[template_args]` and only the fields that differ from the
  template (so the file stays as short as the one in the example). A field the template
  provides cannot be deleted in the form, only overridden.
- **Suites**: a view with a `suite.toml` opens in the same tab (double click, or Open in
  the context menu): the list of tests, **Run suite** / **Cancel** on one job queue with
  a progress line, and the combined table (PASS green, FAIL red, ERROR amber, a measure
  that misses its spec shown in red, `all measurements` for every column), the verdict
  line and the list of what did not pass. The last stored report is shown on opening.
- Test: `cdept-gui/scripts/tests.script` (docs/GUI-TESTING.md).

## 6. Examples and tests

`examples/ihp-inv` (IHP SG13G2), run for real through xschem and ngspice:

| view | template | what it gives (tt, 27 C) |
|---|---|---|
| `cells/tb_inv/tran_delay` | `tran_delay` | tpd 43.6 ps, rise 54 ps, fall 51 ps, 4.3 % overshoot |
| `cells/tb_inv/dc_transfer` | `dc_transfer` | trip 0.62 V, peak gain 17 |
| `cells/tb_inv/power` | `power` | 15.5 uA average, 321 uA peak, 18.6 fJ per cycle (10 fF) |
| `cells/tb_inv_ac/ac_response` | `ac_response` | the inverter biased near its trip point as an amplifier: 22 dB, 306 MHz, UGBW 3.9 GHz, phase margin 87 deg (made with `sim new --dut`) |
| `cells/tb_inv/suite` | suite | the four together over tt/ff/ss: `cdept sim suite cells tb_inv` |
| `cells/ota5t/ac_open` | `ac_response` | the 5-transistor OTA open loop into 100 fF: 35.8 dB, -3 dB at 3.7 MHz, UGBW 145 MHz, phase margin 59 deg |
| `cells/ota5t/tran_step` | `startup_settle` | unity-gain 200 mV step: settles to 1 % in 9.2 ns, 8.2 % overshoot, final error 0.4 mV |
| `cells/ota5t/dc_op` | `dc_op` | out 0.422 V (common mode 0.42 V), 39 uA, smallest vds - vdsat of the six transistors 108 mV |
| `cells/ota5t/power` | `power` | 39 uA, 47 uW average over a 1 us step period |
| `cells/ota5t/psrr` | `psrr_cmrr` | out/vdd -27.6 dB at 1 kHz, PSRR 63.6 dB against the 36 dB open-loop gain |
| `cells/ota5t/cmrr` | `psrr_cmrr` | out/cm -14.4 dB at 1 kHz, CMRR 50.4 dB |
| `cells/ota5t/ac_cload` | project template `ac_load` | the load swept 50 fF to 1 pF by the template's `[[sweep]]`: UGBW 234 to 26 MHz, phase margin 56 to 78 deg |
| `cells/ota5t/suite` | suite | the six above (not `ac_cload`) over tt/ff/ss/fs/sf and -40/27/125 C: 90 runs, all pass (`cdept sim suite cells ota5t`) |
| `cells/flash3/inl_dnl` | `inl_dnl` | a behavioural 3-bit flash ADC (resistor ladder, R5 10 % high; B-source comparators): DNL 0.082 LSB, INL 0.049 LSB, as the ladder gives by hand |

The OTA (`cells/ota5t`, testbench schematics `cells/tb_ota5t` and `cells/tb_ota5t_step`):
PMOS input pair M1/M2 30/0.5 um with their bodies on the tail, NMOS mirror load M3/M4
2/4 um, PMOS tail M5 32/1 um mirrored 1:1 from M6, 20 uA into `ibias`, 1.2 V. The
open-loop testbench closes the DC loop with a 1 TH inductor from out to inn and drives
inn through a 1 F capacitor, so out/in is minus the open-loop gain, which is the loop
gain of the unity-gain follower; the AC sources on the supply and the common mode are
design variables (`ac_dm`, `ac_vdd`, `ac_cm`), so PSRR and CMRR use the same schematic.
Over the 15 corner points: gain 32.1 to 36.6 dB (lowest at 125 C), UGBW 111 to 176 MHz,
phase margin 57.9 to 62.4 deg, settling 7.8 to 11.6 ns, saturation margin at least 33 mV.
The specs (gain >= 30 dB, UGBW >= 80 MHz, PM >= 50 deg, settling <= 20 ns, overshoot
<= 15 %, final error <= 25 mV, margin >= 30 mV, 0.40 V <= out <= 0.50 V, supply
<= 60 uA, average <= 50 uA / 60 uW, PSRR >= 50 dB, CMRR >= 40 dB) leave room. NMOS input
pairs were tried first and dropped: the lv NMOS has a low Early voltage (2 to 6 V at
L = 2 to 8 um), so the gain stayed below 30 dB.

Tests: unit tests in `template.rs` (substitution, derived values, `when`, override and
merge, every error message, bad templates, netlist requirements, shadowing, every
built-in loading with its defaults), `starter.rs`, `suite.rs`, `calc/measure.rs`
(`coherent`) and `testlib.rs` (minimal overlay); `libcdept/tests/real_templates.rs` runs the
four examples and the suite for real (delay equal to the hand-written `.meas`, trip point,
power consistency, AC figures, suite counts and stored report, a failing spec and a
missing net failing the suite; skipped without ngspice, the helper or the PDK);
`libcdept/tests/real_ota.rs` loads the OTA's views, suite and the `ac_load` template
without a PDK (`mod hermetic`) and runs the OTA suite at tt, 27 C (all specs pass, gain,
UGBW and phase margin in windows), the load sweep and the flash ADC for real
(`mod real_pdk`); GUI: `cdept-gui/scripts/tests.script` and `cdept-gui/scripts/ota.script`
(the OTA suite from the Simulation tab, 90 PASS rows).

Checked by hand on the inverter, not in the tests: `dc_op`, `noise` (spot 14 uV/sqrt(Hz)
out, 1.09 uV/sqrt(Hz) in at 1 kHz), `thd_fft` (a `sim new --dut` testbench), `startup_settle`,
`psrr_cmrr` has real examples on the OTA (`cells/ota5t/psrr`, `cmrr`), `inl_dnl` on the
flash ADC (`cells/flash3/inl_dnl`).

## 7. Open

- Settling inside `tran_delay` (use `startup_settle`); jitter and PSD are not in the calculator.
- `psrr_cmrr` takes the differential gain as a number (`adm_db`), not from the
  `ac_response` test of the same suite.
- Templates for Xyce-only analyses; one template, one analysis set: a family that needs
  two stimuli (PSRR and CMRR) is two testbenches in a suite.
- The suite panel has no per-test run, and no waveform viewer (Phase 5).

## 8. Tests and the environment (Rust tests)

CDePT reads `PDK`, `PDK_ROOT`, `PDKPATH`, `HOME`, `XDG_*` and the `CDEPT_*` variables, so
a test that loads a project can be changed by the shell it runs from (with `PDK=ihp-sg13g2`
exported, a project that names another PDK warns about the variable). The Rust tests are
therefore of two kinds, both using `libcdept::testutil` (a `pub` module behind the
`testutil` feature, which `libcdept` and `cdept-gui` switch on for their own tests):

- **Hermetic** tests build a synthetic project or PDK in a scratch folder and start with
  `let _env = libcdept::testutil::hermetic_env();` (`crate::testutil` inside `libcdept`).
  While the guard lives `PDK`, `PDK_ROOT`, `PDKPATH`, `CDEPT_PDK_PATH`, `CDEPT_PDK_DATA`,
  `CDEPT_INDEX_CACHE`, `KLAYOUT_*` and `XDG_RUNTIME_DIR` are unset and `HOME` and the
  `XDG_*_HOME` folders are scratch folders (`env.scratch()`); everything is put back on
  drop. The tool variables (`CDEPT_XSCHEM`, `CDEPT_NGSPICE`, `CDEPT_KLAYOUT`, ...) are
  left as they are, tests that need a tool skip when it is missing;
  `hermetic_env_without_tools()` unsets them too and `hermetic_env_with(&[(name, value)])`
  sets more variables. Every test that calls `Project::load` / `create*`, opens an
  `AppCtx` project or uses the user's trash and caches takes the guard.
- **Real-PDK** tests need an installed PDK (ihp-sg13g2, sky130A). They live in
  `libcdept/tests/real_*.rs`, in a `mod real_pdk` of their file, or are named `real_*`
  when they also need a real Magic, and start with
  `let Some(pdk) = libcdept::testutil::real_pdk("ihp-sg13g2") else { return };`. Without
  the PDK that prints `skipped: ihp-sg13g2 not installed` and the test passes; with it the
  guard sets `PDK`, `PDK_ROOT` and `PDKPATH` to it (`pdk.root`, `pdk.dir`), whatever the
  shell has. The PDKs are looked for in `$CDEPT_TEST_PDK_ROOT`, else `/opt/ext/OpenPDKs`
  (never in `$PDK_ROOT`).

Guards of the same kind run in parallel; one that asks for another environment waits for
them, and a test must not hold two guards.

How to run them: `cargo test --workspace` (the default) runs both kinds, and the real-PDK
tests skip themselves where the PDK or a tool is absent. Only the real-PDK files:
`cargo test -p libcdept --test 'real_*'` (plus `real_pdk` to select the `mod real_pdk`
tests of `link`, `import`, `importer` and `vacask`, and of `cdept-gui`'s `pdkscan`). Another
PDK folder: `CDEPT_TEST_PDK_ROOT=/path/to/pdks cargo test --workspace`. The results do not
depend on the shell: the workspace passes with `PDK` unset, with `PDK=ihp-sg13g2` exported
and with `PDK=sky130A PDK_ROOT=/nonexistent`.
