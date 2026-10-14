// Root of the r1os shell (docs/specs/experience.md, ADR-0007, ADR-0011).
//
//   first run → onboarding (resumable; r1d stores the step)
//   home      = your agents on the wheel, then New agent.   tap: its space · hold: talk · double tap: launcher
//   space     = one agent's chats, memory and settings (space.tsx).   double tap: back
//   launcher  = new app, apps, tools, settings.   double tap: home
//   screens   = a chat, an app, New app, a tool, a settings page.   double tap: back
//
// Screens stack: double tap goes back to wherever you came from (home → space → chat → back to the
// space). Input comes only from r1d's event stream (never DOM key events; the host swallows DPAD).
// The open screen's handler (home's too) sees events first. The press that wakes the screen is
// ignored: gestures are dropped while the page is hidden and for WAKE_GUARD_MS after it becomes
// visible again.
//
// Hold to talk anywhere after onboarding (voice.tsx): r1d listens and streams `voice` events; the
// overlay shows them. The final words go to the open screen first (home sends them to the agent on
// screen, a space to its agent, a chat to itself); anywhere else they go to the agent you last talked
// to, in its current conversation (or a new one).
//
// Sleep (home's bar, the launcher) turns the display off through r1d; the side button wakes it. The
// simulator has no display to turn off, so it draws a black screen until the next input.

import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { agentsApi, chatApi, currentChat, onboardingApi, type Agent, type AgentInfo, type OnboardingStep } from "./api";
import { APPS } from "./apps";
import { AppView } from "./apps/AppView";
import { ChatView } from "./chat";
import { CreateApp } from "./create";
import { AgentHome, NEW } from "./home";
import { AppInputContext, type InputHandler } from "./input";
import { Launcher, type Target } from "./launcher";
import { AppInfo, ManageApps } from "./manage";
import { Onboarding } from "./onboarding";
import { api, onEvent, useR1Connection, type R1State } from "./r1";
import { Settings, SETTINGS_TITLE } from "./settings";
import { SimFrame } from "./sim";
import { AgentLook, AgentSpace, ManageChats, MemoryList, NewAgent } from "./space";
import { colorOf } from "./ui/Character";
import { VoiceOverlay, type VoiceUi } from "./voice";

type View = { v: "home" } | { v: "launcher" } | { v: "screen"; target: Target; back: View };

/** Screens deeper than this go back home instead (voice can keep opening chats over apps). */
const MAX_DEPTH = 6;
const depth = (v: View): number => (v.v === "screen" ? 1 + depth(v.back) : 0);

const WAKE_GUARD_MS = 700;
let visibleSince = performance.now();
document.addEventListener("visibilitychange", () => {
  if (!document.hidden) visibleSince = performance.now();
});
const justWoke = () => document.hidden || performance.now() - visibleSince < WAKE_GUARD_MS;

const DEFAULT_AGENT: AgentInfo = {
  id: "",
  name: "Pip",
  form: "orb",
  color: "orange",
  finish: "soft",
  personality: "friendly",
  custom: "",
  role: "",
  brain: "claude",
  state: "idle",
  unread: 0,
};

/** The agent a screen belongs to, if it belongs to one. */
function agentOf(t: Target): string | undefined {
  switch (t.t) {
    case "chat":
    case "chats":
    case "memory":
      return t.agent;
    case "agent":
    case "agent-look":
      return t.id;
    default:
      return undefined;
  }
}

