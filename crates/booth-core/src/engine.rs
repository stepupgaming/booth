//! Starts `zonos3-server` once and shares it between the app and booth-mcp.
//!
//! Whoever needs the engine first starts it on a free localhost port and writes `engine.json`
//! (pid, port) in Booth's folder. Everyone else reads that file and attaches.
//!
//! Which engine runs: the NVIDIA build (downloaded on Windows) on the graphics card, the bundled
//! Metal build on Apple Silicon, else the bundled CPU build on the processor.
//! The engine keeps running after the starter exits, so a Claude Desktop request doesn't pay the
//! model load again; `stop()` ends it.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::api::Zonos;
use crate::models::{self, Pack};
use crate::paths;
use crate::settings::{self, Device};

/// What the engine computes on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Accel {
    Cuda,
    Metal,
    Cpu,
}

impl Accel {
    fn flag(self) -> &'static str {
        if self == Accel::Cpu { "--cpu" } else { "--gpu" }
    }
}

/// An engine folder and how to run it.
#[derive(Debug, Clone)]
pub struct EngineChoice {
    pub dir: PathBuf,
    pub accel: Accel,
}

/// The engines to try for `device`, best first.
pub fn choices(device: Device) -> Vec<EngineChoice> {
    let mut out = Vec::new();
    if device == Device::Auto {
        if let Some(dir) = models::installed_cuda_engine() {
            out.push(EngineChoice { dir, accel: Accel::Cuda });
        }
    }
    if let Some(dir) = paths::bundled_engine_dir() {
        // 0.1 shipped the CUDA build as `engine/`; it only runs on the graphics card, and is
        // already listed above when present.
        if dir.file_name().is_some_and(|n| n == "engine") && cfg!(windows) {
            return out;
        }
        let accel = if cfg!(target_os = "macos") && device == Device::Auto { Accel::Metal } else { Accel::Cpu };
        out.push(EngineChoice { dir, accel });
    }
    out
}

pub const DEFAULT_PORT: u16 = 47219;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineLock {
    pub pid: u32,
    pub port: u16,
    pub started_by: String,
    pub started_at: chrono::DateTime<chrono::Utc>,
    /// Packs loaded at start; a pack downloaded later needs a restart to be used.
    #[serde(default)]
    pub packs: Vec<Pack>,
    /// What it runs on (missing in locks written by 0.1, which always ran CUDA).
    #[serde(default)]
    pub accel: Option<Accel>,
}

