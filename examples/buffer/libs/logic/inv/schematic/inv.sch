v {xschem version=3.4.5 file_version=1.2
}
G {}
K {}
V {}
S {}
E {}
N -20 -50 -20 50 {lab=A}
N -60 0 -20 0 {lab=A}
N 20 -20 20 20 {lab=Y}
N 20 0 80 0 {lab=Y}
N 20 -110 20 -80 {lab=VDD}
N 20 -110 50 -110 {lab=VDD}
N 50 -110 50 -50 {lab=VDD}
N 20 -50 50 -50 {lab=VDD}
N 20 80 20 110 {lab=VSS}
N 20 110 50 110 {lab=VSS}
N 50 50 50 110 {lab=VSS}
N 20 50 50 50 {lab=VSS}
C {ipin.sym} -60 0 0 0 {name=p1 lab=A}
C {opin.sym} 80 0 0 0 {name=p2 lab=Y}
C {iopin.sym} 20 -110 3 0 {name=p3 lab=VDD}
C {iopin.sym} 20 110 1 0 {name=p4 lab=VSS}
C {pmos4.sym} 0 -50 0 0 {name=M2 model=pmos w=2u l=0.35u del=0 m=1}
C {nmos4.sym} 0 50 0 0 {name=M1 model=nmos w=1u l=0.35u del=0 m=1}
