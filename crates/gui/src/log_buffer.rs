//! A tiny in-memory ring buffer of formatted tracing lines, so the GUI can
//! show a live log panel without mouseshare-core needing any bespoke
//! event/callback API -- it just keeps emitting `tracing` events like it
//! always has, and this is the `tracing_subscriber::fmt` writer that
//! catches them.

use std::collections::VecDeque;
use std::io;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct LogBuffer {
    inner: Arc<Mutex<VecDeque<String>>>,
    capacity: usize,
}

impl LogBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(VecDeque::with_capacity(capacity))),
            capacity,
        }
    }

    /// A snapshot of the current lines, oldest first. Cheap enough to call
    /// once per UI frame for a log panel this small.
    pub fn snapshot(&self) -> Vec<String> {
        self.inner.lock().unwrap().iter().cloned().collect()
    }

    fn push_line(&self, line: String) {
        let mut buf = self.inner.lock().unwrap();
        if buf.len() >= self.capacity {
            buf.pop_front();
        }
        buf.push_back(line);
    }
}

pub struct LogWriter(LogBuffer);

impl io::Write for LogWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        for line in String::from_utf8_lossy(buf).lines() {
            if !line.is_empty() {
                self.0.push_line(line.to_string());
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogBuffer {
    type Writer = LogWriter;

    fn make_writer(&'a self) -> Self::Writer {
        LogWriter(self.clone())
    }
}
