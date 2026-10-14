// The home page's hero: "your agents," and the reel of what they do, over an R1 showing each one. The two turn
// together: as the words spring to the next detent, the R1's wheel ticks and its screen changes to that story; for dock
// mode the R1 springs onto its side, and back after. It plays in once: the line rises word by word, the reel turns to
// its first detent, the R1 lands and the facts' icons pop in under it. It waits while you hover the R1, and rests when
// it's scrolled away or the tab is hidden (battery first, even here): the reel stops and the screen's story is put
// away, to start again from the top when it's seen. With reduced motion it stays on its first story, and the dots or a
// click on the wheel step it by hand.

import { Icon } from "@r1os/icons";
import { AnimatePresence, motion, useInView, useReducedMotion, type Variants } from "motion/react";
import { Fragment, useEffect, useRef, useState } from "react";
import { Link } from "react-router";
import { R1 } from "../device/R1";
import { FEATURES, SPOKEN } from "./features";
import { Reel } from "./Reel";
import { bouncy, soft, spring } from "../site/motion";
import { rest } from "../site/seen";

const mod = (i: number, n: number) => ((i % n) + n) % n;

const LINE = "your agents,";
const REEL = FEATURES.map((x) => ({ text: x.phrase, color: x.accent }));

/** When each part of the hero comes in (s): the line's words one after another, then the reel's first detent. */
const WORD_AT = 0.04;
const WORD_GAP = 0.08;
const DETENT_AT = 0.3;

const FACTS = [
  { icon: "android-logo", text: "a full android\ndevice" },
  { icon: "cursor-click", text: "agents that use\nyour apps" },
  { icon: "key", text: "your own claude\nor codex" },
  { icon: "git-branch", text: "open source,\nmit licence" },
];

/** A fact rising in, and its icon popping a beat after (`custom`: its place). */
const FACT: Variants = {
  hidden: { opacity: 0, y: 14 },
  show: (i: number) => ({ opacity: 1, y: 0, transition: { ...soft, delay: i * 0.06 } }),
};
const FACT_ICON: Variants = {
  hidden: { opacity: 0, scale: 0.8 },
  show: (i: number) => ({ opacity: 1, scale: 1, transition: { type: "spring", stiffness: 420, damping: 15, delay: 0.1 + i * 0.06 } }),
};

/** A story's screen coming and going: up the glass, or in place while the R1 turns onto its side or back
 *  (`custom`: whether it's turning), so the turn is the only movement. */
const SLIDE: Variants = {
  in: (turning: boolean) => ({ opacity: 0, y: turning ? 0 : 26, scale: 0.98 }),
  on: { opacity: 1, y: 0, scale: 1 },
  out: (turning: boolean) => ({ opacity: 0, y: turning ? 0 : -26, scale: 0.98 }),
};

