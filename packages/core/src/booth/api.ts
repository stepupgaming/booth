// Typed wrappers for the Tauri commands in apps/native/src-tauri/src/lib.rs.
import { getVersion } from "@tauri-apps/api/app";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type Pack = "core" | "gpu_engine" | "vulkan_engine" | "exact" | "full_decoder";
/** What the engine computes on: an NVIDIA card, Apple's Metal, an AMD or Intel card (Vulkan, beta), or the processor. */
export type Accel = "cuda" | "metal" | "vulkan" | "cpu";
export type Device = "auto" | "cpu" | "vulkan";
export type Mode = "exact" | "shapeable";
export type SpeakerMode = "reference" | "embedding";

export type EngineState =
  | { state: "no_engine"; looked_in: string }
  | { state: "no_models"; looked_in: string }
  | { state: "stopped" }
  | {
      state: "running";
      port: number;
      pid: number;
      packs: Pack[];
      accel: Accel;
    };

export interface Settings {
  default_voice: string | null;
  decoder: string | null;
  mcp_allow_cloning: boolean;
  keep_engine_running: boolean;
  laci: boolean;
  /** "auto" uses the graphics card when Booth can; "cpu" always the processor. */
  device: Device;
  /** Language of the text (e.g. "es", "fr_fr"); null = US English. */
  language: string | null;
  /** Words the voice says differently (booth-core pronounce.rs). */
  pronunciations: Pronunciation[];
}

/** One entry of the pronunciation list: where the text has `write`, the voice says `say`. */
export interface Pronunciation {
  /** Only for this language (e.g. "es", "en_gb"); missing = every language. */
  language?: string | null;
  match_case: boolean;
  /** How to say it, spelled the way it sounds. */
  say: string;
  /** The word or phrase as written. */
  write: string;
}

/** A pasted script split into the lines Booth speaks (booth-core script.rs). */
export interface ParsedScript {
  lines: { speaker: string | null; text: string; para: boolean }[];
  /** Headings and notes left out of the audio. */
  skipped: string[];
  /** Speaker label ("" = no label) and how many lines it has, in order of appearance. */
  speakers: [string, number][];
}

export type LineState = "waiting" | "making" | "done" | "failed";

export interface ScriptJobLine {
  speaker: string | null;
  voice: string;
  voice_name: string;
  text: string;
  para: boolean;
  state: LineState;
  seconds: number;
  seed: number | null;
  version: number;
  voice_match?: number | null;
  drifted?: boolean;
  error?: string | null;
  path: string | null;
}

export interface ScriptExported {
  folder: string;
  voiceover: string | null;
  captions: string | null;
  lines: string[];
  take_id: string | null;
  seconds: number;
}

/** A script being made line by line (booth-core batch.rs). */
export interface ScriptJob {
  id: string;
  title: string;
  created_at: string;
  lines: ScriptJobLine[];
  skipped: string[];
  source: string;
  voices: Record<string, string>;
  via: string;
  running: boolean;
  exported?: ScriptExported | null;
}

