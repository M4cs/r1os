// The R1's look from the computer (ADR-0043): everything Settings › Look has (mode, accent, font; home's and the
// glance screen's backgrounds; the glance's clock, alignment, agent and parts), in one list, beside a small R1 that
// shows it as the R1 draws it (../r1screen). Changes go to `PUT /api/look` as only what changes.

import { Icon } from "@r1os/icons";
import { LayoutGroup, motion } from "motion/react";
import { useCallback, useEffect, useState } from "react";
import { type R1Info, api, apiFile, useR1Events } from "../link";
import { soft, spring } from "../motion";
import { ClockFace, CLOCKS, FONTS, fontOf } from "../r1screen/Clock";
import { DARK, LIGHT, css, normal } from "../r1screen/paint";
import { R1Screen, type ScreenFacts, Swatch, type Widget } from "../r1screen/R1Screen";
import { ACCENTS, type Background, type Look, type LookPatch, PALETTES, colorOf } from "../theme";
import type { R1Data } from "./R1View";
import { Item, Level, Section, Seg, Switch } from "./Settings";

const KINDS: [string, string][] = [["plain", "plain"], ["tint", "tint"], ["gradient", "gradient"], ["mesh", "soft"], ["texture", "texture"], ["photo", "photo"]];
const GRADIENTS = ["glow", "dusk", "diagonal", "horizon"];
const TEXTURES = ["grain", "noise"];
const PARTS: [string, string][] = [["date", "date"], ["status", "battery & wi-fi"], ["alarm", "next alarm"], ["notifications", "notifications"], ["playing", "now playing"], ["widgets", "widgets"], ["greeting", "hello"]];
const ALIGNS: ["left" | "center", string][] = [["left", "left"], ["center", "middle"]];
const PLACES: [string, string][] = [["center", "middle"], ["left", "left"], ["right", "right"], ["hidden", "hidden"]];

interface Photo {
  id: string;
  at: number;
}

export function LookPage({ r1, data }: { r1: R1Info; data: R1Data }) {
  const look = data.look;
  const set = data.changeLook;
  const [screen, setScreen] = useState<"home" | "glance">("home");
  const [wake, setWake] = useState(0);
  const show = (s: "home" | "glance") => {
    if (s === screen) return;
    setScreen(s);
    if (s === "glance") setWake((w) => w + 1);
  };
  // A change to one screen's look shows that screen (only a change: picking a screen above is yours to keep).
  const setHome = (p: LookPatch, now?: boolean) => {
    show("home");
    set(p, now);
  };
  const setGlance = (p: LookPatch, now?: boolean) => {
    show("glance");
    set(p, now);
  };
  const extra = useGlanceFacts(r1.id);
  const { photos, file } = usePhotos(r1.id);
  const facts: ScreenFacts = { battery: data.facts?.system?.battery_pct, charging: data.facts?.system?.charging, ...extra };
  const bgNow = normal(screen === "home" ? look?.home?.background : look?.glance?.background);
  const mode = look?.mode ?? "dark";

  return (
    <div className="look">
      <div className="look-side">
        <div className="r1-body" onClick={() => screen === "glance" && setWake((w) => w + 1)}>
          <div className="r1-glass">
            <R1Screen screen={screen} look={look} agents={data.agents} focus={data.focus} facts={facts} photo={file} wake={wake} />
          </div>
          <div className="r1-cam">
            <div className="r1-barrel">
              <span className="r1-lens" />
            </div>
          </div>
          <div className="r1-slot">
            <div className="r1-roller" />
          </div>
          <div className="r1-edge" />
        </div>
        <Seg id="screen" items={[["home", "home"], ["glance", "glance"]]} value={screen} onChange={show} />
        <p className="muted small look-hint">
          {bgNow.flow ? "scroll over it to turn the wheel; press it for the button" : screen === "glance" ? "click it to wake it again" : "as your r1 draws it"}
        </p>
      </div>

      <motion.div className="list-card look-list" initial={{ opacity: 0, y: 12 }} animate={{ opacity: 1, y: 0, transition: soft }}>
        <Section title="everywhere" />
        <Item label="mode">
          <Seg id="mode" items={[["dark", "dark"], ["light", "light"]]} value={mode} onChange={(m) => set({ mode: m })} />
        </Item>
        <Item label="accent" wide>
          <Colours id="accent" value={look?.accent ?? "orange"} onChange={(c, now) => set({ accent: c }, now)} />
        </Item>
        <Item label="font" wide>
          <div className="font-grid">
            {FONTS.map((f) => (
              <Choice key={f.id} id="font" on={(look?.font ?? "rubik") === f.id} onClick={() => set({ font: f.id })}>
                <span className="font-name" style={{ fontFamily: f.css }}>{f.label}</span>
              </Choice>
            ))}
          </div>
        </Item>

        <Section title="home" />
        <BackgroundItems which="home" look={look} set={setHome} photos={photos} file={file} />

        <Section title="glance screen" />
        <Item label="clock" wide>
          <div className="clock-grid">
            {CLOCKS.map((c) => (
              <Choice key={c} id="clock" on={(look?.glance?.clock ?? "bold") === c} onClick={() => setGlance({ glance: { clock: c } })}>
                <ClockTile face={c} look={look} />
                <span className="choice-label">{c}</span>
              </Choice>
            ))}
          </div>
        </Item>
        <Item label="lined up">
          <Seg id="align" items={ALIGNS} value={look?.glance?.align ?? "left"} onChange={(a) => setGlance({ glance: { align: a } })} />
        </Item>
        <Item label="agent" sub="where it peeks up">
          <Seg id="place" items={PLACES} value={look?.glance?.agent ?? "center"} onChange={(p) => setGlance({ glance: { agent: p } })} />
        </Item>
        <Item label="shows" wide>
          <div className="parts">
            {PARTS.map(([p, label]) => {
              const hide = look?.glance?.hide ?? [];
              const on = !hide.includes(p);
              return (
                <motion.button
                  key={p}
                  className={`part ${on ? "on" : ""}`}
                  whileTap={{ scale: 0.95 }}
                  onClick={() => setGlance({ glance: { hide: on ? [...hide, p] : hide.filter((x) => x !== p) } })}
                >
                  <Icon name={on ? "check" : "minus"} weight="bold" size={13} />
                  {label}
                </motion.button>
              );
            })}
          </div>
        </Item>
        <BackgroundItems which="glance" look={look} set={setGlance} photos={photos} file={file} />
      </motion.div>
    </div>
  );
}

