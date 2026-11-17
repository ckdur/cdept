//! The `schematic=` attribute of xschem symbols.
//!
//! xschem finds the schematic behind a symbol by swapping `.sym` for `.sch` in the same
//! folder. In the `cells` layout the two files are in different view folders
//! (`cell/symbol/cell.sym`, `cell/schematic/cell.sch`), so the symbol has to say where
//! its schematic is, with a `schematic=` attribute among its global attributes:
//!
//! ```text
//! schematic=mylib/@symname/schematic/@symname.sch
//! ```
//!
//! The value is looked up like a symbol reference (see [`setup`](super::setup)), and
//! xschem replaces `@symname` with the name of the symbol file without its extension.
//! Without the attribute xschem writes an empty subcircuit and reports nothing.
//!
//! The value names the library folder. So it is written when a symbol view is created,
//! rewritten when a cell is copied or renamed, and [`check`] finds the symbols where it
//! is missing or wrong, e.g. after files were moved by hand.
//!
//! Only the attribute block of the file is rewritten; the rest stays byte for byte.

use std::fmt;
use std::fs;
use std::ops::Range;
use std::path::{Path, PathBuf};

use eyre::{Context, Result, eyre};

use super::setup;
use crate::{Cell, Format, Library, LibraryLayout, Project, Template};

/// Name of the attribute.
pub const ATTRIBUTE: &str = "schematic";

/// What xschem replaces with the symbol's file name, without folder and extension.
const SYMNAME: &str = "@symname";

// ---------------------------------------------------------------------------------
// The file text
// ---------------------------------------------------------------------------------

/// The text between the braces of the block holding the symbol's global attributes:
/// `K {...}`, or `G {...}` in files that have no (or an empty) `K` block.
fn attribute_block(text: &str) -> Option<Range<usize>> {
    let bytes = text.as_bytes();
    let mut k: Option<Range<usize>> = None;
    let mut g: Option<Range<usize>> = None;
    let mut embedded = 0usize; // inside `[ ... ]`: the copy of another symbol
    let mut line_start = true;
    let mut tag = 0u8;
    let mut first_group = false;
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'\n' {
            line_start = true;
        } else if line_start && !c.is_ascii_whitespace() {
            line_start = false;
            tag = c;
            first_group = true;
            match c {
                b'[' => embedded += 1,
                b']' => embedded = embedded.saturating_sub(1),
                _ => {}
            }
        }
        if c != b'{' {
            i += 1;
            continue;
        }
        // A group runs to the next brace that is not escaped.
        let start = i + 1;
        let mut end = start;
        while end < bytes.len() && bytes[end] != b'}' {
            end += if bytes[end] == b'\\' { 2 } else { 1 };
        }
        let end = end.min(bytes.len());
        if first_group && embedded == 0 {
            match tag {
                b'K' if k.is_none() => k = Some(start..end),
                b'G' if g.is_none() => g = Some(start..end),
                _ => {}
            }
        }
        first_group = false;
        i = end + 1;
    }
    let blank = |range: &Range<usize>| text[range.clone()].trim().is_empty();
    match (k, g) {
        (Some(k), Some(g)) if blank(&k) && !blank(&g) => Some(g),
        (Some(k), _) => Some(k),
        (None, g) => g,
    }
}

/// Undo the escaping of a `{...}` group: a backslash makes the next character literal.
fn unescape_group(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            out.extend(chars.next());
        } else {
            out.push(c);
        }
    }
    out
}

fn escape_group(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '\\' | '{' | '}') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// One `name=value` item of an attribute string. Ranges are byte ranges in that string.
struct Token {
    name: Range<usize>,
    /// The value as written, with its quotes.
    value: Option<Range<usize>>,
}

impl Token {
    fn span(&self) -> Range<usize> {
        let end = self.value.as_ref().map_or(self.name.end, |v| v.end);
        self.name.start..end
    }
}

