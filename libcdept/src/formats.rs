//! Per-project file formats: `[suffixes]` in `cdept.toml`.
//!
//! CDePT tells the format of a file by its suffix (`.sch`, `.gds`, `.lib`, …; see
//! [`Format::split_name`]). A project can say otherwise:
//!
//! ```toml
//! [suffixes]
//! ".lib" = "spice"      # model libraries here, not Liberty
//! ".net" = "spice"      # a suffix CDePT does not know
//! ".v"   = "none"       # not a design file in this project
//! ```
//!
//! The names are those of [`NAMES`]. The longest suffix wins, as with the built-in
//! ones; at equal length the project's wins.
//!
//! How it reaches the scan without every function carrying a table: while a project
//! file is loaded its table is the *loading* table of the thread ([`Loading`]), and
//! [`Library::scan`] *enters* its library ([`enter`]), which remembers the table for
//! that folder. A later scan of the library, on any thread, finds it there.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use toml_edit::DocumentMut;

use crate::library::LIBRARY_FILE;
use crate::{Format, Library, LibraryLayout};

/// The table in the project file.
pub const SUFFIXES_KEY: &str = "suffixes";

/// The name of every format in `[suffixes]`. `none` (not in this list) means "not a
/// design file".
pub const NAMES: &[(&str, Format)] = &[
    ("xschem-schematic", Format::XschemSchematic),
    ("xschem-symbol", Format::XschemSymbol),
    ("qucs-schematic", Format::QucsSchematic),
    ("gds", Format::Gds),
    ("oasis", Format::Oasis),
    ("magic", Format::Magic),
    ("spice", Format::Spice),
    ("cdl", Format::Cdl),
    ("verilog", Format::Verilog),
    ("systemverilog", Format::SystemVerilog),
    ("vhdl", Format::Vhdl),
    ("verilog-a", Format::VerilogA),
    ("lef", Format::Lef),
    ("liberty", Format::Liberty),
    ("klayout-pcell", Format::KlayoutPcell),
];

/// The name of a format in `[suffixes]`.
pub fn format_name(format: Format) -> Option<&'static str> {
    NAMES.iter().find(|(_, f)| *f == format).map(|(n, _)| *n)
}

/// The format a name of `[suffixes]` means. `Some(None)` is `none`.
pub fn format_from_name(name: &str) -> Option<Option<Format>> {
    if name == "none" {
        return Some(None);
    }
    NAMES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, f)| Some(*f))
}

/// A project's own suffixes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Suffixes {
    /// Lower case, with the dot → the format, `None` for "not a design file".
    entries: Vec<(String, Option<Format>)>,
}

impl Suffixes {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The entries as written: suffix and format name.
    pub fn entries(&self) -> Vec<(String, &'static str)> {
        self.entries
            .iter()
            .map(|(suffix, format)| {
                let name = format.and_then(format_name).unwrap_or("none");
                (suffix.clone(), name)
            })
            .collect()
    }

    /// `[suffixes]` of a project file. What cannot be understood is left out, with a
    /// warning.
    pub fn parse(doc: &DocumentMut, warnings: &mut Vec<String>) -> Suffixes {
        let mut out = Suffixes::default();
        let Some(item) = doc.get(SUFFIXES_KEY) else {
            return out;
        };
        let Some(table) = item.as_table_like() else {
            warnings.push(format!("[{SUFFIXES_KEY}] must be a table"));
            return out;
        };
        let known = || {
            let mut names: Vec<&str> = NAMES.iter().map(|(n, _)| *n).collect();
            names.push("none");
            names.join(", ")
        };
        for (suffix, value) in table.iter() {
            let lower = suffix.to_ascii_lowercase();
            if !lower.starts_with('.') || lower.len() < 2 || lower.contains('/') {
                warnings.push(format!(
                    "[{SUFFIXES_KEY}] '{suffix}': a suffix starts with a dot, e.g. \".cir\""
                ));
                continue;
            }
            match value.as_str().and_then(format_from_name) {
                Some(format) => out.entries.push((lower, format)),
                None => warnings.push(format!(
                    "[{SUFFIXES_KEY}] '{suffix}': the format must be one of {}",
                    known()
                )),
            }
        }
        out
    }

    /// `[suffixes]` of a project file on disk (for display).
    pub fn read(project_file: &Path) -> Suffixes {
        std::fs::read_to_string(project_file)
            .ok()
            .and_then(|text| text.parse::<DocumentMut>().ok())
            .map(|doc| Suffixes::parse(&doc, &mut Vec::new()))
            .unwrap_or_default()
    }

