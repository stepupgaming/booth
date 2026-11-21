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
use booth_core::audio::{self, Format};
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
    /// An NVIDIA card (Windows, Linux) or the Apple chip (macOS).
    gpu: Option<String>,
    /// The card's memory, or the Mac's unified memory.
    vram_gb: Option<f64>,
    /// "nvidia", "apple" or "none": what the graphics card can do for Booth.
    gpu_kind: &'static str,
    cpu: Option<String>,
    cpu_threads: usize,
    ram_gb: f64,
    disk: Option<String>,
    disk_free_gb: Option<f64>,
    os: Option<String>,
    platform: &'static str,
    engine_found: bool,
}

/// Graphics card, processor, free disk where the models go, and OS, for the first setup screen.
#[tauri::command]
async fn system_check() -> Res<SystemCheck> {
    use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};
    let sys = System::new_with_specifics(
        RefreshKind::nothing().with_cpu(CpuRefreshKind::nothing()).with_memory(MemoryRefreshKind::nothing().with_ram()),
    );
    let cpu = sys.cpus().first().map(|c| c.brand().trim().to_string()).filter(|s| !s.is_empty());
    // On Android /proc/cpuinfo lists only the cores that are awake, so a phone reports 1 core while
    // the little cluster is parked. The affinity mask knows how many it may use.
    let affinity = std::thread::available_parallelism().map_or(1, |n| n.get());
    let cpu_threads = sys.cpus().len().max(affinity).max(1);
    let ram_gb = (sys.total_memory() as f64 / 1e9).round();
    let (gpu, vram_gb, gpu_kind) = if cfg!(target_os = "macos") {
        let chip = tokio::task::spawn_blocking(apple_chip).await.map_err(err)?;
        match chip {
            Some(name) => (Some(name), Some(ram_gb), "apple"),
            None => (None, None, "none"),
        }
    } else {
        let (name, vram) = tokio::task::spawn_blocking(nvidia_gpu).await.map_err(err)?;
        let kind = if name.is_some() { "nvidia" } else { "none" };
        (name, vram, kind)
    };
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
        disk: disk.map(|d| d.mount_point().to_string_lossy().trim_end_matches('\\').to_string()),
        disk_free_gb: disk.map(|d| d.available_space() as f64 / 1e9),
        os: sysinfo::System::long_os_version(),
        platform: std::env::consts::OS,
        engine_found: !engine::choices(settings::load().device).is_empty(),
        gpu,
        vram_gb,
        gpu_kind,
        cpu,
        cpu_threads,
        ram_gb,
    })
}

