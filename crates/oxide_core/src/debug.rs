//! Unity `Debug.Log` / `LogWarning` / `LogError` feeding the Console.
//!
//! Reference: <https://docs.unity3d.com/Manual/Console.html>
//!
//! Every message goes to an in-memory ring buffer (read by the Console window and the
//! `get_console_logs` MCP tool) and is echoed to stderr. Messages from other crates that use
//! the `log` facade (wgpu, winit, ...) are captured too, see [`install_log_bridge`].

use serde::Serialize;
use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

const CAPACITY: usize = 10_000;

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, serde::Deserialize,
)]
pub enum LogType {
    Log,
    Warning,
    Error,
    Exception,
}

impl LogType {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "log" | "info" => Some(Self::Log),
            "warning" | "warn" => Some(Self::Warning),
            "error" => Some(Self::Error),
            "exception" => Some(Self::Exception),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct LogEntry {
    /// Monotonic id; pass the last one you saw to [`entries_since`] to get only new entries.
    pub seq: u64,
    /// Seconds since startup.
    pub time: f64,
    pub log_type: LogType,
    pub message: String,
    /// Module or crate that logged it.
    pub source: String,
}

#[derive(Default)]
struct Console {
    entries: VecDeque<LogEntry>,
    next_seq: u64,
    counts: [u64; 4],
}

fn console() -> &'static Mutex<Console> {
    static CONSOLE: OnceLock<Mutex<Console>> = OnceLock::new();
    CONSOLE.get_or_init(Default::default)
}

/// Appends a message to the Console. Prefer the `debug_log!` family of macros.
pub fn log(log_type: LogType, source: &str, message: impl Into<String>) {
    let message = message.into();
    eprintln!("[{log_type:?}] {message}");
    let mut c = console().lock().unwrap_or_else(|e| e.into_inner());
    c.next_seq += 1;
    c.counts[log_type as usize] += 1;
    let entry = LogEntry {
        seq: c.next_seq,
        time: crate::time::realtime_since_startup(),
        log_type,
        message,
        source: source.to_owned(),
    };
    if c.entries.len() == CAPACITY {
        c.entries.pop_front();
    }
    c.entries.push_back(entry);
}

/// Entries with `seq > after_seq` and severity `>= min_type`, oldest first, at most `limit`
/// (the most recent ones are kept when truncating).
pub fn entries_since(after_seq: u64, min_type: LogType, limit: usize) -> Vec<LogEntry> {
    let c = console().lock().unwrap_or_else(|e| e.into_inner());
    let matching: Vec<_> = c
        .entries
        .iter()
        .filter(|e| e.seq > after_seq && e.log_type >= min_type)
        .cloned()
        .collect();
    let skip = matching.len().saturating_sub(limit);
    matching.into_iter().skip(skip).collect()
}

/// Unity Console "Clear". Counts reset too; sequence numbers keep increasing.
pub fn clear() {
    let mut c = console().lock().unwrap_or_else(|e| e.into_inner());
    c.entries.clear();
    c.counts = [0; 4];
}

/// `(logs, warnings, errors + exceptions)`, like the Console toolbar counters.
pub fn counts() -> (u64, u64, u64) {
    let c = console().lock().unwrap_or_else(|e| e.into_inner());
    (c.counts[0], c.counts[1], c.counts[2] + c.counts[3])
}

/// Unity `Debug.Log`.
#[macro_export]
macro_rules! debug_log {
    ($($arg:tt)*) => {
        $crate::debug::log($crate::debug::LogType::Log, module_path!(), format!($($arg)*))
    };
}

/// Unity `Debug.LogWarning`.
#[macro_export]
macro_rules! debug_log_warning {
    ($($arg:tt)*) => {
        $crate::debug::log($crate::debug::LogType::Warning, module_path!(), format!($($arg)*))
    };
}

/// Unity `Debug.LogError`.
#[macro_export]
macro_rules! debug_log_error {
    ($($arg:tt)*) => {
        $crate::debug::log($crate::debug::LogType::Error, module_path!(), format!($($arg)*))
    };
}

struct Bridge;

impl log::Log for Bridge {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let log_type = match record.level() {
            log::Level::Error => LogType::Error,
            log::Level::Warn => LogType::Warning,
            _ => LogType::Log,
        };
        log(log_type, record.target(), record.args().to_string());
    }

    fn flush(&self) {}
}

/// Routes the `log` facade (used by wgpu, winit, ...) into the Console at `max_level`.
/// Safe to call more than once; only the first call installs the bridge.
pub fn install_log_bridge(max_level: log::LevelFilter) {
    static BRIDGE: Bridge = Bridge;
    if log::set_logger(&BRIDGE).is_ok() {
        log::set_max_level(max_level);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_and_filter() {
        let start = entries_since(0, LogType::Log, usize::MAX)
            .last()
            .map_or(0, |e| e.seq);
        crate::debug_log!("hello {}", 1);
        crate::debug_log_warning!("careful");
        crate::debug_log_error!("broken");
        let all = entries_since(start, LogType::Log, 100);
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].message, "hello 1");
        let errors = entries_since(start, LogType::Error, 100);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].log_type, LogType::Error);
        let last_two = entries_since(start, LogType::Log, 2);
        assert_eq!(last_two[1].message, "broken");
    }
}
