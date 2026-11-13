"use client";

import { useEffect, useRef, useState } from "react";
import { api, canExact, canShape, errText, fileUrl, type Take, type Voice } from "@workspace/core/booth/api";
import { packPresent, round2, useBooth } from "@workspace/core/booth/store";
import { DoseSlider, Dropdown, Icon, Orb, PlayButton, usePlayer, useToast, Wave } from "@workspace/core/booth/ui";

// ---------- audio helpers ----------

interface Clip {
  name: string;
  buffer: AudioBuffer;
  peaks: number[];
}

const PASSAGE =
  "When the sunlight strikes raindrops in the air, they act as a prism and form a rainbow. " +
  "People look, but no one ever finds it. Some say there is a pot of gold at one end. " +
  "I usually take the long way home, past the old bridge and the bakery on the corner, " +
  "and on a good day I stop for a coffee and just watch the street for a while.";

const TEST_LINE = "The quick brown fox jumps over the lazy dog, and nobody seems to mind.";

let decodeCtx: AudioContext | null = null;
async function decode(data: ArrayBuffer): Promise<AudioBuffer> {
  decodeCtx ??= new AudioContext();
  return await decodeCtx.decodeAudioData(data);
}

function peaksOf(buf: AudioBuffer, n: number): number[] {
  const d = buf.getChannelData(0);
  const step = Math.max(1, Math.floor(d.length / n));
  const out: number[] = [];
  for (let i = 0; i < n; i++) {
    let m = 0;
    for (let j = i * step; j < Math.min(d.length, (i + 1) * step); j += 4) {
      const v = Math.abs(d[j] as number);
      if (v > m) {
        m = v;
      }
    }
    out.push(m);
  }
  const top = Math.max(0.05, ...out);
  return out.map((v) => Math.max(0.03, (v / top) ** 0.8));
}

/** The selected part as 16-bit mono WAV (the engine reads WAV everywhere). */
function wavOf(buf: AudioBuffer, start: number, len: number): Uint8Array {
  const sr = buf.sampleRate;
  const a = Math.floor(start * sr);
  const b = Math.min(buf.length, Math.floor((start + len) * sr));
  const n = Math.max(0, b - a);
  const chans = Array.from({ length: buf.numberOfChannels }, (_, c) => buf.getChannelData(c));
  const out = new DataView(new ArrayBuffer(44 + n * 2));
  const w = (o: number, s: string) => {
    for (let i = 0; i < s.length; i++) {
      out.setUint8(o + i, s.charCodeAt(i));
    }
  };
  w(0, "RIFF");
  out.setUint32(4, 36 + n * 2, true);
  w(8, "WAVE");
  w(12, "fmt ");
  out.setUint32(16, 16, true);
  out.setUint16(20, 1, true);
  out.setUint16(22, 1, true);
  out.setUint32(24, sr, true);
  out.setUint32(28, sr * 2, true);
  out.setUint16(32, 2, true);
  out.setUint16(34, 16, true);
  w(36, "data");
  out.setUint32(40, n * 2, true);
  for (let i = 0; i < n; i++) {
    let s = 0;
    for (const ch of chans) {
      s += ch[a + i] as number;
    }
    s = Math.max(-1, Math.min(1, s / chans.length));
    out.setInt16(44 + i * 2, s < 0 ? s * 0x8000 : s * 0x7fff, true);
  }
  return new Uint8Array(out.buffer);
}

function base64(bytes: Uint8Array): string {
  let s = "";
  for (let i = 0; i < bytes.length; i += 0x8000) {
    s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  }
  return btoa(s);
}

const mmss = (sec: number) => `${Math.floor(sec / 60)}:${String(Math.floor(sec % 60)).padStart(2, "0")}`;
const secs = (sec: number) => (sec >= 60 ? `${Math.floor(sec / 60)} min ${Math.round(sec % 60)} s` : `${Math.round(sec)} s`);

// ---------- pieces ----------

