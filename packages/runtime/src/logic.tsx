// The logic lane (r1os ADR-0041): an app's own code, for what its blocks, SQL and services can't do.
//
// - **Where it runs:** a Web Worker started by a hidden sandboxed iframe, as games run (ADR-0038): an opaque origin
//   whose policy allows nothing but its own scripts, so no network, storage, DOM of the app's, or r1d of its own.
// - **What it gets** (the `r1` object, see PRELUDE): what the app sees (`state`, `settings`, `q`, `services`,
//   `player`, kept current, with a `change` event), `set` to publish values the views bind (`{{logic.x}}`), `save`
//   for the app's declared state, `run` and `query` for its own actions and queries (the same checks as its buttons),
//   `fetch` through r1d (only the hosts its `net.http` lists), `every`/`after` timers that pause while the app is
//   hidden, and `action` to answer the app's `run:` steps.
// - **When it goes wrong:** an error, or a worker that stops answering for 4 s, ends the lane and is reported like any
//   app error (so `fix_app` can repair it); the views keep the values they last had.

import { useLayoutEffect, useRef } from "react";

import type { LogicBridge, Runner } from "./runner";
import type { EneeStore } from "./store";
import { type CompiledApp, EneeError, type Host } from "./types";

/** The frame's page: starts the worker, relays messages both ways, and watches its heartbeat. */
const BOOT = `<!doctype html><meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'unsafe-inline' blob:; worker-src blob:">
<script>
(() => {
  let worker = null, beat = 0, watch = 0;
  const up = (m) => parent.postMessage(m, "*");
  const stopWatch = () => { clearInterval(watch); watch = 0; };
  const startWatch = () => {
    stopWatch();
    beat = Date.now();
    watch = setInterval(() => {
      if (worker && Date.now() - beat > 4000) { worker.terminate(); worker = null; stopWatch(); up({ t: "stuck" }); }
    }, 1000);
  };
  addEventListener("message", (e) => {
    if (e.source !== parent || !e.data || typeof e.data.t !== "string") return;
    const m = e.data;
    if (m.t === "start") {
      if (worker) return;
      try {
        worker = new Worker(URL.createObjectURL(new Blob([m.code], { type: "text/javascript" })));
      } catch (err) {
        up({ t: "error", message: "the app's logic couldn't start: " + err });
        return;
      }
      worker.onmessage = (ev) => {
        const d = ev.data || {};
        if (d.t === "beat") { beat = Date.now(); return; }
        if (d.t === "error") stopWatch();
        up(d);
      };
      worker.onerror = (ev) => { ev.preventDefault(); stopWatch(); up({ t: "error", message: ev.message || "the app's logic failed" }); };
      worker.postMessage({ t: "init", view: m.view });
      startWatch();
      return;
    }
    if (!worker) return;
    if (m.t === "pause") stopWatch();
    if (m.t === "resume") startWatch();
    worker.postMessage(m);
  });
})();
</script>`;

