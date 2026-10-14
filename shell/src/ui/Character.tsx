// An agent, drawn (docs/specs/experience.md §2.1 and §3, ADR-0007 §6).
//
// Nine base forms, each an inline SVG, in one of seven finishes (soft light, matte, grain, noise,
// gradients), with one state model shared by all: idle, listening, thinking, waiting, speaking,
// working, sleeping, error. The states are CSS classes (character.css) animating only transforms and
// opacity, so it stays smooth on the R1's A53 WebView. Grain and noise are images made once
// (noise.ts), never per-frame filters. While working, a small badge says what the agent is doing
// (searching, building, remembering…), and an unread count can sit on its left shoulder. Its forms,
// eyes and everything it can wear come from the wardrobe (wardrobe/wardrobe.json, T-062), which the
// R1's CharacterView draws too.

import type { ReactElement } from "react";
import "./character.css";
import { noiseTile, noiseUrl, type NoiseKind } from "./noise";
import { drawAt, EYE_STYLES, eyeStyle, hue, inksOf, mix, pinPoint, slotPin, spotsOf, WARDROBE, wornOf, type Look } from "./wardrobe";

export { EYE_STYLES, ITEMS, itemsIn, SLOTS } from "./wardrobe";

export type Form = "orb" | "blob" | "bot" | "sprite" | "cat" | "ghost" | "bunny" | "bean" | "cloud";
export type Finish = "soft" | "matte" | "grain" | "noise" | "gradient" | "grainy" | "aurora";
export type CharState = "idle" | "listening" | "thinking" | "waiting" | "speaking" | "working" | "sleeping" | "error";
/** What a working agent is doing, from its last tool (activityOf). */
export type Activity = "search" | "build" | "memory" | "files" | "shell" | "apps";

export const FORMS = Object.entries(WARDROBE.forms).map(([id, f]) => ({ id: id as Form, label: f.label }));

export const FINISHES: { id: Finish; label: string }[] = [
  { id: "soft", label: "Soft" },
  { id: "matte", label: "Matte" },
  { id: "grain", label: "Grain" },
  { id: "noise", label: "Noise" },
  { id: "gradient", label: "Gradient" },
  { id: "grainy", label: "Grainy gradient" },
  { id: "aurora", label: "Aurora" },
];

/** The app palette's named accents (packages/runtime styles.css), as body colours. */
export const COLORS: Record<string, string> = {
  orange: "#ff8a3d",
  amber: "#f5b83d",
  yellow: "#f0d04a",
  lime: "#a3d65c",
  green: "#3ddc84",
  teal: "#2fc4a8",
  cyan: "#3cc8e0",
  blue: "#4ea1ff",
  indigo: "#8b8cf8",
  purple: "#b07cf6",
  pink: "#f06aa8",
  red: "#f2555a",
};

/** The eyes you can pick (the wardrobe's, ADR-0043). */
export const EYES = EYE_STYLES.map((e) => e.id);

