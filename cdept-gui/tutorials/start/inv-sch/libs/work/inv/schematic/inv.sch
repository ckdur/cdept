v {xschem version=3.4.5 file_version=1.2
}
G {}
K {}
V {}
S {}
E {}
N -20 -50 -20 0 {lab=A}
N -20 0 -20 50 {lab=A}
N -60 0 -20 0 {lab=A}
N 20 -20 20 0 {lab=Y}
N 20 0 20 20 {lab=Y}
N 20 0 80 0 {lab=Y}
N 20 -110 20 -80 {lab=VDD}
N 20 80 20 110 {lab=VSS}
C {ipin.sym} -60 0 0 0 {name=p1 lab=A}
C {opin.sym} 80 0 0 0 {name=p2 lab=Y}
C {iopin.sym} 20 -110 3 0 {name=p3 lab=VDD}
C {iopin.sym} 20 110 1 0 {name=p4 lab=VSS}
C {lab_pin.sym} 20 -50 2 0 {name=l1 sig_type=std_logic lab=VDD}
C {lab_pin.sym} 20 50 2 0 {name=l2 sig_type=std_logic lab=VSS}
C {sg13g2_pr/sg13_lv_pmos.sym} 0 -50 0 0 {name=M2
l=0.13u
w=1.5u
ng=1
m=1
model=sg13_lv_pmos
spiceprefix=X
}
C {sg13g2_pr/sg13_lv_nmos.sym} 0 50 0 0 {name=M1
l=0.13u
w=0.74u
ng=1
m=1
model=sg13_lv_nmos
spiceprefix=X
}
