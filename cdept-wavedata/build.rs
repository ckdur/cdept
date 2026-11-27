//! Builds UberWave's data engine with its C interface (`uwdata`, in the `wp3-data-engine`
//! folder of the uberwave repository) into this crate, or falls back to a stub with the
//! same Rust API.
//!
//! Only the engine is built: CMake configures `wp3-data-engine` on its own with
//! `WP3_BUILD_TOOLS=OFF`, so it needs `cmake` and a C++20 compiler, nothing else (no
//! OpenGL, GTK, GLFW, ImGui, ImPlot, no network). A CI job that has those two builds the
//! real thing from the submodule.
//!
//! * The source: `$UBERWAVE_SRC` (the repository root), else the submodule
//!   `cdept-uberwave/uberwave`, else a sibling `../uberwave` of the CDePT repository; it
//!   must have `wp3-data-engine/capi/include/uwdata.h`.
//! * `$CDEPT_WAVEDATA_STUB=1` builds the stub on purpose (to check that it compiles).
//!
//! Any failure is a `cargo:warning` and the stub: the workspace still builds. On success
//! `--cfg wavedata_engine` is set.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

const HEADER: &str = "wp3-data-engine/capi/include/uwdata.h";

fn main() {
    println!("cargo::rustc-check-cfg=cfg(wavedata_engine)");
    for var in ["UBERWAVE_SRC", "CDEPT_WAVEDATA_STUB"] {
        println!("cargo:rerun-if-env-changed={var}");
    }
    println!("cargo:rerun-if-changed=build.rs");
    if env::var_os("CDEPT_WAVEDATA_STUB").is_some_and(|v| !v.is_empty() && v != "0") {
        println!(
            "cargo:warning=CDEPT_WAVEDATA_STUB is set: the waveform data engine is not built in"
        );
        return;
    }
    if cfg!(not(target_os = "linux")) {
        println!("cargo:warning=the waveform data engine is only built on Linux for now");
        return;
    }
    match build() {
        Ok(()) => println!("cargo:rustc-cfg=wavedata_engine"),
        Err(why) => println!(
            "cargo:warning=the waveform data engine is not built in ({why}); waveform tabs will say so. \
             It needs the uberwave source (the submodule cdept-uberwave/uberwave), cmake and a C++20 compiler"
        ),
    }
}

fn build() -> Result<(), String> {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let src = find_source(&manifest)?;
    let engine = src.join("wp3-data-engine");
    for dir in ["src", "include", "capi", "CMakeLists.txt"] {
        println!("cargo:rerun-if-changed={}", engine.join(dir).display());
    }
    let dir = out.join("uwdata-build");
    cmake_build(&engine, &dir)?;
    println!("cargo:rustc-link-search=native={}", dir.display());
    println!("cargo:rustc-link-lib=static=uwdata");
    println!("cargo:rustc-link-lib=static=wp3_core");
    println!("cargo:rustc-link-lib=dylib=stdc++");
    println!("cargo:rustc-link-lib=dylib=pthread");
    println!("cargo:rustc-link-lib=dylib=m");
    Ok(())
}

fn find_source(manifest: &Path) -> Result<PathBuf, String> {
    if let Some(v) = env::var_os("UBERWAVE_SRC").filter(|v| !v.is_empty()) {
        let p = PathBuf::from(v);
        // The wp5-viewer folder is accepted too (cdept-uberwave takes it).
        let root = if p.join(HEADER).is_file() {
            p
        } else {
            p.parent().map(Path::to_path_buf).unwrap_or(p)
        };
        return if root.join(HEADER).is_file() {
            Ok(root)
        } else {
            Err(format!("UBERWAVE_SRC has no {HEADER}"))
        };
    }
    let mut candidates = vec![manifest.join("../cdept-uberwave/uberwave")];
    if let Some(repo) = manifest.parent()
        && let Some(parent) = repo.parent()
    {
        candidates.push(parent.join("uberwave"));
    }
    candidates
        .into_iter()
        .find(|c| c.join(HEADER).is_file())
        .ok_or_else(|| {
            "no UBERWAVE_SRC, no cdept-uberwave/uberwave submodule with the data interface, no sibling ../uberwave"
                .to_string()
        })
}

/// CMake refuses a build folder that was configured for another source folder.
fn configured_elsewhere(dir: &Path, src: &Path) -> bool {
    let Ok(cache) = std::fs::read_to_string(dir.join("CMakeCache.txt")) else {
        return false;
    };
    let Some(home) = cache
        .lines()
        .find_map(|line| line.strip_prefix("CMAKE_HOME_DIRECTORY:INTERNAL="))
    else {
        return false;
    };
    match (Path::new(home).canonicalize(), src.canonicalize()) {
        (Ok(a), Ok(b)) => a != b,
        _ => true,
    }
}

fn cmake_build(engine: &Path, dir: &Path) -> Result<(), String> {
    let have_cmake = Command::new("cmake")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !have_cmake {
        return Err("cmake not found".into());
    }
    if configured_elsewhere(dir, engine) {
        std::fs::remove_dir_all(dir)
            .map_err(|e| format!("cannot remove the old CMake folder {}: {e}", dir.display()))?;
    }
    run(
        Command::new("cmake")
            .arg("-S")
            .arg(engine)
            .arg("-B")
            .arg(dir)
            .arg("-DCMAKE_BUILD_TYPE=Release")
            .arg("-DCMAKE_POSITION_INDEPENDENT_CODE=ON")
            .arg("-DWP3_BUILD_TOOLS=OFF")
            .arg("-DWP3_QUIET=ON")
            // Archives in the build folder itself, whatever the generator.
            .arg(format!(
                "-DCMAKE_ARCHIVE_OUTPUT_DIRECTORY={}",
                dir.display()
            )),
        "cmake configure",
    )?;
    let jobs = env::var("NUM_JOBS").unwrap_or_else(|_| "4".into());
    run(
        Command::new("cmake")
            .arg("--build")
            .arg(dir)
            .arg("--target")
            .arg("uwdata")
            .arg("--parallel")
            .arg(jobs),
        "cmake build (a C++20 compiler is needed)",
    )
}

fn run(cmd: &mut Command, what: &str) -> Result<(), String> {
    let out = cmd
        .output()
        .map_err(|e| format!("{what}: cannot run: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    let mut text = String::from_utf8_lossy(&out.stderr).into_owned();
    if text.trim().is_empty() {
        text = String::from_utf8_lossy(&out.stdout).into_owned();
    }
    let tail: Vec<&str> = text.lines().rev().take(6).collect();
    let tail: Vec<&str> = tail.into_iter().rev().collect();
    Err(format!("{what} failed: {}", tail.join(" | ")))
}
