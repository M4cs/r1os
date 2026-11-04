// Home: a wheel of your agents (docs/specs/experience.md §2.1, ADR-0011).
//
//   wheel       moves through your agents, then "New agent" at the end
//   tap         the agent's space: its chats, memory and settings (space.tsx); on New agent, make one
//   hold        talk (voice.tsx): to the agent on screen, in its current chat (the one you had with it
//               most lately, ADR-0057) or, with none yet, a new one, which then opens
//   double tap  launcher (App.tsx)
//   touch       an agent opens its space; a neighbour selects it; the corners: Apps · Sleep
//
// Apps and Sleep are two small frosted-glass buttons in the bottom corners (a small backdrop blur
// plus a faint, still frost), over the wheel: the next agent peeks up between them. While an agent
// works, a badge says what it's doing; replies you haven't seen sit on its shoulder as a count, and
// ring its page dot.
//
// The agent on screen is large, with its name and what it's doing; its neighbours peek above and
// below, small and still. Moving animates only transform and opacity, and a fast spin retargets
// mid-flight. Only the agents near the one on screen are drawn, and only that one animates.

import { useEffect, useRef, useState } from "react";
import { Icon } from "@r1os/icons";
import { accountsApi, chatApi, currentChat, type Accounts, type AgentInfo } from "./api";
import { useAppInput } from "./input";
import type { Target } from "./launcher";
import { onEvent } from "./r1";
import { activityOf, Character, colorOf, type Activity, type CharState } from "./ui/Character";
import { noiseUrl } from "./ui/noise";

/** The distance between agents on the wheel, in px. */
const STEP = 214;
/** Agents drawn on each side of the one on screen. */
const WINDOW = 2;
/** The id of the "New agent" card at the end of the wheel. */
export const NEW = "new";

interface Build {
  id: number;
  stage: string;
  section?: string;
}

function charState(a: AgentInfo, build: Build | undefined, connected: boolean): CharState {
  if (!connected) return "sleeping";
  if (build && build.stage !== "done" && build.stage !== "failed") return "working";
  if (build?.stage === "failed") return "error";
  return a.state;
}

function activity(a: AgentInfo, build: Build | undefined): Activity | undefined {
  if (build && build.stage !== "done" && build.stage !== "failed") return "build";
  return activityOf(a.tool);
}

function statusOf(a: AgentInfo, build: Build | undefined, acc: Accounts | null, connected: boolean): { text: string; cls: string } {
  if (!connected) return { text: "Starting up…", cls: "" };
  if (build) {
    switch (build.stage) {
      case "repairing":
        return { text: "Fixing up the app…", cls: "" };
      case "done":
        return { text: "Your app is ready", cls: "" };
      case "failed":
        return { text: "That build didn't work", cls: " error" };
      default:
        return { text: `Building an app${build.section ? ` · ${build.section}` : "…"}`, cls: "" };
    }
  }
  if (acc && !acc.claude && !acc.codex && !acc.hermes) return { text: "Not connected: Settings › Accounts", cls: " error" };
  switch (a.state) {
    case "thinking":
      return { text: "Thinking…", cls: "" };
    case "speaking":
      return { text: "Talking…", cls: "" };
    case "working":
      return { text: "Working on it…", cls: "" };
    case "waiting":
      return { text: "Waiting for room to run…", cls: "" };
    case "error":
      return { text: "Something went wrong", cls: " error" };
    default:
      return { text: "Hold the button to talk", cls: " idle" };
  }
}

