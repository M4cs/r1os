// Three chats as the R1 draws them (host/…/shell/ChatScreen.kt, CardView.kt): an answer on a card (ADR-0039), one agent
// asking another (ADR-0012), and an agent working on your computer asking before it changes anything (ADR-0047,
// ADR-0045).

import { Icon } from "@r1os/icons";
import { motion } from "motion/react";
import { useEffect } from "react";
import { Character } from "@shell/ui/Character";
import { member } from "../crew";
import { Screen, useDevice } from "../R1";
import { Bubble, Card, ChatHead, Dots, fill, INK, Mono, pop, T, Thread, useSteps } from "../ui";

const deg = (n: number) => `${n}°`;

export function CardScreen({ accent }: { accent: string }) {
  const sol = member("sol");
  const step = useSteps([250, 800, 1700, 2600]);
  const days = [
    { day: "sat", icon: "cloud", lo: 59, hi: 71 },
    { day: "sun", icon: "cloud-rain", lo: 61, hi: 65 },
    { day: "mon", icon: "sun", lo: 58, hi: 74 },
  ];
  return (
    <Screen>
      <ChatHead name="sol" form={sol.form} color={sol.color} finish={sol.finish} />
      <Thread>
        {step >= 1 && (
          <Bubble mine color={accent}>
            What's the weather this week?
          </Bubble>
        )}
        {step === 2 && <Dots color={accent} />}
        {step >= 3 && (
          <Card accent={accent} title="portland">
            <div style={{ display: "flex", alignItems: "center", gap: 14 }}>
              <Icon name="cloud" weight="bold" size={58} style={{ color: accent }} />
              <div>
                <div style={{ fontSize: 42, fontWeight: 700, lineHeight: 1, color: T.fg }}>66°F</div>
                <div style={{ fontSize: 16, fontWeight: 600, color: T.muted, textTransform: "lowercase" }}>overcast</div>
              </div>
            </div>
            <div style={{ fontSize: 13, fontWeight: 600, color: T.muted, margin: "6px 0 8px" }}>feels 61°</div>
            <div style={{ height: 1, background: T.subtle }} />
            {days.map((d, i) => (
              <motion.div key={d.day} initial={{ opacity: 0, x: -8 }} animate={{ opacity: 1, x: 0 }} transition={{ ...pop, delay: 0.08 * i + 0.1 }} style={{ display: "flex", alignItems: "center", marginTop: 7, fontSize: 15, fontWeight: 600 }}>
                <span style={{ flex: 1, color: T.fg }}>{d.day}</span>
                <Icon name={d.icon} weight="bold" size={19} style={{ marginRight: 16, color: T.fg }} />
                <span style={{ width: 40, color: T.muted }}>{deg(d.lo)}</span>
                <span style={{ width: 34, textAlign: "right", fontWeight: 700, color: T.fg }}>{deg(d.hi)}</span>
              </motion.div>
            ))}
          </Card>
        )}
        {step >= 4 && <Bubble color={accent}>Mild and grey today. Sunday brings rain, then it warms up for Monday.</Bubble>}
      </Thread>
    </Screen>
  );
}

const HEADLINES = [
  { title: "A new comet is bright enough to see this week", from: "science", hue: 210 },
  { title: "The city opens its longest bike path yet", from: "local", hue: 140 },
  { title: "The year's most borrowed library books", from: "books", hue: 30 },
];

