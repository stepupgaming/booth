"use client";

import { useEffect, useState } from "react";
import {
  accelWhere,
  api,
  type DownloadEvent,
  errText,
  fileUrl,
  gb,
  LANGUAGE_GROUPS,
  type PackInfo,
  type Pronunciation,
  type Settings as S,
} from "@workspace/core/booth/api";
import { useBooth } from "@workspace/core/booth/store";
import { Dropdown, Icon, Switch, usePlayer, useToast } from "@workspace/core/booth/ui";

const TOOLS: [string, string][] = [
  ["status", "Is the engine up, which packs are installed"],
  ["list_voices", "Presets and your own voices"],
  ["list_controls", "Moods, delivery, shape and accent controls"],
  ["speak", "One line in one voice, saved as a take"],
  ["render_script", "A multi-voice script as one file"],
  ["list_takes", "Recent takes with their settings"],
  ["design_voice", "Save a shaped voice from a preset"],
  ["set_pronunciation", "Teach Booth how to say a word"],
  ["create_voice", "Clone from a clip (only if you allow it below)"],
];

function Section({ title, children, sub }: { title: string; sub?: string; children: React.ReactNode }) {
  return (
    <section className="glass" style={{ padding: 22, display: "flex", flexDirection: "column", gap: 14 }}>
      <div>
        <h2 className="h2">{title}</h2>
        {sub ? (
          <p className="muted" style={{ margin: "4px 0 0", fontSize: 13 }}>
            {sub}
          </p>
        ) : null}
      </div>
      {children}
    </section>
  );
}

function Row({ title, sub, children }: { title: string; sub?: string; children: React.ReactNode }) {
  return (
    <div style={{ display: "flex", gap: 14, alignItems: "center", flexWrap: "wrap" }}>
      <span style={{ display: "flex", flexDirection: "column", flex: "1 1 260px", lineHeight: 1.45 }}>
        <span style={{ fontWeight: 500 }}>{title}</span>
        {sub ? <span className="muted small">{sub}</span> : null}
      </span>
      {children}
    </div>
  );
}

function Mcp() {
  const toast = useToast();
  const claude = useBooth((s) => s.claude);
  const loadClaude = useBooth((s) => s.loadClaude);
  const [tab, setTab] = useState<"desktop" | "code" | "cursor">("desktop");
  const exe = claude?.exe ?? "booth-mcp.exe";
  const snippet =
    tab === "code" ? `claude mcp add booth -- "${exe}"` : JSON.stringify(claude?.config_snippet ?? {}, null, 2);

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(snippet);
      toast.ok("Copied");
    } catch (e) {
      toast.bad(errText(e));
    }
  };

  return (
    <>
      <div aria-label="Client" className="seg" role="tablist">
        {(
          [
            ["desktop", "Claude Desktop"],
            ["code", "Claude Code"],
            ["cursor", "Cursor and others"],
          ] as const
        ).map(([id, label]) => (
          <button aria-selected={tab === id} key={id} onClick={() => setTab(id)} role="tab" type="button">
            {label}
          </button>
        ))}
      </div>
      {tab === "desktop" ? (
        <Row
          sub={
            claude?.connected
              ? "Booth is set up in Claude Desktop. Restart Claude if its tools don’t show."
              : claude?.desktop_found
                ? "Opens the Booth extension in Claude Desktop, which asks you to confirm."
                : "Claude Desktop wasn’t found on this computer. Install it first, or use the snippet."
          }
          title={claude?.connected ? "Connected" : "One-click install"}
        >
          {claude?.connected ? <span className="badge ok">Connected</span> : null}
          <button
            className="btn primary"
            disabled={!claude?.mcpb}
            onClick={async () => {
              try {
                await api.installMcpb();
                toast.ok("Confirm the install in Claude Desktop");
                setTimeout(loadClaude, 8000);
              } catch (e) {
                toast.bad(errText(e));
              }
            }}
            title={claude?.mcpb ? undefined : "booth.mcpb isn’t bundled with this build"}
            type="button"
          >
            {claude?.connected ? "Install again" : "Add to Claude Desktop"}
          </button>
        </Row>
      ) : null}
      <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
        <span className="muted small">
          {tab === "code"
            ? "Run this once in a terminal:"
            : tab === "cursor"
              ? "Add this to the client’s MCP config (Cursor: .cursor/mcp.json):"
              : "Or add it by hand to claude_desktop_config.json:"}
        </span>
        <div style={{ position: "relative" }}>
          <pre className="code" style={{ margin: 0, whiteSpace: "pre-wrap", wordBreak: "break-all" }}>
            {snippet}
          </pre>
          <button className="btn ghost sm" onClick={copy} style={{ position: "absolute", top: 8, right: 8 }} type="button">
            Copy
          </button>
        </div>
      </div>
      <details>
        <summary className="muted" style={{ cursor: "pointer", fontSize: 13 }}>
          What Claude can do with it ({TOOLS.length} tools)
        </summary>
        <ul style={{ margin: "10px 0 0", padding: 0, listStyle: "none", display: "grid", gap: 6 }}>
          {TOOLS.map(([n, d]) => (
            <li key={n} style={{ fontSize: 13 }}>
              <span className="mono">{n}</span> <span className="muted">· {d}</span>
            </li>
          ))}
        </ul>
      </details>
    </>
  );
}

