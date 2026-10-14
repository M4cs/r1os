// An R1's connections from the computer (R8, ADR-0052): the apps its agents reach on your accounts. Claude agents have
// the connectors you added on claude.ai, Codex agents the apps on your ChatGPT account; r1os keeps no tokens. Signing in
// opens claude.ai in this computer's browser. Each is on for every agent unless turned off for one; reading runs, and
// anything that acts asks on the R1 (allow, always allow, don't allow). "Always" answers are listed here to take back.
// Skills (T-086): the agents' Agent Skills, added here by link (`npx skills add` in the R1's agent box), on or off.

import { Icon } from "@r1os/icons";
import { motion } from "motion/react";
import { useState } from "react";
import { type R1Info, api, openUrl, useGet } from "../link";
import { soft } from "../motion";
import { Empty, Twice } from "./parts";
import { Item, Section, Switch } from "./Settings";

interface Server {
  name: string;
  label: string;
  status: "connected" | "needs-auth" | "failed" | "pending";
  /** `claudeai` (a claude.ai connector), else one of yours added here with a key. */
  source: string;
  slug: string;
  reads: string[];
  acts: string[];
}
interface AgentRules {
  id: string;
  name: string;
  brain: "claude" | "codex";
  off: string[];
  always: string[];
}
interface State {
  servers: Server[];
  agents: AgentRules[];
  add_url: string;
  chatgpt_url: string;
}

const STATUS: Record<Server["status"], string> = { connected: "connected", "needs-auth": "needs signing in", failed: "couldn't connect", pending: "connecting" };
const words = (t: string) => t.replace(/_/g, " ");

export function Connections({ r1 }: { r1: R1Info }) {
  const { data, set } = useGet<State>(r1.id, "/api/connections", ["connections_changed"]);
  const [looking, setLooking] = useState(false);
  const [signing, setSigning] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const fail = (e: unknown) => setErr(e instanceof Error ? e.message : String(e));
  const check = () => {
    setLooking(true);
    api<State>(r1.id, "POST", "/api/connections/check")
      .then(set)
      .catch(fail)
      .finally(() => setLooking(false));
  };
  const signIn = (s: Server) => {
    setSigning(s.name);
    api<{ url: string }>(r1.id, "POST", "/api/connections/login", { server: s.name })
      .then((r) => openUrl(r.url))
      .catch(fail)
      .finally(() => setSigning(null));
  };
  const turn = (a: AgentRules, s: Server, on: boolean) => api<State>(r1.id, "POST", "/api/connections/agent", { agent: a.id, server: s.name, on }).then(set).catch(fail);
  const remove = (s: Server) => api<State>(r1.id, "POST", "/api/connections/remove", { server: s.name }).then(set).catch(fail);
  const forget = (a: AgentRules, tool: string) => api<State>(r1.id, "POST", "/api/connections/always", { agent: a.id, tool, allow: false }).then(set).catch(fail);
  const claude = (data?.agents ?? []).filter((a) => a.brain === "claude");
  const codex = (data?.agents ?? []).filter((a) => a.brain === "codex");

  return (
    <motion.div className="list-card" initial={{ opacity: 0, y: 12 }} animate={{ opacity: 1, y: 0, transition: soft }}>
      <p className="muted small pad">Your agents use the apps you connect to your accounts. r1os keeps no passwords or tokens: claude.ai and ChatGPT do. Reading runs; anything that acts (sending, changing, deleting) asks on the R1.</p>
      <Section title="claude agents · claude.ai" />
      {data && data.servers.length === 0 && <Empty icon="plugs">none yet: add Gmail, Calendar, Drive and more on claude.ai</Empty>}
      {(data?.servers ?? []).map((s) => {
        const always = claude.flatMap((a) => a.always.filter((t) => t.startsWith(`mcp__${s.slug}__`)).map((t) => ({ a, t })));
        return (
          <div key={s.name} className="k-connection">
            <Item label={s.label} sub={[STATUS[s.status], s.source === "claudeai" ? "" : "yours, with a key", s.reads.length + s.acts.length ? `reads ${s.reads.length}, asks for ${s.acts.length}` : ""].filter(Boolean).join(" · ")}>
              {s.source === "claudeai" ? (
                <motion.button className="btn small" whileTap={{ scale: 0.95 }} disabled={signing === s.name} onClick={() => signIn(s)}>
                  <Icon name="sign-in" weight="bold" size={14} />
                  {signing === s.name ? "opening…" : s.status === "connected" ? "sign in again" : "sign in"}
                </motion.button>
              ) : (
                <Twice label="" armed="remove?" icon="trash" onConfirm={() => void remove(s)} />
              )}
            </Item>
            {(s.reads.length > 0 || s.acts.length > 0) && (
              <p className="muted small pad">
                {s.reads.length > 0 && <>reads without asking: {s.reads.map(words).join(", ")}. </>}
                {s.acts.length > 0 && <>asks first: {s.acts.map(words).join(", ")}.</>}
              </p>
            )}
            {claude.map((a) => (
              <Item key={a.id} label={a.name} sub={`uses ${s.label}`}>
                <Switch on={!a.off.includes(s.name)} onChange={(on) => void turn(a, s, on)} />
              </Item>
            ))}
            {always.map(({ a, t }) => (
              <Item key={a.id + t} label={`${a.name}: ${words(t.slice(`mcp__${s.slug}__`.length))}`} sub="always allowed">
                <button className="btn small" onClick={() => void forget(a, t)}>ask again</button>
              </Item>
            ))}
          </div>
        );
      })}
      <Item label="add more on claude.ai" sub="Gmail, Google Calendar, Drive, and your own by URL; your Claude agents have them from their next chat">
        <div className="row-buttons">
          <button className="btn small" onClick={() => data && void openUrl(data.add_url)}>
            <Icon name="arrow-square-out" weight="bold" size={14} />
            open
          </button>
          <button className="btn small" disabled={looking} onClick={check}>
            <Icon name="arrows-clockwise" weight="bold" size={14} />
            {looking ? "looking…" : "look again"}
          </button>
        </div>
      </Item>
      <AddServer r1={r1} onAdded={set} onError={fail} />
      <Section title="codex agents · chatgpt" />
      <Item label="chatgpt apps" sub={`the apps on your ChatGPT account, as you set them up there${codex.length ? ` · ${codex.map((a) => a.name).join(", ")}` : ""}`}>
        <button className="btn small" onClick={() => data && void openUrl(data.chatgpt_url)}>
          <Icon name="arrow-square-out" weight="bold" size={14} />
          open
        </button>
      </Item>
      {err && <p className="err">{err}</p>}
      <Skills r1={r1} />
    </motion.div>
  );
}

