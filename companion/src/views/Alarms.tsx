// An R1's timers, alarms, reminders and routines from the computer (ADR-0028, ADR-0050, R7): start, pause and cancel
// timers and pomodoros; alarms and routines on and off; set new ones with a keyboard; let them go. They ring (and run)
// on the R1, in its time zone.

import { Icon } from "@r1os/icons";
import { motion } from "motion/react";
import { useEffect, useMemo, useState } from "react";
import { type Agent, type R1Info, api, useGet } from "../link";
import { soft } from "../motion";
import { Empty, Twice } from "./parts";
import { Item, Section, Switch } from "./Settings";

interface Alarm {
  id: string;
  kind: "alarm" | "reminder";
  label: string;
  minute?: number;
  days: number;
  at?: number;
  on: boolean;
  /** A reminder that went off, not yet seen. */
  fired?: number;
  /** A to-do ticked off (T-069). */
  done?: number;
}
const DAYS = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
/** A reminder with no time: on the to-do list (T-069). */
const isTodo = (a: Alarm) => a.kind === "reminder" && a.at == null && a.fired == null && a.minute == null;

interface Routine {
  id: string;
  agent: string;
  title: string;
  prompt: string;
  minute: number;
  days: number;
  on: boolean;
  at?: number;
  last?: number;
  chat?: string;
  show: boolean;
}

interface Timer {
  id: string;
  kind: "timer" | "pomodoro";
  label: string;
  title: string;
  length: number;
  ends?: number;
  left?: number;
  ringing?: number;
  phase?: "focus" | "break" | "long";
  round?: number;
  cycle?: { rounds: number };
}

/** 4:05, 1:02:30. */
function countdown(ms: number): string {
  const s = Math.max(0, Math.round(ms / 1000));
  const two = (n: number) => String(n).padStart(2, "0");
  return s >= 3600 ? `${Math.floor(s / 3600)}:${two(Math.floor((s % 3600) / 60))}:${two(s % 60)}` : `${Math.floor(s / 60)}:${two(s % 60)}`;
}

/** 7:30 am. */
function clock(minute: number): string {
  const h = Math.floor(minute / 60);
  return `${h % 12 || 12}:${String(minute % 60).padStart(2, "0")} ${h < 12 ? "am" : "pm"}`;
}
/** "every day", "weekdays", "mon, wed", "once". */
function days(mask: number): string {
  if (mask === 0) return "once";
  if (mask === 0b1111111) return "every day";
  if (mask === 0b0011111) return "weekdays";
  if (mask === 0b1100000) return "weekends";
  return DAYS.filter((_, i) => mask & (1 << i)).join(", ");
}

