//! Voices: the defaults that ship with the model (served by the engine) and the user's own
//! (clones and designed voices), stored as folders under `%LOCALAPPDATA%\NoFaceTube\Booth\voices\<id>\`:
//!
//! - `voice.json`      what it is (this file's `Voice`)
//! - `clip.<ext>`      the reference clip, for clones (used by Exact mode)
//! - `embedding.npy`   the speaker embedding (used by Shapeable mode and designed voices)

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::api::{b64, GenerateRequest, SpeakerMode, Zonos};
use crate::paths;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceKind {
    /// Ships with ZONOS3-Mini.
    Default,
    /// Cloned from a clip the user recorded or picked.
    Clone,
    /// Shaped from another voice with steering controls.
    Designed,
}

/// Proof that the person whose voice was cloned agreed to it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Consent {
    /// The statement the user confirmed, word for word.
    pub statement: String,
    pub confirmed_at: chrono::DateTime<chrono::Utc>,
    /// `app` or `mcp`.
    pub via: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Voice {
    pub id: String,
    pub name: String,
    pub kind: VoiceKind,
    /// The mode used when this voice speaks.
    pub mode: SpeakerMode,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub accent: Option<String>,
    #[serde(default)]
    pub gender: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Default voices: the engine's speaker id. Designed voices: the voice they were shaped from.
    #[serde(default)]
    pub speaker_id: Option<String>,
    /// Modes the voice can use.
    #[serde(default)]
    pub modes: Vec<SpeakerMode>,
    /// Designed voices: the steering doses baked into the voice.
    #[serde(default)]
    pub controls: BTreeMap<String, f64>,
    /// Words of the reference clip, when known.
    #[serde(default)]
    pub transcript: Option<String>,
    #[serde(default)]
    pub consent: Option<Consent>,
    /// Folder of a user voice (not serialized into voice.json).
    #[serde(skip_deserializing, default)]
    pub dir: Option<PathBuf>,
}

impl Voice {
    fn file(&self, name: &str) -> Option<PathBuf> {
        self.dir.as_ref().map(|d| d.join(name)).filter(|p| p.is_file())
    }
    /// The reference clip (`clip.wav`, `clip.mp3`, ...).
    pub fn clip_path(&self) -> Option<PathBuf> {
        let dir = self.dir.as_ref()?;
        std::fs::read_dir(dir).ok()?.filter_map(|e| e.ok()).map(|e| e.path()).find(|p| {
            p.is_file() && p.file_stem().is_some_and(|s| s == "clip")
        })
    }
    fn clip_name(&self) -> String {
        self.clip_path()
            .and_then(|p| p.extension().map(|e| format!("{}.{}", self.id, e.to_string_lossy())))
            .unwrap_or_else(|| format!("{}.wav", self.id))
    }
    pub fn embedding_path(&self) -> Option<PathBuf> {
        self.file("embedding.npy")
    }

    /// Fill the speaker part of a request for this voice, in `mode` (or the voice's own mode).
    pub fn apply(&self, req: &mut GenerateRequest, mode: Option<SpeakerMode>) -> Result<()> {
        let mode = mode.unwrap_or(self.mode);
        if !self.modes.is_empty() && !self.modes.contains(&mode) {
            bail!(
                "{} can't speak in {} mode",
                self.name,
                if mode == SpeakerMode::Reference { "Exact" } else { "Shapeable" }
            );
        }
        req.speaker_mode = Some(mode);
        for (k, v) in &self.controls {
            req.controls.entry(k.clone()).or_insert(*v);
        }
        match self.kind {
            VoiceKind::Default => {
                req.speaker_embedding_id = self.speaker_id.clone();
            }
            VoiceKind::Clone | VoiceKind::Designed => match mode {
                SpeakerMode::Reference => {
                    let clip = self.clip_path().with_context(|| format!("{} has no clip", self.name))?;
                    req.speaker_audio_base64 = Some(b64(&std::fs::read(&clip)?));
                    req.speaker_audio_name = Some(self.clip_name());
                }
                SpeakerMode::Embedding => {
                    if let Some(emb) = self.embedding_path() {
                        req.speaker_embedding_base64 = Some(b64(&std::fs::read(&emb)?));
                        req.speaker_embedding_name = Some(format!("{}.npy", self.id));
                    } else if let Some(base) = &self.speaker_id {
                        req.speaker_embedding_id = Some(base.clone());
                    } else if let Some(clip) = self.clip_path() {
                        req.speaker_audio_base64 = Some(b64(&std::fs::read(&clip)?));
                        req.speaker_audio_name = Some(self.clip_name());
                    } else {
                        bail!("{} has neither a clip nor an embedding", self.name);
                    }
                }
            },
        }
        Ok(())
    }
}

