// Chats (ADR-0051): the companion's second panel. Your agents and their chats down the side, and a chat window to talk
// to one: typed, with photos (picked, pasted or dropped), its replies streaming in with their tool steps, cards and
// builds. It's the same chat as on the R1: what you say here shows there, and what you say there shows here. An agent
// asking to change something still asks on the R1, which is where it's granted.

import { Icon } from "@r1os/icons";
import { AnimatePresence, motion } from "motion/react";
import { type ClipboardEvent, type DragEvent, useCallback, useEffect, useRef, useState } from "react";
import { Character } from "@shell/ui/Character";
import { toolLabel } from "@shell/tool-labels";
import { type Agent, type R1Info, api, upload, useGet, useR1Events } from "../link";
import { soft, spring } from "../motion";
import { BuildLine, ChatCard, MailLine, Markdown, Photo } from "./ChatBits";
import { asJpeg } from "./Files";
import { ago, Twice } from "./parts";
import { type R1Data, STATE_WORDS } from "./R1View";
import { Seg } from "./Settings";

interface Chat {
  id: string;
  agent: string;
  title: string;
  brain: string;
  updated: number;
  last?: string;
  unread: number;
  mode?: string;
}

interface Message {
  id: number;
  role: string;
  text: string;
  at: number;
  meta?: Record<string, unknown>;
}

type Item =
  | { k: "user"; key: string; text: string; images?: string[]; at?: number }
  | { k: "agent"; key: string; text: string; at?: number }
  | { k: "error"; key: string; text: string }
  | { k: "note"; key: string; text: string }
  | { k: "routine"; key: string; title: string; at: number }
  | { k: "tool"; key: string; name: string }
  | { k: "card"; key: string; card: { type: string; [k: string]: unknown } }
  | { k: "build"; key: string; id: number; stage: string; app?: string; edit?: boolean; error?: string; label?: string }
  | { k: "mail"; key: string; text: string; out: boolean; who: string; kind?: string }
  | { k: "ask"; key: string; title: string };

const BUSY = ["waiting", "thinking", "speaking", "working"];

const BRAINS: [string, string][] = [["claude", "claude"], ["codex", "codex"]];
/** How a chat on a computer asks (r1d's MODES), as Agents' "new chats" says it. */
const MODES: [string, string][] = [["manual", "asks"], ["acceptEdits", "edits freely"], ["auto", "auto"], ["plan", "plans only"]];

export function ChatsPanel({ r1, data }: { r1: R1Info; data: R1Data }) {
  const agents = data.agents;
  const [agentId, setAgentId] = useState<string | null>(() => localStorage.getItem(`chats:${r1.id}:agent`));
  const agent = agents.find((a) => a.id === agentId) ?? data.focus ?? agents[0] ?? null;
  // The chat open per agent (null: a new one), kept while you look at others.
  const [open, setOpen] = useState<Record<string, string | null>>({});
  const chatId = agent ? (open[agent.id] ?? null) : null;
  const pick = (a: Agent) => {
    setAgentId(a.id);
    try {
      localStorage.setItem(`chats:${r1.id}:agent`, a.id);
    } catch {
      // No storage: it starts on the focused agent next time.
    }
  };
  const openChat = useCallback((agent: string, chat: string | null) => setOpen((o) => ({ ...o, [agent]: chat })), []);

  if (!agent) return <div className="center muted">no agents yet: make one on your r1</div>;
  return (
    <div className="chats">
      <aside className="chat-side">
        <div className="side-label">agents</div>
        {agents.map((a) => (
          <AgentRow key={a.id} a={a} on={a.id === agent.id} onClick={() => pick(a)} />
        ))}
        <ChatList key={agent.id} r1={r1} agent={agent} open={chatId} onOpen={(c) => openChat(agent.id, c)} />
      </aside>
      <ChatWindow key={`${agent.id}`} r1={r1} agent={agent} agents={agents} chatId={chatId} onChat={(c) => openChat(agent.id, c)} />
    </div>
  );
}