/// End of the word starting at `i`: white space outside quotes ends it, and so does `=`
/// when `stop_at_equals` is set. A backslash makes the next character literal.
fn word_end(bytes: &[u8], mut i: usize, stop_at_equals: bool) -> usize {
    let mut quoted = false;
    let mut escaped = false;
    while i < bytes.len() {
        let c = bytes[i];
        if (stop_at_equals && c == b'=') || (c.is_ascii_whitespace() && !quoted && !escaped) {
            break;
        }
        if c == b'"' && !escaped {
            quoted = !quoted;
        }
        escaped = c == b'\\' && !escaped;
        i += 1;
    }
    i
}

fn skip_space(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

/// Split an attribute string into its items. White space is allowed around `=`.
fn tokens(attributes: &str) -> Vec<Token> {
    let bytes = attributes.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    loop {
        i = skip_space(bytes, i);
        if i >= bytes.len() {
            break;
        }
        if bytes[i] == b'=' {
            i += 1; // an `=` with no name in front
            continue;
        }
        let name = i..word_end(bytes, i, true);
        let after = skip_space(bytes, name.end);
        if after < bytes.len() && bytes[after] == b'=' {
            let start = skip_space(bytes, after + 1);
            let value = start..word_end(bytes, start, false);
            i = value.end;
            out.push(Token {
                name,
                value: Some(value),
            });
        } else {
            i = name.end;
            out.push(Token { name, value: None });
        }
    }
    out
}

/// A value without the quotes around it. Backslashes are kept, as xschem keeps them when
/// it reads this attribute.
fn unquote(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut escaped = false;
    for c in raw.chars() {
        if c != '"' || escaped {
            out.push(c);
        }
        escaped = c == '\\' && !escaped;
    }
    out
}

fn quote(value: &str) -> Result<String, String> {
    if value.contains(['"', '\\']) {
        return Err(format!(
            "cannot write {value:?} as an xschem attribute value"
        ));
    }
    if value.is_empty() || value.contains(char::is_whitespace) {
        Ok(format!("\"{value}\""))
    } else {
        Ok(value.to_string())
    }
}

fn find<'a>(attributes: &str, tokens: &'a [Token]) -> Option<&'a Token> {
    tokens
        .iter()
        .find(|t| &attributes[t.name.clone()] == ATTRIBUTE)
}

/// The `schematic=` attribute of the symbol in `text`, as written (not resolved).
pub fn schematic(text: &str) -> Option<String> {
    let attributes = unescape_group(&text[attribute_block(text)?]);
    let tokens = tokens(&attributes);
    let token = find(&attributes, &tokens)?;
    let value = unquote(&attributes[token.value.clone()?]);
    (!value.is_empty()).then_some(value)
}

/// `text` with the `schematic=` attribute set to `value`, or removed for `None`.
pub fn set_schematic(text: &str, value: Option<&str>) -> Result<String, String> {
    let Some(block) = attribute_block(text) else {
        return match value {
            Some(_) => Err("the symbol has no attribute block (K {...})".into()),
            None => Ok(text.to_string()),
        };
    };
    let mut attributes = unescape_group(&text[block.clone()]);
    let tokens = tokens(&attributes);
    let existing = find(&attributes, &tokens).map(Token::span);
    match (existing, value) {
        (Some(span), Some(value)) => {
            attributes.replace_range(span, &format!("{ATTRIBUTE}={}", quote(value)?));
        }
        (None, Some(value)) => {
            let ends_with_newline = attributes.ends_with('\n');
            attributes.truncate(attributes.trim_end().len());
            if !attributes.is_empty() {
                attributes.push('\n');
            }
            attributes.push_str(&format!("{ATTRIBUTE}={}", quote(value)?));
            if ends_with_newline {
                attributes.push('\n');
            }
        }
        (Some(span), None) => {
            let line_start = attributes[..span.start].rfind('\n').map_or(0, |n| n + 1);
            let line_end = attributes[span.end..]
                .find('\n')
                .map_or(attributes.len(), |n| span.end + n);
            let first_on_line = attributes[line_start..span.start].trim().is_empty();
            let last_on_line = attributes[span.end..line_end].trim().is_empty();
            let remove = if first_on_line && last_on_line {
                // The attribute has the line to itself: the line goes, with one line end.
                if line_end < attributes.len() {
                    line_start..line_end + 1
                } else {
                    line_start.saturating_sub(1)..line_end
                }
            } else if first_on_line {
                let after = &attributes[span.end..line_end];
                span.start..span.end + (after.len() - after.trim_start().len())
            } else {
                attributes[..span.start].trim_end().len()..span.end
            };
            attributes.replace_range(remove, "");
        }
        (None, None) => return Ok(text.to_string()),
    }
    let mut out = String::with_capacity(text.len() + 64);
    out.push_str(&text[..block.start]);
    out.push_str(&escape_group(&attributes));
    out.push_str(&text[block.end..]);
    Ok(out)
}

