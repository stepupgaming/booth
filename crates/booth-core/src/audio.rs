//! Writing a voiceover to disk a line at a time, as WAV or MP3, so a two-hour script never sits
//! in memory. Optionally evens out loudness: every line to the same level (EBU R128), the whole
//! voiceover to YouTube's -14 LUFS, and a look-ahead limiter that keeps true peaks under -1 dBTP.

use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use ebur128::{EbuR128, Mode};
use serde::{Deserialize, Serialize};

/// The voiceover's file type.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    #[default]
    Wav,
    Mp3,
}

impl Format {
    pub fn ext(self) -> &'static str {
        match self {
            Format::Wav => "wav",
            Format::Mp3 => "mp3",
        }
    }
}

/// MP3 bitrate: constant (video editors keep sync with CBR), and plenty for one voice.
pub const MP3_KBPS: u32 = 128;
/// Where "Even out loudness" puts the whole voiceover (YouTube's level).
pub const TARGET_LUFS: f64 = -14.0;
/// The true-peak ceiling the limiter keeps to, in dBTP.
pub const CEILING_DBTP: f64 = -1.0;
/// The limiter aims a little under the ceiling: its peak estimate and EBU R128's differ slightly.
const LIMIT_DBTP: f64 = CEILING_DBTP - 0.3;
/// The most a line is raised or lowered to sound like the others.
const LINE_RANGE_DB: f64 = 6.0;
/// Lines shorter than this keep their level: too short to measure (a "Yes." or a name).
const LINE_MIN_SECS: f64 = 1.0;

/// One line's audio, as 16-bit samples (interleaved when there is more than one channel).
pub struct Pcm {
    pub rate: u32,
    pub channels: u16,
    pub samples: Vec<i16>,
}

impl Pcm {
    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels.max(1) as usize
    }
    pub fn seconds(&self) -> f64 {
        self.frames() as f64 / self.rate.max(1) as f64
    }
}

/// Read a WAV as 16-bit samples (deeper or float files are brought down to 16 bits).
pub fn read_wav(path: &Path) -> Result<Pcm> {
    let mut r = hound::WavReader::open(path).with_context(|| format!("reading {}", path.display()))?;
    let s = r.spec();
    let samples: Vec<i16> = match s.sample_format {
        hound::SampleFormat::Int if s.bits_per_sample <= 16 => r.samples::<i16>().filter_map(|x| x.ok()).collect(),
        hound::SampleFormat::Int => {
            let shift = s.bits_per_sample - 16;
            r.samples::<i32>().filter_map(|x| x.ok()).map(|x| (x >> shift) as i16).collect()
        }
        hound::SampleFormat::Float => {
            r.samples::<f32>().filter_map(|x| x.ok()).map(|x| (x.clamp(-1.0, 1.0) * 32767.0) as i16).collect()
        }
    };
    Ok(Pcm { rate: s.sample_rate, channels: s.channels.max(1), samples })
}

fn mp3_err(e: rusty_mp3::Error) -> anyhow::Error {
    anyhow!("MP3: {e}")
}

/// An MP3 written as it is encoded (rusty_mp3, pure Rust), with a LAME/Info tag in front so
/// players know the exact length and trim the encoder's delay.
///
/// rusty_mp3's own stream encoder keeps every frame of an MPEG-1 stream (44.1 kHz, Booth's rate)
/// in memory until the end, to share bits between frames (the bit reservoir). This drives its
/// frame encoder directly and lets the reservoir out every [`Mp3Out::BANK`] frames, so memory
/// stays bounded and a quiet or easy frame still lends bits to a hard one nearby.
pub struct Mp3Out {
    enc: Box<rusty_mp3::Mp3Encode>,
    header: rusty_mp3::header::FrameHeader,
    tag_header: rusty_mp3::header::FrameHeader,
    stats: rusty_mp3::encode::lametag::TagStats,
    /// Input not yet encoded, per channel.
    pcm: Vec<Vec<f32>>,
    /// Samples per channel taken in.
    input: u64,
    reservoir: bool,
    banked: usize,
    file: BufWriter<File>,
    tag_len: usize,
}

impl Mp3Out {
    /// Frames held for the bit reservoir before they are written (about 27 s at 44.1 kHz).
    const BANK: usize = 1024;

