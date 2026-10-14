// Managing apps (docs/specs/experience.md §2.6): every app with its history, undo, delete, and the
// trash. Changes are made by voice: on an app's page, hold the button and say what to change.

import { useEffect, useState } from "react";
import { Icon } from "@r1os/icons";
import { appsApi, type AppSummary, type AppVersion, type Trashed } from "./apps/host";
import type { Target } from "./launcher";
import { onEvent } from "./r1";
import { useFocusList } from "./setup";
import { COLORS } from "./ui/Character";

function ago(ms: number): string {
  const s = Math.max(0, (Date.now() - ms) / 1000);
  if (s < 60) return "just now";
  if (s < 3600) return `${Math.floor(s / 60)} min ago`;
  if (s < 86400) return `${Math.floor(s / 3600)} h ago`;
  return `${Math.floor(s / 86400)} d ago`;
}

function Glyph({ app }: { app: Pick<AppSummary, "icon" | "accent"> }) {
  return (
    <span className="l-glyph" style={{ ["--accent" as string]: COLORS[app.accent ?? ""] ?? "#9aa4b2" }}>
      <Icon name={app.icon} weight="bold" size={20} />
    </span>
  );
}

/** All apps, then the trash. Tap an app for its page; tap a trashed one to bring it back. */
export function ManageApps({ onOpen }: { onOpen: (t: Target) => void }) {
  const [apps, setApps] = useState<AppSummary[]>([]);
  const [trash, setTrash] = useState<Trashed[]>([]);
  const [keepDays, setKeepDays] = useState(7);
  const [msg, setMsg] = useState<string | null>(null);
  useEffect(() => {
    const load = () => {
      void appsApi.list().then(setApps, () => {});
      void appsApi.trash().then((t) => {
        setTrash(t.apps);
        setKeepDays(t.keep_days);
      }, () => {});
    };
    load();
    return onEvent((ev) => {
      if (ev.type === "app_installed" || ev.type === "app_removed") load();
    });
  }, []);
  const ref = useFocusList<HTMLDivElement>([apps.length, trash.length]);
  const restore = (t: Trashed) =>
    appsApi.restore(t.id).then(
      () => setMsg(`${t.name} is back.`),
      (e) => setMsg(String(e instanceof Error ? e.message : e)),
    );

  return (
    <div className="launcher" ref={ref}>
      {apps.length === 0 && <div className="muted manage-empty">No apps yet. Hold the button and ask for one.</div>}
      {apps.map((a) => (
        <button key={a.id} className="l-item" onClick={() => onOpen({ t: "app-info", id: a.id })}>
          <Glyph app={a} />
          <span className="grow ellipsis">{a.name}</span>
          <span className="muted small">›</span>
        </button>
      ))}
      {trash.length > 0 && <div className="l-section">Trash · kept {keepDays} days</div>}
      {trash.map((t) => (
        <button key={`${t.id}-${t.removed_at}`} className="l-item trashed" onClick={() => void restore(t)}>
          <span className="l-glyph">↺</span>
          <span className="grow ellipsis">{t.name}</span>
          <span className="muted small">{ago(t.removed_at)}</span>
        </button>
      ))}
      {msg && <div className="manage-msg">{msg}</div>}
    </div>
  );
}

type Busy = { stage: string; section?: string } | null;