function AgentRow({ a, on, onClick }: { a: Agent; on: boolean; onClick: () => void }) {
  const busy = BUSY.includes(a.state ?? "");
  const where = a.computer ? `on ${a.computer.name ?? "a computer"}${a.computer.connected ? "" : " · away"}` : "on the r1";
  return (
    <motion.button className={`chat-agent ${on ? "on" : ""}`} onClick={onClick} whileTap={{ scale: 0.98 }} layout transition={spring}>
      {on && <motion.span className="chat-agent-fill" layoutId="chat-agent-fill" transition={spring} />}
      <span className="chat-agent-char">
        <Character form={a.form} color={a.color} finish={a.finish} eyes={a.eyes} wear={a.wear} state={busy ? "thinking" : "idle"} size={36} />
      </span>
      <span className="chat-agent-text">
        <span className="chat-agent-name">{a.name}</span>
        <span className="chat-agent-sub">{busy ? `${STATE_WORDS[a.state ?? ""] ?? a.state}…` : where}</span>
      </span>
      {(a.unread ?? 0) > 0 && <span className="badge">{a.unread}</span>}
    </motion.button>
  );
}

function ChatList({ r1, agent, open, onOpen }: { r1: R1Info; agent: Agent; open: string | null; onOpen: (c: string | null) => void }) {
  const { data, reload } = useGet<{ chats: Chat[] }>(r1.id, `/api/chats?agent=${agent.id}`, ["unread", "agents_changed"]);
  // A turn that ended moves its chat to the top, with its newest words.
  useR1Events(r1.id, (e) => e.type === "chat" && e.agent === agent.id && (e.kind === "done" || e.kind === "user" || e.kind === "routine") && reload());
  const chats = data?.chats ?? [];
  return (
    <>
      <div className="side-label chat-list-label">{agent.name.toLowerCase()}'s chats</div>
      <button className={`chat-row new ${open === null ? "on" : ""}`} onClick={() => onOpen(null)}>
        <Icon name="plus" weight="bold" size={15} />
        <span>new chat</span>
      </button>
      <div className="chat-rows">
        {chats.map((c) => (
          <div key={c.id} className={`chat-row ${open === c.id ? "on" : ""}`} onClick={() => onOpen(c.id)} role="button" tabIndex={0}>
            <span className="chat-row-text">
              <span className="chat-row-title keep-case">{c.title || "a chat"}</span>
              <span className="chat-row-sub keep-case">{c.last ? `${c.last} · ` : ""}{ago(c.updated)}</span>
            </span>
            {c.unread > 0 && <span className="badge">{c.unread}</span>}
            <span className="chat-row-del" onClick={(e) => e.stopPropagation()}>
              <Twice label="" armed="delete?" icon="trash" onConfirm={() => void api(r1.id, "DELETE", `/api/chats/${c.id}`).then(() => { if (open === c.id) onOpen(null); reload(); })} />
            </span>
          </div>
        ))}
        {data && chats.length === 0 && <div className="muted chat-none">nothing yet</div>}
      </div>
    </>
  );
}

/** When a routine ran: its time, and its day if it wasn't today. */
function runAt(at: number): string {
  const d = new Date(at);
  const time = d.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  return d.toDateString() === new Date().toDateString() ? time : `${d.toLocaleDateString([], { weekday: "short" })} ${time}`;
}

/** A stored message as what the chat shows. */
function fromMessage(m: Message, name: (id: unknown) => string): Item | null {
  const meta = m.meta ?? {};
  const key = `m${m.id}`;
  switch (m.role) {
    case "user":
      // A routine's prompt (T-092): you wrote it once, when you set it up; its runs show as dividers.
      if (typeof meta.routine === "string") return { k: "routine", key, title: meta.routine, at: m.at };
      return { k: "user", key, text: m.text, at: m.at, images: Array.isArray(meta.images) ? (meta.images as string[]) : undefined };
    case "assistant":
      return { k: "agent", key, text: m.text, at: m.at };
    case "error":
      return { k: "error", key, text: m.text };
    case "note":
      return { k: "note", key, text: m.text };
    case "card":
      return meta.card ? { k: "card", key, card: meta.card as { type: string } } : null;
    case "build":
      return { k: "build", key, id: Number(meta.build ?? 0), stage: String(meta.stage ?? "done"), app: typeof meta.app === "string" ? meta.app : undefined, edit: meta.kind === "edit", label: m.text };
    case "mail":
      return { k: "mail", key, text: m.text, out: false, who: name(meta.from), kind: typeof meta.kind === "string" ? meta.kind : undefined };
    case "mail_out":
      return { k: "mail", key, text: m.text, out: true, who: name(meta.to), kind: typeof meta.kind === "string" ? meta.kind : undefined };
    default:
      return null;
  }
}