export interface ScriptExportOptions {
  gap_ms: number;
  para_gap_ms: number;
  voiceover: boolean;
  captions: boolean;
  lines: boolean;
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
  /** Only on a rendered script: Rust leaves it out when it's empty. */
  parts?: ScriptLine[];
  /** Sampling temperature, when the take didn't use Zyphra's default. */
  temperature?: number | null;
  /** How close the take sounds to the voice (cosine of voiceprints), when Booth checked. */
  voice_match?: number | null;
  /** How many times Booth made this take again because it sounded unlike the voice. */
  redone?: number;
  /** Still unlike the voice after the retries; this was the closest try. */
  drifted?: boolean;
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
    temperature?: number | null;
  }) => invoke<Take>("speak", { a }),
  renderScript: (lines: ScriptLine[], gapMs?: number) =>
    invoke<Take>("render_script", { lines, gapMs }),
  exportTakes: (ids: string[], gapMs?: number) =>
    invoke<Take>("export_takes", { ids, gapMs }),
  scriptParse: (text: string) => invoke<ParsedScript>("script_parse", { text }),
  /** Save a pasted script (speaker label -> voice id) and start making it in the background. */
  scriptCreate: (
    text: string,
    voices: Record<string, string>,
    fallback: string
  ) => invoke<ScriptJob>("script_create", { text, voices, fallback }),
  scriptRun: (id: string) => invoke<ScriptJob>("script_run", { id }),
  scriptStop: () => invoke<void>("script_stop"),
  scriptRedo: (id: string, index: number, text?: string | null) =>
    invoke<ScriptJob>("script_redo", { id, index, text }),
  scriptGet: (id: string) => invoke<ScriptJob>("script_get", { id }),
  scriptList: (limit?: number) => invoke<ScriptJob[]>("script_list", { limit }),
  scriptCaptions: (id: string, options: ScriptExportOptions) =>
    invoke<string>("script_captions", { id, options }),
  scriptExport: (id: string, options: ScriptExportOptions) =>
    invoke<ScriptExported>("script_export", { id, options }),
  /** Android's share sheet with these Booth files. */
  shareFiles: (paths: string[], title?: string) =>
    invoke<void>("share_files", { paths, title }),
  /** A script's progress, after every line. */
  onScript: (cb: (j: ScriptJob) => void): Promise<UnlistenFn> =>
    listen<ScriptJob>("script", (ev) => cb(ev.payload)),
  onScriptError: (
    cb: (e: { id: string; error: string }) => void
  ): Promise<UnlistenFn> =>
    listen<{ id: string; error: string }>("script-error", (ev) =>
      cb(ev.payload)
    ),
  listTakes: (limit?: number) => invoke<Take[]>("list_takes", { limit }),
  starTake: (id: string, starred: boolean) =>
    invoke<Take>("star_take", { id, starred }),
  deleteTake: (id: string) => invoke<void>("delete_take", { id }),
  /** Say a word in this voice with the saved pronunciation list applied; no take is saved. */
  hearPronunciation: (text: string, voice: string | null) =>
    invoke<string>("hear_pronunciation", { text, voice }),
  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (s: Settings) => invoke<Settings>("save_settings", { s }),
  claudeInfo: () => invoke<ClaudeInfo>("claude_info"),
  /** Show a take or Booth folder in the file manager. */
  reveal: (path: string) => invoke<void>("reveal", { path }),
  /** Open a nofacetube.com or Booth GitHub page in the browser. */
  openUrl: (url: string) => invoke<void>("open_link", { url }),
  /** Open booth.mcpb in Claude Desktop (it asks the user to confirm). */
  installMcpb: () => invoke<void>("install_mcpb"),
  onDownload: (cb: (e: DownloadEvent) => void): Promise<UnlistenFn> =>
    listen<DownloadEvent>("download", (ev) => cb(ev.payload)),
  version: () => getVersion(),
  /** A newer Booth on GitHub, or null. */
  checkUpdate: () => invoke<UpdateInfo | null>("check_update"),
  /** Download, verify and install the update, then restart Booth. */
  installUpdate: () => invoke<void>("install_update"),
  /** booth.log and the engine's log, for Settings → Log. */
  readLog: () => invoke<string>("read_log"),
  /** One line into booth.log. Never throws: logging must not break anything. */
  logNote: (text: string) =>
    invoke<void>("log_note", { text }).catch(() => undefined),
  onUpdateProgress: (cb: (e: UpdateProgress) => void): Promise<UnlistenFn> =>
    listen<UpdateProgress>("update-progress", (ev) => cb(ev.payload)),
};

/** Where the engine runs, in words: "graphics card" or "processor". */
export const accelWhere = (a: Accel | undefined) =>
  a === "cpu" ? "processor" : "graphics card";

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

// The engine's languages (Zyphra's languages.json). English first; the rest by name.
const OTHER_LANGUAGES = [
  ["ar", "Arabic"],
  ["bn", "Bengali"],
  ["bg", "Bulgarian"],
  ["cs", "Czech"],
  ["da", "Danish"],
  ["nl", "Dutch"],
  ["fi", "Finnish"],
  ["fr_fr", "French"],
  ["ka", "Georgian"],
  ["de", "German"],
  ["el", "Greek"],
  ["gu", "Gujarati"],
  ["he", "Hebrew"],
  ["hi", "Hindi"],
  ["hu", "Hungarian"],
  ["id", "Indonesian"],
  ["it", "Italian"],
  ["ja", "Japanese"],
  ["kn", "Kannada"],
  ["ko", "Korean"],
  ["ms", "Malay"],
  ["ml", "Malayalam"],
  ["cmn", "Mandarin"],
  ["mr", "Marathi"],
  ["no", "Norwegian"],
  ["or", "Odia"],
  ["pl", "Polish"],
  ["pt_br", "Portuguese"],
  ["pa", "Punjabi"],
  ["ro", "Romanian"],
  ["ru", "Russian"],
  ["sk", "Slovak"],
  ["es", "Spanish"],
  ["sv", "Swedish"],
  ["tl", "Tagalog"],
  ["ta", "Tamil"],
  ["te", "Telugu"],
  ["th", "Thai"],
  ["tr", "Turkish"],
  ["uk", "Ukrainian"],
  ["ur", "Urdu"],
  ["vi", "Vietnamese"],
].map(([value, label]) => ({ value: value as string, label: label as string }));

/** The Language picker's groups: English first, then the rest by name. */
export const LANGUAGE_GROUPS = [
  {
    label: "English",
    options: [
      { value: "en_us", label: "English (US)" },
      { value: "en_gb", label: "English (UK)" },
    ],
  },
  { label: "Other languages", options: OTHER_LANGUAGES },
];

export const languageName = (code: string) =>
  LANGUAGE_GROUPS.flatMap((g) => g.options).find((o) => o.value === code)
    ?.label ?? code;

const ENGLISH_TEST_LINE =
  "The quick brown fox jumps over the lazy dog, and nobody seems to mind.";

