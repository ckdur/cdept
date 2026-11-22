//! Search across the libraries of a project.
//!
//! A [`Query`] is a few words and filters, written the way one types them in the
//! quick-open box or after `cdept find`:
//!
//! ```text
//! ota                      cells whose name has "ota" (best matches first; a few
//!                          letters in order also match: "o5" finds "ota5t")
//! nand*_1                  a pattern with * ? [a-z] matches the whole name
//! analog/ota               library "analog…", cell "ota…"
//! lib:analog kind:layout   filters: lib: kind: view: format: cat:
//! cat:amplifiers           cells in a category (or below it)
//! in:content vdd           also look inside schematics, symbols and netlists
//! ```
//!
//! Without a view filter the hits are cells; with `kind:`, `view:` or `format:` they
//! are the views that fit. Looking inside files is bounded (file size, number of
//! files, time), and says so when it stopped early.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::category::{self, Categories};
use crate::{Cell, Entry, Format, Library, Project, ViewKind};

/// What to look for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    /// Words that must all match the cell name (or, with [`content`](Self::content),
    /// one line of a file).
    pub terms: Vec<String>,
    /// Library names (patterns allowed); empty: every library.
    pub libraries: Vec<String>,
    pub kinds: Vec<ViewKind>,
    /// View names (patterns allowed).
    pub views: Vec<String>,
    pub formats: Vec<Format>,
    /// Categories (patterns allowed): a cell must be in one of them, or below.
    pub categories: Vec<String>,
    /// Also look inside text views.
    pub content: bool,
    /// The most hits to return.
    pub limit: usize,
    /// Filters that were not understood, for the user.
    pub problems: Vec<String>,
}

impl Default for Query {
    fn default() -> Self {
        Query {
            terms: Vec::new(),
            libraries: Vec::new(),
            kinds: Vec::new(),
            views: Vec::new(),
            formats: Vec::new(),
            categories: Vec::new(),
            content: false,
            limit: 200,
            problems: Vec::new(),
        }
    }
}

/// A format from what a user would type: a suffix (`gds`, `sch`, `.v`) or a word of
/// its name (`oasis`, `verilog`, `liberty`).
pub fn format_from_name(name: &str) -> Option<Format> {
    let name = name.trim().trim_start_matches('.').to_ascii_lowercase();
    if name.is_empty() {
        return None;
    }
    if let Some((_, format)) = Format::split_name(&format!("x.{name}")) {
        return Some(format);
    }
    const ALL: [Format; 15] = [
        Format::XschemSchematic,
        Format::XschemSymbol,
        Format::QucsSchematic,
        Format::Gds,
        Format::Oasis,
        Format::Magic,
        Format::Spice,
        Format::Cdl,
        Format::Verilog,
        Format::SystemVerilog,
        Format::Vhdl,
        Format::VerilogA,
        Format::Lef,
        Format::Liberty,
        Format::KlayoutPcell,
    ];
    ALL.into_iter().find(|f| {
        f.label()
            .to_ascii_lowercase()
            .split(|c: char| !c.is_ascii_alphanumeric())
            .any(|word| word == name)
    })
}

impl Query {
    /// Read a query as typed (see the module text).
    pub fn parse(input: &str) -> Query {
        let mut query = Query::default();
        for word in input.split_whitespace() {
            let Some((key, value)) = word.split_once(':').filter(|(_, v)| !v.is_empty()) else {
                query.term(word);
                continue;
            };
            match key.to_ascii_lowercase().as_str() {
                "lib" | "library" => query.libraries.push(value.to_string()),
                "kind" => match ViewKind::ALL.into_iter().find(|k| k.name() == value) {
                    Some(kind) => query.kinds.push(kind),
                    None => query.problems.push(format!(
                        "kind:{value} is not a kind of view ({})",
                        ViewKind::ALL.map(|k| k.name()).join(", ")
                    )),
                },
                "view" => query.views.push(value.to_string()),
                "format" | "fmt" => match format_from_name(value) {
                    Some(format) => query.formats.push(format),
                    None => query
                        .problems
                        .push(format!("format:{value} is not a file format CDePT knows")),
                },
                "cat" | "category" => query.categories.push(value.to_string()),
                "in" if value.eq_ignore_ascii_case("content") => query.content = true,
                "in" if value.eq_ignore_ascii_case("names") => query.content = false,
                // Not a filter: a word with a colon in it.
                _ => query.term(word),
            }
        }
        query
    }

