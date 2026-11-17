"use client";

import {
  api,
  errText,
  fileUrl,
  type ParsedScript,
  type ScriptExportOptions,
  type ScriptJob,
  type ScriptJobLine,
  type Voice,
} from "@workspace/core/booth/api";
import { type DragDropEvent, getCurrentWebview } from "@tauri-apps/api/webview";
import { isPhone, useBooth } from "@workspace/core/booth/store";
import {
  Dropdown,
  Icon,
  PlayButton,
  playAll,
  usePref,
  useToast,
} from "@workspace/core/booth/ui";
import {
  type ChangeEvent,
  type CSSProperties,
  type DragEvent,
  memo,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";

const eyebrow = {
  fontSize: 12,
  letterSpacing: "0.08em",
  textTransform: "uppercase" as const,
};

const srOnly = {
  position: "absolute" as const,
  width: 1,
  height: 1,
  overflow: "hidden",
  clip: "rect(0 0 0 0)",
};

const andList = (xs: unknown[]) => {
  const w = xs.filter((x): x is string => typeof x === "string" && !!x);
  return w.length > 1
    ? `${w.slice(0, -1).join(", ")} and ${w.at(-1)}`
    : (w[0] ?? "nothing");
};
const words = (s: string) => s.split(/\s+/).filter(Boolean).length;
/** About how long a line takes to say (2.6 words a second, an easy voiceover pace), plus pauses. */
const estimate = (lines: { text: string; pause_ms?: number }[]) =>
  lines.reduce((n, l) => n + words(l.text) / 2.6 + (l.pause_ms ?? 0) / 1000, 0);
const pauseText = (ms: number) => `pause ${Number((ms / 1000).toFixed(1))} s`;
const secs = (s: number) =>
  s >= 90
    ? `${Math.floor(s / 60)} min ${Math.round(s % 60)} s`
    : `${Math.round(s)} s`;
const plural = (n: number, one: string, many = `${one}s`) =>
  `${n} ${n === 1 ? one : many}`;
const speakerLabel = (s: string) => s || "Every line";

/** A voice per speaker: the voice picked in the Studio first, then others in its language. */
function pickVoices(
  speakers: string[],
  voices: Voice[],
  first: string | null,
  keep: Record<string, string>
) {
  const out: Record<string, string> = {};
  const lead = voices.find((v) => v.id === first) ?? voices[0];
  const pool = [
    ...voices.filter((v) => v.id !== lead?.id && v.language === lead?.language),
    ...voices.filter((v) => v.id !== lead?.id && v.language !== lead?.language),
  ];
  for (const [i, s] of speakers.entries()) {
    const kept = keep[s];
    if (kept && voices.some((v) => v.id === kept)) {
      out[s] = kept;
      continue;
    }
    const used = new Set(Object.values(out));
    const next = i === 0 ? lead : (pool.find((v) => !used.has(v.id)) ?? lead);
    if (next) {
      out[s] = next.id;
    }
  }
  return out;
}

// ---------- the draft (kept in memory, saved to storage a moment after typing stops) ----------

const DRAFT_KEY = "booth.booth.script.draft"; // usePref's key, so older drafts still load
/** Longer drafts (~2 MB in storage) stay in memory only, for this session. */
const DRAFT_MAX = 1_000_000;
let draft: string | null = null;
let draftTimer: ReturnType<typeof setTimeout> | undefined;
const draftSubs = new Set<() => void>();

function readDraft(): string {
  if (draft === null) {
    try {
      const s = localStorage.getItem(DRAFT_KEY);
      draft = s ? (JSON.parse(s) as string) : "";
    } catch {
      draft = "";
    }
  }
  return draft;
}

function saveDraft() {
  clearTimeout(draftTimer);
  draftTimer = undefined;
  try {
    if (!draft || draft.length > DRAFT_MAX) {
      // too long to keep: don't bring back an older one next time either
      localStorage.removeItem(DRAFT_KEY);
    } else {
      localStorage.setItem(DRAFT_KEY, JSON.stringify(draft));
    }
  } catch {
    // storage full or unavailable: the draft stays in memory for this session
  }
}

function setDraft(text: string) {
  draft = text;
  for (const f of draftSubs) {
    f();
  }
  clearTimeout(draftTimer);
  draftTimer = setTimeout(saveDraft, 800);
}

function subscribeDraft(f: () => void) {
  draftSubs.add(f);
  window.addEventListener("pagehide", saveDraft);
  return () => {
    draftSubs.delete(f);
    if (draftTimer !== undefined) {
      saveDraft();
    }
  };
}

const useDraft = () =>
  useSyncExternalStore(subscribeDraft, readDraft, () => "");

/** usePref's key for the open script, so Long-form opens on the paste box after a hand-off. */
const OPEN_KEY = "booth.booth.script.open";

/** Studio's "Open in Long-form": put this text in the paste box, closing any open script.
 * The caller then switches tabs. */
export function openInLongForm(text: string) {
  setDraft(text);
  saveDraft();
  try {
    localStorage.setItem(OPEN_KEY, JSON.stringify(null));
  } catch {
    // storage unavailable: Long-form may open on the last script, the text is still in its box
  }
}

const TAGLINE =
  "Paste or open a whole script. It runs in the background, then export WAV or MP3 with captions.";

// ---------- opening files ----------

/** What Booth can read into a script (booth-core doc.rs). */
const DOC_EXT = [
  "txt",
  "text",
  "md",
  "markdown",
  "srt",
  "vtt",
  "fountain",
  "docx",
  "pdf",
  "epub",
  "html",
  "htm",
  "xhtml",
  "rtf",
];
const ACCEPT = [
  ...DOC_EXT.map((e) => `.${e}`),
  "text/plain",
  "text/markdown",
  "text/html",
  "application/pdf",
  "application/epub+zip",
  "application/rtf",
  "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
].join(",");
const extOf = (name: string) => name.toLowerCase().split(".").pop() ?? "";
const SLASH = /[\\/]/;
const baseName = (path: string) => path.split(SLASH).pop() ?? path;
/** A long document waits a little longer for typing to stop before it is split again. */
const parseDelay = (n: number) => {
  if (n > 50_000) {
    return 500;
  }
  return n > 10_000 ? 300 : 180;
};

const CHUNK = 32_768;

function base64(bytes: Uint8Array): string {
  let s = "";
  for (let i = 0; i < bytes.length; i += CHUNK) {
    s += String.fromCharCode(...bytes.subarray(i, i + CHUNK));
  }
  return btoa(s);
}

/** A picked or dropped file: a File (picker, browser) or a path (desktop drop). */
type DocSource = File | { name: string; path: string };

// ---------- pasting ----------

function PasteView({
  onStarted,
  onOpen,
  opening,
  recent,
}: {
  onStarted: (j: ScriptJob) => void;
  onOpen: (f: DocSource) => void;
  opening: boolean;
  recent: ScriptJob[];
}) {
  const voices = useBooth((s) => s.voices);
  const voiceId = useBooth((s) => s.voiceId);
  const system = useBooth((s) => s.system);
  const work = useBooth((s) => s.work);
  const phone = isPhone(system);
  const toast = useToast();
  const text = useDraft();
  const setText = setDraft;
  const [chosen, setChosen] = usePref<Record<string, string>>(
    "booth.script.voices",
    {}
  );
  const [parsed, setParsed] = useState<ParsedScript | null>(null);
  const [starting, setStarting] = useState(false);
  const file = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (!text.trim()) {
      setParsed(null);
      return;
    }
    let alive = true;
    const t = setTimeout(() => {
      api
        .scriptParse(text)
        .then((p) => alive && setParsed(p))
        .catch(() => alive && setParsed(null));
    }, parseDelay(text.length));
    return () => {
      alive = false;
      clearTimeout(t);
    };
  }, [text]);

  const speakers = parsed?.speakers.map(([s]) => s) ?? [];
  const map = useMemo(
    () =>
      pickVoices(speakers.length ? speakers : [""], voices, voiceId, chosen),
    [speakers.join("\n"), voices, voiceId, chosen]
  );
  const lines = parsed?.lines ?? [];
  const options = voices.map((v) => ({
    value: v.id,
    label: v.name,
    hint: v.kind === "default" ? undefined : "yours",
  }));

  const openPicked = (e: ChangeEvent<HTMLInputElement>) => {
    const f = e.target.files?.[0];
    e.target.value = "";
    if (f) {
      onOpen(f);
    }
  };
  const openButton = (className: string, style?: CSSProperties) => (
    <button
      className={className}
      disabled={opening}
      onClick={() => file.current?.click()}
      style={style}
      type="button"
    >
      {opening ? <span className="spinner" /> : null}
      Open a file
    </button>
  );
  const paste = async () => {
    try {
      const t = await navigator.clipboard.readText();
      if (t.trim()) {
        setText(t);
        return;
      }
    } catch {
      // some web views don't allow reading the clipboard
    }
    toast.bad("Press and hold in the box, then choose Paste.");
  };
  const start = async () => {
    const first = voices[0]?.id ?? "";
    setStarting(true);
    try {
      const job = await work(() =>
        api.scriptCreate(text, map, map[""] ?? first)
      );
      onStarted(job);
    } catch (e) {
      toast.bad(errText(e));
    } finally {
      setStarting(false);
    }
  };

  const summary = (
    <>
      <div style={{ display: "flex", flexWrap: "wrap", gap: 10 }}>
        <div
          className="tile"
          style={{
            flex: "1 1 90px",
            padding: "12px 14px",
            display: "flex",
            flexDirection: "column",
            gap: 2,
          }}
        >
          <span className="mono" style={{ fontSize: 20 }}>
            {lines.length}
          </span>
          <span className="faint small">
            {lines.length === 1 ? "line" : "lines"}
          </span>
        </div>
        <div
          className="tile"
          style={{
            flex: "1 1 90px",
            padding: "12px 14px",
            display: "flex",
            flexDirection: "column",
            gap: 2,
          }}
        >
          <span className="mono" style={{ fontSize: 20 }}>
            ~{secs(estimate(lines))}
          </span>
          <span className="faint small">of audio</span>
        </div>
        <div
          className="tile"
          style={{
            flex: "1 1 90px",
            padding: "12px 14px",
            display: "flex",
            flexDirection: "column",
            gap: 2,
          }}
        >
          <span className="mono" style={{ fontSize: 20 }}>
            {parsed?.skipped.length ?? 0}
          </span>
          <span className="faint small">notes left out</span>
        </div>
      </div>

      <fieldset
        style={{
          border: "none",
          margin: 0,
          padding: 0,
          display: "flex",
          flexDirection: "column",
          gap: 10,
        }}
      >
        <legend className="faint" style={{ ...eyebrow, padding: "0 0 10px" }}>
          Voices
        </legend>
        {(speakers.length ? speakers : [""]).map((s) => {
          const n = parsed?.speakers.find(([x]) => x === s)?.[1] ?? 0;
          return (
            <div
              className="tile"
              key={s}
              style={{
                padding: "10px 14px",
                display: "flex",
                alignItems: "center",
                justifyContent: "space-between",
                gap: 12,
                flexWrap: "wrap",
              }}
            >
              <span style={{ fontSize: 14 }}>
                <span
                  className="mono"
                  style={{ fontSize: 12, color: "var(--b-text-2)" }}
                >
                  {speakerLabel(s)}
                </span>{" "}
                <span className="faint small">· {plural(n, "line")}</span>
              </span>
              <Dropdown
                label={`Voice for ${speakerLabel(s)}`}
                onChange={(v) => setChosen({ ...chosen, [s]: v })}
                options={options}
                style={{ minWidth: 170 }}
                value={map[s] ?? ""}
              />
            </div>
          );
        })}
        {speakers.length <= 1 ? (
          <div className="faint small" style={{ lineHeight: 1.5 }}>
            Lines that start with a name, like NARRATOR: or Host:, get that
            speaker’s own voice.
          </div>
        ) : null}
      </fieldset>

      {parsed?.skipped.length ? (
        <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
          <div className="faint" style={eyebrow}>
            Left out, not spoken
          </div>
          <div
            className="mono muted"
            style={{
              fontSize: 12,
              lineHeight: 1.8,
              maxHeight: 140,
              overflowY: "auto",
              wordBreak: "break-word",
            }}
          >
            {parsed.skipped.map((s, i) => (
              <div key={`${i}-${s}`}>{s}</div>
            ))}
          </div>
        </div>
      ) : null}

      <div
        style={{
          marginTop: "auto",
          display: "flex",
          flexDirection: "column",
          gap: 10,
        }}
      >
        <button
          className="btn primary"
          disabled={!lines.length || starting || !voices.length}
          onClick={start}
          style={{ justifyContent: "center", width: "100%" }}
          type="button"
        >
          {starting ? <span className="spinner" /> : null}
          {lines.length
            ? `Make all ${plural(lines.length, "line")}`
            : "Paste a script first"}
        </button>
        <div className="faint small" style={{ textAlign: "center" }}>
          You can redo any single line afterwards.
        </div>
      </div>
    </>
  );

  const box = (
    <>
      <label htmlFor="b-script" style={srOnly}>
        Your script
      </label>
      <textarea
        className="field mono"
        id="b-script"
        onChange={(e) => setText(e.target.value)}
        placeholder={
          "NARRATOR: You walk outside, it's minus ten…\n\n[B-roll: snowy street]\n\nNARRATOR: It isn't broken."
        }
        spellCheck={false}
        style={{
          flex: 1,
          minHeight: phone ? 180 : 420,
          resize: phone ? "none" : "vertical",
          padding: phone ? 14 : 16,
          borderRadius: 14,
          fontSize: phone ? 12.5 : 13,
          lineHeight: 1.7,
          color: "var(--b-text-2)",
        }}
        value={text}
      />
      <input
        accept={ACCEPT}
        hidden
        onChange={openPicked}
        ref={file}
        type="file"
      />
    </>
  );

  if (phone) {
    return (
      <div
        style={{
          display: "flex",
          flexDirection: "column",
          gap: 14,
          minHeight: "100%",
        }}
      >
        <header style={{ display: "flex", flexDirection: "column", gap: 4 }}>
          <h1 className="h1" style={{ margin: 0, fontSize: 26 }}>
            Long-form
          </h1>
          <div className="muted" style={{ fontSize: 14, lineHeight: 1.5 }}>
            {TAGLINE} Write [pause 2s] for a longer pause.
          </div>
        </header>
        <div style={{ display: "flex", gap: 10 }}>
          <button
            className="btn primary"
            onClick={paste}
            style={{ flex: 1, justifyContent: "center" }}
            type="button"
          >
            Paste
          </button>
          {openButton("btn ghost", { flex: 1, justifyContent: "center" })}
        </div>
        {box}
        <section
          aria-label="What Booth will make"
          className="glass"
          style={{
            padding: 16,
            display: "flex",
            flexDirection: "column",
            gap: 14,
            borderRadius: 18,
          }}
        >
          {summary}
        </section>
        <Recent onOpen={onStarted} recent={recent} />
      </div>
    );
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 22 }}>
      <header style={{ display: "flex", flexDirection: "column", gap: 6 }}>
        <h1 className="h1" style={{ margin: 0, fontSize: 28 }}>
          Long-form
        </h1>
        <div
          className="muted"
          style={{ fontSize: 14, maxWidth: 640, lineHeight: 1.55 }}
        >
          {TAGLINE}
        </div>
        <div
          className="faint small"
          style={{ maxWidth: 640, lineHeight: 1.55 }}
        >
          Booth leaves out notes like [B-roll] and (SFX). Lines starting with a
          name, like NARRATOR:, get that speaker’s voice. Write [pause 2s] where
          you want a longer pause.
        </div>
      </header>
      <div
        style={{
          display: "flex",
          flexWrap: "wrap",
          gap: 22,
          alignItems: "stretch",
        }}
      >
        <section
          aria-label="Your script"
          className="glass"
          style={{
            flex: "1 1 380px",
            minWidth: 0,
            padding: 20,
            display: "flex",
            flexDirection: "column",
            gap: 14,
          }}
        >
          <div
            style={{
              display: "flex",
              justifyContent: "space-between",
              alignItems: "center",
              gap: 10,
              flexWrap: "wrap",
            }}
          >
            <span style={{ fontSize: 15, fontWeight: 600 }}>Your script</span>
            <div style={{ display: "flex", gap: 8 }}>
              {openButton("btn ghost sm")}
              <button
                className="btn ghost sm"
                disabled={!text}
                onClick={() => setText("")}
                type="button"
              >
                Clear
              </button>
            </div>
          </div>
          {box}
          <div className="faint small" style={{ lineHeight: 1.5 }}>
            Long paragraphs are split at sentence ends. Open or drop a Word
            file, PDF, ebook, web page or text file.
          </div>
        </section>
        <section
          aria-label="What Booth will make"
          className="glass"
          style={{
            flex: "1 1 380px",
            minWidth: 0,
            padding: 20,
            display: "flex",
            flexDirection: "column",
            gap: 16,
          }}
        >
          <h2 className="h2" style={{ margin: 0 }}>
            What Booth will make
          </h2>
          {summary}
        </section>
      </div>
      <Recent onOpen={onStarted} recent={recent} />
    </div>
  );
}

