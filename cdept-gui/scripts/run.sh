#!/bin/sh
# Run the GUI test scripts on examples/buffer (see docs/GUI-TESTING.md).
#
#   cdept-gui/scripts/run.sh [script ...]     default: the buffer scripts
#
# Needs a display, and `cargo build && cargo build -p cdept-xschem` done before.
# Settings, xschem's files in the home folder and the project that is written to are
# all scratch copies: nothing of the user's and nothing in the repository changes.
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
cdept=${CDEPT:-$root/target/debug/cdept}
[ $# -gt 0 ] || set -- "$here/buffer-smoke.script" "$here/buffer-edit.script" \
  "$here/buffer-sim.script" "$here/buffer-preview.script"
status=0
for script in "$@"; do
  scratch=$(mktemp -d)
  cp -r "$root/examples/buffer" "$scratch/project"
  mkdir "$scratch/home" "$scratch/data"
  # A script that hangs must not hang the run.
  if HOME="$scratch/home" XDG_DATA_HOME="$scratch/data" \
      timeout 180 "$cdept" -p "$scratch/project" gui --script "$script" >"$scratch/log" 2>&1; then
    echo "ok      $(basename "$script")"
  else
    status=1
    echo "FAILED  $(basename "$script")"
    sed 's/\x1b\[[0-9;]*m//g' "$scratch/log" | grep -E 'ERROR|Error|cdept::script' | tail -5
  fi
  rm -rf "$scratch"
done
exit $status