function Steps({ labels, step, max, onGo }: { labels: string[]; step: number; max: number; onGo: (i: number) => void }) {
  return (
    <ol aria-label="Steps" style={{ display: "flex", gap: 8, listStyle: "none", margin: 0, padding: 0, flexWrap: "wrap", alignItems: "center" }}>
      {labels.map((l, i) => {
        const cur = i === step;
        const done = i < step;
        return (
          <li key={l} style={{ display: "flex", alignItems: "center", gap: 8 }}>
            <button
              aria-current={cur ? "step" : undefined}
              disabled={i > max}
              onClick={() => onGo(i)}
              style={{
                all: "unset",
                cursor: i > max ? "default" : "pointer",
                display: "flex",
                alignItems: "center",
                gap: 8,
                padding: "6px 12px 6px 6px",
                borderRadius: 999,
                color: cur ? "#fff" : done ? "var(--b-text-2)" : "var(--b-fainter)",
                background: cur ? "rgba(255,255,255,0.09)" : "transparent",
                border: `1px solid ${cur ? "rgba(255,255,255,0.14)" : "transparent"}`,
              }}
              type="button"
            >
              <span
                style={{
                  width: 24,
                  height: 24,
                  borderRadius: 12,
                  display: "flex",
                  alignItems: "center",
                  justifyContent: "center",
                  fontSize: 12,
                  fontWeight: 600,
                  background: cur ? "var(--b-red-grad)" : done ? "rgba(255,255,255,0.14)" : "rgba(255,255,255,0.06)",
                }}
              >
                {i + 1}
              </span>
              {l}
            </button>
            {i < labels.length - 1 ? <span style={{ width: 18, height: 1, background: "rgba(255,255,255,0.14)" }} /> : null}
          </li>
        );
      })}
    </ol>
  );
}

