# OTA 5: sizing against a spec

<!-- Made from cdept-gui/tutorials/ota-sizing.toml by `cdept tutorial ota-sizing --print`. Edit that file, not this one. -->

You sweep the load capacitance to find how much capacitance the OTA adds itself, then
change a size to meet a tighter gain limit, and check what the change costs.

About 30 minutes. To be guided through it inside CDePT:

```sh
cdept tutorial ota-sizing
```

## Before you start

- [OTA 3](ota-ac.md): gain, unity-gain frequency, phase margin.

## 1. Get the project

Make the starting project.

## 2. Select tb_ota5t

In the Libraries panel, click the cell "tb_ota5t".

## 3. New testbench: ac_load

Open Tools > New Testbench….

## 4. The template ac_load

Choose ac_load, a project template (templates/ac_load.toml): the AC response with a
sweep of the load capacitance "cl".

## 5. Fill in, then Create

Set the parameter loads to this list, then press "Create":

    50f 100f 200f 500f 1p

## 6. Run

The design variables come from the AC testbench, which simulates the same schematic.
Press "Run": five points, one per load.

## 7. What the OTA adds

Fold "Setup" and look at the five points (scroll):

- 50 fF: ugbw 259 MHz, pm 57.5°
- 100 fF: 159 MHz, 59.8°
- 200 fF: 99.7 MHz, 62.3°
- 500 fF: 51.3 MHz, 70.1°
- 1 pF: 28.6 MHz, 77.8°

The DC gain does not change: the load only matters at high frequency. With 1 pF the
load dominates, and ugbw = gm1 / (2π · C) gives C = 201 µS / (2π · 28.6 MHz) =
1.12 pF: about 0.12 pF more than the load. At 100 fF the same formula gives C = 201 fF.
So the OTA itself puts about 100 to 120 fF on its output, as much as the small load.
That is why OTA 3's estimate with CL alone was off by two.

A heavier load also raises the phase margin: it moves the dominant pole down and away
from the mirror's.

## 8. Back to the AC testbench

Open the view ac_response of tb_ota5t.

## 9. Ask for more gain

Suppose the specification asks for at least 36.5 dB. Under "Test template" set
gain_db_min = 36.5, press "Save", then "Run". The gain, 35.2 dB, now FAILS.

## 10. Which knob?

A = gm1 / (gds2 + gds4). Either the pair gets more gm from the same current, or the
output conductances get smaller.

For the same current, a transistor gives more gm per ampere the lower its current
density: further into weak inversion. So doubling the input pair's width at the same
tail current raises gm1.

Double it with m=2, two of the same unit device in parallel, rather than a wider
device: the unit keeps its model (same width per finger), and in layout two
identical units are what you match.

## 11. Open the OTA

Open the schematic of ota5t.

## 12. Set m=2 on M1 and M2

Select M1, press q, set m=2, press OK. Do the same with M2. Both: the pair must stay
identical.

The guide checks the schematic `work/ota5t/schematic`:

| Part | Symbol | Values | Pins on nets |
|---|---|---|---|
| M5, the tail current source | `sg13_lv_pmos` | `l=1u` `m=1` `ng=4` `w=32u` | `B=vdd` `D=(the tail net)` `G=ibias` `S=vdd` |
| M6, the diode that sets the bias | `sg13_lv_pmos` | `l=1u` `m=1` `ng=4` `w=32u` | `B=vdd` `D=ibias` `G=ibias` `S=vdd` |
| M1, the input transistor on inp | `sg13_lv_pmos` | `l=0.5u` `m=2` `ng=4` `w=30u` | `B=(the tail net)` `D=(the mirror net)` `G=inp` `S=(the tail net)` |
| M2, the input transistor on inn | `sg13_lv_pmos` | `l=0.5u` `m=2` `ng=4` `w=30u` | `B=(the tail net)` `D=out` `G=inn` `S=(the tail net)` |
| M3, the mirror's diode | `sg13_lv_nmos` | `l=4u` `m=1` `w=2u` | `B=vss` `D=(the mirror net)` `G=(the mirror net)` `S=vss` |
| M4, the mirror's output | `sg13_lv_nmos` | `l=4u` `m=1` `w=2u` | `B=vss` `D=out` `G=(the mirror net)` `S=vss` |
| the input pin inp | `ipin` | `lab=inp` |  |
| the input pin inn | `ipin` | `lab=inn` |  |
| the bias input pin ibias | `ipin` | `lab=ibias` |  |
| the output pin out | `opin` | `lab=out` |  |
| the supply pin vdd | `iopin` | `lab=vdd` |  |
| the ground pin vss | `iopin` | `lab=vss` |  |

## 13. Save the schematic

CDePT simulates the saved schematic.

Save with Ctrl+S (answer Yes).

The guide checks the schematic `work/ota5t/schematic`:

| Part | Symbol | Values | Pins on nets |
|---|---|---|---|
| M5, the tail current source | `sg13_lv_pmos` | `l=1u` `m=1` `ng=4` `w=32u` | `B=vdd` `D=(the tail net)` `G=ibias` `S=vdd` |
| M6, the diode that sets the bias | `sg13_lv_pmos` | `l=1u` `m=1` `ng=4` `w=32u` | `B=vdd` `D=ibias` `G=ibias` `S=vdd` |
| M1, the input transistor on inp | `sg13_lv_pmos` | `l=0.5u` `m=2` `ng=4` `w=30u` | `B=(the tail net)` `D=(the mirror net)` `G=inp` `S=(the tail net)` |
| M2, the input transistor on inn | `sg13_lv_pmos` | `l=0.5u` `m=2` `ng=4` `w=30u` | `B=(the tail net)` `D=out` `G=inn` `S=(the tail net)` |
| M3, the mirror's diode | `sg13_lv_nmos` | `l=4u` `m=1` `w=2u` | `B=vss` `D=(the mirror net)` `G=(the mirror net)` `S=vss` |
| M4, the mirror's output | `sg13_lv_nmos` | `l=4u` `m=1` `w=2u` | `B=vss` `D=out` `G=(the mirror net)` `S=vss` |
| the input pin inp | `ipin` | `lab=inp` |  |
| the input pin inn | `ipin` | `lab=inn` |  |
| the bias input pin ibias | `ipin` | `lab=ibias` |  |
| the output pin out | `opin` | `lab=out` |  |
| the supply pin vdd | `iopin` | `lab=vdd` |  |
| the ground pin vss | `iopin` | `lab=vss` |  |

## 14. Run the AC testbench again

Open ac_response again.

## 15. Run

Press "Run".

## 16. What changed

Compare with before:

- gain_db 35.2 to 36.9 dB: passes. gm1 went from 201 to 237 µS while the output
  conductance hardly moved (the estimate 237 / (1.60 + 1.70) = 71.8 is 37.1 dB).
- ugbw 159 to 168 MHz: more gm.
- pm 59.8 to 55.1°: the wider pair loads the mirror node and the output. Still above
  50°.

## 17. Check the bias again

A new size means a new operating point. Open ota_op.

## 18. Run

Press "Run".

## 19. And the price

Every spec still passes here: sat_margin went from 127 to 111 mV.

More is not always better. With m=4 the gain would be 38.5 dB, but the phase margin
falls to 49° and fails the 50° limit. A longer pair (l=1u, m=2) reaches 37.2 dB, but
its phase margin drops to 40°.

All of this is at one corner and one temperature. At 125 °C the gain of this OTA is
about 4 dB lower than at 27 °C, so a 36.5 dB limit would not survive "OTA 7". A spec
without its corners is half a spec.

Next: "OTA 6: rejection and power".
