//! What both front ends do: speak a line, render a script, as saved takes.

use std::collections::BTreeMap;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use crate::api::{GenerateRequest, SpeakerMode, Zonos};
use crate::settings;
use crate::takes::{self, ScriptLine, Take};
use crate::voices;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SpeakOptions {
    /// Voice id or name; None = the default voice from settings, else the first voice.
    pub voice: Option<String>,
    /// Force Exact (`reference`) or Shapeable (`embedding`).
    pub mode: Option<SpeakerMode>,
    /// Extra controls on top of the voice's own: friendly ids (`age`, `happy`, `accent:british`)
    /// or raw directions (`identity.age`).
    pub controls: BTreeMap<String, f64>,
    pub seed: Option<u64>,
    pub language: Option<String>,
    /// `app` or `mcp`, recorded on the take.
    pub via: String,
}

async fn pick_voice(z: &Zonos, key: Option<&str>) -> Result<voices::Voice> {
    let key = key.map(str::to_string).or_else(|| settings::load().default_voice);
    match key {
        Some(k) => voices::find(Some(z), &k).await,
        None => {
            let all = voices::all(Some(z)).await;
            // Prefer an English default voice when nothing is chosen.
            all.iter()
                .find(|v| v.language.as_deref() == Some("English"))
                .or(all.first())
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("no voices yet"))
        }
    }
}

/// Speak `text` and save it as a take.
pub async fn speak(z: &Zonos, text: &str, opts: &SpeakOptions) -> Result<Take> {
    if text.trim().is_empty() {
        bail!("nothing to say");
    }
    let voice = pick_voice(z, opts.voice.as_deref()).await?;
    let s = settings::load();
    let cat = z.catalog().await?;
    let controls = cat.expand(&opts.controls)?;
    let mut mode = opts.mode;
    let all_controls = voice.controls.keys().chain(controls.keys());
    if mode.is_none() && voice.mode == SpeakerMode::Reference && all_controls.clone().any(|d| cat.needs_shapeable(d)) {
        if voice.modes.contains(&SpeakerMode::Embedding) || voice.modes.is_empty() {
            mode = Some(SpeakerMode::Embedding); // these controls only work on a Shapeable voice
        }
    }
    if mode == Some(SpeakerMode::Reference) {
        if let Some(d) = all_controls.clone().find(|d| cat.needs_shapeable(d)) {
            bail!("{d} needs Shapeable mode; Exact voices can't change gender, age, pitch, texture, accent or room");
        }
    }
    let mut req = GenerateRequest {
        text: text.to_string(),
        language: opts.language.clone(),
        controls,
        seed: opts.seed,
        decoder: s.decoder.clone(),
        laci: s.laci.then_some(true),
        ..Default::default()
    };
    voice.apply(&mut req, mode)?;
    let audio = z.generate(&req).await?;
    let take = Take {
        id: String::new(),
        text: text.to_string(),
        voice_id: voice.id.clone(),
        voice_name: voice.name.clone(),
        created_at: chrono::Utc::now(),
        seconds: 0.0,
        seed: audio.seed,
        mode: req.speaker_mode.map(|m| match m {
            SpeakerMode::Reference => "exact".into(),
            SpeakerMode::Embedding => "shapeable".into(),
        }),
        controls: req.controls.clone(),
        decoder: audio.decoder.clone(),
        via: opts.via.clone(),
        parts: Vec::new(),
        starred: false,
        path: Default::default(),
    };
    takes::save(take, &audio.wav)
}

/// Speak every line in order and join them into one take, with `gap_ms` between lines.
pub async fn render_script(z: &Zonos, lines: &[ScriptLine], gap_ms: u32, via: &str) -> Result<Take> {
    if lines.is_empty() {
        bail!("the script has no lines");
    }
    let s = settings::load();
    let mut wavs = Vec::new();
    let mut names = Vec::new();
    for line in lines {
        let voice = pick_voice(z, Some(&line.voice)).await?;
        let mut req = GenerateRequest {
            text: line.text.clone(),
            decoder: s.decoder.clone(),
            laci: s.laci.then_some(true),
            ..Default::default()
        };
        voice.apply(&mut req, None)?;
        wavs.push(z.generate(&req).await?.wav);
        if !names.contains(&voice.name) {
            names.push(voice.name.clone());
        }
    }
    let wav = takes::concat_wavs(&wavs, gap_ms)?;
    let text = lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join(" ");
    let take = Take {
        id: String::new(),
        text,
        voice_id: "script".into(),
        voice_name: names.join(" + "),
        created_at: chrono::Utc::now(),
        seconds: 0.0,
        seed: None,
        mode: None,
        controls: BTreeMap::new(),
        decoder: None,
        via: via.into(),
        parts: lines.to_vec(),
        starred: false,
        path: Default::default(),
    };
    takes::save(take, &wav)
}
