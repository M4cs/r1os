// A small R1 showing its home or glance screen as the R1 draws them (host/…/shell/HomeScreen.kt, Glance.kt): laid out
// in the R1's own 360×480 dp and scaled to fit, its background painted as the R1 paints it (./paint), its fonts and
// clock faces the R1's. Turning the mouse wheel over a flowing background moves it as the R1's wheel does; pressing
// on it, as the button does.

import { Icon } from "@r1os/icons";
import { AnimatePresence, motion } from "motion/react";
import { type CSSProperties, useEffect, useLayoutEffect, useRef, useState } from "react";
import { Character } from "@shell/ui/Character";
import type { Agent } from "../link";
import { type Background, type Look, colorOf } from "../theme";
import { ClockFace, fontOf, hm } from "./Clock";
import { DARK, H, LIGHT, Painter, type Tokens, W } from "./paint";

export interface Widget {
  app: string;
  name: string;
  icon?: string;
  accent?: string;
  note?: string;
  /** The app's glance card, filled in (crate::cards). */
  card?: Card;
}

/** A card as r1d checked it (crate::cards): the kinds a widget draws. */
export interface Card {
  type: string;
  title?: string;
  summary?: string;
  items?: { label?: string; value?: string | number; delta?: string; trend?: string }[];
  unit?: string;
  place?: string;
  temp?: number;
  condition?: string;
  icon?: string;
  high?: number;
  low?: number;
  feels?: number;
  days?: { day?: string; high?: number; low?: number; icon?: string }[];
}

export interface ScreenFacts {
  battery?: number | null;
  charging?: boolean | null;
  /** The next alarm, as the glance writes it ("7:00 am"). */
  alarm?: string | null;
  widgets?: Widget[];
}

interface Props {
  screen: "home" | "glance";
  look: Look | null;
  agents: Agent[];
  focus: Agent | null;
  facts: ScreenFacts;
  /** A photo background's picture, by its media id. */
  photo: (id: string) => string | null;
  /** Bumped to wake the glance screen again (the agent rises, the greeting shows). */
  wake: number;
}

export function R1Screen({ screen, look, agents, focus, facts, photo, wake }: Props) {
  const box = useRef<HTMLDivElement>(null);
  const [k, setK] = useState(0.56);
  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setK(el.clientWidth / W));
    ro.observe(el);
    setK(el.clientWidth / W);
    return () => ro.disconnect();
  }, []);
  const light = look?.mode === "light";
  const t = light ? LIGHT : DARK;
  const accent = colorOf(look?.accent);
  const font = fontOf(look?.font);
  const now = useMinute();

  // The wheel and the button, for a flowing background.
  const painter = useRef<Painter | null>(null);
  useEffect(() => {
    const el = box.current;
    if (!el) return;
    let last = 0;
    const wheel = (e: WheelEvent) => {
      const p = painter.current;
      if (!p?.flows) return;
      e.preventDefault();
      if (e.timeStamp - last < 45 || Math.abs(e.deltaY) < 1) return;
      last = e.timeStamp;
      p.nudge(e.deltaY);
    };
    el.addEventListener("wheel", wheel, { passive: false });
    return () => el.removeEventListener("wheel", wheel);
  }, []);

  const bg = screen === "home" ? look?.home?.background : look?.glance?.background;
  return (
    <div
      className="r1-screen"
      ref={box}
      onPointerDown={() => painter.current?.press(true)}
      onPointerUp={() => painter.current?.press(false)}
      onPointerLeave={(e) => e.buttons && painter.current?.press(false)}
    >
      <div className="r1-dp" style={{ transform: `scale(${k})`, fontFamily: font, color: t.fg }}>
        <AnimatePresence initial={false}>
          <motion.div key={screen} className="r1-layer" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }} transition={{ duration: 0.28 }}>
            <Backdrop bg={bg} light={light} accent={accent} k={k} photo={bg?.kind === "photo" && bg.photo ? photo(bg.photo) : null} onPainter={(p) => (painter.current = p)} />
            {screen === "home" ? (
              <Home t={t} agents={agents} focus={focus} facts={facts} now={now} />
            ) : (
              <Glance t={t} look={look} font={font} accent={accent} focus={focus} facts={facts} now={now} wake={wake} />
            )}
          </motion.div>
        </AnimatePresence>
      </div>
    </div>
  );
}

