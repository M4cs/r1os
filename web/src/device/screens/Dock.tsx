// Dock mode (T-103, T-156, T-157; host/…/shell/Glance.kt setDock, HomeScreen.kt dockMic, VoiceOverlay.kt): r1os on
// its side, in 480×360 dp, for an R1 that's `docked`. The glance screen resting, dimmed and still; a touch wakes it and
// the four agents along the bottom stir, each to its own beat; a swipe turns its page; a swipe up unlocks to home, and
// another brings the next agent; a tap on the mic, a few words, send. Back on the clock it's unplugged and stays on,
// then rests again, as it began. Touches are drawn as a soft finger.

import { Icon } from "@r1os/icons";
import { AnimatePresence, animate, motion, type MotionValue, useMotionValue, useTransform } from "motion/react";
import { type CSSProperties, type ReactNode, useEffect, useMemo, useState } from "react";
import { Character, colorOf } from "@shell/ui/Character";
import { noiseUrl } from "@shell/ui/noise";
import { ClockFace, hm } from "@companion/r1screen/Clock";
import { member, type Member } from "../crew";
import { LandscapeScreen } from "../R1";
import { INK, T, useSteps } from "../ui";

/** The story's beats, in ms from the start; each brings the next step. */
const AT = [1300, 2050, 2900, 3700, 4500, 4750, 5150, 5450, 5750, 6150, 6750, 7250, 7950];
const S = { wake: 1, page: 2, unlock: 3, next: 4, tap: 5, listen: 6, send: 10, back: 11, unplug: 12, rest: 13 } as const;
/** The whole story, with a moment of rest at its end: how often a page can play it again. It ends at rest, as it
 *  starts, so one round runs into the next. */
export const DOCK_STORY_MS = 8700;

/** What the story is showing: resting, worked by touch, talking to an agent, unplugged and still on. */
export type DockPhase = "rest" | "touch" | "chat" | "power";
const phaseOf = (s: number): DockPhase => (s >= S.rest ? "rest" : s >= S.back ? "power" : s >= S.tap ? "chat" : s >= S.wake ? "touch" : "rest");

/** Along the bottom of the glance screen (Glance.kt dockAgents), and on home in the same order. */
const FOUR: Member[] = ["sol", "kiwi", "nova", "bolt"].map(member);
const KIWI = 1;
const NOVA = 2;
const WORDS = ["what's", "on", "tomorrow?"];

/** Where each touch lands (and slides to), by the step it comes with. */
const TOUCHES: Partial<Record<number, { from: [number, number]; to?: [number, number] }>> = {
  [S.wake]: { from: [150, 116] },
  [S.page]: { from: [432, 96], to: [292, 96] },
  [S.unlock]: { from: [240, 348], to: [240, 168] },
  [S.next]: { from: [240, 300], to: [240, 132] },
  [S.tap]: { from: [440, 320] },
  [S.send]: { from: [297, 278] },
};

export function DockScreen({ accent, onPhase }: { accent: string; onPhase?: (phase: DockPhase) => void }) {
  const step = useSteps(AT);
  // A still screen (reduced motion, or a `Still` around it) starts at its end: it shows the clock awake, not resting.
  const [still] = useState(step === AT.length);
  const [now] = useState(() => new Date());
  const phase = phaseOf(step);
  useEffect(() => onPhase?.(phase), [phase]); // eslint-disable-line react-hooks/exhaustive-deps
  const layer = step < S.unlock || step >= S.back ? "glance" : step < S.listen ? "home" : "voice";
  return (
    <LandscapeScreen>
      <AnimatePresence initial={false}>
        {layer === "home" && (
          <motion.div key="home" className="r1-layer" style={{ zIndex: 1 }} initial={{ opacity: 0, scale: 0.97 }} animate={{ opacity: 1, scale: 1 }} exit={{ opacity: 0, transition: { duration: 0.25, delay: 0.1 } }} transition={{ type: "spring", stiffness: 300, damping: 30 }}>
            <Home step={step} now={now} accent={accent} />
          </motion.div>
        )}
        {layer === "voice" && (
          <motion.div key="voice" className="r1-layer" style={{ zIndex: 2 }} initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }} transition={{ duration: 0.3 }}>
            <Voice step={step} accent={accent} />
          </motion.div>
        )}
        {layer === "glance" && (
          <motion.div key="glance" className="r1-layer" style={{ zIndex: 3 }} initial={{ opacity: 0 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0, y: -72, transition: { duration: 0.35, ease: [0.4, 0, 0.2, 1] } }} transition={{ duration: 0.4 }}>
            <Glance step={step} now={now} accent={accent} still={still} />
          </motion.div>
        )}
      </AnimatePresence>
      {Object.entries(TOUCHES).map(([at, t]) => {
        const s = Number(at);
        // A touch shows from its beat into the next one, and fades by itself.
        return t && step >= s && step <= s + 1 ? <Finger key={at} from={t.from} to={t.to} /> : null;
      })}
    </LandscapeScreen>
  );
}

