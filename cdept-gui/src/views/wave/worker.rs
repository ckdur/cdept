//! The threads of a waveform tab. A few loader threads open its files (a family may
//! have hundreds; each open may convert a large raw file to its `.uwraw` cache); one
//! query thread answers what the view wants drawn: it asks the engine (or a memory
//! series) for every drawn line decimated to the pane's width, fits the y ranges, and
//! draws each pane into an image (`raster.rs`). The UI thread never calls the engine and
//! never draws a trace itself: it sends requests and shows the newest images.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use cdept_wavedata::{
    Analysis, Dataset, OpenOptions, Phase, Prepared, QueryInfo, Series, Trace, XOrder,
};

use super::model::{InstKey, PaneId, SourceId};
use super::raster::{Canvas, Stroke};

/// Where caches go when the folder of a raw file is not writable.
pub fn cache_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(base.join("cdept").join("waveforms"))
}

/// Files with more points get min/max pyramids (smaller ones are read directly: they
/// are a few ms). The engine builds each pyramid on a thread of its own, so a request
/// with more large files than [`PREPARE_MAX_FILES`] (a family of hundreds) builds none.
pub const PREPARE_MIN_POINTS: u64 = 100_000;
pub const PREPARE_MAX_FILES: usize = 8;
/// Files opened at once by a tab.
const LOADERS: usize = 4;

/// Progress of an open, shared with the loading thread.
#[derive(Default)]
pub struct LoadProgress {
    cancel: AtomicBool,
    phase: AtomicU8,
    fraction: AtomicU32,
}

impl LoadProgress {
    pub fn phase(&self) -> Phase {
        match self.phase.load(Ordering::Relaxed) {
            1 => Phase::Wait,
            2 => Phase::Convert,
            3 => Phase::Open,
            _ => Phase::Header,
        }
    }

    pub fn fraction(&self) -> f32 {
        f32::from_bits(self.fraction.load(Ordering::Relaxed))
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

/// A file that opened: the dataset, the time it took, the analyses of the file (its
/// plots, or a PSF folder's analyses) and the one opened.
pub struct OpenedFile {
    pub ds: Dataset,
    pub took: Duration,
    pub analyses: Vec<Analysis>,
    pub analysis: u32,
}

pub type Opened = Result<OpenedFile, cdept_wavedata::Error>;

/// Which analysis of a file to open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Want {
    /// This index of the file's analyses.
    Index(u32),
    /// The one of this name, else of this kind (a family's other files), else the best:
    /// what a PSF folder prefers, else the first transient, else the first that has
    /// more than one point.
    Like { name: String, kind: String },
    Best,
}

/// The analysis to open of `list` for `want`.
pub fn choose(list: &[Analysis], want: &Want) -> u32 {
    let ok = |a: &&Analysis| a.plottable;
    let best = || {
        list.iter()
            .filter(ok)
            .find(|a| a.preferred && list.len() > 1 && a.points > 1)
            .or_else(|| list.iter().filter(ok).find(|a| a.kind == "tran" && a.points > 1))
            .or_else(|| list.iter().filter(ok).find(|a| a.points > 1))
            .or_else(|| list.iter().find(ok))
            .map_or(0, |a| a.index)
    };
    match want {
        Want::Index(i) => *i,
        Want::Like { name, kind } => list
            .iter()
            .filter(ok)
            .find(|a| &a.name == name && &a.kind == kind)
            .or_else(|| list.iter().filter(ok).find(|a| &a.kind == kind))
            .map_or_else(best, |a| a.index),
        Want::Best => best(),
    }
}

struct Job {
    source: SourceId,
    generation: u64,
    path: PathBuf,
    want: Want,
    progress: Arc<LoadProgress>,
}

/// What a loader thread answers: the source, the generation of the open (a reload makes
/// older answers stale), the dataset or the error.
pub type Loaded = (SourceId, u64, Opened);

/// The loader threads of a tab: up to [`LOADERS`] files are opened at once, the rest
/// wait in a queue. Dropping it cancels the opens in progress and the queued ones.
pub struct Loader {
    queue: Arc<Mutex<(VecDeque<Job>, usize)>>,
    tx: Sender<Loaded>,
    rx: Receiver<Loaded>,
    closed: Arc<AtomicBool>,
    ctx: egui::Context,
}

impl Loader {
    pub fn new(ctx: &egui::Context) -> Loader {
        let (tx, rx) = mpsc::channel();
        Loader {
            queue: Arc::default(),
            tx,
            rx,
            closed: Arc::default(),
            ctx: ctx.clone(),
        }
    }

    /// Queue an open; the progress is the caller's to show and to cancel.
    pub fn open(
        &self,
        source: SourceId,
        generation: u64,
        path: PathBuf,
        want: Want,
    ) -> Arc<LoadProgress> {
        let progress = Arc::new(LoadProgress::default());
        let mut q = self.queue.lock().unwrap_or_else(|e| e.into_inner());
        q.0.push_back(Job {
            source,
            generation,
            path,
            want,
            progress: progress.clone(),
        });
        if q.1 < LOADERS {
            q.1 += 1;
            let (queue, tx, closed, ctx) = (
                self.queue.clone(),
                self.tx.clone(),
                self.closed.clone(),
                self.ctx.clone(),
            );
            let spawned = std::thread::Builder::new()
                .name("cdept-wave-open".into())
                .spawn(move || load_jobs(&queue, &tx, &closed, &ctx));
            if spawned.is_err() {
                q.1 -= 1;
                if let Some(job) = q.0.pop_back() {
                    let _ = self.tx.send((
                        job.source,
                        job.generation,
                        Err(cdept_wavedata::Error {
                            status: cdept_wavedata::Status::Internal,
                            message: "cannot start a thread to open the file".into(),
                        }),
                    ));
                }
            }
        }
        progress
    }