/** The time, ticking over each minute. */
function useMinute(): Date {
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    const id = setTimeout(() => setNow(new Date()), 60_000 - (Date.now() % 60_000) + 50);
    return () => clearTimeout(id);
  }, [now]);
  return now;
}

/** The background, on a canvas at the size it's shown. */
function Backdrop({ bg, light, accent, k, photo, onPainter }: { bg?: Background; light: boolean; accent: string; k: number; photo: string | null; onPainter: (p: Painter) => void }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const painter = useRef<Painter | null>(null);
  const [img, setImg] = useState<HTMLImageElement | null>(null);
  useEffect(() => {
    if (!photo) return setImg(null);
    const i = new Image();
    i.onload = () => setImg(i);
    i.src = photo;
  }, [photo]);
  useEffect(() => {
    const c = canvas.current!;
    painter.current ??= new Painter(c);
    onPainter(painter.current);
    return () => painter.current?.stop();
  }, []); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => {
    const c = canvas.current!;
    const dpr = window.devicePixelRatio || 1;
    c.width = Math.max(1, Math.round(W * k * dpr));
    c.height = Math.max(1, Math.round(H * k * dpr));
    painter.current!.set(bg, light, accent, img);
  }, [bg, light, accent, k, img]);
  return <canvas ref={canvas} className="r1-canvas" />;
}

/** A small background on its own (the look page's choices). */
export function Swatch({ bg, look, photo, width = 60 }: { bg: Background; look: Look | null; photo?: string | null; width?: number }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const [img, setImg] = useState<HTMLImageElement | null>(null);
  useEffect(() => {
    if (!photo) return setImg(null);
    const i = new Image();
    i.onload = () => setImg(i);
    i.src = photo;
  }, [photo]);
  const key = JSON.stringify([bg, look?.mode, look?.accent]);
  useEffect(() => {
    const c = canvas.current!;
    const dpr = window.devicePixelRatio || 1;
    c.width = Math.round(width * dpr);
    c.height = Math.round(((width * H) / W) * dpr);
    new Painter(c).set(bg, look?.mode === "light", colorOf(look?.accent), img);
  }, [key, width, img]); // eslint-disable-line react-hooks/exhaustive-deps
  return <canvas ref={canvas} className="swatch-canvas" />;
}

// ---- the parts both screens share ------------------------------------------------------------------------

const text = (size: number, weight: number, color: string, extra?: CSSProperties): CSSProperties => ({
  fontSize: size,
  fontWeight: weight >= 650 ? 700 : weight >= 500 ? 500 : 400,
  color,
  lineHeight: "normal",
  ...extra,
});

/** Battery, charging and Wi-Fi (and on the glance, the next alarm), in the muted colour. */
function Status({ t, facts, alarm, style }: { t: Tokens; facts: ScreenFacts; alarm?: string | null; style?: CSSProperties }) {
  return (
    <div style={{ display: "flex", alignItems: "center", color: t.muted, ...style }}>
      {facts.battery != null && <span style={text(13, 600, t.muted, { fontVariantNumeric: "tabular-nums" })}>{facts.battery}%</span>}
      {facts.charging && <Icon name="lightning" weight="bold" size={13} style={{ marginLeft: 2 }} />}
      <Icon name="wifi-high" weight="bold" size={15} style={{ marginLeft: facts.battery != null ? 9 : 0 }} />
      {alarm && (
        <>
          <Icon name="alarm" weight="bold" size={14} style={{ marginLeft: 10 }} />
          <span style={text(13, 600, t.muted, { marginLeft: 3, fontVariantNumeric: "tabular-nums" })}>{alarm}</span>
        </>
      )}
    </div>
  );
}