function Models() {
  const toast = useToast();
  const overview = useBooth((s) => s.overview);
  const refresh = useBooth((s) => s.refresh);
  const [packs, setPacks] = useState<PackInfo[] | null>(null);
  const [dl, setDl] = useState<DownloadEvent | null>(null);
  const [downloading, setDownloading] = useState<string | null>(null);
  const [stopping, setStopping] = useState(false);
  const system = useBooth((s) => s.system);
  const engine = overview?.engine;
  const state = engine?.state;
  const where = accelWhere(engine?.state === "running" ? engine.accel : undefined);
  // The NVIDIA engine row only where it can help.
  const shown = (packs ?? []).filter((p) => p.pack !== "gpu_engine" || p.present || system?.gpu_kind === "nvidia");

  useEffect(() => {
    api.packInfo().then(setPacks).catch(() => setPacks([]));
  }, []);

  const download = async (p: PackInfo) => {
    setDownloading(p.pack);
    const un = await api.onDownload(setDl);
    try {
      await api.downloadPacks([p.pack]);
      toast.ok(`${p.label} downloaded`);
      setPacks(await api.packInfo());
      await refresh();
    } catch (e) {
      toast.bad(errText(e));
    } finally {
      un();
      setDl(null);
      setDownloading(null);
    }
  };

  const stop = async () => {
    setStopping(true);
    try {
      await api.engineStop();
      await refresh();
      toast.ok("Engine stopped. Its memory is free.");
    } catch (e) {
      toast.bad(errText(e));
    } finally {
      setStopping(false);
    }
  };

  return (
    <>
      <Row
        sub={
          state === "running"
            ? `Loaded on the ${where}. Stopping waits for any line in progress to finish.`
            : "Not running. It wakes up on your next line."
        }
        title={state === "running" ? "Engine running" : "Engine asleep"}
      >
        {state === "running" ? (
          <button className="btn ghost" disabled={stopping} onClick={stop} type="button">
            {stopping ? "Stopping…" : "Stop the engine"}
          </button>
        ) : null}
      </Row>
      <div className="divider" />
      {shown.map((p) => {
        const mine = downloading === p.pack && dl?.pack === p.pack;
        const pct = mine && dl && dl.total_bytes ? Math.round((dl.done_bytes / dl.total_bytes) * 100) : 0;
        return (
          <Row
            key={p.pack}
            sub={p.pack === "gpu_engine" ? `${gb(p.bytes)} · runs Booth on your NVIDIA card` : `${gb(p.bytes)} · ${p.files.length} files`}
            title={p.label}
          >
            {p.present ? (
              <span className="badge ok">Installed</span>
            ) : downloading === p.pack ? (
              <span className="muted small" style={{ display: "flex", alignItems: "center", gap: 8 }}>
                <span className="spinner" /> {pct}% · {dl ? `${(dl.rate / 1e6).toFixed(1)} MB/s` : "starting"}
              </span>
            ) : (
              <button className="btn ghost sm" disabled={!!downloading} onClick={() => download(p)} type="button">
                Download
              </button>
            )}
          </Row>
        );
      })}
      {packs === null ? (
        <span className="muted small" style={{ display: "flex", gap: 8, alignItems: "center" }}>
          <span className="spinner" /> Checking the model folder…
        </span>
      ) : null}
      {overview ? (
        <Row sub={overview.models_dir} title="Model folder">
          <button className="btn ghost sm" onClick={() => api.reveal(overview.models_dir).catch((e) => toast.bad(errText(e)))} type="button">
            Show
          </button>
        </Row>
      ) : null}
    </>
  );
}

