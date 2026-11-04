// A conversation with one of your agents (docs/specs/experience.md §2.3). You talk by holding the
// side button (voice.tsx; there's no keyboard: the R1's screen has no room for one). Replies stream
// in; a build the agent starts shows as a live card. A chat runs as long as you keep it: when r1d
// tidies a long one into a fresh session, a quiet note marks the spot.

import { useEffect, useRef, useState } from "react";
import { chatApi, type Agent, type Message } from "./api";
import { appsApi } from "./apps/host";
import { useAppInput } from "./input";
import { onEvent } from "./r1";
import { toolLabel } from "./tool-labels";
import { activityOf, Character, colorOf, type Activity, type CharState } from "./ui/Character";

type Item =
  | { kind: "msg"; m: Pick<Message, "role" | "text"> & { key: string } }
  /** r1d tidied the context here (docs/specs/agents.md §3.3): the chat goes on, in a fresh session. */
  | { kind: "note"; key: string; text: string }
  | { kind: "tool"; key: string; name: string }
  | {
      kind: "build";
      key: string;
      id: number;
      stage: string;
      section?: string;
      app?: string;
      error?: string;
      label?: string;
      edit?: boolean;
      said?: string;
      deleted?: string[];
      undone?: boolean;
    };

/** A stored message as a chat item: build results come back as their card. */
function fromMessage(m: Message): Item {
  if (m.role === "build") {
    const meta = m.meta ?? {};
    return {
      kind: "build",
      key: `m${m.id}`,
      id: Number(meta.build ?? 0),
      stage: String(meta.stage ?? "done"),
      app: typeof meta.app === "string" ? meta.app : undefined,
      label: m.text,
      edit: meta.kind === "edit",
      deleted: Array.isArray(meta.deleted) ? (meta.deleted as string[]) : undefined,
    };
  }
  if (m.role === "note") return { kind: "note", key: `m${m.id}`, text: m.text };
  return { kind: "msg", m: { role: m.role, text: m.text, key: `m${m.id}` } };
}

