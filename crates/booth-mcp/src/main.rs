//! booth-mcp: Booth's MCP server over stdio. Claude Desktop runs it from the Booth extension
//! (`booth.mcpb`); it shares the engine, voices, takes and settings with the Booth app.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use booth_core::{
    engine, models, paths, settings,
    studio::{self, SpeakOptions},
    takes::{self, ScriptLine, Take},
    voices::{self, Consent, Voice, VoiceKind},
    SpeakerMode, Zonos,
};
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig},
    schemars, tool, tool_handler, tool_router, ErrorData, ServerHandler, ServiceExt,
};
use serde::Deserialize;
use serde_json::{json, Value};

const INSTRUCTIONS: &str = "Booth speaks text out loud with ZONOS3-Mini, a voice model running on this PC. \
Every result is a WAV file saved in Booth's takes folder; give the user the file path. \
Pick a voice with list_voices (call it before the first speak). Exact voices sound closest to their clip \
but can't be reshaped; Shapeable voices accept controls (see list_controls). \
For several lines or several speakers, use render_script instead of many speak calls. \
create_voice clones a real person's voice: only use it when the user asks, only with a clip of themselves \
or of someone who agreed, and only if Booth's settings allow it.";

#[derive(Clone)]
struct Booth {
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ListVoicesArgs {
    /// Only voices whose language, accent, gender or name contains this (e.g. "English", "British", "female").
    #[serde(default)]
    filter: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SpeakArgs {
    /// What to say. Plain text; punctuation shapes the delivery.
    text: String,
    /// Voice id or name from list_voices. Default: the voice chosen in Booth.
    #[serde(default)]
    voice: Option<String>,
    /// "exact" (closest to the voice's clip) or "shapeable" (accepts every control).
    #[serde(default)]
    mode: Option<String>,
    /// Controls by id from list_controls, id -> dose, e.g. {"happy": 3, "pace": -0.5, "accent:british": 1.2}.
    #[serde(default)]
    controls: BTreeMap<String, f64>,
    /// Seed for a repeatable take.
    #[serde(default)]
    seed: Option<u64>,
    /// Language of the text. Default: the language chosen in Booth (US English unless changed). Codes: en_us en_gb fr_fr de es it pt_br ja cmn ko ar bn bg cs
    /// da nl fi ka el gu he hi hu id kn ms ml mr no or pl pa ro ru sk sv tl ta te th tr uk ur vi.
    #[serde(default)]
    language: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ScriptArgs {
    /// The lines in order, each with its voice.
    lines: Vec<ScriptLine>,
    /// Silence between lines in milliseconds (default 350).
    #[serde(default)]
    gap_ms: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ListTakesArgs {
    /// How many, newest first (default 10).
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DesignArgs {
    /// Name for the new voice.
    name: String,
    /// The voice to start from (id or name).
    base_voice: String,
    /// Controls baked into the new voice, id -> dose (see list_controls), e.g. {"age": 1.5, "deep": 2, "accent:scottish": 1.5}.
    /// A preset name from list_controls works too: {"preset:BBC newsreader": 1}.
    controls: BTreeMap<String, f64>,
    /// A short note about the voice.
    #[serde(default)]
    description: Option<String>,
    /// If set, speak this line with the new voice as a preview.
    #[serde(default)]
    preview_text: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CreateVoiceArgs {
    /// Name for the new voice.
    name: String,
    /// Full path of a clip of the speaker (wav, mp3, flac, m4a; 5 to 30 seconds of clean speech works best).
    audio_path: String,
    /// "me" if the clip is the user's own voice, "consented" if it is someone who agreed to be cloned.
    speaker: String,
    /// Must be true: the user confirmed the speaker agreed to this clone.
    consent_confirmed: bool,
    /// The exact words spoken in the clip, if known.
    #[serde(default)]
    transcript: Option<String>,
}

fn fail(msg: impl std::fmt::Display) -> ErrorData {
    ErrorData::internal_error(msg.to_string(), None)
}

fn ok_json(v: Value) -> Result<CallToolResult, ErrorData> {
    Ok(CallToolResult::success(vec![ContentBlock::text(
        serde_json::to_string_pretty(&v).unwrap_or_default(),
    )]))
}

fn tool_error(msg: impl std::fmt::Display) -> Result<CallToolResult, ErrorData> {
    Ok(CallToolResult::error(vec![ContentBlock::text(msg.to_string())]))
}

fn voice_json(v: &Voice) -> Value {
    let modes: Vec<&str> = v
        .modes
        .iter()
        .map(|m| match m {
            SpeakerMode::Reference => "exact",
            SpeakerMode::Embedding => "shapeable",
        })
        .collect();
    json!({
        "id": v.id,
        "name": v.name,
        "kind": match v.kind { VoiceKind::Default => "built-in", VoiceKind::Clone => "clone", VoiceKind::Designed => "designed" },
        "modes": modes,
        "language": v.language,
        "accent": v.accent,
        "gender": v.gender,
        "description": v.description,
    })
}

fn take_json(t: &Take) -> Value {
    json!({
        "id": t.id,
        "file": t.path.display().to_string(),
        "seconds": (t.seconds * 10.0).round() / 10.0,
        "voice": t.voice_name,
        "text": t.text,
        "seed": t.seed,
        "mode": t.mode,
        "created_at": t.created_at,
    })
}

fn parse_mode(m: Option<&str>) -> Result<Option<SpeakerMode>, String> {
    match m.map(|s| s.trim().to_lowercase()) {
        None => Ok(None),
        Some(s) if s.is_empty() => Ok(None),
        Some(s) if s == "exact" || s == "reference" => Ok(Some(SpeakerMode::Reference)),
        Some(s) if s == "shapeable" || s == "embedding" => Ok(Some(SpeakerMode::Embedding)),
        Some(s) => Err(format!("mode must be \"exact\" or \"shapeable\", not \"{s}\"")),
    }
}

async fn engine_ready() -> Result<Zonos, String> {
    engine::ensure("booth-mcp").await.map_err(|e| {
        format!("{e:#}. Open the Booth app once to finish setup (it downloads the voice engine and models).")
    })
}

#[tool_router]
impl Booth {
    fn new() -> Self {
        Self { tool_router: Self::tool_router() }
    }

    #[tool(
        description = "Is Booth's voice engine installed and running? Shows downloaded model packs, how many voices exist, where takes are saved, and whether voice cloning from Claude is allowed.",
        annotations(title = "Booth status", read_only_hint = true)
    )]
    async fn status(&self) -> Result<CallToolResult, ErrorData> {
        let state = engine::state().await;
        let s = settings::load();
        ok_json(json!({
            "engine": state,
            "packs": models::status(&paths::models_dir()),
            "my_voices": voices::mine().len(),
            "takes_folder": paths::takes_dir().display().to_string(),
            "cloning_from_claude_allowed": s.mcp_allow_cloning,
            "default_voice": s.default_voice,
        }))
    }

    #[tool(
        description = "List the voices Booth can speak with: the user's own (clones, designed voices) and the built-in ones. Starts the engine if needed.",
        annotations(title = "List voices", read_only_hint = true)
    )]
    async fn list_voices(&self, Parameters(a): Parameters<ListVoicesArgs>) -> Result<CallToolResult, ErrorData> {
        let z = engine_ready().await.ok();
        let mut list = voices::all(z.as_ref()).await;
        if let Some(f) = a.filter.as_deref().map(str::to_lowercase).filter(|f| !f.is_empty()) {
            list.retain(|v| {
                [Some(&v.name), v.language.as_ref(), v.accent.as_ref(), v.gender.as_ref(), v.description.as_ref()]
                    .iter()
                    .any(|x| x.is_some_and(|x| x.to_lowercase().contains(&f)))
            });
        }
        let mut out = json!({ "voices": list.iter().map(voice_json).collect::<Vec<_>>() });
        if z.is_none() {
            out["note"] = json!("The engine isn't running, so only your own voices are listed.");
        }
        ok_json(out)
    }

    #[tool(
        description = "The controls a voice accepts: who (gender, age, pitch), texture, room, delivery, mood, and accents, with dose ranges, what each does, and named presets. Pass them to speak or design_voice as {id: dose}.",
        annotations(title = "List controls", read_only_hint = true)
    )]
    async fn list_controls(&self) -> Result<CallToolResult, ErrorData> {
        let z = match engine_ready().await {
            Ok(z) => z,
            Err(e) => return tool_error(e),
        };
        let cat = z.catalog().await.map_err(fail)?;
        ok_json(cat.summary())
    }