/** The list as saved: no blank rows, no stray spaces. */
const cleanList = (rows: Pronunciation[]): Pronunciation[] =>
  rows
    .filter((r) => r.write.trim())
    .map((r) => ({
      write: r.write.trim(),
      say: r.say.trim(),
      match_case: r.match_case,
      ...(r.language ? { language: r.language } : {}),
    }));

const ANY_LANGUAGE = "";

/** "Say it like this": words the voice reads differently, applied to everything Booth speaks. */
function Pronunciations({ settings, save }: { settings: S | undefined; save: (p: Partial<S>) => Promise<void> }) {
  const toast = useToast();
  const work = useBooth((s) => s.work);
  const voiceId = useBooth((s) => s.voiceId);
  const play = usePlayer((s) => s.play);
  // Rows being edited, blank ones included; saved without the blanks.
  const [rows, setRows] = useState<Pronunciation[] | null>(null);
  const [hearing, setHearing] = useState<number | null>(null);
  // A row just added, to put the cursor in.
  const [added, setAdded] = useState<number | null>(null);

  useEffect(() => {
    if (settings && rows === null) {
      setRows(settings.pronunciations ?? []);
    }
  }, [settings, rows]);

  const list = rows ?? [];
  const persist = async (next: Pronunciation[]) => {
    const clean = cleanList(next);
    if (JSON.stringify(clean) !== JSON.stringify(settings?.pronunciations ?? [])) {
      await save({ pronunciations: clean });
    }
  };
  const edit = (i: number, patch: Partial<Pronunciation>, now = false) => {
    const next = list.map((r, j) => (j === i ? { ...r, ...patch } : r));
    setRows(next);
    if (now) {
      persist(next);
    }
  };
  const remove = (i: number) => {
    const next = list.filter((_, j) => j !== i);
    setRows(next);
    persist(next);
  };
  const hear = async (i: number) => {
    const row = list[i];
    if (!row?.write.trim()) {
      return;
    }
    setHearing(i);
    try {
      await persist(list);
      const path = await work(() => api.hearPronunciation(row.write.trim(), voiceId));
      play(fileUrl(path));
    } catch (e) {
      toast.bad(errText(e));
    } finally {
      setHearing(null);
    }
  };

  return (
    <>
      {list.length === 0 ? (
        <span className="muted small">No words yet. Add one when a name or an acronym comes out wrong.</span>
      ) : (
        <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
          {list.map((r, i) => (
            <div
              // biome-ignore lint/suspicious/noArrayIndexKey: rows have no id, and order only changes by removal
              key={i}
              style={{
                display: "flex",
                gap: 8,
                alignItems: "center",
                flexWrap: "wrap",
                ...(i > 0 ? { borderTop: "1px solid rgba(255, 255, 255, 0.08)", paddingTop: 12 } : {}),
              }}
            >
              <span
                style={{
                  display: "grid",
                  gridTemplateColumns: "minmax(0, 1fr) auto minmax(0, 1fr)",
                  gap: 8,
                  alignItems: "center",
                  flex: "1 1 280px",
                }}
              >
                <input
                  aria-label="Write"
                  className="field"
                  onBlur={() => persist(list)}
                  onChange={(e) => edit(i, { write: e.target.value })}
                  placeholder="Write, like GIF"
                  ref={(el) => {
                    if (el && i === added) {
                      el.focus();
                      setAdded(null);
                    }
                  }}
                  style={{ minWidth: 0 }}
                  value={r.write}
                />
                <span aria-hidden="true" className="faint">
                  →
                </span>
                <input
                  aria-label="Say"
                  className="field"
                  onBlur={() => persist(list)}
                  onChange={(e) => edit(i, { say: e.target.value })}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      hear(i);
                    }
                  }}
                  placeholder="Say, like jif"
                  style={{ minWidth: 0 }}
                  value={r.say}
                />
              </span>
              <span style={{ display: "flex", gap: 8, alignItems: "center", flexWrap: "wrap" }}>
                <label className="small" style={{ display: "flex", gap: 6, alignItems: "center", cursor: "pointer" }}>
                  <input
                    checked={r.match_case}
                    className="check"
                    onChange={() => edit(i, { match_case: !r.match_case }, true)}
                    style={{ margin: 0 }}
                    type="checkbox"
                  />
                  Match case
                </label>
                <Dropdown
                  groups={LANGUAGE_GROUPS}
                  label="Language"
                  onChange={(v) => edit(i, { language: v || null }, true)}
                  options={[{ value: ANY_LANGUAGE, label: "Any language" }]}
                  style={{ width: 140 }}
                  value={r.language ?? ANY_LANGUAGE}
                />
                <button
                  className="btn ghost sm"
                  disabled={!r.write.trim() || hearing !== null}
                  onClick={() => hear(i)}
                  title="Say it with the voice chosen in Studio"
                  type="button"
                >
                  {hearing === i ? <span className="spinner" /> : Icon.play}
                  Hear it
                </button>
                <button
                  aria-label={`Remove ${r.write || "this word"}`}
                  className="btn ghost sm icon-btn"
                  onClick={() => remove(i)}
                  type="button"
                >
                  {Icon.trash}
                </button>
              </span>
            </div>
          ))}
        </div>
      )}
      <div>
        <button
          className="btn ghost sm"
          disabled={!settings}
          onClick={() => {
            setRows([...list, { write: "", say: "", match_case: false }]);
            setAdded(list.length);
          }}
          type="button"
        >
          {Icon.plus}Add a word
        </button>
      </div>
    </>
  );
}

