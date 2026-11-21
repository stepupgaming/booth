//! Streaming writers for M4A (AAC-LC), Ogg Opus and FLAC. All three are pure Rust, so they
//! build the same on Windows, Mac, Linux and Android, with no system FFmpeg.
//!
//! - M4A: the AAC encoder from `third_party/oxideav-aac` (vendored with speed fixes) and a small
//!   MP4 writer here. An edit list hides the encoder's one-frame delay, so players start on time.
//! - Ogg Opus: `opus-pure`. Opus only runs at 48 kHz, so 44.1 kHz audio goes through [`Resampler`].
//! - FLAC: `flacenc`, one 4096-sample block at a time, with STREAMINFO filled in at the end.

use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::Path;

use anyhow::{anyhow, bail, Result};

// ---------------------------------------------------------------- resampler

/// A fixed-ratio polyphase windowed-sinc resampler for interleaved audio (44.1 kHz to 48 kHz for
/// Opus). Each output sample sits exactly at input time `k * m / l`, so it adds no delay.
pub struct Resampler {
    l: usize,
    m: usize,
    half: usize,
    ch: usize,
    bank: Vec<f32>,
    /// Interleaved input not yet fully used, starting at input frame `base`.
    buf: Vec<f32>,
    base: u64,
    /// The next output frame.
    k: u64,
    fed: u64,
}

fn gcd(a: usize, b: usize) -> usize {
    if b == 0 { a } else { gcd(b, a % b) }
}

impl Resampler {
    pub fn new(from: u32, to: u32, channels: usize) -> Self {
        let g = gcd(from as usize, to as usize);
        let (l, m) = (to as usize / g, from as usize / g);
        let half = 32;
        // Cutoff as a fraction of the input's Nyquist: below the lower of the two rates.
        let fc = 0.92 * (l as f64 / m as f64).min(1.0);
        let mut bank = vec![0f32; l * 2 * half];
        for p in 0..l {
            let row = &mut bank[p * 2 * half..(p + 1) * 2 * half];
            let mut sum = 0.0f64;
            let mut vals = [0f64; 64];
            for (j, v) in vals.iter_mut().enumerate().take(2 * half) {
                let u = p as f64 / l as f64 + half as f64 - 1.0 - j as f64;
                let x = std::f64::consts::PI * fc * u;
                let sinc = if x.abs() < 1e-12 { 1.0 } else { x.sin() / x };
                // Blackman window over |u| < half
                let w = u / half as f64;
                let win = if w.abs() >= 1.0 {
                    0.0
                } else {
                    let a = std::f64::consts::PI * (w + 1.0);
                    0.42 - 0.5 * a.cos() + 0.08 * (2.0 * a).cos()
                };
                *v = fc * sinc * win;
                sum += *v;
            }
            for j in 0..2 * half {
                row[j] = (vals[j] / sum) as f32;
            }
        }
        // `half - 1` frames of silence before the first sample, so the first outputs have history.
        Self { l, m, half, ch: channels, bank, buf: vec![0.0; (half - 1) * channels], base: 0, k: 0, fed: 0 }
    }

    /// Output frames for `frames` input frames.
    pub fn out_len(&self, frames: u64) -> u64 {
        (frames * self.l as u64).div_ceil(self.m as u64)
    }

    pub fn push(&mut self, input: &[f32], out: &mut Vec<f32>) {
        self.buf.extend_from_slice(input);
        self.fed += (input.len() / self.ch) as u64;
        self.run(out, self.out_len(self.fed));
    }

    /// Feed the trailing silence the filter needs and return the rest of the output.
    pub fn finish(&mut self, out: &mut Vec<f32>) {
        self.buf.extend(std::iter::repeat_n(0.0, (self.half + 1) * self.ch));
        self.run(out, self.out_len(self.fed));
    }

