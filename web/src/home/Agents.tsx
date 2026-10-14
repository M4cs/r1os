// Make them yours: a set of eight agents, each its own look, name, job and brain, drawn by the shell's own Character as
// on the device. The big one turns into the next every 2.4 s (it shrinks away as the next pops in, the glow behind it
// eases to the new colour and the name rolls over), and the set sits under it with the accent's focus moving along, as
// the wheel moves it on the R1. Nobody designs anything here: hovering, focusing or tapping one only shows it. The cycle
// waits while the pointer or the keyboard's focus is on it, and runs only while the section is on screen in a visible
// tab. With reduced motion it's still: the first one, and the set.

import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { type FocusEvent, useEffect, useRef, useState } from "react";
import { Character, colorOf } from "@shell/ui/Character";
import { type Member, member } from "../device/crew";
import { rise, spring } from "../site/motion";
import { rest, useSeen } from "../site/seen";
import { Head } from "./parts";
import "../styles/agents.css";

/** Eight, as many as an R1 holds (r1d's MAX_AGENTS): the crew's own looks where they fit, and four more made up. */
const SET: Member[] = [
  member("pip"),
  member("bolt"),
  member("nova"),
  { name: "biscuit", form: "bunny", color: "amber", finish: "noise", eyes: "sparkle", wear: ["headphones"], role: "picks the music", brain: "claude" },
  member("sol"),
  { name: "figgy", form: "cat", color: "pink", finish: "gradient", eyes: "lashes", wear: ["bow", "blush"], role: "the weekly shop", brain: "claude" },
  { name: "ziggy", form: "blob", color: "teal", finish: "grainy", eyes: "cyclops", wear: ["party", "bell"], role: "builds apps", brain: "codex" },
  { name: "otto", form: "bean", color: "indigo", finish: "soft", eyes: "sleepy", wear: ["beanie", "scarf"], role: "alarms and reminders", brain: "claude" },
];

/** How long each one stays before the next. */
const STAY_MS = 2400;

/** The next one popping in: a little past full size, then settling. */
const pop = { type: "spring", stiffness: 340, damping: 21, mass: 0.9, delay: 0.05, opacity: { duration: 0.2, delay: 0.05 } } as const;
/** The last one going: quick, shrinking and turning away. */
const away = { opacity: 0, scale: 0.6, rotate: 8, transition: { duration: 0.22, ease: [0.4, 0, 1, 1] } } as const;

/** Whether focus came from the keyboard: a click focuses a button too, and that shouldn't hold the cycle. */
const keyboard = (el: Element) => {
  try {
    return el.matches(":focus-visible");
  } catch {
    return true;
  }
};

export function MeetAgents() {
  const section = useRef<HTMLElement>(null);
  const seen = useSeen(section);
  const reduce = useReducedMotion() ?? false;
  const [at, setAt] = useState(0);
  const [hover, setHover] = useState(false);
  const [focus, setFocus] = useState(false);
  const a = SET[at]!;
  const hex = colorOf(a.color);

  // One timer at a time, started afresh whenever the one shown changes, so a picked one gets its full turn.
  useEffect(() => {
    if (!seen || reduce || hover || focus) return;
    const t = setTimeout(() => setAt((i) => (i + 1) % SET.length), STAY_MS);
    return () => clearTimeout(t);
  }, [seen, reduce, hover, focus, at]);

  const leave = (e: FocusEvent<HTMLDivElement>) => {
    if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setFocus(false);
  };

  return (
    <section className="section" id="agents" ref={section} {...rest(seen)}>
      <div className="wrap">
        <Head title="make them yours." sub="up to eight agents, each the real claude or codex, signed in as you. give each a look, a name and a job." />
        <motion.div
          className="agents"
          {...(reduce ? {} : rise)}
          onPointerEnter={(e) => {
            if (e.pointerType === "mouse") setHover(true);
          }}
          onPointerLeave={() => setHover(false)}
          onFocus={(e) => setFocus(keyboard(e.target))}
          onBlur={leave}
        >
          <AnimatePresence initial={false}>
            <motion.div
              key={hex}
              className="agents-glow"
              style={{ ["--agent" as string]: hex }}
              initial={{ opacity: 0, scale: 0.85 }}
              animate={{ opacity: 1, scale: 1 }}
              exit={{ opacity: 0 }}
              transition={{ duration: 0.9, ease: "easeOut" }}
            />
          </AnimatePresence>

          <div className="agents-stage">
            <AnimatePresence initial={false}>
              <motion.div key={a.name} className="agents-big" initial={{ opacity: 0, scale: 0.45, rotate: -12 }} animate={{ opacity: 1, scale: 1, rotate: 0 }} exit={away} transition={pop}>
                <Character form={a.form} color={a.color} finish={a.finish} eyes={a.eyes} wear={a.wear} state="idle" size={240} />
              </motion.div>
            </AnimatePresence>
          </div>

          <div className="agents-says">
            <AnimatePresence mode="popLayout" initial={false}>
              <motion.div
                key={a.name}
                className="agents-line"
                style={{ ["--accent" as string]: hex }}
                initial={{ opacity: 0, y: 22 }}
                animate={{ opacity: 1, y: 0 }}
                exit={{ opacity: 0, y: -22 }}
                transition={spring}
              >
                <div className="agents-name">{a.name}</div>
                <div className="agents-role">
                  {a.role}
                  <span className="agents-dot" aria-hidden>
                    ·
                  </span>
                  <span className="agents-brain">{a.brain}</span>
                </div>
              </motion.div>
            </AnimatePresence>
          </div>

          <div className="agents-row">
            {SET.map((m, i) => (
              <button
                key={m.name}
                type="button"
                className="agents-pick"
                aria-label={`${m.name}: ${m.role}, on ${m.brain}`}
                aria-current={i === at ? "true" : undefined}
                onClick={() => setAt(i)}
                onPointerEnter={(e) => {
                  if (e.pointerType === "mouse") setAt(i);
                }}
                onFocus={() => setAt(i)}
              >
                {i === at && <motion.span className="agents-ring" layoutId="agents-ring" transition={spring} />}
                <Character form={m.form} color={m.color} finish={m.finish} eyes={m.eyes} wear={m.wear} state="idle" size={44} />
              </button>
            ))}
          </div>
        </motion.div>
      </div>
    </section>
  );
}
