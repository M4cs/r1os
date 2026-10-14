// Tools drawn like hardware (ADR-0037): the Translator as a two-channel mixer (ADR-0034) and the Recorder as a
// tape-to-tape deck (ADR-0033), as their screens look on the R1.

import { Icon } from "@r1os/icons";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { useEffect } from "react";
import { Screen, useDevice } from "../R1";
import { Mono, pop, Row, T, Title, useSteps, useTicker } from "../ui";

// ---- the translator ----------------------------------------------------------------------------------------

const LINES = [
  { who: "you", heard: "where's the train station?", said: "dov'è la stazione dei treni?" },
  { who: "them", heard: "a destra, dopo il ponte.", said: "on the right, after the bridge." },
];

function Channel({ who, code, name, live, accent, x }: { who: string; code: string; name: string; live: boolean; accent: string; x: number }) {
  return (
    <motion.div animate={{ background: live ? accent : T.card }} transition={{ duration: 0.25 }} style={{ position: "absolute", left: x, top: 64, width: 158, height: 124, borderRadius: 14, padding: "8px 11px" }}>
      <div style={{ display: "flex", justifyContent: "space-between", fontSize: 11, fontWeight: 600, color: live ? "rgba(0,0,0,.6)" : T.muted }}>
        {who}
        <motion.span animate={{ opacity: live ? [1, 0.3, 1] : 0.5 }} transition={{ duration: 0.8, repeat: live ? Infinity : 0 }} style={{ width: 7, height: 7, borderRadius: 4, background: live ? "#0b0b0c" : T.muted }} />
      </div>
      <div style={{ fontSize: 46, fontWeight: 700, lineHeight: 1, marginTop: 4, color: live ? "#0b0b0c" : "#3b3e45" }}>{code}</div>
      <div style={{ fontSize: 13, fontWeight: 600, color: live ? "rgba(0,0,0,.7)" : T.muted }}>{name}</div>
      <div style={{ position: "absolute", left: 11, right: 11, bottom: 10, display: "flex", gap: 3, alignItems: "flex-end", height: 14 }}>
        {Array.from({ length: 16 }, (_, i) => (
          <motion.span
            key={i}
            style={{ flex: 1, height: 14, borderRadius: 1, transformOrigin: "50% 100%", background: live ? "rgba(0,0,0,.55)" : "#2f3238" }}
            initial={{ scaleY: 4 / 14 }}
            animate={{ scaleY: live ? [4, 6 + ((i * 29) % 9), 3, 9 + ((i * 13) % 6), 4].map((h) => h / 14) : 4 / 14 }}
            transition={{ duration: 0.7, repeat: live ? Infinity : 0, delay: (i % 5) * 0.06 }}
          />
        ))}
      </div>
    </motion.div>
  );
}