    fn run(&mut self, out: &mut Vec<f32>, limit: u64) {
        let (ch, taps) = (self.ch, 2 * self.half);
        // buf holds input frames from (base - (half - 1)) on.
        let avail = (self.buf.len() / ch) as u64;
        while self.k < limit {
            let pos = self.k * self.m as u64;
            let n = pos / self.l as u64;
            let p = (pos % self.l as u64) as usize;
            // taps cover input frames n - half + 1 ..= n + half, i.e. buf index (n - base) .. +taps
            let start = n - self.base;
            if start + taps as u64 > avail {
                break;
            }
            let row = &self.bank[p * taps..(p + 1) * taps];
            for c in 0..ch {
                let s = start as usize;
                let acc: f32 = row.iter().enumerate().map(|(j, w)| self.buf[(s + j) * ch + c] * w).sum();
                out.push(acc);
            }
            self.k += 1;
        }
        // Drop frames no future output needs.
        let next_n = self.k * self.m as u64 / self.l as u64;
        let drop = next_n.saturating_sub(self.base).min(avail) as usize;
        if drop > 4096 {
            self.buf.drain(..drop * ch);
            self.base += drop as u64;
        }
    }
}

// ---------------------------------------------------------------- M4A (AAC-LC)

const AAC_RATES: [u32; 12] = [96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000];
const AAC_FRAME: usize = 1024;

pub struct M4aOut {
    w: BufWriter<File>,
    enc: oxideav_aac::encoder::StreamEncoder,
    rate: u32,
    channels: u16,
    bitrate: u32,
    pending: Vec<i16>,
    sizes: Vec<u32>,
    mdat_at: u64,
    samples: u64,
}

fn bx(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(body.len() + 8);
    v.extend_from_slice(&((body.len() + 8) as u32).to_be_bytes());
    v.extend_from_slice(kind);
    v.extend_from_slice(body);
    v
}

fn full(kind: &[u8; 4], flags: u32, body: &[u8]) -> Vec<u8> {
    let mut b = flags.to_be_bytes().to_vec(); // version 0
    b.extend_from_slice(body);
    bx(kind, &b)
}

const MATRIX: [u32; 9] = [0x0001_0000, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000];

impl M4aOut {
    pub fn create(path: &Path, rate: u32, channels: u16) -> Result<Self> {
        if !AAC_RATES.contains(&rate) || !(1..=2).contains(&channels) {
            bail!("AAC can't take {rate} Hz with {channels} channels");
        }
        let bitrate = if channels == 1 { 128_000 } else { 192_000 };
        let enc = oxideav_aac::encoder::StreamEncoder::new(oxideav_aac::encoder::EncoderConfig {
            sample_rate: rate,
            channels: channels as u8,
            bitrate,
        })
        .map_err(|e| anyhow!("AAC encoder: {e:?}"))?;
        let mut w = BufWriter::new(File::create(path)?);
        let mut ftyp = b"M4A ".to_vec();
        ftyp.extend_from_slice(&0u32.to_be_bytes());
        for b in [b"M4A ", b"mp42", b"isom"] {
            ftyp.extend_from_slice(b);
        }
        w.write_all(&bx(b"ftyp", &ftyp))?;
        let mdat_at = w.stream_position()?;
        w.write_all(&[0, 0, 0, 0])?;
        w.write_all(b"mdat")?;
        Ok(Self { w, enc, rate, channels, bitrate, pending: Vec::new(), sizes: Vec::new(), mdat_at, samples: 0 })
    }

    fn put(&mut self, adts: Vec<u8>) -> Result<()> {
        if adts.len() < 7 || adts[0] != 0xFF || adts[1] & 0xF0 != 0xF0 {
            bail!("the AAC encoder gave a frame without an ADTS header");
        }
        let head = if adts[1] & 1 == 1 { 7 } else { 9 };
        self.w.write_all(&adts[head..])?;
        self.sizes.push((adts.len() - head) as u32);
        Ok(())
    }

