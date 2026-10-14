// Hold to talk (ADR-0009; host/…/shell/TalkOverlay.kt): the button goes down, the agent listens inside a ring that
// breathes with your voice, your words land live in big type, and letting go sends them. Here: a reminder, set.

import { Icon } from "@r1os/icons";
import { AnimatePresence, motion } from "motion/react";
import { useEffect } from "react";
import { Character } from "@shell/ui/Character";
import { member } from "../crew";
import { Screen, useDevice } from "../R1";
import { Card, INK, Mono, pop, T, useSteps } from "../ui";

const WORDS = "remind me at five to call mom".split(" ");

export function TalkScreen({ accent }: { accent: string }) {
  const kiwi = member("kiwi");
  // Each word, then letting go, then the agent's answer.
  const at = [...WORDS.map((_, i) => 380 + i * 230), 380 + WORDS.length * 230 + 260, 380 + WORDS.length * 230 + 1100];
  const step = useSteps(at);
  const heard = Math.min(step, WORDS.length);
  const sent = step > WORDS.length;
  const done = step > WORDS.length + 1;
  const { hold } = useDevice();
  useEffect(() => {
    hold(!sent);
    return () => hold(false);
  }, [sent, hold]);
  return (
    <Screen>
      <AnimatePresence>
        {!done && (
          <motion.div key="listen" className="r1-layer" exit={{ opacity: 0, transition: { duration: 0.2 } }}>
            <div style={{ position: "absolute", left: 180 - 62, top: 36, width: 124, height: 124, display: "grid", placeItems: "center" }}>
              <motion.div
                style={{ position: "absolute", inset: 0, borderRadius: "50%", border: `2px solid ${accent}` }}
                animate={sent ? { scale: 0.7, opacity: 0 } : { scale: [1, 1.08, 0.98, 1.12, 1], opacity: 1 }}
                transition={sent ? { duration: 0.25 } : { duration: 1.1, repeat: Infinity }}
              />
              <motion.div style={{ position: "absolute", inset: 10, borderRadius: "50%", background: accent, filter: "blur(22px)" }} animate={{ opacity: sent ? 0 : [0.18, 0.34, 0.18] }} transition={{ duration: 0.9, repeat: Infinity }} />
              <Character form={kiwi.form} color={kiwi.color} finish={kiwi.finish} state={sent ? "thinking" : "listening"} size={96} />
            </div>
            <motion.div
              animate={sent ? { y: -60, opacity: 0, scale: 0.92 } : { y: 0, opacity: 1, scale: 1 }}
              transition={{ type: "spring", stiffness: 260, damping: 26 }}
              style={{ position: "absolute", left: 22, right: 22, top: 196, textAlign: "center", fontSize: 30, fontWeight: 700, lineHeight: 1.18, letterSpacing: "-0.01em" }}
            >
              {WORDS.slice(0, heard).map((w, i) => (
                <motion.span key={i} initial={{ opacity: 0, y: 8, filter: "blur(4px)" }} animate={{ opacity: 1, y: 0, filter: "blur(0px)" }} transition={pop} style={{ display: "inline-block", marginRight: 8, color: i === heard - 1 && !sent ? T.fg : "#dfe1e5" }}>
                  {w}
                </motion.span>
              ))}
            </motion.div>
            <div style={{ position: "absolute", left: 0, right: 0, bottom: 26, display: "flex", justifyContent: "center", gap: 4, alignItems: "flex-end", height: 28 }}>
              {Array.from({ length: 21 }, (_, i) => (
                <motion.span
                  key={i}
                  style={{ width: 4, height: 28, borderRadius: 2, transformOrigin: "50% 100%", background: i % 5 === 2 ? accent : T.subtle }}
                  animate={{ scaleY: sent ? 4 / 28 : [6, 10 + ((i * 37) % 18), 5, 8 + ((i * 53) % 20), 6].map((h) => h / 28) }}
                  transition={{ duration: 0.8, repeat: sent ? 0 : Infinity, delay: (i % 7) * 0.05 }}
                />
              ))}
            </div>
            <div style={{ position: "absolute", left: 0, right: 0, bottom: 64, textAlign: "center", fontSize: 13, fontWeight: 600, color: T.muted }}>
              {sent ? "sending…" : "listening · let go to send"}
            </div>
          </motion.div>
        )}
      </AnimatePresence>
      {done && (
        <motion.div className="r1-layer" initial={{ opacity: 0 }} animate={{ opacity: 1 }}>
          <div style={{ position: "absolute", left: 16, top: 12, display: "flex", alignItems: "center", gap: 10 }}>
            <Character form={kiwi.form} color={kiwi.color} finish={kiwi.finish} state="speaking" size={34} />
            <div style={{ fontSize: 24, fontWeight: 700 }}>kiwi</div>
          </div>
          <div style={{ position: "absolute", left: 12, right: 12, bottom: 16, display: "flex", flexDirection: "column", gap: 10 }}>
            <motion.div initial={{ opacity: 0, y: 12 }} animate={{ opacity: 1, y: 0 }} transition={pop} style={{ alignSelf: "flex-end", maxWidth: "86%", padding: "10px 14px", borderRadius: "18px 18px 6px 18px", background: accent, color: INK, fontSize: 17, fontWeight: 600 }}>
              remind me at five to call mom
            </motion.div>
            <Card accent={accent} title="reminder" style={{ display: "flex", flexDirection: "column", gap: 4 }}>
              <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
                <div style={{ width: 44, height: 44, borderRadius: 12, display: "grid", placeItems: "center", background: `${accent}2e`, color: accent }}>
                  <Icon name="bell-ringing" weight="bold" size={24} />
                </div>
                <div>
                  <div style={{ fontSize: 22, fontWeight: 700, color: T.fg }}>call mom</div>
                  <Mono style={{ fontSize: 13, color: T.muted }}>today · 5:00 pm</Mono>
                </div>
              </div>
            </Card>
            <motion.div initial={{ opacity: 0, y: 12 }} animate={{ opacity: 1, y: 0 }} transition={{ ...pop, delay: 0.25 }} style={{ alignSelf: "flex-start", maxWidth: "86%", padding: "10px 14px", borderRadius: "18px 18px 18px 6px", background: T.card, fontSize: 17, fontWeight: 600, textTransform: "none" }}>
              Done. I'll wake you at five.
            </motion.div>
          </div>
        </motion.div>
      )}
    </Screen>
  );
}