/// "Apple M2 Pro" on Apple Silicon; None on an Intel Mac (no Metal build for those).
fn apple_chip() -> Option<String> {
    if !cfg!(target_arch = "aarch64") {
        return None;
    }
    let out = std::process::Command::new("sysctl").args(["-n", "machdep.cpu.brand_string"]).output().ok()?;
    let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Some(if name.is_empty() { "Apple Silicon".into() } else { name })
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
    let client = booth_core::http_client();
    let models_dir = paths::models_dir();
    let mut out = Vec::new();
    for pack in Pack::ALL.iter().copied() {
        let bytes = match models::plan(&client, pack).await {
            Ok(files) => files.iter().map(|f| f.size).sum(),
            Err(_) => match pack {
                Pack::Core => 1_160_000_000,
                Pack::Exact => 5_600_000_000,
                Pack::FullDecoder => 2_200_000_000,
                Pack::GpuEngine => models::CUDA_ENGINE_SIZE,
                Pack::VulkanEngine => models::VULKAN_ENGINE_SIZE,
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
        let client = booth_core::http_client();
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
        temperature: None,
        voice_match: None,
        redone: 0,
        drifted: false,
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
    #[serde(default)]
    temperature: Option<f64>,
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
        temperature: a.temperature,
        // The app sends a fresh random seed per take, so a redo with another seed is fine.
        drift_check: true,
    };
    studio::speak(&z, &a.text, &opts).await.map_err(err_chain)
}

/// Say a word from the pronunciation list in this voice (the saved list applied), without
/// saving a take. Only the newest file is kept.
#[tauri::command]
async fn hear_pronunciation(text: String, voice: Option<String>) -> Res<PathBuf> {
    let z = zonos().await?;
    let opts = SpeakOptions { voice, via: "app".into(), ..Default::default() };
    let (_, wav) = studio::speak_audio(&z, &text, &opts).await.map_err(err_chain)?;
    let dir = paths::home().join("cache");
    paths::ensure_dir(&dir).map_err(err)?;
    if let Ok(old) = std::fs::read_dir(&dir) {
        for f in old.flatten().filter(|f| f.file_name().to_string_lossy().starts_with("pronounce-")) {
            let _ = std::fs::remove_file(f.path());
        }
    }
    // A new name each time, so the web view doesn't play an older file from its cache.
    let file = dir.join(format!("pronounce-{}.wav", booth_core::chrono::Utc::now().timestamp_millis()));
    std::fs::write(&file, wav).map_err(err)?;
    Ok(file)
}

#[tauri::command]
async fn render_script(lines: Vec<ScriptLine>, gap_ms: Option<u32>) -> Res<Take> {
    let z = zonos().await?;
    studio::render_script(&z, &lines, gap_ms.unwrap_or(350), "app").await.map_err(err_chain)
}

// ---------- scripts (a whole script made line by line, in the background) ----------

use booth_core::batch::{self, ExportOptions, Exported, Job};
use booth_core::script;

/// Split pasted text into lines, speakers and left-out notes (as you type).
#[tauri::command]
async fn script_parse(text: String) -> Res<script::Parsed> {
    Ok(script::parse(&text, &[]))
}

/// Read a document (.docx, .pdf, .epub, .md, .html, .rtf, .srt, .fountain, .txt...) as script text.
/// Desktop sends a path (picked or dropped); Android and the file picker send the bytes, base64.
#[tauri::command]
async fn script_open(name: Option<String>, path: Option<PathBuf>, data: Option<String>) -> Res<String> {
    let bytes = match data {
        Some(d) => Some(base64::engine::general_purpose::STANDARD.decode(d.as_bytes()).map_err(err)?),
        None => None,
    };
    tauri::async_runtime::spawn_blocking(move || match (path, bytes) {
        (_, Some(b)) => booth_core::doc::open_bytes(name.as_deref().unwrap_or(""), &b),
        (Some(p), None) => booth_core::doc::open_path(&p),
        (None, None) => Err(anyhow::anyhow!("no file given")),
    })
    .await
    .map_err(err)?
    .map_err(err_chain)
}

/// Start (or keep going) making a script's lines; progress comes as `script` events.
fn start_run(app: &AppHandle, id: String) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let result = async {
            let z = zonos().await?;
            let app2 = app.clone();
            let r = batch::run(&z, &id, move |j: &Job| {
                let _ = app2.emit("script", j);
                #[cfg(target_os = "android")]
                if j.running {
                    let n = j.lines.iter().position(|l| l.state == batch::LineState::Making).map_or(j.done(), |i| i + 1);
                    android_work::status(Some(("Making your script", &format!("Line {n} of {}", j.lines.len()))));
                }
            })
            .await
            .map_err(err_chain);
            #[cfg(target_os = "android")]
            android_work::status(None);
            r
        }
        .await;
        if let Err(e) = result {
            booth_core::diag::note(format!("script {id}: {e}"));
            let _ = app.emit("script-error", json!({ "id": id, "error": e }));
            if let Ok(j) = batch::get(&id) {
                let _ = app.emit("script", &j);
            }
        }
    });
}

/// Save a pasted script with a voice per speaker and start making it.
#[tauri::command]
async fn script_create(app: AppHandle, text: String, voices: BTreeMap<String, String>, fallback: String) -> Res<Job> {
    let z = zonos().await?;
    let job = batch::create(&z, &text, &voices, &fallback, "app").await.map_err(err_chain)?;
    start_run(&app, job.id.clone());
    Ok(job)
}

#[tauri::command]
async fn script_run(app: AppHandle, id: String) -> Res<Job> {
    let job = batch::get(&id).map_err(err_chain)?;
    start_run(&app, id);
    Ok(job)
}

#[tauri::command]
async fn script_stop() -> Res<()> {
    batch::stop();
    Ok(())
}

/// Make one line again (with new words, if given).
#[tauri::command]
async fn script_redo(app: AppHandle, id: String, index: usize, text: Option<String>) -> Res<Job> {
    let job = batch::redo(&id, index, text).map_err(err_chain)?;
    start_run(&app, id);
    Ok(job)
}

