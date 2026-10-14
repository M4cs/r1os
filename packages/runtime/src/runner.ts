// The action runner (contract §5): `run({ name, args })` coerces args, asks for confirmation, executes the steps in
// order, and keeps `/q` fresh after host writes. Pure TS: the React layer injects `ui` (toasts, confirm, navigate).

import { queryPath, sqlParamNames } from "./names";
import { isUiWritablePath, type EneeStore } from "./store";
import {
  EneeError,
  type ActionDef,
  type CompiledApp,
  type ConfirmRequest,
  type CopyParts,
  type Host,
  type ParamDef,
  type QueryDef,
  type Row,
  type Step,
  type ToastTone,
  type Value,
} from "./types";
import { evalValue, type FormatOptions, readPointer, toText, type ValueScope } from "./value";

export interface RunnerUi {
  toast(message: string, tone?: ToastTone): void;
  /** Resolve `true` to proceed, `false` to cancel. */
  confirm(request: ConfirmRequest): Promise<boolean>;
  navigate(view: string): void;
}

export interface RunnerOptions {
  app: CompiledApp;
  host: Host;
  store: EneeStore;
  ui: RunnerUi;
  onError?: (error: EneeError) => void;
  format?: FormatOptions;
  /** The logic lane (ADR-0041), once it runs: what a `run` step calls. */
  logic?: LogicBridge;
}

/** How the runner reaches the app's script: filled in by the logic lane while it runs. */
export interface LogicBridge {
  call?: (name: string, params: Record<string, unknown>) => Promise<void>;
}

export type RunOutcome = { ok: true } | { ok: false; cancelled: true } | { ok: false; error: EneeError };

export interface Runner {
  /** Run `actions[name]`. Never rejects: failures are toasted, reported to `onError`, and returned. */
  run(name: string, args?: unknown): Promise<RunOutcome>;
  /** Re-run queries (all when `names` is omitted) and patch `/q/<name>` in a single store update. */
  refresh(names?: readonly string[]): Promise<void>;
  /** Re-run every query whose `tables` intersect `tables`. */
  refreshTables(tables: readonly string[]): Promise<void>;
}

// Moved to ./names (no json-render imports, so the shell-side bridge can use them); re-exported for compatibility.
export { queryPath, sqlParamNames };

/** Human label for a param name in error messages: `duration_s` → "Duration s". */
function label(name: string): string {
  const s = name.replace(/_/g, " ");
  return s.charAt(0).toUpperCase() + s.slice(1);
}

function isBlank(v: unknown): boolean {
  return v === undefined || v === null || (typeof v === "string" && v.trim() === "");
}

/** Coerce one argument to its declared type (contract §5 step 1). */
export function coerceArg(name: string, def: ParamDef, raw: unknown, action?: string): unknown {
  const ctx = action === undefined ? {} : { action };
  if (isBlank(raw)) {
    if (def.required) throw new EneeError("E_ARG_MISSING", `${label(name)} is required`, ctx);
    return null;
  }
  const type = def.type.trim().toLowerCase().replace(/[?!]$/, "");
  const asNumber = () => (typeof raw === "string" ? Number(raw.trim()) : raw);
  switch (type) {
    case "int":
    case "integer":
    case "duration": {
      const n = asNumber();
      if (typeof n !== "number" || !Number.isFinite(n) || !Number.isInteger(n)) {
        throw new EneeError("E_ARG_TYPE", `${label(name)} must be a whole number`, ctx);
      }
      return n;
    }
    case "real":
    case "float":
    case "number": {
      const n = asNumber();
      if (typeof n !== "number" || !Number.isFinite(n)) {
        throw new EneeError("E_ARG_TYPE", `${label(name)} must be a number`, ctx);
      }
      return n;
    }
    case "bool":
    case "boolean": {
      if (typeof raw === "boolean") return raw;
      if (raw === 1 || raw === "1" || raw === "true") return true;
      if (raw === 0 || raw === "0" || raw === "false") return false;
      throw new EneeError("E_ARG_TYPE", `${label(name)} must be true or false`, ctx);
    }
    case "text":
    case "string":
    case "datetime":
    case "date":
    case "time": {
      if (raw instanceof Date) return raw.toISOString();
      if (typeof raw === "string" || typeof raw === "number" || typeof raw === "boolean") return String(raw);
      throw new EneeError("E_ARG_TYPE", `${label(name)} must be text`, ctx);
    }
    case "json":
      return raw;
    default: {
      const enumMatch = /^enum\((.*)\)$/.exec(type);
      if (enumMatch) {
        const options = (enumMatch[1] ?? "").split("|").map((o) => o.trim());
        const v = String(raw);
        if (!options.includes(v.toLowerCase()) && !options.includes(v)) {
          throw new EneeError("E_ARG_TYPE", `${label(name)} must be one of ${options.join(", ")}`, ctx);
        }
        return v;
      }
      return raw;
    }
  }
}

