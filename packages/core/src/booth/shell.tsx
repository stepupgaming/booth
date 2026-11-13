"use client";

import type { ReactNode } from "react";
import { useBooth, type View } from "@workspace/core/booth/store";
import { Backdrop, Brand, Icon } from "@workspace/core/booth/ui";

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
  const gpu = system?.gpu?.replace(/^NVIDIA (GeForce )?/, "");
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
        lineHeight: 1.6,
      }}
    >
      <div style={{ display: "flex", alignItems: "center", gap: 8, color: "var(--b-text)", fontWeight: 500 }}>
        <span className={dot} />
        {title}
      </div>
      {running || busy > 0 ? null : <div>Wakes up on your first line</div>}
      {gpu ? (
        <div>
          {gpu}
          {system?.vram_gb ? ` · ${Math.round(system.vram_gb)} GB` : ""}
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
          <EngineCard />
        </nav>
        <main className="b-main">{children}</main>
      </div>
    </div>
  );
}