/** One app: what it is, change it by voice, its history, undo, open, delete. */
export function AppInfo({ id, onOpen, onBack }: { id: string; onOpen: (t: Target) => void; onBack: () => void }) {
  const [app, setApp] = useState<AppSummary | null>(null);
  const [versions, setVersions] = useState<AppVersion[]>([]);
  const [busy, setBusy] = useState<Busy>(null);
  const [msg, setMsg] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [build, setBuild] = useState<number | null>(null);

  useEffect(() => {
    const load = () => {
      void appsApi.list().then((as) => setApp(as.find((a) => a.id === id) ?? null), () => {});
      void appsApi.history(id).then(setVersions, () => setVersions([]));
    };
    load();
    return onEvent((ev) => {
      if (ev.type === "app_installed" && ev.app === id) load();
      if (ev.type === "build" && ev.id === build) {
        if (ev.stage === "done") {
          setBusy(null);
          setMsg(ev.said ?? "Changed.");
          if (ev.deleted?.length) setMsg(`${ev.said ?? "Changed."} Deleted: ${ev.deleted.join(", ")}.`);
        } else if (ev.stage === "failed") {
          setBusy(null);
          setMsg(`That change didn't work: ${ev.error ?? "unknown error"}`);
        } else {
          setBusy({ stage: ev.stage, section: ev.section });
        }
      }
    });
  }, [id, build]);

  useEffect(() => {
    if (!confirmDelete) return;
    const t = window.setTimeout(() => setConfirmDelete(false), 3000);
    return () => window.clearTimeout(t);
  }, [confirmDelete]);

  const change = (request: string) => {
    setMsg(null);
    setBusy({ stage: "starting" });
    appsApi.edit(id, request).then(
      (r) => setBuild(r.build),
      (e) => {
        setBusy(null);
        setMsg(String(e instanceof Error ? e.message : e));
      },
    );
  };
  const undo = () =>
    appsApi.undo(id).then(
      (r) => setMsg(`Undid: ${r.undone.said || r.undone.request}${r.restored.length ? ` · brought back ${r.restored.join(", ")}` : ""}`),
      (e) => setMsg(String(e instanceof Error ? e.message : e)),
    );
  const remove = () => {
    if (!confirmDelete) {
      setConfirmDelete(true);
      return;
    }
    void appsApi.remove(id).then(onBack, (e) => setMsg(String(e instanceof Error ? e.message : e)));
  };

  // Held-button words change this app. (Through the focus list's `extra`: a screen has one input
  // handler, and a second useAppInput would replace the list's.)
  const ref = useFocusList<HTMLDivElement>([versions.length, !!busy], (ev) => {
    if (ev.type === "voice" && ev.state === "final" && ev.text?.trim()) {
      if (!busy) change(ev.text.trim());
      return true;
    }
    return false;
  });

  if (!app) return <div className="center muted">…</div>;
  const changes = versions.filter((v) => v.kind !== "create").reverse();
  return (
    <div className="launcher app-info" ref={ref}>
      <div className="info-head">
        <Glyph app={app} />
        <div className="grow">
          <div className="info-name">{app.name}</div>
          {app.description && <div className="muted small">{app.description}</div>}
        </div>
      </div>
      {busy ? (
        <div className="create-talk">
          <span className="talk-dot" />
          {busy.stage === "repairing" ? "Fixing up the change…" : `Changing it${busy.section ? ` · ${busy.section}` : "…"}`}
        </div>
      ) : (
        <div className="create-talk">
          <span className="talk-dot" />
          Hold the button and say what to change
        </div>
      )}
      {msg && <div className="manage-msg">{msg}</div>}
      <button className="l-item" onClick={() => onOpen({ t: "app", id })}>
        <span className="l-glyph">▶</span>
        <span className="grow">Open</span>
      </button>
      {changes.length > 0 && (
        <button className="l-item" onClick={() => void undo()}>
          <span className="l-glyph">↶</span>
          <span className="grow ellipsis">Undo: {changes[0]!.said || changes[0]!.request}</span>
        </button>
      )}
      <button className={`l-item${confirmDelete ? " danger" : ""}`} onClick={remove}>
        <span className="l-glyph">🗑</span>
        <span className="grow">{confirmDelete ? "Tap again to remove (kept 7 days)" : "Remove"}</span>
      </button>
      {versions.length > 0 && <div className="l-section">History</div>}
      {[...versions].reverse().map((v) => (
        <div key={v.at} className="history-row">
          <span className="muted small">{ago(v.at)}</span>
          <span className="grow">{v.kind === "create" ? `Created${v.request ? `: ${v.request}` : ""}` : v.said || v.request}</span>
          {v.deleted.length > 0 && <span className="small error">deleted {v.deleted.join(", ")}</span>}
        </div>
      ))}
    </div>
  );
}
