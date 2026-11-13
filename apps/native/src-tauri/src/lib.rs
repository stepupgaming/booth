//! Booth's desktop app: Tauri commands over booth-core. The web view does the drawing; every
//! model, engine and file operation happens here.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use base64::Engine as _;
use booth_core::engine::{self, EngineState};
use booth_core::models::{self, Pack};
use booth_core::studio::{self, SpeakOptions};
use booth_core::takes::{self, ScriptLine, Take};
use booth_core::voices::{self, Consent, Voice};
use booth_core::{paths, settings, SpeakerMode, Zonos};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

type Res<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn err_chain(e: anyhow::Error) -> String {
    format!("{e:#}")
}

/// The running engine, started for the app if needed.
async fn zonos() -> Res<Zonos> {
    engine::ensure("app").await.map_err(err_chain)
}

// ---------- first run and status ----------

#[derive(Serialize)]
struct Overview {
    engine: EngineState,
    packs: Vec<models::PackStatus>,
    models_dir: PathBuf,
    takes_dir: PathBuf,
    home: PathBuf,
    engine_dir: PathBuf,
    settings: settings::Settings,
}

#[tauri::command]
async fn overview() -> Res<Overview> {
    let models_dir = paths::models_dir();
    Ok(Overview {
        engine: engine::state().await,
        packs: models::status(&models_dir),
        models_dir,
        takes_dir: paths::takes_dir(),
        home: paths::home(),
        engine_dir: paths::engine_dir(),
        settings: settings::load(),
    })
}

#[derive(Serialize)]
struct SystemCheck {
    gpu: Option<String>,
    vram_gb: Option<f64>,
    disk: Option<String>,
    disk_free_gb: Option<f64>,
    os: Option<String>,
    engine_found: bool,
}

/// GPU, free disk where the models go, and Windows version, for the first setup screen.
#[tauri::command]
async fn system_check() -> Res<SystemCheck> {
    let (gpu, vram_gb) = tokio::task::spawn_blocking(nvidia_gpu).await.map_err(err)?;
    let models_dir = paths::models_dir();
    let disks = sysinfo::Disks::new_with_refreshed_list();
    // The disk whose mount point is the longest prefix of the models folder.
    let target = models_dir.to_string_lossy().to_lowercase();
    let disk = disks
        .list()
        .iter()
        .filter(|d| target.starts_with(&d.mount_point().to_string_lossy().to_lowercase()))
        .max_by_key(|d| d.mount_point().as_os_str().len());
    Ok(SystemCheck {
        gpu,
        vram_gb,
        disk: disk.map(|d| d.mount_point().to_string_lossy().trim_end_matches('\\').to_string()),
        disk_free_gb: disk.map(|d| d.available_space() as f64 / 1e9),
        os: sysinfo::System::long_os_version(),
        engine_found: paths::engine_dir().join(paths::SERVER_EXE).is_file(),
    })
}

fn nvidia_gpu() -> (Option<String>, Option<f64>) {
    let mut cmd = std::process::Command::new("nvidia-smi");
    cmd.args(["--query-gpu=name,memory.total", "--format=csv,noheader,nounits"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let Ok(out) = cmd.output() else { return (None, None) };
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().next().unwrap_or("");
    let mut parts = line.split(',').map(str::trim);
    let name = parts.next().filter(|s| !s.is_empty()).map(str::to_string);
    let mib: Option<f64> = parts.next().and_then(|s| s.parse().ok());
    (name, mib.map(|m| (m / 1024.0).round()))
}

#[derive(Serialize)]
struct PackInfo {
    pack: Pack,
    label: String,
    present: bool,
    bytes: u64,
    files: Vec<String>,
}

/// Each pack with its download size from the Hub (a fixed estimate when offline).
#[tauri::command]
async fn pack_info() -> Res<Vec<PackInfo>> {
    let client = reqwest::Client::new();
    let models_dir = paths::models_dir();
    let mut out = Vec::new();
    for pack in Pack::ALL {
        let bytes = match models::plan(&client, pack).await {
            Ok(files) => files.iter().map(|f| f.size).sum(),
            Err(_) => match pack {
                Pack::Core => 1_160_000_000,
                Pack::Exact => 5_600_000_000,
                Pack::FullDecoder => 2_200_000_000,
            },
        };
        out.push(PackInfo {
            pack,
            label: pack.label().into(),
            present: models::pack_present(&models_dir, pack),
            bytes,
            files: pack.files().iter().map(|f| f.trim_end_matches(".gguf").trim_end_matches('/').to_string()).collect(),
        });
    }
    Ok(out)
}

static DOWNLOADING: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Serialize)]
struct DownloadEvent {
    pack: Pack,
    file: String,
    done_bytes: u64,
    total_bytes: u64,
    /// Bytes per second over the last few seconds.
    rate: f64,
    index: usize,
    count: usize,
}

