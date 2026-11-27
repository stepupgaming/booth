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
    /// Sampling temperature, when the take didn't use the engine's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    /// Voiceprint match with the voice (cosine), when Booth checked the take.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice_match: Option<f64>,
    /// Times the take was made again because it sounded unlike the voice.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub redone: u32,
    /// Still unlike the voice after the retries; this was the closest try.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub drifted: bool,
    /// No try came out whole: the take may stop before the text is done (it was too short for
    /// its text, or ended chopped mid-word, every time Booth made it).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cut_off: bool,
    #[serde(default)]
    pub starred: bool,
    #[serde(skip_deserializing, default)]
    pub path: PathBuf,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
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

/// A free `<day>\<HHMMSS>-<slug>.wav` for a new take: `(folder, day, stem)`.
fn new_place(take: &Take) -> Result<(PathBuf, String, String)> {
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
    Ok((dir, day, stem))
}

fn record(mut take: Take, dir: &Path, day: &str, stem: &str, seconds: f64) -> Result<Take> {
    take.id = format!("{day}_{stem}");
    take.seconds = seconds;
    std::fs::write(dir.join(format!("{stem}.json")), serde_json::to_vec_pretty(&take)?)?;
    take.path = dir.join(format!("{stem}.wav"));
    Ok(take)
}

/// Save a take's audio and its record. Returns the record with `path` set.
pub fn save(take: Take, wav: &[u8]) -> Result<Take> {
    let (dir, day, stem) = new_place(&take)?;
    std::fs::write(dir.join(format!("{stem}.wav")), wav)?;
    record(take, &dir, &day, &stem, wav_seconds(wav))
}

/// Save a take whose audio is already a WAV file on disk (a long voiceover, never read into
/// memory). The file is moved when `keep` is false, else copied.
pub fn save_file(take: Take, wav: &Path, keep: bool) -> Result<Take> {
    let seconds = hound::WavReader::open(wav)
        .map(|r| r.duration() as f64 / r.spec().sample_rate as f64)
        .with_context(|| format!("reading {}", wav.display()))?;
    let (dir, day, stem) = new_place(&take)?;
    let dst = dir.join(format!("{stem}.wav"));
    if keep || std::fs::rename(wav, &dst).is_err() {
        std::fs::copy(wav, &dst)?;
        if !keep {
            let _ = std::fs::remove_file(wav);
        }
    }
    record(take, &dir, &day, &stem, seconds)
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
/// How loud a take's last 10 ms are, in dBFS (None when the WAV can't be read). A take whose voice
/// stops cleanly ends far below [`CUT_DB`]; one that stops mid-sound ends above it.
pub fn ending_db(wav: &[u8]) -> Option<f64> {
    let (spec, samples) = read_i16(wav)?;
    let n = (spec.sample_rate as usize / 100 * spec.channels.max(1) as usize).min(samples.len());
    if n == 0 {
        return None;
    }
    let tail = &samples[samples.len() - n..];
    let ms = tail.iter().map(|&s| (s as f64 / 32768.0).powi(2)).sum::<f64>() / n as f64;
    Some(10.0 * ms.max(1e-12).log10())
}

/// A take whose last 10 ms are louder than this stopped before its last sound finished.
pub const CUT_DB: f64 = -50.0;
/// Louder than this at the very end and the take was chopped mid-word.
pub const HARD_CUT_DB: f64 = -30.0;

/// Letters and digits per second of speech above which a take is too short for its text: the
/// model stopped early and left words out, but ended in clean silence, so [`ending_db`] can't
/// tell. Whole English takes ran 10 to 14 a second, the fastest pace included; one that dropped
/// its second sentence ran at 43 (2026-11-27 test on ZONOS3 96d9e0e, 70 takes).
pub const MAX_LETTERS_PER_SECOND: f64 = 25.0;
/// Texts with fewer letters and digits than this aren't judged by length: a word or two can be
/// drawn out or said fast, and the engine pads very short texts on its own.
const MIN_LETTERS_JUDGED: usize = 20;
/// Louder than this, a 10 ms window counts as speech for [`speech_seconds`].
const SPEECH_DB: f64 = -45.0;

/// How a finished take ended, as far as Booth can tell from its audio.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ending {
    /// How loud the last 10 ms are ([`ending_db`]).
    pub db: f64,
    /// Too short for its text ([`too_short`]).
    pub short: bool,
}

impl Ending {
    /// The voice finished: a quiet ending and a length that fits the text.
    pub fn clean(&self) -> bool {
        self.db <= CUT_DB && !self.short
    }

    /// Of two tries, the one to keep: one with all its words first, then the quieter ending.
    pub fn better_than(&self, other: &Ending) -> bool {
        (self.short, self.db) < (other.short, other.db)
    }
}

