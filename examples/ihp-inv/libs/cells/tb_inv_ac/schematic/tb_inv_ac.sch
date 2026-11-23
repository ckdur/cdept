v {xschem version=3.4.5 file_version=1.2
}
G {}
K {}
V {}
S {}
E {}
C {cells/inv/symbol/inv.sym} 0 0 0 0 {name=x1}
N -40 0 -100 0 {lab=in}
C {lab_wire.sym} -100 0 0 0 {name=ln0 lab=in}
N 40 0 100 0 {lab=out}
C {lab_wire.sym} 100 0 0 0 {name=ln1 lab=out}
N 0 -40 0 -100 {lab=VDD}
C {vdd.sym} 0 -100 0 0 {name=lv2 lab=VDD}
N 0 40 0 100 {lab=0}
C {gnd.sym} 0 100 0 0 {name=lg3 lab=0}
C {vsource.sym} -400 230 0 0 {name=Vdd value="\{vdd\}" savecurrent=false}
C {vdd.sym} -400 200 0 0 {name=lsv lab=VDD}
C {gnd.sym} -400 260 0 0 {name=lsg lab=0}
C {vsource.sym} -280 230 0 0 {name=Vin value="dc \{vbias\} ac 1" savecurrent=false}
C {lab_wire.sym} -280 200 0 0 {name=lsi lab=in}
C {gnd.sym} -280 260 0 0 {name=lig lab=0}
C {capa.sym} -160 230 0 0 {name=CL value=10f m=1}
C {lab_wire.sym} -160 200 0 0 {name=lso lab=out}
C {gnd.sym} -160 260 0 0 {name=lcg lab=0}
