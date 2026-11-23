#!/bin/sh
# Regenerate the manual's screenshots: runs docs/screenshots/*.script (or the scripts
# given as arguments) in the real GUI, each on a scratch copy of examples/ihp-inv with a
# scratch HOME and XDG_DATA_HOME, so nothing of the repository or the user changes.
# PNGs go to docs/_static/screenshots/. Needs a display, a built target/debug/cdept,
# cdept-xschem, cdept-klayout/.bin/klayout, and the PDK/ngspice below.
#
#   scripts/gen-doc-screenshots.sh [script ...]
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/.." && pwd)
cdept=${CDEPT:-$root/target/debug/cdept}
out=$root/docs/_static/screenshots
mkdir -p "$out"
export PDK_ROOT=${PDK_ROOT:-/opt/ext/OpenPDKs}
export CDEPT_NGSPICE=${CDEPT_NGSPICE:-/opt/ext/OpenLane/bin/ngspice}
export CDEPT_XSCHEM=${CDEPT_XSCHEM:-$root/target/debug/cdept-xschem}
[ $# -gt 0 ] || set -- "$root"/docs/screenshots/*.script
status=0
for script in "$@"; do
  scratch=$(mktemp -d)
  cp -r "$root/examples/ihp-inv" "$scratch/project"
  mkdir "$scratch/home" "$scratch/data"
  sed "s|@SHOTS@|$out|g" "$script" >"$scratch/run.script"
  if HOME="$scratch/home" XDG_DATA_HOME="$scratch/data" \
      timeout 300 "$cdept" -p "$scratch/project" gui --script "$scratch/run.script" \
      >"$scratch/log" 2>&1; then
    echo "ok      $(basename "$script")"
  else
    status=1
    echo "FAILED  $(basename "$script")"
    sed 's/\x1b\[[0-9;]*m//g' "$scratch/log" | grep -E 'ERROR|Error|cdept::script' | tail -5
  fi
  rm -rf "$scratch"
done
# Keep the PNGs small: quantize to 128 colours when ImageMagick is there.
if command -v convert >/dev/null 2>&1; then
  for f in "$out"/*.png; do
    [ "$(wc -c <"$f")" -gt 300000 ] && convert "$f" -colors 128 -strip "$f"
  done
fi
exit $status
