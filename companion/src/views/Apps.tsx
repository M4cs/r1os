// An R1's apps from the computer (docs/specs/apps.md, ADR-0042, ADR-0044): the glance screen's widgets and the apps
// on home, in order; each app's settings and permissions (a grant waits for your tap on the R1's approval card); its
// spec exported as a file, or one imported; removed apps kept a week in the trash.

import { Icon } from "@r1os/icons";
import { AnimatePresence, Reorder, motion } from "motion/react";
import { useRef, useState } from "react";
import { type R1Info, api, saveDownload, useGet } from "../link";
import { soft } from "../motion";
import { colorOf } from "../theme";
import { Empty, Field, Sheet, Twice, ago } from "./parts";
import { Item, Section, Seg, Switch } from "./Settings";

interface App {
  id: string;
  name: string;
  icon: string;
  accent?: string;
  description?: string;
  glance?: boolean;
}
interface Setting {
  type: string;
  label?: string;
  default?: unknown;
  min?: number;
  max?: number;
  options?: string[];
}
interface Permission {
  id: string;
  label: string;
  reason?: string;
  state: "ask" | "granted" | "denied" | string;
}
const MAX_PINS = 12;

/** r1os's own tools that can be pinned to the R1's home (T-091; the shell's HomeScreen.TOOLS, r1d's screens::pinnable). */
const TOOLS: { id: string; name: string; icon: string }[] = [
  { id: "camera", name: "camera", icon: "camera" },
  { id: "photos", name: "photos", icon: "images" },
  { id: "recorder", name: "recorder", icon: "microphone" },
  { id: "journal", name: "journal", icon: "notebook" },
  { id: "translator", name: "translator", icon: "translate" },
  { id: "browser", name: "browser", icon: "globe" },
  { id: "whats_up", name: "what's up", icon: "sparkle" },
  { id: "alarms", name: "alarms", icon: "alarm" },
  { id: "reminders", name: "reminders", icon: "check-square" },
  { id: "timer", name: "timer", icon: "timer" },
  { id: "routines", name: "routines", icon: "repeat" },
  { id: "clock", name: "clock", icon: "clock" },
];

