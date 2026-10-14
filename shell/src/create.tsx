// "New app": describe an app, and the agent in the R1's box builds it (r1d /api/create; docs/specs/apps.md §4).
// Hold the button and describe it (push-to-talk, voice.tsx), or pick an example with the wheel.

import { useEffect, useRef, useState } from "react";
import { WheelFocus } from "./apps/focus";
import { useAppInput } from "./input";
import { onEvent } from "./r1";

const EXAMPLES = [
  "Count the coffees I drink each day, with a daily limit",
  "A pushup tracker: log sets and show this week's total",
  "Track my mood from 1 to 5 each day with an optional note",
  "A tally counter I can bump up or down and reset",
];

type BuildState =
  | { stage: "idle" }
  | { stage: "starting" }
  | { stage: "writing" | "repairing" | "installing"; section?: string; chars?: number; problems?: string[] }
  | { stage: "done"; app: string; cost?: number; seconds?: number }
  | { stage: "failed"; error: string; problems?: string[] };

interface BuildEvent {
  type: "build";
  id: number;
  stage: "writing" | "repairing" | "installing" | "done" | "failed";
  section?: string;
  chars?: number;
  app?: string;
  error?: string;
  problems?: string[];
  cost_usd?: number;
  seconds?: number;
}

export function CreateApp({ onOpen }: { onOpen: (appId: string) => void }) {
  const [brief, setBrief] = useState("");
  const [build, setBuild] = useState<BuildState>({ stage: "idle" });
  const buildId = useRef<number | null>(null);
  const box = useRef<HTMLDivElement>(null);
  const focus = useRef<WheelFocus | null>(null);

  useEffect(() => {
    if (box.current) focus.current = new WheelFocus(box.current);
  }, []);

  useEffect(
    () =>
      onEvent((raw) => {
        const ev = raw as unknown as BuildEvent;
        if (ev.type !== "build" || ev.id !== buildId.current) return;
        if (ev.stage === "done" && ev.app) {
          setBuild({ stage: "done", app: ev.app, cost: ev.cost_usd, seconds: ev.seconds });
          window.setTimeout(() => onOpen(ev.app!), 1200);
        } else if (ev.stage === "failed") {
          setBuild({ stage: "failed", error: ev.error ?? "failed", problems: ev.problems });
        } else {
          setBuild((b) => ({
            stage: ev.stage as "writing" | "repairing" | "installing",
            section: ev.section ?? ("section" in b ? b.section : undefined),
            chars: ev.chars ?? ("chars" in b ? b.chars : undefined),
            problems: ev.problems ?? ("problems" in b ? b.problems : undefined),
          }));
        }
      }),
    [onOpen],
  );

  const start = async (text: string) => {
    const b = text.trim();
    if (!b || (build.stage !== "idle" && build.stage !== "failed")) return;
    setBrief(b);
    setBuild({ stage: "starting" });
    try {
      const r = await fetch("/api/create", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ brief: b }),
      });
      const json = await r.json();
      if (!r.ok) throw new Error(json.error ?? `HTTP ${r.status}`);
      buildId.current = json.build;
    } catch (e) {
      setBuild({ stage: "failed", error: String(e) });
    }
  };

  useAppInput((ev) => {
    if (build.stage !== "idle" && build.stage !== "failed") return ev.type === "wheel" || ev.type === "gesture" && ev.kind === "tap";
    if (ev.type === "voice" && ev.state === "final" && ev.text?.trim()) {
      void start(ev.text);
      return true;
    }
    if (ev.type === "wheel") {
      focus.current?.move(ev.delta);
      return true;
    }
    if (ev.type === "gesture" && ev.kind === "tap") return focus.current?.activate() ?? false;
    return false;
  });

  if (build.stage !== "idle" && build.stage !== "failed") {
    return (
      <div className="center col create-progress">
        <div className="orb small live" />
        <div className="create-brief">“{brief}”</div>
        {build.stage === "done" ? (
          <div className="big-ok">
            Built in {build.seconds?.toFixed(0)} s{build.cost != null ? ` · $${build.cost.toFixed(3)}` : ""}
          </div>
        ) : (
          <>
            <div className="create-stage">
              {build.stage === "starting" && "Starting the agent…"}
              {build.stage === "writing" && `Writing ${"section" in build && build.section ? build.section : "the spec"}…`}
              {build.stage === "repairing" && "Fixing what the compiler found…"}
              {build.stage === "installing" && "Installing…"}
            </div>
            {"chars" in build && build.chars ? <div className="muted small">{build.chars} characters</div> : null}
          </>
        )}
      </div>
    );
  }

  return (
    <div className="col fill create" ref={box}>
      <div className="create-talk">
        <span className="talk-dot" />
        Hold the button and describe your app
      </div>
      {build.stage === "failed" && (
        <div className="error small">
          {build.error}
          {build.problems?.slice(0, 3).map((p) => (
            <div key={p}>{p}</div>
          ))}
        </div>
      )}
      <div className="muted small">Or pick one:</div>
      <ul className="list examples">
        {EXAMPLES.map((ex) => (
          <li key={ex}>
            <button onClick={() => void start(ex)}>{ex}</button>
          </li>
        ))}
      </ul>
    </div>
  );
}
