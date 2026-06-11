//! Logging initialization: colored stderr plus an optional daily-rotated,
//! plain-text log file under the XDG state directory.
//!
//! The file appender is implemented locally on top of `chrono` (already a
//! workspace dependency) rather than pulling in `tracing-appender`, which
//! transitively requires a version of the `time` crate that conflicts with the
//! project MSRV / security-audit policy. Log volume for a desktop daemon is
//! low, so a simple mutex-guarded synchronous writer is sufficient.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, fmt};
use vox_core::config::AppConfig;

/// Crates whose log output we scope to the configured level. Used to keep the
/// config-driven verbosity from drowning in dependency (e.g. `zbus`) spam.
const OWN_CRATES: &[&str] = &[
    "vox_daemon",
    "vox_core",
    "vox_capture",
    "vox_transcribe",
    "vox_storage",
    "vox_summarize",
    "vox_diarize",
    "vox_gui",
    "vox_tray",
    "vox_notify",
];

const FILENAME_PREFIX: &str = "vox-daemon";
const FILENAME_SUFFIX: &str = "log";

/// Initializes logging to stderr (colored) and, when enabled in `config`, a
/// daily-rotated plain-text log file under [`vox_core::paths::logs_dir`].
pub fn init(config: &AppConfig, verbosity: u8) {
    let filter = build_env_filter(&config.logging.level, verbosity);
    let stderr_layer = fmt::layer().with_writer(std::io::stderr).with_target(true);
    let registry = tracing_subscriber::registry()
        .with(filter)
        .with(stderr_layer);

    if config.logging.file_enabled {
        let dir = vox_core::paths::logs_dir();
        // init() runs before ensure_dirs(), so create the log dir here.
        if let Err(e) = fs::create_dir_all(&dir) {
            registry.init();
            tracing::warn!("could not create log directory {}: {e}", dir.display());
            return;
        }

        let appender = DailyAppender::new(dir.clone(), config.logging.retention_days);
        let file_layer = fmt::layer()
            .with_writer(move || appender.clone())
            .with_ansi(false)
            .with_target(true);
        registry.with(file_layer).init();
        tracing::debug!("file logging enabled at {}", dir.display());
    } else {
        registry.init();
    }
}

/// Builds the tracing `EnvFilter` from, in order of precedence:
/// 1. The `RUST_LOG` environment variable (if set).
/// 2. The `-v`/`-vv` CLI flags (global `debug`/`trace`, legacy behavior).
/// 3. The configured `[logging] level`, scoped to our own crates.
fn build_env_filter(level: &str, verbosity: u8) -> EnvFilter {
    if let Ok(filter) = EnvFilter::try_from_default_env() {
        return filter;
    }

    let directive = match verbosity {
        0 => {
            let lvl = normalize_level(level);
            OWN_CRATES
                .iter()
                .map(|crate_name| format!("{crate_name}={lvl}"))
                .collect::<Vec<_>>()
                .join(",")
        }
        1 => "debug".to_owned(),
        _ => "trace".to_owned(),
    };

    EnvFilter::new(directive)
}

/// Maps a config level string to a valid tracing level, defaulting to `info`.
fn normalize_level(level: &str) -> &'static str {
    match level.to_ascii_lowercase().as_str() {
        "error" => "error",
        "warn" => "warn",
        "debug" => "debug",
        "trace" => "trace",
        _ => "info",
    }
}

/// A thread-safe, daily-rotating file writer.
///
/// Cloneable (shares one underlying file handle via `Arc<Mutex<_>>`) so it can
/// be used as a `tracing_subscriber` `MakeWriter` via a `move || self.clone()`
/// closure. On each write it ensures the file for the current UTC date is open,
/// rotating and pruning old files when the date changes.
#[derive(Clone)]
struct DailyAppender {
    inner: Arc<Mutex<Inner>>,
}

struct Inner {
    dir: PathBuf,
    retention_days: u32,
    /// The UTC date (`%Y-%m-%d`) of the currently open file, if any.
    current_date: String,
    file: Option<File>,
}

impl DailyAppender {
    fn new(dir: PathBuf, retention_days: u32) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                dir,
                retention_days,
                current_date: String::new(),
                file: None,
            })),
        }
    }
}

impl Inner {
    /// Ensures `self.file` points at the log file for the current UTC date,
    /// rotating (and pruning) when the date has changed since the last write.
    fn ensure_current(&mut self) -> io::Result<()> {
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        if self.file.is_some() && self.current_date == today {
            return Ok(());
        }

        let path = self
            .dir
            .join(format!("{FILENAME_PREFIX}.{today}.{FILENAME_SUFFIX}"));
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        self.file = Some(file);
        self.current_date = today;
        // Best-effort prune; a failure here must not break logging.
        if let Err(e) = prune_old_logs(&self.dir, self.retention_days) {
            // Avoid recursing into the tracing machinery from inside a writer.
            eprintln!("vox-daemon: failed to prune old log files: {e}");
        }
        Ok(())
    }
}