// ---- the glance screen, docked ----------------------------------------------------------------------------------

function Glance({ step, now, accent, still }: { step: number; now: Date; accent: string; still: boolean }) {
  const awake = step >= S.wake && (step < S.rest || still);
  return (
    <div className={`r1-layer${awake ? "" : " r1-still"}`}>
      <Sky />
      <div style={{ position: "absolute", left: 20, top: 22 }}>
        <ClockFace face="stacked" now={now} font="Rubik" t={T} accent={accent} />
        <div style={{ marginTop: 8, fontSize: 17, fontWeight: 500, color: T.muted }}>{now.toLocaleDateString("en-US", { weekday: "long", month: "long", day: "numeric" })}</div>
        <Status charging={step < S.unplug} style={{ marginTop: 8 }} />
      </div>
      <Pages step={step} />
      {FOUR.map((m, i) => (
        <Peek key={m.name} m={m} i={i} stir={awake && step < S.unlock} />
      ))}
      {/* Resting: the backlight down (Shell.kt REST_BRIGHTNESS), and slower going down than coming up; the waking touch
          only wakes it. */}
      <motion.div className="r1-layer" style={{ background: "#000", pointerEvents: "none" }} initial={false} animate={{ opacity: awake ? 0 : 0.55 }} transition={{ duration: awake ? 0.3 : 0.6 }} />
    </div>
  );
}

/** The look's backdrop, as a soft dark mesh with grain. */
function Sky() {
  const grain = useMemo(() => noiseUrl("grain"), []);
  return (
    <div
      className="r1-layer"
      style={{
        background:
          "radial-gradient(55% 75% at 14% 18%, rgba(78,161,255,.22), transparent 70%), radial-gradient(50% 70% at 86% 26%, rgba(176,124,246,.2), transparent 72%), radial-gradient(60% 60% at 58% 104%, rgba(240,106,168,.16), transparent 70%), #030304",
      }}
    >
      {grain && <div className="r1-layer" style={{ backgroundImage: `url(${grain})`, backgroundSize: "96px 96px", opacity: 0.55 }} />}
    </div>
  );
}

/** Battery, charging and Wi-Fi, in the muted colour. */
function Status({ charging, style }: { charging: boolean; style?: CSSProperties }) {
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 3, color: T.muted, fontSize: 13, fontWeight: 600, fontVariantNumeric: "tabular-nums", ...style }}>
      92%
      <AnimatePresence initial={false}>
        {charging && (
          <motion.span key="bolt" style={{ display: "inline-flex" }} exit={{ opacity: 0, scale: 0.3 }} transition={{ duration: 0.35 }}>
            <Icon name="lightning" weight="fill" size={13} />
          </motion.span>
        )}
      </AnimatePresence>
      <Icon name="wifi-high" weight="bold" size={15} style={{ marginLeft: 8 }} />
    </div>
  );
}

type Page = "weather" | "water";
const PAGES: Page[] = ["weather", "water"];
/** Each page's card is as tall as what's on it; the dots follow it. */
const HEIGHT: Record<Page, number> = { weather: 172, water: 96 };
/** A page comes in from the right as the one before it leaves to the left. */
const SLIDE = { in: { x: 232 }, at: { x: 0 }, out: { x: -232 } };

/** The pages at the top right, one card at a time with its dots under it. */
function Pages({ step }: { step: number }) {
  const page: Page = step >= S.page ? "water" : "weather";
  return (
    <div style={{ position: "absolute", left: 250, top: 18, width: 216 }}>
      <div style={{ position: "relative", height: Math.max(...Object.values(HEIGHT)), overflow: "hidden" }}>
        <AnimatePresence initial={false}>
          <motion.div key={page} variants={SLIDE} initial="in" animate="at" exit="out" transition={{ type: "spring", stiffness: 360, damping: 36 }} style={{ position: "absolute", left: 0, right: 0, top: 0, height: HEIGHT[page] }}>
            {page === "weather" ? <Weather /> : <Water />}
          </motion.div>
        </AnimatePresence>
      </div>
      <motion.div initial={false} animate={{ y: HEIGHT[page] + 8 }} transition={{ type: "spring", stiffness: 360, damping: 34 }} style={{ position: "absolute", left: 0, right: 0, top: 0, display: "flex", justifyContent: "center", gap: 8 }}>
        {PAGES.map((p) => (
          <motion.span key={p} initial={false} animate={{ opacity: p === page ? 1 : 0.45 }} transition={{ type: "spring", stiffness: 420, damping: 30 }} style={{ width: 6, height: 6, borderRadius: 3, background: p === page ? T.fg : T.muted }} />
        ))}
      </motion.div>
    </div>
  );
}

