// The home page's last parts: private by default, and the close: yours to change. Few words each; the docs carry the
// detail. The close's characters rest while it's scrolled away.

import { Icon } from "@r1os/icons";
import { motion, useReducedMotion } from "motion/react";
import { useRef } from "react";
import { Link } from "react-router";
import { Character } from "@shell/ui/Character";
import { after, popIn, reveal, soft, STAGGER } from "../site/motion";
import { rest, useSeen } from "../site/seen";
import { Head, Words } from "./parts";

const PRIVATE = [
  { icon: "key", title: "your login", text: "your agents keep their own sign-in. r1os keeps none." },
  { icon: "cloud-slash", title: "no cloud of ours", text: "no account, no r1os server, no middleman." },
  { icon: "cube", title: "agents in a box", text: "on the device, each turn runs sandboxed, with no special powers." },
  { icon: "waveform", title: "heard on the device", text: "speech turns to text right here." },
  { icon: "hand-palm", title: "asks first", text: "orders, emails and settings wait for your yes." },
  { icon: "lock-key", title: "locked down", text: "a new key every boot, and a pin lock." },
];

const OPEN = ["claude + codex", "skills and connectors", "a full api", "a skill for coding agents", "android apps, no google account"];

const CLOSE = "yours to change.";

/** The close's characters, popping in one after another before its words. */
const CREW = [
  { form: "orb", color: "orange", finish: "soft", state: "idle" },
  { form: "cat", color: "pink", finish: "grain", eyes: "happy", state: "idle" },
  { form: "bot", color: "lime", finish: "matte", eyes: "dots", wear: ["headphones"], state: "working", activity: "build" },
  { form: "cloud", color: "purple", finish: "aurora", state: "thinking" },
] as const;

const LINKS = [
  { to: "/docs/start", icon: "rocket-launch", title: "get started" },
  { to: "/docs/extend", icon: "plugs-connected", title: "bring your agent" },
  { to: "/docs/skill", icon: "graduation-cap", title: "the r1os skill" },
];

export function Private() {
  const section = useRef<HTMLElement>(null);
  const seen = useSeen(section, 0);
  const reduce = useReducedMotion();
  return (
    <section className="section" id="private" ref={section} {...rest(seen)}>
      <div className="wrap">
        <Head title="private by default." sub="just your device, your agents and the services you already pay for." />
        <div className="grid3">
          {PRIVATE.map((p, i) => (
            <motion.div
              key={p.title}
              className="cell"
              {...(reduce ? {} : { initial: { opacity: 0, y: 18 }, whileInView: { opacity: 1, y: 0 }, viewport: { once: true, margin: "-40px" }, transition: { ...soft, delay: (i % 3) * 0.05 } })}
            >
              <Icon name={p.icon} weight="bold" size={22} />
              <b>{p.title}</b>
              <span>{p.text}</span>
            </motion.div>
          ))}
        </div>
      </div>
    </section>
  );
}

export function OpenSource() {
  const section = useRef<HTMLElement>(null);
  const seen = useSeen(section, 0);
  const reduce = useReducedMotion();
  // The characters pop in, then the words rise, then the line, the chips and the links.
  const words = CREW.length * 0.05 + 0.1;
  const line = words + CLOSE.split(" ").length * STAGGER + 0.1;
  return (
    <section className="section" id="open" ref={section} {...rest(seen)}>
      <div className="wrap">
        <motion.div className="open" variants={after(0, 26)} {...(reduce ? {} : reveal)}>
          <div className="open-chars" aria-hidden>
            {CREW.map((ch, i) => (
              <motion.div key={ch.form} variants={popIn(0.08)} custom={i}>
                <Character {...ch} wear={"wear" in ch ? [...ch.wear] : undefined} size={76} />
              </motion.div>
            ))}
          </div>
          <h2>
            <Words text={CLOSE} from={Math.round(words / STAGGER)} />
          </h2>
          <motion.p variants={after(line)}>open source under the mit licence, with an api and a skill that teaches coding agents how r1os works.</motion.p>
          <motion.div className="open-chips" variants={after(line + 0.08, 10)}>
            {OPEN.map((o) => (
              <span key={o} className="chip">
                {o}
              </span>
            ))}
          </motion.div>
          <motion.div className="open-links" variants={after(line + 0.16, 10)}>
            {LINKS.map((l, i) => (
              <Link key={l.to} to={l.to} className={`btn ${i === 0 ? "primary" : "ghost"}`}>
                <Icon name={l.icon} weight="bold" size={17} /> {l.title}
              </Link>
            ))}
          </motion.div>
        </motion.div>
      </div>
    </section>
  );
}
