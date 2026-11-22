//! Set-up files of layout tools found in a folder being imported: KLayout technology
//! (`.lyt`) and layer properties (`.lyp`) files, a Magic start-up file.

use std::path::{Path, PathBuf};

/// What was found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolFiles {
    /// KLayout technology files, with the technology name each gives.
    pub lyt: Vec<(PathBuf, Option<String>)>,
    /// KLayout layer properties files.
    pub lyp: Vec<PathBuf>,
    /// Magic start-up files (`.magicrc`, `<name>.magicrc`).
    pub magicrc: Vec<PathBuf>,
}

/// The `<name>` of a KLayout technology file: the first one in the text (it is a
/// direct child of `<technology>` and comes first).
pub fn technology_name(text: &str) -> Option<String> {
    let start = text.find("<name>")? + "<name>".len();
    let end = text[start..].find("</name>")? + start;
    let name = text[start..end].trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// Look in `folder` and, `depth` levels down, in its folders (hidden ones left out).
pub fn find(folder: &Path, depth: usize) -> ToolFiles {
    let mut out = ToolFiles::default();
    walk(folder, depth, &mut out);
    out.lyt.sort();
    out.lyp.sort();
    out.magicrc.sort();
    out
}

fn walk(dir: &Path, depth: usize, out: &mut ToolFiles) {
    let Ok(items) = std::fs::read_dir(dir) else {
        return;
    };
    for item in items.flatten() {
        let path = item.path();
        let name = item.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            if depth > 0 && !name.starts_with('.') && !super::is_generated_name(&name) {
                walk(&path, depth - 1, out);
            }
            continue;
        }
        if name.ends_with(".magicrc") {
            out.magicrc.push(path);
        } else if name.ends_with(".lyt") {
            let small = std::fs::metadata(&path).is_ok_and(|m| m.len() < (4 << 20));
            let technology = small
                .then(|| std::fs::read_to_string(&path).ok())
                .flatten()
                .and_then(|text| technology_name(&text));
            out.lyt.push((path, technology));
        } else if name.ends_with(".lyp") {
            out.lyp.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_name_of_a_technology_file() {
        let text = "<?xml version=\"1.0\"?>\n<technology>\n <name>sg13g2</name>\n \
                    <description>x</description>\n</technology>";
        assert_eq!(technology_name(text).as_deref(), Some("sg13g2"));
        assert_eq!(technology_name("<technology><name></name>"), None);
    }
}
