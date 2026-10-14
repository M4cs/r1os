// Need an app? Ask (docs/specs/apps.md; ADR-0025): the build card runs in the chat with its timer, the app is checked
// and installed, and it opens in r1os's look: big bold lowercase type, the accent filling what the wheel's on.

import { Icon } from "@r1os/icons";
import { AnimatePresence, motion } from "motion/react";
import { useEffect, useId } from "react";
import { member } from "../crew";
import { Screen, useDevice } from "../R1";
import { Bubble, ChatHead, fill, INK, Mono, pop, Row, T, Thread, useSteps, useTicker } from "../ui";

export function BuildScreen({ accent }: { accent: string }) {
  // Its own fills' names: it can be drawn twice on the page (the hero and the apps), and the two mustn't share them.
  const uid = useId();
  const mochi = member("mochi");
  // asked · building · ready · open · +1 · +1
  const step = useSteps([250, 900, 2500, 3150, 3750, 4350]);
  const building = step >= 2 && step < 3;
  const secs = useTicker(building, 170, 1);
  const open = step >= 4;
  const glasses = 3 + Math.max(0, step - 4);
  const { roll } = useDevice();
  useEffect(() => {
    if (step === 3) roll(1);
  }, [step, roll]);
  return (
    <Screen>
      <AnimatePresence initial={false}>
        {!open ? (
          <motion.div key="chat" className="r1-layer" exit={{ opacity: 0, scale: 0.96, transition: { duration: 0.2 } }}>
            <ChatHead name="mochi" form={mochi.form} color={mochi.color} finish={mochi.finish} />
            <Thread>
              {step >= 1 && (
                <Bubble mine color={accent}>
                  make me a water tracker, eight glasses a day
                </Bubble>
              )}
              {step >= 2 && (
                <motion.div initial={{ opacity: 0, y: 14 }} animate={{ opacity: 1, y: 0 }} transition={pop} style={{ position: "relative", display: "flex", alignItems: "center", gap: 12, padding: "12px 14px", borderRadius: 18, background: T.card, overflow: "hidden" }}>
                  {step >= 3 && <motion.div layoutId={`build-fill-${uid}`} transition={fill} style={{ position: "absolute", inset: 0, background: accent }} />}
                  <div style={{ position: "relative", width: 40, height: 40, borderRadius: 11, display: "grid", placeItems: "center", background: step >= 3 ? "rgba(0,0,0,.12)" : `${accent}2e`, color: step >= 3 ? INK : accent }}>
                    <motion.span animate={building ? { rotate: [0, -24, 8, 0] } : { rotate: 0 }} transition={{ duration: 0.5, repeat: building ? Infinity : 0 }} style={{ display: "grid" }}>
                      <Icon name={step >= 3 ? "check-circle" : "hammer"} weight="bold" size={22} />
                    </motion.span>
                  </div>
                  <div style={{ position: "relative", flex: 1 }}>
                    <div style={{ fontSize: 18, fontWeight: 700, color: step >= 3 ? INK : T.fg }}>{step >= 3 ? "water is ready" : "building water…"}</div>
                    <div style={{ fontSize: 13, fontWeight: 600, color: step >= 3 ? "rgba(0,0,0,.6)" : T.muted }}>{step >= 3 ? "click to open" : "writing, compiling, checking"}</div>
                  </div>
                  <Mono style={{ position: "relative", fontSize: 15, color: step >= 3 ? INK : T.fg }}>0:{String(Math.min(secs, 10)).padStart(2, "0")}</Mono>
                </motion.div>
              )}
            </Thread>
          </motion.div>
        ) : (
          <motion.div key="app" className="r1-layer" initial={{ opacity: 0, scale: 1.04 }} animate={{ opacity: 1, scale: 1 }} transition={{ type: "spring", stiffness: 300, damping: 30 }}>
            <div style={{ position: "absolute", left: 16, top: 12, display: "flex", alignItems: "center", gap: 10, fontSize: 24, fontWeight: 700 }}>
              <Icon name="drop" weight="fill" size={24} style={{ color: accent }} /> water
            </div>
            <div style={{ position: "absolute", left: 18, right: 18, top: 62 }}>
              <div style={{ fontSize: 13, fontWeight: 700, color: T.muted }}>today</div>
              <div style={{ display: "flex", alignItems: "baseline", gap: 10 }}>
                <AnimatePresence mode="popLayout" initial={false}>
                  <motion.div key={glasses} initial={{ y: 24, opacity: 0, scale: 0.8 }} animate={{ y: 0, opacity: 1, scale: 1 }} exit={{ y: -24, opacity: 0 }} transition={{ type: "spring", stiffness: 420, damping: 20 }} style={{ fontSize: 88, fontWeight: 700, lineHeight: 1, letterSpacing: "-0.03em" }}>
                    {glasses}
                  </motion.div>
                </AnimatePresence>
                <div style={{ fontSize: 22, fontWeight: 700, color: T.muted }}>of 8 glasses</div>
              </div>
              <div style={{ display: "flex", gap: 5, marginTop: 14 }}>
                {Array.from({ length: 8 }, (_, i) => (
                  <motion.div key={`${i}-${i < glasses}`} initial={{ scaleY: i === glasses - 1 ? 1.7 : 1 }} animate={{ scaleY: 1 }} transition={{ type: "spring", stiffness: 500, damping: 14 }} style={{ flex: 1, height: 12, borderRadius: 6, background: i < glasses ? accent : T.subtle }} />
                ))}
              </div>
            </div>
            <div style={{ position: "absolute", left: 10, right: 10, top: 238, display: "flex", flexDirection: "column", gap: 4 }}>
              <Row icon="plus" label="+1 glass" on accent={accent} layoutId={`water-fill-${uid}`} />
              <Row icon="arrow-counter-clockwise" label="undo" accent={accent} />
              <Row icon="chart-bar" label="this week" sub="34 glasses" accent={accent} />
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </Screen>
  );
}