/// How xschem finds the schematic of a symbol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    /// No attribute: the `.sch` file next to the symbol.
    Default,
    /// A value only xschem can work out: Tcl (`tcleval(...)`), a generator script, a
    /// web address, or a path relative to the folder xschem runs in.
    Dynamic(String),
    /// A file, looked up on the library path unless absolute. `@symname` is replaced.
    Path(String),
}

/// The link of the symbol in `text`. `symbol_stem` is the symbol's file name without
/// `.sym`.
pub fn link(text: &str, symbol_stem: &str) -> Link {
    let Some(value) = schematic(text) else {
        return Link::Default;
    };
    let dynamic = value.starts_with("tcleval(")
        || value.contains('\\')
        || value.contains("://")
        || value.starts_with("./")
        || value.starts_with("../")
        || (value.contains('(') && value.trim_end().ends_with(')'));
    if dynamic {
        Link::Dynamic(value)
    } else {
        Link::Path(value.replace(SYMNAME, symbol_stem))
    }
}

/// The attribute value for the schematic at `inside` (e.g. `["schematic", "ota.sch"]`,
/// relative to the cell folder) of `cell` in the library folder named `library_folder`.
///
/// When the symbol file is named after the cell, the cell name is written as
/// `@symname`, so the value survives a rename of the cell.
pub fn reference<S: AsRef<str>>(
    library_folder: &str,
    cell: &str,
    inside: &[S],
    symbol_stem: &str,
) -> String {
    let generic = symbol_stem == cell;
    let mut parts = vec![library_folder.to_string()];
    parts.push(if generic { SYMNAME } else { cell }.to_string());
    for (n, part) in inside.iter().enumerate() {
        let part = part.as_ref();
        let last = n + 1 == inside.len();
        match part.strip_prefix(cell) {
            Some(rest) if generic && last && rest.starts_with('.') => {
                parts.push(format!("{SYMNAME}{rest}"));
            }
            _ => parts.push(part.to_string()),
        }
    }
    parts.join("/")
}

// ---------------------------------------------------------------------------------
// Libraries on disk
// ---------------------------------------------------------------------------------

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The name xschem knows a library by: the name of its folder.
fn folder_name(library: &Library) -> Option<String> {
    Some(library.path.file_name()?.to_string_lossy().into_owned())
}

/// `ota` for `…/ota.sym`.
fn symbol_stem(symbol: &Path) -> Option<String> {
    let name = symbol.file_name()?.to_str()?;
    match Format::split_name(name)? {
        (stem, Format::XschemSymbol) => Some(stem.to_string()),
        _ => None,
    }
}

/// The xschem symbol files of a cell.
fn symbols(cell: &Cell) -> Vec<PathBuf> {
    cell.entries
        .iter()
        .flat_map(|entry| &entry.files)
        .filter(|file| symbol_stem(file).is_some())
        .cloned()
        .collect()
}

/// The schematic the symbols of `cell` stand for, when it is in a view folder: the
/// view name and the file name. The view named `schematic` is preferred.
pub(crate) fn own_schematic(cell: &Cell) -> Option<(String, String)> {
    let mut views = cell
        .entries
        .iter()
        .filter(|e| e.format == Some(Format::XschemSchematic) && e.path.is_dir());
    let view = views
        .clone()
        .find(|e| e.name == Template::XschemSchematic.view_name())
        .or_else(|| views.next())?;
    Some((view.name.clone(), file_name(view.main_file.as_ref()?)))
}

