// An R1's own settings, from the computer (ADR-0045's display and sound, and the rest of Settings): one list in one
// card, read from `/api/device`, changed through `PUT /api/device/change` (the user's change: no approval), following
// the R1's `device_set` events.

import { LayoutGroup, motion } from "motion/react";
import { useCallback, useEffect, useRef, useState } from "react";
import { type R1Info, api, useGet, useR1Events } from "../link";
import { soft, spring } from "../motion";

interface Device {
  brightness?: number;
  auto_brightness?: boolean;
  screen_timeout?: number;
  volume?: number;
  sounds?: boolean;
  haptics?: string;
  haptics_wheel?: boolean;
  time_zone?: string;
  time_auto?: boolean;
  build_model?: string;
  agents_paused?: boolean;
}

const TIMEOUTS: [number, string][] = [[15, "15 s"], [30, "30 s"], [60, "1 min"], [120, "2 min"], [300, "5 min"], [600, "10 min"]];
const MODELS: [string, string][] = [["haiku", "haiku"], ["sonnet", "sonnet"], ["opus", "opus"]];
const HAPTICS: [string, string][] = [["off", "off"], ["light", "light"], ["medium", "medium"], ["strong", "strong"]];

export function Settings({ r1 }: { r1: R1Info }) {
  const [d, setD] = useState<Device | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const load = useCallback(() => {
    api<{ device: Device }>(r1.id, "GET", "/api/device").then((r) => setD(r.device)).catch((e) => setErr(String(e)));
  }, [r1.id]);
  useEffect(load, [load]);
  useR1Events(r1.id, (e) => e.type === "device_set" && load());

  /** Shown at once; sent (a level being dragged, a few times a second). */
  const pending = useRef<Partial<Device>>({});
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const change = (c: Partial<Device>, now = false) => {
    setD((x) => ({ ...(x ?? {}), ...c }));
    pending.current = { ...pending.current, ...c };
    clearTimeout(timer.current);
    timer.current = setTimeout(
      () => {
        const changes = pending.current;
        pending.current = {};
        api(r1.id, "PUT", "/api/device/change", { changes }).catch((e) => {
          setErr(String(e));
          load();
        });
      },
      now ? 0 : 140,
    );
  };

  if (!d) return <p className="muted">{err ?? "…"}</p>;
  return (
    <motion.div className="list-card" initial={{ opacity: 0, y: 12 }} animate={{ opacity: 1, y: 0, transition: soft }}>
      <Section title="display" />
      <Item label="brightness" sub={d.auto_brightness ? "automatic: moving it sets it yourself" : undefined}>
        <Level value={d.brightness ?? 0} onChange={(v) => change(d.auto_brightness ? { brightness: v, auto_brightness: false } : { brightness: v })} />
      </Item>
      <Item label="automatic brightness" sub="follows the light around it">
        <Switch on={!!d.auto_brightness} onChange={(on) => change({ auto_brightness: on }, true)} />
      </Item>
      <Item label="screen sleeps after">
        <Seg id="timeout" items={TIMEOUTS} value={d.screen_timeout ?? 60} onChange={(v) => change({ screen_timeout: v }, true)} />
      </Item>

      <Section title="sound" />
      <Item label="volume">
        <Level value={d.volume ?? 0} onChange={(v) => change({ volume: v })} />
      </Item>
      <Item label="r1os sounds" sub="clicks and ticks as you use it">
        <Switch on={d.sounds ?? true} onChange={(on) => change({ sounds: on }, true)} />
      </Item>

      <Item label="haptics" sub="a touch from the r1's motor as you use it">
        <Seg id="haptics" items={HAPTICS} value={d.haptics ?? "medium"} onChange={(v) => change({ haptics: v }, true)} />
      </Item>
      {(d.haptics ?? "medium") !== "off" && (
        <Item label="on the wheel too" sub="a tick as you turn it">
          <Switch on={d.haptics_wheel ?? true} onChange={(on) => change({ haptics_wheel: on }, true)} />
        </Item>
      )}

      <Section title="agents" />
      <Item label="talk to each other" sub={d.agents_paused ? "paused: their messages wait until you talk to them" : "they can ask each other for help"}>
        <Switch on={!d.agents_paused} onChange={(on) => change({ agents_paused: !on }, true)} />
      </Item>
      <Item label="app builds use" sub="the model that builds and changes apps">
        <Seg id="model" items={MODELS} value={d.build_model ?? "sonnet"} onChange={(v) => change({ build_model: v }, true)} />
      </Item>

      <Section title="time" />
      <Item label="time zone">
        <span className="value">{(d.time_zone ?? "").replace(/_/g, " ")}</span>
      </Item>
      <Item label="set automatically" sub="from where the r1 is connected">
        <Switch on={!!d.time_auto} onChange={(on) => change({ time_auto: on }, true)} />
      </Item>

      <Wifi r1={r1} />
      {err && <p className="err">{err}</p>}
    </motion.div>
  );
}

interface Network {
  ssid: string;
  rssi: number;
  security: string;
  bars: number;
}