    fn create(path: &Path, rate: u32, channels: u16) -> Result<Self> {
        use rusty_mp3::encode::lametag;
        let header = rusty_mp3::encoder_header(rate, channels, MP3_KBPS).map_err(mp3_err)?;
        let tag_header = lametag::tag_header(&header);
        let stats = lametag::TagStats::new(tag_header.frame_size());
        let mut file = BufWriter::new(File::create(path).with_context(|| format!("writing {}", path.display()))?);
        // A placeholder tag of the final length; `finish` writes the real one over it.
        let info = lametag::TagInfo { vbr: false, delay: lametag::ENCODER_DELAY, padding: 0 };
        let tag = lametag::tag_frame(&tag_header, &stats, &info);
        file.write_all(&tag)?;
        Ok(Self {
            enc: Box::new(rusty_mp3::Mp3Encode::new()),
            reservoir: header.version == rusty_mp3::header::MpegVersion::V1,
            pcm: vec![Vec::new(); header.channel_mode.channels()],
            header,
            tag_header,
            stats,
            input: 0,
            banked: 0,
            file,
            tag_len: tag.len(),
        })
    }

    fn put(&mut self, frame: &[u8]) -> Result<()> {
        self.stats.add_frame(frame);
        self.file.write_all(frame)?;
        Ok(())
    }

    fn let_out_bank(&mut self) -> Result<()> {
        if self.banked == 0 {
            return Ok(());
        }
        // Every frame comes back at the stream's fixed frame size.
        let stream = self.enc.finish_reservoir();
        let size = self.header.frame_size();
        for f in stream.chunks(size) {
            self.put(f)?;
        }
        self.banked = 0;
        Ok(())
    }

    /// Encode every whole frame waiting.
    fn encode_ready(&mut self) -> Result<()> {
        let spf = self.header.version.samples_per_frame();
        let mut off = 0;
        while self.pcm[0].len() - off >= spf {
            let block: Vec<&[f32]> = self.pcm.iter().map(|c| &c[off..off + spf]).collect();
            if self.reservoir {
                self.enc.encode_frame_reservoir(&self.header, &block, 0.2);
                self.banked += 1;
            } else {
                let bytes = self.enc.encode_frame(&self.header, &block, None).map_err(mp3_err)?;
                self.put(&bytes)?;
            }
            off += spf;
            if self.banked >= Self::BANK {
                for c in &mut self.pcm {
                    c.drain(..off);
                }
                off = 0;
                self.let_out_bank()?;
            }
        }
        for c in &mut self.pcm {
            c.drain(..off);
        }
        Ok(())
    }

    fn write(&mut self, samples: &[i16], channels: u16) -> Result<()> {
        let ch = channels.max(1) as usize;
        for frame in samples.chunks_exact(ch) {
            for (c, dst) in self.pcm.iter_mut().enumerate() {
                dst.push(frame[c.min(ch - 1)] as f32 / 32768.0);
            }
        }
        self.input += (samples.len() / ch) as u64;
        self.encode_ready()
    }

    fn finish(mut self) -> Result<()> {
        use rusty_mp3::encode::lametag;
        let spf = self.header.version.samples_per_frame();
        // Pad the end with silence so the decoder's delay and the last sample both fit, the
        // way rusty_mp3's own encoder does.
        let mut prime = 0;
        if self.input < spf as u64 {
            prime = lametag::extra_priming(self.input, spf);
            for c in &mut self.pcm {
                c.splice(0..0, std::iter::repeat_n(0.0, prime));
            }
        }
        let delay = lametag::ENCODER_DELAY + prime;
        let total = lametag::frames_for(self.input, delay, spf) * spf as u64;
        let pad = (total - self.input) as usize - prime;
        for c in &mut self.pcm {
            c.resize(c.len() + pad, 0.0);
        }
        self.encode_ready()?;
        self.let_out_bank()?;
        let info = lametag::TagInfo { vbr: false, delay, padding: lametag::padding_for(self.input, delay, spf) as usize };
        let tag = lametag::tag_frame(&self.tag_header, &self.stats, &info);
        if tag.len() == self.tag_len {
            self.file.seek(SeekFrom::Start(0))?;
            self.file.write_all(&tag)?;
        }
        self.file.flush()?;
        Ok(())
    }
}

