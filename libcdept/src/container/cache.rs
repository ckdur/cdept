//! The index cache: what [`super::read`] found in a file, kept so that a file of
//! tens of megabytes is read once.
//!
//! One JSON file per indexed file under `$XDG_CACHE_HOME/cdept/index/` (default
//! `~/.cache/cdept/index/`), named by a hash of the file's path. An entry is used when
//! the path, the size and the modification time it records are those of the file.
//! Entries are written to a temporary file and renamed into place, so several
//! processes can fill the cache at once; a damaged entry counts as missing.
//! `CDEPT_INDEX_CACHE=off` turns the cache off, any other value is the folder to use.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::UNIX_EPOCH;

use eyre::Result;
use serde::{Deserialize, Serialize};

use super::{Index, Reader};

pub const SCHEMA: &str = "cdept-index";
/// Raise when an [`Index`] changes shape: older entries are then read again.
pub const VERSION: u32 = 1;

/// Files smaller than this are read each time: reading the entry would cost as much.
const MIN_SIZE: u64 = 64 * 1024;

#[derive(Serialize, Deserialize)]
struct Entry {
    schema: String,
    version: u32,
    path: String,
    /// The reader the file was read with (`gds`, `lef`, `liberty`, …).
    reader: String,
    size: u64,
    /// Seconds and nanoseconds since the epoch.
    mtime: (u64, u32),
    index: Index,
}

/// Where indexes are kept. Cheap to clone; owns no open files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cache {
    dir: Option<PathBuf>,
    min_size: u64,
}

impl Cache {
    /// The user's cache: `$CDEPT_INDEX_CACHE`, else `$XDG_CACHE_HOME/cdept/index`, else
    /// `~/.cache/cdept/index`. Off when none of them can be worked out.
    pub fn user() -> Cache {
        let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty());
        let dir = match var("CDEPT_INDEX_CACHE") {
            Some(value) if value == "off" || value == "0" => None,
            Some(value) => Some(PathBuf::from(value)),
            None => var("XDG_CACHE_HOME")
                .map(PathBuf::from)
                .or_else(|| var("HOME").map(|home| PathBuf::from(home).join(".cache")))
                .map(|cache| cache.join("cdept").join("index")),
        };
        Cache {
            dir,
            min_size: MIN_SIZE,
        }
    }

    /// A cache in a folder of its own (tests, tools).
    pub fn at(dir: impl Into<PathBuf>) -> Cache {
        Cache {
            dir: Some(dir.into()),
            min_size: MIN_SIZE,
        }
    }

    /// No cache: every file is read each time.
    pub fn off() -> Cache {
        Cache {
            dir: None,
            min_size: MIN_SIZE,
        }
    }

    /// Also keep files of `bytes` and more (default 64 KiB).
    pub fn min_size(mut self, bytes: u64) -> Cache {
        self.min_size = bytes;
        self
    }

    pub fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    /// What `path` holds: from the cache when its entry is current, else read (and
    /// kept for next time). The reader goes by the file name ([`super::reader_of`]).
    pub fn get(&self, path: &Path) -> Result<Index> {
        match super::reader_of(path) {
            Some(reader) => self.get_as(path, reader),
            None => super::read(path),
        }
    }

    /// [`Cache::get`] with the reader given (a project may give a suffix another
    /// meaning than its usual one).
    pub fn get_as(&self, path: &Path, reader: Reader) -> Result<Index> {
        let Some((entry_file, key)) = self.entry_of(path, reader) else {
            return super::read_as(path, reader);
        };
        if let Some(index) = load(&entry_file, &key) {
            return Ok(index);
        }
        let index = super::read_as(path, reader)?;
        let entry = Entry {
            schema: SCHEMA.into(),
            version: VERSION,
            path: key.path,
            reader: key.reader,
            size: key.size,
            mtime: key.mtime,
            index,
        };
        if let Err(e) = store(&entry_file, &entry) {
            tracing::debug!("index cache: cannot write {}: {e}", entry_file.display());
        }
        Ok(entry.index)
    }

    /// Whether `path` has a current entry.
    pub fn has(&self, path: &Path) -> bool {
        super::reader_of(path)
            .and_then(|reader| self.entry_of(path, reader))
            .is_some_and(|(file, key)| load(&file, &key).is_some())
    }

    /// The entry file for `path` and what makes it current. `None`: not cached (cache
    /// off, small file, file not readable).
    fn entry_of(&self, path: &Path, reader: Reader) -> Option<(PathBuf, Key)> {
        let dir = self.dir.as_ref()?;
        let meta = fs::metadata(path).ok()?;
        if meta.len() < self.min_size {
            return None;
        }
        let absolute = std::path::absolute(path).ok()?;
        let mtime = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
        let key = Key {
            path: absolute.to_string_lossy().into_owned(),
            reader: format!("{reader:?}").to_ascii_lowercase(),
            size: meta.len(),
            mtime: (mtime.as_secs(), mtime.subsec_nanos()),
        };
        let name = format!(
            "{:016x}.json",
            fnv1a(format!("{}\0{}", key.path, key.reader).as_bytes())
        );
        Some((dir.join(name), key))
    }
}

