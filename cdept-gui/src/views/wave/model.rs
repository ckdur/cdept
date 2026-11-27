//! What a waveform tab shows, without drawing: the panes (strips) and the signals in
//! them, the x groups (panes of one x unit share their zoom, pan and cursors), trace
//! styles, and the grouping and search of the signal list.
//!
//! A signal is plotted by name: in a tab of several files (a family of run points) it
//! is drawn once for every visible file that has it. One drawn line, a signal of a
//! source, is an instance ([`InstKey`]).

use super::raster::Dash;

pub type SourceId = u32;
pub type PaneId = u32;

/// A signal of a source: a signal of its file (index in the dataset, 0 is x) or one of
/// the series handed to it (a calculator result).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SigRef {
    File(u32),
    Extra(u32),
}

/// One drawn line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct InstKey {
    pub source: SourceId,
    pub sig: SigRef,
}

/// Trace colours: UberWave's palettes (settings/TraceStyle.hpp), dark and light.
pub const DARK: [[u8; 3]; 10] = [
    [255, 217, 26],
    [26, 230, 230],
    [255, 140, 0],
    [51, 230, 77],
    [255, 89, 204],
    [242, 64, 64],
    [77, 166, 255],
    [153, 255, 51],
    [235, 235, 235],
    [140, 140, 140],
];
pub const LIGHT: [[u8; 3]; 10] = [
    [13, 64, 179],
    [166, 26, 13],
    [0, 102, 77],
    [128, 26, 166],
    [153, 77, 0],
    [0, 102, 140],
    [191, 13, 89],
    [77, 102, 0],
    [38, 38, 38],
    [102, 64, 38],
];

/// How a line is drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Style {
    pub color: [u8; 3],
    pub dash: Dash,
    pub width: f32,
}

/// The style of the `index`-th line of a tab: the palette's colours, then (once they
/// would repeat) the same colours dashed, dotted, dash-dotted, then all of that wider.
/// The first 80 lines differ from each other.
pub fn default_style(index: usize, dark: bool) -> Style {
    let palette = if dark { &DARK } else { &LIGHT };
    let n = palette.len();
    Style {
        color: palette[index % n],
        dash: Dash::ALL[(index / n) % Dash::ALL.len()],
        width: if (index / (n * Dash::ALL.len())) % 2 == 0 {
            1.5
        } else {
            2.5
        },
    }
}

/// A unit in one spelling, to tell panes of different x apart.
pub fn norm_unit(unit: &str) -> String {
    let u = unit.trim();
    match u.to_ascii_lowercase().as_str() {
        "s" | "sec" | "second" | "seconds" | "time" => "s".into(),
        "hz" | "frequency" => "Hz".into(),
        "v" | "volt" | "voltage" => "V".into(),
        "a" | "amp" | "current" => "A".into(),
        _ => u.to_string(),
    }
}

/// A strip of the tab: its own y axis, the signals in it.
#[derive(Debug, Clone, PartialEq)]
pub struct Pane {
    pub id: PaneId,
    pub signals: Vec<String>,
    /// The x unit of its signals (normalised); "" until the first one comes.
    pub x_unit: String,
    /// A y range chosen by the user; `None`: fitted to what is drawn.
    pub y_manual: Option<(f64, f64)>,
}

/// Where a signal goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Active,
    Pane(PaneId),
    New,
}

/// The panes of a tab, top to bottom, and the active one (where "Show" puts signals).
#[derive(Debug, Clone, PartialEq)]
pub struct Panes {
    pub list: Vec<Pane>,
    pub active: PaneId,
    next: PaneId,
}

impl Default for Panes {
    fn default() -> Panes {
        Panes {
            list: vec![Pane {
                id: 1,
                signals: vec![],
                x_unit: String::new(),
                y_manual: None,
            }],
            active: 1,
            next: 2,
        }
    }
}

impl Panes {
    pub fn get(&self, id: PaneId) -> Option<&Pane> {
        self.list.iter().find(|p| p.id == id)
    }

