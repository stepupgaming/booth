"use client";

import { CheckIcon, ChevronDownIcon } from "lucide-react";
import { Select as SelectPrimitive } from "radix-ui";
import {
  createContext,
  type CSSProperties,
  type ReactNode,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { create } from "zustand";
import { errText } from "@workspace/core/booth/api";

// ---------- background and brand ----------

export function Backdrop() {
  return (
    <>
      <div aria-hidden="true" className="b-orbs">
        <span className="o1" />
        <span className="o2" />
      </div>
      <div aria-hidden="true" className="b-grain" />
    </>
  );
}

export function Brand() {
  return (
    <span className="b-brand">
      <img alt="" height={36} src="/booth-logo.png" width={36} />
      <span>
        <b>Booth</b>
        <small>by NoFaceTube</small>
      </span>
    </span>
  );
}

const ORBS = [
  "radial-gradient(circle at 32% 28%, #FFF 0%, #FFB0B0 11%, #F0202A 44%, #3A0408 100%)",
  "radial-gradient(circle at 32% 28%, #FFF 0%, #B9FFF1 16%, #4FE3CF 48%, #2856C8 100%)",
  "radial-gradient(circle at 32% 28%, #FFF 0%, #E2D4FF 16%, #9C7BFF 48%, #3B1F8F 100%)",
  "radial-gradient(circle at 32% 28%, #FFF 0%, #FFE7A8 16%, #FFB24A 48%, #B4402E 100%)",
  "radial-gradient(circle at 32% 28%, #FFF 0%, #FFD0E4 16%, #FF6FA8 48%, #5A2A9E 100%)",
  "radial-gradient(circle at 32% 28%, #FFF 0%, #CDE9FF 16%, #6FB6FF 48%, #2B3A8F 100%)",
];

export const hash = (s: string) => {
  let h = 2_166_136_261;
  for (let i = 0; i < s.length; i++) {
    h = Math.imul(h ^ s.charCodeAt(i), 16_777_619);
  }
  return Math.abs(h);
};

export const orbFor = (id: string) => ORBS[hash(id) % ORBS.length] as string;

export function Orb({
  id,
  size = 44,
  spin = false,
}: {
  id: string;
  size?: number;
  spin?: boolean;
}) {
  const bg = spin ? ORBS[0] : orbFor(id);
  return (
    <span className="orb" style={{ width: size, height: size }}>
      {spin ? <i className="spin" /> : <i className="glow" style={{ background: bg }} />}
      <i className="ball" style={{ background: bg }} />
    </span>
  );
}

// ---------- icons ----------

export const Icon = {
  play: (
    <svg aria-hidden="true" fill="currentColor" height="12" viewBox="0 0 24 24" width="12">
      <path d="M7 4l13 8-13 8z" />
    </svg>
  ),
  pause: (
    <svg aria-hidden="true" fill="currentColor" height="12" viewBox="0 0 24 24" width="12">
      <path d="M6 4h4v16H6zM14 4h4v16h-4z" />
    </svg>
  ),
  studio: (
    <svg aria-hidden="true" fill="none" height="18" stroke="currentColor" strokeWidth="1.7" viewBox="0 0 24 24" width="18">
      <path d="M4 12h2l2-6 4 12 3-9 2 3h3" />
    </svg>
  ),
  voices: (
    <svg aria-hidden="true" fill="none" height="18" stroke="currentColor" strokeWidth="1.7" viewBox="0 0 24 24" width="18">
      <circle cx="12" cy="8" r="4" />
      <path d="M4 21c1-4 4-6 8-6s7 2 8 6" />
    </svg>
  ),
  mic: (
    <svg aria-hidden="true" fill="none" height="18" stroke="currentColor" strokeWidth="1.7" viewBox="0 0 24 24" width="18">
      <rect height="11" rx="3" width="6" x="9" y="3" />
      <path d="M5 11a7 7 0 0 0 14 0M12 18v3" />
    </svg>
  ),
  settings: (
    <svg aria-hidden="true" fill="none" height="18" stroke="currentColor" strokeWidth="1.7" viewBox="0 0 24 24" width="18">
      <circle cx="12" cy="12" r="3" />
      <path d="M12 2v3M12 19v3M2 12h3M19 12h3M5 5l2 2M17 17l2 2M5 19l2-2M17 7l2-2" />
    </svg>
  ),
  plus: (
    <svg aria-hidden="true" fill="none" height="16" stroke="currentColor" strokeWidth="2.2" viewBox="0 0 24 24" width="16">
      <path d="M12 5v14M5 12h14" />
    </svg>
  ),
  upload: (
    <svg aria-hidden="true" fill="none" height="24" stroke="#F0202A" strokeWidth="1.8" viewBox="0 0 24 24" width="24">
      <path d="M12 16V4M7 9l5-5 5 5M4 20h16" />
    </svg>
  ),
  micRed: (
    <svg aria-hidden="true" fill="none" height="24" stroke="#F0202A" strokeWidth="1.8" viewBox="0 0 24 24" width="24">
      <rect height="11" rx="3" width="6" x="9" y="3" />
      <path d="M5 11a7 7 0 0 0 14 0M12 18v3" />
    </svg>
  ),
  search: (
    <svg aria-hidden="true" fill="none" height="16" stroke="#B3B8C5" strokeWidth="2" viewBox="0 0 24 24" width="16">
      <circle cx="11" cy="11" r="7" />
      <path d="M20 20l-4-4" />
    </svg>
  ),
  star: (on: boolean) => (
    <svg aria-hidden="true" fill={on ? "currentColor" : "none"} height="14" stroke="currentColor" strokeWidth="1.8" viewBox="0 0 24 24" width="14">
      <path d="M12 3l2.7 5.6 6.1.9-4.4 4.3 1 6.1L12 17l-5.4 2.9 1-6.1-4.4-4.3 6.1-.9z" />
    </svg>
  ),
};

// ---------- audio player (one at a time) ----------

interface PlayerState {
  src: string | null;
  playing: boolean;
  progress: number;
  play: (src: string) => void;
  stop: () => void;
  seek: (src: string, p: number) => void;
}

let audio: HTMLAudioElement | null = null;
let raf = 0;

export const usePlayer = create<PlayerState>((set, get) => {
  const tick = () => {
    if (audio && audio.duration > 0) {
      set({ progress: audio.currentTime / audio.duration });
    }
    if (get().playing) {
      raf = requestAnimationFrame(tick);
    }
  };
  const ensure = () => {
    if (!audio) {
      audio = new Audio();
      audio.addEventListener("ended", () => {
        cancelAnimationFrame(raf);
        set({ playing: false, progress: 1 });
        const q = queue.shift();
        if (q) {
          get().play(q);
        }
      });
      audio.addEventListener("pause", () => set({ playing: false }));
      audio.addEventListener("play", () => {
        set({ playing: true });
        raf = requestAnimationFrame(tick);
      });
    }
    return audio;
  };
  return {
    src: null,
    playing: false,
    progress: 0,
    play: (src) => {
      const a = ensure();
      if (get().src === src) {
        if (a.paused) {
          if (a.ended || get().progress >= 1) {
            a.currentTime = 0;
          }
          a.play().catch(() => undefined);
        } else {
          a.pause();
        }
        return;
      }
      a.src = src;
      set({ src, progress: 0 });
      a.play().catch(() => undefined);
    },
    stop: () => {
      queue.length = 0;
      audio?.pause();
    },
    seek: (src, p) => {
      const a = ensure();
      if (get().src !== src) {
        a.src = src;
        set({ src });
        a.addEventListener(
          "loadedmetadata",
          () => {
            a.currentTime = p * a.duration;
          },
          { once: true }
        );
        a.play().catch(() => undefined);
        return;
      }
      if (a.duration > 0) {
        a.currentTime = p * a.duration;
        set({ progress: p });
      }
    },
  };
});

const queue: string[] = [];
/** Play several files back to back. */
export const playAll = (srcs: string[]) => {
  if (srcs.length === 0) {
    return;
  }
  queue.length = 0;
  queue.push(...srcs.slice(1));
  const p = usePlayer.getState();
  if (p.src === srcs[0]) {
    p.seek(srcs[0] as string, 0);
    audio?.play().catch(() => undefined);
  } else {
    p.play(srcs[0] as string);
  }
};

// ---------- hairline waveform ----------

/** A made-up but speech-shaped envelope (words, syllables, pauses), for files not loaded yet. */
export function speechBars(seed: number, n: number): number[] {
  let x = (seed * 7919) % 233_280;
  const rnd = () => {
    x = (x * 9301 + 49_297) % 233_280;
    return x / 233_280;
  };
  const amp: number[] = [];
  while (amp.length < n * 4) {
    const len = 14 + Math.floor(rnd() * 26);
    const peak = 0.3 + rnd() ** 0.7 * 0.7;
    const syl = 1 + Math.floor(rnd() * 3);
    for (let i = 0; i < len; i++) {
      const u = i / len;
      amp.push(
        Math.max(
          0.03,
          peak * Math.sin(Math.PI * u) ** 0.5 * (0.65 + 0.35 * Math.abs(Math.sin(Math.PI * u * syl))) * (0.55 + rnd() * 0.45)
        )
      );
    }
    const g = rnd() < 0.25 ? 12 + Math.floor(rnd() * 10) : 3 + Math.floor(rnd() * 5);
    for (let i = 0; i < g; i++) {
      amp.push(0.02 + rnd() * 0.02);
    }
  }
  return Array.from({ length: n }, (_, i) => Math.max(...amp.slice(i * 4, i * 4 + 4)));
}

const peakCache = new Map<string, Promise<number[]>>();
let ctx: AudioContext | null = null;

async function loadPeaks(url: string, n: number): Promise<number[]> {
  const buf = await fetch(url).then((r) => r.arrayBuffer());
  ctx ??= new AudioContext();
  const audioBuf = await ctx.decodeAudioData(buf);
  const data = audioBuf.getChannelData(0);
  const step = Math.max(1, Math.floor(data.length / n));
  const out: number[] = [];
  for (let i = 0; i < n; i++) {
    let m = 0;
    for (let j = i * step; j < Math.min(data.length, (i + 1) * step); j++) {
      const v = Math.abs(data[j] as number);
      if (v > m) {
        m = v;
      }
    }
    out.push(m);
  }
  const top = Math.max(0.05, ...out);
  return out.map((v) => Math.max(0.03, (v / top) ** 0.8));
}

export function usePeaks(url: string | null, n: number, seed: number) {
  const [bars, setBars] = useState<number[] | null>(null);
  useEffect(() => {
    setBars(null);
    if (!url) {
      return;
    }
    const key = `${url}#${n}`;
    let p = peakCache.get(key);
    if (!p) {
      p = loadPeaks(url, n);
      peakCache.set(key, p);
      p.catch(() => peakCache.delete(key));
    }
    let live = true;
    p.then((b) => live && setBars(b)).catch(() => undefined);
    return () => {
      live = false;
    };
  }, [url, n]);
  return useMemo(() => bars ?? speechBars(seed, n), [bars, seed, n]);
}

const PLAYED = "#FFFFFF";
const UNPLAYED = "rgba(255,255,255,0.20)";
const REST = "rgba(255,255,255,0.32)";

/** Dense 1.5px bars. With `src` it draws the real file and shows the play head when it plays. */
export function Wave({
  src,
  bars = 96,
  height = 44,
  seed = 1,
}: {
  src?: string | null;
  bars?: number;
  height?: number;
  seed?: number;
}) {
  const peaks = usePeaks(src ?? null, bars, seed);
  const cur = usePlayer((s) => (src && s.src === src ? s.progress : null));
  const active = cur !== null && cur > 0 && cur < 1;
  const seek = usePlayer((s) => s.seek);
  const ref = useRef<HTMLDivElement>(null);
  const onClick = useCallback(
    (e: React.MouseEvent) => {
      if (!(src && ref.current)) {
        return;
      }
      const r = ref.current.getBoundingClientRect();
      seek(src, Math.min(1, Math.max(0, (e.clientX - r.left) / r.width)));
    },
    [src, seek]
  );
  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: the play button is the accessible control; clicking the wave is a shortcut
    // biome-ignore lint/a11y/useKeyWithClickEvents: as above
    <div
      aria-hidden="true"
      className={`wave${src ? " seekable" : ""}`}
      onClick={onClick}
      ref={ref}
      style={{ height: height + 4 }}
    >
      {peaks.map((v, i) => {
        const x = i / (peaks.length - 1);
        const c = active ? (x <= (cur as number) ? PLAYED : UNPLAYED) : REST;
        return (
          // biome-ignore lint/suspicious/noArrayIndexKey: fixed-length bar list
          <i key={i} style={{ height: Math.max(2, Math.round(v * height)), background: c }} />
        );
      })}
      {active ? <span className="head" style={{ left: `${(cur as number) * 100}%` }} /> : null}
    </div>
  );
}