export default function App() {
  const { state, connected } = useR1Connection();
  const [agents, setAgents] = useState<AgentInfo[]>([]);
  /** The agent you last talked to (r1d's focus). */
  const [focus, setFocus] = useState("");
  const [max, setMax] = useState(8);
  /** Where the home wheel is: an agent's id, or NEW. */
  const [wheelAt, setWheelAt] = useState<string | null>(null);
  const [step, setStep] = useState<OnboardingStep | null>(null);
  const [view, setView] = useState<View>({ v: "home" });
  const [voice, setVoice] = useState<VoiceUi | null>(null);
  const voiceTimer = useRef<number | undefined>(undefined);
  const [asleep, setAsleep] = useState(false);
  const asleepRef = useRef(asleep);
  asleepRef.current = asleep;
  const simRef = useRef(false);
  simRef.current = !!state?.sim;
  const viewRef = useRef(view);
  viewRef.current = view;
  const stepRef = useRef(step);
  stepRef.current = step;
  const agentsRef = useRef(agents);
  agentsRef.current = agents;
  const focusRef = useRef(focus);
  focusRef.current = focus;
  const handler = useRef<InputHandler | null>(null);
  const setHandler = useCallback((h: InputHandler | null) => {
    handler.current = h;
  }, []);
  const errorTimers = useRef<Record<string, number>>({});
  /** "Nova replied": a reply landed in a chat that isn't on screen. Touch opens it. */
  const [toast, setToast] = useState<{ agent: string; chat: string } | null>(null);
  const toastTimer = useRef<number | undefined>(undefined);
  /** The chat a ChatView made for itself (a new chat has no id in its target). */
  const madeChat = useRef<string | undefined>(undefined);

  const loadAgents = useCallback(
    () =>
      agentsApi.list().then(
        (r) => {
          setAgents(r.agents);
          setFocus(r.focus);
          setMax(r.max);
          setWheelAt((w) => (w && (w === NEW || r.agents.some((a) => a.id === w)) ? w : r.focus));
        },
        () => {},
      ),
    [],
  );

  useEffect(() => {
    if (!connected) return;
    void loadAgents();
    void onboardingApi.get().then(setStep, () => setStep("done"));
  }, [connected, loadAgents]);

  /** The agent you last talked to: words spoken outside home, a space or a chat go to it. */
  const focusAgent = (): AgentInfo => agentsRef.current.find((a) => a.id === focusRef.current) ?? agentsRef.current[0] ?? DEFAULT_AGENT;

  const open = useCallback((target: Target, from?: View) => {
    if (target.t === "android") {
      void api.openAndroid("settings").catch(() => {});
      return;
    }
    if (target.t === "sleep") {
      setView({ v: "home" });
      if (simRef.current) setAsleep(true);
      else void api.screen(false).catch(() => {});
      return;
    }
    if (target.t === "chat") madeChat.current = undefined;
    const back = from ?? viewRef.current;
    setView({ v: "screen", target, back: depth(back) >= MAX_DEPTH ? { v: "home" } : back });
  }, []);

  /** Shows a voice outcome for a moment, then clears it (unless a new hold started meanwhile). */
  const flashVoice = useCallback((v: VoiceUi, ms: number) => {
    setVoice(v);
    window.clearTimeout(voiceTimer.current);
    voiceTimer.current = window.setTimeout(() => setVoice((cur) => (cur?.phase === "listening" ? cur : null)), ms);
  }, []);

  useEffect(
    () =>
      onEvent((ev) => {
        // Agents: what each is doing, and changes to the list. An error shows for a moment.
        if (ev.type === "agents_changed") {
          void loadAgents();
          return;
        }
        if (ev.type === "chat" && ev.kind === "tool" && ev.agent) {
          const id = ev.agent;
          setAgents((as) => as.map((a) => (a.id === id ? { ...a, tool: ev.text } : a)));
        }
        if (ev.type === "unread") {
          setAgents((as) => as.map((a) => (a.id === ev.agent ? { ...a, unread: ev.count } : a)));
          const v = viewRef.current;
          const onScreen = v.v === "screen" && v.target.t === "chat" ? (v.target.id ?? madeChat.current) : undefined;
          if (ev.count > 0 && ev.chat !== onScreen) {
            setToast({ agent: ev.agent, chat: ev.chat });
            window.clearTimeout(toastTimer.current);
            toastTimer.current = window.setTimeout(() => setToast(null), 3500);
          }
        }
        if (ev.type === "agent_state" && ev.agent) {
          const id = ev.agent;
          window.clearTimeout(errorTimers.current[id]);
          if (ev.state === "thinking") setFocus(id); // r1d makes the agent you talk to the focus
          const done = ev.state === "idle" || ev.state === "error";
          setAgents((as) =>
            as.map((a) => (a.id === id ? { ...a, state: ev.state, chat: ev.state === "idle" ? undefined : ev.chat, tool: done ? undefined : a.tool } : a)),
          );
          if (ev.state === "error") {
            errorTimers.current[id] = window.setTimeout(
              () => setAgents((as) => as.map((a) => (a.id === id && a.state === "error" ? { ...a, state: "idle", chat: undefined } : a))),
              2500,
            );
          }
        }
        if ((ev.type === "gesture" || ev.type === "button") && justWoke()) return;
        if (asleepRef.current) {
          if (ev.type === "gesture" || ev.type === "wheel") setAsleep(false);
          return;
        }
        if (ev.type === "open") {
          open({ t: "app", id: ev.app });
          return;
        }
        if (stepRef.current && stepRef.current !== "done") {
          handler.current?.(ev);
          return;
        }
        if (ev.type === "gesture" && ev.kind === "hold_end") {
          setVoice((v) => (v?.phase === "listening" ? { ...v, released: true } : v));
        }
        if (ev.type === "voice_level") {
          setVoice((v) => (v?.phase === "listening" ? { ...v, level: ev.level } : v));
          return;
        }
        if (ev.type === "voice") {
          window.clearTimeout(voiceTimer.current);
          const text = ev.text?.trim() ?? "";
          switch (ev.state) {
            case "listening": {
              setVoice({ phase: "listening", text: "", level: 0 });
              // Start the agent while the user talks. Home, spaces and chats warm their own target.
              const v = viewRef.current;
              const own = v.v === "home" || (v.v === "screen" && ["chat", "agent", "app-info"].includes(v.target.t));
              if (!own) {
                const a = focusAgent();
                if (a.id) void currentChat(a.id).then((id) => chatApi.warm(a.id, id));
              }
              return;
            }
            case "partial":
              setVoice((v) => ({ phase: "listening", text, level: v?.level ?? 0, released: v?.released }));
              return;
            case "cancelled":
              setVoice(null);
              return;
            case "error":
              flashVoice({ phase: "error", text: text || "Something went wrong", level: 0 }, 2600);
              return;
            case "final":
              if (!text) {
                flashVoice({ phase: "error", text: "Didn't catch that", level: 0 }, 1600);
                return;
              }
              flashVoice({ phase: "sending", text, level: 0 }, 650);
              if (handler.current?.(ev)) return;
              {
                const v = viewRef.current;
                const a = focusAgent();
                // Talking over an open app: the agent is told which one ("log two", "make this weekly").
                const onScreen = v.v === "screen" && v.target.t === "app" ? v.target.id : undefined;
                if (a.id) void currentChat(a.id).then((id) => open({ t: "chat", agent: a.id, id, say: text, app: onScreen }));
              }
              return;
          }
        }
        const v = viewRef.current;
        if (handler.current?.(ev)) return;
        if (ev.type === "gesture" && ev.kind === "double_tap") {
          setView(v.v === "home" ? { v: "launcher" } : v.v === "launcher" ? { v: "home" } : v.back);
        }
      }),
    [open, flashVoice, loadAgents],
  );

  const byId = (id?: string) => agents.find((a) => a.id === id);
  // Whose face the push-to-talk overlay shows: the agent the words will go to.
  const viewAgent = view.v === "home" ? byId(wheelAt ?? undefined) : view.v === "screen" ? byId(agentOf(view.target)) : undefined;
  const voiceAgent = viewAgent ?? focusAgent();

  let body: ReactNode;
  if (step === null || agents.length === 0) {
    // r1d always has at least one agent: wait for the list rather than flash an empty wheel.
    body = <div className="center muted">{connected ? "" : "Starting…"}</div>;
  } else if (step !== "done") {
    body = <Onboarding step={step} agent={agents[0] ?? DEFAULT_AGENT} onStep={setStep} onAgent={() => void loadAgents()} />;
  } else if (view.v === "home") {
    body = (
      <AgentHome
        agents={agents}
        max={max}
        at={wheelAt ?? focus}
        connected={connected}
        onAt={setWheelAt}
        onOpen={(t) => open(t)}
        onLauncher={() => setView({ v: "launcher" })}
        onSleep={() => open({ t: "sleep" })}
      />
    );
  } else if (view.v === "launcher") {
    body = <Launcher onOpen={(t) => open(t)} />;
  } else {
    body = (
      <Screen
        target={view.target}
        agents={agents}
        focus={focusAgent()}
        state={state}
        onBack={() => setView(view.back)}
        onHome={() => setView({ v: "home" })}
        onOpen={open}
        onChat={(id) => {
          madeChat.current = id;
        }}
        onMade={(a) => {
          setWheelAt(a.id);
          setFocus(a.id);
          void loadAgents();
          setView({ v: "home" });
        }}
      />
    );
  }

  const unread = agents.filter((a) => a.unread > 0);
  const toastAgent = toast && byId(toast.agent);
  const screen = (
    <div className="screen">
      <StatusBar state={state} connected={connected} unread={view.v === "home" ? [] : unread} />
      {toast && toastAgent && step === "done" && (
        <button
          className="toast"
          style={{ ["--accent" as string]: colorOf(toastAgent.color) }}
          onClick={() => {
            setToast(null);
            open({ t: "chat", agent: toast.agent, id: toast.chat });
          }}
        >
          <span className="toast-dot" />
          {toastAgent.name} replied
        </button>
      )}
      <AppInputContext.Provider value={setHandler}>{body}</AppInputContext.Provider>
      {voice && step === "done" && <VoiceOverlay v={voice} agent={voiceAgent} />}
      {!connected && <div className="offline">Connecting to r1d…</div>}
      {asleep && <div className="asleep" onClick={() => setAsleep(false)} />}
    </div>
  );
  return state?.sim ? <SimFrame>{screen}</SimFrame> : screen;
}

