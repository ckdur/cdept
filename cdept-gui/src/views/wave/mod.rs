//! A waveform tab: CDePT's own viewer on UberWave's data engine (`cdept-wavedata`).
//!
//! A tab holds one or more sources: files (a raw file, or every run point of a
//! testbench view: a family of corners, sweeps or Monte Carlo runs, each tagged with its
//! point name) and series handed over in memory (calculator results). Signals are
//! plotted by name in panes (strips): a signal is drawn for every visible source that has
//! it. Panes of one x unit share their x range and cursors; each pane has its own y
//! range. Every tab is independent: its own datasets (several tabs on one file share the
//! `.uwraw` cache on disk, nothing else), its own loader threads and query thread
//! (`worker.rs`). The tab is an ordinary dock tab: it docks, floats and moves to other
//! windows like the others.
//!
//! The query thread asks the engine for every drawn line decimated to the width of its
//! pane, with each bucket's minimum and maximum kept (a one-sample spike is drawn at any
//! zoom), and draws each pane into an image (`raster.rs`); the UI shows the newest image
//! at the place it was drawn for while the next one is computed.
//!
//! Mouse on a pane: wheel zooms x around the pointer (Ctrl+wheel zooms that pane's y,
//! Shift+wheel pans x), drag pans, a click puts cursor A, a right click cursor B, a
//! double click fits. Signals are dragged from the list onto a pane; a legend entry's
//! right-click menu changes its colour, dash and width or moves it.

pub(crate) mod model;
pub(crate) mod raster;
pub(crate) mod script;
mod units;
pub(crate) mod worker;

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use cdept_wavedata::{Dataset, Mode, Phase, Prepared, Series};
use egui::{Color32, Event, MouseWheelUnit, PointerButton, Rect, RichText, Sense, Ui};
use egui_plot::{Plot, PlotBounds, PlotImage, PlotPoint, Points, Text, VLine};

use self::model::{
    Grouping, InstKey, Kind, PaneId, Panes, SigRef, SourceId, Style, Target, XGroup,
};
use self::raster::{Dash, Stroke};
use self::worker::{Answer, Data, InstReq, LoadProgress, Loader, PaneReq, Reply, Request, Worker};
use super::View;
use crate::state::AppCtx;

/// How many signals a single file shows when it opens (a testbench uses its defaults).
const FIRST_SHOWN: usize = 2;
/// Frame statistics kept for `wv stats`.
const FRAME_SAMPLES: usize = 4000;
/// Legend entries shown above a pane; the rest are counted.
const LEGEND_MAX: usize = 40;
/// The smallest pane height (points); more panes than fit scroll.
const PANE_MIN_H: f32 = 90.0;
/// Width of every pane's y axis, so the x axes line up.
const Y_AXIS_W: f32 = 64.0;

/// A testbench view: library, cell, view.
pub type Run = (String, String, String);

/// A file or a set of series in the tab.
struct Source {
    id: SourceId,
    /// The point name (family), or the file name.
    tag: String,
    path: Option<PathBuf>,
    state: SourceState,
    /// Series handed to it (calculator results of this point).
    extras: Vec<Arc<Series>>,
    visible: bool,
    /// Opens of this source so far; an answer of an older one is dropped.
    generation: u64,
}

enum SourceState {
    Loading(Arc<LoadProgress>, Instant),
    Ready(Box<Ready>),
    Failed(String),
    /// No file: only series.
    Memory,
}

struct Ready {
    ds: Arc<Dataset>,
    /// Signals 1.. (0 is x): name, unit.
    signals: Vec<(String, String)>,
    index: HashMap<String, u32>,
    x_unit: String,
    x_name: String,
    full: Option<(f64, f64)>,
    x_log: bool,
    points: u64,
    cache: String,
    converted: bool,
    open_time: Duration,
}

/// A mark where the calculator measured (a crossing, an edge), in panes of its x unit.
#[derive(Debug, Clone, PartialEq)]
pub struct Marker {
    pub label: String,
    pub x: f64,
    pub y: Option<f64>,
    pub x_unit: String,
}

/// A signal of the list: what the tab's sources have under that name.
#[derive(Debug, Clone)]
struct Entry {
    name: String,
    unit: String,
    x_unit: String,
    kind: Kind,
    /// Sources that have it.
    count: usize,
}

/// A row of the signal list.
#[derive(Debug, Clone, PartialEq)]
enum Row {
    Header {
        key: String,
        title: String,
        depth: usize,
        count: usize,
    },
    Signal {
        entry: usize,
        depth: usize,
    },
}

/// What is dragged from the signal list or a legend.
#[derive(Debug, Clone)]
struct DragSignals(Vec<String>);

#[derive(Default)]
struct ListState {
    filter: String,
    grouping: Grouping,
    selected: BTreeSet<String>,
    anchor: Option<String>,
    closed: HashSet<String>,
    /// Where each visible row was drawn (scripts drag from there).
    row_rects: HashMap<String, Rect>,
    points_filter: String,
}

/// Frame times and query times since the last `wv stats`.
#[derive(Default)]
struct Stats {
    /// When the tab was drawn last, in which window, at which pass of that window. An
    /// interval counts only between two passes in a row of one window: a tab behind
    /// another one (or moved to another window) is not drawn meanwhile, and that pause
    /// is no frame (it was reported as a 3.4 s frame after a detach).
    last_frame: Option<(Instant, egui::ViewportId, u64)>,
    frames: VecDeque<f32>,
    ui_ms: VecDeque<f32>,
    /// Per reply: queries ms, drawing ms, lines.
    replies: VecDeque<(f64, f64, usize)>,
    modes: [usize; 3],
}

impl Stats {
    fn push<T>(q: &mut VecDeque<T>, v: T) {
        if q.len() >= FRAME_SAMPLES {
            q.pop_front();
        }
        q.push_back(v);
    }
}

fn percentile(sorted: &[f32], p: f64) -> f32 {
    if sorted.is_empty() {
        return 0.0;
    }
    let i = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[i.min(sorted.len() - 1)]
}

/// Cursors and the values under them, as last told to the calculator panel.
type Published = (
    Option<f64>,
    Option<f64>,
    Vec<(String, Option<f64>, Option<f64>)>,
);

/// One line of the tab: pane, signal name, source (index), signal, style index.
type Inst = (PaneId, String, usize, SigRef, usize);

pub struct WaveView {
    id: u64,
    title: String,
    run: Option<Run>,
    show_req: Option<libcdept::wave::show::ShowRequest>,
    sources: Vec<Source>,
    next_source: SourceId,
    loader: Loader,
    worker: Worker,
    panes: Panes,
    groups: Vec<XGroup>,
    styles: HashMap<(String, SourceId), Style>,
    markers: Vec<Marker>,
    show_markers: bool,
    list: ListState,
    catalog: Vec<Entry>,
    catalog_dirty: bool,
    reply: Option<Reply>,
    textures: HashMap<PaneId, egui::TextureHandle>,
    sent: Option<(Request, Instant)>,
    seq: u64,
    /// Plot size of each pane drawn in the last frame, in pixels, and its rectangle.
    pane_px: HashMap<PaneId, (u32, u32)>,
    plot_rects: HashMap<PaneId, Rect>,
    /// Sources whose file changed on disk.
    changed: Vec<SourceId>,
    /// The default signals are still to be shown (when the first file is open).
    defaults_pending: bool,
    dark: bool,
    published: Option<Published>,
    stats: Stats,
    watched: Vec<SourceId>,
}

/// VmRSS, RssFile and RssAnon of this process in KiB.
pub(crate) fn rss_kib() -> (u64, u64, u64) {
    let text = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let field = |name: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(name))
            .and_then(|v| v.split_whitespace().next()?.parse().ok())
            .unwrap_or(0)
    };
    (field("VmRSS:"), field("RssFile:"), field("RssAnon:"))
}

/// Peak RSS, and the memory of the cgroup the process runs in (a systemd scope: page
/// cache counted once), for `wv stats`.
fn memory_note() -> String {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let hwm: u64 = status
        .lines()
        .find_map(|l| l.strip_prefix("VmHWM:"))
        .and_then(|v| v.split_whitespace().next()?.parse().ok())
        .unwrap_or(0);
    let cgroup = std::fs::read_to_string("/proc/self/cgroup")
        .ok()
        .and_then(|t| {
            let rel = t.lines().find_map(|l| l.strip_prefix("0::"))?.to_string();
            let dir = std::path::Path::new("/sys/fs/cgroup").join(rel.trim_start_matches('/'));
            let read = |f: &str| -> Option<u64> {
                std::fs::read_to_string(dir.join(f))
                    .ok()?
                    .trim()
                    .parse()
                    .ok()
            };
            Some((read("memory.current")? >> 20, read("memory.peak")? >> 20))
        });
    match cgroup {
        Some((c, p)) => format!(
            ", peak RSS {} MiB; cgroup {c} MiB, cgroup peak {p} MiB",
            hwm / 1024
        ),
        None => format!(", peak RSS {} MiB", hwm / 1024),
    }
}