export function Alarms({ r1 }: { r1: R1Info }) {
  const { data, reload, set } = useGet<{ alarms: Alarm[]; zone: string }>(r1.id, "/api/alarms", ["alarms_changed"]);
  const [err, setErr] = useState<string | null>(null);
  const fail = (e: unknown) => setErr(e instanceof Error ? e.message : String(e));
  const alarms = (data?.alarms ?? []).filter((a) => a.kind === "alarm").sort((a, b) => (a.minute ?? 0) - (b.minute ?? 0));
  const turn = (a: Alarm, on: boolean) => {
    set({ ...data!, alarms: data!.alarms.map((x) => (x.id === a.id ? { ...x, on } : x)) });
    api(r1.id, "POST", `/api/alarms/${a.id}/on`, { on }).catch((e) => {
      fail(e);
      reload();
    });
  };
  const remove = (a: Alarm) => api(r1.id, "DELETE", `/api/alarms/${a.id}`).then(reload).catch(fail);
  const tick = (a: Alarm, done: boolean) => api(r1.id, "POST", `/api/alarms/${a.id}/done`, { done }).then(reload).catch(fail);
  const todos = (data?.alarms ?? []).filter(isTodo);
  const timed = (data?.alarms ?? []).filter((a) => a.kind === "reminder" && !isTodo(a));

  return (
    <motion.div className="list-card" initial={{ opacity: 0, y: 12 }} animate={{ opacity: 1, y: 0, transition: soft }}>
      <Timers r1={r1} onError={fail} />

      <Section title={`alarms${data ? ` · ${data.zone.replace(/_/g, " ").toLowerCase()}` : ""}`} />
      {data && alarms.length === 0 && <Empty icon="alarm">no alarms</Empty>}
      {alarms.map((a) => (
        <Item key={a.id} label={clock(a.minute ?? 0)} sub={[days(a.days), a.label].filter(Boolean).join(" · ")}>
          <div className="row-buttons">
            <Switch on={a.on} onChange={(on) => turn(a, on)} />
            <Twice label="" armed="delete?" icon="trash" onConfirm={() => void remove(a)} />
          </div>
        </Item>
      ))}
      <NewAlarm r1={r1} onDone={reload} onError={fail} />

      <Section title="to do" />
      {data && todos.length === 0 && <Empty icon="check-square">nothing to do: ask an agent ("add milk to my list"), or add one here</Empty>}
      {[...todos.filter((a) => !a.done), ...todos.filter((a) => a.done)].map((a) => (
        <Item key={a.id} label={a.label} sub={a.done ? "done" : "to do"}>
          <div className="row-buttons">
            <Switch on={!!a.done} onChange={(done) => void tick(a, done)} />
            <Twice label="" armed="delete?" icon="trash" onConfirm={() => void remove(a)} />
          </div>
        </Item>
      ))}
      {todos.some((a) => a.done) && (
        <Item label="clear done" sub={`${todos.filter((a) => a.done).length} ticked off; they go by themselves a day after`}>
          <button className="btn small" onClick={() => void api(r1.id, "POST", "/api/alarms/clear_done").then(reload).catch(fail)}>clear</button>
        </Item>
      )}
      <Section title="reminders" />
      {data && timed.length === 0 && <Empty icon="bell">no reminders: ask an agent, or add one here</Empty>}
      {timed.map((a) => (
        <Item
          key={a.id}
          label={a.label}
          sub={
            a.minute != null && a.days
              ? `${days(a.days)} ${clock(a.minute)}`
              : a.at
                ? new Date(a.at).toLocaleString("en-US", { weekday: "short", hour: "numeric", minute: "2-digit" }).toLowerCase()
                : "went off"
          }
        >
          <Twice label="" armed="delete?" icon="trash" onConfirm={() => void remove(a)} />
        </Item>
      ))}
      <NewReminder r1={r1} onDone={reload} onError={fail} />

      <Routines r1={r1} onError={fail} />
      {err && <p className="err">{err}</p>}
    </motion.div>
  );
}

