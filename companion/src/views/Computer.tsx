// This computer (docs/specs/companion.md §5, §7.2): starting with Windows, how long an R1's link stays when nothing's
// going on (ADR-0049), the R1s it trusts, and the agents that work here: the CLI they run, what they're doing, the
// last turns.

import { motion } from "motion/react";
import { useEffect, useState } from "react";
import { type LinkState, type Settings, autostart, claudeSignIn, codexSignIn, inTauri, link, settings as settingsApi, usePcStatus } from "../link";
import { page, soft } from "../motion";
import { Empty, Twice, ago } from "./parts";
import { Item, Section, Seg, Toggle } from "./Settings";

const IDLE: [number, string][] = [[0, "screen on"], [60, "1 min"], [300, "5 min"], [900, "15 min"], [1800, "30 min"], [3600, "1 h"]];

export function Computer({ state }: { state: LinkState }) {
  const [start, setStart] = useState<boolean | null>(null);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [signing, setSigning] = useState<string | null>(null);
  const pc = usePcStatus();
  useEffect(() => {
    autostart.get().then(setStart).catch(() => setStart(false));
    settingsApi.get().then(setSettings).catch(() => setSettings({ idle_s: 300 }));
  }, []);
  const r1Name = (key: string) => state.r1s.find((r) => r.id === key)?.name ?? "an r1";
  return (
    <motion.section className="computer" {...page}>
      <h1>{state.me.name}</h1>
      <div className="chips">
        <span className="chip mono" title={state.me.id}>{state.me.id.slice(0, 16)}…</span>
      </div>
      <motion.div className="list-card" initial={{ opacity: 0, y: 12 }} animate={{ opacity: 1, y: 0, transition: soft }}>
        <Section title="r1os on this computer" />
        <Toggle
          label="start with windows"
          sub={inTauri ? "in the tray, so your r1 can link up whenever it's awake" : "in the app only"}
          on={!!start}
          onChange={(on) => {
            setStart(on);
            autostart.set(on).catch(() => setStart(!on));
          }}
        />
        <Item label="keep an r1 linked" sub="after its screen goes off, for as long as nothing's going on here or there. an agent working here keeps it too." wide>
          {settings && (
            <Seg
              id="idle"
              items={IDLE}
              value={settings.idle_s}
              onChange={(idle_s) => {
                const s = { ...settings, idle_s };
                setSettings(s);
                void settingsApi.set(s);
              }}
            />
          )}
        </Item>

        <Section title="agents working here" />
        <Item
          label="claude code"
          sub={!pc ? "…" : !pc.claude.path ? "not installed: npm i -g @anthropic-ai/claude-code" : pc.claude.signed_in ? `${pc.claude.version ?? ""} · signed in` : (signing ?? `${pc.claude.version ?? ""} · not signed in`)}
        >
          {pc?.claude.path && !pc.claude.signed_in ? (
            <button
              className="btn small"
              onClick={() => {
                setSigning("finish signing in in your browser…");
                claudeSignIn()
                  .then(() => setSigning(null))
                  .catch((e) => setSigning(e instanceof Error ? e.message : String(e)));
              }}
            >
              sign in
            </button>
          ) : (
            <span className={`dot ${pc?.claude.path && pc.claude.signed_in ? "awake" : ""}`} />
          )}
        </Item>
        <Item
          label="codex"
          sub={
            !pc ? "…" : !pc.codex.path ? "not installed: npm i -g @openai/codex" : pc.codex.signed_in ? `signed in${pc.codex.account ? ` as ${pc.codex.account}` : ""} (r1os's own codex login)` : (signing ?? "sign in once for codex agents here (r1os keeps its own codex login, apart from yours)")
          }
        >
          {pc?.codex.path && !pc.codex.signed_in ? (
            <button
              className="btn small"
              disabled={signing === "finish signing in in your browser…"}
              onClick={() => {
                setSigning("finish signing in in your browser…");
                codexSignIn()
                  .then(() => setSigning(null))
                  .catch((e) => setSigning(e instanceof Error ? e.message : String(e)));
              }}
            >
              sign in
            </button>
          ) : (
            <span className={`dot ${pc?.codex.signed_in ? "awake" : ""}`} />
          )}
        </Item>
        <Item label="their folders" sub="each starts in its own, and works in the folders you give it (on the r1's agents page)">
          <span className="value keep-case mono small">{pc?.workspaces ?? "…"}</span>
        </Item>
        {pc?.running.map((t) => (
          <Item key={`${t.agent}-${t.started}`} label={`${t.agent} is working`} sub={`for ${r1Name(t.r1)} · since ${ago(t.started)}${t.tools.length ? ` · ${t.tools.slice(-3).join(", ").toLowerCase()}` : ""}`}>
            <span className="dot awake pulse" />
          </Item>
        ))}
        {pc && pc.running.length === 0 && pc.log.length === 0 && <Empty icon="desktop">no agent has worked here yet: move one here on your r1's agents page</Empty>}
        {pc?.log.slice(0, 8).map((t) => (
          <Item
            key={`${t.agent}-${t.at}`}
            label={`${t.agent}${t.quiet ? " tidied a chat" : ""}`}
            sub={`${ago(t.at)} · ${Math.round(t.ms / 1000)} s${t.tools.length ? ` · ${t.tools.length} tool${t.tools.length > 1 ? "s" : ""}` : ""}${t.ok ? "" : ` · ${t.error ?? "failed"}`}`}
          >
            <span className={`dot ${t.ok ? "awake" : "bad"}`} />
          </Item>
        ))}

        <Section title="r1s it trusts" />
        {state.r1s.length === 0 && <Empty icon="link">none yet</Empty>}
        {state.r1s.map((r) => (
          <Item key={r.id} label={r.name} sub={`${r.id.slice(0, 16)}… · ${r.online ? "linked now" : "not linked now"}`}>
            <Twice label="unlink" armed="click again: unlink" onConfirm={() => void link.unlink(r.id)} />
          </Item>
        ))}
        <p className="muted small list-foot">an r1 you unlink can't reach this computer until it reads a new code.</p>
      </motion.div>
    </motion.section>
  );
}
