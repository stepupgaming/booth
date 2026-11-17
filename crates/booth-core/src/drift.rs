//! Drift check: after a take renders, compare its voiceprint with the voice's own and make the
//! take again when it sounds like someone else.
//!
//! Tested on Rupert (2026-11-15): with no controls, about one Exact take in a hundred came out as
//! a different-sounding person (voiceprint match 0.21 to 0.32, where normal Exact takes score 0.67
//! and up). At Booth's strongest moods it was about one in eight (as low as 0.03). Shapeable takes
//! never drifted in 340 renders, so only Exact takes are checked. Moods move the voice on purpose,
//! so the cutoff drops with the engine's identity curve for each control, squared: in Exact, a
//! mood moves the voice about that much further than the curve (measured on Shapeable) says.
//!
//! Retested on Rupert, Cillian and Imogen (2026-11-17, 162 Exact takes, 6 seeds each): under
//! about 1.5 s the voiceprint can't tell voices apart (a take's match with its own voice was
//! 0.05 to 0.6, with other voices up to 0.31), so short takes aren't checked. A built-in voice's
//! print is now made from its preview clip, the same way a take's is: the engine's stored print
//! for Cillian matched his own takes at only 0.4, so every Cillian line was made three times and
//! flagged. From the preview, own takes of 2 s and longer score 0.6 to 0.88 for all three (the
//! odd outlier at 0.23 to 0.31), and other voices at most 0.41.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Mutex, OnceLock};

use anyhow::{bail, Context, Result};

use crate::api::{Audio, GenerateRequest, Zonos};
use crate::controls::Catalog;
use crate::voices::{Voice, VoiceKind};

/// Lowest voiceprint match a take may have with no moods or delivery controls on.
pub const BASE: f64 = 0.5;
/// The cutoff never scales below this share of `BASE`, however strong the controls (angry at
/// Booth's strongest is about 0.41).
const FLOOR: f64 = 0.4;
/// Extra tries after the first take before Booth keeps the closest one.
pub const RETRIES: u32 = 2;
/// Takes shorter than this aren't checked: too little voice to match.
pub const MIN_SECONDS: f64 = 1.5;

/// What the check found for one kept take.
#[derive(Debug, Clone, Copy)]
pub struct Verdict {
    /// Cosine of the kept take's voiceprint with the voice's.
    pub voice_match: f64,
    /// How many times the take was made again.
    pub redone: u32,
    /// Still under the cutoff after every retry; the kept take was the closest.
    pub drifted: bool,
}

/// A 1-D little-endian float32 `.npy` file (what `api::npy_f32` writes and Booth stores).
pub fn parse_npy_f32(bytes: &[u8]) -> Result<Vec<f32>> {
    if bytes.len() < 10 || &bytes[..6] != b"\x93NUMPY" {
        bail!("not a .npy file");
    }
    let (header_len, start) = match bytes[6] {
        1 => (u16::from_le_bytes([bytes[8], bytes[9]]) as usize, 10),
        2 | 3 if bytes.len() >= 12 => (u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize, 12),
        v => bail!(".npy version {v} isn't supported"),
    };
    let data = start + header_len;
    let header = std::str::from_utf8(bytes.get(start..data).context("short .npy header")?)?;
    if !header.contains("'<f4'") {
        bail!(".npy isn't little-endian float32: {header}");
    }
    Ok(bytes[data..].chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
}

pub fn cosine(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let (mut dot, mut na, mut nb) = (0.0f64, 0.0f64, 0.0f64);
    for (&x, &y) in a.iter().zip(b) {
        let (x, y) = (f64::from(x), f64::from(y));
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na.sqrt() * nb.sqrt())
    }
}

/// The voice's own voiceprint: its saved embedding (made from its clip), or for a built-in voice
/// the print of its preview clip (made once per voice; the engine's stored print if the preview
/// can't be had). None when the voice has neither.
pub async fn voiceprint(z: &Zonos, voice: &Voice) -> Option<Vec<f32>> {
    static PRESETS: OnceLock<Mutex<HashMap<String, Vec<f32>>>> = OnceLock::new();
    if let Some(path) = voice.embedding_path() {
        return std::fs::read(&path).ok().and_then(|b| parse_npy_f32(&b).ok());
    }
    if voice.kind != VoiceKind::Default {
        return None;
    }
    let id = voice.speaker_id.clone()?;
    let cache = PRESETS.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(v) = cache.lock().unwrap().get(&id) {
        return Some(v.clone());
    }
    let from_preview = async {
        let wav = z.speaker_preview(&id).await?;
        parse_npy_f32(&z.embed_clip(&wav, "preview.wav").await?)
    };
    let print = match from_preview.await {
        Ok(v) => Ok(v),
        Err(e) => {
            crate::diag::note(format!("drift check: no preview print for {}, using the stored one: {e:#}", voice.name));
            z.speaker_embedding(&id).await.and_then(|b| parse_npy_f32(&b))
        }
    };
    match print {
        Ok(v) => {
            cache.lock().unwrap().insert(id, v.clone());
            Some(v)
        }
        Err(e) => {
            crate::diag::note(format!("drift check: no voiceprint for {}: {e:#}", voice.name));
            None
        }
    }
}

/// Identity similarity a curve gives at `dose`: straight lines between its points, flat past the ends.
fn on_curve(curve: &[[f64; 2]], dose: f64) -> Option<f64> {
    let first = curve.first()?;
    if dose <= first[0] {
        return Some(first[1]);
    }
    for w in curve.windows(2) {
        let ([x0, y0], [x1, y1]) = (w[0], w[1]);
        if dose <= x1 {
            return Some(if x1 > x0 { y0 + (y1 - y0) * (dose - x0) / (x1 - x0) } else { y1 });
        }
    }
    curve.last().map(|p| p[1])
}

