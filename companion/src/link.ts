// The companion's core, from the window: the link's state (`link` events), pairing, and calls to an R1's allowed API
// (src-tauri/src/link.rs; r1d's crate::link decides what's allowed). In a plain browser (no Tauri) it's a demo R1, so
// the window can be designed and checked without one.

import { useCallback, useEffect, useRef, useState } from "react";
import { demo } from "./demo";

export interface R1Info {
  id: string;
  name: string;
  online: boolean;
  path: "direct" | "relay" | "none" | null;
}

export interface LinkState {
  me: { id: string; name: string };
  r1s: R1Info[];
  pairing: { uri: string; svg: string; left_s: number } | null;
  asking: { r1: string; name: string; code: string } | null;
}

/** An agent, as r1d's `/api/agents` gives it. */
export interface Agent {
  id: string;
  name: string;
  form: string;
  color: string;
  finish: string;
  eyes?: string;
  wear?: string[];
  role: string;
  /** One line the user added, at most 200 characters. */
  custom?: string;
  brain: string;
  personality: string;
  state?: string;
  unread?: number;
  can?: Record<string, "ask" | "allow" | "never">;
  /** Where it works: `r1`, or `computer:<key>` (a linked computer runs its turns). */
  works_on?: string;
  /** On a computer: the folders it may work in besides its own. */
  folders?: string[];
  /** On a computer: the permission mode its new chats start in. */
  mode?: string;
  /** On a computer: which, and whether it's connected now. */
  computer?: { name: string | null; connected: boolean };
}

export const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!inTauri) return demo.invoke(cmd, args) as Promise<T>;
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(cmd, args);
}

async function listen<T>(event: string, fn: (payload: T) => void): Promise<() => void> {
  if (!inTauri) return demo.listen(event, fn as (p: unknown) => void);
  const { listen } = await import("@tauri-apps/api/event");
  return listen<T>(event, (e) => fn(e.payload));
}

export const link = {
  state: () => invoke<LinkState>("link_state"),
  pair: () => invoke<LinkState>("pair_start"),
  stop: () => invoke<void>("pair_stop"),
  answer: (yes: boolean) => invoke<void>("pair_answer", { yes }),
  unlink: (id: string) => invoke<void>("unlink", { id }),
};

/** One of an R1's routes: its JSON, or an error with r1d's words. */
export async function api<T = unknown>(id: string, method: string, path: string, body?: unknown): Promise<T> {
  const r = await invoke<{ status: number; body: unknown }>("r1_api", { id, method, path, body: body ?? null });
  if (r.status >= 400) {
    const b = r.body as { error?: string } | string;
    throw new Error(typeof b === "string" ? b : (b?.error ?? `r1d said ${r.status}`));
  }
  return r.body as T;
}

/** A file from one of r1d's routes (a photo): a URL for it in this window. */
export async function apiFile(id: string, path: string): Promise<string> {
  const r = await invoke<{ status: number; mime?: string; base64?: string; body?: unknown }>("r1_api", { id, method: "GET", path, body: null });
  if (r.status >= 400 || !r.base64) throw new Error(`r1d said ${r.status}`);
  const bytes = Uint8Array.from(atob(r.base64), (c) => c.charCodeAt(0));
  return URL.createObjectURL(new Blob([bytes], { type: r.mime || "application/octet-stream" }));
}

/** The link's state, live. */
export function useLink(): LinkState | null {
  const [s, setS] = useState<LinkState | null>(null);
  useEffect(() => {
    let off: (() => void) | undefined;
    let gone = false;
    link.state().then((v) => !gone && setS(v));
    listen<LinkState>("link", (v) => setS(v)).then((f) => (gone ? f() : (off = f)));
    return () => {
      gone = true;
      off?.();
    };
  }, []);
  return s;
}

/** An R1's events (the ones r1d forwards to computers), while `id` is set. */
export function useR1Events(id: string | null, fn: (event: { type: string; [k: string]: unknown }) => void) {
  const ref = useRef(fn);
  ref.current = fn;
  useEffect(() => {
    if (!id) return;
    let off: (() => void) | undefined;
    let gone = false;
    listen<{ r1: string; event: { type: string } }>("r1", (p) => p.r1 === id && ref.current(p.event)).then((f) =>
      gone ? f() : (off = f),
    );
    return () => {
      gone = true;
      off?.();
    };
  }, [id]);
}

/** The menu's "link an R1". */
export function onPairRequest(fn: () => void): Promise<() => void> {
  return listen("pair", fn);
}

export async function windowDo(what: "minimize" | "toggleMaximize" | "close") {
  if (!inTauri) return;
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  await getCurrentWindow()[what]();
}

