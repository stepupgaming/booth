//! booth-mcp: Booth's MCP server over stdio. Claude Desktop runs it from the Booth extension
//! (`booth.mcpb`); it shares the engine, voices, takes and settings with the Booth app.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use booth_core::{
    audio, batch, captions, engine, models, paths,
    pronounce::Pronunciation,
    settings,
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
For a whole script the user pasted or wrote (narration, a YouTube or TikTok voiceover, dialogue with NAME: labels), \
use make_script: it leaves out notes like [B-roll] and (SFX), gives each speaker a voice, and returns one voiceover \
WAV plus captions (.srt). It also shows up in the Booth app's Long-form page. \
create_voice clones a real person's voice: only use it when the user asks, only with a clip of themselves \
or of someone who agreed, and only if Booth's settings allow it. If a word comes out wrong (a brand name, an acronym), set_pronunciation teaches Booth how to say it everywhere; keep the user's text as written.";

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
    /// Seed for a repeatable take. With a seed, Booth won't redo an Exact take that sounds like a
    /// different person (without one, it does). A take that stops before the text is done is
    /// still redone with a new seed; the seed returned is the one that made the kept take.
    #[serde(default)]
    seed: Option<u64>,
    /// Sampling temperature, 0.4 to 0.75. Default: the engine's 0.75. Lower makes takes of the
    /// same line sound more alike, with a little less life.
    #[serde(default)]
    temperature: Option<f64>,
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
struct MakeScriptArgs {
    /// The whole script as written. Lines may start with a speaker label ("NARRATOR:", "Host:");
    /// headings, [bracketed] and (SFX ...) notes, timestamps and markdown are left out, and long
    /// paragraphs are split at sentence ends. Blank lines mark paragraphs (a longer pause).
    /// [pause], [pause 2s] or [pause 500ms] adds silence there (1 s by default, up to 10 s).
    script: String,
    /// Voice per speaker label, label -> voice id or name from list_voices, e.g. {"NARRATOR": "Rupert", "GUEST": "Maya"}.
    /// Use "" for lines without a label.
    #[serde(default)]
    voices: BTreeMap<String, String>,
    /// Voice for every speaker not in `voices`. Default: the voice chosen in Booth.
    #[serde(default)]
    voice: Option<String>,
    /// Pause between lines in ms (default 250) and before a new paragraph (default 600).
    #[serde(default)]
    gap_ms: Option<u32>,
    #[serde(default)]
    paragraph_gap_ms: Option<u32>,
    /// Also save each line as its own file (default false).
    #[serde(default)]
    line_files: bool,
    /// Save the voiceover and line files as MP3 (128 kbps) instead of WAV (default false).
    /// `format` wins when both are given.
    #[serde(default)]
    mp3: bool,
    /// File type for the voiceover and line files: "wav" (default), "mp3", "m4a" (AAC),
    /// "flac" (lossless, about half a WAV) or "ogg" (Opus, the smallest).
    #[serde(default)]
    format: Option<audio::Format>,
    /// Even out loudness: lines at a like level, the voiceover at -14 LUFS with peaks under
    /// -1 dBTP (default false).
    #[serde(default)]
    even_loudness: bool,
    /// Short captions of 2 to 4 words, for Shorts, TikTok and Reels, instead of sentences (default false).
    #[serde(default)]
    short_captions: bool,
    /// Also write the captions as WebVTT (.vtt) next to the .srt (default false).
    #[serde(default)]
    vtt: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ScriptStatusArgs {
    /// The script id from make_script.
    id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct RedoLineArgs {
    /// The script id from make_script.
    id: String,
    /// Line number, counting from 1 as make_script lists them.
    line: usize,
    /// New words for the line. Leave out to make the same words again (a new take).
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ListTakesArgs {
    /// How many, newest first (default 10).
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SetPronunciationArgs {
    /// The word or phrase as written, e.g. "SQL", "GIF", "NoFaceTube".
    write: String,
    /// How to say it, spelled the way it sounds, e.g. "sequel", "jif", "no face tube".
    #[serde(default)]
    say: String,
    /// Only when the case matches too ("US" but not "us"). Default false.
    #[serde(default)]
    match_case: bool,
    /// Only for this language (e.g. "en", "es", "fr_fr"). Default: every language.
    #[serde(default)]
    language: Option<String>,
    /// Remove this word from the list instead.
    #[serde(default)]
    remove: bool,
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
    /// Optional. What the clip says, saved with the voice as a note only. Cloning doesn't use it, so leave it out
    /// unless the user wants it kept.
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
        "redone": t.redone,
        "sounds_off": t.drifted,
        "may_be_cut_off": t.cut_off,
    })
}

/// How long a script tool waits before answering with progress (Claude's tool calls time out).
const SCRIPT_WAIT: Duration = Duration::from_secs(100);

fn script_json(j: &batch::Job, out: Option<&batch::Exported>) -> Value {
    let done = j.lines.iter().filter(|l| l.state == batch::LineState::Done).count();
    let failed: Vec<Value> = j
        .lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.state == batch::LineState::Failed)
        .map(|(i, l)| json!({ "line": i + 1, "text": l.text, "error": l.error }))
        .collect();
    let status = if done == j.lines.len() {
        "done"
    } else if j.running {
        "making"
    } else if failed.is_empty() {
        "stopped"
    } else {
        "failed"
    };
    let mut v = json!({
        "id": j.id,
        "title": j.title,
        "status": status,
        "lines_done": done,
        "lines_total": j.lines.len(),
        "lines": j.lines.iter().enumerate().map(|(i, l)| json!({
            "line": i + 1,
            "speaker": l.speaker,
            "voice": l.voice_name,
            "text": l.text,
            "seconds": (l.seconds * 10.0).round() / 10.0,
            "sounds_off": l.drifted,
            "may_be_cut_off": l.cut_off,
        })).collect::<Vec<_>>(),
        "left_out": j.skipped,
    });
    if !failed.is_empty() {
        v["failed"] = json!(failed);
    }
    if let Some(o) = out {
        v["voiceover"] = json!(o.voiceover.as_ref().map(|p| p.display().to_string()));
        v["captions"] = json!(o.captions.as_ref().map(|p| p.display().to_string()));
        if let Some(vtt) = &o.vtt {
            v["captions_vtt"] = json!(vtt.display().to_string());
        }
        if let Some(l) = &o.loudness {
            v["loudness"] = json!({ "lufs": (l.lufs * 10.0).round() / 10.0, "true_peak_dbtp": (l.true_peak * 10.0).round() / 10.0 });
        }
        if !o.lines.is_empty() {
            v["line_files"] = json!(o.lines.iter().map(|p| p.display().to_string()).collect::<Vec<_>>());
        }
        v["seconds"] = json!((o.seconds * 10.0).round() / 10.0);
        v["folder"] = json!(o.folder.display().to_string());
    } else if status == "making" {
        v["next"] = json!("Still making. Call script_status with this id to wait for the rest.");
    }
    v
}

/// Make a script's waiting lines in the background, wait up to `SCRIPT_WAIT`, and export once
/// every line is made.
async fn make_and_report(z: Zonos, id: &str, o: &batch::ExportOptions) -> Result<CallToolResult, ErrorData> {
    let job = batch::get(id).map_err(fail)?;
    let waiting = job.lines.iter().any(|l| l.state == batch::LineState::Waiting);
    let mut task = None;
    if waiting && !job.running {
        let id = id.to_string();
        task = Some(tokio::spawn(async move { batch::run(&z, &id, |_| {}).await }));
    }
    let start = std::time::Instant::now();
    loop {
        if task.as_ref().is_some_and(|t| t.is_finished()) {
            if let Some(t) = task.take() {
                match t.await {
                    Ok(Err(e)) => return tool_error(format!("{e:#}")),
                    Err(e) => return tool_error(format!("making the script stopped: {e}")),
                    Ok(Ok(_)) => {}
                }
            }
        }
        let job = batch::get(id).map_err(fail)?;
        let all_done = job.lines.iter().all(|l| l.state == batch::LineState::Done);
        if all_done {
            let (export_id, export_o) = (id.to_string(), o.clone());
            let out = tokio::task::spawn_blocking(move || batch::export(&export_id, &export_o)).await.map_err(fail)?;
            return match out {
                Ok(out) => ok_json(script_json(&batch::get(id).map_err(fail)?, Some(&out))),
                Err(e) => tool_error(format!("{e:#}")),
            };
        }
        if (!job.running && task.is_none()) || start.elapsed() >= SCRIPT_WAIT {
            return ok_json(script_json(&job, None));
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
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
        description = "Speak one line out loud and save it as a WAV take. Returns the file path, length and seed. may_be_cut_off true means every try stopped before the text was done (mid-word, or words left out): listen to it, or speak it again, reworded or split in two.",
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
            temperature: a.temperature,
            drift_check: a.seed.is_none(),
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
        description = "Make a voiceover from a whole script: Booth splits it into lines, leaves out notes ([B-roll], (SFX), headings), speaks each line with its speaker's voice, and returns one WAV plus captions (.srt) with matching timings. Long scripts keep going in the background: if status is \"making\", call script_status with the id.",
        annotations(title = "Make a script", read_only_hint = false, destructive_hint = false)
    )]
    async fn make_script(&self, Parameters(a): Parameters<MakeScriptArgs>) -> Result<CallToolResult, ErrorData> {
        let z = match engine_ready().await {
            Ok(z) => z,
            Err(e) => return tool_error(e),
        };
        let fallback = match studio::pick_voice(&z, a.voice.as_deref()).await {
            Ok(v) => v.id,
            Err(e) => return tool_error(format!("{e:#}")),
        };
        let job = match batch::create(&z, &a.script, &a.voices, &fallback, "mcp").await {
            Ok(j) => j,
            Err(e) => return tool_error(format!("{e:#}")),
        };
        let mut o = batch::ExportOptions {
            lines: a.line_files,
            format: a.format.unwrap_or(if a.mp3 { audio::Format::Mp3 } else { audio::Format::Wav }),
            even_loudness: a.even_loudness,
            caption_style: if a.short_captions { captions::Style::Short } else { captions::Style::Sentences },
            vtt: a.vtt,
            ..Default::default()
        };
        if let Some(g) = a.gap_ms {
            o.gap_ms = g.min(5000);
        }
        if let Some(g) = a.paragraph_gap_ms {
            o.para_gap_ms = g.min(5000);
        }
        remember_options(&job.id, &o);
        make_and_report(z, &job.id, &o).await
    }

    #[tool(
        description = "Check on a script from make_script: waits for lines still being made (up to about 100 seconds), then returns the voiceover and captions paths once every line is done.",
        annotations(title = "Script status", read_only_hint = false, destructive_hint = false)
    )]
    async fn script_status(&self, Parameters(a): Parameters<ScriptStatusArgs>) -> Result<CallToolResult, ErrorData> {
        let z = match engine_ready().await {
            Ok(z) => z,
            Err(e) => return tool_error(e),
        };
        let o = options_for(&a.id);
        make_and_report(z, &a.id, &o).await
    }

    #[tool(
        description = "Make one line of a script again, with new words or as a fresh take, then rebuild the voiceover and captions.",
        annotations(title = "Redo a script line", read_only_hint = false, destructive_hint = false)
    )]
    async fn redo_script_line(&self, Parameters(a): Parameters<RedoLineArgs>) -> Result<CallToolResult, ErrorData> {
        if a.line == 0 {
            return tool_error("line numbers start at 1");
        }
        let z = match engine_ready().await {
            Ok(z) => z,
            Err(e) => return tool_error(e),
        };
        if let Err(e) = batch::redo(&a.id, a.line - 1, a.text) {
            return tool_error(format!("{e:#}"));
        }
        let o = options_for(&a.id);
        make_and_report(z, &a.id, &o).await
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
        description = "Booth's pronunciation list: words the voice says differently from how they're written (\"SQL\" as \"sequel\"). Every take and script uses it.",
        annotations(title = "List pronunciations", read_only_hint = true)
    )]
    async fn list_pronunciations(&self) -> Result<CallToolResult, ErrorData> {
        ok_json(json!({ "pronunciations": settings::load().pronunciations }))
    }

    #[tool(
        description = "Add or change a word in Booth's pronunciation list, so every take says it this way (or remove it with remove=true). The text stays as written; only the voice changes.",
        annotations(title = "Set pronunciation", read_only_hint = false, destructive_hint = false)
    )]
    async fn set_pronunciation(&self, Parameters(a): Parameters<SetPronunciationArgs>) -> Result<CallToolResult, ErrorData> {
        let write = a.write.trim().to_string();
        if write.is_empty() {
            return tool_error("write is empty");
        }
        if !a.remove && a.say.trim().is_empty() {
            return tool_error("say is empty; use remove=true to take a word off the list");
        }
        let language = a.language.map(|l| l.trim().to_string()).filter(|l| !l.is_empty());
        let mut s = settings::load();
        let same = |p: &Pronunciation| p.write.trim().to_lowercase() == write.to_lowercase() && p.language == language;
        let before = s.pronunciations.len();
        s.pronunciations.retain(|p| !same(p));
        if a.remove {
            if s.pronunciations.len() == before {
                return tool_error(format!("\"{write}\" isn't on the list"));
            }
        } else {
            s.pronunciations.push(Pronunciation { write, say: a.say.trim().to_string(), match_case: a.match_case, language });
        }
        if let Err(e) = settings::save(&s) {
            return tool_error(format!("{e:#}"));
        }
        ok_json(json!({ "pronunciations": s.pronunciations }))
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

/// Export settings per script made in this session, so script_status and redo_script_line export
/// the same way make_script was asked to.
static SCRIPT_OPTIONS: std::sync::Mutex<BTreeMap<String, batch::ExportOptions>> = std::sync::Mutex::new(BTreeMap::new());

fn remember_options(id: &str, o: &batch::ExportOptions) {
    SCRIPT_OPTIONS.lock().unwrap_or_else(|e| e.into_inner()).insert(id.to_string(), o.clone());
}

fn options_for(id: &str) -> batch::ExportOptions {
    SCRIPT_OPTIONS.lock().unwrap_or_else(|e| e.into_inner()).get(id).cloned().unwrap_or_default()
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