/** Coerce all declared params; undeclared args are dropped. */
export function coerceArgs(params: ActionDef["params"], args: unknown, action?: string): Record<string, unknown> {
  if (args !== undefined && args !== null && (typeof args !== "object" || Array.isArray(args))) {
    throw new EneeError("E_ARG_TYPE", "Action arguments must be an object", action === undefined ? {} : { action });
  }
  const input = (args ?? {}) as Record<string, unknown>;
  const out: Record<string, unknown> = {};
  for (const [name, def] of Object.entries(params)) out[name] = coerceArg(name, def, input[name], action);
  return out;
}

function confirmRequest(def: ActionDef): ConfirmRequest {
  const destructive = def.steps.some((s) => "sql" in s && /^\s*(?:WITH\b[\s\S]*?\b)?DELETE\b/i.test(s.sql));
  const title = def.confirm ?? "Are you sure?";
  return destructive
    ? { title, message: "This can't be undone.", confirmLabel: "Delete", cancelLabel: "Cancel", variant: "danger" }
    : { title, confirmLabel: "Confirm", cancelLabel: "Cancel", variant: "default" };
}

function toEneeError(err: unknown, code: EneeError["code"], details: { action?: string; step?: number }): EneeError {
  if (err instanceof EneeError) return err;
  const message = err instanceof Error ? err.message : String(err);
  return new EneeError(code, message, { ...details, cause: err });
}

