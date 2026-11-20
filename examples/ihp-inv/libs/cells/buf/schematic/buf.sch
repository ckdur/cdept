v {xschem version=3.4.5 file_version=1.2
}
G {}
K {}
V {}
S {}
E {}
N -100 0 -40 0 {lab=A}
N 40 0 80 0 {lab=mid}
N 160 0 200 0 {lab=Y}
C {ipin.sym} -100 0 0 0 {name=p1 lab=A}
C {opin.sym} 200 0 0 0 {name=p2 lab=Y}
C {iopin.sym} 0 -80 3 0 {name=p3 lab=VDD}
C {iopin.sym} 0 80 1 0 {name=p4 lab=VSS}
C {cells/inv/symbol/inv.sym} 0 0 0 0 {name=x1}
C {cells/inv/symbol/inv.sym} 120 0 0 0 {name=x2}
C {lab_pin.sym} 0 -40 0 0 {name=l1 lab=VDD}
C {lab_pin.sym} 0 40 0 0 {name=l2 lab=VSS}
C {lab_pin.sym} 120 -40 0 0 {name=l3 lab=VDD}
C {lab_pin.sym} 120 40 0 0 {name=l4 lab=VSS}
