// Typed wrappers for the Tauri commands in apps/native/src-tauri/src/lib.rs.
import { getVersion } from "@tauri-apps/api/app";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type Pack = "core" | "gpu_engine" | "exact" | "full_decoder";
/** What the engine computes on: an NVIDIA card, Apple's Metal, or the processor. */
export type Accel = "cuda" | "metal" | "cpu";
export type Device = "auto" | "cpu";
export type Mode = "exact" | "shapeable";
export type SpeakerMode = "reference" | "embedding";

export type EngineState =
  | { state: "no_engine"; looked_in: string }
  | { state: "no_models"; looked_in: string }
  | { state: "stopped" }
  | { state: "running"; port: number; pid: number; packs: Pack[]; accel: Accel };

export interface Settings {
  default_voice: string | null;
  decoder: string | null;
  mcp_allow_cloning: boolean;
  keep_engine_running: boolean;
  laci: boolean;
  /** "auto" uses the graphics card when Booth can; "cpu" always the processor. */
  device: Device;
}

export interface Overview {
  engine: EngineState;
  packs: { pack: Pack; label: string; present: boolean }[];
  models_dir: string;
  takes_dir: string;
  home: string;
  engine_dir: string;
  settings: Settings;
}

export interface SystemCheck {
  /** An NVIDIA card (Windows, Linux) or the Apple chip (macOS). */
  gpu: string | null;
  vram_gb: number | null;
  gpu_kind: "nvidia" | "apple" | "none";
  cpu: string | null;
  cpu_threads: number;
  ram_gb: number;
  disk: string | null;
  disk_free_gb: number | null;
  os: string | null;
  /** "windows", "macos" or "linux". */
  platform: string;
  engine_found: boolean;
}

export interface UpdateInfo {
  version: string;
  current: string;
  notes: string | null;
}

export interface UpdateProgress {
  done_bytes: number;
  total_bytes: number | null;
}

export interface PackInfo {
  pack: Pack;
  label: string;
  present: boolean;
  bytes: number;
  files: string[];
}

export interface DownloadEvent {
  pack: Pack;
  file: string;
  done_bytes: number;
  total_bytes: number;
  rate: number;
  index: number;
  count: number;
}

export interface Voice {
  id: string;
  name: string;
  kind: "default" | "clone" | "designed";
  mode: SpeakerMode;
  language: string | null;
  accent: string | null;
  gender: string | null;
  description: string | null;
  created_at: string | null;
  speaker_id: string | null;
  modes: SpeakerMode[];
  controls: Record<string, number>;
  transcript: string | null;
  dir: string | null;
}

export interface ScriptLine {
  voice: string;
  text: string;
}

export interface Take {
  id: string;
  text: string;
  voice_id: string;
  voice_name: string;
  created_at: string;
  seconds: number;
  seed: number | null;
  mode: Mode | null;
  controls: Record<string, number>;
  decoder: string | null;
  via: string;
  parts: ScriptLine[];
  starred: boolean;
  path: string;
}

export interface Control {
  id: string;
  group: string;
  label: string;
  type: "bipolar" | "unipolar";
  poles: [string, string] | null;
  map: Record<string, Record<string, number>>;
  range: [number, number] | null;
  safe: [number, number] | null;
  effect: string | null;
  shapeable_only: boolean;
}

export interface Accent {
  id: string;
  label: string;
  tab: string | null;
  safe: [number, number] | null;
}

export interface Catalog {
  groups: { id: string; label: string }[];
  controls: Control[];
  accents: Accent[];
  presets: Record<string, Record<string, number>>;
  preview_text: string | null;
  moods: { id: string; label: string; control: string; dose: number }[];
  directions: string[];
  raw_sets: string[];
}

export interface ClaudeInfo {
  mcpb: string | null;
  exe: string | null;
  desktop_found: boolean;
  connected: boolean;
  config_snippet: unknown;
}