    pub fn write(&mut self, s: &[i16]) -> Result<()> {
        self.samples += (s.len() / self.channels as usize) as u64;
        self.pending.extend_from_slice(s);
        let hop = AAC_FRAME * self.channels as usize;
        let mut at = 0;
        while self.pending.len() - at >= hop {
            let f = self.enc.encode_frame(&self.pending[at..at + hop]).map_err(|e| anyhow!("AAC: {e:?}"))?;
            self.put(f)?;
            at += hop;
        }
        self.pending.drain(..at);
        Ok(())
    }

    pub fn finish(mut self) -> Result<()> {
        if !self.pending.is_empty() || self.sizes.is_empty() {
            let rest = std::mem::take(&mut self.pending);
            let f = self.enc.encode_frame(&rest).map_err(|e| anyhow!("AAC: {e:?}"))?;
            self.put(f)?;
        }
        let f = self.enc.finish().map_err(|e| anyhow!("AAC: {e:?}"))?;
        self.put(f)?;
        let end = self.w.stream_position()?;
        let mdat_len = end - self.mdat_at;
        let moov = self.moov(mdat_len);
        self.w.write_all(&moov)?;
        self.w.seek(SeekFrom::Start(self.mdat_at))?;
        self.w.write_all(&u32::try_from(mdat_len).map_err(|_| anyhow!("M4A file over 4 GB"))?.to_be_bytes())?;
        self.w.flush()?;
        Ok(())
    }