fn file_tag(path: &std::path::Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

impl WaveView {
    /// A tab on one file; it starts loading at once.
    pub fn open(ctx: &egui::Context, id: u64, path: PathBuf, title: Option<String>) -> WaveView {
        let title = title.unwrap_or_else(|| file_tag(&path));
        let tag = file_tag(&path);
        WaveView::open_files(ctx, id, title, vec![(tag, path)], None, None)
    }

    /// A tab on several files (tags = point names): a family when `run` is given; the
    /// default signals of `show` (a testbench's) are plotted when the first file opens.
    pub fn open_files(
        ctx: &egui::Context,
        id: u64,
        title: String,
        files: Vec<(String, PathBuf)>,
        run: Option<Run>,
        show: Option<libcdept::wave::show::ShowRequest>,
    ) -> WaveView {
        let mut v = WaveView {
            id,
            title,
            run,
            show_req: show,
            sources: Vec::new(),
            next_source: 1,
            loader: Loader::new(ctx),
            worker: Worker::start(ctx),
            panes: Panes::default(),
            groups: Vec::new(),
            styles: HashMap::new(),
            markers: Vec::new(),
            show_markers: true,
            list: ListState::default(),
            catalog: Vec::new(),
            catalog_dirty: true,
            reply: None,
            textures: HashMap::new(),
            sent: None,
            seq: 0,
            pane_px: HashMap::new(),
            plot_rects: HashMap::new(),
            changed: Vec::new(),
            defaults_pending: true,
            dark: ctx.global_style().visuals.dark_mode,
            published: None,
            stats: Stats::default(),
            watched: Vec::new(),
        };
        for (tag, path) in files {
            v.add_file(tag, path);
        }
        v
    }

    /// The testbench view the tab shows points of.
    pub fn run(&self) -> Option<&Run> {
        self.run.as_ref()
    }

    /// Add a file (it starts loading).
    pub fn add_file(&mut self, tag: String, path: PathBuf) -> SourceId {
        let id = self.next_source;
        self.next_source += 1;
        let progress = self.loader.open(id, 1, path.clone());
        self.sources.push(Source {
            id,
            tag,
            path: Some(path),
            state: SourceState::Loading(progress, Instant::now()),
            extras: Vec::new(),
            visible: true,
            generation: 1,
        });
        self.catalog_dirty = true;
        id
    }

    fn reload_source(&mut self, id: SourceId) {
        let Some(s) = self.sources.iter_mut().find(|s| s.id == id) else {
            return;
        };
        let Some(path) = s.path.clone() else { return };
        s.generation += 1;
        let progress = self.loader.open(id, s.generation, path);
        s.state = SourceState::Loading(progress, Instant::now());
        self.changed.retain(|c| *c != id);
        self.catalog_dirty = true;
    }

    /// Open the files again (what Reload and Try Again do): all, or the changed and
    /// failed ones.
    fn reload_all(&mut self, only_changed: bool) {
        let ids: Vec<SourceId> = self
            .sources
            .iter()
            .filter(|s| s.path.is_some())
            .filter(|s| {
                !only_changed
                    || self.changed.contains(&s.id)
                    || matches!(s.state, SourceState::Failed(_))
            })
            .map(|s| s.id)
            .collect();
        for id in ids {
            self.reload_source(id);
        }
    }

    /// Hand a series (a calculator result) to the source tagged `tag` (a new source
    /// without a file when no source has that tag) and plot it in a pane of its x
    /// unit. A series of that name of that source is replaced.
    pub fn add_series(&mut self, tag: &str, series: Series) {
        let name = series.name.clone();
        let x_unit = model::norm_unit(&series.x_unit);
        let si = match self.sources.iter().position(|s| s.tag == tag) {
            Some(i) => i,
            None => {
                let id = self.next_source;
                self.next_source += 1;
                self.sources.push(Source {
                    id,
                    tag: tag.to_string(),
                    path: None,
                    state: SourceState::Memory,
                    extras: Vec::new(),
                    visible: true,
                    generation: 0,
                });
                self.sources.len() - 1
            }
        };
        let s = &mut self.sources[si];
        let series = Arc::new(series);
        match s.extras.iter().position(|e| e.name == name) {
            Some(i) => s.extras[i] = series,
            None => s.extras.push(series),
        }
        self.catalog_dirty = true;
        self.defaults_pending = false;
        self.refresh_catalog();
        if self.panes.pane_of(&name).is_none() {
            self.panes.add(&name, &x_unit, Target::Active);
        }
        self.refresh_groups();
    }

    /// Replace the calculator's markers.
    pub fn set_markers(&mut self, markers: Vec<Marker>) {
        self.markers = markers;
    }

    // --- sources and the catalogue ---

    fn loading(&self) -> usize {
        self.sources
            .iter()
            .filter(|s| matches!(s.state, SourceState::Loading(..)))
            .count()
    }

    fn failed(&self) -> usize {
        self.sources
            .iter()
            .filter(|s| matches!(s.state, SourceState::Failed(_)))
            .count()
    }

    fn ready(&self) -> usize {
        self.sources
            .iter()
            .filter(|s| matches!(s.state, SourceState::Ready(_)))
            .count()
    }

    /// Every frame, for every tab, shown or not: files that finished opening, answers
    /// of the query thread.
    pub(crate) fn tick(&mut self, ctx: &egui::Context) {
        let mut any = false;
        for (id, generation, result) in self.loader.poll() {
            let Some(s) = self.sources.iter_mut().find(|s| s.id == id) else {
                continue;
            };
            if s.generation != generation {
                continue;
            }
            any = true;
            let path = s
                .path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            s.state = match result {
                Ok((ds, took)) => {
                    tracing::info!(
                        target: "cdept::wave",
                        "{path}: opened in {:.0} ms ({} signals, {} points, {})",
                        took.as_secs_f64() * 1e3,
                        ds.signal_count(),
                        ds.point_count(),
                        if ds.converted() { "cache written" } else { "cache reused" }
                    );
                    SourceState::Ready(Box::new(ready_of(ds, took)))
                }
                Err(e) => {
                    tracing::warn!(target: "cdept::wave", "{path}: {e}");
                    SourceState::Failed(e.message)
                }
            };
        }
        if any {
            self.catalog_dirty = true;
            self.refresh_catalog();
            self.refresh_groups();
            if self.defaults_pending && self.ready() > 0 {
                self.show_defaults();
            }
            let watched: Vec<SourceId> = self
                .sources
                .iter()
                .filter(|s| matches!(s.state, SourceState::Ready(_)))
                .map(|s| s.id)
                .collect();
            if watched != self.watched {
                self.watched = watched;
                self.worker.watch(
                    self.sources
                        .iter()
                        .filter_map(|s| match &s.state {
                            SourceState::Ready(r) => Some((s.id, r.ds.clone())),
                            _ => None,
                        })
                        .collect(),
                );
            }
        }
        for a in self.worker.answers() {
            match a {
                Answer::Changed(ids) => self.changed = ids,
                Answer::Drawn(r) => self.take_reply(ctx, r),
            }
        }
        // also from a tab behind another one (a script, a cursor moved before)
        self.publish_cursors();
    }

    fn take_reply(&mut self, ctx: &egui::Context, mut r: Reply) {
        let lines: usize = r.panes.iter().map(|p| p.insts.len()).sum();
        Stats::push(&mut self.stats.replies, (r.query_ms, r.raster_ms, lines));
        for p in &r.panes {
            for a in &p.insts {
                let m = match a.info.mode {
                    Mode::Direct => 0,
                    Mode::MinMax => 1,
                    Mode::Pyramid => 2,
                };
                self.stats.modes[m] += 1;
            }
        }
        for e in &r.errors {
            tracing::warn!(target: "cdept::wave", "query: {e}");
        }
        for p in &mut r.panes {
            if let Some(image) = p.image.take() {
                match self.textures.get_mut(&p.pane) {
                    Some(t) => t.set(image, egui::TextureOptions::LINEAR),
                    None => {
                        let t = ctx.load_texture(
                            format!("wave-{}-{}", self.id, p.pane),
                            image,
                            egui::TextureOptions::LINEAR,
                        );
                        self.textures.insert(p.pane, t);
                    }
                }
            }
        }
        let panes: HashSet<PaneId> = self.panes.list.iter().map(|p| p.id).collect();
        self.textures.retain(|id, _| panes.contains(id));
        self.reply = Some(r);
        self.publish_cursors();
    }

    /// The list of signals: every name of every source, once.
    fn refresh_catalog(&mut self) {
        if !self.catalog_dirty {
            return;
        }
        self.catalog_dirty = false;
        let mut seen: HashMap<String, usize> = HashMap::new();
        let mut list: Vec<Entry> = Vec::new();
        for s in &self.sources {
            let mut add = |name: &str, unit: &str, x_unit: &str, calc: bool| {
                if let Some(&i) = seen.get(name) {
                    list[i].count += 1;
                } else {
                    seen.insert(name.to_string(), list.len());
                    list.push(Entry {
                        name: name.to_string(),
                        unit: unit.to_string(),
                        x_unit: model::norm_unit(x_unit),
                        kind: model::kind_of(name, unit, calc),
                        count: 1,
                    });
                }
            };
            if let SourceState::Ready(r) = &s.state {
                for (n, u) in &r.signals {
                    add(n, u, &r.x_unit, false);
                }
            }
            for e in &s.extras {
                add(&e.name, &e.unit, &e.x_unit, true);
            }
        }
        self.catalog = list;
    }

    fn entry(&self, name: &str) -> Option<&Entry> {
        self.catalog.iter().find(|e| e.name == name)
    }

    /// The x groups: one per x unit of the panes, with the range of all the sources of
    /// that unit.
    fn refresh_groups(&mut self) {
        let mut ranges: HashMap<String, ((f64, f64), bool)> = HashMap::new();
        let mut merge = |unit: &str, r: (f64, f64), log: bool| {
            let e = ranges
                .entry(unit.to_string())
                .or_insert(((f64::INFINITY, f64::NEG_INFINITY), false));
            e.0 = (e.0.0.min(r.0), e.0.1.max(r.1));
            e.1 |= log;
        };
        for s in &self.sources {
            if let SourceState::Ready(r) = &s.state
                && let Some(full) = r.full
            {
                merge(&r.x_unit, full, r.x_log);
            }
            for e in &s.extras {
                if let Some(full) = e.x_range() {
                    merge(&model::norm_unit(&e.x_unit), full, false);
                }
            }
        }
        let units: Vec<String> = self.panes.list.iter().map(|p| p.x_unit.clone()).collect();
        for unit in units {
            let Some((range, log)) = ranges.get(&unit).copied() else {
                if !self.groups.iter().any(|g| g.unit == unit) {
                    // an empty pane (or one whose files are not open yet) still gets an axis
                    self.groups.push(XGroup::new(&unit, (0.0, 1.0), false));
                }
                continue;
            };
            match self.groups.iter_mut().find(|g| g.unit == unit) {
                Some(g) => {
                    if g.full != range {
                        g.set_full(range);
                    }
                    if log && !g.log && range.0 > 0.0 {
                        g.log = true;
                    }
                }
                None => self.groups.push(XGroup::new(&unit, range, log)),
            }
        }
    }

    fn group_of(&self, x_unit: &str) -> Option<usize> {
        self.groups.iter().position(|g| g.unit == x_unit)
    }

    /// The signals a tab plots when its first file opens: the testbench's defaults, else
    /// the first two of the file.
    fn show_defaults(&mut self) {
        self.defaults_pending = false;
        if self.panes.shown().next().is_some() {
            return;
        }
        let Some(r) = self.sources.iter().find_map(|s| match &s.state {
            SourceState::Ready(r) => Some(r),
            _ => None,
        }) else {
            return;
        };
        let names: Vec<String> = match &self.show_req {
            Some(req) => {
                let signals: Vec<libcdept::wave::ipc::Signal> = r
                    .signals
                    .iter()
                    .map(|(n, u)| libcdept::wave::ipc::Signal {
                        name: n.clone(),
                        file: String::new(),
                        unit: u.clone(),
                        complex: false,
                    })
                    .collect();
                libcdept::wave::show::pick_default(&signals, req)
            }
            None => r
                .signals
                .iter()
                .take(FIRST_SHOWN)
                .map(|(n, _)| n.clone())
                .collect(),
        };
        let x_unit = r.x_unit.clone();
        for n in names {
            self.panes.add(&n, &x_unit, Target::Active);
        }
        self.refresh_groups();
    }

    /// Show `names` (signal names) in `target`.
    fn show(&mut self, names: &[String], target: Target) {
        self.refresh_catalog();
        let mut target = target;
        for n in names {
            let x_unit = self.entry(n).map(|e| e.x_unit.clone()).unwrap_or_default();
            let pane = self.panes.add(n, &x_unit, target);
            // the next ones of a "new pane" go to the same new pane
            if target == Target::New {
                target = Target::Pane(pane);
            }
        }
        self.refresh_groups();
    }

    fn hide(&mut self, names: &[String]) {
        for n in names {
            self.panes.remove_signal(n);
        }
    }

    // --- the lines ---

    /// Every line of the tab in pane order: pane, signal name, source, signal, its index
    /// for the default style (counted over all sources, so hiding a point does not change
    /// the others' colours).
    fn instances(&self) -> Vec<Inst> {
        let mut out = Vec::new();
        let mut index = 0;
        for (pane, name) in self.panes.shown() {
            for (si, s) in self.sources.iter().enumerate() {
                let sig = match &s.state {
                    SourceState::Ready(r) => r.index.get(name).map(|i| SigRef::File(*i)),
                    _ => None,
                }
                .or_else(|| {
                    s.extras
                        .iter()
                        .position(|e| &e.name == name)
                        .map(|i| SigRef::Extra(i as u32))
                });
                let present = sig.is_some()
                    || matches!(s.state, SourceState::Loading(..) | SourceState::Failed(_));
                if let Some(sig) = sig {
                    out.push((pane.id, name.clone(), si, sig, index));
                }
                if present {
                    index += 1;
                }
            }
        }
        out
    }

    fn style_of(&self, name: &str, source: SourceId, index: usize) -> Style {
        self.styles
            .get(&(name.to_string(), source))
            .copied()
            .unwrap_or_else(|| model::default_style(index, self.dark))
    }

    fn label_of(&self, name: &str, source: usize) -> String {
        if self.sources.len() > 1 {
            format!("{name} @ {}", self.sources[source].tag)
        } else {
            name.to_string()
        }
    }

    fn data_of(&self, source: usize, sig: SigRef) -> Option<Data> {
        let s = &self.sources[source];
        match sig {
            SigRef::File(i) => match &s.state {
                SourceState::Ready(r) => Some(Data::File(r.ds.clone(), i)),
                _ => None,
            },
            SigRef::Extra(i) => s.extras.get(i as usize).map(|e| Data::Series(e.clone())),
        }
    }

    /// What the panes need drawn now.
    fn wanted(&self) -> Request {
        let insts = self.instances();
        let panes = self
            .panes
            .list
            .iter()
            .map(|p| {
                let g = self.group_of(&p.x_unit).map(|i| &self.groups[i]);
                let (w, h) = self.pane_px.get(&p.id).copied().unwrap_or((0, 0));
                let (x0, x1) = g.map_or((0.0, 1.0), |g| g.view);
                PaneReq {
                    pane: p.id,
                    x0,
                    x1,
                    log: g.is_some_and(|g| g.log),
                    w,
                    h,
                    y: p.y_manual,
                    insts: insts
                        .iter()
                        .filter(|(pane, _, si, _, _)| *pane == p.id && self.sources[*si].visible)
                        .filter_map(|(_, name, si, sig, index)| {
                            let style = self.style_of(name, self.sources[*si].id, *index);
                            Some(InstReq {
                                key: InstKey {
                                    source: self.sources[*si].id,
                                    sig: *sig,
                                },
                                data: self.data_of(*si, *sig)?,
                                stroke: Stroke {
                                    rgba: [style.color[0], style.color[1], style.color[2], 255],
                                    width: style.width,
                                    dash: style.dash,
                                },
                            })
                        })
                        .collect(),
                    cursors: g.map_or([None, None], |g| g.cursors),
                }
            })
            .collect();
        Request { seq: 0, panes }
    }

    /// Send a request when what is on screen changed (or pyramids are being built).
    fn request(&mut self) {
        let wanted = self.wanted();
        let same = self.sent.as_ref().is_some_and(|(r, _)| r.same(&wanted));
        let again = same
            && self.reply.as_ref().is_some_and(Reply::building)
            && self
                .sent
                .as_ref()
                .is_some_and(|(_, t)| t.elapsed() > Duration::from_millis(300));
        if same && !again {
            return;
        }
        self.seq += 1;
        let request = Request {
            seq: self.seq,
            ..wanted
        };
        self.worker.send(request.clone());
        self.sent = Some((request, Instant::now()));
    }

    /// The newest answer covers what is wanted now.
    fn up_to_date(&self) -> bool {
        match (&self.reply, &self.sent) {
            (Some(r), Some((s, _))) => {
                r.request.seq == s.seq
                    && s.same(&self.wanted())
                    && self
                        .worker
                        .pending
                        .load(std::sync::atomic::Ordering::SeqCst)
                        == 0
            }
            _ => false,
        }
    }

    /// Tell the calculator panel where the cursors are and what is under them (the
    /// group of time, else the first), as UberWave's cursor callback does.
    fn publish_cursors(&mut self) {
        let Some(gi) =
            self.groups
                .iter()
                .position(|g| g.unit == "s")
                .or(if self.groups.is_empty() {
                    None
                } else {
                    Some(0)
                })
        else {
            return;
        };
        let g = &self.groups[gi];
        let [a, b] = g.cursors;
        let mut values = Vec::new();
        if let Some(r) = &self.reply {
            for p in &r.panes {
                let Some(pane) = self.panes.get(p.pane) else {
                    continue;
                };
                if pane.x_unit != g.unit {
                    continue;
                }
                for ans in &p.insts {
                    if let Some(si) = self.sources.iter().position(|s| s.id == ans.key.source) {
                        let name = self.sig_name(si, ans.key.sig);
                        values.push((self.label_of(&name, si), ans.at_cursor[0], ans.at_cursor[1]));
                    }
                }
            }
        }
        let now = (a, b, values);
        if self.published.as_ref() == Some(&now) {
            return;
        }
        let first = self.published.is_none();
        self.published = Some(now.clone());
        // a tab that never had a cursor says nothing
        if first && a.is_none() && b.is_none() {
            return;
        }
        crate::views::simulation::publish_tab_cursors(now.0, now.1, now.2);
    }

    fn sig_name(&self, source: usize, sig: SigRef) -> String {
        let s = &self.sources[source];
        match sig {
            SigRef::File(i) => match &s.state {
                SourceState::Ready(r) => r
                    .signals
                    .get(i.saturating_sub(1) as usize)
                    .map(|(n, _)| n.clone())
                    .unwrap_or_default(),
                _ => String::new(),
            },
            SigRef::Extra(i) => s
                .extras
                .get(i as usize)
                .map(|e| e.name.clone())
                .unwrap_or_default(),
        }
    }

    // --- drawing ---

    fn toolbar(&mut self, ui: &mut Ui) {
        ui.horizontal_wrapped(|ui| {
            if ui
                .button("Fit")
                .on_hover_text("The whole x range, y fitted (double click)")
                .clicked()
            {
                self.fit_all();
            }
            let active_unit = self
                .panes
                .get(self.panes.active)
                .map(|p| p.x_unit.clone())
                .unwrap_or_default();
            if let Some(gi) = self.group_of(&active_unit) {
                let g = &mut self.groups[gi];
                let (u0, u1) = g.plot_view();
                let c = g.data_x((u0 + u1) / 2.0);
                if ui.button("Zoom In").clicked() {
                    g.zoom_at(c, 0.5);
                }
                if ui.button("Zoom Out").clicked() {
                    g.zoom_at(c, 2.0);
                }
            }
            let any_cursor = self
                .groups
                .iter()
                .any(|g| g.cursors.iter().any(Option::is_some));
            if ui
                .add_enabled(any_cursor, egui::Button::new("Clear Cursors"))
                .clicked()
            {
                for g in &mut self.groups {
                    g.cursors = [None, None];
                }
            }
            if ui
                .button("+ Pane")
                .on_hover_text("A new empty pane below; it becomes the active one")
                .clicked()
            {
                self.panes.add_pane();
                self.refresh_groups();
            }
            if !self.markers.is_empty() {
                ui.checkbox(
                    &mut self.show_markers,
                    format!("Markers ({})", self.markers.len()),
                )
                .on_hover_text("Where the calculator measured (crossings, edges, extremes)");
            }
            ui.separator();
            let (loading, failed) = (self.loading(), self.failed());
            if self.sources.len() > 1 || loading > 0 || failed > 0 {
                let files = self.sources.iter().filter(|s| s.path.is_some()).count();
                let mut text = format!("{} of {files} files open", self.ready());
                if loading > 0 {
                    let queued = self.loader.queued();
                    text.push_str(&format!(" · {loading} loading"));
                    if queued > 0 {
                        text.push_str(&format!(" ({queued} waiting)"));
                    }
                }
                if failed > 0 {
                    text.push_str(&format!(" · {failed} failed"));
                }
                ui.weak(text);
                if failed > 0
                    && ui
                        .small_button("Try Again")
                        .on_hover_text("Open the files that failed again")
                        .clicked()
                {
                    self.reload_all(true);
                }
            }
            let (lines, points, building) = self.reply.as_ref().map_or((0, 0, 0), |r| {
                let lines: usize = r.panes.iter().map(|p| p.insts.len()).sum();
                let points: usize = r
                    .panes
                    .iter()
                    .flat_map(|p| &p.insts)
                    .map(|a| a.points)
                    .sum();
                let building = r
                    .panes
                    .iter()
                    .flat_map(|p| &p.insts)
                    .filter(|a| matches!(a.prepared, Prepared::Building(_)))
                    .count();
                (lines, points, building)
            });
            ui.weak(format!(
                "{lines} lines · {points} points drawn · {:.1} + {:.1} ms{}",
                self.reply.as_ref().map_or(0.0, |r| r.query_ms),
                self.reply.as_ref().map_or(0.0, |r| r.raster_ms),
                if building > 0 {
                    format!(" · indexing {building} signal(s)")
                } else {
                    String::new()
                }
            ))
            .on_hover_text(
                "lines drawn, points the engine gave for them, query and drawing time of the newest answer",
            );
            if !self.changed.is_empty() {
                ui.separator();
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    if self.changed.len() == 1 && self.sources.len() == 1 {
                        "The file changed on disk".to_string()
                    } else {
                        format!("{} file(s) changed on disk", self.changed.len())
                    },
                );
                if ui.button("Reload").clicked() {
                    self.reload_all(true);
                }
            }
        });
    }

    fn fit_all(&mut self) {
        for g in &mut self.groups {
            g.fit();
        }
        for p in &mut self.panes.list {
            p.y_manual = None;
        }
    }

    /// The rows of the signal list for the filter and grouping.
    fn rows(&self) -> Vec<Row> {
        let filter = self.list.filter.trim();
        let entries: Vec<usize> = (0..self.catalog.len())
            .filter(|i| filter.is_empty() || model::matches(filter, &self.catalog[*i].name))
            .collect();
        let mut rows = Vec::new();
        match self.list.grouping {
            Grouping::Flat => {
                rows.extend(entries.iter().map(|&i| Row::Signal { entry: i, depth: 0 }));
            }
            Grouping::Kind => {
                for kind in [Kind::Voltage, Kind::Current, Kind::Calculator, Kind::Other] {
                    let mine: Vec<usize> = entries
                        .iter()
                        .copied()
                        .filter(|i| self.catalog[*i].kind == kind)
                        .collect();
                    if mine.is_empty() {
                        continue;
                    }
                    let key = kind.title().to_string();
                    let open = !self.list.closed.contains(&key);
                    rows.push(Row::Header {
                        key: key.clone(),
                        title: kind.title().into(),
                        depth: 0,
                        count: mine.len(),
                    });
                    if open {
                        rows.extend(mine.into_iter().map(|i| Row::Signal { entry: i, depth: 1 }));
                    }
                }
            }
            Grouping::Hierarchy => {
                let mut paths: Vec<(Vec<String>, String, usize)> = entries
                    .iter()
                    .map(|&i| {
                        let (p, leaf) = model::hierarchy(&self.catalog[i].name);
                        (p, leaf, i)
                    })
                    .collect();
                paths.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
                let mut emitted: HashSet<String> = HashSet::new();
                for (path, _, i) in &paths {
                    let mut hidden = false;
                    for d in 0..path.len() {
                        let key = path[..=d].join(".");
                        if !emitted.contains(&key) && !hidden {
                            emitted.insert(key.clone());
                            let count = paths
                                .iter()
                                .filter(|(p, _, _)| p.len() > d && p[..=d] == path[..=d])
                                .count();
                            rows.push(Row::Header {
                                key: key.clone(),
                                title: path[d].clone(),
                                depth: d,
                                count,
                            });
                        }
                        if self.list.closed.contains(&key) {
                            hidden = true;
                        }
                    }
                    if !hidden {
                        rows.push(Row::Signal {
                            entry: *i,
                            depth: path.len(),
                        });
                    }
                }
            }
        }
        rows
    }

    fn signal_list(&mut self, ui: &mut Ui) {
        self.refresh_catalog();
        let shown: HashSet<String> = self.panes.shown().map(|(_, s)| s.clone()).collect();
        ui.horizontal(|ui| {
            ui.strong("Signals");
            ui.weak(format!("{} shown of {}", shown.len(), self.catalog.len()));
        });
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.list.filter)
                    .hint_text("search (words, or a pattern with *)")
                    .desired_width((ui.available_width() - 72.0).max(60.0)),
            );
            egui::ComboBox::from_id_salt(("wave-grouping", self.id))
                .width(60.0)
                .selected_text(match self.list.grouping {
                    Grouping::Flat => "flat",
                    Grouping::Kind => "kind",
                    Grouping::Hierarchy => "tree",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.list.grouping, Grouping::Flat, "flat");
                    ui.selectable_value(&mut self.list.grouping, Grouping::Kind, "kind")
                        .on_hover_text("Voltages, currents, calculator results, others");
                    ui.selectable_value(&mut self.list.grouping, Grouping::Hierarchy, "tree")
                        .on_hover_text("By instance: v(x1.x2.out) under x1 › x2");
                });
        });
        let selected: Vec<String> = self.list.selected.iter().cloned().collect();
        ui.horizontal_wrapped(|ui| {
            let any = !selected.is_empty();
            if ui
                .add_enabled(any, egui::Button::new("Show").small())
                .on_hover_text("The selected signals into the active pane")
                .clicked()
            {
                self.show(&selected, Target::Active);
            }
            if ui
                .add_enabled(any, egui::Button::new("New Pane").small())
                .on_hover_text("The selected signals into a new pane")
                .clicked()
            {
                self.show(&selected, Target::New);
            }
            if ui
                .add_enabled(any, egui::Button::new("Hide").small())
                .clicked()
            {
                self.hide(&selected);
            }
            if any
                && ui
                    .small_button("Clear")
                    .on_hover_text("Clear the selection")
                    .clicked()
            {
                self.list.selected.clear();
            }
            if any {
                ui.weak(format!("{} selected", selected.len()));
            }
        });
        let rows = self.rows();
        let order: Vec<String> = rows
            .iter()
            .filter_map(|r| match r {
                Row::Signal { entry, .. } => Some(self.catalog[*entry].name.clone()),
                _ => None,
            })
            .collect();
        let colors = self.list_colors();
        let row_h = ui.spacing().interact_size.y;
        self.list.row_rects.clear();
        let mut toggle: Option<(String, bool)> = None;
        let mut header_toggle: Option<String> = None;
        let mut click: Option<(String, egui::Modifiers)> = None;
        let many_sources = self.sources.len();
        egui::ScrollArea::vertical()
            .id_salt(("wave-signal-rows", self.id))
            .auto_shrink([false, false])
            .show_rows(ui, row_h, rows.len(), |ui, range| {
                for row in &rows[range] {
                    match row {
                        Row::Header {
                            key,
                            title,
                            depth,
                            count,
                        } => {
                            let open = !self.list.closed.contains(key);
                            ui.horizontal(|ui| {
                                ui.add_space(*depth as f32 * 12.0);
                                let arrow = if open { "⏷" } else { "⏵" };
                                if ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new(format!(
                                                "{arrow} {} ({count})",
                                                model::shorten(title, 28)
                                            ))
                                            .strong(),
                                        )
                                        .frame(false),
                                    )
                                    .clicked()
                                {
                                    header_toggle = Some(key.clone());
                                }
                            });
                        }
                        Row::Signal { entry, depth } => {
                            let e = &self.catalog[*entry];
                            ui.horizontal(|ui| {
                                ui.add_space(*depth as f32 * 12.0);
                                let mut on = shown.contains(&e.name);
                                if ui
                                    .checkbox(&mut on, "")
                                    .on_hover_text("Show in the active pane / hide")
                                    .changed()
                                {
                                    toggle = Some((e.name.clone(), on));
                                }
                                let is_sel = self.list.selected.contains(&e.name);
                                let mut text = RichText::new(model::shorten(&e.name, 32));
                                if let Some(c) = colors.get(&e.name) {
                                    text = text.color(*c);
                                }
                                let r = ui.add(
                                    egui::Button::selectable(is_sel, text)
                                        .sense(Sense::click_and_drag()),
                                );
                                let mut tip =
                                    format!("{}\nunit: {}  x: {}", e.name, e.unit, e.x_unit);
                                if many_sources > 1 {
                                    tip.push_str(&format!(
                                        "\nin {} of {many_sources} files",
                                        e.count
                                    ));
                                }
                                let r = r.on_hover_text(tip);
                                if r.clicked() {
                                    click = Some((e.name.clone(), ui.input(|i| i.modifiers)));
                                }
                                if r.dragged() {
                                    let names: Vec<String> = if self.list.selected.contains(&e.name)
                                    {
                                        self.list.selected.iter().cloned().collect()
                                    } else {
                                        vec![e.name.clone()]
                                    };
                                    let n = names.len();
                                    r.dnd_set_drag_payload(DragSignals(names));
                                    if let Some(pos) = ui.ctx().pointer_interact_pos() {
                                        let text = if n == 1 {
                                            model::shorten(&e.name, 48)
                                        } else {
                                            format!("{n} signals")
                                        };
                                        ui.ctx()
                                            .layer_painter(egui::LayerId::new(
                                                egui::Order::Tooltip,
                                                egui::Id::new(("wave-drag", self.id)),
                                            ))
                                            .text(
                                                pos + egui::vec2(12.0, 4.0),
                                                egui::Align2::LEFT_TOP,
                                                text,
                                                egui::TextStyle::Body.resolve(ui.style()),
                                                ui.visuals().strong_text_color(),
                                            );
                                    }
                                }
                                self.list.row_rects.insert(e.name.clone(), r.rect);
                                if !e.unit.is_empty() {
                                    ui.weak(&e.unit);
                                }
                            });
                        }
                    }
                }
            });
        if let Some(k) = header_toggle
            && !self.list.closed.remove(&k)
        {
            self.list.closed.insert(k);
        }
        if let Some((name, on)) = toggle {
            if on {
                self.show(std::slice::from_ref(&name), Target::Active);
            } else {
                self.hide(std::slice::from_ref(&name));
            }
        }
        if let Some((name, m)) = click {
            self.select_click(&name, m, &order);
        }
    }

    /// A click in the list: select it alone; Ctrl adds or removes it; Shift selects the
    /// rows from the last clicked one.
    fn select_click(&mut self, name: &str, m: egui::Modifiers, order: &[String]) {
        if m.shift
            && let Some(anchor) = &self.list.anchor
            && let (Some(a), Some(b)) = (
                order.iter().position(|n| n == anchor),
                order.iter().position(|n| n == name),
            )
        {
            if !m.ctrl && !m.command {
                self.list.selected.clear();
            }
            for n in &order[a.min(b)..=a.max(b)] {
                self.list.selected.insert(n.clone());
            }
            return;
        }
        if m.ctrl || m.command {
            if !self.list.selected.remove(name) {
                self.list.selected.insert(name.to_string());
            }
        } else {
            self.list.selected.clear();
            self.list.selected.insert(name.to_string());
        }
        self.list.anchor = Some(name.to_string());
    }

    /// The colour of each shown signal in the list: its first visible line's.
    fn list_colors(&self) -> HashMap<String, Color32> {
        let mut out = HashMap::new();
        for (_, name, si, _, idx) in self.instances() {
            if !self.sources[si].visible || out.contains_key(&name) {
                continue;
            }
            let s = self.style_of(&name, self.sources[si].id, idx);
            out.insert(name, Color32::from_rgb(s.color[0], s.color[1], s.color[2]));
        }
        out
    }

    fn points_list(&mut self, ui: &mut Ui) {
        let n = self.sources.len();
        ui.horizontal(|ui| {
            ui.strong("Points");
            let visible = self.sources.iter().filter(|s| s.visible).count();
            ui.weak(format!("{visible} shown of {n}"));
            if ui.small_button("All").clicked() {
                for s in &mut self.sources {
                    s.visible = true;
                }
            }
            if ui.small_button("None").clicked() {
                for s in &mut self.sources {
                    s.visible = false;
                }
            }
        });
        if n > 12 {
            ui.add(
                egui::TextEdit::singleline(&mut self.list.points_filter)
                    .hint_text("filter points")
                    .desired_width(f32::INFINITY),
            );
        }
        let filter = self.list.points_filter.clone();
        let rows: Vec<usize> = (0..n)
            .filter(|i| filter.trim().is_empty() || model::matches(&filter, &self.sources[*i].tag))
            .collect();
        let row_h = ui.spacing().interact_size.y;
        let mut colours: HashMap<usize, Color32> = HashMap::new();
        for (_, name, si, _, idx) in self.instances() {
            colours.entry(si).or_insert_with(|| {
                let s = self.style_of(&name, self.sources[si].id, idx);
                Color32::from_rgb(s.color[0], s.color[1], s.color[2])
            });
        }
        egui::ScrollArea::vertical()
            .id_salt(("wave-point-rows", self.id))
            .max_height(170.0)
            .auto_shrink([false, true])
            .show_rows(ui, row_h, rows.len(), |ui, range| {
                for &i in &rows[range] {
                    let colour = colours.get(&i).copied();
                    let s = &mut self.sources[i];
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut s.visible, "");
                        let mut text = RichText::new(model::shorten(&s.tag, 28));
                        if let (Some(c), true) = (colour, s.visible) {
                            text = text.color(c);
                        }
                        let path = s
                            .path
                            .as_ref()
                            .map(|p| p.display().to_string())
                            .unwrap_or_else(|| "(calculator results)".into());
                        ui.label(text).on_hover_text(format!("{}\n{path}", s.tag));
                        match &s.state {
                            SourceState::Loading(p, _) => {
                                ui.weak(match p.phase() {
                                    Phase::Convert => {
                                        format!("indexing {:.0}%", p.fraction() * 100.0)
                                    }
                                    Phase::Wait => "waiting".into(),
                                    _ => "loading".into(),
                                });
                            }
                            SourceState::Failed(e) => {
                                ui.colored_label(ui.visuals().error_fg_color, "failed")
                                    .on_hover_text(e);
                            }
                            SourceState::Ready(r) => {
                                ui.weak(format!("{} pts", r.points));
                            }
                            SourceState::Memory => {
                                ui.weak(format!("{} series", s.extras.len()));
                            }
                        }
                    });
                }
            });
    }

    fn readout(&self, ui: &mut Ui) {
        let any = self
            .groups
            .iter()
            .any(|g| g.cursors.iter().any(Option::is_some));
        if !any {
            ui.weak(
                "Click: cursor A · right click: cursor B · wheel: zoom · drag: pan · double click: fit · drag signals onto a pane",
            );
            return;
        }
        let Some(reply) = &self.reply else { return };
        type Row = (String, Color32, String, Option<f64>, Option<f64>);
        let mut rows: Vec<Row> = Vec::new();
        let insts = self.instances();
        for g in &self.groups {
            if g.cursors.iter().all(Option::is_none) {
                continue;
            }
            let xs = |x: Option<f64>| x.map_or("–".to_string(), |x| units::si(x, &g.unit, 6));
            ui.horizontal_wrapped(|ui| {
                if self.groups.len() > 1 {
                    ui.strong(if g.unit.is_empty() {
                        "x"
                    } else {
                        g.unit.as_str()
                    });
                }
                ui.label(format!("A: {}", xs(g.cursors[0])));
                ui.label(format!("B: {}", xs(g.cursors[1])));
                if let [Some(a), Some(b)] = g.cursors {
                    let dx = b - a;
                    ui.label(format!("B−A: {}", units::si(dx, &g.unit, 6)));
                    if dx != 0.0 && g.unit == "s" {
                        ui.label(format!("1/(B−A): {}", units::si(1.0 / dx.abs(), "Hz", 5)));
                    }
                }
            });
            for p in &reply.panes {
                if self
                    .panes
                    .get(p.pane)
                    .is_none_or(|pane| pane.x_unit != g.unit)
                {
                    continue;
                }
                for a in &p.insts {
                    let Some(si) = self.sources.iter().position(|s| s.id == a.key.source) else {
                        continue;
                    };
                    let name = self.sig_name(si, a.key.sig);
                    let idx = insts
                        .iter()
                        .find(|(_, n, s, _, _)| *n == name && *s == si)
                        .map_or(0, |x| x.4);
                    let st = self.style_of(&name, a.key.source, idx);
                    let unit = self
                        .entry(&name)
                        .map(|e| e.unit.clone())
                        .unwrap_or_default();
                    rows.push((
                        self.label_of(&name, si),
                        Color32::from_rgb(st.color[0], st.color[1], st.color[2]),
                        unit,
                        a.at_cursor[0],
                        a.at_cursor[1],
                    ));
                }
            }
        }
        let row_h = ui.spacing().interact_size.y;
        let w = ui.available_width();
        let cols = [w * 0.4, w * 0.2, w * 0.2, w * 0.2];
        ui.horizontal(|ui| {
            for (t, cw) in ["Signal", "A", "B", "B−A"].iter().zip(cols) {
                ui.add_sized(
                    [(cw - 8.0).max(10.0), row_h],
                    egui::Label::new(RichText::new(*t).strong()),
                );
            }
        });
        egui::ScrollArea::vertical()
            .id_salt(("wave-readout-rows", self.id))
            .auto_shrink([false, false])
            .show_rows(ui, row_h, rows.len(), |ui, range| {
                for (label, c, unit, a, b) in &rows[range] {
                    let v = |x: Option<f64>| x.map_or("–".to_string(), |x| units::si(x, unit, 6));
                    ui.horizontal(|ui| {
                        ui.add_sized(
                            [(cols[0] - 8.0).max(10.0), row_h],
                            egui::Label::new(RichText::new(model::shorten(label, 60)).color(*c))
                                .truncate(),
                        );
                        ui.add_sized([(cols[1] - 8.0).max(10.0), row_h], egui::Label::new(v(*a)));
                        ui.add_sized([(cols[2] - 8.0).max(10.0), row_h], egui::Label::new(v(*b)));
                        // As UberWave's measurement: a difference only of values in the same unit.
                        let d = match (a, b) {
                            (Some(a), Some(b)) => units::si(b - a, unit, 6),
                            _ => "–".into(),
                        };
                        ui.add_sized([(cols[3] - 8.0).max(10.0), row_h], egui::Label::new(d));
                    });
                }
            });
    }

    /// The legend of a pane, above it: its lines (the first [`LEGEND_MAX`]), each with
    /// a menu to change its style or move it.
    fn legend(&mut self, ui: &mut Ui, pane: PaneId) {
        let insts: Vec<Inst> = self
            .instances()
            .into_iter()
            .filter(|(p, _, si, _, _)| *p == pane && self.sources[*si].visible)
            .collect();
        let pane_ids: Vec<PaneId> = self.panes.list.iter().map(|p| p.id).collect();
        let many = self.sources.len() > 1;
        let mut action: Option<LegendAction> = None;
        egui::ScrollArea::vertical()
            .id_salt(("wave-legend", self.id, pane))
            .max_height(ui.spacing().interact_size.y * 2.2)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if insts.is_empty() {
                        ui.weak("empty: drag signals here, or check them in the list");
                    }
                    for (_, name, si, _, idx) in insts.iter().take(LEGEND_MAX) {
                        let sid = self.sources[*si].id;
                        let st = self.style_of(name, sid, *idx);
                        let full = self.label_of(name, *si);
                        let r = legend_entry(ui, &st, &model::shorten(&full, 48));
                        let r = r.on_hover_text(format!(
                            "{full}\nright click: colour, dash, width, pane; drag: to another pane"
                        ));
                        if r.dragged() {
                            r.dnd_set_drag_payload(DragSignals(vec![name.clone()]));
                        }
                        r.context_menu(|ui| {
                            let mut st2 = st;
                            ui.horizontal(|ui| {
                                ui.label("colour");
                                egui::color_picker::color_edit_button_srgb(ui, &mut st2.color);
                            });
                            egui::ComboBox::from_label("dash")
                                .selected_text(st2.dash.name())
                                .show_ui(ui, |ui| {
                                    for d in Dash::ALL {
                                        ui.selectable_value(&mut st2.dash, d, d.name());
                                    }
                                });
                            ui.add(egui::Slider::new(&mut st2.width, 0.5..=6.0).text("width"));
                            if st2 != st {
                                action = Some(LegendAction::Style(name.clone(), sid, st2));
                            }
                            if ui.button("Default style").clicked() {
                                action = Some(LegendAction::ResetStyle(name.clone(), sid));
                                ui.close();
                            }
                            ui.separator();
                            ui.menu_button("Move to pane", |ui| {
                                for (k, id) in pane_ids.iter().enumerate() {
                                    if ui.button(format!("Pane {}", k + 1)).clicked() {
                                        action = Some(LegendAction::Move(
                                            name.clone(),
                                            Target::Pane(*id),
                                        ));
                                        ui.close();
                                    }
                                }
                                if ui.button("New pane").clicked() {
                                    action = Some(LegendAction::Move(name.clone(), Target::New));
                                    ui.close();
                                }
                            });
                            if ui.button("Hide signal").clicked() {
                                action = Some(LegendAction::Hide(name.clone()));
                                ui.close();
                            }
                            if many && ui.button("Hide this point").clicked() {
                                action = Some(LegendAction::HidePoint(sid));
                                ui.close();
                            }
                        });
                    }
                    if insts.len() > LEGEND_MAX {
                        ui.weak(format!("+{} more (see Points)", insts.len() - LEGEND_MAX));
                    }
                });
            });
        match action {
            Some(LegendAction::Style(n, s, st)) => {
                self.styles.insert((n, s), st);
            }
            Some(LegendAction::ResetStyle(n, s)) => {
                self.styles.remove(&(n, s));
            }
            Some(LegendAction::Move(n, t)) => self.show(&[n], t),
            Some(LegendAction::Hide(n)) => self.hide(&[n]),
            Some(LegendAction::HidePoint(s)) => {
                if let Some(src) = self.sources.iter_mut().find(|x| x.id == s) {
                    src.visible = false;
                }
            }
            None => {}
        }
    }

    fn panes_ui(&mut self, ui: &mut Ui) {
        let n = self.panes.list.len();
        let avail = ui.available_height();
        let pane_h = (avail / n as f32).max(PANE_MIN_H);
        self.pane_px.clear();
        self.plot_rects.clear();
        let ids: Vec<PaneId> = self.panes.list.iter().map(|p| p.id).collect();
        let total = pane_h * n as f32;
        egui::ScrollArea::vertical()
            .id_salt(("wave-panes", self.id))
            .auto_shrink([false, false])
            .show_viewport(ui, |ui, viewport| {
                ui.set_height(total);
                let origin = ui.min_rect().min;
                let width = ui.available_width();
                for (k, id) in ids.iter().enumerate() {
                    let y0 = k as f32 * pane_h;
                    if y0 + pane_h < viewport.min.y || y0 > viewport.max.y {
                        continue;
                    }
                    let rect = Rect::from_min_size(
                        origin + egui::vec2(0.0, y0),
                        egui::vec2(width, pane_h),
                    );
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).id_salt((
                        "wave-pane",
                        self.id,
                        *id,
                    )));
                    // the x axis under the last pane of each run of panes of one x unit
                    let axis =
                        k + 1 == n || self.panes.list[k + 1].x_unit != self.panes.list[k].x_unit;
                    self.pane_ui(&mut child, *id, axis);
                }
            });
    }

    fn pane_ui(&mut self, ui: &mut Ui, id: PaneId, last: bool) {
        let number = self.panes.number(id).unwrap_or(0);
        let active = self.panes.active == id;
        let unit = self
            .panes
            .get(id)
            .map(|p| p.x_unit.clone())
            .unwrap_or_default();
        let stroke = if active {
            egui::Stroke::new(1.0, ui.visuals().selection.stroke.color)
        } else {
            egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color)
        };
        egui::Frame::new()
            .stroke(stroke)
            .inner_margin(2.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let title = if unit.is_empty() {
                        format!("Pane {number}")
                    } else {
                        format!("Pane {number} · x in {unit}")
                    };
                    if ui
                        .add(egui::Button::new(RichText::new(title).small()).frame(false))
                        .on_hover_text("The active pane takes the signals checked in the list")
                        .clicked()
                    {
                        self.panes.active = id;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .small_button("✕")
                            .on_hover_text("Remove this pane")
                            .clicked()
                        {
                            self.panes.remove_pane(id);
                        }
                        if ui.small_button("⏷").on_hover_text("Move down").clicked() {
                            self.panes.move_pane(id, 1);
                        }
                        if ui.small_button("⏶").on_hover_text("Move up").clicked() {
                            self.panes.move_pane(id, -1);
                        }
                        if self.panes.get(id).is_some_and(|p| p.y_manual.is_some())
                            && ui
                                .small_button("Fit y")
                                .on_hover_text(
                                    "Fit the y range to what is drawn (Ctrl+wheel chose it)",
                                )
                                .clicked()
                            && let Some(p) = self.panes.get_mut(id)
                        {
                            p.y_manual = None;
                        }
                    });
                });
                if self.panes.get(id).is_none() {
                    return;
                }
                self.legend(ui, id);
                self.plot(ui, id, last);
            });
    }

    fn plot(&mut self, ui: &mut Ui, id: PaneId, last: bool) {
        let Some(pane) = self.panes.get(id).cloned() else {
            return;
        };
        let Some(gi) = self.group_of(&pane.x_unit) else {
            return;
        };
        let g = self.groups[gi].clone();
        let (u0, u1) = g.plot_view();
        let reply_pane = self.reply.as_ref().and_then(|r| r.pane(id));
        let (y0, y1) = pane
            .y_manual
            .or_else(|| reply_pane.map(|p| (p.y0, p.y1)))
            .unwrap_or((-1.0, 1.0));
        let image = reply_pane.and_then(|p| {
            let t = self.textures.get(&id)?;
            Some((t.id(), p.u0, p.u1, p.y0, p.y1))
        });
        let y_unit = {
            let units: Vec<String> = pane
                .signals
                .iter()
                .filter_map(|s| self.entry(s).map(|e| e.unit.clone()))
                .collect();
            match units.first() {
                Some(u) if units.iter().all(|x| x == u) => u.clone(),
                _ => String::new(),
            }
        };
        let x_name = self
            .sources
            .iter()
            .find_map(|s| match &s.state {
                SourceState::Ready(r) if r.x_unit == g.unit => Some(r.x_name.clone()),
                _ => None,
            })
            .unwrap_or_else(|| {
                if g.unit == "s" {
                    "time".into()
                } else if g.unit == "Hz" {
                    "frequency".into()
                } else {
                    "x".into()
                }
            });
        let cursor_color = ui.visuals().strong_text_color();
        let markers: Vec<Marker> = if self.show_markers {
            self.markers
                .iter()
                .filter(|m| m.x_unit.is_empty() || model::norm_unit(&m.x_unit) == g.unit)
                .cloned()
                .collect()
        } else {
            vec![]
        };
        let marker_color = Color32::from_rgb(230, 120, 255);
        let (xu, yu, log) = (g.unit.clone(), y_unit.clone(), g.log);
        let height = ui.available_height().max(40.0);
        // about one labelled y tick per 30 points
        let y_div = ((height - if last { 40.0 } else { 0.0 }) / 30.0).clamp(1.5, 6.0) as f64;
        let mut plot = Plot::new(("wave-plot", self.id, id))
            .height(height)
            .allow_drag(false)
            .allow_zoom(false)
            .allow_scroll(false)
            .allow_boxed_zoom(false)
            .allow_double_click_reset(false)
            .show_x(false)
            .show_y(false)
            .y_axis_min_width(Y_AXIS_W)
            .show_axes([last, true])
            .x_grid_spacer(move |input| {
                if log {
                    units::log_grid(input.bounds)
                } else {
                    units::grid(input.bounds, 8.0)
                }
            })
            .y_grid_spacer(move |input| units::grid(input.bounds, y_div))
            .x_axis_formatter(move |mark, range| {
                if log {
                    units::si(10f64.powf(mark.value), &xu, 3)
                } else {
                    units::tick(mark.value, range.end() - range.start(), mark.step_size, &xu)
                }
            })
            .y_axis_formatter(move |mark, range| {
                units::tick(mark.value, range.end() - range.start(), mark.step_size, &yu)
            });
        if last {
            plot = plot.x_axis_label(units::x_label(&x_name, &g.unit));
        }
        let response = plot.show(ui, |pui| {
            pui.set_plot_bounds(PlotBounds::from_min_max([u0, y0], [u1, y1]));
            if let Some((tex, a, b, c, d)) = image {
                pui.image(PlotImage::new(
                    "",
                    tex,
                    PlotPoint::new((a + b) / 2.0, (c + d) / 2.0),
                    egui::vec2((b - a) as f32, (d - c) as f32),
                ));
            }
            for (i, c) in g.cursors.iter().enumerate() {
                if let Some(x) = c {
                    // A solid, B dashed.
                    pui.vline(
                        VLine::new("", g.to_plot(*x))
                            .color(cursor_color)
                            .style(if i == 0 {
                                egui_plot::LineStyle::Solid
                            } else {
                                egui_plot::LineStyle::dashed_dense()
                            })
                            .width(1.0),
                    );
                }
            }
            let mut markers = markers;
            markers.sort_by(|a, b| a.x.total_cmp(&b.x));
            for (k, m) in markers.iter().enumerate() {
                let u = g.to_plot(m.x);
                if !u.is_finite() {
                    continue;
                }
                pui.vline(
                    VLine::new("", u)
                        .color(marker_color)
                        .style(egui_plot::LineStyle::dashed_loose())
                        .width(1.0),
                );
                pui.text(
                    Text::new(
                        "",
                        // neighbours' labels on rows of their own
                        PlotPoint::new(u, y1 - (y1 - y0) * (0.02 + 0.09 * (k % 4) as f64)),
                        model::shorten(&m.label, 60),
                    )
                    .color(marker_color)
                    .anchor(egui::Align2::LEFT_TOP),
                );
                if let Some(y) = m.y.filter(|y| y.is_finite()) {
                    pui.points(
                        Points::new("", vec![[u, y]])
                            .radius(3.0)
                            .color(marker_color),
                    );
                }
            }
        });
        let frame = *response.transform.frame();
        self.plot_rects.insert(id, frame);
        let ppp = ui.ctx().pixels_per_point();
        self.pane_px.insert(
            id,
            (
                (frame.width() * ppp).round().clamp(1.0, 16384.0) as u32,
                (frame.height() * ppp).round().clamp(1.0, 16384.0) as u32,
            ),
        );
        if let Some(p) = response.response.dnd_release_payload::<DragSignals>() {
            self.show(&p.0, Target::Pane(id));
            self.panes.active = id;
        } else if response
            .response
            .dnd_hover_payload::<DragSignals>()
            .is_some()
        {
            ui.painter().rect_stroke(
                frame,
                2.0,
                egui::Stroke::new(2.0, ui.visuals().selection.stroke.color),
                egui::StrokeKind::Inside,
            );
        }
        self.interact(ui, &response.response, frame, id, gi, (y0, y1));
    }

    /// Mouse on a pane (see the module documentation).
    fn interact(
        &mut self,
        ui: &Ui,
        response: &egui::Response,
        frame: Rect,
        pane: PaneId,
        gi: usize,
        y: (f64, f64),
    ) {
        let (u0, u1) = self.groups[gi].plot_view();
        let width = (frame.width() as f64).max(1.0);
        let to_u = |px: f32| u0 + (px - frame.left()) as f64 / width * (u1 - u0);
        if response.hovered() {
            let (mut notches, mut mods) = (0.0f32, egui::Modifiers::NONE);
            ui.input(|i| {
                for e in &i.events {
                    if let Event::MouseWheel {
                        unit,
                        delta,
                        modifiers,
                        ..
                    } = e
                    {
                        let d = if delta.y != 0.0 { delta.y } else { delta.x };
                        notches += match unit {
                            MouseWheelUnit::Line => d,
                            MouseWheelUnit::Page => d * 10.0,
                            MouseWheelUnit::Point => d / 40.0,
                        };
                        mods = *modifiers;
                    }
                }
            });
            if notches != 0.0
                && let Some(pos) = response.hover_pos()
            {
                let factor = 0.8f64.powf(notches as f64);
                if mods.shift {
                    self.groups[gi].pan(0.1 * -(notches as f64));
                } else if mods.ctrl || mods.command {
                    let (y0, y1) = y;
                    let height = (frame.height() as f64).max(1.0);
                    let yc = y1 - (pos.y - frame.top()) as f64 / height * (y1 - y0);
                    if let Some(p) = self.panes.get_mut(pane) {
                        p.y_manual = Some((yc - (yc - y0) * factor, yc + (y1 - yc) * factor));
                    }
                } else {
                    let g = &mut self.groups[gi];
                    let x = g.data_x(to_u(pos.x));
                    g.zoom_at(x, factor);
                }
            }
        }
        if response.dragged_by(PointerButton::Primary) {
            let d = response.drag_delta();
            self.groups[gi].pan(-(d.x as f64) / width);
            if let Some(p) = self.panes.get_mut(pane)
                && let Some((y0, y1)) = p.y_manual
            {
                let dy = d.y as f64 / (frame.height() as f64).max(1.0) * (y1 - y0);
                p.y_manual = Some((y0 + dy, y1 + dy));
            }
        }
        let g = &mut self.groups[gi];
        if response.double_clicked() {
            g.fit();
            if let Some(p) = self.panes.get_mut(pane) {
                p.y_manual = None;
            }
        } else if response.clicked() {
            if let Some(pos) = response.interact_pointer_pos() {
                g.cursors[0] = Some(g.data_x(to_u(pos.x)));
            }
            self.panes.active = pane;
        } else if response.secondary_clicked()
            && let Some(pos) = response.interact_pointer_pos()
        {
            g.cursors[1] = Some(g.data_x(to_u(pos.x)));
            self.panes.active = pane;
        }
    }
}