    pub fn get_mut(&mut self, id: PaneId) -> Option<&mut Pane> {
        self.list.iter_mut().find(|p| p.id == id)
    }

    /// 1-based position of a pane (as scripts and the header number it).
    pub fn number(&self, id: PaneId) -> Option<usize> {
        self.list.iter().position(|p| p.id == id).map(|i| i + 1)
    }

    pub fn by_number(&self, n: usize) -> Option<PaneId> {
        self.list.get(n.checked_sub(1)?).map(|p| p.id)
    }

    /// A new empty pane at the bottom; it becomes the active one.
    pub fn add_pane(&mut self) -> PaneId {
        let id = self.next;
        self.next += 1;
        self.list.push(Pane {
            id,
            signals: vec![],
            x_unit: String::new(),
            y_manual: None,
        });
        self.active = id;
        id
    }

    /// The pane a signal is in.
    pub fn pane_of(&self, name: &str) -> Option<PaneId> {
        self.list
            .iter()
            .find(|p| p.signals.iter().any(|s| s == name))
            .map(|p| p.id)
    }

    /// Put `name` (whose x unit is `x_unit`) into a pane: the one asked for when its x
    /// unit fits (or it is empty), else the first pane of that unit, else a new one. A
    /// signal shown already moves. Returns the pane.
    pub fn add(&mut self, name: &str, x_unit: &str, target: Target) -> PaneId {
        let unit = norm_unit(x_unit);
        let empty = self
            .list
            .iter()
            .find(|p| p.signals.is_empty())
            .map(|p| p.id);
        let pane = match target {
            Target::New => match empty {
                Some(id) => id,
                None => self.add_pane(),
            },
            Target::Active | Target::Pane(_) => {
                let wanted = match target {
                    Target::Pane(id) => id,
                    _ => self.active,
                };
                match self.get(wanted) {
                    Some(p) if p.signals.is_empty() || p.x_unit == unit => p.id,
                    _ => match self
                        .list
                        .iter()
                        .find(|p| !p.signals.is_empty() && p.x_unit == unit)
                    {
                        Some(p) => p.id,
                        None => match empty {
                            Some(id) => id,
                            None => self.add_pane(),
                        },
                    },
                }
            }
        };
        self.remove_signal(name);
        let p = self.get_mut(pane).expect("the pane exists");
        if p.signals.is_empty() {
            p.x_unit = unit;
        }
        p.signals.push(name.to_string());
        pane
    }

    /// Take a signal out of its pane (an emptied pane stays).
    pub fn remove_signal(&mut self, name: &str) {
        for p in &mut self.list {
            p.signals.retain(|s| s != name);
        }
    }

    /// Remove a pane and its signals; the last pane stays (emptied).
    pub fn remove_pane(&mut self, id: PaneId) -> Vec<String> {
        let Some(i) = self.list.iter().position(|p| p.id == id) else {
            return vec![];
        };
        if self.list.len() == 1 {
            let p = &mut self.list[0];
            p.y_manual = None;
            p.x_unit.clear();
            return std::mem::take(&mut p.signals);
        }
        let gone = self.list.remove(i);
        if self.active == id {
            self.active = self.list[i.min(self.list.len() - 1)].id;
        }
        gone.signals
    }

    /// Move a pane up (-1) or down (+1).
    pub fn move_pane(&mut self, id: PaneId, by: i32) {
        let Some(i) = self.list.iter().position(|p| p.id == id) else {
            return;
        };
        let j = i as i32 + by;
        if j >= 0 && (j as usize) < self.list.len() {
            self.list.swap(i, j as usize);
        }
    }

    /// All shown signals in pane order.
    pub fn shown(&self) -> impl Iterator<Item = (&Pane, &String)> {
        self.list
            .iter()
            .flat_map(|p| p.signals.iter().map(move |s| (p, s)))
    }
}

