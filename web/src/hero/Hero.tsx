// The home page's hero: "your agents," and the reel of what they do, over an R1 showing each one. The two turn
// together: as the words spring to the next detent, the R1's wheel ticks and its screen changes to that story; for dock
// mode the R1 springs onto its side, and back after. It plays in once: the line rises word by word, the reel turns to
// its first detent, the R1 lands. Under the calls to act, one line says where r1os stands today. The stories wait while
// you hover the R1 or have the keyboard's focus on their controls, stop for good with the pause button, and rest when
// they're scrolled away or the tab is hidden (battery first, even here): the reel stops and the screen's story is put
// away, to start again from the top when it's seen. With reduced motion it stays on its first story, and the dots or a
// click on the wheel step it by hand.

import { Icon } from "@r1os/icons";
import { AnimatePresence, motion, useReducedMotion, type Variants } from "motion/react";
import { type FocusEvent, Fragment, useEffect, useRef, useState } from "react";
import { Link } from "react-router";
import { R1 } from "../device/R1";
import { FEATURES, SPOKEN } from "./features";
import { Reel } from "./Reel";
import { soft, spring } from "../site/motion";
import { REPO } from "../site/repo";
import { rest } from "../site/seen";

const mod = (i: number, n: number) => ((i % n) + n) % n;

const LINE = "your agents,";
const REEL = FEATURES.map((x) => ({ text: x.phrase, color: x.accent }));

/** When each part of the hero comes in (s): the line's words one after another, then the reel's first detent. */
const WORD_AT = 0.04;
const WORD_GAP = 0.08;
const DETENT_AT = 0.3;

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
  const [focused, setFocused] = useState(false);
  const [paused, setPaused] = useState(false);
  // Its own watch rather than useSeen's, which starts unseen: the hero is on screen when the page opens, and its first
  // story mustn't start twice.
  const [seen, setSeen] = useState(true);
  const [widest, setWidest] = useState<number>();
  const box = useRef<HTMLElement>(null);
  const n = FEATURES.length;
  const f = FEATURES[mod(index, n)]!;
  const running = !held && !focused && !paused && seen && !reduce;

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
  // The keyboard's focus on the dots or the pause button holds the stories; a click's focus doesn't.
  const focusIn = (e: FocusEvent<HTMLDivElement>) => {
    try {
      setFocused(e.target.matches(":focus-visible"));
    } catch {
      setFocused(true);
    }
  };
  const focusOut = (e: FocusEvent<HTMLDivElement>) => {
    if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setFocused(false);
  };
  const enter = (delay: number, y = 10) => ({ initial: reduce ? false : { opacity: 0, y }, animate: { opacity: 1, y: 0 }, transition: { ...spring, delay } }) as const;
  return (
    <section className="hero" ref={box} style={{ ["--feature" as string]: f.accent }} {...rest(seen)}>
      <div className="hero-glow" aria-hidden />
      <div className="wrap hero-inner">
        <h1 className="hero-title" style={widest ? { ["--reel-em" as string]: widest.toFixed(3) } : undefined}>
          <span className="hero-line">
            {LINE.split(" ").map((w, i) => (
              <Fragment key={i}>
                {i > 0 && " "}
                <motion.span className="hero-word" initial={reduce ? false : { opacity: 0, y: "0.25em" }} animate={{ opacity: 1, y: "0em" }} transition={{ ...soft, delay: WORD_AT + i * WORD_GAP }}>
                  {w}
                </motion.span>
              </Fragment>
            ))}{" "}
          </span>
          <Reel items={REEL} index={index} delay={DETENT_AT} onWidest={setWidest} />
          <span className="sr">{SPOKEN}</span>
        </h1>
        <motion.p className="hero-lede" {...enter(DETENT_AT + 0.06)}>
          r1os is an agent-first os for the rabbit r1: the real claude and codex built in, with a full android device underneath.
        </motion.p>
        <motion.div className="hero-ctas" {...enter(DETENT_AT + 0.12)}>
          <Link to="/docs/start" className="btn primary">
            install r1os
          </Link>
          <Link to="/docs/develop#the-simulator" className="btn ghost">
            try the simulator <Icon name="arrow-right" weight="bold" size={16} />
          </Link>
        </motion.div>
        <motion.p className="hero-today" {...enter(DETENT_AT + 0.16)}>
          for now you build it from <a href={REPO}>the source</a> and flash it yourself. the companion is getting an installer that does it for you.
        </motion.p>
        <motion.div
          className="hero-device"
          initial={reduce ? false : { opacity: 0, y: 24, scale: 0.98 }}
          animate={{ opacity: 1, y: 0, scale: 1 }}
          transition={{ type: "spring", stiffness: 120, damping: 24, delay: DETENT_AT + 0.1 }}
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
          <div className="hero-controls" onFocus={focusIn} onBlur={focusOut}>
            <div className="hero-dots" role="group" aria-label="what your agents do">
              {FEATURES.map((x, i) => {
                const on = i === mod(index, n);
                return (
                  <button key={x.id} className="hero-dot" aria-label={x.phrase} aria-current={on ? "true" : undefined} onClick={() => go(i)}>
                    <motion.span animate={{ width: on ? 26 : 7 }} transition={spring} style={{ background: on ? x.accent : "var(--faint)" }} />
                  </button>
                );
              })}
            </div>
            {!reduce && (
              <button className="hero-pause" onClick={() => setPaused((p) => !p)} aria-pressed={paused} aria-label={paused ? "play the stories" : "pause the stories"}>
                <Icon name={paused ? "play" : "pause"} weight="fill" size={14} />
              </button>
            )}
          </div>
          <AnimatePresence mode="popLayout" initial={false}>
            <motion.span key={f.id} className="hero-how-text" initial={{ opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0, y: -8 }} transition={spring}>
              {f.how}
            </motion.span>
          </AnimatePresence>
        </div>
      </div>
    </section>
  );
}