export function PlayButton({
  src,
  label,
  variant,
  onNeedSrc,
}: {
  src: string | null;
  label: string;
  variant?: "solid" | "red";
  onNeedSrc?: () => Promise<string | null>;
}) {
  const playing = usePlayer((s) => s.playing && src !== null && s.src === src);
  const play = usePlayer((s) => s.play);
  const [busy, setBusy] = useState(false);
  const toast = useToast();
  const click = async () => {
    if (src) {
      play(src);
      return;
    }
    if (!onNeedSrc) {
      return;
    }
    setBusy(true);
    try {
      const s = await onNeedSrc();
      if (s) {
        play(s);
      }
    } catch (e) {
      toast.bad(errText(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <button
      aria-label={`${playing ? "Pause" : "Play"} ${label}`}
      className={`play${variant ? ` ${variant}` : ""}`}
      onClick={click}
      type="button"
    >
      {busy ? <span className="spinner" /> : playing ? Icon.pause : Icon.play}
    </button>
  );
}

// ---------- form bits ----------

/** A control slider over a dose range. Bipolar ranges fill from the centre. */
export function DoseSlider({
  label,
  poles,
  min,
  max,
  value,
  onChange,
  disabled,
  title,
  step,
}: {
  label: string;
  poles?: string;
  min: number;
  max: number;
  value: number;
  onChange: (v: number) => void;
  disabled?: boolean;
  title?: string;
  step?: number;
}) {
  const span = max - min || 1;
  const pos = ((value - min) / span) * 100;
  const zero = ((0 - min) / span) * 100;
  const bipolar = min < 0;
  const style = bipolar
    ? ({
        "--a": `${Math.min(pos, zero)}%`,
        "--w": `${Math.abs(pos - zero)}%`,
      } as React.CSSProperties)
    : ({ "--p": `${pos}%` } as React.CSSProperties);
  return (
    <label className={`slider${disabled ? " off" : ""}`} title={title ?? undefined}>
      <span className="row">
        <span>{label}</span>
        <span className="poles">{poles}</span>
      </span>
      <input
        aria-label={label}
        className={bipolar ? "center" : undefined}
        disabled={disabled}
        max={max}
        min={min}
        onChange={(e) => onChange(Number(e.target.value))}
        onDoubleClick={() => onChange(0)}
        step={step ?? span / 100}
        style={style}
        type="range"
        value={value}
      />
    </label>
  );
}

export function Switch({
  on,
  onChange,
  label,
}: {
  on: boolean;
  onChange: (v: boolean) => void;
  label: string;
}) {
  return (
    <button
      aria-checked={on}
      aria-label={label}
      className="switch"
      onClick={() => onChange(!on)}
      role="switch"
      type="button"
    >
      <span />
    </button>
  );
}

// ---------- dropdown ----------

export interface DropdownOption {
  value: string;
  label: string;
  disabled?: boolean;
  /** Small faint text after the label, e.g. why it is disabled. */
  hint?: string;
}
export interface DropdownGroup {
  label: string;
  options: DropdownOption[];
}

// Radix reserves "" for "no value", so the empty choice travels under this key.
const EMPTY = "__empty__";
const toKey = (v: string) => (v === "" ? EMPTY : v);
const fromKey = (k: string) => (k === EMPTY ? "" : k);

/** Booth's dropdown. Native <select> menus ignore the dark theme in WebView2, so this replaces them. */
export function Dropdown({
  value,
  onChange,
  options,
  groups,
  disabled,
  label,
  style,
}: {
  value: string;
  onChange: (v: string) => void;
  options?: DropdownOption[];
  groups?: DropdownGroup[];
  disabled?: boolean;
  /** Accessible name when no visible label wraps it. */
  label?: string;
  style?: CSSProperties;
}) {
  const item = (o: DropdownOption) => (
    <SelectPrimitive.Item className="dd-item" disabled={o.disabled} key={o.value} value={toKey(o.value)}>
      <SelectPrimitive.ItemText>{o.label}</SelectPrimitive.ItemText>
      {o.hint ? <span className="dd-hint">{o.hint}</span> : null}
      <SelectPrimitive.ItemIndicator className="dd-check">
        <CheckIcon size={14} strokeWidth={2.2} />
      </SelectPrimitive.ItemIndicator>
    </SelectPrimitive.Item>
  );
  return (
    <SelectPrimitive.Root disabled={disabled} onValueChange={(k) => onChange(fromKey(k))} value={toKey(value)}>
      <SelectPrimitive.Trigger aria-label={label} className="field dd-trigger" style={style}>
        <SelectPrimitive.Value />
        <SelectPrimitive.Icon className="dd-chev">
          <ChevronDownIcon size={15} />
        </SelectPrimitive.Icon>
      </SelectPrimitive.Trigger>
      <SelectPrimitive.Portal>
        <SelectPrimitive.Content className="dd-menu" collisionPadding={12} position="popper" sideOffset={6}>
          <SelectPrimitive.Viewport className="dd-viewport">
            {options?.map(item)}
            {groups?.map((g, i) => (
              <SelectPrimitive.Group key={g.label}>
                {i > 0 || options?.length ? <SelectPrimitive.Separator className="dd-sep" /> : null}
                <SelectPrimitive.Label className="dd-group">{g.label}</SelectPrimitive.Label>
                {g.options.map(item)}
              </SelectPrimitive.Group>
            ))}
          </SelectPrimitive.Viewport>
        </SelectPrimitive.Content>
      </SelectPrimitive.Portal>
    </SelectPrimitive.Root>
  );
}

// ---------- toasts ----------

interface ToastApi {
  ok: (msg: string) => void;
  bad: (msg: string) => void;
}
const ToastCtx = createContext<ToastApi>({ ok: () => undefined, bad: () => undefined });
export const useToast = () => useContext(ToastCtx);

export function ToastHost({ children }: { children: ReactNode }) {
  const [t, setT] = useState<{ msg: string; bad: boolean } | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const show = useCallback((msg: string, bad: boolean) => {
    setT({ msg, bad });
    if (timer.current) {
      clearTimeout(timer.current);
    }
    timer.current = setTimeout(() => setT(null), bad ? 9000 : 3500);
  }, []);
  const api = useMemo(
    () => ({ ok: (m: string) => show(m, false), bad: (m: string) => show(m, true) }),
    [show]
  );
  return (
    <ToastCtx.Provider value={api}>
      {children}
      {t ? (
        <div className={`toast${t.bad ? " bad" : ""}`} role={t.bad ? "alert" : "status"}>
          <span style={{ flex: 1 }}>{t.msg}</span>
          <button className="link small" onClick={() => setT(null)} type="button">
            Close
          </button>
        </div>
      ) : null}
    </ToastCtx.Provider>
  );
}

/** Persisted per-viewer preference (localStorage, safe when storage is unavailable). */
export function usePref<T>(key: string, initial: T): [T, (v: T) => void] {
  const [v, setV] = useState<T>(() => {
    try {
      const s = localStorage.getItem(`booth.${key}`);
      return s ? (JSON.parse(s) as T) : initial;
    } catch {
      return initial;
    }
  });
  const set = useCallback(
    (nv: T) => {
      setV(nv);
      try {
        localStorage.setItem(`booth.${key}`, JSON.stringify(nv));
      } catch {
        // storage unavailable: keep it for this session only
      }
    },
    [key]
  );
  return [v, set];
}