#[tauri::command]
async fn script_get(id: String) -> Res<Job> {
    batch::get(&id).map_err(err_chain)
}

/// Recent scripts, newest first.
#[tauri::command]
async fn script_list(limit: Option<usize>) -> Res<Vec<Job>> {
    Ok(batch::list().into_iter().take(limit.unwrap_or(20)).collect())
}

#[tauri::command]
async fn script_captions(id: String, options: Option<ExportOptions>) -> Res<String> {
    let job = batch::get(&id).map_err(err_chain)?;
    Ok(batch::captions_preview(&job, &options.unwrap_or_default()))
}

#[tauri::command]
async fn script_export(id: String, options: Option<ExportOptions>) -> Res<Exported> {
    // A long script takes a while to write (and to even out, read twice more), so it runs off
    // the async threads.
    tokio::task::spawn_blocking(move || batch::export(&id, &options.unwrap_or_default()))
        .await
        .map_err(err)?
        .map_err(err_chain)
}

/// Where a take is written in another format before it leaves Booth: Studio's Export on a
/// computer, and the share sheet or Save to phone on a phone (which copy it out of here).
fn exports_dir() -> PathBuf {
    paths::home().join("exports")
}

/// A plain file name from a take's words ("Cold phones are the worst"), at most 60 characters.
fn take_file_name(name: &str) -> String {
    let s: String = name
        .chars()
        // "didn't" stays one word: apostrophes go, other punctuation becomes a space.
        .filter(|c| !matches!(c, '\'' | '\u{2019}'))
        .map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let s: String = s.chars().take(60).collect::<String>().trim_end().to_string();
    if s.is_empty() { "Booth".into() } else { s }
}

fn booth_audio(path: &std::path::Path) -> Res<()> {
    if !path.starts_with(paths::home()) || !path.is_file() {
        return Err(format!("{} is not one of Booth's takes", path.display()));
    }
    Ok(())
}

/// Write a take (a WAV in Booth's folder) as `format`, named after `name`, into the exports
/// folder. On a phone the folder only holds the latest one: the share sheet and Save to phone copy
/// it out straight away.
#[tauri::command]
async fn convert_take(path: PathBuf, format: Format, name: Option<String>, even: Option<bool>) -> Res<PathBuf> {
    booth_audio(&path)?;
    let dir = exports_dir();
    #[cfg(mobile)]
    let _ = std::fs::remove_dir_all(&dir);
    paths::ensure_dir(&dir).map_err(err)?;
    let stem = take_file_name(name.as_deref().unwrap_or("Booth"));
    let dst = dir.join(format!("{stem}.{}", format.ext()));
    let src = path.clone();
    let out = dst.clone();
    tokio::task::spawn_blocking(move || audio::convert(&src, &out, format, even.unwrap_or(false)))
        .await
        .map_err(err)?
        .map_err(err_chain)?;
    Ok(dst)
}

#[derive(Serialize)]
struct SavedAs {
    path: PathBuf,
    format: Format,
}

/// The Library's "Save as…" on a computer: the system's save dialog, with the remembered format
/// first. The file type picked in the dialog (or the extension typed) decides the format. `None`
/// when the dialog is cancelled.
#[tauri::command]
async fn save_as(app: AppHandle, path: PathBuf, format: Format, name: Option<String>) -> Res<Option<SavedAs>> {
    booth_audio(&path)?;
    #[cfg(desktop)]
    {
        use tauri_plugin_dialog::DialogExt;
        let label = |f: Format| match f {
            Format::Wav => "WAV",
            Format::Mp3 => "MP3",
            Format::M4a => "M4A (AAC)",
            Format::Flac => "FLAC",
            Format::Ogg => "OGG (Opus)",
        };
        let stem = take_file_name(name.as_deref().unwrap_or("Booth"));
        let mut dialog = app.dialog().file().set_title("Save as").set_file_name(format!("{stem}.{}", format.ext()));
        let order = std::iter::once(format).chain(Format::ALL.into_iter().filter(|f| *f != format));
        for f in order {
            dialog = dialog.add_filter(label(f), &[f.ext()]);
        }
        if let Ok(dir) = app.path().audio_dir() {
            dialog = dialog.set_directory(dir);
        }
        let picked = tokio::task::spawn_blocking(move || dialog.blocking_save_file()).await.map_err(err)?;
        let Some(picked) = picked else { return Ok(None) };
        let mut dst = picked.into_path().map_err(err)?;
        let chosen = match dst.extension().and_then(|e| e.to_str()).and_then(Format::from_ext) {
            Some(f) => f,
            None => {
                let mut name = dst.file_name().map(|n| n.to_os_string()).unwrap_or_default();
                name.push(format!(".{}", format.ext()));
                dst.set_file_name(name);
                format
            }
        };
        let (src, out) = (path.clone(), dst.clone());
        tokio::task::spawn_blocking(move || audio::convert(&src, &out, chosen, false))
            .await
            .map_err(err)?
            .map_err(err_chain)?;
        Ok(Some(SavedAs { path: dst, format: chosen }))
    }
    #[cfg(mobile)]
    {
        let _ = (app, format, name);
        Err("on a phone, use Save to phone or Share".into())
    }
}

