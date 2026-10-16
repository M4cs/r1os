// About an R1 (docs/specs/companion.md §6): its versions, battery, memory and space, and a backup of r1os's data to
// this computer (crate::backup in r1d: core.db, the apps and their data, photos and recordings; never the agent box,
// the CLIs' logins or partitions).

import { Icon } from "@r1os/icons";
import { motion } from "motion/react";
import { useState } from "react";
import { type R1Info, backup, inTauri, reveal } from "../link";
import { soft } from "../motion";
import { size } from "./parts";
import type { R1Data } from "./R1View";
import { Item, Section } from "./Settings";

function uptime(s?: number): string {
  if (!s) return "…";
  const h = Math.floor(s / 3600);
  return h >= 24 ? `${Math.floor(h / 24)} d ${h % 24} h` : h ? `${h} h ${Math.floor((s % 3600) / 60)} min` : `${Math.floor(s / 60)} min`;
}

export function About({ r1, data }: { r1: R1Info; data: R1Data }) {
  const s = data.facts?.system;
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null);
  const [saved, setSaved] = useState<{ dir: string; files: number; bytes: number } | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const run = async () => {
    setErr(null);
    setSaved(null);
    setProgress({ done: 0, total: 1 });
    const d = new Date();
    const label = `${r1.name} ${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")} ${String(d.getHours()).padStart(2, "0")}.${String(d.getMinutes()).padStart(2, "0")}`;
    try {
      setSaved(await backup(r1.id, label, (done, total) => setProgress({ done, total })));
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setProgress(null);
    }
  };
  return (
    <motion.div className="list-card" initial={{ opacity: 0, y: 12 }} animate={{ opacity: 1, y: 0, transition: soft }}>
      <Section title="this r1" />
      <Item label="name">
        <span className="value keep-case">{r1.name}</span>
      </Item>
      <Item label="r1d" sub="r1os's core on the r1">
        <span className="value mono">{data.facts?.version ?? "…"}</span>
      </Item>
      <Item label="running for">
        <span className="value">{uptime(data.facts?.uptime_s)}</span>
      </Item>
      <Item label="reached" sub={r1.path === "relay" ? "through a relay, so it works from anywhere" : "directly"}>
        <span className="value">{r1.path === "relay" ? "relay" : "direct"}</span>
      </Item>

      <Section title="battery and space" />
      <Item label="battery" sub={s?.charging ? "charging" : undefined}>
        <Meter value={s?.battery_pct ?? null} max={100} text={s?.battery_pct != null ? `${s.battery_pct}%` : "…"} />
      </Item>
      <Item label="memory free">
        <Meter value={s?.mem_available_mb ?? null} max={s?.mem_total_mb ?? 1} text={s?.mem_available_mb != null ? `${(s.mem_available_mb / 1024).toFixed(1)} of ${((s.mem_total_mb ?? 0) / 1024).toFixed(1)} gb` : "…"} />
      </Item>
      <Item label="space free" sub="photos, recordings, apps and on-device models">
        <Meter value={s?.storage_free_mb ?? null} max={s?.storage_total_mb ?? 1} text={s?.storage_free_mb != null ? `${(s.storage_free_mb / 1024).toFixed(1)} of ${((s.storage_total_mb ?? 0) / 1024).toFixed(1)} gb` : "…"} />
      </Item>

      <Section title="backup" />
      <Item label="back up to this computer" sub="agents, chats, memories, the journal, alarms, the look, your apps and their data, photos and recordings. never sign-ins or the system." wide>
        <div className="row-buttons">
          <motion.button className="btn small" whileTap={{ scale: 0.95 }} disabled={!!progress || !inTauri} onClick={() => void run()}>
            <Icon name="cloud-arrow-down" weight="bold" size={14} />
            {progress ? "backing up…" : "back up now"}
          </motion.button>
          {!inTauri && <span className="muted small">in the app only</span>}
        </div>
        {progress && (
          <div className="track">
            <motion.div className="track-fill" animate={{ width: `${(100 * progress.done) / Math.max(1, progress.total)}%` }} transition={soft} />
          </div>
        )}
        {saved && (
          <div className="row-buttons">
            <span className="muted small">
              {saved.files} files, {size(saved.bytes)}, in {saved.dir}
            </span>
            <button className="btn small" onClick={() => void reveal(saved.dir)}>
              <Icon name="folder-open" weight="bold" size={14} /> show
            </button>
          </div>
        )}
        {err && <p className="err">{err}</p>}
      </Item>
    </motion.div>
  );
}

function Meter({ value, max, text }: { value: number | null; max: number; text: string }) {
  return (
    <div className="level-inline">
      <div className="track">
        <motion.div className="track-fill" initial={{ width: 0 }} animate={{ width: `${value == null ? 0 : Math.max(2, (100 * value) / max)}%` }} transition={soft} />
      </div>
      <span className="muted small meter-text">{text}</span>
    </div>
  );
}
