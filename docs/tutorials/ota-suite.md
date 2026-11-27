# OTA 7: every test over every corner

<!-- Made from cdept-gui/tutorials/ota-suite.toml by `cdept tutorial ota-suite --print`. Edit that file, not this one. -->

You run all six tests over five process corners and three temperatures (90
simulations) as one suite, find a real weakness of this design at 125 °C, understand
it and fix it.

About 30 minutes. To be guided through it inside CDePT:

```sh
cdept tutorial ota-suite
```

## Before you start

- [OTA 2](ota-bias.md) to [OTA 6](ota-rejection.md); the starting project has all their testbenches.

## 1. Get the project

Make the starting project.

## 2. Corners

Transistors vary from wafer to wafer, and every chip works over a temperature range.
The PDK's corner sets bound the process: tt typical, ff both fast, ss both slow, fs
and sf one fast and one slow. Temperatures: -40, 27 and 125 °C. Five times three is
15 points per test, six tests: 90 runs.

A suite is a view that lists testbenches and runs them together into one table.

## 3. Select ota5t

In the Libraries panel click the cell "ota5t": the suite will live with the design it tests.

## 4. New suite

Open Tools > New Suite….

## 5. Choose the tests and corners

The view name is suite. Tick all six testbenches: tb_ota5t/ac_response, cmrr, ota_op
and psrr, tb_ota5t_step/ota_step and power.

Under "For every test", type the corners and temperatures. They replace each test's
own:

    tt ff ss fs sf

    -40 27 125

Press "Create".

## 6. Run the suite

Press "Run suite". The runs go in parallel; wait for the verdict.

## 7. Read the verdict

SUITE FAIL. Find the red rows in the table:

- ota_op: sat_margin is negative at 125 °C in tt, ff, ss and sf (down to -53 mV): a
  transistor has left saturation.
- cmrr: at 125 °C the rejection falls below 40 dB, down to 22 dB at sf.

Everything else passes: the gain falls to 30.5 dB at 125 °C but stays above its 30 dB
limit, the phase margin stays above 58°, and the step settles within 10.1 ns.

Every test passed at 27 °C when you ran it alone.

## 8. Why

Both failures have one cause. M1's drain-source voltage is the tail node minus the
mirror node: vds1 = tail - x.

- When hot, the PMOS pair needs less |VGS|, so the tail comes down.
- The NMOS mirror's diode M3, long and narrow, needs a high gate voltage, and it
  rises when hot.

At tt, 27 °C: tail 0.815 V, x 0.557 V, so M1 has 0.26 V and needs 0.12 V. At sf,
125 °C: tail 0.726 V, x 0.621 V, so M1 has 0.105 V and needs 0.158 V: it is in the
triode region. Its gds shoots up, and by OTA 6's reasoning so does the common-mode
gain.

The fix: lower x. Give the mirror twice the width: each unit then carries half the
current and needs less gate voltage.

## 9. Open the OTA

Open the schematic of ota5t.

## 10. Set m=2 on M3 and M4

Select M3, press q, set m=2, OK. The same for M4: a mirror needs two identical units.

The guide checks the schematic `work/ota5t/schematic`:

| Part | Symbol | Values | Pins on nets |
|---|---|---|---|
| M5, the tail current source | `sg13_lv_pmos` | `l=1u` `m=1` `ng=4` `w=32u` | `B=vdd` `D=(the tail net)` `G=ibias` `S=vdd` |
| M6, the diode that sets the bias | `sg13_lv_pmos` | `l=1u` `m=1` `ng=4` `w=32u` | `B=vdd` `D=ibias` `G=ibias` `S=vdd` |
| M1, the input transistor on inp | `sg13_lv_pmos` | `l=0.5u` `m=1` `ng=4` `w=30u` | `B=(the tail net)` `D=(the mirror net)` `G=inp` `S=(the tail net)` |
| M2, the input transistor on inn | `sg13_lv_pmos` | `l=0.5u` `m=1` `ng=4` `w=30u` | `B=(the tail net)` `D=out` `G=inn` `S=(the tail net)` |
| M3, the mirror's diode | `sg13_lv_nmos` | `l=4u` `m=2` `w=2u` | `B=vss` `D=(the mirror net)` `G=(the mirror net)` `S=vss` |
| M4, the mirror's output | `sg13_lv_nmos` | `l=4u` `m=2` `w=2u` | `B=vss` `D=out` `G=(the mirror net)` `S=vss` |
| the input pin inp | `ipin` | `lab=inp` |  |
| the input pin inn | `ipin` | `lab=inn` |  |
| the bias input pin ibias | `ipin` | `lab=ibias` |  |
| the output pin out | `opin` | `lab=out` |  |
| the supply pin vdd | `iopin` | `lab=vdd` |  |
| the ground pin vss | `iopin` | `lab=vss` |  |

## 11. Save the schematic

The suite simulates the saved schematic.

Save with Ctrl+S (answer Yes).

The guide checks the schematic `work/ota5t/schematic`:

| Part | Symbol | Values | Pins on nets |
|---|---|---|---|
| M5, the tail current source | `sg13_lv_pmos` | `l=1u` `m=1` `ng=4` `w=32u` | `B=vdd` `D=(the tail net)` `G=ibias` `S=vdd` |
| M6, the diode that sets the bias | `sg13_lv_pmos` | `l=1u` `m=1` `ng=4` `w=32u` | `B=vdd` `D=ibias` `G=ibias` `S=vdd` |
| M1, the input transistor on inp | `sg13_lv_pmos` | `l=0.5u` `m=1` `ng=4` `w=30u` | `B=(the tail net)` `D=(the mirror net)` `G=inp` `S=(the tail net)` |
| M2, the input transistor on inn | `sg13_lv_pmos` | `l=0.5u` `m=1` `ng=4` `w=30u` | `B=(the tail net)` `D=out` `G=inn` `S=(the tail net)` |
| M3, the mirror's diode | `sg13_lv_nmos` | `l=4u` `m=2` `w=2u` | `B=vss` `D=(the mirror net)` `G=(the mirror net)` `S=vss` |
| M4, the mirror's output | `sg13_lv_nmos` | `l=4u` `m=2` `w=2u` | `B=vss` `D=out` `G=(the mirror net)` `S=vss` |
| the input pin inp | `ipin` | `lab=inp` |  |
| the input pin inn | `ipin` | `lab=inn` |  |
| the bias input pin ibias | `ipin` | `lab=ibias` |  |
| the output pin out | `opin` | `lab=out` |  |
| the supply pin vdd | `iopin` | `lab=vdd` |  |
| the ground pin vss | `iopin` | `lab=vss` |  |

## 12. Back to the suite

Open the view suite of ota5t.

## 13. Run the suite again

Press "Run suite".

## 14. What the fix cost and bought

SUITE PASS: all 90 runs.

- sat_margin: at least 81 mV everywhere (it was -53 mV).
- CMRR: at least 48.7 dB (it was 22 dB); at tt, 27 °C it rose from 45 to 59 dB,
  because M1 and M2 now see more equal voltages.
- The price: 0.75 dB less gain (34.4 dB at tt, 27 °C) and a little less bandwidth
  (147 instead of 159 MHz).

A design is checked when its suite passes, not when one corner does.

Next: "OTA 8: mismatch".
