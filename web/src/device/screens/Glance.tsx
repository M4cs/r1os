// The glance screen, as the companion draws it (companion/src/r1screen/R1Screen.tsx, after host/…/shell/Glance.kt):
// what the R1 wakes to. The clock's face, the date and status, the next alarm, a widget (an app's glance card) and the
// agent rising from the bottom to say hello.

import { useMemo } from "react";
import { R1Screen } from "@companion/r1screen/R1Screen";
import { type Look, PALETTES } from "@companion/theme";
import type { Agent } from "@companion/link";
import { member } from "../crew";

export function GlanceScreen({ accent }: { accent: string }) {
  const look: Look = useMemo(() => ({ mode: "dark", accent, glance: { clock: "stacked", background: { kind: "mesh", colors: PALETTES.dusk, dim: 0.62, grain: 0.6 } } }), [accent]);
  const sol = member("sol");
  const agent: Agent = { id: "sol", name: sol.name, form: sol.form, color: sol.color, finish: sol.finish, eyes: sol.eyes, role: sol.role, brain: sol.brain, personality: "friendly" };
  return (
    <R1Screen
      screen="glance"
      look={look}
      agents={[agent]}
      focus={agent}
      photo={() => null}
      wake={1}
      facts={{
        battery: 88,
        charging: false,
        alarm: "7:00 am",
        widgets: [
          { app: "water-log", name: "water", accent: "blue", card: { type: "stats", title: "water", items: [{ label: "today", value: "3 of 8" }, { label: "last glass", value: "9:40" }] } },
          { app: "weather-live", name: "weather now" },
          { app: "pushups", name: "pushups" },
        ],
      }}
    />
  );
}
