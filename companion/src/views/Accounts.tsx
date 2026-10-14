// Signing the R1's agent box in from the computer (docs/specs/companion.md §7.1, ADR-0004): the sign-in page opens in
// this computer's browser; Claude's code is pasted here and crosses the link once, straight to the CLI (never kept);
// Codex shows a code to enter on its page. The R1's `account` events say how it's going.

import { Icon } from "@r1os/icons";
import { AnimatePresence, motion } from "motion/react";
import { useCallback, useEffect, useState } from "react";
import { type R1Info, api, openUrl, useR1Events } from "../link";
import { soft, spring } from "../motion";

type Brain = "claude" | "codex";
interface Flow {
  brain: Brain;
  /** Claude: what the code goes back with. */
  token?: string;
  /** Codex: the code to enter on its page. */
  code?: string;
  url: string;
  state: "open" | "checking" | "done" | "failed";
  why?: string;
}

const NAMES: Record<Brain, string> = { claude: "claude", codex: "codex" };

export function Accounts({ r1 }: { r1: R1Info }) {
  const [acc, setAcc] = useState<Record<Brain, boolean> | null>(null);
  const [flow, setFlow] = useState<Flow | null>(null);
  const [code, setCode] = useState("");
  const [armed, setArmed] = useState<Brain | null>(null);
  const [err, setErr] = useState<string | null>(null);

  const load = useCallback(() => {
    api<{ accounts: Record<Brain, boolean> }>(r1.id, "GET", "/api/accounts").then((r) => setAcc(r.accounts)).catch((e) => setErr(String(e)));
  }, [r1.id]);
  useEffect(load, [load]);
  useR1Events(r1.id, (e) => {
    if (e.type !== "account") return;
    const state = e.state as string;
    setFlow((f) => {
      if (!f || f.brain !== e.brain) return f;
      if (state === "code_entered") return { ...f, state: "checking" };
      if (state === "signed_in") return { ...f, state: "done" };
      if (state === "failed" || state === "expired") return { ...f, state: "failed", why: state === "expired" ? "it took too long: start again" : (e.detail as string) ?? "it didn't work" };
      return f;
    });
    if (state === "signed_in" || state === "signed_out") load();
  });

  const start = async (brain: Brain) => {
    setErr(null);
    setCode("");
    try {
      if (brain === "claude") {
        const r = await api<{ auth_url: string; token: string }>(r1.id, "POST", "/api/accounts/claude/login", { via: "computer" });
        setFlow({ brain, token: r.token, url: r.auth_url, state: "open" });
        await openUrl(r.auth_url);
      } else {
        const r = await api<{ url: string; code: string }>(r1.id, "POST", "/api/accounts/codex/login");
        setFlow({ brain, code: r.code, url: r.url, state: "open" });
        await openUrl(r.url);
      }
    } catch (e) {
      setErr(String(e));
    }
  };

  const send = async () => {
    if (!flow?.token || !code.trim()) return;
    setFlow({ ...flow, state: "checking" });
    try {
      await api(r1.id, "POST", "/api/accounts/claude/code", { token: flow.token, code: code.trim() });
    } catch (e) {
      setFlow({ ...flow, state: "failed", why: String(e) });
    }
    setCode("");
  };

  const signOut = (brain: Brain) => {
    if (armed !== brain) {
      setArmed(brain);
      setTimeout(() => setArmed((a) => (a === brain ? null : a)), 3000);
      return;
    }
    setArmed(null);
    api(r1.id, "POST", `/api/accounts/${brain}/logout`).then(load).catch((e) => setErr(String(e)));
  };

  return (
    <div className="accounts">
      <p className="muted lede">one sign-in serves every agent on the r1. the agents' own apps keep the logins, in the r1's agent box; r1os never keeps them.</p>
      {(["claude", "codex"] as Brain[]).map((b, i) => {
        const on = acc?.[b];
        const f = flow?.brain === b ? flow : null;
        return (
          <motion.div key={b} className="card account" initial={{ opacity: 0, y: 12 }} animate={{ opacity: 1, y: 0, transition: { ...soft, delay: 0.05 * i } }}>
            <div className="account-head">
              <div>
                <div className="account-name">{NAMES[b]}</div>
                <div className="muted small">{acc === null ? "…" : on ? "connected · every agent can use it" : "not connected"}</div>
              </div>
              {on ? (
                <button className={`btn small ${armed === b ? "danger" : ""}`} onClick={() => signOut(b)}>
                  {armed === b ? "click again: sign out" : "sign out"}
                </button>
              ) : (
                <motion.button className="btn primary small" whileTap={{ scale: 0.96 }} onClick={() => start(b)}>
                  sign in
                </motion.button>
              )}
            </div>
            <AnimatePresence>
              {f && (
                <motion.div className="flow" initial={{ height: 0, opacity: 0 }} animate={{ height: "auto", opacity: 1, transition: soft }} exit={{ height: 0, opacity: 0 }}>
                  {f.state === "done" ? (
                    <div className="flow-done"><Icon name="check-circle" weight="fill" size={22} /> signed in</div>
                  ) : f.state === "failed" ? (
                    <div className="flow-row">
                      <span className="err">{f.why}</span>
                      <button className="btn small" onClick={() => start(b)}>start again</button>
                    </div>
                  ) : b === "claude" ? (
                    <div className="flow-col">
                      <div className="muted">claude's page opened in your browser. approve it, then paste the code it shows here.</div>
                      <div className="flow-row">
                        <input className="field mono" placeholder="paste the code" value={code} onChange={(e) => setCode(e.target.value)} onKeyDown={(e) => e.key === "Enter" && send()} disabled={f.state === "checking"} autoFocus />
                        <motion.button className="btn primary" whileTap={{ scale: 0.96 }} onClick={send} disabled={f.state === "checking" || !code.trim()}>
                          {f.state === "checking" ? "checking…" : "connect"}
                        </motion.button>
                      </div>
                      <button className="link" onClick={() => openUrl(f.url)}>open the page again <Icon name="arrow-up-right" size={13} /></button>
                    </div>
                  ) : (
                    <div className="flow-col">
                      <div className="muted">enter this code on the page that opened in your browser:</div>
                      <div className="code mono small-code">
                        {(f.code ?? "").split("").map((ch, k) => (
                          <motion.span key={k} initial={{ y: 12, opacity: 0 }} animate={{ y: 0, opacity: 1 }} transition={{ ...spring, delay: 0.03 * k }}>{ch}</motion.span>
                        ))}
                      </div>
                      <div className="flow-row">
                        <button className="btn small" onClick={() => navigator.clipboard?.writeText(f.code ?? "")}>copy the code</button>
                        <button className="link" onClick={() => openUrl(f.url)}>open the page again <Icon name="arrow-up-right" size={13} /></button>
                      </div>
                    </div>
                  )}
                </motion.div>
              )}
            </AnimatePresence>
          </motion.div>
        );
      })}
      {err && <p className="err">{err}</p>}
    </div>
  );
}
