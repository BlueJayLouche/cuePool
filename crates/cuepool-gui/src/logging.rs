//! Global log capture for the in-app log window and field diagnostics.

use log::{Level, LevelFilter, Log, Metadata, Record};
use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

const MAX_ENTRIES: usize = 2000;
const MAX_MESSAGE_BYTES: usize = 16 * 1024;
const MAX_LOG_BYTES: u64 = 1024 * 1024;
const LOG_BACKUPS: usize = 3;

/// Info-level field events on this target are persisted even without `RUST_LOG`.
pub const PERSIST_TARGET: &str = "cuepool::field";

/// A single captured log line.
#[derive(Clone, Debug)]
pub struct LogEntry {
    pub cursor: u64,
    pub level: Level,
    pub target: String,
    pub message: String,
    pub timestamp: String,
    pub recorded_at: String,
}

/// Global ring buffer of recent log entries.
static LOG_BUFFER: Mutex<VecDeque<LogEntry>> = Mutex::new(VecDeque::new());
static NEXT_CURSOR: AtomicU64 = AtomicU64::new(1);

/// Initialize stderr, in-app, and rotating file logging.
///
/// The logger is still installed if the file cannot be opened; the returned
/// error is only the persistent sink's status.
pub fn init_logger(log_path: &Path) -> Result<PathBuf, String> {
    let mut builder = env_logger::Builder::from_default_env();
    builder.format_timestamp_millis();
    let stderr = builder.build();
    let max_level = std::cmp::max(stderr.filter(), LevelFilter::Info);

    let (file, status) = match RotatingFile::open(log_path, MAX_LOG_BYTES, LOG_BACKUPS) {
        Ok(file) => (Some(file), Ok(log_path.to_path_buf())),
        Err(error) => (None, Err(format!("{} ({error})", log_path.display()))),
    };

    log::set_boxed_logger(Box::new(FieldLogger {
        stderr,
        file: Mutex::new(file),
    }))
    .map(|()| log::set_max_level(max_level))
    .expect("Failed to set logger");

    if let Err(error) = &status {
        log::warn!(target: PERSIST_TARGET, "Persistent log unavailable: {error}");
    }
    status
}

/// Read a snapshot of the current log buffer.
pub fn read_log_buffer() -> Vec<LogEntry> {
    match LOG_BUFFER.lock() {
        Ok(buf) => buf.iter().cloned().collect(),
        Err(_) => Vec::new(),
    }
}

/// Clear the in-memory buffer without touching the persistent log.
pub fn clear_log_buffer() {
    if let Ok(mut buf) = LOG_BUFFER.lock() {
        buf.clear();
    }
}

fn routes(level: Level, target: &str, stderr_enabled: bool) -> (bool, bool) {
    let warning = level <= Level::Warn;
    let in_app = warning || stderr_enabled;
    let persistent = warning || target == PERSIST_TARGET;
    (in_app, persistent)
}

struct FieldLogger {
    stderr: env_logger::Logger,
    // ponytail: synchronous writes fit today's low-volume, non-audio-callback
    // logs; replace this with a bounded writer thread if that contract changes.
    file: Mutex<Option<RotatingFile>>,
}

impl Log for FieldLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        let stderr_enabled = self.stderr.enabled(metadata);
        let (_, persistent) = routes(metadata.level(), metadata.target(), stderr_enabled);
        persistent || stderr_enabled
    }

    fn log(&self, record: &Record) {
        let stderr_enabled = self.stderr.enabled(record.metadata());
        let (in_app, persistent) = routes(record.level(), record.target(), stderr_enabled);
        if !in_app && !persistent {
            return;
        }

        if stderr_enabled {
            self.stderr.log(record);
        }

        let now = chrono::Local::now();
        let message = bounded_message(*record.args());
        let recorded_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

        if persistent {
            let line = format!(
                "{recorded_at} {:<5} {} {message}\n",
                record.level(),
                record.target()
            );
            if let Ok(mut slot) = self.file.lock()
                && let Some(file) = slot.as_mut()
                && let Err(error) = file.write_line(line.as_bytes())
            {
                eprintln!("CuePool persistent logging failed: {error}");
                *slot = None;
            }
        }

        if in_app {
            // Allocate the cursor under the same lock as insertion so cursor
            // order is also buffer order for polling clients.
            if let Ok(mut buf) = LOG_BUFFER.lock() {
                let entry = LogEntry {
                    cursor: NEXT_CURSOR.fetch_add(1, Ordering::Relaxed),
                    level: record.level(),
                    target: record.target().to_string(),
                    message,
                    timestamp: now.format("%H:%M:%S%.3f").to_string(),
                    recorded_at,
                };
                if buf.len() >= MAX_ENTRIES {
                    buf.pop_front();
                }
                buf.push_back(entry);
            }
        }
    }

    fn flush(&self) {
        self.stderr.flush();
        if let Ok(mut slot) = self.file.lock()
            && let Some(file) = slot.as_mut()
        {
            let _ = file.flush();
        }
    }
}

struct RotatingFile {
    path: PathBuf,
    file: Option<File>,
    bytes: u64,
    max_bytes: u64,
    backups: usize,
}

