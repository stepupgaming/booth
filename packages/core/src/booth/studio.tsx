"use client";

import { useMemo, useRef, useState } from "react";
import { api, canExact, canShape, errText, fileUrl, type Mode, type Take, voiceDesc } from "@workspace/core/booth/api";
import { controlById, currentVoice, packPresent, round2, toDose, useBooth } from "@workspace/core/booth/store";
import { DoseSlider, Dropdown, Orb, PlayButton, playAll, usePref, useToast, Wave } from "@workspace/core/booth/ui";

interface Line {
  id: string;
  text: string;
  takes: Take[];
  kept: string | null;
}

interface Sound {
  mood: string | null;
  strength: number;
  extras: string[];
  delivery: Record<string, number>;
  gender: number;
  age: number;
  texture: string | null;
  textureAmt: number;
  room: number;
  accent: string | null;
  accentAmt: number;
}

const NEW_SOUND: Sound = {
  mood: null,
  strength: 60,
  extras: [],
  delivery: {},
  gender: 0,
  age: 0,
  texture: null,
  textureAmt: 50,
  room: 0,
  accent: null,
  accentAmt: 70,
};

const DELIVERY: { key: string; label: string; poles: string }[] = [
  { key: "pitch", label: "Pitch", poles: "lower · higher" },
  { key: "projection", label: "Projection", poles: "whisper · loud" },
  { key: "pace", label: "Pace", poles: "slower · faster" },
  { key: "chatty", label: "Style", poles: "read · chatty" },
  { key: "lively", label: "Liveliness", poles: "flat · lively" },
  { key: "hesitant", label: "Hesitation", poles: "fluent · hesitant" },
];
const TEXTURES = ["deep", "bright", "husky", "raspy", "soft", "authority"];
/** delivery.pitch works in Exact too; +2 is about +15 semitones, so keep it small. */
const PITCH_MAX = 0.7;

const newId = () => Math.random().toString(36).slice(2, 10);
// The engine's languages (Zyphra's languages.json). English first; the rest by name.
const OTHER_LANGUAGES = [
  ["ar", "Arabic"], ["bn", "Bengali"], ["bg", "Bulgarian"], ["cs", "Czech"], ["da", "Danish"],
  ["nl", "Dutch"], ["fi", "Finnish"], ["fr_fr", "French"], ["ka", "Georgian"], ["de", "German"],
  ["el", "Greek"], ["gu", "Gujarati"], ["he", "Hebrew"], ["hi", "Hindi"], ["hu", "Hungarian"],
  ["id", "Indonesian"], ["it", "Italian"], ["ja", "Japanese"], ["kn", "Kannada"], ["ko", "Korean"],
  ["ms", "Malay"], ["ml", "Malayalam"], ["cmn", "Mandarin"], ["mr", "Marathi"], ["no", "Norwegian"],
  ["or", "Odia"], ["pl", "Polish"], ["pt_br", "Portuguese"], ["pa", "Punjabi"], ["ro", "Romanian"],
  ["ru", "Russian"], ["sk", "Slovak"], ["es", "Spanish"], ["sv", "Swedish"], ["tl", "Tagalog"],
  ["ta", "Tamil"], ["te", "Telugu"], ["th", "Thai"], ["tr", "Turkish"], ["uk", "Ukrainian"],
  ["ur", "Urdu"], ["vi", "Vietnamese"],
].map(([value, label]) => ({ value: value as string, label: label as string }));

const seed = () => 1 + Math.floor(Math.random() * 2_147_483_000);
const firstLines: Line[] = [
  { id: "a", text: "I honestly didn't think we'd make it this far.", takes: [], kept: null },
  { id: "b", text: "But here we are, and the whole team is waiting outside.", takes: [], kept: null },
];

