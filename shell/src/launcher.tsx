// The launcher (double tap from home; docs/specs/experience.md §2.2): new app, your apps, tools and
// settings, on the wheel. Chats live in each agent's space (space.tsx), reached from home.

import { useEffect, useState } from "react";
import { Icon } from "@r1os/icons";
import { appsApi, type AppSummary } from "./apps/host";
import { onEvent } from "./r1";
import { SETTINGS_TITLE, type SettingsPage } from "./settings";
import { useFocusList } from "./setup";
import { COLORS } from "./ui/Character";

export type Target =
  | { t: "create" }
  | { t: "app"; id: string }
  /** A chat with `agent`: an existing one (`id`), or a new one. */
  | { t: "chat"; agent: string; id?: string; say?: string; app?: string }
  /** An agent's space: its chats, memory and settings. */
  | { t: "agent"; id: string }
  | { t: "new-agent" }
  | { t: "agent-look"; id: string }
  | { t: "chats"; agent: string }
  | { t: "memory"; agent: string }
  | { t: "manage" }
  | { t: "app-info"; id: string }
  | { t: "tool"; id: string }
  | { t: "settings"; page: SettingsPage }
  | { t: "android" }
  | { t: "sleep" };

const TOOLS = [
  { id: "camera", name: "Camera", glyph: "◐" },
  { id: "inputs", name: "Inputs", glyph: "◎" },
  { id: "clock", name: "Clock", glyph: "◷" },
  { id: "system", name: "System", glyph: "▤" },
];

const SETTINGS: SettingsPage[] = ["you", "accounts", "wifi", "about"];

export function Launcher({ onOpen }: { onOpen: (t: Target) => void }) {
  const [apps, setApps] = useState<AppSummary[]>([]);
  useEffect(() => {
    const load = () => void appsApi.list().then(setApps, () => {});
    load();
    return onEvent((ev) => {
      if (ev.type === "app_installed" || ev.type === "app_removed") load();
    });
  }, []);
  const ref = useFocusList<HTMLDivElement>([apps.length]);

  return (
    <div className="launcher" ref={ref}>
      <button className="l-item l-new" onClick={() => onOpen({ t: "create" })}>
        <span className="l-glyph" style={{ ["--accent" as string]: COLORS.orange }}>
          +
        </span>
        <span className="grow">New app</span>
      </button>

      {apps.length > 0 && <div className="l-section">Apps</div>}
      {apps.map((a) => (
        <button key={a.id} className="l-item" onClick={() => onOpen({ t: "app", id: a.id })}>
          <span className="l-glyph" style={{ ["--accent" as string]: COLORS[a.accent ?? ""] ?? "#9aa4b2" }}>
            <Icon name={a.icon} weight="bold" size={20} />
          </span>
          <span className="grow">{a.name}</span>
        </button>
      ))}

      <button className="l-item" onClick={() => onOpen({ t: "manage" })}>
        <span className="l-glyph">☰</span>
        <span className="grow">Manage apps</span>
      </button>

      <div className="l-section">Tools</div>
      {TOOLS.map((t) => (
        <button key={t.id} className="l-item" onClick={() => onOpen({ t: "tool", id: t.id })}>
          <span className="l-glyph">{t.glyph}</span>
          <span className="grow">{t.name}</span>
        </button>
      ))}

      <div className="l-section">Settings</div>
      {SETTINGS.map((page) => (
        <button key={page} className="l-item" onClick={() => onOpen({ t: "settings", page })}>
          <span className="l-glyph">{page === "you" ? "☺" : "⚙"}</span>
          <span className="grow">{SETTINGS_TITLE[page]}</span>
        </button>
      ))}
      <button className="l-item" onClick={() => onOpen({ t: "android" })}>
        <span className="l-glyph">◇</span>
        <span className="grow">Android settings</span>
      </button>
      <button className="l-item" onClick={() => onOpen({ t: "sleep" })}>
        <span className="l-glyph">☾</span>
        <span className="grow">Sleep</span>
      </button>
    </div>
  );
}
