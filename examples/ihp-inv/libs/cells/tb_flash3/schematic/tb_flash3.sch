v {xschem version=3.4.5 file_version=1.2
}
G {}
K {}
V {}
S {}
E {}
T {The flash ADC with its input swept by a DC analysis (Vin), reference Vref} -300 -160 0 0 0.3 0.3 {}
C {cells/flash3/symbol/flash3.sym} 0 0 0 0 {name=xadc}
C {vsource.sym} -300 200 0 0 {name=Vref value="\{vref\}" savecurrent=false}
C {vsource.sym} -200 200 0 0 {name=Vin value=0 savecurrent=false}
C {lab_pin.sym} -60 -20 0 0 {name=l1 sig_type=std_logic lab=in}
C {lab_pin.sym} -60 20 0 0 {name=l2 sig_type=std_logic lab=ref}
C {lab_pin.sym} 60 0 0 0 {name=l3 sig_type=std_logic lab=code}
C {lab_pin.sym} 0 60 0 0 {name=l4 sig_type=std_logic lab=0}
C {lab_pin.sym} -300 170 0 0 {name=l5 sig_type=std_logic lab=ref}
C {lab_pin.sym} -300 230 0 0 {name=l6 sig_type=std_logic lab=0}
C {lab_pin.sym} -200 170 0 0 {name=l7 sig_type=std_logic lab=in}
C {lab_pin.sym} -200 230 0 0 {name=l8 sig_type=std_logic lab=0}
