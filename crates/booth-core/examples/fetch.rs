//! Download one pack into a folder with Booth's downloader, for testing the sources:
//!
//!   cargo run -p booth-core --example fetch -- core C:\tmp\models
//!   BOOTH_MODEL_SOURCE=github cargo run -p booth-core --example fetch -- full_decoder C:\tmp\models
//!   BOOTH_HF_BASE=http://127.0.0.1:9 cargo run ...   (Hugging Face unreachable: falls back to GitHub)
//!   cargo run -p booth-core --example fetch -- refresh C:\tmp\models   (the pre-start small-file refresh)
use std::time::Instant;

use booth_core::models::{self, Pack};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    if std::env::args().nth(1).as_deref() == Some("refresh") {
        let dir = std::path::PathBuf::from(std::env::args().nth(2).expect("give a folder"));
        let t = Instant::now();
        models::refresh_small_files(&dir).await;
        println!("refreshed in {:.1}s", t.elapsed().as_secs_f32());
        return Ok(());
    }
    let pack: Pack = serde_json::from_value(serde_json::Value::String(args.next().unwrap_or("core".into())))?;
    let dir = std::path::PathBuf::from(args.next().expect("give a folder"));
    std::fs::create_dir_all(&dir)?;
    let client = reqwest::Client::new();
    let t = Instant::now();
    let mut last = Instant::now();
    models::download(&client, &dir, pack, |p| {
        if last.elapsed().as_secs_f32() > 2.0 || p.done_bytes == p.total_bytes {
            last = Instant::now();
            eprintln!(
                "{:>6.1}s {:>5.1}% {:>7.1} MB/s  {}",
                t.elapsed().as_secs_f32(),
                p.done_bytes as f64 * 100.0 / p.total_bytes as f64,
                p.done_bytes as f64 / 1e6 / t.elapsed().as_secs_f64(),
                p.file
            );
        }
    })
    .await?;
    println!("done in {:.1}s", t.elapsed().as_secs_f32());
    Ok(())
}
