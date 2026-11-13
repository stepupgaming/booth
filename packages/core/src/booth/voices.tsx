"use client";

import { useState } from "react";
import { api, canExact, canShape, errText, fileUrl, type Voice, voiceDesc } from "@workspace/core/booth/api";
import { useBooth } from "@workspace/core/booth/store";
import { hash, Icon, Orb, PlayButton, useToast, Wave } from "@workspace/core/booth/ui";

function Tag({ label, strong }: { label: string; strong?: boolean }) {
  return <span className={`badge${strong ? " strong" : ""}`}>{label}</span>;
}

function VoiceCard({ v, current }: { v: Voice; current: boolean }) {
  const toast = useToast();
  const pickVoice = useBooth((s) => s.pickVoice);
  const loadVoices = useBooth((s) => s.loadVoices);
  const work = useBooth((s) => s.work);
  const [src, setSrc] = useState<string | null>(null);
  const [editing, setEditing] = useState(false);
  const [name, setName] = useState(v.name);
  const [confirmDel, setConfirmDel] = useState(false);
  const mine = v.kind !== "default";

  const save = async () => {
    try {
      await api.renameVoice(v.id, name.trim() || v.name);
      await loadVoices();
      setEditing(false);
    } catch (e) {
      toast.bad(errText(e));
    }
  };
  const del = async () => {
    if (!confirmDel) {
      setConfirmDel(true);
      return;
    }
    try {
      await api.deleteVoice(v.id);
      await loadVoices();
      toast.ok(`${v.name} moved to Booth’s bin`);
    } catch (e) {
      toast.bad(errText(e));
    }
  };

  return (
    <div className={`glass${current ? " lit" : ""}`} style={{ padding: 16, display: "flex", flexDirection: "column", gap: 12 }}>
      <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
        <Orb id={v.id} />
        <span style={{ display: "flex", flexDirection: "column", lineHeight: 1.35, minWidth: 0, flex: 1 }}>
          <span style={{ fontWeight: 600, fontSize: 15 }}>{v.name}</span>
          <span className="muted small">{voiceDesc(v)}</span>
        </span>
        <PlayButton
          label={`a preview of ${v.name}`}
          onNeedSrc={async () => {
            const p = await work(() => api.voicePreview(v.id));
            const u = fileUrl(p);
            setSrc(u);
            return u;
          }}
          src={src}
        />
      </div>
      <Wave bars={48} height={22} seed={(hash(v.id) % 97) + 11} src={src} />
      <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
        {canExact(v) ? <Tag label="Exact" strong={true} /> : null}
        {canShape(v) ? <Tag label="Shapeable" /> : null}
        {v.kind === "designed" ? <Tag label="designed" /> : null}
        {v.kind === "clone" && !canExact(v) ? <Tag label="Quick clone" /> : null}
      </div>
      {editing ? (
        <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
          <input
            aria-label="Voice name"
            className="field"
            onChange={(e) => setName(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && save()}
            value={name}
          />
          <div style={{ display: "flex", gap: 8 }}>
            <button className="btn ghost sm" onClick={save} style={{ flex: 1 }} type="button">
              Save name
            </button>
            <button className="btn ghost sm" onClick={del} style={confirmDel ? { borderColor: "rgba(240,32,42,0.6)" } : undefined} type="button">
              {confirmDel ? "Really delete?" : "Delete"}
            </button>
            <button
              className="btn ghost sm"
              onClick={() => {
                setEditing(false);
                setConfirmDel(false);
                setName(v.name);
              }}
              type="button"
            >
              Cancel
            </button>
          </div>
        </div>
      ) : (
        <div style={{ display: "flex", gap: 8 }}>
          <button className="btn ghost sm" onClick={() => pickVoice(v.id)} style={{ flex: 1, fontSize: 13 }} type="button">
            Use in Studio
          </button>
          {mine ? (
            <button className="btn ghost sm" onClick={() => setEditing(true)} style={{ fontSize: 13 }} type="button">
              Edit
            </button>
          ) : null}
        </div>
      )}
    </div>
  );
}

export function Voices() {
  const voices = useBooth((s) => s.voices);
  const voiceId = useBooth((s) => s.voiceId);
  const go = useBooth((s) => s.go);
  const [tab, setTab] = useState<"presets" | "mine">("presets");
  const [lang, setLang] = useState("All");
  const [q, setQ] = useState("");

  const presets = voices.filter((v) => v.kind === "default");
  const mine = voices.filter((v) => v.kind !== "default");
  const counts = new Map<string, number>();
  for (const v of presets) {
    if (v.language) {
      counts.set(v.language, (counts.get(v.language) ?? 0) + 1);
    }
  }
  const langs = ["All", ...[...counts.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])).map(([l]) => l)];
  const needle = q.trim().toLowerCase();
  const list = (tab === "mine" ? mine : presets.filter((v) => lang === "All" || v.language === lang)).filter(
    (v) => !needle || `${v.name} ${voiceDesc(v)}`.toLowerCase().includes(needle)
  );

  return (
    <>
      <div className="glass" style={{ padding: "20px 22px", display: "flex", flexDirection: "column", gap: 16 }}>
        <div style={{ display: "flex", alignItems: "center", gap: 12, flexWrap: "wrap" }}>
          <h1 className="h1">Voices</h1>
          <div style={{ flex: 1 }} />
          <button className="btn primary" onClick={() => go("create")} type="button">
            {Icon.plus}Create a voice
          </button>
        </div>
        <div style={{ display: "flex", gap: 10, flexWrap: "wrap", alignItems: "center" }}>
          <div aria-label="Voice lists" className="seg" role="tablist">
            <button aria-selected={tab === "presets"} onClick={() => setTab("presets")} role="tab" type="button">
              Presets · {presets.length}
            </button>
            <button aria-selected={tab === "mine"} onClick={() => setTab("mine")} role="tab" type="button">
              My voices · {mine.length}
            </button>
          </div>
          <label
            style={{
              display: "flex",
              alignItems: "center",
              gap: 8,
              padding: "0 14px",
              height: 46,
              borderRadius: 14,
              background: "rgba(0,0,0,0.25)",
              border: "1px solid rgba(255,255,255,0.10)",
              flex: "1 1 220px",
              maxWidth: 320,
            }}
          >
            {Icon.search}
            <input
              aria-label="Search voices"
              onChange={(e) => setQ(e.target.value)}
              placeholder="Search voices"
              style={{ background: "none", border: "none", outline: "none", color: "var(--b-text)", font: "inherit", width: "100%" }}
              value={q}
            />
          </label>
          {tab === "presets" && langs.length > 2 ? (
            <div className="chips" style={{ gap: 6 }}>
              {langs.map((l) => (
                <button aria-pressed={lang === l} className="chip" key={l} onClick={() => setLang(l)} type="button">
                  {l}
                </button>
              ))}
            </div>
          ) : null}
        </div>
      </div>

      {list.length ? (
        <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fill, minmax(230px, 1fr))", gap: 14 }}>
          {list.map((v) => (
            <VoiceCard current={v.id === voiceId} key={v.id} v={v} />
          ))}
        </div>
      ) : (
        <div className="glass muted" style={{ padding: 22, lineHeight: 1.6 }}>
          {tab === "mine" && !needle ? (
            <>
              <div style={{ color: "var(--b-text)", fontWeight: 600, fontSize: 15, marginBottom: 4 }}>No voices of your own yet</div>
              Clone one from a short recording, or shape a new one from a preset.{" "}
              <button className="link" onClick={() => go("create")} type="button">
                Create a voice
              </button>
            </>
          ) : voices.length === 0 ? (
            <span style={{ display: "flex", gap: 8, alignItems: "center" }}>
              <span className="spinner" /> Loading voices…
            </span>
          ) : (
            "No voice matches that search."
          )}
        </div>
      )}

      <p className="faint" style={{ margin: 0, fontSize: 13, padding: "0 6px", lineHeight: 1.6 }}>
        Presets ship with the model and work as Exact or Shapeable. Your own voices are Exact when the High-fidelity pack was installed when
        you made them; otherwise they are Shapeable.
      </p>
    </>
  );
}
