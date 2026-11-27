# OTA 3: gain, unity-gain frequency, phase margin

<!-- Made from cdept-gui/tutorials/ota-ac.toml by `cdept tutorial ota-ac --print`. Edit that file, not this one. -->

You measure the open-loop AC response of the OTA, compare the DC gain and bandwidth
with hand estimates, and read it as a waveform.

About 20 minutes. To be guided through it inside CDePT:

```sh
cdept tutorial ota-ac
```

## Before you start

- [OTA 2](ota-bias.md), or the starting project (it has the operating-point
  testbench).

## 1. Get the project

Make the starting project: the OTA, its testbench schematics and the operating-point
view of "OTA 2".

## 2. Estimate first

At DC the OTA's gain is the pair's transconductance times the resistance at the
output: A = gm1 / (gds2 + gds4). From the operating point of this design:
gm1 = 201 µS, gds2 = 1.74 µS, gds4 = 1.69 µS. So A = 201 / 3.43 = 58.6, which is
35.4 dB.

With the load alone, the unity-gain frequency would be gm1 / (2π · CL) =
201 µS / (2π · 100 fF) = 320 MHz. Keep both numbers.

## 3. Select tb_ota5t

In the Libraries panel, open "work" and click the cell "tb_ota5t".

## 4. New testbench

Open Tools > New Testbench….

## 5. The template ac_response

Choose the template ac_response. It sweeps the frequency and reports the DC gain,
the -3 dB bandwidth, the unity-gain frequency (ugbw) and the phase margin (pm) of out
over the node "in".

## 6. Parameters, then Create

Set these parameters in the window (scroll the list):

- inverting: yes. Here "in" drives inn, the inverting input, so the response is -out/in.
  With "no" the phase starts at 180° and the phase margin is wrong.
- gain_db_min: 30
- ugbw_min: 80meg
- pm_min: 50

Then press "Create".

## 7. Turn the AC source on, and save

CDePT copied the design variables from the operating-point testbench, which
simulates the same schematic (the Console says so). One must differ: there the AC
source was off. In the Variables list (fold "Test template" first), set

- ac_dm = 1

(vbias, which the template adds, is not used by this schematic.) This is the 1 V of AC amplitude on Vin. An AC analysis is linear, so 1 V is only a
unit: out then reads as the gain. Check that inverting is "yes" and the limits are set
(under Test template; you can change them there). Then press "Save".

## 8. Run

Press "Run".

## 9. Compare with the estimate

Fold "Setup" to see the results (numbers are written like 3.5187e1 = 35.187):

- gain_db 35.2 dB: the estimate was 35.4 dB. The simple formula is very good.
- bw_3db 4.5 MHz, ugbw 159 MHz.
- pm 59.8°.

The unity-gain frequency is half the 320 MHz estimate: the OTA's own transistors add
about as much capacitance at out as the load does (OTA 5 measures it). The response
also has more than one pole: 58 times the 4.5 MHz bandwidth is 260 MHz, not 159.
The mirror adds a pole at its gate node and a zero at about twice that frequency,
because M1's current reaches the output only through it.

## 10. Look at the response

In the results, press "Waveform Tab" on the point.

## 11. Read it with a cursor

The tab shows the AC magnitude in dB of out (and of "in", flat at 0 dB). The flat part
is the DC gain, 35 dB. Click the curve to put cursor A on it; the table below reads
frequency and value. Click where out crosses 0 dB: that is the unity-gain
frequency, 159 MHz. The wheel zooms, "Fit" shows everything again.

The phase margin is 180° plus the phase of the response (taken as -out/in) at that
frequency; the pm line of the results has it.

## 12. What you have

DC gain 35 dB, unity-gain frequency 159 MHz, phase margin 60°, at tt and 27 °C. A gain
of 58 is modest: one stage of this OTA is not an op-amp. Its use is speed and
simplicity.

Next: "OTA 4: the step response".