impl EngineLock {
    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum EngineState {
    /// The engine binaries are missing (no engine pack).
    NoEngine { looked_in: String },
    /// The core model pack is missing.
    NoModels { looked_in: String },
    Stopped,
    Running { port: u16, pid: u32, packs: Vec<Pack>, accel: Accel },
}

fn read_lock() -> Option<EngineLock> {
    let text = std::fs::read_to_string(paths::engine_lock_file()).ok()?;
    serde_json::from_str(&text).ok()
}

/// The running engine, if its lock file points at a server that answers.
pub async fn running() -> Option<(EngineLock, Zonos)> {
    let lock = read_lock()?;
    let z = Zonos::new(lock.base_url());
    if z.health().await {
        Some((lock, z))
    } else {
        None
    }
}

pub async fn state() -> EngineState {
    if let Some((lock, _)) = running().await {
        let accel = lock.accel.unwrap_or(Accel::Cuda);
        return EngineState::Running { port: lock.port, pid: lock.pid, packs: lock.packs, accel };
    }
    if choices(settings::load().device).is_empty() {
        return EngineState::NoEngine { looked_in: paths::engine_dir().display().to_string() };
    }
    let models = paths::models_dir();
    if !models::pack_present(&models, Pack::Core) {
        return EngineState::NoModels { looked_in: models.display().to_string() };
    }
    EngineState::Stopped
}

/// Serialises starts inside one process (the app asks for the engine from several commands at once).
static STARTING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Attach to the running engine or start one. `who` names the starter in the lock file.
///
/// Only one engine may ever start: two would both load onto the GPU, and the second overwrites
/// `engine.json`, so the first is never stopped. A mutex covers this process, and the
/// `engine.starting` marker (created exclusively) covers the app and booth-mcp racing each other.
pub async fn ensure(who: &str) -> Result<Zonos> {
    if let Some((_, z)) = running().await {
        return Ok(z);
    }
    let _guard = STARTING.lock().await;
    if let Some((_, z)) = running().await {
        return Ok(z);
    }
    let marker = paths::home().join("engine.starting");
    paths::ensure_dir(&paths::home())?;
    let deadline = Instant::now() + Duration::from_secs(200);
    loop {
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&marker) {
            Ok(_) => break,
            Err(_) => {
                // Left over from a crash: a start never takes this long.
                let stale = std::fs::metadata(&marker)
                    .and_then(|m| m.modified())
                    .map(|t| t.elapsed().unwrap_or_default() > Duration::from_secs(200))
                    .unwrap_or(true);
                if stale {
                    std::fs::remove_file(&marker).ok();
                    continue;
                }
                if let Some((_, z)) = running().await {
                    return Ok(z);
                }
                if Instant::now() > deadline {
                    bail!("another Booth process is still starting the voice engine; try again in a moment");
                }
                tokio::time::sleep(Duration::from_millis(400)).await;
            }
        }
    }
    // Someone may have finished starting it between our check and taking the marker.
    let result = match running().await {
        Some((_, z)) => Ok(z),
        None => start(who).await,
    };
    std::fs::remove_file(&marker).ok();
    result
}

fn free_port() -> u16 {
    if std::net::TcpListener::bind(("127.0.0.1", DEFAULT_PORT)).is_ok() {
        return DEFAULT_PORT;
    }
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .unwrap_or(DEFAULT_PORT)
}

/// The command line for the server, from what is on disk.
pub fn command_line(engine: &Path, accel: Accel, models: &Path, port: u16) -> Result<(PathBuf, Vec<String>, Vec<Pack>)> {
    let exe = engine.join(paths::SERVER_EXE);
    if !exe.is_file() {
        bail!("the voice engine isn't installed ({} is missing)", exe.display());
    }
    let backbone = models::backbone_path(models)
        .with_context(|| format!("the voice model isn't downloaded yet (looked in {})", models.display()))?;
    let p = |n: &str| models.join(n);
    let s = |p: PathBuf| p.display().to_string();
    let mut args = vec![
        s(backbone),
        accel.flag().into(),
        "--host".into(),
        "127.0.0.1".into(),
        "--port".into(),
        port.to_string(),
        "--batch".into(),
        "2".into(),
    ];
    let mut packs = vec![Pack::Core];
    if p("voices").is_dir() {
        args.extend(["--tts-default-voices-dir".into(), s(p("voices"))]);
    }
    if p("spk-encoder.gguf").is_file() {
        args.extend(["--spk-encoder".into(), s(p("spk-encoder.gguf"))]);
    }
    if p("steer.gguf").is_file() {
        args.extend(["--steer".into(), s(p("steer.gguf"))]);
    }
    if p("zac-encoder.gguf").is_file() && p("ssl-teacher.gguf").is_file() {
        args.extend([
            "--zac-encoder".into(),
            s(p("zac-encoder.gguf")),
            "--ssl-teacher".into(),
            s(p("ssl-teacher.gguf")),
        ]);
        packs.push(Pack::Exact);
    }
    // zac-keep2.gguf (default) and zac.gguf beside the backbone are discovered by the server.
    if p("zac.gguf").is_file() {
        packs.push(Pack::FullDecoder);
    }
    for (lang, file) in laci_heads(&engine.join("laci_heads")) {
        args.extend(["--laci-heads".into(), format!("{lang}={}", file.display())]);
    }
    Ok((exe, args, packs))
}

