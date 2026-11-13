// Preview mode: when Booth's UI runs in a plain browser (no Tauri), this stands in for the Rust
// backend with sample data, so the screens can be worked on without the engine or a GPU.
// Nothing here is spoken: "takes" point at the bundled Rupert sample.
import { emit } from "@tauri-apps/api/event";
import { mockIPC } from "@tauri-apps/api/mocks";
import type { Catalog, Control, Overview, PackInfo, Settings, Take, Voice } from "@workspace/core/booth/api";

const SAMPLE = "/sample-rupert.mp3";

const voice = (id: string, name: string, language: string, accent: string, gender: string): Voice => ({
  id,
  name,
  kind: "default",
  mode: "reference",
  language,
  accent,
  gender,
  description: null,
  created_at: null,
  speaker_id: id,
  modes: ["reference", "embedding"],
  controls: {},
  transcript: null,
  dir: null,
});

const voices: Voice[] = [
  voice("rupert", "Rupert", "English", "British", "male"),
  voice("maya", "Maya", "English", "American", "female"),
  voice("ines", "Inês", "Portuguese", "Brazilian", "female"),
  voice("kofi", "Kofi", "English", "Ghanaian", "male"),
  voice("lena", "Lena", "German", "Standard", "female"),
  voice("aarav", "Aarav", "Hindi", "Delhi", "male"),
  voice("sofia", "Sofía", "Spanish", "Mexican", "female"),
  voice("theo", "Theo", "English", "Australian", "male"),
  {
    ...voice("my-voice", "My voice", "English", "", ""),
    kind: "clone",
    modes: ["embedding"],
    description: "Quick clone from a voice note",
  },
];

const ctl = (
  id: string,
  group: string,
  label: string,
  type: "bipolar" | "unipolar",
  safe: [number, number],
  poles: [string, string] | null = null,
  dir = `${group}.${id}`
): Control => ({
  id,
  group,
  label,
  type,
  poles,
  map: { "+": { [dir]: 1 } },
  range: [safe[0] * 2, safe[1] * 2],
  safe,
  effect: null,
  shapeable_only: ["who", "texture", "room"].includes(group),
});

const controls: Control[] = [
  ctl("gender", "who", "Gender", "bipolar", [-2, 2], ["masculine", "feminine"]),
  ctl("age", "who", "Age", "bipolar", [-2, 2], ["younger", "older"]),
  ctl("pitch", "who", "Pitch", "bipolar", [-2, 2], ["lower", "higher"]),
  ctl("range", "who", "Range", "bipolar", [-2, 2], ["monotone", "melodic"]),
  ...["deep", "bright", "husky", "raspy", "soft", "authority"].map((t) =>
    ctl(t, "texture", t[0]?.toUpperCase() + t.slice(1), "unipolar", [0, 4])
  ),
  ctl("room", "room", "Room", "bipolar", [-2, 4], ["phone", "studio"]),
  ctl("projection", "delivery", "Projection", "bipolar", [-1.5, 2], ["whisper", "loud"]),
  ctl("pace", "delivery", "Pace", "bipolar", [-2, 2], ["slower", "faster"]),
  ctl("chatty", "delivery", "Style", "bipolar", [-4, 4], ["read", "chatty"]),
  ctl("hesitant", "delivery", "Hesitation", "bipolar", [-8, 8], ["fluent", "hesitant"]),
  ctl("lively", "delivery", "Liveliness", "bipolar", [-8, 8], ["flat", "lively"]),
  ctl("emphasis", "delivery", "Emphasis", "unipolar", [0, 6]),
  ctl("angry", "mood", "Angry", "unipolar", [0, 4], null, "emotions.angry"),
  ctl("happy", "mood", "Happy", "unipolar", [0, 4], null, "emotions.happy"),
  ctl("sad", "mood", "Sad", "unipolar", [0, 6], null, "emotions.sad"),
  ctl("calm", "mood", "Calm", "unipolar", [0, 6], null, "emotions.serene"),
  ctl("scared", "mood", "Scared", "unipolar", [0, 12], null, "emotions.scared"),
  ctl("confused", "mood", "Confused", "unipolar", [0, 3], null, "emotions.confused"),
];

