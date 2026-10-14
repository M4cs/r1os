// The demo R1's apps, alarms, memories, chats, recordings, journal and Wi-Fi (./demo has its agents, look and
// settings). Made up; nothing here talks to a real R1.

import type { Agent } from "./link";

type Reply = { status: number; body: unknown };
type Emit = (event: { type: string; [k: string]: unknown }) => void;

const HOUR = 3600_000;
const apps = [
  { id: "water-log", name: "Water", icon: "drop", accent: "blue", description: "Glasses of water through the day, against a goal.", glance: true },
  { id: "weather-live", name: "Weather now", icon: "cloud-sun", accent: "cyan", description: "The weather where you are, and the next days.", glance: true },
  { id: "pushup-tracker", name: "Push-ups", icon: "barbell", accent: "orange", description: "Sets of push-ups, and your best day.", glance: true },
  { id: "dice", name: "Dice", icon: "dice-five", accent: "purple", description: "Roll dice by turning the wheel.", glance: false },
  { id: "radio", name: "Radio", icon: "radio", accent: "pink", description: "Internet radio stations you like.", glance: false },
];
const settings: Record<string, Record<string, unknown>> = { "water-log": { daily_goal: 8, remind_every: 7200 } };
const specs: Record<string, unknown> = {
  "water-log": { daily_goal: { type: "int", default: 8, label: "Daily goal", min: 1 }, remind_every: { type: "duration", default: 7200, label: "Remind me every" } },
  radio: { station: { type: "enum", options: ["jazz", "lo-fi", "news"], default: "lo-fi", label: "Station" }, autoplay: { type: "bool", default: false, label: "Play when opened" } },
};
const perms: Record<string, { id: string; label: string; reason: string; state: string }[]> = {
  "water-log": [{ id: "notify", label: "send you reminders", reason: "to remind you to drink", state: "granted" }],
  "weather-live": [{ id: "location", label: "know roughly where you are", reason: "for your local weather", state: "ask" }, { id: "net", label: "use the internet", reason: "to fetch the forecast", state: "granted" }],
};
let glance = ["water-log", "weather-live"];
let pins = ["pushup-tracker", "dice", "android:com.aurora.store"];
let trash = [{ id: "ram-glance", name: "RAM", removed_at: Date.now() - 26 * HOUR }];
interface DemoTimer {
  id: string;
  kind: "timer" | "pomodoro";
  label: string;
  title: string;
  length: number;
  ends?: number;
  left?: number;
  phase?: string;
  round?: number;
  cycle?: { rounds: number };
}
let timers: DemoTimer[] = [
  { id: "t1", kind: "timer", label: "pasta", title: "pasta", length: 10 * 60_000, ends: Date.now() + 7 * 60_000 + 12_000 },
  { id: "t2", kind: "pomodoro", label: "", title: "pomodoro", length: 25 * 60_000, ends: Date.now() + 18 * 60_000, phase: "focus", round: 2, cycle: { rounds: 4 } },
];
let alarms = [
  { id: "a1", kind: "alarm", label: "gym", minute: 7 * 60, days: 0b0011111, on: true, at: Date.now() + 9 * HOUR },
  { id: "a2", kind: "alarm", label: "", minute: 9 * 60 + 30, days: 0b1100000, on: false },
  { id: "r1", kind: "reminder", label: "call mum back", days: 0, on: true, at: Date.now() + 3 * HOUR },
];
/** 7:30 am, `days` from today. */
const halfSeven = (days: number) => new Date(new Date().setHours(7, 30, 0, 0)).getTime() + days * 24 * HOUR;
let routines = [
  { id: "ro1", agent: "4f3e9879", title: "morning", prompt: "Tell me the weather and what's on my day.", minute: 7 * 60 + 30, days: 0b0011111, on: true, at: halfSeven(1), last: halfSeven(0), chat: "c2", show: true },
  { id: "ro2", agent: "9c21aa04", title: "the news", prompt: "Three headlines worth knowing, one line each.", minute: 18 * 60, days: 0b1111111, on: false, show: false } as Record<string, unknown>,
];
/** The demo R1's connections: claude.ai connectors, and each agent's rules. */
const connectors: { name: string; label: string; status: string; source: string; slug: string; reads: string[]; acts: string[] }[] = [
  { name: "claude.ai Gmail", label: "Gmail", status: "connected", source: "claudeai", slug: "claude_ai_Gmail", reads: ["search_threads", "read_thread"], acts: ["create_draft", "send_email"] },
  { name: "claude.ai Google Calendar", label: "Google Calendar", status: "connected", source: "claudeai", slug: "claude_ai_Google_Calendar", reads: ["list_events"], acts: ["create_event", "update_event"] },
  { name: "claude.ai Google Drive", label: "Google Drive", status: "needs-auth", source: "claudeai", slug: "claude_ai_Google_Drive", reads: [], acts: [] },
];
const rules: Record<string, { off: string[]; always: string[] }> = { "4f3e9879": { off: [], always: ["mcp__claude_ai_Gmail__create_draft"] } };
const connectionState = () => ({
  servers: connectors,
  agents: [
    { id: "4f3e9879", name: "archer", brain: "claude" },
    { id: "1b7d33e2", name: "kiwi", brain: "claude" },
    { id: "9c21aa04", name: "newsey", brain: "codex" },
  ].map((a) => ({ ...a, off: rules[a.id]?.off ?? [], always: rules[a.id]?.always ?? [] })),
  add_url: "https://claude.ai/customize/connectors",
  chatgpt_url: "https://chatgpt.com/apps",
});
let memories = [
  { id: 1, text: "Sam likes their coffee black", at: Date.now() - 50 * HOUR, shared: true, by: "4f3e9879" },
  { id: 2, text: "The fern on the windowsill needs water every Sunday", at: Date.now() - 20 * HOUR, shared: false, by: "1b7d33e2" },
];
let chats = [
  { id: "c1", agent: "4f3e9879", title: "So just to light mode", brain: "claude", updated: Date.now() - 2 * HOUR, unread: 0 },
  { id: "c2", agent: "4f3e9879", title: "What's on tomorrow", brain: "claude", updated: Date.now() - 30 * HOUR, unread: 1 },
  { id: "c3", agent: "9c21aa04", title: "the news", brain: "codex", updated: Date.now() - 6 * HOUR, unread: 0 },
];
/** The demo chats' messages: one rich enough to show what a chat draws. */
type DemoMsg = { id: number; role: string; text: string; at: number; meta?: Record<string, unknown> };
let demoSkills: { name: string; description: string; from: "added" | "claude.ai"; on: boolean }[] = [
  { name: "docx", description: "Create, read and edit Word documents.", from: "claude.ai", on: true },
  { name: "pptx", description: "Make and edit PowerPoint decks.", from: "claude.ai", on: true },
  { name: "computer-use", description: "For claude.ai's own computer use.", from: "claude.ai", on: false },
];
const skillState = (agents: Agent[]) => ({ skills: demoSkills, agents: agents.map((a) => ({ id: a.id, name: a.name, brain: a.brain, off: [] as string[] })) });
/** Replies on their way, by chat: what's been said so far, and the timers to come (a stop clears them). */
const replying: Record<string, { agent: string; said: string; timers: ReturnType<typeof setTimeout>[] }> = {};
const msgs: Record<string, DemoMsg[]> = {
  c1: [
    { id: 1, role: "user", text: "Switch the R1 to light mode, and what's the weather like?", at: Date.now() - 2 * HOUR - 90_000 },
    { id: 2, role: "card", text: "Fells, 55°F, overcast", at: Date.now() - 2 * HOUR - 60_000, meta: { card: { type: "weather", place: "Fells", temp: 55, unit: "F", condition: "Overcast", icon: "cloud", high: 62, low: 50, feels: 52, days: [{ day: "Sun", high: 62, low: 50, icon: "cloud" }, { day: "Mon", high: 72, low: 53, icon: "sun" }] } } },
    { id: 3, role: "assistant", text: "Done: the R1's in **light mode** now.\n\nIt's 55° and overcast in Fells, warming to 72° tomorrow. A few options for the glance screen:\n\n- keep the dark glance screen\n- match it to light mode\n- try the `dial` clock face\n\nWant me to match it?", at: Date.now() - 2 * HOUR },
  ],
  // A routine's chat (T-092): each run shows as a divider, not its prompt.
  c3: [
    { id: 1, role: "user", text: "Three headlines worth knowing, one line each.", at: Date.now() - 30 * HOUR - 20_000, meta: { routine: "the news" } },
    { id: 2, role: "assistant", text: "Markets opened flat ahead of the jobs report; a storm is moving up the coast tonight; the council approved the new bike lanes.", at: Date.now() - 30 * HOUR },
    { id: 3, role: "user", text: "Three headlines worth knowing, one line each.", at: Date.now() - 6 * HOUR - 20_000, meta: { routine: "the news" } },
    { id: 4, role: "assistant", text: "The storm brought down trees across the county; the jobs report beat forecasts; the ferry runs again from Monday.", at: Date.now() - 6 * HOUR },
  ],
};

