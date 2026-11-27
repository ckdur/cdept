#!/bin/sh
# The waveform tabs (views/wave) in the real GUI: hostile files, caches, a file that
# changes and goes, a read-only folder, a non-ASCII path under LC_ALL=C, a one-sample
# spike in 10 million samples, two tabs indexing one file, closing and cancelling a
# load, repeated open/close, and the large SAR ADC file (docs/NOTES.md, own waveform
# viewer).
#
#   cdept-gui/scripts/wave/run.sh [torture|loop|sar ...]   (default: torture loop sar)
#
# Needs a display, python3, unzip and a release build (cargo build --release -p cdept).
# Everything goes under target/wave-test (the SAR file, 845 MB unzipped, into
# target/sar; delete it afterwards). Each GUI run is capped at 6 GB.
set -eu
REPO=$(cd "$(dirname "$0")/../../.." && pwd)
HERE=$REPO/cdept-gui/scripts/wave
W=$REPO/target/wave-test
SAR=$REPO/target/sar
CDEPT=${CDEPT:-$REPO/target/release/cdept}
DATA=$W/torture
SHOTS=$W/shots
mkdir -p "$W/xdg" "$W/home" "$SHOTS" "$SAR"
[ -f "$SAR/SAR_top_1GB_tb_tran.raw" ] ||
    unzip -o -q "$REPO/cdept-uberwave/uberwave/release/SAR_top_1GB_tb_tran.raw.zip" -d "$SAR"

gui() { # script [env...]
    script=$1
    shift
    sed "s|@DATA@|$DATA|g; s|@SHOTS@|$SHOTS|g; s|@SAR@|$SAR|g" "$HERE/$script.script.in" >"$W/$script.script"
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
for what in ${*:-torture loop sar}; do
    case $what in
    torture)
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
        rm -f "$SAR/SAR_top_1GB_tb_tran.uwraw"
        gui sar || status=1
        grep -a "wv stats\|check\|drawn" "$W/sar.log" | sed 's/.*cdept::script: //' | cut -c 1-220
        ;;
    *) echo "unknown: $what" && exit 2 ;;
    esac
done
exit $status