/// The panes of one x unit share their x range and cursors.
#[derive(Debug, Clone, PartialEq)]
pub struct XGroup {
    pub unit: String,
    /// The x range of the data (all sources).
    pub full: (f64, f64),
    /// The x range on screen.
    pub view: (f64, f64),
    pub cursors: [Option<f64>; 2],
    /// Drawn on a logarithmic axis (an AC sweep).
    pub log: bool,
}

impl XGroup {
    pub fn new(unit: &str, full: (f64, f64), log: bool) -> XGroup {
        let full = sane_range(full);
        XGroup {
            unit: unit.to_string(),
            full,
            view: full,
            cursors: [None, None],
            log: log && full.0 > 0.0,
        }
    }

    /// The data range grew (another file of a family loaded): a view that showed it all
    /// keeps showing it all.
    pub fn set_full(&mut self, full: (f64, f64)) {
        let full = sane_range(full);
        let showed_all = self.view == self.full;
        self.full = full;
        if showed_all {
            self.view = full;
        }
        if self.log && full.0 <= 0.0 {
            self.log = false;
        }
    }

    pub fn fit(&mut self) {
        self.view = self.full;
    }

    /// Keep the range finite and wider than nothing, no wider than the data; panning
    /// stops at the ends.
    pub fn set_view(&mut self, x0: f64, x1: f64) {
        let span_full = self.full.1 - self.full.0;
        let min_span = (span_full * 1e-12).max(f64::MIN_POSITIVE * 1e6);
        if !(x0.is_finite() && x1.is_finite()) {
            return;
        }
        let (mut a, mut b) = (x0.min(x1), x0.max(x1));
        if b - a < min_span {
            let c = (a + b) / 2.0;
            a = c - min_span / 2.0;
            b = c + min_span / 2.0;
        }
        if b - a >= span_full {
            self.view = self.full;
            return;
        }
        if a < self.full.0 {
            b += self.full.0 - a;
            a = self.full.0;
        }
        if b > self.full.1 {
            a -= b - self.full.1;
            b = self.full.1;
        }
        self.view = (a, b);
    }

    /// Plot coordinate of a data x (log10 on a log axis).
    pub fn to_plot(&self, x: f64) -> f64 {
        if self.log { x.log10() } else { x }
    }

    pub fn from_plot(&self, u: f64) -> f64 {
        if self.log { 10f64.powf(u) } else { u }
    }

    /// The view in plot coordinates.
    pub fn plot_view(&self) -> (f64, f64) {
        (self.to_plot(self.view.0), self.to_plot(self.view.1))
    }

    /// Zoom around a data x by `factor` (<1 zooms in), in plot coordinates.
    pub fn zoom_at(&mut self, x: f64, factor: f64) {
        let (u0, u1) = self.plot_view();
        let c = self.to_plot(x);
        let (a, b) = (c - (c - u0) * factor, c + (u1 - c) * factor);
        self.set_view(self.from_plot(a), self.from_plot(b));
    }

    /// Pan by a fraction of the view's width.
    pub fn pan(&mut self, fraction: f64) {
        let (u0, u1) = self.plot_view();
        let d = (u1 - u0) * fraction;
        let (a, b) = (self.from_plot(u0 + d), self.from_plot(u1 + d));
        // panning keeps the width: clamp in plot space
        if a < self.full.0 {
            let w = u1 - u0;
            let s = self.to_plot(self.full.0);
            self.view = (self.full.0, self.from_plot(s + w).min(self.full.1));
        } else if b > self.full.1 {
            let w = u1 - u0;
            let e = self.to_plot(self.full.1);
            self.view = (self.from_plot(e - w).max(self.full.0), self.full.1);
        } else {
            self.view = (a, b);
        }
    }
}

fn sane_range(r: (f64, f64)) -> (f64, f64) {
    let (a, b) = r;
    if !(a.is_finite() && b.is_finite()) {
        return (0.0, 1.0);
    }
    let (a, b) = (a.min(b), a.max(b));
    if b > a {
        (a, b)
    } else if a == 0.0 {
        (0.0, 1.0)
    } else {
        (a - a.abs() * 0.5, a + a.abs() * 0.5)
    }
}