let recordings = [
  { id: "rec1", kind: "recording", at: Date.now() - 4 * HOUR, mime: "audio/wav", bytes: 380_000, duration_ms: 12_000, source: "recorder", label: "idea for the garden", transcript: "Put the tomatoes along the south fence this year, and the herbs by the door." },
];
let journal = [
  { id: "j1", at: Date.now() - 5 * HOUR, kind: "note", text: "Finished the first draft of the talk." },
  { id: "j2", at: Date.now() - 4 * HOUR, kind: "recording", text: "", media: "rec1", media_item: recordings[0] },
];

/** A recording to play: a second of a soft tone, as a WAV. */
function tone(): string {
  const rate = 8000;
  const n = rate;
  const buf = new DataView(new ArrayBuffer(44 + n * 2));
  const w = (o: number, s: string) => [...s].forEach((c, i) => buf.setUint8(o + i, c.charCodeAt(0)));
  w(0, "RIFF");
  buf.setUint32(4, 36 + n * 2, true);
  w(8, "WAVEfmt ");
  buf.setUint32(16, 16, true);
  buf.setUint16(20, 1, true);
  buf.setUint16(22, 1, true);
  buf.setUint32(24, rate, true);
  buf.setUint32(28, rate * 2, true);
  buf.setUint16(32, 2, true);
  buf.setUint16(34, 16, true);
  w(36, "data");
  buf.setUint32(40, n * 2, true);
  for (let i = 0; i < n; i++) buf.setInt16(44 + i * 2, Math.sin((i / rate) * 2 * Math.PI * 440) * 6000 * Math.min(1, (n - i) / 2000), true);
  let s = "";
  new Uint8Array(buf.buffer).forEach((b) => (s += String.fromCharCode(b)));
  return btoa(s);
}