export function createRunner(options: RunnerOptions): Runner {
  const { app, host, store, ui, onError, format, logic } = options;
  const querySeq = new Map<string, number>();
  const queriesByName = new Map(app.queries.map((q) => [q.name, q] as const));

  const report = (error: EneeError, toast = true) => {
    if (toast) ui.toast(error.message, "danger");
    onError?.(error);
  };

  const stateScope = (): ValueScope => ({ read: (path) => store.get(path), ...(format ? { format } : {}) });

  async function runQuery(def: QueryDef): Promise<[string, Row[], number] | null> {
    const seq = (querySeq.get(def.name) ?? 0) + 1;
    querySeq.set(def.name, seq);
    try {
      const params = evalValue(def.params ?? {}, stateScope()) as Record<string, unknown>;
      const rows = await host.query(def, params);
      return [def.name, Array.isArray(rows) ? rows : [], seq];
    } catch (err) {
      report(toEneeError(err, "E_QUERY", {}), false);
      return null;
    }
  }

  async function refreshDefs(defs: readonly QueryDef[]): Promise<void> {
    if (defs.length === 0) return;
    const results = await Promise.all(defs.map(runQuery));
    const updates: Record<string, unknown> = {};
    for (const r of results) {
      // Drop results overtaken by a newer refresh of the same query (async hosts can reorder).
      if (r && querySeq.get(r[0]) === r[2]) updates[queryPath(r[0])] = r[1];
    }
    store.hostUpdate(updates);
  }

  async function refresh(names?: readonly string[]): Promise<void> {
    if (!names) return refreshDefs(app.queries);
    const defs: QueryDef[] = [];
    for (const name of names) {
      const def = queriesByName.get(name);
      if (!def) throw new EneeError("E_STEP", `Unknown query "${name}"`);
      defs.push(def);
    }
    return refreshDefs(defs);
  }

  async function refreshTables(tables: readonly string[]): Promise<void> {
    if (tables.length === 0) return;
    const changed = new Set(tables.map((t) => t.toLowerCase()));
    if (changed.has("settings") && host.settings) {
      const values = await host.settings();
      const updates: Record<string, unknown> = {};
      for (const key of Object.keys(app.settings)) updates[`/settings/${key}`] = values[key] ?? app.settings[key]?.default;
      store.hostUpdate(updates);
    }
    await refreshDefs(app.queries.filter((q) => q.tables.some((t) => changed.has(t.toLowerCase()))));
  }

  async function execStep(action: string, index: number, step: Step, args: Record<string, unknown>): Promise<void> {
    const scope: ValueScope = {
      read: (path) => {
        if (path === "/params") return args;
        if (path.startsWith("/params/")) return readPointer(args, path.slice("/params".length));
        return store.get(path);
      },
      ...(format ? { format } : {}),
    };
    const ref = { action, step: index };

    if ("sql" in step) {
      const names = step.params ?? sqlParamNames(step.sql);
      const bound: Record<string, unknown> = {};
      for (const name of names) {
        if (Object.prototype.hasOwnProperty.call(args, name)) bound[name] = args[name];
        else if (store.get(`/state/${name}`) !== undefined) bound[name] = store.get(`/state/${name}`);
        else if (store.get(`/settings/${name}`) !== undefined) bound[name] = store.get(`/settings/${name}`);
        else throw new EneeError("E_SQL_PARAM", `No value for SQL parameter :${name}`, ref);
      }
      let result;
      try {
        result = await host.exec(step.sql, bound, ref);
      } catch (err) {
        throw toEneeError(err, "E_HOST", ref);
      }
      await refreshTables(result.changedTables ?? []);
      return;
    }
    if ("toast" in step) {
      const text = toText(evalValue(step.toast, scope));
      if (text) ui.toast(text);
      return;
    }
    if ("set" in step) {
      const updates: Record<string, unknown> = {};
      for (const [path, v] of Object.entries(step.set)) {
        if (!isUiWritablePath(path)) throw new EneeError("E_STEP", `"set" may only write /state paths (got ${path})`, ref);
        updates[path] = evalValue(v, scope);
      }
      store.update(updates);
      return;
    }
    if ("navigate" in step) {
      if (!app.views.some((v) => v.name === step.navigate)) {
        throw new EneeError("E_STEP", `Unknown view "${step.navigate}"`, ref);
      }
      ui.navigate(step.navigate);
      return;
    }
    if ("notify" in step) {
      if (!host.notify) throw new EneeError("E_STEP_UNSUPPORTED", "Notifications are not available here", ref);
      const title = toText(evalValue(step.notify.title, scope));
      const body = step.notify.body === undefined ? undefined : toText(evalValue(step.notify.body, scope));
      await host.notify(title, body, ref);
      return;
    }
    if ("refresh" in step) {
      if (!queriesByName.has(step.refresh)) throw new EneeError("E_STEP", `Unknown query "${step.refresh}"`, ref);
      await refresh([step.refresh]);
      return;
    }
    if ("open_url" in step) {
      if (!host.openUrl) throw new EneeError("E_STEP_UNSUPPORTED", "Opening links is not available here", ref);
      await host.openUrl(toText(evalValue(step.open_url, scope)), ref);
      return;
    }
    if ("copy" in step) {
      if (!host.copy) throw new EneeError("E_STEP_UNSUPPORTED", "Copying is not available here", ref);
      await host.copy(toText(evalValue(step.copy, scope)), ref);
      return;
    }
    if ("copy_parts" in step) {
      if (!host.copyParts) throw new EneeError("E_STEP_UNSUPPORTED", "Copying is not available here", ref);
      const parts: CopyParts = {};
      for (const part of ["text", "image", "files"] as const) {
        const value = step.copy_parts[part];
        if (value !== undefined) parts[part] = toText(evalValue(value, scope));
      }
      await host.copyParts(parts, ref);
      return;
    }
    if ("run" in step) {
      if (!logic?.call) throw new EneeError("E_STEP_UNSUPPORTED", "The app's logic isn't running", ref);
      await logic.call(step.run, args);
      return;
    }
    if ("play" in step) {
      if (!host.media) throw new EneeError("E_STEP_UNSUPPORTED", "Playing audio is not available here", ref);
      const text = (v: Value | undefined) => (v === undefined ? undefined : toText(evalValue(v, scope)));
      await host.media("play", { url: text(step.play.url), title: text(step.play.title), subtitle: text(step.play.subtitle) }, ref);
      return;
    }
    for (const op of ["pause", "resume", "stop", "toggle"] as const) {
      if (op in step) {
        if (!host.media) throw new EneeError("E_STEP_UNSUPPORTED", "Playing audio is not available here", ref);
        await host.media(op, {}, ref);
        return;
      }
    }
    if ("launch" in step) {
      if (!host.launch) throw new EneeError("E_STEP_UNSUPPORTED", "Starting apps is not available here", ref);
      await host.launch(toText(evalValue(step.launch, scope)), ref);
      return;
    }
    throw new EneeError("E_STEP_UNSUPPORTED", `Unknown step: ${Object.keys(step as object).join(", ")}`, ref);
  }

  async function run(name: string, rawArgs?: unknown): Promise<RunOutcome> {
    try {
      const def = Object.prototype.hasOwnProperty.call(app.actions, name) ? app.actions[name] : undefined;
      if (!def) throw new EneeError("E_ACTION_UNKNOWN", `Unknown action "${name}"`, { action: name });
      const args = coerceArgs(def.params ?? {}, rawArgs, name);
      if (def.confirm !== undefined && !(await ui.confirm(confirmRequest(def)))) {
        return { ok: false, cancelled: true };
      }
      for (let i = 0; i < def.steps.length; i++) {
        try {
          await execStep(name, i, def.steps[i] as Step, args);
        } catch (err) {
          throw toEneeError(err, "E_STEP", { action: name, step: i });
        }
      }
      return { ok: true };
    } catch (err) {
      const error = toEneeError(err, "E_STEP", { action: name });
      report(error);
      return { ok: false, error };
    }
  }

  return { run, refresh, refreshTables };
}