export function AgentHome({
  agents,
  max,
  at,
  connected,
  onAt,
  onOpen,
  onLauncher,
  onSleep,
}: {
  agents: AgentInfo[];
  /** The most agents there's room for. */
  max: number;
  /** The id on screen (an agent's, or NEW). */
  at: string;
  connected: boolean;
  onAt: (id: string) => void;
  onOpen: (t: Target) => void;
  onLauncher: () => void;
  onSleep: () => void;
}) {
  const [builds, setBuilds] = useState<Record<string, Build>>({});
  const [acc, setAcc] = useState<Accounts | null>(null);
  const entries = [...agents.map((a) => a.id), NEW];
  const sel = Math.max(0, entries.indexOf(at));
  const cur = agents[sel];
  const curRef = useRef(cur);
  curRef.current = cur;

  useEffect(() => {
    void accountsApi.get().then(setAcc, () => {});
    // Which agent a build belongs to comes with its chat's `build` event.
    const owner = new Map<number, string>();
    const timers: number[] = [];
    const off = onEvent((ev) => {
      if (ev.type === "chat" && ev.kind === "build" && ev.build != null && ev.agent) {
        owner.set(ev.build, ev.agent);
        setBuilds((b) => ({ ...b, [ev.agent!]: { id: ev.build!, stage: "writing" } }));
      } else if (ev.type === "build") {
        const agent = owner.get(ev.id);
        if (!agent) return;
        setBuilds((b) => ({ ...b, [agent]: { id: ev.id, stage: ev.stage, section: ev.section } }));
        if (ev.stage === "done" || ev.stage === "failed") {
          timers.push(
            window.setTimeout(() => setBuilds((b) => (b[agent]?.id === ev.id ? Object.fromEntries(Object.entries(b).filter(([k]) => k !== agent)) : b)), 8000),
          );
        }
      } else if (ev.type === "account" && (ev.state === "signed_in" || ev.state === "signed_out")) {
        void accountsApi.get().then(setAcc, () => {});
      }
    });
    return () => {
      off();
      timers.forEach((t) => window.clearTimeout(t));
    };
  }, []);

  // Start the agent on screen while the user talks, so its answer comes sooner.
  useEffect(
    () =>
      onEvent((ev) => {
        const a = curRef.current;
        if (ev.type === "voice" && ev.state === "listening" && a) void currentChat(a.id).then((c) => chatApi.warm(a.id, c));
      }),
    [],
  );

  const move = (d: number) => onAt(entries[Math.max(0, Math.min(entries.length - 1, sel + d))]!);
  const tapEntry = (i: number) => {
    const id = entries[i]!;
    if (i !== sel) return onAt(id);
    if (id === NEW) {
      if (agents.length < max) onOpen({ t: "new-agent" });
      return;
    }
    onOpen({ t: "agent", id });
  };

  useAppInput((ev) => {
    if (ev.type === "voice" && ev.state === "final" && ev.text?.trim()) {
      if (!cur) return false; // on New agent: the words go to the agent you last talked to
      const say = ev.text.trim();
      void currentChat(cur.id).then((id) => onOpen({ t: "chat", agent: cur.id, id, say }));
      return true;
    }
    if (ev.type === "wheel") {
      move(ev.delta > 0 ? 1 : -1);
      return true;
    }
    if (ev.type === "gesture" && ev.kind === "tap") {
      tapEntry(sel);
      return true;
    }
    return false;
  });

  const accent = cur ? colorOf(cur.color) : "#9aa4b2";
  const frost = { ["--frost" as string]: `url(${noiseUrl("frost")})` };
  return (
    <div className="agent-home" style={{ ["--accent" as string]: accent }}>
      <div className="wheel" aria-label="Your agents">
        {entries.map((_, i) => {
          const d = i - sel;
          if (Math.abs(d) > WINDOW) return null;
          const on = d === 0;
          const a = agents[i];
          const style = { transform: `translate(-50%, calc(-50% + ${d * STEP}px)) scale(${on ? 1 : 0.5})`, opacity: on ? 1 : Math.abs(d) === 1 ? 0.42 : 0 };
          if (!a) {
            const full = agents.length >= max;
            return (
              <button key={NEW} className={`slide new${on ? " on" : ""}`} style={style} onClick={() => tapEntry(i)} aria-label="New agent">
                <span className="slide-name">New agent</span>
                <span className="new-ring">
                  <Icon name="plus" weight="bold" size={54} />
                </span>
                <span className="slide-status idle">{full ? `That's ${max}, the most there's room for` : "Tap to make one"}</span>
              </button>
            );
          }
          const status = statusOf(a, builds[a.id], acc, connected);
          return (
            <button key={a.id} className={`slide${on ? " on" : ""}`} style={style} onClick={() => tapEntry(i)} aria-label={a.name}>
              <span className="slide-name">{a.name}</span>
              <Character
                form={a.form}
                color={a.color}
                finish={a.finish}
                eyes={a.eyes}
                wear={a.wear}
                state={charState(a, builds[a.id], connected)}
                activity={activity(a, builds[a.id])}
                unread={a.unread}
                size={176}
              />
              <span className={`slide-status${status.cls}`}>{status.text}</span>
            </button>
          );
        })}
        <div className="wheel-dots" aria-hidden>
          {entries.map((id, i) => {
            const a = agents[i];
            const busy = a && a.state !== "idle" && a.state !== "error";
            return (
              <span
                key={id}
                className={`${i === sel ? "on" : ""}${busy ? " busy" : ""}${a && a.unread > 0 ? " unread" : ""}${id === NEW ? " new" : ""}`}
                style={a ? { ["--dot" as string]: colorOf(a.color) } : undefined}
              />
            );
          })}
        </div>
      </div>

      <button className="home-corner left" style={frost} onClick={onLauncher} aria-label="Apps">
        <Icon name="squares-four" weight="bold" size={22} />
      </button>
      <button className="home-corner right" style={frost} onClick={onSleep} aria-label="Sleep">
        <Icon name="moon" weight="bold" size={22} />
      </button>
    </div>
  );
}
