v {xschem version=3.4.5 file_version=1.2
}
G {}
K {}
V {}
S {}
E {}
T {Behavioural 3-bit flash ADC: a ladder of eight resistors between vref and vss (R5 is 10 % high on purpose), seven comparators (B sources, tanh with a 2 mV width) and an adder; code is the output code as a voltage, 0 to 7} -300 -560 0 0 0.25 0.25 {}
C {res.sym} -200 -460 0 0 {name=R8 value=10k footprint=1206 device=resistor m=1}
C {res.sym} -200 -400 0 0 {name=R7 value=10k footprint=1206 device=resistor m=1}
C {res.sym} -200 -340 0 0 {name=R6 value=10k footprint=1206 device=resistor m=1}
C {res.sym} -200 -280 0 0 {name=R5 value=11k footprint=1206 device=resistor m=1}
C {res.sym} -200 -220 0 0 {name=R4 value=10k footprint=1206 device=resistor m=1}
C {res.sym} -200 -160 0 0 {name=R3 value=10k footprint=1206 device=resistor m=1}
C {res.sym} -200 -100 0 0 {name=R2 value=10k footprint=1206 device=resistor m=1}
C {res.sym} -200 -40 0 0 {name=R1 value=10k footprint=1206 device=resistor m=1}
C {bsource.sym} 0 -200 0 0 {name=B1 VAR=V FUNC="0.5*(1+tanh((V(vin)-V(r1))/2e-3))" m=1}
C {bsource.sym} 80 -200 0 0 {name=B2 VAR=V FUNC="0.5*(1+tanh((V(vin)-V(r2))/2e-3))" m=1}
C {bsource.sym} 160 -200 0 0 {name=B3 VAR=V FUNC="0.5*(1+tanh((V(vin)-V(r3))/2e-3))" m=1}
C {bsource.sym} 240 -200 0 0 {name=B4 VAR=V FUNC="0.5*(1+tanh((V(vin)-V(r4))/2e-3))" m=1}
C {bsource.sym} 320 -200 0 0 {name=B5 VAR=V FUNC="0.5*(1+tanh((V(vin)-V(r5))/2e-3))" m=1}
C {bsource.sym} 400 -200 0 0 {name=B6 VAR=V FUNC="0.5*(1+tanh((V(vin)-V(r6))/2e-3))" m=1}
C {bsource.sym} 480 -200 0 0 {name=B7 VAR=V FUNC="0.5*(1+tanh((V(vin)-V(r7))/2e-3))" m=1}
C {bsource.sym} 300 -50 0 0 {name=Bcode VAR=V FUNC="V(t1)+V(t2)+V(t3)+V(t4)+V(t5)+V(t6)+V(t7)" m=1}
C {ipin.sym} -400 -100 0 0 {name=p1 lab=vin}
C {ipin.sym} -400 -60 0 0 {name=p2 lab=vref}
C {opin.sym} 420 -100 0 0 {name=p3 lab=code}
C {iopin.sym} -400 -20 2 0 {name=p4 lab=vss}
C {lab_pin.sym} -200 -490 0 0 {name=l1 sig_type=std_logic lab=vref}
C {lab_pin.sym} -200 -430 0 0 {name=l2 sig_type=std_logic lab=r7}
C {lab_pin.sym} -200 -370 0 0 {name=l3 sig_type=std_logic lab=r6}
C {lab_pin.sym} -200 -310 0 0 {name=l4 sig_type=std_logic lab=r5}
C {lab_pin.sym} -200 -250 0 0 {name=l5 sig_type=std_logic lab=r4}
C {lab_pin.sym} -200 -190 0 0 {name=l6 sig_type=std_logic lab=r3}
C {lab_pin.sym} -200 -130 0 0 {name=l7 sig_type=std_logic lab=r2}
C {lab_pin.sym} -200 -70 0 0 {name=l8 sig_type=std_logic lab=r1}
C {lab_pin.sym} -200 -10 0 0 {name=l9 sig_type=std_logic lab=vss}
C {lab_pin.sym} 0 -230 0 0 {name=l10 sig_type=std_logic lab=t1}
C {lab_pin.sym} 0 -170 0 0 {name=l11 sig_type=std_logic lab=vss}
C {lab_pin.sym} 80 -230 0 0 {name=l12 sig_type=std_logic lab=t2}
C {lab_pin.sym} 80 -170 0 0 {name=l13 sig_type=std_logic lab=vss}
C {lab_pin.sym} 160 -230 0 0 {name=l14 sig_type=std_logic lab=t3}
C {lab_pin.sym} 160 -170 0 0 {name=l15 sig_type=std_logic lab=vss}
C {lab_pin.sym} 240 -230 0 0 {name=l16 sig_type=std_logic lab=t4}
C {lab_pin.sym} 240 -170 0 0 {name=l17 sig_type=std_logic lab=vss}
C {lab_pin.sym} 320 -230 0 0 {name=l18 sig_type=std_logic lab=t5}
C {lab_pin.sym} 320 -170 0 0 {name=l19 sig_type=std_logic lab=vss}
C {lab_pin.sym} 400 -230 0 0 {name=l20 sig_type=std_logic lab=t6}
C {lab_pin.sym} 400 -170 0 0 {name=l21 sig_type=std_logic lab=vss}
C {lab_pin.sym} 480 -230 0 0 {name=l22 sig_type=std_logic lab=t7}
C {lab_pin.sym} 480 -170 0 0 {name=l23 sig_type=std_logic lab=vss}
C {lab_pin.sym} 300 -80 0 0 {name=l24 sig_type=std_logic lab=code}
C {lab_pin.sym} 300 -20 0 0 {name=l25 sig_type=std_logic lab=vss}
