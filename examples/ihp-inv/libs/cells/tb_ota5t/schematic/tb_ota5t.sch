v {xschem version=3.4.5 file_version=1.2
}
G {}
K {}
V {}
S {}
E {}
T {OTA open loop: the DC loop is closed by Lfb (1 TH), AC goes into inn through Cin (1 F); AC on Vcm for CMRR, on Vdd for PSRR} -460 -160 0 0 0.3 0.3 {}
C {cells/ota5t/symbol/ota5t.sym} 0 0 0 0 {name=xdut}
C {vsource.sym} -400 200 0 0 {name=Vdd value="dc \{vdd\} ac \{ac_vdd\}" savecurrent=false}
C {isource.sym} -300 200 0 0 {name=Ibias value="\{ibias\}"}
C {vsource.sym} -200 200 0 0 {name=Vcm value="dc \{vcm\} ac \{ac_cm\}" savecurrent=false}
C {vsource.sym} -100 200 0 0 {name=Vin value="dc 0 ac \{ac_dm\}" savecurrent=false}
C {capa.sym} 0 200 0 0 {name=Cin value=1 m=1}
C {ind.sym} 100 200 0 0 {name=Lfb value=1T m=1}
C {capa.sym} 200 200 0 0 {name=CL value="\{cl\}" m=1}
C {lab_pin.sym} -80 -20 0 0 {name=l1 sig_type=std_logic lab=cm}
C {lab_pin.sym} -80 20 0 0 {name=l2 sig_type=std_logic lab=inn}
C {lab_pin.sym} 80 0 0 0 {name=l3 sig_type=std_logic lab=out}
C {lab_pin.sym} 0 -60 0 0 {name=l4 sig_type=std_logic lab=vdd}
C {lab_pin.sym} 0 60 0 0 {name=l5 sig_type=std_logic lab=0}
C {lab_pin.sym} -20 60 0 0 {name=l6 sig_type=std_logic lab=ibias}
C {lab_pin.sym} -400 170 0 0 {name=l7 sig_type=std_logic lab=vdd}
C {lab_pin.sym} -400 230 0 0 {name=l8 sig_type=std_logic lab=0}
C {lab_pin.sym} -300 170 0 0 {name=l9 sig_type=std_logic lab=ibias}
C {lab_pin.sym} -300 230 0 0 {name=l10 sig_type=std_logic lab=0}
C {lab_pin.sym} -200 170 0 0 {name=l11 sig_type=std_logic lab=cm}
C {lab_pin.sym} -200 230 0 0 {name=l12 sig_type=std_logic lab=0}
C {lab_pin.sym} -100 170 0 0 {name=l13 sig_type=std_logic lab=in}
C {lab_pin.sym} -100 230 0 0 {name=l14 sig_type=std_logic lab=cm}
C {lab_pin.sym} 0 170 0 0 {name=l15 sig_type=std_logic lab=in}
C {lab_pin.sym} 0 230 0 0 {name=l16 sig_type=std_logic lab=inn}
C {lab_pin.sym} 100 170 0 0 {name=l17 sig_type=std_logic lab=out}
C {lab_pin.sym} 100 230 0 0 {name=l18 sig_type=std_logic lab=inn}
C {lab_pin.sym} 200 170 0 0 {name=l19 sig_type=std_logic lab=out}
C {lab_pin.sym} 200 230 0 0 {name=l20 sig_type=std_logic lab=0}
