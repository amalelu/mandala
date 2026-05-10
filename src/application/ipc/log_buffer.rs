// SPDX-License-Identifier: MPL-2.0

//! Custom `log::Log` impl that tees every log record into:
//!   1. The underlying `env_logger` (so stderr output is unchanged),
//!   2. A bounded in-memory ring buffer (`/logs` endpoint reads it),
//!   3. The IPC broadcast channel as an `IpcEvent::Log` (SSE clients
//!      see it).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tokio::sync::broadcast;

use super::sse::IpcEvent;

/// Maximum number of log lines retained in the ring buffer.
pub const LOG_RING_CAPACITY: usize = 4096;

/// One captured log line. `seq` is a monotonic id used by clients
/// to fetch only-new entries via `GET /logs?since=N`.
#[derive(Clone, Debug, Serialize)]
pub struct LogLine {
    pub seq: u64,
    pub ts_ms: u64,
    pub level: String,
    pub target: String,
    pub message: String,
}

pub struct IpcLogger {
    inner: env_logger::Logger,
    buf: Arc<Mutex<VecDeque<LogLine>>>,
    event_tx: broadcast::Sender<IpcEvent>,
    seq: AtomicU64,
}

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl log::Log for IpcLogger {
    fn enabled(&self, m: &log::Metadata) -> bool {
        self.inner.enabled(m)
    }

    fn log(&self, record: &log::Record) {
        // Tee to stderr first so behaviour matches non-IPC runs.
        self.inner.log(record);
        let line = LogLine {
            seq: self.seq.fetch_add(1, Ordering::Relaxed),
            ts_ms: now_ms(),
            level: record.level().as_str().to_lowercase(),
            target: record.target().to_string(),
            message: format!("{}", record.args()),
        };
        {
            let mut buf = self.buf.lock().expect("log ring lock poisoned");
            if buf.len() == LOG_RING_CAPACITY {
                buf.pop_front();
            }
            buf.push_back(line.clone());
        }
        let _ = self.event_tx.send(IpcEvent::Log(line));
    }

    fn flush(&self) {
        self.inner.flush();
    }
}

/// Replace the global `log` logger with an [`IpcLogger`]. Must be
/// called once at startup, *before* the first `log!()` macro fires.
/// Idempotent across reentry: a second call is a no-op because
/// `log::set_boxed_logger` returns `Err` on the second install.
pub fn init_with_ipc(buf: Arc<Mutex<VecDeque<LogLine>>>, event_tx: broadcast::Sender<IpcEvent>) {
    let inner = env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info"),
    )
    .build();
    let logger = IpcLogger {
        inner,
        buf,
        event_tx,
        seq: AtomicU64::new(0),
    };
    log::set_max_level(log::LevelFilter::Trace);
    let _ = log::set_boxed_logger(Box::new(logger));
}

/// Filter the ring buffer by the per-request `since` cursor and
/// minimum level. Returns lines in insertion order.
pub fn filter_lines(
    buf: &VecDeque<LogLine>,
    since: Option<u64>,
    min_level: Option<log::Level>,
) -> Vec<LogLine> {
    buf.iter()
        .filter(|line| since.map(|s| line.seq > s).unwrap_or(true))
        .filter(|line| match min_level {
            None => true,
            Some(min) => level_ge(&line.level, min),
        })
        .cloned()
        .collect()
}

fn level_ge(name: &str, min: log::Level) -> bool {
    let line_level = match name {
        "error" => log::Level::Error,
        "warn" => log::Level::Warn,
        "info" => log::Level::Info,
        "debug" => log::Level::Debug,
        "trace" => log::Level::Trace,
        _ => return true,
    };
    line_level <= min
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(seq: u64, level: &str) -> LogLine {
        LogLine {
            seq,
            ts_ms: 0,
            level: level.into(),
            target: "t".into(),
            message: "m".into(),
        }
    }

    #[test]
    fn test_filter_since_returns_only_newer_entries() {
        let mut buf = VecDeque::new();
        for i in 0..5 {
            buf.push_back(line(i, "info"));
        }
        let out = filter_lines(&buf, Some(2), None);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].seq, 3);
        assert_eq!(out[1].seq, 4);
    }

    #[test]
    fn test_filter_min_level_warn_drops_info_and_below() {
        let mut buf = VecDeque::new();
        buf.push_back(line(0, "info"));
        buf.push_back(line(1, "warn"));
        buf.push_back(line(2, "error"));
        buf.push_back(line(3, "debug"));
        let out = filter_lines(&buf, None, Some(log::Level::Warn));
        let levels: Vec<&str> = out.iter().map(|l| l.level.as_str()).collect();
        assert_eq!(levels, vec!["warn", "error"]);
    }

    #[test]
    fn test_filter_no_since_no_level_returns_everything() {
        let mut buf = VecDeque::new();
        for i in 0..3 {
            buf.push_back(line(i, "info"));
        }
        assert_eq!(filter_lines(&buf, None, None).len(), 3);
    }
}