    /// Add a word; `library/cell` also sets the library.
    fn term(&mut self, word: &str) {
        match word.rsplit_once('/') {
            Some((library, cell)) if !library.is_empty() && !library.contains('/') => {
                self.libraries.push(format!("{library}*"));
                if !cell.is_empty() {
                    self.terms.push(cell.to_string());
                }
            }
            _ => self.terms.push(word.to_string()),
        }
    }

    /// Hits are views (not cells) when a filter is about views.
    pub fn wants_views(&self) -> bool {
        !(self.kinds.is_empty() && self.views.is_empty() && self.formats.is_empty())
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
            && self.libraries.is_empty()
            && self.categories.is_empty()
            && !self.wants_views()
    }
}

/// What a line found inside a file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentKind {
    /// An instance in a schematic: the line names the symbol.
    Instance,
    /// A pin of a schematic or symbol.
    Pin,
    Text,
}

impl ContentKind {
    pub fn label(self) -> &'static str {
        match self {
            ContentKind::Instance => "instance",
            ContentKind::Pin => "pin",
            ContentKind::Text => "text",
        }
    }
}

/// What made a hit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Matched {
    /// The name of the cell (and the filters).
    Name,
    /// A line inside the view's file.
    Content {
        /// Line number, from 1.
        line: usize,
        text: String,
        kind: ContentKind,
    },
}

/// One result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub library: String,
    pub cell: String,
    /// The view, when the hit is one.
    pub view: Option<String>,
    pub kind: Option<ViewKind>,
    pub file: Option<PathBuf>,
    /// Higher is better.
    pub score: i32,
    pub matched: Matched,
}

/// The results of a search.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Results {
    /// The best hits first, at most `Query::limit`.
    pub hits: Vec<Hit>,
    /// How many hits there were before the limit.
    pub total: usize,
    /// Files looked into.
    pub files_scanned: usize,
    /// Looking inside files stopped early (too many files or too long).
    pub truncated: bool,
}

/// Bounds for looking inside files.
#[derive(Debug, Clone, Copy)]
pub struct ContentLimits {
    /// Larger files are skipped.
    pub max_file_bytes: u64,
    pub max_files: usize,
    pub max_hits_per_file: usize,
    pub time: Duration,
}

impl Default for ContentLimits {
    fn default() -> Self {
        ContentLimits {
            max_file_bytes: 2 << 20,
            max_files: 5000,
            max_hits_per_file: 5,
            time: Duration::from_secs(3),
        }
    }
}

/// `pattern` has `*`, `?` or `[…]`.
pub fn is_pattern(text: &str) -> bool {
    text.contains(['*', '?', '['])
}

/// Match a whole name against a pattern with `*`, `?` and `[abc]`/`[a-z]`/`[!a]`,
/// ignoring case.
pub fn glob(pattern: &str, name: &str) -> bool {
    fn class(pattern: &[char], c: char) -> Option<(bool, usize)> {
        // pattern[0] == '['; returns (matched, length of the class).
        let mut i = 1;
        let negate = matches!(pattern.get(i), Some('!' | '^'));
        if negate {
            i += 1;
        }
        let mut matched = false;
        let mut first = true;
        loop {
            let ch = *pattern.get(i)?;
            if ch == ']' && !first {
                return Some((matched != negate, i + 1));
            }
            first = false;
            if pattern.get(i + 1) == Some(&'-') && pattern.get(i + 2).is_some_and(|e| *e != ']') {
                let end = pattern[i + 2];
                matched |= (ch..=end).contains(&c);
                i += 3;
            } else {
                matched |= ch == c;
                i += 1;
            }
        }
    }
    fn go(pattern: &[char], name: &[char]) -> bool {
        match pattern.split_first() {
            None => name.is_empty(),
            Some(('*', rest)) => (0..=name.len()).any(|skip| go(rest, &name[skip..])),
            Some(('?', rest)) => !name.is_empty() && go(rest, &name[1..]),
            Some(('[', _)) => match name.split_first() {
                Some((c, tail)) => match class(pattern, *c) {
                    Some((true, len)) => go(&pattern[len..], tail),
                    Some((false, _)) => false,
                    // No closing bracket: a plain character.
                    None => *c == '[' && go(&pattern[1..], tail),
                },
                None => false,
            },
            Some((p, rest)) => name.first() == Some(p) && go(rest, &name[1..]),
        }
    }
    let pattern: Vec<char> = pattern.to_lowercase().chars().collect();
    let name: Vec<char> = name.to_lowercase().chars().collect();
    go(&pattern, &name)
}

