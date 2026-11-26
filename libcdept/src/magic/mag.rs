//! Reading and rewriting Magic's `.mag` files (the parts CDePT needs).
//!
//! A `.mag` file is one cell: `magic`, `tech <name>`, `timestamp <n>`, layers as
//! `<< layer >>` sections of `rect` lines, labels, and the cells it places:
//!
//! ```text
//! use inv  inv_0 ../../../logic/inv/layout
//! timestamp 1795726927
//! transform 1 0 0 0 1 0
//! box 0 0 100 100
//! ```
//!
//! The third word of a `use` line is optional: the folder of the placed cell's file,
//! relative to the folder of this file, absolute, with `~`, or with `$PDKPATH` and
//! similar variables. Without it Magic searches its path (`addpath`).
//!
//! The reader is tolerant: unknown lines are skipped, a file cut short gives what was
//! read. Only the first [`MAX_USES`] `use` lines are kept, and files larger than
//! [`MAX_FILE`] are refused, so a broken or hostile file cannot exhaust memory.

use std::path::{Path, PathBuf};

use eyre::{Result, WrapErr, bail};

/// Largest `.mag` file read (bytes).
pub const MAX_FILE: u64 = 512 * 1024 * 1024;
/// Most `use` lines kept from one file.
pub const MAX_USES: usize = 200_000;
/// Most labels kept from one file.
pub const MAX_LABELS: usize = 200_000;

/// One placed cell.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Use {
    /// The placed cell's name (its file is `<name>.mag`).
    pub cell: String,
    /// The instance name (Magic's "use id"), e.g. `inv_0`, `XM1`.
    pub instance: String,
    /// The folder of the placed cell's file, as written (may be empty).
    pub path: String,
    /// `transform a b c d e f` (Magic internal units).
    pub transform: Option<[i64; 6]>,
    /// `box llx lly urx ury` of the placed cell, in its own coordinates (internal units).
    pub bbox: Option<[i64; 4]>,
    /// 1-based line number of the `use` line.
    pub line: usize,
}

/// A label (`rlabel`/`flabel`), with its port number when it is a port.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Label {
    pub layer: String,
    pub text: String,
    /// `[llx, lly, urx, ury]` in internal units.
    pub rect: [i64; 4],
    pub port: Option<u32>,
    /// `port <n> <dir>`: the class (`input`, `output`, `bidirectional`…), when given.
    pub direction: Option<String>,
}

/// What CDePT reads from a `.mag` file.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MagFile {
    pub tech: Option<String>,
    pub timestamp: Option<i64>,
    /// `magscale num den`: internal units per lambda, when present.
    pub magscale: Option<(i64, i64)>,
    pub uses: Vec<Use>,
    pub labels: Vec<Label>,
    /// `<< end >>` was seen: the file is complete.
    pub complete: bool,
    /// The first line was `magic`.
    pub has_header: bool,
    /// Names of the layers with geometry.
    pub layers: Vec<String>,
    /// Problems found while reading (for the user; the file is still used).
    pub warnings: Vec<String>,
}

impl MagFile {
    /// Cells placed, each once, in the order of first use.
    pub fn children(&self) -> Vec<&str> {
        let mut seen = std::collections::BTreeSet::new();
        self.uses
            .iter()
            .filter(|u| seen.insert(u.cell.as_str()))
            .map(|u| u.cell.as_str())
            .collect()
    }

    /// The ports (labels with a port number), by number.
    pub fn ports(&self) -> Vec<&Label> {
        let mut ports: Vec<&Label> = self.labels.iter().filter(|l| l.port.is_some()).collect();
        ports.sort_by_key(|l| l.port);
        ports
    }
}

/// Read a `.mag` file.
pub fn read(file: &Path) -> Result<MagFile> {
    let size = std::fs::metadata(file)
        .wrap_err_with(|| format!("cannot read {}", file.display()))?
        .len();
    if size > MAX_FILE {
        bail!(
            "{} is {size} bytes, more than the {MAX_FILE} CDePT reads",
            file.display()
        );
    }
    let bytes = std::fs::read(file).wrap_err_with(|| format!("cannot read {}", file.display()))?;
    Ok(parse(&String::from_utf8_lossy(&bytes)))
}

fn int(word: Option<&str>) -> Option<i64> {
    word?.parse().ok()
}

fn ints<const N: usize>(words: &[&str]) -> Option<[i64; N]> {
    if words.len() < N {
        return None;
    }
    let mut out = [0i64; N];
    for (slot, w) in out.iter_mut().zip(words) {
        *slot = w.parse().ok()?;
    }
    Some(out)
}