/// An audio file being written, a block of samples at a time.
pub enum AudioFile {
    Wav(hound::WavWriter<BufWriter<File>>),
    Mp3(Box<Mp3Out>, u16),
}

impl AudioFile {
    pub fn create(path: &Path, format: Format, rate: u32, channels: u16) -> Result<Self> {
        Ok(match format {
            Format::Wav => {
                let spec =
                    hound::WavSpec { channels, sample_rate: rate, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
                // hound writes the header with a placeholder size and fills it in on finalize.
                AudioFile::Wav(hound::WavWriter::create(path, spec)?)
            }
            Format::Mp3 => AudioFile::Mp3(Box::new(Mp3Out::create(path, rate, channels)?), channels),
        })
    }

    pub fn write(&mut self, samples: &[i16]) -> Result<()> {
        match self {
            AudioFile::Wav(w) => {
                let mut w = w.get_i16_writer(samples.len() as u32);
                for s in samples {
                    w.write_sample(*s);
                }
                w.flush()?;
            }
            AudioFile::Mp3(m, channels) => m.write(samples, *channels)?,
        }
        Ok(())
    }

    pub fn finish(self) -> Result<()> {
        match self {
            AudioFile::Wav(w) => w.finalize()?,
            AudioFile::Mp3(m, _) => m.finish()?,
        }
        Ok(())
    }
}

fn db_to_gain(db: f64) -> f32 {
    10f64.powf(db / 20.0) as f32
}

fn integrated(meter: &EbuR128) -> Option<f64> {
    meter.loudness_global().ok().filter(|l| l.is_finite() && *l > -70.0)
}

/// How much to turn each line up or down (in dB) so the lines sound alike and the whole
/// voiceover lands on [`TARGET_LUFS`]. Reads every line three times, one at a time: to measure
/// each line, to measure the voiceover with the lines evened out (pauses included), and once
/// more through the limiter to make up what it takes off.
pub fn plan_gains(paths: &[PathBuf], gaps_ms: &[u32]) -> Result<Vec<f64>> {
    let mut levels = Vec::with_capacity(paths.len());
    for p in paths {
        let pcm = read_wav(p)?;
        let level = if pcm.seconds() >= LINE_MIN_SECS {
            let mut m = EbuR128::new(pcm.channels as u32, pcm.rate, Mode::I)?;
            m.add_frames_i16(&pcm.samples)?;
            integrated(&m)
        } else {
            None
        };
        levels.push(level);
    }
    let mut known: Vec<f64> = levels.iter().flatten().copied().collect();
    known.sort_by(f64::total_cmp);
    let mut gains: Vec<f64> = match known.get(known.len() / 2) {
        Some(mid) => levels.iter().map(|l| l.map_or(0.0, |l| (mid - l).clamp(-LINE_RANGE_DB, LINE_RANGE_DB))).collect(),
        None => vec![0.0; paths.len()],
    };
    let mut meter: Option<EbuR128> = None;
    for (i, p) in paths.iter().enumerate() {
        let pcm = read_wav(p)?;
        let m = match &mut meter {
            Some(m) => m,
            None => meter.insert(EbuR128::new(pcm.channels as u32, pcm.rate, Mode::I | Mode::HISTOGRAM)?),
        };
        if i > 0 {
            let frames = (pcm.rate as u64 * gaps_ms.get(i).copied().unwrap_or(0) as u64 / 1000) as usize;
            m.add_frames_f32(&vec![0.0; frames * pcm.channels as usize])?;
        }
        let g = db_to_gain(gains[i]);
        let block: Vec<f32> = pcm.samples.iter().map(|s| *s as f32 / 32768.0 * g).collect();
        m.add_frames_f32(&block)?;
    }
    if let Some(level) = meter.as_ref().and_then(integrated) {
        for g in &mut gains {
            *g += TARGET_LUFS - level;
        }
        // The limiter takes the tops off the loudest peaks, which leaves the voiceover a little
        // quieter than planned. A third read runs it through the limiter and makes that up.
        let mut join = Joiner::new(Vec::new(), true);
        for (i, p) in paths.iter().enumerate() {
            let pcm = read_wav(p)?;
            join.pause(gaps_ms.get(i).copied().unwrap_or(0))?;
            join.line(&pcm, gains[i])?;
        }
        if let (_, Some(l)) = join.finish()? {
            let short = (TARGET_LUFS - l.lufs).clamp(0.0, 2.0);
            for g in &mut gains {
                *g += short;
            }
        }
    }
    Ok(gains)
}

/// A look-ahead peak limiter: it turns the audio down just before a peak would pass the ceiling
/// and back up over a short release. Peaks are measured between samples too (4x, Lanczos), so
/// the result stays under the ceiling as EBU R128 measures true peak. Audio comes out delayed by
/// [`Limiter::delay`] frames; `flush` lets the rest out.
pub struct Limiter {
    ch: usize,
    ceiling: f32,
    look: usize,
    release: f32,
    /// Input frames not yet out, interleaved; `base` is the number of the first.
    buf: VecDeque<f32>,
    base: u64,
    /// Frames taken in.
    taken: u64,
    /// Frames given out.
    given: u64,
    prev_peak: f32,
    /// Smallest gain needed over the last `look` frames (a falling queue of (frame, gain)).
    mins: VecDeque<(u64, f32)>,
    /// The last `look` of those minimums and their sum, to ramp the gain down smoothly.
    ramp: VecDeque<f32>,
    ramp_sum: f64,
    gain: f32,
    /// 4x interpolation weights for the points a quarter, half and three quarters between two
    /// samples, from the 5 samples before to the 6 after.
    taps: [[f32; 12]; 3],
}

impl Limiter {
    const AHEAD: u64 = 6;

