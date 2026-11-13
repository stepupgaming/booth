//! The model files Booth needs, grouped into packs, and a resumable, checksummed downloader.
//!
//! Weights come from the Hugging Face repo `Zyphra/ZONOS3-Mini` (Apache-2.0). Sizes and SHA-256
//! checksums are read from the Hub's tree API at download time, so a re-upload upstream never
//! leaves Booth with a stale list.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

pub const HF_REPO: &str = "Zyphra/ZONOS3-Mini";
const HF_REVISION: &str = "main";

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
    /// SHA-256 for LFS files (the big ones); small git files have none.
    pub sha256: Option<String>,
    /// Where to fetch it; None = the file of that name in the Hugging Face repo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Deserialize)]
struct TreeEntry {
    #[serde(rename = "type")]
    kind: String,
    path: String,
    size: u64,
    lfs: Option<TreeLfs>,
}

#[derive(Deserialize)]
struct TreeLfs {
    oid: String,
}

async fn tree(client: &reqwest::Client, dir: &str) -> Result<Vec<TreeEntry>> {
    let url = format!(
        "https://huggingface.co/api/models/{HF_REPO}/tree/{HF_REVISION}/gguf/{dir}?recursive=1"
    );
    let entries: Vec<TreeEntry> = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("listing {url}"))?
        .error_for_status()?
        .json()
        .await?;
    Ok(entries)
}

/// The files of `pack` with their sizes and checksums, from the Hub.
pub async fn plan(client: &reqwest::Client, pack: Pack) -> Result<Vec<RemoteFile>> {
    if pack == Pack::GpuEngine {
        return Ok(vec![RemoteFile {
            rel: "booth-engine-cuda.zip".into(),
            size: CUDA_ENGINE_SIZE,
            sha256: Some(CUDA_ENGINE_SHA256.into()),
            url: Some(CUDA_ENGINE_URL.into()),
        }]);
    }
    let top = tree(client, "").await?;
    let mut out = Vec::new();
    for f in pack.files() {
        if let Some(dir) = f.strip_suffix('/') {
            for e in tree(client, dir).await? {
                if e.kind == "file" {
                    out.push(remote(e));
                }
            }
        } else {
            let want = format!("gguf/{f}");
            let e = top
                .iter()
                .find(|e| e.path == want)
                .with_context(|| format!("{want} is not in {HF_REPO}"))?;
            out.push(RemoteFile {
                rel: f.to_string(),
                size: e.size,
                sha256: e.lfs.as_ref().map(|l| l.oid.clone()),
                url: None,
            });
        }
    }
    Ok(out)
}

fn remote(e: TreeEntry) -> RemoteFile {
    RemoteFile {
        rel: e.path.trim_start_matches("gguf/").to_string(),
        size: e.size,
        sha256: e.lfs.map(|l| l.oid),
        url: None,
    }
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
/// and checking SHA-256 where the Hub gives one.
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
    let mut have = tokio::fs::metadata(&part).await.map(|m| m.len()).unwrap_or(0);
    if have > f.size {
        tokio::fs::remove_file(&part).await.ok();
        have = 0;
    }
    let url = f.url.clone().unwrap_or_else(|| {
        format!(
            "https://huggingface.co/{HF_REPO}/resolve/{HF_REVISION}/gguf/{}",
            f.rel.split('/').map(urlencode).collect::<Vec<_>>().join("/")
        )
    });
    if have < f.size {
        let mut req = client.get(&url);
        if have > 0 {
            req = req.header(reqwest::header::RANGE, format!("bytes={have}-"));
        }
        let resp = req.send().await?.error_for_status()?;
        if have > 0 && resp.status() != reqwest::StatusCode::PARTIAL_CONTENT {
            have = 0; // the server ignored the range: start over
        }
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .append(have > 0)
            .truncate(have == 0)
            .open(&part)
            .await?;
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            file.write_all(&chunk).await?;
            have += chunk.len() as u64;
            on_bytes(have);
        }
        file.flush().await?;
    }
    if have != f.size {
        bail!("got {have} of {} bytes", f.size);
    }
    if let Some(want) = &f.sha256 {
        let got = sha256_file(&part).await?;
        if &got != want {
            tokio::fs::remove_file(&part).await.ok();
            bail!("checksum mismatch (expected {want}, got {got}); the partial file was removed, try again");
        }
    }
    tokio::fs::rename(&part, dest).await?;
    Ok(())
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