    /// Opens still queued (not started).
    pub fn queued(&self) -> usize {
        self.queue.lock().map(|q| q.0.len()).unwrap_or(0)
    }

    /// The answers that came.
    pub fn poll(&self) -> Vec<Loaded> {
        self.rx.try_iter().collect()
    }
}

impl Drop for Loader {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Relaxed);
        let mut q = self.queue.lock().unwrap_or_else(|e| e.into_inner());
        for job in q.0.drain(..) {
            job.progress.cancel();
        }
    }
}

fn load_jobs(
    queue: &Mutex<(VecDeque<Job>, usize)>,
    tx: &Sender<Loaded>,
    closed: &AtomicBool,
    ctx: &egui::Context,
) {
    loop {
        let job = {
            let mut q = queue.lock().unwrap_or_else(|e| e.into_inner());
            match q.0.pop_front() {
                Some(j) if !closed.load(Ordering::Relaxed) => j,
                _ => {
                    q.1 -= 1;
                    return;
                }
            }
        };
        let started = Instant::now();
        // The analyses first (an ascii raw file is read once for its plots): which one
        // to open. A file that cannot be listed is opened as it is (and says why).
        let analyses = cdept_wavedata::list_analyses(&job.path).unwrap_or_default();
        let analysis = choose(&analyses, &job.want);
        let options = OpenOptions {
            cache_dir: cache_dir(),
            plot: analysis,
            ..Default::default()
        };
        let p = &job.progress;
        let mut last = Instant::now();
        let r = Dataset::open(&job.path, &options, |phase, fraction| {
            p.phase.store(
                match phase {
                    Phase::Header => 0,
                    Phase::Wait => 1,
                    Phase::Convert => 2,
                    Phase::Open => 3,
                },
                Ordering::Relaxed,
            );
            p.fraction
                .store((fraction as f32).to_bits(), Ordering::Relaxed);
            if last.elapsed() > Duration::from_millis(100) {
                last = Instant::now();
                ctx.request_repaint();
            }
            !p.cancelled() && !closed.load(Ordering::Relaxed)
        });
        // A closed tab no longer listens: the dataset is dropped here.
        let _ = tx.send((
            job.source,
            job.generation,
            r.map(|ds| OpenedFile {
                ds,
                took: started.elapsed(),
                analyses,
                analysis,
            }),
        ));
        ctx.request_repaint();
    }
}

/// The data of one line.
#[derive(Clone)]
pub enum Data {
    File(Arc<Dataset>, u32),
    Series(Arc<Series>),
}

