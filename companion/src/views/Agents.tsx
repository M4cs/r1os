// An R1's agents from the computer (docs/specs/agents.md, ADR-0043, ADR-0045): who each one is and what it's for,
// how it looks (form, colour, finish, eyes, what it wears), what it may change, what it remembers and its chats.
// Drag them into order; add one or let one go. A profile is saved whole (`PUT /api/agents/{id}`).

import { Icon } from "@r1os/icons";
import { LayoutGroup, Reorder, motion } from "motion/react";
import { useEffect, useState } from "react";
import { COLORS, Character, EYE_STYLES, FINISHES, FORMS, ITEMS, itemsIn, SLOTS } from "@shell/ui/Character";
import { type Agent, type R1Info, api, pickFolder, useGet, useLink, usePcStatus } from "../link";
import { soft, spring } from "../motion";
import { Empty, Field, Sheet, Twice, ago } from "./parts";
import type { R1Data } from "./R1View";
import { Item, Section, Seg } from "./Settings";

const PERSONALITIES: [string, string][] = [["friendly", "friendly"], ["concise", "concise"], ["playful", "playful"], ["calm", "calm"]];
const BRAINS: [string, string][] = [["claude", "claude"], ["codex", "codex"]];
const AREAS: { id: string; label: string; sub: string }[] = [
  { id: "look", label: "look and home", sub: "colours, widgets, home" },
  { id: "device", label: "the r1's settings", sub: "brightness, sleep, volume" },
  { id: "apps", label: "apps", sub: "settings and permissions" },
];
const LEVELS: [string, string][] = [["ask", "asks you"], ["allow", "allowed"], ["never", "never"]];
const MAX_AGENTS = 8;

/** What r1d keeps of an agent (crate::core::Agent): the whole of it goes back on a change. */
function profile(a: Agent) {
  const { id, name, form, color, finish, eyes, wear, personality, custom, role, brain, can, works_on, folders, mode } = a;
  return {
    id, name, form, color, finish, eyes: eyes ?? "classic", wear: wear ?? [], personality, custom: custom ?? "", role, brain, can: can ?? {},
    works_on: works_on ?? "r1", folders: folders ?? [], mode: mode ?? "manual",
  };
}

export function Agents({ r1, data }: { r1: R1Info; data: R1Data }) {
  const [picked, setPicked] = useState<string | null>(null);
  const [order, setOrder] = useState<string[]>([]);
  const [edits, setEdits] = useState<Record<string, Partial<Agent>>>({});
  const [err, setErr] = useState<string | null>(null);
  useEffect(() => {
    setOrder(data.agents.map((a) => a.id));
    setEdits({});
  }, [data.agents]);
  const agents = order.map((id) => data.agents.find((a) => a.id === id)).filter((a): a is Agent => !!a).map((a) => ({ ...a, ...edits[a.id] }));
  const agent = agents.find((a) => a.id === picked) ?? agents.find((a) => a.id === data.focus?.id) ?? agents[0] ?? null;

  const change = (a: Agent, c: Partial<Agent>) => {
    setEdits((e) => ({ ...e, [a.id]: { ...e[a.id], ...c } }));
    api(r1.id, "PUT", `/api/agents/${a.id}`, profile({ ...a, ...c })).catch((e) => {
      setErr(e instanceof Error ? e.message : String(e));
      setEdits((x) => ({ ...x, [a.id]: {} }));
    });
  };
  const add = () => {
    const form = FORMS[Math.floor(Math.random() * FORMS.length)].id;
    const colors = Object.keys(COLORS);
    api<{ agent: Agent }>(r1.id, "POST", "/api/agents", { name: "new agent", form, color: colors[Math.floor(Math.random() * colors.length)] })
      .then((r) => setPicked(r.agent.id))
      .catch((e) => setErr(e instanceof Error ? e.message : String(e)));
  };
  const remove = (a: Agent) => {
    api(r1.id, "DELETE", `/api/agents/${a.id}`)
      .then(() => setPicked(null))
      .catch((e) => setErr(e instanceof Error ? e.message : String(e)));
  };
  const saveOrder = () => api(r1.id, "PUT", "/api/agents/order", { ids: order }).catch((e) => setErr(String(e)));

  return (
    <div className="agents-page">
      <Reorder.Group axis="x" values={order} onReorder={setOrder} className="agent-strip" as="div">
        <LayoutGroup id="agent-pick">
          {agents.map((a) => (
            <Reorder.Item key={a.id} value={a.id} as="div" className="agent-pick-wrap" onDragEnd={saveOrder} whileDrag={{ scale: 1.06, zIndex: 2 }}>
              <button className={`agent-pick ${a.id === agent?.id ? "on" : ""}`} onClick={() => setPicked(a.id)}>
                {a.id === agent?.id && <motion.span className="agent-pick-ring" layoutId="ring" transition={spring} />}
                <Character form={a.form} color={a.color} finish={a.finish} eyes={a.eyes} wear={a.wear} state={(a.state as never) ?? "idle"} unread={a.unread ?? 0} size={72} />
                <span className="agent-pick-name">{a.name}</span>
              </button>
            </Reorder.Item>
          ))}
        </LayoutGroup>
        {agents.length < MAX_AGENTS && (
          <motion.button className="agent-pick add" onClick={add} whileTap={{ scale: 0.95 }} title="a new agent">
            <span className="agent-add-ring">
              <Icon name="plus" weight="bold" size={22} />
            </span>
            <span className="agent-pick-name">new</span>
          </motion.button>
        )}
      </Reorder.Group>
      <p className="muted small strip-hint">drag them into the order the wheel goes through them</p>
      {err && <p className="err">{err}</p>}
      {agent && <AgentCard key={agent.id} r1={r1} agent={agent} change={(c) => change(agent, c)} onDelete={() => remove(agent)} lastOne={agents.length <= 1} />}
    </div>
  );
}

