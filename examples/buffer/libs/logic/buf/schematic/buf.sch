v {xschem version=3.4.5 file_version=1.2
}
G {}
K {}
V {}
S {}
E {}
N -140 0 -100 0 {lab=A}
N -20 0 20 0 {lab=n1}
N 100 0 140 0 {lab=Y}
N -100 -60 60 -60 {lab=VDD}
N -60 -60 -60 -40 {lab=VDD}
N 60 -60 60 -40 {lab=VDD}
N -100 60 60 60 {lab=VSS}
N -60 40 -60 60 {lab=VSS}
N 60 40 60 60 {lab=VSS}
C {ipin.sym} -140 0 0 0 {name=p1 lab=A}
C {opin.sym} 140 0 0 0 {name=p2 lab=Y}
C {iopin.sym} -100 -60 0 1 {name=p3 lab=VDD}
C {iopin.sym} -100 60 0 1 {name=p4 lab=VSS}
C {logic/inv/symbol/inv.sym} -60 0 0 0 {name=x1}
C {logic/inv/symbol/inv.sym} 60 0 0 0 {name=x2}