export function Apps({ r1 }: { r1: R1Info }) {
  const apps = useGet<{ apps: App[] }>(r1.id, "/api/apps", ["app_installed", "app_removed"]);
  const glance = useGet<{ widgets: { app: string }[] }>(r1.id, "/api/glance", ["glance", "app_installed", "app_removed"]);
  const home = useGet<{ pins: string[] }>(r1.id, "/api/home", ["home", "app_installed", "app_removed"]);
  const android = useGet<{ apps: { package: string; name: string }[] }>(r1.id, "/api/android/apps");
  const trash = useGet<{ apps: { id: string; name: string; removed_at: number }[]; keep_days: number }>(r1.id, "/api/trash", ["app_installed", "app_removed"]);
  const [open, setOpen] = useState<App | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const fail = (e: unknown) => setErr(e instanceof Error ? e.message : String(e));

  const list = apps.data?.apps ?? [];
  const named = (pin: string) => {
    if (pin.startsWith("tool:")) {
      const t = TOOLS.find((x) => x.id === pin.slice(5));
      return { name: t?.name ?? pin.slice(5), icon: t?.icon ?? "app-window", accent: undefined };
    }
    if (pin.startsWith("android:")) {
      const pkg = pin.slice(8);
      return { name: android.data?.apps.find((a) => a.package === pkg)?.name ?? pkg, icon: "android-logo", accent: undefined };
    }
    const a = list.find((x) => x.id === pin);
    return { name: a?.name ?? pin, icon: a?.icon ?? "app-window", accent: a?.accent };
  };

  // The widgets and the pins: shown at once in their new order, sent when the drag ends.
  const widgets = glance.data?.widgets.map((w) => w.app) ?? [];
  const pins = home.data?.pins ?? [];
  const setWidgets = (ids: string[], send = true) => {
    glance.set({ widgets: ids.map((app) => ({ app })) });
    if (send) api(r1.id, "PUT", "/api/glance", { apps: ids }).catch(fail);
  };
  const setPins = (ids: string[], send = true) => {
    home.set({ pins: ids });
    if (send) api(r1.id, "PUT", "/api/home", { pins: ids }).catch(fail);
  };

  const exportApp = async (a: App) => {
    const r = await api<{ source: string }>(r1.id, "GET", `/api/apps/${a.id}`);
    const where = await saveDownload(`${a.id}.enee.yaml`, { text: r.source, mime: "text/yaml" });
    setNote(`saved ${where.split(/[\\/]/).pop()} in downloads`);
  };
  // With its data and settings (T-048): an `.enee.json` that importing puts back whole.
  const exportWithData = async (a: App) => {
    const r = await api<Record<string, unknown>>(r1.id, "GET", `/api/apps/${a.id}/export`);
    const where = await saveDownload(`${a.id}.enee.json`, { text: JSON.stringify(r, null, 1), mime: "application/json" });
    setNote(`saved ${where.split(/[\\/]/).pop()} in downloads`);
  };

  return (
    <div className="apps-page">
      {err && <p className="err">{err}</p>}
      <motion.div className="list-card" initial={{ opacity: 0, y: 12 }} animate={{ opacity: 1, y: 0, transition: soft }}>
        <Section title={`glance widgets · drag to order`} />
        {glance.data && widgets.length === 0 && <Empty icon="cards">no widgets: switch one on below</Empty>}
        <Reorder.Group axis="y" values={widgets} onReorder={(ids) => setWidgets(ids, false)} as="div">
          {widgets.map((id) => (
            <Reorder.Item key={id} value={id} as="div" className="drag-row" onDragEnd={() => setWidgets(widgets)}>
              <AppRow app={named(id)} drag>
                <button className="btn small" onClick={() => setWidgets(widgets.filter((w) => w !== id))}>
                  take off
                </button>
              </AppRow>
            </Reorder.Item>
          ))}
        </Reorder.Group>

        <Section title={`home · ${pins.length} of ${MAX_PINS} · drag to order`} />
        {home.data && pins.length === 0 && <Empty icon="push-pin">nothing pinned: pin apps below</Empty>}
        <Reorder.Group axis="y" values={pins} onReorder={(ids) => setPins(ids, false)} as="div">
          {pins.map((id) => (
            <Reorder.Item key={id} value={id} as="div" className="drag-row" onDragEnd={() => setPins(pins)}>
              <AppRow app={named(id)} drag>
                <button className="btn small" onClick={() => setPins(pins.filter((p) => p !== id))}>
                  unpin
                </button>
              </AppRow>
            </Reorder.Item>
          ))}
        </Reorder.Group>

        <Section title={`apps · ${list.length}`} />
        {list.map((a) => (
          <AppRow key={a.id} app={a} sub={a.description} onOpen={() => setOpen(a)}>
            <div className="row-buttons">
              {a.glance && (
                <IconToggle icon="cards" on={widgets.includes(a.id)} title={widgets.includes(a.id) ? "a widget (click to take it off)" : "make it a widget"} onChange={(on) => setWidgets(on ? [...widgets, a.id] : widgets.filter((w) => w !== a.id))} />
              )}
              <IconToggle icon="push-pin" on={pins.includes(a.id)} disabled={!pins.includes(a.id) && pins.length >= MAX_PINS} title={pins.includes(a.id) ? "on home (click to unpin)" : "pin to home"} onChange={(on) => setPins(on ? [...pins, a.id] : pins.filter((p) => p !== a.id))} />
              <Icon name="caret-right" weight="bold" size={16} className="muted" />
            </div>
          </AppRow>
        ))}
        <Import r1={r1} onDone={(name) => setNote(`installed ${name}`)} />
        <AnimatePresence>{note && <motion.p className="ok-note" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }} onAnimationComplete={() => setTimeout(() => setNote(null), 2600)}>{note}</motion.p>}</AnimatePresence>

        <Section title="r1os's tools on home" />
        {TOOLS.map((t) => {
          const pin = `tool:${t.id}`;
          return (
            <AppRow key={t.id} app={{ name: t.name, icon: t.icon }} sub="r1os">
              <IconToggle icon="push-pin" on={pins.includes(pin)} disabled={!pins.includes(pin) && pins.length >= MAX_PINS} title={pins.includes(pin) ? "on home (click to unpin)" : "pin to home"} onChange={(on) => setPins(on ? [...pins, pin] : pins.filter((p) => p !== pin))} />
            </AppRow>
          );
        })}

        {!!android.data?.apps.length && <Section title="android apps on home" />}
        {android.data?.apps.map((a) => {
          const pin = `android:${a.package}`;
          return (
            <AppRow key={a.package} app={{ name: a.name, icon: "android-logo" }} sub={a.package}>
              <IconToggle icon="push-pin" on={pins.includes(pin)} disabled={!pins.includes(pin) && pins.length >= MAX_PINS} title="pin to home" onChange={(on) => setPins(on ? [...pins, pin] : pins.filter((p) => p !== pin))} />
            </AppRow>
          );
        })}

        {!!trash.data?.apps.length && <Section title={`trash · kept ${trash.data.keep_days} days`} />}
        {trash.data?.apps.map((t) => (
          <Item key={`${t.id}-${t.removed_at}`} label={t.name} sub={`removed ${ago(t.removed_at)}`}>
            <button className="btn small" onClick={() => void api(r1.id, "POST", `/api/apps/${t.id}/restore`).then(() => trash.reload()).catch(fail)}>
              bring back
            </button>
          </Item>
        ))}
      </motion.div>
      <Sheet open={!!open} onClose={() => setOpen(null)} title={open?.name}>
        {open && (
          <AppSheet
            r1={r1}
            app={open}
            onExport={() => void exportApp(open).catch(fail)}
            onExportData={() => void exportWithData(open).catch(fail)}
            onRemove={() =>
              void api(r1.id, "DELETE", `/api/apps/${open.id}`)
                .then(() => {
                  setOpen(null);
                  apps.reload();
                })
                .catch(fail)
            }
          />
        )}
      </Sheet>
    </div>
  );
}