    fn moov(&self, _mdat_len: u64) -> Vec<u8> {
        let rate = self.rate;
        let frames = self.sizes.len() as u32;
        let dur = u32::try_from(self.samples).unwrap_or(u32::MAX);
        let media_dur = frames * AAC_FRAME as u32;
        let be = |v: u32| v.to_be_bytes();
        let mut mvhd = Vec::new();
        for v in [0, 0, rate, dur, 0x0001_0000] {
            mvhd.extend_from_slice(&be(v));
        }
        mvhd.extend_from_slice(&0x0100u16.to_be_bytes());
        mvhd.extend_from_slice(&[0; 10]);
        for v in MATRIX {
            mvhd.extend_from_slice(&be(v));
        }
        mvhd.extend_from_slice(&[0; 24]);
        mvhd.extend_from_slice(&be(2));

        let mut tkhd = Vec::new();
        for v in [0, 0, 1, 0, dur, 0, 0] {
            tkhd.extend_from_slice(&be(v));
        }
        tkhd.extend_from_slice(&[0, 0, 0, 0, 0x01, 0x00, 0, 0]); // layer, group, volume, reserved
        for v in MATRIX {
            tkhd.extend_from_slice(&be(v));
        }
        tkhd.extend_from_slice(&[0; 8]);

        // The encoder runs one frame behind: skip it, and stop where the audio stops.
        let mut elst = Vec::new();
        for v in [1, dur, AAC_FRAME as u32, 0x0001_0000] {
            elst.extend_from_slice(&be(v));
        }
        let edts = bx(b"edts", &full(b"elst", 0, &elst));

        let mut mdhd = Vec::new();
        for v in [0, 0, rate, media_dur] {
            mdhd.extend_from_slice(&be(v));
        }
        mdhd.extend_from_slice(&[0x55, 0xC4, 0, 0]);
        let mut hdlr = vec![0u8; 4];
        hdlr.extend_from_slice(b"soun");
        hdlr.extend_from_slice(&[0; 12]);
        hdlr.extend_from_slice(b"SoundHandler\0");

        let smhd = full(b"smhd", 0, &[0; 4]);
        let mut dref = be(1).to_vec();
        dref.extend_from_slice(&full(b"url ", 1, &[]));
        let dinf = bx(b"dinf", &full(b"dref", 0, &dref));

        let fsi = AAC_RATES.iter().position(|&r| r == rate).unwrap_or(4) as u16;
        let asc = ((2u16 << 11) | (fsi << 7) | (self.channels << 3)).to_be_bytes();
        let max = self.sizes.iter().copied().max().unwrap_or(0);
        let mut dcd = vec![0x40, 0x15];
        dcd.extend_from_slice(&max.to_be_bytes()[1..]);
        dcd.extend_from_slice(&be(self.bitrate));
        dcd.extend_from_slice(&be(self.bitrate));
        dcd.extend_from_slice(&[0x05, 2, asc[0], asc[1]]);
        let mut es = vec![0x00, 0x01, 0x00];
        es.push(0x04);
        es.push(dcd.len() as u8);
        es.extend_from_slice(&dcd);
        es.extend_from_slice(&[0x06, 1, 0x02]);
        let mut esd = vec![0x03, es.len() as u8];
        esd.extend_from_slice(&es);
        let esds = full(b"esds", 0, &esd);
        let mut mp4a = vec![0; 6];
        mp4a.extend_from_slice(&1u16.to_be_bytes());
        mp4a.extend_from_slice(&[0; 8]);
        mp4a.extend_from_slice(&self.channels.to_be_bytes());
        mp4a.extend_from_slice(&16u16.to_be_bytes());
        mp4a.extend_from_slice(&[0; 4]);
        mp4a.extend_from_slice(&be(rate << 16));
        mp4a.extend_from_slice(&esds);
        let mut stsd = be(1).to_vec();
        stsd.extend_from_slice(&bx(b"mp4a", &mp4a));

        let mut stts = Vec::new();
        for v in [1, frames, AAC_FRAME as u32] {
            stts.extend_from_slice(&be(v));
        }
        let mut stsc = Vec::new();
        for v in [1, 1, frames, 1] {
            stsc.extend_from_slice(&be(v));
        }
        let mut stsz = Vec::with_capacity(8 + 4 * self.sizes.len());
        stsz.extend_from_slice(&be(0));
        stsz.extend_from_slice(&be(frames));
        for s in &self.sizes {
            stsz.extend_from_slice(&be(*s));
        }
        let mut stco = be(1).to_vec();
        stco.extend_from_slice(&be(self.mdat_at as u32 + 8));

        let stbl = bx(
            b"stbl",
            &[
                full(b"stsd", 0, &stsd),
                full(b"stts", 0, &stts),
                full(b"stsc", 0, &stsc),
                full(b"stsz", 0, &stsz),
                full(b"stco", 0, &stco),
            ]
            .concat(),
        );
        let minf = bx(b"minf", &[smhd, dinf, stbl].concat());
        let mdia = bx(b"mdia", &[full(b"mdhd", 0, &mdhd), full(b"hdlr", 0, &hdlr), minf].concat());
        let trak = bx(b"trak", &[full(b"tkhd", 3, &tkhd), edts, mdia].concat());
        bx(b"moov", &[full(b"mvhd", 0, &mvhd), trak].concat())
    }
}

// ---------------------------------------------------------------- Ogg Opus

const OPUS_RATE: u32 = 48_000;
const OPUS_FRAME: usize = 960; // 20 ms
/// Opus bitrate per channel group: more than a voice needs (64 kbps already sounds clean).
pub const OPUS_KBPS: i32 = 96;

pub struct OpusOut {
    enc: opus_pure::OpusEncoder,
    w: opus_pure::OggOpusWriter<BufWriter<File>>,
    rs: Resampler,
    rate: u32,
    channels: usize,
    pre_skip: u16,
    buf: Vec<f32>,
    tmp: Vec<f32>,
    packet: Vec<u8>,
    frames_in: u64,
    encoded: u64,
}