fn language_name(code: &str) -> Option<&'static str> {
    Some(match code {
        "en" => "English",
        "hi" => "Hindi",
        "te" => "Telugu",
        "fr" => "French",
        "it" => "Italian",
        "zh" => "Mandarin Chinese",
        "pt" => "Portuguese",
        "de" => "German",
        "ko" => "Korean",
        "es" => "Spanish",
        "ja" => "Japanese",
        _ => return None,
    })
}

/// `Imogen_en-British_F_1.wav` → name Imogen, language English, style British, gender female.
fn parse_default_name(original: &str) -> (String, Option<String>, Option<String>, Option<String>) {
    let stem = Path::new(original)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(original);
    let parts: Vec<&str> = stem.split('_').collect();
    let name = parts.first().copied().unwrap_or(stem).to_string();
    let (lang, style) = match parts.get(1) {
        Some(tag) => {
            let mut it = tag.splitn(2, '-');
            let code = it.next().unwrap_or("");
            (language_name(code).map(str::to_string), it.next().map(str::to_string))
        }
        None => (None, None),
    };
    let gender = parts.get(2).and_then(|g| match *g {
        "F" => Some("female".to_string()),
        "M" => Some("male".to_string()),
        _ => None,
    });
    (name, lang, style, gender)
}

/// The engine's default voices as Booth voices.
pub async fn defaults(z: &Zonos) -> Result<Vec<Voice>> {
    let mut out = Vec::new();
    for sp in z.speakers().await? {
        let is_default = sp.extra.get("is_default").and_then(|v| v.as_bool()).unwrap_or(false);
        if !is_default {
            continue;
        }
        let original = sp
            .extra
            .get("original_name")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .or_else(|| sp.label.clone())
            .unwrap_or_else(|| sp.id.clone());
        let (name, language, accent, gender) = parse_default_name(&original);
        let modes: Vec<SpeakerMode> = sp
            .extra
            .get("speaker_modes")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        let mode = if modes.contains(&SpeakerMode::Reference) { SpeakerMode::Reference } else { SpeakerMode::Embedding };
        out.push(Voice {
            id: sp.id.clone(),
            name,
            kind: VoiceKind::Default,
            mode,
            language,
            accent,
            gender,
            description: None,
            created_at: None,
            speaker_id: Some(sp.id.clone()),
            modes,
            controls: BTreeMap::new(),
            transcript: sp.extra.get("transcript").and_then(|v| v.as_str()).map(str::to_string),
            consent: None,
            dir: None,
        });
    }
    out.sort_by(|a, b| (a.language.clone(), a.name.clone()).cmp(&(b.language.clone(), b.name.clone())));
    Ok(out)
}

/// The user's own voices, newest first.
pub fn mine() -> Vec<Voice> {
    let Ok(rd) = std::fs::read_dir(paths::voices_dir()) else { return Vec::new() };
    let mut out: Vec<Voice> = rd
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let dir = e.path();
            let text = std::fs::read_to_string(dir.join("voice.json")).ok()?;
            let mut v: Voice = serde_json::from_str(&text).ok()?;
            v.dir = Some(dir);
            Some(v)
        })
        .collect();
    out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    out
}

/// Every voice: the user's first, then the defaults (empty if the engine isn't running).
pub async fn all(z: Option<&Zonos>) -> Vec<Voice> {
    let mut list = mine();
    if let Some(z) = z {
        if let Ok(d) = defaults(z).await {
            list.extend(d);
        }
    }
    list
}

/// Find a voice by id, or by name (case-insensitive) when no id matches.
pub async fn find(z: Option<&Zonos>, key: &str) -> Result<Voice> {
    let list = all(z).await;
    if let Some(v) = list.iter().find(|v| v.id == key) {
        return Ok(v.clone());
    }
    let lower = key.to_lowercase();
    let mut by_name = list.iter().filter(|v| v.name.to_lowercase() == lower);
    match (by_name.next(), by_name.next()) {
        (Some(v), None) => Ok(v.clone()),
        (Some(_), Some(_)) => bail!("more than one voice is called '{key}'; use its id"),
        _ => bail!("no voice called '{key}' (list_voices shows them all)"),
    }
}

fn slug(name: &str) -> String {
    let s: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let s = s.trim_matches('-').to_string();
    let s = if s.is_empty() { "voice".into() } else { s };
    s.chars().take(32).collect()
}

