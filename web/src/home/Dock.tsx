// Dock mode (T-103, T-156, T-157): set it on its side and r1os becomes a bedside clock, run by touch. The R1 comes up
// the page upright and turns onto its side once it's well in view (back upright when it's scrolled most of the way out
// below); held there a moment (0.7 s, as the R1 waits before its screen turns), it plays its docked story, again every few seconds while it's on screen. Beside it, four points light
// in turn with the story: resting, worked by touch, talking to an agent, on battery, and resting again. With reduced
// motion it's on its side from the start, still.

import { Icon } from "@r1os/icons";
import { AnimatePresence, motion, useMotionValue, useMotionValueEvent, useReducedMotion, useScroll, useSpring } from "motion/react";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { R1 } from "../device/R1";
import { DOCK_STORY_MS, type DockPhase, DockScreen } from "../device/screens/Dock";
import { GlanceScreen } from "../device/screens/Glance";
import { Still } from "../device/ui";
import { useLook } from "../site/look";
import { soft, spring } from "../site/motion";
import { rest, useSeen } from "../site/seen";
import { Head } from "./parts";
import "../styles/dock.css";

const POINTS: { id: DockPhase; icon: string; title: string; line: string }[] = [
  { id: "rest", icon: "moon", title: "rests when you do", line: "after a while untouched it goes still and dims to save battery." },
  { id: "touch", icon: "hand-tap", title: "touch first", line: "swipe up to unlock, swipe through your agents, tap one for its chat." },
  { id: "chat", icon: "chat-circle-dots", title: "talk by touch", line: "tap the mic, say it, then send or cancel." },
  { id: "power", icon: "plugs-connected", title: "plugged in or not", line: "it stays on while it charges, or on battery until it runs low." },
];

/** How long it's held still on its side before its screen turns (Shell.kt TILT_SETTLE_MS). */
const SETTLE_MS = 700;
const fade = { initial: { opacity: 0 }, animate: { opacity: 1 }, exit: { opacity: 0 }, transition: { duration: 0.3 } } as const;

export function DockMode() {
  const { look } = useLook();
  const accent = look.accent;
  const box = useRef<HTMLElement>(null);
  const stage = useRef<HTMLDivElement>(null);
  const seen = useSeen(box);
  const reduce = useReducedMotion() ?? false;

  // Upright as it comes up the page; it turns onto its side once it is well in view, and back only once it's most of
  // the way out below. Two states with a gap between them, so it never rests part-way over, whatever the window's size.
  const { scrollYProgress } = useScroll({ target: stage, offset: ["start end", "center center"] });
  const over = useMotionValue(0);
  const tip = (p: number) => {
    if (p > 0.4) over.set(1);
    else if (p < 0.25) over.set(0);
  };
  useMotionValueEvent(scrollYProgress, "change", tip);
  const side = useSpring(over, { stiffness: 150, damping: 22, mass: 1 });

  // Docked once it has been still on its side a moment; upright again as soon as it tips back.
  const [docked, setDocked] = useState(false);
  const settle = useRef<ReturnType<typeof setTimeout>>(undefined);
  const check = (v: number) => {
    if (v > 0.97) {
      settle.current ??= setTimeout(() => setDocked(true), SETTLE_MS);
      return;
    }
    clearTimeout(settle.current);
    settle.current = undefined;
    if (v < 0.9) setDocked(false);
  };
  useMotionValueEvent(side, "change", check);
  useEffect(() => {
    tip(scrollYProgress.get());
    check(side.get());
    return () => clearTimeout(settle.current);
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // The story plays while it's docked and seen, from the start each time.
  const playing = seen && docked && !reduce;
  const [round, setRound] = useState(0);
  const [phase, setPhase] = useState<DockPhase | null>(null);
  useEffect(() => {
    if (!playing) return;
    const id = setInterval(() => setRound((r) => r + 1), DOCK_STORY_MS);
    return () => clearInterval(id);
  }, [playing]);
  const lit = playing ? phase : null;

  // Off screen, the glass stays dark: nothing on it runs.
  let screen: ReactNode = null;
  if (reduce)
    screen = (
      <div key="still" className="r1-slide">
        <Still>
          <DockScreen accent={accent} />
        </Still>
      </div>
    );
  else if (playing)
    screen = (
      <motion.div key={`dock${round}`} className="r1-slide" {...fade}>
        <DockScreen accent={accent} onPhase={setPhase} />
      </motion.div>
    );
  else if (seen && !docked)
    screen = (
      <motion.div key="upright" className="r1-slide" {...fade}>
        <GlanceScreen accent={accent} />
      </motion.div>
    );

  return (
    <section className="section" id="dock" ref={box} {...rest(seen)}>
      <div className="wrap">
        <Head title="set it on its side." sub="with dock mode on, it becomes a bedside clock with your widgets and agents, run by touch." link={{ to: "/docs/using#dock-mode", label: "how dock mode works" }} />
        <div className="dock">
          <div className="dock-device" ref={stage} aria-hidden>
            <R1 glow={accent} docked={reduce || side}>
              <AnimatePresence initial={false}>{screen}</AnimatePresence>
            </R1>
          </div>
          <ul className="dock-points">
            {POINTS.map((p, i) => {
              const on = lit === p.id;
              return (
                <motion.li key={p.id} className={`dock-point${on ? " on" : ""}`} {...(reduce ? {} : { initial: { opacity: 0, y: 18 }, whileInView: { opacity: 1, y: 0 }, viewport: { once: true, margin: "-40px" }, transition: { ...soft, delay: i * 0.06 } })}>
                  {on && <motion.span className="dock-point-fill" layoutId="dock-point-fill" transition={spring} />}
                  <span className="dock-icon">
                    <Icon name={p.icon} weight="bold" size={22} />
                  </span>
                  <b>{p.title}</b>
                  <span className="dock-line">{p.line}</span>
                </motion.li>
              );
            })}
          </ul>
        </div>
      </div>
    </section>
  );
}
