// A small game an agent wrote (r1os ADR-0038): the one place an app runs code of its own, sandboxed.
//
// - **Where it runs:** a Web Worker started by a sandboxed iframe (`sandbox="allow-scripts"`, an opaque origin) whose
//   policy allows nothing but its own scripts and the worker: no network, no storage, no DOM of the app's, no r1d.
//   It draws on an OffscreenCanvas the frame hands it.
// - **What it gets** (the `r1` object, see PRELUDE): the canvas and its size, the app's colours, the wheel, the click
//   and the button's edges while you play, a frame loop that runs only while you play, `save`/`load` of the app's
//   own declared state keys, and `action` for the actions the block lists.
// - **Taking part:** the block is a focus stop. A click takes the wheel and starts (or goes on with) the game; a
//   double click pauses it and gives the wheel back. The game pauses too when the page is hidden.
// - **When it goes wrong:** an error, or a worker that stops answering for 4 s (a loop that never ends), ends the game
//   and reports it like any app error, so the agent can fix it.

import { useActions, useStateStore } from "@json-render/react";
import { useEffect, useRef, useState } from "react";

import { useRuntime } from "../context";
import { eneeId } from "../spec";
import { ENGAGED_ATTR, useWheelInput, WHEEL_ATTR } from "../wheel-input";
import type { Ctx } from "./types";

