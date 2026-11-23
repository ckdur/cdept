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
- Monte Carlo: process variation only on the IHP models; `mismatch` and `both` are refused
  unless `mismatch_corner` / `both_corner` names a corner set that has transistor
  mismatch. See the `xyce_mc` view of `tb_inv`.

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
- Works on ihp-sg13g2 (tpd within about 0.2 % of ngspice on the example). Not on
  sky130A: its MOS models do not convert, and the run says "has no usable VACASK models".
- Monte Carlo is refused when the setup loads (the statistical models are not converted).
- Netlist limits (B, K, F/H sources, `.ic`, `.include` of model files) are listed in
  [VACASK.md](../VACASK.md).
- `.meas` is not available in VACASK: the setup's `measures` are evaluated by CDePT with a
  subset (`trig`/`targ`, `max`, `min`, `pp`, `avg`, `rms`, `integ`, `find ... at=`);
  calculator `[[output]]` expressions work on its raw files.

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
