v {xschem version=3.4.5 file_version=1.2
}
G {}
K {}
V {}
S {}
E {}
N -220 -250 0 -250 {lab=vdd}
N 0 -290 0 -250 {lab=vdd}
N -220 -190 -220 -170 {lab=ibias}
N -220 -170 -180 -170 {lab=ibias}
N -180 -170 -180 -220 {lab=ibias}
N -180 -220 -40 -220 {lab=ibias}
N -180 -140 -180 -170 {lab=ibias}
N 0 -190 0 -130 {lab=tail}
N -80 -130 80 -130 {lab=tail}
N -80 -70 -80 50 {lab=x}
N -80 50 80 50 {lab=x}
N -120 20 -80 20 {lab=x}
N 80 -70 80 -20 {lab=out}
N 120 20 120 -20 {lab=out}
N 80 -20 120 -20 {lab=out}
N 120 -20 180 -20 {lab=out}
N -120 80 120 80 {lab=vss}
N 0 80 0 120 {lab=vss}
T {5-transistor OTA: PMOS pair M1/M2 (bodies on the tail), NMOS mirror load M3/M4, PMOS tail M5 mirrored 1:1 from M6; ibias sinks the bias current} -300 -380 0 0 0.25 0.25 {}
C {sg13g2_pr/sg13_lv_pmos.sym} -20 -220 0 0 {name=M5
l=1u
w=32u
ng=1
m=1
model=sg13_lv_pmos
spiceprefix=X
}
C {sg13g2_pr/sg13_lv_pmos.sym} -200 -220 0 1 {name=M6
l=1u
w=32u
ng=1
m=1
model=sg13_lv_pmos
spiceprefix=X
}
C {sg13g2_pr/sg13_lv_pmos.sym} -100 -100 0 0 {name=M1
l=0.5u
w=30u
ng=1
m=1
model=sg13_lv_pmos
spiceprefix=X
}
C {sg13g2_pr/sg13_lv_pmos.sym} 100 -100 0 1 {name=M2
l=0.5u
w=30u
ng=1
m=1
model=sg13_lv_pmos
spiceprefix=X
}
C {sg13g2_pr/sg13_lv_nmos.sym} -100 50 0 1 {name=M3
l=4u
w=2u
ng=1
m=1
model=sg13_lv_nmos
spiceprefix=X
}
C {sg13g2_pr/sg13_lv_nmos.sym} 100 50 0 0 {name=M4
l=4u
w=2u
ng=1
m=1
model=sg13_lv_nmos
spiceprefix=X
}
C {ipin.sym} -300 -100 0 0 {name=p1 lab=inp}
C {ipin.sym} -300 -60 0 0 {name=p2 lab=inn}
C {ipin.sym} -180 -140 1 0 {name=p3 lab=ibias}
C {opin.sym} 180 -20 0 0 {name=p4 lab=out}
C {iopin.sym} 0 -290 3 0 {name=p5 lab=vdd}
C {iopin.sym} 0 120 1 0 {name=p6 lab=vss}
C {lab_pin.sym} 0 -220 0 0 {name=l1 sig_type=std_logic lab=vdd}
C {lab_pin.sym} -220 -220 2 0 {name=l2 sig_type=std_logic lab=vdd}
C {lab_pin.sym} -80 -100 0 0 {name=l3 sig_type=std_logic lab=tail}
C {lab_pin.sym} 80 -100 2 0 {name=l4 sig_type=std_logic lab=tail}
C {lab_pin.sym} -120 50 2 0 {name=l5 sig_type=std_logic lab=vss}
C {lab_pin.sym} 120 50 0 0 {name=l6 sig_type=std_logic lab=vss}
C {lab_pin.sym} -120 -100 2 0 {name=l7 sig_type=std_logic lab=inp}
C {lab_pin.sym} 120 -100 0 0 {name=l8 sig_type=std_logic lab=inn}
C {lab_pin.sym} -120 20 2 0 {name=l9 sig_type=std_logic lab=x}
C {lab_pin.sym} 0 -130 0 0 {name=l10 sig_type=std_logic lab=tail}