/// Download the packs in order, emitting `download` events. Restarts the engine afterwards
/// (when it was running) so it picks up the new files.
#[tauri::command]
async fn download_packs(app: AppHandle, packs: Vec<Pack>) -> Res<()> {
    if DOWNLOADING.swap(true, Ordering::SeqCst) {
        return Err("a download is already running".into());
    }
    let result = async {
        let client = reqwest::Client::new();
        let models_dir = paths::models_dir();
        paths::ensure_dir(&models_dir).map_err(err)?;
        let count = packs.len();
        for (index, pack) in packs.iter().copied().enumerate() {
            let mut last_emit = Instant::now() - Duration::from_secs(1);
            let mut window: (Instant, u64) = (Instant::now(), 0);
            let mut rate = 0.0;
            let app2 = app.clone();
            models::download(&client, &models_dir, pack, move |p| {
                let now = Instant::now();
                let dt = now.duration_since(window.0).as_secs_f64();
                if dt >= 1.0 {
                    rate = p.done_bytes.saturating_sub(window.1) as f64 / dt;
                    window = (now, p.done_bytes);
                }
                if now.duration_since(last_emit) >= Duration::from_millis(150) || p.done_bytes == p.total_bytes {
                    last_emit = now;
                    let _ = app2.emit(
                        "download",
                        DownloadEvent {
                            pack: p.pack,
                            file: p.file,
                            done_bytes: p.done_bytes,
                            total_bytes: p.total_bytes,
                            rate,
                            index,
                            count,
                        },
                    );
                }
            })
            .await
            .map_err(err_chain)?;
        }
        if engine::running().await.is_some() {
            engine::restart("app").await.map_err(err_chain)?;
        }
        Ok(())
    }
    .await;
    DOWNLOADING.store(false, Ordering::SeqCst);
    result
}

#[tauri::command]
async fn engine_start() -> Res<EngineState> {
    engine::ensure("app").await.map_err(err_chain)?;
    Ok(engine::state().await)
}

/// Stops the engine once nothing is mid-render (never kills a job on the GPU).
#[tauri::command]
async fn engine_stop() -> Res<bool> {
    engine::stop_when_idle(Duration::from_secs(120)).await.map_err(err_chain)
}

#[tauri::command]
async fn engine_restart() -> Res<EngineState> {
    engine::restart("app").await.map_err(err_chain)?;
    Ok(engine::state().await)
}

// ---------- voices ----------

#[tauri::command]
async fn list_voices() -> Res<Vec<Voice>> {
    // The preset list comes from the engine; keep a copy so the app opens without loading the
    // model onto the GPU. The engine starts only when something is spoken.
    let cache = cache_file("default-voices.json");
    if let Some((_, z)) = engine::running().await {
        let list = voices::all(Some(&z)).await;
        let defaults: Vec<&Voice> = list.iter().filter(|v| v.kind == voices::VoiceKind::Default).collect();
        if !defaults.is_empty() {
            let _ = std::fs::write(&cache, serde_json::to_vec(&defaults).unwrap_or_default());
        }
        return Ok(list);
    }
    if let Some(defaults) = std::fs::read(&cache).ok().and_then(|b| serde_json::from_slice::<Vec<Voice>>(&b).ok()) {
        let mut list = voices::mine();
        list.extend(defaults);
        return Ok(list);
    }
    let z = zonos().await?;
    Ok(voices::all(Some(&z)).await)
}

fn cache_file(name: &str) -> PathBuf {
    let dir = paths::home().join("cache");
    let _ = paths::ensure_dir(&dir);
    dir.join(name)
}

