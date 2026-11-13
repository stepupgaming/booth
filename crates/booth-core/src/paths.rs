//! Where Booth keeps things. Everything lives under one folder, `%LOCALAPPDATA%\NoFaceTube\Booth`,
//! unless an environment variable points somewhere else (handy in development).
//!
//! | What            | Default                          | Override            |
//! |-----------------|----------------------------------|---------------------|
//! | everything      | `%LOCALAPPDATA%\NoFaceTube\Booth`           | `BOOTH_HOME`        |
//! | model weights   | `<home>\models`                  | `BOOTH_MODELS_DIR`  |
//! | engine binaries | next to the app, else `<home>\engine` | `BOOTH_ENGINE_DIR` |
//! | voices          | `<home>\voices`                  |                     |
//! | takes           | `<home>\takes`                   | `BOOTH_TAKES_DIR`   |

use std::path::{Path, PathBuf};

fn env_dir(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from)
}

pub fn home() -> PathBuf {
    env_dir("BOOTH_HOME").unwrap_or_else(|| {
        dirs::data_local_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("NoFaceTube").join("Booth")
    })
}

pub fn models_dir() -> PathBuf {
    env_dir("BOOTH_MODELS_DIR").unwrap_or_else(|| home().join("models"))
}

pub fn voices_dir() -> PathBuf {
    home().join("voices")
}

pub fn takes_dir() -> PathBuf {
    env_dir("BOOTH_TAKES_DIR").unwrap_or_else(|| home().join("takes"))
}

pub fn logs_dir() -> PathBuf {
    home().join("logs")
}

pub fn settings_file() -> PathBuf {
    home().join("settings.json")
}

/// The lock file of the running engine (pid and port), shared by the app and booth-mcp.
pub fn engine_lock_file() -> PathBuf {
    home().join("engine.json")
}

/// The folder holding `zonos3-server.exe` and its DLLs.
/// Order: `BOOTH_ENGINE_DIR`, `engine\` next to the running executable (the installed app),
/// then `<home>\engine` (downloaded engine pack).
pub fn engine_dir() -> PathBuf {
    if let Some(dir) = env_dir("BOOTH_ENGINE_DIR") {
        return dir;
    }
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|p| p.join("engine")))
        .filter(|d| d.join(SERVER_EXE).is_file())
    {
        return dir;
    }
    home().join("engine")
}

pub const SERVER_EXE: &str = if cfg!(windows) { "zonos3-server.exe" } else { "zonos3-server" };

pub fn ensure_dir(p: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(p)
}