function AppRow({ app, sub, drag, onOpen, children }: { app: { name: string; icon: string; accent?: string }; sub?: string; drag?: boolean; onOpen?: () => void; children?: React.ReactNode }) {
  return (
    <div className={`list-item app-row ${onOpen ? "opens" : ""}`} onClick={onOpen}>
      {drag && <Icon name="dots-six-vertical" weight="bold" size={16} className="drag-handle" />}
      <span className="app-icon" style={{ color: app.accent ? colorOf(app.accent) : "var(--accent)" }}>
        <Icon name={app.icon} weight="bold" size={20} />
      </span>
      <div className="list-label">
        <div className="keep-case">{app.name}</div>
        {sub && <div className="muted small keep-case one-line">{sub}</div>}
      </div>
      <div className="list-control" onClick={(e) => e.stopPropagation()}>
        {children}
      </div>
    </div>
  );
}

function IconToggle({ icon, on, onChange, title, disabled }: { icon: string; on: boolean; onChange: (on: boolean) => void; title: string; disabled?: boolean }) {
  return (
    <motion.button className={`icon-toggle ${on ? "on" : ""}`} title={title} disabled={disabled} whileTap={{ scale: 0.9 }} onClick={() => onChange(!on)}>
      <Icon name={icon} weight={on ? "fill" : "bold"} size={16} />
    </motion.button>
  );
}

/** An app's spec from a file (`.enee.yaml`), or an export with its data (`.enee.json`): r1d compiles and checks it; what's wrong is shown. */
function Import({ r1, onDone }: { r1: R1Info; onDone: (name: string) => void }) {
  const input = useRef<HTMLInputElement>(null);
  const [problems, setProblems] = useState<string[] | null>(null);
  const [busy, setBusy] = useState(false);
  const take = async (f: File) => {
    setBusy(true);
    setProblems(null);
    try {
      const text = await f.text();
      const body = f.name.endsWith(".json") ? (({ source, data, settings }) => ({ source, data, settings }))(JSON.parse(text)) : { source: text };
      const r = await api<{ app: { name: string }; import_error?: string }>(r1.id, "POST", "/api/apps", body);
      if (r.import_error) setProblems([`installed; its data wasn't: ${r.import_error}`]);
      onDone(r.app.name);
    } catch (e) {
      setProblems([e instanceof Error ? e.message : String(e)]);
    } finally {
      setBusy(false);
    }
  };
  return (
    <Item label="import an app" sub="an app's spec (.enee.yaml) or an app with its data (.enee.json), from a file you exported or were sent">
      <input ref={input} type="file" accept=".yaml,.yml,.json" hidden onChange={(e) => e.target.files?.[0] && void take(e.target.files[0])} />
      <button className="btn small" disabled={busy} onClick={() => input.current?.click()}>
        <Icon name="upload-simple" weight="bold" size={14} />
        {busy ? "installing…" : "choose a file"}
      </button>
      {problems && <p className="err small">{problems.join(" · ")}</p>}
    </Item>
  );
}