enum LegendAction {
    Style(String, SourceId, Style),
    ResetStyle(String, SourceId),
    Move(String, Target),
    Hide(String),
    HidePoint(SourceId),
}

/// A legend entry: a short line in the trace's style and the label.
fn legend_entry(ui: &mut Ui, st: &Style, label: &str) -> egui::Response {
    let font = egui::TextStyle::Small.resolve(ui.style());
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_string(), font, ui.visuals().text_color());
    let size = egui::vec2(22.0 + galley.size().x + 6.0, galley.size().y.max(12.0));
    let (rect, r) = ui.allocate_exact_size(size, Sense::click_and_drag());
    if r.hovered() {
        ui.painter()
            .rect_filled(rect, 2.0, ui.visuals().widgets.hovered.weak_bg_fill);
    }
    let c = Color32::from_rgb(st.color[0], st.color[1], st.color[2]);
    let y = rect.center().y;
    let stroke = egui::Stroke::new(st.width.min(4.0), c);
    let (x0, x1) = (rect.left() + 1.0, rect.left() + 19.0);
    let pattern: &[f32] = match st.dash {
        Dash::Solid => &[18.0, 0.0],
        Dash::Dashed => &[5.0, 3.0],
        Dash::Dotted => &[1.5, 2.5],
        Dash::DashDot => &[5.0, 2.0, 1.5, 2.0],
    };
    let mut x = x0;
    let mut k = 0;
    while x < x1 {
        let l = pattern[k % pattern.len()];
        if k % 2 == 0 && l > 0.0 {
            ui.painter()
                .line_segment([egui::pos2(x, y), egui::pos2((x + l).min(x1), y)], stroke);
        }
        x += l.max(0.5);
        k += 1;
    }
    ui.painter().galley(
        egui::pos2(rect.left() + 22.0, rect.top()),
        galley,
        ui.visuals().text_color(),
    );
    r
}

