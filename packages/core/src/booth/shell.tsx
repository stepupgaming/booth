"use client";

import { type ReactNode, useState } from "react";
import { accelWhere, api, errText } from "@workspace/core/booth/api";
import { useBooth, type View } from "@workspace/core/booth/store";
import { Backdrop, Brand, Icon, useToast } from "@workspace/core/booth/ui";

/** "Booth 0.2.1 is ready", with a button that installs it and restarts. */
function UpdateCard() {
  const update = useBooth((s) => s.update);
  const toast = useToast();
  const [pct, setPct] = useState<number | null>(null);
  if (!update) {
    return null;
  }
  const install = async () => {
    setPct(0);
    const un = await api.onUpdateProgress((p) => setPct(p.total_bytes ? Math.round((p.done_bytes / p.total_bytes) * 100) : null));
    try {
      await api.installUpdate();
    } catch (e) {
      toast.bad(errText(e));
      setPct(null);
    } finally {
      un();
    }
  };
  return (
    <div
      style={{
        marginTop: "auto",
        padding: 14,
        borderRadius: 16,
        background: "rgba(255,255,255,0.05)",
        border: "1px solid rgba(255,255,255,0.08)",
        fontSize: 12,
        color: "var(--b-muted)",
        lineHeight: 1.5,
        display: "flex",
        flexDirection: "column",
        gap: 8,
      }}
    >
      <div style={{ color: "var(--b-text)", fontWeight: 500 }}>Booth {update.version} is ready</div>
      <div>{pct == null ? "Installs in a few seconds and reopens Booth." : `Downloading… ${pct}%`}</div>
      <button className="btn ghost sm" disabled={pct != null} onClick={install} type="button">
        {pct == null ? "Update and restart" : "Updating…"}
      </button>
    </div>
  );
}

const NAV: { view: View; label: string; icon: ReactNode }[] = [
  { view: "studio", label: "Studio", icon: Icon.studio },
  { view: "voices", label: "Voices", icon: Icon.voices },
  { view: "create", label: "Create a voice", icon: Icon.mic },
  { view: "settings", label: "Settings", icon: Icon.settings },
];

function EngineCard() {
  const overview = useBooth((s) => s.overview);
  const system = useBooth((s) => s.system);
  const claude = useBooth((s) => s.claude);
  const busy = useBooth((s) => s.busy);
  const state = overview?.engine.state;
  const hasGpuEngine = overview?.packs.some((p) => p.pack === "gpu_engine" && p.present) ?? false;
  const running = state === "running";
  let title = "Engine asleep";
  let dot = "dot-live off";
  if (busy > 0) {
    title = running ? "Speaking" : "Starting the engine";
    dot = "dot-live busy";
  } else if (running) {
    title = "Engine ready";
    dot = "dot-live";
  } else if (state === "no_engine" || state === "no_models") {
    title = state === "no_engine" ? "Engine missing" : "Voices not downloaded";
  }
  const engine = overview?.engine;
  const accel = engine?.state === "running" ? engine.accel : undefined;
  // The chip it runs on: the graphics card when it uses one, else the processor.
  const onGpu = accel ? accel !== "cpu" : system?.gpu_kind === "apple" || (system?.gpu_kind === "nvidia" && hasGpuEngine);
  const chip = onGpu ? system?.gpu?.replace(/^NVIDIA (GeForce )?/, "") : system?.cpu?.replace(/\s+\d+-Core Processor$/, "");
  const update = useBooth((s) => s.update);
  return (
    <div
      style={{
        marginTop: update ? 0 : "auto",
        padding: 14,
        borderRadius: 16,
        background: "rgba(255,255,255,0.05)",
        border: "1px solid rgba(255,255,255,0.08)",
        fontSize: 12,
        color: "var(--b-muted)",
        lineHeight: 1.6,
      }}
    >
      <div style={{ display: "flex", alignItems: "center", gap: 8, color: "var(--b-text)", fontWeight: 500 }}>
        <span className={dot} />
        {title}
      </div>
      {running || busy > 0 ? <div>On the {accelWhere(accel)}</div> : <div>Wakes up on your first line</div>}
      {chip ? (
        <div>
          {chip}
          {onGpu && system?.gpu_kind === "nvidia" && system.vram_gb ? ` · ${Math.round(system.vram_gb)} GB` : ""}
        </div>
      ) : null}
      <div>{claude?.connected ? "Connected to Claude" : "Claude not connected yet"}</div>
    </div>
  );
}

export function Shell({ children }: { children: ReactNode }) {
  const view = useBooth((s) => s.view);
  const go = useBooth((s) => s.go);
  return (
    <div className="booth">
      <Backdrop />
      <div className="b-shell">
        <nav aria-label="Main" className="b-nav glass">
          <div style={{ padding: "2px 10px 22px" }}>
            <Brand />
          </div>
          {NAV.map((n) => (
            <button
              aria-current={view === n.view ? "page" : undefined}
              key={n.view}
              onClick={() => go(n.view)}
              type="button"
            >
              {n.icon}
              {n.label}
            </button>
          ))}
          <UpdateCard />
          <EngineCard />
        </nav>
        <main className="b-main">{children}</main>
      </div>
    </div>
  );
}
