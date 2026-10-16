// Agents that use your apps (ADR-0061, ADR-0063; T-113, T-144): an R1 replaying an agent ordering in a made-up food
// app, beside four points that light as the story reaches each one, and the full Android device underneath
// (ADR-0013). It all rests when scrolled away, and shows the end at once with reduced motion.

import { Icon } from "@r1os/icons";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { colorOf } from "@companion/theme";
import { member } from "../device/crew";
import { R1 } from "../device/R1";
import { AppWindowScreen } from "../device/screens/AppWindow";
import { useLook } from "../site/look";
import { rise, soft, spring } from "../site/motion";
import { rest, useSeen } from "../site/seen";
import { Head } from "./parts";
import "../styles/android.css";

const POINTS = [
  { icon: "app-window", title: "a screen of its own", text: "the app runs on a hidden second screen, signed in as you, and leaves yours free." },
  { icon: "cursor-click", title: "watch it work", text: "a live window in the chat, with the agent's pointer on every tap." },
  { icon: "hand-palm", title: "asks before it buys", text: "orders, payments and messages wait for your yes, every time." },
  { icon: "credit-card", title: "your card stays yours", text: "card and password screens go black to the agent and come back to you." },
];

/** Which point the story is on: the app opening, the pointer at work, the final step's card, the order going in. */
const pointOf = (step: number) => (step >= 12 ? 3 : step >= 8 ? 2 : step >= 4 ? 1 : 0);

const CHIPS = [
  { icon: "android-logo", text: "android 15" },
  { icon: "storefront", text: "aurora store" },
  { icon: "user-circle-minus", text: "no google account" },
  { icon: "bell", text: "notifications on the glance screen" },
  { icon: "mouse-scroll", text: "the wheel scrolls your apps" },
  { icon: "microphone", text: "hold to talk over any app" },
];

/** Plays its child again every `ms` while `on`, each round fading over the last; with reduced motion, once, at its end. */
function Replay({ on, ms, children }: { on: boolean; ms: number; children: ReactNode }) {
  const reduce = useReducedMotion();
  const [n, setN] = useState(0);
  useEffect(() => {
    if (!on || reduce) return;
    const id = setInterval(() => setN((x) => x + 1), ms);
    return () => clearInterval(id);
  }, [on, ms, reduce]);
  return (
    <div className="r1-slide">
      <AnimatePresence initial={false}>
        {(on || reduce) && (
          <motion.div key={n} className="r1-slide" initial={reduce ? false : { opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }} transition={{ duration: 0.3 }}>
            {children}
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

export function AppControl() {
  const box = useRef<HTMLElement>(null);
  const device = useRef<HTMLDivElement>(null);
  const seen = useSeen(box);
  const playing = useSeen(device, 0.3);
  const reduce = useReducedMotion();
  const { look } = useLook();
  const agent = colorOf(member("mochi").color);
  const [beat, setBeat] = useState(0);
  const [hover, setHover] = useState<number | null>(null);
  const lit = hover ?? beat;
  /** Rises in once, a little after the one before it; already there with reduced motion. */
  const enter = (i: number) =>
    reduce ? {} : ({ initial: { opacity: 0, y: 12 }, whileInView: { opacity: 1, y: 0 }, viewport: { once: true, margin: "-60px" }, transition: { ...soft, delay: i * 0.07 } } as const);
  return (
    <section className="section" id="android" ref={box} {...rest(seen)}>
      <div className="wrap">
        <Head title="agents that use your apps." sub="ask for dinner or a song, and watch your agent tap through the app for you." link={{ to: "/docs/android", label: "how they use your apps" }} />
        <div className="droid">
          <motion.div className="droid-device" ref={device} {...(reduce ? {} : rise)}>
            <R1 glow={agent}>
              <Replay on={playing} ms={8000}>
                <AppWindowScreen accent={look.accent} onStep={(s) => setBeat(pointOf(s))} />
              </Replay>
            </R1>
          </motion.div>
          <div className="droid-side">
            <ul className="droid-points" onMouseLeave={() => setHover(null)}>
              {POINTS.map((p, i) => {
                const on = lit === i;
                return (
                  <motion.li key={p.title} className={`droid-point${on ? " on" : ""}`} onMouseEnter={() => setHover(i)} {...enter(i)}>
                    {on && <motion.div layoutId="droid-point-fill" className="droid-point-fill" transition={reduce ? { duration: 0 } : spring} />}
                    <span className="droid-icon">
                      <Icon name={p.icon} weight="bold" size={22} />
                    </span>
                    <span className="droid-words">
                      <b>{p.title}</b>
                      <span>{p.text}</span>
                    </span>
                  </motion.li>
                );
              })}
            </ul>
          </div>
        </div>
        <motion.div className="droid-band" {...(reduce ? {} : rise)}>
          <h3>and a full android device.</h3>
          <p>it runs android 15. install apps from google play's catalog with aurora store, no google account needed. apps that need google's services may not work yet.</p>
          <div className="droid-chips">
            {CHIPS.map((c, i) => (
              <motion.span
                key={c.text}
                className="droid-chip"
                {...(reduce ? {} : { initial: { opacity: 0, y: 8 }, whileInView: { opacity: 1, y: 0 }, viewport: { once: true, margin: "-40px" }, transition: { ...soft, delay: 0.15 + i * 0.04 } })}
              >
                <Icon name={c.icon} weight="bold" size={17} />
                {c.text}
              </motion.span>
            ))}
          </div>
        </motion.div>
      </div>
    </section>
  );
}