    #[tool(
        description = "Speak one line out loud and save it as a WAV take. Returns the file path, length and seed.",
        annotations(title = "Speak", read_only_hint = false, destructive_hint = false)
    )]
    async fn speak(&self, Parameters(a): Parameters<SpeakArgs>) -> Result<CallToolResult, ErrorData> {
        let mode = match parse_mode(a.mode.as_deref()) {
            Ok(m) => m,
            Err(e) => return tool_error(e),
        };
        let z = match engine_ready().await {
            Ok(z) => z,
            Err(e) => return tool_error(e),
        };
        let opts = SpeakOptions {
            voice: a.voice,
            mode,
            controls: a.controls,
            seed: a.seed,
            language: a.language,
            via: "mcp".into(),
        };
        match studio::speak(&z, &a.text, &opts).await {
            Ok(t) => ok_json(take_json(&t)),
            Err(e) => tool_error(format!("{e:#}")),
        }
    }

    #[tool(
        description = "Render a script: several lines, each with its own voice, joined into one WAV take with a short pause between lines. Use for dialogue, narration in parts, or ads.",
        annotations(title = "Render script", read_only_hint = false, destructive_hint = false)
    )]
    async fn render_script(&self, Parameters(a): Parameters<ScriptArgs>) -> Result<CallToolResult, ErrorData> {
        let z = match engine_ready().await {
            Ok(z) => z,
            Err(e) => return tool_error(e),
        };
        match studio::render_script(&z, &a.lines, a.gap_ms.unwrap_or(350), "mcp").await {
            Ok(t) => ok_json(take_json(&t)),
            Err(e) => tool_error(format!("{e:#}")),
        }
    }

    #[tool(
        description = "Recent takes (made in the app or by Claude), newest first, with file paths.",
        annotations(title = "List takes", read_only_hint = true)
    )]
    async fn list_takes(&self, Parameters(a): Parameters<ListTakesArgs>) -> Result<CallToolResult, ErrorData> {
        let list = takes::list(a.limit.unwrap_or(10).clamp(1, 200));
        ok_json(json!({ "takes": list.iter().map(take_json).collect::<Vec<_>>() }))
    }

    #[tool(
        description = "Design a new voice by reshaping an existing one with steering controls (e.g. older, deeper, warmer, a British accent), and save it under a name. Optionally speaks a preview line.",
        annotations(title = "Design voice", read_only_hint = false, destructive_hint = false)
    )]
    async fn design_voice(&self, Parameters(a): Parameters<DesignArgs>) -> Result<CallToolResult, ErrorData> {
        let z = match engine_ready().await {
            Ok(z) => z,
            Err(e) => return tool_error(e),
        };
        let base = match voices::find(Some(&z), &a.base_voice).await {
            Ok(v) => v,
            Err(e) => return tool_error(e),
        };
        let cat = z.catalog().await.map_err(fail)?;
        let mut friendly = BTreeMap::new();
        for (k, dose) in a.controls {
            if let Some(name) = k.strip_prefix("preset:") {
                let Some(p) = cat.presets.iter().find(|(n, _)| n.eq_ignore_ascii_case(name.trim())) else {
                    return tool_error(format!("no preset '{name}'"));
                };
                for (pk, pv) in p.1 {
                    *friendly.entry(pk.clone()).or_insert(0.0) += pv * dose;
                }
            } else {
                *friendly.entry(k).or_insert(0.0) += dose;
            }
        }
        let raw = match cat.expand(&friendly) {
            Ok(r) => r,
            Err(e) => return tool_error(e),
        };
        let v = match voices::create_designed(&z, &a.name, &base, raw, a.description).await {
            Ok(v) => v,
            Err(e) => return tool_error(format!("{e:#}")),
        };
        let mut out = json!({ "voice": voice_json(&v) });
        if let Some(text) = a.preview_text.filter(|t| !t.trim().is_empty()) {
            let opts = SpeakOptions { voice: Some(v.id.clone()), via: "mcp".into(), ..Default::default() };
            match studio::speak(&z, &text, &opts).await {
                Ok(t) => out["preview"] = take_json(&t),
                Err(e) => out["preview_error"] = json!(format!("{e:#}")),
            }
        }
        ok_json(out)
    }

    #[tool(
        description = "Clone a voice from an audio clip and save it. Off unless the user enabled \"Let Claude clone voices\" in Booth's settings. Only for the user's own voice or someone who agreed; never for a public figure or anyone who hasn't consented.",
        annotations(title = "Create voice (clone)", read_only_hint = false, destructive_hint = false)
    )]
    async fn create_voice(&self, Parameters(a): Parameters<CreateVoiceArgs>) -> Result<CallToolResult, ErrorData> {
        if !settings::load().mcp_allow_cloning {
            return tool_error(
                "Voice cloning from Claude is off. The user can turn on \"Let Claude clone voices\" in Booth → Settings, or clone the voice in the Booth app (Voices → New voice).",
            );
        }
        let speaker = a.speaker.trim().to_lowercase();
        if !a.consent_confirmed || !(speaker == "me" || speaker == "consented") {
            return tool_error(
                "Cloning needs consent: set speaker to \"me\" or \"consented\" and consent_confirmed to true, after the user confirms it.",
            );
        }
        let path = PathBuf::from(a.audio_path.trim().trim_matches('"'));
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => return tool_error(format!("can't read {}: {e}", path.display())),
        };
        let ext = path.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_else(|| "wav".into());
        let z = match engine_ready().await {
            Ok(z) => z,
            Err(e) => return tool_error(e),
        };
        let exact = engine::running()
            .await
            .is_some_and(|(lock, _)| lock.packs.contains(&models::Pack::Exact));
        let statement = if speaker == "me" {
            "This is my own voice, and I want to clone it."
        } else {
            "The person in this clip agreed to have their voice cloned."
        };
        let consent = Consent {
            statement: statement.into(),
            confirmed_at: booth_core::chrono::Utc::now(),
            via: "mcp".into(),
        };
        match voices::create_clone(&z, &a.name, &bytes, &ext, a.transcript, consent, exact).await {
            Ok(v) => ok_json(json!({ "voice": voice_json(&v) })),
            Err(e) => tool_error(format!("{e:#}")),
        }
    }
}

