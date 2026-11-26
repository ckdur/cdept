v {xschem version=3.4.5 file_version=1.2
}
G {}
K {}
V {}
S {}
E {}
T {Behavioural 8-bit SAR ADC, 1 MS/s: a sampling switch (Ssh, Ch) holds in while ph0 is high (95 ns), then for each bit k (MSB first, 100 ns each) the trial phase tk adds the bit's weight to the DAC, the comparator (Bcmp) decides hold > dac, and the latch phase lk (5 to 80 ns into the bit) stores the decision on dk (switch Slk, capacitor Cdk; Srk clears it while sampling). code is the output code as a voltage, valid from 880 ns to the end of the 1 us conversion. msb_err is the relative error of the MSB weight (a capacitor mismatch): the testbench sets it ([variables] msb_err = 0).} -600 -900 0 0 0.25 0.25 {}
C {ipin.sym} -600 -700 0 0 {name=p1 lab=in}
C {ipin.sym} -600 -660 0 0 {name=p2 lab=ref}
C {opin.sym} -480 -700 0 0 {name=p3 lab=code}
C {iopin.sym} -600 -620 2 0 {name=p4 lab=vss}
C {code_shown.sym} -360 -720 0 0 {name=s1 only_toplevel=false value=".model swm sw vt=0.5 vh=0 ron=100 roff=1e12"}
C {vsource.sym} -560 -480 0 0 {name=Vph0 value="PULSE(0 1 0 0.5n 0.5n 95n 1u)" savecurrent=false}
C {lab_pin.sym} -560 -510 0 0 {name=l1 sig_type=std_logic lab=ph0}
C {lab_pin.sym} -560 -450 0 0 {name=l2 sig_type=std_logic lab=vss}
C {switch_ngspice.sym} -420 -480 0 0 {name=Ssh model=swm}
C {lab_pin.sym} -420 -510 0 0 {name=l3 sig_type=std_logic lab=in}
C {lab_pin.sym} -420 -450 0 0 {name=l4 sig_type=std_logic lab=hold}
C {lab_pin.sym} -460 -480 0 0 {name=l5 sig_type=std_logic lab=ph0}
C {lab_pin.sym} -460 -460 0 0 {name=l6 sig_type=std_logic lab=vss}
C {capa.sym} -320 -480 0 0 {name=Ch value=1p m=1}
C {lab_pin.sym} -320 -510 0 0 {name=l7 sig_type=std_logic lab=hold}
C {lab_pin.sym} -320 -450 0 0 {name=l8 sig_type=std_logic lab=vss}
C {bsource.sym} -220 -480 0 0 {name=Bcmp VAR=V FUNC="0.5*(1+tanh((V(hold)-V(dac))/1e-4))" m=1}
C {lab_pin.sym} -220 -510 0 0 {name=l9 sig_type=std_logic lab=cmp}
C {lab_pin.sym} -220 -450 0 0 {name=l10 sig_type=std_logic lab=vss}
C {vsource.sym} -560 -260 0 0 {name=Vt1 value="PULSE(0 1 100n 0.5n 0.5n 99n 1u)" savecurrent=false}
C {lab_pin.sym} -560 -290 0 0 {name=l11 sig_type=std_logic lab=t1}
C {lab_pin.sym} -560 -230 0 0 {name=l12 sig_type=std_logic lab=vss}
C {vsource.sym} -480 -260 0 0 {name=Vl1 value="PULSE(0 1 105n 0.5n 0.5n 75n 1u)" savecurrent=false}
C {lab_pin.sym} -480 -290 0 0 {name=l13 sig_type=std_logic lab=l1}
C {lab_pin.sym} -480 -230 0 0 {name=l14 sig_type=std_logic lab=vss}
C {switch_ngspice.sym} -520 -100 0 0 {name=Sl1 model=swm}
C {lab_pin.sym} -520 -130 0 0 {name=l15 sig_type=std_logic lab=cmp}
C {lab_pin.sym} -520 -70 0 0 {name=l16 sig_type=std_logic lab=d1}
C {lab_pin.sym} -560 -100 0 0 {name=l17 sig_type=std_logic lab=l1}
C {lab_pin.sym} -560 -80 0 0 {name=l18 sig_type=std_logic lab=vss}
C {switch_ngspice.sym} -420 -100 0 0 {name=Sr1 model=swm}
C {lab_pin.sym} -420 -130 0 0 {name=l19 sig_type=std_logic lab=d1}
C {lab_pin.sym} -420 -70 0 0 {name=l20 sig_type=std_logic lab=vss}
C {lab_pin.sym} -460 -100 0 0 {name=l21 sig_type=std_logic lab=ph0}
C {lab_pin.sym} -460 -80 0 0 {name=l22 sig_type=std_logic lab=vss}
C {capa.sym} -520 60 0 0 {name=Cd1 value=1p m=1}
C {lab_pin.sym} -520 30 0 0 {name=l23 sig_type=std_logic lab=d1}
C {lab_pin.sym} -520 90 0 0 {name=l24 sig_type=std_logic lab=vss}
C {vsource.sym} -360 -260 0 0 {name=Vt2 value="PULSE(0 1 200n 0.5n 0.5n 99n 1u)" savecurrent=false}
C {lab_pin.sym} -360 -290 0 0 {name=l25 sig_type=std_logic lab=t2}
C {lab_pin.sym} -360 -230 0 0 {name=l26 sig_type=std_logic lab=vss}
C {vsource.sym} -280 -260 0 0 {name=Vl2 value="PULSE(0 1 205n 0.5n 0.5n 75n 1u)" savecurrent=false}
C {lab_pin.sym} -280 -290 0 0 {name=l27 sig_type=std_logic lab=l2}
C {lab_pin.sym} -280 -230 0 0 {name=l28 sig_type=std_logic lab=vss}
C {switch_ngspice.sym} -320 -100 0 0 {name=Sl2 model=swm}
C {lab_pin.sym} -320 -130 0 0 {name=l29 sig_type=std_logic lab=cmp}
C {lab_pin.sym} -320 -70 0 0 {name=l30 sig_type=std_logic lab=d2}
C {lab_pin.sym} -360 -100 0 0 {name=l31 sig_type=std_logic lab=l2}
C {lab_pin.sym} -360 -80 0 0 {name=l32 sig_type=std_logic lab=vss}
C {switch_ngspice.sym} -220 -100 0 0 {name=Sr2 model=swm}
C {lab_pin.sym} -220 -130 0 0 {name=l33 sig_type=std_logic lab=d2}
C {lab_pin.sym} -220 -70 0 0 {name=l34 sig_type=std_logic lab=vss}
C {lab_pin.sym} -260 -100 0 0 {name=l35 sig_type=std_logic lab=ph0}
C {lab_pin.sym} -260 -80 0 0 {name=l36 sig_type=std_logic lab=vss}
C {capa.sym} -320 60 0 0 {name=Cd2 value=1p m=1}
C {lab_pin.sym} -320 30 0 0 {name=l37 sig_type=std_logic lab=d2}
C {lab_pin.sym} -320 90 0 0 {name=l38 sig_type=std_logic lab=vss}
C {vsource.sym} -160 -260 0 0 {name=Vt3 value="PULSE(0 1 300n 0.5n 0.5n 99n 1u)" savecurrent=false}
C {lab_pin.sym} -160 -290 0 0 {name=l39 sig_type=std_logic lab=t3}
C {lab_pin.sym} -160 -230 0 0 {name=l40 sig_type=std_logic lab=vss}
C {vsource.sym} -80 -260 0 0 {name=Vl3 value="PULSE(0 1 305n 0.5n 0.5n 75n 1u)" savecurrent=false}
C {lab_pin.sym} -80 -290 0 0 {name=l41 sig_type=std_logic lab=l3}
C {lab_pin.sym} -80 -230 0 0 {name=l42 sig_type=std_logic lab=vss}
C {switch_ngspice.sym} -120 -100 0 0 {name=Sl3 model=swm}
C {lab_pin.sym} -120 -130 0 0 {name=l43 sig_type=std_logic lab=cmp}
C {lab_pin.sym} -120 -70 0 0 {name=l44 sig_type=std_logic lab=d3}
C {lab_pin.sym} -160 -100 0 0 {name=l45 sig_type=std_logic lab=l3}
C {lab_pin.sym} -160 -80 0 0 {name=l46 sig_type=std_logic lab=vss}
C {switch_ngspice.sym} -20 -100 0 0 {name=Sr3 model=swm}
C {lab_pin.sym} -20 -130 0 0 {name=l47 sig_type=std_logic lab=d3}
C {lab_pin.sym} -20 -70 0 0 {name=l48 sig_type=std_logic lab=vss}
C {lab_pin.sym} -60 -100 0 0 {name=l49 sig_type=std_logic lab=ph0}
C {lab_pin.sym} -60 -80 0 0 {name=l50 sig_type=std_logic lab=vss}
C {capa.sym} -120 60 0 0 {name=Cd3 value=1p m=1}
C {lab_pin.sym} -120 30 0 0 {name=l51 sig_type=std_logic lab=d3}
C {lab_pin.sym} -120 90 0 0 {name=l52 sig_type=std_logic lab=vss}
C {vsource.sym} 40 -260 0 0 {name=Vt4 value="PULSE(0 1 400n 0.5n 0.5n 99n 1u)" savecurrent=false}
C {lab_pin.sym} 40 -290 0 0 {name=l53 sig_type=std_logic lab=t4}
C {lab_pin.sym} 40 -230 0 0 {name=l54 sig_type=std_logic lab=vss}
C {vsource.sym} 120 -260 0 0 {name=Vl4 value="PULSE(0 1 405n 0.5n 0.5n 75n 1u)" savecurrent=false}
C {lab_pin.sym} 120 -290 0 0 {name=l55 sig_type=std_logic lab=l4}
C {lab_pin.sym} 120 -230 0 0 {name=l56 sig_type=std_logic lab=vss}
C {switch_ngspice.sym} 80 -100 0 0 {name=Sl4 model=swm}
C {lab_pin.sym} 80 -130 0 0 {name=l57 sig_type=std_logic lab=cmp}
C {lab_pin.sym} 80 -70 0 0 {name=l58 sig_type=std_logic lab=d4}
C {lab_pin.sym} 40 -100 0 0 {name=l59 sig_type=std_logic lab=l4}
C {lab_pin.sym} 40 -80 0 0 {name=l60 sig_type=std_logic lab=vss}
C {switch_ngspice.sym} 180 -100 0 0 {name=Sr4 model=swm}
C {lab_pin.sym} 180 -130 0 0 {name=l61 sig_type=std_logic lab=d4}
C {lab_pin.sym} 180 -70 0 0 {name=l62 sig_type=std_logic lab=vss}
C {lab_pin.sym} 140 -100 0 0 {name=l63 sig_type=std_logic lab=ph0}
C {lab_pin.sym} 140 -80 0 0 {name=l64 sig_type=std_logic lab=vss}
C {capa.sym} 80 60 0 0 {name=Cd4 value=1p m=1}
C {lab_pin.sym} 80 30 0 0 {name=l65 sig_type=std_logic lab=d4}
C {lab_pin.sym} 80 90 0 0 {name=l66 sig_type=std_logic lab=vss}
C {vsource.sym} 240 -260 0 0 {name=Vt5 value="PULSE(0 1 500n 0.5n 0.5n 99n 1u)" savecurrent=false}
C {lab_pin.sym} 240 -290 0 0 {name=l67 sig_type=std_logic lab=t5}
C {lab_pin.sym} 240 -230 0 0 {name=l68 sig_type=std_logic lab=vss}
C {vsource.sym} 320 -260 0 0 {name=Vl5 value="PULSE(0 1 505n 0.5n 0.5n 75n 1u)" savecurrent=false}
C {lab_pin.sym} 320 -290 0 0 {name=l69 sig_type=std_logic lab=l5}
C {lab_pin.sym} 320 -230 0 0 {name=l70 sig_type=std_logic lab=vss}
C {switch_ngspice.sym} 280 -100 0 0 {name=Sl5 model=swm}
C {lab_pin.sym} 280 -130 0 0 {name=l71 sig_type=std_logic lab=cmp}
C {lab_pin.sym} 280 -70 0 0 {name=l72 sig_type=std_logic lab=d5}
C {lab_pin.sym} 240 -100 0 0 {name=l73 sig_type=std_logic lab=l5}
C {lab_pin.sym} 240 -80 0 0 {name=l74 sig_type=std_logic lab=vss}
C {switch_ngspice.sym} 380 -100 0 0 {name=Sr5 model=swm}
C {lab_pin.sym} 380 -130 0 0 {name=l75 sig_type=std_logic lab=d5}
C {lab_pin.sym} 380 -70 0 0 {name=l76 sig_type=std_logic lab=vss}
C {lab_pin.sym} 340 -100 0 0 {name=l77 sig_type=std_logic lab=ph0}
C {lab_pin.sym} 340 -80 0 0 {name=l78 sig_type=std_logic lab=vss}
C {capa.sym} 280 60 0 0 {name=Cd5 value=1p m=1}
C {lab_pin.sym} 280 30 0 0 {name=l79 sig_type=std_logic lab=d5}
C {lab_pin.sym} 280 90 0 0 {name=l80 sig_type=std_logic lab=vss}
C {vsource.sym} 440 -260 0 0 {name=Vt6 value="PULSE(0 1 600n 0.5n 0.5n 99n 1u)" savecurrent=false}
C {lab_pin.sym} 440 -290 0 0 {name=l81 sig_type=std_logic lab=t6}
C {lab_pin.sym} 440 -230 0 0 {name=l82 sig_type=std_logic lab=vss}
C {vsource.sym} 520 -260 0 0 {name=Vl6 value="PULSE(0 1 605n 0.5n 0.5n 75n 1u)" savecurrent=false}
C {lab_pin.sym} 520 -290 0 0 {name=l83 sig_type=std_logic lab=l6}
C {lab_pin.sym} 520 -230 0 0 {name=l84 sig_type=std_logic lab=vss}
C {switch_ngspice.sym} 480 -100 0 0 {name=Sl6 model=swm}
C {lab_pin.sym} 480 -130 0 0 {name=l85 sig_type=std_logic lab=cmp}
C {lab_pin.sym} 480 -70 0 0 {name=l86 sig_type=std_logic lab=d6}
C {lab_pin.sym} 440 -100 0 0 {name=l87 sig_type=std_logic lab=l6}
C {lab_pin.sym} 440 -80 0 0 {name=l88 sig_type=std_logic lab=vss}
C {switch_ngspice.sym} 580 -100 0 0 {name=Sr6 model=swm}
C {lab_pin.sym} 580 -130 0 0 {name=l89 sig_type=std_logic lab=d6}
C {lab_pin.sym} 580 -70 0 0 {name=l90 sig_type=std_logic lab=vss}
C {lab_pin.sym} 540 -100 0 0 {name=l91 sig_type=std_logic lab=ph0}
C {lab_pin.sym} 540 -80 0 0 {name=l92 sig_type=std_logic lab=vss}
C {capa.sym} 480 60 0 0 {name=Cd6 value=1p m=1}
C {lab_pin.sym} 480 30 0 0 {name=l93 sig_type=std_logic lab=d6}
C {lab_pin.sym} 480 90 0 0 {name=l94 sig_type=std_logic lab=vss}
C {vsource.sym} 640 -260 0 0 {name=Vt7 value="PULSE(0 1 700n 0.5n 0.5n 99n 1u)" savecurrent=false}
C {lab_pin.sym} 640 -290 0 0 {name=l95 sig_type=std_logic lab=t7}
C {lab_pin.sym} 640 -230 0 0 {name=l96 sig_type=std_logic lab=vss}
C {vsource.sym} 720 -260 0 0 {name=Vl7 value="PULSE(0 1 705n 0.5n 0.5n 75n 1u)" savecurrent=false}
C {lab_pin.sym} 720 -290 0 0 {name=l97 sig_type=std_logic lab=l7}
C {lab_pin.sym} 720 -230 0 0 {name=l98 sig_type=std_logic lab=vss}
C {switch_ngspice.sym} 680 -100 0 0 {name=Sl7 model=swm}
C {lab_pin.sym} 680 -130 0 0 {name=l99 sig_type=std_logic lab=cmp}
C {lab_pin.sym} 680 -70 0 0 {name=l100 sig_type=std_logic lab=d7}
C {lab_pin.sym} 640 -100 0 0 {name=l101 sig_type=std_logic lab=l7}
C {lab_pin.sym} 640 -80 0 0 {name=l102 sig_type=std_logic lab=vss}
C {switch_ngspice.sym} 780 -100 0 0 {name=Sr7 model=swm}
C {lab_pin.sym} 780 -130 0 0 {name=l103 sig_type=std_logic lab=d7}
C {lab_pin.sym} 780 -70 0 0 {name=l104 sig_type=std_logic lab=vss}
C {lab_pin.sym} 740 -100 0 0 {name=l105 sig_type=std_logic lab=ph0}
C {lab_pin.sym} 740 -80 0 0 {name=l106 sig_type=std_logic lab=vss}
C {capa.sym} 680 60 0 0 {name=Cd7 value=1p m=1}
C {lab_pin.sym} 680 30 0 0 {name=l107 sig_type=std_logic lab=d7}
C {lab_pin.sym} 680 90 0 0 {name=l108 sig_type=std_logic lab=vss}
C {vsource.sym} 840 -260 0 0 {name=Vt8 value="PULSE(0 1 800n 0.5n 0.5n 99n 1u)" savecurrent=false}
C {lab_pin.sym} 840 -290 0 0 {name=l109 sig_type=std_logic lab=t8}
C {lab_pin.sym} 840 -230 0 0 {name=l110 sig_type=std_logic lab=vss}
C {vsource.sym} 920 -260 0 0 {name=Vl8 value="PULSE(0 1 805n 0.5n 0.5n 75n 1u)" savecurrent=false}
C {lab_pin.sym} 920 -290 0 0 {name=l111 sig_type=std_logic lab=l8}
C {lab_pin.sym} 920 -230 0 0 {name=l112 sig_type=std_logic lab=vss}
C {switch_ngspice.sym} 880 -100 0 0 {name=Sl8 model=swm}
C {lab_pin.sym} 880 -130 0 0 {name=l113 sig_type=std_logic lab=cmp}
C {lab_pin.sym} 880 -70 0 0 {name=l114 sig_type=std_logic lab=d8}
C {lab_pin.sym} 840 -100 0 0 {name=l115 sig_type=std_logic lab=l8}
C {lab_pin.sym} 840 -80 0 0 {name=l116 sig_type=std_logic lab=vss}
C {switch_ngspice.sym} 980 -100 0 0 {name=Sr8 model=swm}
C {lab_pin.sym} 980 -130 0 0 {name=l117 sig_type=std_logic lab=d8}
C {lab_pin.sym} 980 -70 0 0 {name=l118 sig_type=std_logic lab=vss}
C {lab_pin.sym} 940 -100 0 0 {name=l119 sig_type=std_logic lab=ph0}
C {lab_pin.sym} 940 -80 0 0 {name=l120 sig_type=std_logic lab=vss}
C {capa.sym} 880 60 0 0 {name=Cd8 value=1p m=1}
C {lab_pin.sym} 880 30 0 0 {name=l121 sig_type=std_logic lab=d8}
C {lab_pin.sym} 880 90 0 0 {name=l122 sig_type=std_logic lab=vss}
C {bsource.sym} -100 -480 0 0 {name=Bdac VAR=V FUNC="V(ref)/256*(128*(1+\{msb_err\})*max(0.5*(1+tanh((V(d1)-0.5)/0.02)),V(t1))+64*max(0.5*(1+tanh((V(d2)-0.5)/0.02)),V(t2))+32*max(0.5*(1+tanh((V(d3)-0.5)/0.02)),V(t3))+16*max(0.5*(1+tanh((V(d4)-0.5)/0.02)),V(t4))+8*max(0.5*(1+tanh((V(d5)-0.5)/0.02)),V(t5))+4*max(0.5*(1+tanh((V(d6)-0.5)/0.02)),V(t6))+2*max(0.5*(1+tanh((V(d7)-0.5)/0.02)),V(t7))+1*max(0.5*(1+tanh((V(d8)-0.5)/0.02)),V(t8)))" m=1}
C {lab_pin.sym} -100 -510 0 0 {name=l123 sig_type=std_logic lab=dac}
C {lab_pin.sym} -100 -450 0 0 {name=l124 sig_type=std_logic lab=vss}
C {bsource.sym} 20 -480 0 0 {name=Bcode VAR=V FUNC="128*u(V(d1)-0.5)+64*u(V(d2)-0.5)+32*u(V(d3)-0.5)+16*u(V(d4)-0.5)+8*u(V(d5)-0.5)+4*u(V(d6)-0.5)+2*u(V(d7)-0.5)+1*u(V(d8)-0.5)" m=1}
C {lab_pin.sym} 20 -510 0 0 {name=l125 sig_type=std_logic lab=code}
C {lab_pin.sym} 20 -450 0 0 {name=l126 sig_type=std_logic lab=vss}