function TakeCard({ t, n }: { t: Take; n: number }) {
  const src = fileUrl(t.path);
  return (
    <div className="glass" style={{ padding: 16, display: "flex", flexDirection: "column", gap: 12 }}>
      <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
        <PlayButton label={`test take ${n}`} src={src} variant="solid" />
        <span style={{ display: "flex", flexDirection: "column", lineHeight: 1.3, minWidth: 0 }}>
          <span style={{ fontWeight: 600 }}>Take {n}</span>
          <span
            className="mono faint"
            style={{ fontSize: 11, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}
            title={`seed ${t.seed ?? "?"}`}
          >
            {t.seconds.toFixed(1)} s · seed {t.seed ?? "?"}
          </span>
        </span>
      </div>
      <Wave bars={80} height={38} seed={t.seed ?? n} src={src} />
    </div>
  );
}

function Tile({ icon, title, sub, onClick }: { icon: React.ReactNode; title: string; sub: string; onClick: () => void }) {
  return (
    <button
      className="tile"
      onClick={onClick}
      style={{
        font: "inherit",
        color: "var(--b-text)",
        cursor: "pointer",
        padding: "26px 18px",
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        gap: 8,
        flex: "1 1 220px",
        borderStyle: "dashed",
      }}
      type="button"
    >
      <span
        style={{
          width: 48,
          height: 48,
          borderRadius: 24,
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          background: "rgba(240,32,42,0.10)",
          border: "1px solid rgba(240,32,42,0.3)",
        }}
      >
        {icon}
      </span>
      <span style={{ fontWeight: 600 }}>{title}</span>
      <span className="muted small">{sub}</span>
    </button>
  );
}

// ---------- the page ----------

export function CreateVoice() {
  const toast = useToast();
  const overview = useBooth((s) => s.overview);
  const catalog = useBooth((s) => s.catalog);
  const voices = useBooth((s) => s.voices);
  const voiceId = useBooth((s) => s.voiceId);
  const loadVoices = useBooth((s) => s.loadVoices);
  const pickVoice = useBooth((s) => s.pickVoice);
  const work = useBooth((s) => s.work);
  const play = usePlayer((s) => s.play);
  const hifiInstalled = packPresent(overview, "exact");

  const [src, setSrc] = useState<"clip" | "design">("clip");
  const [step, setStep] = useState(0);
  const [maxStep, setMaxStep] = useState(0);

  // clip
  const [clip, setClip] = useState<Clip | null>(null);
  const [start, setStart] = useState(0);
  const [len, setLen] = useState(30);
  const [recording, setRecording] = useState<{ rec: MediaRecorder; t0: number } | null>(null);
  const [recSecs, setRecSecs] = useState(0);
  const [hifi, setHifi] = useState(true);
  const [transcript, setTranscript] = useState("");
  const [whose, setWhose] = useState<"me" | "consented">("me");
  const [consent, setConsent] = useState(false);
  const fileRef = useRef<HTMLInputElement>(null);

  // design
  const [base, setBase] = useState<string>("");
  const [ctrl, setCtrl] = useState<Record<string, number>>({});
  const [description, setDescription] = useState("");

  // test and save
  const [testLine, setTestLine] = useState(TEST_LINE);
  const [tests, setTests] = useState<Take[]>([]);
  const [busy, setBusy] = useState<string | null>(null);
  const [made, setMade] = useState<{ voice: Voice; key: string } | null>(null);
  const [name, setName] = useState("");
  const [saved, setSaved] = useState<Voice | null>(null);

  // A test clone that was never saved is removed when you leave the page.
  const pending = useRef<string | null>(null);
  pending.current = made && !saved ? made.voice.id : null;
  useEffect(
    () => () => {
      if (pending.current) {
        api.deleteVoice(pending.current).then(() => loadVoices()).catch(() => undefined);
      }
    },
    [loadVoices]
  );

  useEffect(() => {
    if (!recording) {
      return;
    }
    const t = setInterval(() => setRecSecs((Date.now() - recording.t0) / 1000), 200);
    return () => clearInterval(t);
  }, [recording]);

  const shapeBases = voices.filter(canShape);
  const baseId = base || (shapeBases.find((v) => v.id === voiceId) ?? shapeBases.find((v) => v.name === "Rupert") ?? shapeBases[0])?.id || "";
  const baseVoice = voices.find((v) => v.id === baseId);

  const labels = src === "clip" ? ["Clip", "How to clone", "Test", "Save"] : ["Shape", "Test", "Save"];
  const testStep = src === "clip" ? 2 : 1;
  const saveStep = testStep + 1;

  const reset = (s: "clip" | "design") => {
    setSrc(s);
    setStep(0);
    setMaxStep(0);
    setTests([]);
    setSaved(null);
  };

  const useBuffer = async (fileName: string, data: ArrayBuffer) => {
    try {
      const buffer = await decode(data);
      setClip({ name: fileName, buffer, peaks: peaksOf(buffer, 240) });
      setStart(0);
      setLen(Math.min(buffer.duration, 45));
      setName((n) => n || fileName.replace(/\.[^.]+$/, "").replace(/[-_]+/g, " ").slice(0, 40));
      setTests([]);
    } catch (e) {
      toast.bad(`Couldn’t read that file as audio: ${errText(e)}`);
    }
  };

  const onFile = async (f: File | undefined) => {
    if (f) {
      await useBuffer(f.name, await f.arrayBuffer());
    }
  };

  const record = async () => {
    if (recording) {
      recording.rec.stop();
      return;
    }
    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: { echoCancellation: false, noiseSuppression: false } });
      const rec = new MediaRecorder(stream);
      const chunks: Blob[] = [];
      rec.ondataavailable = (e) => chunks.push(e.data);
      rec.onstop = async () => {
        for (const t of stream.getTracks()) {
          t.stop();
        }
        setRecording(null);
        const blob = new Blob(chunks, { type: rec.mimeType });
        await useBuffer("My recording", await blob.arrayBuffer());
      };
      rec.start();
      setRecSecs(0);
      setRecording({ rec, t0: Date.now() });
    } catch (e) {
      toast.bad(`Couldn’t use the microphone: ${errText(e)}`);
    }
  };

  const dur = clip?.buffer.duration ?? 0;
  const selLen = Math.min(len, dur);
  const selStart = Math.min(start, Math.max(0, dur - selLen));
  const clipKey = clip ? `${clip.name}|${dur}|${selStart.toFixed(2)}|${selLen.toFixed(2)}|${hifi}|${transcript}` : "";

  const previewSelection = () => {
    if (!clip) {
      return;
    }
    const url = URL.createObjectURL(new Blob([wavOf(clip.buffer, selStart, selLen) as BlobPart], { type: "audio/wav" }));
    play(url);
  };

  const makeTests = async () => {
    setBusy("Starting…");
    setTests([]);
    try {
      let voice: Voice;
      let mode: "exact" | "shapeable" = "shapeable";
      let controls: Record<string, number> = {};
      if (src === "clip") {
        if (!clip) {
          return;
        }
        if (made && made.key === clipKey) {
          voice = made.voice;
        } else {
          if (made) {
            await api.deleteVoice(made.voice.id).catch(() => undefined);
          }
          setBusy(hifi && hifiInstalled ? "Listening to your clip (about 20 s)…" : "Reading the voice from your clip…");
          voice = await work(() =>
            api.createClone({
              name: name.trim() || "New voice",
              audio_base64: base64(wavOf(clip.buffer, selStart, selLen)),
              ext: "wav",
              transcript: transcript.trim() || null,
              speaker: whose,
              exact: hifi && hifiInstalled,
            })
          );
          setMade({ voice, key: clipKey });
        }
        mode = canExact(voice) ? "exact" : "shapeable";
      } else {
        if (!baseVoice) {
          return;
        }
        voice = baseVoice;
        controls = ctrl;
      }
      const out: Take[] = [];
      for (let i = 0; i < 3; i++) {
        setBusy(`Take ${i + 1} of 3…`);
        const t = await work(() =>
          api.speak({ text: testLine.trim() || TEST_LINE, voice: voice.id, mode, controls, seed: 1 + Math.floor(Math.random() * 2e9) })
        );
        out.push(t);
        setTests([...out]);
      }
    } catch (e) {
      toast.bad(errText(e));
    } finally {
      setBusy(null);
    }
  };

  const save = async () => {
    setBusy("Saving…");
    try {
      let v: Voice;
      if (src === "clip") {
        if (!made) {
          return;
        }
        v = await api.renameVoice(made.voice.id, name.trim() || "My voice");
      } else {
        v = await work(() =>
          api.designVoice({ name: name.trim() || "New voice", base: baseId, controls: ctrl, description: description.trim() || null })
        );
      }
      setSaved(v);
      await loadVoices();
      toast.ok(`${v.name} saved`);
    } catch (e) {
      toast.bad(errText(e));
    } finally {
      setBusy(null);
    }
  };

  const go = (i: number) => {
    setStep(i);
    setMaxStep((m) => Math.max(m, i));
    if (i === testStep && tests.length === 0 && !busy) {
      makeTests();
    }
  };

  const blocked =
    (src === "clip" && step === 0 && (!clip || selLen < 3 || !!recording)) ||
    (src === "clip" && step === 1 && !consent) ||
    (step === testStep && (tests.length === 0 || !!busy)) ||
    (src === "design" && step === 0 && !baseVoice);
  const nextLabel = src === "clip" && step === 1 ? "Make 3 test takes" : src === "design" && step === 0 ? "Make 3 test takes" : "Next";
  const hint =
    src === "clip" && step === 1 && !consent
      ? "Tick the permission box to continue."
      : src === "clip" && step === 0 && clip && selLen < 10
        ? "Under 10 s: it works, but 20–60 s sounds more like them."
        : "";

  const shapeControls = (catalog?.controls ?? []).filter((c) => ["who", "texture", "room"].includes(c.group));
  const accent = Object.keys(ctrl).find((k) => k.startsWith("accent:"))?.slice(7) ?? "";
  const accentInfo = catalog?.accents.find((a) => a.id === accent);
  const setC = (k: string, v: number) => {
    const next = { ...ctrl };
    if (v === 0) {
      delete next[k];
    } else {
      next[k] = round2(v);
    }
    setCtrl(next);
  };

  return (
    <>
      <div className="glass" style={{ padding: "20px 22px", display: "flex", flexDirection: "column", gap: 16 }}>
        <div style={{ display: "flex", alignItems: "center", gap: 12, flexWrap: "wrap" }}>
          <h1 className="h1">Create a voice</h1>
          <div style={{ flex: 1 }} />
          <div aria-label="How to make the voice" className="seg" role="group">
            <button aria-pressed={src === "clip"} disabled={!!busy} onClick={() => reset("clip")} type="button">
              From a recording
            </button>
            <button aria-pressed={src === "design"} disabled={!!busy} onClick={() => reset("design")} type="button">
              Design from a preset
            </button>
          </div>
        </div>
        <Steps labels={labels} max={busy ? step : maxStep} onGo={go} step={step} />
      </div>

      {/* Step: clip */}
      {src === "clip" && step === 0 ? (
        <div style={{ display: "flex", gap: 16, flexWrap: "wrap", alignItems: "flex-start" }}>
          <div className="glass" style={{ flex: "999 1 480px", padding: 22, display: "flex", flexDirection: "column", gap: 16, minWidth: 0 }}>
            <div style={{ display: "flex", gap: 14, flexWrap: "wrap" }}>
              <Tile icon={Icon.upload} onClick={() => fileRef.current?.click()} sub="WAV, MP3, M4A, MP4 · trim it next" title="Choose a file" />
              <Tile
                icon={recording ? <span className="dot-live busy" style={{ width: 14, height: 14, borderRadius: 7 }} /> : Icon.micRed}
                onClick={record}
                sub={recording ? `Recording ${mmss(recSecs)} · click to stop` : "Read a short passage out loud"}
                title={recording ? "Stop recording" : "Record now"}
              />
              <input
                accept="audio/*,video/mp4,.m4a,.mp4,.wav,.mp3,.ogg,.flac,.webm"
                hidden={true}
                onChange={(e) => onFile(e.target.files?.[0])}
                ref={fileRef}
                type="file"
              />
            </div>
            {recording ? (
              <div className="tile" style={{ padding: 18, fontSize: 16, lineHeight: 1.7, color: "var(--b-text)" }}>
                <div className="muted small" style={{ marginBottom: 6 }}>
                  Read this in your normal voice. About 30 seconds is plenty.
                </div>
                {PASSAGE}
              </div>
            ) : null}
            {clip ? (
              <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
                <div style={{ display: "flex", justifyContent: "space-between", gap: 10, flexWrap: "wrap", fontSize: 13 }}>
                  <span style={{ fontWeight: 500 }}>
                    {clip.name} <span className="faint">· {secs(dur)}</span>
                  </span>
                  <span className="muted">selected {secs(selLen)} · best 20–60 s</span>
                </div>
                <div className="tile" style={{ padding: "14px 14px 8px", position: "relative" }}>
                  <div aria-hidden="true" style={{ position: "relative", height: 64, display: "flex", alignItems: "center", justifyContent: "space-between" }}>
                    {clip.peaks.map((v, i) => {
                      const x = (i / (clip.peaks.length - 1)) * dur;
                      const inSel = x >= selStart && x <= selStart + selLen;
                      return (
                        <i
                          // biome-ignore lint/suspicious/noArrayIndexKey: fixed-length bar list
                          key={i}
                          style={{
                            flex: "none",
                            width: 1.5,
                            borderRadius: 1,
                            height: Math.max(2, Math.round(v * 60)),
                            background: inSel ? "#fff" : "rgba(255,255,255,0.2)",
                          }}
                        />
                      );
                    })}
                    <span
                      style={{
                        position: "absolute",
                        top: -6,
                        bottom: -6,
                        left: `${(selStart / dur) * 100}%`,
                        width: `${(selLen / dur) * 100}%`,
                        border: "1.5px solid var(--b-red)",
                        borderRadius: 8,
                        background: "rgba(240,32,42,0.07)",
                        pointerEvents: "none",
                      }}
                    />
                  </div>
                  <div className="mono faint" style={{ display: "flex", justifyContent: "space-between", fontSize: 11, marginTop: 6 }}>
                    <span>0:00</span>
                    <span>{mmss(dur / 2)}</span>
                    <span>{mmss(dur)}</span>
                  </div>
                </div>
                <div style={{ display: "flex", gap: 16, flexWrap: "wrap", alignItems: "center" }}>
                  <PlayButton label="the selected part" onNeedSrc={async () => (previewSelection(), null)} src={null} />
                  <div style={{ flex: "1 1 180px" }}>
                    <DoseSlider
                      disabled={dur <= selLen}
                      label="Start"
                      max={Math.max(0.1, dur - selLen)}
                      min={0}
                      onChange={setStart}
                      poles={mmss(selStart)}
                      step={0.1}
                      value={selStart}
                    />
                  </div>
                  <div style={{ flex: "1 1 180px" }}>
                    <DoseSlider
                      label="Clip length"
                      max={Math.max(3, Math.min(60, dur))}
                      min={Math.min(3, dur)}
                      onChange={setLen}
                      poles={secs(selLen)}
                      step={0.5}
                      value={selLen}
                    />
                  </div>
                </div>
              </div>
            ) : null}
          </div>
          <aside className="glass muted" style={{ flex: "1 1 260px", maxWidth: 360, padding: 20, lineHeight: 1.6, fontSize: 13 }}>
            <div style={{ color: "var(--b-text)", fontWeight: 600, fontSize: 15, marginBottom: 6 }}>Tips for a good clone</div>
            One person talking, no music, no other voices. A phone voice note is fine. Up to 60 s helps High-fidelity; Quick is happy with
            10 s. The clone copies the room too, so an echoey hall stays echoey.
          </aside>
        </div>
      ) : null}

      {/* Step: how to clone */}
      {src === "clip" && step === 1 ? (
        <div style={{ display: "flex", flexDirection: "column", gap: 14 }}>
          {[
            {
              id: true,
              name: "High-fidelity",
              meta: "Exact + Shapeable · about 20 s",
              off: !hifiInstalled,
              desc: "The model keeps your recording and listens to it every time it speaks. Closest to the real person, especially on clean audio. Also saves the fingerprint, so you can still shape it.",
              note: hifiInstalled
                ? "Some takes can jump in pitch: the Test step makes three so you can hear it."
                : "Needs the High-fidelity pack (5.6 GB). Download it in Settings.",
            },
            {
              id: false,
              name: "Quick",
              meta: "Shapeable only · about 1 s",
              off: false,
              desc: "Saves a fingerprint of the voice. Works from 10 seconds. A bit less like them, but every control works and it needs no extra download.",
              note: "Good for noisy clips: on rough audio it is about as close as High-fidelity.",
            },
          ].map((m) => {
            const on = (hifi && hifiInstalled) === m.id;
            return (
              <label
                className={`glass${on ? " lit" : ""}`}
                key={m.name}
                style={{ display: "flex", gap: 14, padding: 18, cursor: m.off ? "not-allowed" : "pointer", opacity: m.off ? 0.55 : 1 }}
              >
                <input
                  checked={on}
                  className="check"
                  disabled={m.off}
                  name="mode"
                  onChange={() => setHifi(m.id)}
                  type="radio"
                />
                <span style={{ display: "flex", flexDirection: "column", gap: 4 }}>
                  <span style={{ fontWeight: 600, fontSize: 15 }}>
                    {m.name} <span className="faint" style={{ fontWeight: 400 }}>· {m.meta}</span>
                  </span>
                  <span style={{ color: "var(--b-text-2)", lineHeight: 1.5 }}>{m.desc}</span>
                  <span className="faint small">{m.note}</span>
                </span>
              </label>
            );
          })}
          <div className="glass" style={{ padding: 20, display: "flex", flexDirection: "column", gap: 14 }}>
            <label className="lbl">
              What the clip says, word for word (optional, makes High-fidelity cloning better)
              <textarea className="field" onChange={(e) => setTranscript(e.target.value)} rows={2} value={transcript} />
            </label>
            <div style={{ display: "flex", gap: 10, alignItems: "center", flexWrap: "wrap", fontSize: 13 }}>
              <span className="muted">Whose voice is it?</span>
              <div aria-label="Whose voice" className="seg" role="group">
                <button aria-pressed={whose === "me"} onClick={() => setWhose("me")} type="button">
                  Mine
                </button>
                <button aria-pressed={whose === "consented"} onClick={() => setWhose("consented")} type="button">
                  Someone who agreed
                </button>
              </div>
            </div>
            <label style={{ display: "flex", gap: 10, fontSize: 13, lineHeight: 1.5, cursor: "pointer" }}>
              <input checked={consent} className="check" onChange={() => setConsent(!consent)} type="checkbox" />
              <span>
                {whose === "me" ? "This is my voice." : "I have the speaker’s permission to clone their voice."} I won’t use it to deceive
                people or pretend to be someone else.
              </span>
            </label>
          </div>
        </div>
      ) : null}

      {/* Step: shape (design) */}
      {src === "design" && step === 0 ? (
        <div className="glass" style={{ padding: 22, display: "flex", flexDirection: "column", gap: 14 }}>
          <div style={{ display: "flex", alignItems: "center", gap: 14, flexWrap: "wrap" }}>
            {baseVoice ? <Orb id={baseVoice.id} size={44} /> : null}
            <label className="lbl" style={{ flex: "1 1 240px" }}>
              Start from
              <Dropdown
                onChange={setBase}
                options={shapeBases.map((v) => ({
                  value: v.id,
                  label: v.name,
                  hint: v.kind === "default" ? undefined : "yours",
                }))}
                value={baseId}
              />
            </label>
          </div>
          <p className="muted" style={{ margin: 0, fontSize: 13 }}>
            No recording needed. Saves a Shapeable voice with these settings baked in.
          </p>
          {catalog ? (
            <div className="chips" style={{ gap: 6 }}>
              {Object.entries(catalog.presets).map(([label, map]) => (
                <button className="chip" key={label} onClick={() => setCtrl({ ...map })} type="button">
                  {label}
                </button>
              ))}
              <button className="chip" onClick={() => setCtrl({})} type="button">
                Clear
              </button>
            </div>
          ) : null}
          <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fill, minmax(220px, 1fr))", gap: "4px 22px" }}>
            {shapeControls.map((c) => {
              const [lo, hi] = c.safe ?? c.range ?? [c.type === "unipolar" ? 0 : -2, 2];
              return (
                <DoseSlider
                  key={c.id}
                  label={c.label}
                  max={hi}
                  min={c.type === "unipolar" ? 0 : lo}
                  onChange={(v) => setC(c.id, v)}
                  poles={c.poles ? c.poles.join(" · ") : String(ctrl[c.id] ?? 0)}
                  step={(hi - lo) / 100}
                  title={c.effect ?? undefined}
                  value={ctrl[c.id] ?? 0}
                />
              );
            })}
          </div>
          <div style={{ display: "flex", gap: 16, flexWrap: "wrap", alignItems: "flex-end" }}>
            <label className="lbl" style={{ flex: "1 1 220px" }}>
              Accent
              <Dropdown
                onChange={(v) => {
                  const next = Object.fromEntries(Object.entries(ctrl).filter(([k]) => !k.startsWith("accent:")));
                  if (v) {
                    const a = catalog?.accents.find((x) => x.id === v);
                    next[`accent:${v}`] = round2((a?.safe?.[1] ?? 1.5) * 0.7);
                  }
                  setCtrl(next);
                }}
                options={[
                  { value: "", label: "Keep the voice’s own" },
                  ...(catalog?.accents ?? []).map((a) => ({ value: a.id, label: a.label })),
                ]}
                value={accent}
              />
            </label>
            {accent ? (
              <div style={{ flex: "1 1 220px" }}>
                <DoseSlider
                  label="Accent strength"
                  max={accentInfo?.safe?.[1] ?? 1.5}
                  min={0}
                  onChange={(v) => setC(`accent:${accent}`, v)}
                  step={0.05}
                  value={ctrl[`accent:${accent}`] ?? 0}
                />
              </div>
            ) : null}
          </div>
          <label className="lbl">
            Description (optional, shown on the voice card)
            <input className="field" onChange={(e) => setDescription(e.target.value)} placeholder="e.g. Rupert, deeper and older" value={description} />
          </label>
        </div>
      ) : null}

      {/* Step: test */}
      {step === testStep ? (
        <div className="glass" style={{ padding: 22, display: "flex", flexDirection: "column", gap: 16 }}>
          <p style={{ margin: 0, color: "var(--b-text-2)", lineHeight: 1.6 }}>
            {src === "clip"
              ? "Three test takes of the same line. If none sound like them, go back and pick a cleaner part of the recording."
              : "Three test takes of the same line. Not quite right? Go back and move the sliders."}
          </p>
          <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(220px, 1fr))", gap: 14 }}>
            {tests.map((t, i) => (
              <TakeCard key={t.id} n={i + 1} t={t} />
            ))}
          </div>
          {busy ? (
            <div className="muted" style={{ display: "flex", gap: 10, alignItems: "center" }}>
              <span className="spinner" /> {busy}
            </div>
          ) : null}
          <div style={{ display: "flex", gap: 10, alignItems: "flex-end", flexWrap: "wrap" }}>
            <label className="lbl" style={{ flex: "1 1 320px" }}>
              Test line
              <input className="field" onChange={(e) => setTestLine(e.target.value)} value={testLine} />
            </label>
            <button className="btn ghost" disabled={!!busy} onClick={makeTests} type="button">
              Make 3 more
            </button>
          </div>
        </div>
      ) : null}

      {/* Step: save */}
      {step === saveStep ? (
        <div className="glass" style={{ padding: 22, display: "flex", flexDirection: "column", gap: 16, maxWidth: 640 }}>
          <div style={{ display: "flex", gap: 14, alignItems: "flex-end" }}>
            <Orb id={saved?.id ?? made?.voice.id ?? name} size={52} />
            <label className="lbl" style={{ flex: 1 }}>
              Name
              <input className="field" disabled={!!saved} onChange={(e) => setName(e.target.value)} placeholder="My voice" value={name} />
            </label>
          </div>
          <div className="muted" style={{ fontSize: 13, lineHeight: 1.6 }}>
            {src === "clip" ? (
              <>
                Saves as{" "}
                {made && canExact(made.voice) ? (
                  <>
                    <span className="badge strong">Exact</span> and <span className="badge">Shapeable</span>: the trimmed clip, its codes and
                    its voice fingerprint.
                  </>
                ) : (
                  <>
                    <span className="badge">Shapeable</span>: the trimmed clip and its voice fingerprint.
                  </>
                )}{" "}
                Kept on this computer only.
              </>
            ) : (
              <>
                Saves as <span className="badge">Shapeable</span>: {baseVoice?.name ?? "the base voice"} with your settings baked in. Kept on
                this computer only.
              </>
            )}
          </div>
          {saved ? (
            <div style={{ display: "flex", gap: 10, flexWrap: "wrap", alignItems: "center" }}>
              <span className="badge ok">Saved</span>
              <button className="btn primary" onClick={() => pickVoice(saved.id)} type="button">
                Use {saved.name} in the Studio
              </button>
              <button
                className="btn ghost"
                onClick={() => {
                  setMade(null);
                  setClip(null);
                  setName("");
                  setConsent(false);
                  setTranscript("");
                  setCtrl({});
                  reset(src);
                }}
                type="button"
              >
                Make another
              </button>
            </div>
          ) : null}
        </div>
      ) : null}

      <div style={{ display: "flex", gap: 10, alignItems: "center", flexWrap: "wrap" }}>
        {step > 0 && !saved ? (
          <button className="btn ghost" disabled={!!busy} onClick={() => setStep(step - 1)} type="button">
            Back
          </button>
        ) : null}
        {step < saveStep ? (
          <button className="btn primary" disabled={blocked} onClick={() => go(step + 1)} type="button">
            {nextLabel}
          </button>
        ) : null}
        {step === saveStep && !saved ? (
          <button className="btn primary" disabled={!!busy} onClick={save} type="button">
            Save voice
          </button>
        ) : null}
        <span className="faint" style={{ fontSize: 13 }}>
          {hint}
        </span>
      </div>
    </>
  );
}
