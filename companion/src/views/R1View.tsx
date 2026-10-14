// One R1 (docs/specs/companion.md §6): its pages (listed under it in the side), each read from and written to the R1
// through the link, following the R1's own changes. A sleeping R1 still looks like itself.

import { Icon } from "@r1os/icons";
import { AnimatePresence, motion } from "motion/react";
import { useCallback, useEffect, useRef, useState } from "react";
import { Character } from "@shell/ui/Character";
import { type Agent, type R1Info, api, useR1Events } from "../link";
import { page, soft } from "../motion";
import { type Look, type LookPatch, applyLook, mergeLook, mergePatch } from "../theme";
import { About } from "./About";
import { Accounts } from "./Accounts";
import { Connections } from "./Connections";
import { Agents } from "./Agents";
import { Alarms } from "./Alarms";
import { Apps } from "./Apps";
import { Files } from "./Files";
import { LookPage } from "./Look";
import { Settings } from "./Settings";

/** What the window last saw of an R1, so a sleeping one still looks like itself. */
interface Remembered {
  look: Look | null;
  focus: Agent | null;
}
function remembered(id: string | undefined): Remembered | null {
  if (!id) return null;
  try {
    return JSON.parse(localStorage.getItem(`r1:${id}`) ?? "null");
  } catch {
    return null;
  }
}

export interface R1Facts {
  version?: string;
  uptime_s?: number;
  system?: {
    battery_pct?: number | null;
    charging?: boolean | null;
    mem_available_mb?: number | null;
    mem_total_mb?: number | null;
    storage_free_mb?: number | null;
    storage_total_mb?: number | null;
  };
}

export interface R1Data {
  facts: R1Facts | null;
  agents: Agent[];
  focus: Agent | null;
  look: Look | null;
  error: string | null;
  reload: () => void;
  /** Changes the look: shown at once, sent to the R1 (a few times a second while something's dragged), the R1's
   *  checked look taken back. */
  changeLook: (patch: LookPatch, now?: boolean) => void;
}

/** What the window shows of the R1 you picked, kept up to date by its events while it's awake. */
export function useR1(r1: R1Info | null): R1Data {
  const id = r1?.online ? r1.id : null;
  const [facts, setFacts] = useState<R1Facts | null>(null);
  const [agents, setAgents] = useState<Agent[]>([]);
  const [focusId, setFocusId] = useState<string | null>(null);
  const [look, setLook] = useState<Look | null>(null);
  const [error, setError] = useState<string | null>(null);

  const loadAgents = useCallback(() => {
    if (!id) return;
    api<{ agents: Agent[]; focus?: string }>(id, "GET", "/api/agents")
      .then((r) => {
        setAgents(r.agents);
        setFocusId(r.focus ?? null);
      })
      .catch((e) => setError(String(e)));
  }, [id]);
  const loadLook = useCallback(() => {
    if (!id) return;
    api<{ look: Look }>(id, "GET", "/api/look").then((r) => setLook(r.look)).catch((e) => setError(String(e)));
  }, [id]);
  const reload = useCallback(() => {
    if (!id) return;
    setError(null);
    api<R1Facts>(id, "GET", "/api/state").then(setFacts).catch((e) => setError(String(e)));
    loadAgents();
    loadLook();
  }, [id, loadAgents, loadLook]);

  useEffect(reload, [reload]);
  const kept = remembered(r1?.id);
  useEffect(() => applyLook(id ? look : (kept?.look ?? null)), [id, look, kept?.look?.accent, kept?.look?.mode]); // eslint-disable-line react-hooks/exhaustive-deps
  // A change being sent: the R1's echoes of earlier ones would undo it on screen, so its answer is waited for.
  const sending = useRef<LookPatch>({});
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const inFlight = useRef(0);
  const changeLook = useCallback(
    (patch: LookPatch, now = true) => {
      if (!id) return;
      setLook((l) => mergeLook(l ?? {}, patch));
      sending.current = mergePatch(sending.current, patch);
      clearTimeout(timer.current);
      timer.current = setTimeout(
        () => {
          const body = sending.current;
          sending.current = {};
          timer.current = undefined;
          inFlight.current++;
          api<{ look: Look }>(id, "PUT", "/api/look", body)
            .then((r) => {
              inFlight.current--;
              if (!inFlight.current && !timer.current) setLook(r.look);
            })
            .catch((e) => {
              inFlight.current--;
              setError(String(e instanceof Error ? e.message : e));
              loadLook();
            });
        },
        now ? 0 : 120,
      );
    },
    [id, loadLook],
  );
  useR1Events(id, (e) => {
    if (e.type === "look" && !timer.current && !inFlight.current) setLook(e.look as Look);
    if (e.type === "agents_changed" || e.type === "agent_state" || e.type === "unread") loadAgents();
  });

  const live = agents.find((a) => a.id === focusId) ?? agents[0] ?? null;
  // Awake: what it looks like is kept, for when it sleeps.
  useEffect(() => {
    if (id && (look || live)) {
      try {
        localStorage.setItem(`r1:${id}`, JSON.stringify({ look, focus: live }));
      } catch {
        // No storage: a sleeping R1 just looks plain.
      }
    }
  }, [id, look, live]);
  return {
    facts, agents, error, reload, changeLook,
    focus: id ? live : (kept?.focus ?? null),
    look: id ? look : (kept?.look ?? null),
  };
}