export function TranslatorScreen({ accent }: { accent: string }) {
  // you speak · it's translated · they answer · it's translated
  const step = useSteps([300, 1400, 2300, 3300]);
  const { hold } = useDevice();
  const speaking = step === 1 ? "you" : step === 3 ? "them" : null;
  useEffect(() => {
    hold(step === 1 || step === 3);
    return () => hold(false);
  }, [step, hold]);
  const line = step >= 3 ? LINES[1]! : LINES[0]!;
  const done = step === 2 || step >= 4;
  return (
    <Screen>
      <Title>translator</Title>
      <div style={{ position: "absolute", right: 14, top: 12, padding: "3px 8px", borderRadius: 6, border: `1.5px solid ${T.subtle}`, lineHeight: 1.15 }}>
        <Mono style={{ fontSize: 13, color: T.fg }}>live</Mono>
        <br />
        <Mono style={{ fontSize: 11, color: T.muted }}>en ⇄ it</Mono>
      </div>
      <Channel who="you" code="en" name="english" live={speaking === "you"} accent={accent} x={14} />
      <Channel who="them" code="it" name="italian" live={speaking === "them"} accent={accent} x={188} />
      <div style={{ position: "absolute", left: 167, top: 112, width: 26, height: 26, borderRadius: 13, display: "grid", placeItems: "center", background: "#000", border: `1.5px solid ${T.subtle}`, color: T.muted }}>
        <Icon name="arrows-left-right" weight="bold" size={13} />
      </div>
      <div style={{ position: "absolute", left: 16, right: 16, top: 206, textTransform: "none" }}>
        <AnimatePresence mode="wait">
          {step === 0 ? (
            <motion.div key="ready" exit={{ opacity: 0 }} style={{ fontSize: 16, fontWeight: 700, color: T.muted }}>
              hold and speak either language
            </motion.div>
          ) : (
            <motion.div key={line.who} initial={{ opacity: 0, y: 10 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0, y: -10 }} transition={pop}>
              <div style={{ fontSize: 15, fontWeight: 600, color: T.muted }}>{line.heard}</div>
              {done && (
                <motion.div initial={{ opacity: 0, y: 8, filter: "blur(4px)" }} animate={{ opacity: 1, y: 0, filter: "blur(0px)" }} transition={pop} style={{ fontSize: 26, fontWeight: 700, lineHeight: 1.15, marginTop: 6, color: T.fg }}>
                  {line.said}
                </motion.div>
              )}
              {!done && <div style={{ fontSize: 13, fontWeight: 700, color: accent, marginTop: 8 }}>listening…</div>}
            </motion.div>
          )}
        </AnimatePresence>
      </div>
      <div style={{ position: "absolute", left: 14, right: 14, bottom: 14, display: "flex", gap: 6 }}>
        {[
          { icon: "circle", label: "hold", on: speaking !== null },
          { icon: "play", label: "again", on: false },
          { icon: "stop", label: "stop", on: false },
        ].map((b) => (
          <motion.div key={b.label} animate={{ background: b.on ? accent : T.card, color: b.on ? "#0b0b0c" : T.fg }} style={{ flex: 1, height: 56, borderRadius: 12, display: "grid", placeItems: "center", alignContent: "center", gap: 3, fontSize: 11, fontWeight: 600 }}>
            <Icon name={b.icon} weight="fill" size={17} />
            {b.label}
          </motion.div>
        ))}
      </div>
    </Screen>
  );
}

// ---- the recorder ------------------------------------------------------------------------------------------

function Reel({ x, tape, spin }: { x: number; tape: number; spin: number }) {
  return (
    <div style={{ position: "absolute", left: x - 74, top: 82, width: 148, height: 148 }}>
      <svg width={148} height={148} viewBox="0 0 148 148" style={{ position: "absolute", inset: 0 }}>
        <circle cx={74} cy={74} r={72} fill="none" stroke="#3a3d44" strokeWidth={1.5} />
        <circle cx={74} cy={74} r={tape} fill="#1c1d21" stroke="#2a2c31" strokeWidth={1} />
        {[46, 52, 58, 64].filter((r) => r < tape).map((r) => (
          <circle key={r} cx={74} cy={74} r={r} fill="none" stroke="#25272c" strokeWidth={1} />
        ))}
        <circle cx={74} cy={74} r={33} fill="#000" stroke="#4a4d55" strokeWidth={1.5} />
      </svg>
      <motion.svg width={148} height={148} viewBox="0 0 148 148" style={{ position: "absolute", inset: 0 }} animate={{ rotate: spin }} transition={{ duration: 6, ease: "linear", repeat: spin ? Infinity : 0 }}>
        {[0, 120, 240].map((a) => (
          <rect key={a} x={71} y={47} width={6} height={18} rx={3} fill="#f2f2f2" transform={`rotate(${a} 74 74)`} />
        ))}
        <circle cx={74} cy={74} r={4} fill="#f2f2f2" />
      </motion.svg>
    </div>
  );
}

