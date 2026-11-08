//! The UI's minimal warn/error-only log (spec: Diagnostics and
//! observability): a bounded file beside the daemon's settings, riding
//! into the diagnostics bundle at export time.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_BYTES: u64 = 256 * 1024;

/// The UI's warn/error log: append-only, rotated, no user knob.
pub struct UiLog {
    path: PathBuf,
}

impl UiLog {
    pub fn open(path: impl Into<PathBuf>) -> UiLog {
        UiLog { path: path.into() }
    }

    /// Append a warn line, rotating first when the file is full.
    pub fn warn(&self, message: &str) {
        self.write_line("WARN", message);
    }

    /// Append an error line, rotating first when the file is full.
    pub fn error(&self, message: &str) {
        self.write_line("ERROR", message);
    }

    fn write_line(&self, level: &str, message: &str) {
        if let Ok(metadata) = std::fs::metadata(&self.path) {
            if metadata.len() >= MAX_BYTES {
                self.rotate();
            }
        }
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&self.path) {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let _ = writeln!(file, "[{now}] [{level}] {message}");
        }
    }

    fn rotate(&self) {
        let backup = suffixed(&self.path, 1);
        let _ = std::fs::remove_file(&backup);
        let _ = std::fs::rename(&self.path, backup);
    }

    /// The log's bytes for the diagnostics bundle (empty when absent).
    pub fn read(&self) -> Vec<u8> {
        std::fs::read(&self.path).unwrap_or_default()
    }
}

fn suffixed(path: &Path, n: usize) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{n}"));
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "bluepad-ui-log-{name}-{}.log",
            std::process::id()
        ))
    }

    #[test]
    fn warn_and_error_lines_land_with_level_and_timestamp() {
        let log = UiLog::open(path("lines"));
        let _ = std::fs::remove_file(&log.path);
        log.warn("daemon unreachable");
        log.error("update check failed");
        let content = String::from_utf8(log.read()).unwrap();
        assert!(content.contains("[WARN] daemon unreachable"), "{content}");
        assert!(content.contains("[ERROR] update check failed"), "{content}");
        // Every line carries a unix timestamp.
        assert!(content.starts_with('['), "{content}");
        let _ = std::fs::remove_file(&log.path);
    }

    #[test]
    fn rotation_keeps_one_backup_and_caps_the_live_file() {
        let log = UiLog::open(path("rotate"));
        let _ = std::fs::remove_file(&log.path);
        let _ = std::fs::remove_file(suffixed(&log.path, 1));
        // Fill past the cap: each line ~64 bytes, cap 256 KB.
        for i in 0..8_000 {
            log.warn(&"x".repeat(48));
            if i == 0 {
                // marker line in what becomes the rotated backup
            }
        }
        let live = std::fs::metadata(&log.path).unwrap().len();
        assert!(live < MAX_BYTES + 2048, "live file stays small: {live}");
        assert!(suffixed(&log.path, 1).exists(), "one rotated backup");
        assert!(!suffixed(&log.path, 2).exists(), "capped at two files");
        let _ = std::fs::remove_file(&log.path);
        let _ = std::fs::remove_file(suffixed(&log.path, 1));
    }

    #[test]
    fn read_is_empty_when_nothing_was_ever_logged() {
        let log = UiLog::open("/nonexistent/bluepad-ui/ui.log");
        assert!(log.read().is_empty());
    }
}