function ChatWindow({ r1, agent, agents, chatId, onChat }: { r1: R1Info; agent: Agent; agents: Agent[]; chatId: string | null; onChat: (c: string) => void }) {
  const [items, setItems] = useState<Item[]>([]);
  const [chat, setChat] = useState<Chat | null>(null);
  const [stream, setStream] = useState("");
  const [busy, setBusy] = useState(false);
  const [tool, setTool] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const chatRef = useRef<string | null>(chatId);
  // A new chat's brain and, on a computer, its mode (T-014): its agent's, unless you pick others before your first words.
  const [pick, setPick] = useState({ brain: agent.brain, mode: agent.mode ?? "manual" });
  /** A chat this window just made: it's empty, so it isn't loaded (that would race its first reply). */
  const fresh = useRef<string | null>(null);
  /** Photos sent with the message on its way, shown with it when it lands. */
  const sentPhotos = useRef<string[] | undefined>(undefined);
  const scroller = useRef<HTMLDivElement>(null);
  // Agents' names for their messages, through a ref: the list reloads with every state change, and the chat mustn't.
  const agentsRef = useRef(agents);
  agentsRef.current = agents;
  const name = useCallback((id: unknown) => agentsRef.current.find((a) => a.id === id)?.name ?? "another agent", []);

  const seen = useCallback((id: string) => void api(r1.id, "POST", `/api/chats/${id}/seen`).catch(() => undefined), [r1.id]);

  // The chat opened (or a new one): its messages, whether it's answering, and it's seen.
  useEffect(() => {
    chatRef.current = chatId;
    setStream("");
    setTool(null);
    setErr(null);
    if (!chatId) {
      setItems([]);
      setChat(null);
      setBusy(false);
      setPick({ brain: agent.brain, mode: agent.mode ?? "manual" });
      return;
    }
    if (fresh.current === chatId) return;
    setLoading(true);
    let gone = false;
    api<{ chat: Chat; messages: Message[]; busy: boolean }>(r1.id, "GET", `/api/chats/${chatId}`)
      .then((r) => {
        if (gone) return;
        setChat(r.chat);
        setItems(r.messages.map((m) => fromMessage(m, name)).filter((x): x is Item => !!x));
        setBusy(r.busy);
        seen(chatId);
      })
      .catch((e) => !gone && setErr(e instanceof Error ? e.message : String(e)))
      .finally(() => !gone && setLoading(false));
    return () => {
      gone = true;
    };
    // Not on the agent's brain or mode: they're only where a new chat starts, and a change mustn't reload the chat open.
  }, [r1.id, chatId, name, seen]);

  useR1Events(r1.id, (e) => {
    const id = chatRef.current;
    if (e.type === "agent_state" && e.agent === agent.id && (!e.chat || e.chat === id)) {
      const s = String(e.state ?? "idle");
      if (e.chat === id || !BUSY.includes(s)) setBusy(BUSY.includes(s));
      if (!BUSY.includes(s)) setTool(null);
      return;
    }
    if (e.type === "unread" && id && e.chat === id && Number(e.count) > 0) {
      seen(id);
      return;
    }
    if (e.type === "build") {
      setItems((xs) => xs.map((x) => (x.k === "build" && x.id === Number(e.id) ? { ...x, stage: String(e.stage), app: (e.app as string) ?? x.app, error: e.error as string | undefined } : x)));
      return;
    }
    if (e.type === "approval_asked") {
      const a = e.approval as { agent?: string; title?: string; id?: string } | undefined;
      if (a?.agent === agent.id && id) setItems((xs) => [...xs, { k: "ask", key: `ask${a.id}`, title: a.title ?? "something" }]);
      return;
    }
    if (e.type !== "chat" || !id || e.chat !== id) return;
    const key = `e${Date.now()}${Math.random()}`;
    const text = typeof e.text === "string" ? e.text : "";
    switch (e.kind) {
      case "user":
        setItems((xs) => [...xs, { k: "user", key, text, images: sentPhotos.current, at: Date.now() }]);
        // A new chat takes its first words as its title, as r1d names it.
        setChat((c) => (c && c.title === "New chat" ? { ...c, title: text.length > 40 ? `${text.slice(0, 40).trimEnd()}…` : text } : c));
        sentPhotos.current = undefined;
        setBusy(true);
        break;
      case "routine":
        setItems((xs) => [...xs, { k: "routine", key, title: text, at: Date.now() }]);
        setBusy(true);
        break;
      case "delta":
        setStream((s) => s + text);
        setBusy(true);
        break;
      case "tool":
        setStream("");
        setTool(text);
        setItems((xs) => [...xs, { k: "tool", key, name: text }]);
        break;
      case "card":
        if (e.card) setItems((xs) => [...xs, { k: "card", key, card: e.card as { type: string } }]);
        break;
      case "build":
        setItems((xs) => [...xs, { k: "build", key, id: Number(e.build ?? 0), stage: "writing", app: e.app as string | undefined, edit: !!e.app }]);
        break;
      case "done":
        setStream("");
        setTool(null);
        setBusy(false);
        if (text) setItems((xs) => [...xs, { k: "agent", key, text, at: Date.now() }]);
        break;
      case "error":
        setStream("");
        setTool(null);
        setBusy(false);
        setItems((xs) => [...xs, { k: "error", key, text: text || "something went wrong" }]);
        break;
      case "note":
        setItems((xs) => [...xs, { k: "note", key, text }]);
        break;
      // You stopped it (here or on the R1): what it had said by then, and a note.
      case "stopped":
        setStream("");
        setTool(null);
        setBusy(false);
        setItems((xs) => [...xs, ...(text ? [{ k: "agent" as const, key, text, at: Date.now() }] : []), { k: "note" as const, key: `${key}s`, text: "stopped" }]);
        break;
      case "mail":
      case "mail_out": {
        const m = (e.mail ?? {}) as Record<string, unknown>;
        const out = e.kind === "mail_out";
        setItems((xs) => [...xs, { k: "mail", key, text, out, who: name(out ? m.to : m.from), kind: m.kind as string | undefined }]);
        break;
      }
    }
  });

  // Down to the newest as it comes in, unless you've scrolled up to read.
  const pinned = useRef(true);
  useEffect(() => {
    const el = scroller.current;
    if (el && pinned.current) el.scrollTo({ top: el.scrollHeight, behavior: items.length > 0 ? "smooth" : "auto" });
  }, [items, stream, busy]);

  // Stops its turn (r1d's crate::stops): it stops wherever it runs, and the chat says so.
  const stop = () => {
    const id = chatRef.current;
    if (id) void api(r1.id, "POST", `/api/chats/${id}/stop`).catch((e) => setErr(e instanceof Error ? e.message : String(e)));
  };

  const send = async (text: string, photos: File[]) => {
    setErr(null);
    pinned.current = true;
    let id = chatRef.current;
    try {
      if (!id) {
        const r = await api<{ chat: Chat }>(r1.id, "POST", "/api/chats", { agent: agent.id, brain: pick.brain, ...(agent.computer ? { mode: pick.mode } : {}) });
        id = r.chat.id;
        fresh.current = id;
        chatRef.current = id;
        setChat(r.chat);
        onChat(id);
      }
      const images: string[] = [];
      for (const f of photos) {
        const r = await upload<{ media: { id: string } }>(r1.id, "POST", "/api/media?source=chat", "image/jpeg", await asJpeg(f));
        images.push(r.media.id);
      }
      sentPhotos.current = images.length ? images : undefined;
      await api(r1.id, "POST", `/api/chats/${id}/messages`, { text, images });
      return true;
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
      return false;
    }
  };

  const where = agent.computer ? `on ${agent.computer.name ?? "a computer"}` : "on your r1";
  const away = agent.computer && !agent.computer.connected;
  return (
    <section className="chat-main">
      <header className="chat-head">
        <Character form={agent.form} color={agent.color} finish={agent.finish} eyes={agent.eyes} wear={agent.wear} state={busy ? (stream ? "speaking" : "thinking") : "idle"} size={44} />
        <div className="chat-head-text">
          <div className="chat-head-name">{agent.name}</div>
          <div className="chat-head-sub keep-case">{chat?.title && chat.title !== "New chat" ? chat.title : "a new chat"}</div>
        </div>
        <div className="chips">
          <span className="chip">{chat?.brain ?? pick.brain}</span>
          <span className="chip">
            <span className={`dot ${away ? "" : "awake"}`} /> {where}
            {away ? " · away" : ""}
          </span>
          {(chat ? chat.mode : agent.computer && pick.mode) && <span className="chip">{MODES.find(([m]) => m === (chat?.mode ?? pick.mode))?.[1] ?? chat?.mode}</span>}
        </div>
      </header>
      <div
        className="chat-scroll"
        ref={scroller}
        onScroll={(e) => {
          const el = e.currentTarget;
          pinned.current = el.scrollHeight - el.scrollTop - el.clientHeight < 80;
        }}
      >
        {!chatId && items.length === 0 ? (
          <motion.div className="chat-hello" initial={{ opacity: 0, y: 10 }} animate={{ opacity: 1, y: 0, transition: soft }}>
            <Character form={agent.form} color={agent.color} finish={agent.finish} eyes={agent.eyes} wear={agent.wear} state="idle" size={120} />
            <div className="chat-hello-name">say something to {agent.name}</div>
            <div className="muted">{agent.role || "your agent"} · it works {where.replace("on your", "on the")}</div>
            <div className="chat-pick">
              <Seg id="new-brain" items={BRAINS} value={pick.brain} onChange={(brain) => setPick((p) => ({ ...p, brain }))} />
              {agent.computer && <Seg id="new-mode" items={MODES} value={pick.mode} onChange={(mode) => setPick((p) => ({ ...p, mode }))} />}
            </div>
          </motion.div>
        ) : (
          <div className="chat-items">
            {loading && items.length === 0 && <div className="muted chat-none">…</div>}
            <AnimatePresence initial={false}>
              {items.map((it) => (
                <motion.div key={it.key} className={`chat-item k-${it.k}`} initial={{ opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0, transition: soft }} layout="position">
                  <ItemView r1={r1} it={it} />
                </motion.div>
              ))}
            </AnimatePresence>
            {busy && (
              <motion.div className="chat-item k-agent" initial={{ opacity: 0 }} animate={{ opacity: 1 }}>
                {stream ? (
                  <div className="bubble agent streaming keep-case">
                    <Markdown text={stream} />
                  </div>
                ) : (
                  <div className="chat-typing">
                    <span className="chat-dots"><i /><i /><i /></span>
                    <span className="muted">{tool ? toolLabel(tool).toLowerCase() : `${agent.name.toLowerCase()} is thinking`}</span>
                  </div>
                )}
              </motion.div>
            )}
          </div>
        )}
      </div>
      {err && <p className="err chat-err keep-case">{err}</p>}
      <Composer agent={agent} busy={busy} r1={r1} chatId={chatId} onSend={send} onStop={stop} />
    </section>
  );
}

