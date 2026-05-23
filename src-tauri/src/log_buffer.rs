use std::sync::{Arc, Mutex};
use std::time::Instant;
use tracing_subscriber::Layer;

const MAX_LINES: usize = 5000;

#[derive(Clone)]
pub struct LogBuffer {
    inner: Arc<Mutex<Vec<String>>>,
    start: Instant,
}

impl LogBuffer {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Vec::with_capacity(512))),
            start: Instant::now(),
        }
    }

    pub fn lines(&self) -> Vec<String> {
        self.inner.lock().unwrap().clone()
    }

    fn push(&self, line: String) {
        let mut buf = self.inner.lock().unwrap();
        if buf.len() >= MAX_LINES {
            let cut = buf.len() / 4;
            buf.drain(..cut);
        }
        buf.push(line);
    }
}

pub struct BufferLayer {
    buf: LogBuffer,
}

impl BufferLayer {
    pub fn new(buf: LogBuffer) -> Self {
        Self { buf }
    }
}

impl<S: tracing::Subscriber> Layer<S> for BufferLayer {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let meta = event.metadata();
        let mut visitor = MessageVisitor(String::new());
        event.record(&mut visitor);

        let elapsed = self.buf.start.elapsed();
        let secs = elapsed.as_secs();
        let millis = elapsed.subsec_millis();
        let line = format!(
            "+{:>4}.{:03}s {:>5} {}: {}",
            secs,
            millis,
            meta.level(),
            meta.target(),
            visitor.0
        );
        self.buf.push(line);
    }
}

struct MessageVisitor(String);

impl tracing::field::Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if !self.0.is_empty() {
            self.0.push(' ');
        }
        if field.name() == "message" {
            self.0.push_str(&format!("{:?}", value));
        } else {
            self.0.push_str(&format!("{}={:?}", field.name(), value));
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if !self.0.is_empty() {
            self.0.push(' ');
        }
        if field.name() == "message" {
            self.0.push_str(value);
        } else {
            self.0.push_str(&format!("{}={}", field.name(), value));
        }
    }
}