fn new_dir(name: &str) -> Result<(String, PathBuf)> {
    let root = paths::voices_dir();
    paths::ensure_dir(&root)?;
    let base = slug(name);
    for n in 0..1000 {
        let id = if n == 0 { base.clone() } else { format!("{base}-{n}") };
        let dir = root.join(&id);
        if !dir.exists() {
            std::fs::create_dir_all(&dir)?;
            return Ok((id, dir));
        }
    }
    bail!("too many voices called {name}")
}

fn save(v: &Voice) -> Result<()> {
    let dir = v.dir.as_ref().context("voice has no folder")?;
    std::fs::write(dir.join("voice.json"), serde_json::to_vec_pretty(v)?)?;
    Ok(())
}

/// Save a clone from a clip (any format the engine's ffmpeg reads; `clip_ext` names it). Computes the embedding too when the engine can, so the voice
/// works in both modes. `exact_available` says whether the Exact pack is loaded.
pub async fn create_clone(
    z: &Zonos,
    name: &str,
    clip: &[u8],
    clip_ext: &str,
    transcript: Option<String>,
    consent: Consent,
    exact_available: bool,
) -> Result<Voice> {
    if consent.statement.trim().is_empty() {
        bail!("a clone needs the speaker's consent");
    }
    let (id, dir) = new_dir(name)?;
    let ext = clip_ext.trim_start_matches('.').to_lowercase();
    let ext = if ext.is_empty() { "wav".to_string() } else { ext };
    std::fs::write(dir.join(format!("clip.{ext}")), clip)?;
    let mut modes = Vec::new();
    match z.embed_clip(clip, &format!("{id}.{ext}")).await {
        Ok(npy) => {
            std::fs::write(dir.join("embedding.npy"), npy)?;
            modes.push(SpeakerMode::Embedding);
        }
        Err(e) if !exact_available => {
            std::fs::remove_dir_all(&dir).ok();
            return Err(e.context("couldn't read a voice from the clip"));
        }
        Err(_) => {}
    }
    if exact_available {
        modes.insert(0, SpeakerMode::Reference);
    }
    let v = Voice {
        id,
        name: name.trim().to_string(),
        kind: VoiceKind::Clone,
        mode: modes[0],
        language: None,
        accent: None,
        gender: None,
        description: None,
        created_at: Some(chrono::Utc::now()),
        speaker_id: None,
        modes,
        controls: BTreeMap::new(),
        transcript,
        consent: Some(consent),
        dir: Some(dir),
    };
    save(&v)?;
    Ok(v)
}

/// Save a designed voice: `base` reshaped by `controls` (Shapeable mode only).
pub async fn create_designed(
    z: &Zonos,
    name: &str,
    base: &Voice,
    controls: BTreeMap<String, f64>,
    description: Option<String>,
) -> Result<Voice> {
    let (id, dir) = new_dir(name)?;
    // Keep the base's embedding with the voice, so it still works if the base is deleted.
    let npy = if let Some(p) = base.embedding_path() {
        Some(std::fs::read(p)?)
    } else if let Some(sid) = &base.speaker_id {
        z.speaker_embedding(sid).await.ok()
    } else {
        None
    };
    if let Some(npy) = npy {
        std::fs::write(dir.join("embedding.npy"), npy)?;
    }
    let mut all = base.controls.clone();
    all.extend(controls);
    let v = Voice {
        id,
        name: name.trim().to_string(),
        kind: VoiceKind::Designed,
        mode: SpeakerMode::Embedding,
        language: base.language.clone(),
        accent: base.accent.clone(),
        gender: base.gender.clone(),
        description,
        created_at: Some(chrono::Utc::now()),
        speaker_id: base.speaker_id.clone(),
        modes: vec![SpeakerMode::Embedding],
        controls: all,
        transcript: None,
        consent: None,
        dir: Some(dir),
    };
    save(&v)?;
    Ok(v)
}

pub fn rename(id: &str, name: &str) -> Result<Voice> {
    let mut v = mine().into_iter().find(|v| v.id == id).context("no such voice")?;
    v.name = name.trim().to_string();
    save(&v)?;
    Ok(v)
}

/// Moves a user voice to `voices\.deleted\` (kept, not erased).
pub fn delete(id: &str) -> Result<()> {
    let v = mine().into_iter().find(|v| v.id == id).context("no such voice")?;
    let dir = v.dir.context("no folder")?;
    let bin = paths::voices_dir().join(".deleted");
    paths::ensure_dir(&bin)?;
    let dest = bin.join(format!("{}-{}", v.id, chrono::Utc::now().format("%Y%m%d%H%M%S")));
    std::fs::rename(dir, dest)?;
    Ok(())
}