impl OpusOut {
    pub fn create(path: &Path, rate: u32, channels: u16) -> Result<Self> {
        let ch = channels as usize;
        let mut enc = opus_pure::OpusEncoder::new(OPUS_RATE as i32, ch, opus_pure::Application::Audio).map_err(|e| anyhow!("Opus encoder: {e:?}"))?;
        enc.bitrate_bps = OPUS_KBPS * 1000;
        let head = opus_pure::OpusHead::for_encoder(&enc, rate);
        let pre_skip = head.pre_skip;
        let w = opus_pure::OggOpusWriter::new(BufWriter::new(File::create(path)?), head).map_err(|e| anyhow!("Ogg writer: {e:?}"))?;
        Ok(Self {
            enc,
            w,
            rs: Resampler::new(rate, OPUS_RATE, ch),
            rate,
            channels: ch,
            pre_skip,
            buf: Vec::new(),
            tmp: Vec::new(),
            packet: vec![0; opus_pure::MAX_PACKET_BYTES],
            frames_in: 0,
            encoded: 0,
        })
    }

    pub fn write(&mut self, s: &[i16]) -> Result<()> {
        self.frames_in += (s.len() / self.channels) as u64;
        self.tmp.clear();
        self.tmp.extend(s.iter().map(|&v| f32::from(v) / 32768.0));
        if self.rate == OPUS_RATE {
            self.buf.extend_from_slice(&self.tmp);
        } else {
            let tmp = std::mem::take(&mut self.tmp);
            self.rs.push(&tmp, &mut self.buf);
            self.tmp = tmp;
        }
        self.drain(false, 0)
    }

    fn drain(&mut self, last: bool, final_granule: u64) -> Result<()> {
        let hop = OPUS_FRAME * self.channels;
        let mut at = 0;
        while self.buf.len() - at >= hop {
            let n = self.enc.encode(&self.buf[at..at + hop], OPUS_FRAME, &mut self.packet).map_err(|e| anyhow!("Opus: {e:?}"))?;
            at += hop;
            self.encoded += 1;
            if last && self.buf.len() - at < hop {
                let d = final_granule - self.w.granule() as u64;
                self.w.write_packet_with_duration(&self.packet[..n], d as u32).map_err(|e| anyhow!("Opus: {e:?}"))?;
            } else {
                self.w.write_packet(&self.packet[..n]).map_err(|e| anyhow!("Opus: {e:?}"))?;
            }
        }
        self.buf.drain(..at);
        Ok(())
    }

    pub fn finish(mut self) -> Result<()> {
        if self.rate != OPUS_RATE {
            self.rs.finish(&mut self.buf);
        }
        let total = if self.rate == OPUS_RATE { self.frames_in } else { self.rs.out_len(self.frames_in) };
        let final_granule = u64::from(self.pre_skip) + total;
        // Encode whole frames covering the audio plus the encoder's delay.
        let frames = (total + u64::from(self.pre_skip)).div_ceil(OPUS_FRAME as u64).max(1);
        let need = (frames - self.encoded) as usize * OPUS_FRAME * self.channels;
        self.buf.resize(need, 0.0);
        self.drain(true, final_granule)?;
        self.w.finish().map_err(|e| anyhow!("Opus: {e:?}"))?;
        Ok(())
    }
}

// ---------------------------------------------------------------- FLAC

const FLAC_BLOCK: usize = 4096;

pub struct FlacOut {
    w: BufWriter<File>,
    cfg: flacenc::error::Verified<flacenc::config::Encoder>,
    info: flacenc::component::StreamInfo,
    fb: flacenc::source::FrameBuf,
    ctx: flacenc::source::Context,
    channels: usize,
    pending: Vec<i32>,
}

impl FlacOut {
    pub fn create(path: &Path, rate: u32, channels: u16) -> Result<Self> {
        use flacenc::error::Verify;
        let ch = channels as usize;
        let cfg = flacenc::config::Encoder::default().into_verified().map_err(|e| anyhow!("FLAC config: {e:?}"))?;
        let mut info = flacenc::component::StreamInfo::new(rate as usize, ch, 16).map_err(|e| anyhow!("FLAC: {e:?}"))?;
        info.set_block_sizes(FLAC_BLOCK, FLAC_BLOCK).map_err(|e| anyhow!("FLAC: {e:?}"))?;
        let mut w = BufWriter::new(File::create(path)?);
        w.write_all(b"fLaC")?;
        // STREAMINFO for now; rewritten with the totals and checksum at the end.
        w.write_all(&[0x80, 0, 0, 34])?;
        w.write_all(&[0; 34])?;
        Ok(Self {
            w,
            cfg,
            info,
            fb: flacenc::source::FrameBuf::with_size(ch, FLAC_BLOCK).map_err(|e| anyhow!("FLAC: {e:?}"))?,
            ctx: flacenc::source::Context::new(16, ch),
            channels: ch,
            pending: Vec::new(),
        })
    }

