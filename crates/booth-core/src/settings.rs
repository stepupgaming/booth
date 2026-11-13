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
    /// Guard against skips and early endings (needs LACI heads in the engine pack).
    pub laci: bool,
    /// Where the voice model runs: the graphics card when there is one Booth can use, or the processor.
    pub device: Device,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Device {
    /// The graphics card when Booth can use one (NVIDIA with the GPU engine, or Apple Silicon), else the processor.
    #[default]
    Auto,
    /// Always the processor.
    Cpu,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            default_voice: None,
            decoder: None,
            mcp_allow_cloning: false,
            keep_engine_running: false,
            laci: false,
            device: Device::Auto,
        }
    }
}

pub fn load() -> Settings {
    std::fs::read_to_string(paths::settings_file())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save(s: &Settings) -> anyhow::Result<()> {
    paths::ensure_dir(&paths::home())?;
    std::fs::write(paths::settings_file(), serde_json::to_vec_pretty(s)?)?;
    Ok(())
}