function Recent({
  recent,
  onOpen,
}: {
  recent: ScriptJob[];
  onOpen: (j: ScriptJob) => void;
}) {
  if (!recent.length) {
    return null;
  }
  return (
    <section
      aria-label="Recent scripts"
      style={{ display: "flex", flexDirection: "column", gap: 8 }}
    >
      <div className="faint" style={eyebrow}>
        Recent scripts
      </div>
      {recent.map((j) => {
        const done = j.lines.filter((l) => l.state === "done").length;
        return (
          <button
            className="tile line-pick"
            key={j.id}
            onClick={() => onOpen(j)}
            style={{
              padding: "10px 14px",
              display: "flex",
              justifyContent: "space-between",
              gap: 12,
              textAlign: "left",
            }}
            type="button"
          >
            <span
              style={{
                minWidth: 0,
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
              }}
            >
              {j.title}
            </span>
            <span className="faint small mono" style={{ flex: "none" }}>
              {done === j.lines.length
                ? plural(j.lines.length, "line")
                : `${done} of ${j.lines.length} made`}
            </span>
          </button>
        );
      })}
    </section>
  );
}

// ---------- making and exporting ----------

const GAPS = [
  { value: "150", label: "Tight · 0.15 s" },
  { value: "250", label: "Normal · 0.25 s" },
  { value: "400", label: "Relaxed · 0.4 s" },
];
const FORMATS = [
  { value: "wav", label: "WAV · best quality" },
  { value: "mp3", label: "MP3 · smaller file" },
];
const CAPTION_STYLES = [
  { value: "sentences", label: "Sentences" },
  { value: "short", label: "Short · 2 to 4 words, for Shorts, TikTok, Reels" },
];
const PARA_GAPS = [
  { value: "400", label: "0.4 s" },
  { value: "600", label: "0.6 s" },
  { value: "1000", label: "1 s" },
];