/** Graphics card (when Booth has one to use) or the processor. Applies on the engine's next start. */
function RunOn({ settings, save }: { settings: S | undefined; save: (p: Partial<S>) => Promise<void> }) {
  const toast = useToast();
  const system = useBooth((s) => s.system);
  const overview = useBooth((s) => s.overview);
  const refresh = useBooth((s) => s.refresh);
  const gpu = system?.gpu_kind === "apple" || (system?.gpu_kind === "nvidia" && system.platform === "windows");
  if (!gpu) {
    return null;
  }
  const change = async (device: string) => {
    await save({ device: device === "cpu" ? "cpu" : "auto" });
    if (overview?.engine.state === "running") {
      try {
        await api.engineRestart();
        await refresh();
        toast.ok(device === "cpu" ? "Now on the processor" : "Now on the graphics card");
      } catch (e) {
        toast.bad(errText(e));
      }
    }
  };
  return (
    <>
      <Row sub="The graphics card is faster. The processor keeps the card free for games or video work." title="Run on">
        <Dropdown
          label="Run on"
          onChange={change}
          options={[
            { value: "auto", label: "Graphics card" },
            { value: "cpu", label: "Processor" },
          ]}
          style={{ width: 170 }}
          value={settings?.device ?? "auto"}
        />
      </Row>
      <div className="divider" />
    </>
  );
}

function Updates() {
  const toast = useToast();
  const version = useBooth((s) => s.version);
  const update = useBooth((s) => s.update);
  const checkUpdate = useBooth((s) => s.checkUpdate);
  const [checking, setChecking] = useState(false);
  const check = async () => {
    setChecking(true);
    try {
      const u = await checkUpdate();
      if (!u) {
        toast.ok("Booth is up to date");
      }
    } catch (e) {
      toast.bad(errText(e));
    } finally {
      setChecking(false);
    }
  };
  return (
    <Row
      sub={update ? `Version ${update.version} is ready. Use the button at the bottom left to install it.` : "Booth checks GitHub for a new version when it starts."}
      title={version ? `Booth ${version}` : "Booth"}
    >
      <button className="btn ghost sm" disabled={checking} onClick={check} type="button">
        {checking ? "Checking…" : "Check for updates"}
      </button>
    </Row>
  );
}

