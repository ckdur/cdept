#!/bin/sh
# The waveform tabs (views/wave) in the real GUI: hostile files, caches, a file that
# changes and goes, a read-only folder, a non-ASCII path under LC_ALL=C, a one-sample
# spike in 10 million samples, two tabs indexing one file, closing and cancelling a
# load, repeated open/close, and the large SAR ADC file (docs/NOTES.md, own waveform
# viewer).
#
#   cdept-gui/scripts/wave/run.sh [smoke|menu|features|family|analyses|big|torture|loop|sar|huge ...]
#       (default: smoke menu features family analyses torture loop sar)
#
# smoke: small inputs (small.py), what CI runs; menu: File > Open in UberWave... with
# the built-in viewer and the process (fake-uberwave.sh stands in for UberWaveViewer).
# analyses: AC, Bode, eye, noise, operating point, plots of one file, sweeps that go
# back, PSF folders, hostile complex files (analyses.py, plus ngspice files when
# CDEPT_NGSPICE is set, Xyce's with CDEPT_XYCE, VACASK's with CDEPT_VACASK; without them
# the script is cut to what the generated files cover). huge: a binary raw file of
# HUGE_POINTS points (default 100 million; HUGE_SIGNALS signals, default 2), its first
# open, cache, memory, zoom times and a one-sample spike; deleted afterwards.
#
# Needs a display, python3, unzip and a release build (cargo build --release -p cdept).
# Everything goes under target/wave-test (the SAR file, 845 MB unzipped, into
# target/sar, removed at the end unless KEEP_SAR=1). Each GUI run is capped at 6 GB.
set -eu
REPO=$(cd "$(dirname "$0")/../../.." && pwd)
HERE=$REPO/cdept-gui/scripts/wave
W=$REPO/target/wave-test
SAR=$REPO/target/sar
CDEPT=${CDEPT:-$REPO/target/release/cdept}
DATA=$W/torture
SHOTS=$W/shots
SMALL=$W/small
mkdir -p "$W/xdg" "$W/home" "$SHOTS"
need_sar() {
    mkdir -p "$SAR"
    [ -f "$SAR/SAR_top_1GB_tb_tran.raw" ] ||
        unzip -o -q "$REPO/cdept-uberwave/uberwave/release/SAR_top_1GB_tb_tran.raw.zip" -d "$SAR"
}
fresh_small() {
    rm -rf "$SMALL"
    python3 -I "$HERE/small.py" "$SMALL" >/dev/null
}

gui() { # script [env...]
    script=$1
    shift
    data=$DATA
    case $script in smoke | menu | features | family) data=$SMALL ;; esac
    sed "s|@DATA@|$data|g; s|@SHOTS@|$SHOTS|g; s|@SAR@|$SAR|g" "$HERE/$script.script.in" >"$W/$script.script"
    env "$@" DISPLAY="${DISPLAY:-:0}" XDG_DATA_HOME="$W/xdg" HOME="$W/home" \
        timeout 900 systemd-run --user --scope -q -p MemoryMax=6G -p MemorySwapMax=0 \
        "$CDEPT" gui --script "$W/$script.script" >"$W/$script.log" 2>&1 || {
        echo "$script: FAILED (see $W/$script.log)"
        grep -a "ERROR" "$W/$script.log" | tail -3
        return 1
    }
    echo "$script: ok ($W/$script.log)"
}

fresh_data() {
    chmod -R u+w "$DATA" 2>/dev/null || true
    rm -rf "$DATA" "$W/home/.cache" "$SHOTS"/flag*.png
    python3 -I "$HERE/gen.py" "$DATA" >/dev/null
    gui prep
    python3 -I "$HERE/corrupt.py" "$DATA" >/dev/null
}

