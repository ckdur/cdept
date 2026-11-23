//! LEF abstract views in the embedded KLayout (docs/KLAYOUT.md §9).
//!
//! A view of kind `abstract` whose main file is a `.lef` opens as a layout the helper
//! builds with KLayout's LEF/DEF reader: the LEF stays the view's file, nothing is
//! converted or written next to it, and the tab is read-only. The reader needs the
//! technology LEF (layers, units); [`tech_lef`] finds it.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Project;

/// What the tab and `save` say when someone tries to change a LEF abstract.
pub const READ_ONLY_NOTICE: &str = "LEF abstract: read-only";

/// What the helper needs to read a LEF file ([`crate::layout::RefTable::lef`]).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LefInfo {
    /// The technology LEF read before the macros, when one was found.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tech_lef: Option<PathBuf>,
}

/// Whether `file` is a LEF (by its suffix).
pub fn is_lef(file: &Path) -> bool {
    file.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("lef"))
}

/// What the helper gets for the LEF `file` of `project`.
pub fn info(project: &Project, file: &Path) -> LefInfo {
    LefInfo {
        tech_lef: tech_lef(project, file),
    }
}

/// The technology LEF to read with `lef`, in this order:
///
/// 1. the technology snapshot (`tech.toml`): the `techlef` files of the reference
///    library the LEF belongs to;
/// 2. a technology LEF next to the file or in a `techlef/` or `lef/` folder beside it
///    ([`find_tech_lef`]).
///
/// `None`: the helper numbers the layers by the order the LEF names them.
pub fn tech_lef(project: &Project, lef: &Path) -> Option<PathBuf> {
    if let Some(found) = snapshot_tech_lef(project, lef) {
        return Some(found);
    }
    find_tech_lef(lef)
}

/// From the snapshot: the library whose folder holds `lef` names its technology LEFs.
fn snapshot_tech_lef(project: &Project, lef: &Path) -> Option<PathBuf> {
    let tech = project.snapshot()?;
    let mut best: Option<(usize, Vec<PathBuf>)> = None;
    for library in &tech.refs.libraries {
        let Some(folder) = project.tech_path(&library.path) else {
            continue;
        };
        if !lef.starts_with(&folder) {
            continue;
        }
        // The files of the LibreLane configuration first, then the folder scan's.
        let configured = library.files.as_ref().map(|f| f.tech_lef.as_slice());
        let files: Vec<PathBuf> = configured
            .unwrap_or_default()
            .iter()
            .chain(library.techlef.iter())
            .filter_map(|file| project.tech_path(file))
            .filter(|file| file.is_file())
            .collect();
        // The deepest folder is the library the file belongs to.
        let depth = folder.components().count();
        if !files.is_empty() && best.as_ref().is_none_or(|(d, _)| depth > *d) {
            best = Some((depth, files));
        }
    }
    pick(best?.1)
}

/// A technology LEF by looking around `lef`: the folder of the file, its `techlef/`
/// and `lef/` siblings (a PDK's `libs.ref/<lib>/{lef,techlef}`). A technology LEF is a
/// file whose name says `tech` or ends in `.tlef`; among several the nominal corner
/// (`nom`) wins, then the shortest name.
pub fn find_tech_lef(lef: &Path) -> Option<PathBuf> {
    let dir = lef.parent()?;
    let mut folders = vec![dir.to_path_buf()];
    if let Some(library) = dir.parent() {
        folders.push(library.join("techlef"));
        folders.push(library.join("lef"));
        folders.push(library.join("tech"));
    }
    folders.push(dir.join("techlef"));
    let mut found = Vec::new();
    for folder in folders {
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path == lef || !path.is_file() || !looks_like_tech_lef(&path) {
                continue;
            }
            found.push(path);
        }
        // The nearest folder with one is enough.
        if !found.is_empty() {
            break;
        }
    }
    pick(found)
}

fn looks_like_tech_lef(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".tlef") || (lower.ends_with(".lef") && lower.contains("tech"))
}

/// One of several technology LEFs: the nominal corner, else the shortest name, else
/// alphabetical (so the choice never changes between runs).
fn pick(mut files: Vec<PathBuf>) -> Option<PathBuf> {
    files.sort();
    files.dedup();
    files.into_iter().min_by_key(|file| {
        let name = file
            .file_name()
            .map(|n| n.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        (
            !name.contains("nom"),
            name.contains("rram") || name.contains("min") || name.contains("max"),
            name.len(),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn touch(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "VERSION 5.8 ;\n").unwrap();
    }

    #[test]
    fn lef_by_suffix() {
        assert!(is_lef(Path::new("/a/b/cells.lef")));
        assert!(is_lef(Path::new("CELLS.LEF")));
        assert!(!is_lef(Path::new("cells.gds")));
        assert!(!is_lef(Path::new("lef")));
    }

    #[test]
    fn tech_lef_next_to_the_file() {
        let tmp = tempfile::tempdir().unwrap();
        let lef = tmp.path().join("lib/lef/cells.lef");
        let tech = tmp.path().join("lib/lef/cells_tech.lef");
        touch(&lef);
        touch(&tech);
        assert_eq!(find_tech_lef(&lef), Some(tech));
    }

    #[test]
    fn tech_lef_in_a_techlef_folder_prefers_nominal() {
        let tmp = tempfile::tempdir().unwrap();
        let lef = tmp.path().join("lib/lef/hd.lef");
        touch(&lef);
        for name in [
            "hd__max.tlef",
            "hd__min.tlef",
            "hd__nom.tlef",
            "hd__rram__nom.tlef",
        ] {
            touch(&tmp.path().join("lib/techlef").join(name));
        }
        assert_eq!(
            find_tech_lef(&lef),
            Some(tmp.path().join("lib/techlef/hd__nom.tlef"))
        );
    }

    #[test]
    fn no_tech_lef_is_none_and_the_file_itself_is_not_one() {
        let tmp = tempfile::tempdir().unwrap();
        let lef = tmp.path().join("lib/lef/technology_cells.lef");
        touch(&lef);
        touch(&tmp.path().join("lib/lef/other.lef"));
        assert_eq!(find_tech_lef(&lef), None);
    }

    #[test]
    fn the_choice_is_stable() {
        let a = pick(vec!["/x/b_tech.lef".into(), "/x/a_tech.lef".into()]);
        let b = pick(vec!["/x/a_tech.lef".into(), "/x/b_tech.lef".into()]);
        assert_eq!(a, b);
        assert_eq!(a, Some(PathBuf::from("/x/a_tech.lef")));
    }

    #[test]
    fn the_info_travels_as_json_only_when_there_is_something() {
        assert_eq!(serde_json::to_string(&LefInfo::default()).unwrap(), "{}");
        let with = LefInfo {
            tech_lef: Some("/t.lef".into()),
        };
        assert_eq!(
            serde_json::to_string(&with).unwrap(),
            r#"{"tech_lef":"/t.lef"}"#
        );
    }
}
