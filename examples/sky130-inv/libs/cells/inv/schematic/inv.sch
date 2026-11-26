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
N 20 -110 20 -80 {lab=VPWR}
N 20 80 20 110 {lab=VGND}
C {ipin.sym} -60 0 0 0 {name=p1 lab=A}
C {opin.sym} 80 0 0 0 {name=p2 lab=Y}
C {iopin.sym} 20 -110 3 0 {name=p3 lab=VPWR}
C {iopin.sym} 20 110 1 0 {name=p4 lab=VGND}
C {lab_pin.sym} 20 -50 2 0 {name=l1 sig_type=std_logic lab=VPWR}
C {lab_pin.sym} 20 50 2 0 {name=l2 sig_type=std_logic lab=VGND}
C {sky130_fd_pr/pfet_01v8.sym} 0 -50 0 0 {name=M2
W=2
L=0.15
nf=1
mult=1
model=pfet_01v8
spiceprefix=X
}
C {sky130_fd_pr/nfet_01v8.sym} 0 50 0 0 {name=M1
W=1
L=0.15
nf=1
mult=1
model=nfet_01v8
spiceprefix=X
}
