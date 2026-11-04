// Settings pages (launcher › Settings): what every agent knows about you, the same setup screens as
// onboarding, and About. Each agent's own look, brain and memory are in its space (space.tsx).

import { useEffect, useState } from "react";
import { accountsApi, onboardingApi, type Accounts, type Agent } from "./api";
import type { R1State } from "./r1";
import { BrainSetup, useFocusList, WifiSetup } from "./setup";
import { MemoryList } from "./space";

export type SettingsPage = "you" | "accounts" | "wifi" | "about";

export const SETTINGS_TITLE: Record<SettingsPage, string> = { you: "About you", accounts: "Accounts", wifi: "Wi-Fi", about: "About" };

export function Settings({ page, agent, state, onDone }: { page: SettingsPage; agent: Agent; state: R1State | null; onDone: () => void }) {
  switch (page) {
    case "you":
      return <MemoryList />;
    case "accounts":
      return <AccountsSettings agent={agent} onDone={onDone} />;
    case "wifi":
      return <WifiSetup onDone={onDone} doneLabel="Done" />;
    case "about":
      return <About state={state} />;
  }
}

/** The sign-ins every agent shares. Which one an agent talks with is set in its space. */
function AccountsSettings({ agent, onDone }: { agent: Agent; onDone: () => void }) {
  const [acc, setAcc] = useState<Accounts | null>(null);
  const [connecting, setConnecting] = useState(false);
  useEffect(() => {
    void accountsApi.get().then(setAcc, () => {});
  }, [connecting]);
  const ref = useFocusList<HTMLDivElement>([acc?.claude, acc?.codex, connecting]);
  if (connecting) return <BrainSetup agent={agent} doneLabel="Done" onDone={() => setConnecting(false)} />;
  return (
    <div className="setup" ref={ref}>
      {(["claude", "codex"] as const).map((b) => (
        <div key={b} className={`brain${acc?.[b] ? " on" : ""}`}>
          <span className="brain-name">{b === "claude" ? "Claude" : "Codex"}</span>
          <span className="brain-sub">{acc === null ? "…" : acc[b] ? "Connected · every agent can use it" : "Not connected"}</span>
          {acc?.[b] && (
            <button className="ghost small" onClick={() => void accountsApi.logout(b).then(() => accountsApi.get().then(setAcc))}>
              Sign out
            </button>
          )}
        </div>
      ))}
      {/* Your own Hermes gateway (ADR-0070): its address and key are too much for the wheel, so the companion adds it. */}
      <div className={`brain${acc?.hermes ? " on" : ""}`}>
        <span className="brain-name">Hermes</span>
        <span className="brain-sub">{acc === null ? "…" : acc.hermes ? `Your gateway · ${(acc.hermes_url ?? "").replace(/^.*:\/\//, "")}` : "Not connected · add your gateway in the companion"}</span>
        {acc?.hermes && (
          <button className="ghost small" onClick={() => void accountsApi.logout("hermes").then(() => accountsApi.get().then(setAcc))}>
            Disconnect
          </button>
        )}
      </div>
      <button className="primary" onClick={() => setConnecting(true)}>
        Connect an account
      </button>
      <button className="ghost" onClick={onDone}>
        Done
      </button>
    </div>
  );
}

function About({ state }: { state: R1State | null }) {
  const ref = useFocusList<HTMLDivElement>([]);
  const s = state;
  const rows: [string, string][] = [
    ["r1os", s ? `r1d v${s.version}` : "…"],
    ["Battery", s?.system.battery_pct != null ? `${s.system.battery_pct}%${s.system.charging ? " (charging)" : ""}` : "…"],
    ["Memory free", s?.system.mem_available_mb != null ? `${s.system.mem_available_mb} MB` : "…"],
    ["Mode", s?.sim ? "simulator" : "R1"],
  ];
  return (
    <div className="setup" ref={ref}>
      <dl className="kv">
        {rows.map(([k, v]) => (
          <div key={k}>
            <dt>{k}</dt>
            <dd>{v}</dd>
          </div>
        ))}
      </dl>
      <button
        className="ghost"
        onClick={() => {
          void onboardingApi.set("hello").then(() => location.reload());
        }}
      >
        Run setup again
      </button>
    </div>
  );
}
