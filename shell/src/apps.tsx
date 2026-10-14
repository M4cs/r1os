// Built-in apps for the R1 skeleton. Generated apps (EneeSpec → json-render) replace most of
// these from R2/R5; these exist to exercise the hardware and the shell.

import { useEffect, useRef, useState, type ComponentType } from "react";
import { useAppInput } from "./input";
import { api, onEvent, type R1Event, type R1State } from "./r1";

export interface AppDef {
  id: string;
  name: string;
  /** A text glyph (built-ins) or a Phosphor icon name (installed apps, `icon`). */
  glyph?: string;
  icon?: string;
  accent: string;
  Component: ComponentType<{ state: R1State | null; nav: Nav }>;
}

/** What an app screen can ask the shell to do. */
export interface Nav {
  /** Opens an installed enee app by its id. */
  openApp(appId: string): void;
  home(): void;
}

export const APPS: AppDef[] = [
  { id: "clock", name: "Clock", glyph: "◷", accent: "#7aa2ff", Component: Clock },
  { id: "camera", name: "Camera", glyph: "◐", accent: "#ff8a3d", Component: Camera },
  { id: "inputs", name: "Inputs", glyph: "◎", accent: "#5bd6a0", Component: Inputs },
  { id: "system", name: "System", glyph: "▤", accent: "#c79bff", Component: SystemInfo },
  { id: "android", name: "Android", glyph: "⚙", accent: "#9aa4b2", Component: AndroidSettings },
];

function Clock() {
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    const t = window.setInterval(() => setNow(new Date()), 1000);
    return () => window.clearInterval(t);
  }, []);
  return (
    <div className="center col">
      <div className="clock-time">{now.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}</div>
      <div className="clock-sec">{String(now.getSeconds()).padStart(2, "0")}</div>
      <div className="muted">{now.toLocaleDateString([], { weekday: "long", month: "long", day: "numeric" })}</div>
    </div>
  );
}

const PRESETS = [0, 90, 180];

/** The rotating camera's motor: wheel = ±10°, tap = cycle 0/90/180. */
function Camera({ state }: { state: R1State | null }) {
  const actual = state?.motor.deg ?? 90;
  const [target, setTarget] = useState<number | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const sending = useRef(false);
  const pending = useRef<number | null>(null);
  const shown = target ?? actual;

  // Send the latest target only: wheel spins outpace the motor, so coalesce.
  const send = (deg: number) => {
    pending.current = deg;
    if (sending.current) return;
    sending.current = true;
    void (async () => {
      while (pending.current !== null) {
        const d = pending.current;
        pending.current = null;
        try {
          await api.motor(d);
          setErr(null);
        } catch (e) {
          setErr(String(e));
        }
      }
      sending.current = false;
    })();
  };
  const move = (deg: number) => {
    const d = Math.max(0, Math.min(180, deg));
    setTarget(d);
    send(d);
  };

  useAppInput((ev) => {
    if (ev.type === "wheel") {
      move(shown + ev.delta * 10);
      return true;
    }
    if (ev.type === "gesture" && ev.kind === "tap") {
      const next = PRESETS.find((p) => p > shown) ?? 0;
      move(next);
      return true;
    }
    return false;
  });

  const a = (Math.PI * (180 - shown)) / 180;
  return (
    <div className="center col">
      <svg viewBox="-110 -110 220 120" className="gauge">
        <path d="M -100 0 A 100 100 0 0 1 100 0" className="gauge-track" />
        <line x1="0" y1="0" x2={Math.cos(a) * 90} y2={-Math.sin(a) * 90} className="gauge-needle" />
        <circle r="6" className="gauge-hub" />
      </svg>
      <div className="big">{shown}°</div>
      <div className="muted">
        wheel ±10° · tap 0/90/180{state?.motor.sim ? " · simulated" : ""}
      </div>
      {err && <div className="error">{err}</div>}
    </div>
  );
}

