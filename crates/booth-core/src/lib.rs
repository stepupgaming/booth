//! Booth's shared core: used by the desktop app and by booth-mcp, so both see the same engine,
//! voices, takes and settings.

pub mod api;
pub mod controls;
pub mod engine;
pub mod models;
pub mod paths;
pub mod settings;
pub mod studio;
pub mod takes;
pub mod voices;

pub use api::{GenerateRequest, SpeakerMode, Zonos};
pub use chrono;