/// How `wav` ended for `text` (None when the WAV can't be read).
pub fn ending(text: &str, wav: &[u8]) -> Option<Ending> {
    Some(Ending { db: ending_db(wav)?, short: too_short(text, wav) })
}

/// Seconds from the first to the last 10 ms that sound like speech (0 when none do).
pub fn speech_seconds(wav: &[u8]) -> f64 {
    let Some((spec, samples)) = read_i16(wav) else { return 0.0 };
    let win = (spec.sample_rate as usize / 100 * spec.channels.max(1) as usize).max(1);
    let loud = |c: &[i16]| {
        let ms = c.iter().map(|&s| (s as f64 / 32768.0).powi(2)).sum::<f64>() / c.len() as f64;
        10.0 * ms.max(1e-12).log10() > SPEECH_DB
    };
    let windows: Vec<bool> = samples.chunks(win).map(loud).collect();
    match (windows.iter().position(|&l| l), windows.iter().rposition(|&l| l)) {
        (Some(a), Some(b)) => (b + 1 - a) as f64 / 100.0,
        _ => 0.0,
    }
}

/// True when the take is far too short to have said all of `text`. Counting letters and digits
/// leaves out spaces and punctuation; numbers, initials and symbols take longer to say than they
/// have letters, so they never make a take look short. Languages written with one sign per
/// syllable (Chinese, Japanese) are never flagged at this rate, which is the safe side.
pub fn too_short(text: &str, wav: &[u8]) -> bool {
    let letters = text.chars().filter(|c| c.is_alphanumeric()).count();
    if letters < MIN_LETTERS_JUDGED {
        return false;
    }
    let secs = speech_seconds(wav);
    secs > 0.0 && letters as f64 / secs > MAX_LETTERS_PER_SECOND
}