export function moreApi(method: string, path: string, body: unknown, agents: Agent[], emit: Emit): Reply | null {
  const ok = (b: unknown): Reply => ({ status: 200, body: b });
  const b = (body ?? {}) as Record<string, unknown>;
  let m: RegExpMatchArray | null;
  const route = (re: RegExp) => (m = path.split("?")[0].match(re));

  // Apps, widgets, home.
  if (method === "GET" && path === "/api/apps") return ok({ apps });
  if (method === "POST" && path === "/api/apps") {
    const src = String(b.source ?? "");
    const id = src.match(/id:\s*([\w-]+)/)?.[1];
    const name = src.match(/name:\s*(.+)/)?.[1]?.trim();
    if (!id || !name) return { status: 422, body: { error: "that isn't an app's spec: it needs an id and a name" } };
    apps.push({ id, name, icon: "app-window", accent: "teal", description: "Imported from a file.", glance: false });
    emit({ type: "app_installed", app: id });
    return ok({ ok: true, app: { id, name } });
  }
  if (route(/^\/api\/apps\/([^/]+)$/)) {
    const a = apps.find((x) => x.id === m![1]);
    if (method === "GET") return a ? ok({ app: { app: a, settings: specs[a.id] ?? {} }, source: `enee: 1\napp:\n  id: ${a.id}\n  name: ${a.name}\n  icon: ${a.icon}\n` }) : { status: 404, body: { error: "no app" } };
    if (method === "DELETE" && a) {
      apps.splice(apps.indexOf(a), 1);
      trash = [{ id: a.id, name: a.name, removed_at: Date.now() }, ...trash];
      glance = glance.filter((g) => g !== a.id);
      pins = pins.filter((p) => p !== a.id);
      emit({ type: "app_removed", app: a.id });
      return ok({ removed: true });
    }
  }
  if (route(/^\/api\/apps\/([^/]+)\/settings$/)) {
    const id = m![1];
    if (method === "POST") settings[id] = { ...(settings[id] ?? {}), ...(b.values as object) };
    return ok(settings[id] ?? {});
  }
  if (route(/^\/api\/apps\/([^/]+)\/permissions$/)) {
    const list = perms[m![1]] ?? [];
    if (method === "POST") for (const [k, v] of Object.entries(b.decisions as object)) list.forEach((p) => p.id === k && (p.state = v ? "granted" : "denied"));
    return ok({ permissions: list });
  }
  if (route(/^\/api\/apps\/([^/]+)\/restore$/) && method === "POST") {
    const t = trash.find((x) => x.id === m![1]);
    trash = trash.filter((x) => x !== t);
    if (t) apps.push({ id: t.id, name: t.name, icon: "app-window", accent: "green", description: "", glance: false });
    emit({ type: "app_installed", app: m![1] });
    return ok({ app: t });
  }
  if (method === "GET" && path === "/api/trash") return ok({ apps: trash, keep_days: 7 });
  if (path === "/api/home") {
    if (method === "PUT") pins = (b.pins as string[]).slice(0, 12);
    return ok({ pins });
  }
  if (method === "PUT" && path === "/api/glance") {
    glance = b.apps as string[];
    return ok({ widgets: glance.map((app) => ({ app })) });
  }
  if (method === "GET" && path === "/api/android/apps") return ok({ apps: [{ package: "com.aurora.store", name: "Aurora Store", system: true }, { package: "org.mozilla.firefox", name: "Firefox", system: false }] });

  // Timers and the pomodoro.
  if (method === "GET" && path === "/api/timers") return ok({ timers, now: Date.now() });
  if (method === "POST" && path === "/api/timers") {
    const length = b.pomodoro ? 25 * 60_000 : Math.round((Number(b.minutes ?? 0) * 60 + Number(b.seconds ?? 0)) * 1000);
    const t: DemoTimer = b.pomodoro
      ? { id: `t${Date.now()}`, kind: "pomodoro", label: "", title: "pomodoro", length, ends: Date.now() + length, phase: "focus", round: 1, cycle: { rounds: 4 } }
      : { id: `t${Date.now()}`, kind: "timer", label: String(b.label ?? ""), title: String(b.label || `${length / 60_000} min timer`), length, ends: Date.now() + length };
    timers = [...timers.filter((x) => !(b.pomodoro && x.kind === "pomodoro")), t];
    emit({ type: "timers_changed" });
    return ok({ timer: t });
  }
  if (route(/^\/api\/timers\/([^/]+)$/) && method === "DELETE") {
    timers = timers.filter((t) => t.id !== m![1]);
    emit({ type: "timers_changed" });
    return ok({ removed: m![1] });
  }
  if (route(/^\/api\/timers\/([^/]+)\/([a-z]+)$/) && method === "POST") {
    const now = Date.now();
    timers = timers.flatMap((t) => {
      if (t.id !== m![1]) return [t];
      if (m![2] === "pause" && t.ends) return [{ ...t, ends: undefined, left: t.ends - now }];
      if (m![2] === "resume" && t.left != null) return [{ ...t, left: undefined, ends: now + t.left }];
      if (m![2] === "add") return [t.ends ? { ...t, ends: t.ends + 60_000 } : { ...t, left: (t.left ?? 0) + 60_000 }];
      if (m![2] === "skip") return [{ ...t, phase: t.phase === "focus" ? "break" : "focus", ends: now + (t.phase === "focus" ? 5 : 25) * 60_000, left: undefined }];
      return [];
    });
    emit({ type: "timers_changed" });
    return ok({ ok: true });
  }

  // Connections (ADR-0052).
  if (method === "GET" && path === "/api/connections") return ok(connectionState());
  // Skills (T-086).
  if (method === "GET" && path === "/api/skills") return ok(skillState(agents));
  if (method === "POST" && path === "/api/skills") {
    const picked = (b.skills as string[] | undefined) ?? [];
    if (!picked.length) return ok({ choose: [{ name: "pdf", description: "Read, fill and merge PDFs." }, { name: "frontend-design", description: "Distinctive, production-grade web pages." }] });
    for (const name of picked) if (!demoSkills.some((s) => s.name === name)) demoSkills.unshift({ name, description: "Added in the demo.", from: "added", on: true });
    return ok({ added: picked });
  }
  if (method === "POST" && path === "/api/skills/on") {
    const s = demoSkills.find((x) => x.name === b.skill);
    if (s && !b.agent) s.on = !!b.on;
    return ok(skillState(agents));
  }
  if (method === "DELETE" && path.startsWith("/api/skills/")) {
    const name = decodeURIComponent(path.slice("/api/skills/".length));
    demoSkills = demoSkills.filter((s) => s.name !== name);
    return ok(skillState(agents));
  }
  if (method === "POST" && path === "/api/connections/check") return ok(connectionState());
  if (method === "POST" && path === "/api/connections/login") return ok({ url: "https://claude.ai/customize/connectors" });
  if (method === "POST" && path === "/api/connections/agent") {
    const r = rules[String(b.agent)] ?? (rules[String(b.agent)] = { off: [], always: [] });
    r.off = b.on ? r.off.filter((s) => s !== b.server) : [...r.off, String(b.server)];
    emit({ type: "connections_changed" });
    return ok(connectionState());
  }
  if (method === "POST" && path === "/api/connections/add") {
    connectors.push({ name: String(b.name), label: String(b.name), status: "connected", source: "user", slug: String(b.name), reads: [], acts: [] });
    emit({ type: "connections_changed" });
    return ok(connectionState());
  }
  if (method === "POST" && path === "/api/connections/remove") {
    connectors.splice(connectors.findIndex((c) => c.name === b.server), 1);
    emit({ type: "connections_changed" });
    return ok(connectionState());
  }
  if (method === "POST" && path === "/api/connections/always") {
    const r = rules[String(b.agent)] ?? (rules[String(b.agent)] = { off: [], always: [] });
    r.always = b.allow ? [...r.always, String(b.tool)] : r.always.filter((t) => t !== b.tool);
    emit({ type: "connections_changed" });
    return ok(connectionState());
  }

  // Routines.
  if (method === "GET" && path === "/api/routines") return ok({ routines, now: Date.now() });
  if (method === "POST" && path === "/api/routines") {
    const old = routines.find((r) => r.id === b.id);
    if (old) {
      Object.assign(old, Object.fromEntries(Object.entries(b).filter(([k]) => k !== "id")));
    } else {
      const days = ((b.days as string[]) ?? []).reduce((acc, d) => acc | (1 << ["mon", "tue", "wed", "thu", "fri", "sat", "sun"].indexOf(d)), 0);
      const [h, mi] = String(b.time ?? "7:30").split(":").map(Number);
      const prompt = String(b.prompt ?? "");
      routines = [...routines, { id: `ro${Date.now()}`, agent: String(b.agent ?? "4f3e9879"), title: String(b.title || prompt.slice(0, 40)), prompt, minute: h * 60 + mi, days, on: true, at: Date.now() + 5 * HOUR, show: Boolean(b.show) }];
    }
    emit({ type: "routines_changed" });
    return ok({ routine: old ?? routines[routines.length - 1] });
  }
  if (route(/^\/api\/routines\/([^/]+)$/) && method === "DELETE") {
    routines = routines.filter((r) => r.id !== m![1]);
    emit({ type: "routines_changed" });
    return ok({ removed: m![1] });
  }
  if (route(/^\/api\/routines\/([^/]+)\/on$/) && method === "POST") {
    routines = routines.map((r) => (r.id === m![1] ? { ...r, on: Boolean(b.on) } : r));
    emit({ type: "routines_changed" });
    return ok({ ok: true });
  }
  if (route(/^\/api\/routines\/([^/]+)\/run$/) && method === "POST") {
    routines = routines.map((r) => (r.id === m![1] ? { ...r, last: Date.now() } : r));
    emit({ type: "routines_changed" });
    return { status: 202, body: { running: m![1] } };
  }

  // Alarms.
  if (method === "GET" && path === "/api/alarms") return ok({ alarms, zone: "America/New_York", now: Date.now() });
  if (method === "POST" && path === "/api/alarms") {
    const days = ((b.days as string[]) ?? []).reduce((acc, d) => acc | (1 << ["mon", "tue", "wed", "thu", "fri", "sat", "sun"].indexOf(d)), 0);
    const [h, mi] = String(b.time ?? "7:00").split(":").map(Number);
    alarms = [...alarms, b.kind === "reminder" ? { id: `r${Date.now()}`, kind: "reminder", label: String(b.text), days: 0, on: true, at: new Date(String(b.at)).getTime() } : { id: `a${Date.now()}`, kind: "alarm", label: String(b.label ?? ""), minute: h * 60 + mi, days, on: true }];
    emit({ type: "alarms_changed" });
    return ok({ alarm: alarms[alarms.length - 1] });
  }
  if (route(/^\/api\/alarms\/([^/]+)$/) && method === "DELETE") {
    alarms = alarms.filter((a) => a.id !== m![1]);
    emit({ type: "alarms_changed" });
    return ok({ removed: m![1] });
  }
  if (route(/^\/api\/alarms\/([^/]+)\/on$/) && method === "POST") {
    alarms = alarms.map((a) => (a.id === m![1] ? { ...a, on: !!b.on } : a));
    return ok({ ok: true });
  }

  // Agents: profiles, a new one, letting one go, their order; memories and chats.
  if (route(/^\/api\/agents\/([^/]+)$/) && method === "PUT") {
    const a = agents.find((x) => x.id === m![1]);
    if (a) Object.assign(a, b);
    setTimeout(() => emit({ type: "agents_changed" }), 60);
    return ok({ agent: a });
  }
  if (route(/^\/api\/agents\/([^/]+)$/) && method === "DELETE") {
    const i = agents.findIndex((x) => x.id === m![1]);
    if (i >= 0) agents.splice(i, 1);
    setTimeout(() => emit({ type: "agents_changed" }), 60);
    return ok({ deleted: true });
  }
  if (method === "POST" && path === "/api/agents") {
    const a = { id: Math.random().toString(16).slice(2, 10), name: "new agent", form: "orb", color: "orange", finish: "soft", eyes: "classic", wear: [], role: "", brain: "claude", personality: "friendly", state: "idle", ...(b as object) } as Agent;
    agents.push(a);
    setTimeout(() => emit({ type: "agents_changed" }), 60);
    return ok({ agent: a });
  }
  if (method === "PUT" && path === "/api/agents/order") {
    const ids = b.ids as string[];
    agents.sort((x, y) => ids.indexOf(x.id) - ids.indexOf(y.id));
    setTimeout(() => emit({ type: "agents_changed" }), 60);
    return ok({ ordered: true });
  }
  if (method === "GET" && path.startsWith("/api/memories")) {
    const agent = new URLSearchParams(path.split("?")[1]).get("agent");
    return ok({ memories: memories.filter((x) => x.shared || x.by === agent) });
  }
  if (route(/^\/api\/memories\/(\d+)$/) && method === "DELETE") {
    memories = memories.filter((x) => x.id !== Number(m![1]));
    return ok({ forgotten: true });
  }
  if (method === "GET" && path.startsWith("/api/chats?")) {
    const agent = new URLSearchParams(path.split("?")[1]).get("agent");
    return ok({ chats: chats.filter((c) => !agent || c.agent === agent) });
  }
  // Chats from the companion (ADR-0051): a new one, a message (its reply streams back as the R1's events would).
  if (method === "POST" && path === "/api/chats") {
    const agent = String(b.agent ?? agents[0]?.id);
    const who = agents.find((a) => a.id === agent);
    const c = { id: `c${Date.now()}`, agent, title: "New chat", brain: String(b.brain ?? who?.brain ?? "claude"), updated: Date.now(), unread: 0, ...(who?.computer ? { mode: String(b.mode ?? who.mode ?? "manual") } : {}) };
    chats = [c, ...chats];
    msgs[c.id] = [];
    return ok({ chat: c });
  }
  if (method === "POST" && (path === "/api/chats/warm" || /^\/api\/chats\/[^/]+\/seen$/.test(path))) return ok({ ok: true });
  // Stopping a reply (T-013): what it had said, then a note.
  if (route(/^\/api\/chats\/([^/]+)\/stop$/) && method === "POST") {
    const id = m![1];
    const r = replying[id];
    if (!r) return ok({ stopped: false });
    r.timers.forEach(clearTimeout);
    delete replying[id];
    const list = (msgs[id] ??= []);
    if (r.said.trim()) list.push({ id: list.length + 1, role: "assistant", text: r.said.trim(), at: Date.now() });
    list.push({ id: list.length + 1, role: "note", text: "stopped", at: Date.now() });
    emit({ type: "chat", agent: r.agent, chat: id, kind: "stopped", text: r.said.trim() });
    emit({ type: "agent_state", agent: r.agent, chat: id, state: "idle" });
    return ok({ stopped: true });
  }
  if (method === "POST" && path.startsWith("/api/media?source=chat")) return ok({ media: { id: `p${Date.now()}`, kind: "photo" } });
  if (route(/^\/api\/chats\/([^/]+)\/messages$/) && method === "POST") {
    const id = m![1];
    const c = chats.find((x) => x.id === id);
    const agent = c?.agent ?? agents[0]?.id;
    const text = String(b.text ?? "");
    const list = (msgs[id] ??= []);
    list.push({ id: list.length + 1, role: "user", text, at: Date.now() });
    if (c && c.title === "New chat") c.title = text.slice(0, 40);
    emit({ type: "chat", agent, chat: id, kind: "user", text });
    emit({ type: "agent_state", agent, chat: id, state: "thinking" });
    const reply = `Here's what I found about **${text.slice(0, 40)}**:\n\n1. It's a demo R1, so this is made up.\n2. On your R1 the answer streams in just like this.\n\nAsk me anything else.`;
    const r = (replying[id] = { agent, said: "", timers: [] as ReturnType<typeof setTimeout>[] });
    r.timers.push(setTimeout(() => emit({ type: "chat", agent, chat: id, kind: "tool", text: "WebSearch" }), 500));
    const words = reply.split(/(?<= )/);
    words.forEach((w, i) => r.timers.push(setTimeout(() => {
      if (i === 0) emit({ type: "agent_state", agent, chat: id, state: "speaking" });
      r.said += w;
      emit({ type: "chat", agent, chat: id, kind: "delta", text: w });
    }, 1100 + i * 90)));
    r.timers.push(setTimeout(() => {
      delete replying[id];
      list.push({ id: list.length + 1, role: "assistant", text: reply, at: Date.now() });
      if (c) c.updated = Date.now();
      emit({ type: "chat", agent, chat: id, kind: "done", text: reply });
      emit({ type: "agent_state", agent, chat: id, state: "idle" });
    }, 1200 + words.length * 90));
    return ok({ accepted: true });
  }
  if (route(/^\/api\/chats\/([^/]+)$/)) {
    if (method === "DELETE") {
      chats = chats.filter((c) => c.id !== m![1]);
      return ok({ deleted: true });
    }
    const c = chats.find((x) => x.id === m![1]);
    return ok({
      chat: c,
      busy: false,
      messages: msgs[m![1]] ?? [
        { id: 1, role: "user", text: c?.title ?? "", at: (c?.updated ?? Date.now()) - 60_000 },
        { id: 2, role: "assistant", text: "Done: the R1's in light mode now. Want the glance screen to match?", at: c?.updated ?? Date.now() },
      ],
    });
  }

  // Recordings and the journal (photos are in ./demo).
  if (method === "GET" && path.startsWith("/api/media?kind=recording")) return ok({ media: recordings });
  if (method === "GET" && path === "/api/journal") return ok({ entries: journal });
  if (route(/^\/api\/journal\/([^/]+)$/) && method === "DELETE") {
    journal = journal.filter((j) => j.id !== m![1]);
    emit({ type: "journal_changed" });
    return ok({ removed: true });
  }
  if (route(/^\/api\/media\/(rec\d+)\/file$/)) return { status: 200, mime: "audio/wav", base64: tone() } as never;
  if (route(/^\/api\/media\/([^/]+)$/) && method === "DELETE") {
    recordings = recordings.filter((r) => r.id !== m![1]);
    emit({ type: "media_changed", kind: "recording" });
    return ok({ removed: true });
  }

  // Wi-Fi.
  if (method === "GET" && path === "/api/wifi")
    return ok({
      status: { enabled: true, ssid: "home", rssi: -55 },
      networks: [
        { ssid: "home", rssi: -55, security: "wpa2", bars: 4 },
        { ssid: "home-5g", rssi: -62, security: "wpa2", bars: 3 },
        { ssid: "cafe guest", rssi: -78, security: "open", bars: 1 },
      ],
    });
  if (method === "POST" && (path === "/api/wifi/scan" || path === "/api/wifi/connect")) return ok({ ok: true });
  return null;
}