/** Booth's log and the engine's: what to send when something got stuck or failed. */
function LogView() {
  const toast = useToast();
  const [text, setText] = useState<string | null>(null);
  const load = () =>
    api
      .readLog()
      .then(setText)
      .catch((e) => toast.bad(errText(e)));
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(text ?? (await api.readLog()));
      toast.ok("Log copied");
    } catch (e) {
      toast.bad(errText(e));
    }
  };
  return (
    <>
      <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
        <button className="btn ghost sm" onClick={() => (text === null ? load() : setText(null))} type="button">
          {text === null ? "Show log" : "Hide log"}
        </button>
        {text === null ? null : (
          <button className="btn ghost sm" onClick={load} type="button">
            Refresh
          </button>
        )}
        <button className="btn ghost sm" onClick={copy} type="button">
          Copy log
        </button>
      </div>
      {text === null ? null : (
        <pre
          className="mono"
          ref={(el) => {
            if (el) el.scrollTop = el.scrollHeight;
          }}
          style={{
            margin: 0,
            maxHeight: 360,
            overflow: "auto",
            fontSize: 11,
            lineHeight: 1.55,
            whiteSpace: "pre-wrap",
            wordBreak: "break-word",
            padding: 12,
            borderRadius: 12,
            background: "rgba(0,0,0,0.35)",
            color: "var(--b-muted)",
          }}
        >
          {text || "Nothing logged yet."}
        </pre>
      )}
    </>
  );
}

export function Settings() {
  const toast = useToast();
  const overview = useBooth((s) => s.overview);
  const refresh = useBooth((s) => s.refresh);
  const settings = overview?.settings;

  const save = async (patch: Partial<S>) => {
    if (!settings) {
      return;
    }
    try {
      await api.saveSettings({ ...settings, ...patch });
      await refresh();
    } catch (e) {
      toast.bad(errText(e));
    }
  };

  return (
    <>
      <div className="glass" style={{ padding: "20px 22px" }}>
        <h1 className="h1">Settings</h1>
      </div>

      <Section title="Output">
        {overview ? (
          <Row sub={overview.takes_dir} title="Takes folder">
            <button
              className="btn ghost sm"
              onClick={() => api.reveal(overview.takes_dir).catch((e) => toast.bad(errText(e)))}
              type="button"
            >
              Show
            </button>
          </Row>
        ) : null}
        <Row
          sub="Checks each take for skipped words and early endings and redoes it. A little slower."
          title="Guard against skips and early endings"
        >
          <Switch label="Guard against skips" on={!!settings?.laci} onChange={(v) => save({ laci: v })} />
        </Row>
        <Row sub="Faster first line next time, but the model keeps its memory." title="Keep the engine loaded when Booth closes">
          <Switch
            label="Keep the engine loaded"
            on={!!settings?.keep_engine_running}
            onChange={(v) => save({ keep_engine_running: v })}
          />
        </Row>
      </Section>

      <Section
        sub="Words the voice should say differently, like SQL as sequel. Your text and takes keep the words as you wrote them."
        title="Pronunciation"
      >
        <Pronunciations save={save} settings={settings} />
      </Section>

      <Section sub="Booth runs ZONOS3-Mini on this computer. Nothing leaves it." title="Models">
        <RunOn save={save} settings={settings} />
        <Models />
      </Section>

      <Section title="Updates">
        <Updates />
      </Section>

      <Section sub="Let Claude speak with your Booth voices. Takes land in the same folder." title="Use Booth from Claude">
        <Mcp />
        <label style={{ display: "flex", gap: 10, fontSize: 13, lineHeight: 1.5, cursor: "pointer" }}>
          <input
            checked={!!settings?.mcp_allow_cloning}
            className="check"
            onChange={() => save({ mcp_allow_cloning: !settings?.mcp_allow_cloning })}
            type="checkbox"
          />
          <span>Let Claude clone voices from audio files (off by default; clones still need your permission to use the voice).</span>
        </label>
      </Section>

      <Section sub="What Booth asked the voice engine to do and how it went. Copy it into a bug report if something gets stuck." title="Log">
        <LogView />
      </Section>
    </>
  );
}
