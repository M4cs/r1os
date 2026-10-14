// <EneeApp>: renders a compiled enee app (docs/specs/compiled-app.md) with json-render 0.21.0.
//
// Provider layout (why it is nested this way):
//
//   JSONUIProvider  store={host-controlled EneeStore}  directives={[$x]}
//   │   (State · Visibility · Validation · ActionProvider#outer · Functions · Directives contexts, plus the
//   │    built-in white ConfirmDialog bound to ActionProvider#outer)
//   └─ ActionProvider#inner  handlers={{ run }}     ← shadows #outer for everything rendered below
//      ├─ Renderer                                  ← elements dispatch on.press → #inner.execute → run
//      └─ ConfirmLayer                              ← themed dialog for our runner + #inner.pendingConfirmation
//
// DirectivesContext is not exported by @json-render/react 0.21.0, so JSONUIProvider is the only way to install `$x`;
// shadowing its ActionProvider is what lets us drop the unstyled dialog without forking the package.

import type { ActionBinding, StateModel } from "@json-render/core";
import { ActionProvider, type ComponentRenderProps, JSONUIProvider, Renderer, useActions } from "@json-render/react";
import {
  Component,
  type ErrorInfo,
  type ReactNode,
  type Ref,
  type RefObject,
  useCallback,
  useEffect,
  useImperativeHandle,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";

import { RuntimeContext, type RuntimeContextValue } from "./context";
import { DIRECTIVES } from "./directive";
import { LogicLane } from "./logic";
import { initialModel, keptStateUpdates, settingsUpdates } from "./model";
import { registry } from "./registry";
import { createRunner, type LogicBridge, type Runner } from "./runner";
import { normalizeView, specErrors } from "./spec";
import { createEneeStore, type EneeStore, isUiWritablePath } from "./store";
import { type CompiledApp, type ConfirmRequest, EneeError, type Host } from "./types";
import { ConfirmLayer, createConfirmController } from "./ui/confirm";
import { createToastStore, Toasts } from "./ui/toasts";
import { valuePaths, type FormatOptions } from "./value";
import "./styles.css";

export interface EneeAppHandle {
  /** Dispatch `run` through the app's ActionProvider: the same path as a Button's on.press. */
  run(name: string, args?: Record<string, unknown>): Promise<void>;
  /** Dispatch any json-render action binding (tests, devtools). */
  dispatch(binding: ActionBinding): Promise<void>;
  get(path: string): unknown;
  getState(): StateModel;
  /** The open confirm dialog, if any. */
  pendingConfirm(): ConfirmRequest | null;
  answerConfirm(ok: boolean): void;
  isReady(): boolean;
}

export interface EneeAppProps {
  compiled: CompiledApp;
  host: Host;
  /** View to show (default: the first). Navigation steps switch views internally. */
  view?: string;
  onError?: (error: EneeError) => void;
  onNavigate?: (view: string) => void;
  /** An input wants the keyboard: the overlay shell should activate its window (Peek → Focused). */
  onRequestFocus?: () => void;
  /** Boot finished: the first view rendered (`ready`) or init failed (`error`). Called once per app/host. */
  onBoot?: (phase: "ready" | "error") => void;
  /** Locale/zone for filters (default: the user's). */
  format?: FormatOptions;
  ref?: Ref<EneeAppHandle>;
}

interface Runtime {
  id: number;
  store: EneeStore;
  runner: Runner;
  /** The logic lane's way in, for `run` steps (ADR-0041). */
  bridge: LogicBridge;
  report: (error: EneeError) => void;
}

let runtimeIds = 0;
/** How long service URL values must settle before the host hears them. */
const SERVICE_VARS_DEBOUNCE_MS = 500;
/** How long `persist` state waits after a change before it's saved. */
const PERSIST_DEBOUNCE_MS = 500;
/** A query over state runs again this long after the state stops changing (a wheel turning, typing). */
const STATE_QUERY_DEBOUNCE_MS = 120;


function asEneeError(err: unknown, code: EneeError["code"]): EneeError {
  if (err instanceof EneeError) return err;
  return new EneeError(code, err instanceof Error ? err.message : String(err), { cause: err });
}

export function EneeApp({ compiled, host, view, onError, onNavigate, onRequestFocus, onBoot, format, ref }: EneeAppProps) {
  const callbacks = useRef({ onError, onNavigate, onRequestFocus, onBoot });
  callbacks.current = { onError, onNavigate, onRequestFocus, onBoot };

  const firstView = compiled.views[0]?.name ?? "";
  const [currentView, setCurrentView] = useState(view ?? firstView);
  const [prevViewProp, setPrevViewProp] = useState(view);
  if (view !== prevViewProp) {
    setPrevViewProp(view);
    if (view) setCurrentView(view);
  }

  const toasts = useMemo(() => createToastStore(), []);
  const confirm = useMemo(() => createConfirmController(), []);
  const fmt = useMemo<FormatOptions>(() => format ?? {}, [format]);

  const runtime = useMemo<Runtime>(() => {
    const report = (error: EneeError) => callbacks.current.onError?.(error);
    const store = createEneeStore(initialModel(compiled), {
      onRejectedWrite: (path) => report(new EneeError("E_STEP", `Blocked a UI write outside /state: ${path}`)),
    });
    const bridge: LogicBridge = {};
    const runner = createRunner({
      app: compiled,
      host,
      store,
      format: fmt,
      logic: bridge,
      onError: report,
      ui: {
        toast: (message, tone) => toasts.push(message, tone),
        confirm: (request) => confirm.request(request),
        navigate: (name) => {
          setCurrentView(name);
          callbacks.current.onNavigate?.(name);
        },
      },
    });
    return { id: ++runtimeIds, store, runner, bridge, report };
  }, [compiled, host, fmt, toasts, confirm]);

  // Boot status is tagged with the runtime it belongs to, so a new runtime (new app/host) never renders before its
  // own init finished.
  const [status, setStatus] = useState<{ id: number; phase: "ready" | "error"; error?: EneeError } | null>(null);
  const current = status?.id === runtime.id ? status : null;
  const phase = current?.phase ?? "loading";
  const phaseRef = useRef(phase);
  phaseRef.current = phase;

  // Committed (after the Renderer of the first view mounted, since effects run child-first).
  useEffect(() => {
    if (phase !== "loading") callbacks.current.onBoot?.(phase);
  }, [phase, runtime]);

  // Boot: host.init (DDL + settings) → all queries → render. Re-runs if the app or host changes.
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const init = await host.init(compiled);
        if (cancelled) return;
        runtime.store.hostUpdate({ ...settingsUpdates(compiled, init.settings), ...keptStateUpdates(compiled, init.state) });
        await runtime.runner.refresh();
        if (!cancelled) setStatus({ id: runtime.id, phase: "ready" });
      } catch (err) {
        if (cancelled) return;
        const error = asEneeError(err, "E_INIT");
        runtime.report(error);
        setStatus({ id: runtime.id, phase: "error", error });
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [runtime, compiled, host]);

  // Host-originated patches (contract §2: /q, /settings, /services are host-written; /state is ours).
  useEffect(() => {
    if (phase !== "ready" || !host.subscribe) return;
    return host.subscribe((updates) => {
      const allowed: Record<string, unknown> = {};
      for (const [path, value] of Object.entries(updates)) {
        if (isUiWritablePath(path)) runtime.report(new EneeError("E_HOST", `Host tried to write runtime-owned ${path}`));
        else allowed[path] = value;
      }
      runtime.store.hostUpdate(allowed);
    });
  }, [phase, host, runtime]);

  // The host's own writes (a trigger, the command bar): re-run the queries that read the tables they changed. Only the
  // app knows its state, so the host can't re-run a query bound to a search box's text itself.
  useEffect(() => {
    if (phase !== "ready" || !host.onChanged) return;
    return host.onChanged((tables) => {
      runtime.runner.refreshTables(tables).catch((err: unknown) => runtime.report(asEneeError(err, "E_QUERY")));
    });
  }, [phase, host, runtime]);

  // Queries whose params read state (a converter's `{{state.km}}`, a search box): run again when that state changes,
  // debounced so a quick spin of a wheel or typing runs them once it settles a little (r1os ADR-0038).
  useEffect(() => {
    if (phase !== "ready") return;
    const deps = compiled.queries
      .map((q) => ({ name: q.name, paths: [...valuePaths(q.params ?? {})].filter((p) => p === "/state" || p.startsWith("/state/")) }))
      .filter((q) => q.paths.length > 0);
    if (deps.length === 0) return;
    const read = (paths: string[]) => JSON.stringify(paths.map((p) => runtime.store.get(p) ?? null));
    const last = new Map(deps.map((q) => [q.name, read(q.paths)] as const));
    let timer: ReturnType<typeof setTimeout> | undefined;
    const check = () => {
      timer = undefined;
      const due = deps.filter((q) => {
        const now = read(q.paths);
        if (now === last.get(q.name)) return false;
        last.set(q.name, now);
        return true;
      });
      if (due.length) runtime.runner.refresh(due.map((q) => q.name)).catch((err: unknown) => runtime.report(asEneeError(err, "E_QUERY")));
    };
    const unsubscribe = runtime.store.subscribe(() => {
      clearTimeout(timer);
      timer = setTimeout(check, STATE_QUERY_DEBOUNCE_MS);
    });
    return () => {
      clearTimeout(timer);
      unsubscribe();
    };
  }, [phase, runtime, compiled]);

  // `persist` state keys: saved for the next start when they change, debounced like service values.
  useEffect(() => {
    const keys = compiled.persist ?? [];
    const save = host.saveState?.bind(host);
    if (phase !== "ready" || !save || keys.length === 0) return;
    const snapshot = () => Object.fromEntries(keys.map((k) => [k, runtime.store.get(`/state/${k}`) ?? null]));
    let last = JSON.stringify(snapshot());
    let timer: ReturnType<typeof setTimeout> | undefined;
    const send = () => {
      timer = undefined;
      const values = snapshot();
      const key = JSON.stringify(values);
      if (key === last) return;
      last = key;
      save(values).catch((err: unknown) => runtime.report(asEneeError(err, "E_HOST")));
    };
    // A change still waiting is saved as the app unloads: an unmount, or the shell dropping the frame (best effort).
    const flush = () => {
      if (timer === undefined) return;
      clearTimeout(timer);
      send();
    };
    const unsubscribe = runtime.store.subscribe(() => {
      clearTimeout(timer);
      timer = setTimeout(send, PERSIST_DEBOUNCE_MS);
    });
    window.addEventListener("pagehide", flush);
    return () => {
      window.removeEventListener("pagehide", flush);
      flush();
      unsubscribe();
    };
  }, [phase, host, runtime, compiled]);

  // Service URLs that bind state or settings (a ticker search): report their values to the host when they change,
  // debounced so typing doesn't fetch on every key.
  useEffect(() => {
    const deps = [...new Set((compiled.services ?? []).flatMap((s) => s.deps ?? []))];
    const report = host.serviceVars?.bind(host);
    if (phase !== "ready" || !report || deps.length === 0) return;
    let last = "";
    const send = () => {
      const values = Object.fromEntries(deps.map((p) => [p, runtime.store.get(p) ?? null]));
      const key = JSON.stringify(values);
      if (key === last) return;
      last = key;
      report(values).catch((err: unknown) => runtime.report(asEneeError(err, "E_HOST")));
    };
    send();
    let timer: ReturnType<typeof setTimeout> | undefined;
    const unsubscribe = runtime.store.subscribe(() => {
      clearTimeout(timer);
      timer = setTimeout(send, SERVICE_VARS_DEBOUNCE_MS);
    });
    return () => {
      clearTimeout(timer);
      unsubscribe();
    };
  }, [phase, host, runtime, compiled]);

  const specs = useMemo(() => new Map(compiled.views.map((v) => [v.name, normalizeView(v.spec)] as const)), [compiled]);
  useEffect(() => {
    for (const [name, spec] of specs) {
      const errors = specErrors(spec);
      if (errors.length) runtime.report(new EneeError("E_SPEC", `View "${name}": ${errors.join("; ")}`));
    }
  }, [specs, runtime]);
  const spec = specs.get(currentView) ?? specs.get(firstView) ?? null;

  const handlers = useMemo(
    () => ({
      run: async (params: Record<string, unknown>) => {
        const name = params?.name;
        if (typeof name !== "string" || name === "") {
          const error = new EneeError("E_ACTION_UNKNOWN", "run: missing action name");
          toasts.push(error.message, "danger");
          runtime.report(error);
          return;
        }
        await runtime.runner.run(name, params.args);
      },
    }),
    [runtime, toasts],
  );

  const context = useMemo<RuntimeContextValue>(
    () => ({
      format: fmt,
      requestFocus: () => callbacks.current.onRequestFocus?.(),
      reportError: (err) => {
        const error = asEneeError(err, "E_STEP");
        toasts.push(error.message, "danger");
        runtime.report(error);
      },
    }),
    [fmt, runtime, toasts],
  );

  const navigate = useCallback((name: string) => setCurrentView(name), []);
  const execute = useRef<((binding: ActionBinding) => Promise<void>) | null>(null);
  useImperativeHandle(
    ref,
    () => ({
      async run(name, args) {
        const exec = execute.current;
        if (!exec) throw new Error("EneeApp is not ready");
        await exec({ action: "run", params: { name, args: args ?? {} } });
      },
      async dispatch(binding) {
        const exec = execute.current;
        if (!exec) throw new Error("EneeApp is not ready");
        await exec(binding);
      },
      get: (path) => runtime.store.get(path),
      getState: () => runtime.store.getSnapshot(),
      pendingConfirm: () => confirm.getSnapshot(),
      answerConfirm: (ok) => confirm.answer(ok),
      isReady: () => phaseRef.current === "ready" && execute.current !== null,
    }),
    [runtime, confirm],
  );

  const theme = compiled.app.theme ?? "dark";
  return (
    <div className="enee-app" data-theme={theme} data-accent={compiled.app.accent ?? "neutral"} data-enee-app={compiled.app.id}>
      {phase === "error" ? <InitFailed error={current?.error ?? null} /> : null}
      {phase === "ready" && spec ? (
        <JSONUIProvider key={runtime.id} registry={registry} store={runtime.store} directives={DIRECTIVES}>
          <ActionProvider handlers={handlers} navigate={navigate}>
            <RuntimeContext.Provider value={context}>
              <ExecuteBridge target={execute} />
              {compiled.views.length > 1 ? (
                <ViewTabs app={compiled} current={currentView} onSelect={setCurrentView} />
              ) : null}
              <div className="enee-viewport">
                <div className="enee-scroll">
                  <ViewBoundary key={currentView} onError={(err) => runtime.report(asEneeError(err, "E_RENDER"))}>
                    <Renderer spec={spec} registry={registry} fallback={UnknownComponent} />
                  </ViewBoundary>
                </div>
              </div>
              <ConfirmLayer controller={confirm} />
              <LogicLane
                app={compiled}
                host={host}
                store={runtime.store}
                runner={runtime.runner}
                bridge={runtime.bridge}
                report={runtime.report}
              />
            </RuntimeContext.Provider>
          </ActionProvider>
        </JSONUIProvider>
      ) : null}
      <Toasts store={toasts} />
    </div>
  );
}