/** Timers and the pomodoro (ADR-0050): their countdowns tick here once a second, only while one is running. */
function Timers({ r1, onError }: { r1: R1Info; onError: (e: unknown) => void }) {
  const { data, reload } = useGet<{ timers: Timer[]; now: number }>(r1.id, "/api/timers", ["timers_changed"]);
  // The R1's clock against this computer's, from when the list came.
  const skew = useMemo(() => (data ? data.now - Date.now() : 0), [data]);
  const [now, setNow] = useState(Date.now());
  const running = (data?.timers ?? []).some((t) => t.ends);
  useEffect(() => {
    if (!running) return;
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, [running]);
  const op = (t: Timer, what: string, body?: object) => api(r1.id, "POST", `/api/timers/${t.id}/${what}`, body).then(reload).catch(onError);
  const cancel = (t: Timer) => api(r1.id, "DELETE", `/api/timers/${t.id}`).then(reload).catch(onError);
  const timers = data?.timers ?? [];
  const pomodoro = timers.some((t) => t.kind === "pomodoro");
  const state = (t: Timer) => {
    const phase = t.kind === "pomodoro" && !t.ringing ? `${t.phase === "long" ? "long break" : t.phase} · round ${t.round} of ${t.cycle?.rounds ?? 4}` : "";
    const when = t.ringing ? "time's up" : t.left != null ? "paused" : t.ends ? `ends ${new Date(t.ends - skew).toLocaleTimeString("en-US", { hour: "numeric", minute: "2-digit" }).toLowerCase()}` : "";
    return [phase, when].filter(Boolean).join(" · ");
  };
  return (
    <>
      <Section title="timers" />
      {data && timers.length === 0 && <Empty icon="timer">no timers: "set a timer for ten minutes" on the r1, or start one here</Empty>}
      {timers.map((t) => (
        <Item key={t.id} label={t.title} sub={state(t)}>
          <div className="row-buttons">
            <span className={`countdown ${t.ringing ? "ringing" : ""}`}>{countdown(t.ends ? t.ends - skew - now : (t.left ?? 0))}</span>
            {t.ringing ? (
              <button className="btn small" onClick={() => void op(t, "stop")}>stop</button>
            ) : (
              <button className="icon-btn" title={t.ends ? "pause" : "resume"} onClick={() => void op(t, t.ends ? "pause" : "resume")}>
                <Icon name={t.ends ? "pause" : "play"} weight="fill" size={15} />
              </button>
            )}
            <button className="icon-btn" title="a minute more" onClick={() => void op(t, "add", { seconds: 60 })}>
              <Icon name="plus" weight="bold" size={15} />
            </button>
            {t.kind === "pomodoro" && !t.ringing && (
              <button className="icon-btn" title={t.phase === "focus" ? "skip to the break" : "skip to focus"} onClick={() => void op(t, "skip")}>
                <Icon name="skip-forward" weight="fill" size={15} />
              </button>
            )}
            <Twice label="" armed="cancel?" icon="trash" onConfirm={() => void cancel(t)} />
          </div>
        </Item>
      ))}
      <NewTimer r1={r1} pomodoro={pomodoro} onDone={reload} onError={onError} />
    </>
  );
}

/** "sun 7:30 pm". */
function when(ms: number): string {
  return new Date(ms).toLocaleString("en-US", { weekday: "short", hour: "numeric", minute: "2-digit" }).toLowerCase().replace(",", "");
}

/**
 * Routines (R7): an agent's turn on a schedule, in the chat you have with it now (its most recent). On and off, run one now (its reply goes to its
 * chat, on the R1 and under Chats here), let it go, or make one: who, when, and what to do each time.
 */
function Routines({ r1, onError }: { r1: R1Info; onError: (e: unknown) => void }) {
  const { data, reload, set } = useGet<{ routines: Routine[] }>(r1.id, "/api/routines", ["routines_changed"]);
  const { data: who } = useGet<{ agents: Agent[] }>(r1.id, "/api/agents", ["agents_changed"]);
  const agents = who?.agents ?? [];
  const [started, setStarted] = useState<string | null>(null);
  const routines = data?.routines ?? [];
  const name = (id: string) => agents.find((a) => a.id === id)?.name ?? "";
  const turn = (r: Routine, on: boolean) => {
    set({ ...data!, routines: routines.map((x) => (x.id === r.id ? { ...x, on } : x)) });
    api(r1.id, "POST", `/api/routines/${r.id}/on`, { on }).catch((e) => {
      onError(e);
      reload();
    });
  };
  const run = (r: Routine) => {
    setStarted(r.id);
    setTimeout(() => setStarted((s) => (s === r.id ? null : s)), 4000);
    api(r1.id, "POST", `/api/routines/${r.id}/run`).catch(onError);
  };
  const remove = (r: Routine) => api(r1.id, "DELETE", `/api/routines/${r.id}`).then(reload).catch(onError);
  return (
    <>
      <Section title="routines" />
      {data && routines.length === 0 && <Empty icon="repeat">no routines: "every weekday at 7:30, tell me the weather and my day" to any agent, or make one here</Empty>}
      {routines.map((r) => (
        <Item
          key={r.id}
          label={r.title}
          sub={[r.on && r.at ? `next ${when(r.at)}` : clock(r.minute), days(r.days), name(r.agent), r.last ? `last ran ${when(r.last)}` : ""].filter(Boolean).join(" · ")}
        >
          <div className="row-buttons">
            <button className="icon-btn" title={`run it now: "${r.prompt}"`} disabled={started === r.id} onClick={() => run(r)}>
              <Icon name={started === r.id ? "check" : "play"} weight="fill" size={15} />
            </button>
            <Switch on={r.on} onChange={(on) => turn(r, on)} />
            <Twice label="" armed="delete?" icon="trash" onConfirm={() => void remove(r)} />
          </div>
        </Item>
      ))}
      {agents.length > 0 && <NewRoutine r1={r1} agents={agents} onDone={reload} onError={onError} />}
    </>
  );
}

function NewRoutine({ r1, agents, onDone, onError }: { r1: R1Info; agents: Agent[]; onDone: () => void; onError: (e: unknown) => void }) {
  const [agent, setAgent] = useState(agents[0].id);
  const [time, setTime] = useState("07:30");
  const [on, setOn] = useState<string[]>(DAYS.slice(0, 5));
  const [title, setTitle] = useState("");
  const [prompt, setPrompt] = useState("");
  const [show, setShow] = useState(true);
  const add = () => {
    if (!prompt.trim()) return;
    api(r1.id, "POST", "/api/routines", { agent, time, days: on, title: title.trim(), prompt: prompt.trim(), show })
      .then(() => {
        setTitle("");
        setPrompt("");
        onDone();
      })
      .catch(onError);
  };
  return (
    <Item label="a new routine" wide>
      <div className="new-row">
        <div className="day-chips">
          {agents.map((a) => (
            <button key={a.id} className={`part ${agent === a.id ? "on" : ""}`} onClick={() => setAgent(a.id)}>
              {a.name}
            </button>
          ))}
        </div>
        <input className="field time" type="time" value={time} onChange={(e) => setTime(e.target.value)} />
        <div className="day-chips">
          {DAYS.map((d) => (
            <button key={d} className={`part ${on.includes(d) ? "on" : ""}`} onClick={() => setOn(on.includes(d) ? on.filter((x) => x !== d) : [...on, d])}>
              {d}
            </button>
          ))}
        </div>
      </div>
      <div className="new-row">
        <input className="field whole" placeholder="what to do each time: tell me the weather and what's on my day" value={prompt} maxLength={1000} onChange={(e) => setPrompt(e.target.value)} onKeyDown={(e) => e.key === "Enter" && add()} />
        <input className="field" placeholder="its name (optional)" value={title} maxLength={60} onChange={(e) => setTitle(e.target.value)} onKeyDown={(e) => e.key === "Enter" && add()} />
        <label className="muted small check" title="wakes the r1 and shows the reply when it's done">
          <Switch on={show} onChange={setShow} />
          show it when done
        </label>
        <motion.button className="btn small" whileTap={{ scale: 0.95 }} onClick={add} disabled={!prompt.trim()}>
          <Icon name="plus" weight="bold" size={14} />
          {on.length === 0 ? "once" : "set it"}
        </motion.button>
      </div>
    </Item>
  );
}

const PRESETS = [1, 3, 5, 10, 15, 25, 30, 60];

function NewTimer({ r1, pomodoro, onDone, onError }: { r1: R1Info; pomodoro: boolean; onDone: () => void; onError: (e: unknown) => void }) {
  const [minutes, setMinutes] = useState(10);
  const [label, setLabel] = useState("");
  const start = (body: object) =>
    api(r1.id, "POST", "/api/timers", body)
      .then(() => {
        setLabel("");
        onDone();
      })
      .catch(onError);
  return (
    <Item label="a new timer" wide>
      <div className="new-row">
        <div className="day-chips">
          {PRESETS.map((m) => (
            <button key={m} className={`part ${minutes === m ? "on" : ""}`} onClick={() => setMinutes(m)}>
              {m < 60 ? `${m} min` : `${m / 60} h`}
            </button>
          ))}
        </div>
        <input className="field time" type="number" min={1} max={1440} value={minutes} onChange={(e) => setMinutes(Math.max(1, Math.min(1440, Number(e.target.value) || 1)))} />
        <input className="field" placeholder="what it's for (optional)" value={label} maxLength={60} onChange={(e) => setLabel(e.target.value)} onKeyDown={(e) => e.key === "Enter" && start({ minutes, label: label.trim() })} />
        <motion.button className="btn small" whileTap={{ scale: 0.95 }} onClick={() => start({ minutes, label: label.trim() })}>
          <Icon name="play" weight="fill" size={13} />
          start
        </motion.button>
        {!pomodoro && (
          <motion.button className="btn small" whileTap={{ scale: 0.95 }} title="25 min focus, 5 min breaks, 4 rounds" onClick={() => start({ pomodoro: true })}>
            <Icon name="timer" weight="bold" size={14} />
            pomodoro
          </motion.button>
        )}
      </div>
    </Item>
  );
}

function NewAlarm({ r1, onDone, onError }: { r1: R1Info; onDone: () => void; onError: (e: unknown) => void }) {
  const [time, setTime] = useState("07:00");
  const [on, setOn] = useState<string[]>(DAYS.slice(0, 5));
  const [label, setLabel] = useState("");
  const add = () =>
    api(r1.id, "POST", "/api/alarms", { kind: "alarm", time, days: on, label: label.trim() })
      .then(() => {
        setLabel("");
        onDone();
      })
      .catch(onError);
  return (
    <Item label="a new alarm" wide>
      <div className="new-row">
        <input className="field time" type="time" value={time} onChange={(e) => setTime(e.target.value)} />
        <div className="day-chips">
          {DAYS.map((d) => (
            <button key={d} className={`part ${on.includes(d) ? "on" : ""}`} onClick={() => setOn(on.includes(d) ? on.filter((x) => x !== d) : [...on, d])}>
              {d}
            </button>
          ))}
        </div>
        <input className="field" placeholder="what it's for (optional)" value={label} maxLength={60} onChange={(e) => setLabel(e.target.value)} onKeyDown={(e) => e.key === "Enter" && add()} />
        <motion.button className="btn small" whileTap={{ scale: 0.95 }} onClick={add}>
          <Icon name="plus" weight="bold" size={14} />
          set it
        </motion.button>
      </div>
    </Item>
  );
}

/** How a new reminder goes off: once at a date and time, every so often at a time of day, or never (a to-do). */
const REPEATS: { label: string; days: string[] | null }[] = [
  { label: "once", days: null },
  { label: "every day", days: ["daily"] },
  { label: "weekdays", days: ["weekdays"] },
  { label: "weekends", days: ["weekends"] },
  { label: "to do (no time)", days: [] },
];

function NewReminder({ r1, onDone, onError }: { r1: R1Info; onDone: () => void; onError: (e: unknown) => void }) {
  const [text, setText] = useState("");
  const [repeat, setRepeat] = useState(0);
  const [time, setTime] = useState("09:00");
  const [when, setWhen] = useState(() => {
    const d = new Date(Date.now() + 3600_000);
    d.setMinutes(0, 0, 0);
    return new Date(d.getTime() - d.getTimezoneOffset() * 60000).toISOString().slice(0, 16);
  });
  const add = () => {
    if (!text.trim()) return;
    // The R1's own local time, as the fields write it (YYYY-MM-DDTHH:MM, HH:MM).
    const r = REPEATS[repeat];
    const body = r.days === null ? { at: when } : r.days.length ? { time, days: r.days } : {};
    api(r1.id, "POST", "/api/alarms", { kind: "reminder", text: text.trim(), ...body })
      .then(() => {
        setText("");
        onDone();
      })
      .catch(onError);
  };
  return (
    <Item label="a new reminder" wide>
      <div className="new-row">
        <input className="field" placeholder="what to remind you of" value={text} maxLength={120} onChange={(e) => setText(e.target.value)} onKeyDown={(e) => e.key === "Enter" && add()} />
        <select className="field time" value={repeat} onChange={(e) => setRepeat(Number(e.target.value))}>
          {REPEATS.map((r, i) => (
            <option key={r.label} value={i}>
              {r.label}
            </option>
          ))}
        </select>
        {REPEATS[repeat].days === null && <input className="field time" type="datetime-local" value={when} onChange={(e) => setWhen(e.target.value)} />}
        {!!REPEATS[repeat].days?.length && <input className="field time" type="time" value={time} onChange={(e) => setTime(e.target.value)} />}
        <motion.button className="btn small" whileTap={{ scale: 0.95 }} onClick={add} disabled={!text.trim()}>
          <Icon name="plus" weight="bold" size={14} />
          remind me
        </motion.button>
      </div>
    </Item>
  );
}
