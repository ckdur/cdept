#!/bin/sh
# Replay a CDePT demo on scratch copies of the examples (nothing in the repository
# changes):
#
#   demo/run.sh buffer      the made-up technology, no PDK needed
#   demo/run.sh ihp         IHP SG13G2: technology, standard cells, nets, simulation
#
# DEMO_PACE=0.3 makes the pauses shorter (default 1); DEMO_PACE=0 runs it as a test.
# The window stays open at the end; close it when done. Environment: PDK_ROOT (default
# /opt/ext/OpenPDKs), CDEPT_NGSPICE, CDEPT_KLAYOUT, CDEPT_XSCHEM are set when the usual
# places have them.
set -e
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/.." && pwd)
demo=${1:-buffer}
case "$demo" in buffer) example=buffer ;; ihp) example=ihp-inv ;; *) echo "usage: $0 buffer|ihp" >&2; exit 2 ;; esac
cdept=${CDEPT:-$root/target/debug/cdept}
export PDK_ROOT=${PDK_ROOT:-/opt/ext/OpenPDKs}
[ -n "$CDEPT_NGSPICE" ] || { [ -x /opt/ext/OpenLane/bin/ngspice ] && export CDEPT_NGSPICE=/opt/ext/OpenLane/bin/ngspice; }
[ -n "$CDEPT_KLAYOUT" ] || { [ -x "$root/cdept-klayout/.bin/klayout" ] && export CDEPT_KLAYOUT=$root/cdept-klayout/.bin/klayout; }
[ -n "$CDEPT_XSCHEM" ] || { [ -x "$root/target/debug/cdept-xschem" ] && export CDEPT_XSCHEM=$root/target/debug/cdept-xschem; }
unset PDK
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
cp -r "$root/examples/$example" "$scratch/project"
mkdir "$scratch/home" "$scratch/data"
pace=${DEMO_PACE:-1}
awk -v pace="$pace" '$1 == "pause" { print "wait " ($2 * pace); next } { print }' \
  "$here/$demo.script" > "$scratch/demo.script"
[ "$pace" = 0 ] && echo quit >> "$scratch/demo.script"
HOME="$scratch/home" XDG_DATA_HOME="$scratch/data" \
  "$cdept" -p "$scratch/project" gui --script "$scratch/demo.script"
