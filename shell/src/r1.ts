// The shell's link to r1d (docs/specs/r1d.md): the /ws event stream, /api calls, and in the
// simulator, /harness input injection.

import { useEffect, useRef, useState } from "react";

export type GestureKind = "tap" | "double_tap" | "hold_start" | "hold_end";

/** What an agent is doing (r1d's `agent_state`): waiting = for room in the box to run. */
/** An agent's request to change something on the R1 (ADR-0045), waiting for the user's answer (`GET /api/approvals`). */
export interface Approval {
  id: string;
  agent: string;
  name: string;
  /** look · device · apps */
  area: string;
  area_words: string;
  /** What it wants to do, after "<name> wants to". */
  title: string;
  detail: string;
  why: string;
  /** Whether "always allow" is offered (not for granting an app a permission). */
  always: boolean;
  at: number;
}

export type AgentLive = "idle" | "waiting" | "thinking" | "speaking" | "working" | "error";

/** A mail card's details (a `mail` or `mail_out` message's meta; ADR-0012). */
export interface MailCard {
  mail: number;
  kind: "ask" | "tell" | "reply";
  /** Who sent it (incoming), or who it went to (outgoing). */
  from?: string;
  to?: string;
  error?: boolean;
  thread?: string;
}

/** What System One made of what was said (crate::system1, ADR-0032). */
export interface SystemOneIntent {
  /** `android`: an Android app, its package in `app`. */
  kind: "look" | "selfie" | "camera" | "photos" | "journal" | "record" | "stop_record" | "open" | "android" | "media" | "edit" | "run";
  app?: string;
  action?: string;
  args?: Record<string, unknown>;
  text?: string;
  score: number;
  via: "rule" | "model";
}