const BUSY: Record<string, string> = { thinking: "thinking…", working: "working…", speaking: "talking…", listening: "listening…", waiting: "waiting for you" };

// ---- home ------------------------------------------------------------------------------------------------

/** Home: the time at the top left, the agent on screen with its name, its neighbours peeking, the side dots. */
function Home({ t, agents, focus, facts, now }: { t: Tokens; agents: Agent[]; focus: Agent | null; facts: ScreenFacts; now: Date }) {
  const i = Math.max(0, agents.findIndex((a) => a.id === focus?.id));
  const prev = i > 0 ? agents[i - 1] : null;
  const next: Agent | "new" | null = agents.length === 0 ? null : i < agents.length - 1 ? agents[i + 1] : "new";
  const busy = focus?.state ? BUSY[focus.state] : undefined;
  const failed = focus?.state === "error";
  return (
    <>
      <div style={{ position: "absolute", left: 20, top: 14 }}>
        <div style={text(34, 700, t.fg, { fontVariantNumeric: "tabular-nums", letterSpacing: "-0.01em" })}>{hm(now)}</div>
        <Status t={t} facts={facts} style={{ marginTop: 5 }} />
      </div>
      {prev && <Neighbour agent={prev} y={4} />}
      {next && <Neighbour agent={next} y={492} t={t} />}
      {focus && (
        <>
          <div style={{ position: "absolute", left: 0, right: 0, bottom: H - 136, textAlign: "center", textTransform: "lowercase", ...text(30, 700, t.fg, { letterSpacing: "-0.01em" }) }}>{focus.name}</div>
          <div style={{ position: "absolute", left: 74, top: 142, width: 212, height: 212 }}>
            <Character form={focus.form} color={focus.color} finish={focus.finish} eyes={focus.eyes} wear={focus.wear} state={(focus.state as never) ?? "idle"} unread={focus.unread ?? 0} size={212} />
          </div>
          <div style={{ position: "absolute", left: 28, right: 28, top: 360, minHeight: 24, textAlign: "center", ...(failed ? text(17, 650, t.bad) : busy ? text(17, 650, t.fg) : text(14, 500, t.muted)) }}>
            {failed ? "something went wrong" : (busy ?? "")}
          </div>
        </>
      )}
      <SideDots t={t} agents={agents} focus={focus} />
    </>
  );
}

/** The agent above or below, half size and faint; below the last, the new agent's dashed ring. */
function Neighbour({ agent, y, t }: { agent: Agent | "new"; y: number; t?: Tokens }) {
  const style: CSSProperties = { position: "absolute", left: 180 - 53, top: y - 53, width: 106, height: 106, opacity: 0.42, display: "grid", placeItems: "center" };
  if (agent === "new")
    return (
      <div style={style}>
        <svg width={90} height={90} viewBox="0 0 90 90">
          <circle cx={45} cy={45} r={44.25} fill="none" stroke={t === LIGHT ? t.subtle : "#3a3e46"} strokeWidth={1.5} strokeDasharray="4.5 3" />
        </svg>
      </div>
    );
  return (
    <div style={style}>
      <Character form={agent.form} color={agent.color} finish={agent.finish} eyes={agent.eyes} wear={agent.wear} state="idle" size={106} />
    </div>
  );
}

/** The column of dots at the right edge: one per agent (the one on screen tall), and the new agent's. */
function SideDots({ t, agents, focus }: { t: Tokens; agents: Agent[]; focus: Agent | null }) {
  const entries = [...agents.map((a) => ({ a, on: a.id === focus?.id })), { a: null, on: false }];
  const height = entries.reduce((s, e) => s + (e.on ? 18 : 6), 0) + 7 * (entries.length - 1);
  return (
    <div style={{ position: "absolute", left: 343, top: 248 - height / 2, width: 6, display: "flex", flexDirection: "column", gap: 7 }}>
      {entries.map(({ a, on }, i) => {
        const lit = a && (on || (a.state && a.state !== "idle" && a.state !== "sleeping") || (a.unread ?? 0) > 0);
        const color = a && lit ? colorOf(a.color) : on ? t.muted : t.subtle;
        return (
          <motion.div
            key={a?.id ?? `new${i}`}
            animate={{ height: on ? 18 : 6 }}
            transition={{ duration: 0.26 }}
            style={{ width: 6, borderRadius: a ? 3 : 1, background: color, outline: a && (a.unread ?? 0) > 0 ? `1.5px solid ${colorOf(a.color)}` : undefined, outlineOffset: 2 }}
          />
        );
      })}
    </div>
  );
}