interface SkillInfo {
  name: string;
  description: string;
  /** `added` on the R1 (removable), or the claude.ai account's. */
  from: "added" | "claude.ai";
  on: boolean;
}
interface SkillsState {
  skills: SkillInfo[];
  agents: { id: string; name: string; brain: "claude" | "codex"; off: string[] }[];
}

/**
 * The R1 agents' skills (r1d's crate::skills): each on or off for every agent and for each, added ones removable. Added
 * here by link: `owner/repo`, a GitHub link to a skill or a folder of them, or a SKILL.md's; one holding several asks
 * which.
 */
function Skills({ r1 }: { r1: R1Info }) {
  const { data, set } = useGet<SkillsState>(r1.id, "/api/skills", ["connections_changed"]);
  const [err, setErr] = useState<string | null>(null);
  const [source, setSource] = useState("");
  const [busy, setBusy] = useState(false);
  const [choose, setChoose] = useState<{ name: string; description: string }[] | null>(null);
  const [picked, setPicked] = useState<string[]>([]);
  const [open, setOpen] = useState<string | null>(null);
  const fail = (e: unknown) => setErr(e instanceof Error ? e.message : String(e));
  const turn = (skill: string, on: boolean, agent?: string) => api<SkillsState>(r1.id, "POST", "/api/skills/on", { skill, on, agent }).then(set).catch(fail);
  const remove = (name: string) => api<SkillsState>(r1.id, "DELETE", `/api/skills/${encodeURIComponent(name)}`).then(set).catch(fail);
  const add = (skills: string[]) => {
    const s = source.trim();
    if (!s || busy) return;
    setBusy(true);
    setErr(null);
    api<{ added?: string[]; choose?: { name: string; description: string }[] }>(r1.id, "POST", "/api/skills", { source: s, skills })
      .then((r) => {
        if (r.choose) {
          setChoose(r.choose);
          setPicked([]);
          return;
        }
        setSource("");
        setChoose(null);
        return api<SkillsState>(r1.id, "GET", "/api/skills").then(set);
      })
      .catch(fail)
      .finally(() => setBusy(false));
  };
  const short = (t: string) => (t.length > 110 ? `${t.slice(0, 110).trimEnd()}…` : t);

  return (
    <>
      <Section title="skills" />
      <p className="muted small pad">Instructions, and sometimes scripts, for a kind of task (a PowerPoint, a PDF, a design), for every agent on the R1: your claude.ai account's and any added here. Add only skills you trust: they can run scripts in the agents' box.</p>
      {data && data.skills.length === 0 && <Empty icon="puzzle-piece">none yet</Empty>}
      {(data?.skills ?? []).map((k) => {
        const agents = (data?.agents ?? []).filter((a) => k.from === "added" || a.brain === "claude");
        return (
          <div key={k.name} className="k-connection">
            <Item label={k.name} sub={[k.from === "claude.ai" ? "from claude.ai" : "added", short(k.description)].filter(Boolean).join(" · ")}>
              <div className="row-buttons">
                {k.on && agents.length > 0 && (
                  <button className="btn small" onClick={() => setOpen(open === k.name ? null : k.name)}>
                    agents
                  </button>
                )}
                <Switch on={k.on} onChange={(on) => void turn(k.name, on)} />
                {k.from === "added" && <Twice label="" armed="remove?" icon="trash" onConfirm={() => void remove(k.name)} />}
              </div>
            </Item>
            {open === k.name &&
              k.on &&
              agents.map((a) => (
                <Item key={a.id} label={a.name} sub={`uses ${k.name}`}>
                  <Switch on={!a.off.includes(k.name)} onChange={(on) => void turn(k.name, on, a.id)} />
                </Item>
              ))}
          </div>
        );
      })}
      <Item label="add a skill" sub="owner/repo, a GitHub link to a skill or a folder of them, or a link to a SKILL.md (skills.sh lists many)" wide>
        <div className="new-row">
          <input
            className="field whole keep-case"
            placeholder="anthropics/skills"
            value={source}
            onChange={(e) => {
              setSource(e.target.value.trim());
              setChoose(null);
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter") add([]);
            }}
          />
          <motion.button className="btn small" whileTap={{ scale: 0.95 }} disabled={!source.trim() || busy} onClick={() => add([])}>
            <Icon name="plus" weight="bold" size={14} />
            {busy ? "adding…" : "add"}
          </motion.button>
        </div>
        {choose && (
          <div className="skill-choose">
            <p className="muted small">It has {choose.length} skills: pick the ones to add.</p>
            {choose.map((c) => (
              <label key={c.name} className="skill-pick">
                <input type="checkbox" checked={picked.includes(c.name)} onChange={(e) => setPicked((p) => (e.target.checked ? [...p, c.name] : p.filter((x) => x !== c.name)))} />
                <span>
                  <b>{c.name}</b> <span className="muted small keep-case">{short(c.description)}</span>
                </span>
              </label>
            ))}
            <motion.button className="btn small" whileTap={{ scale: 0.95 }} disabled={picked.length === 0 || busy} onClick={() => add(picked)}>
              <Icon name="plus" weight="bold" size={14} />
              {busy ? "adding…" : `add ${picked.length || ""}`}
            </motion.button>
          </div>
        )}
      </Item>
      {err && <p className="err keep-case">{err}</p>}
    </>
  );
}

