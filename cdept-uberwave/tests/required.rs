//! CI's job `uberwave` sets `CDEPT_REQUIRE_ENGINES=1`: there the stub is a failure (a stub
//! build only prints a cargo:warning and the workspace still builds).

#[test]
fn uberwave_is_built_in_when_required() {
    if std::env::var_os("CDEPT_REQUIRE_ENGINES").is_some_and(|v| v == "1") {
        assert!(
            cdept_uberwave::available(),
            "CDEPT_REQUIRE_ENGINES=1 but UberWave is the stub (see the cargo:warning of cdept-uberwave's build)"
        );
    }
}
