"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { api, errText, fileUrl, type Take, type Voice } from "@workspace/core/booth/api";
import { isPhone, useBooth } from "@workspace/core/booth/store";
import { Icon, Orb, PlayButton, playAll, usePref, useToast, Wave } from "@workspace/core/booth/ui";

/**
 * A shelf is one group of takes in the Library. Today there is one shelf per voice (plus one for
 * exported scripts). Projects can become a second way to build shelves later: everything below
 * the grouping works on shelves and doesn't care how they were made.
 */
interface Shelf {
  key: string;
  title: string;
  orb: string;
  /** The voice to open in the Studio, when the shelf is one voice that still exists. */
  voiceId: string | null;
  takes: Take[];
}

const SCRIPT = "script";

function shelvesByVoice(takes: Take[], voices: Voice[]): Shelf[] {
  const map = new Map<string, Shelf>();
  for (const t of takes) {
    let s = map.get(t.voice_id);
    if (!s) {
      const v = voices.find((x) => x.id === t.voice_id);
      s =
        t.voice_id === SCRIPT
          ? { key: SCRIPT, title: "Exported scripts", orb: SCRIPT, voiceId: null, takes: [] }
          : { key: t.voice_id, title: v?.name ?? t.voice_name, orb: t.voice_id, voiceId: v ? v.id : null, takes: [] };
      map.set(t.voice_id, s);
    }
    s.takes.push(t);
  }
  // newest first, inside a shelf and between shelves
  const out = [...map.values()];
  for (const s of out) {
    s.takes.sort((a, b) => b.created_at.localeCompare(a.created_at));
  }
  return out.sort((a, b) => (b.takes[0]?.created_at ?? "").localeCompare(a.takes[0]?.created_at ?? ""));
}

const when = (iso: string) => {
  const d = new Date(iso);
  const today = new Date().toDateString() === d.toDateString();
  return today
    ? `Today ${d.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}`
    : d.toLocaleDateString([], { month: "short", day: "numeric", year: d.getFullYear() === new Date().getFullYear() ? undefined : "numeric" });
};
const count = (n: number, saved: boolean) => `${n} ${saved ? "saved " : ""}take${n === 1 ? "" : "s"}`;

function TakeRow({
  t,
  phone,
  onSaved,
  onDeleted,
}: {
  t: Take;
  phone: boolean;
  onSaved: (t: Take) => void;
  onDeleted: (id: string) => void;
}) {
  const toast = useToast();
  const [confirmDel, setConfirmDel] = useState(false);
  const src = fileUrl(t.path);
  const toggle = async () => {
    try {
      onSaved(await api.starTake(t.id, !t.starred));
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
      await api.deleteTake(t.id);
      onDeleted(t.id);
    } catch (e) {
      toast.bad(errText(e));
    }
  };
  return (
    <div className="tile lib-take">
      <div style={{ display: "flex", alignItems: "flex-start", gap: 12 }}>
        <PlayButton label={`“${t.text}”`} src={src} variant="solid" />
        <span style={{ display: "flex", flexDirection: "column", gap: 3, minWidth: 0, flex: 1 }}>
          <span className="lib-text">{t.text}</span>
          <span className="mono faint" style={{ fontSize: 11 }}>
            {when(t.created_at)} · {t.seconds.toFixed(1)} s
            {t.parts?.length ? ` · ${t.parts.length} lines · ${t.voice_name}` : t.mode ? ` · ${t.mode === "exact" ? "Exact" : "Shapeable"}` : ""}
            {t.via === "mcp" ? " · from Claude" : ""}
          </span>
        </span>
      </div>
      <Wave bars={phone ? 56 : 120} height={30} seed={t.seed ?? 7} src={src} />
      <div style={{ display: "flex", gap: 6, justifyContent: "flex-end" }}>
        {phone ? null : (
          <button
            aria-label="Show the file in its folder"
            className="btn ghost sm icon-btn"
            onClick={() => api.reveal(t.path).catch((e) => toast.bad(errText(e)))}
            title="Show in folder"
            type="button"
          >
            {Icon.folder}
          </button>
        )}
        <button
          aria-label={confirmDel ? "Really delete this take?" : "Delete this take"}
          className="btn ghost sm icon-btn"
          onBlur={() => setConfirmDel(false)}
          onClick={del}
          style={confirmDel ? { borderColor: "rgba(240,32,42,0.6)" } : undefined}
          title="Delete"
          type="button"
        >
          {confirmDel ? "Delete?" : Icon.trash}
        </button>
        <button
          aria-pressed={t.starred}
          className={`btn sm ${t.starred ? "saved" : "ghost"}`}
          onClick={toggle}
          title={t.starred ? "Take it out of the saved takes" : "Keep it with the saved takes"}
          type="button"
        >
          {t.starred ? Icon.savedOn : Icon.save}
          {t.starred ? "Saved" : "Save"}
        </button>
      </div>
    </div>
  );
}

