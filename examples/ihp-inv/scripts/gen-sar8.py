"""Generate libs/cells/sar8/schematic/sar8.sch (python3 scripts/gen-sar8.py libs/cells/sar8/schematic/sar8.sch): a behavioural 8-bit SAR
ADC drawn as xschem devices joined by net labels (as cells/flash3), so every element is
visible and editable in the schematic editor."""
import sys

N = 8
T = 100  # ns
out = []
out.append("v {xschem version=3.4.5 file_version=1.2\n}\nG {}\nK {}\nV {}\nS {}\nE {}")
out.append("T {Behavioural 8-bit SAR ADC, 1 MS/s: a sampling switch (Ssh, Ch) holds in while ph0 is high (95 ns), "
           "then for each bit k (MSB first, 100 ns each) the trial phase tk adds the bit's weight to the DAC, "
           "the comparator (Bcmp) decides hold > dac, and the latch phase lk (5 to 80 ns into the bit) stores the "
           "decision on dk (switch Slk, capacitor Cdk; Srk clears it while sampling). code is the output code as a "
           "voltage, valid from 880 ns to the end of the 1 us conversion. msb_err is the relative error of the MSB "
           "weight (a capacitor mismatch): the testbench sets it ([variables] msb_err = 0).} -600 -900 0 0 0.25 0.25 {}")
labels = 0


def lab(x, y, name, rot=0):
    global labels
    labels += 1
    out.append(f"C {{lab_pin.sym}} {x} {y} {rot} 0 {{name=l{labels} sig_type=std_logic lab={name}}}")


def two_pin(sym, x, y, attrs, top, bottom):
    out.append(f"C {{{sym}}} {x} {y} 0 0 {{{attrs}}}")
    lab(x, y - 30, top)
    lab(x, y + 30, bottom)


def switch(x, y, name, p, m, cp, cm):
    out.append(f"C {{switch_ngspice.sym}} {x} {y} 0 0 {{name={name} model=swm}}")
    lab(x, y - 30, p)
    lab(x, y + 30, m)
    lab(x - 40, y, cp)
    lab(x - 40, y + 20, cm)


# pins
out.append("C {ipin.sym} -600 -700 0 0 {name=p1 lab=in}")
out.append("C {ipin.sym} -600 -660 0 0 {name=p2 lab=ref}")
out.append("C {opin.sym} -480 -700 0 0 {name=p3 lab=code}")
out.append("C {iopin.sym} -600 -620 2 0 {name=p4 lab=vss}")
# the switch model, inside the subcircuit
out.append('C {code_shown.sym} -360 -720 0 0 {name=s1 only_toplevel=false value=".model swm sw vt=0.5 vh=0 ron=100 roff=1e12"}')
# sampling phase, sample and hold, comparator
two_pin("vsource.sym", -560, -480, 'name=Vph0 value="PULSE(0 1 0 0.5n 0.5n 95n 1u)" savecurrent=false', "ph0", "vss")
switch(-420, -480, "Ssh", "in", "hold", "ph0", "vss")
two_pin("capa.sym", -320, -480, "name=Ch value=1p m=1", "hold", "vss")
two_pin("bsource.sym", -220, -480, 'name=Bcmp VAR=V FUNC="0.5*(1+tanh((V(hold)-V(dac))/1e-4))" m=1', "cmp", "vss")
dac = []
code = []
for k in range(1, N + 1):
    x = -560 + (k - 1) * 200
    y = -260
    td = k * T
    two_pin("vsource.sym", x, y, f'name=Vt{k} value="PULSE(0 1 {td}n 0.5n 0.5n 99n 1u)" savecurrent=false', f"t{k}", "vss")
    two_pin("vsource.sym", x + 80, y, f'name=Vl{k} value="PULSE(0 1 {td + 5}n 0.5n 0.5n 75n 1u)" savecurrent=false', f"l{k}", "vss")
    switch(x + 40, y + 160, f"Sl{k}", "cmp", f"d{k}", f"l{k}", "vss")
    switch(x + 140, y + 160, f"Sr{k}", f"d{k}", "vss", "ph0", "vss")
    two_pin("capa.sym", x + 40, y + 320, f"name=Cd{k} value=1p m=1", f"d{k}", "vss")
    w = 2 ** (N - k)
    weight = f"{w}*(1+\\{{msb_err\\}})" if k == 1 else f"{w}"
    dac.append(f"{weight}*max(0.5*(1+tanh((V(d{k})-0.5)/0.02)),V(t{k}))")
    code.append(f"{w}*u(V(d{k})-0.5)")
two_pin("bsource.sym", -100, -480, 'name=Bdac VAR=V FUNC="V(ref)/256*(' + "+".join(dac) + ')" m=1', "dac", "vss")
two_pin("bsource.sym", 20, -480, 'name=Bcode VAR=V FUNC="' + "+".join(code) + '" m=1', "code", "vss")
open(sys.argv[1], "w").write("\n".join(out) + "\n")
