//! Builds UberWave (`uberwave_embed`, Symbiotic EDA's waveform viewer) into this crate, or
//! falls back to a stub with the same Rust API.
//!
//! * The source is `$UBERWAVE_SRC` (the root of the uberwave repository, or its
//!   `wp5-viewer` folder), else the submodule `cdept-uberwave/uberwave` when it is checked
//!   out, else a sibling `../uberwave` of the CDePT repository; each needs
//!   `wp5-viewer/shim/uberwave_shim.h`.
//! * `$UBERWAVE_LIB_DIR` names an existing CMake build directory of that source (one that
//!   built the target `uberwave_embed_probe`): nothing is built, its libraries are linked.
//! * Otherwise cmake builds the library in `OUT_DIR`. CMake fetches GLFW, Dear ImGui,
//!   ImPlot, NFD-Extended and nlohmann/json with FetchContent (network, git), unless
//!   `$UBERWAVE_OFFLINE` is set: then nothing is fetched and `$UBERWAVE_DEPS_DIR` (a
//!   FetchContent base directory that has the sources) is used.
//! * Needs `cmake`, a C++20 compiler, OpenGL and GTK 3 development files
//!   (`libgl1-mesa-dev libgtk-3-dev`).
//!
//! Any failure is a `cargo:warning` and the stub: the workspace still builds (CI, machines
//! without the private source). On success `--cfg uberwave_embedded` is set.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

const HEADER: &str = "wp5-viewer/shim/uberwave_shim.h";
const PROBE: &str = "uberwave_embed_probe";

fn main() {
    println!("cargo::rustc-check-cfg=cfg(uberwave_embedded)");
    for var in [
        "UBERWAVE_SRC",
        "UBERWAVE_LIB_DIR",
        "UBERWAVE_OFFLINE",
        "UBERWAVE_DEPS_DIR",
    ] {
        println!("cargo:rerun-if-env-changed={var}");
    }
    println!("cargo:rerun-if-changed=build.rs");
    // Checking the submodule out later must rebuild.
    println!("cargo:rerun-if-changed=uberwave");
    if cfg!(not(target_os = "linux")) {
        println!(
            "cargo:warning=UberWave is not built in: only Linux is supported; cdept starts UberWaveViewer as a process"
        );
        return;
    }
    match build() {
        Ok(()) => println!("cargo:rustc-cfg=uberwave_embedded"),
        Err(why) => println!(
            "cargo:warning=UberWave is not built into this cdept ({why}); `cdept wave` falls back to the UberWaveViewer process. \
             Set UBERWAVE_SRC to the uberwave repository and rebuild (docs/howto/view-waveforms.md)"
        ),
    }
}

fn build() -> Result<(), String> {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let src = find_source(&manifest)?;
    println!(
        "cargo:rerun-if-changed={}",
        src.join("wp5-viewer").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        src.join("wp3-data-engine").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        src.join("CMakeLists.txt").display()
    );
    let build_dir = match env::var_os("UBERWAVE_LIB_DIR").filter(|v| !v.is_empty()) {
        Some(dir) => {
            let dir = PathBuf::from(dir);
            if !link_file(&dir).is_file() {
                return Err(format!(
                    "UBERWAVE_LIB_DIR {} has no built {PROBE} (build that target there first)",
                    dir.display()
                ));
            }
            dir
        }
        None => cmake_build(&src, &out.join("uberwave-build"))?,
    };
    emit_link(&build_dir)
}

/// CMake refuses a build folder that was configured for another source folder (the source
/// moves when `UBERWAVE_SRC` changes or the submodule is checked out).
fn cache_of_another_source(dir: &Path, src: &Path) -> bool {
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
        // The old source is gone.
        _ => true,
    }
}

/// The uberwave repository root.
fn find_source(manifest: &Path) -> Result<PathBuf, String> {
    let mut candidates = vec![];
    if let Some(v) = env::var_os("UBERWAVE_SRC").filter(|v| !v.is_empty()) {
        let p = PathBuf::from(v);
        // The wp5-viewer folder is accepted too.
        if p.join("shim/uberwave_shim.h").is_file()
            && let Some(root) = p.parent()
        {
            candidates.push(root.to_path_buf());
        }
        candidates.push(p);
        return candidates
            .into_iter()
            .find(|c| c.join(HEADER).is_file())
            .ok_or_else(|| format!("UBERWAVE_SRC has no {HEADER}"));
    }
    // The submodule: empty until `git submodule update --init`.
    candidates.push(manifest.join("uberwave"));
    // <repo>/cdept-uberwave -> <repo>/../uberwave
    if let Some(repo) = manifest.parent()
        && let Some(parent) = repo.parent()
    {
        candidates.push(parent.join("uberwave"));
    }
    candidates
        .into_iter()
        .find(|c| c.join(HEADER).is_file())
        .ok_or_else(|| {
            "no UBERWAVE_SRC, no cdept-uberwave/uberwave submodule and no sibling ../uberwave with the shim"
                .to_string()
        })
}

