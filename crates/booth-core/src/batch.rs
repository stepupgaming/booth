//! A script being made: every line spoken in order and kept under `scripts\<id>\`, then exported
//! as one voiceover (WAV or MP3), captions (.srt, and .vtt if wanted) and, if wanted, each line
//! as its own file.
//!
//! The loop runs here rather than in the page, so it keeps going when the window is hidden or a
//! phone's screen is off, and booth-mcp makes scripts the same way the app does.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::api::Zonos;
use crate::audio::{self, Format, Joiner, Loudness};
use crate::captions::{self, Style};
use crate::engine;
use crate::paths;
use crate::script::{self, Parsed};
use crate::studio::{self, SpeakOptions};
use crate::takes::{self, ScriptLine, Take};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LineState {
    Waiting,
    Making,
    Done,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobLine {
    #[serde(default)]
    pub speaker: Option<String>,
    /// Voice id.
    pub voice: String,
    #[serde(default)]
    pub voice_name: String,
    pub text: String,
    /// Starts a new paragraph (a longer pause before it).
    #[serde(default)]
    pub para: bool,
    /// Extra silence before the line from pause tags in the script, in ms.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub pause_ms: u32,
    pub state: LineState,
    #[serde(default)]
    pub seconds: f64,
    #[serde(default)]
    pub seed: Option<u64>,
    /// Times this line was made; the audio file's name changes with it, so players reload.
    #[serde(default)]
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice_match: Option<f64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub drifted: bool,
    /// The line's take may stop before its text is done (see `Take::cut_off`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cut_off: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The line's audio, once made.
    #[serde(default)]
    pub path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub title: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub lines: Vec<JobLine>,
    /// Headings and notes left out of the audio.
    #[serde(default)]
    pub skipped: Vec<String>,
    /// The script as pasted. Kept once in `source.txt` next to script.json rather than in it, so
    /// saving a long script's progress after every line doesn't rewrite the whole text each time
    /// (older script.json files still have it, and it moves out on their next save).
    #[serde(default, skip_serializing)]
    pub source: String,
    /// Speaker label -> voice id (`""` for lines without a label).
    #[serde(default)]
    pub voices: BTreeMap<String, String>,
    /// `app` or `mcp`.
    #[serde(default)]
    pub via: String,
    /// True while a run is making its lines (in this or another process).
    #[serde(default)]
    pub running: bool,
    /// The last export, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exported: Option<Exported>,
}

impl Job {
    pub fn done(&self) -> usize {
        self.lines.iter().filter(|l| l.state == LineState::Done).count()
    }
    pub fn folder(&self) -> PathBuf {
        scripts_dir().join(&self.id)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportOptions {
    /// Pause between lines, in ms.
    #[serde(default = "default_gap")]
    pub gap_ms: u32,
    /// Pause before a new paragraph, in ms.
    #[serde(default = "default_para_gap")]
    pub para_gap_ms: u32,
    #[serde(default = "yes")]
    pub voiceover: bool,
    #[serde(default = "yes")]
    pub captions: bool,
    /// Each line as its own file too.
    #[serde(default)]
    pub lines: bool,
    /// WAV or MP3, for the voiceover and the line files.
    #[serde(default)]
    pub format: Format,
    /// Every line to the same loudness, the whole at -14 LUFS with true peaks under -1 dBTP.
    #[serde(default)]
    pub even_loudness: bool,
    /// Sentence captions, or short ones (2 to 4 words) for Shorts, TikTok and Reels.
    #[serde(default)]
    pub caption_style: Style,
    /// WebVTT (.vtt) captions next to the .srt.
    #[serde(default)]
    pub vtt: bool,
}

fn default_gap() -> u32 {
    250
}
fn default_para_gap() -> u32 {
    600
}
fn yes() -> bool {
    true
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            gap_ms: default_gap(),
            para_gap_ms: default_para_gap(),
            voiceover: true,
            captions: true,
            lines: false,
            format: Format::Wav,
            even_loudness: false,
            caption_style: Style::Sentences,
            vtt: false,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Exported {
    pub folder: PathBuf,
    #[serde(default)]
    pub voiceover: Option<PathBuf>,
    #[serde(default)]
    pub captions: Option<PathBuf>,
    /// The captions as WebVTT, when asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vtt: Option<PathBuf>,
    #[serde(default)]
    pub lines: Vec<PathBuf>,
    /// The voiceover's take in the Library.
    #[serde(default)]
    pub take_id: Option<String>,
    #[serde(default)]
    pub seconds: f64,
    /// Measured loudness of the voiceover, when it was evened out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loudness: Option<Loudness>,
}

pub fn scripts_dir() -> PathBuf {
    paths::home().join("scripts")
}

fn job_file(id: &str) -> PathBuf {
    scripts_dir().join(id).join("script.json")
}

/// Every read-change-write of a script.json goes through this lock, so a redo asked for while
/// a run is going isn't lost when the run saves its next line.
static FILES: Mutex<()> = Mutex::new(());

pub fn load(id: &str) -> Result<Job> {
    let _g = FILES.lock().unwrap_or_else(|e| e.into_inner());
    read(id)
}

fn read(id: &str) -> Result<Job> {
    if id.is_empty() || id.contains(['/', '\\']) || id.contains("..") {
        bail!("no such script");
    }
    let bytes = std::fs::read(job_file(id)).with_context(|| format!("no script {id}"))?;
    Ok(serde_json::from_slice(&bytes)?)
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

fn write(job: &Job) -> Result<()> {
    let dir = job.folder();
    paths::ensure_dir(&dir)?;
    let source = dir.join("source.txt");
    if !job.source.is_empty() && !source.exists() {
        std::fs::write(&source, &job.source)?;
    }
    let tmp = dir.join("script.json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(job)?)?;
    std::fs::rename(&tmp, dir.join("script.json"))?;
    Ok(())
}

/// Change a script on disk under the lock and return the new version.
pub fn update(id: &str, f: impl FnOnce(&mut Job) -> Result<()>) -> Result<Job> {
    let _g = FILES.lock().unwrap_or_else(|e| e.into_inner());
    let mut job = read(id)?;
    f(&mut job)?;
    write(&job)?;
    Ok(job)
}

/// Scripts, newest first. A run that ended with the app (closed mid-script) shows as stopped.
pub fn list() -> Vec<Job> {
    let mut out: Vec<Job> = std::fs::read_dir(scripts_dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| load(&e.file_name().to_string_lossy()).ok())
        .map(settle)
        .collect();
    out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    out
}

/// A script's state as seen from outside a run: if no run in this process is making it and its
/// file says one is, that run ended with its process.
fn settle(mut job: Job) -> Job {
    let mine = RUNNING.lock().unwrap_or_else(|e| e.into_inner()).as_deref() == Some(job.id.as_str());
    if job.running && !mine && !held_elsewhere(&job) {
        job.running = false;
        for l in &mut job.lines {
            if l.state == LineState::Making {
                l.state = LineState::Waiting;
            }
        }
    }
    job
}

/// Another process (booth-mcp, or the app while this is booth-mcp) is making this script.
fn held_elsewhere(job: &Job) -> bool {
    let lock = job.folder().join("running.pid");
    let pid = std::fs::read_to_string(lock).ok().and_then(|s| s.trim().parse::<u32>().ok());
    pid.is_some_and(|p| p != std::process::id() && crate::diag::pid_alive(p))
}

pub fn get(id: &str) -> Result<Job> {
    load(id).map(settle)
}

fn title_of(parsed: &Parsed) -> String {
    let heading = parsed.skipped.iter().find(|s| s.starts_with('#')).map(|s| s.trim_start_matches('#').trim().to_string());
    heading.filter(|h| !h.is_empty()).unwrap_or_else(|| {
        let words: Vec<&str> = parsed.lines.first().map(|l| l.text.split_whitespace().take(7).collect()).unwrap_or_default();
        words.join(" ").trim_end_matches([',', '.', '!', '?', ';', ':']).to_string()
    })
}

fn slug(text: &str) -> String {
    let s: String = text
        .split_whitespace()
        .take(6)
        .map(|w| w.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if s.is_empty() { "script".into() } else { s.chars().take(48).collect() }
}

/// Make a new script from pasted text. `voices` maps each speaker label to a voice id or name
/// (`""` for lines without a label); a speaker left out gets `fallback`.
pub async fn create(z: &Zonos, source: &str, voices: &BTreeMap<String, String>, fallback: &str, via: &str) -> Result<Job> {
    let parsed = script::parse(source, &[]);
    if parsed.lines.is_empty() {
        bail!("there's nothing to say in this script");
    }
    let now = chrono::Utc::now();
    let title = title_of(&parsed);
    let base = format!("{}-{}", now.format("%Y-%m-%d_%H%M%S"), slug(&title));
    let mut id = base.clone();
    let mut n = 1;
    while scripts_dir().join(&id).exists() {
        n += 1;
        id = format!("{base}-{n}");
    }
    let voice_for = |speaker: &Option<String>| -> String {
        let key = speaker.clone().unwrap_or_default();
        voices
            .get(&key)
            .or_else(|| voices.iter().find(|(k, _)| k.eq_ignore_ascii_case(&key)).map(|(_, v)| v))
            .cloned()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| fallback.to_string())
    };
    let mut found: BTreeMap<String, crate::voices::Voice> = BTreeMap::new();
    for l in &parsed.lines {
        let key = voice_for(&l.speaker);
        if key.is_empty() {
            let who = l.speaker.as_deref().unwrap_or("lines without a name");
            bail!("pick a voice for {who}");
        }
        if !found.contains_key(&key) {
            found.insert(key.clone(), crate::voices::find(Some(z), &key).await?);
        }
    }
    let lines = parsed
        .lines
        .iter()
        .map(|l| JobLine {
            speaker: l.speaker.clone(),
            voice: found[&voice_for(&l.speaker)].id.clone(),
            voice_name: found[&voice_for(&l.speaker)].name.clone(),
            text: l.text.clone(),
            para: l.para,
            pause_ms: l.pause_ms,
            state: LineState::Waiting,
            seconds: 0.0,
            seed: None,
            version: 0,
            voice_match: None,
            drifted: false,
            cut_off: false,
            error: None,
            path: None,
        })
        .collect::<Vec<_>>();
    let job = Job {
        id,
        title,
        created_at: now,
        lines,
        skipped: parsed.skipped,
        source: source.to_string(),
        voices: voices.clone(),
        via: via.into(),
        running: false,
        exported: None,
    };
    let _g = FILES.lock().unwrap_or_else(|e| e.into_inner());
    write(&job)?;
    Ok(job)
}

/// Make a line again (with new text, if given). The next run, or the one going now, picks it up.
pub fn redo(id: &str, index: usize, text: Option<String>) -> Result<Job> {
    update(id, |j| {
        let line = j.lines.get_mut(index).context("no such line")?;
        // Typed words are spoken as they are, except pause tags and notes (never read out).
        if let Some(t) = text.map(|t| script::spoken_only(&t)).filter(|t| !t.is_empty()) {
            line.text = t;
        }
        line.state = LineState::Waiting;
        line.error = None;
        j.exported = None;
        Ok(())
    })
}

/// The script being made in this process, and a request to stop it after the current line.
static RUNNING: Mutex<Option<String>> = Mutex::new(None);
static STOP: AtomicBool = AtomicBool::new(false);

/// Stop the running script after the line it's on. Lines already made are kept.
pub fn stop() {
    STOP.store(true, Ordering::SeqCst);
}

/// Which script this process is making, if any.
pub fn running() -> Option<String> {
    RUNNING.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

struct RunGuard {
    lock: PathBuf,
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        *RUNNING.lock().unwrap_or_else(|e| e.into_inner()) = None;
        let _ = std::fs::remove_file(&self.lock);
    }
}

fn seed() -> u64 {
    // Same range as the app's Studio (a positive 31-bit number).
    let n = u64::from_le_bytes(uuid::Uuid::new_v4().as_bytes()[..8].try_into().unwrap_or([7; 8]));
    1 + n % 2_147_483_000
}

/// Make every line that isn't made yet, in order, calling `on_change` after each change.
/// Returns when every line is made or failed, or after `stop()`.
pub async fn run(z: &Zonos, id: &str, on_change: impl Fn(&Job)) -> Result<Job> {
    // Claim the slot, then let go of RUNNING before anything else: `get` locks it again, and a
    // std Mutex locked twice on one thread hangs it (a Redo while a run was going froze every
    // later script call until Booth restarted).
    let busy = {
        let mut r = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
        match r.clone() {
            Some(other) => Some(other),
            None => {
                *r = Some(id.to_string());
                None
            }
        }
    };
    if let Some(other) = busy {
        if other == id {
            return get(id); // already going; it picks up new work
        }
        let title = load(&other).map(|j| j.title).unwrap_or_default();
        bail!("Booth is still making \"{title}\". Stop it first or wait for it to finish.");
    }
    let job = load(id)?;
    let lock = job.folder().join("running.pid");
    let _guard = RunGuard { lock: lock.clone() };
    if held_elsewhere(&job) {
        bail!("this script is already being made by another Booth (Claude or the app)");
    }
    std::fs::write(&lock, std::process::id().to_string())?;
    STOP.store(false, Ordering::SeqCst);
    // Keep Android's foreground service (and the engine) up for the whole script, not per line.
    let _work = engine::Working::new();

    let mut failed_in_a_row = 0;
    loop {
        let job = update(id, |j| {
            j.running = true;
            for l in &mut j.lines {
                if l.state == LineState::Making {
                    l.state = LineState::Waiting;
                }
            }
            if let Some(l) = j.lines.iter_mut().find(|l| l.state == LineState::Waiting) {
                l.state = LineState::Making;
            }
            Ok(())
        })?;
        let Some(i) = job.lines.iter().position(|l| l.state == LineState::Making) else { break };
        if STOP.load(Ordering::SeqCst) {
            update(id, |j| {
                j.lines[i].state = LineState::Waiting;
                Ok(())
            })?;
            break;
        }
        on_change(&job);
        let line = job.lines[i].clone();
        let opts = SpeakOptions {
            voice: Some(line.voice.clone()),
            seed: Some(seed()),
            via: job.via.clone(),
            drift_check: true,
            ..Default::default()
        };
        let made = studio::speak_audio(z, &line.text, &opts).await;
        let version = line.version + 1;
        let made = made.and_then(|(take, wav)| {
            let path = job.folder().join(format!("line-{:03}-v{version}.wav", i + 1));
            std::fs::write(&path, &wav)?;
            if let Some(old) = &line.path {
                if *old != path {
                    let _ = std::fs::remove_file(old);
                }
            }
            Ok((take, path))
        });
        let ok = made.is_ok();
        let job = update(id, |j| {
            let l = &mut j.lines[i];
            // A redo asked for while this line was being made: make it again.
            let redone_meanwhile = l.state == LineState::Waiting;
            match &made {
                Ok((take, path)) => {
                    l.seconds = take.seconds;
                    l.seed = take.seed;
                    l.voice_match = take.voice_match;
                    l.drifted = take.drifted;
                    l.cut_off = take.cut_off;
                    l.path = Some(path.clone());
                    l.version = version;
                    l.error = None;
                    if !redone_meanwhile {
                        l.state = LineState::Done;
                    }
                }
                Err(e) => {
                    l.error = Some(format!("{e:#}"));
                    if !redone_meanwhile {
                        l.state = LineState::Failed;
                    }
                }
            }
            Ok(())
        })?;
        on_change(&job);
        failed_in_a_row = if ok { 0 } else { failed_in_a_row + 1 };
        if failed_in_a_row >= 2 {
            break; // the engine is likely down; don't fail every line in turn
        }
    }
    let job = update(id, |j| {
        j.running = false;
        Ok(())
    })?;
    on_change(&job);
    Ok(job)
}

/// Where each made line sits in the joined audio, as the pause before it (pause tags add to it).
fn gaps(job: &Job, o: &ExportOptions) -> Vec<u32> {
    job.lines.iter().map(|l| if l.para { o.para_gap_ms.max(o.gap_ms) } else { o.gap_ms } + l.pause_ms).collect()
}

/// Captions for a script whose lines are all made, from the lines' lengths (no audio read).
pub fn captions_preview(job: &Job, o: &ExportOptions) -> String {
    let g = gaps(job, o);
    let mut t = 0.0;
    let mut spans = Vec::new();
    for (i, l) in job.lines.iter().enumerate() {
        if i > 0 {
            t += g[i] as f64 / 1000.0;
        }
        spans.push((l.text.clone(), t, l.seconds));
        t += l.seconds;
    }
    captions::to_srt(&captions::cues(&spans, o.caption_style))
}

fn file_stem(job: &Job) -> String {
    let s: String = job
        .title
        .chars()
        .map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let s: String = s.chars().take(60).collect();
    if s.is_empty() { "Voiceover".into() } else { s }
}

/// Write the voiceover, captions and line files into the script's `export` folder, and add the
/// voiceover to the Library. Every line must be made.
///
/// The lines are read one at a time and the voiceover is written as it goes, so memory stays
/// the same however long the script is. Evening out loudness reads the lines twice more first.
pub fn export(id: &str, o: &ExportOptions) -> Result<Exported> {
    let job = get(id)?;
    if let Some((n, _)) = job.lines.iter().enumerate().find(|(_, l)| l.state != LineState::Done) {
        bail!("line {} isn't made yet", n + 1);
    }
    let sources: Vec<PathBuf> =
        job.lines.iter().map(|l| l.path.clone().context("a line has no audio")).collect::<Result<_>>()?;
    let gaps = gaps(&job, o);
    let gains = if o.even_loudness { audio::plan_gains(&sources, &gaps)? } else { vec![0.0; sources.len()] };
    let folder = job.folder().join("export");
    let _ = std::fs::remove_dir_all(&folder);
    paths::ensure_dir(&folder)?;
    let stem = file_stem(&job);
    let mut out = Exported { folder: folder.clone(), ..Default::default() };

    // The voiceover, plus a WAV for the Library when the voiceover is in another format.
    let mut outs = Vec::new();
    let mut library = None;
    if o.voiceover {
        let p = folder.join(format!("{stem}.{}", o.format.ext()));
        outs.push((p.clone(), o.format));
        out.voiceover = Some(p.clone());
        library = Some(if o.format == Format::Wav {
            p
        } else {
            let wav = folder.join(".library.wav");
            outs.push((wav.clone(), Format::Wav));
            wav
        });
    }
    let line_dir = folder.join("lines");
    if o.lines {
        paths::ensure_dir(&line_dir)?;
    }
    let mut join = Joiner::new(outs, o.even_loudness && o.voiceover);
    let mut spans = Vec::with_capacity(job.lines.len());
    for (i, (l, src)) in job.lines.iter().zip(&sources).enumerate() {
        let pcm = audio::read_wav(src)?;
        if i > 0 {
            join.pause(gaps[i])?;
        }
        spans.push((l.text.clone(), join.seconds(), pcm.seconds()));
        join.line(&pcm, gains[i]).with_context(|| format!("line {}", i + 1))?;
        if o.lines {
            let who = l.speaker.as_deref().map(|s| format!(" {}", file_safe(s))).unwrap_or_default();
            let p = line_dir.join(format!("{:02}{who}.{}", i + 1, o.format.ext()));
            audio::write_line(src, &p, o.format, o.even_loudness.then_some(gains[i]))?;
            out.lines.push(p);
        }
    }
    let (seconds, loudness) = join.finish()?;
    out.seconds = seconds;
    out.loudness = loudness;

    if let Some(wav) = library {
        let names = job.lines.iter().fold(Vec::<String>::new(), |mut acc, l| {
            if !acc.contains(&l.voice_name) {
                acc.push(l.voice_name.clone());
            }
            acc
        });
        let take = Take {
            id: String::new(),
            text: job.lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join(" "),
            voice_id: "script".into(),
            voice_name: names.join(" + "),
            created_at: chrono::Utc::now(),
            seconds: 0.0,
            seed: None,
            mode: None,
            controls: BTreeMap::new(),
            decoder: None,
            via: job.via.clone(),
            parts: job.lines.iter().map(|l| ScriptLine { voice: l.voice_name.clone(), text: l.text.clone() }).collect(),
            temperature: None,
            voice_match: None,
            redone: 0,
            drifted: false,
            cut_off: job.lines.iter().any(|l| l.cut_off),
            starred: true,
            path: Default::default(),
        };
        // The WAV voiceover stays in the export folder; another format's helper WAV moves to the Library.
        out.take_id = takes::save_file(take, &wav, o.format == Format::Wav).ok().map(|t| t.id);
        if o.format != Format::Wav {
            let _ = std::fs::remove_file(&wav);
        }
    }
    if o.captions {
        let cues = captions::cues(&spans, o.caption_style);
        let p = folder.join(format!("{stem}.srt"));
        std::fs::write(&p, captions::to_srt(&cues))?;
        out.captions = Some(p);
        if o.vtt {
            let p = folder.join(format!("{stem}.vtt"));
            std::fs::write(&p, captions::to_vtt(&cues))?;
            out.vtt = Some(p);
        }
    }
    let saved = out.clone();
    update(id, |j| {
        j.exported = Some(saved);
        Ok(())
    })?;
    Ok(out)
}

fn file_safe(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-').collect::<String>().trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(secs: f64) -> Vec<u8> {
        let spec = hound::WavSpec { channels: 1, sample_rate: 24_000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
        let mut out = std::io::Cursor::new(Vec::new());
        let mut w = hound::WavWriter::new(&mut out, spec).unwrap();
        for i in 0..(secs * 24_000.0) as usize {
            w.write_sample(((i as f32 * 0.05).sin() * 8000.0) as i16).unwrap();
        }
        w.finalize().unwrap();
        out.into_inner()
    }

    #[test]
    fn export_voiceover_captions_and_lines() {
        let home = std::env::temp_dir().join(format!("booth-batch-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::env::set_var("BOOTH_HOME", &home);
        std::env::set_var("BOOTH_TAKES_DIR", home.join("takes"));
        let parsed = script::parse("# Cold phones\nNARRATOR: It isn't broken.\n\nQUOTE: Warm it up, and most of that charge comes back.", &[]);
        let id = "2026-11-15_120000-cold-phones".to_string();
        let folder = scripts_dir().join(&id);
        std::fs::create_dir_all(&folder).unwrap();
        let lines = parsed
            .lines
            .iter()
            .enumerate()
            .map(|(i, l)| {
                let path = folder.join(format!("line-{:03}-v1.wav", i + 1));
                std::fs::write(&path, tone(1.0 + i as f64)).unwrap();
                JobLine {
                    speaker: l.speaker.clone(),
                    voice: "v".into(),
                    voice_name: if i == 0 { "Cillian".into() } else { "Imogen".into() },
                    text: l.text.clone(),
                    para: l.para,
                    pause_ms: l.pause_ms,
                    state: LineState::Done,
                    seconds: 1.0 + i as f64,
                    seed: Some(1),
                    version: 1,
                    voice_match: None,
                    drifted: false,
                    cut_off: false,
                    error: None,
                    path: Some(path),
                }
            })
            .collect();
        let job = Job {
            id: id.clone(),
            title: title_of(&parsed),
            created_at: chrono::Utc::now(),
            lines,
            skipped: parsed.skipped.clone(),
            source: String::new(),
            voices: BTreeMap::new(),
            via: "app".into(),
            running: true, // left over from a run that ended with its process
            exported: None,
        };
        write(&job).unwrap();
        assert!(!get(&id).unwrap().running);
        let opts = ExportOptions { lines: true, ..Default::default() };
        let preview = captions_preview(&job, &opts);
        assert!(preview.contains("00:00:01,600 --> 00:00:03,600"), "{preview}");
        let out = export(&id, &opts).unwrap();
        assert_eq!(out.voiceover.as_ref().unwrap().file_name().unwrap(), "Cold phones.wav");
        assert!((out.seconds - 3.6).abs() < 0.01, "{}", out.seconds);
        let srt = std::fs::read_to_string(out.captions.unwrap()).unwrap();
        assert_eq!(srt, preview);
        assert_eq!(out.lines.len(), 2);
        assert!(out.lines[1].ends_with("02 QUOTE.wav"));
        let take = takes::find(&out.take_id.unwrap()).unwrap();
        assert_eq!(take.voice_name, "Cillian + Imogen");
        // the same as MP3, evened out, with short captions as .srt and .vtt
        let mp3 = ExportOptions { format: Format::Mp3, even_loudness: true, caption_style: Style::Short, vtt: true, ..opts.clone() };
        let out = export(&id, &mp3).unwrap();
        assert_eq!(out.voiceover.as_ref().unwrap().file_name().unwrap(), "Cold phones.mp3");
        assert!(out.lines.iter().all(|p| p.extension().unwrap() == "mp3") && out.lines.len() == 2);
        assert!(!out.folder.join(".library.wav").exists());
        let take = takes::find(&out.take_id.unwrap()).unwrap();
        assert!((take.seconds - 3.6).abs() < 0.01 && take.path.extension().unwrap() == "wav");
        let loud = out.loudness.unwrap();
        assert!((loud.lufs - audio::TARGET_LUFS).abs() < 1.0 && loud.true_peak <= audio::CEILING_DBTP + 0.05, "{loud:?}");
        let vtt = std::fs::read_to_string(out.vtt.unwrap()).unwrap();
        assert!(vtt.starts_with("WEBVTT

00:00:00.000 --> "), "{vtt}");
        let srt = std::fs::read_to_string(out.captions.unwrap()).unwrap();
        assert!(srt.contains("
It isn't broken.
") && srt.contains("
Warm it up,
"), "{srt}");
        assert_eq!(srt, captions_preview(&job, &mp3));
        // a redo clears the export and waits for the next run
        let j = redo(&id, 1, Some("Warm it up.".into())).unwrap();
        assert_eq!(j.lines[1].state, LineState::Waiting);
        assert!(j.exported.is_none() && export(&id, &opts).is_err());
        // A redo while this script's run is going returns at once (it used to lock RUNNING twice
        // and hang the thread, and every script call after it).
        *RUNNING.lock().unwrap() = Some(id.clone());
        let (tx, rx) = std::sync::mpsc::channel();
        let id2 = id.clone();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            let z = Zonos::new("http://127.0.0.1:9");
            let _ = tx.send(rt.block_on(run(&z, &id2, |_| {})).map(|j| j.id));
        });
        let got = rx.recv_timeout(std::time::Duration::from_secs(10)).expect("run hung with the script already running");
        assert_eq!(got.unwrap(), id);
        assert_eq!(running().as_deref(), Some(id.as_str()));
        *RUNNING.lock().unwrap() = None;
        let _ = std::fs::remove_dir_all(&home);
    }

    /// A/B files for "Even out loudness" from real line recordings. `BOOTH_AB_LINES` names a text
    /// file with one WAV path per line (read only, copied first); the voiceover with the option
    /// off and on goes to `BOOTH_AB_OUT`, and the measurements are printed.
    /// `cargo test -p booth-core --release ab_even_loudness -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn ab_even_loudness() {
        use ebur128::{EbuR128, Mode};
        let list = std::fs::read_to_string(std::env::var("BOOTH_AB_LINES").unwrap()).unwrap();
        let out_dir = PathBuf::from(std::env::var("BOOTH_AB_OUT").unwrap());
        std::fs::create_dir_all(&out_dir).unwrap();
        let home = std::env::temp_dir().join(format!("booth-ab-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::env::set_var("BOOTH_HOME", &home);
        std::env::set_var("BOOTH_TAKES_DIR", home.join("takes"));
        let id = "ab".to_string();
        let folder = scripts_dir().join(&id);
        std::fs::create_dir_all(&folder).unwrap();
        let mut lines = Vec::new();
        for (i, src) in list.lines().map(str::trim).filter(|l| !l.is_empty()).enumerate() {
            let src = PathBuf::from(src);
            let path = folder.join(format!("line-{:03}-v1.wav", i + 1));
            std::fs::copy(&src, &path).unwrap();
            let meta: serde_json::Value = std::fs::read(src.with_extension("json"))
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or_default();
            lines.push(JobLine {
                speaker: meta["voice_name"].as_str().map(String::from),
                voice: "v".into(),
                voice_name: meta["voice_name"].as_str().unwrap_or("").into(),
                text: meta["text"].as_str().unwrap_or("line").into(),
                para: false,
                pause_ms: 0,
                state: LineState::Done,
                seconds: audio::read_wav(&path).unwrap().seconds(),
                seed: None,
                version: 1,
                voice_match: None,
                drifted: false,
                cut_off: false,
                error: None,
                path: Some(path),
            });
        }
        let job = Job {
            id: id.clone(),
            title: "Booth AB".into(),
            created_at: chrono::Utc::now(),
            lines,
            skipped: Vec::new(),
            source: String::new(),
            voices: BTreeMap::new(),
            via: "app".into(),
            running: false,
            exported: None,
        };
        write(&job).unwrap();
        let measure = |p: &std::path::Path| {
            let pcm = audio::read_wav(p).unwrap();
            let mut m = EbuR128::new(1, pcm.rate, Mode::I | Mode::TRUE_PEAK).unwrap();
            m.add_frames_i16(&pcm.samples).unwrap();
            let whole = (m.loudness_global().unwrap(), 20.0 * m.true_peak(0).unwrap().log10());
            // each line where it sits in the voiceover (the same pauses as the export)
            let g = gaps(&job, &ExportOptions::default());
            let mut at = 0usize;
            let mut per = Vec::new();
            for (i, l) in job.lines.iter().enumerate() {
                if i > 0 {
                    at += (pcm.rate as usize * g[i] as usize) / 1000;
                }
                let n = (l.seconds * pcm.rate as f64).round() as usize;
                let mut lm = EbuR128::new(1, pcm.rate, Mode::I).unwrap();
                lm.add_frames_i16(&pcm.samples[at..at + n]).unwrap();
                per.push(lm.loudness_global().unwrap());
                at += n;
            }
            (whole, per)
        };
        let sources: Vec<PathBuf> = job.lines.iter().map(|l| l.path.clone().unwrap()).collect();
        let solo: Vec<String> = sources
            .iter()
            .map(|p| {
                let pcm = audio::read_wav(p).unwrap();
                let mut m = EbuR128::new(1, pcm.rate, Mode::I).unwrap();
                m.add_frames_i16(&pcm.samples).unwrap();
                format!("{:.1} ({:.1} s)", m.loudness_global().unwrap(), pcm.seconds())
            })
            .collect();
        eprintln!("lines on their own: {solo:?}");
        eprintln!("gains: {:.1?}", audio::plan_gains(&sources, &gaps(&job, &ExportOptions::default())).unwrap());
        for (name, even, format) in [("off", false, Format::Wav), ("off", false, Format::Mp3), ("on", true, Format::Wav), ("on", true, Format::Mp3)] {
            let o = ExportOptions { even_loudness: even, format, ..Default::default() };
            let out = export(&id, &o).unwrap();
            let src = out.voiceover.unwrap();
            let dst = out_dir.join(format!("even-loudness-{name}.{}", format.ext()));
            std::fs::copy(&src, &dst).unwrap();
            if format == Format::Wav {
                let ((lufs, tp), per) = measure(&dst);
                let ok: Vec<f64> = per.iter().copied().filter(|l| l.is_finite()).collect();
                let spread = ok.iter().copied().fold(f64::MIN, f64::max) - ok.iter().copied().fold(f64::MAX, f64::min);
                let mean = ok.iter().sum::<f64>() / ok.len() as f64;
                let sd = (ok.iter().map(|l| (l - mean).powi(2)).sum::<f64>() / ok.len() as f64).sqrt();
                eprintln!("{name}: {lufs:.1} LUFS, {tp:.2} dBTP, lines spread {spread:.1} dB (sd {sd:.1}), per line {per:.1?}");
            } else {
                eprintln!("{name} mp3: {:?}", out.loudness);
            }
        }
        let _ = std::fs::remove_dir_all(&home);
    }
}