const catalog: Catalog = {
  groups: [
    { id: "who", label: "Who" },
    { id: "texture", label: "Texture" },
    { id: "room", label: "Room" },
    { id: "delivery", label: "Delivery" },
    { id: "mood", label: "Mood" },
  ],
  controls,
  accents: [
    ...["american", "british", "scottish", "irish", "welsh", "australian", "indian"].map((a) => ({
      id: a,
      label: a[0]?.toUpperCase() + a.slice(1),
      tab: "English",
      safe: [0, 1.5] as [number, number],
    })),
    ...(
      [
        ["spa", "Spanish"],
        ["fra", "French"],
        ["deu", "German"],
        ["mandarin", "Mandarin"],
      ] as const
    ).map(([id, label]) => ({ id, label, tab: "World", safe: [0, 1.5] as [number, number] })),
  ],
  presets: {
    "Deep narrator": { deep: 2.5, age: 0.8, pace: -0.6 },
    "Bright host": { bright: 2, lively: 3, pace: 0.5 },
    "Old storyteller": { age: 1.8, husky: 1.5, pace: -1 },
    "Radio voice": { authority: 2.5, room: 3, projection: 1 },
  },
  preview_text: "Hello, this is a preview of my voice.",
  moods: [
    { id: "happy", label: "Happy", control: "happy", dose: 4 },
    { id: "calm", label: "Calm", control: "calm", dose: 4 },
    { id: "sad", label: "Sad", control: "sad", dose: 6 },
    { id: "angry", label: "Angry", control: "angry", dose: 4 },
    { id: "scared", label: "Scared", control: "scared", dose: 12 },
    { id: "confused", label: "Confused", control: "confused", dose: 3 },
  ],
  directions: [
    "emotions.angry",
    "emotions.happy",
    "emotions.sad",
    "emotions.serene",
    "emotions.scared",
    "emotions.confused",
    "emotions.amused",
    "emotions.tender",
    "emotions.proud",
    "emotions.disgusted",
    "delivery.pitch",
  ],
  raw_sets: [],
};

let settings: Settings = {
  default_voice: "rupert",
  decoder: "keep2_plain",
  mcp_allow_cloning: false,
  keep_engine_running: false,
  laci: false,
  device: "auto",
  language: null,
};

const packs: PackInfo[] = [
  { pack: "core", label: "Core voices", present: true, bytes: 1.16e9, files: ["a", "b", "c"] },
  { pack: "gpu_engine", label: "NVIDIA graphics engine", present: false, bytes: 0.74e9, files: ["zonos3-server"] },
  { pack: "full_decoder", label: "Studio decoder", present: false, bytes: 2.2e9, files: ["a"] },
  { pack: "exact", label: "High-fidelity cloning", present: false, bytes: 5.6e9, files: ["a", "b"] },
];

const overview = (): Overview => ({
  engine: { state: "stopped" },
  packs: packs.map(({ pack, label, present }) => ({ pack, label, present })),
  models_dir: "C:\\Users\\you\\AppData\\Local\\NoFaceTube\\Booth\\models",
  takes_dir: "C:\\Users\\you\\Documents\\Booth takes",
  home: "C:\\Users\\you\\AppData\\Local\\NoFaceTube\\Booth",
  engine_dir: "C:\\Program Files\\Booth\\engine",
  settings,
});

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));
let takeN = 0;
const takes: Take[] = [];
const take = (text: string, v: string, seed: number | null, controls: Record<string, number> = {}): Take => ({
  id: `t${++takeN}`,
  text,
  voice_id: v,
  voice_name: voices.find((x) => x.id === v)?.name ?? v,
  created_at: new Date().toISOString(),
  seconds: 2 + text.length / 14,
  seed,
  mode: "exact",
  controls,
  decoder: settings.decoder,
  via: "app",
  parts: [],
  starred: false,
  path: SAMPLE,
});

const keep = (t: Take) => (takes.unshift(t), t);

type Args = Record<string, any>;

/** The preview URL's flags, read when used (not on the server, where there is no location). */
const q = { has: (k: string) => typeof location !== "undefined" && new URLSearchParams(location.search).has(k) };

/** ?cpu: a PC without an NVIDIA card; ?mac: an Apple Silicon Mac. */
const systemCheck = () => {
  const base = { cpu: "AMD Ryzen 9 9900X 12-Core Processor", cpu_threads: 24, ram_gb: 64, disk_free_gb: 412, engine_found: true };
  if (q.has("mac")) {
    return { ...base, gpu: "Apple M3 Pro", vram_gb: 36, gpu_kind: "apple", cpu: "Apple M3 Pro", cpu_threads: 12, ram_gb: 36, disk: "/", os: "macOS 15.1", platform: "macos" };
  }
  if (q.has("android")) {
    return { ...base, cpu: null, cpu_threads: 8, ram_gb: 12, disk_free_gb: 6, gpu: null, vram_gb: null, gpu_kind: "none", disk: "/data", os: "Android 17 on SM-S938U", platform: "android" };
  }
  if (q.has("cpu")) {
    return { ...base, gpu: null, vram_gb: null, gpu_kind: "none", disk: "C:", os: "Windows 11", platform: "windows" };
  }
  return { ...base, gpu: "NVIDIA GeForce RTX 5090", vram_gb: 32, gpu_kind: "nvidia", disk: "C:", os: "Windows 11", platform: "windows" };
};

