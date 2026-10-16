// Everything you'd expect, and more: the device's other screens floating around what's built in as on OS3, the screens
// drifting at their own speeds as you scroll, then a band of what only r1os does, each linked to where the docs explain
// it (and say what it costs: root over USB is also on the privacy page's "not safe yet"). The floating screens are drawn
// only while they're on screen, and rest with the section; on a phone they're left out, too small to read.
// Compared with rabbitOS 2.3.1 and OS3 as of October 2026 (docs/research/2026-10-11-rabbitos-os3-gaps.md): nothing
// planned is shown, and what OS3 also has isn't marked as only on r1os. What's still to come is in the docs'
// introduction, not here (Max, 2026-10-13: the page sells r1os as OS3's abilities and more).

import { Icon } from "@r1os/icons";
import { motion, useReducedMotion, useScroll, useTransform, type MotionValue } from "motion/react";
import { type ReactNode, useRef } from "react";
import { Link } from "react-router";
import { COLORS } from "@shell/ui/Character";
import { Still } from "../device/ui";
import { CardScreen } from "../device/screens/Chats";
import { GlanceScreen } from "../device/screens/Glance";
import { CameraScreen, WhatsUpScreen } from "../device/screens/Launcher";
import { RecorderScreen, TranslatorScreen } from "../device/screens/Tools";
import { ACCENTS } from "../site/look";
import { after, reveal } from "../site/motion";
import { rest, useSeen } from "../site/seen";
import { Head } from "./parts";

const c = COLORS as Record<string, string>;
const a = ACCENTS as Record<string, string>;

const MINIS: { at: string; speed: number; rot: number; screen: ReactNode; label: string }[] = [
  { at: "m1", speed: 36, rot: -3, label: "recorder", screen: <RecorderScreen accent={a.red!} /> },
  { at: "m2", speed: 16, rot: 2, label: "translator", screen: <TranslatorScreen accent={a.teal!} /> },
  { at: "m3", speed: 46, rot: -1.5, label: "what's up", screen: <WhatsUpScreen accent={a.purple!} /> },
  { at: "m4", speed: 30, rot: 3, label: "glance", screen: <GlanceScreen accent={a.amber!} /> },
  { at: "m5", speed: 12, rot: -2, label: "cards", screen: <CardScreen accent={a.blue!} /> },
  { at: "m6", speed: 42, rot: 2, label: "camera", screen: <CameraScreen accent={a.orange!} /> },
];

interface Item {
  icon: string;
  label: string;
  color: string | undefined;
  to?: string;
}

/** The everyday tools, OS3's among them. */
const BUILT_IN: Item[] = [
  { icon: "sun-horizon", label: "glance + widgets", color: c.amber },
  { icon: "cassette-tape", label: "recorder", color: c.red },
  { icon: "notebook", label: "journal", color: c.yellow },
  { icon: "translate", label: "translator", color: c.teal },
  { icon: "camera", label: "camera", color: c.blue },
  { icon: "alarm", label: "alarms + timers", color: c.orange },
  { icon: "check-square", label: "reminders", color: c.green },
  { icon: "repeat", label: "routines", color: c.purple },
  { icon: "sparkle", label: "what's up", color: c.cyan },
  { icon: "music-notes", label: "music by voice", color: c.pink },
  { icon: "keyboard", label: "a keyboard", color: c.amber },
  { icon: "palette", label: "themes", color: c.indigo },
  { icon: "puzzle-piece", label: "skills + connectors", color: c.lime },
];

/** What OS3 doesn't do on the device, in three columns: the agents, Android, and the device itself. Android 15 against
 *  rabbitOS's 13 is the research's (docs/research/2026-10-08-r1-platform.md); there are no updates over the air yet, so
 *  it says newer, not updated. */
