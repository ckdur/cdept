v {xschem version=3.4.8RC file_version=1.3}
G {}
K {}
V {}
S {}
E {}
N -190 -20 -150 -20 {lab=in}
N 150 -20 190 -20 {lab=VDD}
N 150 0 250 0 {lab=out}
N 150 20 190 20 {lab=0}
N 190 20 190 60 {lab=0}
C {work/inv/symbol/inv.sym} 0 0 0 0 {name=x1}
C {lab_pin.sym} -190 -20 0 0 {name=l1 sig_type=std_logic lab=in}
C {lab_pin.sym} 190 -20 2 0 {name=l2 sig_type=std_logic lab=VDD}
C {lab_pin.sym} 250 0 2 0 {name=l3 sig_type=std_logic lab=out}
C {gnd.sym} 190 60 0 0 {name=l4 lab=0}
C {vsource.sym} -300 190 0 0 {name=Vdd value="\{vdd\}" savecurrent=false}
C {lab_pin.sym} -300 160 0 0 {name=l5 sig_type=std_logic lab=VDD}
C {gnd.sym} -300 220 0 0 {name=l6 lab=0}
C {vsource.sym} -80 190 0 0 {name=Vin value="pulse(0 \{vdd\} 200p 50p 50p 400p 1n)" savecurrent=false}
C {lab_pin.sym} -80 160 0 0 {name=l7 sig_type=std_logic lab=in}
C {gnd.sym} -80 220 0 0 {name=l8 lab=0}
C {capa.sym} 300 190 0 0 {name=CL value=10f m=1}
C {lab_pin.sym} 300 160 0 0 {name=l9 sig_type=std_logic lab=out}
C {gnd.sym} 300 220 0 0 {name=l10 lab=0}