// ---- the glance screen -----------------------------------------------------------------------------------

const RISE = {
  y: [90, -18, 5, -2, 0],
  scaleY: [0.86, 1.1, 0.9, 1.03, 1],
  scaleX: [1.08, 0.93, 1.08, 0.98, 1],
  rotate: [-5, 4, -2.5, 1, 0],
};
const RISE_EASE = [0.42, 0, 0.58, 1] as const;
const riseAt = (times: number[]) => ({ duration: 0.9, delay: 0.09, ease: RISE_EASE, times });

function greeting(focus: Agent | null, now: Date): string {
  const n = focus?.unread ?? 0;
  if (n > 1) return `${n} new from me`;
  if (n === 1) return "i've got a reply for you";
  const h = now.getHours();
  return h >= 5 && h <= 10 ? "morning!" : h >= 11 && h <= 16 ? "hey there" : h >= 17 && h <= 21 ? "evening!" : "still up?";
}

/** The glance screen: the clock (its face), the date and status, the widgets, the agent peeking up and its hello. */
function Glance({ t, look, font, accent, focus, facts, now, wake }: { t: Tokens; look: Look | null; font: string; accent: string; focus: Agent | null; facts: ScreenFacts; now: Date; wake: number }) {
  const g = look?.glance ?? {};
  const hide = new Set(g.hide ?? []);
  const center = g.align === "center";
  const place = g.agent ?? "center";
  const widgets = hide.has("widgets") ? [] : (facts.widgets ?? []);
  const peek = widgets.length ? 124 : 172;
  const info = useRef<HTMLDivElement>(null);
  const [infoH, setInfoH] = useState(130);
  useLayoutEffect(() => setInfoH(info.current?.offsetHeight ?? 130));
  const top = Math.max(160, 22 + infoH + 12);
  const showStatus = !hide.has("status");
  const alarm = hide.has("alarm") ? null : facts.alarm;
  const agentLeft = place === "left" ? -34 : place === "right" ? 162 : 64;
  return (
    <>
      <div ref={info} style={{ position: "absolute", top: 22, left: center ? 0 : 20, right: center ? 0 : undefined, display: "flex", flexDirection: "column", alignItems: center ? "center" : "flex-start", textAlign: center ? "center" : "left" }}>
        <ClockFace face={g.clock} now={now} font={font} t={t} accent={accent} />
        {!hide.has("date") && (
          <div style={text(17, 500, t.muted, { marginTop: 8, textTransform: "lowercase" })}>
            {now.toLocaleDateString("en-US", { weekday: "long", month: "long", day: "numeric" })}
          </div>
        )}
        {(showStatus || alarm) && <Status t={t} facts={showStatus ? facts : {}} alarm={alarm} style={{ marginTop: 8 }} />}
      </div>
      {widgets.length > 0 && (
        <div style={{ position: "absolute", left: 14, right: 14, top, bottom: peek + 10, display: "flex", flexDirection: "column" }}>
          <div style={{ flex: 1, minHeight: 0, overflow: "hidden" }}>
            <WidgetCard w={widgets[0]} t={t} accent={accent} />
          </div>
          {widgets.length > 1 && (
            <div style={{ height: 6, marginTop: 8, display: "flex", justifyContent: "center" }}>
              {widgets.map((w, i) => (
                <span key={w.app} style={{ width: 6, height: 6, borderRadius: 3, margin: "0 3px", background: i === 0 ? t.fg : t.muted, opacity: i === 0 ? 1 : 0.45 }} />
              ))}
            </div>
          )}
        </div>
      )}
      {focus && place !== "hidden" && (
        <motion.div
          key={`rise${wake}`}
          style={{ position: "absolute", left: agentLeft, top: H - peek, width: 232, height: 232, originX: 0.5, originY: 1 }}
          initial={{ y: 90, scaleY: 0.86, scaleX: 1.08, rotate: -5 }}
          animate={RISE}
          transition={{ y: riseAt([0, 0.42, 0.68, 0.86, 1]), scaleY: riseAt([0, 0.38, 0.6, 0.8, 1]), scaleX: riseAt([0, 0.38, 0.6, 0.8, 1]), rotate: riseAt([0, 0.4, 0.65, 0.85, 1]) }}
        >
          <Character form={focus.form} color={focus.color} finish={focus.finish} eyes={focus.eyes} wear={focus.wear} state="idle" size={232} />
        </motion.div>
      )}
      {focus && place !== "hidden" && !hide.has("greeting") && (
        <motion.div
          key={`hello${wake}`}
          style={{
            position: "absolute",
            bottom: peek + 10,
            ...(place === "left" ? { left: 18 } : place === "right" ? { right: 18 } : { left: 0, right: 0, marginInline: "auto", width: "fit-content" }),
            padding: "9px 14px 10px",
            borderRadius: 18,
            background: t.surface,
            textTransform: "lowercase",
            ...text(16, 650, t.fg),
          }}
          initial={{ opacity: 0, y: 8 }}
          animate={{ opacity: [0, 1, 1, 0], y: [8, 0, 0, 0] }}
          transition={{ delay: 1.79, duration: 4.2, times: [0, 0.3 / 4.2, 3.9 / 4.2, 1] }}
        >
          {greeting(focus, now)}
        </motion.div>
      )}
    </>
  );
}

