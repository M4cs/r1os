// Everything you'd expect, and more: the device on home with its other screens floating around it, drifting at their
// own speeds as you scroll, then what's built in and what only r1os does, as two groups of tiles. The tiles arrive a
// column at a time; a soft accent focus springs from tile to tile under the pointer, as the device's focus moves
// between rows. The floating screens are drawn only while the collage is on screen, and rest with the section.
// Compared with rabbitOS 2.3.1 and OS3 as of October 2026 (docs/research/2026-10-11-rabbitos-os3-gaps.md): nothing
// planned is shown, and what OS3 also has isn't marked as only on r1os. What's still to come is in the docs'
// introduction, not here (Max, 2026-10-13: the page sells r1os as OS3's abilities and more).

import { Icon } from "@r1os/icons";
import { LayoutGroup, motion, useReducedMotion, useScroll, useTransform, type MotionValue, type Variants } from "motion/react";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { COLORS } from "@shell/ui/Character";
import { R1 } from "../device/R1";
import { Still } from "../device/ui";
import { CardScreen } from "../device/screens/Chats";
import { GlanceScreen } from "../device/screens/Glance";
import { HomeScreen } from "../device/screens/Home";
import { CameraScreen, WhatsUpScreen } from "../device/screens/Launcher";
import { RecorderScreen, TranslatorScreen } from "../device/screens/Tools";
import { ACCENTS } from "../site/look";
import { after, popIn, reveal, soft, spring } from "../site/motion";
import { rest, useSeen } from "../site/seen";
import { Head } from "./parts";

const c = COLORS as Record<string, string>;
const a = ACCENTS as Record<string, string>;

const MINIS: { at: string; speed: number; rot: number; screen: ReactNode; label: string }[] = [
  { at: "m1", speed: 70, rot: -7, label: "recorder", screen: <RecorderScreen accent={a.red!} /> },
  { at: "m2", speed: 30, rot: 5, label: "translator", screen: <TranslatorScreen accent={a.teal!} /> },
  { at: "m3", speed: 95, rot: -3, label: "what's up", screen: <WhatsUpScreen accent={a.purple!} /> },
  { at: "m4", speed: 60, rot: 6, label: "glance", screen: <GlanceScreen accent={a.amber!} /> },
  { at: "m5", speed: 25, rot: -5, label: "cards", screen: <CardScreen accent={a.blue!} /> },
  { at: "m6", speed: 85, rot: 4, label: "camera", screen: <CameraScreen accent={a.orange!} /> },
];

interface Tile {
  icon: string;
  label: string;
  color: string | undefined;
}

/** The everyday tools, OS3's among them. */
const BUILT_IN: Tile[] = [
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

/** What OS3 doesn't do on the device. */
const ONLY: Tile[] = [
  { icon: "terminal-window", label: "claude + codex on the device", color: c.orange },
  { icon: "app-window", label: "agents in your apps", color: c.lime },
  { icon: "magic-wand", label: "apps in seconds", color: c.cyan },
  { icon: "globe", label: "a browser they drive", color: c.teal },
  { icon: "users-three", label: "agents that team up", color: c.indigo },
  { icon: "brain", label: "memory you can see", color: c.pink },
  { icon: "android-logo", label: "android apps", color: c.green },
  { icon: "device-rotate", label: "dock mode, by touch", color: c.purple },
];

/** A tile, arriving with its column (`custom`; the columns are set in sections.css, `cols` is the widest count). */
const tileIn: Variants = {
  hidden: { opacity: 0, y: 18, scale: 0.94 },
  show: (col: number) => ({ opacity: 1, y: 0, scale: 1, transition: { ...soft, delay: 0.08 + col * 0.045 } }),
};

function Mini({ m, progress, on }: { m: (typeof MINIS)[number]; progress: MotionValue<number>; on: boolean }) {
  const y = useTransform(progress, [0, 1], [m.speed, -m.speed]);
  const reduce = useReducedMotion();
  return (
    <motion.div className={`mini ${m.at}`} style={{ y: reduce ? 0 : y, rotate: m.rot }} aria-label={m.label}>
      <div className="mini-float">{on && <Still>{m.screen}</Still>}</div>
    </motion.div>
  );
}

function Group({ label, tiles, cols, only, hover, setHover }: { label: string; tiles: Tile[]; cols: number; only?: boolean; hover: string | null; setHover: (l: string) => void }) {
  const reduce = useReducedMotion();
  const badge = popIn(0.12 + Math.min(tiles.length, cols) * 0.045 + 0.25);
  return (
    <motion.div className={`group ${only ? "only" : ""}`} {...(reduce ? {} : reveal)}>
      <motion.h3 className="group-label" variants={after(0, 10)}>
        <Icon name={only ? "star-four" : "check-circle"} weight="fill" size={16} />
        {label}
      </motion.h3>
      <ul className="tiles">
        {tiles.map((t, i) => (
          <motion.li key={t.label} className="tile" style={{ ["--tile" as string]: t.color }} variants={tileIn} custom={i % cols} onMouseEnter={() => setHover(t.label)}>
            {hover === t.label && <motion.span className="tile-focus" layoutId="tile-focus" transition={spring} aria-hidden />}
            <span className="tile-icon">
              <Icon name={t.icon} weight="fill" size={only ? 28 : 24} />
              {only && (
                <motion.span className="tile-badge" variants={badge} custom={i} aria-hidden>
                  <Icon name="star-four" weight="fill" size={11} />
                </motion.span>
              )}
            </span>
            <span className="tile-label">{t.label}</span>
          </motion.li>
        ))}
      </ul>
    </motion.div>
  );
}

export function Everything() {
  const section = useRef<HTMLElement>(null);
  const box = useRef<HTMLDivElement>(null);
  const seen = useSeen(section, 0);
  // The floating screens and the device's story are drawn once the collage comes into view: the story plays when
  // you get there, and the screens' own loops (the recorder's reels) stop when it's gone.
  const near = useSeen(box, 0);
  const [met, setMet] = useState(false);
  useEffect(() => {
    if (near) setMet(true);
  }, [near]);
  const { scrollYProgress } = useScroll({ target: box, offset: ["start end", "end start"] });
  const [hover, setHover] = useState<string | null>(null);
  const reduce = useReducedMotion();
  return (
    <section className="section" id="everything" ref={section} {...rest(seen)}>
      <div className="wrap">
        <Head title="everything you'd expect, and more." sub="os3's everyday tools are built in, plus the things only r1os does." />
        <div className="collage" ref={box}>
          {MINIS.map((m) => (
            <Mini key={m.at} m={m} progress={scrollYProgress} on={near} />
          ))}
          <div className="collage-device">
            <R1 glow={a.orange}>
              <div className="r1-slide">{met && <HomeScreen accent={a.orange!} at={[1400, 2800, 4200]} />}</div>
            </R1>
          </div>
        </div>
        <LayoutGroup id="tiles">
          <div className="groups" onMouseLeave={() => setHover(null)}>
            <Group label="built in" tiles={BUILT_IN} cols={7} hover={hover} setHover={setHover} />
            <Group label="only on r1os" tiles={ONLY} cols={4} only hover={hover} setHover={setHover} />
          </div>
        </LayoutGroup>
        <motion.p className="groups-note" variants={after(0.1, 8)} {...(reduce ? {} : reveal)}>
          compared with rabbitos 2.3.1 and os3, october 2026.
        </motion.p>
      </div>
    </section>
  );
}
