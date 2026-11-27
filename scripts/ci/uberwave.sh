#!/bin/sh
# The CI job `uberwave` (.github/workflows/ci.yml): build the REAL waveform data engine
# (cdept-wavedata, uwdata) and the REAL built-in UberWave (cdept-uberwave) from the
# submodule cdept-uberwave/uberwave, fail when either is the stub, and run the tests
# that work without a desktop. The ones that need a display run under xvfb-run when it
# is installed, else on $DISPLAY when that is set; else they are skipped and the job
# fails (CDEPT_CI_NEED_DISPLAY=0 allows that, for a run by hand).
#
#   git submodule update --init --depth 1 cdept-uberwave/uberwave
#   scripts/ci/uberwave.sh
#
# Needs cmake, a C++20 compiler, OpenGL and GTK 3 development files, git and the
# network (UberWave's CMake fetches GLFW, Dear ImGui, ImPlot, NFD and nlohmann/json),
# python3. CARGO_BUILD_JOBS and CI_JOBS limit the parallel jobs.
set -eu
cd "$(dirname "$0")/../.."
root=$PWD
jobs=${CI_JOBS:-$(nproc)}
target=${CARGO_TARGET_DIR:-$root/target}
fail() {
    echo "::error::$*" >&2
    exit 1
}
[ -f cdept-uberwave/uberwave/wp5-viewer/shim/uberwave_shim.h ] ||
    fail "the submodule cdept-uberwave/uberwave is not checked out"
[ -f cdept-uberwave/uberwave/wp3-data-engine/capi/include/uwdata.h ] ||
    fail "the submodule has no uwdata.h (too old a commit)"
# Nothing may point the builds elsewhere or ask for the stub.
unset UBERWAVE_SRC UBERWAVE_LIB_DIR UBERWAVE_OFFLINE UBERWAVE_DEPS_DIR CDEPT_WAVEDATA_STUB
export CDEPT_REQUIRE_ENGINES=1

echo "== build (the real engines)"
mkdir -p "$target"
log=$target/ci-uberwave-build.log
cargo build -p cdept >"$log" 2>&1 || {
    tail -50 "$log"
    fail "cargo build failed"
}
grep -E "^(warning|error)" "$log" || true
# A stub build only warns: look for the warnings of both build scripts.
if grep -E "warning: cdept-(wavedata|uberwave)@.*(not built in|CDEPT_WAVEDATA_STUB)" "$log"; then
    fail "a stub was built (the warning above)"
fi
tools=$("$target/debug/cdept" tools)
echo "$tools" | grep -E "^(UberWave|Waveform tabs) "
echo "$tools" | grep -Eq "^UberWave .*built in \(embedded\)" || fail "cdept tools: UberWave is not built in"
echo "$tools" | grep -Eq "^Waveform tabs .*data engine built in" || fail "cdept tools: the data engine is not built in"

echo "== uwdata's own tests (C interface)"
cmake -S cdept-uberwave/uberwave/wp3-data-engine -B "$target/ci-uwdata" -DCMAKE_BUILD_TYPE=Release >/dev/null
cmake --build "$target/ci-uwdata" --parallel "$jobs" \
    --target UwdataTest NumberParseTest HostileHeaderTest FallbackReaderTest >/dev/null
ctest --test-dir "$target/ci-uwdata" --output-on-failure \
    -R '^(Uwdata|NumberParse|HostileHeaders|FallbackReaderTest)$'

echo "== Rust tests without a display"
# The display tests of these crates skip themselves here; they run again below.
env -u DISPLAY -u WAYLAND_DISPLAY cargo test -p cdept-wavedata -p cdept-uberwave -- --test-threads="$jobs"
env -u DISPLAY -u WAYLAND_DISPLAY cargo test -p libcdept --lib wave -- --test-threads="$jobs"
env -u DISPLAY -u WAYLAND_DISPLAY cargo test -p libcdept --test wave -- --test-threads="$jobs"

echo "== tests with a display"
if command -v xvfb-run >/dev/null; then
    x() { xvfb-run -a -s "-screen 0 1920x1200x24" env -u WAYLAND_DISPLAY "$@"; }
elif [ -n "${DISPLAY:-}" ]; then
    x() { env -u WAYLAND_DISPLAY "$@"; }
elif [ "${CDEPT_CI_NEED_DISPLAY:-1}" = 0 ]; then
    echo "no xvfb-run and no DISPLAY: the display tests are skipped"
    exit 0
else
    fail "no xvfb-run and no DISPLAY for the display tests"
fi
# One viewer window per process: one test at a time.
x cargo test -p cdept-uberwave --test embedded -- --test-threads=1
x cargo test -p libcdept --test wave_embedded -- --test-threads=1
# The built-in viewer end to end: a hidden window saved as a PNG.
shots="$target/ci-uberwave"
rm -rf "$shots"
mkdir -p "$shots/home" "$shots/data"
python3 -I cdept-gui/scripts/wave/small.py "$shots"
x env HOME="$shots/home" XDG_DATA_HOME="$shots/data" \
    "$target/debug/cdept" wave "$shots/small.raw" --screenshot "$shots/uberwave.png"
[ -s "$shots/uberwave.png" ] || fail "cdept wave --screenshot wrote nothing"
# The waveform tabs in the real GUI (cdept-gui/scripts/wave/smoke.script.in).
sed "s|@DATA@|$shots|g" cdept-gui/scripts/wave/smoke.script.in >"$shots/smoke.script"
x env HOME="$shots/home" XDG_DATA_HOME="$shots/data" timeout 300 \
    "$target/debug/cdept" gui --script "$shots/smoke.script"
echo "== ok: the real data engine and the real built-in UberWave"