/** A row redraws only when its own line changes, not on every line of a long script. */
const sameRow = (
  a: { jobId: string; line: ScriptJobLine; i: number; phone: boolean },
  b: { jobId: string; line: ScriptJobLine; i: number; phone: boolean }
) =>
  a.jobId === b.jobId &&
  a.i === b.i &&
  a.phone === b.phone &&
  a.line.state === b.line.state &&
  a.line.version === b.line.version &&
  a.line.text === b.line.text &&
  a.line.seconds === b.line.seconds &&
  a.line.path === b.line.path &&
  a.line.error === b.line.error &&
  a.line.drifted === b.line.drifted &&
  a.line.speaker === b.line.speaker &&
  a.line.voice_name === b.line.voice_name &&
  a.line.para === b.line.para &&
  a.line.pause_ms === b.line.pause_ms;

const LineRow = memo(function LineRow({
  jobId,
  line,
  i,
  phone,
}: {
  jobId: string;
  line: ScriptJobLine;
  i: number;
  phone: boolean;
}) {
  const toast = useToast();
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(line.text);
  const src = line.state === "done" && line.path ? fileUrl(line.path) : null;
  const redo = async (text?: string) => {
    setEditing(false);
    try {
      await api.scriptRedo(jobId, i, text);
    } catch (e) {
      toast.bad(errText(e));
    }
  };
  const making = line.state === "making";
  const waiting = line.state === "waiting";
  return (
    <div
      style={{
        marginTop: line.para && i > 0 ? 10 : undefined,
        // long scripts: rows off screen aren't laid out until scrolled to
        contentVisibility: "auto",
        containIntrinsicSize: "auto 64px",
      }}
    >
      {line.pause_ms && i > 0 ? (
        <div
          className="mono faint"
          style={{ fontSize: 11.5, padding: "0 0 6px 44px" }}
        >
          {pauseText(line.pause_ms)}
        </div>
      ) : null}
      <div
        className={making ? "tile on" : "tile"}
        style={{
          padding: phone ? "10px 12px" : "12px 14px",
          display: "flex",
          alignItems: "center",
          gap: 12,
        }}
      >
        <span
          className="mono"
          style={{
            color: "var(--b-fainter)",
            fontSize: 12,
            width: 18,
            flex: "none",
          }}
        >
          {String(i + 1).padStart(2, "0")}
        </span>
        <div
          style={{
            flex: 1,
            minWidth: 0,
            display: "flex",
            flexDirection: "column",
            gap: 3,
          }}
        >
          {phone ? null : (
            <span className="faint small">
              {line.speaker ? (
                <>
                  <span className="mono">{line.speaker}</span> ·{" "}
                </>
              ) : null}
              {line.voice_name}
              {line.drifted ? (
                <span className="warn"> · may not sound like the voice</span>
              ) : null}
            </span>
          )}
          {editing ? (
            <input
              aria-label={`Line ${i + 1}`}
              autoFocus
              className="field"
              onBlur={() => setEditing(false)}
              onChange={(e) => setDraft(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  redo(draft);
                } else if (e.key === "Escape") {
                  setEditing(false);
                }
              }}
              value={draft}
            />
          ) : (
            <button
              aria-label={`Edit line ${i + 1}: ${line.text}`}
              className="line-pick"
              disabled={making}
              onClick={() => {
                setDraft(line.text);
                setEditing(true);
              }}
              style={{
                fontSize: phone ? 14 : 15,
                lineHeight: 1.45,
                color: waiting ? "var(--b-muted)" : undefined,
                ...(phone
                  ? {
                      whiteSpace: "nowrap",
                      overflow: "hidden",
                      textOverflow: "ellipsis",
                    }
                  : {}),
              }}
              type="button"
            >
              {line.text}
            </button>
          )}
          {line.state === "failed" ? (
            <span className="warn small">
              {line.error ?? "Couldn’t make this line."}
            </span>
          ) : null}
        </div>
        {making ? (
          <span
            className="small"
            style={{
              display: "inline-flex",
              gap: 6,
              alignItems: "center",
              flex: "none",
            }}
          >
            <span className="spinner" /> Making
          </span>
        ) : waiting ? (
          <span className="faint small" style={{ flex: "none" }}>
            Waiting
          </span>
        ) : (
          <>
            {line.state === "done" ? (
              <span
                className="mono small"
                style={{ color: "var(--b-ok)", flex: "none" }}
              >
                {line.seconds.toFixed(1)} s
              </span>
            ) : null}
            {src ? <PlayButton label={`line ${i + 1}`} src={src} /> : null}
            {phone && line.state === "done" ? null : (
              <button
                aria-label={`Redo line ${i + 1}`}
                className="btn ghost sm"
                onClick={() => redo()}
                style={{ flex: "none" }}
                title="Make this line again"
                type="button"
              >
                {Icon.redo}
                {phone ? null : line.state === "failed" ? "Try again" : "Redo"}
              </button>
            )}
          </>
        )}
      </div>
    </div>
  );
}, sameRow);

