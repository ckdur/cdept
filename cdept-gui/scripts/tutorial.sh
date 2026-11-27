#!/bin/sh
# Replay tutorials on the real GUI (needs a display, the IHP PDK, and for most of them
# `cargo build -p cdept-xschem`, ngspice and KLayout).
#
#   cdept-gui/scripts/tutorial.sh [-o FOLDER] [TUTORIAL ...]
#
# Without names, every script of tutorials/ runs. FOLDER gets a folder per tutorial
# with the screenshots, the log and the projects made (default: a new temporary one).
# PDKS is the folder that holds ihp-sg13g2 (default /opt/ext/OpenPDKs). HOME and the
# settings are scratch folders. pdk-setup runs without PDK_ROOT: it must work for
# someone who has not set it; the others get PDK_ROOT=$PDKS.
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
cdept=${CDEPT:-$root/target/debug/cdept}
pdks=${PDKS:-/opt/ext/OpenPDKs}
out=
if [ "$1" = -o ]; then out=$2; shift 2; fi
out=${out:-$(mktemp -d)}
mkdir -p "$out"
out=$(cd "$out" && pwd)
if [ $# -eq 0 ]; then
  for script in "$here"/tutorials/*.script; do
    set -- "$@" "$(basename "$script" .script)"
  done
fi
[ -n "$CDEPT_NGSPICE" ] || { [ -x /opt/ext/OpenLane/bin/ngspice ] && CDEPT_NGSPICE=/opt/ext/OpenLane/bin/ngspice && export CDEPT_NGSPICE; }
status=0
for name; do
  work=$out/$name
  rm -rf "$work"
  mkdir -p "$work/home" "$work/data" "$work/cwd"
  { echo "guide start $name"
    sed -e "s|@SCRATCH@|$work|g" -e "s|@PDKS@|$pdks|g" "$here/tutorials/$name.script"
  } > "$work/replay.script"
  if [ "$name" = pdk-setup ]; then root_var=; else root_var=$pdks; fi
  if (cd "$work/cwd" && env -u PDK -u PDK_ROOT ${root_var:+PDK_ROOT=$root_var} \
        HOME="$work/home" XDG_DATA_HOME="$work/data" \
        CDEPT_TUTORIAL_DIR="$work/home/cdept-tutorial" \
        timeout "${CDEPT_SCRIPT_TIMEOUT:-600}" "$cdept" gui --script "$work/replay.script" \
        >"$work/log" 2>&1); then
    echo "ok      $name"
  else
    status=1
    echo "FAILED  $name ($work/log)"
    sed 's/\x1b\[[0-9;]*m//g' "$work/log" | grep -E 'ERROR|Error|cdept::(script|guide)' | tail -6
  fi
done
echo "in $out"
exit $status