    pub fn new(rate: u32, channels: u16, ceiling_dbtp: f64) -> Self {
        let look = ((rate as f64 * 0.005).round() as usize).max(8);
        let mut taps = [[0f32; 12]; 3];
        for (k, row) in taps.iter_mut().enumerate() {
            let f = (k + 1) as f64 / 4.0;
            let sinc = |x: f64| if x.abs() < 1e-9 { 1.0 } else { (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x) };
            let w: Vec<f64> = (-5..=6).map(|t| f - t as f64).map(|u| sinc(u) * sinc(u / 6.0)).collect();
            let sum: f64 = w.iter().sum();
            for (dst, v) in row.iter_mut().zip(w) {
                *dst = (v / sum) as f32;
            }
        }
        Self {
            ch: channels.max(1) as usize,
            ceiling: db_to_gain(ceiling_dbtp),
            look,
            release: (1.0 - (-1.0 / (0.06 * rate as f64)).exp()) as f32,
            buf: VecDeque::new(),
            base: 0,
            taken: 0,
            given: 0,
            prev_peak: 0.0,
            mins: VecDeque::new(),
            ramp: std::iter::repeat_n(1.0, look).collect(),
            ramp_sum: look as f64,
            gain: 1.0,
            taps,
        }
    }

    /// How many frames late the audio comes out.
    pub fn delay(&self) -> u64 {
        Self::AHEAD + self.look as u64 - 1
    }

    fn at(&self, frame: i64, c: usize) -> f32 {
        if frame < self.base as i64 {
            return 0.0;
        }
        self.buf.get((frame as u64 - self.base) as usize * self.ch + c).copied().unwrap_or(0.0)
    }

    /// Take interleaved samples (full scale = 1.0); limited samples go to `out`.
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        for frame in input.chunks_exact(self.ch) {
            self.buf.extend(frame);
            self.taken += 1;
            self.step(out, u64::MAX);
        }
    }

    /// Let out what's still inside (feeds silence through until every frame is out).
    pub fn flush(&mut self, out: &mut Vec<f32>) {
        let end = self.taken;
        while self.given < end {
            self.buf.extend(std::iter::repeat_n(0.0, self.ch));
            self.taken += 1;
            self.step(out, end);
        }
    }