struct Key {
    path: String,
    reader: String,
    size: u64,
    mtime: (u64, u32),
}

/// FNV-1a, 64 bit: stable across runs and versions (unlike the standard hasher).
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, b| {
        (hash ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

fn load(file: &Path, key: &Key) -> Option<Index> {
    let text = fs::read(file).ok()?;
    let entry: Entry = serde_json::from_slice(&text).ok()?;
    (entry.schema == SCHEMA
        && entry.version == VERSION
        && entry.path == key.path
        && entry.reader == key.reader
        && entry.size == key.size
        && entry.mtime == key.mtime)
        .then_some(entry.index)
}

fn store(file: &Path, entry: &Entry) -> std::io::Result<()> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let dir = file.parent().expect("an entry is in the cache folder");
    fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(
        ".{}.{}.{}.tmp",
        file.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let text = serde_json::to_vec(entry).map_err(std::io::Error::other)?;
    fs::write(&tmp, text)?;
    fs::rename(&tmp, file).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn liberty(cells: &[&str]) -> String {
        let mut text = String::from("library (demo) {\n");
        for cell in cells {
            text.push_str(&format!("  cell ({cell}) {{ area : 1; }}\n"));
        }
        text.push_str("}\n");
        text
    }

    #[test]
    fn entries_follow_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::at(dir.path().join("cache")).min_size(0);
        let file = dir.path().join("cells.lib");
        fs::write(&file, liberty(&["a", "b"])).unwrap();
        assert!(!cache.has(&file));
        assert_eq!(cache.get(&file).unwrap().names(), ["a", "b"]);
        assert!(cache.has(&file));
        assert_eq!(fs::read_dir(dir.path().join("cache")).unwrap().count(), 1);
        // The entry answers even when the file cannot be parsed again: change the
        // file without changing size or time is not something we can see, so check
        // the other way round: a changed file is read again.
        fs::write(&file, liberty(&["a", "b", "c"])).unwrap();
        assert!(!cache.has(&file));
        assert_eq!(cache.get(&file).unwrap().names(), ["a", "b", "c"]);
        // Still one entry: the new one replaced the old.
        assert_eq!(fs::read_dir(dir.path().join("cache")).unwrap().count(), 1);
    }

    #[test]
    fn a_damaged_entry_counts_as_missing() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::at(dir.path().join("cache")).min_size(0);
        let file = dir.path().join("cells.lib");
        fs::write(&file, liberty(&["a"])).unwrap();
        cache.get(&file).unwrap();
        let entry = fs::read_dir(dir.path().join("cache"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        fs::write(&entry, "{ not json").unwrap();
        assert!(!cache.has(&file));
        assert_eq!(cache.get(&file).unwrap().names(), ["a"]);
        assert!(cache.has(&file));
    }

    #[test]
    fn small_files_and_no_cache() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("cells.lib");
        fs::write(&file, liberty(&["a"])).unwrap();
        let cache = Cache::at(dir.path().join("cache"));
        assert_eq!(cache.get(&file).unwrap().names(), ["a"]);
        assert!(!dir.path().join("cache").exists());
        assert_eq!(Cache::off().get(&file).unwrap().names(), ["a"]);
        assert!(Cache::off().dir().is_none());
    }

    #[test]
    fn many_writers_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::at(dir.path().join("cache")).min_size(0);
        let file = dir.path().join("cells.lib");
        fs::write(&file, liberty(&["a", "b"])).unwrap();
        let paths: Vec<(&Path, Reader)> =
            (0..16).map(|_| (file.as_path(), Reader::Liberty)).collect();
        for result in crate::container::read_many(&paths, &cache) {
            assert_eq!(result.unwrap().names(), ["a", "b"]);
        }
        assert!(cache.has(&file));
        let left: Vec<_> = fs::read_dir(dir.path().join("cache"))
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(left.len(), 1, "{left:?}");
    }
}
