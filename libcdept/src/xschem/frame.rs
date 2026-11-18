//! The pixels of xschem's drawing window, shared between the helper and CDePT.
//!
//! The picture panes of dialogs (xschem's previews) travel the same way, one file
//! each (see `protocol`).
//!
//! A frame is a file both programs map into memory: `height` rows of `stride` bytes,
//! four bytes per pixel in the order R G B A, alpha premultiplied. The helper writes
//! it ([`FrameWriter`]) and says which rectangle changed; CDePT copies that rectangle
//! out ([`FrameReader`]).
//!
//! Rules that keep this safe:
//! - A frame file never changes size. A new window size means a new file, announced
//!   with a new serial number. The old file may be deleted right away: a mapping stays
//!   valid after its file is deleted.
//! - Nothing waits for the other side. A reader may see a rectangle while it is being
//!   written; the next damage message makes it copy again.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

use memmap2::{Mmap, MmapMut};
use serde::{Deserialize, Serialize};

pub const BYTES_PER_PIXEL: u32 = 4;

/// Largest window side accepted, in pixels. Guards against a nonsense size turning
/// into a huge file.
pub const MAX_SIDE: u32 = 16384;

/// A rectangle in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// The smallest rectangle holding both.
    pub fn union(self, other: Rect) -> Rect {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return self;
        }
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let right = (self.x + self.width).max(other.x + other.width);
        let bottom = (self.y + self.height).max(other.y + other.height);
        Rect::new(x, y, right - x, bottom - y)
    }

    /// The part inside a `width` × `height` frame.
    pub fn clamp(self, width: u32, height: u32) -> Rect {
        let x = self.x.min(width);
        let y = self.y.min(height);
        Rect::new(x, y, self.width.min(width - x), self.height.min(height - y))
    }
}

fn check_size(width: u32, height: u32) -> io::Result<()> {
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("bad frame size {width}x{height}"),
        ));
    }
    Ok(())
}

/// The helper's side of a frame.
pub struct FrameWriter {
    map: MmapMut,
    path: PathBuf,
    width: u32,
    height: u32,
    stride: u32,
}

impl FrameWriter {
    /// Create the frame file `frame-<serial>` in `dir`, all transparent black.
    pub fn create(dir: &Path, serial: u64, width: u32, height: u32) -> io::Result<FrameWriter> {
        Self::create_named(dir, &format!("frame-{serial}"), width, height)
    }

    /// Create the frame file `name` in `dir` (a file that must not exist yet), all
    /// transparent black. Picture panes use this with names of their own.
    pub fn create_named(
        dir: &Path,
        name: &str,
        width: u32,
        height: u32,
    ) -> io::Result<FrameWriter> {
        check_size(width, height)?;
        let stride = width * BYTES_PER_PIXEL;
        let path = dir.join(name);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)?;
        file.set_len(u64::from(stride) * u64::from(height))?;
        // SAFETY: the file is ours, was just created with its final size and is never
        // resized (module rules).
        let map = unsafe { MmapMut::map_mut(&file)? };
        Ok(FrameWriter {
            map,
            path,
            width,
            height,
            stride,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn stride(&self) -> u32 {
        self.stride
    }

    /// All pixels, row after row.
    pub fn pixels_mut(&mut self) -> &mut [u8] {
        &mut self.map
    }

    /// Delete the file. The mapping (and any reader's) stays usable.
    pub fn unlink(&self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// CDePT's side of a frame.
pub struct FrameReader {
    map: Mmap,
    width: u32,
    height: u32,
    stride: u32,
}

impl FrameReader {
    /// Map the frame file announced by the helper.
    pub fn open(path: &Path, width: u32, height: u32, stride: u32) -> io::Result<FrameReader> {
        check_size(width, height)?;
        if stride < width * BYTES_PER_PIXEL {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("stride {stride} is too small for width {width}"),
            ));
        }
        let file = File::open(path)?;
        let needed = u64::from(stride) * u64::from(height);
        if file.metadata()?.len() < needed {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{} is shorter than its frame", path.display()),
            ));
        }
        // SAFETY: frame files are never resized (module rules), and the length was
        // checked. The helper writes while we read: we only ever copy bytes out, and
        // a half-written rectangle is copied again on the next damage message.
        let map = unsafe { Mmap::map(&file)? };
        Ok(FrameReader {
            map,
            width,
            height,
            stride,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Copy the pixels of `rect` (clamped to the frame) into `out`, tightly packed:
    /// `rect.width * 4` bytes per row. Returns the rectangle actually copied.
    pub fn copy_rect(&self, rect: Rect, out: &mut Vec<u8>) -> Rect {
        let rect = rect.clamp(self.width, self.height);
        out.clear();
        if rect.is_empty() {
            return rect;
        }
        let row_bytes = (rect.width * BYTES_PER_PIXEL) as usize;
        out.reserve(row_bytes * rect.height as usize);
        for row in rect.y..rect.y + rect.height {
            let start = (row * self.stride + rect.x * BYTES_PER_PIXEL) as usize;
            out.extend_from_slice(&self.map[start..start + row_bytes]);
        }
        rect
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rectangles() {
        let a = Rect::new(10, 10, 5, 5);
        let b = Rect::new(0, 12, 4, 10);
        assert_eq!(a.union(b), Rect::new(0, 10, 15, 12));
        assert_eq!(a.union(Rect::default()), a);
        assert_eq!(Rect::default().union(b), b);
        assert_eq!(
            Rect::new(90, 90, 50, 50).clamp(100, 100),
            Rect::new(90, 90, 10, 10)
        );
        assert!(Rect::new(200, 0, 5, 5).clamp(100, 100).is_empty());
    }

    #[test]
    fn a_reader_sees_what_the_writer_writes() {
        let dir = tempfile::tempdir().unwrap();
        let mut writer = FrameWriter::create(dir.path(), 1, 4, 3).unwrap();
        assert_eq!(writer.stride(), 16);
        let reader = FrameReader::open(writer.path(), 4, 3, writer.stride()).unwrap();

        // Pixel (2, 1) = opaque red, written after the reader mapped the file.
        let at = (16 + 2 * 4) as usize;
        writer.pixels_mut()[at..at + 4].copy_from_slice(&[255, 0, 0, 255]);
        let mut out = Vec::new();
        let copied = reader.copy_rect(Rect::new(2, 1, 10, 10), &mut out);
        assert_eq!(copied, Rect::new(2, 1, 2, 2));
        assert_eq!(out.len(), 2 * 2 * 4);
        assert_eq!(&out[..4], &[255, 0, 0, 255]);
        assert_eq!(&out[4..8], &[0, 0, 0, 0]);

        // Deleting the file leaves both mappings usable.
        writer.unlink();
        assert!(!writer.path().exists());
        writer.pixels_mut()[0] = 7;
        reader.copy_rect(Rect::new(0, 0, 1, 1), &mut out);
        assert_eq!(out[0], 7);
    }

    #[test]
    fn bad_sizes_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        assert!(FrameWriter::create(dir.path(), 1, 0, 10).is_err());
        assert!(FrameWriter::create(dir.path(), 2, MAX_SIDE + 1, 10).is_err());
        let writer = FrameWriter::create(dir.path(), 3, 4, 4).unwrap();
        assert!(FrameReader::open(writer.path(), 8, 8, 32).is_err());
        assert!(FrameReader::open(writer.path(), 4, 4, 8).is_err());
    }
}