function AgentCard({ r1, agent: a, change, onDelete, lastOne }: { r1: R1Info; agent: Agent; change: (c: Partial<Agent>) => void; onDelete: () => void; lastOne: boolean }) {
  const wear = a.wear ?? [];
  const wearOf = (kind: string[]) => wear.find((w) => kind.includes(w)) ?? null;
  const putOn = (kind: string[], item: string | null) => change({ wear: [...wear.filter((w) => !kind.includes(w)), ...(item ? [item] : [])] });
  /** The agent as it is, with one thing different (a tile). */
  const tile = (c: Partial<Agent>, size = 52) => (
    <Character form={c.form ?? a.form} color={c.color ?? a.color} finish={c.finish ?? a.finish} eyes={c.eyes ?? a.eyes} wear={c.wear ?? wear} state="idle" size={size} />
  );
  return (
    <motion.div className="list-card" initial={{ opacity: 0, y: 12 }} animate={{ opacity: 1, y: 0, transition: soft }}>
      <Section title="who it is" />
      <Item label="name">
        <Field value={a.name} max={24} onSave={(name) => name && change({ name })} />
      </Item>
      <Item label="what it's for" sub="a few words; empty is a bit of everything">
        <Field value={a.role ?? ""} max={120} placeholder="a bit of everything" onSave={(role) => change({ role })} wide />
      </Item>
      <Item label="personality">
        <Seg id={`pers-${a.id}`} items={PERSONALITIES} value={a.personality} onChange={(personality) => change({ personality })} />
      </Item>
      <Item label="anything else" sub="one line it keeps in mind">
        <Field value={a.custom ?? ""} max={200} placeholder="say hi in spanish" onSave={(custom) => change({ custom })} wide />
      </Item>
      <Item label="brain" sub="new chats use it; a chat keeps the one it started with">
        <Seg id={`brain-${a.id}`} items={BRAINS} value={a.brain} onChange={(brain) => change({ brain })} />
      </Item>

      <Section title="how it looks" />
      <Item label="form" wide>
        <Tiles id={`form-${a.id}`} items={FORMS.map((f) => f.id)} on={a.form} onPick={(form) => change({ form })} render={(form) => tile({ form })} />
      </Item>
      <Item label="colour" wide>
        <LayoutGroup id={`color-${a.id}`}>
          <div className="swatches">
            {Object.entries(COLORS).map(([name, hex]) => (
              <motion.button key={name} className="swatch" title={name} style={{ background: hex }} whileHover={{ scale: 1.12 }} whileTap={{ scale: 0.92 }} onClick={() => change({ color: name })}>
                {a.color === name && <motion.span className="swatch-ring" layoutId="ring" transition={spring} style={{ borderColor: hex }} />}
              </motion.button>
            ))}
            {/* One of its own (T-060): any colour, as #rrggbb. */}
            <label className="swatch swatch-own" title="your own" style={{ background: a.color.startsWith("#") ? a.color : undefined }}>
              <input type="color" value={a.color.startsWith("#") ? a.color : "#8a8f98"} onChange={(e) => change({ color: e.target.value })} />
              {a.color.startsWith("#") && <motion.span className="swatch-ring" layoutId="ring" transition={spring} style={{ borderColor: a.color }} />}
            </label>
          </div>
        </LayoutGroup>
      </Item>
      <Item label="finish" wide>
        <Tiles id={`finish-${a.id}`} items={FINISHES.map((f) => f.id)} on={a.finish} onPick={(finish) => change({ finish })} render={(finish) => tile({ finish })} />
      </Item>
      <Item label="eyes" wide>
        <Tiles
          id={`eyes-${a.id}`}
          items={EYE_STYLES.map((e) => e.id)}
          on={a.eyes ?? "classic"}
          onPick={(eyes) => change({ eyes })}
          render={(eyes) => tile({ eyes })}
          label={(e) => EYE_STYLES.find((x) => x.id === e)?.label.toLowerCase() ?? e}
        />
      </Item>
      {/* What it wears (T-062): one thing in each of the wardrobe's slots. */}
      {SLOTS.map((slot) => {
        const kind = itemsIn(slot.id).map((i) => i.id);
        return (
          <Item key={slot.id} label={slot.label.toLowerCase()} wide>
            <Tiles
              id={`${slot.id}-${a.id}`}
              items={["", ...kind]}
              on={wearOf(kind) ?? ""}
              onPick={(w) => putOn(kind, w || null)}
              render={(w) => tile({ wear: [...wear.filter((x) => !kind.includes(x)), ...(w ? [w] : [])] })}
              label={(w) => ITEMS.find((i) => i.id === w)?.label.toLowerCase() ?? "nothing"}
            />
          </Item>
        );
      })}

      <Where r1={r1} agent={a} change={change} />

      <Section title="what it can change" />
      {AREAS.map((area) => (
        <Item key={area.id} label={area.label} sub={area.sub}>
          <CanSeg r1={r1} agent={a} area={area.id} />
        </Item>
      ))}

      <Memories r1={r1} agent={a} />
      <Chats r1={r1} agent={a} />

      <Section title="" />
      <Item label={`let ${a.name} go`} sub={lastOne ? "your r1 needs at least one agent" : "its chats, its own memories and its files go too"}>
        <Twice label="delete" armed={`click again: delete ${a.name}`} onConfirm={onDelete} disabled={lastOne} icon="trash" />
      </Item>
    </motion.div>
  );
}