export function installMock() {
  // ?setup shows the first-run screen (no packs downloaded yet).
  if (new URLSearchParams(location.search).has("setup")) {
    for (const p of packs) {
      p.present = false;
    }
  }
  // ?android: a phone, which only gets the core pack.
  if (new URLSearchParams(location.search).has("android")) {
    packs.splice(1);
  }
  // A few takes for the Library, some saved.
  const lines = ["Welcome back to the channel.", "This one took three tries to get right.", "Thanks for watching, see you next week."];
  for (const [i, v] of ["rupert", "rupert", voices[1]?.id ?? "rupert", voices[2]?.id ?? "rupert"].entries()) {
    for (const [j, text] of lines.entries()) {
      const t = take(text, v, 1000 + i * 10 + j);
      t.created_at = new Date(Date.now() - (i * 3 + j) * 3_600_000).toISOString();
      t.starred = (i + j) % 2 === 0;
      takes.unshift(t);
    }
  }
  const script = take("Welcome back to the channel. Thanks for watching, see you next week.", "script", null);
  script.voice_name = "Rupert + Ava";
  script.starred = true;
  takes.unshift(script);
  // A plain browser can load the sample directly; keep "file" paths as they are.
  (window as unknown as { __TAURI_INTERNALS__: Record<string, unknown> }).__TAURI_INTERNALS__ ??= {};
  mockIPC(
    async (cmd, raw) => {
      const p = (raw ?? {}) as Args;
      switch (cmd) {
        case "overview":
          return overview();
        case "system_check":
          return systemCheck();
        case "plugin:app|version":
          return "0.2.0";
        case "check_update":
          return q.has("update") ? { version: "0.2.1", current: "0.2.0", notes: null } : null;
        case "install_update":
          for (let i = 1; i <= 10; i++) {
            await wait(150);
            await emit("update-progress", { done_bytes: i * 1e6, total_bytes: 1e7 });
          }
          return null;
        case "engine_restart":
          return overview().engine;
        case "claude_info":
          return {
            mcpb: "booth.mcpb",
            exe: "C:\\Program Files\\Booth\\booth-mcp.exe",
            desktop_found: true,
            connected: false,
            config_snippet: { mcpServers: { booth: { command: "C:\\Program Files\\Booth\\booth-mcp.exe" } } },
          };
        case "pack_info":
          return packs;
        case "list_voices":
          return voices;
        case "controls":
          return catalog;
        case "get_settings":
          return settings;
        case "save_settings":
          settings = { ...p.s };
          return settings;
        case "voice_preview":
          await wait(400);
          return SAMPLE;
        case "speak":
          await wait(700);
          return keep(take(p.a.text, p.a.voice ?? "rupert", p.a.seed ?? null, p.a.controls));
        case "export_takes":
          return keep({ ...take("export", "script", null), starred: true });
        case "create_clone": {
          await wait(900);
          const v: Voice = {
            ...voice(`clone-${Date.now()}`, p.a.name, "English", "", ""),
            kind: "clone",
            modes: p.a.exact ? ["reference", "embedding"] : ["embedding"],
          };
          voices.unshift(v);
          return v;
        }
        case "design_voice": {
          const v: Voice = { ...voice(`designed-${Date.now()}`, p.a.name, "English", "", ""), kind: "designed", modes: ["embedding"] };
          voices.unshift(v);
          return v;
        }
        case "rename_voice": {
          const v = voices.find((x) => x.id === p.id);
          if (v) {
            v.name = p.name;
          }
          return v;
        }
        case "delete_voice": {
          const i = voices.findIndex((x) => x.id === p.id);
          if (i >= 0) {
            voices.splice(i, 1);
          }
          return null;
        }
        case "download_packs":
          for (const pk of p.packs as string[]) {
            const info = packs.find((x) => x.pack === pk);
            if (!info) {
              continue;
            }
            for (let i = 1; i <= 20; i++) {
              await wait(120);
              await emit("download", {
                pack: pk,
                file: "model.safetensors",
                done_bytes: (info.bytes * i) / 20,
                total_bytes: info.bytes,
                rate: 48e6,
                index: 0,
                count: 1,
              });
            }
            info.present = true;
          }
          return null;
        case "list_takes":
          return [...takes].sort((a, b) => b.created_at.localeCompare(a.created_at)).slice(0, p.limit ?? 50);
        case "star_take": {
          const t = takes.find((x) => x.id === p.id);
          if (!t) {
            throw "no such take";
          }
          t.starred = p.starred;
          return t;
        }
        case "delete_take": {
          const i = takes.findIndex((x) => x.id === p.id);
          if (i >= 0) {
            takes.splice(i, 1);
          }
          return null;
        }
        case "engine_stop":
          return true;
        case "reveal":
        case "install_mcpb":
          return null;
        default:
          throw `${cmd} isn’t available in preview mode`;
      }
    },
    { shouldMockEvents: true }
  );
  // Sample paths are already URLs here.
  (window as unknown as { __TAURI_INTERNALS__: { convertFileSrc: (p: string) => string } }).__TAURI_INTERNALS__.convertFileSrc = (
    path: string
  ) => path;
}