    fn frame(&mut self, s: &[i32]) -> Result<()> {
        use flacenc::component::BitRepr;
        use flacenc::source::Fill;
        self.fb.fill_interleaved(s).map_err(|e| anyhow!("FLAC: {e:?}"))?;
        self.ctx.fill_interleaved(s).map_err(|e| anyhow!("FLAC: {e:?}"))?;
        let n = self.ctx.current_frame_number().unwrap_or(0);
        let frame = flacenc::encode_fixed_size_frame(&self.cfg, &self.fb, n, &self.info).map_err(|e| anyhow!("FLAC: {e:?}"))?;
        self.info.update_frame_info(&frame);
        let mut sink = flacenc::bitsink::ByteSink::new();
        frame.write(&mut sink).map_err(|e| anyhow!("FLAC: {e:?}"))?;
        self.w.write_all(sink.as_slice())?;
        Ok(())
    }

    pub fn write(&mut self, s: &[i16]) -> Result<()> {
        self.pending.extend(s.iter().map(|&v| i32::from(v)));
        let hop = FLAC_BLOCK * self.channels;
        let mut at = 0;
        let pending = std::mem::take(&mut self.pending);
        while pending.len() - at >= hop {
            self.frame(&pending[at..at + hop])?;
            at += hop;
        }
        self.pending = pending[at..].to_vec();
        Ok(())
    }

    pub fn finish(mut self) -> Result<()> {
        use flacenc::component::BitRepr;
        if !self.pending.is_empty() {
            let rest = std::mem::take(&mut self.pending);
            self.frame(&rest)?;
        }
        let mut info = self.info.clone();
        info.set_md5_digest(&self.ctx.md5_digest());
        info.set_total_samples(self.ctx.total_samples());
        // The minimum leaves out the last block (FLAC format): a fixed-size stream says so, or
        // decoders that trust it read frame numbers as sample numbers.
        info.set_block_sizes(FLAC_BLOCK, FLAC_BLOCK).map_err(|e| anyhow!("FLAC: {e:?}"))?;
        let mut sink = flacenc::bitsink::ByteSink::new();
        info.write(&mut sink).map_err(|e| anyhow!("FLAC: {e:?}"))?;
        self.w.seek(SeekFrom::Start(8))?;
        self.w.write_all(sink.as_slice())?;
        self.w.flush()?;
        Ok(())
    }
}

/// Decoding for the tests: what a player would hear from each file.
#[cfg(test)]
pub(crate) mod decode {
    use std::path::Path;

    use symphonia::core::audio::SampleBuffer;

    /// Sample rate, channels and the interleaved samples, with each format's encoder delay
    /// removed the way a player removes it.
    pub fn any(path: &Path) -> (u32, usize, Vec<f32>) {
        match path.extension().and_then(|e| e.to_str()) {
            Some("ogg") => opus(path),
            Some("mp3") => mp3(path),
            Some("wav") => {
                let p = crate::audio::read_wav(path).unwrap();
                (p.rate, p.channels as usize, p.samples.iter().map(|&v| f32::from(v) / 32768.0).collect())
            }
            Some("m4a") => {
                // symphonia 0.5 doesn't apply the edit list, so drop the priming frame here.
                let (rate, ch, s) = symphonia(path);
                let skip = (super::AAC_FRAME * ch).min(s.len());
                (rate, ch, s[skip..].to_vec())
            }
            _ => symphonia(path),
        }
    }

