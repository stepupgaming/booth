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
}

impl Pack {
    pub const ALL: [Pack; 3] = [Pack::Core, Pack::Exact, Pack::FullDecoder];

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
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Pack::Core => "Voice engine",
            Pack::Exact => "Exact cloning",
            Pack::FullDecoder => "Studio decoder",
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
    let url = format!(
        "https://huggingface.co/{HF_REPO}/resolve/{HF_REVISION}/gguf/{}",
        f.rel.split('/').map(urlencode).collect::<Vec<_>>().join("/")
    );
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