function ItemView({ r1, it }: { r1: R1Info; it: Item }) {
  switch (it.k) {
    case "user":
      return (
        <div className="bubble mine keep-case">
          {it.images && it.images.length > 0 && (
            <div className="chat-photos">
              {it.images.map((m) => (
                <Photo key={m} r1={r1} id={m} />
              ))}
            </div>
          )}
          <div className="bubble-text">{it.text}</div>
        </div>
      );
    case "agent":
      return (
        <div className="bubble agent keep-case">
          <Markdown text={it.text} />
        </div>
      );
    case "error":
      return <div className="chat-error keep-case">{it.text}</div>;
    case "note":
      return <div className="chat-note keep-case">{it.text}</div>;
    case "routine":
      return (
        <div className="chat-note keep-case">
          {it.title} · {runAt(it.at)}
        </div>
      );
    case "tool":
      return (
        <div className="chat-tool">
          <Icon name="lightning" size={13} />
          {toolLabel(it.name).toLowerCase()}
        </div>
      );
    case "card":
      return <ChatCard card={it.card} />;
    case "build":
      return <BuildLine stage={it.stage} app={it.app} edit={it.edit} error={it.error} label={it.label} />;
    case "mail":
      return <MailLine text={it.text} out={it.out} who={it.who} kind={it.kind} />;
    case "ask":
      return (
        <div className="chat-ask">
          <Icon name="device-mobile" size={16} />
          <span className="keep-case">asked on your r1: {it.title}. answer it there.</span>
        </div>
      );
  }
}