function JobView({ job, onNew }: { job: ScriptJob; onNew: () => void }) {
  const system = useBooth((s) => s.system);
  const phone = isPhone(system);
  const toast = useToast();
  const [gap, setGap] = usePref("booth.script.gap", "250");
  const [paraGap, setParaGap] = usePref("booth.script.paraGap", "600");
  const [wantVoiceover, setWantVoiceover] = usePref(
    "booth.script.voiceover",
    true
  );
  const [wantCaptions, setWantCaptions] = usePref(
    "booth.script.captions",
    true
  );
  const [wantLines, setWantLines] = usePref("booth.script.lines", false);
  const [format, setFormat] = usePref<"wav" | "mp3">(
    "booth.script.format",
    "wav"
  );
  const [evenLoudness, setEvenLoudness] = usePref(
    "booth.script.evenLoudness",
    false
  );
  const [captionStyle, setCaptionStyle] = usePref<"sentences" | "short">(
    "booth.script.captionStyle",
    "sentences"
  );
  const [wantVtt, setWantVtt] = usePref("booth.script.vtt", false);
  const [srt, setSrt] = useState("");
  const [exporting, setExporting] = useState(false);

  const n = job.lines.length;
  const done = job.lines.filter((l) => l.state === "done").length;
  const failed = job.lines.filter((l) => l.state === "failed").length;
  const makingAt = job.lines.findIndex((l) => l.state === "making");
  const allDone = done === n;
  const seconds = job.lines.reduce(
    (s, l) => s + l.seconds + (l.pause_ms ?? 0) / 1000,
    0
  );
  const options: ScriptExportOptions = {
    gap_ms: Number(gap),
    para_gap_ms: Number(paraGap),
    voiceover: wantVoiceover,
    captions: wantCaptions,
    lines: wantLines && !phone,
    format,
    even_loudness: evenLoudness,
    caption_style: captionStyle,
    vtt: wantVtt,
  };
  const hasParas = job.lines.some((l) => l.para);

  useEffect(() => {
    if (!allDone || phone) {
      setSrt("");
      return;
    }
    api
      .scriptCaptions(job.id, options)
      .then(setSrt)
      .catch(() => setSrt(""));
  }, [
    allDone,
    job.id,
    gap,
    paraGap,
    captionStyle,
    phone,
    job.lines.map((l) => `${l.version}`).join(),
  ]);

  const status = job.running
    ? makingAt >= 0
      ? `Making line ${makingAt + 1} of ${n}`
      : "Starting…"
    : allDone
      ? `All ${plural(n, "line")} made`
      : failed
        ? `${plural(failed, "line")} didn’t work`
        : `Stopped · ${done} of ${n} made`;

  const exportNow = async () => {
    setExporting(true);
    try {
      const out = await api.scriptExport(job.id, options);
      if (phone) {
        const files = [out.voiceover, out.captions, out.vtt].filter(
          (p): p is string => !!p
        );
        await api.shareFiles(files, job.title);
      } else {
        toast.ok(
          `Saved ${andList([out.voiceover && "the voiceover", out.captions && "captions", out.lines.length && "each line"])}`
        );
      }
    } catch (e) {
      toast.bad(errText(e));
    } finally {
      setExporting(false);
    }
  };

  const exported = job.exported;
  const check = (on: boolean, set: (v: boolean) => void, label: string) => (
    <label
      style={{
        display: "flex",
        gap: 12,
        alignItems: "center",
        fontSize: 14,
        minHeight: 32,
        cursor: "pointer",
      }}
    >
      <input
        checked={on}
        onChange={(e) => set(e.target.checked)}
        style={{ width: 18, height: 18, margin: 0, accentColor: "#f0202a" }}
        type="checkbox"
      />
      {label}
    </label>
  );
  const nothingPicked = !(
    options.voiceover ||
    options.captions ||
    options.lines
  );

  const exportPanel = (
    <aside
      aria-label={phone ? "When it’s done" : "Export"}
      className="glass"
      style={{
        flex: "1 1 300px",
        maxWidth: phone ? undefined : 380,
        padding: phone ? 16 : 20,
        borderRadius: phone ? 18 : undefined,
        display: "flex",
        flexDirection: "column",
        gap: 14,
        alignSelf: "flex-start",
      }}
    >
      <div className="faint" style={eyebrow}>
        {phone ? "When it’s done" : "Export"}
      </div>
      <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
        {check(
          wantVoiceover,
          setWantVoiceover,
          "The whole voiceover, one file"
        )}
        {check(
          wantCaptions,
          setWantCaptions,
          phone ? "Captions (.srt) for CapCut or YouTube" : "Captions (.srt)"
        )}
        {phone
          ? null
          : check(wantLines, setWantLines, "Each line as its own file")}
        {wantCaptions
          ? check(wantVtt, setWantVtt, "Captions as .vtt too")
          : null}
        {check(evenLoudness, setEvenLoudness, "Even out loudness")}
      </div>
      <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
        <span className="faint small">Save audio as</span>
        <Dropdown
          label="Save audio as"
          onChange={(v) => setFormat(v as "wav" | "mp3")}
          options={FORMATS}
          value={format}
        />
        {wantCaptions ? (
          <>
            <span className="faint small">Caption style</span>
            <Dropdown
              label="Caption style"
              onChange={(v) => setCaptionStyle(v as "sentences" | "short")}
              options={CAPTION_STYLES}
              value={captionStyle}
            />
          </>
        ) : null}
        <span className="faint small">Pause between lines</span>
        <Dropdown
          label="Pause between lines"
          onChange={setGap}
          options={GAPS}
          value={gap}
        />
        {hasParas ? (
          <>
            <span className="faint small">Pause between paragraphs</span>
            <Dropdown
              label="Pause between paragraphs"
              onChange={setParaGap}
              options={PARA_GAPS}
              value={paraGap}
            />
          </>
        ) : null}
      </div>
      {srt ? (
        <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
          <span className="faint small">Captions preview</span>
          <pre
            className="mono"
            style={{
              margin: 0,
              maxHeight: 180,
              overflow: "auto",
              fontSize: 11.5,
              lineHeight: 1.55,
              padding: 12,
              borderRadius: 12,
              background: "rgba(0,0,0,0.3)",
              color: "var(--b-text-2)",
              whiteSpace: "pre-wrap",
            }}
          >
            {srt.split("\n\n").slice(0, 6).join("\n\n")}
          </pre>
        </div>
      ) : null}
      <button
        className="btn primary"
        disabled={!allDone || exporting || nothingPicked}
        onClick={exportNow}
        style={{ justifyContent: "center", width: "100%" }}
        type="button"
      >
        {exporting ? <span className="spinner" /> : phone ? Icon.share : null}
        {allDone
          ? phone
            ? "Share"
            : "Export"
          : phone
            ? "Share when ready"
            : `Export when all ${plural(n, "line")} are made`}
      </button>
      {exported && !phone ? (
        <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
          <div className="muted small" style={{ lineHeight: 1.5 }}>
            Saved {secs(exported.seconds)} of audio
            {exported.loudness
              ? `, evened out to ${exported.loudness.lufs.toFixed(1)} LUFS`
              : ""}
            . The voiceover is in your Library too.
          </div>
          <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
            {exported.voiceover ? (
              <PlayButton
                label="the voiceover"
                src={fileUrl(exported.voiceover)}
              />
            ) : null}
            <button
              className="btn ghost sm"
              onClick={() =>
                api
                  .reveal(exported.voiceover ?? exported.folder)
                  .catch((e) => toast.bad(errText(e)))
              }
              type="button"
            >
              {Icon.folder} Show in folder
            </button>
          </div>
        </div>
      ) : null}
    </aside>
  );

  const madeSrcs = job.lines
    .filter((l) => l.state === "done" && l.path)
    .map((l) => fileUrl(l.path as string));

  return (
    <div
      style={{ display: "flex", flexDirection: "column", gap: phone ? 14 : 20 }}
    >
      <header
        style={{
          display: "flex",
          alignItems: "flex-end",
          justifyContent: "space-between",
          gap: 12,
          flexWrap: "wrap",
        }}
      >
        <div
          style={{
            display: "flex",
            flexDirection: "column",
            gap: 4,
            minWidth: 0,
          }}
        >
          <div className="faint" style={eyebrow}>
            Long-form
          </div>
          <h1
            className="h1"
            style={{ margin: 0, fontSize: phone ? 22 : 28, lineHeight: 1.25 }}
          >
            {job.title}
          </h1>
          <div className="muted small">
            {plural(n, "line")} ·{" "}
            {job.lines.reduce((s, l) => s + words(l.text), 0)} words ·{" "}
            {allDone ? secs(seconds) : `~${secs(estimate(job.lines))}`}
          </div>
        </div>
        <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
          {madeSrcs.length && !phone ? (
            <button
              className="btn ghost sm"
              onClick={() => playAll(madeSrcs)}
              type="button"
            >
              {Icon.play} Play all
            </button>
          ) : null}
          <button className="btn ghost sm" onClick={onNew} type="button">
            New script
          </button>
        </div>
      </header>

      <section
        aria-label="Progress"
        className="glass"
        style={{
          padding: "14px 18px",
          display: "flex",
          flexDirection: "column",
          gap: 10,
          borderRadius: 18,
        }}
      >
        <div
          style={{
            display: "flex",
            justifyContent: "space-between",
            alignItems: "center",
            gap: 10,
            flexWrap: "wrap",
          }}
        >
          <span aria-live="polite" style={{ fontWeight: 600 }}>
            {status}
          </span>
          <span style={{ display: "flex", gap: 10, alignItems: "center" }}>
            <span className="mono muted small">{done} done</span>
            {job.running ? (
              <button
                className="btn ghost sm"
                onClick={() => api.scriptStop().catch(() => undefined)}
                type="button"
              >
                Stop
              </button>
            ) : allDone ? null : (
              <button
                className="btn primary sm"
                onClick={() =>
                  api.scriptRun(job.id).catch((e) => toast.bad(errText(e)))
                }
                type="button"
              >
                {failed ? "Try again" : "Make the rest"}
              </button>
            )}
          </span>
        </div>
        <div
          style={{
            height: 6,
            borderRadius: 3,
            background: "rgba(255,255,255,0.08)",
            overflow: "hidden",
          }}
        >
          <div
            style={{
              width: `${(done / Math.max(n, 1)) * 100}%`,
              height: "100%",
              background: "var(--b-red-grad)",
              transition: "width 300ms ease",
            }}
          />
        </div>
        {job.running ? (
          <div className="muted small" style={{ lineHeight: 1.45 }}>
            {phone
              ? "Keeps going with the screen off. Booth shows a notification while it works."
              : "Keeps going if you switch pages or use other apps."}
          </div>
        ) : null}
      </section>

      <div
        style={{
          display: "flex",
          gap: 16,
          flexWrap: "wrap",
          alignItems: "flex-start",
        }}
      >
        <section
          aria-label="Lines"
          style={{
            flex: "999 1 480px",
            minWidth: 0,
            display: "flex",
            flexDirection: "column",
            gap: 8,
          }}
        >
          {job.lines.map((l, i) => (
            <LineRow
              i={i}
              jobId={job.id}
              key={`${i}-${l.version}`}
              line={l}
              phone={phone}
            />
          ))}
          {job.skipped.length ? (
            <details className="tile" style={{ padding: "10px 14px" }}>
              <summary className="muted small" style={{ cursor: "pointer" }}>
                {plural(job.skipped.length, "note")} left out
              </summary>
              <div
                className="mono faint"
                style={{
                  fontSize: 12,
                  lineHeight: 1.8,
                  marginTop: 8,
                  wordBreak: "break-word",
                }}
              >
                {job.skipped.map((s, i) => (
                  <div key={`${i}-${s}`}>{s}</div>
                ))}
              </div>
            </details>
          ) : null}
        </section>
        {exportPanel}
      </div>
    </div>
  );
}

