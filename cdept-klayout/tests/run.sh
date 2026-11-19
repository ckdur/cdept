#!/bin/sh
# Run the helper's tests. Uses ../.bin/klayout (built by ../build.sh), or
# $CDEPT_TEST_KLAYOUT, or — when neither exists — the asic-all image through docker
# (override the image with CDEPT_TEST_DOCKER=<image>).
#   tests/run.sh                 all tests
#   tests/run.sh Headless        one class (unittest names)
#   tests/run.sh test_verify.Markers   a class of another test file
set -e
here=$(cd "$(dirname "$0")" && pwd)
if [ -z "$CDEPT_TEST_KLAYOUT" ] && [ -z "$CDEPT_TEST_DOCKER" ] && [ ! -x "$here/../.bin/klayout" ]; then
    if command -v klayout >/dev/null 2>&1; then
        CDEPT_TEST_KLAYOUT=$(command -v klayout)
        export CDEPT_TEST_KLAYOUT
    else
        CDEPT_TEST_DOCKER=factory.symbioticeda.com/asic-all:dev
        export CDEPT_TEST_DOCKER
    fi
fi
cd "$here"
if [ $# -eq 0 ]; then set -- test_helper test_verify; else set -- $(for t in "$@"; do case $t in *.*) echo "$t" ;; *) echo "test_helper.$t" ;; esac; done); fi
exec python3 -m unittest -v "$@" 2>&1