/// A filter value against a name: a pattern when it has `*?[`, the same text (ignoring
/// case) otherwise.
fn filter_matches(filter: &str, name: &str) -> bool {
    if is_pattern(filter) {
        glob(filter, name)
    } else {
        filter.eq_ignore_ascii_case(name)
    }
}

/// How well `term` matches `name` (ignoring case), `None` if it does not. From best to
/// worst: the same, a prefix, a part starting at a word, a part, the letters in order.
pub fn score(term: &str, name: &str) -> Option<i32> {
    if is_pattern(term) {
        return glob(term, name).then_some(700);
    }
    let t = term.to_lowercase();
    let n = name.to_lowercase();
    if t.is_empty() {
        return Some(0);
    }
    if n == t {
        return Some(1000);
    }
    let shorter = (n.len() as i32 - t.len() as i32).min(100);
    if n.starts_with(&t) {
        return Some(900 - shorter);
    }
    if let Some(at) = n.find(&t) {
        let word_start = n[..at]
            .chars()
            .next_back()
            .is_some_and(|c| !c.is_ascii_alphanumeric());
        let base = if word_start { 750 } else { 600 };
        return Some(base - shorter - (at as i32).min(50));
    }
    fuzzy(&t, &n)
}

/// The letters of `term` in order somewhere in `name`: consecutive letters and letters
/// at the start of a word count more, gaps less. Both lower case.
fn fuzzy(term: &str, name: &str) -> Option<i32> {
    let name: Vec<char> = name.chars().collect();
    let mut total = 0;
    let mut at = 0;
    let mut previous: Option<usize> = None;
    for c in term.chars() {
        let found = (at..name.len()).find(|&i| name[i] == c)?;
        total += 10;
        match previous {
            Some(p) if found == p + 1 => total += 15,
            Some(p) => total -= ((found - p - 1) as i32).min(8),
            None => total -= (found as i32).min(8),
        }
        let word_start = found == 0
            || !name[found - 1].is_ascii_alphanumeric()
            || (name[found - 1].is_ascii_digit() != name[found].is_ascii_digit());
        if word_start {
            total += 12;
        }
        previous = Some(found);
        at = found + 1;
    }
    // Always below a plain part of the name.
    Some((100 + total).clamp(1, 499))
}

/// The sum of the scores of every term against a cell name; `None` when one does not
/// match.
fn score_terms(terms: &[String], name: &str) -> Option<i32> {
    let mut total = 0;
    for term in terms {
        total += score(term, name)?;
    }
    Some(total)
}

fn view_passes(query: &Query, entry: &Entry) -> bool {
    (query.kinds.is_empty() || query.kinds.contains(&entry.kind))
        && (query.formats.is_empty() || entry.format.is_some_and(|f| query.formats.contains(&f)))
        && (query.views.is_empty() || query.views.iter().any(|v| filter_matches(v, &entry.name)))
}

fn library_passes(query: &Query, library: &Library) -> bool {
    library.error.is_none()
        && (query.libraries.is_empty()
            || query
                .libraries
                .iter()
                .any(|l| filter_matches(l, &library.name)))
}

/// The cells of a library that are in one of the wanted categories.
fn in_categories<'a>(wanted: &[String], categories: &'a Categories) -> BTreeSet<&'a str> {
    let mut cells = BTreeSet::new();
    for name in categories.names() {
        // A filter fits a category by its whole path or by its last part.
        let last = name.rsplit('/').next().unwrap_or(&name);
        if wanted
            .iter()
            .any(|w| filter_matches(w, &name) || filter_matches(w, last))
        {
            cells.extend(categories.cells_under(&name));
        }
    }
    cells
}