/// Fade the last `ms` of a WAV to silence, so a take that ends mid-sound doesn't click.
pub fn fade_out(wav: &[u8], ms: u32) -> Result<Vec<u8>> {
    let (spec, mut samples) = read_i16(wav).context("reading the take's audio")?;
    let ch = spec.channels.max(1) as usize;
    let frames = samples.len() / ch;
    let n = (spec.sample_rate as usize * ms as usize / 1000).min(frames);
    for i in 0..n {
        let g = (n - i) as f64 / n as f64; // 1 at the start of the fade, ~0 on the last frame
        for c in 0..ch {
            let k = (frames - n + i) * ch + c;
            samples[k] = (samples[k] as f64 * g * g) as i16;
        }
    }
    let spec = hound::WavSpec { bits_per_sample: 16, sample_format: hound::SampleFormat::Int, ..spec };
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

fn read_i16(wav: &[u8]) -> Option<(hound::WavSpec, Vec<i16>)> {
    let mut r = hound::WavReader::new(Cursor::new(wav)).ok()?;
    let s = r.spec();
    let v: Vec<i16> = match s.sample_format {
        hound::SampleFormat::Int if s.bits_per_sample <= 16 => r.samples::<i16>().filter_map(|x| x.ok()).collect(),
        hound::SampleFormat::Int => {
            let shift = s.bits_per_sample - 16;
            r.samples::<i32>().filter_map(|x| x.ok()).map(|x| (x >> shift) as i16).collect()
        }
        hound::SampleFormat::Float => {
            r.samples::<f32>().filter_map(|x| x.ok()).map(|x| (x.clamp(-1.0, 1.0) * 32767.0) as i16).collect()
        }
    };
    Some((s, v))
}

pub fn concat_wavs(parts: &[Vec<u8>], gap_ms: u32) -> Result<Vec<u8>> {
    let gaps = vec![gap_ms; parts.len()];
    Ok(concat_timed(parts, &gaps)?.0)
}

/// Join WAV files with a pause before each one (`gaps_ms[i]` before part `i`; the first is
/// ignored), and say where each part landed: `(start, seconds)`, for captions.
pub fn concat_timed(parts: &[Vec<u8>], gaps_ms: &[u32]) -> Result<(Vec<u8>, Vec<(f64, f64)>)> {
    let mut spec: Option<hound::WavSpec> = None;
    let mut samples: Vec<i16> = Vec::new();
    let mut spans = Vec::with_capacity(parts.len());
    for (i, wav) in parts.iter().enumerate() {
        let mut r = hound::WavReader::new(Cursor::new(wav)).context("reading a line's audio")?;
        let s = r.spec();
        let spec = *spec.get_or_insert(s);
        if s.sample_rate != spec.sample_rate || s.channels != spec.channels {
            anyhow::bail!("line {} has a different audio format", i + 1);
        }
        let frame = spec.channels.max(1) as usize;
        if i > 0 {
            let gap_ms = gaps_ms.get(i).copied().unwrap_or(0) as u64;
            let gap = (spec.sample_rate as u64 * gap_ms / 1000) as usize * frame;
            samples.extend(std::iter::repeat_n(0i16, gap));
        }
        let before = samples.len();
        match s.sample_format {
            hound::SampleFormat::Int if s.bits_per_sample <= 16 => {
                samples.extend(r.samples::<i16>().filter_map(|x| x.ok()))
            }
            hound::SampleFormat::Int => {
                let shift = s.bits_per_sample - 16;
                samples.extend(r.samples::<i32>().filter_map(|x| x.ok()).map(|x| (x >> shift) as i16))
            }
            hound::SampleFormat::Float => samples.extend(
                r.samples::<f32>().filter_map(|x| x.ok()).map(|x| (x.clamp(-1.0, 1.0) * 32767.0) as i16),
            ),
        }
        let rate = spec.sample_rate as f64 * frame as f64;
        spans.push((before as f64 / rate, (samples.len() - before) as f64 / rate));
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
    Ok((out.into_inner(), spans))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav(samples: &[i16]) -> Vec<u8> {
        let spec = hound::WavSpec { channels: 1, sample_rate: 8000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
        let mut out = Cursor::new(Vec::new());
        {
            let mut w = hound::WavWriter::new(&mut out, spec).unwrap();
            for &s in samples {
                w.write_sample(s).unwrap();
            }
            w.finalize().unwrap();
        }
        out.into_inner()
    }

    #[test]
    fn a_take_that_stops_mid_sound_is_cut_and_the_fade_clears_it() {
        let loud: Vec<i16> = (0..8000).map(|i| ((i as f64 * 0.3).sin() * 8000.0) as i16).collect();
        let mut clean = loud.clone();
        clean.extend(std::iter::repeat_n(0i16, 800));
        assert!(ending_db(&wav(&loud)).unwrap() > CUT_DB);
        assert!(ending_db(&wav(&clean)).unwrap() < CUT_DB);
        let faded = fade_out(&wav(&loud), 30).unwrap();
        assert!(ending_db(&faded).unwrap() < ending_db(&wav(&loud)).unwrap() - 15.0);
        assert_eq!(wav_seconds(&faded), wav_seconds(&wav(&loud)));
    }

    /// `secs` of a tone, with 200 ms of silence on each side (8 kHz).
    fn speech(secs: f64) -> Vec<u8> {
        let mut s = vec![0i16; 1600];
        s.extend((0..(secs * 8000.0) as usize).map(|i| ((i as f64 * 0.3).sin() * 8000.0) as i16));
        s.extend(std::iter::repeat_n(0i16, 1600));
        wav(&s)
    }

    const TWO_SENTENCES: &str = "Then a leaked intelligence file hits the press. U.S. spies were watching his secret \
        affair with Alice Huang, an executive at a Singapore datacenter firm.";

    #[test]
    fn speech_seconds_leave_out_the_silence_around_the_voice() {
        assert!((speech_seconds(&speech(2.0)) - 2.0).abs() < 0.02);
        assert_eq!(speech_seconds(&wav(&[0; 8000])), 0.0);
    }

    #[test]
    fn a_take_that_dropped_its_second_sentence_is_too_short() {
        // The 2026-11-27 repro: the whole line takes about 11 s; the take that stopped after
        // "hits the press." was 3.2 s with silence after it.
        assert!(too_short(TWO_SENTENCES, &speech(2.9)));
        assert!(!too_short(TWO_SENTENCES, &speech(11.0)));
        // Faster than the fastest pace measured (about 14 a second) is still fine.
        assert!(!too_short(TWO_SENTENCES, &speech(7.8)));
        let e = ending(TWO_SENTENCES, &speech(2.9)).unwrap();
        assert!(e.db < CUT_DB && e.short && !e.clean());
    }

    #[test]
    fn short_texts_and_numbers_are_never_too_short() {
        assert!(!too_short("Hi there.", &speech(0.2)));
        // 21 digits said as words take far longer than 21 letters.
        assert!(!too_short("Call 555 0199 or 800 555 0123 today", &speech(3.5)));
    }

    #[test]
    fn the_kept_try_has_all_its_words_before_a_quiet_ending() {
        let whole_but_loud = Ending { db: -35.0, short: false };
        let short_but_quiet = Ending { db: -110.0, short: true };
        let clean = Ending { db: -90.0, short: false };
        assert!(whole_but_loud.better_than(&short_but_quiet));
        assert!(clean.better_than(&whole_but_loud));
        assert!(!short_but_quiet.better_than(&clean));
    }
}