fn ready_of(ds: Dataset, took: Duration) -> Ready {
    let n = ds.signal_count();
    let signals: Vec<(String, String)> = (1..n)
        .map(|i| (ds.signal_name(i), ds.signal_unit(i)))
        .collect();
    let mut index = HashMap::new();
    for (i, (name, _)) in signals.iter().enumerate() {
        // the first of two signals of one name wins
        index.entry(name.clone()).or_insert(i as u32 + 1);
    }
    let unit = ds.text(cdept_wavedata::Text::XUnit);
    let x_name = ds.text(cdept_wavedata::Text::XName);
    let x_unit = model::norm_unit(if unit.is_empty() { &x_name } else { &unit });
    Ready {
        full: ds
            .x_range()
            .ok()
            .filter(|(a, b)| a.is_finite() && b.is_finite()),
        x_log: ds.x_log(),
        points: ds.point_count(),
        cache: ds.text(cdept_wavedata::Text::Cache),
        converted: ds.converted(),
        open_time: took,
        x_unit,
        x_name,
        signals,
        index,
        ds: Arc::new(ds),
    }
}

impl View for WaveView {
    fn title(&self, _ctx: &AppCtx) -> String {
        let title = model::shorten(&self.title, 60);
        if self.sources.is_empty() {
            return title;
        }
        if self.ready() == 0 && self.loading() > 0 {
            format!("{title} (loading)")
        } else if self.ready() == 0
            && self.failed() > 0
            && self.sources.iter().all(|s| s.path.is_some())
        {
            format!("{title} (failed)")
        } else {
            title
        }
    }

