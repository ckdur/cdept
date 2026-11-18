#!/bin/sh
# Run the tests of the Tk stand-in in the xschem engine (cdx-tcl).
#   tests/run.sh [test.tcl ...]      default: every t_*.tcl here
# Each test prints PASS or FAIL lines; the exit status is non-zero on any FAIL or error.
# CDX_TCL names the cdx-tcl program (default: target/debug/cdx-tcl of the workspace).
# CDX_DISPLAY=WxH runs with the drawing window (needs the stand-in X of the C side).
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../../.." && pwd)
cdx=${CDX_TCL:-$root/target/debug/cdx-tcl}
[ $# -gt 0 ] || set -- "$here"/t_*.tcl
status=0
for test in "$@"; do
  name=$(basename "$test")
  test=$(cd "$(dirname "$test")" && pwd)/$name
  out=$(mktemp)
  # A fresh home: xschem writes recent files and clipboard files into ~/.xschem.
  home=$(mktemp -d)
  # ... and some commands write into the current folder.
  cd "$home" || exit 1
  HOME=$home timeout 120 "$cdx" ${CDX_DISPLAY:+--display $CDX_DISPLAY} \
    -e "set ::faketk_dir {$here/..}" "$here/boot.tcl" "$here/lib.tcl" "$test" >"$out" 2>&1
  code=$?
  cd "$here" || exit 1
  rm -rf "$home"
  grep -E '^(PASS|FAIL)' "$out"
  if [ $code -ne 0 ] || grep -q '^FAIL' "$out" || grep -q '^error:' "$out"; then
    status=1
    echo "== $name: FAILED (exit $code)"
    grep -vE '^(PASS|ui: )' "$out" | cut -c1-300 | tail -15
  else
    echo "== $name: ok ($(grep -c '^PASS' "$out") checks, $(grep -c '^ui: ' "$out") updates)"
  fi
  if [ -n "$UICHECK" ]; then "$UICHECK" <"$out" || status=1; fi
  [ -n "$KEEP" ] && cp "$out" "$KEEP/$name.out"
  rm -f "$out"
done
exit $status