/// The cutoff for a take made with these controls (friendly ids or raw directions): `BASE`
/// lowered by how much the engine says each control moves the voice away from itself.
///
/// At Booth's strongest settings this gives happy 0.31, calm 0.39, sad 0.38, angry 0.21,
/// scared 0.30, confused 0.36; every normal take in the test cleared those and every take that
/// sounded like someone else fell under them.
pub fn cutoff(cat: &Catalog, controls: &BTreeMap<String, f64>) -> f64 {
    let mut factor = 1.0;
    for (key, &dose) in controls {
        if dose == 0.0 {
            continue;
        }
        let key = key.trim();
        // A friendly id, or a raw direction that is exactly one control's only direction.
        let control = cat.controls.iter().find(|c| c.id.eq_ignore_ascii_case(key)).or_else(|| {
            cat.controls.iter().find(|c| {
                c.map.values().any(|m| m.len() == 1 && m.get(key).is_some_and(|&u| u == 1.0 || u == -1.0))
            })
        });
        let Some(curve) = control.and_then(|c| c.identity.as_deref()) else {
            continue;
        };
        if let (Some(at0), Some(at)) = (on_curve(curve, 0.0), on_curve(curve, dose.abs())) {
            if at0 > 0.0 {
                factor *= (at / at0).min(1.0);
            }
        }
    }
    BASE * (factor * factor).max(FLOOR)
}

fn fresh_seed() -> u64 {
    1 + (uuid::Uuid::new_v4().as_u128() % 2_147_483_000) as u64
}

async fn score(z: &Zonos, target: &[f32], wav: &[u8]) -> Result<f64> {
    let npy = z.embed_clip(wav, "take.wav").await?;
    Ok(cosine(target, &parse_npy_f32(&npy)?))
}

/// Check `first` against the voice and make it again (new seed) while it scores under `cut`,
/// up to `RETRIES` times; returns the closest take. If the check itself fails, the take is kept
/// as it is and the verdict is None.
pub async fn keep_closest(
    z: &Zonos,
    req: &mut GenerateRequest,
    first: Audio,
    target: &[f32],
    cut: f64,
) -> (Audio, Option<Verdict>) {
    let mut best = match score(z, target, &first.wav).await {
        Ok(s) => (first, s),
        Err(e) => {
            crate::diag::note(format!("drift check: couldn't score the take: {e:#}"));
            return (first, None);
        }
    };
    let mut redone = 0;
    while best.1 < cut && redone < RETRIES {
        redone += 1;
        crate::diag::note(format!("drift check: match {:.3} under {cut:.3}, redoing ({redone}/{RETRIES})", best.1));
        req.seed = Some(fresh_seed());
        let next = match z.generate(req).await {
            Ok(a) => a,
            Err(e) => {
                crate::diag::note(format!("drift check: redo failed, keeping the closest: {e:#}"));
                break;
            }
        };
        match score(z, target, &next.wav).await {
            Ok(s) if s > best.1 => best = (next, s),
            Ok(_) => {}
            Err(e) => {
                crate::diag::note(format!("drift check: couldn't score a redo: {e:#}"));
                break;
            }
        }
    }
    let (audio, voice_match) = best;
    (audio, Some(Verdict { voice_match, redone, drifted: voice_match < cut }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn npy_round_trip() {
        let v = vec![0.5f32, -1.25, 3.0];
        assert_eq!(parse_npy_f32(&crate::api::npy_f32(&v)).unwrap(), v);
    }

    #[test]
    fn cosine_basics() {
        assert!((cosine(&[1.0, 0.0], &[2.0, 0.0]) - 1.0).abs() < 1e-9);
        assert!(cosine(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-9);
        assert_eq!(cosine(&[1.0], &[1.0, 2.0]), 0.0);
    }

    #[test]
    fn cutoff_follows_identity_curves() {
        let doc = serde_json::json!({ "ui": { "controls": [
            { "id": "angry", "group": "mood", "label": "Angry", "type": "unipolar",
              "map": { "+": { "emotions.angry": 1 } }, "range": [0, 6],
              "identity": [[0.0, 0.945], [4, 0.785], [6, 0.606], [8, 0.515]] },
            { "id": "pace", "group": "delivery", "label": "Pace", "type": "bipolar",
              "map": { "-": { "delivery.tempo": -1 }, "+": { "delivery.tempo": 1 } } }
        ] } });
        let cat = Catalog::from_doc(&doc).unwrap();
        let none = BTreeMap::new();
        assert_eq!(cutoff(&cat, &none), BASE);
        let angry: BTreeMap<String, f64> = [("angry".to_string(), 6.0)].into();
        let f = 0.606 / 0.945;
        assert!((cutoff(&cat, &angry) - BASE * f * f).abs() < 1e-9);
        // Raw direction ids find the same control.
        let raw: BTreeMap<String, f64> = [("emotions.angry".to_string(), 6.0)].into();
        assert!((cutoff(&cat, &raw) - cutoff(&cat, &angry)).abs() < 1e-9);
        // No curve, no change; and never under the floor.
        let pace: BTreeMap<String, f64> = [("pace".to_string(), 2.0)].into();
        assert_eq!(cutoff(&cat, &pace), BASE);
        let huge: BTreeMap<String, f64> = [("angry".to_string(), 50.0)].into();
        assert!((cutoff(&cat, &huge) - BASE * FLOOR).abs() < 1e-9);
    }
}
