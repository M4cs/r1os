// Quick, then quiet: r1os is fast while you use it and idle once it sleeps, with the numbers measured on the device
// (all from /docs/architecture#speed-and-battery and the introduction; change them there first). A band of light
// streaks glides by, near ones quicker than far ones: CSS animations, transform only, resting while the band is out of
// sight or the tab is hidden, and a still frame of them with reduced motion. Under it, four points rise in one after
// another, each led by its number in one of the R1's mono readouts.

import { motion, useReducedMotion, type Variants } from "motion/react";
import { type CSSProperties, useRef } from "react";
import { after, reveal, soft } from "../site/motion";
import { rest, useSeen } from "../site/seen";
import { Head } from "./parts";
import "../styles/fast.css";

const POINTS = [
  { figure: "36 s", title: "on in half the time", line: "the screen is on 36 s after the kernel starts, down from 72 s." },
  { figure: "1.6 s", title: "replies start quickly", line: "a warm claude's first word comes about 1.6 s after your message. it warms up while you're still holding the button." },
  { figure: "60 fps", title: "smooth on the wheel", line: "the wheel, talking and streaming replies run at 60 fps, and your words show up as you say them." },
  { figure: "0 polls", title: "idle when it sleeps", line: "asleep, r1os itself polls nothing, and apps you've left alone close on their own." },
];

/** A streak's lane: where it runs (`y`, % of the band), how thick (px) and long (% of the band) it is, one crossing's
 *  seconds, where it starts in its crossing, and where it stands when nothing moves (`at`, %). Near ones are thicker,
 *  quicker and in the accent; far ones thin, slower and grey. */
type Lane = { y: number; h: number; len: number; dur: number; delay: number; at: number; near?: boolean };

const LANES: Lane[] = [
  { y: 12, h: 2, len: 16, dur: 4.8, delay: -0.7, at: 6 },
  { y: 23, h: 4, len: 30, dur: 2.6, delay: -1.9, at: 54, near: true },
  { y: 34, h: 2, len: 13, dur: 4.1, delay: -3.2, at: 30 },
  { y: 46, h: 6, len: 36, dur: 2.2, delay: -0.5, at: 18, near: true },
  { y: 57, h: 2, len: 19, dur: 4.4, delay: -2.5, at: 72 },
  { y: 66, h: 3, len: 24, dur: 3.1, delay: -1.2, at: 42, near: true },
  { y: 78, h: 2, len: 15, dur: 5.2, delay: -3.9, at: 10 },
  { y: 88, h: 4, len: 27, dur: 2.8, delay: -2.3, at: 62, near: true },
];

const laneStyle = (l: Lane) =>
  ({
    "--y": `${l.y}%`,
    "--h": `${l.h}px`,
    "--len": `${l.len}%`,
    "--dur": `${l.dur}s`,
    "--delay": `${l.delay}s`,
    "--at": `${l.at}%`,
  }) as CSSProperties;

/** A point's accent bar, drawn from the left as the point rises. */
const draw = (delay: number): Variants => ({
  hidden: { scaleX: 0 },
  show: { scaleX: 1, transition: { ...soft, delay } },
});

export function Speed() {
  const reduce = useReducedMotion();
  return (
    <section className="section" id="speed">
      <div className="wrap">
        <Head
          title="quick, then quiet."
          sub="fast while you use it, idle the moment it sleeps, and tuned on the device to stay that way."
          link={{ to: "/docs/architecture#speed-and-battery", label: "how it's measured" }}
        />
        <Streaks still={!!reduce} />
        <motion.ul className="fast-points" {...(reduce ? {} : reveal)}>
          {POINTS.map((p, i) => (
            <motion.li key={p.title} className="fast-point" variants={after(i * 0.09, 12)}>
              <motion.span className="fast-bar" variants={draw(0.15 + i * 0.09)} aria-hidden />
              <span className="readout">
                <b>{p.figure}</b>
              </span>
              <h3>{p.title}</h3>
              <p>{p.line}</p>
            </motion.li>
          ))}
        </motion.ul>
      </div>
    </section>
  );
}

/** The streaks, gliding left to right while the band is seen. */
function Streaks({ still }: { still: boolean }) {
  const ref = useRef<HTMLDivElement>(null);
  const seen = useSeen(ref, 0);
  return (
    <motion.div
      className="fast-flow"
      ref={ref}
      aria-hidden
      {...rest(seen)}
      initial={still ? false : { opacity: 0 }}
      whileInView={{ opacity: 1 }}
      viewport={{ once: true, margin: "-60px" }}
      transition={{ duration: 0.8, ease: "easeOut" }}
    >
      {LANES.map((l) => (
        <span key={l.y} className={l.near ? "fast-lane near" : "fast-lane"} style={laneStyle(l)}>
          <i />
        </span>
      ))}
    </motion.div>
  );
}