    fn ui(&mut self, ui: &mut Ui, _ctx: &mut AppCtx) {
        let started = Instant::now();
        self.tick(ui.ctx());
        self.dark = ui.visuals().dark_mode;
        let now = Instant::now();
        let viewport = ui.ctx().viewport_id();
        let pass = ui.ctx().cumulative_pass_nr();
        if let Some((last, vp, n)) = self.stats.last_frame
            && vp == viewport
            && n + 1 == pass
        {
            Stats::push(&mut self.stats.frames, (now - last).as_secs_f32() * 1e3);
        }
        self.stats.last_frame = Some((now, viewport, pass));
        // One file that is loading or failed: the whole tab says so.
        if self.sources.len() == 1 && self.sources[0].extras.is_empty() {
            let mut reload = false;
            match &self.sources[0].state {
                SourceState::Loading(p, t) => {
                    let (p, t) = (p.clone(), *t);
                    loading_ui(ui, &self.title, self.sources[0].path.as_deref(), &p, t);
                    return;
                }
                SourceState::Failed(e) => {
                    let e = e.clone();
                    ui.vertical_centered(|ui| {
                        ui.add_space(40.0);
                        ui.heading(model::shorten(&self.title, 80));
                        if let Some(p) = &self.sources[0].path {
                            ui.weak(p.display().to_string());
                        }
                        ui.add_space(12.0);
                        ui.colored_label(ui.visuals().error_fg_color, e);
                        if ui.button("Try Again").clicked() {
                            reload = true;
                        }
                    });
                    if reload {
                        self.reload_all(false);
                    }
                    return;
                }
                _ => {}
            }
        }
        self.toolbar(ui);
        ui.separator();
        let id = self.id;
        egui::Panel::left(egui::Id::new(("wave-signals", id)))
            .resizable(true)
            .default_size(220.0)
            .show(ui, |ui| {
                if self.sources.len() > 1 || self.failed() > 0 {
                    self.points_list(ui);
                    ui.separator();
                }
                self.signal_list(ui);
            });
        egui::Panel::bottom(egui::Id::new(("wave-readout", id)))
            .resizable(true)
            .default_size(150.0)
            .show(ui, |ui| self.readout(ui));
        egui::CentralPanel::default().show(ui, |ui| self.panes_ui(ui));
        self.refresh_groups();
        self.request();
        self.publish_cursors();
        if self.reply.as_ref().is_some_and(Reply::building) || self.loading() > 0 {
            ui.ctx().request_repaint_after(Duration::from_millis(300));
        }
        Stats::push(&mut self.stats.ui_ms, started.elapsed().as_secs_f32() * 1e3);
    }
}

