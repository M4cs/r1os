// Setup screens shared by onboarding and Settings (docs/specs/experience.md §2.4):
// Wi-Fi, making the agent, and connecting a brain (Claude / Codex).

import { useEffect, useRef, useState } from "react";
import { WheelFocus } from "./apps/focus";
import { accountsApi, NAMES, PERSONALITIES, ROLES, wifiApi, type Accounts, type Agent, type WifiNetwork, type WifiStatus } from "./api";
import { useAppInput, type InputHandler } from "./input";
import { onEvent } from "./r1";
import { Character, COLORS, FINISHES, FORMS } from "./ui/Character";

/** Wheel focus over a container's buttons/inputs; tap activates. Returns the container ref. */
export function useFocusList<T extends HTMLElement>(deps: unknown[] = [], extra?: InputHandler) {
  const ref = useRef<T>(null);
  const focus = useRef<WheelFocus | null>(null);
  useEffect(() => {
    if (ref.current) {
      focus.current = new WheelFocus(ref.current);
      // Start on the first control, so a tap does something straight away.
      window.setTimeout(() => focus.current?.move(1), 0);
    }
  }, deps);
  useAppInput((ev) => {
    if (extra?.(ev)) return true;
    if (ev.type === "wheel") {
      focus.current?.move(ev.delta);
      return true;
    }
    if (ev.type === "gesture" && ev.kind === "tap") return focus.current?.activate() ?? false;
    return false;
  });
  return ref;
}

// ---- Wi-Fi --------------------------------------------------------------------------------------