/** Live event log: the tool for tuning gestures and the wheel guard (Q12). */
function Inputs({ state }: { state: R1State | null }) {
  const [log, setLog] = useState<R1Event[]>([]);
  useEffect(() => onEvent((ev) => setLog((l) => [ev, ...l].slice(0, 40))), []);
  useAppInput(() => false); // observe only; double tap still goes home
  const s = state?.stats;
  return (
    <div className="col fill">
      <div className="muted small">
        wheel {state?.devices.wheel ?? "—"} · button {state?.devices.button ?? "—"}
        {s ? ` · since connect: ${s.wheel} ticks, ${s.wheel_suppressed} suppressed` : ""}
      </div>
      <ol className="log">
        {log.map((e) => (
          <li key={e.seq} className={`ev ev-${e.type}`}>
            <span className="t">{(e.t_ms / 1000).toFixed(2)}</span>
            <span className="k">{describe(e)}</span>
            {e.source === "sim" && <span className="badge">sim</span>}
          </li>
        ))}
        {log.length === 0 && <li className="muted">Spin the wheel or press the button…</li>}
      </ol>
    </div>
  );
}

function describe(e: R1Event): string {
  switch (e.type) {
    case "wheel":
      return e.delta > 0 ? "wheel ↓" : "wheel ↑";
    case "wheel_suppressed":
      return `wheel ${e.delta > 0 ? "↓" : "↑"} (suppressed)`;
    case "button":
      return e.down ? "button down" : "button up";
    case "gesture":
      return e.held_ms != null ? `${e.kind.replace("_", " ")} · ${e.held_ms} ms` : e.kind.replace("_", " ");
    case "motor":
      return `motor → ${e.deg}°`;
    case "app_changed":
      return `${e.app}: ${e.tables.join(", ")} changed`;
    case "app_installed":
      return `${e.app} installed`;
    case "app_removed":
      return `${e.app} removed`;
    case "home":
      return `home: ${e.pins.join(", ") || "nothing pinned"}`;
    case "look":
      return "look changed";
    case "approval_asked":
      return `${e.approval.name} asks: ${e.approval.title}`;
    case "approval_done":
      return `approval ${e.allowed ? "allowed" : "not allowed"}`;
    case "device_set":
      return `device: ${Object.keys(e.changes).join(", ")}`;
    case "link":
      return `link: ${e.state} ${e.name}`;
    case "glance":
      return `glance: ${e.widgets.map((w) => w.app).join(", ") || "no widgets"}`;
    case "app_services":
      return `${e.app}: ${Object.keys(e.values).join(", ")}`;
    case "app_player":
      return `${e.app}: player ${String(e.player.state ?? "")}`;
    case "app_permissions":
      return `${e.app}: permissions`;
    case "notify":
      return `${e.name}: ${e.title}`;
    case "build":
      return `build ${e.id}: ${e.stage}${e.section ? ` · ${e.section}` : ""}`;
    case "account":
      return `${e.brain}: ${e.state.replace("_", " ")}`;
    case "chat":
      return `chat ${e.kind}`;
    case "agent_state":
      return `agent ${e.state}`;
    case "agents_changed":
      return "agents changed";
    case "unread":
      return `${e.count} unread`;
    case "mail":
      return `mail ${e.kind}${e.woke ? " (woke)" : ""}: ${e.text}`;
    case "open":
      return `open ${e.app}`;
    case "voice":
      return `voice ${e.state}${e.text ? `: ${e.text}` : ""}`;
    case "voice_level":
      return `voice level ${e.level.toFixed(2)}`;
    case "volume":
      return e.on ? "volume on" : "volume off";
    case "app_errors":
      return `${e.app}: ${e.count} problem${e.count === 1 ? "" : "s"}`;
    case "open_link":
      return `${e.app}: open ${e.url}`;
    case "copy":
      return `${e.app}: copied`;
    case "alarm":
      return `alarm ${e.ringing ? "ringing" : "stopped"}${e.label ? `: ${e.label}` : ""}`;
    case "reminder":
      return `reminder: ${e.text}`;
    case "alarms_changed":
      return "alarms changed";
    case "timer":
      return `timer ${e.ringing ? "ringing" : "stopped"}: ${e.label}`;
    case "timer_phase":
      return `pomodoro: ${e.phase} · round ${e.round} of ${e.rounds}`;
    case "timers_changed":
      return "timers changed";
    case "routine":
      return `routine ran: ${e.title}`;
    case "routines_changed":
      return "routines changed";
    case "awake":
      return `${e.on ? "awake" : "may sleep"} · ${e.why}`;
    case "connections_changed":
      return "connections changed";
    case "browser":
      return `browser ${e.op}`;
    case "open_screen":
      return `open ${e.target.id ?? e.target.page ?? e.target.t}`;
    case "lock":
      return e.locked ? "locked" : e.set ? "unlocked" : "no pin";
    case "intent":
      return `system one: ${e.intent?.kind ?? "nothing"}`;
    case "media_changed":
      return `${e.kind || "media"} changed`;
    case "recording":
      return `recording ${e.state} · ${e.seconds}s`;
    case "recording_level":
      return `level ${e.level.toFixed(2)}`;
    case "note":
      return `note ${e.state}${e.text ? ` · ${e.text}` : ""}`;
    case "translator_stage":
      return `translator ${e.stage}`;
    case "translation":
      return `translation ${e.state}${e.text ? ` · ${e.text}` : e.heard ? ` · ${e.heard}` : ""}`;
    case "journal_changed":
      return "journal changed";
  }
}

