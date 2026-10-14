// Home: the wheel of agents (host/…/shell/HomeScreen.kt; companion/src/r1screen/R1Screen.tsx's Home). The time at the
// top left, the agent on screen with its name, its neighbours peeking half size above and below, the side dots. The
// wheel turns a tick at a time and each agent springs into place.

import { AnimatePresence, animate, motion, useMotionValue, useReducedMotion, useTransform, type MotionValue } from "motion/react";
import { useEffect, useState } from "react";
import { Icon } from "@r1os/icons";
import { Character, colorOf } from "@shell/ui/Character";
import { hm } from "@companion/r1screen/Clock";
import { PALETTES } from "@companion/theme";
import { Backdrop } from "../Backdrop";
import { CREW, type Member } from "../crew";
import { Screen, useDevice } from "../R1";
import { T, useSteps } from "../ui";

const ON_HOME = ["pip", "kiwi", "nova", "mochi", "bolt"].map((n) => CREW.find((m) => m.name === n)!);
const GAP = 244;

export function HomeScreen({ accent, at = [650, 1450, 2250] }: { accent: string; at?: number[] }) {
  const step = useSteps(at);
  const pos = useMotionValue(0);
  const { roll } = useDevice();
  const [now] = useState(() => new Date());
  const reduce = useReducedMotion();
  useEffect(() => {
    if (step === 0) return;
    roll(1);
    if (reduce) pos.jump(step);
    else animate(pos, step, { type: "spring", stiffness: 300, damping: 24 });
  }, [step]); // eslint-disable-line react-hooks/exhaustive-deps
  const focus = ON_HOME[Math.min(step, ON_HOME.length - 1)]!;
  return (
    <Screen>
      <Backdrop bg={{ kind: "mesh", colors: PALETTES.ink, dim: 0.72, grain: 0.6 }} accent={accent} />
      <div style={{ position: "absolute", left: 20, top: 14 }}>
        <div style={{ fontSize: 34, fontWeight: 700, letterSpacing: "-0.01em", fontVariantNumeric: "tabular-nums" }}>{hm(now)}</div>
        <div style={{ display: "flex", alignItems: "center", gap: 3, marginTop: 4, color: T.muted, fontSize: 13, fontWeight: 600 }}>
          94% <Icon name="lightning" weight="bold" size={13} />
          <Icon name="wifi-high" weight="bold" size={15} style={{ marginLeft: 8 }} />
        </div>
      </div>
      {ON_HOME.map((m, i) => (
        <Agent key={m.name} m={m} i={i} pos={pos} />
      ))}
      <AnimatePresence mode="popLayout" initial={false}>
        <motion.div
          key={focus.name}
          initial={{ opacity: 0, y: 14 }}
          animate={{ opacity: 1, y: 0 }}
          exit={{ opacity: 0, y: -14 }}
          transition={{ type: "spring", stiffness: 400, damping: 30 }}
          style={{ position: "absolute", left: 0, right: 0, top: 100, textAlign: "center", fontSize: 30, fontWeight: 700, letterSpacing: "-0.01em" }}
        >
          {focus.name}
        </motion.div>
      </AnimatePresence>
      <SideDots focus={Math.min(step, ON_HOME.length - 1)} />
    </Screen>
  );
}

function Agent({ m, i, pos }: { m: Member; i: number; pos: MotionValue<number> }) {
  const y = useTransform(pos, (p) => 248 + (i - p) * GAP - 106);
  const scale = useTransform(pos, (p) => 1 - 0.5 * Math.min(Math.abs(i - p), 1));
  const opacity = useTransform(pos, (p) => 1 - 0.58 * Math.min(Math.abs(i - p), 1));
  return (
    <motion.div style={{ position: "absolute", left: 74, top: 0, width: 212, height: 212, y, scale, opacity }}>
      <Character form={m.form} color={m.color} finish={m.finish} eyes={m.eyes} wear={m.wear} state="idle" size={212} />
    </motion.div>
  );
}

function SideDots({ focus }: { focus: number }) {
  const n = ON_HOME.length + 1;
  return (
    <div style={{ position: "absolute", left: 343, top: 248 - (n * 13 + 12) / 2, width: 6, display: "flex", flexDirection: "column", gap: 7 }}>
      {Array.from({ length: n }, (_, i) => {
        const m = ON_HOME[i];
        const on = i === focus;
        return (
          <motion.div
            key={i}
            animate={{ height: on ? 18 : 6, background: m && on ? colorOf(m.color) : on ? T.muted : T.subtle }}
            transition={{ type: "spring", stiffness: 420, damping: 30 }}
            style={{ width: 6, borderRadius: m ? 3 : 1 }}
          />
        );
      })}
    </div>
  );
}