export function Studio() {
  const toast = useToast();
  const catalog = useBooth((s) => s.catalog);
  const catalogError = useBooth((s) => s.catalogError);
  const loadCatalog = useBooth((s) => s.loadCatalog);
  const overview = useBooth((s) => s.overview);
  const voice = useBooth(currentVoice);
  const go = useBooth((s) => s.go);
  const work = useBooth((s) => s.work);
  const refresh = useBooth((s) => s.refresh);

  const [lines, setLines] = usePref<Line[]>("lines", firstLines);
  const [sel, setSel] = usePref("selLine", 0);
  const [modePref, setMode] = usePref<Mode>("mode", "exact");
  const [sound, setSound] = usePref<Sound>("sound", NEW_SOUND);
  const [perLine] = usePref("takesPerLine", 3);
  const [running, setRunning] = useState<string | null>(null);
  const stopRef = useRef(false);
  const linesRef = useRef(lines);
  linesRef.current = lines;

  const exactOk = voice ? canExact(voice) : true;
  const shapeOk = voice ? canShape(voice) : true;
  const mode: Mode = modePref === "exact" && !exactOk ? "shapeable" : modePref === "shapeable" && !shapeOk ? "exact" : modePref;
  const exact = mode === "exact";
  const selIdx = Math.min(sel, lines.length - 1);
  const selLine = lines[selIdx];

  const up = (patch: Partial<Sound>) => setSound({ ...sound, ...patch });
  const setLine = (i: number, patch: Partial<Line>) => {
    const next = linesRef.current.map((l, j) => (j === i ? { ...l, ...patch } : l));
    linesRef.current = next;
    setLines(next);
  };

  const controls = useMemo(() => {
    const out: Record<string, number> = {};
    if (!catalog) {
      return out;
    }
    const k = sound.strength / 60;
    const mood = catalog.moods.find((m) => m.id === sound.mood);
    if (mood) {
      const c = controlById(catalog, mood.control);
      const max = c?.range?.[1] ?? mood.dose * 1.5;
      out[mood.control] = round2(Math.min(max, mood.dose * k));
    }
    for (const e of sound.extras) {
      out[`emotions.${e}`] = round2(3 * k);
    }
    for (const d of DELIVERY) {
      const v = sound.delivery[d.key] ?? 0;
      if (v === 0) {
        continue;
      }
      if (d.key === "pitch") {
        out["delivery.pitch"] = round2((v / 100) * PITCH_MAX);
      } else if (controlById(catalog, d.key)) {
        out[d.key] = toDose(controlById(catalog, d.key), v);
      }
    }
    if (!exact) {
      if (sound.gender) {
        out.gender = toDose(controlById(catalog, "gender"), sound.gender);
      }
      if (sound.age) {
        out.age = toDose(controlById(catalog, "age"), sound.age);
      }
      if (sound.room) {
        out.room = toDose(controlById(catalog, "room"), sound.room);
      }
      if (sound.texture && sound.textureAmt > 0) {
        out[sound.texture] = toDose(controlById(catalog, sound.texture), sound.textureAmt);
      }
      const acc = catalog.accents.find((a) => a.id === sound.accent);
      if (acc && sound.accentAmt > 0) {
        out[`accent:${acc.id}`] = round2((sound.accentAmt / 100) * (acc.safe?.[1] ?? 1.5));
      }
    }
    return out;
  }, [catalog, sound, exact]);

  const renderLine = async (i: number, replace: boolean) => {
    const line = linesRef.current[i];
    if (!(line?.text.trim() && voice)) {
      return;
    }
    let takes = replace ? [] : line.takes;
    setLine(i, { takes, kept: replace ? null : line.kept });
    for (let n = 0; n < perLine && !stopRef.current; n++) {
      const engineUp = useBooth.getState().overview?.engine.state === "running";
      setRunning(`${engineUp ? "Line" : "Starting the engine · line"} ${i + 1} · take ${n + 1} of ${perLine}`);
      const t = await work(() =>
        api.speak({ text: line.text.trim(), voice: voice.id, mode, controls, seed: seed() })
      );
      takes = [...takes, t];
      const cur = linesRef.current[i];
      setLine(i, { takes, kept: cur?.kept ?? t.id });
    }
  };

  const run = async (which: number[], replace: boolean) => {
    if (running) {
      stopRef.current = true;
      return;
    }
    stopRef.current = false;
    try {
      for (const i of which) {
        if (stopRef.current) {
          break;
        }
        setSel(i);
        await renderLine(i, replace);
      }
    } catch (e) {
      toast.bad(errText(e));
    } finally {
      setRunning(null);
      stopRef.current = false;
      refresh();
    }
  };

  const allDone = lines.every((l) => !l.text.trim() || l.takes.length > 0);
  const generate = () => {
    const todo = lines.map((l, i) => ({ l, i })).filter(({ l }) => l.text.trim() && (allDone || l.takes.length === 0));
    run(
      todo.map((x) => x.i),
      true
    );
  };

  const keptTake = (l: Line) => l.takes.find((t) => t.id === l.kept) ?? l.takes[0];
  const keptList = lines.map(keptTake).filter((t): t is Take => !!t);

  const exportAll = async () => {
    try {
      const t = await api.exportTakes(
        keptList.map((t) => t.id),
        250
      );
      toast.ok(`Exported ${t.seconds.toFixed(1)} s to ${t.path}`);
      api.reveal(t.path).catch(() => undefined);
    } catch (e) {
      toast.bad(errText(e));
    }
  };

  const onPaste = (i: number, e: React.ClipboardEvent<HTMLInputElement>) => {
    const text = e.clipboardData.getData("text");
    const parts = text
      .split(/\r?\n/)
      .map((s) => s.trim())
      .filter(Boolean);
    if (parts.length < 2) {
      return;
    }
    e.preventDefault();
    const cur = lines[i];
    const fresh = parts.map((p) => ({ id: newId(), text: p, takes: [], kept: null }));
    const next = [...lines.slice(0, i), ...(cur?.text.trim() ? [cur] : []), ...fresh, ...lines.slice(i + 1)];
    setLines(next);
  };

  const decoder = overview?.settings.decoder ?? "keep2_plain";
  const fullOk = packPresent(overview, "full_decoder");
  const setDecoder = async (d: string) => {
    const s = overview?.settings;
    if (!s) {
      return;
    }
    try {
      await api.saveSettings({ ...s, decoder: d });
      await refresh();
    } catch (e) {
      toast.bad(errText(e));
    }
  };

  const language = overview?.settings.language ?? "en_us";
  const setLanguage = async (l: string) => {
    const s = overview?.settings;
    if (!s) {
      return;
    }
    try {
      await api.saveSettings({ ...s, language: l === "en_us" ? null : l });
      await refresh();
    } catch (e) {
      toast.bad(errText(e));
    }
  };

  const moodIds = new Set(catalog?.moods.map((m) => controlById(catalog, m.control)) .flatMap((c) => Object.keys(c?.map["+"] ?? {})) ?? []);
  const extraEmotions = (catalog?.directions ?? [])
    .filter((d) => d.startsWith("emotions.") && !moodIds.has(d))
    .map((d) => d.slice("emotions.".length));

  return (
    <div style={{ display: "flex", gap: 16, flexWrap: "wrap", alignItems: "flex-start" }}>
      <section aria-label="Script" style={{ flex: "999 1 520px", minWidth: 0, display: "flex", flexDirection: "column", gap: 16 }}>
        <div
          className="glass"
          style={{ padding: "22px 24px", display: "flex", alignItems: "center", gap: 20, flexWrap: "wrap", overflow: "hidden" }}
        >
          <Orb id={voice?.id ?? "x"} size={76} spin={running !== null} />
          <div style={{ display: "flex", flexDirection: "column", gap: 4, flex: "1 1 200px" }}>
            <span className="muted" style={{ fontSize: 12, letterSpacing: "0.08em", textTransform: "uppercase" }}>
              Speaking as
            </span>
            <span style={{ fontSize: 30, fontWeight: 600, letterSpacing: "-0.02em", lineHeight: 1.1 }}>{voice?.name ?? "…"}</span>
            <span className="muted">
              {voice ? `${voiceDesc(voice)} · ${voice.kind === "default" ? "preset" : voice.kind} · ` : ""}
              <button className="link" onClick={() => go("voices")} type="button">
                change voice
              </button>
            </span>
          </div>
          <div style={{ display: "flex", flexDirection: "column", gap: 8, alignItems: "flex-end" }}>
            <div aria-label="Voice mode" className="seg" role="group">
              <button aria-pressed={exact} disabled={!exactOk} onClick={() => setMode("exact")} type="button">
                Exact
              </button>
              <button aria-pressed={!exact} disabled={!shapeOk} onClick={() => setMode("shapeable")} type="button">
                Shapeable
              </button>
            </div>
            <span className="muted" style={{ fontSize: 12, maxWidth: 300, textAlign: "right", lineHeight: 1.5 }}>
              {exact
                ? "Hears the recording itself. Closest to the real person. Mood, delivery and pitch work."
                : exactOk
                  ? "Uses a fingerprint of the voice. A little less like them, but gender, age, texture, accent and room open up."
                  : "This voice is a fingerprint only (a Quick clone or a designed voice). Every control works."}
            </span>
          </div>
        </div>

        <div className="glass" style={{ padding: "20px 22px", display: "flex", flexDirection: "column", gap: 10 }}>
          <div style={{ display: "flex", alignItems: "baseline", justifyContent: "space-between", gap: 10, flexWrap: "wrap" }}>
            <h1 style={{ margin: 0, fontSize: 18, fontWeight: 600, letterSpacing: "-0.01em" }}>Script</h1>
            <span className="faint small">One line per take · paste a whole script to split it into lines</span>
          </div>
          {lines.map((l, i) => {
            const on = i === selIdx;
            const k = l.takes.findIndex((t) => t.id === l.kept);
            const status = running && on ? "rendering…" : l.takes.length ? `take ${k + 1} of ${l.takes.length} kept` : "no takes yet";
            return (
              <div
                key={l.id}
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: 12,
                  padding: "10px 12px 10px 14px",
                  borderRadius: 14,
                  ...(on
                    ? {
                        background: "rgba(240,32,42,0.06)",
                        border: "1px solid rgba(240,32,42,0.42)",
                        boxShadow: "inset 3px 0 0 #F0202A",
                      }
                    : { background: "rgba(255,255,255,0.03)", border: "1px solid rgba(255,255,255,0.07)" }),
                }}
              >
                <span className="mono" style={{ color: "var(--b-fainter)", fontSize: 12 }}>
                  {String(i + 1).padStart(2, "0")}
                </span>
                <input
                  aria-label={`Line ${i + 1}`}
                  onChange={(e) => setLine(i, { text: e.target.value })}
                  onFocus={() => setSel(i)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      const next = [...lines];
                      next.splice(i + 1, 0, { id: newId(), text: "", takes: [], kept: null });
                      setLines(next);
                      setSel(i + 1);
                    }
                  }}
                  onPaste={(e) => onPaste(i, e)}
                  placeholder="Type a line…"
                  style={{
                    all: "unset",
                    flex: 1,
                    minWidth: 0,
                    lineHeight: 1.6,
                    fontSize: 15,
                  }}
                  value={l.text}
                />
                <span className="muted small" style={{ whiteSpace: "nowrap" }}>
                  {status}
                </span>
                <button
                  aria-label={`Redo line ${i + 1}`}
                  className="btn ghost sm"
                  disabled={!!running || !l.text.trim()}
                  onClick={() => run([i], true)}
                  type="button"
                >
                  Redo
                </button>
                {lines.length > 1 ? (
                  <button
                    aria-label={`Remove line ${i + 1}`}
                    className="btn ghost sm"
                    disabled={!!running}
                    onClick={() => {
                      setLines(lines.filter((_, j) => j !== i));
                      setSel(Math.max(0, i - 1));
                    }}
                    type="button"
                  >
                    ×
                  </button>
                ) : null}
              </div>
            );
          })}
          <div style={{ display: "flex", gap: 10, flexWrap: "wrap", alignItems: "center", paddingTop: 6 }}>
            <button className="btn primary" disabled={!voice} onClick={generate} type="button">
              {running ? "Stop" : allDone && lines.some((l) => l.takes.length) ? "Generate again" : "Generate"}
            </button>
            <button
              className="btn ghost"
              disabled={keptList.length === 0}
              onClick={() => playAll(keptList.map((t) => fileUrl(t.path)))}
              type="button"
            >
              Play all
            </button>
            <button className="btn ghost" disabled={keptList.length === 0 || !!running} onClick={exportAll} type="button">
              Export
            </button>
            <button
              className="btn ghost"
              disabled={!!running}
              onClick={() => {
                setLines([...lines, { id: newId(), text: "", takes: [], kept: null }]);
                setSel(lines.length);
              }}
              type="button"
            >
              Add line
            </button>
            <span className="faint small" style={{ marginLeft: "auto" }}>
              {running ?? `${perLine} take${perLine === 1 ? "" : "s"} per line`}
            </span>
          </div>
        </div>

        {selLine && selLine.takes.length > 0 ? (
          <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(230px, 1fr))", gap: 14 }}>
            {selLine.takes.map((t, n) => {
              const k = t.id === selLine.kept;
              const src = fileUrl(t.path);
              return (
                <div className={`glass${k ? " lit" : ""}`} key={t.id} style={{ padding: 16, display: "flex", flexDirection: "column", gap: 12 }}>
                  <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
                    <PlayButton label={`take ${n + 1}`} src={src} variant={k ? "red" : "solid"} />
                    <span style={{ display: "flex", flexDirection: "column", lineHeight: 1.3, minWidth: 0, flex: "1 1 0" }}>
                      <span style={{ fontWeight: 600 }}>Take {n + 1}</span>
                      <span
                        className="mono faint"
                        style={{ fontSize: 11, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}
                        title={`seed ${t.seed ?? "?"}`}
                      >
                        {t.seconds.toFixed(1)} s · seed {t.seed ?? "?"}
                      </span>
                    </span>
                    <button
                      className={`btn sm ${k ? "kept" : "ghost"}`}
                      onClick={() => setLine(selIdx, { kept: t.id })}
                      type="button"
                    >
                      {k ? "Kept" : "Keep"}
                    </button>
                  </div>
                  <Wave height={44} seed={t.seed ?? n + 1} src={src} />
                  <div className="faint small">
                    {t.mode === "exact" ? "Exact" : "Shapeable"}
                    {t.decoder ? ` · ${t.decoder === "full_mr" ? "studio decoder" : "fast decoder"}` : ""}
                    {(() => { const n = Object.keys(t.controls).length; return n ? ` · ${n} ${n === 1 ? "control" : "controls"}` : ""; })()}
                  </div>
                </div>
              );
            })}
          </div>
        ) : null}
      </section>

      <aside
        aria-label="Sound controls"
        className="glass"
        style={{ flex: "1 1 340px", maxWidth: 400, minWidth: 300, padding: 22, display: "flex", flexDirection: "column", gap: 22 }}
      >
        {catalog ? null : (
          <div className="muted small" style={{ lineHeight: 1.6 }}>
            {catalogError ? (
              <>
                <span className="warn">Couldn’t load the controls: {catalogError}</span>{" "}
                <button className="link" onClick={() => loadCatalog()} type="button">
                  Try again
                </button>
              </>
            ) : (
              <span style={{ display: "flex", gap: 8, alignItems: "center" }}>
                <span className="spinner" /> Loading the controls…
              </span>
            )}
          </div>
        )}

        <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
          <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between" }}>
            <h2 className="h2">Mood</h2>
            <span className="badge">both modes</span>
          </div>
          <div className="chips">
            <button aria-pressed={sound.mood === null} className="chip" onClick={() => up({ mood: null })} type="button">
              None
            </button>
            {catalog?.moods.map((m) => (
              <button aria-pressed={sound.mood === m.id} className="chip" key={m.id} onClick={() => up({ mood: m.id })} type="button">
                {m.label}
              </button>
            ))}
          </div>
          <DoseSlider
            label="Strength"
            max={100}
            min={0}
            onChange={(v) => up({ strength: v })}
            poles={`${sound.strength}%`}
            step={1}
            value={sound.strength}
          />
          {extraEmotions.length ? (
            <details className="muted" style={{ fontSize: 13 }}>
              <summary style={{ cursor: "pointer", color: "var(--b-text)" }}>
                {extraEmotions.length} more emotions{sound.extras.length ? ` · ${sound.extras.length} on` : ""}
              </summary>
              <div className="chips" style={{ marginTop: 10 }}>
                {extraEmotions.map((e) => {
                  const on = sound.extras.includes(e);
                  return (
                    <button
                      aria-pressed={on}
                      className="chip"
                      key={e}
                      onClick={() => up({ extras: on ? sound.extras.filter((x) => x !== e) : [...sound.extras, e] })}
                      style={{ minHeight: 30, padding: "5px 11px", fontSize: 12 }}
                      type="button"
                    >
                      {e}
                    </button>
                  );
                })}
              </div>
            </details>
          ) : null}
        </div>

        <div className="divider" />

        <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
          <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", marginBottom: 4 }}>
            <h2 className="h2">Delivery</h2>
            <span className="badge">both modes</span>
          </div>
          {DELIVERY.map((d) => (
            <DoseSlider
              disabled={d.key !== "pitch" && !!catalog && !controlById(catalog, d.key)}
              key={d.key}
              label={d.label}
              max={100}
              min={-100}
              onChange={(v) => up({ delivery: { ...sound.delivery, [d.key]: v } })}
              poles={d.poles}
              step={1}
              title="Double-click to reset"
              value={sound.delivery[d.key] ?? 0}
            />
          ))}
        </div>

        <div
          style={{
            display: "flex",
            flexDirection: "column",
            gap: 8,
            padding: 16,
            borderRadius: 18,
            ...(exact
              ? { background: "rgba(0,0,0,0.22)", border: "1px dashed rgba(255,255,255,0.14)" }
              : { background: "rgba(255,255,255,0.05)", border: "1px solid rgba(255,255,255,0.14)" }),
          }}
        >
          <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 8, flexWrap: "wrap" }}>
            <h2 className="h2">Shape the voice</h2>
            <span className={`badge${exact ? "" : " ok"}`}>{exact ? "Shapeable only" : "on"}</span>
          </div>
          {exact ? (
            <p style={{ margin: 0, fontSize: 13, color: "var(--b-text-2)", lineHeight: 1.5 }}>
              These change who is speaking, so they need the Shapeable voice.{" "}
              {shapeOk ? (
                <button className="link" onClick={() => setMode("shapeable")} type="button">
                  Switch
                </button>
              ) : null}
            </p>
          ) : null}
          <DoseSlider
            disabled={exact}
            label="Gender"
            max={100}
            min={-100}
            onChange={(v) => up({ gender: v })}
            poles="masculine · feminine"
            step={1}
            value={sound.gender}
          />
          <DoseSlider
            disabled={exact}
            label="Age"
            max={100}
            min={-100}
            onChange={(v) => up({ age: v })}
            poles="younger · older"
            step={1}
            value={sound.age}
          />
          <span className={exact ? "faint" : undefined} style={{ fontSize: 13, marginTop: 4 }}>
            Texture
          </span>
          <div className="chips" style={{ gap: 6 }}>
            {TEXTURES.filter((t) => !catalog || controlById(catalog, t)).map((t) => (
              <button
                aria-pressed={sound.texture === t}
                className="chip"
                disabled={exact}
                key={t}
                onClick={() => up({ texture: sound.texture === t ? null : t })}
                style={{ minHeight: 30, padding: "5px 11px", fontSize: 12 }}
                type="button"
              >
                {controlById(catalog, t)?.label ?? t}
              </button>
            ))}
          </div>
          {sound.texture ? (
            <DoseSlider
              disabled={exact}
              label="Texture amount"
              max={100}
              min={0}
              onChange={(v) => up({ textureAmt: v })}
              poles={`${sound.textureAmt}%`}
              step={1}
              value={sound.textureAmt}
            />
          ) : null}
          <DoseSlider
            disabled={exact}
            label="Room"
            max={100}
            min={-100}
            onChange={(v) => up({ room: v })}
            poles="phone · studio"
            step={1}
            value={sound.room}
          />
          <label className="lbl" style={{ color: exact ? "var(--b-fainter)" : "var(--b-text)" }}>
            Accent
            <Dropdown
              disabled={exact}
              groups={[
                {
                  label: "English",
                  options: (catalog?.accents ?? [])
                    .filter((a) => a.tab !== "world")
                    .map((a) => ({ value: a.id, label: a.label })),
                },
                {
                  label: "Other languages",
                  options: (catalog?.accents ?? [])
                    .filter((a) => a.tab === "world")
                    .map((a) => ({ value: a.id, label: a.label })),
                },
              ]}
              onChange={(v) => up({ accent: v || null })}
              options={[{ value: "", label: "Keep the voice’s own" }]}
              value={sound.accent ?? ""}
            />
          </label>
          {sound.accent ? (
            <DoseSlider
              disabled={exact}
              label="Accent strength"
              max={100}
              min={0}
              onChange={(v) => up({ accentAmt: v })}
              poles={`${sound.accentAmt}%`}
              step={1}
              value={sound.accentAmt}
            />
          ) : null}
        </div>

        <label style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 12, fontSize: 13 }}>
          Language
          <Dropdown
            groups={[
              {
                label: "English",
                options: [
                  { value: "en_us", label: "English (US)" },
                  { value: "en_gb", label: "English (UK)" },
                ],
              },
              { label: "Other languages", options: OTHER_LANGUAGES },
            ]}
            label="Language"
            onChange={setLanguage}
            value={language}
          />
        </label>
        <label style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 12, fontSize: 13 }}>
          Quality
          <Dropdown
            onChange={setDecoder}
            options={[
              {
                value: "full_mr",
                label: "Best (studio decoder)",
                disabled: !fullOk,
                hint: fullOk ? undefined : "download in Settings",
              },
              { value: "keep2_plain", label: "Fast (pruned decoder)" },
            ]}
            value={fullOk ? decoder : "keep2_plain"}
          />
        </label>
        <button
          className="link small faint"
          onClick={() => setSound(NEW_SOUND)}
          style={{ alignSelf: "flex-start" }}
          type="button"
        >
          Reset all controls
        </button>
      </aside>
    </div>
  );
}
