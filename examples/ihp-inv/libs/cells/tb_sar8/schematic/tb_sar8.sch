v {xschem version=3.4.5 file_version=1.2
}
G {}
K {}
V {}
S {}
E {}
T {The 8-bit SAR ADC converting a sine: Vin = vref/2 + ampl sin(2 pi fin t), fin = 11/128 MHz so that 128 conversions at 1 MS/s hold a whole, odd number of cycles (coherent sampling); reference Vref} -420 -160 0 0 0.3 0.3 {}
C {cells/sar8/symbol/sar8.sym} 0 0 0 0 {name=xadc}
C {vsource.sym} -400 200 0 0 {name=Vref value="\{vref\}" savecurrent=false}
C {vsource.sym} -300 200 0 0 {name=Vin value="SIN(\{vref/2\} \{ampl\} \{fin\})" savecurrent=false}
C {lab_pin.sym} -150 -10 0 0 {name=l1 sig_type=std_logic lab=in}
C {lab_pin.sym} -150 10 0 0 {name=l2 sig_type=std_logic lab=ref}
C {lab_pin.sym} 150 -10 0 0 {name=l3 sig_type=std_logic lab=code}
C {lab_pin.sym} 150 10 0 0 {name=l4 sig_type=std_logic lab=0}
C {lab_pin.sym} -400 170 0 0 {name=l5 sig_type=std_logic lab=ref}
C {lab_pin.sym} -400 230 0 0 {name=l6 sig_type=std_logic lab=0}
C {lab_pin.sym} -300 170 0 0 {name=l7 sig_type=std_logic lab=in}
C {lab_pin.sym} -300 230 0 0 {name=l8 sig_type=std_logic lab=0}
