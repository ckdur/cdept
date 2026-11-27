//! The threads of a waveform tab: one opens the file (it may convert a large raw file
//! to its `.uwraw` cache), one answers the queries of the view. The UI thread never
//! calls the engine: it sends requests and draws the newest answer.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use cdept_wavedata::{Dataset, OpenOptions, Phase, Prepared, QueryInfo, Trace};

/// Where caches go when the folder of a raw file is not writable.
pub fn cache_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(base.join("cdept").join("waveforms"))
}

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

/// An open in progress. Dropping it cancels the open (closing a tab mid-load).
pub struct Loading {
    pub progress: Arc<LoadProgress>,
    result: Receiver<Opened>,
    pub started: Instant,
}

impl Loading {
    pub fn start(ctx: &egui::Context, path: PathBuf) -> Loading {
        let progress = Arc::new(LoadProgress::default());
        let (tx, result) = mpsc::channel();
        let (p, ctx) = (progress.clone(), ctx.clone());
        let spawned = std::thread::Builder::new()
            .name("cdept-wave-open".into())
            .spawn(move || {
                let started = Instant::now();
                let options = OpenOptions {
                    cache_dir: cache_dir(),
                    ..Default::default()
                };
                let mut last = Instant::now();
                let r = Dataset::open(&path, &options, |phase, fraction| {
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
                    if last.elapsed() > Duration::from_millis(50) {
                        last = Instant::now();
                        ctx.request_repaint();
                    }
                    !p.cancelled()
                });
                // A closed tab no longer listens: the dataset is dropped here.
                let _ = tx.send(r.map(|ds| (ds, started.elapsed())));
                ctx.request_repaint();
            });
        if let Err(e) = spawned {
            let (tx, rx) = mpsc::channel();
            let _ = tx.send(Err(cdept_wavedata::Error {
                status: cdept_wavedata::Status::Internal,
                message: format!("cannot start a thread: {e}"),
            }));
            return Loading {
                progress,
                result: rx,
                started: Instant::now(),
            };
        }
        Loading {
            progress,
            result,
            started: Instant::now(),
        }
    }

    pub fn poll(&mut self) -> Option<Opened> {
        match self.result.try_recv() {
            Ok(r) => Some(r),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(cdept_wavedata::Error {
                status: cdept_wavedata::Status::Internal,
                message: "the loading thread ended without an answer".into(),
            })),
        }
    }
}

impl Drop for Loading {
    fn drop(&mut self) {
        self.progress.cancel();
    }
}

/// What the view wants drawn.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub seq: u64,
    pub x0: f64,
    pub x1: f64,
    pub pixels: u32,
    pub signals: Vec<u32>,
    pub cursors: [Option<f64>; 2],
}

/// The engine's answer for one signal.
#[derive(Debug, Clone)]
pub struct Answer {
    pub signal: u32,
    pub trace: Trace,
    pub info: QueryInfo,
    /// The values at cursor A and B.
    pub at_cursor: [Option<f64>; 2],
}

#[derive(Debug, Clone)]
pub struct Reply {
    pub request: Request,
    pub answers: Vec<Answer>,
    pub prepared: Vec<(u32, Prepared)>,
    /// Time the whole request took on the worker.
    pub ms: f64,
    pub error: Option<String>,
}

impl Reply {
    pub fn building(&self) -> bool {
        self.prepared
            .iter()
            .any(|(_, p)| matches!(p, Prepared::Building(_)))
    }
}

/// The query thread of a tab. Requests that pile up while one runs are skipped: only
/// the newest is answered. Dropping the worker ends the thread.
pub struct Worker {
    requests: Option<Sender<Request>>,
    replies: Receiver<Reply>,
    thread: Option<JoinHandle<()>>,
}

impl Worker {
    pub fn start(ctx: &egui::Context, ds: Arc<Dataset>) -> Worker {
        let (req_tx, req_rx) = mpsc::channel::<Request>();
        let (rep_tx, replies) = mpsc::channel();
        let ctx = ctx.clone();
        let thread = std::thread::Builder::new()
            .name("cdept-wave-query".into())
            .spawn(move || serve(&ds, &req_rx, &rep_tx, &ctx))
            .ok();
        Worker {
            requests: Some(req_tx),
            replies,
            thread,
        }
    }

    pub fn send(&self, request: Request) {
        if let Some(tx) = &self.requests {
            let _ = tx.send(request);
        }
    }

    /// The newest reply, if any came.
    pub fn newest(&self) -> Option<Reply> {
        let mut last = None;
        while let Ok(r) = self.replies.try_recv() {
            last = Some(r);
        }
        last
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        // Closing the channel ends the loop; the thread is not waited for (a long
        // query would hold the UI), it ends after its present query.
        self.requests.take();
        self.thread.take();
    }
}

fn serve(ds: &Dataset, requests: &Receiver<Request>, replies: &Sender<Reply>, ctx: &egui::Context) {
    let mut prepared_for: Vec<u32> = Vec::new();
    while let Ok(mut request) = requests.recv() {
        while let Ok(newer) = requests.try_recv() {
            request = newer;
        }
        let started = Instant::now();
        let mut error = None;
        if request.signals != prepared_for {
            if let Err(e) = ds.prepare(&request.signals) {
                error = Some(e.to_string());
            }
            prepared_for = request.signals.clone();
        }
        let mut answers = Vec::with_capacity(request.signals.len());
        for &signal in &request.signals {
            let mut trace = Trace::default();
            let info = match ds.query(signal, request.x0, request.x1, request.pixels, &mut trace) {
                Ok(info) => info,
                Err(e) => {
                    error = Some(e.to_string());
                    QueryInfo::default()
                }
            };
            let at = |c: Option<f64>| c.and_then(|x| ds.value_at(signal, x).ok().map(|v| v.0));
            answers.push(Answer {
                signal,
                trace,
                info,
                at_cursor: [at(request.cursors[0]), at(request.cursors[1])],
            });
        }
        let prepared = request
            .signals
            .iter()
            .map(|&s| (s, ds.prepared(s)))
            .collect();
        let reply = Reply {
            request,
            answers,
            prepared,
            ms: started.elapsed().as_secs_f64() * 1e3,
            error,
        };
        if replies.send(reply).is_err() {
            break;
        }
        ctx.request_repaint();
    }
}