    fn symphonia(path: &Path) -> (u32, usize, Vec<f32>) {
        let src = std::fs::File::open(path).unwrap();
        let mss = symphonia::core::io::MediaSourceStream::new(Box::new(src), Default::default());
        let mut hint = symphonia::core::probe::Hint::new();
        hint.with_extension(path.extension().unwrap().to_str().unwrap());
        let probed = symphonia::default::get_probe()
            .format(&hint, mss, &Default::default(), &Default::default())
            .unwrap();
        let mut fmt = probed.format;
        let track = fmt.default_track().unwrap().clone();
        let mut dec = symphonia::default::get_codecs().make(&track.codec_params, &Default::default()).unwrap();
        let rate = track.codec_params.sample_rate.unwrap();
        let (mut ch, mut out) = (0, Vec::new());
        while let Ok(pkt) = fmt.next_packet() {
            let buf = dec.decode(&pkt).unwrap();
            ch = buf.spec().channels.count();
            let mut sb = SampleBuffer::<f32>::new(buf.capacity() as u64, *buf.spec());
            sb.copy_interleaved_ref(buf);
            out.extend_from_slice(sb.samples());
        }
        (rate, ch, out)
    }

    fn opus(path: &Path) -> (u32, usize, Vec<f32>) {
        let file = std::io::BufReader::new(std::fs::File::open(path).unwrap());
        let mut r = opus_pure::OggOpusReader::new(file).unwrap();
        let head = r.head().clone();
        let ch = head.channel_count as usize;
        let mut d = head.decoder(super::OPUS_RATE as i32).unwrap();
        let mut trim = opus_pure::Trim::new(&head, super::OPUS_RATE as i32, ch).unwrap();
        let mut block = vec![0f32; opus_pure::MAX_PACKET_SAMPLES * ch];
        let mut out = Vec::new();
        for pk in r.packets() {
            let pk = pk.unwrap();
            let n = d.decode(&pk.data, opus_pure::MAX_PACKET_SAMPLES, &mut block).unwrap();
            out.extend_from_slice(trim.keep(&pk, &block[..n * ch]));
        }
        (super::OPUS_RATE, ch, out)
    }