fn cmake_build(src: &Path, dir: &Path) -> Result<PathBuf, String> {
    let have = |tool: &str| {
        Command::new(tool)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    };
    if !have("cmake") {
        return Err("cmake not found".into());
    }
    if cache_of_another_source(dir, src) {
        std::fs::remove_dir_all(dir)
            .map_err(|e| format!("cannot remove the old CMake folder {}: {e}", dir.display()))?;
    }
    let mut configure = Command::new("cmake");
    configure
        .arg("-S")
        .arg(src)
        .arg("-B")
        .arg(dir)
        .arg("-DCMAKE_BUILD_TYPE=Release")
        .arg("-DCMAKE_POSITION_INDEPENDENT_CODE=ON")
        // The GPL-2.0 test engine stays out of a program that embeds the viewer.
        .arg("-DUBERWAVE_TEST_ENGINE=OFF")
        .arg("-DCMAKE_INSTALL_MESSAGE=NEVER");
    if env::var_os("UBERWAVE_OFFLINE").is_some_and(|v| !v.is_empty() && v != "0") {
        configure.arg("-DFETCHCONTENT_FULLY_DISCONNECTED=ON");
        if let Some(deps) = env::var_os("UBERWAVE_DEPS_DIR").filter(|v| !v.is_empty()) {
            let mut arg = std::ffi::OsString::from("-DFETCHCONTENT_BASE_DIR=");
            arg.push(deps);
            configure.arg(arg);
        }
    }
    run(
        &mut configure,
        "cmake configure (needs network for FetchContent, or UBERWAVE_OFFLINE)",
    )?;
    let jobs = env::var("NUM_JOBS").unwrap_or_else(|_| "4".into());
    run(
        Command::new("cmake")
            .arg("--build")
            .arg(dir)
            .arg("--target")
            .arg(PROBE)
            .arg("--parallel")
            .arg(jobs),
        "cmake build (OpenGL and GTK 3 development files are needed)",
    )?;
    Ok(dir.to_path_buf())
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
    Err(format!(
        "{what} failed: {}",
        tail.join(" | ").replace('\n', " ")
    ))
}

fn link_file(build_dir: &Path) -> PathBuf {
    build_dir.join(format!("wp5-viewer/CMakeFiles/{PROBE}.dir/link.txt"))
}

/// Link what the probe executable of the build links: the link line CMake wrote says
/// which libraries, in which order, with the system libraries.
fn emit_link(build_dir: &Path) -> Result<(), String> {
    let file = link_file(build_dir);
    let line = std::fs::read_to_string(&file)
        .map_err(|e| format!("cannot read {}: {e}", file.display()))?;
    let work = build_dir.join("wp5-viewer");
    let mut tokens = line.split_whitespace().peekable();
    let mut searched: Vec<PathBuf> = vec![];
    let mut libs: Vec<(bool, String)> = vec![]; // (static, name)
    let mut seen_output = false;
    let add_dir = |d: PathBuf, searched: &mut Vec<PathBuf>| {
        if !searched.contains(&d) {
            searched.push(d);
        }
    };
    while let Some(t) = tokens.next() {
        if t == "-o" {
            tokens.next();
            seen_output = true;
            continue;
        }
        if !seen_output {
            continue;
        }
        if let Some(name) = t.strip_prefix("-l") {
            if !matches!(name, "pthread" | "c") {
                libs.push((false, name.to_string()));
            }
        } else if let Some(dir) = t.strip_prefix("-L") {
            add_dir(PathBuf::from(dir), &mut searched);
        } else if t.starts_with('-') {
            // -Wl,... and other flags: not needed
        } else {
            let path = if Path::new(t).is_absolute() {
                PathBuf::from(t)
            } else {
                work.join(t)
            };
            let path = path.components().collect::<PathBuf>();
            let file = path
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_default();
            let Some(stem) = file.strip_prefix("lib") else {
                continue;
            };
            if let Some(name) = stem.strip_suffix(".a") {
                add_dir(path.parent().unwrap().to_path_buf(), &mut searched);
                libs.push((true, name.to_string()));
            } else if let Some(i) = stem.find(".so") {
                add_dir(path.parent().unwrap().to_path_buf(), &mut searched);
                libs.push((false, stem[..i].to_string()));
            }
        }
    }
    if !libs.iter().any(|(_, n)| n == "uberwave_embed") {
        return Err(format!("{} does not link uberwave_embed", file.display()));
    }
    for d in &searched {
        println!("cargo:rustc-link-search=native={}", d.display());
    }
    for (is_static, name) in &libs {
        let kind = if *is_static { "static" } else { "dylib" };
        println!("cargo:rustc-link-lib={kind}={name}");
    }
    println!("cargo:rustc-link-lib=dylib=stdc++");
    println!("cargo:rustc-link-lib=dylib=m");
    println!("cargo:rustc-link-lib=dylib=dl");
    Ok(())
}
