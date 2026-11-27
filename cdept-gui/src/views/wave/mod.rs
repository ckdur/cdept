//! A waveform tab: CDePT's own viewer on UberWave's data engine (`cdept-wavedata`).
//!
//! Every tab is independent: its own file, its own engine dataset (several tabs on one
//! file share the `.uwraw` cache on disk, nothing else), its own loading thread and
//! query thread (`worker.rs`). The tab is an ordinary dock tab: it docks, floats and
//! moves to other windows like the others.
//!
//! The view asks the engine for every shown signal decimated to the width of the plot in
//! pixels, with each bucket's minimum and maximum kept, so a spike one sample wide is
//! drawn at any zoom. It draws the newest answer while the next one is computed.
//!
//! Prototype scope (docs/ROADMAP.md Phase 5, steps 1 and 2): one plot with a shared y
//! axis, a signal list with a filter, zoom and pan (wheel, drag, Fit), two cursors with
//! readouts and their difference. Panes, families, calculator traces, Bode and eye
//! diagrams are steps 3 and 4.
//!
//! Mouse: wheel zooms x around the pointer (Ctrl+wheel zooms y, Shift+wheel pans x),
//! drag pans, a click puts cursor A, a right click cursor B, a double click fits.

pub(crate) mod script;
mod units;
pub(crate) mod worker;

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use cdept_wavedata::{Dataset, Mode, Phase, Prepared};
use egui::{Color32, Event, MouseWheelUnit, PointerButton, RichText, Ui};
use egui_plot::{Legend, Line, Plot, PlotBounds, PlotPoints, VLine};

use self::worker::{Loading, Reply, Request, Worker};
use super::View;
use crate::state::AppCtx;

