use std::{
    collections::VecDeque,
    fmt::Write as _,
    sync::{Arc, Mutex},
};

use tracing::{Event, Level, Subscriber, field::Visit};
use tracing_subscriber::{Layer, layer::Context};

const MAX_LOG_LINES: usize = 1000;

#[derive(Clone)]
pub struct LogEntry {
    pub level: Level,
    pub line: String,
}

#[derive(Clone, Default)]
pub struct LogBuffer {
    inner: Arc<Mutex<VecDeque<LogEntry>>>,
}

impl LogBuffer {
    pub fn push(&self, entry: LogEntry) {
        let mut guard = self.inner.lock().expect("log buffer mutex poisoned");
        if guard.len() >= MAX_LOG_LINES {
            guard.pop_front();
        }
        guard.push_back(entry);
    }

    pub fn snapshot(&self, limit: usize) -> Vec<LogEntry> {
        let guard = self.inner.lock().expect("log buffer mutex poisoned");
        let len = guard.len();
        let start = len.saturating_sub(limit);
        guard.iter().skip(start).cloned().collect()
    }
}

pub struct LogBufferLayer {
    buffer: LogBuffer,
}

impl LogBufferLayer {
    pub fn new(buffer: LogBuffer) -> Self {
        Self { buffer }
    }
}

impl<S> Layer<S> for LogBufferLayer
where
    S: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);

        let metadata = event.metadata();
        let mut line = format!("[{} {}]", metadata.level(), metadata.target());
        if !visitor.message.is_empty() {
            let _ = write!(line, " {}", visitor.message);
        }
        if !visitor.fields.is_empty() {
            let _ = write!(line, " {}", visitor.fields.join(" "));
        }

        self.buffer.push(LogEntry {
            level: *metadata.level(),
            line,
        });
    }
}

#[derive(Default)]
struct MessageVisitor {
    message: String,
    fields: Vec<String>,
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let rendered = format!("{value:?}");
        if field.name() == "message" {
            self.message = rendered.trim_matches('"').to_owned();
        } else {
            self.fields.push(format!("{}={rendered}", field.name()));
        }
    }
}
