//! How well the drift check's voiceprint match works by line length. Needs Booth's engine.
//!
//!   cargo run -p booth-core --release --example drift_lengths -- C:\tmp\drift [seeds] [voice,voice,...] [check]
//!
//! With `check` the takes are made with the drift check on, as the app makes them, and the CSV
//! also has what the check did (match, redos, flagged).
//!
//! For each voice, line and seed it makes one Exact take and scores it against
//! its own voice ("own") and against every other voice in the run ("other", a stand-in for a take
//! that sounds like someone else), both as made and repeated to at least 6 s ("tiled").
//! Writes results.csv and every take's WAV to the folder.
use std::io::Cursor;

use booth_core::{drift, engine, studio, voices};

const LINES: &[&str] = &[
    "Okay.",
    "It worked.",
    "Let's hear it.",
    "Then we waited.",
    "Honestly, the cheapest one surprised me.",
    "Alright, let's hear what it sounds like.",
    "Okay, that's actually really good for the price.",
    "Welcome back. Today we're testing three cheap microphones, all under thirty dollars.",
    "We set it up on the desk, plugged it in, and to be honest, none of us expected much from it at all.",
];

fn tiled(wav: &[u8], min_secs: f64) -> Vec<u8> {
    let mut r = hound::WavReader::new(Cursor::new(wav)).unwrap();
    let spec = r.spec();
    let s: Vec<i16> = r.samples::<i16>().map(|x| x.unwrap()).collect();
    let gap = vec![0i16; spec.sample_rate as usize * 3 / 10];
    let mut out: Vec<i16> = Vec::new();
    while (out.len() as f64) < min_secs * spec.sample_rate as f64 {
        out.extend_from_slice(&s);
        out.extend_from_slice(&gap);
    }
    let mut buf = Cursor::new(Vec::new());
    let mut w = hound::WavWriter::new(&mut buf, spec).unwrap();
    for x in out {
        w.write_sample(x).unwrap();
    }
    w.finalize().unwrap();
    buf.into_inner()
}

async fn score(z: &booth_core::api::Zonos, wav: &[u8], prints: &[(String, Vec<f32>)]) -> Vec<f64> {
    let e = drift::parse_npy_f32(&z.embed_clip(wav, "take.wav").await.unwrap()).unwrap();
    prints.iter().map(|(_, p)| drift::cosine(p, &e)).collect()
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let dir = std::path::PathBuf::from(args.next().expect("give a folder"));
    let seeds: u64 = args.next().map(|s| s.parse().unwrap()).unwrap_or(6);
    let names: Vec<String> = args.next().unwrap_or("Rupert,Cillian,Imogen".into()).split(',').map(String::from).collect();
    let check = args.next().is_some_and(|a| a == "check");
    std::fs::create_dir_all(&dir)?;
    let z = engine::ensure("drift-lengths").await?;
    let all = voices::all(Some(&z)).await;
    let mut chosen = Vec::new();
    for n in &names {
        match all.iter().find(|v| v.name.eq_ignore_ascii_case(n)) {
            Some(v) => chosen.push(v.clone()),
            None => eprintln!("no voice {n}; have {:?}", all.iter().map(|v| &v.name).collect::<Vec<_>>()),
        }
    }
    let mut prints = Vec::new();
    for v in &chosen {
        prints.push((v.name.clone(), drift::voiceprint(&z, v).await.expect("voiceprint")));
    }
    let mut csv = String::from("voice,line,seed,seconds,own,other_max,own_tiled,other_tiled_max,checked_match,redone,drifted,file\n");
    for (vi, v) in chosen.iter().enumerate() {
        for (li, text) in LINES.iter().enumerate() {
            for seed in 1..=seeds {
                let opts = studio::SpeakOptions { voice: Some(v.id.clone()), seed: Some(seed), via: "test".into(), drift_check: check, ..Default::default() };
                let (take, wav) = studio::speak_audio(&z, text, &opts).await?;
                let file = format!("{}-l{li}-s{seed}.wav", v.name);
                std::fs::write(dir.join(&file), &wav)?;
                let raw = score(&z, &wav, &prints).await;
                let til = score(&z, &tiled(&wav, 6.0), &prints).await;
                let other = |s: &[f64]| s.iter().enumerate().filter(|(i, _)| *i != vi).map(|(_, x)| *x).fold(f64::MIN, f64::max);
                let row = format!(
                    "{},{li},{seed},{:.2},{:.3},{:.3},{:.3},{:.3},{},{},{},{file}\n",
                    v.name, take.seconds, raw[vi], other(&raw), til[vi], other(&til),
                    take.voice_match.map(|m| format!("{m:.3}")).unwrap_or_default(), take.redone, take.drifted
                );
                eprint!("{row}");
                csv.push_str(&row);
            }
        }
    }
    std::fs::write(dir.join("results.csv"), csv)?;
    Ok(())
}