/** The Long-form page (the Script view): paste a whole script, make every line in the background, export the
 * voiceover with captions (desktop) or share it (phone). */
export function Script() {
  const toast = useToast();
  const [openId, setOpenId] = usePref<string | null>("booth.script.open", null);
  const [job, setJob] = useState<ScriptJob | null>(null);
  const [recent, setRecent] = useState<ScriptJob[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [opening, setOpening] = useState(false);
  /** A file is being dragged over the window (the name, when the app knows it). */
  const [dragging, setDragging] = useState<string | null>(null);
  const dragDepth = useRef(0);

  /** Read a document into the draft (and back to the paste box if a script is open). */
  const openDoc = useCallback(
    async (f: DocSource) => {
      // booth-core says which kinds it opens when it can't read one (and sniffs files without
      // an extension, which Android pickers sometimes hand over).
      setOpening(true);
      try {
        const text = await api.scriptOpen(
          f instanceof File
            ? {
                name: f.name,
                data: base64(new Uint8Array(await f.arrayBuffer())),
              }
            : f
        );
        setDraft(text);
        setOpenId(null);
        setJob(null);
      } catch (e) {
        toast.bad(errText(e));
      } finally {
        setOpening(false);
      }
    },
    [toast, setOpenId]
  );

  // Desktop: the window hands over dropped files as paths. Only while this page shows.
  useEffect(() => {
    const tauri = (
      window as unknown as { __TAURI_INTERNALS__?: { metadata?: unknown } }
    ).__TAURI_INTERNALS__;
    if (!tauri?.metadata) {
      return; // the browser preview uses the page's own drop events below
    }
    let dead = false;
    let un: (() => void) | undefined;
    getCurrentWebview()
      .onDragDropEvent((ev: { payload: DragDropEvent }) => {
        const p = ev.payload;
        if (p.type === "enter") {
          const path = p.paths[0];
          setDragging(path ? baseName(path) : "");
        } else if (p.type === "leave") {
          setDragging(null);
        } else if (p.type === "drop") {
          setDragging(null);
          const path =
            p.paths.find((x: string) => DOC_EXT.includes(extOf(x))) ??
            p.paths[0];
          if (path) {
            openDoc({ name: baseName(path), path });
          }
        }
      })
      .then((f) => {
        if (dead) {
          f();
        } else {
          un = f;
        }
      })
      .catch(() => undefined);
    return () => {
      dead = true;
      un?.();
      setDragging(null);
    };
  }, [openDoc]);

  const hasFiles = (e: DragEvent) => e.dataTransfer.types.includes("Files");
  const dropProps = {
    onDragEnter: (e: DragEvent) => {
      if (hasFiles(e)) {
        dragDepth.current += 1;
        setDragging("");
      }
    },
    onDragOver: (e: DragEvent) => {
      if (hasFiles(e)) {
        e.preventDefault();
        e.dataTransfer.dropEffect = "copy";
      }
    },
    onDragLeave: (e: DragEvent) => {
      if (hasFiles(e)) {
        dragDepth.current = Math.max(0, dragDepth.current - 1);
        if (!dragDepth.current) {
          setDragging(null);
        }
      }
    },
    onDrop: (e: DragEvent) => {
      if (!hasFiles(e)) {
        return;
      }
      e.preventDefault();
      dragDepth.current = 0;
      setDragging(null);
      const f = e.dataTransfer.files[0];
      if (f) {
        openDoc(f);
      }
    },
  };
  const dropHint =
    dragging === null ? null : (
      <div
        aria-hidden
        style={{
          position: "fixed",
          inset: 0,
          zIndex: 50,
          pointerEvents: "none",
          padding: 14,
          background: "rgba(8,8,10,0.55)",
          backdropFilter: "blur(2px)",
        }}
      >
        <div
          style={{
            height: "100%",
            borderRadius: 22,
            border: "2px dashed #f0202a",
            display: "flex",
            flexDirection: "column",
            alignItems: "center",
            justifyContent: "center",
            gap: 6,
          }}
        >
          <span style={{ fontSize: 20, fontWeight: 600 }}>Drop to open</span>
          <span className="muted small">
            {dragging.includes(".") && !DOC_EXT.includes(extOf(dragging))
              ? `Booth can’t read ${dragging}`
              : dragging || "Word, PDF, ebook, web page or text"}
          </span>
        </div>
      </div>
    );

  useEffect(() => {
    let alive = true;
    const load = async () => {
      const list = await api.scriptList(6).catch(() => [] as ScriptJob[]);
      if (!alive) {
        return;
      }
      setRecent(list);
      const open =
        (openId && list.find((j) => j.id === openId)) ||
        list.find((j) => j.running) ||
        null;
      if (open) {
        setJob(await api.scriptGet(open.id).catch(() => open));
      }
      setLoaded(true);
    };
    load();
    return () => {
      alive = false;
    };
    // only on opening the page
  }, []);

  useEffect(() => {
    const un = [
      api.onScript((j) => {
        setJob((cur) => (cur && cur.id === j.id ? j : cur));
        setRecent((r) => r.map((x) => (x.id === j.id ? j : x)));
      }),
      api.onScriptError((e) => toast.bad(e.error)),
    ];
    return () => {
      for (const p of un) {
        p.then((f) => f()).catch(() => undefined);
      }
    };
  }, [toast]);

  if (!loaded) {
    return null;
  }
  return (
    <div {...dropProps} style={{ minHeight: "100%" }}>
      {job ? (
        <JobView
          job={job}
          onNew={() => {
            setOpenId(null);
            setJob(null);
            api
              .scriptList(6)
              .then(setRecent)
              .catch(() => undefined);
          }}
        />
      ) : (
        <PasteView
          onOpen={openDoc}
          onStarted={(j) => {
            setOpenId(j.id);
            setJob(j);
          }}
          opening={opening}
          recent={recent}
        />
      )}
      {dropHint}
    </div>
  );
}
