// A demo R1 for the window outside Tauri (a plain browser): linked, awake, three agents and a look. Pairing plays
// itself out (the QR, then "link R1?" a few seconds later). The install wizard gets a pretend R1 on USB
// (./demo-install). Nothing here talks to a real R1.

import type { Agent, LinkState } from "./link";
import { installDemo } from "./demo-install";
import { moreApi } from "./demo-more";
import { type Look, type LookPatch, mergeLook } from "./theme";

type Fn = (p: unknown) => void;
const listeners = new Map<string, Set<Fn>>();

const agents: Agent[] = [
  { id: "4f3e9879", name: "archer", form: "ghost", color: "#5aa0ff", finish: "aurora", eyes: "happy", wear: ["beanie", "scarf"], role: "", brain: "claude", personality: "friendly", state: "idle", unread: 1, can: { look: "ask", device: "ask" } },
  { id: "9c21aa04", name: "newsey", form: "orb", color: "amber", finish: "soft", eyes: "dots", wear: ["glasses", "bowtie"], role: "the news, every morning", brain: "codex", personality: "concise", state: "working" },
  { id: "1b7d33e2", name: "Kiwi", form: "bunny", color: "lime", finish: "grain", eyes: "sparkle", wear: ["flower", "blush"], role: "plants and the garden", brain: "claude", personality: "playful", state: "working", works_on: "computer:5596c5bb", folders: [String.raw`C:\Users\you\Documents\garden`], computer: { name: "DESKTOP-R1OS", connected: true } },
];
let device = { brightness: 40, auto_brightness: false, screen_timeout: 60, volume: 56, sounds: true, haptics: "medium", haptics_wheel: true, time_zone: "America/New_York", time_auto: true, build_model: "sonnet", agents_paused: false };
const accounts = { claude: true, codex: false };
let look: Look = { mode: "dark", accent: "orange", font: "rubik", home: { background: { kind: "mesh", colors: ["#4ea1ff", "#b07cf6", "#f06aa8"], grain: 0.5 } } };
const state: LinkState = {
  me: { id: "5596c5bb", name: "DESKTOP-R1OS" },
  r1s: [{ id: "b97e6ef64c", name: "R1", online: true, path: "direct" }],
  pairing: null,
  asking: null,
};

function emit(event: string, payload: unknown) {
  listeners.get(event)?.forEach((f) => f(payload));
}

/** A QR-looking square: a seeded pattern with the three finder squares. */
function fakeQr(): string {
  const n = 29;
  let s = 7;
  const rnd = () => ((s = (s * 16807) % 2147483647) % 100) / 100;
  let cells = "";
  const finder = (x: number, y: number) =>
    `<rect x="${x}" y="${y}" width="7" height="7" fill="#0b0b0c"/><rect x="${x + 1}" y="${y + 1}" width="5" height="5" fill="#fff"/><rect x="${x + 2}" y="${y + 2}" width="3" height="3" fill="#0b0b0c"/>`;
  for (let y = 0; y < n; y++)
    for (let x = 0; x < n; x++) {
      const inFinder = (x < 8 && y < 8) || (x > n - 9 && y < 8) || (x < 8 && y > n - 9);
      if (!inFinder && rnd() > 0.52) cells += `<rect x="${x}" y="${y}" width="1" height="1" fill="#0b0b0c"/>`;
    }
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${n} ${n}" shape-rendering="crispEdges"><rect width="${n}" height="${n}" fill="#fff"/>${cells}${finder(0, 0)}${finder(n - 7, 0)}${finder(0, n - 7)}</svg>`;
}

let timer: ReturnType<typeof setTimeout> | undefined;
let demoSettings = { idle_s: 300 };

/** Tomorrow at seven, or today if it's still to come. */
function nextSeven(): number {
  const d = new Date();
  d.setHours(7, 0, 0, 0);
  if (d.getTime() < Date.now()) d.setDate(d.getDate() + 1);
  return d.getTime();
}