fn loading_ui(
    ui: &mut Ui,
    title: &str,
    path: Option<&std::path::Path>,
    p: &LoadProgress,
    started: Instant,
) {
    ui.vertical_centered(|ui| {
        ui.add_space(40.0);
        ui.heading(model::shorten(title, 80));
        if let Some(path) = path {
            ui.weak(path.display().to_string());
        }
        ui.add_space(12.0);
        let (text, fraction) = match p.phase() {
            Phase::Header => ("Reading the header…".to_string(), None),
            Phase::Wait => (
                "Waiting for another tab that indexes this file…".to_string(),
                None,
            ),
            Phase::Convert => (
                format!(
                    "Indexing the file (writing its .uwraw cache) {:.0}%",
                    p.fraction() * 100.0
                ),
                Some(p.fraction()),
            ),
            Phase::Open => ("Opening…".to_string(), None),
        };
        ui.label(text);
        let bar = egui::ProgressBar::new(fraction.unwrap_or(0.0)).desired_width(320.0);
        ui.add(if fraction.is_some() {
            bar
        } else {
            bar.animate(true)
        });
        ui.weak(format!("{:.1} s", started.elapsed().as_secs_f64()));
        if p.cancelled() {
            ui.weak("Cancelling…");
        } else if ui.button("Cancel").clicked() {
            p.cancel();
        }
    });
    ui.ctx().request_repaint_after(Duration::from_millis(100));
}

