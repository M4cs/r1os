// The R1's screen parts, in dp, as the native shell draws them (host/…/shell, ui/Theme.kt; seen in the harness's
// screenshots): a bold lowercase title at the top left, rows with an icon tile that the accent fills when focused,
// chat bubbles, cards on #1a1c21, mono readouts. Each screen on the page is a little story told with these.

import { Icon } from "@r1os/icons";
import { motion, useReducedMotion } from "motion/react";
import { createContext, type CSSProperties, type ReactNode, useContext, useEffect, useState } from "react";
import { Character } from "@shell/ui/Character";
import { DARK } from "@companion/r1screen/paint";

export const T = DARK;
export const INK = "#0b0b0c";
export const MONO = "'R1 Droid Sans Mono', ui-monospace, monospace";
export const fill = { type: "spring", stiffness: 520, damping: 38, mass: 0.8 } as const;
export const pop = { type: "spring", stiffness: 380, damping: 22 } as const;

/** Screens inside `Still` show the end of their story at once (the small screens around the R1 on the home page). */
const StillCtx = createContext(false);
export function Still({ children }: { children: ReactNode }) {
  return <StillCtx.Provider value={true}>{children}</StillCtx.Provider>;
}

/** Steps through a story: the step reached after each delay (ms from the start), all at once with reduced motion or
 *  when the screen is still. */
export function useSteps(at: number[]): number {
  const still = useContext(StillCtx);
  const reduce = useReducedMotion() || still;
  const [step, setStep] = useState(reduce ? at.length : 0);
  useEffect(() => {
    if (reduce) return;
    const ids = at.map((ms, i) => setTimeout(() => setStep(i + 1), ms));
    return () => ids.forEach(clearTimeout);
  }, []); // eslint-disable-line react-hooks/exhaustive-deps
  return step;
}

/** A count that ticks up every `ms` while `on`, and stops at `max`. With reduced motion it stays at `from`. */
export function useTicker(on: boolean, ms: number, from = 0, max = Infinity): number {
  const reduce = useReducedMotion();
  const [n, setN] = useState(from);
  const run = on && !reduce && n < max;
  useEffect(() => {
    if (!run) return;
    const id = setInterval(() => setN((x) => (x >= max ? x : x + 1)), ms);
    return () => clearInterval(id);
  }, [run, ms, max]);
  return reduce ? from : n;
}

export function Title({ children, icon, color }: { children: ReactNode; icon?: ReactNode; color?: string }) {
  return (
    <div style={{ position: "absolute", left: 16, top: 12, right: 16, display: "flex", alignItems: "center", gap: 10, fontSize: 24, fontWeight: 700, letterSpacing: "-0.01em", color: color ?? T.fg }}>
      {icon}
      {children}
    </div>
  );
}