/** A widget: the app's card as the R1 draws cards (host/…/shell/CardView.kt), or its name and a note when it has none.
 *  A card taller than its place is shrunk (to 0.8 at most) and cut off, as the glance does. */
function WidgetCard({ w, t, accent }: { w: Widget; t: Tokens; accent: string }) {
  const own = w.accent ? colorOf(w.accent) : accent;
  const box = useRef<HTMLDivElement>(null);
  const [scale, setScale] = useState(1);
  useLayoutEffect(() => {
    const el = box.current;
    const room = el?.parentElement?.clientHeight ?? 0;
    if (el && room) setScale(Math.max(0.8, Math.min(1, room / el.offsetHeight)));
  });
  const style: CSSProperties = { borderRadius: 18, background: t.card, textTransform: "none", transform: `scale(${scale})`, transformOrigin: "50% 0" };
  if (!w.card)
    return (
      <div ref={box} style={{ ...style, display: "flex", alignItems: "center", gap: 12, padding: 14 }}>
        <span style={{ color: own, display: "grid" }}>
          <Icon name={w.icon ?? "app-window"} weight="bold" size={22} />
        </span>
        <div style={{ minWidth: 0 }}>
          <div style={text(16, 700, t.fg)}>{w.name}</div>
          {w.note && <div style={text(13, 600, t.muted, one)}>{w.note}</div>}
        </div>
      </div>
    );
  return (
    <div ref={box} style={{ ...style, padding: "12px 14px 14px" }}>
      <CardBody card={w.card} t={t} accent={own} />
    </div>
  );
}

