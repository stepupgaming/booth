//! A stand-in for the one ffmpeg command the voice engine runs, for platforms without ffmpeg
//! (Android). The engine decodes a cloning clip with
//!
//! ```text
//! ffmpeg -nostdin -v error -i <clip> -ac 1 -ar <rate> -t <seconds> -f f32le -
//! ```
//!
//! and reads mono 32-bit float samples from stdout. This program takes the same arguments
//! (others are ignored), decodes wav, mp3, m4a/aac, flac, ogg/vorbis, aiff, caf and mkv/webm
//! (not Opus) with Symphonia, mixes to mono, resamples and writes the same output.
//! Booth points the engine at it with `ZONOS3_FFMPEG`.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::process::ExitCode;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

struct Args {
    input: String,
    rate: u32,
    seconds: Option<f64>,
}

fn parse() -> Result<Args, String> {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let value = |flag: &str| argv.iter().position(|a| a == flag).and_then(|i| argv.get(i + 1)).cloned();
    let input = value("-i").ok_or("no input (-i)")?;
    let rate = value("-ar").ok_or("no output rate (-ar)")?.parse().map_err(|_| "bad -ar")?;
    let seconds = value("-t").and_then(|t| t.parse().ok());
    Ok(Args { input, rate, seconds })
}

/// The clip as mono samples at its own rate, stopping once `max_seconds` are decoded.
fn decode(path: &str, max_seconds: Option<f64>) -> Result<(Vec<f32>, u32), String> {
    let file = File::open(path).map_err(|e| format!("{path}: {e}"))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = Path::new(path).extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions { enable_gapless: true, ..Default::default() }, &MetadataOptions::default())
        .map_err(|e| format!("unknown audio format: {e}"))?;
    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or("no audio track")?;
    let track_id = track.id;
    let mut rate = track.codec_params.sample_rate.unwrap_or(0);
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| format!("unsupported codec: {e}"))?;

    let mut mono = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(Error::ResetRequired) => break,
            Err(e) => return Err(format!("reading the clip: {e}")),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let buf = match decoder.decode(&packet) {
            Ok(b) => b,
            Err(Error::DecodeError(_)) => continue, // a damaged packet: skip it, like ffmpeg
            Err(e) => return Err(format!("decoding the clip: {e}")),
        };
        let spec = *buf.spec();
        rate = spec.rate;
        let channels = spec.channels.count().max(1);
        let mut samples = SampleBuffer::<f32>::new(buf.capacity() as u64, spec);
        samples.copy_interleaved_ref(buf);
        mono.extend(samples.samples().chunks(channels).map(|f| f.iter().sum::<f32>() / channels as f32));
        if let Some(max) = max_seconds {
            if rate > 0 && mono.len() as f64 >= max * rate as f64 {
                mono.truncate((max * rate as f64) as usize);
                break;
            }
        }
    }
    if rate == 0 || mono.is_empty() {
        return Err("the clip has no audio".into());
    }
    Ok((mono, rate))
}

/// Windowed-sinc resampling (Blackman window, 32 zero crossings), low-passed below the
/// lower of the two Nyquist rates.
fn resample(x: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to {
        return x.to_vec();
    }
    let step = from as f64 / to as f64;
    let cutoff = (to as f64 / from as f64).min(1.0) * 0.97;
    let half = (32.0 / cutoff).ceil() as i64;
    let n_out = (x.len() as f64 / step).floor() as usize;
    let mut out = Vec::with_capacity(n_out);
    for i in 0..n_out {
        let t = i as f64 * step;
        let centre = t.floor() as i64;
        let mut acc = 0.0f64;
        for k in (centre - half + 1)..=(centre + half) {
            if k < 0 || k as usize >= x.len() {
                continue;
            }
            let d = t - k as f64;
            let w = d / half as f64; // -1..1
            if w.abs() >= 1.0 {
                continue;
            }
            let window = 0.42 + 0.5 * (std::f64::consts::PI * w).cos() + 0.08 * (2.0 * std::f64::consts::PI * w).cos();
            let a = std::f64::consts::PI * cutoff * d;
            let sinc = if a.abs() < 1e-9 { 1.0 } else { a.sin() / a };
            acc += x[k as usize] as f64 * cutoff * sinc * window;
        }
        out.push(acc as f32);
    }
    out
}

fn main() -> ExitCode {
    let run = || -> Result<(), String> {
        let args = parse()?;
        let (mono, rate) = decode(&args.input, args.seconds)?;
        let out = resample(&mono, rate, args.rate);
        let mut w = BufWriter::new(std::io::stdout().lock());
        for s in out {
            w.write_all(&s.to_le_bytes()).map_err(|e| e.to_string())?;
        }
        w.flush().map_err(|e| e.to_string())
    };
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("booth-decode: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::resample;

    #[test]
    fn resampling_keeps_a_tone() {
        let from = 48_000;
        let to = 16_000;
        let tone: Vec<f32> = (0..from).map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / from as f32).sin()).collect();
        let out = resample(&tone, from, to);
        assert_eq!(out.len(), to as usize);
        // compare the middle against the ideal tone at the new rate
        let err: f32 = (2000..14000)
            .map(|i| (out[i] - (2.0 * std::f32::consts::PI * 440.0 * i as f32 / to as f32).sin()).abs())
            .fold(0.0, f32::max);
        assert!(err < 0.01, "max error {err}");
    }
}
