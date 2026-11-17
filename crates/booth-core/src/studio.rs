//! What both front ends do: speak a line, render a script, as saved takes.

use std::collections::BTreeMap;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use crate::api::{GenerateRequest, SpeakerMode, Zonos};
use crate::drift;
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
    /// None = the language chosen in the app (US English unless changed).
    pub language: Option<String>,
    /// `app` or `mcp`, recorded on the take.
    pub via: String,
    /// Sampling temperature; None = the engine's default (0.75). Kept to 0.4 to 0.75, the range tested.
    pub temperature: Option<f64>,
    /// Redo a take that sounds unlike the voice (see `drift`). Off when the caller pinned a seed.
    pub drift_check: bool,
}

/// The voice for `key` (id or name), or the one chosen in Booth, or Rupert.
pub async fn pick_voice(z: &Zonos, key: Option<&str>) -> Result<voices::Voice> {
    let key = key.map(str::to_string).or_else(|| settings::load().default_voice);
    match key {
        Some(k) => voices::find(Some(z), &k).await,
        None => {
            let all = voices::all(Some(z)).await;
            // Nothing chosen: Rupert, the voice the app opens with, then any English voice.
            all.iter()
                .find(|v| v.name == "Rupert")
                .or_else(|| all.iter().find(|v| v.language.as_deref() == Some("English")))
                .or(all.first())
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("no voices yet"))
        }
    }
}

/// Speak `text` and save it as a take.
pub async fn speak(z: &Zonos, text: &str, opts: &SpeakOptions) -> Result<Take> {
    let (take, wav) = speak_audio(z, text, opts).await?;
    takes::save(take, &wav)
}

/// Speak `text` without saving it: the take's record (no id or path yet) and its WAV.
pub async fn speak_audio(z: &Zonos, text: &str, opts: &SpeakOptions) -> Result<(Take, Vec<u8>)> {
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
        language: opts.language.clone().or_else(|| s.language.clone()),
        controls,
        seed: opts.seed,
        temperature: opts.temperature.map(|t| t.clamp(0.4, 0.75)),
        decoder: s.decoder.clone(),
        laci: s.laci.then_some(true),
        ..Default::default()
    };
    voice.apply(&mut req, mode)?;
    let audio = z.generate(&req).await?;
    // Only Exact takes drifted in testing, and only when nothing in the request changes who is
    // speaking on purpose (gender, age, texture, accent, room, or a designed voice's own shape).
    let check = opts.drift_check
        && req.speaker_mode == Some(SpeakerMode::Reference)
        && !req.controls.keys().any(|d| cat.needs_shapeable(d))
        && takes::wav_seconds(&audio.wav) >= drift::MIN_SECONDS;
    let target = if check { drift::voiceprint(z, &voice).await } else { None };
    let (audio, verdict) = match target {
        Some(t) => {
            let mut friendly = opts.controls.clone();
            for (k, v) in &voice.controls {
                friendly.entry(k.clone()).or_insert(*v);
            }
            let cut = drift::cutoff(&cat, &friendly);
            drift::keep_closest(z, &mut req, audio, &t, cut).await
        }
        None => (audio, None),
    };
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
        temperature: req.temperature,
        voice_match: verdict.map(|v| (v.voice_match * 1000.0).round() / 1000.0),
        redone: verdict.map_or(0, |v| v.redone),
        drifted: verdict.is_some_and(|v| v.drifted),
        starred: false,
        path: Default::default(),
    };
    let seconds = takes::wav_seconds(&audio.wav);
    Ok((Take { seconds, ..take }, audio.wav))
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
            language: s.language.clone(),
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
        temperature: None,
        voice_match: None,
        redone: 0,
        drifted: false,
        starred: false,
        path: Default::default(),
    };
    takes::save(take, &wav)
}