/// How the signal list is grouped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Grouping {
    /// One list.
    Flat,
    /// Voltages, currents, calculator results, others.
    #[default]
    Kind,
    /// By the instance path in the name (`v(x1.x2.out)`: x1 › x2 › out).
    Hierarchy,
}

impl Grouping {
    pub fn parse(text: &str) -> Option<Grouping> {
        Some(match text {
            "flat" => Grouping::Flat,
            "kind" => Grouping::Kind,
            "hierarchy" | "tree" => Grouping::Hierarchy,
            _ => return None,
        })
    }
}

/// The kind of a signal, for the list's groups.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Voltage,
    Current,
    Calculator,
    Other,
}

impl Kind {
    pub fn title(self) -> &'static str {
        match self {
            Kind::Voltage => "Voltages",
            Kind::Current => "Currents",
            Kind::Calculator => "Calculator",
            Kind::Other => "Other",
        }
    }
}

pub fn kind_of(name: &str, unit: &str, calculator: bool) -> Kind {
    if calculator {
        return Kind::Calculator;
    }
    let n = name.to_ascii_lowercase();
    let u = norm_unit(unit);
    if n.starts_with("i(") || n.ends_with("#branch") || u == "A" {
        Kind::Current
    } else if n.starts_with("v(") || u == "V" {
        Kind::Voltage
    } else {
        Kind::Other
    }
}

/// The instance path of a signal name: `v(x1.x2.out)` gives `["x1", "x2"]` and the leaf
/// `out`; `x1:nm` or `x1/nm` work too.
pub fn hierarchy(name: &str) -> (Vec<String>, String) {
    let inner = name
        .find('(')
        .filter(|_| name.ends_with(')'))
        .map_or(name, |i| &name[i + 1..name.len() - 1]);
    let inner = inner.strip_suffix("#branch").unwrap_or(inner);
    let parts: Vec<&str> = inner
        .split(['.', ':', '/'])
        .filter(|p| !p.is_empty())
        .collect();
    match parts.split_last() {
        Some((leaf, path)) if !path.is_empty() => (
            path.iter().map(|s| s.to_string()).collect(),
            leaf.to_string(),
        ),
        _ => (vec![], name.to_string()),
    }
}

/// The search of the signal list: words that must all be in the name (any case); a
/// word with `*` or `?` is a pattern for the whole name.
pub fn matches(filter: &str, name: &str) -> bool {
    let lower = name.to_lowercase();
    filter.split_whitespace().all(|w| {
        let w = w.to_lowercase();
        if w.contains(['*', '?']) {
            glob(&w, &lower)
        } else {
            lower.contains(&w)
        }
    })
}