function Panel({ children, style }: { children: ReactNode; style?: CSSProperties }) {
  return <div style={{ position: "absolute", inset: 0, borderRadius: 20, background: T.card, padding: "14px 16px", ...style }}>{children}</div>;
}

/** A weather app's card, somewhere made up. */
function Weather() {
  const days: [string, string, number, number][] = [
    ["wed", "cloud", 21, 12],
    ["thu", "sun", 19, 11],
  ];
  return (
    <Panel>
      <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between" }}>
        <span style={{ fontSize: 13, fontWeight: 700, color: colorOf("cyan") }}>ottervale</span>
        <Icon name="cloud-sun" weight="fill" size={22} style={{ color: T.muted }} />
      </div>
      <div style={{ fontSize: 44, fontWeight: 700, letterSpacing: "-0.02em", lineHeight: 1.15 }}>18°</div>
      <div style={{ fontSize: 13, fontWeight: 600, color: T.muted }}>partly cloudy · feels 16°</div>
      <div style={{ display: "flex", flexDirection: "column", gap: 6, marginTop: 12 }}>
        {days.map(([d, icon, hi, lo]) => (
          <div key={d} style={{ display: "flex", alignItems: "center", gap: 10, fontSize: 14, fontWeight: 600 }}>
            <span style={{ width: 34 }}>{d}</span>
            <Icon name={icon} weight="bold" size={16} style={{ color: T.muted }} />
            <span style={{ flex: 1 }} />
            <span style={{ fontWeight: 700 }}>{hi}°</span>
            <span style={{ color: T.muted, width: 28, textAlign: "right" }}>{lo}°</span>
          </div>
        ))}
      </div>
    </Panel>
  );
}

/** The water app's glance card (fixtures/apps/water.enee.yaml), a stats card as r1d's cards are drawn. */
function Water() {
  return (
    <Panel style={{ padding: "12px 14px 14px" }}>
      <div style={{ fontSize: 13, fontWeight: 700, color: colorOf("blue") }}>water</div>
      <div style={{ display: "flex", marginTop: 10 }}>
        {[
          ["today", "3 of 8"],
          ["last glass", "9:40"],
        ].map(([label, value]) => (
          <div key={label} style={{ flex: 1, minWidth: 0 }}>
            <div style={{ fontSize: 13, fontWeight: 600, color: T.muted }}>{label}</div>
            <div style={{ fontSize: 24, fontWeight: 700 }}>{value}</div>
          </div>
        ))}
      </div>
    </Panel>
  );
}

/** Each agent's own rhythm along the bottom (CharacterView's calm beat), and how it stirs when the screen wakes. */
const BEATS = [
  { "--beat": "3.2s", "--blink": "4.6s", "--lag": "0s" },
  { "--beat": "2.7s", "--blink": "5.6s", "--lag": "-0.9s" },
  { "--beat": "4.1s", "--blink": "6.1s", "--lag": "-1.7s" },
  { "--beat": "3.6s", "--blink": "4.9s", "--lag": "-2.4s" },
];
const STIR = [
  { y: [0, -14, 0, -4, 0] },
  { scaleY: [1, 0.86, 1.07, 0.98, 1], scaleX: [1, 1.1, 0.95, 1.01, 1] },
  { rotate: [0, -7, 5, -2, 0] },
  { y: [0, -7, 0], scaleY: [1, 1.04, 1] },
];

/** One of the four, 128 dp with 92 of it showing (DOCK_CHAR_DP, DOCK_PEEK_DP); the row is centred, wider than the
 *  screen, so the ends lose a little of their empty margin. */
function Peek({ m, i, stir }: { m: Member; i: number; stir: boolean }) {
  return (
    <motion.div
      className="r1-peek"
      style={{ position: "absolute", left: (480 - 4 * 128) / 2 + i * 128, top: 360 - 92, width: 128, height: 128, originY: 1, ...(BEATS[i] as CSSProperties) }}
      animate={stir ? STIR[i] : undefined}
      transition={{ duration: 0.7, delay: 0.06 + i * 0.12, ease: "easeInOut" }}
    >
      <Character form={m.form} color={m.color} finish={m.finish} eyes={m.eyes} wear={m.wear} state="idle" size={128} />
    </motion.div>
  );
}

