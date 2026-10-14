// An agent's space (tap an agent on home; docs/specs/experience.md §2.7, ADR-0011): everything that
// belongs to one agent.
//
//   New chat · its chats, newest first (tap resumes one) · Memory · Look and personality ·
//   Talks with Claude/Codex · Manage chats · Move earlier/later · Remove
//
//   wheel  moves the focus ring       tap  opens it
//   hold   talk: into the chat under the ring (New chat → a new one), else its current chat
//   double tap  back home
//
// Also here: making a new agent (the end of the home wheel), redesigning one, managing its chats,
// and the memory list (an agent's own facts plus the shared ones; Settings › About you shows the
// shared ones alone).

import { useEffect, useState } from "react";
import { Icon } from "@r1os/icons";
import { accountsApi, agentsApi, chatApi, currentChat, memoryApi, NAMES, type Accounts, type Agent, type AgentInfo, type Chat, type Memory } from "./api";
import type { Target } from "./launcher";
import { onEvent } from "./r1";
import { AgentDesigner, useFocusList } from "./setup";
import { activityOf, Character, COLORS, FORMS, colorOf, type CharState } from "./ui/Character";

function ago(ms: number): string {
  const s = Math.max(0, (Date.now() - ms) / 1000);
  if (s < 60) return "now";
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h`;
  return `${Math.floor(s / 86400)}d`;
}

const BRAIN = { claude: "Claude", codex: "Codex" } as const;

const LIVE: Record<string, string> = {
  waiting: "Waiting for room to run…",
  thinking: "Thinking…",
  speaking: "Talking…",
  working: "Working on it…",
  error: "Something went wrong",
};

/** Taps a row twice to confirm: the first arms it for 3 s. */
function useConfirm(): [string | null, (key: string, run: () => void) => void] {
  const [armed, setArmed] = useState<string | null>(null);
  useEffect(() => {
    if (!armed) return;
    const t = window.setTimeout(() => setArmed(null), 3000);
    return () => window.clearTimeout(t);
  }, [armed]);
  return [
    armed,
    (key, run) => {
      if (armed === key) {
        setArmed(null);
        run();
      } else {
        setArmed(key);
      }
    },
  ];
}

function useChats(agent: string): [Chat[] | null, () => void] {
  const [chats, setChats] = useState<Chat[] | null>(null);
  const load = () => void chatApi.list(agent).then(setChats, () => setChats((c) => c ?? []));
  useEffect(() => {
    load();
    return onEvent((ev) => {
      if (ev.type === "chat" && ev.agent === agent && (ev.kind === "done" || ev.kind === "error" || ev.kind === "note")) load();
      if (ev.type === "unread" && ev.agent === agent) load();
    });
  }, [agent]);
  return [chats, load];
}

export function AgentSpace({ agent, agents, onOpen, onHome }: { agent: AgentInfo; agents: AgentInfo[]; onOpen: (t: Target) => void; onHome: () => void }) {
  const [chats] = useChats(agent.id);
  const [memories, setMemories] = useState<number | null>(null);
  const [acc, setAcc] = useState<Accounts | null>(null);
  const [msg, setMsg] = useState<string | null>(null);
  const [armed, confirm] = useConfirm();
  useEffect(() => {
    void memoryApi.list(agent.id).then((ms) => setMemories(ms.filter((m) => !m.shared).length), () => {});
    void accountsApi.get().then(setAcc, () => {});
  }, [agent.id]);
  // One input handler per screen: held-button words go through the focus list's `extra`.
  const ref = useFocusList<HTMLDivElement>([chats?.length, acc?.claude, acc?.codex], (ev) => {
    if (ev.type === "voice" && ev.state === "final" && ev.text?.trim()) {
      const say = ev.text.trim();
      void target().then((id) => onOpen({ t: "chat", agent: agent.id, id, say }));
      return true;
    }
    return false;
  });
  const at = agents.findIndex((a) => a.id === agent.id);

  /** The chat under the focus ring: an id, "new", or undefined (another row). */
  const focused = () => ref.current?.querySelector(".r1-focus")?.getAttribute("data-chat") ?? undefined;
  const target = async (): Promise<string | undefined> => {
    const f = focused();
    if (f === "new") return undefined;
    return f ?? (await currentChat(agent.id));
  };

  useEffect(
    () =>
      onEvent((ev) => {
        if (ev.type === "voice" && ev.state === "listening") void target().then((c) => chatApi.warm(agent.id, c));
      }),
    [agent.id],
  );

  const other = agent.brain === "claude" ? "codex" : "claude";
  const switchBrain = () => {
    if (!acc?.[other]) {
      setMsg(`${BRAIN[other]} isn't connected (Settings › Accounts).`);
      return;
    }
    void agentsApi.put({ ...agent, brain: other }).then(
      () => setMsg(`New chats with ${agent.name} use ${BRAIN[other]}. Older chats keep theirs.`),
      (e) => setMsg(String(e instanceof Error ? e.message : e)),
    );
  };
  const move = (d: number) => {
    const ids = agents.map((a) => a.id);
    const j = at + d;
    if (at < 0 || j < 0 || j >= ids.length) return;
    [ids[at], ids[j]] = [ids[j]!, ids[at]!];
    void agentsApi.order(ids).catch(() => {});
  };
  const remove = () =>
    confirm("remove", () =>
      void agentsApi.remove(agent.id).then(onHome, (e) => setMsg(String(e instanceof Error ? e.message : e))),
    );

  const state: CharState = agent.state;
  return (
    <div className="launcher space" ref={ref} style={{ ["--accent" as string]: colorOf(agent.color) }}>
      <div className="space-head">
        <Character form={agent.form} color={agent.color} finish={agent.finish} eyes={agent.eyes} wear={agent.wear} state={state} activity={activityOf(agent.tool)} size={56} />
        <div className="grow">
          <div className="space-role ellipsis">{agent.role ? `For ${agent.role}` : "A bit of everything"}</div>
          <div className="muted small">{LIVE[agent.state] ?? "Hold the button to talk"}</div>
        </div>
      </div>
      <button className="l-item l-new" data-chat="new" onClick={() => onOpen({ t: "chat", agent: agent.id })}>
        <span className="l-glyph">
          <Icon name="plus" weight="bold" size={20} />
        </span>
        <span className="grow">New chat</span>
      </button>
      {chats && chats.length > 0 && <div className="l-section">Chats</div>}
      {chats?.slice(0, 30).map((c) => (
        <button
          key={c.id}
          className={`l-item chat-row${c.unread > 0 ? " unread" : ""}`}
          data-chat={c.id}
          onClick={() => onOpen({ t: "chat", agent: agent.id, id: c.id })}
        >
          <span className="grow">
            <span className="ellipsis chat-row-title">{c.title}</span>
            {c.last && <span className="muted small ellipsis chat-row-last">{c.last}</span>}
          </span>
          {c.unread > 0 ? (
            <span className="unread-count" aria-label={`${c.unread} unread`}>
              {c.unread}
            </span>
          ) : (
            <span className="muted small">{agent.chat === c.id && agent.state !== "idle" ? "…" : ago(c.updated)}</span>
          )}
        </button>
      ))}
      {chats?.length === 0 && <div className="muted manage-empty">No chats yet. Hold the button and say hi.</div>}

      <div className="l-section">{agent.name}</div>
      <button className="l-item" onClick={() => onOpen({ t: "memory", agent: agent.id })}>
        <span className="l-glyph">✦</span>
        <span className="grow">Memory</span>
        <span className="muted small">{memories ?? ""}</span>
      </button>
      <button className="l-item" onClick={() => onOpen({ t: "agent-look", id: agent.id })}>
        <span className="l-glyph">◑</span>
        <span className="grow">Look and personality</span>
      </button>
      <button className="l-item" onClick={switchBrain}>
        <span className="l-glyph">⚡</span>
        <span className="grow">Talks with {BRAIN[agent.brain]}</span>
        <span className="muted small">{acc?.[other] ? `switch to ${BRAIN[other]}` : ""}</span>
      </button>
      {chats && chats.length > 0 && (
        <button className="l-item" onClick={() => onOpen({ t: "chats", agent: agent.id })}>
          <span className="l-glyph">☰</span>
          <span className="grow">Manage chats</span>
        </button>
      )}
      {at > 0 && (
        <button className="l-item" onClick={() => move(-1)}>
          <span className="l-glyph">↑</span>
          <span className="grow">Move earlier on the wheel</span>
        </button>
      )}
      {at >= 0 && at < agents.length - 1 && (
        <button className="l-item" onClick={() => move(1)}>
          <span className="l-glyph">↓</span>
          <span className="grow">Move later on the wheel</span>
        </button>
      )}
      {agents.length > 1 && (
        <button className={`l-item${armed === "remove" ? " danger" : ""}`} onClick={remove}>
          <span className="l-glyph">🗑</span>
          <span className="grow">{armed === "remove" ? `Tap again: remove ${agent.name}, its chats and memory` : `Remove ${agent.name}`}</span>
        </button>
      )}
      {msg && <div className="manage-msg">{msg}</div>}
    </div>
  );
}

