// Your PC, too: the desktop companion (docs/specs/companion.md; companion/src/App.tsx), its window drawn small, as it
// is: the title bar with manage and chats, your devices down the side with a device's pages, and the one you picked.
// The window takes the device's look, as the real one does. A chat's lines arrive one after another as you open
// chats, and its characters rest while it's scrolled away.

import { Icon } from "@r1os/icons";
import { AnimatePresence, LayoutGroup, motion, useReducedMotion } from "motion/react";
import { useRef, useState } from "react";
import { Character } from "@shell/ui/Character";
import { CREW } from "../device/crew";
import { pop } from "../device/ui";
import { rise, spring } from "../site/motion";
import { rest, useSeen } from "../site/seen";
import { Head } from "./parts";

const PAGES = [
  { id: "overview", icon: "house" },
  { id: "agents", icon: "smiley" },
  { id: "apps", icon: "squares-four" },
  { id: "look", icon: "palette" },
  { id: "alarms", icon: "alarm" },
  { id: "files", icon: "images" },
  { id: "connections", icon: "plugs-connected" },
];

const CHIPS = ["pair with a qr code", "end-to-end encrypted", "agents in your folders"];

/** A chat line arriving in its turn (`i`); already there with reduced motion. */
const line = (i: number, still: boolean | null) => ({ initial: still ? false : { opacity: 0, y: 12 }, animate: { opacity: 1, y: 0 }, transition: { ...pop, delay: 0.12 + i * 0.32 } }) as const;