function Screen({
  target,
  agents,
  focus,
  state,
  onBack,
  onHome,
  onOpen,
  onChat,
  onMade,
}: {
  target: Target;
  agents: AgentInfo[];
  focus: AgentInfo;
  state: R1State | null;
  onBack: () => void;
  onHome: () => void;
  onOpen: (t: Target) => void;
  /** A ChatView made its chat (it started as a new chat). */
  onChat: (id: string) => void;
  onMade: (a: Agent) => void;
}) {
  const nav = { openApp: (id: string) => onOpen({ t: "app", id }), home: onBack };
  const id = agentOf(target);
  const agent = agents.find((a) => a.id === id);
  let title = "";
  let content: ReactNode = null;
  switch (target.t) {
    case "app":
      content = <AppView id={target.id} />;
      break;
    case "chat": {
      const a = agent ?? focus;
      content = (
        <ChatView
          key={target.id ?? `new-${a.id}`}
          id={target.id}
          say={target.say}
          sayApp={target.app}
          agent={a}
          onChat={onChat}
          onOpenApp={(app) => onOpen({ t: "app", id: app })}
        />
      );
      break;
    }
    case "agent":
      title = agent?.name ?? "";
      content = agent ? <AgentSpace agent={agent} agents={agents} onOpen={onOpen} onHome={onHome} /> : null;
      break;
    case "new-agent":
      title = "New agent";
      content = <NewAgent agents={agents} onMade={onMade} />;
      break;
    case "agent-look":
      title = "Look and personality";
      content = agent ? <AgentLook agent={agent} onDone={onBack} /> : null;
      break;
    case "chats":
      title = agent ? `Chats with ${agent.name}` : "Chats";
      content = agent ? <ManageChats agent={agent} /> : null;
      break;
    case "memory":
      title = agent ? `${agent.name}'s memory` : "Memory";
      content = agent ? <MemoryList agent={agent} agents={agents} /> : null;
      break;
    case "manage":
      title = "Manage apps";
      content = <ManageApps onOpen={(t) => onOpen(t)} />;
      break;
    case "app-info":
      title = "App";
      content = <AppInfo id={target.id} onOpen={(t) => onOpen(t)} onBack={onBack} />;
      break;
    case "create":
      title = "New app";
      content = <CreateApp onOpen={(app) => onOpen({ t: "app", id: app })} />;
      break;
    case "tool": {
      const tool = APPS.find((a) => a.id === target.id);
      title = tool?.name ?? "";
      content = tool ? <tool.Component state={state} nav={nav} /> : null;
      break;
    }
    case "settings":
      title = SETTINGS_TITLE[target.page];
      content = target.page === "you" ? <MemoryList agents={agents} /> : <Settings page={target.page} agent={focus} state={state} onDone={onBack} />;
      break;
    default:
      content = null;
  }
  return (
    <div className="app" key={JSON.stringify(target)}>
      {title && (
        <header className="app-header">
          <button className="back" onClick={onBack} aria-label="Back">
            ‹
          </button>
          <span>{title}</span>
        </header>
      )}
      {content}
    </div>
  );
}

/** The clock, unread replies (off home: a pill in the first such agent's colour), battery, link. */
function StatusBar({ state, connected, unread }: { state: R1State | null; connected: boolean; unread: AgentInfo[] }) {
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    const t = window.setInterval(() => setNow(new Date()), 10_000);
    return () => window.clearInterval(t);
  }, []);
  const b = state?.system;
  return (
    <div className="status">
      <span>{now.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}</span>
      <span className="status-right">
        {unread.length > 0 && (
          <span className="unread-pill" style={{ ["--accent" as string]: colorOf(unread[0]!.color) }} aria-label="Unread replies">
            <span className="toast-dot" />
            {unread.reduce((n, a) => n + a.unread, 0)}
          </span>
        )}
        {state?.sim && <span className="badge">SIM</span>}
        {b?.battery_pct != null && (
          <span>
            {b.charging ? "⚡" : ""}
            {b.battery_pct}%
          </span>
        )}
        <span className={`dot ${connected ? "up" : "down"}`} />
      </span>
    </div>
  );
}