/// Text files worth looking into.
fn is_text(format: Format) -> bool {
    !matches!(format, Format::Gds | Format::Oasis)
}

/// What a line of a file is, by the format: instances and pins of xschem files.
fn classify(format: Format, line: &str) -> ContentKind {
    let xschem = matches!(format, Format::XschemSchematic | Format::XschemSymbol);
    if !xschem {
        return ContentKind::Text;
    }
    if let Some(rest) = line.strip_prefix("C {") {
        let symbol = rest.split('}').next().unwrap_or("");
        let pin = ["ipin.sym", "opin.sym", "iopin.sym"]
            .iter()
            .any(|p| symbol.ends_with(p));
        return if pin {
            ContentKind::Pin
        } else {
            ContentKind::Instance
        };
    }
    if line.starts_with("B 5 ") {
        return ContentKind::Pin;
    }
    ContentKind::Text
}

/// The lines of `text` that have every term (ignoring case).
fn matching_lines(text: &str, terms: &[String], most: usize) -> Vec<(usize, String)> {
    let terms: Vec<String> = terms.iter().map(|t| t.to_lowercase()).collect();
    let mut out = Vec::new();
    for (number, line) in text.lines().enumerate() {
        let lower = line.to_lowercase();
        if terms.iter().all(|t| lower.contains(t.as_str())) {
            let mut shown: String = line.trim().chars().take(160).collect();
            if line.trim().chars().count() > 160 {
                shown.push('…');
            }
            out.push((number + 1, shown));
            if out.len() >= most {
                break;
            }
        }
    }
    out
}

/// Search with the default bounds for looking inside files.
pub fn search(project: &Project, query: &Query) -> Results {
    search_with(project, query, ContentLimits::default())
}