/** A list row: the icon in its tile, the label and a line under it; the accent fills it when it's focused. */
export function Row({ icon, label, sub, on, accent, big, right, layoutId }: { icon: string; label: string; sub?: string; on?: boolean; accent: string; big?: boolean; right?: ReactNode; layoutId?: string }) {
  const h = big ? 70 : sub ? 58 : 52;
  return (
    <div style={{ position: "relative", height: h, display: "flex", alignItems: "center", gap: 12, padding: "0 10px" }}>
      {on && <motion.div layoutId={layoutId ?? "row-fill"} transition={fill} style={{ position: "absolute", inset: 0, borderRadius: 12, background: accent }} />}
      <div style={{ position: "relative", width: big ? 48 : 38, height: big ? 48 : 38, borderRadius: big ? 12 : 10, display: "grid", placeItems: "center", background: on ? "rgba(0,0,0,.12)" : T.subtle, color: on ? INK : T.fg }}>
        <Icon name={icon} weight="bold" size={big ? 24 : 20} />
      </div>
      <div style={{ position: "relative", flex: 1, minWidth: 0 }}>
        <div style={{ fontSize: big ? 25 : 18, fontWeight: 700, color: on ? INK : T.fg, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>{label}</div>
        {sub && <div style={{ fontSize: 13, fontWeight: 600, color: on ? "rgba(0,0,0,.62)" : T.muted, whiteSpace: "nowrap" }}>{sub}</div>}
      </div>
      {right && <div style={{ position: "relative", fontSize: 13, fontWeight: 700, color: on ? INK : T.muted }}>{right}</div>}
    </div>
  );
}

/** A chat's header: the agent, small, and its name. */
export function ChatHead({ name, form, color, finish, chip }: { name: string; form: string; color: string; finish?: string; chip?: ReactNode }) {
  return (
    <div style={{ position: "absolute", left: 14, top: 12, right: 14, display: "flex", alignItems: "center", gap: 10 }}>
      <Character form={form} color={color} finish={finish ?? "grain"} state="idle" size={34} />
      <div style={{ fontSize: 24, fontWeight: 700, color: T.fg }}>{name}</div>
      <Icon name="camera" weight="bold" size={20} style={{ color: T.muted }} />
      <div style={{ flex: 1 }} />
      {chip}
    </div>
  );
}

/** A message: yours in the agent's colour, the agent's on a card. */
export function Bubble({ mine, color, children, style }: { mine?: boolean; color: string; children: ReactNode; style?: CSSProperties }) {
  return (
    <motion.div
      initial={{ opacity: 0, y: 14, scale: 0.96 }}
      animate={{ opacity: 1, y: 0, scale: 1 }}
      transition={pop}
      style={{
        alignSelf: mine ? "flex-end" : "flex-start",
        maxWidth: "86%",
        padding: "10px 14px",
        borderRadius: mine ? "18px 18px 6px 18px" : "18px 18px 18px 6px",
        background: mine ? color : T.card,
        color: mine ? INK : T.fg,
        fontSize: 17,
        fontWeight: 600,
        lineHeight: 1.25,
        textTransform: "none",
        transformOrigin: mine ? "100% 100%" : "0 100%",
        ...style,
      }}
    >
      {children}
    </motion.div>
  );
}

/** A card, as r1d's cards are drawn (crate::cards; CardView.kt). */
export function Card({ children, title, accent, style }: { children: ReactNode; title?: string; accent: string; style?: CSSProperties }) {
  return (
    <motion.div initial={{ opacity: 0, y: 16 }} animate={{ opacity: 1, y: 0 }} transition={pop} style={{ borderRadius: 18, background: T.card, padding: "12px 14px", textTransform: "none", ...style }}>
      {title && <div style={{ fontSize: 13, fontWeight: 700, color: accent, textTransform: "lowercase", marginBottom: 6 }}>{title}</div>}
      {children}
    </motion.div>
  );
}

/** The column a chat's messages stack in, newest at the bottom. */
export function Thread({ children, top = 62 }: { children: ReactNode; top?: number }) {
  return <div style={{ position: "absolute", left: 12, right: 12, top, bottom: 12, display: "flex", flexDirection: "column", justifyContent: "flex-end", gap: 10, overflow: "hidden" }}>{children}</div>;
}

/** Three dots, rising in turn: the agent is thinking. */
export function Dots({ color }: { color: string }) {
  return (
    <div style={{ alignSelf: "flex-start", display: "flex", gap: 5, padding: "14px 16px", borderRadius: "18px 18px 18px 6px", background: T.card }}>
      {[0, 1, 2].map((i) => (
        <motion.span key={i} style={{ width: 7, height: 7, borderRadius: 4, background: color }} animate={{ y: [0, -4, 0], opacity: [0.35, 1, 0.35] }} transition={{ duration: 1.05, repeat: Infinity, delay: i * 0.15 }} />
      ))}
    </div>
  );
}

export function Mono({ children, style }: { children: ReactNode; style?: CSSProperties }) {
  return <span style={{ fontFamily: MONO, textTransform: "none", ...style }}>{children}</span>;
}
