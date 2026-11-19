#!/bin/sh
# Build KLayout from the submodule into .build/ and install it into .bin/
# (cdept-klayout/.bin/klayout). Takes 20–40 minutes; later runs are incremental.
#
#   ./build.sh                 Qt5 (default)
#   ./build.sh -j8             fewer parallel jobs (default: all cores)
#   QMAKE=qmake6 ./build.sh    Qt6: also needs qt6-5compat-dev and qt6-multimedia-dev
#   EXTRA="-without-qt-multimedia" ./build.sh   when QtMultimedia is not installed
#
# Needs: a C++ compiler, Qt development files (qtbase5-dev qttools5-dev
# libqt5svg5-dev libqt5xmlpatterns5-dev qtmultimedia5-dev), python3-dev, ruby-dev,
# zlib1g-dev, libgit2-dev. After cloning: git submodule update --init cdept-klayout/klayout
set -e
here=$(cd "$(dirname "$0")" && pwd)
jobs=-j$(nproc 2>/dev/null || echo 4)
[ -n "$1" ] && jobs=$1
if [ ! -x "$here/klayout/build.sh" ]; then
    echo "cdept-klayout/klayout is empty: git submodule update --init cdept-klayout/klayout" >&2
    exit 1
fi
cd "$here/klayout"
# shellcheck disable=SC2086
./build.sh -qmake "${QMAKE:-qmake}" -python python3 -ruby ruby $EXTRA \
    -build ../.build -bin ../.bin -option "$jobs"
"$here/.bin/klayout" -v