impl RotatingFile {
    fn open(path: &Path, max_bytes: u64, backups: usize) -> io::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if let Ok(metadata) = path.metadata() {
            if !metadata.is_file() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "log path is not a file",
                ));
            }
            if metadata.len() >= max_bytes {
                rotate_files(path, backups)?;
            }
        }
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        let bytes = file.metadata()?.len();
        Ok(Self {
            path: path.to_path_buf(),
            file: Some(file),
            bytes,
            max_bytes,
            backups,
        })
    }

    fn write_line(&mut self, line: &[u8]) -> io::Result<()> {
        if self.bytes > 0 && self.bytes.saturating_add(line.len() as u64) > self.max_bytes {
            self.rotate()?;
        }
        self.file
            .as_mut()
            .ok_or_else(|| io::Error::other("log file is closed"))?
            .write_all(line)?;
        self.bytes = self.bytes.saturating_add(line.len() as u64);
        Ok(())
    }

    fn rotate(&mut self) -> io::Result<()> {
        self.file.take();
        rotate_files(&self.path, self.backups)?;
        self.file = Some(
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)?,
        );
        self.bytes = 0;
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.file.as_mut() {
            Some(file) => file.flush(),
            None => Ok(()),
        }
    }
}

fn rotate_files(path: &Path, backups: usize) -> io::Result<()> {
    for index in (1..=backups).rev() {
        let source = if index == 1 {
            path.to_path_buf()
        } else {
            backup_path(path, index - 1)
        };
        if source.exists() {
            let destination = backup_path(path, index);
            if destination.exists() {
                std::fs::remove_file(&destination)?;
            }
            std::fs::rename(source, destination)?;
        }
    }
    Ok(())
}

fn backup_path(path: &Path, index: usize) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(format!(".{index}"));
    name.into()
}

fn bounded_message(arguments: std::fmt::Arguments<'_>) -> String {
    struct BoundedWriter {
        message: String,
        truncated: bool,
    }

    impl std::fmt::Write for BoundedWriter {
        fn write_str(&mut self, value: &str) -> std::fmt::Result {
            let remaining = MAX_MESSAGE_BYTES.saturating_sub(self.message.len());
            if value.len() <= remaining {
                self.message.push_str(value);
            } else {
                let mut end = remaining;
                while !value.is_char_boundary(end) {
                    end -= 1;
                }
                self.message.push_str(&value[..end]);
                self.truncated = true;
            }
            Ok(())
        }
    }

    let mut writer = BoundedWriter {
        message: String::new(),
        truncated: false,
    };
    let _ = std::fmt::write(&mut writer, arguments);
    if writer.truncated {
        writer.message.push_str("… [truncated]");
    }
    writer.message
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_log_dir(name: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "cuepool-logging-{name}-{}-{unique}",
            std::process::id()
        ))
    }

    #[test]
    fn routing_keeps_field_diagnostics_independent_of_stderr() {
        assert_eq!(routes(Level::Error, "decode", false), (true, true));
        assert_eq!(routes(Level::Warn, "decode", false), (true, true));
        assert_eq!(routes(Level::Info, PERSIST_TARGET, false), (false, true));
        assert_eq!(routes(Level::Info, "cue", false), (false, false));
        assert_eq!(routes(Level::Trace, "cue", false), (false, false));

        assert_eq!(routes(Level::Warn, "decode", true), (true, true));
        assert_eq!(routes(Level::Info, PERSIST_TARGET, true), (true, true));
        assert_eq!(routes(Level::Info, "cue", true), (true, false));
        assert_eq!(routes(Level::Debug, "cue", true), (true, false));
    }

    #[test]
    fn rotating_file_keeps_only_the_requested_backups() {
        let dir = temp_log_dir("rotation");
        let path = dir.join("cuepool.log");
        let mut file = RotatingFile::open(&path, 12, 2).unwrap();

        for line in [
            "first\n",
            "second",
            "third\n",
            "fourth",
            "fifth\n",
            "sixth\n",
            "seventh\n",
        ] {
            file.write_line(line.as_bytes()).unwrap();
        }
        file.flush().unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "seventh\n");
        assert_eq!(
            std::fs::read_to_string(backup_path(&path, 1)).unwrap(),
            "fifth\nsixth\n"
        );
        assert_eq!(
            std::fs::read_to_string(backup_path(&path, 2)).unwrap(),
            "third\nfourth"
        );
        assert!(!backup_path(&path, 3).exists());

        drop(file);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn file_open_failure_is_reported_without_a_panic() {
        let dir = temp_log_dir("failure");
        std::fs::create_dir_all(&dir).unwrap();
        assert!(RotatingFile::open(&dir, 12, 2).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn buffered_log_messages_are_bounded_without_splitting_utf8() {
        let text = "🦀".repeat(MAX_MESSAGE_BYTES);
        let message = bounded_message(format_args!("{text}"));

        assert!(message.is_char_boundary(message.len()));
        assert!(message.starts_with('🦀'));
        assert!(message.ends_with("… [truncated]"));
        assert!(message.len() <= MAX_MESSAGE_BYTES + "… [truncated]".len());
    }
}
