# Inverter 4: the first simulation

<!-- Made from cdept-gui/tutorials/inv-transient.toml by `cdept tutorial inv-transient --print`. Edit that file, not this one. -->

You simulate the inverter in time with ngspice: you make a testbench view from a test
template, run it, read the measured delays and edge times, and check the delay
yourself on the waveforms.

About 15 minutes. To be guided through it inside CDePT:

```sh
cdept tutorial inv-transient
```

## Before you start

- [Inverter 3](inv-symbol.md), or let the guide make the starting project.
- ngspice on this machine: on PATH, or named by `CDEPT_NGSPICE` in the shell that
  starts CDePT.
- CDePT built with the xschem helper (`cargo build -p cdept-xschem`).

## 1. Get the project

Either let the guide make the starting project (the inverter, its symbol and the
testbench schematic tb_inv), or open your own project from "Inverter 3" and press
Continue.

## 2. Select the testbench cell

In the Libraries panel on the left, open "work" and click the cell "tb_inv". What you
do next starts from the selected cell.

## 3. A new testbench view

The testbench schematic says what is connected. A testbench view says what to do with
it: which analysis, which corners, what to measure and which limits to check.

Open Tools > New Testbench….

## 4. Choose the test template

The window proposes the template "tran_delay" (a transient of a pulse: delays, edge
times and overshoot) for the cell tb_inv, and names the view after it. The cell has a
schematic already, so the testbench simulates that one.

The template's parameters are its defaults: nets "in" and "out", a supply of 1.2 V,
5 ps maximum time step, 1.2 ns of simulated time. Leave them, and press Create.

## 5. The Simulation tab

The new view opens in the Simulation tab.

- "Test template" holds the template's parameters. Changing one rewrites what the
  testbench does.
- "Setup" holds the rest: the simulator, the corners and temperatures, and the design
  variables. vdd = 1.2 is the value that {vdd} in your schematic gets.

With no corner ticked and no temperature given, one run is made: the typical corner
"tt" at 27 °C, as the line under the buttons says (1 run point).

## 6. Run it

Press "▶ Run". CDePT netlists the schematic, adds the PDK's transistor models for the
corner, the analysis and your variables, and runs ngspice. This takes a few seconds;
the first run of a new project can take longer.

## 7. Read the results

The results are below the form. Click the titles "Test template" and "Setup" to fold
them away (or scroll down with the mouse wheel).

The numbers are in volts, seconds and V/s. In the typical corner you should read
about:

- tpd = 4.36e-11 s: the average propagation delay, 43.6 ps. The delay is measured
  from the input crossing half the supply (0.6 V) to the output crossing it.
- tpd_rise = 42.1 ps and tpd_fall = 45.1 ps: for the output going up and down. They
  are close because the PMOS is twice as wide as the NMOS.
- t_rise = 54.5 ps and t_fall = 51.2 ps: the output's edges from 10 % to 90 %.
- overshoot = 4.3 (%): the output goes beyond the supply rails by this much.

There are no limits yet, so there is nothing to pass or fail: "Inverter 5" adds them.
Small differences from these numbers are normal if you drew the circuit differently.

## 8. Open the waveforms

A number is only as good as the waveform behind it. Next to the point "tt" under
Results, press "Waveform Tab".

## 9. Zoom into an edge

The tab shows the signals the testbench plots: v(in), yellow, and v(out). When the
input is low the output is high: it inverts.

Put the pointer on the rising edge of the input, near 0.2 ns, and roll the mouse
wheel up until the edge fills a good part of the plot. Drag to move the picture,
double-click (or press Fit) to see everything again.

The guide's window can be dragged out of the way.

## 10. Cursor A on the input

Click where the rising input crosses 0.6 V, half the supply. The table under the plot
shows cursor A and the value of each signal there: move the cursor (click again)
until v(in) reads close to 600 mV. That is at about 225 ps: the input starts at 200 ps
and takes 50 ps to rise.

## 11. Cursor B on the output

Now right-click where the falling output crosses 0.6 V. The line above the table
gives B−A: the time from the input's middle to the output's middle. It should be
about 45 ps, the tpd_fall the testbench computed (45.1 ps).

Do it once for the other edge too if you like: the input falls through 0.6 V at about
675 ps and the output rises through it about 42 ps later (tpd_rise).

## 12. Look at the bump

Just before the output falls, it rises a little above 1.2 V, to about 1.25 V. The same
happens below 0 V on the other edge.

This is the gate-to-drain capacitance of the two transistors: while the input moves
and the transistors have not yet taken over, the edge passes straight through that
capacitance to the output. The smaller the load capacitance compared with it, the
larger the bump. This is the "overshoot" of the results (4.3 % of the supply).

It is real, and it also appears on the supply and ground in a real chip. A delay or
level measured without looking at the waveform can be fooled by such features.

## 13. What you now have

The testbench view tran_delay is a file you can read:
libs/work/tb_inv/tran_delay/setup.toml. It holds the template's name and whatever you
changed. The results are under simulations/, one folder per run point, with the
deck, the log and the raw waveform file.

The same run from a terminal:

    cdept -p ~/cdept-tutorial/inv-transient sim run work tb_inv --view tran_delay

Next: "Inverter 5: limits, transfer curve and corners".
