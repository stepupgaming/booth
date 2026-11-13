"use client";

import { useEffect, useMemo, useState } from "react";
import { api, type DownloadEvent, errText, type Pack, type PackInfo, type SystemCheck } from "@workspace/core/booth/api";
import { isPhone, useBooth } from "@workspace/core/booth/store";
import { Backdrop, Brand, PlayButton, useToast, Wave } from "@workspace/core/booth/ui";

const PACK_TEXT: Record<Pack, { name: string; desc: string; files: string }> = {
  core: {
    name: "Core voice engine",
    desc: "27 preset voices, cloning from a short clip, moods and delivery controls, the fast decoder.",
    files: "backbone-ptq · zac-keep2 · spk-encoder · steer · voices",
  },
  gpu_engine: {
    name: "NVIDIA graphics engine",
    desc: "Runs Booth on your NVIDIA card, several times faster than the processor. Without it Booth uses the processor.",
    files: "zonos3-server · CUDA 12.8 · from Booth’s GitHub releases",
  },
  full_decoder: {
    name: "Studio-quality decoder",
    desc: "The full audio decoder. Cleaner, fuller sound. Just as fast on a graphics card; about 3× slower on the processor.",
    files: "zac",
  },
  exact: {
    name: "High-fidelity cloning",
    desc: "The model hears your recording itself instead of a fingerprint of it. Closer to the real person on clean audio. Needed for Exact voices you make yourself.",
    files: "zac-encoder · ssl-teacher",
  },
};
const ORDER: Pack[] = ["core", "gpu_engine", "full_decoder", "exact"];

const CHECK_OK = "M5 12l5 5 9-10";
const CHECK_WARN = "M12 7v6M12 17h.01";

function CheckRow({ ok, title, detail }: { ok: boolean; title: string; detail: string }) {
  return (
    <div className="tile" style={{ display: "flex", alignItems: "center", gap: 14, padding: "14px 16px" }}>
      <span
        style={{
          width: 32,
          height: 32,
          borderRadius: 16,
          flex: "none",
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          background: ok ? "rgba(94,240,192,0.14)" : "rgba(255,181,71,0.12)",
          color: ok ? "var(--b-ok)" : "var(--b-warn)",
          boxShadow: ok ? "0 0 18px rgba(94,240,192,0.25)" : "none",
        }}
      >
        <svg aria-hidden="true" fill="none" height="16" stroke="currentColor" strokeWidth="2.4" viewBox="0 0 24 24" width="16">
          <path d={ok ? CHECK_OK : CHECK_WARN} />
        </svg>
      </span>
      <span style={{ display: "flex", flexDirection: "column", gap: 2, flex: 1 }}>
        <span style={{ fontWeight: 500 }}>{title}</span>
        <span className="muted" style={{ fontSize: 13 }}>
          {detail}
        </span>
      </span>
    </div>
  );
}

/** What Booth will run on: the NVIDIA card, the Mac's graphics, or the processor. */
function ChipRow({ system }: { system: SystemCheck }) {
  if (isPhone(system)) {
    return (
      <CheckRow
        detail="Booth runs on the phone’s processor. A line takes a few seconds."
        ok={true}
        title={`${system.cpu_threads}-core processor · ${Math.round(system.ram_gb)} GB memory`}
      />
    );
  }
  const cpu = system.cpu ? `${system.cpu.replace(/\s+\d+-Core Processor$/, "")} · ${system.cpu_threads} threads` : "Your processor";
  if (system.gpu_kind === "apple") {
    return (
      <CheckRow
        detail="Booth runs on the Mac’s graphics (Metal) while it talks, and frees it when you close the app."
        ok={true}
        title={`${system.gpu} · ${Math.round(system.ram_gb)} GB`}
      />
    );
  }
  if (system.gpu_kind === "nvidia") {
    return (
      <CheckRow
        detail={
          system.platform === "windows"
            ? "Booth can run on this card. Its engine is ticked on the next step; without it Booth uses the processor."
            : "Booth runs on the processor here; the graphics card engine is Windows-only for now."
        }
        ok={true}
        title={`${system.gpu}${system.vram_gb ? ` · ${Math.round(system.vram_gb)} GB` : ""}`}
      />
    );
  }
  return (
    <CheckRow
      detail="No NVIDIA graphics card, so Booth runs on the processor. A line takes a few seconds."
      ok={true}
      title={cpu}
    />
  );
}