status=0
for what in ${*:-smoke menu features family analyses torture loop sar}; do
    case $what in
    analyses)
        A=$W/analyses
        rm -rf "$A"
        mkdir -p "$A/sim"
        python3 -I "$HERE/analyses.py" "$A/data" >/dev/null
        cp -r "$REPO/cdept-uberwave/uberwave/wp3-data-engine/tests/psf" "$A/sim/psf"
        onoise=2.0350e-6
        if [ -n "${CDEPT_NGSPICE:-}" ]; then
            (cd "$A/sim" && sh "$HERE/analyses-sim.sh" ngspice "$CDEPT_NGSPICE") || status=1
            onoise=$(cat "$A/sim/onoise_total" 2>/dev/null || echo 2.0350e-6)
        fi
        [ -n "${CDEPT_XYCE:-}" ] && { (cd "$A/sim" && sh "$HERE/analyses-sim.sh" xyce "$CDEPT_XYCE") || status=1; }
        [ -n "${CDEPT_VACASK:-}" ] && { (cd "$A/sim" && sh "$HERE/analyses-sim.sh" vacask "$CDEPT_VACASK") || status=1; }
        # without a simulator's files, the parts that need them are left out
        python3 -I - "$HERE/analyses.script.in" "$W/analyses.script" "$A" "$SHOTS" "$onoise" <<'PY'
import os, sys
src, dst, a, shots, onoise = sys.argv[1:6]
sim = os.path.join(a, "sim")
need = {"loop.raw": "Bode of a three-pole", "multi.raw": "one file, six", "xyce_ascii.raw": "Xyce and", "ac1.raw": "Xyce and"}
blocks = open(src).read().split("\n# --- ")
out = [blocks[0]]
for b in blocks[1:]:
    files = [f for f, title in need.items() if b.startswith(title)]
    if any(not os.path.exists(os.path.join(sim, f)) for f in files):
        print("analyses: left out (no simulator file):", b.splitlines()[0])
        continue
    out.append(b)
t = "\n# --- ".join(out)
t = t.replace("@DATA@", os.path.join(a, "data")).replace("@SIM@", sim).replace("@SHOTS@", shots).replace("@ONOISE@", onoise)
open(dst, "w").write(t)
PY
        env DISPLAY="${DISPLAY:-:0}" XDG_DATA_HOME="$W/xdg" HOME="$W/home" \
            timeout 900 systemd-run --user --scope -q -p MemoryMax=6G -p MemorySwapMax=0 \
            "$CDEPT" gui --script "$W/analyses.script" >"$W/analyses.log" 2>&1 && echo "analyses: ok ($W/analyses.log)" || {
            echo "analyses: FAILED (see $W/analyses.log)"
            grep -a "ERROR" "$W/analyses.log" | tail -3
            status=1
        }
        ;;
    big)
        B=$W/big
        rm -rf "$B"
        python3 -I "$HERE/big.py" "$B" 600 >/dev/null
        python3 -I - "$HERE/big.script.in" "$W/big.script" "$B" "$SHOTS" <<'PY'
import sys
src, dst, data, shots = sys.argv[1:5]
fam = " ".join("%s/family/p%04d/out.raw" % (data, i) for i in range(600))
hundred = " ".join("%s/family/p%04d/out.raw" % (data, i) for i in range(100, 200))
panes = "\n".join("wv show_in new v(n%03d)" % i for i in range(300))
loop = "\n".join("wv open_files %s\nwv expect_points loading 0\nwv close" % hundred for _ in range(15))
touch = "sleep 1.1 && " + " && ".join("printf x >> %s/family/p%04d/out.raw" % (data, i) for i in (1, 2, 3))
t = open(src).read()
t = t.replace("@OPEN600@", "wv open_files " + fam).replace("@PANES300@", panes)
t = t.replace("@LOOP100@", loop).replace("@TOUCH@", touch)
t = t.replace("@DATA@", data).replace("@SHOTS@", shots)
open(dst, "w").write(t)
PY
        env DISPLAY="${DISPLAY:-:0}" XDG_DATA_HOME="$W/xdg" HOME="$W/home" \
            timeout 900 systemd-run --user --scope -q -p MemoryMax=6G -p MemorySwapMax=0 \
            "$CDEPT" gui --script "$W/big.script" >"$W/big.log" 2>&1 && echo "big: ok ($W/big.log)" || {
            echo "big: FAILED (see $W/big.log)"
            grep -a "ERROR" "$W/big.log" | tail -3
            status=1
        }
        grep -a "wv stats" "$W/big.log" | sed 's/.*wv stats: //' | cut -c 1-400
        rm -rf "$B"
        ;;
    huge)
        # HUGE_DIR: where the file goes (default target/wave-test/huge); it and its cache
        # are removed at the end unless KEEP_HUGE=1.
        H=${HUGE_DIR:-$W/huge}
        n=${HUGE_POINTS:-100000000}
        rm -rf "$H"
        mkdir -p "$H"
        start=$(date +%s.%N)
        python3 -I "$HERE/huge.py" "$H/huge.raw" "$n" "${HUGE_SIGNALS:-2}" || { status=1; continue; }
        echo "huge: wrote $(du -h "$H/huge.raw" | cut -f1) in $(python3 -c "print(round($(date +%s.%N) - $start, 1))") s"
        python3 -I - "$HERE/huge.script.in" "$W/huge.script" "$H/huge.raw" "$SHOTS" "$n" <<'PY'
