//! The model files Booth needs, grouped into packs, and a resumable, checksummed downloader.
//!
//! Weights come from the Hugging Face repo `Zyphra/ZONOS3-Mini` (Apache-2.0), pinned to one commit
//! by `models.lock.json` (made by `scripts/models-lock.mjs`), which also gives every file's size and
//! SHA-256. Each file is fetched from Hugging Face at that commit, and from Booth's GitHub mirror
//! release if Hugging Face fails; either way the bytes must match the lock. Because the bytes are
//! the same everywhere, a download that breaks on one source resumes from the other.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

pub const HF_REPO: &str = "Zyphra/ZONOS3-Mini";

/// A source that sends nothing for this long is dropped for the next one.
const STALL: Duration = Duration::from_secs(30);

#[derive(Deserialize)]
struct Lock {
    repo: String,
    revision: String,
    /// Base URL of the GitHub release that holds the copies.
    mirror: String,
    packs: HashMap<String, Vec<LockFile>>,
}

#[derive(Deserialize)]
struct LockFile {
    rel: String,
    size: u64,
    sha256: String,
    /// The file's copy on the mirror: one asset, or parts to join in order.
    mirror: Vec<LockPart>,
}

#[derive(Deserialize)]
struct LockPart {
    name: String,
    size: u64,
}

fn lock() -> &'static Lock {
    static LOCK: OnceLock<Lock> = OnceLock::new();
    LOCK.get_or_init(|| {
        serde_json::from_str(include_str!("../models.lock.json")).expect("models.lock.json is valid")
    })
}

/// The Hugging Face commit Booth's model files are pinned to.
pub fn pinned_revision() -> &'static str {
    &lock().revision
}

/// A group of files that is downloaded together.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pack {
    /// Backbone, small decoder, speaker encoder, steering, default voices. Needed to speak at all.
    Core,
    /// Raw-reference ("Exact") cloning: the codec encoder and its SSL teacher.
    Exact,
    /// The full-size decoder (`zac.gguf`), a quality option.
    FullDecoder,
    /// The NVIDIA (CUDA) build of the engine, a Windows download. Without it Booth runs on the processor.
    GpuEngine,
}

/// The CUDA engine: `booth-engine.zip` on a prerelease of the Booth repo, built by the release
/// workflow from the same ZONOS3-Mini commit as the bundled CPU engine. Pinned by size and SHA-256.
pub const CUDA_ENGINE_ID: &str = "cuda-f9863b3-12.8";
const CUDA_ENGINE_URL: &str =
    "https://github.com/stepupgaming/booth/releases/download/engine-f9863b3-cuda12.8/booth-engine.zip";
const CUDA_ENGINE_SIZE: u64 = 743_066_931;
const CUDA_ENGINE_SHA256: &str = "9fd0d675b4c6aeac6ea00dd5054593183517297e4f26c6675b421eac21fe53f0";

/// Where the CUDA engine lives once downloaded.
pub fn cuda_engine_dir() -> PathBuf {
    crate::paths::engines_dir().join(CUDA_ENGINE_ID)
}

/// The CUDA engine to use, if there is one: the download, else the `engine/` folder that Booth 0.1
/// installed next to the app (an update to 0.2 leaves it in place). Windows only.
pub fn installed_cuda_engine() -> Option<PathBuf> {
    if !cfg!(windows) {
        return None;
    }
    let exe = crate::paths::SERVER_EXE;
    let legacy = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("engine")));
    [Some(cuda_engine_dir()), legacy].into_iter().flatten().find(|d| d.join(exe).is_file())
}

impl Pack {
    /// The packs offered on this platform. The NVIDIA engine is a Windows download; macOS uses the
    /// bundled Metal engine and Linux the bundled CPU engine.
    #[cfg(windows)]
    pub const ALL: &'static [Pack] = &[Pack::Core, Pack::GpuEngine, Pack::Exact, Pack::FullDecoder];
    #[cfg(not(windows))]
    pub const ALL: &'static [Pack] = &[Pack::Core, Pack::Exact, Pack::FullDecoder];