/// LACI alignment heads per server language, from the engine's `laci_heads` folder.
/// Engines from b171b19 on have `zonos3_mini_heads.json` (English) and one
/// `zonos3_mini_heads_<speech code>.json` per other language; older ones only
/// `zonos3_mini_mect60k_heads.json` (English).
fn laci_heads(dir: &Path) -> Vec<(String, PathBuf)> {
    let english = dir.join("zonos3_mini_heads.json");
    if !english.is_file() {
        let old = dir.join("zonos3_mini_mect60k_heads.json");
        return if old.is_file() { vec![("en_us".into(), old)] } else { Vec::new() };
    }
    let mut out = vec![("en_us".into(), english.clone()), ("en_gb".into(), english)];
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    files.sort();
    for file in files {
        let Some(code) = file
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_prefix("zonos3_mini_heads_")?.strip_suffix(".json"))
        else {
            continue;
        };
        // Speech code → the server's language code, where they differ (Zyphra's languages.json).
        let lang = match code {
            "fr" => "fr_fr",
            "pt" => "pt_br",
            "zh" => "cmn",
            c => c,
        };
        out.push((lang.to_string(), file));
    }
    out
}

/// Start the best engine for the current setting. If the NVIDIA engine can't start (no driver,
/// an old card), fall back to the processor rather than leave Booth without a voice.
pub async fn start(who: &str) -> Result<Zonos> {
    crate::models::refresh_small_files(&paths::models_dir()).await;
    let list = choices(settings::load().device);
    if list.is_empty() {
        bail!("the voice engine isn't installed ({} is missing); reinstalling Booth puts it back", paths::engine_dir().display());
    }
    let mut last = None;
    for (i, choice) in list.iter().enumerate() {
        match start_one(who, choice).await {
            Ok(z) => return Ok(z),
            Err(e) if i + 1 < list.len() => {
                eprintln!("booth: the {:?} engine didn't start, trying the next one: {e:#}", choice.accel);
                last = Some(e);
            }
            Err(e) => return Err(e),
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("no engine started")))
}

