// Two of the plainer screens the launcher opens (ADR-0018, ADR-0046): what's up, and the camera with its knobs.

import { Icon } from "@r1os/icons";
import { motion } from "motion/react";
import { Character } from "@shell/ui/Character";
import { member } from "../crew";
import { Screen } from "../R1";
import { fill, INK, Row, T, Title } from "../ui";

/** What's up: what's new and next, with no agent and no network. */
export function WhatsUpScreen({ accent }: { accent: string }) {
  const kiwi = member("kiwi");
  return (
    <Screen>
      <Title>what's up</Title>
      <div style={{ position: "absolute", left: 18, top: 50, fontSize: 13, fontWeight: 600, color: T.muted }}>new</div>
      <div style={{ position: "absolute", left: 10, right: 10, top: 70 }}>
        <div style={{ position: "relative", display: "flex", alignItems: "center", gap: 12, height: 58, padding: "0 10px" }}>
          <motion.div layoutId="whatsup-fill" transition={fill} style={{ position: "absolute", inset: 0, borderRadius: 12, background: accent }} />
          <div style={{ position: "relative" }}>
            <Character form={kiwi.form} color={kiwi.color} finish={kiwi.finish} state="idle" size={38} />
          </div>
          <div style={{ position: "relative" }}>
            <div style={{ fontSize: 18, fontWeight: 700, color: INK }}>kiwi replied</div>
            <div style={{ fontSize: 13, fontWeight: 600, color: "rgba(0,0,0,.62)" }}>your 5k plan for this week</div>
          </div>
        </div>
      </div>
      <div style={{ position: "absolute", left: 18, top: 140, fontSize: 13, fontWeight: 600, color: T.muted }}>next</div>
      <div style={{ position: "absolute", left: 10, right: 10, top: 160, display: "flex", flexDirection: "column", gap: 4 }}>
        <Row icon="bell-ringing" label="call mom" sub="reminder · 5:00 pm" accent={accent} />
        <Row icon="timer" label="pasta" sub="timer · 6 min left" accent={accent} />
        <Row icon="repeat" label="weekday briefing" sub="routine · 7:30 am · sol" accent={accent} />
        <Row icon="alarm" label="7:00 am" sub="alarm · weekdays" accent={accent} />
      </div>
    </Screen>
  );
}

/** The camera as an instrument (ADR-0046): the viewfinder, a film, and four knobs the wheel turns. */
export function CameraScreen({ accent }: { accent: string }) {
  const knobs = [
    { c: "#4ea1ff", label: "film" },
    { c: "#3ddc84", label: "exp" },
    { c: "#f2f2f2", label: "wb" },
    { c: "#ff8a3d", label: "zoom" },
  ];
  return (
    <Screen>
      <div style={{ position: "absolute", inset: "0 0 96px 0", background: "radial-gradient(120% 90% at 30% 25%, #6d8cab 0%, #3a4c63 38%, #1b2430 70%, #0d1117 100%)" }}>
        <div style={{ position: "absolute", left: "18%", bottom: 0, width: "34%", height: "62%", borderRadius: "48% 48% 6px 6px", background: "linear-gradient(180deg, #2e5d3f, #1b3626)" }} />
        <div style={{ position: "absolute", right: "14%", bottom: 0, width: "26%", height: "40%", borderRadius: "10px 10px 0 0", background: "linear-gradient(180deg, #8a6a4f, #4d3a2b)" }} />
        <div style={{ position: "absolute", inset: 0, backgroundImage: "linear-gradient(rgba(255,255,255,.14) 1px, transparent 1px), linear-gradient(90deg, rgba(255,255,255,.14) 1px, transparent 1px)", backgroundSize: "120px 128px", backgroundPosition: "-1px -1px" }} />
        <div style={{ position: "absolute", left: 14, top: 12, padding: "3px 8px", borderRadius: 6, background: "rgba(0,0,0,.45)", fontSize: 12, fontWeight: 700 }}>film · warm</div>
      </div>
      <div style={{ position: "absolute", left: 12, right: 12, bottom: 12, height: 74, display: "flex", gap: 8 }}>
        {knobs.map((k, i) => (
          <div key={k.label} style={{ flex: 1, borderRadius: 12, background: T.card, display: "grid", placeItems: "center", alignContent: "center", gap: 6, boxShadow: i === 0 ? `inset 0 0 0 2px ${accent}` : "none" }}>
            <div style={{ position: "relative", width: 30, height: 30, borderRadius: 15, background: k.c, boxShadow: "inset 0 -3px 0 rgba(0,0,0,.25)" }}>
              <span style={{ position: "absolute", left: 14, top: 3, width: 2, height: 9, borderRadius: 1, background: "#111", transform: `rotate(${i * 40 - 30}deg)`, transformOrigin: "1px 12px" }} />
            </div>
            <div style={{ fontSize: 11, fontWeight: 600, color: T.muted }}>{k.label}</div>
          </div>
        ))}
      </div>
      <div style={{ position: "absolute", right: 16, bottom: 104, width: 38, height: 38, borderRadius: 19, display: "grid", placeItems: "center", background: "rgba(0,0,0,.45)" }}>
        <Icon name="arrows-clockwise" weight="bold" size={18} />
      </div>
    </Screen>
  );
}