// --- script (`wv ...`, script.rs) ---

impl WaveView {
    fn only_ready(&self) -> Result<&Ready, String> {
        let s = self.sources.first().ok_or("the tab has no file")?;
        match &s.state {
            SourceState::Ready(r) => Ok(r),
            SourceState::Loading(..) => Err("the file is still loading".into()),
            SourceState::Failed(e) => Err(format!("the file did not open: {e}")),
            SourceState::Memory => Err("the tab has no file".into()),
        }
    }

    /// Ok when every file is open (failed ones count as done: `expect_failed`).
    pub(crate) fn script_state(&self) -> Result<(), String> {
        if let Some(s) = self
            .sources
            .iter()
            .find(|s| matches!(s.state, SourceState::Loading(..)))
            && let SourceState::Loading(p, _) = &s.state
        {
            return Err(format!(
                "{} still loading ({:?} {:.0}%)",
                self.loading(),
                p.phase(),
                p.fraction() * 100.0
            ));
        }
        if self.sources.len() == 1
            && let SourceState::Failed(e) = &self.sources[0].state
        {
            return Err(format!("failed: {e}"));
        }
        Ok(())
    }

    pub(crate) fn script_failed(&self) -> Option<&str> {
        match self.sources.first().map(|s| &s.state) {
            Some(SourceState::Failed(e)) if self.sources.len() == 1 => Some(e),
            _ => None,
        }
    }

    pub(crate) fn script_loading(&self) -> bool {
        self.loading() > 0
    }

    /// The cache being read (the first file), and whether its source changed (asked
    /// now).
    pub(crate) fn script_cache(&mut self) -> Result<(String, bool), String> {
        let r = self.only_ready()?;
        Ok((r.cache.clone(), r.ds.source_changed()))
    }

    pub(crate) fn script_reload(&mut self, _ctx: &egui::Context) {
        self.reload_all(false);
    }

    pub(crate) fn script_cancel(&mut self) {
        for s in &self.sources {
            if let SourceState::Loading(p, _) = &s.state {
                p.cancel();
            }
        }
    }

    fn known(&mut self, name: &str) -> Result<String, String> {
        self.refresh_catalog();
        if let Some(e) = self.catalog.iter().find(|e| e.name == name) {
            return Ok(e.name.clone());
        }
        self.catalog
            .iter()
            .find(|e| e.name.eq_ignore_ascii_case(name))
            .map(|e| e.name.clone())
            .ok_or_else(|| format!("no signal '{name}'"))
    }

    fn script_state_ready(&self) -> Result<(), String> {
        if self.ready() == 0 && self.sources.iter().all(|s| s.extras.is_empty()) {
            return Err(match self.sources.first().map(|s| &s.state) {
                Some(SourceState::Loading(..)) => "the file is still loading".into(),
                Some(SourceState::Failed(e)) => format!("the file did not open: {e}"),
                _ => "nothing is open".into(),
            });
        }
        Ok(())
    }

    pub(crate) fn script_show(&mut self, names: &[String], show: bool) -> Result<(), String> {
        self.script_state_ready()?;
        let names: Vec<String> = names
            .iter()
            .map(|n| self.known(n))
            .collect::<Result<_, _>>()?;
        if show {
            self.show(&names, Target::Active);
        } else {
            self.hide(&names);
        }
        Ok(())
    }

    /// Show in pane `n` (1-based), or `new`.
    pub(crate) fn script_show_in(&mut self, pane: &str, names: &[String]) -> Result<(), String> {
        self.script_state_ready()?;
        let target = self.pane_target(pane)?;
        let names: Vec<String> = names
            .iter()
            .map(|n| self.known(n))
            .collect::<Result<_, _>>()?;
        self.show(&names, target);
        Ok(())
    }

    fn pane_target(&self, pane: &str) -> Result<Target, String> {
        if pane == "new" {
            return Ok(Target::New);
        }
        let n: usize = pane
            .parse()
            .map_err(|_| format!("'{pane}' is not a pane (a number or new)"))?;
        self.panes
            .by_number(n)
            .map(Target::Pane)
            .ok_or_else(|| format!("there are {} panes", self.panes.list.len()))
    }

    /// `wv pane new|remove|up|down|select|fit_y <n>`.
    pub(crate) fn script_pane(&mut self, verb: &str, n: Option<&str>) -> Result<(), String> {
        let id = || -> Result<PaneId, String> {
            let n: usize = n
                .ok_or("which pane?")?
                .parse()
                .map_err(|_| "not a pane number".to_string())?;
            self.panes
                .by_number(n)
                .ok_or_else(|| format!("there are {} panes", self.panes.list.len()))
        };
        match verb {
            "new" => {
                self.panes.add_pane();
            }
            "remove" => {
                let id = id()?;
                self.panes.remove_pane(id);
            }
            "up" => {
                let id = id()?;
                self.panes.move_pane(id, -1);
            }
            "down" => {
                let id = id()?;
                self.panes.move_pane(id, 1);
            }
            "select" => {
                self.panes.active = id()?;
            }
            "fit_y" => {
                let id = id()?;
                if let Some(p) = self.panes.get_mut(id) {
                    p.y_manual = None;
                }
            }
            _ => return Err(format!("'{verb}' is not a pane command")),
        }
        self.refresh_groups();
        Ok(())
    }

    /// Signals and x unit of each pane, top to bottom.
    pub(crate) fn script_panes(&self) -> Vec<(Vec<String>, String)> {
        self.panes
            .list
            .iter()
            .map(|p| (p.signals.clone(), p.x_unit.clone()))
            .collect()
    }

    pub(crate) fn script_set_y(&mut self, n: usize, y: Option<(f64, f64)>) -> Result<(), String> {
        let id = self.panes.by_number(n).ok_or("no such pane")?;
        self.panes.get_mut(id).ok_or("no such pane")?.y_manual = y;
        Ok(())
    }

    /// The y range on screen of pane `n`.
    pub(crate) fn script_y(&self, n: usize) -> Result<(f64, f64), String> {
        let id = self.panes.by_number(n).ok_or("no such pane")?;
        let p = self.panes.get(id).ok_or("no such pane")?;
        if let Some(y) = p.y_manual {
            return Ok(y);
        }
        if !self.up_to_date() {
            return Err("the data on screen is not up to date yet".into());
        }
        self.reply
            .as_ref()
            .and_then(|r| r.pane(id))
            .map(|p| (p.y0, p.y1))
            .ok_or_else(|| "the pane has not been drawn yet".into())
    }

    fn active_group(&self) -> Result<usize, String> {
        let unit = self
            .panes
            .get(self.panes.active)
            .map(|p| p.x_unit.clone())
            .unwrap_or_default();
        self.group_of(&unit)
            .or(if self.groups.is_empty() {
                None
            } else {
                Some(0)
            })
            .ok_or_else(|| "nothing is shown yet".into())
    }

    pub(crate) fn script_fit(&mut self) -> Result<(), String> {
        self.script_state_ready()?;
        self.fit_all();
        Ok(())
    }

    pub(crate) fn script_zoom(&mut self, x0: f64, x1: f64) -> Result<(), String> {
        self.script_state_ready()?;
        let g = self.active_group()?;
        self.groups[g].set_view(x0.min(x1), x0.max(x1));
        Ok(())
    }

    pub(crate) fn script_cursor(&mut self, which: usize, x: Option<f64>) -> Result<(), String> {
        self.script_state_ready()?;
        let g = self.active_group()?;
        self.groups[g].cursors[which] = x;
        Ok(())
    }

    /// The active pane's plot rectangle on screen (points), for mouse input.
    pub(crate) fn script_plot_rect(&mut self) -> Result<Rect, String> {
        self.script_state_ready()?;
        self.plot_rects
            .get(&self.panes.active)
            .copied()
            .ok_or_else(|| "the plot has not been drawn yet".into())
    }

    /// Where a signal's row of the list is on screen.
    pub(crate) fn script_row_rect(&mut self, name: &str) -> Result<Rect, String> {
        let name = self.known(name)?;
        self.list
            .row_rects
            .get(&name)
            .copied()
            .ok_or_else(|| format!("'{name}' is not on screen in the list"))
    }

    /// Where pane `n`'s plot is on screen.
    pub(crate) fn script_pane_rect(&self, n: usize) -> Result<Rect, String> {
        let id = self.panes.by_number(n).ok_or("no such pane")?;
        self.plot_rects
            .get(&id)
            .copied()
            .ok_or_else(|| "the pane is not on screen".into())
    }