export function Hero() {
  const reduce = useReducedMotion();
  const [index, setIndex] = useState(0);
  const [held, setHeld] = useState(false);
  // Its own watch rather than useSeen's, which starts unseen: the hero is on screen when the page opens, and its first
  // story mustn't start twice.
  const [seen, setSeen] = useState(true);
  const [landed, setLanded] = useState(false);
  const [widest, setWidest] = useState<number>();
  const box = useRef<HTMLElement>(null);
  const facts = useRef<HTMLDivElement>(null);
  const factsIn = useInView(facts, { once: true, margin: "0px 0px -40px 0px" });
  const n = FEATURES.length;
  const f = FEATURES[mod(index, n)]!;
  const running = !held && seen && !reduce;

  // Docked for dock mode's story. The screens either side of a turn fade in place rather than slide.
  const docked = f.id === "dock";
  const was = useRef(docked);
  const turning = docked !== was.current;
  useEffect(() => {
    was.current = docked;
  }, [docked]);

  useEffect(() => {
    const el = box.current;
    if (!el) return;
    const io = new IntersectionObserver(([e]) => setSeen(!!e && e.isIntersecting && e.intersectionRatio >= 0.15 && document.visibilityState === "visible"), { threshold: [0, 0.15] });
    io.observe(el);
    const vis = () => setSeen(document.visibilityState === "visible" && el.getBoundingClientRect().bottom > 0);
    document.addEventListener("visibilitychange", vis);
    return () => {
      io.disconnect();
      document.removeEventListener("visibilitychange", vis);
    };
  }, []);
  useEffect(() => {
    if (!running) return;
    const id = setTimeout(() => setIndex((i) => i + 1), f.ms);
    return () => clearTimeout(id);
  }, [index, running, f.ms]);

  const go = (to: number) => setIndex((i) => i + mod(to - mod(i, n), n));
  const enter = (delay: number, y = 10) => ({ initial: reduce ? false : { opacity: 0, y }, animate: { opacity: 1, y: 0 }, transition: { ...spring, delay } }) as const;
  return (
    <section className="hero" ref={box} style={{ ["--feature" as string]: f.accent }} {...rest(seen)}>
      <div className="hero-glow" aria-hidden />
      <div className="wrap hero-inner">
        <motion.span className="readout" {...enter(0, 8)}>
          r1os <b>· an agent-first os</b>
        </motion.span>
        <h1 className="hero-title" style={widest ? { ["--reel-em" as string]: widest.toFixed(3) } : undefined}>
          <span className="hero-line">
            <span className="sr">{LINE}</span>
            <span aria-hidden>
              {LINE.split(" ").map((w, i) => (
                <Fragment key={i}>
                  {i > 0 && " "}
                  <motion.span className="hero-word" initial={reduce ? false : { opacity: 0, y: "0.45em" }} animate={{ opacity: 1, y: "0em" }} transition={{ ...bouncy, delay: WORD_AT + i * WORD_GAP }}>
                    {w}
                  </motion.span>
                </Fragment>
              ))}
            </span>
          </span>
          <Reel items={REEL} index={index} delay={DETENT_AT} onWidest={setWidest} />
          <span className="sr">{SPOKEN}</span>
        </h1>
        <motion.p className="hero-lede" {...enter(DETENT_AT + 0.06)}>
          the real claude and codex, built into the os, with a full android device underneath.
        </motion.p>
        <motion.div className="hero-ctas" {...enter(DETENT_AT + 0.12)}>
          <Link to="/docs/start" className="btn primary">
            get started
          </Link>
          <Link to="/docs" className="btn ghost">
            read the docs <Icon name="arrow-right" weight="bold" size={16} />
          </Link>
        </motion.div>
        <motion.div
          className="hero-device"
          initial={reduce ? false : { opacity: 0, y: 40, scale: 0.96 }}
          animate={{ opacity: 1, y: 0, scale: 1 }}
          transition={{ type: "spring", stiffness: 110, damping: 17, delay: DETENT_AT + 0.1 }}
          onAnimationComplete={() => setLanded(true)}
          onMouseEnter={() => setHeld(true)}
          onMouseLeave={() => setHeld(false)}
        >
          <R1 glow={f.accent} turn={index} docked={docked} onWheelClick={() => setIndex((i) => i + 1)}>
            <AnimatePresence initial={false} custom={turning}>
              <motion.div key={index} className="r1-slide" variants={SLIDE} custom={turning} initial="in" animate="on" exit="out" transition={{ type: "spring", stiffness: 260, damping: 28 }}>
                {seen && f.screen(f.accent)}
              </motion.div>
            </AnimatePresence>
          </R1>
        </motion.div>
        <div className="hero-how">
          <div className="hero-dots" role="tablist" aria-label="what your agents do">
            {FEATURES.map((x, i) => {
              const on = i === mod(index, n);
              return (
                <button key={x.id} role="tab" aria-selected={on} aria-label={x.phrase} className="hero-dot" onClick={() => go(i)}>
                  <motion.span animate={{ width: on ? 26 : 7 }} transition={spring} style={{ background: on ? x.accent : "var(--faint)" }} />
                </button>
              );
            })}
          </div>
          <AnimatePresence mode="popLayout" initial={false}>
            <motion.span key={f.id} className="hero-how-text" initial={{ opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0, y: -8 }} transition={spring}>
              {f.how}
            </motion.span>
          </AnimatePresence>
        </div>
        <motion.div className="facts" ref={facts} initial={reduce ? false : "hidden"} animate={reduce || (landed && factsIn) ? "show" : "hidden"}>
          {FACTS.map((x, i) => (
            <motion.div key={x.icon} className="fact" variants={FACT} custom={i}>
              <motion.span className="fact-icon" variants={FACT_ICON} custom={i}>
                <Icon name={x.icon} weight="bold" size={20} />
              </motion.span>
              <span>{x.text}</span>
            </motion.div>
          ))}
        </motion.div>
      </div>
    </section>
  );
}