/// Android: copy files into the phone's shared storage, audio to Music/Booth and captions to
/// Download/Booth, where the Files app, music players and video editors find them. Returns where
/// each one went ("Music/Booth/Cold phones.m4a").
#[tauri::command]
async fn save_to_phone(paths: Vec<PathBuf>) -> Res<Vec<String>> {
    let home = paths::home();
    if paths.is_empty() || paths.iter().any(|p| !p.starts_with(&home) || !p.is_file()) {
        return Err("only Booth's own files can be saved".into());
    }
    #[cfg(target_os = "android")]
    {
        paths.iter().map(|p| android_work::save_to_phone(p)).collect()
    }
    #[cfg(not(target_os = "android"))]
    {
        Err("Save to phone works on Android; on a computer, use Save as…".into())
    }
}

/// Hand files to another app: Android's share sheet. (Desktop shows them in the folder instead.)
#[tauri::command]
async fn share_files(paths: Vec<PathBuf>, title: Option<String>) -> Res<()> {
    let home = paths::home();
    if paths.is_empty() || paths.iter().any(|p| !p.starts_with(&home) || !p.is_file()) {
        return Err("only Booth's own files can be shared".into());
    }
    #[cfg(target_os = "android")]
    {
        android_work::share(&paths, title.as_deref().unwrap_or("Booth"))
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = title;
        Err("sharing works on phones; on a computer, use Show in folder".into())
    }
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
    /// booth-mcp, for a manual `claude_desktop_config.json` entry or Claude Code.
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
    let mcp = format!("booth-mcp{}", std::env::consts::EXE_SUFFIX);
    let exe = find_resource(&app, &mcp, &[&format!("target/release/{mcp}"), &format!("target/debug/{mcp}")]);
    let cfg_dir = claude_desktop_dir();
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
    let config_snippet = json!({ "mcpServers": { "booth": { "command": exe.as_ref().map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|| mcp.clone()) } } });
    Ok(ClaudeInfo { mcpb, exe, desktop_found, connected, config_snippet })
}

/// Claude Desktop's config folder: %APPDATA%\Claude on Windows, ~/Library/Application
/// Support/Claude on macOS. There is no Claude Desktop for Linux.
fn claude_desktop_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("Claude"))
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library").join("Application Support").join("Claude"))
    } else {
        None
    }
}

// ---------- updates ----------

#[derive(Serialize)]
struct UpdateInfo {
    version: String,
    current: String,
    notes: Option<String>,
}

/// The updater, pointed at `BOOTH_UPDATE_URL` when that is set (to test a release before it is
/// "latest").
#[cfg(desktop)]
fn updater(app: &AppHandle) -> Res<tauri_plugin_updater::Updater> {
    use tauri_plugin_updater::UpdaterExt;
    let mut b = app.updater_builder();
    if let Some(url) = std::env::var("BOOTH_UPDATE_URL").ok().filter(|u| !u.is_empty()) {
        b = b.endpoints(vec![url.parse().map_err(err)?]).map_err(err)?;
    }
    b.build().map_err(err)
}

/// A newer Booth on GitHub Releases, or None.
#[tauri::command]
async fn check_update(app: AppHandle) -> Res<Option<UpdateInfo>> {
    #[cfg(desktop)]
    {
        let update = updater(&app)?.check().await.map_err(err)?;
        Ok(update.map(|u| UpdateInfo { version: u.version.clone(), current: u.current_version.clone(), notes: u.body.clone() }))
    }
    #[cfg(mobile)]
    {
        let _ = app;
        Ok(None)
    }
}