// ---- home, docked ---------------------------------------------------------------------------------------------

/** Home on its side (HomeScreen.kt): the time, the agent on screen and its name, the side dots, and the mic in the
 *  corner. A swipe up brings the next agent with the wheel's own spring; no neighbours show in landscape. */
function Home({ step, now, accent }: { step: number; now: Date; accent: string }) {
  const at = step >= S.next ? NOVA : KIWI;
  const pos = useMotionValue(at);
  useEffect(() => {
    const a = animate(pos, at, { type: "spring", stiffness: 300, damping: 24 });
    return () => a.stop();
  }, [at, pos]);
  const focus = FOUR[at]!;
  const tapped = step >= S.tap;
  return (
    <div className="r1-layer">
      <Sky />
      <div style={{ position: "absolute", left: 20, top: 14 }}>
        <div style={{ fontSize: 34, fontWeight: 700, letterSpacing: "-0.01em", fontVariantNumeric: "tabular-nums" }}>{hm(now)}</div>
        <Status charging style={{ marginTop: 4 }} />
      </div>
      <AnimatePresence mode="popLayout" initial={false}>
        <motion.div
          key={focus.name}
          initial={{ opacity: 0, y: 12 }}
          animate={{ opacity: 1, y: 0 }}
          exit={{ opacity: 0, y: -12 }}
          transition={{ type: "spring", stiffness: 400, damping: 30 }}
          style={{ position: "absolute", left: 0, right: 0, top: 16, textAlign: "center", fontSize: 30, fontWeight: 700, letterSpacing: "-0.01em" }}
        >
          {focus.name}
        </motion.div>
      </AnimatePresence>
      {FOUR.map((m, i) => (
        <HomeAgent key={m.name} m={m} i={i} pos={pos} />
      ))}
      <div style={{ position: "absolute", left: 480 - 17 - 6, top: 180 - (5 * 6 + 12 + 4 * 7) / 2, width: 6, display: "flex", flexDirection: "column", gap: 7 }}>
        {[...FOUR, null].map((m, i) => (
          <motion.div key={i} initial={false} animate={{ height: i === at ? 18 : 6, background: m && i === at ? colorOf(m.color) : T.subtle }} transition={{ type: "spring", stiffness: 420, damping: 30 }} style={{ width: 6, borderRadius: m ? 3 : 1 }} />
        ))}
      </div>
      {/* dockMic: 48 dp in the corner, the accent at 20% behind the accent's microphone. */}
      <motion.div
        animate={tapped ? { scale: [1, 0.84, 1.12, 1] } : { scale: 1 }}
        transition={{ duration: 0.35 }}
        style={{ position: "absolute", left: 480 - 16 - 48, top: 360 - 16 - 48, width: 48, height: 48, borderRadius: 24, display: "grid", placeItems: "center", background: `${accent}33`, color: accent }}
      >
        <Icon name="microphone" weight="fill" size={22} />
      </motion.div>
    </div>
  );
}

function HomeAgent({ m, i, pos }: { m: Member; i: number; pos: MotionValue<number> }) {
  const y = useTransform(pos, (p) => 66 + (i - p) * 250);
  const opacity = useTransform(pos, (p) => 1 - Math.min(Math.abs(i - p), 1));
  const scale = useTransform(pos, (p) => 1 - 0.3 * Math.min(Math.abs(i - p), 1));
  return (
    <motion.div style={{ position: "absolute", left: 240 - 106, top: 0, width: 212, height: 212, y, opacity, scale }}>
      <Character form={m.form} color={m.color} finish={m.finish} eyes={m.eyes} wear={m.wear} state="idle" size={212} />
    </motion.div>
  );
}

// ---- talking, docked ------------------------------------------------------------------------------------------

/** The voice overlay on its side (VoiceOverlay.kt touchControls): the agent in its ring, "listening…" breathing, five
 *  level bars, "talk, then send", and cancel and send. */