/// Trace colours: UberWave's palettes (settings/TraceStyle.hpp), dark and light.
const DARK: [[u8; 3]; 10] = [
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
const LIGHT: [[u8; 3]; 10] = [
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

/// How many signals the tab shows when the file opens.
const FIRST_SHOWN: usize = 2;
/// Frame statistics kept for `wv stats`.
const FRAME_SAMPLES: usize = 4000;

pub struct WaveView {
    id: u64,
    path: PathBuf,
    title: String,
    state: State,
}

enum State {
    Loading(Loading),
    Ready(Box<Viewer>),
    Failed(String),
}

#[derive(Debug, Clone)]
struct Signal {
    name: String,
    unit: String,
}

/// An open dataset on screen.
struct Viewer {
    ds: Arc<Dataset>,
    worker: Worker,
    signals: Vec<Signal>,
    x_name: String,
    x_unit: String,
    full: (f64, f64),
    /// The x range on screen.
    view: (f64, f64),
    /// A y range the user chose (Ctrl+wheel); `None`: fitted to what is shown.
    y_manual: Option<(f64, f64)>,
    shown: Vec<u32>,
    filter: String,
    cursors: [Option<f64>; 2],
    reply: Option<Reply>,
    /// The last request sent, and when (pyramids being built: asked again later).
    sent: Option<(Request, Instant)>,
    seq: u64,
    pixels: u32,
    plot_rect: Option<egui::Rect>,
    open_time: Duration,
    converted: bool,
    cache: String,
    points: u64,
    source_changed: bool,
    checked: Instant,
    stats: Stats,
}

/// Frame times and query times since the last `wv stats`.
#[derive(Default)]
struct Stats {
    last_frame: Option<Instant>,
    frames: VecDeque<f32>,
    ui_ms: VecDeque<f32>,
    queries: VecDeque<(f64, Mode, u64)>,
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

impl WaveView {
    /// A tab on `path`; it starts loading at once.
    pub fn open(ctx: &egui::Context, id: u64, path: PathBuf, title: Option<String>) -> WaveView {
        let title = title.unwrap_or_else(|| {
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string())
        });
        let state = State::Loading(Loading::start(ctx, path.clone()));
        WaveView {
            id,
            path,
            title,
            state,
        }
    }

    fn reload(&mut self, ctx: &egui::Context) {
        self.state = State::Loading(Loading::start(ctx, self.path.clone()));
    }

    /// Every frame, for every tab, shown or not: a tab behind another one finishes
    /// loading too (its worker and the file's pyramids start without it being drawn).
    pub(crate) fn tick(&mut self, ctx: &egui::Context) {
        self.poll_loading(ctx);
    }

    fn poll_loading(&mut self, ctx: &egui::Context) {
        let State::Loading(loading) = &mut self.state else {
            return;
        };
        let Some(result) = loading.poll() else {
            return;
        };
        self.state = match result {
            Ok((ds, took)) => {
                tracing::info!(
                    target: "cdept::wave",
                    "{}: opened in {:.0} ms ({} signals, {} points, {})",
                    self.path.display(),
                    took.as_secs_f64() * 1e3,
                    ds.signal_count(),
                    ds.point_count(),
                    if ds.converted() { "cache written" } else { "cache reused" }
                );
                State::Ready(Box::new(Viewer::new(ctx, ds, took)))
            }
            Err(e) => {
                tracing::warn!(target: "cdept::wave", "{}: {e}", self.path.display());
                State::Failed(e.message)
            }
        };
    }

    fn viewer(&mut self) -> Result<&mut Viewer, String> {
        match &mut self.state {
            State::Ready(v) => Ok(v),
            State::Loading(_) => Err("the file is still loading".into()),
            State::Failed(e) => Err(format!("the file did not open: {e}")),
        }
    }

    // --- script (`wv ...`, cdept-gui/src/script.rs) ---

    pub(crate) fn script_state(&self) -> Result<(), String> {
        match &self.state {
            State::Ready(_) => Ok(()),
            State::Loading(l) => Err(format!(
                "still loading ({:?} {:.0}%)",
                l.progress.phase(),
                l.progress.fraction() * 100.0
            )),
            State::Failed(e) => Err(format!("failed: {e}")),
        }
    }

    pub(crate) fn script_failed(&self) -> Option<&str> {
        match &self.state {
            State::Failed(e) => Some(e),
            _ => None,
        }
    }

    pub(crate) fn script_loading(&self) -> bool {
        matches!(self.state, State::Loading(_))
    }

    /// The cache being read, and whether the source changed since it was read (asked
    /// now, not at the once-a-second check of the toolbar).
    pub(crate) fn script_cache(&mut self) -> Result<(String, bool), String> {
        let v = self.viewer()?;
        v.source_changed = v.ds.source_changed();
        Ok((v.cache.clone(), v.source_changed))
    }

    pub(crate) fn script_reload(&mut self, ctx: &egui::Context) {
        self.reload(ctx);
    }

    pub(crate) fn script_cancel(&mut self) {
        if let State::Loading(l) = &self.state {
            l.progress.cancel();
        }
    }

    pub(crate) fn script_show(&mut self, names: &[String], show: bool) -> Result<(), String> {
        let v = self.viewer()?;
        for name in names {
            let i = v.find(name).ok_or_else(|| format!("no signal '{name}'"))?;
            v.set_shown(i, show);
        }
        Ok(())
    }

    pub(crate) fn script_fit(&mut self) -> Result<(), String> {
        let v = self.viewer()?;
        v.fit();
        Ok(())
    }

    pub(crate) fn script_zoom(&mut self, x0: f64, x1: f64) -> Result<(), String> {
        let v = self.viewer()?;
        v.set_view(x0.min(x1), x0.max(x1));
        Ok(())
    }

    pub(crate) fn script_cursor(&mut self, which: usize, x: Option<f64>) -> Result<(), String> {
        let v = self.viewer()?;
        v.cursors[which] = x;
        Ok(())
    }

    /// The plot's rectangle on screen (points), for mouse input.
    pub(crate) fn script_plot_rect(&mut self) -> Result<egui::Rect, String> {
        self.viewer()?
            .plot_rect
            .ok_or_else(|| "the plot has not been drawn yet".into())
    }

    /// The x range on screen.
    pub(crate) fn script_range(&mut self) -> Result<(f64, f64), String> {
        Ok(self.viewer()?.view)
    }

    /// The newest answer covers what is on screen now.
    fn current(&mut self) -> Result<&Reply, String> {
        let v = self.viewer()?;
        let r = v.reply.as_ref().ok_or("no data yet")?;
        let req = &r.request;
        if req.x0 != v.view.0
            || req.x1 != v.view.1
            || req.signals != v.shown
            || req.cursors != v.cursors
        {
            return Err("the data on screen is not up to date yet".into());
        }
        Ok(r)
    }

    fn answer_of(&mut self, signal: &str) -> Result<(u32, worker::Answer), String> {
        let i = self
            .viewer()?
            .find(signal)
            .ok_or_else(|| format!("no signal '{signal}'"))?;
        let r = self.current()?;
        let a = r
            .answers
            .iter()
            .find(|a| a.signal == i)
            .ok_or_else(|| format!("'{signal}' is not shown"))?;
        Ok((i, a.clone()))
    }

    /// The largest and smallest value drawn for `signal` inside the x range on screen.
    pub(crate) fn script_drawn_extremes(
        &mut self,
        signal: &str,
    ) -> Result<(f64, f64, usize), String> {
        let (x0, x1) = self.viewer()?.view;
        let (_, a) = self.answer_of(signal)?;
        let mut lo = f64::INFINITY;
        let mut hi = f64::NEG_INFINITY;
        for (x, y) in a.trace.x.iter().zip(&a.trace.y) {
            if *x >= x0 && *x <= x1 && y.is_finite() {
                lo = lo.min(*y);
                hi = hi.max(*y);
            }
        }
        Ok((lo, hi, a.trace.x.len()))
    }

    /// The exact extremes of `signal` over the x range on screen, read from every
    /// sample (on this thread: for checks only).
    pub(crate) fn script_exact_extremes(
        &mut self,
        signal: &str,
    ) -> Result<(f64, f64, u64), String> {
        let v = self.viewer()?;
        let i = v
            .find(signal)
            .ok_or_else(|| format!("no signal '{signal}'"))?;
        v.ds.exact_minmax(i, v.view.0, v.view.1)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "no samples in the range".into())
    }

    pub(crate) fn script_cursor_value(
        &mut self,
        which: usize,
        signal: &str,
    ) -> Result<f64, String> {
        let (_, a) = self.answer_of(signal)?;
        a.at_cursor[which].ok_or_else(|| "no value at that cursor".into())
    }

    pub(crate) fn script_cursor_x(&mut self, which: usize) -> Result<Option<f64>, String> {
        Ok(self.viewer()?.cursors[which])
    }

    /// Frame and query statistics since the last call, as a log line; they restart.
    pub(crate) fn script_stats(&mut self) -> Result<String, String> {
        let title = self.title.clone();
        let v = self.viewer()?;
        let mut frames: Vec<f32> = v.stats.frames.drain(..).collect();
        let mut ui: Vec<f32> = v.stats.ui_ms.drain(..).collect();
        let queries: Vec<(f64, Mode, u64)> = v.stats.queries.drain(..).collect();
        frames.sort_by(f32::total_cmp);
        ui.sort_by(f32::total_cmp);
        let mut q: Vec<f32> = queries.iter().map(|(ms, _, _)| *ms as f32).collect();
        q.sort_by(f32::total_cmp);
        let modes = |m: Mode| queries.iter().filter(|(_, x, _)| *x == m).count();
        let (rss, rss_file, rss_anon) = rss_kib();
        let mem = memory_note();
        Ok(format!(
            "{title}: {} frames, interval p50 {:.1} ms p95 {:.1} ms max {:.1} ms; ui p50 {:.2} ms p95 {:.2} ms max {:.2} ms; \
             {} replies, query p50 {:.2} ms p95 {:.2} ms max {:.2} ms (direct {}, min/max {}, pyramid {}); \
             RSS {} MiB (file {} MiB, anon {} MiB){mem}; open {:.0} ms, {}",
            frames.len(),
            percentile(&frames, 0.5),
            percentile(&frames, 0.95),
            frames.last().copied().unwrap_or(0.0),
            percentile(&ui, 0.5),
            percentile(&ui, 0.95),
            ui.last().copied().unwrap_or(0.0),
            q.len(),
            percentile(&q, 0.5),
            percentile(&q, 0.95),
            q.last().copied().unwrap_or(0.0),
            modes(Mode::Direct),
            modes(Mode::MinMax),
            modes(Mode::Pyramid),
            rss / 1024,
            rss_file / 1024,
            rss_anon / 1024,
            v.open_time.as_secs_f64() * 1e3,
            if v.converted {
                "cache written"
            } else {
                "cache reused"
            },
        ))
    }
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

impl Viewer {
    fn new(ctx: &egui::Context, ds: Dataset, took: Duration) -> Viewer {
        let ds = Arc::new(ds);
        let signals: Vec<Signal> = (0..ds.signal_count())
            .map(|i| Signal {
                name: ds.signal_name(i),
                unit: ds.signal_unit(i),
            })
            .collect();
        let full = ds.x_range().unwrap_or((0.0, 1.0));
        let full = if full.1 > full.0 {
            full
        } else {
            (full.0, full.0 + 1.0)
        };
        let shown: Vec<u32> = (1..signals.len() as u32).take(FIRST_SHOWN).collect();
        let worker = Worker::start(ctx, ds.clone());
        Viewer {
            x_name: ds.text(cdept_wavedata::Text::XName),
            x_unit: ds.text(cdept_wavedata::Text::XUnit),
            converted: ds.converted(),
            cache: ds.text(cdept_wavedata::Text::Cache),
            points: ds.point_count(),
            ds,
            worker,
            signals,
            full,
            view: full,
            y_manual: None,
            shown,
            filter: String::new(),
            cursors: [None, None],
            reply: None,
            sent: None,
            seq: 0,
            pixels: 0,
            plot_rect: None,
            open_time: took,
            source_changed: false,
            checked: Instant::now(),
            stats: Stats::default(),
        }
    }

    fn find(&self, name: &str) -> Option<u32> {
        self.signals
            .iter()
            .position(|s| s.name == name)
            .or_else(|| {
                self.signals
                    .iter()
                    .position(|s| s.name.eq_ignore_ascii_case(name))
            })
            .map(|i| i as u32)
    }

    fn set_shown(&mut self, signal: u32, show: bool) {
        let at = self.shown.iter().position(|s| *s == signal);
        match (show, at) {
            (true, None) => self.shown.push(signal),
            (false, Some(i)) => {
                self.shown.remove(i);
            }
            _ => {}
        }
    }

    fn fit(&mut self) {
        self.view = self.full;
        self.y_manual = None;
    }

    /// Keep the range finite and wider than nothing; no wider than the data.
    fn set_view(&mut self, x0: f64, x1: f64) {
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
        // Panning stops at the ends of the data.
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

    fn color(&self, ui: &Ui, signal: u32) -> Color32 {
        let slot = self.shown.iter().position(|s| *s == signal).unwrap_or(0) % DARK.len();
        let [r, g, b] = if ui.visuals().dark_mode {
            DARK[slot]
        } else {
            LIGHT[slot]
        };
        Color32::from_rgb(r, g, b)
    }

    /// Send a request when what is on screen changed (or a pyramid became ready).
    fn request(&mut self) {
        if self.pixels == 0 {
            return;
        }
        let wanted = Request {
            seq: 0,
            x0: self.view.0,
            x1: self.view.1,
            pixels: self.pixels,
            signals: self.shown.clone(),
            cursors: self.cursors,
        };
        let same = self.sent.as_ref().is_some_and(|(r, _)| {
            Request {
                seq: 0,
                ..r.clone()
            } == wanted
        });
        // Pyramids being built: ask again every 300 ms, the answers get cheaper.
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

    fn take_replies(&mut self) {
        if let Some(r) = self.worker.newest() {
            for a in &r.answers {
                Stats::push(
                    &mut self.stats.queries,
                    (a.info.ms, a.info.mode, a.info.source_points),
                );
            }
            if let Some(e) = &r.error {
                tracing::warn!(target: "cdept::wave", "query: {e}");
            }
            self.reply = Some(r);
        }
    }

    /// The y range: chosen, or fitted to the data drawn in the x range.
    fn y_range(&self) -> (f64, f64) {
        if let Some(y) = self.y_manual {
            return y;
        }
        let mut lo = f64::INFINITY;
        let mut hi = f64::NEG_INFINITY;
        if let Some(r) = &self.reply {
            for a in r.answers.iter().filter(|a| self.shown.contains(&a.signal)) {
                for (x, y) in a.trace.x.iter().zip(&a.trace.y) {
                    if *x >= self.view.0 && *x <= self.view.1 && y.is_finite() {
                        lo = lo.min(*y);
                        hi = hi.max(*y);
                    }
                }
            }
        }
        if !(lo.is_finite() && hi.is_finite()) {
            return (-1.0, 1.0);
        }
        if hi - lo <= f64::EPSILON * hi.abs().max(1e-300) {
            let pad = if hi == 0.0 { 1.0 } else { hi.abs() * 0.1 };
            return (lo - pad, hi + pad);
        }
        let pad = (hi - lo) * 0.06;
        (lo - pad, hi + pad)
    }

    fn signal_list(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label("Signals");
            ui.weak(format!(
                "{} of {}",
                self.shown.len(),
                self.signals.len().saturating_sub(1)
            ));
        });
        ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text("filter"));
        let filter = self.filter.to_lowercase();
        let rows: Vec<u32> = (1..self.signals.len() as u32)
            .filter(|i| {
                filter.is_empty()
                    || self.signals[*i as usize]
                        .name
                        .to_lowercase()
                        .contains(&filter)
            })
            .collect();
        let row_h = ui.spacing().interact_size.y;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, row_h, rows.len(), |ui, range| {
                for &i in &rows[range] {
                    let mut on = self.shown.contains(&i);
                    let s = &self.signals[i as usize];
                    let text = if on {
                        RichText::new(&s.name).color(self.color(ui, i))
                    } else {
                        RichText::new(&s.name)
                    };
                    if ui.checkbox(&mut on, text).on_hover_text(&s.unit).changed() {
                        self.set_shown(i, on);
                    }
                }
            });
    }

    fn toolbar(&mut self, ui: &mut Ui, reload: &mut bool) {
        ui.horizontal_wrapped(|ui| {
            if ui
                .button("Fit")
                .on_hover_text("The whole x range, y fitted (double click)")
                .clicked()
            {
                self.fit();
            }
            let (a, b) = self.view;
            let c = (a + b) / 2.0;
            if ui.button("Zoom In").clicked() {
                self.set_view(c - (b - a) / 4.0, c + (b - a) / 4.0);
            }
            if ui.button("Zoom Out").clicked() {
                self.set_view(c - (b - a), c + (b - a));
            }
            if ui
                .add_enabled(
                    self.cursors.iter().any(Option::is_some),
                    egui::Button::new("Clear Cursors"),
                )
                .clicked()
            {
                self.cursors = [None, None];
            }
            ui.separator();
            let shown_points: usize = self
                .reply
                .as_ref()
                .map_or(0, |r| r.answers.iter().map(|a| a.trace.x.len()).sum());
            let mode = self
                .reply
                .as_ref()
                .and_then(|r| r.answers.first())
                .map(|a| a.info.mode);
            let building = self.reply.as_ref().map_or(0, |r| {
                r.prepared
                    .iter()
                    .filter(|(_, p)| matches!(p, Prepared::Building(_)))
                    .count()
            });
            ui.weak(format!(
                "{} samples · {shown_points} drawn{} · {:.1} ms{}",
                self.points,
                match mode {
                    Some(Mode::Direct) => " (all)",
                    Some(Mode::MinMax) => " (min/max)",
                    Some(Mode::Pyramid) => " (min/max, pyramid)",
                    None => "",
                },
                self.reply.as_ref().map_or(0.0, |r| r.ms),
                if building > 0 {
                    format!(" · indexing {building} signal(s)")
                } else {
                    String::new()
                }
            ))
            .on_hover_text(format!(
                "cache: {}\nopened in {:.0} ms ({})",
                self.cache,
                self.open_time.as_secs_f64() * 1e3,
                if self.converted {
                    "cache written"
                } else {
                    "cache reused"
                }
            ));
            if self.source_changed {
                ui.separator();
                ui.colored_label(ui.visuals().warn_fg_color, "The file changed on disk");
                if ui.button("Reload").clicked() {
                    *reload = true;
                }
            }
        });
    }

    fn readout(&self, ui: &mut Ui) {
        let [a, b] = self.cursors;
        if a.is_none() && b.is_none() {
            ui.weak("Click: cursor A · right click: cursor B · wheel: zoom · drag: pan · double click: fit");
            return;
        }
        let xs = |x: Option<f64>| x.map_or("–".to_string(), |x| units::si(x, &self.x_unit, 6));
        ui.horizontal_wrapped(|ui| {
            ui.label(format!("A: {}", xs(a)));
            ui.label(format!("B: {}", xs(b)));
            if let (Some(a), Some(b)) = (a, b) {
                let dx = b - a;
                ui.label(format!("B−A: {}", units::si(dx, &self.x_unit, 6)));
                if dx != 0.0 && self.x_unit == "s" {
                    ui.label(format!("1/(B−A): {}", units::si(1.0 / dx.abs(), "Hz", 5)));
                }
            }
        });
        let Some(reply) = &self.reply else {
            return;
        };
        egui::Grid::new("wave-readout")
            .striped(true)
            .num_columns(4)
            .show(ui, |ui| {
                ui.strong("Signal");
                ui.strong("A");
                ui.strong("B");
                ui.strong("B−A");
                ui.end_row();
                for ans in &reply.answers {
                    let s = &self.signals[ans.signal as usize];
                    ui.colored_label(self.color(ui, ans.signal), &s.name);
                    let v =
                        |x: Option<f64>| x.map_or("–".to_string(), |x| units::si(x, &s.unit, 6));
                    ui.label(v(ans.at_cursor[0]));
                    ui.label(v(ans.at_cursor[1]));
                    // As UberWave's measurement: a difference only of values in the same unit.
                    let d = match ans.at_cursor {
                        [Some(a), Some(b)] => units::si(b - a, &s.unit, 6),
                        _ => "–".into(),
                    };
                    ui.label(d);
                    ui.end_row();
                }
            });
    }

    fn plot(&mut self, ui: &mut Ui, id: u64) {
        let (y0, y1) = self.y_range();
        let (x0, x1) = self.view;
        let x_unit = self.x_unit.clone();
        let y_unit = {
            let units: Vec<&str> = self
                .shown
                .iter()
                .map(|s| self.signals[*s as usize].unit.as_str())
                .collect();
            match units.first() {
                Some(u) if units.iter().all(|x| x == u) => u.to_string(),
                _ => String::new(),
            }
        };
        let lines: Vec<(String, Color32, Vec<[f64; 2]>)> = self
            .reply
            .as_ref()
            .map(|r| {
                r.answers
                    .iter()
                    .filter(|a| self.shown.contains(&a.signal))
                    .map(|a| {
                        let pts = a
                            .trace
                            .x
                            .iter()
                            .zip(&a.trace.y)
                            .filter(|(x, y)| x.is_finite() && y.is_finite())
                            .map(|(x, y)| [*x, *y])
                            .collect();
                        (
                            self.signals[a.signal as usize].name.clone(),
                            self.color(ui, a.signal),
                            pts,
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let cursors = self.cursors;
        let cursor_color = ui.visuals().strong_text_color();
        let xu = x_unit.clone();
        let yu = y_unit.clone();
        let response = Plot::new(("wave-plot", id))
            .allow_drag(false)
            .allow_zoom(false)
            .allow_scroll(false)
            .allow_boxed_zoom(false)
            .allow_double_click_reset(false)
            .show_x(false)
            .show_y(false)
            .legend(Legend::default())
            .x_axis_label(units::x_label(&self.x_name, &self.x_unit))
            .x_grid_spacer(|input| units::grid(input.bounds, 8.0))
            .y_grid_spacer(|input| units::grid(input.bounds, 6.0))
            .x_axis_formatter(move |mark, range| {
                units::tick(mark.value, range.end() - range.start(), mark.step_size, &xu)
            })
            .y_axis_formatter(move |mark, range| {
                units::tick(mark.value, range.end() - range.start(), mark.step_size, &yu)
            })
            .show(ui, |pui| {
                pui.set_plot_bounds(PlotBounds::from_min_max([x0, y0], [x1, y1]));
                for (name, color, pts) in lines {
                    pui.line(
                        Line::new(name, PlotPoints::from(pts))
                            .color(color)
                            .width(1.5),
                    );
                }
                for (i, c) in cursors.iter().enumerate() {
                    if let Some(x) = c {
                        // No name: the cursors stay out of the legend. A solid, B dashed.
                        pui.vline(
                            VLine::new("", *x)
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
            });
        let frame = *response.transform.frame();
        self.plot_rect = Some(frame);
        self.pixels = (frame.width() * ui.ctx().pixels_per_point())
            .round()
            .clamp(1.0, 65536.0) as u32;
        self.interact(ui, &response.response, frame);
    }

    /// Mouse on the plot: wheel, drag, clicks (see the module documentation).
    fn interact(&mut self, ui: &Ui, response: &egui::Response, frame: egui::Rect) {
        let (x0, x1) = self.view;
        let width = (frame.width() as f64).max(1.0);
        let to_x = |px: f32| x0 + (px - frame.left()) as f64 / width * (x1 - x0);
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
                    let shift = (x1 - x0) * 0.1 * -(notches as f64);
                    self.set_view(x0 + shift, x1 + shift);
                } else if mods.ctrl || mods.command {
                    let (y0, y1) = self.y_range();
                    let height = (frame.height() as f64).max(1.0);
                    let yc = y1 - (pos.y - frame.top()) as f64 / height * (y1 - y0);
                    self.y_manual = Some((yc - (yc - y0) * factor, yc + (y1 - yc) * factor));
                } else {
                    let xc = to_x(pos.x);
                    self.set_view(xc - (xc - x0) * factor, xc + (x1 - xc) * factor);
                }
            }
        }
        if response.dragged_by(PointerButton::Primary) {
            let d = response.drag_delta();
            let dx = -(d.x as f64) / width * (x1 - x0);
            self.set_view(x0 + dx, x1 + dx);
            if let Some((y0, y1)) = self.y_manual {
                let dy = d.y as f64 / (frame.height() as f64).max(1.0) * (y1 - y0);
                self.y_manual = Some((y0 + dy, y1 + dy));
            }
        }
        if response.double_clicked() {
            self.fit();
        } else if response.clicked() {
            if let Some(pos) = response.interact_pointer_pos() {
                self.cursors[0] = Some(to_x(pos.x));
            }
        } else if response.secondary_clicked()
            && let Some(pos) = response.interact_pointer_pos()
        {
            self.cursors[1] = Some(to_x(pos.x));
        }
    }
}

impl View for WaveView {
    fn title(&self, _ctx: &AppCtx) -> String {
        match &self.state {
            State::Loading(_) => format!("{} (loading)", self.title),
            State::Failed(_) => format!("{} (failed)", self.title),
            State::Ready(_) => self.title.clone(),
        }
    }

    fn ui(&mut self, ui: &mut Ui, _ctx: &mut AppCtx) {
        let started = Instant::now();
        self.poll_loading(ui.ctx());
        let mut reload = false;
        match &mut self.state {
            State::Loading(l) => {
                let p = l.progress.clone();
                ui.vertical_centered(|ui| {
                    ui.add_space(40.0);
                    ui.heading(&self.title);
                    ui.weak(self.path.display().to_string());
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
                    ui.weak(format!("{:.1} s", l.started.elapsed().as_secs_f64()));
                    if p.cancelled() {
                        ui.weak("Cancelling…");
                    } else if ui.button("Cancel").clicked() {
                        p.cancel();
                    }
                });
                ui.ctx().request_repaint_after(Duration::from_millis(100));
            }
            State::Failed(e) => {
                let e = e.clone();
                ui.vertical_centered(|ui| {
                    ui.add_space(40.0);
                    ui.heading(&self.title);
                    ui.weak(self.path.display().to_string());
                    ui.add_space(12.0);
                    ui.colored_label(ui.visuals().error_fg_color, e);
                    if ui.button("Try Again").clicked() {
                        reload = true;
                    }
                });
            }
            State::Ready(v) => {
                let now = Instant::now();
                if let Some(last) = v.stats.last_frame {
                    Stats::push(&mut v.stats.frames, (now - last).as_secs_f32() * 1e3);
                }
                v.stats.last_frame = Some(now);
                if v.checked.elapsed() > Duration::from_secs(1) {
                    v.checked = Instant::now();
                    // A stat and a small read of the cache's header: cheap enough here.
                    v.source_changed = v.ds.source_changed();
                }
                v.take_replies();
                v.toolbar(ui, &mut reload);
                ui.separator();
                let id = self.id;
                egui::Panel::left(egui::Id::new(("wave-signals", id)))
                    .resizable(true)
                    .default_size(190.0)
                    .show(ui, |ui| v.signal_list(ui));
                egui::Panel::bottom(egui::Id::new(("wave-readout", id)))
                    .resizable(true)
                    .default_size(170.0)
                    .show(ui, |ui| {
                        // Without this the scroll area keeps its smallest height and the
                        // panel shows one row of the table.
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show(ui, |ui| v.readout(ui));
                    });
                egui::CentralPanel::default().show(ui, |ui| v.plot(ui, id));
                v.request();
                if v.reply.as_ref().is_some_and(Reply::building) {
                    ui.ctx().request_repaint_after(Duration::from_millis(300));
                }
                Stats::push(&mut v.stats.ui_ms, started.elapsed().as_secs_f32() * 1e3);
            }
        }
        if reload {
            self.reload(ui.ctx());
        }
    }
}
