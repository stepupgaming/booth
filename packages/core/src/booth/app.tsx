"use client";

import { useEffect, useState } from "react";
import { CreateVoice } from "@workspace/core/booth/create-voice";
import { Settings } from "@workspace/core/booth/settings";
import { Setup } from "@workspace/core/booth/setup";
import { Shell } from "@workspace/core/booth/shell";
import { packPresent, useBooth } from "@workspace/core/booth/store";
import { Studio } from "@workspace/core/booth/studio";
import { ToastHost } from "@workspace/core/booth/ui";
import { Voices } from "@workspace/core/booth/voices";

function Views() {
  const view = useBooth((s) => s.view);
  switch (view) {
    case "voices":
      return <Voices />;
    case "create":
      return <CreateVoice />;
    case "settings":
      return <Settings />;
    default:
      return <Studio />;
  }
}

/** The whole Booth window: first-run setup, then the Studio and friends. */
export function BoothApp() {
  const overview = useBooth((s) => s.overview);
  const refresh = useBooth((s) => s.refresh);
  const loadSystem = useBooth((s) => s.loadSystem);
  const loadClaude = useBooth((s) => s.loadClaude);
  const loadVoices = useBooth((s) => s.loadVoices);
  const loadCatalog = useBooth((s) => s.loadCatalog);
  const [booted, setBooted] = useState(false);
  const [setupDone, setSetupDone] = useState(false);
  const [preview, setPreview] = useState(false);

  useEffect(() => {
    const boot = async () => {
      // Outside the Tauri window (a plain browser), run on sample data instead of the engine.
      if (!("__TAURI_INTERNALS__" in window)) {
        (await import("@workspace/core/booth/mock")).installMock();
        setPreview(true);
      }
      await Promise.all([refresh(), loadSystem().catch(() => undefined), loadClaude()]);
    };
    boot().finally(() => setBooted(true));
  }, [refresh, loadSystem, loadClaude]);

  const [inSetup, setInSetup] = useState(false);
  const state = overview?.engine.state;
  const missing = !packPresent(overview, "core") || state === "no_engine" || state === "no_models";
  // Once Setup is showing it stays until its own "Open the Studio" button, even after the
  // download lands and the overview says the core pack is present.
  const needsSetup = inSetup || (!setupDone && missing);
  const ready = booted && overview && !needsSetup;

  useEffect(() => {
    if (booted && overview && missing && !setupDone) {
      setInSetup(true);
    }
  }, [booted, overview, missing, setupDone]);

  useEffect(() => {
    if (ready) {
      loadVoices().catch(() => undefined);
      loadCatalog();
    }
  }, [ready, loadVoices, loadCatalog]);

  // Look for a newer Booth a few seconds after start (quietly: offline is fine).
  const checkUpdate = useBooth((s) => s.checkUpdate);
  useEffect(() => {
    if (!ready) {
      return;
    }
    const t = setTimeout(() => checkUpdate().catch(() => undefined), 4000);
    return () => clearTimeout(t);
  }, [ready, checkUpdate]);

  let body: React.ReactNode;
  if (!booted) {
    body = <div className="booth" />;
  } else if (!overview) {
    body = (
      <div className="booth" style={{ display: "flex", alignItems: "center", justifyContent: "center", padding: 32 }}>
        <div className="glass" style={{ padding: 24, maxWidth: 420, lineHeight: 1.6 }}>
          <h1 className="h2">Booth couldn’t start</h1>
          <p className="muted">The app’s backend didn’t answer. Restart Booth; if it keeps happening, reinstall it.</p>
        </div>
      </div>
    );
  } else if (needsSetup) {
    body = (
      <Setup
        onDone={async () => {
          await refresh();
          setSetupDone(true);
          setInSetup(false);
        }}
      />
    );
  } else {
    body = (
      <Shell>
        <Views />
      </Shell>
    );
  }
  return (
    <ToastHost>
      {body}
      {preview ? (
        <div className="badge" style={{ position: "fixed", right: 14, bottom: 14, zIndex: 50 }} title="Running in a browser on sample data">
          Preview · sample data
        </div>
      ) : null}
    </ToastHost>
  );
}
