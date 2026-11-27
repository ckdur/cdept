#!/bin/sh
# Build CDePT and its xschem helper in release mode and install them, with the
# KLayout helper script.
#
#   scripts/install.sh [--prefix P] [--destdir D] [--no-build] [--no-helper]
#
# Layout (under D/P):
#   bin/cdept                  the program
#   bin/cdept-xschem           the GPL helper (cdept looks next to its own executable)
#   share/cdept-xschem/        what the helper reads at run time: xschem's Tcl and awk
#                              files, systemlib, xschem_library/devices, cdept/ (Tcl)
#   bin/cdept-magic, share/cdept-magic/   the Magic helper (GPL) and its Tcl files
#   share/cdept-klayout/       the KLayout helper (Python, GPL), run by `klayout -zz`;
#                              KLayout itself is not installed: cdept uses $CDEPT_KLAYOUT
#                              or `klayout` next to cdept or on PATH
#   share/cdept/pdk/           device maps of PDKs (schematic symbol -> layout PCell,
#                              docs/LINKING.md)
#   share/doc/cdept/           licences; uberwave/ when UberWave is built in (its licences
#                              and the notices of what it contains)
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
        -h|--help) sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
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
    [ $helper = 0 ] || cargo build --release -p cdept-magic
fi

install -d "$dest/bin"
install -m 755 "$target/cdept" "$dest/bin/cdept"
install -d "$dest/share/doc/cdept"
install -m 644 LICENSE "$dest/share/doc/cdept/LICENSE"

# When cdept has UberWave built in (its viewer or its data engine), its licences and the
# notices of what it contains go with the program. The source is looked for as the
# build scripts do.
rm -rf "$dest/share/doc/cdept/uberwave"
if "$target/cdept" tools | grep -Eq '^(UberWave .*built in \(embedded\)|Waveform tabs .*data engine built in)'; then
    uberwave=
    for candidate in "${UBERWAVE_SRC:-}" "${UBERWAVE_SRC:+$UBERWAVE_SRC/..}" \
        "$root/cdept-uberwave/uberwave" "$root/../uberwave"; do
        if [ -n "$candidate" ] && [ -f "$candidate/LICENSE-MIT" ] && [ -d "$candidate/Docs/THIRD_PARTY_LICENSES" ]; then
            uberwave=$candidate
            break
        fi
    done
    [ -n "$uberwave" ] || {
        echo "install.sh: cdept has UberWave built in, but its source (for the licence files) was not found; set UBERWAVE_SRC" >&2
        exit 1
    }
    install -d "$dest/share/doc/cdept/uberwave/third-party"
    install -m 644 "$uberwave/LICENSE" "$uberwave/LICENSE-MIT" "$uberwave/LICENSE-GPL-2.0" "$dest/share/doc/cdept/uberwave/"
    install -m 644 "$uberwave"/Docs/THIRD_PARTY_LICENSES/* "$dest/share/doc/cdept/uberwave/third-party/"
fi

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
    # The Magic helper (Magic in a tab) and its Tcl: its own and the Tk stand-in.
    install -m 755 "$target/cdept-magic" "$dest/bin/cdept-magic"
    install -m 644 cdept-magic/LICENSE "$dest/share/doc/cdept/LICENSE.cdept-magic"
    rm -rf "$dest/share/cdept-magic"
    install -d "$dest/share/cdept-magic"
    install -m 644 cdept-magic/tcl/cdept-magic.tcl cdept-xschem/shim/tcl/faketk.tcl "$dest/share/cdept-magic/"
fi

if [ -f cdept-klayout/helper/cdept_klayout/main.py ]; then
    rm -rf "$dest/share/cdept-klayout"
    install -d "$dest/share/cdept-klayout"
    cp -r cdept-klayout/helper/. "$dest/share/cdept-klayout/"
    find "$dest/share/cdept-klayout" \( -name __pycache__ -o -name tests \) -prune -exec rm -rf {} +
    [ ! -f cdept-klayout/LICENSE ] ||
        install -m 644 cdept-klayout/LICENSE "$dest/share/doc/cdept/LICENSE.cdept-klayout"
fi

if [ -d pdk ]; then
    rm -rf "$dest/share/cdept/pdk"
    install -d "$dest/share/cdept"
    cp -r pdk "$dest/share/cdept/pdk"
fi

echo "installed to $dest"