/// A file to play for a voice card: the default voice's sample, a clone's own clip, or a
/// spoken preview for a designed voice (made once and kept with the voice).
#[tauri::command]
async fn voice_preview(id: String) -> Res<PathBuf> {
    let z = zonos().await?;
    let v = voices::find(Some(&z), &id).await.map_err(err_chain)?;
    if let Some(sid) = v.speaker_id.as_deref().filter(|_| v.dir.is_none()) {
        let dir = paths::home().join("cache").join("previews");
        paths::ensure_dir(&dir).map_err(err)?;
        let file = dir.join(format!("{sid}.wav"));
        if !file.is_file() {
            let wav = z.speaker_preview(sid).await.map_err(err_chain)?;
            std::fs::write(&file, wav).map_err(err)?;
        }
        return Ok(file);
    }
    if let Some(clip) = v.clip_path() {
        return Ok(clip);
    }
    let dir = v.dir.clone().ok_or("this voice has no folder")?;
    let file = dir.join("preview.wav");
    if !file.is_file() {
        let cat = z.catalog().await.map_err(err_chain)?;
        let text = cat.preview_text.clone().unwrap_or_else(|| "Hello. This is how I sound.".into());
        let mut req = booth_core::GenerateRequest { text, ..Default::default() };
        v.apply(&mut req, None).map_err(err_chain)?;
        let audio = z.generate(&req).await.map_err(err_chain)?;
        std::fs::write(&file, audio.wav).map_err(err)?;
    }
    Ok(file)
}

#[derive(Deserialize)]
struct CloneArgs {
    name: String,
    /// The recording or file, base64.
    audio_base64: String,
    /// `wav`, `webm`, `mp3`...
    ext: String,
    transcript: Option<String>,
    /// "me" or "consented".
    speaker: String,
    /// High-fidelity (Exact) clone when the pack is installed; false = Quick (Shapeable only).
    #[serde(default)]
    exact: Option<bool>,
}

#[tauri::command]
async fn create_clone(a: CloneArgs) -> Res<Voice> {
    let audio = base64::engine::general_purpose::STANDARD.decode(a.audio_base64.as_bytes()).map_err(err)?;
    let z = zonos().await?;
    let exact = a.exact.unwrap_or(true) && engine::running().await.is_some_and(|(lock, _)| lock.packs.contains(&Pack::Exact));
    let statement = match a.speaker.as_str() {
        "me" => "This is my own voice.",
        _ => "The person in this recording agreed to have their voice cloned.",
    };
    let consent = Consent { statement: statement.into(), confirmed_at: booth_core::chrono::Utc::now(), via: "app".into() };
    voices::create_clone(&z, &a.name, &audio, &a.ext, a.transcript.filter(|t| !t.trim().is_empty()), consent, exact)
        .await
        .map_err(err_chain)
}

#[derive(Deserialize)]
struct DesignArgs {
    name: String,
    base: String,
    /// Friendly controls (`age`, `deep`, `accent:british`).
    controls: BTreeMap<String, f64>,
    description: Option<String>,
}

#[tauri::command]
async fn design_voice(a: DesignArgs) -> Res<Voice> {
    let z = zonos().await?;
    let base = voices::find(Some(&z), &a.base).await.map_err(err_chain)?;
    let cat = z.catalog().await.map_err(err_chain)?;
    let raw = cat.expand(&a.controls).map_err(err_chain)?;
    voices::create_designed(&z, &a.name, &base, raw, a.description).await.map_err(err_chain)
}

#[tauri::command]
async fn rename_voice(id: String, name: String) -> Res<Voice> {
    voices::rename(&id, &name).map_err(err_chain)
}

#[tauri::command]
async fn delete_voice(id: String) -> Res<()> {
    voices::delete(&id).map_err(err_chain)
}

/// The friendly control catalog (groups, controls, accents, presets).
#[tauri::command]
async fn controls() -> Res<Value> {
    let cache = cache_file("controls.json");
    let z = match engine::running().await {
        Some((_, z)) => z,
        None => {
            if let Some(v) = std::fs::read(&cache).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()) {
                return Ok(v);
            }
            zonos().await?
        }
    };
    let cat = z.catalog().await.map_err(err_chain)?;
    let doc = z.controls().await.map_err(err_chain)?;
    let moods: Vec<Value> = doc["ui"]["moods"]
        .as_array()
        .map(|a| a.iter().map(|m| json!({ "id": m["id"], "label": m["label"], "control": m["control"], "dose": m["dose"] })).collect())
        .unwrap_or_default();
    let out = json!({
        "groups": cat.groups,
        "controls": cat.controls,
        "accents": cat.accents,
        "presets": cat.presets,
        "preview_text": cat.preview_text,
        "moods": moods,
        "directions": cat.directions,
        "raw_sets": cat.raw_sets,
    });
    let _ = std::fs::write(&cache, serde_json::to_vec(&out).unwrap_or_default());
    Ok(out)
}