/// Parse the text of a `.mag` file.
pub fn parse(text: &str) -> MagFile {
    let mut mag = MagFile::default();
    let mut in_use = false;
    let mut dropped_uses = 0usize;
    for (n, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if n == 0 {
            mag.has_header = line == "magic";
            if !mag.has_header {
                mag.warnings
                    .push("the first line is not 'magic': not a Magic file?".into());
            }
            continue;
        }
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("<<") {
            let name = rest.trim_end_matches(">>").trim().to_string();
            in_use = false;
            if name == "end" {
                mag.complete = true;
                break;
            }
            if !matches!(name.as_str(), "labels" | "properties" | "checkpaint")
                && !mag.layers.contains(&name)
            {
                mag.layers.push(name.clone());
            }
            continue;
        }
        let words: Vec<&str> = line.split_whitespace().collect();
        match words[0] {
            "tech" if mag.tech.is_none() => mag.tech = words.get(1).map(|s| s.to_string()),
            "magscale" => {
                if let (Some(a), Some(b)) = (int(words.get(1).copied()), int(words.get(2).copied()))
                {
                    mag.magscale = Some((a, b));
                }
            }
            "timestamp" if in_use => {}
            "timestamp" => mag.timestamp = int(words.get(1).copied()),
            "use" => {
                in_use = true;
                if words.len() < 2 {
                    mag.warnings.push(format!("line {}: 'use' without a cell", n + 1));
                    in_use = false;
                    continue;
                }
                if mag.uses.len() >= MAX_USES {
                    dropped_uses += 1;
                    in_use = false;
                    continue;
                }
                mag.uses.push(Use {
                    cell: words[1].to_string(),
                    instance: words.get(2).map(|s| s.to_string()).unwrap_or_default(),
                    path: words.get(3..).map(|w| w.join(" ")).unwrap_or_default(),
                    line: n + 1,
                    ..Default::default()
                });
            }
            "transform" if in_use => {
                if let Some(u) = mag.uses.last_mut() {
                    u.transform = ints::<6>(&words[1..]);
                }
            }
            "box" if in_use => {
                if let Some(u) = mag.uses.last_mut() {
                    u.bbox = ints::<4>(&words[1..]);
                }
                in_use = false;
            }
            "array" if in_use => {}
            "rlabel" | "flabel" => {
                if mag.labels.len() >= MAX_LABELS {
                    continue;
                }
                // rlabel <layer> [s] llx lly urx ury pos text
                // flabel <layer> [s] llx lly urx ury pos font size rot ox oy text
                let rest = &words[1..];
                let Some(layer_name) = rest.first() else {
                    continue;
                };
                let mut i = 1;
                if rest.get(i) == Some(&"s") {
                    i += 1;
                }
                let Some(rect) = rest.get(i..).and_then(ints::<4>) else {
                    mag.warnings.push(format!("line {}: bad label", n + 1));
                    continue;
                };
                let skip = if words[0] == "rlabel" { 5 } else { 10 };
                let text = rest.get(i + skip..).map(|w| w.join(" ")).unwrap_or_default();
                mag.labels.push(Label {
                    layer: layer_name.to_string(),
                    text,
                    rect,
                    ..Default::default()
                });
            }
            "port" => {
                if let Some(label) = mag.labels.last_mut() {
                    label.port = words.get(1).and_then(|w| w.parse().ok());
                    label.direction = words.get(3).map(|s| s.to_string());
                }
            }
            _ => {}
        }
    }
    if dropped_uses > 0 {
        mag.warnings.push(format!(
            "{dropped_uses} more 'use' lines were not read (more than {MAX_USES})"
        ));
    }
    if !mag.complete && mag.has_header {
        mag.warnings
            .push("the file ends without '<< end >>': it may be cut short".into());
    }
    mag
}

/// How to change one `use` line: the cell name and the path written after the
/// instance name (`None`: keep it; `Some("")`: remove it).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct UseEdit {
    pub cell: Option<String>,
    pub path: Option<String>,
}

/// Rewrite the `use` lines of `text`: `edit(use)` says what to change. The rest of the
/// file is kept byte for byte. Returns `None` when nothing changed.
pub fn rewrite_uses(text: &str, mut edit: impl FnMut(&Use) -> Option<UseEdit>) -> Option<String> {
    let mut out = String::with_capacity(text.len() + 64);
    let mut changed = false;
    for (n, raw) in text.split_inclusive('\n').enumerate() {
        let body = raw.trim_end_matches(['\n', '\r']);
        let ending = &raw[body.len()..];
        let words: Vec<&str> = body.split_whitespace().collect();
        if words.first() == Some(&"use") && words.len() >= 2 {
            let current = Use {
                cell: words[1].to_string(),
                instance: words.get(2).map(|s| s.to_string()).unwrap_or_default(),
                path: words.get(3..).map(|w| w.join(" ")).unwrap_or_default(),
                line: n + 1,
                ..Default::default()
            };
            if let Some(e) = edit(&current) {
                let cell = e.cell.unwrap_or(current.cell.clone());
                let path = e.path.unwrap_or(current.path.clone());
                // Magic writes two spaces between the cell and the instance.
                let mut line = format!("use {cell}  {}", current.instance);
                if !path.is_empty() {
                    line.push(' ');
                    line.push_str(&path);
                }
                if line != body {
                    changed = true;
                }
                out.push_str(&line);
                out.push_str(ending);
                continue;
            }
        }
        out.push_str(raw);
    }
    changed.then_some(out)
}

