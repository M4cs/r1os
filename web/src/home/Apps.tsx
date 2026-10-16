// Need an app? Just ask (docs/specs/apps.md): the device replaying a build, from the words to the app in use, and
// three short points beside it. How apps are made is in the docs. The replay runs only while it's seen, each round
// fading over the last; with reduced motion it plays once, at its end.

import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { useEffect, useRef, useState } from "react";
import { R1 } from "../device/R1";
import { BuildScreen } from "../device/screens/Build";
import { ACCENTS } from "../site/look";
import { rise } from "../site/motion";
import { rest, useSeen } from "../site/seen";
import { Head } from "./parts";

const STATS = [
  { big: "in seconds", small: "from asking to using it" },
  { big: "no forms", small: "just say what you want" },
  { big: "by voice", small: "change it, fix it or undo it" },
];

/** Plays its child again every `ms` while it's on screen in a visible tab; with reduced motion, once, at its end. */
function Replay({ ms, children }: { ms: number; children: React.ReactNode }) {
  const box = useRef<HTMLDivElement>(null);
  const seen = useSeen(box, 0.3);
  const reduce = useReducedMotion();
  const [n, setN] = useState(0);
  useEffect(() => {
    if (!seen || reduce) return;
    const id = setInterval(() => setN((x) => x + 1), ms);
    return () => clearInterval(id);
  }, [seen, reduce, ms]);
  return (
    <div ref={box} className="r1-slide">
      <AnimatePresence initial={false}>
        {(seen || reduce) && (
          <motion.div key={n} className="r1-slide" initial={reduce ? false : { opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }} transition={{ duration: 0.3 }}>
            {children}
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

export function AskForApps() {
  const accent = ACCENTS.cyan!;
  const reduce = useReducedMotion();
  const section = useRef<HTMLElement>(null);
  const seen = useSeen(section, 0);
  return (
    <section className="section own-accent" id="apps" ref={section} style={{ ["--accent" as string]: accent }} {...rest(seen)}>
      <div className="wrap">
        <Head layout="split" title="need an app? just ask." sub="your agent builds it on the device in seconds, and your data stays there." link={{ to: "/docs/apps", label: "how apps work" }} />
        <motion.div className="apps" {...(reduce ? {} : rise)}>
          <div className="apps-device">
            <R1 glow={accent}>
              <Replay ms={6400}>
                <BuildScreen accent={accent} />
              </Replay>
            </R1>
          </div>
          <div className="stats">
            {STATS.map((s) => (
              <div key={s.small} className="stat">
                <b>{s.big}</b>
                <span>{s.small}</span>
              </div>
            ))}
          </div>
        </motion.div>
      </div>
    </section>
  );
}