// A line to hear a new voice say, per language. Languages without one ask for a line instead.
const TEST_LINES: Record<string, string> = {
  en_us: ENGLISH_TEST_LINE,
  en_gb: ENGLISH_TEST_LINE,
  es: "Esta es una pequeña prueba de mi nueva voz, leída con calma.",
  fr_fr: "Ceci est un petit essai de ma nouvelle voix, lu sans se presser.",
  de: "Das ist ein kurzer Test meiner neuen Stimme, ganz in Ruhe gelesen.",
  it: "Questa è una breve prova della mia nuova voce, letta con calma.",
  pt_br: "Este é um pequeno teste da minha nova voz, lido com calma.",
  nl: "Dit is een korte test van mijn nieuwe stem, rustig voorgelezen.",
  pl: "To jest krótki test mojego nowego głosu, czytany spokojnie.",
  sv: "Det här är ett kort test av min nya röst, läst i lugn takt.",
  da: "Dette er en kort test af min nye stemme, læst i roligt tempo.",
  no: "Dette er en kort test av min nye stemme, lest i rolig tempo.",
  fi: "Tämä on lyhyt testi uudesta äänestäni, luettuna rauhallisesti.",
  cs: "Tohle je krátký test mého nového hlasu, přečtený v klidu.",
  sk: "Toto je krátky test môjho nového hlasu, prečítaný v pokoji.",
  ro: "Acesta este un scurt test al noii mele voci, citit în ritm liniștit.",
  hu: "Ez egy rövid próba az új hangomról, nyugodtan felolvasva.",
  ru: "Это короткая проверка моего нового голоса, прочитанная спокойно.",
  uk: "Це коротка перевірка мого нового голосу, прочитана спокійно.",
  el: "Αυτή είναι μια σύντομη δοκιμή της νέας μου φωνής, διαβασμένη ήρεμα.",
  tr: "Bu, yeni sesimin sakin bir şekilde okunan kısa bir denemesi.",
  id: "Ini adalah tes singkat untuk suara baru saya, dibaca dengan tenang.",
  ms: "Ini ialah ujian ringkas suara baharu saya, dibaca dengan tenang.",
  vi: "Đây là một bài thử ngắn cho giọng nói mới của tôi, đọc chậm rãi.",
  hi: "यह मेरी नई आवाज़ का एक छोटा सा परीक्षण है, आराम से पढ़ा गया।",
  ja: "これは新しい声の短いテストです。ゆっくり読んでいます。",
  cmn: "这是我新声音的简短测试，用平稳的语速朗读。",
  ko: "이것은 제 새 목소리의 짧은 테스트입니다. 천천히 읽고 있어요.",
};

/** The test line for a language, or "" when Booth has none and the person types their own. */
export const testLineFor = (code: string) => TEST_LINES[code] ?? "";

const ENGLISH_PASSAGE =
  "When the sunlight strikes raindrops in the air, they act as a prism and form a rainbow. " +
  "People look, but no one ever finds it. Some say there is a pot of gold at one end. " +
  "I usually take the long way home, past the old bridge and the bakery on the corner, " +
  "and on a good day I stop for a coffee and just watch the street for a while.";