    /// The longest of the project's suffixes `lower` (a lower-case file name) ends
    /// with: its length and format.
    fn longest(&self, lower: &str) -> Option<(usize, Option<Format>)> {
        self.entries
            .iter()
            .filter(|(suffix, _)| lower.ends_with(suffix.as_str()) && lower.len() > suffix.len())
            .max_by_key(|(suffix, _)| suffix.len())
            .map(|(suffix, format)| (suffix.len(), *format))
    }
}

thread_local! {
    /// The table of the project file being loaded on this thread.
    static LOADING: RefCell<Option<Arc<Suffixes>>> = const { RefCell::new(None) };
    /// The table of the library being scanned on this thread.
    static CURRENT: RefCell<Option<Arc<Suffixes>>> = const { RefCell::new(None) };
}

/// Library folder → the table of the project that defines it. Only projects with a
/// `[suffixes]` table are in here.
static REGISTRY: Mutex<Vec<(PathBuf, Arc<Suffixes>)>> = Mutex::new(Vec::new());

/// While it lives, the suffixes of a project file apply to what this thread loads.
#[must_use = "the suffixes apply while this value lives"]
pub struct Loading {
    previous: Option<Arc<Suffixes>>,
}

impl Loading {
    /// Read `[suffixes]` from a project file being loaded.
    pub fn start(doc: &DocumentMut, warnings: &mut Vec<String>) -> Loading {
        let table = Arc::new(Suffixes::parse(doc, warnings));
        Loading {
            previous: LOADING.with(|l| l.replace(Some(table))),
        }
    }
}

impl Drop for Loading {
    fn drop(&mut self) {
        let previous = self.previous.take();
        LOADING.with(|l| *l.borrow_mut() = previous);
    }
}

/// While it lives, the suffixes of a library's project apply on this thread.
#[must_use = "the suffixes apply while this value lives"]
pub struct Scope {
    previous: Option<Arc<Suffixes>>,
}

impl Drop for Scope {
    fn drop(&mut self) {
        let previous = self.previous.take();
        CURRENT.with(|c| *c.borrow_mut() = previous);
    }
}

/// Enter the library in `folder`: its project's suffixes apply until the result is
/// dropped. During a project load this also remembers the table for the folder, so
/// later scans (other threads included) find it.
pub fn enter(folder: &Path) -> Scope {
    let loading = LOADING.with(|l| l.borrow().clone());
    let table = match loading {
        Some(table) => {
            let mut registry = REGISTRY.lock().unwrap_or_else(|e| e.into_inner());
            registry.retain(|(path, _)| path != folder);
            if !table.is_empty() {
                registry.push((folder.to_path_buf(), table.clone()));
            }
            Some(table)
        }
        None => REGISTRY
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|(path, _)| path == folder)
            .map(|(_, table)| table.clone()),
    };
    Scope {
        previous: CURRENT.with(|c| c.replace(table)),
    }
}

/// The project's say on a file name, for [`Format::split_name`]: `None` when the
/// built-in suffixes decide, `Some(result)` when the project's do. `builtin` is the
/// built-in table (suffix in lower case with the dot, format).
pub(crate) fn split_name<'a>(
    file_name: &'a str,
    builtin: &[(&str, Format)],
) -> Option<Option<(&'a str, Format)>> {
    let table = CURRENT
        .with(|c| c.borrow().clone())
        .or_else(|| LOADING.with(|l| l.borrow().clone()))?;
    if table.is_empty() {
        return None;
    }
    let lower = file_name.to_ascii_lowercase();
    let (len, format) = table.longest(&lower)?;
    let builtin_len = builtin
        .iter()
        .filter(|(suffix, _)| lower.ends_with(suffix) && lower.len() > suffix.len())
        .map(|(suffix, _)| suffix.len())
        .max()
        .unwrap_or(0);
    if builtin_len > len {
        return None;
    }
    Some(format.map(|format| (&file_name[..file_name.len() - len], format)))
}

