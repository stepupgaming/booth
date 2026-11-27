//! A small client for the ZONOS3 v2 HTTP API (`servers/api/openapi.yaml` in the ZONOS3 repo).
//! Only the routes Booth uses are wrapped; request bodies are built from `GenerateRequest`.

use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// How many times a take that ends mid-sound is made again (see [`Zonos::generate`]).
pub const CUT_RETRIES: u32 = 2;
/// A take that still ends louder than [`crate::takes::HARD_CUT_DB`] is chopped mid-word in a way a
/// fade can't hide, so it gets more redos than a take that only ends a little loud.
pub const HARD_CUT_RETRIES: u32 = 5;
/// The fade on a take that still ends mid-sound after the retries.
const CUT_FADE_MS: u32 = 30;

fn fresh_seed() -> u64 {
    1 + (uuid::Uuid::new_v4().as_u128() % 2_147_483_000) as u64
}

#[derive(Clone)]
pub struct Zonos {
    pub base: String,
    http: reqwest::Client,
}

/// How the voice is applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SpeakerMode {
    /// Raw reference codes: closest to the clip, but voice-shaping controls are refused (HTTP 400).
    #[default]
    Reference,
    /// Speaker embedding: a bit less exact, and every control works.
    Embedding,
}

/// One synthesis request. Unset fields use the server's defaults.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GenerateRequest {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speaker_mode: Option<SpeakerMode>,
    /// A voice id from `GET /v2/speakers` (the default voices).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speaker_embedding_id: Option<String>,
    /// A reference clip, base64 (any container the server's ffmpeg reads).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speaker_audio_base64: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speaker_audio_name: Option<String>,
    /// A saved `.npy` speaker embedding, base64.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speaker_embedding_base64: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speaker_embedding_name: Option<String>,
    /// Steering controls, `"<set>.<direction>" -> dose`.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub controls: BTreeMap<String, f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accent_strength: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decoder: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub laci: Option<bool>,
}