// A passage to read out loud when recording a voice to clone: about 30 seconds of everyday
// words with a good spread of sounds, per language. Written about other people (no "I"), so
// languages that mark the speaker's gender read the same for everyone.
const PASSAGES: Record<string, string> = {
  en_us: ENGLISH_PASSAGE,
  en_gb: ENGLISH_PASSAGE,
  ar: "في صباح كل سبت، يكون السوق الصغير قرب المحطة مزدحمًا دائمًا. يحمل الناس سلالًا مليئة بالخبز الطازج والتفاح والجبن والزهور. يضحك طفل وهو يطارد حمامة، بينما يشرب صديقان قديمان القهوة ويتجادلان حول الطقس. وقرب الظهيرة تشرق الشمس، وتلمع واجهات المحلات، وتفوح من الشارع كله رائحة السكر الدافئ. وفي المساء تمطر السماء، وتعود المدينة إلى هدوئها.",
  bn: "প্রতি শনিবার সকালে স্টেশনের কাছের ছোট বাজারটা সবসময় মানুষে ভরা থাকে। লোকজন তাজা ফল, সবজি আর ফুলে ভরা ঝুড়ি নিয়ে ঘুরে বেড়ায়। একটা বাচ্চা হাসতে হাসতে একটা পায়রার পেছনে দৌড়ায়, আর দুই পুরোনো বন্ধু চা খেতে খেতে আবহাওয়া নিয়ে তর্ক করে। দুপুরের দিকে রোদ ওঠে, দোকানের কাচ ঝকঝক করে, আর পুরো রাস্তায় গরম গুড়ের গন্ধ ছড়িয়ে পড়ে। সন্ধ্যায় বৃষ্টি নামে, আর শহরটা আবার শান্ত হয়ে যায়।",
  bg: "В събота сутрин малкият пазар до гарата винаги е пълен с хора. Купувачите носят кошници с пресен хляб, ябълки, сирене и цветя. Едно дете се смее и гони гълъб, а двама стари приятели пият кафе и спорят за времето. Към обед излиза слънце, витрините блестят и цялата улица ухае на топла захар. Вечерта вали дъжд и градът отново утихва.",
  cs: "V sobotu ráno je malý trh u nádraží vždycky plný lidí. Nakupující nesou košíky s čerstvým chlebem, jablky, sýrem a květinami. Dítě se směje a honí holuba, zatímco dva staří přátelé pijí kávu a dohadují se o počasí. Kolem poledne vyjde slunce, výlohy se lesknou a celá ulice voní teplým cukrem. Večer prší a město zase utichne.",
  da: "Lørdag morgen er der altid fyldt med mennesker på det lille marked ved stationen. Mange bærer kurve med friskbagt brød, æbler, ost og blomster. Et barn griner og løber efter en due, mens to gamle venner drikker kaffe og småskændes om vejret. Ved middagstid kommer solen frem, butiksvinduerne skinner, og hele gaden dufter af varmt sukker. Om aftenen regner det, og byen bliver stille igen.",
  nl: "Op zaterdagochtend is het marktje bij het station altijd druk. Mensen dragen manden met vers brood, appels, kaas en bloemen. Een kind lacht en rent achter een duif aan, terwijl twee oude vrienden koffie drinken en kibbelen over het weer. Rond het middaguur breekt de zon door, de etalages glanzen en de hele straat ruikt naar warme suiker. ’s Avonds regent het, en dan wordt het weer stil in de stad.",
  fi: "Lauantaiaamuisin aseman lähellä oleva pieni tori on aina täynnä väkeä. Ihmiset kantavat koreja, joissa on tuoretta leipää, omenoita, juustoa ja kukkia. Lapsi nauraa ja jahtaa kyyhkystä, kun kaksi vanhaa ystävää juo kahvia ja kinastelee säästä. Puoliltapäivin aurinko tulee esiin, näyteikkunat kiiltävät ja koko katu tuoksuu lämpimältä sokerilta. Illalla sataa, ja kaupunki hiljenee taas.",
  fr_fr:
    "Le samedi matin, le petit marché près de la gare est toujours plein de monde. Les gens portent des paniers de pain frais, de pommes, de fromage et de fleurs. Un enfant rit et court après un pigeon, pendant que deux vieux amis boivent un café en se chamaillant à propos de la météo. Vers midi, le soleil sort, les vitrines brillent et toute la rue sent le sucre chaud. Le soir, il pleut, et la ville redevient calme.",
  ka: "ყოველ შაბათს დილით სადგურთან ახლოს მდებარე პატარა ბაზარი ყოველთვის ხალხით არის სავსე. მყიდველებს ხელში უჭირავთ კალათები ახალი პურით, ვაშლით, ყველით და ყვავილებით. ბავშვი იცინის და მტრედს დასდევს, ორი ძველი მეგობარი კი ყავას სვამს და ამინდზე კამათობს. შუადღისთვის მზე გამოდის, ვიტრინები ბრწყინავს და მთელ ქუჩას თბილი შაქრის სუნი ასდის. საღამოს წვიმს და ქალაქი ისევ მშვიდდება.",
  de: "Am Samstagmorgen ist der kleine Markt beim Bahnhof immer voller Leute. Sie tragen Körbe mit frischem Brot, Äpfeln, Käse und Blumen. Ein Kind lacht und jagt einer Taube hinterher, während zwei alte Freunde Kaffee trinken und über das Wetter streiten. Gegen Mittag kommt die Sonne heraus, die Schaufenster glänzen, und die ganze Straße riecht nach warmem Zucker. Am Abend regnet es, und in der Stadt wird es wieder still.",
  el: "Το Σάββατο το πρωί, η μικρή λαϊκή αγορά κοντά στον σταθμό είναι πάντα γεμάτη κόσμο. Οι άνθρωποι κουβαλούν καλάθια με φρέσκο ψωμί, μήλα, τυρί και λουλούδια. Ένα παιδί γελάει και κυνηγάει ένα περιστέρι, ενώ δύο παλιοί φίλοι πίνουν καφέ και διαφωνούν για τον καιρό. Γύρω στο μεσημέρι βγαίνει ο ήλιος, οι βιτρίνες λάμπουν και όλος ο δρόμος μυρίζει ζεστή ζάχαρη. Το βράδυ βρέχει, και η πόλη ησυχάζει ξανά.",
  gu: "દર શનિવારે સવારે સ્ટેશન પાસેનું નાનું બજાર હંમેશાં લોકોથી ભરેલું હોય છે. લોકો તાજાં ફળ, શાકભાજી અને ફૂલોથી ભરેલી ટોપલીઓ લઈને ફરે છે. એક નાનું બાળક હસતાં હસતાં કબૂતરની પાછળ દોડે છે, અને બે જૂના મિત્રો ચા પીતાં પીતાં હવામાન વિશે દલીલ કરે છે. બપોર સુધીમાં તડકો નીકળે છે, દુકાનોના કાચ ચમકે છે અને આખી શેરીમાં ગરમ ગોળની સુગંધ ફેલાય છે. સાંજે વરસાદ પડે છે, અને શહેર ફરી શાંત થઈ જાય છે.",
  he: "ביום שישי בבוקר, השוק הקטן ליד התחנה תמיד מלא אנשים. הקונים נושאים סלים עם לחם טרי, תפוחים, גבינה ופרחים. ילד צוחק ורודף אחרי יונה, ושני חברים ותיקים שותים קפה ומתווכחים על מזג האוויר. לקראת הצהריים יוצאת השמש, חלונות הראווה נוצצים, וריח של סוכר חם ממלא את כל הרחוב. בערב יורד גשם, והעיר שוב נרגעת.",
  hi: "हर शनिवार की सुबह स्टेशन के पास वाला छोटा बाज़ार लोगों से भरा रहता है। लोग ताज़े फलों, सब्ज़ियों और फूलों से भरी टोकरियाँ लेकर घूमते हैं। एक बच्चा हँसते हुए कबूतर के पीछे दौड़ता है, और दो पुराने दोस्त चाय पीते हुए मौसम पर बहस करते हैं। दोपहर तक धूप निकल आती है, दुकानों के शीशे चमकने लगते हैं और पूरी गली में गरम गुड़ की खुशबू फैल जाती है। शाम को बारिश होती है, और शहर फिर से शांत हो जाता है।",
  hu: "Szombat reggelente az állomás melletti kis piac mindig tele van emberekkel. A vásárlók kosarakban viszik a friss kenyeret, az almát, a sajtot és a virágot. Egy gyerek nevetve kerget egy galambot, közben két régi barát kávét iszik, és az időjáráson vitatkozik. Délre kisüt a nap, csillognak a kirakatok, és az egész utcát meleg cukor illata lengi be. Este esik az eső, és a város újra elcsendesedik.",
  id: "Setiap Sabtu pagi, pasar kecil di dekat stasiun selalu ramai. Orang-orang membawa keranjang berisi roti segar, buah, sayuran, dan bunga. Seorang anak tertawa sambil mengejar burung merpati, sementara dua sahabat lama minum kopi dan berdebat soal cuaca. Menjelang siang, matahari muncul, kaca etalase toko berkilau, dan seluruh jalan tercium wangi gula hangat. Pada malam hari turun hujan, dan kota kembali sepi.",
  it: "Il sabato mattina il piccolo mercato vicino alla stazione è sempre pieno di gente. Le persone portano cesti di pane fresco, mele, formaggio e fiori. Un bambino ride e insegue un piccione, mentre due vecchi amici bevono un caffè e discutono del tempo. Verso mezzogiorno esce il sole, le vetrine brillano e tutta la strada profuma di zucchero caldo. La sera piove, e la città torna tranquilla.",
  ja: "毎週土曜日の朝、駅の近くの小さな市場は、いつも人でにぎわっています。みんな、焼きたてのパンや果物、野菜、花でいっぱいのかごを持って歩いています。子どもが笑いながらハトを追いかけ、そのそばで古い友だち二人がコーヒーを飲みながら、天気のことで言い合っています。お昼ごろになると日が差してきて、店のガラスがきらきら光り、通りじゅうに温かい砂糖のにおいが広がります。夕方には雨が降り出し、町はまた静かになります。",
  kn: "ಪ್ರತಿ ಶನಿವಾರ ಬೆಳಿಗ್ಗೆ ನಿಲ್ದಾಣದ ಹತ್ತಿರದ ಚಿಕ್ಕ ಮಾರುಕಟ್ಟೆ ಯಾವಾಗಲೂ ಜನರಿಂದ ತುಂಬಿರುತ್ತದೆ. ಜನರು ತಾಜಾ ಹಣ್ಣು, ತರಕಾರಿ ಮತ್ತು ಹೂವುಗಳಿಂದ ತುಂಬಿದ ಬುಟ್ಟಿಗಳನ್ನು ಹಿಡಿದು ಓಡಾಡುತ್ತಾರೆ. ಒಂದು ಮಗು ನಗುತ್ತಾ ಪಾರಿವಾಳದ ಹಿಂದೆ ಓಡುತ್ತದೆ, ಇಬ್ಬರು ಹಳೆಯ ಗೆಳೆಯರು ಚಹಾ ಕುಡಿಯುತ್ತಾ ಹವಾಮಾನದ ಬಗ್ಗೆ ವಾದ ಮಾಡುತ್ತಾರೆ. ಮಧ್ಯಾಹ್ನದ ಹೊತ್ತಿಗೆ ಬಿಸಿಲು ಬರುತ್ತದೆ, ಅಂಗಡಿಗಳ ಗಾಜುಗಳು ಹೊಳೆಯುತ್ತವೆ, ಇಡೀ ಬೀದಿಯಲ್ಲಿ ಬಿಸಿ ಬೆಲ್ಲದ ಪರಿಮಳ ಹರಡುತ್ತದೆ. ಸಂಜೆ ಮಳೆ ಬರುತ್ತದೆ, ಮತ್ತು ನಗರ ಮತ್ತೆ ಶಾಂತವಾಗುತ್ತದೆ.",
  ko: "매주 토요일 아침, 역 근처의 작은 시장은 언제나 사람들로 북적입니다. 사람들은 갓 구운 빵과 과일, 채소, 꽃이 가득 담긴 바구니를 들고 다닙니다. 한 아이가 웃으면서 비둘기를 쫓아가고, 그 옆에서 오랜 친구 두 사람이 커피를 마시며 날씨 이야기로 티격태격합니다. 점심 무렵이 되면 해가 나고, 가게 유리창이 반짝이며, 거리 전체에 따뜻한 설탕 냄새가 퍼집니다. 저녁이 되면 비가 내리고, 도시는 다시 조용해집니다.",
  ms: "Setiap pagi Sabtu, pasar kecil berhampiran stesen sentiasa sesak dengan orang ramai. Mereka membawa bakul berisi roti segar, buah-buahan, sayur-sayuran dan bunga. Seorang kanak-kanak ketawa sambil mengejar burung merpati, manakala dua sahabat lama minum kopi dan bertekak tentang cuaca. Menjelang tengah hari, matahari mula bersinar, cermin kedai berkilau dan seluruh jalan berbau gula yang hangat. Pada waktu malam hujan turun, dan bandar kembali sunyi.",
  ml: "എല്ലാ ശനിയാഴ്ച രാവിലെയും സ്റ്റേഷനടുത്തുള്ള ചെറിയ ചന്ത ആളുകളെക്കൊണ്ട് നിറഞ്ഞിരിക്കും. ആളുകൾ പഴങ്ങളും പച്ചക്കറികളും പൂക്കളും നിറച്ച കൊട്ടകളുമായി നടക്കുന്നു. ഒരു കുട്ടി ചിരിച്ചുകൊണ്ട് ഒരു പ്രാവിന്റെ പിന്നാലെ ഓടുന്നു, രണ്ട് പഴയ കൂട്ടുകാർ ചായ കുടിച്ചുകൊണ്ട് കാലാവസ്ഥയെക്കുറിച്ച് തർക്കിക്കുന്നു. ഉച്ചയോടെ വെയിൽ തെളിയുന്നു, കടകളുടെ ചില്ലുകൾ തിളങ്ങുന്നു, തെരുവ് മുഴുവൻ ചൂടുള്ള ശർക്കരയുടെ മണം പരക്കുന്നു. വൈകുന്നേരം മഴ പെയ്യുന്നു, നഗരം വീണ്ടും ശാന്തമാകുന്നു.",
  cmn: "每个星期六的早上，车站附近的小市场总是热热闹闹的。人们提着篮子，里面装满了新鲜的面包、水果、蔬菜和鲜花。一个小孩一边笑一边追着一只鸽子跑，旁边两个老朋友一边喝咖啡，一边为天气争论不休。快到中午的时候，太阳出来了，商店的玻璃窗闪闪发亮，整条街都飘着暖暖的糖香。到了傍晚，天下起了雨，城市又慢慢安静下来。",
  mr: "दर शनिवारी सकाळी स्टेशनजवळचा छोटा बाजार नेहमी माणसांनी भरलेला असतो. लोक ताजी फळं, भाज्या आणि फुलांनी भरलेल्या टोपल्या घेऊन फिरतात. एक लहान मूल हसत हसत एका कबुतराच्या मागे धावतं, आणि दोन जुने मित्र चहा पीत पीत हवामानावरून वाद घालतात. दुपारपर्यंत ऊन पडतं, दुकानांच्या काचा चमकतात आणि सगळ्या रस्त्यावर गरम गुळाचा वास पसरतो. संध्याकाळी पाऊस पडतो, आणि शहर पुन्हा शांत होतं.",
  no: "Lørdag morgen er det alltid fullt av folk på det lille torget ved stasjonen. Mange bærer kurver med nybakt brød, epler, ost og blomster. Et barn ler og løper etter en due, mens to gamle venner drikker kaffe og krangler om været. Rundt middagstid kommer sola fram, butikkvinduene skinner, og hele gata lukter varmt sukker. Om kvelden regner det, og byen blir stille igjen.",
  or: "ପ୍ରତି ଶନିବାର ସକାଳେ ଷ୍ଟେସନ ପାଖର ଛୋଟ ବଜାରଟି ସବୁବେଳେ ଲୋକଙ୍କ ଭିଡ଼ରେ ଭରି ରହେ। ଲୋକମାନେ ତାଜା ଫଳ, ପନିପରିବା ଆଉ ଫୁଲ ଭର୍ତ୍ତି ଟୋକେଇ ଧରି ବୁଲନ୍ତି। ଗୋଟିଏ ପିଲା ହସି ହସି ଗୋଟିଏ ପାରା ପଛରେ ଦୌଡ଼େ, ଆଉ ଦୁଇଜଣ ପୁରୁଣା ସାଙ୍ଗ ଚା ପିଉ ପିଉ ପାଣିପାଗ ନେଇ ଯୁକ୍ତିତର୍କ କରନ୍ତି। ମଧ୍ୟାହ୍ନ ବେଳକୁ ଖରା ବାହାରେ, ଦୋକାନର କାଚ ଝଲସି ଉଠେ ଆଉ ସାରା ରାସ୍ତାରେ ଗରମ ଗୁଡ଼ର ବାସ୍ନା ଖେଳିଯାଏ। ସନ୍ଧ୍ୟାରେ ବର୍ଷା ହୁଏ, ଆଉ ସହରଟି ପୁଣି ଶାନ୍ତ ହୋଇଯାଏ।",
  pl: "W sobotę rano mały targ obok dworca jest zawsze pełen ludzi. Kupujący niosą kosze ze świeżym chlebem, jabłkami, serem i kwiatami. Dziecko śmieje się i goni gołębia, a dwaj starzy przyjaciele piją kawę i spierają się o pogodę. Koło południa wychodzi słońce, wystawy sklepów błyszczą, a cała ulica pachnie ciepłym cukrem. Wieczorem pada deszcz i miasto znów cichnie.",
  pt_br:
    "Aos sábados de manhã, a pequena feira perto da estação está sempre cheia. As pessoas carregam cestas com pão fresquinho, maçãs, queijo e flores. Uma criança ri e corre atrás de um pombo, enquanto dois velhos amigos tomam café e discutem sobre o tempo. Lá pelo meio-dia o sol aparece, as vitrines brilham e a rua inteira fica com cheiro de açúcar quente. À noite chove, e a cidade fica quieta de novo.",
  pa: "ਹਰ ਸ਼ਨੀਵਾਰ ਸਵੇਰੇ ਸਟੇਸ਼ਨ ਦੇ ਨੇੜੇ ਵਾਲਾ ਛੋਟਾ ਬਾਜ਼ਾਰ ਹਮੇਸ਼ਾ ਲੋਕਾਂ ਨਾਲ ਭਰਿਆ ਰਹਿੰਦਾ ਹੈ। ਲੋਕ ਤਾਜ਼ੇ ਫਲਾਂ, ਸਬਜ਼ੀਆਂ ਅਤੇ ਫੁੱਲਾਂ ਨਾਲ ਭਰੀਆਂ ਟੋਕਰੀਆਂ ਲੈ ਕੇ ਘੁੰਮਦੇ ਹਨ। ਇੱਕ ਬੱਚਾ ਹੱਸਦਾ ਹੋਇਆ ਕਬੂਤਰ ਦੇ ਪਿੱਛੇ ਦੌੜਦਾ ਹੈ, ਅਤੇ ਦੋ ਪੁਰਾਣੇ ਦੋਸਤ ਚਾਹ ਪੀਂਦੇ ਹੋਏ ਮੌਸਮ ਬਾਰੇ ਬਹਿਸ ਕਰਦੇ ਹਨ। ਦੁਪਹਿਰ ਤੱਕ ਧੁੱਪ ਨਿਕਲ ਆਉਂਦੀ ਹੈ, ਦੁਕਾਨਾਂ ਦੇ ਸ਼ੀਸ਼ੇ ਚਮਕਣ ਲੱਗਦੇ ਹਨ ਅਤੇ ਸਾਰੀ ਗਲੀ ਵਿੱਚ ਗਰਮ ਗੁੜ ਦੀ ਖੁਸ਼ਬੂ ਫੈਲ ਜਾਂਦੀ ਹੈ। ਸ਼ਾਮ ਨੂੰ ਮੀਂਹ ਪੈਂਦਾ ਹੈ, ਅਤੇ ਸ਼ਹਿਰ ਫਿਰ ਤੋਂ ਸ਼ਾਂਤ ਹੋ ਜਾਂਦਾ ਹੈ।",
  ro: "Sâmbăta dimineața, piața mică de lângă gară e mereu plină de lume. Oamenii duc coșuri cu pâine proaspătă, mere, brânză și flori. Un copil râde și aleargă după un porumbel, în timp ce doi prieteni vechi beau cafea și se contrazic despre vreme. Pe la prânz iese soarele, vitrinele strălucesc și toată strada miroase a zahăr cald. Seara plouă, iar orașul se liniștește din nou.",
  ru: "В субботу утром на маленьком рынке у вокзала всегда много народу. Люди несут корзины со свежим хлебом, яблоками, сыром и цветами. Ребёнок смеётся и гоняется за голубем, а два старых друга пьют кофе и спорят о погоде. К полудню выходит солнце, витрины блестят, и вся улица пахнет тёплым сахаром. Вечером идёт дождь, и город снова затихает.",
  sk: "V sobotu ráno je malý trh pri stanici vždy plný ľudí. Nakupujúci nesú košíky s čerstvým chlebom, jablkami, syrom a kvetmi. Dieťa sa smeje a naháňa holuba, zatiaľ čo dvaja starí priatelia pijú kávu a dohadujú sa o počasí. Okolo poludnia vyjde slnko, výklady sa lesknú a celá ulica vonia teplým cukrom. Večer prší a mesto znova stíchne.",
  es: "Los sábados por la mañana, el pequeño mercado cerca de la estación siempre está lleno de gente. Muchos llevan cestas con pan recién hecho, manzanas, queso y flores. Un niño se ríe y persigue a una paloma, mientras dos viejos amigos toman café y discuten sobre el tiempo. Hacia el mediodía sale el sol, brillan los escaparates y toda la calle huele a azúcar caliente. Por la noche llueve, y la ciudad vuelve a quedarse en silencio.",
  sv: "På lördagsmorgnarna är det alltid fullt av folk på det lilla torget vid stationen. Människor bär korgar med färskt bröd, äpplen, ost och blommor. Ett barn skrattar och jagar en duva, medan två gamla vänner dricker kaffe och käbblar om vädret. Vid lunchtid tittar solen fram, skyltfönstren glänser och hela gatan doftar varmt socker. På kvällen regnar det, och staden blir tyst igen.",
  tl: "Tuwing Sabado ng umaga, laging maraming tao sa maliit na palengke malapit sa istasyon. May bitbit silang mga basket na puno ng sariwang tinapay, prutas, gulay at bulaklak. Isang bata ang tumatawa habang hinahabol ang isang kalapati, samantalang ang dalawang matagal nang magkaibigan ay nagkakape at nagtatalo tungkol sa panahon. Pagsapit ng tanghali, sumisikat ang araw, kumikinang ang mga bintana ng tindahan, at amoy mainit na asukal ang buong kalye. Sa gabi, umuulan, at muling tumatahimik ang bayan.",
  ta: "ஒவ்வொரு சனிக்கிழமை காலையிலும் ரயில் நிலையத்துக்கு அருகில் உள்ள சின்ன சந்தையில் எப்போதும் மக்கள் கூட்டம் இருக்கும். மக்கள் பழங்கள், காய்கறிகள், பூக்கள் நிறைந்த கூடைகளைச் சுமந்து செல்கிறார்கள். ஒரு குழந்தை சிரித்துக்கொண்டே ஒரு புறாவைத் துரத்துகிறது, இரண்டு பழைய நண்பர்கள் டீ குடித்துக்கொண்டே வானிலையைப் பற்றி வாதம் செய்கிறார்கள். மதியத்துக்குள் வெயில் வருகிறது, கடைகளின் கண்ணாடிகள் மின்னுகின்றன, தெரு முழுவதும் சூடான வெல்லத்தின் மணம் பரவுகிறது. மாலையில் மழை பெய்கிறது, நகரம் மீண்டும் அமைதியாகிறது.",
  te: "ప్రతి శనివారం ఉదయం స్టేషన్ దగ్గర ఉన్న చిన్న మార్కెట్ ఎప్పుడూ జనంతో నిండి ఉంటుంది. జనం తాజా పండ్లు, కూరగాయలు, పూలతో నిండిన బుట్టలు పట్టుకుని తిరుగుతుంటారు. ఒక పిల్లాడు నవ్వుతూ ఒక పావురం వెనక పరిగెడుతున్నాడు, ఇద్దరు పాత స్నేహితులు టీ తాగుతూ వాతావరణం గురించి వాదించుకుంటున్నారు. మధ్యాహ్నానికల్లా ఎండ వస్తుంది, దుకాణాల అద్దాలు మెరుస్తాయి, వీధి అంతటా వేడి బెల్లం వాసన వ్యాపిస్తుంది. సాయంత్రం వర్షం పడుతుంది, నగరం మళ్ళీ ప్రశాంతంగా మారుతుంది.",
  th: "ทุกเช้าวันเสาร์ ตลาดเล็ก ๆ ใกล้สถานีรถไฟมีผู้คนพลุกพล่านเสมอ ผู้คนถือตะกร้าที่เต็มไปด้วยขนมปังสด ผลไม้ ผัก และดอกไม้ เด็กคนหนึ่งหัวเราะพลางวิ่งไล่นกพิราบ ส่วนเพื่อนเก่าสองคนนั่งดื่มกาแฟและเถียงกันเรื่องอากาศ พอใกล้เที่ยง แดดก็ออก กระจกหน้าร้านเป็นประกาย และทั้งถนนก็หอมกลิ่นน้ำตาลอุ่น ๆ พอตกเย็นฝนก็ตก แล้วเมืองก็กลับมาเงียบสงบอีกครั้ง",
  tr: "Cumartesi sabahları istasyonun yanındaki küçük pazar her zaman kalabalık olur. İnsanlar taze ekmek, elma, peynir ve çiçeklerle dolu sepetler taşır. Bir çocuk gülerek bir güvercini kovalarken, iki eski arkadaş kahve içip hava hakkında tartışır. Öğleye doğru güneş açar, vitrinler parlar ve bütün sokak sıcak şeker gibi kokar. Akşam yağmur yağar ve şehir yeniden sessizleşir.",
  uk: "У суботу вранці на маленькому ринку біля вокзалу завжди людно. Люди несуть кошики зі свіжим хлібом, яблуками, сиром і квітами. Дитина сміється й ганяється за голубом, а двоє старих друзів п’ють каву та сперечаються про погоду. Ближче до полудня виходить сонце, вітрини виблискують, і вся вулиця пахне теплим цукром. Увечері йде дощ, і місто знову стихає.",
  ur: "ہر اتوار کی صبح اسٹیشن کے پاس والا چھوٹا بازار لوگوں سے بھرا رہتا ہے۔ لوگ تازہ پھلوں، سبزیوں اور پھولوں سے بھری ٹوکریاں لے کر گھومتے ہیں۔ ایک بچہ ہنستے ہوئے کبوتر کے پیچھے دوڑتا ہے، اور دو پرانے دوست چائے پیتے ہوئے موسم پر بحث کرتے ہیں۔ دوپہر تک دھوپ نکل آتی ہے، دکانوں کے شیشے چمکنے لگتے ہیں اور پوری گلی میں گرم گڑ کی خوشبو پھیل جاتی ہے۔ شام کو بارش ہوتی ہے، اور شہر پھر سے پرسکون ہو جاتا ہے۔",
  vi: "Mỗi sáng thứ Bảy, khu chợ nhỏ gần nhà ga lúc nào cũng đông người. Mọi người xách những chiếc giỏ đựng bánh mì mới ra lò, trái cây, rau và hoa. Một em bé vừa cười vừa đuổi theo con chim bồ câu, trong khi hai người bạn già ngồi uống cà phê và tranh luận về thời tiết. Đến gần trưa, mặt trời ló ra, cửa kính các cửa hàng sáng lấp lánh và cả con phố thơm mùi đường ấm. Buổi tối trời mưa, và thành phố lại yên tĩnh.",
};

/** The passage to read out loud while recording a voice, in a language (English when there's none). */
export const passageFor = (code: string) => PASSAGES[code] ?? ENGLISH_PASSAGE;