/// The attribute for a new symbol view of `cell` in `library`. `cell` is the cell as
/// scanned, if it exists already. `None` where no attribute is needed (flat layout).
pub(crate) fn for_new_symbol(library: &Library, name: &str, cell: Option<&Cell>) -> Option<String> {
    if library.layout != LibraryLayout::Cells {
        return None;
    }
    let (view, file) = cell.and_then(own_schematic).unwrap_or_else(|| {
        (
            Template::XschemSchematic.view_name().to_string(),
            Template::XschemSchematic.file_name(name),
        )
    });
    Some(reference(&folder_name(library)?, name, &[view, file], name))
}

/// After cell `old` of `source` was copied or renamed to `cell` in `target`: make the
/// `schematic=` attribute of its symbols follow.
///
/// - An attribute pointing into the old cell points into the new one. In a flat
///   target it is removed: there the schematic is next to the symbol.
/// - A symbol without attribute that ends up in a view folder (a copy from a flat
///   library) gets one, when the cell has a schematic view.
/// - Any other value was set on purpose and is left alone.
pub(crate) fn follow_cell(
    target: &Library,
    cell: &Cell,
    source: &Library,
    old: &str,
) -> Result<()> {
    let (Some(target_folder), Some(source_folder)) = (folder_name(target), folder_name(source))
    else {
        return Ok(());
    };
    let own = own_schematic(cell);
    for symbol in symbols(cell) {
        let (Ok(text), Some(stem)) = (fs::read_to_string(&symbol), symbol_stem(&symbol)) else {
            continue;
        };
        // The file was renamed with the cell, so `@symname` meant the old name.
        let old_stem = if stem == cell.name { old } else { &stem };
        let value = match (link(&text, old_stem), target.layout) {
            (Link::Path(path), layout) => {
                let mut parts = path.split('/').filter(|p| !p.is_empty());
                if parts.next() != Some(source_folder.as_str()) || parts.next() != Some(old) {
                    continue;
                }
                let mut inside: Vec<String> = parts.map(str::to_string).collect();
                if inside.is_empty() {
                    continue;
                }
                match layout {
                    LibraryLayout::Flat => None,
                    LibraryLayout::Cells => {
                        if let Some(last) = inside.last_mut()
                            && let Some(rest) = last.strip_prefix(old)
                            && rest.starts_with('.')
                        {
                            *last = format!("{}{rest}", cell.name);
                        }
                        Some(reference(&target_folder, &cell.name, &inside, &stem))
                    }
                }
            }
            (Link::Default, LibraryLayout::Cells) => {
                let in_view_folder = symbol.parent() != cell.path.as_deref();
                let beside = symbol.with_extension("sch").exists();
                match &own {
                    Some((view, file)) if in_view_folder && !beside => {
                        Some(reference(&target_folder, &cell.name, &[view, file], &stem))
                    }
                    _ => continue,
                }
            }
            _ => continue,
        };
        if schematic(&text) == value {
            continue;
        }
        // A symbol file CDePT cannot edit is left as it is; `check` reports it.
        if let Ok(new_text) = set_schematic(&text, value.as_deref()) {
            fs::write(&symbol, new_text)
                .wrap_err_with(|| format!("cannot write {}", symbol.display()))?;
        }
    }
    Ok(())
}

/// What is wrong with the `schematic=` attribute of a symbol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// No attribute and no schematic next to the symbol: xschem netlists the cell empty.
    Missing,
    /// The attribute names a file that is not on the library path.
    Dangling(String),
    /// The attribute names the schematic of another cell, although this cell has one.
    /// This happens when a cell folder is copied by hand; it can also be on purpose.
    Foreign { value: String, file: PathBuf },
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => write!(
                f,
                "no {ATTRIBUTE}= attribute: xschem would netlist the cell empty"
            ),
            Self::Dangling(value) => write!(f, "{ATTRIBUTE}={value} is not found"),
            Self::Foreign { value, file } => write!(
                f,
                "{ATTRIBUTE}={value} is the schematic of another cell ({})",
                file.display()
            ),
        }
    }
}

