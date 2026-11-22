//! Deleting to the trash instead of for good.
//!
//! The desktop's trash, by the freedesktop.org Trash specification: the file moves to
//! `$XDG_DATA_HOME/Trash/files` (or, on another disk, to the trash folder at the top of
//! that disk) and a `.trashinfo` file records where it was and when it went. The
//! desktop's file manager shows it there and can put it back; so can
//! [`Trashed::restore`].
//!
//! [`Library::remove_cell`] and [`Library::remove_view`] are the delete operations of
//! the library with the choice of [`Removal`].

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use eyre::{Result, WrapErr, bail, eyre};

use crate::Library;
use crate::layout::meta;

/// How something is deleted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Removal {
    /// Moved to the desktop's trash, from where it can be put back.
    #[default]
    Trash,
    /// Deleted from disk for good.
    Permanent,
}

/// Something that went to the trash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trashed {
    /// Where it was.
    pub original: PathBuf,
    /// Where it is now (`…/Trash/files/<name>`).
    pub file: PathBuf,
    /// Its record (`…/Trash/info/<name>.trashinfo`).
    pub info: PathBuf,
}

impl Trashed {
    /// Put it back where it was. Fails when something else is there now.
    pub fn restore(&self) -> Result<()> {
        if self.original.exists() {
            bail!("{} exists again", self.original.display());
        }
        if let Some(parent) = self.original.parent() {
            fs::create_dir_all(parent)
                .wrap_err_with(|| format!("cannot create {}", parent.display()))?;
        }
        move_path(&self.file, &self.original)?;
        let _ = fs::remove_file(&self.info);
        Ok(())
    }
}

/// A trash folder (with `files` and `info` inside).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trash {
    dir: PathBuf,
    /// Paths are recorded relative to this folder (a trash at the top of a disk);
    /// `None`: absolute.
    top: Option<PathBuf>,
}

impl Trash {
    /// The user's trash: `$XDG_DATA_HOME/Trash`, by default `~/.local/share/Trash`.
    pub fn home() -> Result<Trash> {
        let data = std::env::var_os("XDG_DATA_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .filter(|v| !v.is_empty())
                    .map(|home| PathBuf::from(home).join(".local/share"))
            })
            .ok_or_else(|| eyre!("no trash: neither XDG_DATA_HOME nor HOME is set"))?;
        if cfg!(not(unix)) {
            bail!("no trash on this system");
        }
        Ok(Trash::at(data.join("Trash")))
    }

    /// A trash folder at a given place.
    pub fn at(dir: impl Into<PathBuf>) -> Trash {
        Trash {
            dir: dir.into(),
            top: None,
        }
    }

    /// The trash for `path`: the user's when `path` is on the same disk, else the one
    /// at the top of the disk `path` is on (`.Trash/<uid>` or `.Trash-<uid>`), else
    /// the user's all the same (the file is then copied over).
    pub fn for_path(path: &Path) -> Result<Trash> {
        let home = Trash::home()?;
        Ok(disk_trash(path, &home.dir).unwrap_or(home))
    }

    /// The trash folder.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Move a file or a folder to this trash.
    pub fn put(&self, path: &Path) -> Result<Trashed> {
        let original = absolute(path)?;
        fs::symlink_metadata(&original)
            .wrap_err_with(|| format!("cannot delete {}", original.display()))?;
        let (files, info) = (self.dir.join("files"), self.dir.join("info"));
        for dir in [&files, &info] {
            fs::create_dir_all(dir).wrap_err_with(|| format!("cannot create {}", dir.display()))?;
        }
        let name = original
            .file_name()
            .ok_or_else(|| eyre!("cannot delete {}", original.display()))?
            .to_string_lossy()
            .into_owned();
        let recorded = match &self.top {
            Some(top) => original.strip_prefix(top).unwrap_or(&original),
            None => &original,
        };
        let record = format!(
            "[Trash Info]\nPath={}\nDeletionDate={}\n",
            encode(recorded),
            chrono::Local::now().format("%Y-%m-%dT%H:%M:%S")
        );
        // The info file claims the name: created only if it is not there yet.
        let mut n = 1;
        let (trash_name, info_file) = loop {
            let candidate = if n == 1 {
                name.clone()
            } else {
                format!("{name}.{n}")
            };
            let info_file = info.join(format!("{candidate}.trashinfo"));
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&info_file)
            {
                Ok(mut file) if !files.join(&candidate).exists() => {
                    file.write_all(record.as_bytes())
                        .wrap_err_with(|| format!("cannot write {}", info_file.display()))?;
                    break (candidate, info_file);
                }
                Ok(_) => {
                    let _ = fs::remove_file(&info_file);
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => {
                    return Err(e)
                        .wrap_err_with(|| format!("cannot write {}", info_file.display()));
                }
            }
            n += 1;
            if n > 10_000 {
                bail!("the trash is full of '{name}'");
            }
        };
        let file = files.join(&trash_name);
        if let Err(e) = move_path(&original, &file) {
            let _ = fs::remove_file(&info_file);
            return Err(e);
        }
        Ok(Trashed {
            original,
            file,
            info: info_file,
        })
    }
}

