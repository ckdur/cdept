#!/bin/sh
# The simulators' files of run.sh analyses, written into the current folder.
#   analyses-sim.sh ngspice|xyce|vacask <program>
set -eu
case $1 in
ngspice)
    cat >loop.cir <<'NET'
* three-pole loop gain: A0 = 1e4, poles 1k, 1meg, 20meg
V1 in 0 dc 0 ac 1
E1 a 0 in 0 1e4
R1 a b 1k
C1 b 0 159.1549431n
E2 b2 0 b 0 1
R2 b2 c 1k
C2 c 0 159.1549431p
E3 c2 0 c 0 1
R3 c2 out 1k
C3 out 0 7.957747155p
.control
ac dec 100 1 1g
let gdb = db(v(out))
let ph = 180/pi*cph(v(out))
meas ac ugf when gdb=0
meas ac pm find ph when gdb=0
meas ac f180 when ph=-180
meas ac gm find gdb when ph=-180
write loop.raw v(out) v(in) v(a) v(b) v(c)
.endc
.end
NET
    "$2" -b loop.cir >loop.log 2>&1
    grep -E "^(ugf|pm|f180|gm) " loop.log || true
    cat >multi.cir <<'NET'
* op, a parametric dc whose inner sweep runs backwards, ac, noise, tran
V1 in 0 dc 0.9 ac 1 sin(0.9 0.5 1meg)
V2 vb 0 dc 1
R1 in out 1k
R2 out vb 2k
C1 out 0 1n
.option sparse
.op
.dc V1 1.8 0 -0.05 V2 0 2 1
.ac dec 20 1 1g
.noise v(out) V1 dec 20 1 1g
.tran 10n 5u
.end
NET
    "$2" -b -r multi.raw multi.cir >multi.log 2>&1
    SPICE_ASCIIRAWFILE=1 "$2" -b -r multi_ascii.raw multi.cir >/dev/null 2>&1
    # ngspice's integrated output noise, for the check of the tab's integration
    cat >noise.cir <<'NET'
* integrated noise
V1 in 0 dc 0.9 ac 1
V2 vb 0 dc 1
R1 in out 1k
R2 out vb 2k
C1 out 0 1n
.option sparse
.control
noise v(out) V1 dec 20 1 1g
setplot noise2
print onoise_total
.endc
.end
NET
    "$2" -b noise.cir 2>/dev/null | sed -n 's/^onoise_total *= *//p' | head -1 >onoise_total
    ;;
xyce)
    printf 'RC lowpass\nV1 in 0 dc 0 ac 1\nR1 in out 1k\nC1 out 0 1n\n.ac dec 10 1 100meg\n.print ac v(out)\n.end\n' >rcx.cir
    OMPI_MCA_plm=isolated OMPI_MCA_btl=self PMIX_MCA_gds=hash "$2" -r xyce_ascii.raw -a rcx.cir </dev/null >xyce.log 2>&1
    ;;
vacask)
    cat >rc.sim <<'NET'
RC lowpass

load "resistor.osdi"
load "capacitor.osdi"
include "builtins.inc"

model r resistor
model c capacitor

v1 (in 0) vsource dc=0 mag=1
r1 (in out) r r=1k
c1 (out 0) c c=1n

control
  analysis ac1 ac from=1 to=100e6 mode="dec" points=10
  analysis noise1 noise out="out" in="v1" from=1 to=100e6 mode="dec" points=10
endc
NET
    LD_LIBRARY_PATH="$(dirname "$(dirname "$2")")/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" "$2" rc.sim </dev/null >vacask.log 2>&1
    ;;
esac