import sys
src, dst, f, shots, n = sys.argv[1:6]
n = int(float(n))
spike = int(n * 0.618) * 1e-9
end = (n - 1) * 1e-9
sub = {"@FILE@": f, "@SHOTS@": shots, "@SPIKE@": "%.12g" % spike, "@END@": "%.12g" % end,
       "@SPIKE_A@": "%.12g" % (spike - end / 1000), "@SPIKE_B@": "%.12g" % (spike + end / 1000),
       "@SPIKE_C@": "%.12g" % (spike - 50e-9), "@SPIKE_D@": "%.12g" % (spike + 50e-9)}
t = open(src).read()
for k, v in sub.items():
    t = t.replace(k, v)
open(dst, "w").write(t)
PY
        env DISPLAY="${DISPLAY:-:0}" XDG_DATA_HOME="$W/xdg" HOME="$W/home" \
            timeout 3600 systemd-run --user --scope -q -p MemoryMax=${HUGE_MEMORY:-12G} -p MemorySwapMax=0 \
            "$CDEPT" gui --script "$W/huge.script" >"$W/huge.log" 2>&1 && echo "huge: ok ($W/huge.log)" || {
            echo "huge: FAILED (see $W/huge.log)"
            grep -a "ERROR" "$W/huge.log" | tail -3
            status=1
        }
        grep -a "wv stats\|opened in\|drawn\|exact" "$W/huge.log" | sed 's/.*cdept::[a-z]*: //' | cut -c 1-600
        ls -la "$H"
        [ "${KEEP_HUGE:-0}" = 1 ] || rm -rf "$H"
        ;;
    smoke)
        fresh_small
        gui smoke || status=1
        ;;
    features | family)
        fresh_small
        gui "$what" || status=1
        ;;
    menu)
        fresh_small
        gui menu CDEPT_UBERWAVE="$REPO/cdept-gui/scripts/fake-uberwave.sh" || status=1
        ;;
    torture)
        need_sar
        fresh_data
        rm -f "$SAR/SAR_top_1GB_tb_tran.uwraw"
        sh "$HERE/background.sh" "$DATA" "$SHOTS" "$SAR" &
        helper=$!
        gui torture LC_ALL=C || status=1
        kill $helper 2>/dev/null || true
        ;;
    loop)
        [ -d "$DATA" ] || fresh_data
        gui loop || status=1
        ;;
    sar)
        need_sar
        rm -f "$SAR/SAR_top_1GB_tb_tran.uwraw"
        gui sar || status=1
        grep -a "wv stats\|check\|drawn" "$W/sar.log" | sed 's/.*cdept::script: //' | cut -c 1-220
        ;;
    *) echo "unknown: $what" && exit 2 ;;
    esac
done
# The unzipped SAR file is 845 MB (and its cache as much): gone unless KEEP_SAR=1.
[ "${KEEP_SAR:-0}" = 1 ] || rm -rf "$SAR"
exit $status
