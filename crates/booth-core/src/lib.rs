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

/// An HTTP client builder for everything Booth fetches. On Android it trusts the Mozilla root
/// certificates built into the app: reqwest's default (the system's verifier through
/// rustls-platform-verifier) needs Java-side setup that a Tauri app doesn't do, and panics.
pub fn http() -> reqwest::ClientBuilder {
    let builder = reqwest::Client::builder();
    #[cfg(target_os = "android")]
    let builder = builder.tls_certs_only(
        webpki_root_certs::TLS_SERVER_ROOT_CERTS
            .iter()
            .filter_map(|c| reqwest::Certificate::from_der(c).ok()),
    );
    builder
}

/// [`http`] with no further settings.
pub fn http_client() -> reqwest::Client {
    http().build().expect("http client")
}