/** The script's side: the `r1` object, then the app's script as a function of it. Documented in the builder's prompt. */
const PRELUDE = `"use strict";
const r1 = (() => {
  const handlers = Object.create(null);
  const actions = Object.create(null);
  const pending = new Map();
  const timers = new Set();
  let calls = 0, failed = false, paused = false, beats = 0;
  let view = { state: {}, settings: {}, q: {}, services: {}, player: {} };
  const fail = (e) => {
    if (failed) return;
    failed = true; clearInterval(beats);
    for (const t of timers) clearTimeout(t.handle);
    postMessage({ t: "error", message: String((e && e.stack) || e) });
  };
  const guard = (fn) => (...a) => { try { const r = fn(...a); if (r && typeof r.catch === "function") r.catch(fail); } catch (e) { fail(e); } };
  const fire = (name, arg) => { for (const h of handlers[name] || []) guard(h)(arg); };
  const request = (t, payload) => {
    const id = ++calls;
    postMessage(Object.assign({ t, id }, payload));
    return new Promise((resolve, reject) => pending.set(id, { resolve, reject }));
  };
  const arm = (t) => {
    if (paused || failed) return;
    t.handle = t.every
      ? setInterval(guard(t.fn), t.ms)
      : setTimeout(() => { timers.delete(t); guard(t.fn)(); }, t.ms);
  };
  const beating = (on) => { clearInterval(beats); if (on) beats = setInterval(() => postMessage({ t: "beat" }), 1000); };
  const api = {
    get state() { return view.state; },
    get settings() { return view.settings; },
    get q() { return view.q; },
    get services() { return view.services; },
    get player() { return view.player; },
    on(name, fn) { (handlers[name] || (handlers[name] = [])).push(fn); },
    set(name, value) { postMessage({ t: "set", name: String(name), value: value === undefined ? null : value }); },
    save(key, value) { view.state = Object.assign({}, view.state, { [key]: value }); postMessage({ t: "save", key: String(key), value }); },
    run(name, params) { return request("run", { name: String(name), params: params || {} }); },
    query(name, params) { return request("query", { name: String(name), params: params || {} }); },
    fetch(url, opts) { return request("fetch", { url: String(url), as: (opts && opts.as) || "json" }); },
    action(name, fn) { actions[name] = fn; },
    every(ms, fn) { const t = { every: true, ms: Math.max(250, ms | 0), fn }; timers.add(t); arm(t); return t; },
    after(ms, fn) { const t = { every: false, ms: Math.max(0, ms | 0), fn }; timers.add(t); arm(t); return t; },
    cancel(t) { if (t) { clearTimeout(t.handle); clearInterval(t.handle); timers.delete(t); } },
  };
  onmessage = async (e) => {
    const m = e.data || {};
    switch (m.t) {
      case "init":
        view = m.view || view;
        beating(true);
        try { logic(api); } catch (err) { fail(err); return; }
        fire("start");
        fire("change");
        break;
      case "change": view = m.view || view; fire("change"); break;
      case "pause":
        paused = true; beating(false);
        for (const t of timers) { clearTimeout(t.handle); clearInterval(t.handle); }
        break;
      case "resume":
        paused = false; beating(true);
        for (const t of timers) arm(t);
        break;
      case "call": {
        const fn = actions[m.name];
        try {
          if (!fn) throw new Error("the logic has no action " + m.name + " (r1.action)");
          await fn(m.params || {});
          postMessage({ t: "done", id: m.id });
        } catch (err) {
          postMessage({ t: "done", id: m.id, error: String((err && err.message) || err) });
        }
        break;
      }
      case "reply": {
        const p = pending.get(m.id);
        if (!p) break;
        pending.delete(m.id);
        if (m.ok) p.resolve(m.value); else p.reject(new Error(m.error || "it failed"));
        break;
      }
    }
  };
  return api;
})();
`;

/** The most a published value or a saved state key may be (JSON), and requests a second. */
const VALUE_MAX = 64 * 1024;
const REQUESTS_PER_SECOND = 10;
/** Changes reach the script this long after the app's data stops changing. */
const CHANGE_DEBOUNCE_MS = 80;

/** What the script sees: the app's data, without its own published values (which would loop). */
function viewOf(store: EneeStore): Record<string, unknown> {
  const s = store.getSnapshot() as Record<string, unknown>;
  return { state: s.state ?? {}, settings: s.settings ?? {}, q: s.q ?? {}, services: s.services ?? {}, player: s.player ?? {} };
}

interface LaneProps {
  app: CompiledApp;
  host: Host;
  store: EneeStore;
  runner: Runner;
  bridge: LogicBridge;
  report: (error: EneeError) => void;
}