/** An R1's pages, in the side under it. */
export const PAGES = [
  { id: "overview", icon: "house" },
  { id: "agents", icon: "smiley" },
  { id: "apps", icon: "squares-four" },
  { id: "look", icon: "palette" },
  { id: "settings", icon: "sliders-horizontal" },
  { id: "alarms", icon: "alarm" },
  { id: "files", icon: "images" },
  { id: "accounts", icon: "user-circle" },
  { id: "connections", icon: "plugs-connected" },
  { id: "about", icon: "info" },
] as const;
export type Page = (typeof PAGES)[number]["id"];

export function R1View({ r1, data, page: at, onPage }: { r1: R1Info; data: R1Data; page: Page; onPage: (p: Page) => void }) {
  if (!r1.online) return <Asleep r1={r1} agent={data.focus} />;
  const battery = data.facts?.system?.battery_pct;
  return (
    <motion.section className="r1" {...page}>
      <header className="r1-head">
        <div className="chips">
          <span className="chip">
            <span className="dot awake" /> {r1.name.toLowerCase()} · {r1.path === "relay" ? "through a relay" : r1.path === "direct" ? "direct" : "connecting"}
          </span>
          {battery != null && (
            <span className="chip">
              <Icon name={data.facts?.system?.charging ? "battery-charging" : "battery-high"} size={15} /> {battery}%
            </span>
          )}
        </div>
        <AnimatePresence mode="popLayout" initial={false}>
          <motion.h1 key={at} initial={{ opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0, transition: soft }} exit={{ opacity: 0, y: -6, transition: { duration: 0.12 } }}>
            {at === "overview" ? r1.name : at}
          </motion.h1>
        </AnimatePresence>
      </header>
      {data.error && <p className="err">{data.error}</p>}
      <AnimatePresence mode="wait">
        <motion.div key={at} {...page}>
          {at === "overview" && <Overview r1={r1} data={data} onAgents={() => onPage("agents")} />}
          {at === "agents" && <Agents r1={r1} data={data} />}
          {at === "apps" && <Apps r1={r1} />}
          {at === "look" && <LookPage r1={r1} data={data} />}
          {at === "settings" && <Settings r1={r1} />}
          {at === "alarms" && <Alarms r1={r1} />}
          {at === "files" && <Files r1={r1} />}
          {at === "accounts" && <Accounts r1={r1} />}
          {at === "connections" && <Connections r1={r1} />}
          {at === "about" && <About r1={r1} data={data} />}
        </motion.div>
      </AnimatePresence>
    </motion.section>
  );
}

function Asleep({ r1, agent }: { r1: R1Info; agent: Agent | null }) {
  return (
    <motion.section className="asleep" {...page}>
      <Character form={agent?.form ?? "orb"} color={agent?.color ?? "orange"} finish={agent?.finish ?? "soft"} eyes={agent?.eyes} wear={agent?.wear} state="sleeping" size={160} />
      <h1>{r1.name} is asleep</h1>
      <p className="muted">it links up when it wakes, and stays while you use it here (this computer sets how long). wake it to manage it from here.</p>
    </motion.section>
  );
}

export const STATE_WORDS: Record<string, string> = {
  idle: "here", thinking: "thinking", working: "working", waiting: "waiting", speaking: "talking", listening: "listening", error: "had a problem",
};

function Overview({ r1, data, onAgents }: { r1: R1Info; data: R1Data; onAgents: () => void }) {
  const s = data.facts?.system;
  return (
    <div className="overview">
      <div className="section-head">
        <h2>agents</h2>
        <button className="link" onClick={onAgents}>
          all agents <Icon name="arrow-right" size={14} />
        </button>
      </div>
      <div className="agent-row">
        {data.agents.map((a, i) => (
          <motion.div
            key={a.id}
            className="agent-tile"
            initial={{ opacity: 0, y: 16 }}
            animate={{ opacity: 1, y: 0, transition: { ...soft, delay: 0.05 * i } }}
            whileHover={{ y: -4 }}
          >
            <Character form={a.form} color={a.color} finish={a.finish} eyes={a.eyes} wear={a.wear} state={(a.state as never) ?? "idle"} unread={a.unread ?? 0} size={120} />
            <div className="agent-name">{a.name}</div>
            <div className="agent-sub muted">{STATE_WORDS[a.state ?? "idle"] ?? a.state} · {a.brain}</div>
          </motion.div>
        ))}
      </div>
      <div className="facts">
        <Fact icon="battery-high" label="battery" value={s?.battery_pct != null ? `${s.battery_pct}%` : "…"} bar={s?.battery_pct ?? undefined} />
        <Fact icon="memory" label="memory free" value={s?.mem_available_mb != null ? `${(s.mem_available_mb / 1024).toFixed(1)} gb` : "…"} bar={s?.mem_available_mb && s.mem_total_mb ? (100 * s.mem_available_mb) / s.mem_total_mb : undefined} />
        <Fact icon="link" label="connection" value={r1.path === "relay" ? "relay" : r1.path === "direct" ? "direct" : "connecting"} />
        <Fact icon="cpu" label="r1d" value={data.facts?.version ?? "…"} mono />
      </div>
    </div>
  );
}

function Fact({ icon, label, value, bar, mono }: { icon: string; label: string; value: string; bar?: number; mono?: boolean }) {
  return (
    <div className="fact">
      <div className="fact-top">
        <Icon name={icon} size={16} />
        <span className="muted">{label}</span>
      </div>
      <div className={`fact-value ${mono ? "mono" : ""}`}>{value}</div>
      {bar !== undefined && (
        <div className="bar">
          <motion.div className="bar-fill" initial={{ width: 0 }} animate={{ width: `${Math.max(2, Math.min(100, bar))}%` }} transition={soft} />
        </div>
      )}
    </div>
  );
}