/// Files of a library that CDePT lists nowhere: their suffix says nothing to it.
///
/// In a `cells` library these are the files directly in the library folder and in the
/// cell folders (a view folder may hold anything); in a `flat` library, the files of
/// the library folder. Hidden files, `lib.toml`, layout sidecars and folders are not
/// counted. Other layouts give nothing.
pub fn unrecognised(library: &Library) -> Vec<PathBuf> {
    let _scope = enter(&library.path);
    let mut out = Vec::new();
    let loose = |dir: &Path, out: &mut Vec<PathBuf>| {
        let Ok(items) = std::fs::read_dir(dir) else {
            return;
        };
        for item in items.flatten() {
            let path = item.path();
            let Ok(name) = item.file_name().into_string() else {
                // Not UTF-8: reported by the project's name check.
                continue;
            };
            if name.starts_with('.')
                || name == LIBRARY_FILE
                || crate::layout::meta::is_sidecar_name(&name)
                || !path.is_file()
                || Format::split_name(&name).is_some()
            {
                continue;
            }
            out.push(path);
        }
    };
    if library.layout == LibraryLayout::Cells {
        loose(&library.path, &mut out);
        for cell in &library.cells {
            if let Some(dir) = &cell.path {
                loose(dir, &mut out);
            }
        }
    } else if library.layout == LibraryLayout::Flat {
        loose(&library.path, &mut out);
    }
    out.sort();
    out
}

/// [`unrecognised`] for one cell of a `cells` library (the files directly in its
/// folder). A flat library cannot say which cell a file belongs to.
pub fn unrecognised_in(library: &Library, cell: &str) -> Vec<PathBuf> {
    let Some(dir) = library.cell(cell).and_then(|c| c.path.as_ref()) else {
        return Vec::new();
    };
    unrecognised(library)
        .into_iter()
        .filter(|file| file.parent() == Some(dir.as_path()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(text: &str) -> (Suffixes, Vec<String>) {
        let doc: DocumentMut = text.parse().unwrap();
        let mut warnings = Vec::new();
        (Suffixes::parse(&doc, &mut warnings), warnings)
    }

    #[test]
    fn the_table_is_read_and_checked() {
        let (suffixes, warnings) = table(
            "[suffixes]\n\".LIB\" = \"spice\"\n\".net\" = \"spice\"\n\".v\" = \"none\"\n\
             \"cir\" = \"spice\"\n\".x\" = \"nonsense\"\n",
        );
        assert_eq!(
            suffixes.entries(),
            [
                (".lib".to_string(), "spice"),
                (".net".to_string(), "spice"),
                (".v".to_string(), "none"),
            ]
        );
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert!(warnings[0].contains("starts with a dot"));
        assert!(warnings[1].contains("xschem-schematic"));
        let (none, warnings) = table("name = \"x\"\n");
        assert!(none.is_empty() && warnings.is_empty());
        assert!(!table("suffixes = 3\n").1.is_empty());
    }

    #[test]
    fn every_format_has_a_name() {
        for (name, format) in NAMES {
            assert_eq!(format_name(*format), Some(*name));
            assert_eq!(format_from_name(name), Some(Some(*format)));
        }
        assert_eq!(format_from_name("none"), Some(None));
        assert_eq!(format_from_name("word"), None);
    }

    #[test]
    fn a_project_table_changes_what_a_suffix_means_while_it_applies() {
        let doc: DocumentMut = "[suffixes]\n\".lib\" = \"spice\"\n\".net\" = \"spice\"\n\
                                \".v\" = \"none\"\n\".tb.sp\" = \"none\"\n"
            .parse()
            .unwrap();
        assert_eq!(
            Format::split_name("a.lib"),
            Some(("a", Format::Liberty)),
            "nothing applies yet"
        );
        {
            let _loading = Loading::start(&doc, &mut Vec::new());
            assert_eq!(Format::split_name("a.lib"), Some(("a", Format::Spice)));
            assert_eq!(Format::split_name("a.net"), Some(("a", Format::Spice)));
            assert_eq!(Format::split_name("a.v"), None);
            // The longest suffix wins, whoever has it.
            assert_eq!(Format::split_name("a.tb.sp"), None);
            assert_eq!(Format::split_name("a.sp"), Some(("a", Format::Spice)));
            assert_eq!(Format::split_name("a.gds.gz"), Some(("a", Format::Gds)));
            assert_eq!(Format::split_name(".net"), None);

            // A library entered during the load keeps the table for later scans.
            let folder = Path::new("/nowhere/formats-test/lib");
            drop(enter(folder));
            let other = std::thread::spawn(move || {
                let before = Format::split_name("a.net");
                let _scope = enter(folder);
                (before, Format::split_name("a.net"))
            })
            .join()
            .unwrap();
            assert_eq!(other, (None, Some(("a", Format::Spice))));
        }
        assert_eq!(Format::split_name("a.net"), None, "the load is over");
        // A project without a table forgets the folder.
        let empty: DocumentMut = "".parse().unwrap();
        let _loading = Loading::start(&empty, &mut Vec::new());
        drop(enter(Path::new("/nowhere/formats-test/lib")));
        drop(_loading);
        let _scope = enter(Path::new("/nowhere/formats-test/lib"));
        assert_eq!(Format::split_name("a.net"), None);
    }
}