    /// One frame in: work out the gain for the frame `delay` behind it and let that one out.
    fn step(&mut self, out: &mut Vec<f32>, end: u64) {
        let k = self.taken as i64 - 1;
        let j = k - Self::AHEAD as i64;
        // The highest point between frame j and j+1, on any channel.
        let mut peak = 0f32;
        for c in 0..self.ch {
            let x: [f32; 12] = std::array::from_fn(|t| self.at(j - 5 + t as i64, c));
            peak = peak.max(x[5].abs()).max(x[6].abs());
            for row in &self.taps {
                let v: f32 = row.iter().zip(&x).map(|(w, s)| w * s).sum();
                peak = peak.max(v.abs());
            }
        }
        let worst = peak.max(self.prev_peak);
        self.prev_peak = peak;
        let need = if worst > self.ceiling { self.ceiling / worst } else { 1.0 };
        let jj = j.max(0) as u64;
        while self.mins.back().is_some_and(|(_, g)| *g >= need) {
            self.mins.pop_back();
        }
        self.mins.push_back((jj, need));
        while self.mins.front().is_some_and(|(f, _)| *f + self.look as u64 <= jj) {
            self.mins.pop_front();
        }
        let min = self.mins.front().map_or(1.0, |(_, g)| *g);
        self.ramp.push_back(min);
        self.ramp_sum += min as f64;
        self.ramp_sum -= self.ramp.pop_front().unwrap_or(1.0) as f64;
        let target = (self.ramp_sum / self.look as f64) as f32;
        self.gain = target.min(self.gain + (1.0 - self.gain) * self.release);
        // Every minimum in the ramp covers frame t, so the gain is low enough for it.
        let t = j - self.look as i64 + 1;
        if t >= 0 && (t as u64) < end && t as u64 == self.given {
            for c in 0..self.ch {
                out.push(self.at(t, c) * self.gain);
            }
            self.given += 1;
        }
        // Keep what the next steps still read: frame t+1 onward and j-4 onward.
        let keep_from = (t + 1).min(j - 4).max(0) as u64;
        while self.base < keep_from && !self.buf.is_empty() {
            for _ in 0..self.ch {
                self.buf.pop_front();
            }
            self.base += 1;
        }
    }
}

fn to_i16(x: f32) -> i16 {
    (x * 32768.0).round().clamp(-32768.0, 32767.0) as i16
}

/// Loudness of a finished voiceover, measured as written.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Loudness {
    /// Integrated loudness (LUFS).
    pub lufs: f64,
    /// True peak (dBTP).
    pub true_peak: f64,
}

/// Lines and pauses in, one voiceover out: written as it goes, to any number of files (the
/// voiceover, and a WAV for the Library when the voiceover is an MP3). Without files it only
/// keeps time, for captions.
pub struct Joiner {
    outs: Vec<(PathBuf, Format)>,
    files: Vec<AudioFile>,
    even: bool,
    limiter: Option<Limiter>,
    meter: Option<EbuR128>,
    rate: u32,
    channels: u16,
    frames: u64,
    scratch: Vec<f32>,
}

impl Joiner {
    pub fn new(outs: Vec<(PathBuf, Format)>, even: bool) -> Self {
        Self { outs, files: Vec::new(), even, limiter: None, meter: None, rate: 0, channels: 0, frames: 0, scratch: Vec::new() }
    }

    /// Where the next line will start, in seconds.
    pub fn seconds(&self) -> f64 {
        if self.rate == 0 { 0.0 } else { self.frames as f64 / self.rate as f64 }
    }

    fn open(&mut self, rate: u32, channels: u16) -> Result<()> {
        if self.rate == 0 {
            self.rate = rate;
            self.channels = channels;
            for (p, f) in &self.outs {
                self.files.push(AudioFile::create(p, *f, rate, channels)?);
            }
            if self.even {
                self.limiter = Some(Limiter::new(rate, channels, LIMIT_DBTP));
                self.meter = Some(EbuR128::new(channels as u32, rate, Mode::I | Mode::HISTOGRAM | Mode::TRUE_PEAK)?);
            }
        } else if rate != self.rate || channels != self.channels {
            bail!("the lines have different audio formats");
        }
        Ok(())
    }

    fn emit(&mut self, samples: &[i16]) -> Result<()> {
        for f in &mut self.files {
            f.write(samples)?;
        }
        if let Some(m) = &mut self.meter {
            m.add_frames_i16(samples)?;
        }
        Ok(())
    }

    fn emit_limited(&mut self, input: &[f32]) -> Result<()> {
        let mut out = std::mem::take(&mut self.scratch);
        out.clear();
        if let Some(l) = &mut self.limiter {
            l.process(input, &mut out);
        }
        let samples: Vec<i16> = out.iter().map(|x| to_i16(*x)).collect();
        self.scratch = out;
        self.emit(&samples)
    }

