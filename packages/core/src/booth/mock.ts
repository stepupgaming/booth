// Preview mode: when Booth's UI runs in a plain browser (no Tauri), this stands in for the Rust
// backend with sample data, so the screens can be worked on without the engine or a GPU.
// Nothing here is spoken: "takes" point at the bundled Rupert sample.
import { emit } from "@tauri-apps/api/event";
import { mockIPC } from "@tauri-apps/api/mocks";
import type {
  Catalog,
  Control,
  Overview,
  PackInfo,
  ParsedScript,
  ScriptJob,
  Settings,
  Take,
  Voice,
} from "@workspace/core/booth/api";

const SAMPLE = "/sample-rupert.mp3";

const voice = (
  id: string,
  name: string,
  language: string,
  accent: string,
  gender: string
): Voice => ({
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
  ctl(
    "projection",
    "delivery",
    "Projection",
    "bipolar",
    [-1.5, 2],
    ["whisper", "loud"]
  ),
  ctl("pace", "delivery", "Pace", "bipolar", [-2, 2], ["slower", "faster"]),
  ctl("chatty", "delivery", "Style", "bipolar", [-4, 4], ["read", "chatty"]),
  ctl(
    "hesitant",
    "delivery",
    "Hesitation",
    "bipolar",
    [-8, 8],
    ["fluent", "hesitant"]
  ),
  ctl(
    "lively",
    "delivery",
    "Liveliness",
    "bipolar",
    [-8, 8],
    ["flat", "lively"]
  ),
  ctl("emphasis", "delivery", "Emphasis", "unipolar", [0, 6]),
  ctl("angry", "mood", "Angry", "unipolar", [0, 4], null, "emotions.angry"),
  ctl("happy", "mood", "Happy", "unipolar", [0, 4], null, "emotions.happy"),
  ctl("sad", "mood", "Sad", "unipolar", [0, 6], null, "emotions.sad"),
  ctl("calm", "mood", "Calm", "unipolar", [0, 6], null, "emotions.serene"),
  ctl("scared", "mood", "Scared", "unipolar", [0, 12], null, "emotions.scared"),
  ctl(
    "confused",
    "mood",
    "Confused",
    "unipolar",
    [0, 3],
    null,
    "emotions.confused"
  ),
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
    ...[
      "american",
      "british",
      "scottish",
      "irish",
      "welsh",
      "australian",
      "indian",
    ].map((a) => ({
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
    ).map(([id, label]) => ({
      id,
      label,
      tab: "World",
      safe: [0, 1.5] as [number, number],
    })),
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
  {
    pack: "core",
    label: "Core voices",
    present: true,
    bytes: 1.16e9,
    files: ["a", "b", "c"],
  },
  {
    pack: "gpu_engine",
    label: "NVIDIA graphics engine",
    present: false,
    bytes: 0.74e9,
    files: ["zonos3-server"],
  },
  {
    pack: "full_decoder",
    label: "Studio decoder",
    present: false,
    bytes: 2.2e9,
    files: ["a"],
  },
  {
    pack: "exact",
    label: "High-fidelity cloning",
    present: false,
    bytes: 5.6e9,
    files: ["a", "b"],
  },
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
const take = (
  text: string,
  v: string,
  seed: number | null,
  controls: Record<string, number> = {}
): Take => ({
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
  // no `parts`: like the real app, which leaves it out unless the take is a rendered script
  starred: false,
  path: SAMPLE,
});

const keep = (t: Take) => (takes.unshift(t), t);

type Args = Record<string, any>;

/** The preview URL's flags, read when used (not on the server, where there is no location). */
const q = {
  has: (k: string) =>
    typeof location !== "undefined" &&
    new URLSearchParams(location.search).has(k),
};

/** ?cpu: a PC without an NVIDIA card; ?mac: an Apple Silicon Mac. */
const systemCheck = () => {
  const base = {
    cpu: "AMD Ryzen 9 9900X 12-Core Processor",
    cpu_threads: 24,
    ram_gb: 64,
    disk_free_gb: 412,
    engine_found: true,
  };
  if (q.has("mac")) {
    return {
      ...base,
      gpu: "Apple M3 Pro",
      vram_gb: 36,
      gpu_kind: "apple",
      cpu: "Apple M3 Pro",
      cpu_threads: 12,
      ram_gb: 36,
      disk: "/",
      os: "macOS 15.1",
      platform: "macos",
    };
  }
  if (q.has("android")) {
    return {
      ...base,
      cpu: null,
      cpu_threads: 8,
      ram_gb: 12,
      disk_free_gb: 6,
      gpu: null,
      vram_gb: null,
      gpu_kind: "none",
      disk: "/data",
      os: "Android 17 on SM-S938U",
      platform: "android",
    };
  }
  if (q.has("cpu")) {
    return {
      ...base,
      gpu: null,
      vram_gb: null,
      gpu_kind: "none",
      disk: "C:",
      os: "Windows 11",
      platform: "windows",
    };
  }
  return {
    ...base,
    gpu: "NVIDIA GeForce RTX 5090",
    vram_gb: 32,
    gpu_kind: "nvidia",
    disk: "C:",
    os: "Windows 11",
    platform: "windows",
  };
};

// ---------- scripts (a rough stand-in for booth-core script.rs and batch.rs) ----------

const NOTE =
  /^(\[.*\]|\(.*\)|\{.*\}|(sfx|b-?roll|music|visual|cut to|on screen|graphic|scene)\b.*)$/i;
const LABEL =
  /^\s*(?:\*\*)?([A-Za-z][\w .'-]{0,24}?)(?:\s*\((?:V\.?O\.?|O\.?S\.?)\))?(?:\*\*)?\s*:\s*(.+)$/;

/** [pause], [pause 2s], (pause 500ms): like script.rs, 1 s by default, 0.1 to 10 s. */
const PAUSE =
  /[[(]\s*pause(?:\s+(\d+(?:\.\d+)?)\s*(ms|s|secs?|seconds?)?)?\s*[\])]/gi;

/** Subtitle cue numbers and timings, which the preview drops when opening .srt and .vtt. */
const CUE_LINE = /^\d+$|-->|^WEBVTT/;
const SCAN = /scan/i;

function pauseMs(n?: string, unit?: string): number {
  const v =
    n === undefined
      ? 1000
      : Number(n) * (unit?.toLowerCase() === "ms" ? 1 : 1000);
  return Math.min(10_000, Math.max(100, Math.round(v)));
}

/** A line cut at its pause tags: each piece with the pause before it. */
function splitPauses(line: string): [number, string][] {
  const out: [number, string][] = [];
  let pause = 0;
  let from = 0;
  for (const m of line.matchAll(PAUSE)) {
    out.push([pause, line.slice(from, m.index)]);
    pause = pauseMs(m[1], m[2]);
    from = (m.index ?? 0) + m[0].length;
  }
  out.push([pause, line.slice(from)]);
  return out;
}

function parseScript(text: string): ParsedScript {
  const out: ParsedScript = { lines: [], skipped: [], speakers: [] };
  const counts = new Map<string, number>();
  let para = false;
  let pending = 0;
  for (const raw of text.split(/\r?\n/)) {
    const line = raw
      .replace(/\*\*|__/g, "")
      .replace(/^\s*[-*•]\s+/, "")
      .trim();
    if (!line) {
      para = out.lines.length > 0;
      continue;
    }
    if (!line.replace(PAUSE, "").trim()) {
      for (const [p] of splitPauses(line).slice(1)) {
        pending = Math.min(10_000, pending + p);
      }
      continue;
    }
    if (line.startsWith("#") || NOTE.test(line)) {
      out.skipped.push(line);
      continue;
    }
    const m = LABEL.exec(line);
    const speaker =
      m && /^[A-Z][A-Z .'-]+$|^\w+$/.test(m[1] ?? "")
        ? (m[1] ?? "").trim()
        : null;
    const body = speaker && m ? (m[2] ?? "") : line;
    for (const [p, piece] of splitPauses(body)) {
      pending = Math.min(10_000, pending + p);
      // Like script.rs: only paragraphs over 180 characters split, at sentence ends.
      const parts =
        piece.length > 180
          ? (piece.match(/[^.!?]+[.!?]+["”’)]*|[^.!?]+$/g) ?? [piece])
          : [piece];
      for (const text of parts) {
        if (text.trim()) {
          const pause = out.lines.length ? pending : 0;
          out.lines.push({
            speaker,
            text: text.trim(),
            para,
            ...(pause ? { pause_ms: pause } : {}),
          });
          para = false;
          pending = 0;
          counts.set(speaker ?? "", (counts.get(speaker ?? "") ?? 0) + 1);
        }
      }
    }
  }
  out.speakers = [...counts.entries()];
  return out;
}

const jobs: ScriptJob[] = [];
let runner: string | null = null;
let stopRun = false;

async function runJob(id: string) {
  const job = jobs.find((j) => j.id === id);
  if (!job || runner) {
    return;
  }
  runner = id;
  stopRun = false;
  job.running = true;
  await emit("script", structuredClone(job));
  for (;;) {
    const line = job.lines.find((l) => l.state === "waiting");
    if (!line || stopRun) {
      break;
    }
    line.state = "making";
    await emit("script", structuredClone(job));
    await wait(q.has("slow") ? 2500 : 900);
    // ?fail makes line 3 fail once, to see the Try again path.
    if (q.has("fail") && job.lines.indexOf(line) === 2 && line.version === 0) {
      line.state = "failed";
      line.error = "The engine stopped while making this line.";
      line.version += 1;
    } else {
      line.state = "done";
      line.seconds = Math.max(1.2, line.text.split(/\s+/).length / 2.6);
      line.version += 1;
      line.path = SAMPLE;
      line.seed = 1000 + line.version;
    }
    await emit("script", structuredClone(job));
  }
  job.running = false;
  runner = null;
  await emit("script", structuredClone(job));
}

function sampleJob(): ScriptJob {
  const src =
    "# Why your phone dies in the cold\n\n[B-roll: phone at 1%]\n\nNARRATOR: You walk outside, it's minus ten, and your phone drops from 40 percent to zero. [pause 1.5s] It isn't broken.\n\nQUOTE: Warm it up, and most of that charge comes back.";
  const p = parseScript(src);
  return {
    id: "2026-11-14_101500_why-your-phone-dies-in-the-cold",
    title: "Why your phone dies in the cold",
    created_at: new Date(Date.now() - 86_400_000).toISOString(),
    lines: p.lines.map((l, i) => ({
      ...l,
      voice: l.speaker === "QUOTE" ? "maya" : "rupert",
      voice_name: l.speaker === "QUOTE" ? "Maya" : "Rupert",
      state: "done",
      seconds: 2 + i,
      seed: 1000 + i,
      version: 1,
      path: SAMPLE,
    })),
    skipped: p.skipped,
    source: src,
    voices: { NARRATOR: "rupert", QUOTE: "maya" },
    via: "app",
    running: false,
    exported: null,
  };
}

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
  const lines = [
    "Welcome back to the channel.",
    "This one took three tries to get right.",
    "Thanks for watching, see you next week.",
  ];
  for (const [i, v] of [
    "rupert",
    "rupert",
    voices[1]?.id ?? "rupert",
    voices[2]?.id ?? "rupert",
  ].entries()) {
    for (const [j, text] of lines.entries()) {
      const t = take(text, v, 1000 + i * 10 + j);
      t.created_at = new Date(
        Date.now() - (i * 3 + j) * 3_600_000
      ).toISOString();
      t.starred = (i + j) % 2 === 0;
      takes.unshift(t);
    }
  }
  const script = take(
    "Welcome back to the channel. Thanks for watching, see you next week.",
    "script",
    null
  );
  script.voice_name = "Rupert + Ava";
  script.starred = true;
  takes.unshift(script);
  if (!jobs.length) {
    jobs.push(sampleJob());
  }
  // A plain browser can load the sample directly; keep "file" paths as they are.
  (
    window as unknown as { __TAURI_INTERNALS__: Record<string, unknown> }
  ).__TAURI_INTERNALS__ ??= {};
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
          return q.has("update")
            ? {
                version: "0.4.1",
                current: "0.4.0",
                notes:
                  "- **Script.** A new tab for whole scripts. Paste a script and Booth\n  splits it into lines.\n- **Captions for free.** Export gives you an `.srt` too.\n- Fixed: cloning without ffmpeg.",
              }
            : null;
        case "install_update":
          for (let i = 1; i <= 10; i++) {
            await wait(150);
            await emit("update-progress", {
              done_bytes: i * 1e6,
              total_bytes: 1e7,
            });
          }
          return null;
        case "read_log":
          return [
            "== booth.log (sample)",
            "2026-11-13 16:00:01.120 app[14666] app 0.2.0 started on android aarch64",
            "2026-11-13 16:00:03.402 app[14666] engine: not running, starting it for app",
            "2026-11-13 16:00:21.871 app[14666] engine: ready, pid 14805, in 18.5 s",
            "2026-11-13 16:00:30.010 app[14666] generate 58 chars, voice default_40a3: sent",
            "2026-11-13 16:00:34.300 app[14666] generate 58 chars, voice default_40a3: ok in 4.3 s, 3.6 s of audio",
            "2026-11-13 16:02:44.206 app[14666] ui: app went to the background",
          ].join("\n");
        case "log_note":
          return null;
        case "engine_restart":
          return overview().engine;
        case "claude_info":
          return {
            mcpb: "booth.mcpb",
            exe: "C:\\Program Files\\Booth\\booth-mcp.exe",
            desktop_found: true,
            connected: false,
            config_snippet: {
              mcpServers: {
                booth: { command: "C:\\Program Files\\Booth\\booth-mcp.exe" },
              },
            },
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
          return keep(
            take(
              p.a.text,
              p.a.voice ?? "rupert",
              p.a.seed ?? null,
              p.a.controls
            )
          );
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
          const v: Voice = {
            ...voice(`designed-${Date.now()}`, p.a.name, "English", "", ""),
            kind: "designed",
            modes: ["embedding"],
          };
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
          return [...takes]
            .sort((a, b) => b.created_at.localeCompare(a.created_at))
            .slice(0, p.limit ?? 50);
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
        case "script_parse":
          return parseScript(p.text);
        case "script_open": {
          // The preview reads plain-text kinds; the app reads the rest in Rust (booth-core doc.rs).
          const name = String(p.name);
          const ext = name.toLowerCase().split(".").pop() ?? "";
          if (ext === "pdf" && SCAN.test(name)) {
            throw "This PDF is scanned images, so there's no text to read.";
          }
          if (
            ![
              "txt",
              "text",
              "md",
              "markdown",
              "srt",
              "vtt",
              "fountain",
            ].includes(ext)
          ) {
            throw `The preview can't read .${ext} files. The app can.`;
          }
          const bytes = Uint8Array.from(atob(p.data ?? ""), (c) =>
            c.charCodeAt(0)
          );
          let text = new TextDecoder().decode(bytes);
          if (ext === "srt" || ext === "vtt") {
            text = text
              .split("\n")
              .filter((l) => !CUE_LINE.test(l.trim()))
              .join("\n");
          }
          await wait(250);
          return text.trim();
        }
        case "script_create": {
          const parsed = parseScript(p.text);
          if (!parsed.lines.length) {
            throw "there's nothing to say in this script";
          }
          const title = (
            parsed.skipped
              .find((s) => s.startsWith("#"))
              ?.replace(/^#+\s*/, "") ??
            parsed.lines[0]?.text ??
            "Script"
          ).slice(0, 60);
          const pick = (s: string | null) =>
            voices.find((v) => v.id === (p.voices[s ?? ""] || p.fallback)) ??
            (voices[0] as Voice);
          const job: ScriptJob = {
            id: `${new Date().toISOString().slice(0, 19).replace(/[-:T]/g, "")}-${jobs.length}`,
            title,
            created_at: new Date().toISOString(),
            lines: parsed.lines.map((l) => ({
              ...l,
              voice: pick(l.speaker).id,
              voice_name: pick(l.speaker).name,
              state: "waiting",
              seconds: 0,
              seed: null,
              version: 0,
              path: null,
            })),
            skipped: parsed.skipped,
            source: p.text,
            voices: p.voices,
            via: "app",
            running: true,
            exported: null,
          };
          jobs.unshift(job);
          void runJob(job.id);
          return structuredClone(job);
        }
        case "script_run":
          void runJob(p.id);
          return structuredClone(jobs.find((j) => j.id === p.id));
        case "script_stop":
          stopRun = true;
          return null;
        case "script_redo": {
          const job = jobs.find((j) => j.id === p.id);
          const line = job?.lines[p.index];
          if (!job || !line) {
            throw "no such line";
          }
          if (typeof p.text === "string" && p.text.trim()) {
            line.text = p.text.trim();
          }
          line.state = "waiting";
          line.error = null;
          job.exported = null;
          await emit("script", structuredClone(job));
          void runJob(job.id);
          return structuredClone(job);
        }
        case "script_get":
          return structuredClone(jobs.find((j) => j.id === p.id));
        case "script_list":
          return structuredClone(jobs.slice(0, p.limit ?? 20));
        case "script_captions": {
          const job = jobs.find((j) => j.id === p.id);
          let t = 0;
          const stamp = (s: number) => {
            const ms = Math.round(s * 1000);
            const pad = (n: number, w = 2) => String(n).padStart(w, "0");
            return `${pad(Math.floor(ms / 3_600_000))}:${pad(Math.floor(ms / 60_000) % 60)}:${pad(Math.floor(ms / 1000) % 60)},${pad(ms % 1000, 3)}`;
          };
          return (job?.lines ?? [])
            .map((l, i) => {
              t += i
                ? (l.para ? p.options.para_gap_ms : p.options.gap_ms) / 1000
                : 0;
              const cue = `${i + 1}\n${stamp(t)} --> ${stamp(t + l.seconds)}\n${l.text}`;
              t += l.seconds;
              return cue;
            })
            .join("\n\n");
        }
        case "script_export": {
          const job = jobs.find((j) => j.id === p.id);
          if (!job) {
            throw "no such script";
          }
          await wait(500);
          job.exported = {
            folder: `C:\\Users\\you\\Booth\\scripts\\${job.id}\\export`,
            voiceover: p.options.voiceover ? SAMPLE : null,
            captions: p.options.captions
              ? `C:\\Users\\you\\Booth\\scripts\\${job.id}\\export\\${job.title}.srt`
              : null,
            lines: p.options.lines
              ? job.lines.map(
                  (_, i) => `lines\\${String(i + 1).padStart(2, "0")}.wav`
                )
              : [],
            take_id: "take-script",
            seconds: job.lines.reduce((s, l) => s + l.seconds, 0),
          };
          await emit("script", structuredClone(job));
          return job.exported;
        }
        case "share_files":
          await wait(300);
          return null;
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
  (
    window as unknown as {
      __TAURI_INTERNALS__: { convertFileSrc: (p: string) => string };
    }
  ).__TAURI_INTERNALS__.convertFileSrc = (path: string) => path;
}
