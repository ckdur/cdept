# Use another simulator

`simulator` in `setup.toml` selects the engine. The tools themselves are described in
[NGSPICE.md](../NGSPICE.md) (default), [VACASK.md](../VACASK.md) and [QUCS.md](../QUCS.md);
this page says what CDePT supports and what it refuses.

| setting | engine | program |
|---|---|---|
| `simulator = "ngspice"` (or empty) | ngspice | `$CDEPT_NGSPICE`, else `ngspice` on `PATH` |
| `simulator = "xyce"` | Xyce | `$CDEPT_XYCE`, else `Xyce` on `PATH` |
| `simulator = "vacask"` | VACASK | `$CDEPT_VACASK`; OpenVAF is `$CDEPT_OPENVAF` |

Any other value is an error. The Qucs-S schematic editor is not a `simulator` value: it
is a schematic source, below.

## Xyce

```toml
simulator = "xyce"
```

- The model set is the technology's `xyce` set unless `models` names another; the model
  plugins listed in tech.toml are passed with `-plugin`.
- One analysis per setup (plus `op`); split a testbench that needs two.
- Options are written `package.name` (`timeint.reltol`); a bare name goes to `device`.
- Corners, temperatures, sweeps and calculator outputs work as with ngspice.
- Monte Carlo: `process`, `mismatch` and `both`, with the same seeds, corner sets and
  statistics as ngspice (Xyce's own random numbers, so the same seed is not the same
  circuit as in ngspice). The IHP Xyce models have no transistor mismatch, so CDePT
  writes models that have it (`.cdept/xyce-mm/` in the project). Try
  `cdept sim run cells tb_inv --view mismatch --simulator xyce`; see also the `xyce_mc`
  view of `tb_inv`. Only the MOS transistors have mismatch.

## VACASK

```toml
simulator = "vacask"
```

```bash
cdept sim run cells tb_inv --view vacask
```

- VACASK reads its own netlist format and OSDI models. The first run converts the
  technology's ngspice models and compiles the Verilog-A with OpenVAF into
  `<project>/.cdept/osdi/` and records them as `[models.vacask]` in tech.toml.
- Works on ihp-sg13g2: the inverter's tests, the OTA suite and the test templates agree
  with ngspice (most measurements within 0.1 %, transient edges within a few percent; the
  table is in [VACASK.md](../VACASK.md#6-parity-with-ngspice)). Not on sky130A/B: VACASK's
  converter does not read their binned BSIM4 models, so VACASK is *not available* there:
  the Simulation tab shows it disabled (the reason is the hover text), a testbench that
  names it gets a problem line, and `cdept sim run|suite` stop at once with the reason
  ([availability rules](../VACASK.md#4b-availability-per-technology); `cdept tools`
  prints the line for the project's technology).
- Without OpenVAF the run stops before VACASK starts: `OpenVAF not found: put openvaf-r
  on PATH or set CDEPT_OPENVAF; needed to compile psp103, ...`.
- Any testbench runs with it without editing the file: `cdept sim run cells tb_inv
  --simulator vacask` (also `sim deck` and `sim suite`, and `simulator = "vacask"` in a
  `suite.toml`).
- Monte Carlo works (process, mismatch, both): CDePT makes the random draws itself, seeded
  by `seed` and the run, so a run repeats; ngspice's draws for the same seed are other
  numbers of the same distribution ([Monte Carlo](monte-carlo.md)).
- Netlist limits (B, K, F/H sources, `.ic`, `.include` of model files) are listed in
  [VACASK.md](../VACASK.md); the varicap `sg13_hv_svaricap` is refused by name.
- `.meas` is not available in VACASK: the setup's `measures` are evaluated by CDePT on
  the raw file (`trig`/`targ`, `max`, `min`, `pp`, `avg`, `rms`, `integ`, `find ... at=`,
  `find ... when`); calculator `[[output]]` expressions work on its raw files, noise and
  AC (complex) included.
- A cancelled run point kills VACASK, and so does the time limit: every simulator's run
  point has one, `timeout = "10m"` in the testbench's `setup.toml`, `[sim] timeout` in
  `cdept.toml` (default 30 minutes), `cdept sim run --timeout 10m` (`0` is none). The point
  then ends as an error `run point exceeded its time limit of 10m` with the last lines of
  the log. OpenVAF compiles have `[sim] openvaf_timeout` (default 20 minutes).

## Qucs-S schematics

```toml
schematic = "cells/rc_lowpass"
schematic_kind = "qucs"
simulator = "ngspice"        # or "xyce": picks the netlist flavour
```

CDePT netlists the schematic with `qucs-s -n` (`$CDEPT_QUCS`), removes the schematic's own
analysis and builds the deck as for xschem, so `[[analysis]]`, measures, corners, sweeps
and Monte Carlo apply. `cdept sim run cells rc_lowpass` is the example.

## See also

- [setup.toml](../reference/files/setup-toml.md), [Monte Carlo](monte-carlo.md)
- [Environment variables](../reference/environment.md)
- [NGSPICE.md](../NGSPICE.md), [VACASK.md](../VACASK.md), [QUCS.md](../QUCS.md)