/**
 * Your own server by URL that takes an API key: the R1's Claude agents get it, and its CLI keeps the key (ADR-0052). One
 * that signs in with OAuth is added on claude.ai as a custom connector instead, and signs in like the others.
 */
function AddServer({ r1, onAdded, onError }: { r1: R1Info; onAdded: (s: State) => void; onError: (e: unknown) => void }) {
  const [name, setName] = useState("");
  const [url, setUrl] = useState("");
  const [key, setKey] = useState("");
  const [header, setHeader] = useState("");
  const [busy, setBusy] = useState(false);
  const ok = /^[A-Za-z0-9_-]{1,40}$/.test(name) && /^https?:\/\/\S+$/.test(url);
  const add = () => {
    if (!ok) return;
    setBusy(true);
    api<State>(r1.id, "POST", "/api/connections/add", { name, url, key: key || undefined, header: header || undefined })
      .then((s) => {
        setName("");
        setUrl("");
        setKey("");
        setHeader("");
        onAdded(s);
      })
      .catch(onError)
      .finally(() => setBusy(false));
  };
  return (
    <Item label="your own server, with a key" sub="its URL and API key; one that signs in with OAuth goes on claude.ai as a custom connector" wide>
      <div className="new-row">
        <input className="field" placeholder="name (home-assistant)" value={name} maxLength={40} onChange={(e) => setName(e.target.value.replace(/\s/g, "-"))} />
        <input className="field whole" placeholder="https://… (its MCP URL)" value={url} onChange={(e) => setUrl(e.target.value.trim())} />
        <input className="field" type="password" placeholder="API key (if it takes one)" value={key} onChange={(e) => setKey(e.target.value)} autoComplete="off" />
        <input className="field" placeholder="header (Authorization: Bearer if empty)" value={header} onChange={(e) => setHeader(e.target.value)} />
        <motion.button className="btn small" whileTap={{ scale: 0.95 }} disabled={!ok || busy} onClick={add}>
          <Icon name="plus" weight="bold" size={14} />
          {busy ? "adding…" : "add it"}
        </motion.button>
      </div>
    </Item>
  );
}
