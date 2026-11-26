//! `cdept wave` on a file the viewer refuses ends with an error, not with a crash: the
//! built-in viewer's window, opened before the file failed, is closed (its thread joined)
//! before the process exits. Before the fix the exit raced the viewer's thread and the
//! process died with SIGSEGV. Without a display (or without the built-in UberWave) the
//! command fails earlier, also with an error code.

use std::process::Command;

#[test]
fn a_refused_file_is_an_error_not_a_crash() {
    let dir = tempfile::tempdir().unwrap();
    let raw = dir.path().join("lie.raw");
    // a header that lies about its counts (99999999 variables, none listed)
    let mut bytes = b"Title: t\nPlotname: Transient Analysis\nFlags: real\nNo. Variables: 99999999\nNo. Points: 18446744073709551615\nVariables:\nBinary:\n".to_vec();
    bytes.extend((0..4000u32).map(|i| (i * 37) as u8));
    std::fs::write(&raw, bytes).unwrap();
    let png = dir.path().join("x.png");
    for _ in 0..3 {
        let out = Command::new(env!("CARGO_BIN_EXE_cdept"))
            .arg("wave")
            .arg(&raw)
            .arg("--screenshot")
            .arg(&png)
            .env("CDEPT_UBERWAVE", "/nonexistent/UberWaveViewer")
            .output()
            .unwrap();
        assert_eq!(
            out.status.code(),
            Some(1),
            "{:?}\n{}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
    }
}