// ---- backgrounds -----------------------------------------------------------------------------------------

function BackgroundItems({ which, look, set, photos, file }: { which: "home" | "glance"; look: Look | null; set: R1Data["changeLook"]; photos: Photo[] | null; file: (id: string) => string | null }) {
  const raw = look?.[which]?.background;
  const bg = normal(raw);
  const put = (p: LookPatch, now = true) => set({ [which]: { background: p } }, now);
  const [noPhotos, setNoPhotos] = useState(false);
  const firstPhoto = raw?.photo ?? photos?.[0]?.id;
  /** What each kind would look like, made from what's there now. */
  const sample = (kind: string): Background => {
    switch (kind) {
      case "mesh":
        return { kind, colors: raw?.colors?.length ? raw.colors : PALETTES.sunset, dim: raw?.dim };
      case "gradient":
        return { kind, color: raw?.color, style: raw?.kind === "gradient" ? raw.style : "glow" };
      case "texture":
        return { kind, color: raw?.color, style: raw?.kind === "texture" ? raw.style : "grain" };
      case "photo":
        return { kind, photo: firstPhoto };
      default:
        return { kind, color: raw?.color };
    }
  };
  const toKind = (kind: string) => {
    if (kind === bg.kind) return;
    // What the old kind had (its grain, dim, style, flow) goes back to the new kind's own.
    const reset = { grain: null, dim: null, style: null, flow: null };
    if (kind === "photo") {
      if (!firstPhoto) return setNoPhotos(true);
      return put({ kind, photo: firstPhoto, ...reset });
    }
    setNoPhotos(false);
    put({ kind, ...reset, ...(kind === "mesh" ? { colors: raw?.colors?.length ? raw.colors : PALETTES.sunset } : {}) });
  };
  const light = look?.mode === "light";
  const ownColour = bg.kind === "tint" || bg.kind === "gradient" || bg.kind === "texture";
  return (
    <>
      <Item label="background" wide>
        <div className="tile-grid" style={cols(6)}>
          {KINDS.map(([k, label]) => (
            <Choice key={k} id={`${which}-kind`} on={bg.kind === k} onClick={() => toKind(k)}>
              {k === "photo" && !firstPhoto ? (
                <span className="swatch-empty"><Icon name="image" weight="bold" size={20} /></span>
              ) : (
                <Swatch bg={sample(k)} look={look} photo={k === "photo" && firstPhoto ? file(firstPhoto) : null} />
              )}
              <span className="choice-label">{label}</span>
            </Choice>
          ))}
        </div>
        {noPhotos && <p className="muted small">no photos on your r1 yet: take one with its camera first.</p>}
      </Item>
      {ownColour && (
        <Item label="colour" wide>
          <Colours id={`${which}-colour`} value={raw?.color ?? null} onChange={(c, now) => put({ color: c }, now)} accentLabel />
        </Item>
      )}
      {(bg.kind === "gradient" || bg.kind === "texture") && (
        <Item label="style" wide>
          <div className="tile-grid" style={cols(bg.kind === "gradient" ? 4 : 2)}>
            {(bg.kind === "gradient" ? GRADIENTS : TEXTURES).map((s) => (
              <Choice key={s} id={`${which}-style`} on={bg.style === s} onClick={() => put({ style: s })}>
                <Swatch bg={{ ...bg, style: s }} look={look} />
                <span className="choice-label">{s}</span>
              </Choice>
            ))}
          </div>
        </Item>
      )}
      {bg.kind === "mesh" && <MeshColours which={which} bg={bg} look={look} put={put} />}
      {bg.kind === "photo" && (
        <Item label="photo" wide>
          <div className="photo-grid">
            {(photos ?? []).map((p) => {
              const src = file(p.id);
              return (
                <Choice key={p.id} id={`${which}-photo`} on={bg.photo === p.id} onClick={() => put({ photo: p.id })}>
                  {src ? <img className="photo-thumb" src={src} alt="" /> : <span className="photo-thumb loading" />}
                </Choice>
              );
            })}
          </div>
          <p className="muted small">your r1's latest photos</p>
        </Item>
      )}
      {bg.grain !== undefined && (
        <Item label="grain">
          <Level value={Math.round(bg.grain * 100)} onChange={(v) => put({ grain: v / 100 }, false)} />
        </Item>
      )}
      {bg.dim !== undefined && (
        <Item label={light ? "softened" : "dimmed"} sub="so what's on it stays easy to read">
          <Level value={Math.round(bg.dim * 100)} max={85} onChange={(v) => put({ dim: v / 100 }, false)} />
        </Item>
      )}
      {(bg.kind === "mesh" || bg.kind === "gradient") && (
        <Item label="flow" sub="its glows drift as you turn the wheel">
          <Switch on={!!bg.flow} onChange={(on) => put({ flow: on })} />
        </Item>
      )}
    </>
  );
}