export function Library() {
  const toast = useToast();
  const voices = useBooth((s) => s.voices);
  const pickVoice = useBooth((s) => s.pickVoice);
  const go = useBooth((s) => s.go);
  const takesDir = useBooth((s) => s.overview?.takes_dir);
  const phone = useBooth((s) => isPhone(s.system));
  const [takes, setTakes] = useState<Take[] | null>(null);
  const [show, setShow] = usePref<"saved" | "all">("libraryShow", "saved");
  const [shelfKey, setShelfKey] = usePref<string | null>("libraryShelf", null);
  // On a phone the list of voices and one voice's takes are two screens.
  const [open, setOpen] = useState(false);

  const load = useCallback(() => {
    api
      .listTakes(10_000)
      .then(setTakes)
      .catch((e) => {
        setTakes([]);
        toast.bad(errText(e));
      });
  }, [toast]);
  useEffect(load, [load]);

  const savedCount = takes?.filter((t) => t.starred).length ?? 0;
  const shelves = useMemo(
    () => shelvesByVoice((takes ?? []).filter((t) => show === "all" || t.starred), voices),
    [takes, show, voices]
  );
  const shelf = shelves.find((s) => s.key === shelfKey) ?? shelves[0] ?? null;

  const onSaved = (t: Take) => setTakes((all) => (all ?? []).map((x) => (x.id === t.id ? { ...x, starred: t.starred } : x)));
  const onDeleted = (id: string) => setTakes((all) => (all ?? []).filter((x) => x.id !== id));
  const pick = (key: string) => {
    setShelfKey(key);
    if (phone && !open) {
      // so the phone's back gesture returns to the list instead of closing the app
      history.pushState({ boothShelf: key }, "");
    }
    setOpen(true);
  };
  const back = () => (history.state?.boothShelf ? history.back() : setOpen(false));
  useEffect(() => {
    if (!phone) return;
    const onPop = () => setOpen(false);
    window.addEventListener("popstate", onPop);
    return () => window.removeEventListener("popstate", onPop);
  }, [phone]);

  const header = (
    <div className="glass" style={{ padding: "20px 22px", display: "flex", alignItems: "center", gap: 12, flexWrap: "wrap" }}>
      <h1 className="h1">Library</h1>
      <div style={{ flex: 1 }} />
      <div aria-label="Which takes" className="seg" role="tablist">
        <button aria-selected={show === "saved"} onClick={() => setShow("saved")} role="tab" type="button">
          Saved · {savedCount}
        </button>
        <button aria-selected={show === "all"} onClick={() => setShow("all")} role="tab" type="button">
          All takes · {takes?.length ?? 0}
        </button>
      </div>
      {phone || !takesDir ? null : (
        <button className="btn ghost" onClick={() => api.reveal(takesDir).catch((e) => toast.bad(errText(e)))} type="button">
          {Icon.folder}Open folder
        </button>
      )}
    </div>
  );

  if (takes === null) {
    return (
      <>
        {header}
        <div className="glass muted" style={{ padding: 22, display: "flex", gap: 8, alignItems: "center" }}>
          <span className="spinner" /> Loading takes…
        </div>
      </>
    );
  }

  if (shelves.length === 0) {
    return (
      <>
        {header}
        <div className="glass muted" style={{ padding: 22, lineHeight: 1.6 }}>
          <div style={{ color: "var(--b-text)", fontWeight: 600, fontSize: 15, marginBottom: 4 }}>
            {show === "saved" ? "Nothing saved yet" : "No takes yet"}
          </div>
          {show === "saved" ? (
            <>
              Press {Icon.save} on a take in the Studio to keep it here. Exports land here too.{" "}
              {takes.length ? (
                <button className="link" onClick={() => setShow("all")} type="button">
                  See all {takes.length} takes
                </button>
              ) : null}
            </>
          ) : (
            "Every line you generate shows up here, grouped by voice."
          )}{" "}
          <button className="link" onClick={() => go("studio")} type="button">
            Open the Studio
          </button>
        </div>
      </>
    );
  }

  const list = (
    <nav aria-label="Voices with takes" className="glass lib-shelves">
      {shelves.map((s) => (
        <button aria-current={!phone && s.key === shelf?.key ? "true" : undefined} key={s.key} onClick={() => pick(s.key)} type="button">
          <Orb id={s.orb} size={36} />
          <span style={{ display: "flex", flexDirection: "column", minWidth: 0, flex: 1, textAlign: "left", lineHeight: 1.35 }}>
            <span style={{ fontWeight: 600, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{s.title}</span>
            <span className="faint small">
              {count(s.takes.length, show === "saved")} · {when(s.takes[0]?.created_at ?? "")}
            </span>
          </span>
          {phone ? <span style={{ transform: "rotate(180deg)", color: "var(--b-faint)", display: "flex" }}>{Icon.back}</span> : null}
        </button>
      ))}
    </nav>
  );

  const detail = shelf ? (
    <section aria-label={`Takes of ${shelf.title}`} className="lib-detail">
      <div className="glass" style={{ padding: "16px 18px", display: "flex", alignItems: "center", gap: 14, flexWrap: "wrap" }}>
        {phone ? (
          <button aria-label="Back to all voices" className="btn ghost sm icon-btn" onClick={back} type="button">
            {Icon.back}
          </button>
        ) : null}
        <Orb id={shelf.orb} size={phone ? 40 : 52} />
        <span style={{ display: "flex", flexDirection: "column", minWidth: 0, flex: "1 1 140px", lineHeight: 1.3 }}>
          <span style={{ fontSize: phone ? 20 : 24, fontWeight: 600, letterSpacing: "-0.02em" }}>{shelf.title}</span>
          <span className="muted small">{count(shelf.takes.length, show === "saved")}</span>
        </span>
        <div style={{ display: "flex", gap: 8 }}>
          <button className="btn ghost sm" onClick={() => playAll(shelf.takes.map((t) => fileUrl(t.path)))} type="button">
            {Icon.play}Play all
          </button>
          {shelf.voiceId ? (
            <button className="btn ghost sm" onClick={() => shelf.voiceId && pickVoice(shelf.voiceId)} type="button">
              Use in Studio
            </button>
          ) : null}
        </div>
      </div>
      {shelf.takes.map((t) => (
        <TakeRow key={t.id} onDeleted={onDeleted} onSaved={onSaved} phone={phone} t={t} />
      ))}
    </section>
  ) : null;

  return (
    <>
      {header}
      {phone ? (open ? detail : list) : (
        <div className="lib">
          {list}
          {detail}
        </div>
      )}
    </>
  );
}