#[derive(Clone, Serialize)]
struct UpdateProgress {
    done_bytes: u64,
    total_bytes: Option<u64>,
}

/// Download the update (checked against Booth's signing key), stop the engine once it is idle so
/// its files can be replaced, install, and restart.
#[cfg(mobile)]
#[tauri::command]
async fn install_update(_app: AppHandle) -> Res<()> {
    Err("updates come from a new APK on phones".into())
}

#[cfg(desktop)]
#[tauri::command]
async fn install_update(app: AppHandle) -> Res<()> {
    let update = updater(&app)?.check().await.map_err(err)?.ok_or("Booth is already up to date")?;
    let mut done = 0u64;
    let app2 = app.clone();
    let bytes = update
        .download(
            move |chunk, total| {
                done += chunk as u64;
                let _ = app2.emit("update-progress", UpdateProgress { done_bytes: done, total_bytes: total });
            },
            || {},
        )
        .await
        .map_err(err)?;
    let _ = engine::stop_when_idle(Duration::from_secs(120)).await;
    STOPPING.store(true, Ordering::SeqCst);
    update.install(bytes).map_err(err)?;
    app.restart();
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

/// Opens one of Booth's own web pages (nofacetube.com, the GitHub repo) in the browser.
#[tauri::command]
async fn open_link(app: AppHandle, url: String) -> Res<()> {
    use tauri_plugin_opener::OpenerExt;
    let lower = url.to_ascii_lowercase();
    let ok = ["https://nofacetube.com/", "https://github.com/stepupgaming/"].iter().any(|p| lower.starts_with(p));
    if !ok {
        return Err(format!("{url} is not a Booth page"));
    }
    app.opener().open_url(url, None::<&str>).map_err(err)
}

/// Booth's log and the engine's, newest last, for the Settings page and bug reports.
#[tauri::command]
async fn read_log() -> Res<String> {
    Ok(booth_core::diag::report(300))
}

/// Settings > Log > Save a report: one zip with Booth's logs. On a computer it lands on the Desktop
/// and is shown in its folder; on a phone it opens the share sheet. Returns where the file is.
#[tauri::command]
async fn save_report(app: AppHandle) -> Res<String> {
    let about = format!(
        "Booth {}\n{} {}\nRun on: {:?}\n\nengine.json:\n{}\n",
        app.package_info().version,
        std::env::consts::OS,
        std::env::consts::ARCH,
        settings::load().device,
        std::fs::read_to_string(paths::engine_lock_file()).unwrap_or_else(|_| "(no engine running)".into())
    );
    #[cfg(target_os = "android")]
    {
        let dir = paths::home().join("reports");
        let path = booth_core::diag::save_report(&dir, &about).map_err(err_chain)?;
        android_work::share(std::slice::from_ref(&path), "Booth report")?;
        Ok(path.to_string_lossy().into_owned())
    }
    #[cfg(not(target_os = "android"))]
    {
        use tauri_plugin_opener::OpenerExt;
        let dir = booth_core::diag::report_folder();
        let path = booth_core::diag::save_report(&dir, &about).map_err(err_chain)?;
        let _ = app.opener().reveal_item_in_dir(&path);
        Ok(path.to_string_lossy().into_owned())
    }
}

/// A line from the page (it went to the background, an error it showed) into booth.log.
#[tauri::command]
async fn log_note(text: String) -> Res<()> {
    booth_core::diag::note(format!("ui: {}", text.chars().take(500).collect::<String>()));
    Ok(())
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

/// Kept for the life of the process: it tells booth-mcp that a Booth window is open, so Claude
/// closing down doesn't take the engine away from it.
static HOLD: std::sync::OnceLock<engine::Holder> = std::sync::OnceLock::new();

/// Android: Booth's folder is the app's own storage (there is no `%LOCALAPPDATA%`), and the
/// engine's LACI heads, built into the app, are unpacked next to it. The engine program itself
/// ships in the native library folder (`paths::server_exe`).
#[cfg(target_os = "android")]
fn android_setup(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    static LACI_HEADS: include_dir::Dir = include_dir::include_dir!("$CARGO_MANIFEST_DIR/android-engine/laci_heads");
    if std::env::var_os("BOOTH_HOME").is_none() {
        std::env::set_var("BOOTH_HOME", app.path().app_local_data_dir()?);
    }
    let heads = paths::home().join("engine").join("laci_heads");
    paths::ensure_dir(&heads)?;
    for f in LACI_HEADS.files() {
        let dest = heads.join(f.path());
        if std::fs::read(&dest).ok().as_deref() != Some(f.contents()) {
            std::fs::write(&dest, f.contents())?;
        }
    }
    Ok(())
}

/// Android: keep RenderService (a foreground service with a notification) up while the engine
/// loads or renders, so the system doesn't freeze Booth or kill the engine when you switch apps.
/// It stays up a few seconds after the last take, so the next line of a script, asked for right
/// after, doesn't have to start it again (Android won't allow that from the background).
#[cfg(target_os = "android")]
mod android_work {
    use jni::objects::{JClass, JObject, JValue};
    use std::time::Duration;

    const LINGER: Duration = Duration::from_secs(10);

    pub fn install() {
        booth_core::engine::on_work(|on| {
            if on {
                set(true);
            } else {
                std::thread::spawn(|| {
                    std::thread::sleep(LINGER);
                    if booth_core::engine::working() == 0 {
                        set(false);
                    }
                });
            }
        });
    }

    fn set(on: bool) {
        let r = with_class("com.nofacetube.booth.RenderService", |env, class, context| {
            env.call_static_method(
                class,
                "set",
                "(Landroid/content/Context;Z)V",
                &[JValue::Object(context), JValue::Bool(on as u8)],
            )?;
            Ok(())
        });
        if let Err(e) = r {
            booth_core::diag::note(format!("render service: couldn't turn it {}: {e}", if on { "on" } else { "off" }));
        }
    }

    /// What the render notification says ("Making line 4 of 7"); `None` puts back the plain words.
    pub fn status(words: Option<(&str, &str)>) {
        let _ = with_class("com.nofacetube.booth.RenderService", |env, class, context| {
            match words {
                Some((title, text)) => {
                    let (t, x) = (env.new_string(title)?, env.new_string(text)?);
                    env.call_static_method(
                        class,
                        "status",
                        "(Landroid/content/Context;Ljava/lang/String;Ljava/lang/String;)V",
                        &[JValue::Object(context), JValue::Object(&t), JValue::Object(&x)],
                    )?;
                }
                None => {
                    env.call_static_method(class, "plain", "()V", &[])?;
                }
            }
            Ok(())
        });
    }

    /// Opens Android's share sheet with these files.
    pub fn share(paths: &[std::path::PathBuf], title: &str) -> Result<(), String> {
        with_class("com.nofacetube.booth.Share", |env, class, context| {
            let string = env.find_class("java/lang/String")?;
            let empty = env.new_string("")?;
            let arr = env.new_object_array(paths.len() as i32, &string, &empty)?;
            for (i, p) in paths.iter().enumerate() {
                let s = env.new_string(p.to_string_lossy())?;
                env.set_object_array_element(&arr, i as i32, &s)?;
            }
            let t = env.new_string(title)?;
            env.call_static_method(
                class,
                "files",
                "(Landroid/content/Context;[Ljava/lang/String;Ljava/lang/String;)V",
                &[JValue::Object(context), JValue::Object(&arr), JValue::Object(&t)],
            )?;
            Ok(())
        })
    }

    /// Copies a file into shared storage through MediaStore (Share.save); returns where it went.
    pub fn save_to_phone(path: &std::path::Path) -> Result<String, String> {
        let mut out = String::new();
        with_class("com.nofacetube.booth.Share", |env, class, context| {
            let p = env.new_string(path.to_string_lossy())?;
            let r = env
                .call_static_method(
                    class,
                    "save",
                    "(Landroid/content/Context;Ljava/lang/String;)Ljava/lang/String;",
                    &[JValue::Object(context), JValue::Object(&p)],
                )?
                .l()?;
            out = env.get_string(&jni::objects::JString::from(r))?.into();
            Ok(())
        })?;
        // Share.save answers "ok:<where>" or "error:<why>", so the reason reaches the toast.
        match out.split_once(':') {
            Some(("ok", place)) => Ok(place.to_string()),
            Some(("error", why)) => Err(why.to_string()),
            _ => Err(format!("Save to phone: unexpected answer {out:?}")),
        }
    }

    /// Runs `f` with one of the app's own Kotlin classes and the application context.
    fn with_class(
        name: &str,
        f: impl FnOnce(&mut jni::JNIEnv, &JClass, &JObject) -> jni::errors::Result<()>,
    ) -> Result<(), String> {
        let ctx = ndk_context::android_context();
        // SAFETY: tao hands ndk-context the process's JavaVM and a global ref to the application
        // context; both live as long as the process.
        let vm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) }.map_err(|e| e.to_string())?;
        let mut env = vm.attach_current_thread().map_err(|e| e.to_string())?;
        let context = unsafe { JObject::from_raw(ctx.context().cast()) };
        let r = (|| -> jni::errors::Result<()> {
            // A native thread's FindClass only sees system classes: load ours through the app's loader.
            let loader = env.call_method(&context, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])?.l()?;
            let jname = env.new_string(name)?;
            let class = env
                .call_method(&loader, "loadClass", "(Ljava/lang/String;)Ljava/lang/Class;", &[JValue::Object(&jname)])?
                .l()?;
            let class = JClass::from(class);
            f(&mut env, &class, &context)
        })();
        if r.is_err() && env.exception_check().unwrap_or(false) {
            let _ = env.exception_describe();
            let _ = env.exception_clear();
        }
        r.map_err(|e| e.to_string())
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default().plugin(tauri_plugin_opener::init());
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_updater::Builder::new().build()).plugin(tauri_plugin_dialog::init());
    builder
        .setup(|app| {
            #[cfg(target_os = "android")]
            {
                android_setup(app.handle())?;
                android_work::install();
            }
            booth_core::diag::set_source("app");
            booth_core::diag::note(format!(
                "app {} started on {} {}",
                app.package_info().version,
                std::env::consts::OS,
                std::env::consts::ARCH
            ));
            allow_media(app.handle());
            // So booth-mcp doesn't free the engine from under this window when Claude closes down.
            let _ = HOLD.set(engine::Holder::new("the Booth app"));
            // So booth-mcp, installed in Claude's own folder, finds the engine that shipped here.
            if let Ok(dir) = app.path().resource_dir() {
                paths::record_install(&dir);
                // Booth isn't notarized yet: once the person has opened the app, clear the download
                // quarantine from the engine and booth-mcp too, so macOS doesn't block them on start.
                #[cfg(target_os = "macos")]
                let _ = std::process::Command::new("xattr").arg("-dr").arg("com.apple.quarantine").arg(&dir).status();
            }
            // A Booth update moved to a newer NVIDIA engine: fetch it quietly, keep the old one meanwhile.
            if models::cuda_engine_outdated() && !DOWNLOADING.swap(true, Ordering::SeqCst) {
                tauri::async_runtime::spawn(async {
                    let _ = models::upgrade_cuda_engine(&booth_core::http_client()).await;
                    DOWNLOADING.store(false, Ordering::SeqCst);
                });
            } else {
                tauri::async_runtime::spawn(models::remove_older_cuda_engines());
            }
            // Same for the Vulkan engine (beta), when the person uses it.
            if models::vulkan_engine_outdated() && !DOWNLOADING.swap(true, Ordering::SeqCst) {
                tauri::async_runtime::spawn(async {
                    let _ = models::upgrade_vulkan_engine(&booth_core::http_client()).await;
                    DOWNLOADING.store(false, Ordering::SeqCst);
                });
            }
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
            hear_pronunciation,
            create_clone,
            design_voice,
            rename_voice,
            delete_voice,
            controls,
            export_takes,
            speak,
            render_script,
            script_parse,
            script_open,
            script_create,
            script_run,
            script_stop,
            script_redo,
            script_get,
            script_list,
            script_captions,
            script_export,
            share_files,
            convert_take,
            save_as,
            save_to_phone,
            list_takes,
            star_take,
            delete_take,
            get_settings,
            save_settings,
            claude_info,
            reveal,
            open_link,
            install_mcpb,
            check_update,
            install_update,
            read_log,
            save_report,
            log_note,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Booth")
        .run(|app, event| {
            // Free the engine when the window closes, unless the user keeps the engine for Claude.
            // stop_when_idle waits for any render in flight; it never kills a running job.
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                batch::stop(); // finish the line being made, then let the engine go
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