/** A soft gradient's colours: one of the palettes, or up to three of your own. */
function MeshColours({ which, bg, look, put }: { which: string; bg: Background; look: Look | null; put: (p: LookPatch, now?: boolean) => void }) {
  const colors = (bg.colors?.length ? bg.colors : PALETTES.sunset).map(colorOf);
  const same = (a: string[]) => a.length === colors.length && a.every((c, i) => c.toLowerCase() === colors[i].toLowerCase());
  const named = Object.entries(PALETTES).find(([, p]) => same(p))?.[0];
  return (
    <>
      <Item label="colours" wide>
        <div className="tile-grid" style={cols(4)}>
          {Object.entries(PALETTES).map(([name, p]) => (
            <Choice key={name} id={`${which}-palette`} on={named === name} onClick={() => put({ colors: p })}>
              <Swatch bg={{ kind: "mesh", colors: p, dim: bg.dim }} look={look} />
              <span className="choice-label">{name}</span>
            </Choice>
          ))}
        </div>
      </Item>
      <Item label="your own" sub="one to three colours">
        <div className="own-colours">
          {colors.map((c, i) => (
            <span key={i} className="own-colour">
              <label className="swatch" style={{ background: c }}>
                <input type="color" value={c} onChange={(e) => put({ colors: colors.map((x, j) => (j === i ? e.target.value : x)) }, false)} />
              </label>
              {colors.length > 1 && (
                <button className="own-remove" title="remove" onClick={() => put({ colors: colors.filter((_, j) => j !== i) })}>
                  <Icon name="x" weight="bold" size={10} />
                </button>
              )}
            </span>
          ))}
          {colors.length < 3 && (
            <motion.button className="swatch add" title="add a colour" whileTap={{ scale: 0.9 }} onClick={() => put({ colors: [...colors, colorOf(look?.accent)] })}>
              <Icon name="plus" weight="bold" size={14} />
            </motion.button>
          )}
        </div>
      </Item>
    </>
  );
}

const cols = (n: number) => ({ ["--cols" as string]: n });

// ---- the pickers -----------------------------------------------------------------------------------------