export function CompanionSection() {
  const [mode, setMode] = useState<"manage" | "chats">("manage");
  const [page, setPage] = useState("overview");
  const agents = CREW.slice(0, 4);
  const section = useRef<HTMLElement>(null);
  const seen = useSeen(section, 0);
  const reduce = useReducedMotion();
  return (
    <section className="section" id="companion" ref={section} {...rest(seen)}>
      <div className="wrap">
        <Head title="your pc, too." sub="pair once, then manage r1os and chat from anywhere, and let agents work in your folders." />
        <motion.div className="cwin" {...(reduce ? {} : rise)}>
          <div className="cwin-bar">
            <span className="brand">
              <span className="brand-dot" />
              r1os
            </span>
            <LayoutGroup id="cwin-mode">
              <div className="cwin-tabs">
                {(["manage", "chats"] as const).map((m) => (
                  <button key={m} className={`cwin-tab ${mode === m ? "on" : ""}`} onClick={() => setMode(m)}>
                    {mode === m && <motion.span className="cwin-tab-fill" layoutId="cwin-tab" transition={spring} />}
                    <Icon name={m === "manage" ? "sliders-horizontal" : "chats-circle"} weight={mode === m ? "fill" : "bold"} size={14} />
                    <span>{m}</span>
                    {m === "chats" && <span className="cwin-badge">2</span>}
                  </button>
                ))}
              </div>
            </LayoutGroup>
            <span className="grow" />
            <span className="cwin-win">
              <Icon name="minus" size={14} />
              <Icon name="square" size={12} />
              <Icon name="x" size={14} />
            </span>
          </div>
          <AnimatePresence mode="wait" initial={false}>
            {mode === "manage" ? (
              <motion.div key="manage" className="cwin-body" initial={{ opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0, y: -6 }} transition={spring}>
                <aside className="cwin-side">
                  <div className="cwin-label">your devices</div>
                  <div className="cwin-device on">
                    <Character form="orb" color="orange" finish="soft" state="idle" size={34} />
                    <span>
                      <b>pocket</b>
                      <span className="cwin-sub">
                        <i className="cwin-dot" /> awake · direct
                      </span>
                    </span>
                  </div>
                  <LayoutGroup id="cwin-pages">
                    <nav className="cwin-pages">
                      {PAGES.map((p) => (
                        <button key={p.id} className={`cwin-page ${page === p.id ? "on" : ""}`} onClick={() => setPage(p.id)}>
                          {page === p.id && <motion.span className="cwin-page-fill" layoutId="cwin-page" transition={spring} />}
                          <Icon name={p.icon} weight={page === p.id ? "fill" : "bold"} size={15} />
                          <span>{p.id}</span>
                        </button>
                      ))}
                    </nav>
                  </LayoutGroup>
                </aside>
                <div className="cwin-main">
                  <div className="cwin-chips">
                    <span className="chip">
                      <span className="dot" /> pocket · direct
                    </span>
                    <span className="chip">
                      <Icon name="battery-high" size={14} /> 88%
                    </span>
                  </div>
                  <AnimatePresence mode="popLayout" initial={false}>
                    <motion.h4 key={page} className="cwin-h" initial={{ opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0, y: -6 }} transition={spring}>
                      {page === "overview" ? "pocket" : page}
                    </motion.h4>
                  </AnimatePresence>
                  <div className="cwin-agents">
                    {agents.map((a, i) => (
                      <motion.div key={a.name} className="cwin-agent" initial={{ opacity: 0, y: 10 }} animate={{ opacity: 1, y: 0 }} transition={{ ...spring, delay: i * 0.04 }}>
                        <Character form={a.form} color={a.color} finish={a.finish} eyes={a.eyes} wear={a.wear} state={i === 2 ? "thinking" : "idle"} size={64} />
                        <b>{a.name}</b>
                        <span>
                          {i === 2 ? "thinking" : "here"} · {a.brain}
                        </span>
                      </motion.div>
                    ))}
                  </div>
                  <div className="cwin-facts">
                    {[
                      { icon: "battery-high", label: "battery", value: "88%", bar: 88 },
                      { icon: "memory", label: "memory free", value: "2.4 gb", bar: 60 },
                      { icon: "link", label: "connection", value: "direct" },
                      { icon: "squares-four", label: "apps", value: "12" },
                    ].map((f) => (
                      <div key={f.label} className="cwin-fact">
                        <span className="cwin-sub">
                          <Icon name={f.icon} size={14} /> {f.label}
                        </span>
                        <b>{f.value}</b>
                        {f.bar && (
                          <span className="cwin-bar-track">
                            <motion.span initial={reduce ? false : { width: 0 }} whileInView={{ width: `${f.bar}%` }} viewport={{ once: true }} transition={{ type: "spring", stiffness: 120, damping: 20 }} />
                          </span>
                        )}
                      </div>
                    ))}
                  </div>
                </div>
              </motion.div>
            ) : (
              <motion.div key="chats" className="cwin-body chats" initial={{ opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0, y: -6 }} transition={spring}>
                <aside className="cwin-side">
                  {CREW.slice(0, 5).map((a, i) => (
                    <div key={a.name} className={`cwin-chat-agent ${i === 4 ? "on" : ""}`}>
                      <Character form={a.form} color={a.color} finish={a.finish} eyes={a.eyes} wear={a.wear} state="idle" size={30} />
                      <span>
                        <b>{a.name}</b>
                        <span className="cwin-sub">{a.role}</span>
                      </span>
                    </div>
                  ))}
                </aside>
                <div className="cwin-main cwin-chat">
                  <motion.div className="cwin-bubble mine" {...line(0, reduce)}>
                    tidy the photos folder on my desktop by month
                  </motion.div>
                  <motion.div className="cwin-tool mono" {...line(1, reduce)}>
                    <Icon name="folder-open" size={14} /> reading Desktop/photos · 412 files
                  </motion.div>
                  <motion.div className="cwin-tool mono" {...line(2, reduce)}>
                    <Icon name="terminal-window" size={14} /> moving into 2026-01 … 2026-10
                  </motion.div>
                  <motion.div className="cwin-bubble" {...line(3, reduce)}>
                    Done: 412 photos in 10 month folders. Nothing was deleted, and the 3 without a date are in “unsorted”.
                  </motion.div>
                  <div className="cwin-composer">
                    <span className="muted">message bolt…</span>
                    <span className="cwin-send">
                      <Icon name="arrow-up" weight="bold" size={14} />
                    </span>
                  </div>
                </div>
              </motion.div>
            )}
          </AnimatePresence>
        </motion.div>
        <div className="cwin-chips-under">
          {CHIPS.map((t) => (
            <span key={t} className="chip">
              {t}
            </span>
          ))}
        </div>
      </div>
    </section>
  );
}