/** Replies are for a tiny screen (and soon a voice): markdown links become their text, emphasis goes. */
function plain(text: string): string {
  return text
    .replace(/\[([^\]]+)\]\((?:https?:)?[^)]*\)/g, "$1")
    .replace(/\*\*([^*]+)\*\*/g, "$1")
    .replace(/(^|\s)\*([^*\n]+)\*(?=\s|[.,!?]|$)/g, "$1$2")
    .replace(/`([^`\n]+)`/g, "$1");
}


export function ChatView({
  id,
  say,
  sayApp,
  agent,
  onChat,
  onOpenApp,
}: {
  id?: string;
  /** Words to send as soon as the chat opens (voice from home or elsewhere). */
  say?: string;
  /** The app that was on screen when `say` was spoken. */
  sayApp?: string;
  agent: Agent;
  onChat: (id: string) => void;
  onOpenApp: (id: string) => void;
}) {
  const [chatId, setChatId] = useState<string | undefined>(id);
  const [items, setItems] = useState<Item[]>([]);
  const [stream, setStream] = useState("");
  const [busy, setBusy] = useState(false);
  const [state, setState] = useState<CharState>("idle");
  /** r1d is waiting for room in the box (other agents are running). */
  const [waiting, setWaiting] = useState(false);
  /** What it's doing while it works (its last tool). */
  const [activity, setActivity] = useState<Activity | undefined>(undefined);
  const list = useRef<HTMLDivElement>(null);
  const said = useRef(false);
  const idRef = useRef(chatId);
  idRef.current = chatId;

  useEffect(() => {
    if (!id) return;
    void chatApi.get(id).then((r) => {
      setItems(r.messages.map(fromMessage));
      setBusy(r.busy);
    });
  }, [id]);

  useEffect(
    () =>
      onEvent((ev) => {
        if (ev.type === "agent_state" && ev.chat === idRef.current) {
          setWaiting(ev.state === "waiting");
          setState(ev.state);
          if (ev.state === "idle" || ev.state === "error") setActivity(undefined);
        }
        // Replies here are seen while the chat is on screen (r1d counts the rest as unread).
        if (ev.type === "unread" && ev.chat === idRef.current && ev.count > 0) {
          seen();
          return;
        }
        if (ev.type === "build") {
          setItems((xs) =>
            xs.map((x) =>
              x.kind === "build" && x.id === ev.id
                ? { ...x, stage: ev.stage, section: ev.section ?? x.section, app: ev.app ?? x.app, error: ev.error, edit: ev.kind === "edit" || x.edit, said: ev.said ?? x.said, deleted: ev.deleted ?? x.deleted }
                : x,
            ),
          );
          return;
        }
        // Warm the agent as soon as the user starts talking here.
        if (ev.type === "voice" && ev.state === "listening") {
          void chatApi.warm(agent.id, idRef.current);
          return;
        }
        if (ev.type !== "chat" || ev.chat !== idRef.current) return;
        const key = `e${ev.seq}`;
        switch (ev.kind) {
          case "delta":
            setStream((s) => s + (ev.text ?? ""));
            break;
          case "tool":
            setActivity(activityOf(ev.text));
            setStream("");
            setItems((xs) => [...xs, { kind: "tool", key, name: ev.text ?? "" }]);
            break;
          case "build":
            setItems((xs) => [...xs, { kind: "build", key, id: ev.build ?? 0, stage: "writing", app: ev.app, edit: !!ev.app }]);
            break;
          case "done":
            setStream("");
            setBusy(false);
            if (ev.text) setItems((xs) => [...xs, { kind: "msg", m: { role: "assistant", text: ev.text ?? "", key } }]);
            break;
          // A routine's run with nothing to tell: r1d kept nothing of it.
          case "quiet":
            setStream("");
            setBusy(false);
            break;
          case "error":
            setStream("");
            setBusy(false);
            setItems((xs) => [...xs, { kind: "msg", m: { role: "error", text: ev.text ?? "Something went wrong", key } }]);
            break;
          case "note":
            setItems((xs) => [...xs, { kind: "note", key, text: ev.text ?? "" }]);
            break;
        }
      }),
    [],
  );

  useEffect(() => {
    list.current?.scrollTo({ top: list.current.scrollHeight, behavior: "smooth" });
  }, [items, stream]);

  // Seen when it opens, and when the screen comes back on with replies that came in while it was off.
  useEffect(() => {
    seen();
    const onVisible = () => seen();
    document.addEventListener("visibilitychange", onVisible);
    return () => document.removeEventListener("visibilitychange", onVisible);
  }, []);

  /** Marks the chat's replies seen, unless the screen is off. */
  function seen() {
    if (idRef.current && !document.hidden) void chatApi.seen(idRef.current);
  }

  const send = async (raw: string, app?: string) => {
    const text = raw.trim();
    if (!text || busy) return;
    setBusy(true);
    setState("thinking");
    setItems((xs) => [...xs, { kind: "msg", m: { role: "user", text, key: `u${Date.now()}` } }]);
    try {
      let cid = chatId;
      if (!cid) {
        cid = (await chatApi.create(agent.id)).id;
        setChatId(cid);
        idRef.current = cid;
        onChat(cid);
      }
      await chatApi.send(cid, text, app);
    } catch (e) {
      setBusy(false);
      setState("error");
      setItems((xs) => [...xs, { kind: "msg", m: { role: "error", text: e instanceof Error ? e.message : String(e), key: `x${Date.now()}` } }]);
    }
  };

  useEffect(() => {
    if (say && !said.current) {
      said.current = true;
      void send(say, sayApp);
    } else {
      void chatApi.warm(agent.id, id);
    }
  }, []);

  // Wheel scrolls the conversation; held-button words land here.
  useAppInput((ev) => {
    if (ev.type === "wheel") {
      list.current?.scrollBy({ top: ev.delta * 60, behavior: "smooth" });
      return true;
    }
    if (ev.type === "voice" && ev.state === "final" && ev.text?.trim()) {
      void send(ev.text);
      return true;
    }
    return false;
  });

  return (
    <div className="chat" style={{ ["--accent" as string]: colorOf(agent.color) }}>
      <div className="chat-head">
        <Character form={agent.form} color={agent.color} finish={agent.finish} eyes={agent.eyes} wear={agent.wear} state={busy ? (stream ? "speaking" : state === "idle" ? "thinking" : state) : "idle"} activity={activity} size={44} />
        <span className="chat-name">{agent.name}</span>
      </div>
      <div className="chat-list" ref={list}>
        {items.length === 0 && !stream && <div className="chat-empty muted">Hold the button and ask {agent.name} anything, or ask for an app.</div>}
        {items.map((it) =>
          it.kind === "msg" ? (
            <div key={it.m.key} className={`bubble ${it.m.role}`}>
              {it.m.role === "assistant" ? plain(it.m.text) : it.m.text}
            </div>
          ) : it.kind === "note" ? (
            <div key={it.key} className="chat-note">
              {it.text}
            </div>
          ) : it.kind === "tool" ? (
            <div key={it.key} className="tool-chip">
              {toolLabel(it.name)}
            </div>
          ) : (
            <div key={it.key} className={`build-card ${it.stage}${it.undone ? " undone" : ""}`}>
              <div className="build-title">
                {it.edit
                  ? it.stage === "done"
                    ? it.undone
                      ? "Change undone"
                      : "App changed"
                    : it.stage === "failed"
                      ? "Change didn't work"
                      : "Changing the app"
                  : it.stage === "done"
                    ? "App ready"
                    : it.stage === "failed"
                      ? "Build failed"
                      : "Building your app"}
              </div>
              <div className="muted small">
                {it.stage === "done"
                  ? (it.said ?? it.label ?? it.app)
                  : it.stage === "failed"
                    ? (it.error ?? it.label)
                    : it.stage === "repairing"
                      ? "fixing what the compiler found"
                      : (it.section ?? "starting")}
              </div>
              {it.stage === "done" && it.deleted && it.deleted.length > 0 && !it.undone && (
                <div className="small error">Deleted with its data: {it.deleted.join(", ")}</div>
              )}
              {it.stage === "done" && it.app && (
                <div className="build-actions">
                  <button className="primary" onClick={() => onOpenApp(it.app!)}>
                    Open
                  </button>
                  {it.edit && !it.undone && (
                    <button
                      className="ghost small"
                      onClick={() =>
                        void appsApi.undo(it.app!).then(() => setItems((xs) => xs.map((x) => (x.kind === "build" && x.key === it.key ? { ...x, undone: true } : x))))
                      }
                    >
                      Undo
                    </button>
                  )}
                </div>
              )}
            </div>
          ),
        )}
        {stream && <div className="bubble assistant streaming">{plain(stream)}</div>}
        {busy && !stream && <div className="typing"><span /><span /><span /></div>}
      </div>
      <footer className="chat-talk">
        <span className="talk-dot" />
        {busy ? (waiting ? "Waiting for another agent to finish…" : `${agent.name} is on it…`) : "Hold the button to talk"}
      </footer>
    </div>
  );
}
