import { create } from "zustand";
import {
  api,
  type Catalog,
  type ClaudeInfo,
  type Control,
  errText,
  type Overview,
  type SystemCheck,
  type Voice,
} from "@workspace/core/booth/api";

export type View = "setup" | "studio" | "voices" | "create" | "settings";

interface BoothState {
  view: View;
  overview: Overview | null;
  system: SystemCheck | null;
  claude: ClaudeInfo | null;
  voices: Voice[];
  catalog: Catalog | null;
  voiceId: string | null;
  /** Renders in flight (shows "Speaking" in the nav). */
  busy: number;
  catalogError: string | null;
  go: (v: View) => void;
  refresh: () => Promise<Overview | null>;
  loadSystem: () => Promise<void>;
  loadClaude: () => Promise<void>;
  loadVoices: () => Promise<void>;
  loadCatalog: () => Promise<void>;
  pickVoice: (id: string, open?: boolean) => void;
  /** Wraps a call that may start or use the engine, so the nav shows it working. */
  work: <T>(p: () => Promise<T>) => Promise<T>;
}

export const useBooth = create<BoothState>((set, get) => ({
  view: "studio",
  overview: null,
  system: null,
  claude: null,
  voices: [],
  catalog: null,
  voiceId: null,
  busy: 0,
  catalogError: null,
  go: (view) => set({ view }),
  refresh: async () => {
    try {
      const overview = await api.overview();
      set((s) => ({
        overview,
        voiceId: s.voiceId ?? overview.settings.default_voice,
      }));
      return overview;
    } catch {
      return null;
    }
  },
  loadSystem: async () => {
    set({ system: await api.systemCheck() });
  },
  loadClaude: async () => {
    try {
      set({ claude: await api.claudeInfo() });
    } catch {
      // not fatal: the Settings page shows what it can
    }
  },
  loadVoices: async () => {
    const voices = await get().work(() => api.listVoices());
    set((s) => {
      const known = s.voiceId && voices.some((v) => v.id === s.voiceId);
      const first =
        voices.find((v) => v.name === "Rupert") ?? voices.find((v) => v.kind === "default") ?? voices[0];
      return { voices, voiceId: known ? s.voiceId : (first?.id ?? null) };
    });
  },
  loadCatalog: async () => {
    try {
      const catalog = await get().work(() => api.controls());
      set({ catalog, catalogError: null });
    } catch (e) {
      set({ catalogError: errText(e) });
    }
  },
  pickVoice: (id, open = true) => {
    set({ voiceId: id, ...(open ? { view: "studio" as View } : {}) });
    const s = get().overview?.settings;
    if (s && s.default_voice !== id) {
      api
        .saveSettings({ ...s, default_voice: id })
        .then((settings) => {
          const o = get().overview;
          if (o) {
            set({ overview: { ...o, settings } });
          }
        })
        .catch(() => undefined);
    }
  },
  work: async (p) => {
    set((s) => ({ busy: s.busy + 1 }));
    try {
      return await p();
    } finally {
      set((s) => ({ busy: s.busy - 1 }));
      // the engine may have started for this call
      if (get().overview?.engine.state !== "running") {
        get().refresh();
      }
    }
  },
}));

export const currentVoice = (s: { voices: Voice[]; voiceId: string | null }) =>
  s.voices.find((v) => v.id === s.voiceId) ?? null;

export const controlById = (cat: Catalog | null, id: string): Control | undefined =>
  cat?.controls.find((c) => c.id === id);

export const packPresent = (o: Overview | null, pack: string) =>
  o?.packs.some((p) => p.pack === pack && p.present) ?? false;

export const round2 = (v: number) => Math.round(v * 100) / 100;

/** A slider position (-100..100) to a dose inside the control's safe range. */
export const toDose = (c: Control | undefined, pos: number) => {
  if (!c) {
    return 0;
  }
  const [lo, hi] = c.safe ?? c.range ?? [-1, 1];
  return round2(pos >= 0 ? (pos / 100) * hi : (pos / 100) * -lo);
};