/// Join kept takes (in order) into one WAV take, e.g. a whole script.
#[tauri::command]
async fn export_takes(ids: Vec<String>, gap_ms: Option<u32>) -> Res<Take> {
    let mut wavs = Vec::new();
    let mut parts = Vec::new();
    let mut names: Vec<String> = Vec::new();
    for id in &ids {
        let t = takes::find(id).ok_or_else(|| format!("no take {id}"))?;
        wavs.push(std::fs::read(&t.path).map_err(err)?);
        parts.push(ScriptLine { voice: t.voice_name.clone(), text: t.text.clone() });
        if !names.contains(&t.voice_name) {
            names.push(t.voice_name.clone());
        }
    }
    if wavs.is_empty() {
        return Err("nothing to export".into());
    }
    let wav = takes::concat_wavs(&wavs, gap_ms.unwrap_or(350)).map_err(err_chain)?;
    let take = Take {
        id: String::new(),
        text: parts.iter().map(|p| p.text.as_str()).collect::<Vec<_>>().join(" "),
        voice_id: "script".into(),
        voice_name: names.join(" + "),
        created_at: booth_core::chrono::Utc::now(),
        seconds: 0.0,
        seed: None,
        mode: None,
        controls: BTreeMap::new(),
        decoder: None,
        via: "app".into(),
        parts,
        starred: true,
        path: Default::default(),
    };
    takes::save(take, &wav).map_err(err_chain)
}

// ---------- speaking ----------

#[derive(Deserialize)]
struct SpeakArgs {
    text: String,
    voice: Option<String>,
    /// "exact" or "shapeable"; None = the voice's own.
    mode: Option<String>,
    #[serde(default)]
    controls: BTreeMap<String, f64>,
    seed: Option<u64>,
}

fn mode_of(s: Option<&str>) -> Option<SpeakerMode> {
    match s {
        Some("exact") => Some(SpeakerMode::Reference),
        Some("shapeable") => Some(SpeakerMode::Embedding),
        _ => None,
    }
}

#[tauri::command]
async fn speak(a: SpeakArgs) -> Res<Take> {
    let z = zonos().await?;
    let opts = SpeakOptions {
        voice: a.voice,
        mode: mode_of(a.mode.as_deref()),
        controls: a.controls,
        seed: a.seed,
        language: None,
        via: "app".into(),
    };
    studio::speak(&z, &a.text, &opts).await.map_err(err_chain)
}

#[tauri::command]
async fn render_script(lines: Vec<ScriptLine>, gap_ms: Option<u32>) -> Res<Take> {
    let z = zonos().await?;
    studio::render_script(&z, &lines, gap_ms.unwrap_or(350), "app").await.map_err(err_chain)
}

#[tauri::command]
async fn list_takes(limit: Option<usize>) -> Res<Vec<Take>> {
    Ok(takes::list(limit.unwrap_or(50)))
}

#[tauri::command]
async fn star_take(id: String, starred: bool) -> Res<Take> {
    takes::set_starred(&id, starred).map_err(err_chain)
}

#[tauri::command]
async fn delete_take(id: String) -> Res<()> {
    takes::delete(&id).map_err(err_chain)
}

// ---------- settings and Claude ----------

#[tauri::command]
async fn get_settings() -> Res<settings::Settings> {
    Ok(settings::load())
}

#[tauri::command]
async fn save_settings(s: settings::Settings) -> Res<settings::Settings> {
    settings::save(&s).map_err(err_chain)?;
    Ok(s)
}

#[derive(Serialize)]
struct ClaudeInfo {
    /// The one-click bundle, if it shipped with the app (or was built in the repo).
    mcpb: Option<PathBuf>,
    /// booth-mcp.exe, for a manual `claude_desktop_config.json` entry or Claude Code.
    exe: Option<PathBuf>,
    /// Claude Desktop's config folder exists.
    desktop_found: bool,
    /// Booth appears in Claude Desktop's config or extensions.
    connected: bool,
    config_snippet: Value,
}