fn glob(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti, mut star, mut mark) = (0, 0, None, 0);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// A name for the screen: not longer than `max` characters (an ellipsis in the
/// middle), control characters shown as `?`.
pub fn shorten(name: &str, max: usize) -> String {
    let clean: String = name
        .chars()
        .map(|c| if c.is_control() { '?' } else { c })
        .collect();
    let n = clean.chars().count();
    if n <= max || max < 5 {
        return clean;
    }
    let head = (max - 1) / 2;
    let tail = max - 1 - head;
    let start: String = clean.chars().take(head).collect();
    let end: String = clean.chars().skip(n - tail).collect();
    format!("{start}…{end}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn styles_differ_when_colours_repeat() {
        let styles: Vec<Style> = (0..80).map(|i| default_style(i, true)).collect();
        for i in 0..80 {
            for j in i + 1..80 {
                assert_ne!(styles[i], styles[j], "{i} {j}");
            }
        }
        assert_eq!(default_style(10, true).color, default_style(0, true).color);
        assert_ne!(default_style(10, true).dash, default_style(0, true).dash);
        assert_ne!(
            default_style(40, false).width,
            default_style(0, false).width
        );
    }

    #[test]
    fn signals_go_to_a_pane_of_their_x_unit() {
        let mut p = Panes::default();
        let first = p.add("v(out)", "s", Target::Active);
        assert_eq!(p.add("v(in)", "s", Target::Active), first);
        // a spectrum does not land on a time pane
        let hz = p.add("mag", "Hz", Target::Active);
        assert_ne!(hz, first);
        assert_eq!(p.get(hz).unwrap().x_unit, "Hz");
        // the active pane is now the Hz one; a time signal goes back to the time pane
        assert_eq!(p.active, hz);
        assert_eq!(p.add("v(x)", "time", Target::Active), first);
        // asked for a pane of another unit: to the pane of its unit
        assert_eq!(p.add("phase", "hz", Target::Pane(first)), hz);
        // new: a new pane (the signal moves)
        let new = p.add("v(in)", "s", Target::New);
        assert!(new != first && new != hz);
        assert_eq!(p.pane_of("v(in)"), Some(new));
        assert_eq!(p.get(first).unwrap().signals, vec!["v(out)", "v(x)"]);
        p.move_pane(new, -1);
        assert_eq!(p.number(new), Some(2));
        let gone = p.remove_pane(new);
        assert_eq!(gone, vec!["v(in)"]);
        assert_eq!(p.list.len(), 2);
        p.remove_pane(hz);
        p.remove_pane(first);
        assert_eq!(p.list.len(), 1);
        assert!(p.list[0].signals.is_empty());
        // the one empty pane is reused
        let only = p.list[0].id;
        assert_eq!(p.add("a", "Hz", Target::New), only);
        assert_eq!(p.list[0].x_unit, "Hz");
    }

    #[test]
    fn x_groups_zoom_pan_and_follow_a_growing_range() {
        let mut g = XGroup::new("s", (0.0, 10.0), false);
        g.zoom_at(5.0, 0.5);
        assert_eq!(g.view, (2.5, 7.5));
        g.pan(-1.0);
        assert_eq!(g.view, (0.0, 5.0));
        g.set_full((0.0, 20.0));
        assert_eq!(g.view, (0.0, 5.0));
        g.fit();
        g.set_full((0.0, 30.0));
        assert_eq!(g.view, (0.0, 30.0));
        let mut l = XGroup::new("Hz", (1.0, 1e9), true);
        assert!(l.log);
        l.zoom_at(1e3, 0.5);
        assert!((l.view.0 - 10f64.powf(1.5)).abs() < 1e-6, "{:?}", l.view);
        assert!(!XGroup::new("Hz", (0.0, 1e9), true).log);
        let bad = XGroup::new("s", (f64::NAN, 1.0), false);
        assert_eq!(bad.full, (0.0, 1.0));
        assert_eq!(XGroup::new("s", (3.0, 3.0), false).full, (1.5, 4.5));
    }

    #[test]
    fn list_helpers() {
        assert_eq!(kind_of("v(out)", "V", false), Kind::Voltage);
        assert_eq!(kind_of("vdd#branch", "", false), Kind::Current);
        assert_eq!(kind_of("i(v1)", "A", false), Kind::Current);
        assert_eq!(kind_of("gain", "dB", true), Kind::Calculator);
        assert_eq!(kind_of("frequency", "Hz", false), Kind::Other);
        assert_eq!(
            hierarchy("v(x1.x2.out)"),
            (vec!["x1".into(), "x2".into()], "out".into())
        );
        assert_eq!(hierarchy("v(out)"), (vec![], "v(out)".into()));
        assert_eq!(hierarchy("i(x3.vdd)"), (vec!["x3".into()], "vdd".into()));
        assert!(matches("OUT", "v(out)"));
        assert!(matches("x1 out", "v(x1.out)"));
        assert!(!matches("x2", "v(x1.out)"));
        assert!(matches("v(bit_*)", "v(bit_10)"));
        assert!(!matches("v(bit_?)", "v(bit_10)"));
        assert!(matches("", "anything"));
        assert_eq!(shorten("abcdefghij", 7), "abc…hij");
        assert_eq!(shorten("ab\u{7}c", 10), "ab?c");
        assert_eq!(shorten("ünïcødé", 5), "ün…dé");
    }
}