/** An agent's chats, to delete old ones (tap twice). Opening and resuming is in its space. */
export function ManageChats({ agent }: { agent: AgentInfo }) {
  const [chats, load] = useChats(agent.id);
  const [armed, confirm] = useConfirm();
  const [msg, setMsg] = useState<string | null>(null);
  const ref = useFocusList<HTMLDivElement>([chats?.length]);
  const remove = (c: Chat) =>
    confirm(c.id, () =>
      void chatApi.remove(c.id).then(
        () => (setMsg(`Deleted “${c.title}”.`), load()),
        (e) => setMsg(String(e instanceof Error ? e.message : e)),
      ),
    );
  return (
    <div className="launcher" ref={ref}>
      <div className="muted small manage-hint">Tap a chat twice to delete it.</div>
      {chats?.length === 0 && <div className="muted manage-empty">No chats with {agent.name}.</div>}
      {chats?.map((c) => (
        <button key={c.id} className={`l-item${armed === c.id ? " danger" : ""}`} onClick={() => remove(c)}>
          <span className="grow ellipsis">{armed === c.id ? "Tap again to delete" : c.title}</span>
          <span className="muted small">{ago(c.updated)}</span>
        </button>
      ))}
      {msg && <div className="manage-msg">{msg}</div>}
    </div>
  );
}

