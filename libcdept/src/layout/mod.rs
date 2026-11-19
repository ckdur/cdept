//! Layout views and the references between them (docs/KLAYOUT.md §3).
//!
//! A layout file holds its own cell; cells placed from other views are *ghost cells*
//! (referenced, not defined). This module knows where those live:
//! - [`gds`]: a small GDSII reader and rewriter (structure names, references, renames);
//! - [`oasis`]: the same for OASIS;
//! - [`follow`]: making the views that place a cell follow when it is renamed;
//! - [`meta`]: the sidecar file `<layout file>.json` ([`Meta`]);
//! - [`refs`]: resolving ghost names through the project's libraries ([`RefTable`]).
//!
//! Nothing here depends on KLayout; the helper gets a finished [`RefTable`].

pub mod follow;
pub mod gds;
pub mod meta;
pub mod oasis;
pub mod refs;

pub use meta::{Meta, MetaRef};
pub use refs::{FileRefs, Ref, RefTable};

/// Write `bytes` to `path` through a temporary file in the same folder, then rename.
pub(crate) fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> eyre::Result<()> {
    use eyre::Context;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp);
    std::fs::write(&tmp, bytes).wrap_err_with(|| format!("cannot write {}", tmp.display()))?;
    std::fs::rename(&tmp, path).wrap_err_with(|| {
        let _ = std::fs::remove_file(&tmp);
        format!("cannot replace {}", path.display())
    })
}

/// The cells of a layout file (GDSII or OASIS, by its name) and what each places.
/// Compressed files (`.gds.gz`, `.oas.gz`) are not read.
pub fn read_summary(file: &std::path::Path) -> eyre::Result<gds::GdsSummary> {
    if is_oasis(file) {
        oasis::read_structure(file)
    } else {
        gds::read_structure(file)
    }
}

/// Rename cells (structures) in a layout file and the placements of them, GDSII or
/// OASIS. The file is not touched when nothing changes.
pub fn rename_in_file(
    file: &std::path::Path,
    map: &std::collections::BTreeMap<String, String>,
) -> eyre::Result<()> {
    if is_oasis(file) {
        oasis::rename_structures(file, map)
    } else {
        gds::rename_structures(file, map)
    }
}

/// True for a layout file this module can read and rewrite (`.gds`, `.oas`, `.oasis`).
pub fn is_readable(file: &std::path::Path) -> bool {
    file.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        ["gds", "oas", "oasis"]
            .iter()
            .any(|x| e.eq_ignore_ascii_case(x))
    })
}

fn is_oasis(file: &std::path::Path) -> bool {
    file.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("oas") || e.eq_ignore_ascii_case("oasis"))
}

/// Make a layout file belong to a cell after it was copied or renamed: the top
/// structure `old_top` becomes `new_top` inside a GDS file, and the sidecar (if any)
/// gets the new library, cell, view and top. References are kept. Compressed files
/// (`.gz`) only get their sidecar updated.
pub fn retarget(
    file: &std::path::Path,
    library: &str,
    cell: &str,
    view: &str,
    old_top: &str,
    new_top: &str,
) -> eyre::Result<()> {
    let mut meta = Meta::read_for(file)?;
    let old_top = meta
        .as_ref()
        .map_or(old_top, |m| m.top.as_str())
        .to_string();
    if is_readable(file) && old_top != new_top {
        let map = std::collections::BTreeMap::from([(old_top.clone(), new_top.to_string())]);
        rename_in_file(file, &map)?;
    }
    if let Some(meta) = &mut meta {
        meta.library = library.to_string();
        meta.cell = cell.to_string();
        meta.view = view.to_string();
        if meta.top == old_top {
            meta.top = new_top.to_string();
        }
        meta.write_for(file)?;
    }
    Ok(())
}