const ONLY: Item[] = [
  { icon: "terminal-window", label: "claude + codex on the device", color: c.orange, to: "/docs/agents#brains" },
  { icon: "wallet", label: "your subscription, no api bill", color: c.amber, to: "/docs" },
  { icon: "users-three", label: "agents that team up", color: c.indigo, to: "/docs/agents#agents-working-together" },
  { icon: "brain", label: "memory you can see", color: c.pink, to: "/docs/agents#memory" },
  { icon: "globe", label: "a browser they drive", color: c.teal, to: "/docs/agents#the-browser" },
  { icon: "cursor-click", label: "agents that use your android apps", color: c.lime, to: "/docs/android" },
  { icon: "android-logo", label: "android apps, no google account", color: c.green, to: "/docs/android" },
  { icon: "shield-check", label: "android 15, two versions past stock", color: c.blue, to: "/docs/architecture" },
  { icon: "magic-wand", label: "apps built in seconds", color: c.cyan, to: "/docs/apps" },
  { icon: "lightning", label: "instant actions, no cloud", color: c.yellow, to: "/docs/using#where-your-words-go" },
  { icon: "device-rotate", label: "dock mode, by touch", color: c.purple, to: "/docs/using#dock-mode" },
  { icon: "lock-key-open", label: "root access, over usb", color: c.red, to: "/docs/privacy#not-safe-yet" },
];

function Mini({ m, progress, on }: { m: (typeof MINIS)[number]; progress: MotionValue<number>; on: boolean }) {
  const y = useTransform(progress, [0, 1], [m.speed, -m.speed]);
  const reduce = useReducedMotion();
  return (
    <motion.div className={`mini ${m.at}`} style={{ y: reduce ? 0 : y, rotate: m.rot }} aria-hidden>
      <div className="mini-float">{on && <Still>{m.screen}</Still>}</div>
    </motion.div>
  );
}

function List({ label, items, only }: { label: string; items: Item[]; only?: boolean }) {
  return (
    <motion.div className={`kit ${only ? "only" : ""}`} variants={after(0.04, 10)}>
      <h3 className="kit-label">
        <Icon name={only ? "star-four" : "check-circle"} weight="fill" size={16} />
        {label}
      </h3>
      <ul>
        {items.map((t) => (
          <li key={t.label} style={{ ["--tile" as string]: t.color }}>
            <Icon name={t.icon} weight="fill" size={only ? 18 : 16} />
            {t.to ? <Link to={t.to}>{t.label}</Link> : t.label}
          </li>
        ))}
      </ul>
    </motion.div>
  );
}

export function Everything() {
  const section = useRef<HTMLElement>(null);
  const box = useRef<HTMLDivElement>(null);
  const seen = useSeen(section, 0);
  // The floating screens are drawn once the collage is in view, and their own loops (the recorder's reels) stop when
  // it's gone.
  const near = useSeen(box, 0);
  const { scrollYProgress } = useScroll({ target: box, offset: ["start end", "end start"] });
  const reduce = useReducedMotion();
  return (
    <section className="section" id="everything" ref={section} {...rest(seen)}>
      <div className="wrap">
        <Head layout="stage" title="everything you'd expect, and more." sub="the everyday tools of rabbit's os3 are built in, plus a few things only r1os does." link={{ to: "/docs/using", label: "what's built in" }} />
        <div className="collage" ref={box}>
          {MINIS.map((m) => (
            <Mini key={m.at} m={m} progress={scrollYProgress} on={near} />
          ))}
          <motion.div className="kits" {...(reduce ? {} : reveal)}>
            <List label="built in" items={BUILT_IN} />
          </motion.div>
        </div>
        <motion.div className="only-band" {...(reduce ? {} : reveal)}>
          <List label="only on r1os" items={ONLY} only />
        </motion.div>
        <motion.p className="groups-note" variants={after(0.1, 8)} {...(reduce ? {} : reveal)}>
          compared with rabbit's rabbitos 2.3.1 and os3, october 2026.
        </motion.p>
      </div>
    </section>
  );
}