export function RecorderScreen({ accent }: { accent: string }) {
  const step = useSteps([350, 3000]);
  const rec = step >= 1;
  // Recording moves (the counter, the reels, the level); with reduced motion it's shown still, at its first seconds.
  const reduce = useReducedMotion();
  const live = rec && !reduce;
  const s = useTicker(live, 1000 / 6, 7, 59);
  const { hold } = useDevice();
  useEffect(() => {
    hold(step === 0);
    return () => hold(false);
  }, [step, hold]);
  const t = `0:00:${String(Math.min(s, 59)).padStart(2, "0")}`;
  return (
    <Screen>
      <div style={{ position: "absolute", left: 16, top: 4, display: "flex", alignItems: "center", gap: 12 }}>
        <motion.div animate={{ background: rec ? "#ff5a5a" : "#3a3d44", boxShadow: rec ? "0 0 14px #ff5a5a" : "none" }} style={{ width: 16, height: 16, borderRadius: 3 }} />
        <div style={{ fontSize: 54, fontWeight: 700, letterSpacing: "-0.02em", fontVariantNumeric: "tabular-nums", lineHeight: 1.2 }}>{t}</div>
      </div>
      <Reel x={96} tape={66 - Math.min(s, 40) * 0.15} spin={live ? 360 : 0} />
      <Reel x={264} tape={38 + Math.min(s, 40) * 0.15} spin={live ? 360 : 0} />
      <svg width={360} height={480} style={{ position: "absolute", inset: 0, pointerEvents: "none" }}>
        <path d="M 70 222 L 152 262 L 208 262 L 290 222" fill="none" stroke="#6c717a" strokeWidth={1.4} />
        <circle cx={152} cy={262} r={3.5} fill="#8a8f98" />
        <circle cx={208} cy={262} r={3.5} fill="#8a8f98" />
        <rect x={166} y={254} width={28} height={14} rx={3} fill="#c9ccd2" />
      </svg>
      <div style={{ position: "absolute", left: 0, right: 0, top: 276, textAlign: "center", fontSize: 13, fontWeight: 600, color: T.muted }}>{rec ? "recording · words as you go" : "click to record"}</div>
      <div style={{ position: "absolute", left: 14, right: 14, top: 300, display: "flex", justifyContent: "space-between", alignItems: "flex-end", height: 18 }}>
        {Array.from({ length: 40 }, (_, i) => (
          <motion.span
            key={i}
            style={{ width: 2, height: 18, borderRadius: 1, transformOrigin: "50% 100%", background: i % 5 === 0 ? "#6c717a" : "#3a3d44" }}
            initial={{ scaleY: (i % 5 === 0 ? 12 : 7) / 18 }}
            animate={{ scaleY: live ? [5, 6 + ((i * 41) % 12), 4, 8 + ((i * 17) % 10), 5].map((h) => h / 18) : (i % 5 === 0 ? 12 : 7) / 18 }}
            transition={{ duration: 0.9, repeat: live ? Infinity : 0, delay: (i % 9) * 0.04 }}
          />
        ))}
      </div>
      <div style={{ position: "absolute", left: 12, right: 12, top: 328, display: "flex", gap: 7 }}>
        {[
          { icon: "circle", label: "rec", on: rec, color: "#ff5a5a" },
          { icon: "play", label: "play", on: false, color: T.fg },
          { icon: "stop", label: "stop", on: false, color: T.fg },
        ].map((b) => (
          <div key={b.label} style={{ flex: 1, height: 64, borderRadius: 12, background: T.card, display: "grid", placeItems: "center", alignContent: "center", gap: 4, fontSize: 12, fontWeight: 600, color: T.fg }}>
            <motion.span animate={{ color: b.on ? b.color : "#dfe1e5", filter: b.on ? `drop-shadow(0 0 6px ${b.color})` : "none" }} style={{ display: "grid" }}>
              <Icon name={b.icon} weight="fill" size={22} />
            </motion.span>
            {b.label}
          </div>
        ))}
      </div>
      <div style={{ position: "absolute", left: 18, top: 404, fontSize: 13, fontWeight: 600, color: T.muted }}>tapes</div>
      <div style={{ position: "absolute", left: 8, right: 8, top: 424 }}>
        <Row icon="cassette-tape" label={step >= 2 ? "talk with the builder" : "tape 03"} sub={`today · ${t.slice(2)}`} on accent={accent} right="03" layoutId="tape-fill" />
      </div>
    </Screen>
  );
}