const deg = (v?: number) => (v == null ? "" : `${Math.round(v)}°`);
const one: CSSProperties = { overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" };

/** A card's caption: its title (or place) in the app's accent. */
function Caption({ words, accent }: { words?: string; accent: string }) {
  return words ? <div style={text(13, 700, accent, { ...one, textTransform: "lowercase" })}>{words}</div> : null;
}

function CardBody({ card, t, accent }: { card: Card; t: Tokens; accent: string }) {
  switch (card.type) {
    case "stats": {
      const items = card.items ?? [];
      const rows = Array.from({ length: Math.ceil(items.length / 2) }, (_, i) => items.slice(i * 2, i * 2 + 2));
      return (
        <>
          <Caption words={card.title} accent={accent} />
          {rows.map((pair, i) => (
            <div key={i} style={{ display: "flex", marginTop: 10 }}>
              {pair.map((s, j) => (
                <div key={j} style={{ flex: 1, minWidth: 0 }}>
                  <div style={text(13, 600, t.muted, { ...one, textTransform: "lowercase" })}>{s.label}</div>
                  <div style={text(24, 700, t.fg, one)}>{String(s.value ?? "")}</div>
                  {s.delta && (
                    <div style={text(13, 700, s.trend === "up" ? "#3ddc84" : s.trend === "down" ? t.bad : t.muted, one)}>
                      {s.trend === "up" ? "▲ " : s.trend === "down" ? "▼ " : ""}
                      {s.delta}
                    </div>
                  )}
                </div>
              ))}
              {pair.length === 1 && items.length > 1 && <div style={{ flex: 1 }} />}
            </div>
          ))}
        </>
      );
    }
    case "weather": {
      const range = [card.high != null && `h ${deg(card.high)}`, card.low != null && `l ${deg(card.low)}`, card.feels != null && `feels ${deg(card.feels)}`].filter(Boolean).join(" · ");
      return (
        <>
          <Caption words={card.place} accent={accent} />
          <div style={{ display: "flex", alignItems: "center", marginTop: 6 }}>
            <span style={{ color: accent, display: "grid", marginRight: 12 }}>
              <Icon name={card.icon ?? "sun"} weight="bold" size={54} />
            </span>
            <div style={{ minWidth: 0 }}>
              <div style={text(40, 700, t.fg, { lineHeight: 1.05 })}>
                {deg(card.temp)}
                {card.unit ?? "C"}
              </div>
              <div style={text(16, 600, t.muted, { ...one, textTransform: "lowercase" })}>{card.condition}</div>
            </div>
          </div>
          {range && <div style={text(14, 600, t.muted, { marginTop: 6 })}>{range}</div>}
          {!!card.days?.length && <div style={{ height: 1, background: t.subtle, marginTop: 12 }} />}
          {card.days?.map((d, i) => (
            <div key={i} style={{ display: "flex", alignItems: "center", marginTop: 8 }}>
              <span style={text(15, 600, t.fg, { flex: 1, ...one, textTransform: "lowercase" })}>{d.day}</span>
              <span style={{ display: "grid", marginRight: 14, color: t.fg }}>
                <Icon name={d.icon ?? "sun"} weight="bold" size={20} />
              </span>
              <span style={text(15, 600, t.muted, { width: 38 })}>{deg(d.low)}</span>
              <span style={text(15, 700, t.fg, { width: 38, textAlign: "right" })}>{deg(d.high)}</span>
            </div>
          ))}
        </>
      );
    }
    case "bars": {
      const items = card.items ?? [];
      const max = Math.max(0, ...items.map((b) => Number(b.value) || 0)) || 1;
      return (
        <>
          <Caption words={card.title} accent={accent} />
          {items.map((b, i) => (
            <div key={i} style={{ marginTop: 8 }}>
              <div style={{ display: "flex" }}>
                <span style={text(14, 600, t.fg, { flex: 1, ...one })}>{b.label}</span>
                <span style={text(14, 700, t.muted)}>
                  {String(b.value ?? "")}
                  {card.unit ?? ""}
                </span>
              </div>
              <div style={{ height: 8, marginTop: 4, borderRadius: 4, background: t.focus }}>
                <div style={{ height: 8, borderRadius: 4, background: accent, width: `${(100 * (Number(b.value) || 0)) / max}%` }} />
              </div>
            </div>
          ))}
        </>
      );
    }
    default:
      return (
        <>
          <Caption words={card.title} accent={accent} />
          <div style={text(16, 500, t.fg, { marginTop: card.title ? 8 : 0, lineHeight: 1.12 })}>{card.summary ?? ""}</div>
        </>
      );
  }
}