    /// The x range on screen (of the active pane's group).
    pub(crate) fn script_range(&mut self) -> Result<(f64, f64), String> {
        self.script_state_ready()?;
        let g = self.active_group()?;
        Ok(self.groups[g].view)
    }

    /// The answer of a signal, when what is on screen is up to date: `tag` picks the
    /// source in a tab of several (else the first that has it).
    fn answer_of(&mut self, signal: &str, tag: Option<&str>) -> Result<worker::InstAnswer, String> {
        let name = self.known(signal)?;
        if !self.up_to_date() {
            return Err("the data on screen is not up to date yet".into());
        }
        let insts = self.instances();
        let (_, _, si, sig, _) = insts
            .iter()
            .find(|(_, n, si, _, _)| {
                *n == name
                    && self.sources[*si].visible
                    && tag.is_none_or(|t| self.sources[*si].tag == t)
            })
            .ok_or_else(|| format!("'{signal}' is not shown"))?;
        let key = InstKey {
            source: self.sources[*si].id,
            sig: *sig,
        };
        self.reply
            .as_ref()
            .and_then(|r| r.inst(key))
            .cloned()
            .ok_or_else(|| format!("'{signal}' is not drawn"))
    }

    /// The largest and smallest value drawn for `signal` inside the x range on screen.
    pub(crate) fn script_drawn_extremes(
        &mut self,
        signal: &str,
    ) -> Result<(f64, f64, usize), String> {
        let a = self.answer_of(signal, None)?;
        let (lo, hi) = a.drawn.unwrap_or((f64::INFINITY, f64::NEG_INFINITY));
        Ok((lo, hi, a.points))
    }

    /// The exact extremes of `signal` over the x range on screen, read from every
    /// sample (on this thread: for checks only).
    pub(crate) fn script_exact_extremes(
        &mut self,
        signal: &str,
    ) -> Result<(f64, f64, u64), String> {
        let name = self.known(signal)?;
        let insts = self.instances();
        let (pane, _, si, sig, _) = insts
            .iter()
            .find(|(_, n, si, _, _)| *n == name && self.sources[*si].visible)
            .cloned()
            .ok_or_else(|| format!("'{signal}' is not shown"))?;
        let unit = self
            .panes
            .get(pane)
            .map(|p| p.x_unit.clone())
            .unwrap_or_default();
        let g = self.group_of(&unit).ok_or("no x group")?;
        let (x0, x1) = self.groups[g].view;
        match self.data_of(si, sig).ok_or("no data")? {
            Data::File(ds, i) => ds
                .exact_minmax(i, x0, x1)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "no samples in the range".into()),
            Data::Series(s) => s
                .exact_minmax(x0, x1)
                .ok_or_else(|| "no samples in the range".into()),
        }
    }

    pub(crate) fn script_cursor_value(
        &mut self,
        which: usize,
        signal: &str,
        tag: Option<&str>,
    ) -> Result<f64, String> {
        let a = self.answer_of(signal, tag)?;
        a.at_cursor[which].ok_or_else(|| "no value at that cursor".into())
    }

    pub(crate) fn script_cursor_x(&mut self, which: usize) -> Result<Option<f64>, String> {
        let g = self.active_group()?;
        Ok(self.groups[g].cursors[which])
    }

    /// Lines drawn (in the newest answer, which must be up to date).
    pub(crate) fn script_lines(&self) -> Result<usize, String> {
        if !self.up_to_date() {
            return Err("the data on screen is not up to date yet".into());
        }
        Ok(self
            .reply
            .as_ref()
            .map_or(0, |r| r.panes.iter().map(|p| p.insts.len()).sum()))
    }

    /// Sources: tag, state word, message, visible.
    pub(crate) fn script_sources(&self) -> Vec<(String, &'static str, String, bool)> {
        self.sources
            .iter()
            .map(|s| {
                let (w, m) = match &s.state {
                    SourceState::Loading(..) => ("loading", String::new()),
                    SourceState::Ready(r) => ("ready", format!("{} points", r.points)),
                    SourceState::Failed(e) => ("failed", e.clone()),
                    SourceState::Memory => ("memory", format!("{} series", s.extras.len())),
                };
                (s.tag.clone(), w, m, s.visible)
            })
            .collect()
    }

    pub(crate) fn script_point(&mut self, tag: &str, on: bool) -> Result<(), String> {
        let s = self
            .sources
            .iter_mut()
            .find(|s| s.tag == tag)
            .ok_or_else(|| format!("no point '{tag}'"))?;
        s.visible = on;
        Ok(())
    }

    pub(crate) fn script_points(&mut self, on: bool) {
        for s in &mut self.sources {
            s.visible = on;
        }
    }

    pub(crate) fn script_add_file(&mut self, tag: String, path: PathBuf) {
        self.add_file(tag, path);
    }

    /// The rows of the signal list (after setting the filter and grouping when given).
    pub(crate) fn script_list(
        &mut self,
        filter: Option<&str>,
        grouping: Option<Grouping>,
    ) -> Vec<String> {
        if let Some(f) = filter {
            self.list.filter = f.to_string();
        }
        if let Some(g) = grouping {
            self.list.grouping = g;
        }
        self.refresh_catalog();
        self.rows()
            .into_iter()
            .map(|r| match r {
                Row::Header {
                    title,
                    count,
                    depth,
                    ..
                } => format!("{}# {title} ({count})", "  ".repeat(depth)),
                Row::Signal { entry, depth } => {
                    format!("{}{}", "  ".repeat(depth), self.catalog[entry].name)
                }
            })
            .collect()
    }

    /// Fold or unfold a header of the list.
    pub(crate) fn script_fold(&mut self, key: &str) {
        if !self.list.closed.remove(key) {
            self.list.closed.insert(key.to_string());
        }
    }

    pub(crate) fn script_select(&mut self, names: &[String], add: bool) -> Result<(), String> {
        if !add {
            self.list.selected.clear();
        }
        for n in names {
            let n = self.known(n)?;
            self.list.selected.insert(n);
        }
        Ok(())
    }

    pub(crate) fn script_selected(&self) -> Vec<String> {
        self.list.selected.iter().cloned().collect()
    }

    /// What the list's buttons do with the selection: show (active pane), new pane, hide.
    pub(crate) fn script_send_selected(&mut self, how: &str) -> Result<(), String> {
        let sel: Vec<String> = self.list.selected.iter().cloned().collect();
        if sel.is_empty() {
            return Err("nothing is selected".into());
        }
        match how {
            "show" => self.show(&sel, Target::Active),
            "new" => self.show(&sel, Target::New),
            "hide" => self.hide(&sel),
            _ => return Err(format!("'{how}' is not show, new or hide")),
        }
        Ok(())
    }

    /// Change the style of a line (`tag` in a tab of several sources).
    pub(crate) fn script_style(
        &mut self,
        signal: &str,
        tag: Option<&str>,
        color: Option<[u8; 3]>,
        dash: Option<Dash>,
        width: Option<f32>,
    ) -> Result<(), String> {
        let name = self.known(signal)?;
        let insts = self.instances();
        let (_, _, si, _, idx) = insts
            .iter()
            .find(|(_, n, si, _, _)| *n == name && tag.is_none_or(|t| self.sources[*si].tag == t))
            .cloned()
            .ok_or_else(|| format!("'{signal}' is not shown"))?;
        let sid = self.sources[si].id;
        let mut st = self.style_of(&name, sid, idx);
        if let Some(c) = color {
            st.color = c;
        }
        if let Some(d) = dash {
            st.dash = d;
        }
        if let Some(w) = width {
            st.width = w.clamp(0.3, 12.0);
        }
        self.styles.insert((name, sid), st);
        Ok(())
    }

    /// The styles of the visible lines of pane `n`: label, style.
    pub(crate) fn script_styles(&self, n: usize) -> Result<Vec<(String, Style)>, String> {
        let id = self.panes.by_number(n).ok_or("no such pane")?;
        Ok(self
            .instances()
            .into_iter()
            .filter(|(p, _, si, _, _)| *p == id && self.sources[*si].visible)
            .map(|(_, name, si, _, idx)| {
                (
                    self.label_of(&name, si),
                    self.style_of(&name, self.sources[si].id, idx),
                )
            })
            .collect())
    }

    /// Files that changed on disk (the query thread looks once a second).
    pub(crate) fn script_changed_files(&self) -> usize {
        self.changed.len()
    }

    pub(crate) fn script_markers(&self) -> &[Marker] {
        &self.markers
    }

    pub(crate) fn script_x_unit(&self, n: usize) -> Result<String, String> {
        let id = self.panes.by_number(n).ok_or("no such pane")?;
        Ok(self
            .panes
            .get(id)
            .map(|p| p.x_unit.clone())
            .unwrap_or_default())
    }

    /// Frame and query statistics since the last call, as a log line; they restart.
    pub(crate) fn script_stats(&mut self) -> Result<String, String> {
        let title = model::shorten(&self.title, 40);
        let mut frames: Vec<f32> = self.stats.frames.drain(..).collect();
        let mut ui: Vec<f32> = self.stats.ui_ms.drain(..).collect();
        let replies: Vec<(f64, f64, usize)> = self.stats.replies.drain(..).collect();
        let modes = std::mem::take(&mut self.stats.modes);
        frames.sort_by(f32::total_cmp);
        ui.sort_by(f32::total_cmp);
        let mut q: Vec<f32> = replies.iter().map(|r| r.0 as f32).collect();
        let mut d: Vec<f32> = replies.iter().map(|r| r.1 as f32).collect();
        q.sort_by(f32::total_cmp);
        d.sort_by(f32::total_cmp);
        let lines = replies.last().map_or(0, |r| r.2);
        let (rss, rss_file, rss_anon) = rss_kib();
        let mem = memory_note();
        let open = self
            .sources
            .iter()
            .find_map(|s| match &s.state {
                SourceState::Ready(r) => Some(format!(
                    "open {:.0} ms, {}",
                    r.open_time.as_secs_f64() * 1e3,
                    if r.converted {
                        "cache written"
                    } else {
                        "cache reused"
                    }
                )),
                _ => None,
            })
            .unwrap_or_default();
        Ok(format!(
            "{title}: {} frames, interval p50 {:.1} ms p95 {:.1} ms max {:.1} ms; ui p50 {:.2} ms p95 {:.2} ms max {:.2} ms; \
             {} replies of {lines} lines, query p50 {:.2} ms p95 {:.2} ms max {:.2} ms, drawing p50 {:.2} ms p95 {:.2} ms max {:.2} ms \
             (lines: direct {}, min/max {}, pyramid {}); {} files ({} open, {} failed); \
             RSS {} MiB (file {} MiB, anon {} MiB){mem}; {open}",
            frames.len(),
            percentile(&frames, 0.5),
            percentile(&frames, 0.95),
            frames.last().copied().unwrap_or(0.0),
            percentile(&ui, 0.5),
            percentile(&ui, 0.95),
            ui.last().copied().unwrap_or(0.0),
            replies.len(),
            percentile(&q, 0.5),
            percentile(&q, 0.95),
            q.last().copied().unwrap_or(0.0),
            percentile(&d, 0.5),
            percentile(&d, 0.95),
            d.last().copied().unwrap_or(0.0),
            modes[0],
            modes[1],
            modes[2],
            self.sources.iter().filter(|s| s.path.is_some()).count(),
            self.ready(),
            self.failed(),
            rss / 1024,
            rss_file / 1024,
            rss_anon / 1024,
        ))
    }
}