/// A symbol whose schematic xschem would not find, or would find elsewhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unlinked {
    pub library: String,
    pub cell: String,
    pub symbol: PathBuf,
    pub problem: Problem,
    /// The attribute value that points at the cell's own schematic.
    pub expected: String,
}

/// Check the symbols of the project's `cells` libraries (all of them, or only `library`
/// and `cell`) against the schematic of their own cell. Cells without a schematic view
/// are not checked: there is nothing to descend into.
pub fn check(project: &Project, library: Option<&str>, cell: Option<&str>) -> Vec<Unlinked> {
    let search = setup::search_path(project);
    let mut out = Vec::new();
    let libraries = project.libraries.iter().filter(|l| {
        l.error.is_none()
            && l.layout == LibraryLayout::Cells
            && library.is_none_or(|name| l.name == name)
    });
    for library in libraries {
        let Some(folder) = folder_name(library) else {
            continue;
        };
        for c in &library.cells {
            if cell.is_some_and(|name| c.name != name) {
                continue;
            }
            let (Some(dir), Some((view, file))) = (&c.path, own_schematic(c)) else {
                continue;
            };
            for symbol in symbols(c) {
                let (Ok(text), Some(stem)) = (fs::read_to_string(&symbol), symbol_stem(&symbol))
                else {
                    continue;
                };
                let problem = match link(&text, &stem) {
                    Link::Dynamic(_) => continue,
                    Link::Default if symbol.with_extension("sch").exists() => continue,
                    Link::Default => Problem::Missing,
                    Link::Path(value) => match resolve(&value, &search) {
                        None => Problem::Dangling(value),
                        Some(found) if is_inside(&found, dir) => continue,
                        Some(found) => Problem::Foreign { value, file: found },
                    },
                };
                out.push(Unlinked {
                    library: library.name.clone(),
                    cell: c.name.clone(),
                    expected: reference(&folder, &c.name, &[&view, &file], &stem),
                    symbol,
                    problem,
                });
            }
        }
    }
    out
}

/// Write the expected attribute into the symbol.
pub fn repair(project: &Project, unlinked: &Unlinked) -> Result<()> {
    let library = project
        .library(&unlinked.library)
        .ok_or_else(|| eyre!("no library '{}'", unlinked.library))?;
    library.writable()?;
    let symbol = &unlinked.symbol;
    let text =
        fs::read_to_string(symbol).wrap_err_with(|| format!("cannot read {}", symbol.display()))?;
    let text = set_schematic(&text, Some(&unlinked.expected))
        .map_err(|e| eyre!("{}: {e}", symbol.display()))?;
    fs::write(symbol, text).wrap_err_with(|| format!("cannot write {}", symbol.display()))
}

/// The file xschem opens for `value`: as given when absolute, else the first match on
/// the library path.
fn resolve(value: &str, search: &[PathBuf]) -> Option<PathBuf> {
    let path = Path::new(value);
    if path.is_absolute() {
        return path.is_file().then(|| path.to_path_buf());
    }
    search
        .iter()
        .map(|dir| dir.join(path))
        .find(|file| file.is_file())
}