/** How an agent on a computer asks (r1d's MODES): new chats start in it, and each keeps its own. */
const MODES: [string, string][] = [["manual", "asks"], ["acceptEdits", "edits freely"], ["auto", "auto"], ["plan", "plans only"]];
const MODE_WORDS: Record<string, string> = {
  manual: "anything it isn't allowed asks, as its levels below say",
  acceptEdits: "changes files in its folders without asking; commands still ask",
  auto: "claude decides what's safe to do without asking",
  plan: "plans and explains, changes nothing",
};

const PC_AREAS: { id: string; label: string; sub: string }[] = [
  { id: "files", label: "changing files", sub: "in its folders" },
  { id: "commands", label: "running commands", sub: "programs and scripts" },
  { id: "outside", label: "anything outside", sub: "other files, opening apps" },
  { id: "browser", label: "using your browser", sub: "clicking and typing on pages (ADR-0053)" },
];

/** A folder named for the agent, as the companion makes it ("Archer" → "archer"). */
function folderName(name: string): string {
  return name.trim().toLowerCase().replace(/[^\p{L}\p{N}_-]/gu, "-").replace(/^-+|-+$/g, "") || "agent";
}

/** Where it works (docs/specs/companion.md §7.2): the R1, or this computer, with its folders and what it may do there. */
function Where({ r1, agent: a, change }: { r1: R1Info; agent: Agent; change: (c: Partial<Agent>) => void }) {
  const link = useLink();
  const pc = usePcStatus();
  const mine = link ? `computer:${link.me.id}` : null;
  const now = a.works_on ?? "r1";
  const other = now !== "r1" && now !== mine;
  const items: [string, string][] = [["r1", "the r1"], ...(mine ? [[mine, "this computer"] as [string, string]] : []), ...(other ? [[now, a.computer?.name?.toLowerCase() ?? "another computer"] as [string, string]] : [])];
  const here = now === mine;
  const folders = a.folders ?? [];
  const add = async () => {
    const f = await pickFolder();
    if (f && !folders.includes(f)) change({ folders: [...folders, f] });
  };
  return (
    <>
      <Section title="where it works" />
      <Item
        label="works on"
        sub={
          now === "r1"
            ? "in its own space on the r1"
            : a.computer && !a.computer.connected
              ? `${a.computer.name ?? "its computer"} isn't connected: what you say waits a few minutes for it`
              : "you talk to it on the r1; it works there, in its folders"
        }
      >
        <Seg id={`where-${a.id}`} items={items} value={now} onChange={(works_on) => change({ works_on })} />
      </Item>
      {here && (
        <>
          {a.brain === "codex" && pc && !pc.codex.signed_in && <p className="err small list-foot">codex isn't signed in here: sign in on this computer's page</p>}
          {a.brain !== "codex" && pc && !pc.claude.path && <p className="err small list-foot">claude code isn't installed here: npm i -g @anthropic-ai/claude-code</p>}
          {a.brain !== "codex" && pc?.claude.path && !pc.claude.signed_in && <p className="err small list-foot">claude code isn't signed in here: run claude once and sign in</p>}
          <Item label="its own folder" sub="where it starts (made when it first works here)">
            <span className="value keep-case mono small">{pc ? `${pc.workspaces}\\${folderName(a.name)}` : "…"}</span>
          </Item>
          {folders.map((f) => (
            <Item key={f} label={f} sub="a folder it can work in">
              <button className="btn small" onClick={() => change({ folders: folders.filter((x) => x !== f) })}>
                remove
              </button>
            </Item>
          ))}
          <Item label="add a folder" sub="it can read and change what's in it">
            <button className="btn small" onClick={() => void add()}>
              <Icon name="folder-plus" weight="bold" size={14} /> choose a folder
            </button>
          </Item>
          <Item label="new chats" sub={MODE_WORDS[a.mode ?? "manual"]}>
            <Seg id={`mode-${a.id}`} items={MODES} value={a.mode ?? "manual"} onChange={(mode) => change({ mode })} />
          </Item>
          {PC_AREAS.map((area) => (
            <Item key={area.id} label={area.label} sub={area.sub}>
              <CanSeg r1={r1} agent={a} area={area.id} />
            </Item>
          ))}
        </>
      )}
    </>
  );
}

/** Choices drawn as the agent would look with each. */
function Tiles({ id, items, on, onPick, render, label }: { id: string; items: string[]; on: string; onPick: (v: string) => void; render: (v: string) => React.ReactNode; label?: (v: string) => string }) {
  return (
    <LayoutGroup id={id}>
      <div className="char-grid">
        {items.map((v) => (
          <motion.button key={v || "none"} className={`choice ${v === on ? "on" : ""}`} onClick={() => onPick(v)} whileHover={{ y: -2 }} whileTap={{ scale: 0.95 }}>
            {v === on && <motion.span className="choice-ring" layoutId="ring" transition={spring} />}
            {render(v)}
            <span className="choice-label">{label ? label(v) : v}</span>
          </motion.button>
        ))}
      </div>
    </LayoutGroup>
  );
}

/** An agent's level for an area (ADR-0045): unset, it's the area's default (changing files in its folders: allowed). */
function CanSeg({ r1, agent, area }: { r1: R1Info; agent: Agent; area: string }) {
  const saved = agent.can?.[area] ?? (area === "files" ? "allow" : "ask");
  const [level, setLevel] = useState(saved);
  useEffect(() => setLevel(saved), [saved]);
  return (
    <Seg
      id={`can-${agent.id}-${area}`}
      items={LEVELS}
      value={level}
      onChange={(l) => {
        setLevel(l as typeof level);
        api(r1.id, "PUT", `/api/agents/${agent.id}/can`, { area, level: l }).catch(() => setLevel(saved));
      }}
    />
  );
}

interface Memory {
  id: number;
  text: string;
  at: number;
  shared: boolean;
  by?: string;
}

/** What it remembers: its own facts, and the ones every agent shares. */
function Memories({ r1, agent }: { r1: R1Info; agent: Agent }) {
  const { data, reload } = useGet<{ memories: Memory[] }>(r1.id, `/api/memories?agent=${agent.id}`, ["agents_changed"]);
  const list = data?.memories ?? [];
  const forget = (m: Memory) => api(r1.id, "DELETE", `/api/memories/${m.id}`).then(reload);
  return (
    <>
      <Section title={`what it remembers${list.length ? ` · ${list.length}` : ""}`} />
      {data && list.length === 0 && <Empty icon="brain">nothing yet: it remembers what you tell it to</Empty>}
      {list.map((m) => (
        <Item key={m.id} label={m.text} sub={`${m.shared ? "every agent knows this · " : ""}${ago(m.at)}`}>
          <Twice label="forget" armed="click again: forget" onConfirm={() => void forget(m)} />
        </Item>
      ))}
    </>
  );
}

interface Chat {
  id: string;
  title: string;
  updated: number;
  last?: string;
  unread?: number;
  brain: string;
}
interface Message {
  id: number;
  role: string;
  text: string;
  at: number;
  meta?: Record<string, unknown>;
}

/** Its chats with you, newest first: read one, or delete it. You talk to it on the R1. */
function Chats({ r1, agent }: { r1: R1Info; agent: Agent }) {
  const { data, reload } = useGet<{ chats: Chat[] }>(r1.id, `/api/chats?agent=${agent.id}`, ["unread", "agent_state"]);
  const [open, setOpen] = useState<Chat | null>(null);
  const [messages, setMessages] = useState<Message[] | null>(null);
  const list = data?.chats ?? [];
  const read = (c: Chat) => {
    setOpen(c);
    setMessages(null);
    api<{ messages: Message[] }>(r1.id, "GET", `/api/chats/${c.id}`).then((r) => setMessages(r.messages));
  };
  return (
    <>
      <Section title={`chats${list.length ? ` · ${list.length}` : ""}`} />
      {data && list.length === 0 && <Empty icon="chat-circle">no chats yet: hold the button on your r1 to talk</Empty>}
      {list.slice(0, 20).map((c) => (
        <Item key={c.id} label={c.title || "a chat"} sub={`${ago(c.updated)} · ${c.brain}${c.unread ? ` · ${c.unread} new` : ""}`}>
          <div className="row-buttons">
            <button className="btn small" onClick={() => read(c)}>
              read
            </button>
            <Twice label="delete" armed="click again" onConfirm={() => void api(r1.id, "DELETE", `/api/chats/${c.id}`).then(reload)} />
          </div>
        </Item>
      ))}
      <Sheet open={!!open} onClose={() => setOpen(null)} title={open?.title || "a chat"}>
        {!messages ? (
          <p className="muted">…</p>
        ) : (
          <div className="messages">
            {messages
              // Not a routine's prompt (T-092): only what you said yourself, and the replies.
              .filter((m) => (m.role === "user" && !m.meta?.routine) || m.role === "assistant")
              .map((m) => (
                <motion.div key={m.id} className={`message ${m.role === "user" ? "mine" : ""}`} initial={{ opacity: 0, y: 6 }} animate={{ opacity: 1, y: 0 }}>
                  <div className="message-text">{m.text}</div>
                  <div className="message-at muted">{ago(m.at)}</div>
                </motion.div>
              ))}
          </div>
        )}
      </Sheet>
    </>
  );
}
