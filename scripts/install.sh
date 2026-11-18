#!/bin/sh
# Build CDePT and its xschem helper in release mode and install them.
#
#   scripts/install.sh [--prefix P] [--destdir D] [--no-build] [--no-helper]
#
# Layout (under D/P):
#   bin/cdept                  the program
#   bin/cdept-xschem           the GPL helper (cdept looks next to its own executable)
#   share/cdept-xschem/        what the helper reads at run time: xschem's Tcl and awk
#                              files, systemlib, xschem_library/devices, cdept/ (Tcl)
#   share/doc/cdept/           licences
#
# --prefix   where it will run from (default /usr/local); the files do not depend on it
# --destdir  staging root for packagers: files go to D/P, nothing is recorded with D
# --no-build install what `cargo build --release` already made
# --no-helper install cdept only (no Tcl/cairo/flex/bison needed)
set -eu

prefix=/usr/local
destdir=
build=1
helper=1
while [ $# -gt 0 ]; do
    case $1 in
        --prefix) prefix=$2; shift 2 ;;
        --prefix=*) prefix=${1#*=}; shift ;;
        --destdir) destdir=$2; shift 2 ;;
        --destdir=*) destdir=${1#*=}; shift ;;
        --no-build) build=0; shift ;;
        --no-helper) helper=0; shift ;;
        -h|--help) sed -n '2,16p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "install.sh: unknown argument '$1'" >&2; exit 2 ;;
    esac
done

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
target=${CARGO_TARGET_DIR:-$root/target}/release
dest=$destdir$prefix

if [ $build = 1 ]; then
    cargo build --release -p cdept
    [ $helper = 0 ] || cargo build --release -p cdept-xschem
fi

install -d "$dest/bin"
install -m 755 "$target/cdept" "$dest/bin/cdept"
install -d "$dest/share/doc/cdept"
install -m 644 LICENSE "$dest/share/doc/cdept/LICENSE"

if [ $helper = 1 ]; then
    install -m 755 "$target/cdept-xschem" "$dest/bin/cdept-xschem"
    install -m 644 cdept-xschem/LICENSE "$dest/share/doc/cdept/LICENSE.cdept-xschem"
    # The build assembled the run-time files out of symbolic links into the source
    # tree; ask the helper where, then copy what they point to.
    share=$(env -u CDEPT_XSCHEM_SHAREDIR "$target/cdept-xschem" --print-sharedir)
    rm -rf "$dest/share/cdept-xschem"
    install -d "$dest/share/cdept-xschem"
    cp -rL "$share/." "$dest/share/cdept-xschem/"
    rm -rf "$dest/share/cdept-xschem/cdept/tests"
    [ -f "$dest/share/cdept-xschem/xschem.tcl" ] || { echo "install.sh: share copy failed" >&2; exit 1; }
fi

echo "installed to $dest"