export type R1Event = { seq: number; t_ms: number; source: "hw" | "sim" | "api" } & (
  | { type: "wheel"; delta: number }
  | { type: "wheel_suppressed"; delta: number }
  | { type: "button"; down: boolean }
  /** `woke`: its first press came with the display off (a double click then wakes straight to home, ADR-0016). */
  | { type: "gesture"; kind: GestureKind; held_ms?: number; woke?: boolean }
  /** The camera's motor (ADR-0046): where it's going (`moving`), or where it came to rest. */
  | { type: "motor"; deg: number; moving: boolean }
  | { type: "app_changed"; app: string; tables: string[] }
  | { type: "app_installed"; app: string }
  | { type: "app_services"; app: string; values: Record<string, unknown> }
  /** What the R1's player is doing for this app (ADR-0040): the page's `/player`. */
  | { type: "app_player"; app: string; player: Record<string, unknown> }
  | { type: "app_permissions"; app: string }
  | { type: "notify"; app: string; name: string; icon: string; accent?: string; title: string; body: string }
  | { type: "app_removed"; app: string }
  /** The apps pinned to home changed (ADR-0044): all of them, in order (an app's id, or `android:<package>`). */
  | { type: "home"; pins: string[] }
  /** The device's look changed (ADR-0043). */
  | { type: "look"; look: Record<string, unknown> }
  /** An agent asks to change something on the R1 (ADR-0045): the native shell shows the approval card. */
  | { type: "approval_asked"; approval: Approval }
  /** It was answered, or nobody answered in time: the card goes. */
  | { type: "approval_done"; id: string; allowed: boolean }
  /** The host applies these Android settings (ADR-0045): brightness, auto_brightness, screen_timeout, volume, sounds. */
  | { type: "device_set"; changes: Record<string, unknown> }
  /** Pairing with a computer (ADR-0048): how it went, with the code both screens showed. */
  | { type: "link"; state: "paired" | "refused" | "failed"; name: string; code: string; why?: string | null }
  /** The glance screen's widgets changed (ADR-0042): all of them, in order. `card` is a checked card (ADR-0039); none:
   *  `note` says why. */
  | { type: "glance"; widgets: { app: string; name: string; icon: string; accent?: string; card?: Record<string, unknown>; note?: string }[] }
  /** The app's page reported a problem: `count` distinct ones since its last version (a fix repairs them). */
  | { type: "app_errors"; app: string; count: number }
  /** An app's `open_url` / `copy` step, checked by r1d: the native shell opens the link or copies the text. */
  | { type: "open_link"; app: string; url: string }
  | { type: "copy"; app: string; text: string }
  /** An alarm rings, or stopped ringing (ADR-0028). */
  | { type: "alarm"; id: string; label: string; agent?: string; ringing: boolean }
  /** An agent's reminder went off: the display woke, and the shell shows it until it's seen. */
  | { type: "reminder"; id: string; text: string; agent?: string; at: number }
  | { type: "alarms_changed" }
  /** A timer (or a pomodoro's set) rings, or stopped ringing (ADR-0050). */
  | { type: "timer"; id: string; label: string; agent?: string; ringing: boolean }
  /** A pomodoro moved on to its next phase, which runs until `ends`: the display woke. */
  | { type: "timer_phase"; id: string; label: string; phase: "focus" | "break" | "long"; round: number; rounds: number; length: number; ends: number; agent?: string }
  | { type: "timers_changed" }
  /** A routine ran with `show` on (R7): the display woke; `text` is the start of its agent's reply in `chat`. */
  | { type: "routine"; id: string; agent: string; chat: string; title: string; text: string }
  | { type: "routines_changed" }
  /** Keep the CPU awake for `why` (`routine:<id>`) while `on`: a routine's turn with the screen off (the host's wake lock). */
  | { type: "awake"; why: string; on: boolean }
  /** An agent's `open_screen` (R7): one of r1os's own screens, `{t: tool, id, mine?, theirs?}`, `{t: settings, page}` or `{t: camera}`. */
  /** What the agents' CLIs reach on your accounts changed (R8, ADR-0052). */
  | { type: "connections_changed" }
  /** Do `op` in the R1's browser (ADR-0054): the host answers at `POST /api/browser/result`. */
  | { type: "browser"; id: string; op: string; agent: string; chat: string; args: Record<string, unknown> }
  | { type: "open_screen"; target: { t: string; id?: string; page?: string; mine?: string; theirs?: string } }
  /** The PIN lock (ADR-0016): whether one is set, and whether the R1 is locked now. */
  | { type: "lock"; set: boolean; locked: boolean }
  | { type: "build"; id: number; stage: "writing" | "checking" | "repairing" | "installing" | "done" | "failed"; section?: string; chars?: number; app?: string; name?: string; error?: string; stopped?: boolean; problems?: string[]; cost_usd?: number; seconds?: number; kind?: "edit"; said?: string; deleted?: string[] }
  | { type: "account"; brain: "claude" | "codex"; state: "waiting" | "code_entered" | "signed_in" | "signed_out" | "failed" | "expired"; detail?: string }
  /** `mail` / `mail_out`: another agent's message landed in the chat, or the agent sent one from it (ADR-0012). */
  | { type: "chat"; agent?: string; chat: string; kind: "user" | "delta" | "tool" | "build" | "done" | "error" | "note" | "mail" | "mail_out" | "card" | "stopped" | "routine" | "quiet"; text?: string; build?: number; app?: string; mail?: MailCard }
  /** `by`: the agent whose message started this turn (not the user): the focus stays put. */
  | { type: "agent_state"; agent?: string; state: AgentLive; chat?: string; by?: string }
  | { type: "agents_changed" }
  /** `count`: the agent's unread replies, after a reply in `chat` or the user seeing it; `from`: another agent's message;
   *  `mail`: the newest message from another agent still waiting (none: absent). */
  | { type: "unread"; agent: string; chat: string; count: number; from?: string; mail?: { from: string; kind: string; text: string } }
  /** One agent messaged another (ADR-0012). `woke`: it runs a turn now. */
  | { type: "mail"; from: string; to: string; kind: "ask" | "tell" | "reply"; text: string; woke: boolean }
  | { type: "open"; app: string }
  /** `intent` on `final`: what System One made of the words (ADR-0032), when it's sure. */
  | { type: "voice"; state: "listening" | "partial" | "final" | "cancelled" | "error"; text?: string; intent?: SystemOneIntent }
  /** System One while you talk: what's coming (null: nothing to get ready for). */
  | { type: "intent"; state: "partial"; intent: SystemOneIntent | null }
  /** A photo or recording was added, changed or removed (ADR-0031). */
  | { type: "media_changed"; kind: string }
  /** The Recorder: about every second while recording, then `saved` (ADR-0033). */
  | { type: "recording"; id: string; state: "recording" | "saved"; seconds: number; text: string }
  | { type: "journal_changed" }
  /** The Recorder's meter, ~10/s while recording and the display is on (ADR-0037). */
  | { type: "recording_level"; level: number }
  /** The Journal's voice notes (ADR-0037). */
  | {
      type: "note";
      state: "listening" | "partial" | "saved" | "cancelled" | "error";
      text?: string;
      entry?: string;
      media?: string;
      ms?: number;
      error?: string;
    }
  /** How ready the Translator is (ADR-0034). */
  | { type: "translator_stage"; stage: "loading" | "english" | "ready" | "off" }
  /** The Translator: a turn's progress while its screen is open (ADR-0034). */
  | {
      type: "translation";
      id: number;
      state: "listening" | "partial" | "hearing" | "heard" | "translated" | "spoken" | "error" | "cancelled";
      heard?: string;
      from?: string;
      text?: string;
      to?: string;
      via?: "device" | "agent";
      speech?: number;
      error?: string;
      ms?: number;
    }
  | { type: "voice_level"; level: number }
  | { type: "volume"; on: boolean }
);