    /// A pause, unless nothing has been written yet.
    pub fn pause(&mut self, ms: u32) -> Result<()> {
        if self.rate == 0 {
            return Ok(());
        }
        let frames = (self.rate as u64 * ms as u64 / 1000) as usize;
        let n = frames * self.channels as usize;
        for chunk in (0..n).step_by(1 << 16) {
            let len = (n - chunk).min(1 << 16);
            if self.even {
                self.emit_limited(&vec![0.0; len])?;
            } else {
                self.emit(&vec![0i16; len])?;
            }
        }
        self.frames += frames as u64;
        Ok(())
    }

    /// A line, turned up or down by `gain_db` (only when evening out loudness).
    pub fn line(&mut self, pcm: &Pcm, gain_db: f64) -> Result<()> {
        self.open(pcm.rate, pcm.channels)?;
        if self.even {
            let g = db_to_gain(gain_db);
            let input: Vec<f32> = pcm.samples.iter().map(|s| *s as f32 / 32768.0 * g).collect();
            self.emit_limited(&input)?;
        } else {
            self.emit(&pcm.samples)?;
        }
        self.frames += pcm.frames() as u64;
        Ok(())
    }

    /// Close the files. Returns the voiceover's length and, when evening out, its loudness.
    pub fn finish(mut self) -> Result<(f64, Option<Loudness>)> {
        if let Some(mut l) = self.limiter.take() {
            let mut out = Vec::new();
            l.flush(&mut out);
            let samples: Vec<i16> = out.iter().map(|x| to_i16(*x)).collect();
            self.emit(&samples)?;
        }
        let seconds = self.seconds();
        for f in self.files.drain(..) {
            f.finish()?;
        }
        let loudness = self.meter.as_ref().and_then(|m| {
            let peak = (0..self.channels as u32).filter_map(|c| m.true_peak(c).ok()).fold(0f64, f64::max);
            integrated(m).map(|lufs| Loudness { lufs, true_peak: 20.0 * peak.max(1e-9).log10() })
        });
        Ok((seconds, loudness))
    }
}

