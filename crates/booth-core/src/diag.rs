//! `logs/booth.log`: what Booth asked the engine to do and what came back, one line per event, so
//! a stuck or failed render can be explained afterwards. The engine's own output goes to
//! `logs/engine.log` (and the run before it to `engine.prev.log`).
//!
//! The app and booth-mcp both write here; each line carries the process id and who wrote it.

use std::io::Write;
use std::sync::OnceLock;

use crate::paths;

/// Rotate to `booth.log.1` past this size, so the log never grows without bound.
const MAX_BYTES: u64 = 1024 * 1024;

static SOURCE: OnceLock<String> = OnceLock::new();

/// Name this process in the log ("app", "mcp"). Call once at start-up.
pub fn set_source(name: &str) {
    let _ = SOURCE.set(name.to_string());
}

pub fn log_path() -> std::path::PathBuf {
    paths::logs_dir().join("booth.log")
}

/// Append one line. Never fails: logging must not break what it describes.
pub fn note(msg: impl std::fmt::Display) {
    let dir = paths::logs_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = log_path();
    if std::fs::metadata(&path).map(|m| m.len() > MAX_BYTES).unwrap_or(false) {
        let _ = std::fs::rename(&path, dir.join("booth.log.1"));
    }
    let line = format!(
        "{} {}[{}] {}\n",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
        SOURCE.get().map(String::as_str).unwrap_or("booth"),
        std::process::id(),
        msg.to_string().replace('\n', " | ")
    );
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = f.write_all(line.as_bytes());
    }
}

/// The last `n` lines of a text file ("" if it isn't there).
pub fn tail(path: &std::path::Path, n: usize) -> String {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

/// Everything worth sending when something went wrong: Booth's log, then the engine's.
pub fn report(lines: usize) -> String {
    let logs = paths::logs_dir();
    let mut out = format!("== booth.log ({})\n{}\n", log_path().display(), tail(&log_path(), lines));
    for name in ["engine.log", "engine.prev.log"] {
        let p = logs.join(name);
        if p.exists() {
            out.push_str(&format!("\n== {name}\n{}\n", tail(&p, 40)));
        }
    }
    out
}

/// Whether a process id is still running (unknown counts as alive).
pub fn pid_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        std::path::Path::new(&format!("/proc/{pid}")).exists() || !std::path::Path::new("/proc/self").exists()
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        true
    }
}