/** The accents, and a colour of your own (and, for a background, "your accent": no colour of its own). */
function Colours({ id, value, onChange, accentLabel }: { id: string; value: string | null; onChange: (c: string | null, now?: boolean) => void; accentLabel?: boolean }) {
  const named = value && ACCENTS[value] ? value : null;
  const own = value && !named ? value : null;
  return (
    <LayoutGroup id={id}>
      <div className="swatches">
        {accentLabel && (
          <button className={`chip-btn ${value == null ? "on" : ""}`} onClick={() => onChange(null)}>
            your accent
          </button>
        )}
        {Object.entries(ACCENTS).map(([name, hex]) => (
          <motion.button key={name} className="swatch" title={name} style={{ background: hex }} whileHover={{ scale: 1.12 }} whileTap={{ scale: 0.92 }} onClick={() => onChange(name)}>
            {named === name && <motion.span className="swatch-ring" layoutId="ring" transition={spring} style={{ borderColor: hex }} />}
          </motion.button>
        ))}
        <motion.label className={`swatch own ${own ? "set" : ""}`} title="your own colour" style={own ? { background: own } : undefined} whileHover={{ scale: 1.12 }}>
          <input type="color" value={own ?? colorOf(value)} onChange={(e) => onChange(e.target.value, false)} />
          {own && <motion.span className="swatch-ring" layoutId="ring" transition={spring} style={{ borderColor: own }} />}
          {!own && <Icon name="eyedropper" weight="bold" size={14} />}
        </motion.label>
      </div>
    </LayoutGroup>
  );
}

/** One of a set of choices, ringed in the accent when it's the one. */
function Choice({ id, on, onClick, children }: { id: string; on: boolean; onClick: () => void; children: React.ReactNode }) {
  return (
    <motion.button className={`choice ${on ? "on" : ""}`} onClick={onClick} whileHover={{ y: -2 }} whileTap={{ scale: 0.96 }}>
      {on && <motion.span className="choice-ring" layoutId={`${id}-ring`} transition={spring} />}
      {children}
    </motion.button>
  );
}

/** A clock face, small, on the R1's own ground. */
function ClockTile({ face, look }: { face: string; look: Look | null }) {
  const t = look?.mode === "light" ? LIGHT : DARK;
  const scale = face === "stacked" ? 0.3 : face === "words" ? 0.28 : face === "dial" ? 0.42 : face === "condensed" ? 0.4 : 0.42;
  return (
    <span className="clock-tile" style={{ background: css(t.bg) }}>
      <span style={{ transform: `translate(-50%, -50%) scale(${scale})`, textTransform: "lowercase", textAlign: "center" }}>
        <ClockFace face={face} now={new Date(2026, 9, 10, 8, 22)} font={fontOf(look?.font)} t={t} accent={colorOf(look?.accent)} />
      </span>
    </span>
  );
}

// ---- what the glance shows, and the R1's photos ------------------------------------------------------------

/** The widgets and the next alarm, kept up to date while the page is open. */
function useGlanceFacts(id: string): Pick<ScreenFacts, "widgets" | "alarm"> {
  const [widgets, setWidgets] = useState<Widget[]>([]);
  const [alarm, setAlarm] = useState<string | null>(null);
  const load = useCallback(() => {
    api<{ widgets: Widget[] }>(id, "GET", "/api/glance").then((r) => setWidgets(r.widgets ?? [])).catch(() => setWidgets([]));
    api<{ alarms: { kind: string; on: boolean; at?: number }[] }>(id, "GET", "/api/alarms")
      .then((r) => {
        const next = r.alarms.filter((a) => a.kind === "alarm" && a.on && a.at).sort((a, b) => a.at! - b.at!)[0];
        setAlarm(next ? alarmText(new Date(next.at!)) : null);
      })
      .catch(() => setAlarm(null));
  }, [id]);
  useEffect(load, [load]);
  useR1Events(id, (e) => (e.type === "glance" || e.type === "alarms_changed") && load());
  return { widgets, alarm };
}

/** "7:00 am" today; "sat 7:00 am" another day. */
function alarmText(at: Date): string {
  const time = `${at.getHours() % 12 || 12}:${String(at.getMinutes()).padStart(2, "0")} ${at.getHours() < 12 ? "am" : "pm"}`;
  const today = at.toDateString() === new Date().toDateString();
  return today ? time : `${at.toLocaleDateString("en-US", { weekday: "short" }).toLowerCase()} ${time}`;
}

/** The R1's latest photos (for a photo background), and each one's picture, fetched through the link once. */
const files = new Map<string, string | Promise<void>>();
function usePhotos(id: string): { photos: Photo[] | null; file: (media: string) => string | null } {
  const [photos, setPhotos] = useState<Photo[] | null>(null);
  const [, bump] = useState(0);
  useEffect(() => {
    api<{ media: Photo[] }>(id, "GET", "/api/media?kind=photo&limit=12").then((r) => setPhotos(r.media)).catch(() => setPhotos([]));
  }, [id]);
  const file = useCallback(
    (media: string) => {
      const key = `${id}/${media}`;
      const had = files.get(key);
      if (typeof had === "string") return had;
      if (!had)
        files.set(
          key,
          apiFile(id, `/api/media/${media}/file`)
            .then((url) => {
              files.set(key, url);
              bump((x) => x + 1);
            })
            .catch(() => {
              files.delete(key);
            }),
        );
      return null;
    },
    [id],
  );
  return { photos, file };
}