    fn mp3(path: &Path) -> (u32, usize, Vec<f32>) {
        let mut d = rusty_mp3::Mp3Decoder::new();
        d.push(&std::fs::read(path).unwrap());
        d.flush();
        let (mut rate, mut ch, mut out) = (0, 0, Vec::new());
        while let Ok(f) = d.next_frame() {
            rate = f.sample_rate;
            ch = f.channels as usize;
            out.extend_from_slice(&f.samples);
        }
        (rate, ch, out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(n: usize, rate: u32, hz: f32, amp: f32) -> Vec<i16> {
        (0..n).map(|i| (amp * (std::f32::consts::TAU * hz * i as f32 / rate as f32).sin() * 32767.0) as i16).collect()
    }

    /// The best signal-to-error ratio, in dB, over small shifts of `got` against `want`.
    fn snr(want: &[f32], got: &[f32], max_lag: i64) -> (f64, i64) {
        let mut best = (f64::MIN, 0);
        for lag in -max_lag..=max_lag {
            let (mut sig, mut err) = (0f64, 0f64);
            for (i, &w) in want.iter().enumerate() {
                let j = i as i64 + lag;
                let g = if j >= 0 && (j as usize) < got.len() { got[j as usize] } else { 0.0 };
                sig += f64::from(w).powi(2);
                err += f64::from(w - g).powi(2);
            }
            let db = 10.0 * (sig / err.max(1e-20)).log10();
            if db > best.0 {
                best = (db, lag);
            }
        }
        best
    }

    #[test]
    fn resampler_keeps_a_tone_in_place() {
        let input: Vec<f32> = sine(44_100, 44_100, 1000.0, 0.5).iter().map(|&v| f32::from(v) / 32768.0).collect();
        let mut rs = Resampler::new(44_100, 48_000, 1);
        let mut out = Vec::new();
        // uneven blocks, to cross the buffer trim
        rs.push(&input[..20_000], &mut out);
        rs.push(&input[20_000..], &mut out);
        rs.finish(&mut out);
        assert_eq!(out.len(), 48_000);
        let ideal: Vec<f32> =
            (0..out.len()).map(|i| (std::f32::consts::TAU * 1000.0 * i as f32 / 48_000.0).sin() * 0.5).collect();
        // skip the edges, where the filter sees the silence around the clip
        let (db, lag) = snr(&ideal[100..out.len() - 100], &out[100..], 3);
        assert!(db > 60.0 && lag == 0, "{db} dB at lag {lag}");
    }

    #[test]
    fn each_format_keeps_length_rate_and_sound() {
        let dir = std::env::temp_dir().join(format!("booth-encode-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // 44.1 kHz mono (what the engine makes), a very short clip, and 48 kHz stereo
        for (rate, ch, secs) in [(44_100u32, 1u16, 3.3), (44_100, 1, 0.01), (48_000, 2, 1.5)] {
            let frames = (secs * f64::from(rate)) as usize;
            let mono = sine(frames, rate, 440.0, 0.5);
            let pcm: Vec<i16> = mono.iter().flat_map(|&v| std::iter::repeat_n(v, ch as usize)).collect();
            let want: Vec<f32> = pcm.iter().map(|&v| f32::from(v) / 32768.0).collect();
            for fmt in [crate::audio::Format::M4a, crate::audio::Format::Flac, crate::audio::Format::Ogg] {
                let path = dir.join(format!("t{rate}-{ch}-{frames}.{}", fmt.ext()));
                let mut f = crate::audio::AudioFile::create(&path, fmt, rate, ch).unwrap();
                for c in pcm.chunks(3001 * ch as usize) {
                    f.write(c).unwrap();
                }
                f.finish().unwrap();
                let (got_rate, got_ch, got) = decode::any(&path);
                assert_eq!(got_ch, ch as usize, "{path:?}");
                let got_frames = got.len() / got_ch;
                match fmt {
                    crate::audio::Format::Flac => {
                        assert_eq!(got_rate, rate);
                        assert_eq!(got.len(), want.len());
                        assert!(got.iter().zip(&want).all(|(a, b)| (a - b).abs() < 1e-6), "FLAC is lossless");
                    }
                    crate::audio::Format::M4a => {
                        assert_eq!(got_rate, rate);
                        // whole AAC frames: the audio plus less than one frame of padding
                        assert!((frames..frames + AAC_FRAME + 1).contains(&got_frames), "{got_frames} for {frames}");
                        if secs > 1.0 {
                            let (db, lag) = snr(&want, &got, 4);
                            assert!(db > 20.0 && lag == 0, "M4A {db} dB at lag {lag}");
                        }
                    }
                    _ => {
                        // Opus plays at 48 kHz; the file says what it was made from
                        assert_eq!(got_rate, OPUS_RATE);
                        let want_frames = Resampler::new(rate, OPUS_RATE, 1).out_len(frames as u64) as usize;
                        assert_eq!(got_frames, if rate == OPUS_RATE { frames } else { want_frames });
                        let head = opus_pure::OggOpusReader::new(std::fs::File::open(&path).unwrap()).unwrap().head().clone();
                        assert_eq!(head.input_sample_rate, rate);
                        if secs > 1.0 && rate == OPUS_RATE {
                            let (db, _) = snr(&want, &got, 4);
                            assert!(db > 15.0, "Opus {db} dB");
                        }
                    }
                }
                if secs > 1.0 {
                    let peak = got.iter().fold(0f32, |m, x| m.max(x.abs()));
                    assert!(peak > 0.4 && peak < 0.6, "{path:?} peak {peak}");
                }
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