/** Runs the app's logic for as long as the app is shown. Renders a hidden frame. */
export function LogicLane({ app, host, store, runner, bridge, report }: LaneProps) {
  const frame = useRef<HTMLIFrameElement>(null);
  const logic = app.logic;

  // Before the first paint, so the app's `run:` steps find the lane from its first click.
  useLayoutEffect(() => {
    const el = frame.current;
    if (!logic || !el) return;
    const post = (m: unknown) => el.contentWindow?.postMessage(m, "*");
    let dead = false;
    const fail = (message: string) => {
      if (dead) return;
      dead = true;
      report(new EneeError("E_LOGIC", `The app's logic: ${message}`));
    };
    const calls = new Map<number, { resolve: () => void; reject: (e: Error) => void }>();
    let callIds = 0;
    // Until the frame has loaded and started the worker, a call posted would be lost: it waits.
    let started = false;
    const waiting: unknown[] = [];
    bridge.call = (name, params) => {
      if (dead) return Promise.reject(new EneeError("E_LOGIC", "The app's logic isn't running"));
      const id = ++callIds;
      const m = { t: "call", id, name, params };
      if (started) post(m);
      else waiting.push(m);
      return new Promise((resolve, reject) => calls.set(id, { resolve, reject }));
    };
    // Requests from the script, at most REQUESTS_PER_SECOND.
    let window0 = Date.now();
    let inWindow = 0;
    const allowed = () => {
      const now = Date.now();
      if (now - window0 > 1000) {
        window0 = now;
        inWindow = 0;
      }
      return ++inWindow <= REQUESTS_PER_SECOND;
    };
    const reply = (id: number, p: Promise<unknown>) =>
      p.then(
        (value) => post({ t: "reply", id, ok: true, value: value === undefined ? null : value }),
        (err: unknown) => post({ t: "reply", id, ok: false, error: err instanceof Error ? err.message : String(err) }),
      );
    const size = (v: unknown) => {
      try {
        return JSON.stringify(v ?? null).length;
      } catch {
        return Infinity;
      }
    };

    const onMessage = (e: MessageEvent) => {
      if (e.source !== el.contentWindow || !e.data || typeof e.data.t !== "string") return;
      const m = e.data as Record<string, unknown> & { t: string; id?: number };
      switch (m.t) {
        case "set": {
          const name = String(m.name);
          if (!logic.values.includes(name)) return fail(`it published \`${name}\`, which isn't in logic.values`);
          if (size(m.value) > VALUE_MAX) return fail(`\`${name}\` is over ${VALUE_MAX / 1024} KB`);
          store.hostUpdate({ [`/logic/${name}`]: m.value });
          return;
        }
        case "save": {
          const key = String(m.key);
          if (!Object.prototype.hasOwnProperty.call(app.state, key)) return fail(`it saved \`${key}\`, which isn't a state key`);
          if (size(m.value) > VALUE_MAX) return fail(`state \`${key}\` is over ${VALUE_MAX / 1024} KB`);
          store.update({ [`/state/${key}`]: m.value });
          return;
        }
        case "run": {
          const name = String(m.name);
          const p = !allowed()
            ? Promise.reject(new Error("too many requests"))
            : !Object.prototype.hasOwnProperty.call(app.actions, name)
              ? Promise.reject(new Error(`the app has no action ${name}`))
              : runner.run(name, m.params).then((out) => {
                  if (!out.ok) throw new Error("cancelled" in out ? "cancelled" : out.error.message);
                });
          void reply(m.id ?? 0, p);
          return;
        }
        case "query": {
          const def = app.queries.find((q) => q.name === m.name);
          const p = !allowed()
            ? Promise.reject(new Error("too many requests"))
            : def
              ? host.query(def, (m.params as Record<string, unknown>) ?? {})
              : Promise.reject(new Error(`the app has no query ${String(m.name)}`));
          void reply(m.id ?? 0, p);
          return;
        }
        case "fetch": {
          const as = m.as === "text" || m.as === "feed" ? m.as : "json";
          const p = !allowed()
            ? Promise.reject(new Error("too many requests"))
            : host.fetch
              ? host.fetch(String(m.url), as)
              : Promise.reject(new Error("the web isn't available here"));
          void reply(m.id ?? 0, p);
          return;
        }
        case "done": {
          const c = calls.get(m.id ?? 0);
          if (!c) return;
          calls.delete(m.id ?? 0);
          if (m.error) c.reject(new EneeError("E_LOGIC", String(m.error)));
          else c.resolve();
          return;
        }
        case "error":
          return fail(String(m.message ?? "it failed"));
        case "stuck":
          return fail("it stopped answering (a loop that never ends?)");
      }
    };
    addEventListener("message", onMessage);

    // The script's view of the app, sent again when it changes (its own published values don't count).
    let last = "";
    let timer: ReturnType<typeof setTimeout> | undefined;
    const send = (t: "start" | "change") => {
      const view = viewOf(store);
      const json = JSON.stringify(view);
      if (t === "change" && json === last) return;
      last = json;
      post(t === "start" ? { t, code: `${PRELUDE}\nfunction logic(r1) {\n${logic.script}\n}\n`, view } : { t, view });
    };
    const unsubscribe = store.subscribe(() => {
      clearTimeout(timer);
      timer = setTimeout(() => send("change"), CHANGE_DEBOUNCE_MS);
    });
    const onLoad = () => {
      send("start");
      started = true;
      for (const m of waiting.splice(0)) post(m);
    };
    el.addEventListener("load", onLoad);
    // Hidden (the app left the screen, the display went off): timers and the watchdog pause.
    const onVisibility = () => post({ t: document.hidden ? "pause" : "resume" });
    document.addEventListener("visibilitychange", onVisibility);

    return () => {
      dead = true;
      bridge.call = undefined;
      clearTimeout(timer);
      unsubscribe();
      removeEventListener("message", onMessage);
      el.removeEventListener("load", onLoad);
      document.removeEventListener("visibilitychange", onVisibility);
      for (const c of calls.values()) c.reject(new EneeError("E_LOGIC", "The app's logic stopped"));
    };
  }, [logic, app, host, store, runner, bridge, report]);

  if (!logic) return null;
  return (
    <iframe
      ref={frame}
      title="logic"
      sandbox="allow-scripts"
      srcDoc={BOOT}
      aria-hidden="true"
      tabIndex={-1}
      style={{ position: "absolute", width: 0, height: 0, border: 0, visibility: "hidden" }}
    />
  );
}