pub fn search_with(project: &Project, query: &Query, limits: ContentLimits) -> Results {
    let mut results = Results::default();
    let mut hits: Vec<Hit> = Vec::new();
    let started = Instant::now();
    let content = query.content && !query.terms.is_empty();
    let hit = |library: &Library, cell: &Cell, entry: Option<&Entry>, score: i32| Hit {
        library: library.name.clone(),
        cell: cell.name.clone(),
        view: entry.map(|e| e.name.clone()),
        kind: entry.map(|e| e.kind),
        file: entry.and_then(|e| e.main_file.clone()),
        score,
        matched: Matched::Name,
    };

    for library in project
        .libraries
        .iter()
        .filter(|l| library_passes(query, l))
    {
        let categories = (!query.categories.is_empty()).then(|| category::load(project, library));
        let allowed = categories
            .as_ref()
            .map(|c| in_categories(&query.categories, &c.all));
        for cell in &library.cells {
            if allowed
                .as_ref()
                .is_some_and(|a| !a.contains(cell.name.as_str()))
            {
                continue;
            }
            let views: Vec<&Entry> = cell
                .entries
                .iter()
                .filter(|e| view_passes(query, e))
                .collect();
            if query.wants_views() && views.is_empty() {
                continue;
            }
            if let Some(score) = score_terms(&query.terms, &cell.name) {
                if query.wants_views() {
                    hits.extend(views.iter().map(|e| hit(library, cell, Some(e), score)));
                } else {
                    hits.push(hit(library, cell, None, score));
                }
            }
            if !content {
                continue;
            }
            for entry in views {
                let (Some(file), Some(format)) = (&entry.main_file, entry.format) else {
                    continue;
                };
                if !is_text(format) {
                    continue;
                }
                if results.files_scanned >= limits.max_files || started.elapsed() > limits.time {
                    results.truncated = true;
                    break;
                }
                let small = std::fs::metadata(file).is_ok_and(|m| m.len() <= limits.max_file_bytes);
                if !small {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(file) else {
                    continue;
                };
                results.files_scanned += 1;
                for (line, text) in matching_lines(&text, &query.terms, limits.max_hits_per_file) {
                    let kind = classify(format, &text);
                    // Below any name match; instances and pins before plain text.
                    let score = match kind {
                        ContentKind::Instance | ContentKind::Pin => 60,
                        ContentKind::Text => 40,
                    };
                    hits.push(Hit {
                        matched: Matched::Content { line, text, kind },
                        ..hit(library, cell, Some(entry), score)
                    });
                }
            }
        }
    }

    hits.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.cell.cmp(&b.cell))
            .then_with(|| a.library.cmp(&b.library))
            .then_with(|| a.view.cmp(&b.view))
    });
    results.total = hits.len();
    hits.truncate(query.limit.max(1));
    results.hits = hits;
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queries_are_words_and_filters() {
        let q = Query::parse("lib:analog kind:layout cat:amps ota* in:content");
        assert_eq!(q.terms, ["ota*"]);
        assert_eq!(q.libraries, ["analog"]);
        assert_eq!(q.kinds, [ViewKind::Layout]);
        assert_eq!(q.categories, ["amps"]);
        assert!(q.content && q.wants_views() && q.problems.is_empty());

        let q = Query::parse("analog/ota format:gds format:.sch kind:nothing a:b");
        assert_eq!(q.libraries, ["analog*"]);
        assert_eq!(q.terms, ["ota", "a:b"]);
        assert_eq!(q.formats, [Format::Gds, Format::XschemSchematic]);
        assert_eq!(q.problems.len(), 1);
        assert!(Query::parse("  ").is_empty());
        assert!(!Query::parse("x").is_empty());
    }

    #[test]
    fn formats_by_suffix_or_word() {
        assert_eq!(format_from_name("gds"), Some(Format::Gds));
        assert_eq!(format_from_name(".oas"), Some(Format::Oasis));
        assert_eq!(format_from_name("verilog"), Some(Format::Verilog));
        assert_eq!(format_from_name("liberty"), Some(Format::Liberty));
        assert_eq!(format_from_name("magic"), Some(Format::Magic));
        assert_eq!(format_from_name("nothing"), None);
    }

    #[test]
    fn patterns_match_whole_names() {
        assert!(glob("nand*_1", "NAND2_1"));
        assert!(!glob("nand*_1", "nand2_2"));
        assert!(glob("sg13_[lh]v_?mos", "sg13_lv_nmos"));
        assert!(!glob("sg13_[!l]v_?mos", "sg13_lv_nmos"));
        assert!(glob("a[0-9]b", "a5b"));
        assert!(!glob("a[0-9]b", "axb"));
        assert!(glob("*", ""));
        assert!(!glob("a", "ab"));
    }

    #[test]
    fn better_matches_score_higher() {
        let s = |term: &str, name: &str| score(term, name);
        let exact = s("ota", "ota").unwrap();
        let prefix = s("ota", "ota5t").unwrap();
        let word = s("ota", "tb_ota").unwrap();
        let inside = s("ota", "rotary").unwrap();
        let letters = s("o5", "ota5t").unwrap();
        assert!(exact > prefix && prefix > word && word > inside && inside > letters);
        assert_eq!(s("xyz", "ota5t"), None);
        // Letters in order, but not out of order.
        assert!(s("ot5", "ota5t").is_some());
        assert_eq!(s("5o", "ota5t"), None);
        // A shorter name wins among prefixes.
        assert!(s("inv", "inv_1").unwrap() > s("inv", "inverter_long").unwrap());
        assert_eq!(s("OTA", "ota"), Some(1000));
    }

    #[test]
    fn lines_of_xschem_files_are_classified() {
        let sch = Format::XschemSchematic;
        assert_eq!(
            classify(sch, "C {sg13g2_pr/sg13_lv_nmos.sym} 0 0 0 0 {name=M1}"),
            ContentKind::Instance
        );
        assert_eq!(
            classify(sch, "C {devices/ipin.sym} 0 0 0 0 {name=p1 lab=A}"),
            ContentKind::Pin
        );
        assert_eq!(
            classify(Format::XschemSymbol, "B 5 -2 -2 2 2 {name=A dir=in}"),
            ContentKind::Pin
        );
        assert_eq!(classify(Format::Spice, "C {x}"), ContentKind::Text);
        let lines = matching_lines("a VDD b\nc\nvdd GND\n", &["vdd".into()], 5);
        assert_eq!(
            lines,
            [(1, "a VDD b".to_string()), (3, "vdd GND".to_string())]
        );
        let both = matching_lines("a VDD b\nvdd GND\n", &["vdd".into(), "gnd".into()], 5);
        assert_eq!(both.len(), 1);
    }
}