async fn start_one(who: &str, choice: &EngineChoice) -> Result<Zonos> {
    let engine = choice.dir.clone();
    let models = paths::models_dir();
    let port = free_port();
    let (exe, args, packs) = command_line(&engine, choice.accel, &models, port)?;

    paths::ensure_dir(&paths::logs_dir())?;
    let log_path = paths::logs_dir().join("engine.log");
    let log = std::fs::File::create(&log_path)?;

    // The engine's own DLLs first (Windows looks for them on PATH).
    let mut dirs = vec![engine.join("bin"), engine.clone()];
    if let Some(old) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&old));
    }
    let path_var = std::env::join_paths(dirs).unwrap_or_default();

    let mut cmd = tokio::process::Command::new(&exe);
    #[cfg(target_os = "linux")]
    {
        // The CPU backends are shared libraries next to the server.
        let mut dirs = vec![engine.clone()];
        if let Some(old) = std::env::var_os("LD_LIBRARY_PATH") {
            dirs.extend(std::env::split_paths(&old));
        }
        cmd.env("LD_LIBRARY_PATH", std::env::join_paths(dirs).unwrap_or_default());
    }
    cmd.args(&args)
        .current_dir(&engine)
        .env("PATH", path_var)
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log)
        .kill_on_drop(false);
    let ffmpeg = engine.join(if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" });
    if ffmpeg.is_file() {
        cmd.env("ZONOS3_FFMPEG", &ffmpeg);
    }
    #[cfg(windows)]
    {
        // CREATE_NO_WINDOW | DETACHED_PROCESS-like behaviour: no console flashes, survives us.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP);
    }
    #[cfg(unix)]
    {
        // Its own process group, so a Ctrl-C or hangup aimed at the starter doesn't reach it.
        cmd.process_group(0);
    }
    let mut child = cmd.spawn().with_context(|| format!("starting {}", exe.display()))?;
    let pid = child.id().unwrap_or(0);

    let lock = EngineLock {
        pid,
        port,
        started_by: who.into(),
        started_at: chrono::Utc::now(),
        packs,
        accel: Some(choice.accel),
    };
    std::fs::write(paths::engine_lock_file(), serde_json::to_vec_pretty(&lock)?)?;

    let z = Zonos::new(lock.base_url());
    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        if z.health().await {
            // Detach: the engine outlives this process on purpose.
            std::mem::forget(child);
            return Ok(z);
        }
        if let Ok(Some(status)) = child.try_wait() {
            let tail = log_tail(&log_path, 15);
            std::fs::remove_file(paths::engine_lock_file()).ok();
            bail!("the voice engine stopped while loading ({status}). Last lines of {}:\n{tail}", log_path.display());
        }
        if Instant::now() > deadline {
            bail!("the voice engine didn't answer within 3 minutes; see {}", log_path.display());
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
}

/// Marks a request in flight on the engine. `stop()` waits until none are left, because killing a
/// process in the middle of CUDA work can hang the GPU driver (it has crashed this kind of machine).
pub struct BusyGuard(PathBuf);

impl BusyGuard {
    pub fn new() -> Self {
        let dir = paths::home().join("busy");
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join(format!("{}-{}", std::process::id(), uuid::Uuid::new_v4()));
        let _ = std::fs::write(&p, b"");
        Self(p)
    }
}

impl Default for BusyGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for BusyGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Requests in flight (markers older than 15 minutes are treated as left over from a crash).
fn busy_count() -> usize {
    let Ok(rd) = std::fs::read_dir(paths::home().join("busy")) else { return 0 };
    rd.filter_map(|e| e.ok())
        .filter(|e| {
            e.metadata()
                .and_then(|m| m.modified())
                .map(|t| t.elapsed().unwrap_or_default() < Duration::from_secs(900))
                .unwrap_or(false)
        })
        .count()
}

/// Stop the engine (whoever started it), once no request is running on it.
/// Waits up to `wait` for running requests to finish; gives up (leaving the engine running)
/// rather than cut one off.
pub async fn stop_when_idle(wait: Duration) -> Result<bool> {
    let Some(lock) = read_lock() else { return Ok(true) };
    let deadline = Instant::now() + wait;
    while busy_count() > 0 {
        if Instant::now() > deadline {
            return Ok(false);
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    #[cfg(windows)]
    {
        // The server has no shutdown route and no window to close, so this ends the process.
        // It is idle at this point: no request runs on the GPU.
        let _ = tokio::process::Command::new("taskkill")
            .args(["/PID", &lock.pid.to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
    }
    #[cfg(unix)]
    {
        // SIGTERM, then SIGKILL if it is still there after a few seconds. It is idle (see above).
        let pid = lock.pid.to_string();
        let _ = tokio::process::Command::new("kill").args(["-TERM", &pid]).status().await;
        let mut alive = true;
        for _ in 0..20 {
            if !Zonos::new(lock.base_url()).health().await {
                alive = false;
                break;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        // Only while it still answers, so a reused pid is never hit.
        if alive {
            let _ = tokio::process::Command::new("kill")
                .args(["-KILL", &pid])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await;
        }
    }
    for _ in 0..40 {
        if !Zonos::new(lock.base_url()).health().await {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    std::fs::remove_file(paths::engine_lock_file()).ok();
    Ok(true)
}

pub async fn stop() -> Result<bool> {
    stop_when_idle(Duration::from_secs(600)).await
}

/// Restart so newly downloaded packs are loaded.
pub async fn restart(who: &str) -> Result<Zonos> {
    if !stop().await? {
        bail!("the voice engine is still busy with a take; try again when it finishes");
    }
    ensure(who).await
}

fn log_tail(p: &Path, n: usize) -> String {
    let text = std::fs::read_to_string(p).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}