/**
 * What's remembered. With an agent: its own facts, then the facts every agent shares. Without:
 * the shared facts alone (Settings › About you). Tap a fact twice to forget it.
 */
export function MemoryList({ agent, agents = [] }: { agent?: AgentInfo; agents?: AgentInfo[] }) {
  const [ms, setMs] = useState<Memory[] | null>(null);
  const [armed, confirm] = useConfirm();
  const load = () => void memoryApi.list(agent?.id).then(setMs, () => setMs([]));
  useEffect(load, [agent?.id]);
  const ref = useFocusList<HTMLDivElement>([ms?.length]);
  const nameOf = (id?: string) => agents.find((a) => a.id === id)?.name;
  const forget = (m: Memory) => confirm(String(m.id), () => void memoryApi.forget(m.id).then(load, () => {}));
  const own = ms?.filter((m) => !m.shared) ?? [];
  const shared = ms?.filter((m) => m.shared) ?? [];
  const row = (m: Memory) => (
    <button key={m.id} className={`l-item memory${armed === String(m.id) ? " danger" : ""}`} onClick={() => forget(m)}>
      <span className="grow">
        {armed === String(m.id) ? "Tap again to forget" : m.text}
        {m.shared && nameOf(m.by) && armed !== String(m.id) && <span className="muted small"> · from {nameOf(m.by)}</span>}
      </span>
    </button>
  );
  return (
    <div className="launcher" ref={ref}>
      {ms && ms.length > 0 && <div className="muted small manage-hint">Tap a fact twice to forget it.</div>}
      {agent && <div className="l-section">Only {agent.name}</div>}
      {agent && (own.length ? own.map(row) : <div className="muted manage-empty">Nothing of its own yet.</div>)}
      <div className="l-section">Every agent knows</div>
      {shared.length ? shared.map(row) : <div className="muted manage-empty">Nothing yet. Tell any agent about yourself.</div>}
    </div>
  );
}

/** The end of the home wheel: design a new agent. It lands on the wheel just before New agent. */
export function NewAgent({ agents, onMade }: { agents: AgentInfo[]; onMade: (a: Agent) => void }) {
  const [acc, setAcc] = useState<Accounts | null>(null);
  useEffect(() => {
    void accountsApi.get().then(setAcc, () => setAcc({ claude: true, codex: false }));
  }, []);
  // A look nobody has yet: the next unused colour, name and form.
  const [initial] = useState<Agent>(() => {
    const colors = new Set(agents.map((a) => a.color));
    const names = new Set(agents.map((a) => a.name));
    const color = Object.keys(COLORS).find((c) => !colors.has(c)) ?? "blue";
    const name = NAMES.find((n) => !names.has(n)) ?? "Nova";
    const form = FORMS[agents.length % FORMS.length]!.id;
    return { id: "", name, form, color, finish: "soft", personality: "friendly", custom: "", role: "", brain: "claude" };
  });
  if (!acc) return <div className="center muted">…</div>;
  const brain = acc.claude || !acc.codex ? "claude" : "codex";
  return (
    <AgentDesigner
      initial={{ ...initial, brain }}
      doneLabel="Make it"
      onSave={({ id: _id, ...a }) => agentsApi.create(a)}
      onDone={onMade}
    />
  );
}

/** Redesign an agent: the same steps as making one. */
export function AgentLook({ agent, onDone }: { agent: AgentInfo; onDone: () => void }) {
  return <AgentDesigner initial={agent} doneLabel="Save" onSave={agentsApi.put} onDone={onDone} />;
}