export function AskScreen({ accent }: { accent: string }) {
  const echo = member("echo");
  const nova = member("nova");
  const step = useSteps([250, 900, 1500, 2700]);
  return (
    <Screen>
      <ChatHead name="echo" form={echo.form} color={echo.color} finish={echo.finish} />
      <Thread>
        {step >= 1 && (
          <Bubble mine color="#f06aa8">
            ask nova what's in the news
          </Bubble>
        )}
        {step >= 2 && (
          <motion.div initial={{ opacity: 0, x: -14 }} animate={{ opacity: 1, x: 0 }} transition={pop} style={{ display: "flex", alignItems: "flex-start", gap: 10, padding: "10px 14px", borderRadius: 16, background: T.card, boxShadow: `inset 3px 0 0 ${accent}` }}>
            <Character form={nova.form} color={nova.color} finish={nova.finish} state={step >= 4 ? "idle" : "thinking"} size={30} />
            <div>
              <div style={{ fontSize: 13, fontWeight: 700, color: accent }}>{step >= 4 ? "nova answered" : step >= 3 ? "nova is answering…" : "asked nova"}</div>
              <div style={{ fontSize: 15, fontWeight: 600, color: T.fg, textTransform: "none" }}>What's in the news today?</div>
            </div>
          </motion.div>
        )}
        {step === 3 && <Dots color={accent} />}
        {step >= 4 && (
          <Card accent={accent} title="from nova · headlines">
            <div style={{ display: "flex", flexDirection: "column", gap: 9 }}>
              {HEADLINES.map((h, i) => (
                <motion.div key={h.title} initial={{ opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0 }} transition={{ ...pop, delay: 0.09 * i }} style={{ display: "flex", gap: 11, alignItems: "center" }}>
                  <div style={{ width: 56, height: 42, borderRadius: 9, flex: "none", background: `linear-gradient(135deg, hsl(${h.hue} 70% 58%), hsl(${h.hue + 40} 60% 28%))` }} />
                  <div style={{ minWidth: 0 }}>
                    <div style={{ fontSize: 14, fontWeight: 700, color: T.fg, lineHeight: 1.15 }}>{h.title}</div>
                    <div style={{ fontSize: 12, fontWeight: 600, color: T.muted, textTransform: "lowercase" }}>{h.from}</div>
                  </div>
                </motion.div>
              ))}
            </div>
          </Card>
        )}
      </Thread>
    </Screen>
  );
}

export function PcScreen({ accent }: { accent: string }) {
  const bolt = member("bolt");
  const step = useSteps([250, 850, 1300, 1750, 2250, 3100, 3500]);
  const { hold, roll } = useDevice();
  useEffect(() => {
    if (step === 5) roll(-1);
    if (step === 6) hold(true);
    if (step === 7) hold(false);
  }, [step, hold, roll]);
  // Let the button go if the screen is put away mid-hold.
  useEffect(() => () => hold(false), [hold]);
  const tool = (icon: string, words: string, color?: string) => (
    <motion.div initial={{ opacity: 0, x: -8 }} animate={{ opacity: 1, x: 0 }} transition={pop} style={{ display: "flex", alignItems: "center", gap: 8, fontSize: 13, fontWeight: 600, color: color ?? T.muted, paddingLeft: 4 }}>
      <Icon name={icon} weight="bold" size={15} />
      <Mono>{words}</Mono>
    </motion.div>
  );
  const chosen = step >= 5 ? 0 : 1;
  return (
    <Screen>
      <ChatHead
        name="bolt"
        form={bolt.form}
        color={bolt.color}
        finish={bolt.finish}
        chip={
          <span style={{ display: "inline-flex", alignItems: "center", gap: 5, padding: "4px 9px", borderRadius: 999, background: T.card, fontSize: 12, fontWeight: 700, color: T.muted }}>
            <Icon name="desktop" weight="bold" size={13} /> on your pc
          </span>
        }
      />
      <Thread>
        {step >= 1 && (
          <Bubble mine color={accent}>
            run the tests in my site folder and fix what fails
          </Bubble>
        )}
        {step >= 2 && tool("folder-open", "reading site/")}
        {step >= 3 && tool("terminal-window", "npm test")}
        {step >= 4 && tool("x-circle", "2 of 48 failed", "#ff8f8f")}
        {step >= 5 && step < 7 && (
          <motion.div initial={{ opacity: 0, y: 14 }} animate={{ opacity: 1, y: 0 }} transition={pop} style={{ padding: "12px 12px 10px", borderRadius: 18, background: T.card }}>
            <div style={{ fontSize: 13, fontWeight: 700, color: accent }}>bolt asks</div>
            <div style={{ fontSize: 17, fontWeight: 700, color: T.fg, margin: "2px 0 10px" }}>edit 2 files in site?</div>
            <div style={{ display: "flex", gap: 6 }}>
              {["allow", "always", "not now"].map((w, i) => (
                <div key={w} style={{ position: "relative", flex: 1, height: 38, display: "grid", placeItems: "center", borderRadius: 11, background: T.focus, fontSize: 14, fontWeight: 700, color: chosen === i ? INK : T.fg }}>
                  {chosen === i && <motion.div layoutId="ask-fill" transition={fill} style={{ position: "absolute", inset: 0, borderRadius: 11, background: accent }} />}
                  <span style={{ position: "relative" }}>{w}</span>
                </div>
              ))}
            </div>
          </motion.div>
        )}
        {step >= 7 && tool("pencil-simple", "edited 2 files")}
        {step >= 7 && tool("check-circle", "48 of 48 passed", "#3ddc84")}
      </Thread>
    </Screen>
  );
}
