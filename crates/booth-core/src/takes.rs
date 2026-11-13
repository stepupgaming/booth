//! Takes: every line Booth speaks is saved as `takes\<yyyy-mm-dd>\<HHMMSS>-<slug>.wav` with a
//! `.json` beside it (text, voice, settings, seed), so the app and Claude see the same history.

use std::collections::BTreeMap;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::paths;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Take {
    pub id: String,
    pub text: String,
    pub voice_id: String,
    pub voice_name: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub seconds: f64,
    #[serde(default)]
    pub seed: Option<u64>,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub controls: BTreeMap<String, f64>,
    #[serde(default)]
    pub decoder: Option<String>,
    /// `app` or `mcp`.
    #[serde(default)]
    pub via: String,
    /// For a rendered script: the lines it was made of.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<ScriptLine>,
    #[serde(default)]
    pub starred: bool,
    #[serde(skip_deserializing, default)]
    pub path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ScriptLine {
    /// Voice id or name.
    pub voice: String,
    /// What the voice says.
    pub text: String,
}

fn slug(text: &str) -> String {
    let words: Vec<String> = text
        .split_whitespace()
        .take(5)
        .map(|w| w.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase())
        .filter(|w| !w.is_empty())
        .collect();
    let s = words.join("-");
    if s.is_empty() { "take".into() } else { s.chars().take(40).collect() }
}

/// Length of a WAV file in seconds.
pub fn wav_seconds(wav: &[u8]) -> f64 {
    hound::WavReader::new(Cursor::new(wav))
        .map(|r| r.duration() as f64 / r.spec().sample_rate as f64)
        .unwrap_or(0.0)
}

/// Save a take's audio and its record. Returns the record with `path` set.
pub fn save(mut take: Take, wav: &[u8]) -> Result<Take> {
    let day = take.created_at.format("%Y-%m-%d").to_string();
    let dir = paths::takes_dir().join(&day);
    paths::ensure_dir(&dir)?;
    let base = format!("{}-{}", take.created_at.format("%H%M%S"), slug(&take.text));
    let mut stem = base.clone();
    let mut n = 1;
    while dir.join(format!("{stem}.wav")).exists() {
        n += 1;
        stem = format!("{base}-{n}");
    }
    let path = dir.join(format!("{stem}.wav"));
    std::fs::write(&path, wav)?;
    take.id = format!("{day}_{stem}");
    take.seconds = wav_seconds(wav);
    std::fs::write(dir.join(format!("{stem}.json")), serde_json::to_vec_pretty(&take)?)?;
    take.path = path;
    Ok(take)
}

/// The newest takes first, at most `limit`.
pub fn list(limit: usize) -> Vec<Take> {
    let root = paths::takes_dir();
    let Ok(rd) = std::fs::read_dir(&root) else { return Vec::new() };
    let mut days: Vec<PathBuf> = rd.filter_map(|e| e.ok()).map(|e| e.path())
        .filter(|p| p.is_dir() && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')))
        .collect();
    days.sort();
    days.reverse();
    let mut out = Vec::new();
    for day in days {
        let mut files: Vec<PathBuf> = std::fs::read_dir(&day)
            .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.path()).collect())
            .unwrap_or_default();
        files.retain(|p| p.extension().is_some_and(|e| e == "json"));
        files.sort();
        files.reverse();
        for f in files {
            if let Some(t) = read(&f) {
                out.push(t);
                if out.len() >= limit {
                    return out;
                }
            }
        }
    }
    out
}

fn read(json: &Path) -> Option<Take> {
    let mut t: Take = serde_json::from_str(&std::fs::read_to_string(json).ok()?).ok()?;
    t.path = json.with_extension("wav");
    t.path.is_file().then_some(t)
}

pub fn find(id: &str) -> Option<Take> {
    list(usize::MAX).into_iter().find(|t| t.id == id)
}

pub fn set_starred(id: &str, starred: bool) -> Result<Take> {
    let mut t = find(id).context("no such take")?;
    t.starred = starred;
    std::fs::write(t.path.with_extension("json"), serde_json::to_vec_pretty(&t)?)?;
    Ok(t)
}

/// Moves a take to `takes\.deleted\` (kept, not erased).
pub fn delete(id: &str) -> Result<()> {
    let t = find(id).context("no such take")?;
    let bin = paths::takes_dir().join(".deleted");
    paths::ensure_dir(&bin)?;
    for ext in ["wav", "json"] {
        let src = t.path.with_extension(ext);
        if src.exists() {
            let day = src.parent().and_then(|d| d.file_name()).map(|d| d.to_string_lossy().to_string()).unwrap_or_default();
            std::fs::rename(&src, bin.join(format!("{day}_{}", src.file_name().unwrap().to_string_lossy())))?;
        }
    }
    Ok(())
}

/// Join WAV files (same format) with `gap_ms` of silence between them.
pub fn concat_wavs(parts: &[Vec<u8>], gap_ms: u32) -> Result<Vec<u8>> {
    let mut spec = None;
    let mut samples: Vec<i16> = Vec::new();
    for (i, wav) in parts.iter().enumerate() {
        let mut r = hound::WavReader::new(Cursor::new(wav)).context("reading a line's audio")?;
        let s = r.spec();
        let spec = *spec.get_or_insert(s);
        if i > 0 {
            let gap = (spec.sample_rate as u64 * gap_ms as u64 / 1000) as usize * spec.channels as usize;
            samples.extend(std::iter::repeat_n(0i16, gap));
        }
        match s.sample_format {
            hound::SampleFormat::Int => samples.extend(r.samples::<i16>().filter_map(|x| x.ok())),
            hound::SampleFormat::Float => samples.extend(
                r.samples::<f32>().filter_map(|x| x.ok()).map(|x| (x.clamp(-1.0, 1.0) * 32767.0) as i16),
            ),
        }
    }
    let mut spec = spec.context("no audio")?;
    spec.bits_per_sample = 16;
    spec.sample_format = hound::SampleFormat::Int;
    let mut out = Cursor::new(Vec::new());
    {
        let mut w = hound::WavWriter::new(&mut out, spec)?;
        for s in samples {
            w.write_sample(s)?;
        }
        w.finalize()?;
    }
    Ok(out.into_inner())
}