/// Rewrite the `use` lines of `file` in place (atomically: temporary file + rename).
/// Returns whether the file changed.
pub fn rewrite_file(file: &Path, edit: impl FnMut(&Use) -> Option<UseEdit>) -> Result<bool> {
    let size = std::fs::metadata(file)
        .wrap_err_with(|| format!("cannot read {}", file.display()))?
        .len();
    if size > MAX_FILE {
        bail!("{} is too large to rewrite", file.display());
    }
    let text = std::fs::read_to_string(file)
        .wrap_err_with(|| format!("cannot read {} as text", file.display()))?;
    let Some(new) = rewrite_uses(&text, edit) else {
        return Ok(false);
    };
    write_atomic(file, new.as_bytes())?;
    Ok(true)
}

pub(crate) fn write_atomic(file: &Path, bytes: &[u8]) -> Result<()> {
    let dir = file.parent().unwrap_or(Path::new("."));
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = dir.join(format!(".{name}.cdept-tmp"));
    std::fs::write(&tmp, bytes).wrap_err_with(|| format!("cannot write {}", tmp.display()))?;
    if let Ok(meta) = std::fs::metadata(file) {
        let _ = std::fs::set_permissions(&tmp, meta.permissions());
    }
    std::fs::rename(&tmp, file).wrap_err_with(|| format!("cannot replace {}", file.display()))?;
    Ok(())
}

/// Where a `use` path points, as a folder: relative paths are taken from `parent_dir`
/// (as Magic does), `~` is the home folder, `$VAR`/`${VAR}` come from `vars` then
/// the environment. `None` for an empty path or an unknown variable.
pub fn use_dir(path: &str, parent_dir: &Path, vars: &[(String, String)]) -> Option<PathBuf> {
    if path.is_empty() {
        return None;
    }
    let mut expanded = String::new();
    let mut chars = path.chars().peekable();
    if path.starts_with('~') {
        let home = std::env::var_os("HOME")?;
        expanded.push_str(&home.to_string_lossy());
        chars.next();
    }
    while let Some(c) = chars.next() {
        if c != '$' {
            expanded.push(c);
            continue;
        }
        let braced = chars.peek() == Some(&'{');
        if braced {
            chars.next();
        }
        let mut name = String::new();
        while let Some(&c) = chars.peek() {
            if braced && c == '}' {
                chars.next();
                break;
            }
            if !braced && !(c.is_ascii_alphanumeric() || c == '_') {
                break;
            }
            name.push(c);
            chars.next();
        }
        let value = vars
            .iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v.clone())
            .or_else(|| std::env::var(&name).ok())?;
        expanded.push_str(&value);
    }
    let p = PathBuf::from(expanded);
    Some(if p.is_absolute() {
        p
    } else {
        parent_dir.join(p)
    })
}

/// `to` written relative to the folder `from` (both absolute), with `..` as needed.
pub fn relative_dir(from: &Path, to: &Path) -> PathBuf {
    let a: Vec<_> = from.components().collect();
    let b: Vec<_> = to.components().collect();
    let common = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let mut out = PathBuf::new();
    for _ in common..a.len() {
        out.push("..");
    }
    for c in &b[common..] {
        out.push(c);
    }
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    out
}