export function colorOf(name: string | undefined): string {
  // Any `#rrggbb` as itself, as on the R1 (Theme.colorOf).
  if (name && /^#[0-9a-f]{6}$/i.test(name)) return name.toLowerCase();
  return COLORS[name ?? ""] ?? COLORS.orange!;
}

/** The activity a tool stands for (chat `tool` events carry the tool's name). */
export function activityOf(tool: string | undefined): Activity | undefined {
  if (!tool) return undefined;
  if (tool === "WebSearch" || tool === "WebFetch" || tool === "web_search") return "search";
  if (tool === "create_app" || tool === "edit_app" || tool === "undo_app_change") return "build";
  if (tool === "remember" || tool === "list_memories" || tool === "forget") return "memory";
  if (tool === "Bash") return "shell";
  if (["Read", "Write", "Edit", "Glob", "Grep"].includes(tool)) return "files";
  return "apps";
}

// ---- finishes ----------------------------------------------------------------------------------

/** The finish's colour fill (a gradient in <defs>) and its texture, if any. */
function finishOf(f: Finish, c: string): { id: string; gradient: ReactElement; texture?: NoiseKind } {
  const id = `g-${f}-${c.slice(1)}`;
  // Opaque stops only: a translucent white stop drew as a grey-pink smudge on the R1.
  const soft = (
    <radialGradient id={id} cx="38%" cy="32%" r="78%">
      <stop offset="0%" stopColor={mix(c, "#ffffff", 0.55)} />
      <stop offset="32%" stopColor={c} />
      <stop offset="100%" stopColor={mix(c, "#000000", 0.28)} />
    </radialGradient>
  );
  const linear = (stops: string[]) => (
    <linearGradient id={id} x1="0" y1="0" x2="1" y2="1">
      {stops.map((s, i) => (
        <stop key={i} offset={`${(i / (stops.length - 1)) * 100}%`} stopColor={s} />
      ))}
    </linearGradient>
  );
  switch (f) {
    case "matte":
      return {
        id,
        gradient: (
          <radialGradient id={id} cx="40%" cy="35%" r="80%">
            <stop offset="0%" stopColor={mix(c, "#ffffff", 0.12)} />
            <stop offset="100%" stopColor={mix(c, "#000000", 0.12)} />
          </radialGradient>
        ),
      };
    case "grain":
      return { id, gradient: soft, texture: "grain" };
    case "noise":
      return { id, gradient: soft, texture: "noise" };
    case "gradient":
      return { id, gradient: linear([mix(c, "#ffffff", 0.15), c, hue(c, -42)]) };
    case "grainy":
      return { id, gradient: linear([mix(c, "#ffffff", 0.1), c, hue(c, -48)]), texture: "grain" };
    case "aurora":
      return { id, gradient: linear([hue(mix(c, "#ffffff", 0.2), 55), c, hue(c, -70)]) };
    default:
      return { id, gradient: soft };
  }
}

// ---- what a working agent is doing -------------------------------------------------------------

/** Small line icons for the activity badge, drawn around (0, 0) in a ~16-unit box. */
const ICONS: Record<Activity | "waiting", ReactElement> = {
  search: (
    <>
      <circle cx="-2" cy="-2" r="5.5" />
      <path d="M2.2 2.2 L7 7" />
    </>
  ),
  build: (
    <>
      <path d="M-6.5 6.5 L1.5 -1.5" />
      <path d="M-1 -6 L4 -7.5 L7.5 -4 L6 1 Z" className="ch-icon-fill" />
    </>
  ),
  memory: <path d="M0 -8C1 -2 2 -1 8 0C2 1 1 2 0 8C-1 2 -2 1 -8 0C-2 -1 -1 -2 0 -8Z" className="ch-icon-fill" />,
  files: (
    <>
      <path d="M-5 -7.5 H2 L5.5 -4 V7.5 H-5 Z" />
      <path d="M-2 0 H2.5 M-2 3.5 H2.5" />
    </>
  ),
  shell: <path d="M-6 -4 L-1.5 0 L-6 4 M0.5 5 H6.5" />,
  apps: (
    <>
      <rect x="-6.5" y="-6.5" width="5" height="5" rx="1.2" />
      <rect x="1.5" y="-6.5" width="5" height="5" rx="1.2" />
      <rect x="-6.5" y="1.5" width="5" height="5" rx="1.2" />
      <rect x="1.5" y="1.5" width="5" height="5" rx="1.2" />
    </>
  ),
  waiting: <path d="M-5 -7 H5 M-5 7 H5 M-4 -7 C-4 -2 4 2 4 7 M4 -7 C4 -2 -4 2 -4 7" />,
};

export function Character({
  form = "orb",
  color = "orange",
  finish = "soft",
  state = "idle",
  activity,
  unread = 0,
  size = 180,
  eyes = "classic",
  wear,
}: {
  form?: string;
  color?: string;
  finish?: string;
  state?: CharState;
  /** While working: what it's doing (the badge). */
  activity?: Activity;
  /** Replies waiting for the user: a count on its shoulder. */
  unread?: number;
  size?: number;
  /** Its eyes (the wardrobe's; unknown ones are classic). */
  eyes?: string;
  /** What it wears: the first of each slot's items in it are drawn (the wardrobe's). */
  wear?: string[];
}) {
  const c = colorOf(color);
  const f = (form in WARDROBE.forms ? form : "orb") as Form;
  const fit = WARDROBE.forms[f]!;
  const fin = (FINISHES.some((x) => x.id === finish) ? finish : "soft") as Finish;
  const style = eyeStyle(eyes);
  const worn = wornOf(wear);
  const paint = finishOf(fin, c);
  const badge = state === "working" ? (activity ?? "apps") : state === "waiting" ? "waiting" : undefined;
  const tex = paint.texture;
  // Eyes that are already half shut (happy, sleepy) don't blink.
  const shut = style.blink === false;
  const look: Look = {
    formId: f,
    form: fit,
    inks: inksOf(c, f),
    spots: spotsOf(style),
    uid: `w${hash(`${f}${c}${style.id}${worn.map((i) => i.id).join()}`)}`,
  };
  const [, faceY] = pinPoint(fit, "face");
  const [, mouthY] = pinPoint(fit, "mouth");
  const body = (p: { className?: string; fill: string }) => <path d={fit.body} {...p} />;
  const hat = worn.find((i) => i.slot === "head");
  const front = (slot: string) => worn.filter((i) => i.slot === slot).map((i) => drawAt(i.layers, look, slotPin(slot), `${slot}-${i.id}`));
  return (
    <div
      className={`ch ch-${f} st-${state}${badge ? ` act-${badge}` : ""}${shut ? " ch-shut" : ""}`}
      style={{ width: size, height: size, ["--ch" as string]: c, ["--sz" as string]: `${size}px` }}
    >
      <svg viewBox="0 0 200 200" width={size} height={size} aria-hidden>
        <defs>
          {paint.gradient}
          {tex && (
            <pattern id={`t-${tex}`} patternUnits="userSpaceOnUse" width={noiseTile(tex)} height={noiseTile(tex)}>
              <image href={noiseUrl(tex)} width={noiseTile(tex)} height={noiseTile(tex)} className={`ch-tex-${tex}`} />
            </pattern>
          )}
        </defs>
        <circle className="ch-halo" cx="100" cy="100" r="86" />
        <g className="ch-orbit">
          <circle cx="100" cy="12" r="5" />
          <circle cx="188" cy="100" r="4" />
          <circle cx="100" cy="188" r="3" />
        </g>
        <g className="ch-float">
          <g className="ch-body">
            {/* Behind it: wings, a cape, the back of a crown. */}
            {[...worn].reverse().map((i) => drawAt(i.behind, look, slotPin(i.slot), `behind-${i.id}`))}
            {f === "bot" && (
              <>
                <line x1="100" y1="34" x2="100" y2="52" className="ch-antenna" stroke={c} />
                <circle cx="100" cy="30" r="7" className="ch-antenna-tip" fill={c} />
              </>
            )}
            {body({ className: "ch-fill", fill: `url(#${paint.id})` })}
            {tex && body({ className: "ch-texture", fill: `url(#t-${tex})` })}
            {f === "bot" && <rect x="58" y="70" width="84" height="56" rx="18" className="ch-screen" />}
            {/* What it wears (T-062): straps from its back, at its neck and on its cheeks under the eyes, on its face
                over them, a hat on top. */}
            {front("back")}
            {front("neck")}
            {front("cheeks")}
            <g className="ch-eyes" style={{ transformBox: "view-box", transformOrigin: `100px ${faceY}px` }}>
              {drawAt(style.layers, look, "face", "eyes")}
            </g>
            <ellipse className="ch-mouth" cx="100" cy={mouthY} rx="9" ry="5" />
            {front("face")}
            {front("head")}
            {hat?.ears && fit.ears && (
              // Its ears again, over the hat: the body clipped to the ears, outside the head.
              <g>
                <clipPath id={`${look.uid}-ears`}>
                  <path d={fit.ears} />
                </clipPath>
                <mask id={`${look.uid}-noh`} maskUnits="userSpaceOnUse" x="-50" y="-50" width="300" height="300">
                  <rect x="-50" y="-50" width="300" height="300" fill="#fff" />
                  <path d={fit.head ?? fit.body} fill="#000" />
                </mask>
                <g clipPath={`url(#${look.uid}-ears)`} mask={`url(#${look.uid}-noh)`}>
                  {body({ fill: `url(#${paint.id})` })}
                  {tex && body({ className: "ch-texture", fill: `url(#t-${tex})` })}
                </g>
              </g>
            )}
          </g>
        </g>
        {/* Thinking: dots rising like a thought bubble. */}
        <g className="ch-thought">
          <circle cx="146" cy="50" r="4" />
          <circle cx="158" cy="36" r="5.5" />
          <circle cx="172" cy="20" r="7" />
        </g>
        {badge && (
          // Placed by the outer group's attribute; the inner one animates (a CSS transform would
          // replace the attribute).
          <g transform="translate(156 48)">
            <g className="ch-badge">
              <circle r="17" className="ch-badge-bg" />
              <g className="ch-icon">{ICONS[badge]}</g>
            </g>
          </g>
        )}
      </svg>
      {unread > 0 && <span className="ch-unread">{unread > 9 ? "9+" : unread}</span>}
    </div>
  );
}

/** A short, stable id from a string (FNV-1a), for the drawing's gradients and clips. */
function hash(s: string): string {
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) h = Math.imul(h ^ s.charCodeAt(i), 0x01000193);
  return (h >>> 0).toString(36);
}