export const api = {
  overview: () => invoke<Overview>("overview"),
  systemCheck: () => invoke<SystemCheck>("system_check"),
  packInfo: () => invoke<PackInfo[]>("pack_info"),
  downloadPacks: (packs: Pack[]) => invoke<void>("download_packs", { packs }),
  engineStart: () => invoke<EngineState>("engine_start"),
  engineStop: () => invoke<boolean>("engine_stop"),
  engineRestart: () => invoke<EngineState>("engine_restart"),
  listVoices: () => invoke<Voice[]>("list_voices"),
  voicePreview: (id: string) => invoke<string>("voice_preview", { id }),
  createClone: (a: {
    name: string;
    audio_base64: string;
    ext: string;
    transcript?: string | null;
    speaker: "me" | "consented";
    exact?: boolean;
  }) => invoke<Voice>("create_clone", { a }),
  designVoice: (a: {
    name: string;
    base: string;
    controls: Record<string, number>;
    description?: string | null;
  }) => invoke<Voice>("design_voice", { a }),
  renameVoice: (id: string, name: string) =>
    invoke<Voice>("rename_voice", { id, name }),
  deleteVoice: (id: string) => invoke<void>("delete_voice", { id }),
  controls: () => invoke<Catalog>("controls"),
  speak: (a: {
    text: string;
    voice?: string | null;
    mode?: Mode | null;
    controls?: Record<string, number>;
    seed?: number | null;
  }) => invoke<Take>("speak", { a }),
  renderScript: (lines: ScriptLine[], gapMs?: number) =>
    invoke<Take>("render_script", { lines, gapMs }),
  exportTakes: (ids: string[], gapMs?: number) =>
    invoke<Take>("export_takes", { ids, gapMs }),
  listTakes: (limit?: number) => invoke<Take[]>("list_takes", { limit }),
  starTake: (id: string, starred: boolean) =>
    invoke<Take>("star_take", { id, starred }),
  deleteTake: (id: string) => invoke<void>("delete_take", { id }),
  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (s: Settings) => invoke<Settings>("save_settings", { s }),
  claudeInfo: () => invoke<ClaudeInfo>("claude_info"),
  /** Show a take or Booth folder in the file manager. */
  reveal: (path: string) => invoke<void>("reveal", { path }),
  /** Open booth.mcpb in Claude Desktop (it asks the user to confirm). */
  installMcpb: () => invoke<void>("install_mcpb"),
  onDownload: (cb: (e: DownloadEvent) => void): Promise<UnlistenFn> =>
    listen<DownloadEvent>("download", (ev) => cb(ev.payload)),
  version: () => getVersion(),
  /** A newer Booth on GitHub, or null. */
  checkUpdate: () => invoke<UpdateInfo | null>("check_update"),
  /** Download, verify and install the update, then restart Booth. */
  installUpdate: () => invoke<void>("install_update"),
  onUpdateProgress: (cb: (e: UpdateProgress) => void): Promise<UnlistenFn> =>
    listen<UpdateProgress>("update-progress", (ev) => cb(ev.payload)),
};

/** Where the engine runs, in words: "graphics card" or "processor". */
export const accelWhere = (a: Accel | undefined) => (a === "cpu" ? "processor" : "graphics card");

/** A local file as a URL the web view can load (asset protocol). */
export const fileUrl = (path: string) => convertFileSrc(path);

export const errText = (e: unknown) =>
  typeof e === "string" ? e : e instanceof Error ? e.message : String(e);

export const gb = (bytes: number) => `${(bytes / 1e9).toFixed(2)} GB`;

/** Short description of a voice, e.g. "English · British · male". */
export const voiceDesc = (v: Voice) => {
  if (v.kind === "default") {
    return [v.language, v.accent, v.gender].filter(Boolean).join(" · ");
  }
  if (v.description) {
    return v.description;
  }
  return v.kind === "clone" ? "Your clone" : "Designed voice";
};

export const canExact = (v: Voice) =>
  v.modes.length === 0 || v.modes.includes("reference");
export const canShape = (v: Voice) =>
  v.modes.length === 0 || v.modes.includes("embedding");