/** A few "photos": skies and hills, drawn. */
const scene = (sky: string[], hill: string, sun: string) =>
  `<svg xmlns="http://www.w3.org/2000/svg" width="480" height="640" viewBox="0 0 480 640"><defs><linearGradient id="s" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="${sky[0]}"/><stop offset="1" stop-color="${sky[1]}"/></linearGradient></defs><rect width="480" height="640" fill="url(#s)"/><circle cx="330" cy="250" r="70" fill="${sun}"/><path d="M0 430 Q120 360 240 420 T480 400 V640 H0Z" fill="${hill}"/><path d="M0 520 Q160 470 300 520 T480 500 V640 H0Z" fill="${hill}" opacity=".7"/></svg>`;
const PHOTOS = [
  { id: "p1", svg: scene(["#ff9a62", "#6b4cff"], "#2a1747", "#ffd38a") },
  { id: "p2", svg: scene(["#8fd3ff", "#e8f6ff"], "#3d7a4f", "#fff6c9") },
  { id: "p3", svg: scene(["#14213d", "#3a4a6b"], "#0b1020", "#e9e4d4") },
  { id: "p4", svg: scene(["#ffd1dc", "#ffb38a"], "#7a5c99", "#fff1e0") },
];

export const demo = {
  async invoke(cmd: string, args?: Record<string, unknown>): Promise<unknown> {
    await new Promise((r) => setTimeout(r, 120));
    const usb = installDemo(cmd, args ?? {}, emit);
    if (usb) return usb;
    switch (cmd) {
      case "link_state":
        return structuredClone(state);
      case "pair_start":
        state.pairing = { uri: "r1os-link:1?…", svg: fakeQr(), left_s: 300 };
        clearTimeout(timer);
        timer = setTimeout(() => {
          state.pairing = null;
          state.asking = { r1: "a11ce", name: "R1", code: "6153" };
          emit("link", structuredClone(state));
        }, 5000);
        emit("link", structuredClone(state));
        return structuredClone(state);
      case "pair_stop":
        clearTimeout(timer);
        state.pairing = null;
        emit("link", structuredClone(state));
        return;
      case "pair_answer":
        if (args?.yes && state.asking && !state.r1s.some((r) => r.id === state.asking!.r1)) {
          state.r1s.push({ id: state.asking.r1, name: "R1 (2)", online: true, path: "relay" });
        }
        state.asking = null;
        emit("link", structuredClone(state));
        return;
      case "unlink":
        state.r1s = state.r1s.filter((r) => r.id !== args?.id);
        emit("link", structuredClone(state));
        return;
      case "settings_get":
        return { ...demoSettings };
      case "settings_set":
        demoSettings = { ...(args?.settings as { idle_s: number }) };
        return;
      case "pc_status":
        return {
          claude: { path: String.raw`C:\Program Files\nodejs\node_modules\@anthropic-ai\claude-code\bin\claude.exe`, version: "2.1.280 (Claude Code)", signed_in: true },
          codex: { path: String.raw`C:\Program Files\nodejs\codex.exe`, signed_in: false },
          workspaces: String.raw`C:\Users\you\Documents\r1os agents`,
          running: [{ r1: "b97e6ef64c", agent: "Kiwi", chat: "c9", started: Date.now() - 40_000, tools: ["Read", "Edit"] }],
          log: [
            { r1: "b97e6ef64c", agent: "Kiwi", chat: "c8", at: Date.now() - 3_600_000, ms: 41_000, tools: ["Glob", "Read", "Write"], ok: true },
            { r1: "b97e6ef64c", agent: "Kiwi", chat: "c7", at: Date.now() - 7_200_000, ms: 9_000, tools: ["Bash"], ok: false, error: "you said no on the r1" },
          ],
        };
      case "r1_api":
        return demo.api(String(args?.method), String(args?.path), args?.body);
    }
    throw new Error(`demo: no ${cmd}`);
  },

  api(method: string, path: string, body: unknown): { status: number; body: unknown } {
    const more = moreApi(method, path, body, agents, (event) => emit("r1", { r1: "b97e6ef64c", event }));
    if (more) return more;
    if (method === "GET" && path === "/api/state")
      return { status: 200, body: { version: "0.1.0", uptime_s: 5400, system: { battery_pct: 82, charging: false, mem_available_mb: 1840, mem_total_mb: 3857, storage_free_mb: 41_200, storage_total_mb: 108_900 } } };
    if (method === "GET" && path === "/api/agents") return { status: 200, body: { agents: structuredClone(agents), focus: agents[0].id } };
    if (method === "GET" && path === "/api/look") return { status: 200, body: { look: structuredClone(look) } };
    if (method === "PUT" && path === "/api/look") {
      look = mergeLook(look, body as LookPatch);
      setTimeout(() => emit("r1", { r1: "b97e6ef64c", event: { type: "look", look } }), 60);
      return { status: 200, body: { look } };
    }
    if (method === "GET" && path === "/api/glance")
      return {
        status: 200,
        body: {
          widgets: [
            { app: "water-log", name: "Water", icon: "drop", accent: "blue", card: { type: "stats", title: "Water", items: [{ label: "today", value: "5 of 8" }, { label: "last glass", value: "9:40 pm" }] } },
            { app: "weather-live", name: "Weather now", icon: "cloud-sun", accent: "cyan", card: { type: "weather", place: "Home", temp: 54, condition: "Overcast", unit: "F", icon: "cloud", feels: 48, days: [{ day: "Oct 10", high: 63, low: 49, icon: "cloud" }, { day: "Oct 11", high: 62, low: 50, icon: "cloud-sun" }] } },
          ],
        },
      };
    if (method === "GET" && path === "/api/alarms") return { status: 200, body: { alarms: [{ id: "a1", kind: "alarm", label: "", on: true, days: 31, at: nextSeven() }] } };
    if (method === "GET" && path.startsWith("/api/media?")) return { status: 200, body: { media: PHOTOS.map((p, i) => ({ id: p.id, kind: "photo", at: Date.now() - i * 86_400_000, mime: "image/svg+xml", bytes: p.svg.length, width: 480, height: 640, source: "camera", label: "" })) } };
    const file = path.match(/^\/api\/media\/([^/]+)\/file$/);
    if (method === "GET" && file) {
      const p = PHOTOS.find((x) => x.id === file[1]);
      if (!p) return { status: 404, body: { error: "no such photo" } };
      return { status: 200, mime: "image/svg+xml", base64: btoa(p.svg) } as never;
    }
    if (method === "GET" && path === "/api/device") return { status: 200, body: { device } };
    if (method === "PUT" && path === "/api/device/change") {
      device = { ...device, ...(body as { changes: object }).changes };
      return { status: 200, body: { device } };
    }
    if (method === "GET" && path === "/api/accounts") return { status: 200, body: { accounts } };
    if (method === "POST" && path === "/api/accounts/claude/login") return { status: 200, body: { auth_url: "https://claude.ai/oauth/authorize?demo", token: "demo" } };
    if (method === "POST" && path === "/api/accounts/codex/login") return { status: 200, body: { url: "https://auth.openai.com/codex/device", code: "ABCD-1234" } };
    if (method === "POST" && path === "/api/accounts/claude/code") {
      setTimeout(() => emit("r1", { r1: "b97e6ef64c", event: { type: "account", brain: "claude", state: "signed_in" } }), 900);
      return { status: 200, body: { ok: true } };
    }
    const out = path.match(/^\/api\/accounts\/(claude|codex)\/logout$/);
    if (method === "POST" && out) {
      accounts[out[1] as "claude" | "codex"] = false;
      return { status: 200, body: { signed_out: out[1] } };
    }
    const can = path.match(/^\/api\/agents\/([^/]+)\/can$/);
    if (method === "PUT" && can) {
      const a = agents.find((x) => x.id === can[1]);
      const b = body as { area: string; level: "ask" | "allow" | "never" };
      if (a) a.can = { ...(a.can ?? {}), [b.area]: b.level };
      setTimeout(() => emit("r1", { r1: "b97e6ef64c", event: { type: "agents_changed" } }), 60);
      return { status: 200, body: { can: a?.can } };
    }
    return { status: 404, body: { error: `demo: no ${method} ${path}` } };
  },

  listen(event: string, fn: Fn): Promise<() => void> {
    if (!listeners.has(event)) listeners.set(event, new Set());
    listeners.get(event)!.add(fn);
    return Promise.resolve(() => listeners.get(event)?.delete(fn));
  },
};