/** Wi-Fi: where it's connected, and the networks it sees; a password typed here goes to the R1 once. */
function Wifi({ r1 }: { r1: R1Info }) {
  const { data, reload } = useGet<{ status: { enabled: boolean; ssid?: string; rssi?: number }; networks: Network[] }>(r1.id, "/api/wifi");
  const [joining, setJoining] = useState<Network | null>(null);
  const [password, setPassword] = useState("");
  const [note, setNote] = useState<string | null>(null);
  const [scanning, setScanning] = useState(false);
  const scan = () => {
    setScanning(true);
    api(r1.id, "POST", "/api/wifi/scan")
      .catch(() => undefined)
      .finally(() => setTimeout(() => (reload(), setScanning(false)), 3500));
  };
  const join = (n: Network, pw?: string) => {
    setNote(`joining ${n.ssid}…`);
    api(r1.id, "POST", "/api/wifi/connect", { ssid: n.ssid, security: n.security, password: pw ?? null })
      .then(() => setTimeout(reload, 5000))
      .catch((e) => setNote(e instanceof Error ? e.message : String(e)))
      .finally(() => {
        setJoining(null);
        setPassword("");
      });
  };
  const here = data?.status.ssid;
  const others = (data?.networks ?? []).filter((n) => n.ssid && n.ssid !== here).slice(0, 12);
  return (
    <>
      <Section title="wi-fi" />
      <Item label={here ?? (data ? "not connected" : "…")} sub={here ? "connected" : undefined}>
        <button className="btn small" onClick={scan} disabled={scanning}>
          {scanning ? "looking…" : "look again"}
        </button>
      </Item>
      {others.map((n) => (
        <Item key={n.ssid} label={n.ssid} sub={`${"▮".repeat(n.bars)}${"▯".repeat(Math.max(0, 4 - n.bars))} · ${n.security === "open" ? "open" : n.security}`} wide={joining?.ssid === n.ssid}>
          {joining?.ssid === n.ssid ? (
            <div className="new-row">
              <input
                className="field"
                type="password"
                autoFocus
                placeholder="its password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                onKeyDown={(e) => e.key === "Enter" && password && join(n, password)}
              />
              <button className="btn small" disabled={!password} onClick={() => join(n, password)}>
                join
              </button>
              <button className="btn small" onClick={() => setJoining(null)}>
                cancel
              </button>
            </div>
          ) : (
            <button className="btn small" onClick={() => (n.security === "open" ? join(n) : setJoining(n))}>
              join
            </button>
          )}
        </Item>
      ))}
      {note && <p className="muted small list-foot">{note}</p>}
    </>
  );
}

// ---- the list's parts (the look page uses them too) ------------------------------------------------------

export function Section({ title }: { title: string }) {
  return <div className="list-section">{title}</div>;
}

/** A row: its name (and a line under it) on the left, its control on the right (under it when there's no room, or
 *  always when it's `wide`). */
export function Item({ label, sub, wide, children }: { label: string; sub?: string; wide?: boolean; children: React.ReactNode }) {
  return (
    <div className={`list-item ${wide ? "wide" : ""}`}>
      <div className="list-label">
        <div>{label}</div>
        {sub && <div className="muted small">{sub}</div>}
      </div>
      <div className="list-control">{children}</div>
    </div>
  );
}

/** A level 0–100 (or `max`): a track that fills with the accent, dragged or clicked, and its value. */
export function Level({ value, onChange, max = 100, unit = "%" }: { value: number; onChange: (v: number) => void; max?: number; unit?: string }) {
  return (
    <div className="level-inline">
      <div className="track">
        <motion.div className="track-fill" animate={{ width: `${(100 * value) / max}%` }} transition={spring} />
        <input type="range" min={0} max={max} value={value} onChange={(e) => onChange(Number(e.target.value))} />
      </div>
      <span className="mono muted level-value">{value}{unit}</span>
    </div>
  );
}

export function Switch({ on, onChange }: { on: boolean; onChange: (on: boolean) => void }) {
  return (
    <button className={`switch ${on ? "on" : ""}`} onClick={() => onChange(!on)} role="switch" aria-checked={on}>
      <motion.span className="knob" initial={false} animate={{ x: on ? 18 : 0 }} transition={spring} />
    </button>
  );
}

/** A toggle row on its own (the computer's page). */
export function Toggle({ label, sub, on, onChange }: { label: string; sub?: string; on: boolean; onChange: (on: boolean) => void }) {
  return (
    <Item label={label} sub={sub}>
      <Switch on={on} onChange={onChange} />
    </Item>
  );
}

export function Seg<T extends string | number>({ id, items, value, onChange }: { id: string; items: [T, string][]; value: T; onChange: (v: T) => void }) {
  return (
    <LayoutGroup id={id}>
      <div className="seg">
        {items.map(([v, label]) => (
          <button key={String(v)} className={`seg-item ${v === value ? "on" : ""}`} onClick={() => onChange(v)}>
            {v === value && <motion.span className="seg-fill" layoutId="fill" transition={spring} />}
            <span>{label}</span>
          </button>
        ))}
      </div>
    </LayoutGroup>
  );
}
