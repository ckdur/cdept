# The tutorial plan

Status 2026-11-28: **the plan the tutorials were built from.** I1 to I7 and O1 to O9
exist ([the tutorials](index.md)); O10 does not. What "needs first" names was built,
except as the index says under "Not covered yet". The task numbers (A5, C1, ...) are
those of the index's tables.

Two series. The **inverter** teaches how to work in CDePT, once through the whole flow.
The **OTA** (the five-transistor OTA of `examples/ihp-inv`) teaches analog design with
it, and takes the program's use for granted.

Every tutorial can start by itself: it brings a project in which the earlier ones are
done, so a reader may skip, repeat, or recover from a broken schematic.

## Inverter series

| # | Tutorial | You end with | Tasks | Needs first |
|---|---|---|---|---|
| I1 | `inv-library` | your library and an `inv` cell with an empty schematic | A5 | anchors only |
| I2 | `inv-schematic` | the inverter drawn, with pins and sizes, and a netlist you have read | C1, C3 | schematic support |
| I3 | `inv-symbol` | its symbol, and a testbench schematic that uses it | C2 | I2's support |
| I4 | `inv-transient` | a first simulation, the waveforms, the delay measured | D1, D3, D6, D7 | new testbench in the GUI |
| I5 | `inv-corners` | transfer curve, limits, pass and fail over corners and temperatures | D2, D8, D9 | |
| I6 | `inv-layout` | the layout: generated, placed, routed, with pins; nets all connected | E1-E5 | the layout trial |
| I7 | `inv-verify` | DRC clean and LVS matching, after reading and fixing a violation | F1, F2 | I6 |

## OTA series

| # | Tutorial | You end with | Tasks | Needs first |
|---|---|---|---|---|
| O1 | `ota-schematic` | the OTA drawn (pair, mirror, tail, bias) and its symbol | C1, C2 | |
| O2 | `ota-bias` | the operating point read, every transistor checked for saturation | D2 | |
| O3 | `ota-ac` | gain, unity-gain frequency and phase margin; why the bench closes the loop at DC only | D4, D6 | |
| O4 | `ota-step` | slew and settling as a unity-gain buffer, against what O3 predicts | D3, D7 | |
| O5 | `ota-sizing` | a size changed against a spec and compared with the run before; load swept | D7, D9 | |
| O6 | `ota-rejection` | PSRR, CMRR and power | D4, D8 | |
| O7 | `ota-suite` | every test over 5 corners and 3 temperatures in one table (90 runs) | D9, D11 | new suite in the GUI |
| O8 | `ota-mismatch` | the input offset spread from Monte Carlo | D10 | an offset testbench (the example has none) |
| O9 | `ota-noise` | input-referred noise and its largest sources | D5 | a noise testbench (the example has none) |
| O10 | `ota-layout` | matched pair and mirror, routed, DRC and LVS clean | E2, E3, F1, F2 | I6 and I7 done and judged usable |

Later, on their own: CACE datasheets (D12), importing existing designs and files (A4,
A6), PDK updates (A3), git (A7), export and LEF (G1, G2).

Not possible today: simulation with the extracted layout (F5), gm/ID sizing (B3),
electrical rule checks (F6), Liberty models (G3). Extraction (F4) is command line only
and has capacitance only.

## What has to be built before

1. **Schematic support in the guide** (before I2). xschem's menus, toolbar and dialogs
   are drawn by CDePT, so the guide can be made to point at them. The drawing is
   xschem's own picture. Planned:
   - facts read from the schematic itself ("there is a PMOS with w=1.5u", "A reaches
     both gates"), so a step ticks off when the circuit is right, however it was drawn;
   - a picture of the expected result in the card;
   - the outline on a placed device, where CDePT can work out its place (the test
     driver already clicks instances by name). Wiring is not arrowed point by point.
2. **New testbench and new suite in the GUI** (before I4 and O7). Today a testbench is
   made from a template with `cdept sim new` only; the New View window has no
   testbench or suite entry. This is a gap of CDePT, not only of the tutorials.
3. **Starting points** (before I2): the ready-made projects, and a way for a tutorial
   to make one in a folder of its own.
4. **The layout trial** (before I6 is promised). DRC and LVS have been shown clean on
   the PDK's standard cells. I found no record of a cell routed by hand in CDePT's
   layout tab that passed both; the example inverter is left unwired on purpose. So
   first wire the inverter there to a clean result and note what is missing (path
   widths, vias, taps).

Also unchecked: whether operating-point values can be shown on the schematic (O2
would use it; else it uses the table in the waveform tab).

## Suggested order

| Round | Content | Then |
|---|---|---|
| 1 | schematic support, starting points, I1, I2, I3 | you try them |
| 2 | new testbench in the GUI, I4, I5 | you try them |
| 3 | O1-O4 | you try them |
| 4 | O5-O7, with new suite in the GUI | you try them |
| 5 | layout trial, then I6, I7 | you try them |
| 6 | O8, O9, and O10 if the layout tutorials went well | |

The layout trial may be run beside round 1, so that its findings are known early.