/// A new, empty cell for technology `tech`, as Magic writes it.
pub fn template(tech: &str) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("magic\ntech {tech}\ntimestamp {now}\n<< end >>\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOP: &str = "magic\ntech sky130A\nmagscale 1 2\ntimestamp 1795726933\n\
<< checkpaint >>\nrect -349 -654 1087 926\n<< metal2 >>\nrect 300 0 400 100\n\
use inv  inv_0 ../../../libA/inv/layout\ntimestamp 1795726927\ntransform 1 0 0 0 1 0\n\
box 0 0 100 100\n\
use sky130_fd_sc_hd__inv_1  sky130_fd_sc_hd__inv_1_0 $PDKPATH/libs.ref/sky130_fd_sc_hd/mag\n\
timestamp 1752481779\ntransform 1 0 300 0 1 0\nbox -19 -24 157 296\n\
<< labels >>\nrlabel metal1 s 0 0 10 10 0 A\nport 1 nsew signal input\n\
flabel metal2 s 1 2 3 4 0 FreeSans 400 0 0 0 Y\nport 2 nsew signal output\n<< end >>\n";

    #[test]
    fn a_file_is_read() {
        let m = parse(TOP);
        assert!(m.has_header && m.complete);
        assert_eq!(m.tech.as_deref(), Some("sky130A"));
        assert_eq!(m.timestamp, Some(1795726933));
        assert_eq!(m.magscale, Some((1, 2)));
        assert_eq!(m.uses.len(), 2);
        assert_eq!(m.uses[0].cell, "inv");
        assert_eq!(m.uses[0].instance, "inv_0");
        assert_eq!(m.uses[0].path, "../../../libA/inv/layout");
        assert_eq!(m.uses[0].transform, Some([1, 0, 0, 0, 1, 0]));
        assert_eq!(m.uses[1].bbox, Some([-19, -24, 157, 296]));
        assert_eq!(m.children(), ["inv", "sky130_fd_sc_hd__inv_1"]);
        let ports = m.ports();
        assert_eq!(ports.len(), 2);
        assert_eq!(ports[0].text, "A");
        assert_eq!(ports[0].direction.as_deref(), Some("signal"));
        assert_eq!(ports[1].text, "Y");
        assert_eq!(ports[1].rect, [1, 2, 3, 4]);
        assert!(m.layers.contains(&"metal2".to_string()));
        assert!(m.warnings.is_empty(), "{:?}", m.warnings);
    }

    #[test]
    fn broken_files_give_what_was_read() {
        let cut = &TOP[..TOP.find("transform 1 0 300").unwrap()];
        let m = parse(cut);
        assert!(!m.complete);
        assert_eq!(m.uses.len(), 2);
        assert_eq!(m.uses[1].transform, None);
        assert!(m.warnings.iter().any(|w| w.contains("cut short")));
        let junk = parse("garbage\nuse\nuse a\nbox x y\nrlabel\nflabel m1 s a b c d\n<< end");
        assert!(!junk.has_header);
        assert_eq!(junk.uses.len(), 1);
        assert_eq!(junk.uses[0].instance, "");
        assert!(parse("").uses.is_empty());
        assert!(parse("\u{0}\u{ffff}\n\n").warnings.len() == 1);
    }

    #[test]
    fn use_lines_are_rewritten_and_nothing_else() {
        let new = rewrite_uses(TOP, |u| {
            (u.cell == "inv").then(|| UseEdit {
                cell: Some("inverter".into()),
                path: Some("../../../libC/inverter/layout".into()),
            })
        })
        .unwrap();
        assert!(new.contains("use inverter  inv_0 ../../../libC/inverter/layout\n"));
        assert_eq!(new.len() as isize - TOP.len() as isize, 10);
        assert!(new.contains("$PDKPATH/libs.ref"));
        // Same values: no change reported.
        assert!(rewrite_uses(TOP, |_| Some(UseEdit::default())).is_none());
        // Removing a path.
        let bare = rewrite_uses(TOP, |u| {
            (u.cell == "inv").then(|| UseEdit {
                cell: None,
                path: Some(String::new()),
            })
        })
        .unwrap();
        assert!(bare.contains("use inv  inv_0\n"));
    }

    #[test]
    fn paths_resolve_like_magic() {
        let parent = Path::new("/p/libB/top/layout");
        assert_eq!(
            use_dir("../../../libA/inv/layout", parent, &[]).unwrap(),
            PathBuf::from("/p/libB/top/layout/../../../libA/inv/layout")
        );
        let vars = [("PDKPATH".to_string(), "/pdk/sky130A".to_string())];
        assert_eq!(
            use_dir("$PDKPATH/libs.ref/x/mag", parent, &vars).unwrap(),
            PathBuf::from("/pdk/sky130A/libs.ref/x/mag")
        );
        assert_eq!(
            use_dir("${PDKPATH}/a", parent, &vars).unwrap(),
            PathBuf::from("/pdk/sky130A/a")
        );
        assert!(use_dir("$CDEPT_SURELY_UNSET_VAR_X/a", parent, &[]).is_none());
        assert!(use_dir("", parent, &[]).is_none());
        assert_eq!(
            relative_dir(Path::new("/p/libB/top/layout"), Path::new("/p/libA/inv/layout")),
            PathBuf::from("../../../libA/inv/layout")
        );
        assert_eq!(
            relative_dir(Path::new("/p/a"), Path::new("/p/a")),
            PathBuf::from(".")
        );
    }

    #[test]
    fn the_template_is_a_magic_file() {
        let m = parse(&template("sky130A"));
        assert!(m.has_header && m.complete);
        assert_eq!(m.tech.as_deref(), Some("sky130A"));
    }
}
