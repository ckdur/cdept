//! Puts the tutorials into the program: the files of `tutorials/`, the pictures of the
//! written pages (`docs/tutorials/pictures/`) and the starting projects
//! (`tutorials/start/<layer>/…`). Adding a file there is enough.

use std::fs;
use std::path::{Path, PathBuf};

fn files(dir: &Path, into: &mut Vec<PathBuf>) {
    println!("cargo:rerun-if-changed={}", dir.display());
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            files(&path, into);
        } else {
            into.push(path);
        }
    }
}

fn main() {
    let here = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets it"));
    let mut out = String::new();

    let mut all = Vec::new();
    files(&here.join("tutorials"), &mut all);
    let start = here.join("tutorials/start");
    out += "pub(crate) const TUTORIAL_FILES: &[(&str, &str)] = &[\n";
    for file in &all {
        if file.parent() == Some(&here.join("tutorials"))
            && file.extension().is_some_and(|e| e == "toml")
        {
            let id = file.file_stem().expect("a file").to_string_lossy();
            out += &format!(
                "    ({id:?}, include_str!({:?})),\n",
                file.display().to_string()
            );
        }
    }
    out += "];\n";

    out += "pub(crate) const START_FILES: &[(&str, &str, &[u8])] = &[\n";
    for file in &all {
        let Ok(below) = file.strip_prefix(&start) else {
            continue;
        };
        let mut parts = below.components();
        let layer = parts.next().expect("a layer").as_os_str().to_string_lossy();
        let path = parts.as_path().to_string_lossy();
        if !path.is_empty() {
            out += &format!(
                "    ({layer:?}, {path:?}, include_bytes!({:?})),\n",
                file.display().to_string()
            );
        }
    }
    out += "];\n";

    let mut pictures = Vec::new();
    files(&here.join("../docs/tutorials/pictures"), &mut pictures);
    out += "pub(crate) const PICTURES: &[(&str, &[u8])] = &[\n";
    for file in pictures {
        if file.extension().is_some_and(|e| e == "png") {
            let name = file.file_stem().expect("a file").to_string_lossy();
            out += &format!(
                "    ({name:?}, include_bytes!({:?})),\n",
                file.display().to_string()
            );
        }
    }
    out += "];\n";

    let target = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets it"));
    fs::write(target.join("tutorials.rs"), out).expect("OUT_DIR can be written");
}