export function WifiSetup({ onDone, doneLabel = "Continue" }: { onDone: () => void; doneLabel?: string }) {
  const [status, setStatus] = useState<WifiStatus | null>(null);
  const [nets, setNets] = useState<WifiNetwork[]>([]);
  const [picked, setPicked] = useState<WifiNetwork | null>(null);
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [choosing, setChoosing] = useState(false);

  const load = () =>
    wifiApi.get().then(
      (r) => {
        setStatus(r.status);
        setNets(r.networks);
      },
      (e) => setErr(String(e)),
    );
  useEffect(() => {
    void wifiApi.scan().catch(() => {});
    void load();
    const t = window.setInterval(load, 4000);
    return () => window.clearInterval(t);
  }, []);

  const connect = async (n: WifiNetwork, pw?: string) => {
    setBusy(true);
    setErr(null);
    try {
      await wifiApi.connect(n.ssid, n.security, pw);
      for (let i = 0; i < 25; i++) {
        await new Promise((r) => setTimeout(r, 1000));
        const r = await wifiApi.get();
        if (r.status.ssid === n.ssid) {
          setStatus(r.status);
          setPicked(null);
          setChoosing(false);
          setBusy(false);
          return;
        }
      }
      throw new Error("Couldn't join. Check the password and try again.");
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
      setBusy(false);
    }
  };

  const ref = useFocusList<HTMLDivElement>([status?.ssid, picked, choosing, nets.length, busy]);

  if (err && !status) {
    return (
      <div className="setup" ref={ref}>
        <div className="setup-title">Wi-Fi</div>
        <div className="muted">{err}</div>
        <button className="primary" onClick={onDone}>
          {doneLabel}
        </button>
      </div>
    );
  }
  if (status?.ssid && !choosing) {
    return (
      <div className="setup" ref={ref}>
        <div className="setup-title">You're online</div>
        <div className="wifi-ok">
          <span className="bars b4" />
          {status.ssid}
        </div>
        <button className="primary" onClick={onDone}>
          {doneLabel}
        </button>
        <button className="ghost" onClick={() => setChoosing(true)}>
          Use another network
        </button>
      </div>
    );
  }
  if (picked) {
    return (
      <div className="setup" ref={ref}>
        <div className="setup-title">{picked.ssid}</div>
        <input
          className="field"
          type="password"
          placeholder="Password"
          value={password}
          autoComplete="off"
          onChange={(e) => setPassword(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && void connect(picked, password)}
        />
        <button className="primary" disabled={busy || password.length < 8} onClick={() => void connect(picked, password)}>
          {busy ? "Joining…" : "Join"}
        </button>
        <button className="ghost" onClick={() => (setPicked(null), setPassword(""), setErr(null))}>
          Back
        </button>
        {err && <div className="error small">{err}</div>}
      </div>
    );
  }
  return (
    <div className="setup" ref={ref}>
      <div className="setup-title">Choose Wi-Fi</div>
      {nets.length === 0 && <div className="muted">Looking for networks…</div>}
      <div className="setup-list">
        {nets.slice(0, 12).map((n) => (
          <button key={n.ssid} className="row" onClick={() => (n.security === "open" ? void connect(n) : setPicked(n))}>
            <span className={`bars b${n.bars}`} />
            <span className="grow">{n.ssid}</span>
            {n.security !== "open" && <span className="muted small">🔒</span>}
          </button>
        ))}
      </div>
      {busy && <div className="muted small">Joining…</div>}
      {err && <div className="error small">{err}</div>}
    </div>
  );
}

// ---- Make your agent ------------------------------------------------------------------------------

type DesignStep = "form" | "color" | "finish" | "name" | "role" | "personality";
const COLOR_NAMES = Object.keys(COLORS);

/** The steps where the wheel turns through choices with a live preview, and a tap moves on. */
const PICKS = {
  form: { title: "Pick a shape", list: FORMS.map((f) => f.id as string), label: (id: string) => FORMS.find((f) => f.id === id)?.label ?? id, next: "color" },
  color: { title: "Pick a color", list: COLOR_NAMES, label: (id: string) => id, next: "finish" },
  finish: { title: "Pick a finish", list: FINISHES.map((f) => f.id as string), label: (id: string) => FINISHES.find((f) => f.id === id)?.label ?? id, next: "name" },
} as const satisfies Record<string, { title: string; list: string[]; label: (id: string) => string; next: DesignStep }>;

/**
 * Shape → color → finish → name → what it's for → personality. The last pick saves through `onSave`
 * (create a new agent, or update one), then `onDone` gets the saved agent.
 */
export function AgentDesigner({
  initial,
  onSave,
  onDone,
  doneLabel = "Done",
}: {
  initial: Agent;
  onSave: (a: Agent) => Promise<Agent>;
  onDone: (a: Agent) => void;
  doneLabel?: string;
}) {
  const [a, setA] = useState<Agent>(initial);
  const [step, setStep] = useState<DesignStep>("form");
  const [custom, setCustom] = useState("");
  const [saving, setSaving] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  const save = async (agent: Agent) => {
    setSaving(true);
    setErr(null);
    try {
      onDone(await onSave(agent));
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
      setSaving(false);
    }
  };

  const cycle = <T,>(list: T[], cur: T, d: number) => list[(list.indexOf(cur) + d + list.length) % list.length]!;

  // The shape, colour and finish steps: the wheel changes the choice, a tap moves on.
  const pick = step === "form" || step === "color" || step === "finish" ? PICKS[step] : null;
  const turn = (d: number) => {
    if (step === "form" || step === "color" || step === "finish") {
      const k = step;
      setA((x) => ({ ...x, [k]: cycle<string>(PICKS[k].list, x[k], d) }));
    }
  };
  useAppInput((ev) => {
    if (!pick) return false;
    if (ev.type === "wheel") {
      turn(ev.delta);
      return true;
    }
    if (ev.type === "gesture" && ev.kind === "tap") {
      setStep(pick.next);
      return true;
    }
    return false;
  });

  if (pick && (step === "form" || step === "color" || step === "finish")) {
    const cur = a[step];
    return (
      <div className="setup designer">
        <div className="setup-title">{pick.title}</div>
        <Character form={a.form} color={a.color} finish={a.finish} eyes={a.eyes} wear={a.wear} state="idle" size={170} />
        <div className="picker">
          <button className="arrow" onClick={() => turn(-1)}>
            ‹
          </button>
          <span className="picker-label">{pick.label(cur)}</span>
          <button className="arrow" onClick={() => turn(1)}>
            ›
          </button>
        </div>
        <div className="dots">
          {pick.list.map((x) => (
            <span key={x} className={x === cur ? "on" : ""} style={step === "color" ? { background: COLORS[x] } : undefined} />
          ))}
        </div>
        <button className="primary" onClick={() => setStep(pick.next)}>
          Next
        </button>
        <div className="hint">wheel to change · tap to choose</div>
      </div>
    );
  }
  if (step === "name") return <NameStep a={a} setA={setA} custom={custom} setCustom={setCustom} onNext={() => setStep("role")} />;
  if (step === "role") return <RoleStep a={a} onPick={(role) => (setA((x) => ({ ...x, role })), setStep("personality"))} />;
  return (
    <PersonalityStep
      a={a}
      saving={saving}
      doneLabel={doneLabel}
      error={err}
      onPick={(p) => {
        const next = { ...a, personality: p };
        setA(next);
        void save(next);
      }}
    />
  );
}

function RoleStep({ a, onPick }: { a: Agent; onPick: (role: string) => void }) {
  const ref = useFocusList<HTMLDivElement>([]);
  const [typed, setTyped] = useState(ROLES.some((r) => r.id === a.role) ? "" : a.role);
  return (
    <div className="setup" ref={ref}>
      <div className="setup-row">
        <Character form={a.form} color={a.color} finish={a.finish} eyes={a.eyes} wear={a.wear} state="idle" size={72} />
        <div className="setup-title">What's {a.name} for?</div>
      </div>
      <div className="setup-list">
        {ROLES.map((r) => (
          <button key={r.id || "any"} className={`row${a.role === r.id ? " on" : ""}`} onClick={() => onPick(r.id)}>
            <span className="grow">{r.label}</span>
          </button>
        ))}
      </div>
      <input className="field" placeholder="Or say it in a few words" maxLength={120} value={typed} onChange={(e) => setTyped(e.target.value)} />
      {typed.trim() && (
        <button className="primary" onClick={() => onPick(typed.trim())}>
          Use “{typed.trim()}”
        </button>
      )}
    </div>
  );
}

function NameStep({ a, setA, custom, setCustom, onNext }: { a: Agent; setA: (f: (x: Agent) => Agent) => void; custom: string; setCustom: (s: string) => void; onNext: () => void }) {
  const ref = useFocusList<HTMLDivElement>([]);
  const names = NAMES.includes(a.name) ? NAMES : [a.name, ...NAMES];
  return (
    <div className="setup" ref={ref}>
      <div className="setup-row">
        <Character form={a.form} color={a.color} finish={a.finish} eyes={a.eyes} wear={a.wear} state="idle" size={72} />
        <div className="setup-title">Name it</div>
      </div>
      <div className="chips">
        {names.slice(0, 8).map((n) => (
          <button key={n} className={`chip${a.name === n ? " on" : ""}`} onClick={() => (setA((x) => ({ ...x, name: n })), onNext())}>
            {n}
          </button>
        ))}
      </div>
      <input className="field" placeholder="Or type a name" maxLength={20} value={custom} onChange={(e) => setCustom(e.target.value)} />
      <button className="primary" disabled={!custom.trim()} onClick={() => (setA((x) => ({ ...x, name: custom.trim() })), onNext())}>
        Use “{custom.trim() || "…"}”
      </button>
    </div>
  );
}

function PersonalityStep({ a, saving, doneLabel, error, onPick }: { a: Agent; saving: boolean; doneLabel: string; error: string | null; onPick: (p: string) => void }) {
  const ref = useFocusList<HTMLDivElement>([]);
  return (
    <div className="setup" ref={ref}>
      <div className="setup-row">
        <Character form={a.form} color={a.color} finish={a.finish} eyes={a.eyes} wear={a.wear} state="idle" size={72} />
        <div className="setup-title">How should {a.name} talk?</div>
      </div>
      <div className="setup-list">
        {PERSONALITIES.map((p) => (
          <button key={p.id} className={`row${a.personality === p.id ? " on" : ""}`} disabled={saving} onClick={() => onPick(p.id)}>
            <span className="grow">
              <b>{p.label}</b>
              <span className="muted small"> · {p.hint}</span>
            </span>
          </button>
        ))}
      </div>
      {error && <div className="error small">{error}</div>}
      <div className="hint">{saving ? "Saving…" : `Tap one to ${doneLabel.toLowerCase()}`}</div>
    </div>
  );
}

// ---- Connect a brain ------------------------------------------------------------------------------

type Brain = "claude" | "codex";
type Pairing =
  | { brain: "claude"; pairUrl: string; state: string }
  | { brain: "codex"; url: string; code: string; state: string };

export function BrainSetup({ agent, onDone, doneLabel = "Continue" }: { agent: Agent; onDone: () => void; doneLabel?: string }) {
  const [acc, setAcc] = useState<Accounts | null>(null);
  const [pairing, setPairing] = useState<Pairing | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [starting, setStarting] = useState<Brain | null>(null);

  const load = () => accountsApi.get().then(setAcc, (e) => setErr(String(e)));
  useEffect(() => {
    void load();
    return onEvent((ev) => {
      if (ev.type !== "account") return;
      setPairing((p) => (p && p.brain === ev.brain ? { ...p, state: ev.state } : p));
      if (ev.state === "signed_in" || ev.state === "signed_out") {
        void load();
        if (ev.state === "signed_in") window.setTimeout(() => setPairing((p) => (p?.brain === ev.brain ? null : p)), 1500);
      }
    });
  }, []);

  const start = async (brain: Brain) => {
    setErr(null);
    setStarting(brain);
    try {
      if (brain === "claude") {
        const r = await accountsApi.claudeLogin();
        setPairing({ brain, pairUrl: r.pair_url, state: "waiting" });
      } else {
        const r = await accountsApi.codexLogin();
        setPairing({ brain, url: r.url, code: r.code, state: "waiting" });
      }
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setStarting(null);
    }
  };

  const ref = useFocusList<HTMLDivElement>([acc?.claude, acc?.codex, pairing?.brain], (ev) => {
    if (pairing && ev.type === "gesture" && ev.kind === "double_tap") {
      setPairing(null);
      return true;
    }
    return false;
  });

  if (pairing) {
    const done = pairing.state === "signed_in";
    const stateText: Record<string, string> = {
      waiting: pairing.brain === "claude" ? "Scan with your phone's camera" : "Approve on your phone",
      code_entered: "Checking the code…",
      signed_in: "Connected",
      failed: "That didn't work. Try again.",
      expired: "This code expired. Try again.",
    };
    return (
      <div className="setup pair" ref={ref}>
        <div className="setup-title">{pairing.brain === "claude" ? "Connect Claude" : "Connect Codex"}</div>
        {done ? (
          <div className="pair-done">✓</div>
        ) : (
          <img className="qr" alt="QR code" src={accountsApi.qrUrl(pairing.brain === "claude" ? pairing.pairUrl : pairing.url)} />
        )}
        {pairing.brain === "codex" && !done && <div className="pair-code">{pairing.code}</div>}
        <div className={`pair-state${pairing.state === "failed" || pairing.state === "expired" ? " error" : ""}`}>{stateText[pairing.state] ?? pairing.state}</div>
        {pairing.brain === "claude" && !done && <div className="muted small">Your phone must be on the same Wi-Fi.</div>}
        <button className="ghost" onClick={() => setPairing(null)}>
          Back
        </button>
      </div>
    );
  }

  const card = (brain: Brain, label: string, sub: string) => {
    const on = acc?.[brain];
    return (
      <button className={`brain${on ? " on" : ""}`} disabled={starting !== null} onClick={() => (on ? undefined : void start(brain))}>
        <span className="brain-name">{label}</span>
        <span className="brain-sub">{on ? "Connected ✓" : starting === brain ? "Starting…" : sub}</span>
      </button>
    );
  };
  return (
    <div className="setup" ref={ref}>
      <div className="setup-row">
        <Character form={agent.form} color={agent.color} finish={agent.finish} eyes={agent.eyes} wear={agent.wear} state={acc ? "idle" : "thinking"} size={64} />
        <div className="setup-title">Give {agent.name} a brain</div>
      </div>
      {acc === null && !err && <div className="muted">Checking accounts…</div>}
      {card("claude", "Claude", "Sign in with your Claude account")}
      {card("codex", "Codex", "Sign in with ChatGPT")}
      {err && <div className="error small">{err}</div>}
      <button className="primary" disabled={!acc?.claude && !acc?.codex && !acc?.hermes} onClick={onDone}>
        {doneLabel}
      </button>
      {acc && !acc.claude && !acc.codex && !acc.hermes && (
        <button className="ghost" onClick={onDone}>
          Set up later
        </button>
      )}
    </div>
  );
}