impl Write for DailyAppender {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        inner.ensure_current()?;
        inner
            .file
            .as_mut()
            .expect("file is set by ensure_current")
            .write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        match inner.file.as_mut() {
            Some(f) => f.flush(),
            None => Ok(()),
        }
    }
}

/// Returns `true` if `name` is one of our dated log files
/// (`vox-daemon.YYYY-MM-DD.log`).
fn is_log_filename(name: &str) -> bool {
    let Some(rest) = name.strip_prefix(&format!("{FILENAME_PREFIX}.")) else {
        return false;
    };
    let Some(date) = rest.strip_suffix(&format!(".{FILENAME_SUFFIX}")) else {
        return false;
    };
    // Expect a strict YYYY-MM-DD date so we never touch unrelated files.
    let bytes = date.as_bytes();
    date.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && date.chars().enumerate().all(|(i, c)| {
            if i == 4 || i == 7 {
                c == '-'
            } else {
                c.is_ascii_digit()
            }
        })
}

/// From a list of log filenames, returns those that should be deleted to keep
/// only the newest `retention_days` files. `retention_days == 0` keeps all.
///
/// Filenames embed an ISO `YYYY-MM-DD` date, so lexical sort == chronological.
fn select_prunable(filenames: &[String], retention_days: u32) -> Vec<String> {
    if retention_days == 0 {
        return Vec::new();
    }
    let mut logs: Vec<&String> = filenames.iter().filter(|n| is_log_filename(n)).collect();
    logs.sort();
    let keep = retention_days as usize;
    if logs.len() <= keep {
        return Vec::new();
    }
    logs[..logs.len() - keep]
        .iter()
        .map(|s| (*s).clone())
        .collect()
}

/// Deletes log files in `dir` beyond the newest `retention_days`.
fn prune_old_logs(dir: &Path, retention_days: u32) -> io::Result<()> {
    if retention_days == 0 {
        return Ok(());
    }
    let mut names = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if let Ok(name) = entry.file_name().into_string() {
            names.push(name);
        }
    }
    for name in select_prunable(&names, retention_days) {
        // Ignore individual removal errors (e.g. already gone).
        let _ = fs::remove_file(dir.join(name));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_log_filename() {
        assert!(is_log_filename("vox-daemon.2026-06-11.log"));
        assert!(!is_log_filename("vox-daemon.2026-6-11.log")); // not zero-padded
        assert!(!is_log_filename("vox-daemon.log"));
        assert!(!is_log_filename("vox-daemon.2026-06-11.txt"));
        assert!(!is_log_filename("other.2026-06-11.log"));
        assert!(!is_log_filename("vox-daemon.20260611.log"));
        assert!(!is_log_filename("vox-daemon.2026-06-xx.log"));
    }

    #[test]
    fn test_select_prunable_keeps_newest() {
        let files = vec![
            "vox-daemon.2026-06-01.log".to_owned(),
            "vox-daemon.2026-06-03.log".to_owned(),
            "vox-daemon.2026-06-02.log".to_owned(),
            "unrelated.txt".to_owned(),
        ];
        // Keep newest 2 -> prune the oldest dated file only.
        let prune = select_prunable(&files, 2);
        assert_eq!(prune, vec!["vox-daemon.2026-06-01.log".to_owned()]);
    }

    #[test]
    fn test_select_prunable_zero_keeps_all() {
        let files = vec![
            "vox-daemon.2026-06-01.log".to_owned(),
            "vox-daemon.2026-06-02.log".to_owned(),
        ];
        assert!(select_prunable(&files, 0).is_empty());
    }

    #[test]
    fn test_select_prunable_under_limit() {
        let files = vec!["vox-daemon.2026-06-01.log".to_owned()];
        assert!(select_prunable(&files, 7).is_empty());
    }

    #[test]
    fn test_prune_old_logs_on_disk() {
        let dir = tempfile::tempdir().expect("tempdir");
        for d in ["2026-06-01", "2026-06-02", "2026-06-03", "2026-06-04"] {
            let p = dir.path().join(format!("vox-daemon.{d}.log"));
            std::fs::write(&p, b"x").expect("write");
        }
        // An unrelated file must be left untouched.
        std::fs::write(dir.path().join("keep.me"), b"x").expect("write");

        prune_old_logs(dir.path(), 2).expect("prune");

        let mut remaining: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        remaining.sort();
        assert_eq!(
            remaining,
            vec![
                "keep.me".to_owned(),
                "vox-daemon.2026-06-03.log".to_owned(),
                "vox-daemon.2026-06-04.log".to_owned(),
            ]
        );
    }

    #[test]
    fn test_daily_appender_writes_dated_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut appender = DailyAppender::new(dir.path().to_path_buf(), 7);
        appender.write_all(b"hello\n").expect("write");
        appender.flush().expect("flush");

        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let expected = dir.path().join(format!("vox-daemon.{today}.log"));
        let contents = std::fs::read_to_string(&expected).expect("read log");
        assert_eq!(contents, "hello\n");
    }

    #[test]
    fn test_normalize_level() {
        assert_eq!(normalize_level("DEBUG"), "debug");
        assert_eq!(normalize_level("warn"), "warn");
        assert_eq!(normalize_level("nonsense"), "info");
    }
}
