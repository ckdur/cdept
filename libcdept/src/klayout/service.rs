//! An KLayout engine for background work in the GUI: previews and exports.
//!
//! Jobs run one after the other on a thread of their own, so the caller never waits.
//! The helper is started when the first job arrives, and started again after a job
//! fails (the usual cause is that the helper stopped).

use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;

use eyre::Result;

use super::engine::{Engine, EngineOptions};
use super::session::Wake;

type Job = Box<dyn FnOnce(&mut Option<Engine>, &EngineOptions) + Send>;

/// The result of a job, once it is there.
pub struct Pending<T> {
    result: Receiver<Result<T, String>>,
}

impl<T> Pending<T> {
    /// `None` while the job is still running. The result is handed out once; after
    /// that, and if the service went away, this is an error.
    pub fn poll(&self) -> Option<Result<T, String>> {
        match self.result.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err("the job was dropped".into())),
        }
    }
}

/// A queue of jobs for one engine.
pub struct Service {
    jobs: Sender<Job>,
    wake: Wake,
}

impl Service {
    /// `wake` is called (from the service's thread) whenever a job has finished.
    pub fn new(options: EngineOptions, wake: Wake) -> Service {
        let (jobs, queue) = mpsc::channel::<Job>();
        thread::Builder::new()
            .name("klayout-service".into())
            .spawn(move || {
                let mut engine = None;
                for job in queue {
                    job(&mut engine, &options);
                }
            })
            .expect("cannot start a thread");
        Service { jobs, wake }
    }

    /// Queue `job`. It gets the engine to itself while it runs.
    pub fn run<T: Send + 'static>(
        &self,
        job: impl FnOnce(&mut Engine) -> Result<T> + Send + 'static,
    ) -> Pending<T> {
        let (sender, result) = mpsc::channel();
        let wake = self.wake.clone();
        let job: Job = Box::new(move |slot, options| {
            let outcome = (|| {
                if slot.is_none() {
                    *slot = Some(Engine::start(options.clone())?);
                }
                let engine = slot.as_mut().expect("just started");
                job(engine)
            })();
            if outcome.is_err() {
                // Whatever went wrong, the next job gets a fresh helper.
                *slot = None;
            }
            let _ = sender.send(outcome.map_err(|e| format!("{e:#}")));
            wake();
        });
        // The thread lives as long as `self.jobs`.
        let _ = self.jobs.send(job);
        Pending { result }
    }
}