function SystemInfo() {
  const [s, setS] = useState<R1State | null>(null);
  useEffect(() => {
    let alive = true;
    const load = () => api.state().then((v) => alive && setS(v), () => {});
    load();
    const t = window.setInterval(load, 2000);
    return () => {
      alive = false;
      window.clearInterval(t);
    };
  }, []);
  useAppInput(() => false);
  if (!s) return <div className="center muted">Loading…</div>;
  const rows: [string, string][] = [
    ["Battery", s.system.battery_pct != null ? `${s.system.battery_pct}%${s.system.charging ? " (charging)" : ""}` : "—"],
    ["Memory free", s.system.mem_available_mb != null ? `${s.system.mem_available_mb} / ${s.system.mem_total_mb} MB` : "—"],
    ["Camera", s.motor.deg != null ? `${s.motor.deg}°${s.motor.sim ? " (sim)" : ""}` : "—"],
    ["r1d", `v${s.version} · up ${formatUptime(s.uptime_s)}`],
    ["Mode", s.sim ? "simulator" : "hardware"],
    ["Clients", String(s.clients)],
  ];
  return (
    <dl className="kv">
      {rows.map(([k, v]) => (
        <div key={k}>
          <dt>{k}</dt>
          <dd>{v}</dd>
        </div>
      ))}
    </dl>
  );
}

function formatUptime(s: number): string {
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  return h ? `${h}h ${m}m` : `${m}m ${s % 60}s`;
}

const SCREENS = [
  ["wifi", "Wi-Fi"],
  ["bluetooth", "Bluetooth"],
  ["settings", "All settings"],
  ["developer", "Developer options"],
  ["about", "About"],
] as const;

/** Opens Android's own settings screens (r1d allowlists them). Swipe home to come back. */
function AndroidSettings() {
  const [sel, setSel] = useState(0);
  const [msg, setMsg] = useState<string | null>(null);
  const open = (i: number) =>
    api.openAndroid(SCREENS[i][0]).then(
      () => setMsg(null),
      (e) => setMsg(String(e)),
    );
  useAppInput((ev) => {
    if (ev.type === "wheel") {
      setSel((s) => Math.max(0, Math.min(SCREENS.length - 1, s + ev.delta)));
      return true;
    }
    if (ev.type === "gesture" && ev.kind === "tap") {
      void open(sel);
      return true;
    }
    return false;
  });
  return (
    <div className="col fill">
      <ul className="list">
        {SCREENS.map(([id, label], i) => (
          <li key={id} className={i === sel ? "sel" : ""} onClick={() => (setSel(i), void open(i))}>
            {label}
          </li>
        ))}
      </ul>
      {msg && <div className="error">{msg}</div>}
    </div>
  );
}