function Voice({ step, accent }: { step: number; accent: string }) {
  const m = FOUR[NOVA]!;
  const c = colorOf(m.color);
  const heard = Math.max(0, Math.min(step - S.listen, WORDS.length));
  const sent = step >= S.send;
  const pill: CSSProperties = { padding: "10px 22px", borderRadius: 22, fontSize: 17, fontWeight: 700 };
  return (
    <div className="r1-layer" style={{ background: "#000" }}>
      <div style={{ position: "absolute", left: 240 - 56, top: 14, width: 112, height: 112, display: "grid", placeItems: "center" }}>
        <div className={`r1-ring${sent ? " done" : ""}`} style={{ position: "absolute", inset: 0, borderRadius: "50%", border: `2px solid ${c}` }} />
        <Character form={m.form} color={m.color} finish={m.finish} eyes={m.eyes} wear={m.wear} state={sent ? "thinking" : "listening"} size={92} />
      </div>
      <div style={{ position: "absolute", left: 20, right: 20, top: 138, height: 38, textAlign: "center", fontSize: 28, whiteSpace: "nowrap" }}>
        {heard === 0 ? (
          <span className="r1-breathe" style={{ display: "inline-block", fontWeight: 500, color: T.muted }}>
            listening…
          </span>
        ) : (
          WORDS.slice(0, heard).map((w, i) => (
            <motion.span key={w} initial={{ opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0 }} transition={{ type: "spring", stiffness: 380, damping: 22 }} style={{ display: "inline-block", marginLeft: i ? 8 : 0, fontWeight: 650, color: T.fg }}>
              {w}
            </motion.span>
          ))
        )}
      </div>
      <div style={{ position: "absolute", left: 0, right: 0, top: 188, height: 24, display: "flex", justifyContent: "center", alignItems: "center", gap: 6 }}>
        {[0.5, 0.85, 1, 0.75, 0.55].map((hi, i) => (
          <span
            key={i}
            className={`r1-level${sent ? " done" : heard > 0 ? " loud" : ""}`}
            style={{ width: 6, height: 24, borderRadius: 3, background: c, animationDelay: `${-i * 0.17}s`, ["--hi" as string]: hi, ["--lo" as string]: 0.22 + (i % 2) * 0.1 }}
          />
        ))}
      </div>
      <div style={{ position: "absolute", left: 0, right: 0, top: 226, textAlign: "center", fontSize: 13, fontWeight: 500, color: T.muted }}>{sent ? `sent to ${m.name}` : "talk, then send"}</div>
      <div style={{ position: "absolute", left: 0, right: 0, top: 258, display: "flex", justifyContent: "center", gap: 12 }}>
        <span style={{ ...pill, background: T.card, color: T.fg }}>cancel</span>
        <motion.span animate={sent ? { scale: [1, 0.9, 1.05, 1] } : { scale: 1 }} transition={{ duration: 0.35 }} style={{ ...pill, display: "inline-block", background: accent, color: INK }}>
          send
        </motion.span>
      </div>
    </div>
  );
}

// ---- a finger -------------------------------------------------------------------------------------------------

const DOT: CSSProperties = {
  position: "absolute",
  left: -18,
  top: -18,
  width: 36,
  height: 36,
  borderRadius: "50%",
  background: "radial-gradient(circle, rgba(255,255,255,.8) 0%, rgba(255,255,255,.42) 55%, rgba(255,255,255,.12) 100%)",
  boxShadow: "0 0 18px rgba(255,255,255,.28)",
  pointerEvents: "none",
  zIndex: 10,
};

/** A touch, drawn: a soft dot that lands at `from` (a tap ripples), and with `to` slides there before it lifts. */
function Finger({ from, to }: { from: [number, number]; to?: [number, number] }) {
  const [x, y] = from;
  if (to)
    return (
      <motion.div
        style={DOT}
        initial={{ x, y, opacity: 0, scale: 0.7 }}
        animate={{ x: [x, x, to[0], to[0]], y: [y, y, to[1], to[1]], opacity: [0, 0.9, 0.9, 0], scale: [0.7, 1, 1, 0.85] }}
        transition={{ duration: 0.62, times: [0, 0.2, 0.8, 1], ease: "easeInOut" }}
      />
    );
  return (
    <>
      <motion.div style={DOT} initial={{ x, y, opacity: 0, scale: 0.7 }} animate={{ opacity: [0, 0.9, 0.9, 0], scale: [0.7, 1, 0.92, 0.9] }} transition={{ duration: 0.45, times: [0, 0.25, 0.6, 1] }} />
      <motion.div
        style={{ ...DOT, background: "none", boxShadow: "none", border: "2px solid rgba(255,255,255,.7)" }}
        initial={{ x, y, opacity: 0.8, scale: 0.5 }}
        animate={{ opacity: 0, scale: 2 }}
        transition={{ duration: 0.55, ease: "easeOut", delay: 0.06 }}
      />
    </>
  );
}
