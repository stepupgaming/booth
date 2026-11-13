//! A small client for the ZONOS3 v2 HTTP API (`servers/api/openapi.yaml` in the ZONOS3 repo).
//! Only the routes Booth uses are wrapped; request bodies are built from `GenerateRequest`.

use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::Value;

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
        let http = reqwest::Client::builder()
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
        let session = format!("booth-{}", uuid::Uuid::new_v4());
        let sp = self.add_speaker(&session, name, audio, name).await?;
        self.embedding_npy(&sp.id, Some(&session)).await
    }

    /// Synthesize a whole take as a WAV file.
    pub async fn generate(&self, req: &GenerateRequest) -> Result<Audio> {
        let _busy = crate::engine::BusyGuard::new();
        let mut body = serde_json::to_value(req)?;
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
            bail!("{}", friendly_error(status.as_u16(), &msg));
        }
        let header = |n: &str| r.headers().get(n).and_then(|v| v.to_str().ok()).map(str::to_string);
        let seed = header("x-seed").and_then(|s| s.parse().ok());
        let decoder = header("x-decoder");
        let wav = r.bytes().await?.to_vec();
        Ok(Audio { wav, seed, decoder })
    }
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