    /// Files of the pack, relative to the repo's `gguf/` folder. A trailing `/` is a folder.
    pub fn files(self) -> &'static [&'static str] {
        match self {
            Pack::Core => &[
                "backbone-ptq.gguf",
                "zac-keep2.gguf",
                "spk-encoder.gguf",
                "steer.gguf",
                "voices/",
            ],
            Pack::Exact => &["zac-encoder.gguf", "ssl-teacher.gguf"],
            Pack::FullDecoder => &["zac.gguf"],
            Pack::GpuEngine => &["zonos3-server (CUDA)"],
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Pack::Core => "Voice engine",
            Pack::Exact => "Exact cloning",
            Pack::FullDecoder => "Studio decoder",
            Pack::GpuEngine => "NVIDIA graphics engine",
        }
    }
}

/// The backbone file, under its Hub name or the name the ZONOS3 tools give it.
pub fn backbone_path(models: &Path) -> Option<PathBuf> {
    ["backbone-ptq.gguf", "zonos3-ptq.gguf"]
        .iter()
        .map(|n| models.join(n))
        .find(|p| p.is_file())
}

/// True when every file of the pack is on disk (sizes are checked by `plan`, not here).
pub fn pack_present(models: &Path, pack: Pack) -> bool {
    if pack == Pack::GpuEngine {
        return installed_cuda_engine().is_some();
    }
    pack.files().iter().all(|f| {
        if let Some(dir) = f.strip_suffix('/') {
            models.join(dir).is_dir()
        } else if *f == "backbone-ptq.gguf" {
            backbone_path(models).is_some()
        } else {
            models.join(f).is_file()
        }
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackStatus {
    pub pack: Pack,
    pub label: String,
    pub present: bool,
}

pub fn status(models: &Path) -> Vec<PackStatus> {
    Pack::ALL
        .iter()
        .map(|&pack| PackStatus {
            pack,
            label: pack.label().into(),
            present: pack_present(models, pack),
        })
        .collect()
}

/// One file to fetch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteFile {
    /// Path relative to the repo's `gguf/` folder, e.g. `voices/English/Imogen_en-British_F_1.wav`.
    pub rel: String,
    pub size: u64,
    pub sha256: Option<String>,
    /// Where to get it, best first. Every source serves the same bytes.
    pub sources: Vec<Source>,
}

/// One place to download a file from: a single URL, or parts to join in order (the GitHub mirror
/// splits files over its 2 GiB asset limit).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    pub name: String,
    pub parts: Vec<SourcePart>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourcePart {
    pub url: String,
    pub size: u64,
}

/// The files of `pack` with their sizes, checksums and sources, from the built-in lock (no network).
///
/// `BOOTH_MODEL_SOURCE=hf` or `=github` uses only that source; `BOOTH_HF_BASE` replaces
/// `https://huggingface.co` (both are for testing the fallback).
pub async fn plan(_client: &reqwest::Client, pack: Pack) -> Result<Vec<RemoteFile>> {
    if pack == Pack::GpuEngine {
        return Ok(vec![RemoteFile {
            rel: "booth-engine-cuda.zip".into(),
            size: CUDA_ENGINE_SIZE,
            sha256: Some(CUDA_ENGINE_SHA256.into()),
            sources: vec![Source {
                name: "GitHub".into(),
                parts: vec![SourcePart { url: CUDA_ENGINE_URL.into(), size: CUDA_ENGINE_SIZE }],
            }],
        }]);
    }
    let lock = lock();
    let key = match pack {
        Pack::Core => "core",
        Pack::Exact => "exact",
        Pack::FullDecoder => "full_decoder",
        Pack::GpuEngine => unreachable!(),
    };
    let files = lock.packs.get(key).with_context(|| format!("models.lock.json has no {key} pack"))?;
    let only = std::env::var("BOOTH_MODEL_SOURCE").unwrap_or_default().to_ascii_lowercase();
    let hf_base = std::env::var("BOOTH_HF_BASE").unwrap_or_else(|_| "https://huggingface.co".into());
    Ok(files
        .iter()
        .map(|f| {
            let mut sources = Vec::new();
            if only != "github" {
                let path = f.rel.split('/').map(urlencode).collect::<Vec<_>>().join("/");
                sources.push(Source {
                    name: "Hugging Face".into(),
                    parts: vec![SourcePart {
                        url: format!("{}/{}/resolve/{}/gguf/{path}", hf_base.trim_end_matches('/'), lock.repo, lock.revision),
                        size: f.size,
                    }],
                });
            }
            if only != "hf" {
                sources.push(Source {
                    name: "GitHub mirror".into(),
                    parts: f
                        .mirror
                        .iter()
                        .map(|p| SourcePart { url: format!("{}/{}", lock.mirror, p.name), size: p.size })
                        .collect(),
                });
            }
            RemoteFile { rel: f.rel.clone(), size: f.size, sha256: Some(f.sha256.clone()), sources }
        })
        .collect())
}

/// Progress of a pack download, reported after every chunk (throttle on the receiving side).
#[derive(Debug, Clone, Serialize)]
pub struct Progress {
    pub pack: Pack,
    pub file: String,
    pub done_bytes: u64,
    pub total_bytes: u64,
}

/// Download `pack` into `models`, skipping files already complete, resuming `.part` files,
/// and checking every file against its SHA-256.
pub async fn download(
    client: &reqwest::Client,
    models: &Path,
    pack: Pack,
    mut on_progress: impl FnMut(Progress),
) -> Result<()> {
    if pack == Pack::GpuEngine {
        return download_cuda_engine(client, on_progress).await;
    }
    let files = plan(client, pack).await?;
    let total: u64 = files.iter().map(|f| f.size).sum();
    let mut done: u64 = 0;
    for f in &files {
        let dest = models.join(f.rel.replace('/', std::path::MAIN_SEPARATOR_STR));
        let have = dest.metadata().map(|m| m.len()).unwrap_or(0);
        if have == f.size && dest.is_file() {
            done += f.size;
            on_progress(Progress { pack, file: f.rel.clone(), done_bytes: done, total_bytes: total });
            continue;
        }
        fetch_one(client, f, &dest, |n| {
            on_progress(Progress { pack, file: f.rel.clone(), done_bytes: done + n, total_bytes: total })
        })
        .await
        .with_context(|| format!("downloading {}", f.rel))?;
        done += f.size;
    }
    Ok(())
}

async fn fetch_one(
    client: &reqwest::Client,
    f: &RemoteFile,
    dest: &Path,
    mut on_bytes: impl FnMut(u64),
) -> Result<()> {
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let part = dest.with_extension(format!(
        "{}.part",
        dest.extension().and_then(|e| e.to_str()).unwrap_or("")
    ));
    let size_of = |p: &Path| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
    if size_of(&part) > f.size {
        tokio::fs::remove_file(&part).await.ok();
    }
    if f.sources.is_empty() {
        bail!("no download source is enabled");
    }
    // Each source gets two tries; a break on one resumes from the bytes already on disk.
    let mut errors = Vec::new();
    for src in f.sources.iter().chain(f.sources.iter()) {
        if size_of(&part) < f.size {
            if let Err(e) = fetch_from(client, src, &part, f.size, &mut on_bytes).await {
                errors.push(format!("{}: {e:#}", src.name));
                continue;
            }
        }
        if let Some(want) = &f.sha256 {
            let got = sha256_file(&part).await?;
            if &got != want {
                tokio::fs::remove_file(&part).await.ok();
                errors.push(format!("{}: the file did not match its checksum", src.name));
                continue;
            }
        }
        tokio::fs::rename(&part, dest).await?;
        return Ok(());
    }
    errors.dedup();
    bail!("{}", errors.join("; "))
}

/// Append the rest of the file to `part` from one source, resuming at its current length.
async fn fetch_from(
    client: &reqwest::Client,
    src: &Source,
    part: &Path,
    size: u64,
    on_bytes: &mut impl FnMut(u64),
) -> Result<()> {
    let mut file = tokio::fs::OpenOptions::new().create(true).append(true).open(part).await?;
    let mut have = file.metadata().await?.len();
    let result = async {
        let mut start = 0u64;
        for seg in &src.parts {
            let end = start + seg.size;
            if have < end {
                let offset = have - start;
                let mut req = client.get(&seg.url);
                if offset > 0 {
                    req = req.header(reqwest::header::RANGE, format!("bytes={offset}-"));
                }
                let resp = tokio::time::timeout(STALL, req.send())
                    .await
                    .map_err(|_| anyhow!("no answer in {}s", STALL.as_secs()))??
                    .error_for_status()?;
                if offset > 0 && resp.status() != reqwest::StatusCode::PARTIAL_CONTENT {
                    if start > 0 {
                        bail!("the server can't resume a download");
                    }
                    file.set_len(0).await?; // the server ignored the range: start over
                    have = 0;
                }
                let mut stream = resp.bytes_stream();
                while let Some(chunk) = tokio::time::timeout(STALL, stream.next())
                    .await
                    .map_err(|_| anyhow!("the download stalled for {}s", STALL.as_secs()))?
                {
                    let chunk = chunk?;
                    if have + chunk.len() as u64 > end {
                        bail!("the server sent more bytes than expected");
                    }
                    file.write_all(&chunk).await?;
                    have += chunk.len() as u64;
                    on_bytes(have);
                }
                if have != end {
                    bail!("the connection closed at {have} of {size} bytes");
                }
            }
            start = end;
        }
        if have != size {
            bail!("got {have} of {size} bytes");
        }
        Ok(())
    }
    .await;
    file.flush().await?;
    result
}

/// Download the CUDA engine zip (resumable, checksummed) and unpack it into `cuda_engine_dir()`.
async fn download_cuda_engine(client: &reqwest::Client, mut on_progress: impl FnMut(Progress)) -> Result<()> {
    let pack = Pack::GpuEngine;
    let f = plan(client, pack).await?.remove(0);
    let engines = crate::paths::engines_dir();
    tokio::fs::create_dir_all(&engines).await?;
    let zip = engines.join(&f.rel);
    let have = zip.metadata().map(|m| m.len()).unwrap_or(0);
    if have != f.size {
        fetch_one(client, &f, &zip, |n| {
            on_progress(Progress { pack, file: f.rel.clone(), done_bytes: n, total_bytes: f.size })
        })
        .await
        .context("downloading the NVIDIA engine")?;
    }
    on_progress(Progress { pack, file: f.rel.clone(), done_bytes: f.size, total_bytes: f.size });
    let dest = cuda_engine_dir();
    let tmp = engines.join(format!("{CUDA_ENGINE_ID}.unpacking"));
    let (zip2, tmp2) = (zip.clone(), tmp.clone());
    tokio::task::spawn_blocking(move || unzip(&zip2, &tmp2)).await?.context("unpacking the NVIDIA engine")?;
    if dest.exists() {
        tokio::fs::remove_dir_all(&dest).await.ok();
    }
    tokio::fs::rename(&tmp, &dest).await?;
    tokio::fs::remove_file(&zip).await.ok();
    Ok(())
}

/// Unpack `zip` into a fresh `dest`, refusing entries that would land outside it.
fn unzip(zip: &Path, dest: &Path) -> Result<()> {
    if dest.exists() {
        std::fs::remove_dir_all(dest)?;
    }
    std::fs::create_dir_all(dest)?;
    let mut archive = zip::ZipArchive::new(std::fs::File::open(zip)?)?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        // Archives made by PowerShell may use backslashes.
        let name = entry.name().replace('\\', "/");
        let rel: PathBuf = name
            .split('/')
            .filter(|c| !c.is_empty() && *c != ".")
            .map(|c| if c == ".." || c.contains(':') { None } else { Some(c) })
            .collect::<Option<PathBuf>>()
            .with_context(|| format!("unsafe path in archive: {name}"))?;
        if rel.as_os_str().is_empty() {
            continue;
        }
        let out = dest.join(&rel);
        if entry.is_dir() || name.ends_with('/') {
            std::fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::File::create(&out)?;
        std::io::copy(&mut entry, &mut file)?;
    }
    Ok(())
}

async fn sha256_file(p: &Path) -> Result<String> {
    let p = p.to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<String> {
        use std::io::Read;
        let mut file = std::fs::File::open(&p)?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 1 << 20];
        loop {
            let n = file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        Ok(hex::encode(hasher.finalize()))
    })
    .await?
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}