export interface R1State {
  version: string;
  uptime_s: number;
  sim: boolean;
  harness: boolean;
  devices: { wheel: string | null; button: string | null };
  motor: { deg: number | null; moving: boolean; sim: boolean };
  system: {
    battery_pct: number | null;
    charging: boolean | null;
    mem_available_mb: number | null;
    mem_total_mb: number | null;
  };
  stats: { wheel: number; wheel_suppressed: number; buttons: number; gestures: number };
  clients: number;
}

type Listener = (ev: R1Event) => void;
const listeners = new Set<Listener>();

/** Subscribes to every event from r1d. Returns the unsubscribe function. */
export function onEvent(fn: Listener): () => void {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

/** Delivers one event to every listener. */
export function emit(ev: R1Event): void {
  for (const fn of listeners) fn(ev);
}

/**
 * The native shell's bridge (ADR-0017), when this page is a mini-app in one of its WebViews
 * (app.html). The shell owns r1d's socket and passes events in through `window.r1Event`.
 */
export interface NativeBridge {
  /** The page's script is up: it can take events. */
  ready(): void;
  /** The wheel turned up past the app's first control (the shell shows Back, ADR-0014). */
  overscroll(): void;
  /** The page's token for r1d (ADR-0026): it reaches only this app's runtime endpoints. */
  token?(): string;
  /**
   * A component took the wheel (true) or gave it back (false), ADR-0038: while it has it, the shell passes the
   * double click and the button's edges too, and the double click doesn't go back.
   */
  capture?(on: boolean): void;
}

export const native: NativeBridge | undefined = (window as unknown as { R1Native?: NativeBridge }).R1Native;

/**
 * Keeps one WebSocket to r1d open (reconnecting with backoff) and returns the latest `hello`
 * snapshot plus whether the socket is up. Mount once, at the root.
 */
export function useR1Connection(): { state: R1State | null; connected: boolean } {
  const [state, setState] = useState<R1State | null>(null);
  const [connected, setConnected] = useState(false);
  const retry = useRef(500);

  useEffect(() => {
    let ws: WebSocket | null = null;
    let timer: number | undefined;
    let closed = false;
    let wasConnected = false;

    const connect = () => {
      const url = `${location.protocol === "https:" ? "wss" : "ws"}://${location.host}/ws`;
      ws = new WebSocket(url);
      ws.onopen = () => {
        // r1d came back after a restart (a deploy, or a crash): reload, since a deploy may have
        // replaced the shell's own files too.
        if (wasConnected) {
          location.reload();
          return;
        }
        wasConnected = true;
        retry.current = 500;
        setConnected(true);
      };
      ws.onmessage = (m) => {
        const msg = JSON.parse(m.data as string);
        if (msg.type === "hello") {
          setState(msg.state as R1State);
          return;
        }
        const ev = msg as R1Event;
        if (ev.type === "motor") {
          setState((s) => (s ? { ...s, motor: { ...s.motor, deg: ev.deg, moving: ev.moving } } : s));
        }
        emit(ev);
      };
      ws.onclose = () => {
        setConnected(false);
        if (closed) return;
        timer = window.setTimeout(connect, retry.current);
        retry.current = Math.min(retry.current * 2, 5000);
      };
    };
    connect();
    return () => {
      closed = true;
      window.clearTimeout(timer);
      ws?.close();
    };
  }, []);

  return { state, connected };
}

async function post<T = unknown>(path: string, body: unknown): Promise<T> {
  const r = await fetch(path, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  const json = await r.json();
  if (!r.ok) throw new Error(json.error ?? `${path}: HTTP ${r.status}`);
  return json as T;
}

export const api = {
  state: (): Promise<R1State> => fetch("/api/state").then((r) => r.json()),
  motor: (deg: number) => post<{ deg: number }>("/api/motor", { deg }),
  openAndroid: (screen: "settings" | "wifi" | "bluetooth" | "developer" | "about") =>
    post("/api/android/open", { screen }),
  screen: (on: boolean) => post("/api/screen", { on }),
  // Simulator only (r1d --sim --harness): input goes through r1d's gesture engine like hardware.
  simWheel: (delta: number) => post("/harness/wheel", { delta }),
  simButton: (action: "down" | "up" | "tap" | "double" | "hold") => post("/harness/button", { action }),
};