function Composer({ r1, agent, chatId, busy, onSend, onStop }: { r1: R1Info; agent: Agent; chatId: string | null; busy: boolean; onSend: (text: string, photos: File[]) => Promise<boolean>; onStop: () => void }) {
  const [text, setText] = useState("");
  const [photos, setPhotos] = useState<File[]>([]);
  const [sending, setSending] = useState(false);
  const [over, setOver] = useState(false);
  const box = useRef<HTMLTextAreaElement>(null);
  const warmed = useRef<string | null | undefined>(undefined);
  const previews = usePreviews(photos);

  useEffect(() => box.current?.focus(), [agent.id, chatId]);
  // Its height follows what's typed, up to a point.
  useEffect(() => {
    const el = box.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 200)}px`;
  }, [text]);

  const add = (files: FileList | File[] | null) => {
    const imgs = Array.from(files ?? []).filter((f) => f.type.startsWith("image/"));
    if (imgs.length) setPhotos((p) => [...p, ...imgs].slice(0, 4));
  };
  const go = async () => {
    const t = text.trim();
    if (!t || busy || sending) return;
    setSending(true);
    const ok = await onSend(t, photos);
    setSending(false);
    if (ok) {
      setText("");
      setPhotos([]);
    }
  };
  // The agent's process starts as you start typing, so the answer comes sooner (as holding the button does on the R1).
  const warm = () => {
    if (warmed.current === chatId) return;
    warmed.current = chatId;
    void api(r1.id, "POST", "/api/chats/warm", { agent: agent.id, chat: chatId }).catch(() => undefined);
  };

  return (
    <div
      className={`composer ${over ? "over" : ""}`}
      onDragOver={(e: DragEvent) => {
        e.preventDefault();
        setOver(true);
      }}
      onDragLeave={() => setOver(false)}
      onDrop={(e: DragEvent) => {
        e.preventDefault();
        setOver(false);
        add(e.dataTransfer.files);
      }}
    >
      {photos.length > 0 && (
        <div className="composer-photos">
          {previews.map((u, i) => (
            <span key={u} className="composer-photo">
              <img src={u} alt="" />
              <button className="composer-photo-x" onClick={() => setPhotos((p) => p.filter((_, k) => k !== i))} title="leave it out">
                <Icon name="x" weight="bold" size={11} />
              </button>
            </span>
          ))}
        </div>
      )}
      <div className="composer-row">
        <label className="icon-btn composer-attach" title="send a photo">
          <Icon name="image" weight="bold" size={17} />
          <input type="file" accept="image/*" multiple hidden onChange={(e) => { add(e.target.files); e.target.value = ""; }} />
        </label>
        <textarea
          ref={box}
          className="composer-box keep-case"
          rows={1}
          value={text}
          placeholder={`message ${agent.name}…`}
          onChange={(e) => {
            setText(e.target.value);
            warm();
          }}
          onPaste={(e: ClipboardEvent) => add(e.clipboardData.files)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
              e.preventDefault();
              void go();
            }
          }}
        />
        {busy && chatId ? (
          <motion.button className="composer-send composer-stop" onClick={onStop} whileTap={{ scale: 0.92 }} title={`stop ${agent.name}`}>
            <Icon name="stop" weight="fill" size={15} />
          </motion.button>
        ) : (
          <motion.button className="composer-send" disabled={!text.trim() || busy || sending} onClick={() => void go()} whileTap={{ scale: 0.92 }} title="send (enter)">
            <Icon name={sending ? "circle-notch" : "arrow-up"} weight="bold" size={18} />
          </motion.button>
        )}
      </div>
    </div>
  );
}

/** Object URLs for the photos about to go, let go of when they change. */
function usePreviews(files: File[]): string[] {
  const [urls, setUrls] = useState<string[]>([]);
  useEffect(() => {
    const u = files.map((f) => URL.createObjectURL(f));
    setUrls(u);
    return () => u.forEach((x) => URL.revokeObjectURL(x));
  }, [files]);
  return urls;
}