/** The frame's page: relays between the app and the worker, and watches the worker's heartbeat. */
const BOOT = `<!doctype html><meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'unsafe-inline' blob:; worker-src blob:; style-src 'unsafe-inline'">
<style>html,body{margin:0;height:100%;overflow:hidden;background:transparent}canvas{display:block;width:100%;height:100%}</style>
<canvas></canvas>
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
      const canvas = document.querySelector("canvas").transferControlToOffscreen();
      canvas.width = Math.max(1, Math.round(m.w * m.dpr));
      canvas.height = Math.max(1, Math.round(m.h * m.dpr));
      try {
        worker = new Worker(URL.createObjectURL(new Blob([m.code], { type: "text/javascript" })));
      } catch (err) {
        up({ t: "error", message: "the game couldn't start: " + err });
        return;
      }
      worker.onmessage = (ev) => {
        const d = ev.data || {};
        if (d.t === "beat") { beat = Date.now(); return; }
        // Over (or failed): it stops beating, and that's no hang.
        if (d.t === "end" || d.t === "error") stopWatch();
        up(d);
      };
      worker.onerror = (ev) => { ev.preventDefault(); stopWatch(); up({ t: "error", message: ev.message || "the game failed" }); };
      worker.postMessage({ t: "init", canvas, w: m.w, h: m.h, dpr: m.dpr, theme: m.theme, saved: m.saved }, [canvas]);
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

/**
 * The game's side: the \`r1\` object, then the agent's script as a function of it, run once the canvas arrives.
 * Kept short and plain: it's part of what the builder's prompt documents (prompts/builder.md, "Games").
 */
const PRELUDE = `"use strict";
const r1 = (() => {
  const handlers = Object.create(null);
  const pending = new Map();
  let playing = false, frame = 0, loopFn = null, last = 0, beats = 0, calls = 0, saved = {}, failed = false;
  const raf = typeof requestAnimationFrame === "function"
    ? (f) => requestAnimationFrame(f)
    : (f) => setTimeout(() => f(performance.now()), 16);
  const fail = (e) => {
    if (failed) return;
    failed = true; playing = false; clearInterval(beats);
    postMessage({ t: "error", message: String((e && e.stack) || e) });
  };
  const fire = (name, arg) => { for (const h of handlers[name] || []) { try { h(arg); } catch (e) { fail(e); } } };
  const schedule = () => { if (playing && loopFn && !frame) frame = raf(tick); };
  const tick = (t) => {
    frame = 0;
    if (!playing || !loopFn) return;
    const gap = 1000 / Math.max(1, Math.min(60, api.fps)) - 2;
    if (last && t - last < gap) { schedule(); return; }
    const dt = last ? Math.min(0.1, (t - last) / 1000) : 0;
    last = t;
    try { loopFn(dt); } catch (e) { fail(e); return; }
    schedule();
  };
  const play = (on) => {
    if (failed || on === playing) return;
    playing = on;
    clearInterval(beats);
    if (on) { last = 0; beats = setInterval(() => postMessage({ t: "beat" }), 1000); schedule(); fire("resume"); }
    else fire("pause");
  };
  const api = {
    width: 0, height: 0, scale: 1, canvas: null, ctx: null, theme: {}, fps: 30,
    on(name, fn) { (handlers[name] || (handlers[name] = [])).push(fn); },
    loop(fn) { loopFn = fn; schedule(); },
    save(key, value) { saved[key] = value; postMessage({ t: "save", key: String(key), value }); },
    load(key) { return saved[key]; },
    action(name, args) {
      const id = ++calls;
      postMessage({ t: "action", id, name: String(name), args: args || {} });
      return new Promise((resolve, reject) => pending.set(id, { resolve, reject }));
    },
    end(result) { play(false); postMessage({ t: "end", result: result === undefined ? null : result }); },
  };
  onmessage = (e) => {
    const m = e.data || {};
    switch (m.t) {
      case "init": {
        api.canvas = m.canvas; api.ctx = m.canvas.getContext("2d");
        api.width = m.w; api.height = m.h; api.scale = m.dpr; api.theme = m.theme || {}; saved = m.saved || {};
        api.ctx.scale(m.dpr, m.dpr);
        try { game(api); } catch (err) { fail(err); return; }
        play(true);
        fire("start");
        break;
      }
      case "wheel": fire("wheel", m.d); break;
      case "press": fire("press"); break;
      case "down": fire("down"); break;
      case "up": fire("up"); break;
      case "pause": play(false); break;
      case "resume": play(true); break;
      case "reply": {
        const p = pending.get(m.id);
        if (!p) break;
        pending.delete(m.id);
        if (m.ok) p.resolve(m.value); else p.reject(new Error(m.error || "the action failed"));
        break;
      }
    }
  };
  return api;
})();
`;

/** The most a game may save under one key (JSON), and actions it may run a second. */
const SAVE_MAX = 4096;
const ACTIONS_PER_SECOND = 4;

type Phase = "ready" | "playing" | "paused" | "over" | "failed";

function themeOf(el: HTMLElement): Record<string, string> {
  const cs = getComputedStyle(el);
  const v = (name: string, fallback: string) => cs.getPropertyValue(name).trim() || fallback;
  return {
    bg: v("--enee-surface", "#121418"),
    fg: v("--enee-text", "#f2f2f2"),
    muted: v("--enee-muted", "#8a8f98"),
    accent: v("--enee-accent", "#ff8a3d"),
    onAccent: v("--enee-on-accent", "#0b0b0c"),
    danger: v("--enee-danger", "#ff5a5a"),
    good: "#4ade80",
    font: "system-ui, sans-serif",
  };
}

export function Game({ props }: Ctx<"Game">) {
  const { reportError } = useRuntime();
  const { execute } = useActions();
  const store = useStateStore();
  const ref = useRef<HTMLDivElement>(null);
  const frame = useRef<HTMLIFrameElement>(null);
  const [phase, setPhase] = useState<Phase>("ready");
  const [run, setRun] = useState(0);
  const [note, setNote] = useState<string | null>(null);
  const actionTimes = useRef<number[]>([]);
  const allowed = props.actions ?? [];

  const post = (m: Record<string, unknown>) => frame.current?.contentWindow?.postMessage(m, "*");

  // Messages from the frame (and so the worker): only from our frame, and only these.
  useEffect(() => {
    const onMessage = async (e: MessageEvent) => {
      if (!frame.current || e.source !== frame.current.contentWindow) return;
      const m = (e.data ?? {}) as Record<string, unknown>;
      switch (m.t) {
        case "save": {
          const key = typeof m.key === "string" ? m.key : "";
          const json = JSON.stringify(m.value ?? null);
          // Only the app's own declared state, and small.
          if (!/^[a-z_][a-z0-9_]*$/i.test(key) || store.get(`/state/${key}`) === undefined || json.length > SAVE_MAX) {
            reportError(new Error(`${props.label}: r1.save("${key}") isn't one of the app's state keys (or it's over 4 KB)`));
            return;
          }
          store.set(`/state/${key}`, JSON.parse(json));
          return;
        }
        case "action": {
          const name = typeof m.name === "string" ? m.name : "";
          const reply = (ok: boolean, error?: string) => post({ t: "reply", id: m.id, ok, error });
          const t = Date.now();
          actionTimes.current = actionTimes.current.filter((x) => t - x < 1000);
          if (!allowed.includes(name)) return reply(false, `the game may run only ${allowed.join(", ") || "no actions"}`);
          if (actionTimes.current.length >= ACTIONS_PER_SECOND) return reply(false, "too many actions a second");
          actionTimes.current.push(t);
          try {
            await execute({ action: "run", params: { name, args: (m.args ?? {}) as Record<string, unknown> } });
            reply(true);
          } catch (err) {
            reply(false, err instanceof Error ? err.message : String(err));
          }
          return;
        }
        case "end": {
          const r = m.result as Record<string, unknown> | null;
          const text = r && typeof r === "object" ? (typeof r.text === "string" ? r.text : r.score !== undefined ? `score ${String(r.score)}` : null) : typeof r === "string" || typeof r === "number" ? String(r) : null;
          setNote(text);
          setPhase("over");
          return;
        }
        case "error":
        case "stuck": {
          const message = m.t === "stuck" ? "stopped responding (a loop that never ends?)" : String(m.message ?? "failed");
          setNote(m.t === "stuck" ? "it stopped responding" : "it hit an error");
          setPhase("failed");
          reportError(new Error(`${props.label}: ${message}`));
          return;
        }
      }
    };
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  });

  // Hidden (the screen slept, the app was covered): pause.
  useEffect(() => {
    const onVisible = () => {
      if (document.hidden && phase === "playing") {
        post({ t: "pause" });
        setPhase("paused");
      }
    };
    document.addEventListener("visibilitychange", onVisible);
    return () => document.removeEventListener("visibilitychange", onVisible);
  });

  const start = () => {
    const el = ref.current;
    if (!el) return;
    const box = el.getBoundingClientRect();
    const saved: Record<string, unknown> = {};
    const state = (store.get("/state") ?? {}) as Record<string, unknown>;
    for (const [k, v] of Object.entries(state)) saved[k] = v;
    post({
      t: "start",
      code: `${PRELUDE}\nfunction game(r1) {\n"use strict";\n${props.script}\n}\n`,
      w: Math.round(box.width),
      h: Math.round(box.height),
      dpr: window.devicePixelRatio || 1,
      theme: themeOf(el),
      saved,
    });
  };

  const play = () => {
    setNote(null);
    if (phase === "paused") {
      post({ t: "resume" });
      setPhase("playing");
      return;
    }
    // A new game: a fresh frame (and worker).
    setRun((n) => n + 1);
    setPhase("playing");
  };
  const pause = () => {
    if (phase !== "playing") return;
    post({ t: "pause" });
    setPhase("paused");
  };

  useWheelInput(ref, {
    wheel: (d) => post({ t: "wheel", d }),
    press: () => post({ t: "press" }),
    button: (down) => post({ t: down ? "down" : "up" }),
    cancel: pause,
    release: pause,
  });

  const playing = phase === "playing";
  const height = Math.max(160, Math.min(props.height ?? 360, 640));
  return (
    <div
      ref={ref}
      role="button"
      tabIndex={0}
      className="enee-game"
      style={{ height }}
      aria-label={props.label}
      data-phase={phase}
      {...{ [WHEEL_ATTR]: "" }}
      {...(playing ? { [ENGAGED_ATTR]: "" } : {})}
      onClick={() => (playing ? post({ t: "press" }) : play())}
      {...eneeId(props)}
    >
      {run > 0 && phase !== "failed" ? (
        <iframe
          key={run}
          ref={frame}
          className="enee-game__frame"
          title={props.label}
          sandbox="allow-scripts"
          srcDoc={BOOT}
          onLoad={start}
          tabIndex={-1}
          aria-hidden
        />
      ) : null}
      {!playing ? (
        <div className="enee-game__cover">
          <span className="enee-game__title">{props.label}</span>
          {note ? <span className="enee-game__note">{note}</span> : null}
          <span className="enee-game__hint">
            {phase === "paused" ? "click to go on" : phase === "over" || phase === "failed" ? "click to play again" : "click to play"}
          </span>
        </div>
      ) : null}
    </div>
  );
}