impl PartialEq for Data {
    fn eq(&self, other: &Data) -> bool {
        match (self, other) {
            (Data::File(a, i), Data::File(b, j)) => Arc::ptr_eq(a, b) && i == j,
            (Data::Series(a), Data::Series(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

impl std::fmt::Debug for Data {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Data::File(_, i) => write!(f, "File({i})"),
            Data::Series(s) => write!(f, "Series({})", s.name),
        }
    }
}

/// One line to draw.
#[derive(Debug, Clone, PartialEq)]
pub struct InstReq {
    pub key: InstKey,
    pub data: Data,
    pub stroke: Stroke,
}

/// An eye diagram: every `period` of the data from `offset` on, drawn over each other
/// on 0..period; as lines, or as a density (how many traces cross each pixel).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EyeReq {
    pub period: f64,
    pub offset: f64,
    pub density: bool,
}

/// Periods an eye draws at most; more is refused (a period of 1e-20 s over a
/// microsecond would be 1e14 traces).
pub const EYE_MAX_PERIODS: f64 = 200_000.0;
/// Samples an eye reads at most per line; beyond, the first periods are drawn.
pub const EYE_MAX_SAMPLES: u64 = 20_000_000;

/// One pane to draw: its x range (data units), axis, size in pixels, y range (or fit),
/// the lines and the cursors of its x group.
#[derive(Debug, Clone, PartialEq)]
pub struct PaneReq {
    pub pane: PaneId,
    pub x0: f64,
    pub x1: f64,
    pub log: bool,
    /// y on a logarithmic axis (a noise density): the image and `y` are in log10.
    pub y_log: bool,
    pub w: u32,
    pub h: u32,
    /// Plot coordinates (log10 when `y_log`).
    pub y: Option<(f64, f64)>,
    pub insts: Vec<InstReq>,
    pub cursors: [Option<f64>; 2],
    pub eye: Option<EyeReq>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub seq: u64,
    pub panes: Vec<PaneReq>,
}

impl Request {
    /// The same request apart from its number.
    pub fn same(&self, other: &Request) -> bool {
        self.panes == other.panes
    }
}

/// The answer for one line.
#[derive(Debug, Clone)]
pub struct InstAnswer {
    pub key: InstKey,
    /// Points the query gave.
    pub points: usize,
    /// The smallest and largest finite value drawn inside the x range.
    pub drawn: Option<(f64, f64)>,
    /// The same of the values above zero (for a log y axis).
    pub drawn_pos: Option<(f64, f64)>,
    pub at_cursor: [Option<f64>; 2],
    /// A sweep that goes back (several parts of x): the value of every part at the
    /// cursors (`at_cursor` is the first part's).
    pub parts_at_cursor: [Vec<Option<f64>>; 2],
    pub info: QueryInfo,
    pub prepared: Prepared,
    /// Something to say about the line (an eye that cannot be drawn).
    pub note: Option<String>,
}

/// The answer for one pane: the image covers `x0..x1` (plot coordinates: log10 on a log
/// axis) and `y0..y1`.
pub struct PaneReply {
    pub pane: PaneId,
    pub u0: f64,
    pub u1: f64,
    pub y0: f64,
    pub y1: f64,
    pub image: Option<egui::ColorImage>,
    pub insts: Vec<InstAnswer>,
}

pub struct Reply {
    pub request: Request,
    pub panes: Vec<PaneReply>,
    /// Time of the queries and of the drawing.
    pub query_ms: f64,
    pub raster_ms: f64,
    pub errors: Vec<String>,
}

impl Reply {
    pub fn building(&self) -> bool {
        self.panes.iter().any(|p| {
            p.insts
                .iter()
                .any(|a| matches!(a.prepared, Prepared::Building(_)))
        })
    }

    pub fn pane(&self, id: PaneId) -> Option<&PaneReply> {
        self.panes.iter().find(|p| p.pane == id)
    }

    pub fn inst(&self, key: InstKey) -> Option<&InstAnswer> {
        self.panes
            .iter()
            .flat_map(|p| &p.insts)
            .find(|a| a.key == key)
    }
}

enum Msg {
    Draw(Request),
    /// The datasets whose files are watched for changes (once a second).
    Watch(Vec<(SourceId, Arc<Dataset>)>),
}

/// What the query thread sends.
pub enum Answer {
    Drawn(Reply),
    /// Sources whose file changed on disk (all of them, each time the set changes).
    Changed(Vec<SourceId>),
}

/// The query thread of a tab. Requests that pile up while one runs are skipped: only
/// the newest is answered. Dropping the worker ends the thread.
pub struct Worker {
    requests: Option<Sender<Msg>>,
    replies: Receiver<Answer>,
    thread: Option<JoinHandle<()>>,
    /// Requests sent and not answered yet.
    pub pending: Arc<AtomicUsize>,
}

impl Worker {
    pub fn start(ctx: &egui::Context) -> Worker {
        let (req_tx, req_rx) = mpsc::channel::<Msg>();
        let (rep_tx, replies) = mpsc::channel();
        let ctx = ctx.clone();
        let pending = Arc::new(AtomicUsize::new(0));
        let p = pending.clone();
        let thread = std::thread::Builder::new()
            .name("cdept-wave-query".into())
            .spawn(move || serve(&req_rx, &rep_tx, &ctx, &p))
            .ok();
        Worker {
            requests: Some(req_tx),
            replies,
            thread,
            pending,
        }
    }

    pub fn send(&self, request: Request) {
        if let Some(tx) = &self.requests {
            self.pending.fetch_add(1, Ordering::SeqCst);
            if tx.send(Msg::Draw(request)).is_err() {
                self.pending.fetch_sub(1, Ordering::SeqCst);
            }
        }
    }

    pub fn watch(&self, datasets: Vec<(SourceId, Arc<Dataset>)>) {
        if let Some(tx) = &self.requests {
            let _ = tx.send(Msg::Watch(datasets));
        }
    }

    /// Everything that came, oldest first.
    pub fn answers(&self) -> Vec<Answer> {
        self.replies.try_iter().collect()
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        // Closing the channel ends the loop; the thread is not waited for (a long
        // query would hold the UI), it ends after its present request.
        self.requests.take();
        self.thread.take();
    }
}

fn serve(
    requests: &Receiver<Msg>,
    replies: &Sender<Answer>,
    ctx: &egui::Context,
    pending: &AtomicUsize,
) {
    let mut prepared_for: HashMap<usize, Vec<u32>> = HashMap::new();
    let mut watched: Vec<(SourceId, Arc<Dataset>)> = Vec::new();
    let mut changed: Vec<SourceId> = Vec::new();
    let mut checked = Instant::now();
    loop {
        let msg = match requests.recv_timeout(Duration::from_secs(1)) {
            Ok(m) => Some(m),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => return,
        };
        let mut draw = None;
        let mut skipped = 0;
        for m in msg.into_iter().chain(requests.try_iter()) {
            match m {
                Msg::Draw(r) => {
                    if draw.replace(r).is_some() {
                        skipped += 1;
                    }
                }
                Msg::Watch(w) => watched = w,
            }
        }
        if skipped > 0 {
            pending.fetch_sub(skipped, Ordering::SeqCst);
        }
        if checked.elapsed() >= Duration::from_secs(1) {
            checked = Instant::now();
            let now: Vec<SourceId> = watched
                .iter()
                .filter(|(_, ds)| ds.source_changed())
                .map(|(s, _)| *s)
                .collect();
            if now != changed {
                changed = now.clone();
                if replies.send(Answer::Changed(now)).is_err() {
                    return;
                }
                ctx.request_repaint();
            }
        }
        let Some(request) = draw else { continue };
        let reply = answer(request, &mut prepared_for);
        pending.fetch_sub(1, Ordering::SeqCst);
        if replies.send(Answer::Drawn(reply)).is_err() {
            return;
        }
        ctx.request_repaint();
    }
}

/// Threads for the queries and the drawing of one request.
fn threads(work: usize) -> usize {
    let cores = std::thread::available_parallelism().map_or(2, |n| n.get());
    cores.clamp(1, 4).min(work.max(1))
}

struct Queried {
    trace: Trace,
    answer: InstAnswer,
}

fn query_one(inst: &InstReq, pane: &PaneReq) -> (Queried, Option<String>) {
    let mut trace = Trace::default();
    let mut error = None;
    let mut parts_at: [Vec<Option<f64>>; 2] = [vec![], vec![]];
    let (info, at, prepared) = match &inst.data {
        Data::File(ds, signal) if ds.x_order() == XOrder::Segmented => {
            // every part of x, one after the other, broken between them
            let mut info = QueryInfo::default();
            let mut part = Trace::default();
            for (k, _) in ds.segments().iter().enumerate() {
                match ds.query_segment(*signal, k as u32, pane.x0, pane.x1, pane.w.max(1), &mut part)
                {
                    Ok(i) => {
                        info.source_points += i.source_points;
                        info.mode = i.mode;
                        info.ms += i.ms;
                    }
                    Err(e) => {
                        error.get_or_insert(e.to_string());
                        continue;
                    }
                }
                if !trace.x.is_empty() {
                    trace.x.push(f64::NAN);
                    trace.y.push(f64::NAN);
                }
                trace.x.extend_from_slice(&part.x);
                trace.y.extend_from_slice(&part.y);
                for (part, cursor) in parts_at.iter_mut().zip(pane.cursors) {
                    part.push(cursor.and_then(|x| {
                        ds.value_at_segment(*signal, k as u32, x).ok().map(|v| v.0)
                    }));
                }
            }
            let first = |c: usize| parts_at[c].first().copied().flatten();
            (info, [first(0), first(1)], ds.prepared(*signal))
        }
        Data::File(ds, signal) => {
            let info = match ds.query(*signal, pane.x0, pane.x1, pane.w.max(1), &mut trace) {
                Ok(i) => i,
                Err(e) => {
                    error = Some(e.to_string());
                    QueryInfo::default()
                }
            };
            let unordered = ds.x_order() == XOrder::Unordered;
            let at = |c: Option<f64>| {
                c.filter(|_| !unordered)
                    .and_then(|x| ds.value_at(*signal, x).ok().map(|v| v.0))
            };
            (
                info,
                [at(pane.cursors[0]), at(pane.cursors[1])],
                ds.prepared(*signal),
            )
        }
        Data::Series(s) => {
            let info = s.query(pane.x0, pane.x1, pane.w.max(1), &mut trace);
            let at = |c: Option<f64>| c.and_then(|x| s.value_at(x));
            (
                info,
                [at(pane.cursors[0]), at(pane.cursors[1])],
                Prepared::Ready,
            )
        }
    };
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    let (mut plo, mut phi) = (f64::INFINITY, f64::NEG_INFINITY);
    let unordered = match &inst.data {
        Data::Series(s) => !s.increasing(),
        Data::File(ds, _) => ds.x_order() == XOrder::Unordered,
    };
    let (xa, xb) = (pane.x0.min(pane.x1), pane.x0.max(pane.x1));
    for (x, y) in trace.x.iter().zip(&trace.y) {
        if (unordered || (*x >= xa && *x <= xb)) && y.is_finite() {
            lo = lo.min(*y);
            hi = hi.max(*y);
            if *y > 0.0 {
                plo = plo.min(*y);
                phi = phi.max(*y);
            }
        }
    }
    let answer = InstAnswer {
        key: inst.key,
        points: trace.x.len(),
        drawn: (lo <= hi).then_some((lo, hi)),
        drawn_pos: (plo <= phi).then_some((plo, phi)),
        at_cursor: at,
        parts_at_cursor: parts_at,
        info,
        prepared,
        note: None,
    };
    (Queried { trace, answer }, error)
}

/// The samples of a line over its whole x range, read in pieces: x and y of every
/// sample from `first` on, given to `f` piece by piece until it returns false.
fn stream(data: &Data, max: u64, mut f: impl FnMut(&[f64], &[f64]) -> bool) -> Result<u64, String> {
    match data {
        Data::Series(s) => {
            let n = (s.len() as u64).min(max) as usize;
            f(&s.x()[..n], &s.y()[..n]);
            Ok(n as u64)
        }
        Data::File(ds, sig) => {
            let total = ds.point_count().min(max);
            const PIECE: usize = 1 << 20;
            let (mut xs, mut ys) = (vec![0.0; PIECE], vec![0.0; PIECE]);
            let mut at = 0u64;
            while at < total {
                let n = ((total - at) as usize).min(PIECE);
                ds.samples(0, at, &mut xs[..n]).map_err(|e| e.to_string())?;
                ds.samples(*sig, at, &mut ys[..n]).map_err(|e| e.to_string())?;
                if !f(&xs[..n], &ys[..n]) {
                    break;
                }
                at += n as u64;
            }
            Ok(at)
        }
    }
}

/// The x extent of a line's data and its number of samples.
fn extent(data: &Data) -> Option<(f64, f64, u64)> {
    match data {
        Data::Series(s) => s.x_range().map(|(a, b)| (a, b, s.len() as u64)),
        Data::File(ds, _) => ds
            .x_extent()
            .ok()
            .map(|(a, b)| (a, b, ds.point_count())),
    }
}

/// Whether an eye of `eye` over data of x `lo..hi` with `n` samples can be drawn; the
/// periods it has.
pub fn eye_check(eye: &EyeReq, lo: f64, hi: f64, n: u64) -> Result<f64, String> {
    let p = eye.period;
    if !(p.is_finite() && p > 0.0) {
        return Err("the eye period must be a positive number".into());
    }
    if !(eye.offset.is_finite()) {
        return Err("the eye offset is not a number".into());
    }
    let span = hi - lo;
    if !(span.is_finite() && span > 0.0) || n < 2 {
        return Err("the signal has no x range to fold".into());
    }
    let periods = span / p;
    if periods > EYE_MAX_PERIODS {
        return Err(format!(
            "period {p:e} is too short: {periods:.3e} periods (at most {EYE_MAX_PERIODS})"
        ));
    }
    let spacing = span / (n - 1) as f64;
    if p < 2.0 * spacing {
        return Err(format!(
            "period {p:e} is shorter than two samples ({spacing:e} apart)"
        ));
    }
    Ok(periods.max(1.0))
}

/// An eye line: the samples folded by the period, each period a polyline over a period
/// `fw` pixels wide moved left by `shift` pixels (a zoom inside the period); or counted
/// into `density` (w x h, row by row) when given.
#[allow(clippy::too_many_arguments)]
fn draw_eye_line(
    inst: &InstReq,
    eye: &EyeReq,
    (w, h): (usize, usize),
    fw: f64,
    shift: f64,
    y: (f64, f64),
    canvas: Option<&mut Canvas>,
    density: Option<&mut [u32]>,
) -> Result<String, String> {
    let (lo, hi, n) = extent(&inst.data).ok_or("no data")?;
    let periods = eye_check(eye, lo, hi, n)?;
    let (y0, y1) = y;
    let fh = h as f64;
    let mut canvas = canvas;
    let mut density = density;
    let mut pts: Vec<(f64, f64)> = Vec::with_capacity(4096);
    let mut current: Option<f64> = None;
    let mut traces = 0u64;
    let flush = |pts: &mut Vec<(f64, f64)>, canvas: &mut Option<&mut Canvas>, density: &mut Option<&mut [u32]>| {
        if pts.len() > 1 {
            if let Some(c) = canvas.as_deref_mut() {
                c.polyline(pts, &inst.stroke);
            }
            if let Some(d) = density.as_deref_mut() {
                for s in pts.windows(2) {
                    count_segment(d, w, h, s[0], s[1]);
                }
            }
        }
        pts.clear();
    };
    let read = stream(&inst.data, EYE_MAX_SAMPLES, |xs, ys| {
        for (x, v) in xs.iter().zip(ys) {
            let t = (x - eye.offset) / eye.period;
            if !t.is_finite() || t < 0.0 {
                continue;
            }
            let k = t.floor();
            if current != Some(k) {
                // the next period starts: the line goes on to the first sample of it,
                // drawn past the right edge, so no gap is left at the period's end
                if let Some(last) = pts.last().copied()
                    && current == Some(k - 1.0)
                {
                    let u = (t - k + 1.0) * fw - shift;
                    let p = (u, (y1 - v) / (y1 - y0) * fh);
                    pts.push(p);
                    flush(&mut pts, &mut canvas, &mut density);
                    // and the new period starts with the last sample, before the left edge
                    pts.push((last.0 - fw, last.1));
                } else {
                    flush(&mut pts, &mut canvas, &mut density);
                }
                current = Some(k);
                traces += 1;
            }
            let u = (t - k) * fw - shift;
            pts.push((u, (y1 - v) / (y1 - y0) * fh));
        }
        true
    })?;
    flush(&mut pts, &mut canvas, &mut density);
    let mut note = format!("{traces} traces of {periods:.0} periods");
    if read < n {
        note.push_str(&format!(
            " (the first {read} of {n} samples: at most {EYE_MAX_SAMPLES})"
        ));
    }
    Ok(note)
}

/// Count the pixels a segment crosses (one per column or row step).
fn count_segment(d: &mut [u32], w: usize, h: usize, a: (f64, f64), b: (f64, f64)) {
    if !(a.0.is_finite() && a.1.is_finite() && b.0.is_finite() && b.1.is_finite()) {
        return;
    }
    let steps = ((b.0 - a.0).abs().max((b.1 - a.1).abs()).ceil() as usize).clamp(1, 4 * (w + h));
    for k in 0..steps {
        let t = k as f64 / steps as f64;
        let (x, y) = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
        if x >= 0.0 && y >= 0.0 && (x as usize) < w && (y as usize) < h {
            let i = y as usize * w + x as usize;
            d[i] = d[i].saturating_add(1);
        }
    }
}

/// A density as an image: no traces transparent, the rest from dim to bright on a log
/// scale (a few traces stay visible next to thousands).
fn density_image(d: &[u32], w: usize, h: usize, rgb: [u8; 3]) -> Vec<u8> {
    let max = d.iter().copied().max().unwrap_or(0).max(1) as f64;
    let mut out = vec![0u8; w * h * 4];
    // colour map: the line's colour at its densest, through a dark violet when sparse
    let stops: [[f64; 3]; 3] = [
        [70.0, 30.0, 140.0],
        [rgb[0] as f64, rgb[1] as f64, rgb[2] as f64],
        [255.0, 255.0, 255.0],
    ];
    for (i, c) in d.iter().enumerate() {
        if *c == 0 {
            continue;
        }
        let t = ((*c as f64).ln() + 1.0) / (max.ln() + 1.0);
        let (a, b, f) = if t < 0.7 {
            (stops[0], stops[1], t / 0.7)
        } else {
            (stops[1], stops[2], (t - 0.7) / 0.3)
        };
        let alpha = (0.55 + 0.45 * t).min(1.0);
        for ch in 0..3 {
            out[i * 4 + ch] = ((a[ch] + (b[ch] - a[ch]) * f) * alpha) as u8;
        }
        out[i * 4 + 3] = (alpha * 255.0) as u8;
    }
    out
}

/// The y range that shows `drawn` with a margin.
pub fn fit_y(drawn: impl Iterator<Item = (f64, f64)>) -> (f64, f64) {
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for (a, b) in drawn {
        lo = lo.min(a);
        hi = hi.max(b);
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

fn answer(request: Request, prepared_for: &mut HashMap<usize, Vec<u32>>) -> Reply {
    let started = Instant::now();
    let mut errors = Vec::new();
    // Pyramids for the large files, for the signals drawn from them.
    let mut wanted: HashMap<usize, (Arc<Dataset>, Vec<u32>)> = HashMap::new();
    for p in &request.panes {
        for i in &p.insts {
            if let Data::File(ds, s) = &i.data
                && ds.point_count() >= PREPARE_MIN_POINTS
            {
                let e = wanted
                    .entry(Arc::as_ptr(ds) as usize)
                    .or_insert_with(|| (ds.clone(), vec![]));
                if !e.1.contains(s) {
                    e.1.push(*s);
                }
            }
        }
    }
    if wanted.len() > PREPARE_MAX_FILES {
        wanted.clear();
    }
    // A dataset no longer drawn (or closed) is forgotten: its address may come again.
    prepared_for.retain(|k, _| wanted.contains_key(k));
    for (key, (ds, mut signals)) in wanted {
        signals.sort_unstable();
        if prepared_for.get(&key) != Some(&signals) {
            if let Err(e) = ds.prepare(&signals) {
                errors.push(e.to_string());
            }
            prepared_for.insert(key, signals);
        }
    }
    // Every line of every pane, spread over a few threads.
    let jobs: Vec<(usize, usize)> = request
        .panes
        .iter()
        .enumerate()
        .flat_map(|(pi, p)| (0..p.insts.len()).map(move |ii| (pi, ii)))
        .collect();
    let n = threads(jobs.len());
    let chunk = jobs.len().div_ceil(n).max(1);
    type Part = Vec<(usize, usize, Queried, Option<String>)>;
    let mut results: Vec<Part> = Vec::new();
    std::thread::scope(|scope| {
        let handles: Vec<_> = jobs
            .chunks(chunk)
            .map(|part| {
                let req = &request;
                scope.spawn(move || {
                    part.iter()
                        .map(|&(pi, ii)| {
                            let pane = &req.panes[pi];
                            let inst = &pane.insts[ii];
                            // an eye: the extremes of the whole line (it is all folded)
                            let (q, e) = match (pane.eye, extent(&inst.data)) {
                                (Some(_), Some((lo, hi, _))) => {
                                    let whole = PaneReq {
                                        x0: lo,
                                        x1: hi,
                                        cursors: [None, None],
                                        insts: vec![],
                                        ..pane.clone()
                                    };
                                    query_one(inst, &whole)
                                }
                                _ => query_one(inst, pane),
                            };
                            (pi, ii, q, e)
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        for h in handles {
            if let Ok(r) = h.join() {
                results.push(r);
            }
        }
    });
    let mut per_pane: Vec<Vec<Option<Queried>>> = request
        .panes
        .iter()
        .map(|p| (0..p.insts.len()).map(|_| None).collect())
        .collect();
    for (pi, ii, q, e) in results.into_iter().flatten() {
        if let Some(e) = e
            && errors.len() < 8
        {
            errors.push(e);
        }
        per_pane[pi][ii] = Some(q);
    }
    let query_ms = started.elapsed().as_secs_f64() * 1e3;
    // Draw the panes, a few at a time.
    let drawn_at = Instant::now();
    let n = threads(request.panes.len());
    let mut panes: Vec<Option<PaneReply>> = (0..request.panes.len()).map(|_| None).collect();
    std::thread::scope(|scope| {
        let mut handles = Vec::new();
        let mut work: Vec<(usize, Vec<Option<Queried>>)> =
            per_pane.into_iter().enumerate().collect();
        let per = work.len().div_ceil(n).max(1);
        while !work.is_empty() {
            let part: Vec<_> = work.drain(..per.min(work.len())).collect();
            let req = &request;
            handles.push(scope.spawn(move || {
                part.into_iter()
                    .map(|(pi, q)| (pi, draw_pane(&req.panes[pi], q)))
                    .collect::<Vec<_>>()
            }));
        }
        for h in handles {
            if let Ok(r) = h.join() {
                for (pi, p) in r {
                    panes[pi] = Some(p);
                }
            }
        }
    });
    Reply {
        panes: panes.into_iter().flatten().collect(),
        request,
        query_ms,
        raster_ms: drawn_at.elapsed().as_secs_f64() * 1e3,
        errors,
    }
}

fn draw_pane(pane: &PaneReq, queried: Vec<Option<Queried>>) -> PaneReply {
    let mut queried: Vec<Queried> = queried.into_iter().flatten().collect();
    let ylog = |v: f64| if pane.y_log { v.log10() } else { v };
    let (y0, y1) = pane.y.unwrap_or_else(|| {
        fit_y(queried.iter().filter_map(|q| {
            if pane.y_log {
                q.answer.drawn_pos.map(|(a, b)| (a.log10(), b.log10()))
            } else {
                q.answer.drawn
            }
        }))
    });
    let map = |x: f64| if pane.log { x.log10() } else { x };
    let (u0, u1) = match pane.eye {
        Some(e) => (0.0, e.period),
        None => (map(pane.x0), map(pane.x1)),
    };
    let ok = pane.w > 0 && pane.h > 0 && u1 > u0 && y1 > y0;
    if let (Some(eye), true) = (pane.eye, ok) {
        // the view inside the period (zoomed): drawn over the whole period, the
        // pixels outside the view cut off by the plot
        let (w, h) = (pane.w as usize, pane.h as usize);
        let (vx0, vx1) = (pane.x0.max(0.0), pane.x1.min(eye.period));
        let (vx0, vx1) = if vx1 > vx0 { (vx0, vx1) } else { (0.0, eye.period) };
        let full_w = ((w as f64) * eye.period / (vx1 - vx0)).clamp(1.0, 1e6);
        let shift = vx0 / eye.period * full_w;
        let mut canvas = Canvas::new(w, h);
        let mut density = vec![0u32; if eye.density { w * h } else { 0 }];
        let mut colour = [255u8, 255, 255];
        for (q, inst) in queried.iter_mut().zip(&pane.insts) {
            colour = [inst.stroke.rgba[0], inst.stroke.rgba[1], inst.stroke.rgba[2]];
            let r = draw_eye_line(
                inst,
                &eye,
                (w, h),
                full_w,
                shift,
                (y0, y1),
                (!eye.density).then_some(&mut canvas),
                eye.density.then_some(&mut density[..]),
            );
            match r {
                Ok(note) => q.answer.note = Some(note),
                Err(e) => q.answer.note = Some(format!("no eye: {e}")),
            }
        }
        let rgba = if eye.density {
            density_image(&density, w, h, colour)
        } else {
            canvas.to_rgba()
        };
        let (w, h) = if eye.density { (w, h) } else { (canvas.w, canvas.h) };
        return PaneReply {
            pane: pane.pane,
            u0: vx0,
            u1: vx1,
            y0,
            y1,
            image: Some(egui::ColorImage::from_rgba_premultiplied([w, h], &rgba)),
            insts: queried.into_iter().map(|q| q.answer).collect(),
        };
    }
    let image = ok.then(|| {
        let (w, h) = (pane.w as usize, pane.h as usize);
        let draw =
            |part: &[(&Queried, &InstReq)]| {
                let mut canvas = Canvas::new(w, h);
                let (fw, fh) = (canvas.w as f64, canvas.h as f64);
                let mut pts: Vec<(f64, f64)> = Vec::new();
                for (q, inst) in part {
                    pts.clear();
                    pts.extend(q.trace.x.iter().zip(&q.trace.y).map(|(x, y)| {
                        ((map(*x) - u0) / (u1 - u0) * fw, (y1 - ylog(*y)) / (y1 - y0) * fh)
                    }));
                    canvas.polyline(&pts, &inst.stroke);
                }
                canvas
            };
        let lines: Vec<(&Queried, &InstReq)> = queried.iter().zip(&pane.insts).collect();
        // Many lines: drawn in parts on a few threads, the parts put over each other in
        // order (the same picture as drawing them one after the other).
        let parts = threads(lines.len() / 4);
        let canvas = if parts <= 1 {
            draw(&lines)
        } else {
            let per = lines.len().div_ceil(parts);
            let mut drawn: Vec<Canvas> = std::thread::scope(|scope| {
                let handles: Vec<_> = lines
                    .chunks(per)
                    .map(|part| scope.spawn(move || draw(part)))
                    .collect();
                handles.into_iter().filter_map(|h| h.join().ok()).collect()
            });
            let mut base = drawn.remove(0);
            for top in &drawn {
                base.over(top);
            }
            base
        };
        egui::ColorImage::from_rgba_premultiplied([canvas.w, canvas.h], &canvas.to_rgba())
    });
    PaneReply {
        pane: pane.pane,
        u0,
        u1,
        y0,
        y1,
        image,
        insts: queried.into_iter().map(|q| q.answer).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::views::wave::model::SigRef;
    use crate::views::wave::raster::Dash;

    fn series(name: &str, n: usize, f: impl Fn(usize) -> f64) -> Arc<Series> {
        Arc::new(Series::new(
            name,
            "V",
            "s",
            (0..n).map(|i| i as f64).collect(),
            (0..n).map(f).collect(),
        ))
    }

    #[test]
    fn panes_are_queried_fitted_and_drawn() {
        let a = series("a", 100_000, |i| (i as f64 * 1e-3).sin());
        let b = series("b", 10, |i| if i == 5 { f64::NAN } else { 3.0 });
        let empty = Arc::new(Series::new("e", "", "s", vec![], vec![]));
        let stroke = Stroke {
            rgba: [255, 255, 0, 255],
            width: 1.5,
            dash: Dash::Solid,
        };
        let inst = |s: &Arc<Series>, i: u32| InstReq {
            key: InstKey {
                source: 1,
                sig: SigRef::Extra(i),
            },
            data: Data::Series(s.clone()),
            stroke,
        };
        let req = Request {
            seq: 1,
            panes: vec![
                PaneReq {
                    pane: 1,
                    x0: 0.0,
                    x1: 99_999.0,
                    log: false,
                    y_log: false,
                    eye: None,
                    w: 300,
                    h: 100,
                    y: None,
                    insts: vec![inst(&a, 0), inst(&empty, 2)],
                    cursors: [Some(1000.0), None],
                },
                PaneReq {
                    pane: 2,
                    x0: 0.0,
                    x1: 9.0,
                    log: false,
                    y_log: false,
                    eye: None,
                    w: 0,
                    h: 0,
                    y: Some((0.0, 1.0)),
                    insts: vec![inst(&b, 1)],
                    cursors: [None, None],
                },
            ],
        };
        let mut prep = HashMap::new();
        let r = answer(req, &mut prep);
        assert_eq!(r.panes.len(), 2);
        let p1 = r.pane(1).unwrap();
        assert!(p1.image.is_some());
        assert!(p1.y0 < -0.99 && p1.y1 > 0.99, "{} {}", p1.y0, p1.y1);
        let ia = &p1.insts[0];
        assert!(ia.points <= 4 * 300 + 8);
        assert!((ia.at_cursor[0].unwrap() - 1f64.sin()).abs() < 1e-9);
        assert_eq!(p1.insts[1].points, 0);
        assert!(p1.insts[1].drawn.is_none());
        // a pane of no size gives no image, but its answers
        let p2 = r.pane(2).unwrap();
        assert!(p2.image.is_none());
        assert_eq!(p2.insts[0].drawn, Some((3.0, 3.0)));
    }

    #[test]
    fn eyes_are_bounded_and_folded() {
        let e = |period| EyeReq {
            period,
            offset: 0.0,
            density: false,
        };
        // 1e-20 s over 1 us: refused at once, nothing allocated
        assert!(eye_check(&e(1e-20), 0.0, 1e-6, 1_000_000).unwrap_err().contains("too short"));
        assert!(eye_check(&e(f64::NAN), 0.0, 1e-6, 100).is_err());
        assert!(eye_check(&e(-1.0), 0.0, 1e-6, 100).is_err());
        assert!(eye_check(&e(1e-9), 0.0, 0.0, 100).is_err());
        // a period of less than two samples
        assert!(eye_check(&e(1e-9), 0.0, 1e-6, 100).unwrap_err().contains("two samples"));
        assert_eq!(eye_check(&e(1e-7), 0.0, 1e-6, 10_001).unwrap(), 10.0);
        // a square wave of period 10 drawn folded: lines and density
        let s = series("sq", 10_000, |i| if (i / 5) % 2 == 0 { 1.0 } else { 0.0 });
        for density in [false, true] {
            let req = Request {
                seq: 1,
                panes: vec![PaneReq {
                    pane: 1,
                    x0: 0.0,
                    x1: 10.0,
                    log: false,
                    y_log: false,
                    w: 200,
                    h: 100,
                    y: None,
                    insts: vec![InstReq {
                        key: InstKey {
                            source: 1,
                            sig: SigRef::Extra(0),
                        },
                        data: Data::Series(s.clone()),
                        stroke: Stroke {
                            rgba: [255, 0, 0, 255],
                            width: 1.0,
                            dash: Dash::Solid,
                        },
                    }],
                    cursors: [None, None],
                    eye: Some(EyeReq {
                        period: 10.0,
                        offset: 0.0,
                        density,
                    }),
                }],
            };
            let r = answer(req, &mut HashMap::new());
            let p = r.pane(1).unwrap();
            let note = p.insts[0].note.clone().unwrap();
            assert!(note.starts_with("1000 traces"), "{note}");
            assert!(p.y0 < 0.0 && p.y1 > 1.0);
            let img = p.image.as_ref().unwrap();
            let lit = img.pixels.iter().filter(|c| c.a() > 0).count();
            assert!(lit > 300, "density {density}: {lit} pixels");
        }
        // the absurd period: a note, no image work
        let req = Request {
            seq: 2,
            panes: vec![PaneReq {
                pane: 1,
                x0: 0.0,
                x1: 1e-20,
                log: false,
                y_log: false,
                w: 200,
                h: 100,
                y: None,
                insts: vec![InstReq {
                    key: InstKey {
                        source: 1,
                        sig: SigRef::Extra(0),
                    },
                    data: Data::Series(s),
                    stroke: Stroke {
                        rgba: [255, 0, 0, 255],
                        width: 1.0,
                        dash: Dash::Solid,
                    },
                }],
                cursors: [None, None],
                eye: Some(e(1e-20)),
            }],
        };
        let started = Instant::now();
        let r = answer(req, &mut HashMap::new());
        assert!(started.elapsed() < Duration::from_secs(2));
        let note = r.pane(1).unwrap().insts[0].note.clone().unwrap();
        assert!(note.contains("too short"), "{note}");
    }

    #[test]
    fn analyses_are_chosen() {
        let a = |i: u32, name: &str, kind: &str, points: u64, preferred: bool| Analysis {
            index: i,
            name: name.into(),
            kind: kind.into(),
            points,
            signals: 3,
            complex: kind == "ac",
            plottable: points > 0,
            preferred,
            reason: String::new(),
        };
        let list = vec![
            a(0, "AC Analysis", "ac", 81, false),
            a(1, "Operating Point", "op", 1, false),
            a(2, "Transient Analysis", "tran", 500, false),
        ];
        assert_eq!(choose(&list, &Want::Best), 2);
        assert_eq!(choose(&list, &Want::Index(1)), 1);
        let like = Want::Like {
            name: "x".into(),
            kind: "ac".into(),
        };
        assert_eq!(choose(&list, &like), 0);
        let psf = vec![a(0, "dc2", "dc", 0, false), a(1, "ac1", "ac", 30, true)];
        assert_eq!(choose(&psf, &Want::Best), 1);
        assert_eq!(choose(&[], &Want::Best), 0);
    }

    #[test]
    fn fit_y_of_nothing_flat_and_normal() {
        assert_eq!(fit_y(std::iter::empty()), (-1.0, 1.0));
        assert_eq!(fit_y([(0.0, 0.0)].into_iter()), (-1.0, 1.0));
        let (a, b) = fit_y([(1.0, 1.0)].into_iter());
        assert!((a - 0.9).abs() < 1e-12 && (b - 1.1).abs() < 1e-12);
        let (a, b) = fit_y([(0.0, 1.0), (-1.0, 0.5)].into_iter());
        assert!(a < -1.0 && b > 1.0);
    }
}
