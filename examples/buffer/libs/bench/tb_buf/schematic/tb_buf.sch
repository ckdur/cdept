v {xschem version=3.4.5 file_version=1.2
}
G {}
K {}
V {}
S {}
E {}
N -220 0 -40 0 {lab=in}
N 40 0 120 0 {lab=out}
C {logic/buf/symbol/buf.sym} 0 0 0 0 {name=x1}
C {vdd.sym} 0 -40 0 0 {name=l1 lab=VDD}
C {gnd.sym} 0 40 0 0 {name=l2 lab=0}
C {vsource.sym} -340 30 0 0 {name=Vdd value=1.8 savecurrent=false}
C {vdd.sym} -340 0 0 0 {name=l3 lab=VDD}
C {gnd.sym} -340 60 0 0 {name=l4 lab=0}
C {vsource.sym} -220 30 0 0 {name=Vin value="pulse(0 1.8 1n 100p 100p 2n 5n)" savecurrent=false}
C {gnd.sym} -220 60 0 0 {name=l5 lab=0}
C {lab_wire.sym} -120 0 0 0 {name=l7 lab=in}
C {lab_wire.sym} 80 0 0 0 {name=l8 lab=out}
C {capa.sym} 120 30 0 0 {name=C1 value=20f m=1}
C {gnd.sym} 120 60 0 0 {name=l6 lab=0}
C {code_shown.sym} -360 120 0 0 {name=SIM only_toplevel=true value=".model nmos nmos level=1 vto=0.5 kp=200u
.model pmos pmos level=1 vto=-0.5 kp=80u
.tran 20p 12n
.control
run
meas tran delay trig v(in) val=0.9 rise=1 targ v(out) val=0.9 rise=1
.endc"}
