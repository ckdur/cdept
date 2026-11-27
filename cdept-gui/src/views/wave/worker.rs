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

use cdept_wavedata::{Dataset, OpenOptions, Phase, Prepared, QueryInfo, Series, Trace};

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

pub type Opened = Result<(Dataset, Duration), cdept_wavedata::Error>;

struct Job {
    source: SourceId,
    generation: u64,
    path: PathBuf,
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
    pub fn open(&self, source: SourceId, generation: u64, path: PathBuf) -> Arc<LoadProgress> {
        let progress = Arc::new(LoadProgress::default());
        let mut q = self.queue.lock().unwrap_or_else(|e| e.into_inner());
        q.0.push_back(Job {
            source,
            generation,
            path,
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
        let options = OpenOptions {
            cache_dir: cache_dir(),
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
            r.map(|ds| (ds, started.elapsed())),
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

/// One pane to draw: its x range (data units), axis, size in pixels, y range (or fit),
/// the lines and the cursors of its x group.
#[derive(Debug, Clone, PartialEq)]
pub struct PaneReq {
    pub pane: PaneId,
    pub x0: f64,
    pub x1: f64,
    pub log: bool,
    pub w: u32,
    pub h: u32,
    pub y: Option<(f64, f64)>,
    pub insts: Vec<InstReq>,
    pub cursors: [Option<f64>; 2],
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
    pub at_cursor: [Option<f64>; 2],
    pub info: QueryInfo,
    pub prepared: Prepared,
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
    let (info, at, prepared) = match &inst.data {
        Data::File(ds, signal) => {
            let info = match ds.query(*signal, pane.x0, pane.x1, pane.w.max(1), &mut trace) {
                Ok(i) => i,
                Err(e) => {
                    error = Some(e.to_string());
                    QueryInfo::default()
                }
            };
            let at = |c: Option<f64>| c.and_then(|x| ds.value_at(*signal, x).ok().map(|v| v.0));
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
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    let unordered = matches!(&inst.data, Data::Series(s) if !s.increasing());
    for (x, y) in trace.x.iter().zip(&trace.y) {
        if (unordered || (*x >= pane.x0 && *x <= pane.x1)) && y.is_finite() {
            lo = lo.min(*y);
            hi = hi.max(*y);
        }
    }
    let answer = InstAnswer {
        key: inst.key,
        points: trace.x.len(),
        drawn: (lo <= hi).then_some((lo, hi)),
        at_cursor: at,
        info,
        prepared,
    };
    (Queried { trace, answer }, error)
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
                            let (q, e) = query_one(&pane.insts[ii], pane);
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
    let queried: Vec<Queried> = queried.into_iter().flatten().collect();
    let (y0, y1) = pane
        .y
        .unwrap_or_else(|| fit_y(queried.iter().filter_map(|q| q.answer.drawn)));
    let map = |x: f64| if pane.log { x.log10() } else { x };
    let (u0, u1) = (map(pane.x0), map(pane.x1));
    let image = (pane.w > 0 && pane.h > 0 && u1 > u0 && y1 > y0).then(|| {
        let (w, h) = (pane.w as usize, pane.h as usize);
        let draw =
            |part: &[(&Queried, &InstReq)]| {
                let mut canvas = Canvas::new(w, h);
                let (fw, fh) = (canvas.w as f64, canvas.h as f64);
                let mut pts: Vec<(f64, f64)> = Vec::new();
                for (q, inst) in part {
                    pts.clear();
                    pts.extend(q.trace.x.iter().zip(&q.trace.y).map(|(x, y)| {
                        ((map(*x) - u0) / (u1 - u0) * fw, (y1 - y) / (y1 - y0) * fh)
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
    fn fit_y_of_nothing_flat_and_normal() {
        assert_eq!(fit_y(std::iter::empty()), (-1.0, 1.0));
        assert_eq!(fit_y([(0.0, 0.0)].into_iter()), (-1.0, 1.0));
        let (a, b) = fit_y([(1.0, 1.0)].into_iter());
        assert!((a - 0.9).abs() < 1e-12 && (b - 1.1).abs() < 1e-12);
        let (a, b) = fit_y([(0.0, 1.0), (-1.0, 0.5)].into_iter());
        assert!(a < -1.0 && b > 1.0);
    }
}
