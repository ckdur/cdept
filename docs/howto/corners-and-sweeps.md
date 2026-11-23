# Corners, temperatures and sweeps

Run points are the product of corners, temperatures and design-variable sweeps
([concept](../concepts/simulation.md)).

## Corners and temperatures

```toml
corners = ["tt", "ff", "ss"]     # corner sets of the model set in tech.toml
temperatures = [-40, 27, 125]    # deg C
```

3 x 3 = 9 runs, in folders `tt_m40`, `tt_27`, `tt_125`, ... The corner names are those of
the technology's model set ([tech.toml](../reference/files/tech-toml.md)); leave `corners`
out to run the technology's default corner. On the command line, `--corner ss` (repeatable)
replaces the setup's list for one run.

## Sweep a design variable

The schematic uses the variable as `{vdd}` (in an xschem property
`value="\{vdd\}"`; unescaped braces break the property parser). The setup gives its default
and the sweep:

```toml
[variables]
vdd = "1.2"

[[sweep]]
variable = "vdd"
start = 1.0
stop = 1.4
step = 0.1          # inclusive; or values = ["1.0", 1.2, "1.4"]

[[sweep]]
variable = "cload"
values = ["1f", "5f"]
```

Sweeps cross with each other, the corners and the temperatures; the first varies slowest.
Each point is a separate simulator run with its own measurements and verdicts. A point's
folder carries the values: `tt_27_cload1f_vdd1p2`.

## Run and read

```bash
cdept sim run cells tb_inv --view sweep       # 2 corners x 2 temperatures x 5 vdd = 20 runs
cdept sim results cells tb_inv --view sweep
```

```text
20 run(s): 20 pass, 0 fail, 0 error
measure              n          min          max         mean          std
tpd                 20   5.0654e-11   7.7429e-11   6.1204e-11   7.7243e-12
```

A sweep over a model corner parameter is not supported; sweep a variable that the
schematic or `[variables]` feeds into a `.param`.

## See also

- [setup.toml](../reference/files/setup-toml.md) (`[[sweep]]`), [Results](../reference/files/results.md)
- [Monte Carlo](monte-carlo.md), [SIMULATION.md](../SIMULATION.md)