/** One app's settings (as its spec says them) and its permissions; its spec saved as a file, or the app removed. */
function AppSheet({ r1, app, onExport, onExportData, onRemove }: { r1: R1Info; app: App; onExport: () => void; onExportData: () => void; onRemove: () => void }) {
  const spec = useGet<{ app: { settings?: Record<string, Setting> } }>(r1.id, `/api/apps/${app.id}`);
  const values = useGet<Record<string, unknown>>(r1.id, `/api/apps/${app.id}/settings`);
  const perms = useGet<{ permissions: Permission[] }>(r1.id, `/api/apps/${app.id}/permissions`);
  const [err, setErr] = useState<string | null>(null);
  const [asking, setAsking] = useState<string | null>(null);
  const defs = Object.entries(spec.data?.app.settings ?? {});
  const set = (key: string, v: unknown) => {
    setErr(null);
    values.set({ ...(values.data ?? {}), [key]: v });
    api(r1.id, "POST", `/api/apps/${app.id}/settings`, { values: { [key]: v } }).catch((e) => {
      setErr(e instanceof Error ? e.message : String(e));
      values.reload();
    });
  };
  const decide = (p: Permission, allow: boolean) => {
    setErr(null);
    if (allow) setAsking(p.id);
    api<{ permissions: Permission[] }>(r1.id, "POST", `/api/apps/${app.id}/permissions`, { decisions: { [p.id]: allow }, via: "settings" })
      .then((r) => perms.set(r))
      .catch((e) => setErr(e instanceof Error ? e.message : String(e)))
      .finally(() => setAsking(null));
  };
  return (
    <div className="sheet-list">
      {app.description && <p className="muted keep-case">{app.description}</p>}
      <Section title="settings" />
      {spec.data && defs.length === 0 && <Empty icon="sliders-horizontal">it has no settings</Empty>}
      {defs.map(([key, d]) => (
        <Item key={key} label={d.label ?? key}>
          <SettingControl def={d} value={values.data?.[key] ?? d.default} onChange={(v) => set(key, v)} />
        </Item>
      ))}
      <Section title="permissions" />
      {perms.data && perms.data.permissions.length === 0 && <Empty icon="shield-check">it asks for none</Empty>}
      {perms.data?.permissions.map((p) => (
        <Item key={p.id} label={p.label} sub={asking === p.id ? "allow it on your r1: it's asking there" : p.reason}>
          <Switch on={p.state === "granted"} onChange={(on) => decide(p, on)} />
        </Item>
      ))}
      <Section title="the app" />
      <Item label="save it as a file" sub="its spec (.enee.yaml): import it on another r1, or share it">
        <button className="btn small" onClick={onExport}>
          <Icon name="download-simple" weight="bold" size={14} /> save
        </button>
      </Item>
      <Item label="save it with its data" sub="its spec, what's in it and its settings (.enee.json): importing it puts all of it back">
        <button className="btn small" onClick={onExportData}>
          <Icon name="download-simple" weight="bold" size={14} /> save
        </button>
      </Item>
      <Item label="remove it" sub="it's kept in the trash for a week">
        <Twice label="remove" armed="click again: remove it" icon="trash" onConfirm={onRemove} />
      </Item>
      {err && <p className="err">{err}</p>}
    </div>
  );
}

function SettingControl({ def, value, onChange }: { def: Setting; value: unknown; onChange: (v: unknown) => void }) {
  switch (def.type) {
    case "bool":
      return <Switch on={!!value} onChange={onChange} />;
    case "enum":
      return <Seg id={`enum-${def.label}`} items={(def.options ?? []).map((o) => [o, o] as [string, string])} value={String(value ?? "")} onChange={onChange} />;
    case "duration":
      return <Field value={duration(Number(value ?? 0))} placeholder="25m" onSave={onChange} />;
    case "int":
    case "real":
      return <Field value={String(value ?? "")} onSave={(v) => onChange(Number(v))} />;
    default:
      return <Field value={String(value ?? "")} placeholder={def.type === "time" ? "09:00" : ""} onSave={onChange} />;
  }
}

/** Seconds as you'd write them: 90s, 25m, 2h. */
function duration(s: number): string {
  if (s && s % 3600 === 0) return `${s / 3600}h`;
  if (s && s % 60 === 0) return `${s / 60}m`;
  return `${s}s`;
}
