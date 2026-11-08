//! Leveled, rotated logs (spec: Diagnostics and observability): Debug
//! default for 0.1.0, no user knob; 512 KB × 3 rotation, tier-homed.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tracing_subscriber::fmt::MakeWriter;

const MAX_BYTES: u64 = 512 * 1024;
const MAX_FILES: usize = 3;

/// A size-rotating file writer: `bluepad.log`, `bluepad.log.1`, …
pub struct RotatingFile {
    path: PathBuf,
    inner: Mutex<State>,
}

struct State {
    file: Option<File>,
    written: u64,
}

impl RotatingFile {
    pub fn new(path: impl Into<PathBuf>) -> RotatingFile {
        RotatingFile {
            path: path.into(),
            inner: Mutex::new(State {
                file: None,
                written: 0,
            }),
        }
    }

    fn write_bytes(&self, state: &mut State, bytes: &[u8]) {
        if state.file.is_none() {
            self.open(state);
        }
        if state.file.is_some() && state.written + bytes.len() as u64 > MAX_BYTES {
            self.rotate(state);
        }
        if let Some(file) = state.file.as_mut() {
            let _ = file.write_all(bytes);
            state.written += bytes.len() as u64;
        }
    }

    fn open(&self, state: &mut State) {
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        state.file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .ok();
        state.written = state
            .file
            .as_ref()
            .and_then(|f| f.metadata().ok())
            .map(|m| m.len())
            .unwrap_or(0);
    }

    fn rotate(&self, state: &mut State) {
        // log.1 → log.2 (the old log.2 falls off the end), log → log.1,
        // fresh log. Keeps exactly `MAX_FILES` files.
        for i in (1..MAX_FILES - 1).rev() {
            let from = suffixed(&self.path, i);
            let to = suffixed(&self.path, i + 1);
            let _ = std::fs::rename(&from, &to);
        }
        let _ = std::fs::rename(&self.path, suffixed(&self.path, 1));
        state.file = None;
        state.written = 0;
        self.open(state);
    }
}

fn suffixed(path: &Path, n: usize) -> PathBuf {
    // "bluepad.log" → "bluepad.log.N" (append, not with_extension, so
    // multi-dot names survive).
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{n}"));
    path.with_file_name(name)
}

/// The per-call writer handle tracing holds.
pub struct RotatingFileWriter {
    rotating: &'static RotatingFile,
}

impl Write for RotatingFileWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut state = self.rotating.inner.lock().unwrap();
        self.rotating.write_bytes(&mut state, buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for &'static RotatingFile {
    type Writer = RotatingFileWriter;

    fn make_writer(&'a self) -> Self::Writer {
        RotatingFileWriter { rotating: self }
    }
}

/// Install the process-wide tracing setup: file (rotating) + stderr,
/// Debug level for 0.1.0 — no user knob (spec: Diagnostics).
pub fn install(path: impl Into<PathBuf>) {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    let rotating: &'static RotatingFile = Box::leak(Box::new(RotatingFile::new(path)));
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(rotating)
                .with_ansi(false),
        )
        .with(tracing_subscriber::fmt::layer().with_ansi(false))
        .with(tracing_subscriber::filter::LevelFilter::DEBUG)
        .init();
}

/// The in-process (jailed-tier) variant: file only, never stderr — the
/// hosting UI process owns the terminal, and Debug-level library noise
/// must not garble its output (issue #31). Same tier-homed rotated log
/// as the daemon's.
pub fn install_file_only(path: impl Into<PathBuf>) {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    let rotating: &'static RotatingFile = Box::leak(Box::new(RotatingFile::new(path)));
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(rotating)
                .with_ansi(false),
        )
        .with(tracing_subscriber::filter::LevelFilter::DEBUG)
        .init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_keeps_three_files() {
        let dir = std::env::temp_dir().join(format!("bluepad-rot-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let rotating = RotatingFile::new(dir.join("bluepad.log"));
        for _ in 0..40 {
            let mut state = rotating.inner.lock().unwrap();
            rotating.write_bytes(&mut state, &vec![b'x'; 60 * 1024]);
        }
        assert!(dir.join("bluepad.log").exists());
        assert!(dir.join("bluepad.log.1").exists(), "first rotation");
        assert!(dir.join("bluepad.log.2").exists(), "second rotation");
        assert!(!dir.join("bluepad.log.3").exists(), "capped at 3 files");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