fn is_inside(file: &Path, dir: &Path) -> bool {
    match (file.canonicalize(), dir.canonicalize()) {
        (Ok(file), Ok(dir)) => file.starts_with(dir),
        _ => file.starts_with(dir),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SYMBOL: &str = "v {xschem version=3.4.5 file_version=1.2\n}\nG {}\nK {type=subcircuit\n\
        format=\"@name @pinlist @symname\"\ntemplate=\"name=x1\"\n}\nV {}\nS {}\nE {}\n\
        T {@symname} -10 -5 0 0 0.3 0.3 {}\n";

    #[test]
    fn reads_the_attribute_from_the_right_block() {
        assert_eq!(schematic(SYMBOL), None);
        let k = "G {}\nK {type=subcircuit\nschematic=lib/@symname/schematic/@symname.sch\n}\n";
        assert_eq!(
            schematic(k).as_deref(),
            Some("lib/@symname/schematic/@symname.sch")
        );
        // Files from before the K block keep symbol attributes in G.
        let g = "G {type=subcircuit schematic = \"my lib/a.sch\"}\nV {}\n";
        assert_eq!(schematic(g).as_deref(), Some("my lib/a.sch"));
        // K wins when both are filled.
        let both = "G {schematic=g.sch}\nK {schematic=k.sch}\n";
        assert_eq!(schematic(both).as_deref(), Some("k.sch"));
        // The word inside another value or in a text object is not the attribute.
        let inside = "K {template=\"name=x1 schematic=no.sch\"}\nT {schematic=no.sch} 0 0 {}\n";
        assert_eq!(schematic(inside), None);
        // Nor is the K block of a symbol embedded in the file.
        let embedded = "K {type=subcircuit}\nC {a.sym} 0 0 0 0 {}\n[\nK {schematic=no.sch}\n]\n";
        assert_eq!(schematic(embedded), None);
        assert_eq!(schematic("not an xschem file"), None);
    }

    #[test]
    fn sets_replaces_and_removes_the_attribute() {
        let set = set_schematic(SYMBOL, Some("lib/@symname/schematic/@symname.sch")).unwrap();
        assert!(
            set.contains(
                "template=\"name=x1\"\nschematic=lib/@symname/schematic/@symname.sch\n}\nV {}"
            ),
            "{set}"
        );
        // Everything outside the block is untouched.
        assert!(set.starts_with("v {xschem version=3.4.5 file_version=1.2\n}\nG {}\nK {type="));
        assert!(set.ends_with("T {@symname} -10 -5 0 0 0.3 0.3 {}\n"));

        let replaced = set_schematic(&set, Some("other lib/a.sch")).unwrap();
        assert_eq!(schematic(&replaced).as_deref(), Some("other lib/a.sch"));
        assert!(replaced.contains("schematic=\"other lib/a.sch\"\n}"));
        assert_eq!(replaced.matches("schematic=").count(), 1);

        // Removing gives the original file back.
        assert_eq!(set_schematic(&set, None).unwrap(), SYMBOL);
        assert_eq!(set_schematic(SYMBOL, None).unwrap(), SYMBOL);

        // An attribute in the middle of a line.
        let inline = "K {type=subcircuit schematic=a.sch format=\"@name\"}\n";
        assert_eq!(
            set_schematic(inline, None).unwrap(),
            "K {type=subcircuit format=\"@name\"}\n"
        );
        // An empty block.
        assert_eq!(
            set_schematic("G {}\nK {}\n", Some("a.sch")).unwrap(),
            "G {}\nK {schematic=a.sch}\n"
        );
        assert!(set_schematic("no block", Some("a.sch")).is_err());
        assert!(set_schematic(SYMBOL, Some("a\"b.sch")).is_err());
    }

    #[test]
    fn escaped_braces_survive_a_rewrite() {
        let text = "K {type=subcircuit\nformat=\"@name @pinlist @symname\"\n\
                    template=\"name=x1 m=\\{2*3\\} path=a\\\\\\\\b\"\n}\nV {}\n";
        let set = set_schematic(text, Some("lib/a.sch")).unwrap();
        assert!(
            set.contains("m=\\{2*3\\} path=a\\\\\\\\b\"\nschematic=lib/a.sch\n}"),
            "{set}"
        );
        assert_eq!(set_schematic(&set, None).unwrap(), text);
    }

    #[test]
    fn links_are_classified() {
        let with = |value: &str| set_schematic(SYMBOL, Some(value)).unwrap();
        assert_eq!(link(SYMBOL, "ota"), Link::Default);
        assert_eq!(
            link(&with("lib/@symname/schematic/@symname.sch"), "ota"),
            Link::Path("lib/ota/schematic/ota.sch".into())
        );
        assert!(matches!(
            link(&with("tcleval([pick])"), "ota"),
            Link::Dynamic(_)
        ));
        assert!(matches!(
            link(&with("gen.tcl(@n)"), "ota"),
            Link::Dynamic(_)
        ));
        assert!(matches!(
            link(&with("../other/a.sch"), "ota"),
            Link::Dynamic(_)
        ));
    }

    /// A project with the libraries `main` (cells), `other` (cells) and `flat`.
    fn project(dir: &Path) -> Project {
        let mut project = Project::create(dir, Some("demo")).unwrap();
        for (name, layout) in [
            ("main", LibraryLayout::Cells),
            ("other", LibraryLayout::Cells),
            ("flat", LibraryLayout::Flat),
        ] {
            let options = crate::AddLibrary {
                create: true,
                layout: Some(layout),
                ..Default::default()
            };
            project
                .add_library(name, &format!("libs/{name}"), options)
                .unwrap();
        }
        project
    }

    fn attribute(file: &Path) -> Option<String> {
        schematic(&fs::read_to_string(file).unwrap())
    }

    #[test]
    fn new_symbols_point_at_the_schematic_view_and_follow_the_cell() {
        let dir = tempfile::tempdir().unwrap();
        let mut project = project(dir.path());
        let libs = dir.path().join("libs");
        let generic = Some("main/@symname/schematic/@symname.sch".to_string());

        let main = project.library("main").unwrap();
        let symbol = main.create_view("ota", Template::XschemSymbol).unwrap();
        main.create_view("ota", Template::XschemSchematic).unwrap();
        assert_eq!(attribute(&symbol), generic);
        project.rescan();
        assert_eq!(check(&project, None, None), []);

        // A rename keeps the value: `@symname` follows the file name.
        let main = project.library("main").unwrap();
        main.rename_cell("ota", "ota2").unwrap();
        project.rescan();
        assert_eq!(attribute(&libs.join("main/ota2/symbol/ota2.sym")), generic);

        // A copy to another library names that library.
        let (main, other) = (
            project.library("main").unwrap(),
            project.library("other").unwrap(),
        );
        main.copy_cell("ota2", other, "amp").unwrap();
        assert_eq!(
            attribute(&libs.join("other/amp/symbol/amp.sym")).as_deref(),
            Some("other/@symname/schematic/@symname.sch")
        );

        // In a flat library the schematic is next to the symbol: no attribute.
        let flat = project.library("flat").unwrap();
        main.copy_cell("ota2", flat, "ota_flat").unwrap();
        assert!(libs.join("flat/ota_flat.sch").is_file());
        assert_eq!(attribute(&libs.join("flat/ota_flat.sym")), None);

        // And back from flat to cells it is needed again.
        project.rescan();
        let (main, flat) = (
            project.library("main").unwrap(),
            project.library("flat").unwrap(),
        );
        flat.copy_cell("ota_flat", main, "ota3").unwrap();
        assert_eq!(attribute(&libs.join("main/ota3/symbol/ota3.sym")), generic);

        project.rescan();
        assert_eq!(check(&project, None, None), []);
    }

    #[test]
    fn values_set_by_hand_are_kept_and_a_literal_cell_name_is_renamed() {
        let dir = tempfile::tempdir().unwrap();
        let mut project = project(dir.path());
        let libs = dir.path().join("libs");
        let main = project.library("main").unwrap();
        let symbol = main.create_view("ota", Template::XschemSymbol).unwrap();
        main.create_view("ota", Template::XschemSchematic).unwrap();
        // A second symbol of the same cell: its name is not the cell name, so the
        // attribute spells the cell out.
        let small = symbol.with_file_name("ota_small.sym");
        let text = fs::read_to_string(&symbol).unwrap();
        fs::write(
            &small,
            set_schematic(&text, Some("main/ota/schematic/ota.sch")).unwrap(),
        )
        .unwrap();
        fs::write(
            &symbol,
            set_schematic(&text, Some("tcleval([pick_one])")).unwrap(),
        )
        .unwrap();
        project.rescan();
        assert_eq!(check(&project, None, None), []);

        project
            .library("main")
            .unwrap()
            .rename_cell("ota", "amp")
            .unwrap();
        assert_eq!(
            attribute(&libs.join("main/amp/symbol/amp.sym")).as_deref(),
            Some("tcleval([pick_one])")
        );
        assert_eq!(
            attribute(&libs.join("main/amp/symbol/ota_small.sym")).as_deref(),
            Some("main/amp/schematic/amp.sch")
        );
        project.rescan();
        assert_eq!(check(&project, None, None), []);
    }

    #[test]
    fn check_finds_and_repairs_broken_links() {
        let dir = tempfile::tempdir().unwrap();
        let mut project = project(dir.path());
        let libs = dir.path().join("libs");
        let main = project.library("main").unwrap();
        for cell in ["good", "bare", "moved", "symbol_only"] {
            main.create_view(cell, Template::XschemSymbol).unwrap();
            if cell != "symbol_only" {
                main.create_view(cell, Template::XschemSchematic).unwrap();
            }
        }
        let symbol =
            |library: &str, cell: &str| libs.join(format!("{library}/{cell}/symbol/{cell}.sym"));
        let edit = |file: &Path, value: Option<&str>| {
            let text = fs::read_to_string(file).unwrap();
            fs::write(file, set_schematic(&text, value).unwrap()).unwrap();
        };
        // A symbol brought in from elsewhere, without the attribute.
        edit(&symbol("main", "bare"), None);
        // A library folder renamed by hand.
        edit(
            &symbol("main", "moved"),
            Some("old_name/@symname/schematic/@symname.sch"),
        );
        // A cell folder copied by hand into another library.
        fs::create_dir_all(libs.join("other/good/symbol")).unwrap();
        fs::create_dir_all(libs.join("other/good/schematic")).unwrap();
        fs::copy(symbol("main", "good"), symbol("other", "good")).unwrap();
        fs::copy(
            libs.join("main/good/schematic/good.sch"),
            libs.join("other/good/schematic/good.sch"),
        )
        .unwrap();
        project.rescan();

        let found = check(&project, None, None);
        let summary: Vec<(&str, &str, &Problem)> = found
            .iter()
            .map(|u| (u.library.as_str(), u.cell.as_str(), &u.problem))
            .collect();
        assert_eq!(
            summary,
            [
                ("main", "bare", &Problem::Missing),
                (
                    "main",
                    "moved",
                    &Problem::Dangling("old_name/moved/schematic/moved.sch".into())
                ),
                (
                    "other",
                    "good",
                    &Problem::Foreign {
                        value: "main/good/schematic/good.sch".into(),
                        file: libs.join("main/good/schematic/good.sch"),
                    }
                ),
            ]
        );
        assert_eq!(check(&project, Some("main"), Some("bare")).len(), 1);
        assert_eq!(check(&project, Some("other"), Some("bare")), []);

        for unlinked in &found {
            repair(&project, unlinked).unwrap();
        }
        assert_eq!(check(&project, None, None), []);
        assert_eq!(
            attribute(&symbol("other", "good")).as_deref(),
            Some("other/@symname/schematic/@symname.sch")
        );
    }

    #[test]
    fn references_use_symname_when_the_symbol_is_named_after_the_cell() {
        assert_eq!(
            reference("lib", "ota", &["schematic", "ota.sch"], "ota"),
            "lib/@symname/schematic/@symname.sch"
        );
        // A second symbol of the cell, with its own name.
        assert_eq!(
            reference("lib", "ota", &["schematic", "ota.sch"], "ota_small"),
            "lib/ota/schematic/ota.sch"
        );
        // A schematic file not named after the cell.
        assert_eq!(
            reference("lib", "ota", &["schematic_v2", "top.sch"], "ota"),
            "lib/@symname/schematic_v2/top.sch"
        );
    }
}