/** Exposes the inner ActionProvider's `execute` to the imperative handle. */
function ExecuteBridge({ target }: { target: RefObject<((binding: ActionBinding) => Promise<void>) | null> }) {
  const { execute } = useActions();
  useLayoutEffect(() => {
    target.current = execute;
    return () => {
      if (target.current === execute) target.current = null;
    };
  }, [execute, target]);
  return null;
}

function ViewTabs({ app, current, onSelect }: { app: CompiledApp; current: string; onSelect: (v: string) => void }) {
  return (
    <div className="enee-tabs" role="tablist">
      {app.views.map((v) => (
        <button
          key={v.name}
          type="button"
          role="tab"
          className="enee-tab"
          aria-selected={v.name === current}
          onClick={() => onSelect(v.name)}
        >
          {v.title ?? v.name}
        </button>
      ))}
    </div>
  );
}

function UnknownComponent({ element }: ComponentRenderProps) {
  return <div className="enee-unknown">Unknown component “{element.type}”</div>;
}

function InitFailed({ error }: { error: EneeError | null }) {
  return (
    <div className="enee-failed" role="alert">
      <div className="enee-failed__title">This app couldn’t start</div>
      <div className="enee-failed__message">{error?.message ?? "Unknown error"}</div>
    </div>
  );
}

class ViewBoundary extends Component<{ children: ReactNode; onError: (error: unknown) => void }, { failed: boolean }> {
  override state = { failed: false };
  static getDerivedStateFromError() {
    return { failed: true };
  }
  override componentDidCatch(error: unknown, _info: ErrorInfo) {
    this.props.onError(error);
  }
  override render() {
    if (this.state.failed) {
      return (
        <div className="enee-failed" role="alert">
          <div className="enee-failed__title">This view crashed</div>
          <div className="enee-failed__message">The error was reported.</div>
        </div>
      );
    }
    return this.props.children;
  }
}