/** Opens a sign-in page in the computer's browser. */
export async function openUrl(url: string) {
  if (!/^https?:\/\//.test(url)) return;
  if (!inTauri) {
    window.open(url, "_blank", "noopener");
    return;
  }
  const { openUrl } = await import("@tauri-apps/plugin-opener");
  await openUrl(url);
}

/** A photo or recording from the R1, as a URL in this window: fetched through the link once. */
const mediaUrls = new Map<string, Promise<string>>();
export function mediaUrl(id: string, media: string): Promise<string> {
  const key = `${id}/${media}`;
  let p = mediaUrls.get(key);
  if (!p) {
    p = apiFile(id, `/api/media/${media}/file`);
    p.catch(() => mediaUrls.delete(key));
    mediaUrls.set(key, p);
  }
  return p;
}

/** Bytes to one of r1d's routes (a photo dropped in): its answer's body. */
export async function upload<T = unknown>(id: string, method: string, path: string, mime: string, base64: string): Promise<T> {
  const r = await invoke<{ status: number; body: unknown }>("r1_api", { id, method, path, body: null, base64, mime });
  if (r.status >= 400) {
    const b = r.body as { error?: string } | string;
    throw new Error(typeof b === "string" ? b : (b?.error ?? `r1d said ${r.status}`));
  }
  return r.body as T;
}

/** A file from one of r1d's routes, as it came: its type and bytes (base64). */
export async function apiBytes(id: string, path: string): Promise<{ mime: string; base64: string }> {
  const r = await invoke<{ status: number; mime?: string; base64?: string }>("r1_api", { id, method: "GET", path, body: null });
  if (r.status >= 400 || !r.base64) throw new Error(`r1d said ${r.status}`);
  return { mime: r.mime ?? "application/octet-stream", base64: r.base64 };
}

/** Saves something in Downloads\r1os (`name` made free there) and shows it in Explorer: where it went. Outside
 *  Tauri, the browser downloads it. */
export async function saveDownload(name: string, what: { base64?: string; text?: string; mime?: string }): Promise<string> {
  if (!inTauri) {
    const bytes = what.base64 ? Uint8Array.from(atob(what.base64), (c) => c.charCodeAt(0)) : new TextEncoder().encode(what.text ?? "");
    const a = document.createElement("a");
    a.href = URL.createObjectURL(new Blob([bytes], { type: what.mime ?? "application/octet-stream" }));
    a.download = name;
    a.click();
    return name;
  }
  const path = await invoke<string>("save_download", { name, base64: what.base64 ?? null, text: what.text ?? null });
  await reveal(path);
  return path;
}

/** Shows a file or folder in Explorer. */
export async function reveal(path: string) {
  if (!inTauri) return;
  const { revealItemInDir } = await import("@tauri-apps/plugin-opener");
  await revealItemInDir(path).catch(() => undefined);
}

/** Backs an R1's data up into Documents\r1os backups\<label>, saying how far it's got. */
export async function backup(id: string, label: string, progress: (done: number, total: number) => void): Promise<{ dir: string; files: number; bytes: number }> {
  const off = await listen<{ r1: string; done: number; total: number }>("backup", (p) => p.r1 === id && progress(p.done, p.total));
  try {
    return await invoke("backup_r1", { id, label });
  } finally {
    off();
  }
}

/** One of an R1's routes, read and kept up to date: again whenever one of `events` comes from the R1. */
export function useGet<T>(id: string, path: string | null, events: string[] = []): { data: T | null; error: string | null; reload: () => void; set: (t: T) => void } {
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<string | null>(null);
  const reload = useCallback(() => {
    if (!path) return;
    api<T>(id, "GET", path)
      .then((d) => {
        setData(d);
        setError(null);
      })
      .catch((e) => setError(e instanceof Error ? e.message : String(e)));
  }, [id, path]);
  useEffect(reload, [reload]);
  useR1Events(id, (e) => events.includes(e.type) && reload());
  return { data, error, reload, set: setData };
}

/** The companion's settings: how long an R1's link stays once nothing's going on (seconds; 0: only while its screen
 *  is on; ADR-0049). */
export interface Settings {
  idle_s: number;
}
export const settings = {
  get: () => invoke<Settings>("settings_get"),
  set: (s: Settings) => invoke<void>("settings_set", { settings: s }),
};

/** Agents working on this computer (src-tauri/src/runner.rs): the CLIs here, turns running and the last ones. */
export interface PcStatus {
  claude: { path: string | null; version: string | null; signed_in: boolean };
  codex: { path: string | null; signed_in: boolean; account?: string | null; error?: string };
  workspaces: string;
  running: { r1: string; agent: string; chat: string; started: number; tools: string[] }[];
  log: { r1: string; agent: string; chat: string; at: number; ms: number; tools: string[]; ok: boolean; error?: string | null; quiet?: boolean }[];
}
export function usePcStatus(): PcStatus | null {
  const [s, setS] = useState<PcStatus | null>(null);
  useEffect(() => {
    let gone = false;
    let off: (() => void) | undefined;
    const load = () => invoke<PcStatus>("pc_status").then((v) => !gone && setS(v)).catch(() => undefined);
    load();
    listen("runner", load).then((f) => (gone ? f() : (off = f)));
    return () => {
      gone = true;
      off?.();
    };
  }, []);
  return s;
}

/** Signs Claude Code in on this computer: its own page opens in the browser; this resolves when it's done. */
export function claudeSignIn(): Promise<void> {
  return invoke<void>("claude_sign_in");
}

/** Signs the companion's own Codex in: Codex's page opens in the browser; this resolves when it's done. */
export function codexSignIn(): Promise<void> {
  return invoke<void>("codex_sign_in");
}

/** Picks a folder on this computer (for an agent working here). */
export async function pickFolder(): Promise<string | null> {
  if (!inTauri) return "C:\\Users\\you\\Projects\\garden";
  const { open } = await import("@tauri-apps/plugin-dialog");
  const picked = await open({ directory: true, multiple: false, title: "a folder for your agent" });
  return typeof picked === "string" ? picked : null;
}

/** Starting with Windows (minimised to the tray): the companion's own setting. */
export const autostart = {
  async get(): Promise<boolean> {
    if (!inTauri) return false;
    const { isEnabled } = await import("@tauri-apps/plugin-autostart");
    return isEnabled();
  },
  async set(on: boolean) {
    if (!inTauri) return;
    const { enable, disable } = await import("@tauri-apps/plugin-autostart");
    await (on ? enable() : disable());
  },
};