#[tool_handler]
impl ServerHandler for Booth {
    fn get_info(&self) -> ServerConfig {
        let mut info = Implementation::new("booth", env!("CARGO_PKG_VERSION"));
        info.title = Some("Booth by NoFaceTube".into());
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(info)
            .with_instructions(INSTRUCTIONS)
    }
}

/// Claude runs this server for as long as it is open, so an engine started from here would hold
/// its ~700 MB (and the graphics card) for the rest of the day. Give it back after a quiet spell;
/// the next line pays a couple of seconds to load it again.
const QUIET: Duration = Duration::from_secs(10 * 60);

async fn free_engine_when_quiet() {
    let step = Duration::from_secs(60);
    let mut quiet = Duration::ZERO;
    loop {
        tokio::time::sleep(step).await;
        if engine::working() > 0 {
            quiet = Duration::ZERO;
            continue;
        }
        quiet += step;
        // Checked here as well so an open Booth window doesn't put a line in the log every QUIET.
        if quiet >= QUIET && engine::wanted_elsewhere().is_none() {
            quiet = Duration::ZERO;
            engine::stop_unless_wanted(Duration::ZERO).await;
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if std::env::args().any(|a| a == "--version") {
        println!("booth-mcp {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    booth_core::diag::set_source("mcp");
    let hold = engine::Holder::new("Claude");
    tokio::spawn(free_engine_when_quiet());
    let service = Booth::new().serve(rmcp::transport::stdio()).await?;
    service.waiting().await?;
    // Claude has closed us down: free the engine unless the Booth app is up and wants it.
    drop(hold);
    engine::stop_unless_wanted(Duration::from_secs(20)).await;
    Ok(())
}
