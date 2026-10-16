// The home page's last parts: private by default, and the close: yours to change. Few words each; the docs carry the
// detail (the privacy page says what isn't safe yet, so no claim here outruns it: T-177). Private is a plain list,
// each point a bold word and a line, so it doesn't read as one more grid of icons. The close's characters rest while
// it's scrolled away.

import { Icon } from "@r1os/icons";
import { motion, useReducedMotion } from "motion/react";
import { useRef } from "react";
import { Link } from "react-router";
import { Character } from "@shell/ui/Character";
import { after, popIn, reveal, soft } from "../site/motion";
import { REPO } from "../site/repo";
import { rest, useSeen } from "../site/seen";
import { Head } from "./parts";

/** What stays yours. Asking before it buys is the android section's, so it isn't said again here. */
const PRIVATE = [
  { title: "your login.", text: "your agents keep their own sign-in. r1os keeps none." },
  { title: "no cloud of ours.", text: "no account, no r1os server, no middleman." },
  { title: "agents in a box.", text: "on the device, each turn runs sandboxed, with no special powers." },
  { title: "heard on the device.", text: "speech turns to text right here." },
  { title: "a key every boot.", text: "a fresh key every boot, and a pin lock. usb isn't locked yet." },
];

const CLOSE = "yours to change.";

/** The close's characters, settling in one after another before its words. */
const CREW = [
  { form: "orb", color: "orange", finish: "soft", state: "idle" },
  { form: "cat", color: "pink", finish: "grain", eyes: "happy", state: "idle" },
  { form: "bot", color: "lime", finish: "matte", eyes: "dots", wear: ["headphones"], state: "working", activity: "build" },
  { form: "cloud", color: "purple", finish: "aurora", state: "thinking" },
] as const;

export function Private() {
  const section = useRef<HTMLElement>(null);
  const seen = useSeen(section, 0);
  const reduce = useReducedMotion();
  return (
    <section className="section" id="private" ref={section} {...rest(seen)}>
      <div className="wrap">
        <Head title="private by default." sub="just your device, your agents and the services you already pay for." link={{ to: "/docs/privacy", label: "what leaves the device" }} />
        <ul className="private">
          {PRIVATE.map((p, i) => (
            <motion.li
              key={p.title}
              {...(reduce ? {} : { initial: { opacity: 0, y: 10 }, whileInView: { opacity: 1, y: 0 }, viewport: { once: true, margin: "-40px" }, transition: { ...soft, delay: i * 0.04 } })}
            >
              <b>{p.title}</b> {p.text}
            </motion.li>
          ))}
        </ul>
      </div>
    </section>
  );
}

export function OpenSource() {
  const section = useRef<HTMLElement>(null);
  const seen = useSeen(section, 0);
  const reduce = useReducedMotion();
  // The characters settle in, then the words rise, then the line and the links.
  const words = CREW.length * 0.05 + 0.1;
  const line = words + 0.1;
  return (
    <section className="section" id="open" ref={section} {...rest(seen)}>
      <div className="wrap">
        <motion.div className="open-card" variants={after(0, 26)} {...(reduce ? {} : reveal)}>
          <div className="open-chars" aria-hidden>
            {CREW.map((ch, i) => (
              <motion.div key={ch.form} variants={popIn(0.08)} custom={i}>
                <Character {...ch} wear={"wear" in ch ? [...ch.wear] : undefined} size={76} />
              </motion.div>
            ))}
          </div>
          <motion.h2 variants={after(words, 12)}>{CLOSE}</motion.h2>
          <motion.p variants={after(line)}>
            open source under the mit licence, with an api and <Link to="/docs/skill">a skill</Link> that teaches coding agents how r1os works.
          </motion.p>
          <motion.div className="open-links" variants={after(line + 0.08, 10)}>
            <Link to="/docs/start" className="btn primary">
              <Icon name="rocket-launch" weight="bold" size={17} /> install r1os
            </Link>
            <a href={REPO} className="btn ghost">
              <Icon name="github-logo" weight="bold" size={17} /> the source
            </a>
            <Link to="/docs/extend" className="btn ghost">
              <Icon name="plugs-connected" weight="bold" size={17} /> bring your agent
            </Link>
          </motion.div>
        </motion.div>
      </div>
    </section>
  );
}