/// One line on its own, as `format`, turned by `gain_db` when given (with its own limiter). A
/// WAV left as it was is copied as is.
pub fn write_line(src: &Path, dst: &Path, format: Format, gain_db: Option<f64>) -> Result<()> {
    if format == Format::Wav && gain_db.is_none() {
        std::fs::copy(src, dst).with_context(|| format!("writing {}", dst.display()))?;
        return Ok(());
    }
    let pcm = read_wav(src)?;
    let mut j = Joiner::new(vec![(dst.to_path_buf(), format)], gain_db.is_some());
    j.line(&pcm, gain_db.unwrap_or(0.0))?;
    j.finish()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Booth's engine speaks at 44.1 kHz mono.
    const RATE: u32 = 44_100;

    fn sine(path: &Path, secs: f64, amp: f32, hz: f32, rate: u32) {
        let spec = hound::WavSpec { channels: 1, sample_rate: rate, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
        let mut w = hound::WavWriter::create(path, spec).unwrap();
        for i in 0..(secs * rate as f64) as usize {
            let t = i as f32 / rate as f32;
            w.write_sample((amp * (std::f32::consts::TAU * hz * t).sin() * 32767.0) as i16).unwrap();
        }
        w.finalize().unwrap();
    }

    fn measure(path: &Path) -> (f64, f64) {
        let pcm = read_wav(path).unwrap();
        let mut m = EbuR128::new(1, pcm.rate, Mode::I | Mode::TRUE_PEAK).unwrap();
        m.add_frames_i16(&pcm.samples).unwrap();
        (m.loudness_global().unwrap(), 20.0 * m.true_peak(0).unwrap().log10())
    }

    #[test]
    fn limiter_keeps_length_and_stays_under_the_ceiling() {
        let mut l = Limiter::new(RATE, 1, -1.3);
        let input: Vec<f32> = (0..24_000).map(|i| ((i as f32) * 0.3).sin() * if i % 4000 >= 3700 { 1.6 } else { 0.4 }).collect();
        let mut out = Vec::new();
        for c in input.chunks(1000) {
            l.process(c, &mut out);
        }
        l.flush(&mut out);
        assert_eq!(out.len(), input.len());
        assert!(out.iter().all(|x| x.abs() <= db_to_gain(-1.3) * 1.001));
        // quiet parts pass through untouched (same samples, same place)
        assert!((out[1000] - input[1000]).abs() < 1e-6);
    }

    #[test]
    fn evening_out_brings_quiet_and_loud_lines_together() {
        let dir = std::env::temp_dir().join(format!("booth-audio-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let quiet = dir.join("quiet.wav");
        let loud = dir.join("loud.wav");
        let short = dir.join("short.wav");
        sine(&quiet, 3.0, 0.05, 220.0, RATE);
        sine(&loud, 3.0, 0.9, 330.0, RATE);
        sine(&short, 0.4, 0.9, 440.0, RATE);
        let paths = vec![quiet.clone(), loud.clone(), short.clone()];
        let gains = plan_gains(&paths, &[0, 250, 250]).unwrap();
        // the quiet line is raised to meet the loud one, at most 6 dB; the short one only moves with the whole
        assert!((gains[0] - gains[1] - LINE_RANGE_DB).abs() < 1e-9, "{gains:?}");
        assert_eq!(gains[2], gains[1]);
        for fmt in [Format::Wav, Format::Mp3] {
            let out = dir.join(format!("out.{}", fmt.ext()));
            let mut j = Joiner::new(vec![(out.clone(), fmt)], true);
            for (i, p) in paths.iter().enumerate() {
                j.pause(if i == 0 { 0 } else { 250 }).unwrap();
                j.line(&read_wav(p).unwrap(), gains[i]).unwrap();
            }
            let (secs, loud) = j.finish().unwrap();
            assert!((secs - 6.9).abs() < 0.01, "{secs}");
            let loud = loud.unwrap();
            assert!(loud.true_peak <= CEILING_DBTP + 0.05, "{loud:?}");
            assert!(out.metadata().unwrap().len() > 1000);
            if fmt == Format::Wav {
                let (lufs, tp) = measure(&out);
                assert!((lufs - TARGET_LUFS).abs() < 1.0, "{lufs}");
                assert!(tp <= CEILING_DBTP + 0.05, "{tp}");
                assert_eq!(read_wav(&out).unwrap().frames(), (6.4 * RATE as f64) as usize + 2 * RATE as usize / 4);
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mp3_decodes_to_the_same_length() {
        let dir = std::env::temp_dir().join(format!("booth-mp3-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // 44.1 kHz (MPEG-1, the bit reservoir let out every 1024 frames) and 24 kHz (MPEG-2);
        // 70 s at 44.1 kHz crosses two reservoir hand-offs.
        for (rate, secs) in [(RATE, 70.0), (24_000, 2.0), (RATE, 0.01)] {
            let src = dir.join(format!("a{rate}.wav"));
            sine(&src, secs, 0.5, 440.0, rate);
            let mp3 = dir.join(format!("a{rate}.mp3"));
            write_line(&src, &mp3, Format::Mp3, None).unwrap();
            let bytes = std::fs::read(&mp3).unwrap();
            assert!(bytes[..64].windows(4).any(|w| w == b"Info"), "the first frame is the Info tag");
            let mut d = rusty_mp3::Mp3Decoder::new();
            d.push(&bytes);
            d.flush();
            let (mut n, mut got_rate, mut loud) = (0, 0, 0f32);
            loop {
                match d.next_frame() {
                    Ok(f) => {
                        n += f.samples.len();
                        got_rate = f.sample_rate;
                        loud = f.samples.iter().fold(loud, |m, x| m.max(x.abs()));
                    }
                    Err(rusty_mp3::Error::Again | rusty_mp3::Error::Eof) => break,
                    Err(e) => panic!("{e}"),
                }
            }
            let want = (secs * rate as f64) as usize;
            assert_eq!(got_rate, rate);
            assert!(loud > 0.4 && loud < 0.6, "{loud}");
            // whole frames, with the encoder's delay and padding around the input
            let spf = if rate == RATE { 1152 } else { 576 };
            assert!((want..want + 4 * spf).contains(&n), "{rate}: {n} for {want}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