/// Move `path` to the trash that fits it (see [`Trash::for_path`]).
pub fn put(path: &Path) -> Result<Trashed> {
    Trash::for_path(path)?.put(path)
}

fn absolute(path: &Path) -> Result<PathBuf> {
    std::path::absolute(path).wrap_err_with(|| format!("cannot delete {}", path.display()))
}

/// Move a file or folder, copying when the two places are on different disks.
fn move_path(from: &Path, to: &Path) -> Result<()> {
    if fs::rename(from, to).is_ok() {
        return Ok(());
    }
    copy_all(from, to)
        .wrap_err_with(|| format!("cannot move {} to {}", from.display(), to.display()))?;
    let removed = if fs::symlink_metadata(from).is_ok_and(|m| m.is_dir()) {
        fs::remove_dir_all(from)
    } else {
        fs::remove_file(from)
    };
    removed.wrap_err_with(|| format!("copied to the trash, but cannot delete {}", from.display()))
}

fn copy_all(from: &Path, to: &Path) -> std::io::Result<()> {
    let meta = fs::symlink_metadata(from)?;
    if meta.is_dir() {
        fs::create_dir_all(to)?;
        for entry in fs::read_dir(from)? {
            let entry = entry?;
            copy_all(&entry.path(), &to.join(entry.file_name()))?;
        }
        Ok(())
    } else if meta.file_type().is_symlink() {
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(fs::read_link(from)?, to)
        }
        #[cfg(not(unix))]
        {
            fs::copy(from, to).map(|_| ())
        }
    } else {
        fs::copy(from, to).map(|_| ())
    }
}

/// `Path=` of a `.trashinfo` file: URL-escaped, `/` kept.
fn encode(path: &Path) -> String {
    let mut out = String::new();
    for byte in path.to_string_lossy().bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// The trash at the top of the disk `path` is on, when that is not the disk of the
/// user's trash and such a trash can be used.
#[cfg(unix)]
fn disk_trash(path: &Path, home_trash: &Path) -> Option<Trash> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let device = |p: &Path| fs::metadata(p).ok().map(|m| m.dev());
    let path = std::path::absolute(path).ok()?;
    let parent = path.parent()?;
    let here = device(parent)?;
    let home = home_trash.ancestors().find_map(device)?;
    if here == home {
        return None;
    }
    // The top of the disk: the highest folder still on it.
    let mut top = parent.to_path_buf();
    while let Some(up) = top.parent() {
        if device(up) != Some(here) {
            break;
        }
        top = up.to_path_buf();
    }
    let uid = fs::metadata("/proc/self")
        .or_else(|_| fs::metadata(std::env::var_os("HOME").unwrap_or_default()))
        .ok()?
        .uid();
    // `.Trash/<uid>`, when an administrator made `.Trash` (sticky, not a link).
    let shared = top.join(".Trash");
    if let Ok(meta) = fs::symlink_metadata(&shared)
        && meta.is_dir()
        && meta.permissions().mode() & 0o1000 != 0
    {
        let dir = shared.join(uid.to_string());
        if fs::create_dir_all(&dir).is_ok() {
            return Some(Trash {
                dir,
                top: Some(top),
            });
        }
    }
    let dir = top.join(format!(".Trash-{uid}"));
    if !dir.is_dir() {
        fs::create_dir(&dir).ok()?;
        let _ = fs::set_permissions(&dir, fs::Permissions::from_mode(0o700));
    }
    Some(Trash {
        dir,
        top: Some(top),
    })
}

