//! Settings shared by the app and booth-mcp (`%LOCALAPPDATA%\NoFaceTube\Booth\settings.json`).

use serde::{Deserialize, Serialize};

use crate::paths;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Voice used when a request names none.
    pub default_voice: Option<String>,
    /// Decoder id (`keep2_plain` = fast, `full_mr` = studio); None = the engine's default.
    pub decoder: Option<String>,
    /// Let Claude (booth-mcp) clone voices from audio files. Off until the user turns it on in the app.
    pub mcp_allow_cloning: bool,
    /// Keep the engine loaded after the app closes, so Claude's first request is fast.
    pub keep_engine_running: bool,
    /// Guard against skips and early endings (needs LACI heads in the engine pack). On by default
    /// since 0.6.0; a take the engine can't guard is made without it (see `api::LaciUnavailable`).
    pub laci: bool,
    /// Where the voice model runs: the graphics card when there is one Booth can use, or the processor.
    pub device: Device,
    /// Language of the text (a server code like `es` or `fr_fr`); None = US English.
    pub language: Option<String>,
    /// Words the voice should say differently ("SQL" as "sequel"); see `pronounce`.
    pub pronunciations: Vec<crate::pronounce::Pronunciation>,
    /// Which of [`load`]'s one-time changes this file has had. A file without it is from before
    /// 0.6.0, so it reads as 0 rather than as the default.
    #[serde(default)]
    pub version: u32,
}

/// The settings version this build writes (see [`load`]).
pub const VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Device {
    /// The graphics card when Booth can use one (NVIDIA with the GPU engine, or Apple Silicon), else the processor.
    #[default]
    Auto,
    /// Always the processor.
    Cpu,
    /// The Vulkan engine (beta) on an AMD or Intel graphics card, when it's downloaded; else the processor.
    Vulkan,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            default_voice: None,
            decoder: None,
            mcp_allow_cloning: false,
            keep_engine_running: false,
            laci: true,
            device: Device::Auto,
            language: None,
            pronunciations: Vec::new(),
            version: VERSION,
        }
    }
}

pub fn load() -> Settings {
    std::fs::read_to_string(paths::settings_file())
        .ok()
        .and_then(|t| parse(&t))
        .unwrap_or_default()
}

/// A settings file, with the one-time changes for files from older versions:
/// 1. (0.6.0) Guard against skips turns on: it was off by default, and without it takes that stop
///    after the first sentence went unnoticed.
fn parse(text: &str) -> Option<Settings> {
    let mut s: Settings = serde_json::from_str(text).ok()?;
    if s.version < 1 {
        s.laci = true;
    }
    s.version = s.version.max(VERSION);
    Some(s)
}

pub fn save(s: &Settings) -> anyhow::Result<()> {
    paths::ensure_dir(&paths::home())?;
    std::fs::write(paths::settings_file(), serde_json::to_vec_pretty(s)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guard_against_skips_turns_on_once_for_older_settings() {
        let old = parse(r#"{"laci": false, "default_voice": "Wyatt"}"#).unwrap();
        assert!(old.laci);
        assert_eq!(old.default_voice.as_deref(), Some("Wyatt"));
        assert_eq!(old.version, VERSION);
        // Turned off again after that, it stays off.
        let saved = serde_json::to_string(&Settings { laci: false, ..old }).unwrap();
        assert!(!parse(&saved).unwrap().laci);
        assert!(Settings::default().laci);
        assert!(parse("{}").unwrap().laci);
    }
}