/// A finished take as the server returned it.
pub struct Audio {
    /// A 16-bit WAV file.
    pub wav: Vec<u8>,
    pub seed: Option<u64>,
    pub decoder: Option<String>,
    /// No try came out whole: the kept take is too short for its text or ends chopped mid-word,
    /// so it may stop before the text is done (see [`Zonos::generate`]).
    pub cut_off: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Speaker {
    pub id: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl Zonos {
    pub fn new(base: impl Into<String>) -> Self {
        let http = crate::http()
            .connect_timeout(Duration::from_secs(3))
            .build()
            .expect("http client");
        Self { base: base.into().trim_end_matches('/').to_string(), http }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    pub async fn health(&self) -> bool {
        matches!(
            self.http.get(self.url("/v2/health")).timeout(Duration::from_secs(2)).send().await,
            Ok(r) if r.status().is_success()
        )
    }

    /// Run a request, but give up once the engine is gone: if it died (on Android, the system
    /// kills it in the background) the request would otherwise sit until its timeout and Booth
    /// would look frozen. Health answers while a take renders, but slowly on a busy phone, so a
    /// refused connection (nothing listening) ends the request fast and slow answers only after
    /// about two minutes of them.
    async fn watched<T>(&self, work: impl std::future::Future<Output = Result<T>>) -> Result<T> {
        const EVERY: Duration = Duration::from_secs(5);
        const GONE: u32 = 2;
        const SILENT: u32 = 12;
        let watch = async {
            let (mut gone, mut silent) = (0, 0);
            loop {
                tokio::time::sleep(EVERY).await;
                match self.http.get(self.url("/v2/health")).timeout(Duration::from_secs(5)).send().await {
                    Ok(r) if r.status().is_success() => (gone, silent) = (0, 0),
                    Err(e) if e.is_connect() => {
                        gone += 1;
                        crate::diag::note(format!("engine: refused a health check during a request ({gone}/{GONE})"));
                    }
                    _ => {
                        silent += 1;
                        crate::diag::note(format!("engine: slow to answer a health check during a request ({silent}/{SILENT})"));
                    }
                }
                if gone >= GONE || silent >= SILENT {
                    return;
                }
            }
        };
        let stopped = || {
            anyhow::anyhow!(
                "the voice engine stopped in the middle of this take{}. Generate again and Booth starts it fresh",
                if cfg!(target_os = "android") { " (Android may have closed it while Booth was in the background)" } else { "" }
            )
        };
        tokio::select! {
            r = work => match r {
                // A dropped connection to an engine that no longer answers: say that, not the HTTP error.
                Err(e) if !self.health().await => {
                    crate::diag::note(format!("engine: gone after a failed request ({e:#})"));
                    Err(stopped())
                }
                r => r,
            },
            () = watch => Err(stopped()),
        }
    }

    async fn get_json(&self, path: &str) -> Result<Value> {
        let r = self.http.get(self.url(path)).send().await.with_context(|| format!("GET {path}"))?;
        let status = r.status();
        if !status.is_success() {
            bail!("GET {path}: {status} {}", r.text().await.unwrap_or_default());
        }
        Ok(r.json().await?)
    }

    pub async fn capabilities(&self) -> Result<Value> {
        self.get_json("/v2/capabilities").await
    }

    /// The steering bundle: control sets, directions, UI metadata.
    pub async fn controls(&self) -> Result<Value> {
        self.get_json("/v2/controls").await
    }

    /// The friendly control catalog, fetched once per engine address.
    pub async fn catalog(&self) -> Result<std::sync::Arc<crate::controls::Catalog>> {
        use std::sync::{Arc, Mutex, OnceLock};
        static CACHE: OnceLock<Mutex<Option<(String, Arc<crate::controls::Catalog>)>>> = OnceLock::new();
        let cache = CACHE.get_or_init(|| Mutex::new(None));
        if let Some((base, cat)) = cache.lock().unwrap().as_ref() {
            if *base == self.base {
                return Ok(cat.clone());
            }
        }
        let cat = Arc::new(crate::controls::Catalog::from_doc(&self.controls().await?)?);
        *cache.lock().unwrap() = Some((self.base.clone(), cat.clone()));
        Ok(cat)
    }

    /// The default voices shipped with the model (and any cached in the session).
    pub async fn speakers(&self) -> Result<Vec<Speaker>> {
        let v = self.get_json("/v2/speakers").await?;
        let list = v.get("speakers").cloned().unwrap_or(v);
        Ok(serde_json::from_value(list)?)
    }

    pub async fn speaker_preview(&self, id: &str) -> Result<Vec<u8>> {
        self.get_bytes(&format!("/v2/speakers/{id}/preview")).await
    }

    /// A voice's speaker embedding as `.npy` bytes (the route answers JSON; Booth stores `.npy`).
    pub async fn speaker_embedding(&self, id: &str) -> Result<Vec<u8>> {
        self.embedding_npy(id, None).await
    }

    async fn embedding_npy(&self, id: &str, session: Option<&str>) -> Result<Vec<u8>> {
        let mut req = self.http.get(self.url(&format!("/v2/speakers/{id}/embedding")));
        if let Some(s) = session {
            req = req.header("X-TTS-Session-ID", s);
        }
        let r = req.send().await?;
        let status = r.status();
        if !status.is_success() {
            bail!("embedding: {status} {}", r.text().await.unwrap_or_default());
        }
        #[derive(Deserialize)]
        struct Emb {
            embedding: Vec<f32>,
        }
        let e: Emb = r.json().await.context("reading the embedding")?;
        Ok(npy_f32(&e.embedding))
    }

    async fn get_bytes(&self, path: &str) -> Result<Vec<u8>> {
        let r = self.http.get(self.url(path)).send().await?;
        let status = r.status();
        if !status.is_success() {
            bail!("GET {path}: {status} {}", r.text().await.unwrap_or_default());
        }
        Ok(r.bytes().await?.to_vec())
    }

    /// Cache a clip in a session and return the server's speaker object (its id gives the embedding).
    pub async fn add_speaker(&self, session: &str, label: &str, audio: &[u8], name: &str) -> Result<Speaker> {
        let body = serde_json::json!({
            "label": label,
            "speaker_audio_base64": b64(audio),
            "speaker_audio_name": name,
        });
        let r = self
            .http
            .post(self.url("/v2/speakers"))
            .header("X-TTS-Session-ID", session)
            .json(&body)
            .send()
            .await?;
        let status = r.status();
        if !status.is_success() {
            bail!("POST /v2/speakers: {status} {}", r.text().await.unwrap_or_default());
        }
        let v: Value = r.json().await?;
        Ok(serde_json::from_value(v.get("speaker").cloned().unwrap_or(v))?)
    }

    /// Embedding (`.npy`) of a clip: caches it in a throwaway session, then reads the embedding back.
    pub async fn embed_clip(&self, audio: &[u8], name: &str) -> Result<Vec<u8>> {
        let _busy = crate::engine::BusyGuard::new();
        let t0 = std::time::Instant::now();
        crate::diag::note(format!("embed clip ({} KB): sent", audio.len() / 1024));
        let session = format!("booth-{}", uuid::Uuid::new_v4());
        let r = self
            .watched(async {
                let sp = self.add_speaker(&session, name, audio, name).await?;
                self.embedding_npy(&sp.id, Some(&session)).await
            })
            .await;
        let secs = t0.elapsed().as_secs_f64();
        match &r {
            Ok(_) => crate::diag::note(format!("embed clip: ok in {secs:.1} s")),
            Err(e) => crate::diag::note(format!("embed clip: FAILED after {secs:.1} s: {e:#}")),
        }
        r
    }

    /// Synthesize a whole take as a WAV file.
    ///
    /// The model sometimes stops a take before the text is done: it predicts the end of speech
    /// while the last sound is still going, so the take stops mid-word (about half of Exact takes
    /// in a 2026-11-18 test on ZONOS3 96d9e0e, with LACI on or off; Shapeable takes rarely), or it
    /// ends cleanly after the first sentence or with words left out, so the take is far too short
    /// for its text (see [`crate::takes::too_short`]). Such a take is made again with a fresh seed,
    /// up to [`CUT_RETRIES`] times (up to [`HARD_CUT_RETRIES`] while it still ends loud or is too
    /// short), and the best try is kept: one with all its words first, then the quietest ending.
    /// If every try ends mid-sound, the kept one gets a short fade so it doesn't click; if the kept
    /// one is still too short or chopped mid-word, [`Audio::cut_off`] says so. The kept take's own
    /// seed is returned, so asking for that seed again gives the same take. That holds for a
    /// pinned seed too: a take with that seed that stops early is redone like any other.
    pub async fn generate(&self, req: &GenerateRequest) -> Result<Audio> {
        let first = self.generate_fallback(req).await?;
        let Some(end) = crate::takes::ending(&req.text, &first.wav) else { return Ok(first) };
        let mut best = (end, first);
        let mut redone = 0;
        // On the processor an Exact take can take a minute or more, so one redo at most there.
        let slow = crate::engine::on_processor();
        loop {
            let limit = if slow {
                1
            } else if best.0.short || best.0.db > crate::takes::HARD_CUT_DB {
                HARD_CUT_RETRIES
            } else {
                CUT_RETRIES
            };
            if best.0.clean() || redone >= limit {
                break;
            }
            redone += 1;
            let why = if best.0.short {
                "is too short for its text".to_string()
            } else {
                format!("ends mid-sound ({:.0} dB)", best.0.db)
            };
            crate::diag::note(format!("take {why}, redoing ({redone}/{limit})"));
            let mut again = req.clone();
            again.seed = Some(fresh_seed());
            match self.generate_fallback(&again).await {
                Ok(a) => {
                    let unreadable = crate::takes::Ending { db: 0.0, short: true };
                    let end = crate::takes::ending(&req.text, &a.wav).unwrap_or(unreadable);
                    if end.better_than(&best.0) {
                        best = (end, a);
                    }
                }
                Err(e) => {
                    crate::diag::note(format!("redo failed, keeping the take: {e:#}"));
                    break;
                }
            }
        }
        let (end, mut audio) = best;
        if end.db > crate::takes::CUT_DB {
            crate::diag::note(format!("every try ended mid-sound; fading the last {CUT_FADE_MS} ms"));
            audio.wav = crate::takes::fade_out(&audio.wav, CUT_FADE_MS)?;
        }
        if end.short {
            crate::diag::note("every try was too short for its text; keeping the longest-sounding one");
        }
        // A take that only ends a little loud is a breath or a tail the fade hides (its words were
        // all there in a 2026-11-27 check); one that is too short or still chopped is marked.
        audio.cut_off = end.short || end.db > crate::takes::HARD_CUT_DB;
        Ok(audio)
    }

    /// One take, with the ways out of an engine problem:
    /// - the engine couldn't encode an Exact voice's recording on the graphics card: it restarts on
    ///   the processor and the take is tried once more there;
    /// - the take failed inside the engine (HTTP 500): it is tried again, and if that fails too
    ///   the engine restarts and it is tried once more (a Mac out of graphics memory needs that);
    /// - the take asked for a decoder the engine didn't load: the engine restarts with it, or the
    ///   take uses the engine's own decoder when a restart wouldn't load the one asked for;
    /// - the engine can't guard this take against skips (no LACI heads for its language, or an
    ///   engine started without them): the take is made without the guard.
    async fn generate_fallback(&self, req: &GenerateRequest) -> Result<Audio> {
        let generation = crate::engine::fallback_generation();
        match self.generate_once(req).await {
            Err(e) if e.downcast_ref::<LaciUnavailable>().is_some() => {
                crate::diag::note("the engine can't guard this take against skips; making it without the guard");
                Box::pin(self.generate_fallback(&GenerateRequest { laci: None, ..req.clone() })).await
            }
            Err(e) if e.downcast_ref::<ReferenceEncodeFailed>().is_some() => {
                match crate::engine::processor_fallback(&self.base, generation).await {
                    Some(z) => z.generate_once(req).await,
                    None => Err(e),
                }
            }
            Err(e) if e.downcast_ref::<EngineFailed>().is_some() => {
                crate::diag::note("the engine failed this take; trying it again");
                let r = match self.generate_once(req).await {
                    Err(e) if e.downcast_ref::<EngineFailed>().is_some() => {
                        match crate::engine::recover(&self.base, generation).await {
                            Some(z) => z.generate_once(req).await,
                            None => Err(e),
                        }
                    }
                    r => r,
                };
                r.map_err(|e| if e.downcast_ref::<EngineFailed>().is_some() { anyhow::anyhow!(GAVE_UP) } else { e })
            }
            Err(e) if e.downcast_ref::<UnknownDecoder>().is_some() => {
                let want = req.decoder.clone().unwrap_or_default();
                match crate::engine::switch_decoder(&self.base, generation, &want).await {
                    Some(z) => z.generate_once(req).await,
                    None => {
                        crate::diag::note(format!("the {want} decoder isn't available; using the engine's own"));
                        self.generate_once(&GenerateRequest { decoder: None, ..req.clone() }).await
                    }
                }
            }
            r => r,
        }
    }

    async fn generate_once(&self, req: &GenerateRequest) -> Result<Audio> {
        let _busy = crate::engine::BusyGuard::new();
        let t0 = std::time::Instant::now();
        let what = format!(
            "generate {} chars, {}",
            req.text.chars().count(),
            match (&req.speaker_embedding_id, &req.speaker_audio_base64, &req.speaker_embedding_base64) {
                (Some(id), _, _) => format!("voice {id}"),
                (_, Some(_), _) => "reference clip".into(),
                (_, _, Some(_)) => "embedding".into(),
                _ => "default voice".into(),
            }
        );
        // The pronunciation list applies here, the one place text leaves for the engine, so
        // the app, scripts and Claude all get it while takes keep the words as typed.
        let spoken = crate::pronounce::for_engine(&req.text, req.language.as_deref());
        crate::diag::note(format!("{what}{}: sent", if spoken.is_some() { ", pronunciation list applied" } else { "" }));
        let r = self.watched(self.generate_inner(req, spoken.as_deref())).await;
        let secs = t0.elapsed().as_secs_f64();
        match &r {
            Ok(a) => crate::diag::note(format!(
                "{what}: ok in {secs:.1} s, {:.1} s of audio",
                wav_seconds(&a.wav)
            )),
            Err(e) => crate::diag::note(format!("{what}: FAILED after {secs:.1} s: {e:#}")),
        }
        r
    }

    async fn generate_inner(&self, req: &GenerateRequest, spoken: Option<&str>) -> Result<Audio> {
        let mut body = serde_json::to_value(req)?;
        if let Some(text) = spoken {
            body["text"] = Value::String(text.to_string());
        }
        body["stream"] = Value::Bool(false);
        body["format"] = Value::String("wav".into());
        let r = self
            .http
            .post(self.url("/v2/generate"))
            .json(&body)
            .timeout(Duration::from_secs(600))
            .send()
            .await
            .context("POST /v2/generate")?;
        let status = r.status();
        if !status.is_success() {
            let msg = r.text().await.unwrap_or_default();
            if status.as_u16() == 400 && msg.contains(REFERENCE_ENCODE_FAILED) {
                // The log keeps the engine's own words; people see Booth's.
                crate::diag::note(format!("engine said: {}", msg.trim()));
                return Err(ReferenceEncodeFailed.into());
            }
            if status.as_u16() == 500 && msg.contains(GENERATION_FAILED) {
                crate::diag::note(format!("engine said: {}", msg.trim()));
                return Err(EngineFailed.into());
            }
            if status.as_u16() == 400 && req.laci == Some(true) && msg.contains(LACI_REFUSED) {
                crate::diag::note(format!("engine said: {}", msg.trim()));
                return Err(LaciUnavailable.into());
            }
            if status.as_u16() == 400 && msg.contains(UNKNOWN_DECODER) {
                crate::diag::note(format!("engine said: {}", msg.trim()));
                return Err(UnknownDecoder.into());
            }
            bail!("{}", friendly_error(status.as_u16(), &msg));
        }
        let header = |n: &str| r.headers().get(n).and_then(|v| v.to_str().ok()).map(str::to_string);
        let seed = header("x-seed").and_then(|s| s.parse().ok());
        let decoder = header("x-decoder");
        let wav = r.bytes().await?.to_vec();
        Ok(Audio { wav, seed, decoder, cut_off: false })
    }
}

/// Length of a PCM WAV from its header (0 if it can't tell).
fn wav_seconds(wav: &[u8]) -> f64 {
    if wav.len() < 44 {
        return 0.0;
    }
    let u16_at = |i: usize| u16::from_le_bytes([wav[i], wav[i + 1]]) as f64;
    let rate = u32::from_le_bytes([wav[24], wav[25], wav[26], wav[27]]) as f64;
    let bytes_per_frame = u16_at(22) * u16_at(34) / 8.0;
    if rate == 0.0 || bytes_per_frame == 0.0 {
        return 0.0;
    }
    (wav.len() - 44) as f64 / bytes_per_frame / rate
}

/// A 1-D float32 array as a `.npy` file (format 1.0).
pub fn npy_f32(values: &[f32]) -> Vec<u8> {
    let mut header = format!("{{'descr': '<f4', 'fortran_order': False, 'shape': ({},), }}", values.len());
    // magic (6) + version (2) + header length (2) + header, padded with spaces to a multiple
    // of 64 bytes and ending in a newline.
    while (10 + header.len() + 1) % 64 != 0 {
        header.push(' ');
    }
    header.push('\n');
    let mut out = Vec::with_capacity(10 + header.len() + values.len() * 4);
    out.extend_from_slice(b"\x93NUMPY\x01\x00");
    out.extend_from_slice(&(header.len() as u16).to_le_bytes());
    out.extend_from_slice(header.as_bytes());
    for v in values {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

pub fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Turn the server's error body into one sentence a person can act on.
/// What the engine says (HTTP 400) when it can't turn a voice's recording into the codes Exact
/// mode speaks from. The same words cover an empty clip, one over the length limit and the codec
/// encoder failing, so the "(maximum 60 seconds)" after it is no clue to the cause.
const REFERENCE_ENCODE_FAILED: &str = "reference audio encoding failed";

/// The engine couldn't prepare an Exact voice's recording. Its message is for developers, so
/// people see this one.
#[derive(Debug)]
pub struct ReferenceEncodeFailed;

impl std::fmt::Display for ReferenceEncodeFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(
            "Booth couldn't get this voice's recording ready for Exact mode. The voice still works in \
             Shapeable mode. If it keeps happening, press Save a report in Settings and send us the file.",
        )
    }
}

impl std::error::Error for ReferenceEncodeFailed {}

/// What the engine says (HTTP 500) when a take fails inside it: on a Mac, most often the graphics
/// card running out of memory, after which every take fails until the engine restarts.
const GENERATION_FAILED: &str = "generation failed";

/// The take failed inside the engine (see [`GENERATION_FAILED`]).
#[derive(Debug)]
pub struct EngineFailed;

impl std::fmt::Display for EngineFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the voice engine failed this take (generation failed)")
    }
}

/// What people see when a take still fails after the retry and the restart.
const GAVE_UP: &str = "The voice engine couldn't make this take, even after a restart. If your computer is low on memory, closing other apps can help. If it keeps happening, press Save a report in Settings and send us the file.";

impl std::error::Error for EngineFailed {}

/// What the engine says (HTTP 400) when a request names a decoder it didn't load.
const UNKNOWN_DECODER: &str = "Unknown decoder";

/// The take asked for a decoder the running engine didn't load.
#[derive(Debug)]
pub struct UnknownDecoder;

impl std::fmt::Display for UnknownDecoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the voice engine hasn't loaded the decoder this take asked for")
    }
}

impl std::error::Error for UnknownDecoder {}

/// In each of the engine's refusals (HTTP 400) of a take that asks for LACI: the engine was started
/// without heads, has none for the take's language, or the take uses CFG.
const LACI_REFUSED: &str = "LACI";

/// The engine can't guard this take against skips (see [`LACI_REFUSED`]).
#[derive(Debug)]
pub struct LaciUnavailable;

impl std::fmt::Display for LaciUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the voice engine can't guard this take against skips")
    }
}

impl std::error::Error for LaciUnavailable {}

fn friendly_error(status: u16, body: &str) -> String {
    let detail = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v.get("detail").or_else(|| v.get("error")).cloned())
        .map(|d| match d {
            Value::String(s) => s,
            other => other.to_string(),
        })
        .unwrap_or_else(|| body.trim().to_string());
    if status == 400 && detail.contains("embedding") {
        format!("{detail} (voice shaping needs a Shapeable voice; Exact voices can't change gender, age, texture, accent or room)")
    } else {
        format!("the engine refused the request ({status}): {detail}")
    }
}