fn find_resource(app: &AppHandle, name: &str, dev: &[&str]) -> Option<PathBuf> {
    if let Ok(dir) = app.path().resource_dir() {
        let p = dir.join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(p) = exe.parent().map(|d| d.join(name)).filter(|p| p.is_file()) {
            return Some(p);
        }
    }
    // Running from the repo (`tauri dev`): the workspace root is three levels up from src-tauri.
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("..");
    dev.iter().map(|d| root.join(d)).find(|p| p.is_file()).and_then(|p| p.canonicalize().ok())
}

#[tauri::command]
async fn claude_info(app: AppHandle) -> Res<ClaudeInfo> {
    let mcpb = find_resource(&app, "booth.mcpb", &["dist/booth.mcpb"]);
    let exe = find_resource(&app, "booth-mcp.exe", &["target/release/booth-mcp.exe", "target/debug/booth-mcp.exe"]);
    let cfg_dir = std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("Claude"));
    let desktop_found = cfg_dir.as_ref().is_some_and(|d| d.is_dir());
    let mut connected = false;
    if let Some(d) = &cfg_dir {
        if let Ok(text) = std::fs::read_to_string(d.join("claude_desktop_config.json")) {
            connected |= serde_json::from_str::<Value>(&text).ok().is_some_and(|v| v["mcpServers"].get("booth").is_some());
        }
        if let Ok(rd) = std::fs::read_dir(d.join("Claude Extensions")) {
            connected |= rd.filter_map(|e| e.ok()).any(|e| e.file_name().to_string_lossy().to_lowercase().contains("booth"));
        }
    }
    let config_snippet = json!({ "mcpServers": { "booth": { "command": exe.as_ref().map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|| "booth-mcp.exe".into()) } } });
    Ok(ClaudeInfo { mcpb, exe, desktop_found, connected, config_snippet })
}

/// Shows a take or folder in Explorer. Only paths inside Booth's own folders.
#[tauri::command]
async fn reveal(app: AppHandle, path: PathBuf) -> Res<()> {
    use tauri_plugin_opener::OpenerExt;
    let ok = [paths::home(), paths::takes_dir(), paths::voices_dir(), paths::models_dir()]
        .iter()
        .any(|d| path.starts_with(d));
    if !ok {
        return Err(format!("{} is not a Booth folder", path.display()));
    }
    if path.is_dir() {
        app.opener().open_path(path.to_string_lossy(), None::<&str>).map_err(err)
    } else {
        app.opener().reveal_item_in_dir(&path).map_err(err)
    }
}

/// Opens booth.mcpb with Claude Desktop, which asks the user to confirm the install.
#[tauri::command]
async fn install_mcpb(app: AppHandle) -> Res<()> {
    use tauri_plugin_opener::OpenerExt;
    let mcpb = find_resource(&app, "booth.mcpb", &["dist/booth.mcpb"]).ok_or("booth.mcpb was not found next to the app")?;
    app.opener().open_path(mcpb.to_string_lossy(), None::<&str>).map_err(err)
}

/// Lets the asset protocol serve Booth's own folders (takes, voice clips, previews),
/// including ones moved with BOOTH_HOME / BOOTH_TAKES_DIR.
fn allow_media(app: &AppHandle) {
    let scope = app.asset_protocol_scope();
    for dir in [paths::home(), paths::takes_dir(), paths::voices_dir()] {
        let _ = paths::ensure_dir(&dir);
        let _ = scope.allow_directory(&dir, true);
    }
}

static STOPPING: AtomicBool = AtomicBool::new(false);

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            allow_media(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            overview,
            system_check,
            pack_info,
            download_packs,
            engine_start,
            engine_stop,
            engine_restart,
            list_voices,
            voice_preview,
            create_clone,
            design_voice,
            rename_voice,
            delete_voice,
            controls,
            export_takes,
            speak,
            render_script,
            list_takes,
            star_take,
            delete_take,
            get_settings,
            save_settings,
            claude_info,
            reveal,
            install_mcpb,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Booth")
        .run(|app, event| {
            // Free the GPU when the window closes, unless the user keeps the engine for Claude.
            // stop_when_idle waits for any render in flight; it never kills a running job.
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                if settings::load().keep_engine_running || STOPPING.swap(true, Ordering::SeqCst) {
                    return;
                }
                api.prevent_exit();
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = engine::stop_when_idle(Duration::from_secs(60)).await;
                    app.exit(0);
                });
            }
        });
}