const fmtTime = (s: number) => (s >= 90 ? `${Math.round(s / 60)} min` : `${Math.max(1, Math.round(s))} s`);

export function Setup({ onDone }: { onDone: () => void }) {
  const toast = useToast();
  const system = useBooth((s) => s.system);
  const loadSystem = useBooth((s) => s.loadSystem);
  const refresh = useBooth((s) => s.refresh);
  const [step, setStep] = useState(0);
  const [packs, setPacks] = useState<PackInfo[]>([]);
  const [want, setWant] = useState<Record<Pack, boolean>>({ core: true, gpu_engine: false, full_decoder: false, exact: false });
  const [ev, setEv] = useState<DownloadEvent | null>(null);
  const [queue, setQueue] = useState<Pack[]>([]);
  const [finished, setFinished] = useState(false);
  const [failed, setFailed] = useState<string | null>(null);

  useEffect(() => {
    loadSystem().catch((e) => toast.bad(errText(e)));
    api
      .packInfo()
      .then(setPacks)
      .catch((e) => toast.bad(errText(e)));
  }, [loadSystem, toast]);

  // With an NVIDIA card, the graphics engine is ticked to start with.
  const nvidia = system?.gpu_kind === "nvidia";
  useEffect(() => {
    if (nvidia) {
      setWant((w) => ({ ...w, gpu_engine: true }));
    }
  }, [nvidia]);

  const info = (p: Pack) => packs.find((x) => x.pack === p);
  const missing = (p: Pack) => !info(p)?.present;
  // Only the packs this platform offers (the NVIDIA engine is Windows-only), and the NVIDIA
  // engine only when there is a card for it.
  const offered = ORDER.filter((p) => info(p) && (p !== "gpu_engine" || nvidia || !missing(p)));
  const chosen = offered.filter((p) => missing(p) && (p === "core" || want[p]));
  const bytes = (p: Pack) => info(p)?.bytes ?? 0;
  const totalBytes = chosen.reduce((a, p) => a + bytes(p), 0);
  const totalGb = (totalBytes / 1e9).toFixed(2);

  const progress = useMemo(() => {
    if (!ev || queue.length === 0) {
      return { done: 0, total: queue.reduce((a, p) => a + bytes(p), 0) };
    }
    const before = queue.slice(0, ev.index).reduce((a, p) => a + bytes(p), 0);
    const after = queue.slice(ev.index + 1).reduce((a, p) => a + bytes(p), 0);
    return { done: before + ev.done_bytes, total: before + ev.total_bytes + after };
    // biome-ignore lint/correctness/useExhaustiveDependencies: bytes() reads packs
  }, [ev, queue, packs]);
  const pct = finished ? 100 : progress.total > 0 ? Math.floor((progress.done / progress.total) * 100) : 0;

  const start = async () => {
    setStep(2);
    setFailed(null);
    setFinished(false);
    const list = chosen;
    setQueue(list);
    if (list.length === 0) {
      setFinished(true);
      return;
    }
    const un = await api.onDownload(setEv);
    try {
      await api.downloadPacks(list);
      setFinished(true);
      await refresh();
    } catch (e) {
      setFailed(errText(e));
    } finally {
      un();
    }
  };

  const disk = system?.disk_free_gb;
  // Room for the basics, and for every pack this platform offers (plus takes and voices).
  const basicsGb = Math.max(1.2, bytes("core") / 1e9);
  const allGb = offered.reduce((a, p) => a + bytes(p), 0) / 1e9;
  const needGb = Math.ceil(Math.max(basicsGb, allGb) + 0.5);
  const coreMissing = missing("core");
  const phone = isPhone(system);
  const device = phone ? "phone" : "computer";
  const heads = [`Let’s check your ${device}`, "Choose your sound", finished ? "You’re all set" : "Bringing the voices home"];
  const subs = [
    `Booth runs the ZONOS3-Mini voice model entirely on your ${device}. Nothing you type or record ever leaves it.`,
    offered.length > 1
      ? "Start small and add the rest later from Settings. Each pack works the moment it lands."
      : "Everything you need to speak and clone voices, in one download.",
    finished
      ? "The engine wakes up on your first line, and goes back to sleep when you close Booth."
      : "This is the only time it needs the internet.",
  ];
  const rate = ev?.rate ?? 0;
  const left = rate > 0 ? (progress.total - progress.done) / rate : null;

  return (
    <div className="booth setup-bg setup">
      <Backdrop />
      <div className="setup-brand">
        <Brand />
      </div>
      <div className="setup-body">
        <div className="setup-col">
          <div className="setup-hero">
            <span
              style={{
                position: "absolute",
                inset: 18,
                borderRadius: 40,
                background: "var(--b-red)",
                filter: "blur(48px)",
                opacity: 0.28,
              }}
            />
            <img
              alt="The NoFaceTube figure in a red-lit phone booth, holding the receiver to his hood"
              height={232}
              src="/setup-hero.jpg"
              style={{
                position: "relative",
                display: "block",
                width: "100%",
                height: "100%",
                borderRadius: 28,
                objectFit: "cover",
                border: "1px solid rgba(255,255,255,0.08)",
                boxShadow: "0 24px 60px rgba(0,0,0,0.6)",
              }}
              width={232}
            />
          </div>

          <div style={{ display: "flex", flexDirection: "column", alignItems: "center", gap: 10, textAlign: "center" }}>
            <div className="mono muted" style={{ fontSize: 12, letterSpacing: "0.12em", textTransform: "uppercase" }}>
              Step {step + 1} of 3
            </div>
            <h1 className="setup-head">{heads[step]}</h1>
            <p className="setup-sub">{subs[step]}</p>
          </div>

          <div className="glass setup-card">
            {step === 0 ? (
              system ? (
                <>
                  <ChipRow system={system} />
                  <CheckRow
                    detail={
                      offered.length > 1
                        ? `You need ${basicsGb.toFixed(1)} GB for the basics, up to ${needGb} GB with everything.`
                        : `You need ${basicsGb.toFixed(1)} GB for the voices.`
                    }
                    ok={disk == null || disk > needGb}
                    title={
                      disk == null
                        ? "Disk space: couldn’t check"
                        : `${Math.round(disk)} GB free${/^[A-Z]:$/i.test(system.disk ?? "") ? ` on drive ${system.disk}` : ""}`
                    }
                  />
                  <CheckRow
                    detail={
                      phone
                        ? "The voice engine runs on the phone itself. No account, no cloud."
                        : "No Python or PyTorch: the engine is one native program."
                    }
                    ok={true}
                    title={system.os ?? "Your system"}
                  />
                  {system.engine_found ? null : (
                    <CheckRow
                      detail="The voice engine program wasn’t found next to Booth. Reinstalling Booth puts it back."
                      ok={false}
                      title="Voice engine missing"
                    />
                  )}
                </>
              ) : (
                <div className="muted" style={{ display: "flex", gap: 10, alignItems: "center", padding: 8 }}>
                  <span className="spinner" /> Checking your {device}…
                </div>
              )
            ) : null}

            {step === 1 ? (
              <>
                {offered.map((p) => {
                  const present = !missing(p);
                  const on = p === "core" || present || want[p];
                  const locked = p === "core" || present;
                  const t = PACK_TEXT[p];
                  const size = present ? "installed" : `${p === "core" ? "" : "+"}${(bytes(p) / 1e9).toFixed(2)} GB`;
                  return (
                    <label
                      className={`tile${on ? " on" : ""}`}
                      key={p}
                      style={{ display: "flex", gap: 14, padding: "16px 18px", borderRadius: 18, cursor: locked ? "default" : "pointer" }}
                    >
                      <input
                        checked={on}
                        className="check"
                        disabled={locked}
                        onChange={() => setWant({ ...want, [p]: !want[p] })}
                        style={{ width: 20, height: 20 }}
                        type="checkbox"
                      />
                      <span style={{ display: "flex", flexDirection: "column", gap: 4, flex: 1 }}>
                        <span style={{ display: "flex", justifyContent: "space-between", gap: 12, flexWrap: "wrap" }}>
                          <span style={{ fontWeight: 600, fontSize: 15 }}>
                            {t.name}
                            {p === "full_decoder" && !present ? <span className="faint" style={{ fontWeight: 400 }}> · recommended</span> : null}
                          </span>
                          <span className="mono faint">{size}</span>
                        </span>
                        <span style={{ color: "var(--b-text-2)", lineHeight: 1.5 }}>{t.desc}</span>
                        <span className="mono" style={{ fontSize: 11, color: "var(--b-fainter)" }}>
                          {t.files}
                        </span>
                      </span>
                    </label>
                  );
                })}
                <div
                  className="muted"
                  style={{ display: "flex", justifyContent: "space-between", flexWrap: "wrap", gap: 8, padding: "6px 4px 0", fontSize: 13 }}
                >
                  <span>Voices from Hugging Face · Zyphra/ZONOS3-Mini · Apache-2.0</span>
                  <span style={{ color: "var(--b-text)", fontWeight: 500 }}>Total {totalGb} GB</span>
                </div>
              </>
            ) : null}

            {step === 2 ? (
              <div style={{ display: "flex", flexDirection: "column", gap: 16, padding: 6 }}>
                <div style={{ display: "flex", justifyContent: "space-between", alignItems: "baseline", flexWrap: "wrap", gap: 8 }}>
                  <span style={{ fontSize: 40, fontWeight: 600, letterSpacing: "-0.03em" }}>{pct}%</span>
                  <span className="mono muted" style={{ fontSize: 12 }}>
                    {(progress.done / 1e9).toFixed(2)} / {(progress.total / 1e9).toFixed(2)} GB
                    {rate > 0 && !finished ? ` · ${Math.round(rate / 1e6)} MB/s` : ""}
                    {left != null && !finished ? ` · ${fmtTime(left)} left` : ""}
                  </span>
                </div>
                <div
                  aria-label="Download progress"
                  aria-valuemax={100}
                  aria-valuemin={0}
                  aria-valuenow={pct}
                  role="progressbar"
                  style={{
                    height: 12,
                    borderRadius: 6,
                    background: "rgba(255,255,255,0.08)",
                    overflow: "hidden",
                    boxShadow: "inset 0 1px 2px rgba(0,0,0,0.4)",
                  }}
                >
                  <div
                    style={{
                      height: "100%",
                      width: `${pct}%`,
                      borderRadius: 6,
                      background: "var(--b-red)",
                      boxShadow: "0 0 14px rgba(240,32,42,0.4)",
                      transition: "width 0.3s",
                    }}
                  />
                </div>
                <div className={failed ? "warn" : "muted"} style={{ fontSize: 13 }}>
                  {failed
                    ? `Download stopped: ${failed}`
                    : finished
                      ? "All files verified"
                      : ev
                        ? `Downloading ${ev.file}`
                        : "Starting…"}
                </div>
                <div className="faint" style={{ fontSize: 13, lineHeight: 1.6 }}>
                  Resumes where it left off if you close the app. Every file is checked after download. While you wait, hear a voice: this sample
                  ships with Booth.
                </div>
                <div className="tile" style={{ display: "flex", alignItems: "center", gap: 14, padding: "10px 16px 10px 10px" }}>
                  <PlayButton label="a sample of Rupert" src="/sample-rupert.mp3" />
                  <span style={{ display: "flex", flexDirection: "column", gap: 1, flex: "none", lineHeight: 1.3 }}>
                    <span style={{ fontWeight: 500, fontSize: 13 }}>Hear Rupert</span>
                    <span className="faint" style={{ fontSize: 11 }}>
                      ships with Booth
                    </span>
                  </span>
                  <div style={{ flex: 1, minWidth: 0 }}>
                    <Wave bars={110} height={26} seed={4} src="/sample-rupert.mp3" />
                  </div>
                  <span className="mono faint" style={{ fontSize: 11 }}>
                    0:13
                  </span>
                </div>
              </div>
            ) : null}
          </div>

          <div style={{ display: "flex", gap: 10, alignItems: "center", flexWrap: "wrap", justifyContent: "center" }}>
            {step === 1 ? (
              <button className="btn ghost" onClick={() => setStep(0)} type="button">
                Back
              </button>
            ) : null}
            {step === 0 ? (
              <button className="btn primary big" disabled={!system} onClick={() => setStep(1)} type="button">
                Continue
              </button>
            ) : null}
            {step === 1 ? (
              <button className="btn primary big" disabled={packs.length === 0} onClick={start} type="button">
                {chosen.length === 0 ? "Everything’s here: continue" : `Download ${totalGb} GB`}
              </button>
            ) : null}
            {step === 2 && failed ? (
              <button className="btn primary big" onClick={start} type="button">
                Try again
              </button>
            ) : null}
            {step === 2 && finished ? (
              <button className="btn primary big" onClick={onDone} type="button">
                Open the Studio
              </button>
            ) : null}
          </div>
          <span className="faint" style={{ fontSize: 13 }}>
            {step === 0 && coreMissing ? "Takes a few minutes on a normal connection." : ""}
          </span>
        </div>
      </div>
    </div>
  );
}