#[cfg(not(unix))]
fn disk_trash(_path: &Path, _home_trash: &Path) -> Option<Trash> {
    None
}

impl Library {
    /// What deleting a cell takes away: its folder, or (in a flat library) the files
    /// of its views and their sidecars.
    pub fn cell_paths(&self, name: &str) -> Result<Vec<PathBuf>> {
        let cell = self
            .cell(name)
            .ok_or_else(|| eyre!("no cell '{name}' in library '{}'", self.name))?;
        Ok(match &cell.path {
            Some(dir) => vec![dir.clone()],
            None => cell
                .entries
                .iter()
                .flat_map(|e| entry_paths(&e.path))
                .collect(),
        })
    }

    /// What deleting a view takes away.
    pub fn view_paths(&self, cell: &str, view: &str) -> Result<Vec<PathBuf>> {
        let entry = self
            .cell(cell)
            .ok_or_else(|| eyre!("no cell '{cell}' in library '{}'", self.name))?
            .entry(view)
            .ok_or_else(|| eyre!("cell '{cell}' has no view '{view}'"))?;
        Ok(entry_paths(&entry.path))
    }

    /// Delete a cell and all its views: to the trash, or for good.
    pub fn remove_cell(&self, name: &str, how: Removal) -> Result<Vec<Trashed>> {
        self.writable()?;
        match how {
            Removal::Permanent => self.delete_cell(name).map(|()| Vec::new()),
            Removal::Trash => trash_all(&self.cell_paths(name)?),
        }
    }

    /// Delete one view of a cell: to the trash, or for good.
    pub fn remove_view(&self, cell: &str, view: &str, how: Removal) -> Result<Vec<Trashed>> {
        self.writable()?;
        match how {
            Removal::Permanent => self.delete_view(cell, view).map(|()| Vec::new()),
            Removal::Trash => trash_all(&self.view_paths(cell, view)?),
        }
    }
}

/// A view's folder or file, and the sidecar a loose layout file has.
fn entry_paths(path: &Path) -> Vec<PathBuf> {
    let mut paths = vec![path.to_path_buf()];
    let sidecar = meta::sidecar_path(path);
    if sidecar.is_file() {
        paths.push(sidecar);
    }
    paths
}

fn trash_all(paths: &[PathBuf]) -> Result<Vec<Trashed>> {
    let mut trashed = Vec::new();
    for path in paths {
        match put(path) {
            Ok(item) => trashed.push(item),
            Err(e) => {
                // All or nothing, as far as that can be had.
                for item in trashed.iter().rev() {
                    let _ = item.restore();
                }
                return Err(e);
            }
        }
    }
    Ok(trashed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_goes_to_the_trash_and_comes_back() {
        let tmp = tempfile::tempdir().unwrap();
        let trash = Trash::at(tmp.path().join("Trash"));
        let dir = tmp.path().join("lib/my cell");
        fs::create_dir_all(dir.join("schematic")).unwrap();
        fs::write(dir.join("schematic/a.sch"), "v {}\n").unwrap();

        let item = trash.put(&dir).unwrap();
        assert!(!dir.exists());
        assert_eq!(item.file, tmp.path().join("Trash/files/my cell"));
        assert!(item.file.join("schematic/a.sch").is_file());
        let info = fs::read_to_string(&item.info).unwrap();
        assert!(info.starts_with("[Trash Info]\nPath=/"), "{info}");
        assert!(info.contains("/lib/my%20cell\nDeletionDate=20"), "{info}");

        // The same name again gets another slot.
        fs::create_dir_all(&dir).unwrap();
        let second = trash.put(&dir).unwrap();
        assert_eq!(second.file, tmp.path().join("Trash/files/my cell.2"));
        assert!(second.info.ends_with("my cell.2.trashinfo"));

        item.restore().unwrap();
        assert!(dir.join("schematic/a.sch").is_file());
        assert!(!item.info.exists());
        // Not over something that is there.
        assert!(second.restore().is_err());
        assert!(trash.put(&tmp.path().join("nothing")).is_err());
    }

    #[test]
    fn paths_are_escaped_in_the_record() {
        assert_eq!(encode(Path::new("/a b/ä.sch")), "/a%20b/%C3%A4.sch");
    }
}
