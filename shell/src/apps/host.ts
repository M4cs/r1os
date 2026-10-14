// The runtime's Host for apps rendered inside the shell (ADR-0006), backed by r1d (docs/specs/apps.md).
//
// Queries go by name and SQL steps by reference ({action, step}): the SQL the runtime hands us is ignored,
// and r1d runs the statement from its own compiled copy of the app, binding only declared parameters.
// Web data comes from r1d too (docs/specs/apps.md §6): it fetches the app's services while the shell has
// the app open, and pushes their values (`app_services`), which become the runtime's `/services/…`. The
// page only reports the values its service URLs bind (`serviceVars`); r1d builds the URLs.
// Links and copying go through r1d (`open_url`, `copy`): it checks the step and the app's permission, and the
// native shell opens the link in its viewer or copies the text. So does audio (`media`, ADR-0040): r1d checks the
// step and `audio.play`, the shell's player plays it, and what it's doing comes back as `app_player` (`/player`). Notifications and launching apps are left out,
// so the runtime reports them as "not available here".

import type { CompiledApp, EneeError, ExecResult, FetchResult, Host, HostInit, MediaOp, QueryDef, Row, StepRef } from "@r1os/runtime";
import { native, onEvent } from "../r1";

async function call<T>(path: string, body?: unknown): Promise<T> {
  // In the native shell's WebView, the page's own token (ADR-0026); the simulator is open.
  const token = native?.token?.();
  const headers: Record<string, string> = {};
  if (body !== undefined) headers["Content-Type"] = "application/json";
  if (token) headers.Authorization = `Bearer ${token}`;
  const r = await fetch(path, {
    method: body === undefined ? "GET" : "POST",
    headers,
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const json = await r.json().catch(() => ({}));
  if (!r.ok) throw new Error((json as { error?: string }).error ?? `${path}: HTTP ${r.status}`);
  return json as T;
}

export function createR1Host(appId: string): Host {
  const base = `/api/apps/${encodeURIComponent(appId)}`;
  return {
    init: (_app: CompiledApp) => call<HostInit>(`${base}/init`, {}),
    query: (def: QueryDef, params: Record<string, unknown>) =>
      call<Row[]>(`${base}/query`, { name: def.name, params }),
    exec: (_sql: string, params: Record<string, unknown>, ref?: StepRef) => {
      if (!ref) return Promise.reject(new Error("r1d runs steps by reference only"));
      return call<ExecResult>(`${base}/step`, { action: ref.action, step: ref.step, params });
    },
    settings: () => call<Record<string, unknown>>(`${base}/settings`),
    saveState: (values: Record<string, unknown>) => call<void>(`${base}/state`, { values }),
    // Live values: what r1d has now (kept from the last open), then each change as it's fetched.
    subscribe: (push: (updates: Record<string, unknown>) => void) => {
      // A value already pushed is newer than the snapshot, should the snapshot arrive after it.
      const pushed = new Set<string>();
      const off = onEvent((ev) => {
        if (ev.type === "app_player" && ev.app === appId) {
          pushed.add("/player");
          push({ "/player": ev.player });
          return;
        }
        if (ev.type !== "app_services" || ev.app !== appId) return;
        Object.keys(ev.values).forEach((k) => pushed.add(k));
        push(ev.values);
      });
      // The player as it is now (a page opened while its station plays).
      call<{ player: Record<string, unknown> }>(`${base}/player`)
        .then((r) => {
          if (!pushed.has("/player")) push({ "/player": r.player });
        })
        .catch(() => {});
      call<{ values: Record<string, unknown> }>(`${base}/services`)
        .then((r) => {
          const fresh = Object.fromEntries(Object.entries(r.values).filter(([k]) => !pushed.has(k)));
          if (Object.keys(fresh).length > 0) push(fresh);
        })
        .catch(() => {});
      return off;
    },
    serviceVars: (values: Record<string, unknown>) => call<void>(`${base}/services/vars`, { values }),
    openUrl: (url: string, ref?: StepRef) => call<void>(`${base}/open_url`, { url, action: ref?.action, step: ref?.step }),
    copy: (text: string, ref?: StepRef) => call<void>(`${base}/copy`, { text, action: ref?.action, step: ref?.step }),
    media: (op: MediaOp, args: { url?: string; title?: string; subtitle?: string }, ref?: StepRef) =>
      call<void>(`${base}/media`, { op, ...args, action: ref?.action, step: ref?.step }),
    // The app's logic reaching the web (ADR-0041): r1d allows only its `net.http` hosts.
    fetch: (url: string, as: "json" | "text" | "feed") => call<FetchResult>(`${base}/fetch`, { url, as }),
    // Writes made elsewhere (another client, a future trigger or voice command) refresh this app's queries.
    onChanged: (listener: (tables: string[]) => void) =>
      onEvent((ev) => {
        // Includes this app's own steps: the runtime already refreshed those, so it's a cheap second refresh.
        if (ev.type === "app_changed" && ev.app === appId) listener(ev.tables);
      }),
  };
}

/** Codes that are the host's trouble (r1d away or restarting), not the app's: not reported for a fix. */
const NOT_THE_APPS = new Set(["E_HOST", "E_INIT"]);
const reported = new Set<string>();

/**
 * Tells r1d what went wrong in the app (its runtime's error), so a fix can repair it (docs/specs/apps.md). Once
 * per distinct problem per page.
 */
export function reportError(appId: string, error: EneeError): void {
  const message = error.message ?? String(error);
  if (NOT_THE_APPS.has(error.code) || /token|fetch|network|HTTP 5\d\d/i.test(message)) return;
  const key = `${appId} ${error.code} ${message}`;
  if (reported.has(key)) return;
  reported.add(key);
  const place = error.action ? `action \`${error.action}\`` : error.element ? `element ${error.element}` : "";
  call<void>(`/api/apps/${encodeURIComponent(appId)}/errors`, { code: error.code, message, place }).catch(() => reported.delete(key));
}

export interface AppSummary {
  id: string;
  name: string;
  icon: string;
  accent?: string;
  description?: string;
}

export interface AppVersion {
  at: number;
  kind: "create" | "edit" | "undo" | "restore";
  request: string;
  said: string;
  deleted: string[];
}

export interface Trashed {
  id: string;
  name: string;
  removed_at: number;
}

const app = (id: string) => `/api/apps/${encodeURIComponent(id)}`;

export const appsApi = {
  list: () => call<{ apps: AppSummary[] }>("/api/apps").then((r) => r.apps),
  get: (id: string) => call<{ app: CompiledApp; source: string }>(app(id)),
  history: (id: string) => call<{ versions: AppVersion[] }>(`${app(id)}/history`).then((r) => r.versions),
  /** Starts an edit build; progress arrives as `build` events with `kind: "edit"`. */
  edit: (id: string, request: string) => call<{ build: number }>(`${app(id)}/edit`, { request }),
  undo: (id: string) =>
    call<{ app: AppSummary; undone: Pick<AppVersion, "at" | "kind" | "request" | "said">; restored: string[] }>(`${app(id)}/undo`, {}),
  remove: async (id: string) => {
    const r = await fetch(app(id), { method: "DELETE" });
    if (!r.ok) throw new Error(((await r.json().catch(() => ({}))) as { error?: string }).error ?? `HTTP ${r.status}`);
  },
  trash: () => call<{ apps: Trashed[]; keep_days: number }>("/api/trash"),
  restore: (id: string) => call<{ app: AppSummary }>(`${app(id)}/restore`, {}),
};
