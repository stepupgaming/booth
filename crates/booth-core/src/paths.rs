//! Where Booth keeps things. Everything lives under one folder, `%LOCALAPPDATA%\NoFaceTube\Booth`,
//! unless an environment variable points somewhere else (handy in development).
//!
//! | What            | Default                          | Override            |
//! |-----------------|----------------------------------|---------------------|
//! | everything      | `%LOCALAPPDATA%\NoFaceTube\Booth`           | `BOOTH_HOME`        |
//! | model weights   | `<home>\models`                  | `BOOTH_MODELS_DIR`  |
//! | engine binaries | next to the app (CPU or Metal); `<home>\engines` (NVIDIA download) | `BOOTH_ENGINE_DIR` |
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

/// Names of the engine folder shipped with the app, best first. `engine-cpu` (Windows, Linux) and
/// `engine-metal` (macOS) since 0.2; `engine` is the CUDA build that 0.1 shipped.
const BUNDLED_ENGINE_NAMES: [&str; 3] = ["engine-cpu", "engine-metal", "engine"];

/// The file where the app records its resource folder, so a booth-mcp installed elsewhere
/// (Claude Desktop's extension folder) finds the engine that shipped with the app.
fn install_file() -> PathBuf {
    home().join("install.json")
}

/// Called by the app at startup with its resource folder.
pub fn record_install(resource_dir: &Path) {
    let _ = ensure_dir(&home());
    let body = serde_json::json!({ "resource_dir": resource_dir });
    let _ = std::fs::write(install_file(), body.to_string());
}

fn recorded_resource_dir() -> Option<PathBuf> {
    let text = std::fs::read_to_string(install_file()).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    v["resource_dir"].as_str().map(PathBuf::from)
}

/// The engine that ships with the app: `BOOTH_ENGINE_DIR`, then an engine folder next to the
/// running program, in the macOS bundle's Resources, or in the resource folder the app recorded.
pub fn bundled_engine_dir() -> Option<PathBuf> {
    if let Some(dir) = env_dir("BOOTH_ENGINE_DIR") {
        return Some(dir);
    }
    if cfg!(target_os = "android") {
        // The program is in the app's native library folder (see `server_exe`); its data files
        // (LACI heads) are unpacked by the app into `<home>/engine`.
        let dir = home().join("engine");
        return server_exe(&dir).is_file().then_some(dir);
    }
    let mut bases = Vec::new();
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        bases.push(dir.join("..").join("Resources")); // Booth.app/Contents/MacOS -> Resources
        bases.push(dir.join("..").join("lib").join("Booth")); // Linux packages: /usr/bin -> /usr/lib/Booth
        bases.insert(0, dir);
    }
    bases.extend(recorded_resource_dir());
    bases
        .iter()
        .flat_map(|b| BUNDLED_ENGINE_NAMES.iter().map(move |n| b.join(n)))
        .find(|d| server_exe(d).is_file())
}

/// The engine program for an engine folder. On Android it ships in the app's native library
/// folder as `libzonos3server.so`, the only place an Android app may run a program from.
pub fn server_exe(engine: &Path) -> PathBuf {
    #[cfg(target_os = "android")]
    if let Some(dir) = native_lib_dir() {
        return dir.join("libzonos3server.so");
    }
    engine.join(SERVER_EXE)
}

/// What the engine decodes cloning clips with (it runs `ffmpeg`, or `ZONOS3_FFMPEG` when set):
/// an ffmpeg next to the engine, or on Android, where there is none, Booth's `booth-decode`.
pub fn audio_decoder(engine: &Path) -> Option<PathBuf> {
    #[cfg(target_os = "android")]
    if let Some(dir) = native_lib_dir() {
        return Some(dir.join("libboothdecode.so")).filter(|p| p.is_file());
    }
    Some(engine.join(if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" })).filter(|p| p.is_file())
}

/// Android: the app's native library folder (`nativeLibraryDir`), found from where the app's own
/// library is loaded. The libraries must be extracted on install (`useLegacyPackaging`), or they
/// stay inside the APK and can't be run.
#[cfg(target_os = "android")]
pub fn native_lib_dir() -> Option<PathBuf> {
    let maps = std::fs::read_to_string("/proc/self/maps").ok()?;
    maps.lines()
        .filter_map(|l| l.split_whitespace().nth(5))
        .find(|p| p.ends_with("/libbooth_lib.so") && !p.contains('!'))
        .and_then(|p| Path::new(p).parent().map(Path::to_path_buf))
}

/// Where the downloadable NVIDIA (CUDA) engine is unpacked.
pub fn engines_dir() -> PathBuf {
    home().join("engines")
}

/// The folder Booth runs the engine from by default, for display: the bundled engine, or where
/// it was expected.
pub fn engine_dir() -> PathBuf {
    bundled_engine_dir().unwrap_or_else(|| {
        std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(|p| p.join(BUNDLED_ENGINE_NAMES[0])))
            .unwrap_or_else(|| home().join("engine"))
    })
}

pub const SERVER_EXE: &str = if cfg!(windows) { "zonos3-server.exe" } else { "zonos3-server" };

pub fn ensure_dir(p: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(p)
}
