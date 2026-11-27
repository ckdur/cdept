//! Making a tutorial's starting project: a project for the tutorial's PDK with the
//! library files the earlier tutorials would have left.

use std::path::{Path, PathBuf};

use libcdept::tech::{Catalog, PdkDir, Tech};
use libcdept::{AddLibrary, LibraryLayout, Project};

use super::tutorial::{Tutorial, start_layer};

/// The folder starting projects are made in: `$CDEPT_TUTORIAL_DIR`, else
/// `~/cdept-tutorial`.
pub(crate) fn base_dir() -> Option<PathBuf> {
    let set = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty());
    set("CDEPT_TUTORIAL_DIR")
        .map(PathBuf::from)
        .or_else(|| set("HOME").map(|home| PathBuf::from(home).join("cdept-tutorial")))
}

/// `base/<id>`, or `base/<id>-2`, …: the first that does not exist. Nothing of an
/// earlier try is written over.
fn free_folder(base: &Path, id: &str) -> PathBuf {
    (1..)
        .map(|n| match n {
            1 => base.join(id),
            n => base.join(format!("{id}-{n}")),
        })
        .find(|dir| !dir.exists())
        .expect("some number is free")
}

/// Lay the layers of the starting project over the project folder `root`.
pub(crate) fn lay(tutorial: &Tutorial, root: &Path) -> Result<(), String> {
    for layer in &tutorial.start {
        let mut any = false;
        for (path, bytes) in start_layer(layer) {
            any = true;
            let file = root.join(path);
            let parent = file.parent().expect("below the root");
            std::fs::create_dir_all(parent)
                .and_then(|()| std::fs::write(&file, bytes))
                .map_err(|e| format!("cannot write {}: {e}", file.display()))?;
        }
        if !any {
            return Err(format!("there is no starting layer '{layer}'"));
        }
    }
    Ok(())
}

/// Make the project for a scanned PDK; returns its folder.
pub(crate) fn make(
    tutorial: &Tutorial,
    pdk: &PdkDir,
    tech: &Tech,
    catalog: &Catalog,
    base: &Path,
) -> Result<PathBuf, String> {
    let error = |e: eyre::Report| format!("{e:#}");
    std::fs::create_dir_all(base).map_err(|e| format!("cannot make {}: {e}", base.display()))?;
    let dir = free_folder(base, &tutorial.id);
    let name = dir.file_name().map(|n| n.to_string_lossy().into_owned());
    let mut project =
        Project::create_for_pdk(&dir, name.as_deref(), pdk, tech, catalog).map_err(error)?;
    lay(tutorial, &dir)?;
    // A layer's `libs/<name>` is a library of the project.
    let libs = dir.join("libs");
    let mut names: Vec<String> = std::fs::read_dir(&libs)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.path().is_dir())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    for library in names {
        let options = AddLibrary {
            layout: Some(LibraryLayout::Cells),
            read_only: false,
            create: true,
        };
        project
            .add_library(&library, &format!("libs/{library}"), options)
            .map_err(error)?;
    }
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_free_folder_is_taken_and_the_layers_are_laid() {
        let base = tempfile::tempdir().unwrap();
        assert_eq!(free_folder(base.path(), "t"), base.path().join("t"));
        std::fs::create_dir(base.path().join("t")).unwrap();
        assert_eq!(free_folder(base.path(), "t"), base.path().join("t-2"));
        // Every layer a built-in tutorial names can be laid.
        for tutorial in super::super::builtins() {
            let dir = tempfile::tempdir().unwrap();
            lay(&tutorial, dir.path()).unwrap();
        }
    }
}
