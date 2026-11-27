# Inverter 5: limits, transfer curve and corners

<!-- Made from cdept-gui/tutorials/inv-corners.toml by `cdept tutorial inv-corners --print`. Edit that file, not this one. -->

A chip must work for every transistor the factory makes and at every temperature it
meets. You give the inverter's delay a limit, simulate it over process corners and
temperatures, read which points fail, then measure its DC transfer curve and see how
its switching point moves with the corners.

About 25 minutes. To be guided through it inside CDePT:

```sh
cdept tutorial inv-corners
```

## Before you start

- [Inverter 4](inv-transient.md), or let the guide make the starting project.
- ngspice on this machine: on PATH, or named by `CDEPT_NGSPICE` in the shell that
  starts CDePT.
- CDePT built with the xschem helper (`cargo build -p cdept-xschem`).

## 1. Get the project

Either let the guide make the starting project (the testbench with its transient view
tran_delay), or open your own project from "Inverter 4" and press Continue.

## 2. Open the transient testbench

In the Libraries panel, open "work" and "tb_inv", and double-click the view
"tran_delay". It opens in the Simulation tab.

## 3. Set a limit on the delay

A specification says what the circuit must do. Without one a simulation only gives
numbers.

Under "Test template", click the field tpd_max and type:

    50p

Every run point now passes only if its average delay is at most 50 ps. In the typical
corner at 27 °C you measured 43.6 ps. (If the section is folded, click "Test
template" to open it.)

## 4. Choose the corners

No two wafers are alike. The foundry gives model sets for the extremes it
guarantees, the process corners. In IHP SG13G2:

- tt: typical;
- ff: fast NMOS and fast PMOS;
- ss: slow NMOS and slow PMOS;
- sf, fs: one fast, one slow (the first letter is the NMOS).

Under "Setup", tick tt, ff and ss in the Corners row.

## 5. Choose the temperatures

Temperature changes the transistors too. In the field Temperatures (in °C) type:

    -40, 27, 125

This is the industrial range with room temperature in between. The line under the
buttons now says 9 run points: 3 corners × 3 temperatures.

## 6. Save the testbench

Press "Save" above the form. The limit, corners and temperatures go into the view's
setup.toml.

## 7. Run the nine points

Press "▶ Run". Several simulators run at once; the Jobs list shows each point.

## 8. Find what fails

Fold "Test template" and "Setup" by clicking their titles. Under "Results" the line
says how many limits pass: 6 pass and 3 fail.

Scroll down to "Points (9)" and open it: each point shows tpd with PASS or FAIL. In
our run:

- ss: 50.7 ps at -40 °C, 52.0 ps at 27 °C, 53.5 ps at 125 °C. All three fail.
- tt: 43.0 to 44.6 ps. ff: about 36.5 to 37.6 ps. All pass.

The slow process is the worst case for delay, and heat makes it a little slower. At
the typical corner alone the design seemed to have room to spare; this is why you
simulate corners.

What to do about a failing corner is a design decision: wider transistors, a smaller
load, or a limit that is argued to be too strict. Do not loosen a limit just because
it fails.

## 9. A second testbench: the DC curve

The transfer curve is the output voltage for every input voltage, with time left
out. It tells where the inverter switches and how sharply.

The same schematic can have several testbench views. With tb_inv selected, open
Tools > New Testbench….

## 10. Choose the template dc_transfer

Open the "Template" list and choose dc_transfer. The cell must say tb_inv; the view
is named dc_transfer by itself.

## 11. Limits of the transfer curve

The template sweeps the source named Vin from 0 to the supply in 10 mV steps (this is
why the name mattered in "Inverter 3"). Fill in three limits among the parameters:

- gain_min = 10: the steepest slope must be at least 10. A logic gate needs gain well
  above 1 to turn a weak input into a clean output.
- vtrip_min = 0.5 and vtrip_max = 0.7: the output must cross half the supply while
  the input is between 0.5 and 0.7 V.

Then press Create.

## 12. All five corners

For the switching point the mixed corners matter most: when the NMOS is fast and the
PMOS slow, or the other way round, one pulls harder than the other. Under "Setup"
tick tt, ff, ss, sf and fs. Leave the temperatures empty (27 °C), and press Save.

## 13. Run the DC sweep

Press "▶ Run".

## 14. Read the switching points

All limits pass. For each corner, vtrip is the input voltage at which the output
crosses 0.6 V; vm is the input voltage where the output equals the input. At 27 °C we
got these vtrip values:

- fs: 0.587 V (strong NMOS: it pulls the output down earlier);
- ss: 0.614 V, tt: 0.621 V, ff: 0.625 V;
- sf: 0.654 V (strong PMOS: the input has to go higher).

ss and ff hardly move it: both transistors changed together. The mixed corners move
it by about 35 mV each way. gain_max, the steepest slope, is about 16 to 18.

## 15. See the curves together

Below the statistics (scroll down with the wheel), press "Waveform Tab (family)": one tab with the curves of all five corners. Zoom
around 0.6 V (wheel over the plot): the curves cross half the supply in the order fs,
ss, tt, ff, sf. The Points list on the left shows or hides each corner.

## 16. What you now have

tb_inv now has two testbench views with limits over the corners. Both are plain
setup.toml files: commit them, and anyone can check the design again:

    cdept -p ~/cdept-tutorial/inv-corners sim run work tb_inv --view tran_delay
    cdept -p ~/cdept-tutorial/inv-corners sim run work tb_inv --view dc_transfer

The command fails when a limit fails, so it can guard a change. Several testbenches
can also be run together as a test suite, as the OTA tutorials do.
