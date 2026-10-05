use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tracing_subscriber::Layer;

const MAX_LINES: usize = 5000;
const MAX_LINE_CHARS: usize = 2000;

/// Incremental read result for the `get_logs` command. `next` is the
/// sequence number the next written line will get (= total lines written);
/// `reset: true` means `lines` is the full buffer and the caller should
/// replace, not append.
#[derive(Debug, PartialEq, Eq, serde::Serialize)]
pub struct LogChunk {
    pub lines: Vec<String>,
    pub next: u64,
    pub reset: bool,
}

struct Inner {
    lines: VecDeque<String>,
    next: u64,
}

#[derive(Clone)]
pub struct LogBuffer {
    inner: Arc<Mutex<Inner>>,
    start: Instant,
}

impl LogBuffer {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                lines: VecDeque::with_capacity(512),
                next: 0,
            })),
            start: Instant::now(),
        }
    }

    pub fn since(&self, since: Option<u64>) -> LogChunk {
        let buf = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let oldest = buf.next - buf.lines.len() as u64;
        match since {
            Some(s) if (oldest..=buf.next).contains(&s) => LogChunk {
                lines: buf.lines.range((s - oldest) as usize..).cloned().collect(),
                next: buf.next,
                reset: false,
            },
            // None, already evicted (s < oldest) or from a previous run (s > next).
            _ => LogChunk {
                lines: buf.lines.iter().cloned().collect(),
                next: buf.next,
                reset: true,
            },
        }
    }

    fn push(&self, mut line: String) {
        if let Some((cut, _)) = line.char_indices().nth(MAX_LINE_CHARS) {
            line.truncate(cut);
            line.push('…');
        }
        let mut buf = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if buf.lines.len() >= MAX_LINES {
            buf.lines.pop_front();
        }
        buf.lines.push_back(line);
        buf.next += 1;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn filled(n: usize) -> LogBuffer {
        let buf = LogBuffer::new();
        for i in 0..n {
            buf.push(format!("line {i}"));
        }
        buf
    }

    #[test]
    fn none_returns_everything_with_reset() {
        let chunk = filled(3).since(None);
        assert_eq!(chunk.lines, ["line 0", "line 1", "line 2"]);
        assert_eq!(chunk.next, 3);
        assert!(chunk.reset);
    }

    #[test]
    fn since_returns_only_new_lines() {
        let buf = filled(3);
        let chunk = buf.since(Some(1));
        assert_eq!(chunk.lines, ["line 1", "line 2"]);
        assert_eq!(chunk.next, 3);
        assert!(!chunk.reset);

        let empty = buf.since(Some(3));
        assert!(empty.lines.is_empty());
        assert_eq!(empty.next, 3);
        assert!(!empty.reset);

        assert_eq!(
            LogBuffer::new().since(Some(0)),
            LogChunk {
                lines: vec![],
                next: 0,
                reset: false
            }
        );
    }

    #[test]
    fn evicted_or_future_since_resets() {
        let buf = filled(MAX_LINES + 10);
        let chunk = buf.since(Some(5));
        assert!(chunk.reset);
        assert_eq!(chunk.lines.len(), MAX_LINES);
        assert_eq!(chunk.lines[0], "line 10");
        assert_eq!(chunk.next, (MAX_LINES + 10) as u64);

        let tail = buf.since(Some(MAX_LINES as u64 + 8));
        assert!(!tail.reset);
        assert_eq!(
            tail.lines,
            [
                format!("line {}", MAX_LINES + 8),
                format!("line {}", MAX_LINES + 9)
            ]
        );

        let future = buf.since(Some(u64::MAX));
        assert!(future.reset);
        assert_eq!(future.lines.len(), MAX_LINES);
    }

    #[test]
    fn long_lines_are_truncated() {
        let buf = LogBuffer::new();
        buf.push("日".repeat(MAX_LINE_CHARS + 5));
        buf.push("x".repeat(MAX_LINE_CHARS));
        let lines = buf.since(None).lines;
        assert_eq!(lines[0].chars().count(), MAX_LINE_CHARS + 1);
        assert!(lines[0].ends_with('…'));
        assert_eq!(lines[1].chars().count(), MAX_LINE_CHARS);
        assert!(!lines[1].ends_with('…'));
    }

    #[test]
    fn chunk_serializes_to_contract_shape() {
        let json = serde_json::to_value(filled(1).since(None)).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "lines": ["line 0"], "next": 1, "reset": true })
        );
    }
}
