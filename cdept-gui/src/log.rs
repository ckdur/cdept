//! Captures `tracing` events into a ring buffer shown by the Console panel.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::sync::{Arc, Mutex, MutexGuard};

use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;

const CAPACITY: usize = 10_000;

#[derive(Debug, Clone)]
pub struct LogLine {
    pub time: String,
    pub level: Level,
    pub target: String,
    pub message: String,
}

#[derive(Default)]
struct Inner {
    lines: VecDeque<LogLine>,
    repaint: Option<egui::Context>,
}

/// Shared log buffer. Cheap to clone.
#[derive(Clone, Default)]
pub struct LogBuffer(Arc<Mutex<Inner>>);

impl LogBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    /// The tracing layer that feeds this buffer.
    pub fn layer(&self) -> ConsoleLayer {
        ConsoleLayer(self.clone())
    }

    /// Request a repaint of this context whenever a line is added.
    pub(crate) fn set_repaint_context(&self, ctx: egui::Context) {
        self.inner().repaint = Some(ctx);
    }

    pub fn push(&self, line: LogLine) {
        let repaint = {
            let mut inner = self.inner();
            if inner.lines.len() == CAPACITY {
                inner.lines.pop_front();
            }
            inner.lines.push_back(line);
            inner.repaint.clone()
        };
        if let Some(ctx) = repaint {
            ctx.request_repaint();
        }
    }

    pub fn clear(&self) {
        self.inner().lines.clear();
    }

    pub(crate) fn lines(&self) -> LinesGuard<'_> {
        LinesGuard(self.inner())
    }

    fn inner(&self) -> MutexGuard<'_, Inner> {
        // A panic while holding the lock only loses log lines; keep going.
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

pub(crate) struct LinesGuard<'a>(MutexGuard<'a, Inner>);

impl std::ops::Deref for LinesGuard<'_> {
    type Target = VecDeque<LogLine>;
    fn deref(&self) -> &Self::Target {
        &self.0.lines
    }
}

/// `tracing_subscriber` layer writing into a [`LogBuffer`].
pub struct ConsoleLayer(LogBuffer);

impl<S: Subscriber> Layer<S> for ConsoleLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let target = event.metadata().target();
        // egui may log while it holds its own context lock; requesting a repaint from
        // there would deadlock, and those lines are not useful to the user anyway.
        if ["egui", "eframe", "winit"]
            .iter()
            .any(|p| target.starts_with(p))
        {
            return;
        }
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);
        self.0.push(LogLine {
            time: chrono::Local::now().format("%H:%M:%S%.3f").to_string(),
            level: *event.metadata().level(),
            target: target.to_string(),
            message: visitor.finish(),
        });
    }
}

#[derive(Default)]
struct MessageVisitor {
    message: String,
    fields: String,
}

impl MessageVisitor {
    fn finish(self) -> String {
        if self.fields.is_empty() {
            self.message
        } else if self.message.is_empty() {
            self.fields.trim_start().to_string()
        } else {
            format!("{}{}", self.message, self.fields)
        }
    }
}

impl Visit for MessageVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message.push_str(value);
        } else {
            let _ = write!(self.fields, " {}={value}", field.name());
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            let _ = write!(self.message, "{value:?}");
        } else {
            let _ = write!(self.fields, " {}={value:?}", field.name());
        }
    }
}
