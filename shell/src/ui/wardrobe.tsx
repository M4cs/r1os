// What agents look like and wear, drawn from one catalog (wardrobe/wardrobe.json, T-062).
//
// The catalog holds each form's body, its head and where things sit on it (pins), every kind of eyes, and every
// thing to wear, as layers of vector shapes. This file draws it as SVG; the R1's ui/Wardrobe.kt draws the same file
// on a Canvas, and r1d checks agents' eyes and wear against it. docs/specs/experience.md §3.1 has the format.

import type { ReactElement, ReactNode } from "react";
import data from "./wardrobe/wardrobe.json";

type N = number;
/** x, y, turn in degrees, scale: a frame in the form's 200-unit box. */
type Pin = [N, N, N, N];
type Stop = string | [N, string];
export type Paint = string | { lin: [N, N, N, N]; stops: Stop[] } | { rad: [N, N, N]; stops: Stop[] };
type FormShape = "head" | "body" | "ears";

/** One layer: a shape and its paint, or a group of layers, placed in its parent's frame or at a pin. */
export interface Layer {
  d?: string;
  circle?: [N, N, N];
  ellipse?: [N, N, N, N];
  rect?: [N, N, N, N, N?];
  /** The form's own outline, wherever the layer is (in the form's box). */
  shape?: FormShape;
  g?: Layer[];
  /** A pin's frame instead of the parent's. */
  at?: string;
  t?: [N, N];
  r?: N;
  s?: N | [N, N];
  /** The group again, mirrored about x = 0. */
  mirror?: boolean;
  /** The group at each of the eyes' spots (the right one mirrored); `side` keeps one, `pair` needs two. */
  each?: "eye";
  side?: "L" | "R";
  pair?: boolean;
  /** In `each`: on a single centred eye, drawn mirrored too (a temple on either side of one lens). */
  outer?: boolean;
  /** In `each`: not mirrored on the right eye (a catchlight from the same side on both). */
  fixed?: boolean;
  /** Only on these forms, or on all but these. */
  forms?: string[];
  except?: string[];
  /** Only inside these: a form shape (`!` for outside it), or a shape in the layer's frame. */
  clip?: string | Layer | (string | Layer)[];
  fill?: Paint;
  stroke?: Paint;
  width?: N;
  dash?: [N, N];
  cap?: "butt" | "round" | "square";
  join?: "miter" | "round" | "bevel";
  opacity?: N;
  /** The outline pushed out by this much (a stroke in the fill's paint): wear that hugs a form. */
  grow?: N;
}

export interface FormFit {
  label: string;
  body: string;
  /** The head that hats hug (its body without ears); the body when unset. */
  head?: string;
  /** Ears drawn again over hats that ask for it. */
  ears?: string;
  pins: Record<string, Pin | Pin[]>;
}

export interface EyeStyle {
  id: string;
  label: string;
  /** False: already half shut, so it doesn't blink. */
  blink?: boolean;
  /** A reaction's face, not offered (ADR-0043). */
  hidden?: boolean;
  /** Where its eyes are in the face's frame: x, y, size. What's worn on the face sits on them. */
  spots?: [N, N, N][];
  layers: Layer[];
}

export interface Item {
  id: string;
  slot: string;
  label: string;
  layers?: Layer[];
  /** Drawn behind the body (wings, a cape, the back of a crown). */
  behind?: Layer[];
  /** The form's ears drawn again over it. */
  ears?: boolean;
}

export interface Slot {
  id: string;
  label: string;
  /** Where its items sit unless a layer says otherwise. */
  pin: string;
}

interface Wardrobe {
  colors: Record<string, string>;
  forms: Record<string, FormFit>;
  slots: Slot[];
  eyes: EyeStyle[];
  items: Item[];
}

export const WARDROBE = data as unknown as Wardrobe;
export const SLOTS = WARDROBE.slots;
/** The eyes you can pick, in order. */
export const EYE_STYLES = WARDROBE.eyes.filter((e) => !e.hidden);
export const ITEMS = WARDROBE.items;
const EYES_BY_ID = new Map(WARDROBE.eyes.map((e) => [e.id, e]));
const ITEMS_BY_ID = new Map(ITEMS.map((i) => [i.id, i]));

export const eyeStyle = (id: string | undefined) => EYES_BY_ID.get(id ?? "") ?? EYES_BY_ID.get("classic")!;
export const itemOf = (id: string) => ITEMS_BY_ID.get(id);
export const itemsIn = (slot: string) => ITEMS.filter((i) => i.slot === slot);
/** The worn items, one per slot (the first of each), in the order they're drawn. */
export function wornOf(wear: string[] | undefined): Item[] {
  return SLOTS.map((s) => wear?.map(itemOf).find((i) => i?.slot === s.id)).filter((i): i is Item => !!i);
}

// ---- colours -----------------------------------------------------------------------------------

const ch = (h: string, i: number) => parseInt(h.slice(1 + i * 2, 3 + i * 2), 16);

/** `hex` moved toward `to` by `t` (0–1), as an opaque hex colour. */
export function mix(hex: string, to: string, t: number): string {
  return `#${[0, 1, 2]
    .map((i) => Math.round(ch(hex, i) + (ch(to, i) - ch(hex, i)) * t).toString(16).padStart(2, "0"))
    .join("")}`;
}

/** `hex` with its hue turned by `deg`, keeping saturation and lightness. */
export function hue(hex: string, deg: number): string {
  const [r, g, b] = [0, 1, 2].map((i) => ch(hex, i) / 255) as [number, number, number];
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const l = (max + min) / 2;
  const d = max - min;
  const s = d === 0 ? 0 : d / (1 - Math.abs(2 * l - 1));
  let h = d === 0 ? 0 : max === r ? ((g - b) / d) % 6 : max === g ? (b - r) / d + 2 : (r - g) / d + 4;
  h = (((h * 60 + deg) % 360) + 360) % 360;
  const c = (1 - Math.abs(2 * l - 1)) * s;
  const x = c * (1 - Math.abs(((h / 60) % 2) - 1));
  const m = l - c / 2;
  const [r1, g1, b1] = h < 60 ? [c, x, 0] : h < 120 ? [x, c, 0] : h < 180 ? [0, c, x] : h < 240 ? [0, x, c] : h < 300 ? [x, 0, c] : [c, 0, x];
  return `#${[r1, g1, b1].map((v) => Math.round((v + m) * 255).toString(16).padStart(2, "0")).join("")}`;
}

/** The colours a look draws with: the agent's own (`c`), its opposite (`tone`) and the eyes' ink. */
export interface Inks {
  c: string;
  tone: string;
  ink: string;
}

export function inksOf(c: string, form: string): Inks {
  return { c, tone: mix(hue(c, 160), "#000000", 0.12), ink: form === "bot" ? c : WARDROBE.colors.ink! };
}

/**
 * A catalog colour: `#rrggbb`, `c`, `tone`, `ink` or a named colour, then any of `+N` (N% toward white), `-N`
 * (toward black) and `~N` (the hue turned N degrees), in order: `gold-30`, `c~120+10`.
 */
export function colorSpec(spec: string, inks: Inks): string {
  const m = /^(#[0-9a-fA-F]{6}|[a-z]+)(.*)$/.exec(spec);
  if (!m) return "#ff00ff";
  const base = m[1]!;
  let out = base.startsWith("#") ? base.toLowerCase() : base === "c" ? inks.c : base === "tone" ? inks.tone : base === "ink" ? inks.ink : (WARDROBE.colors[base] ?? "#ff00ff");
  for (const [, op, n] of m[2]!.matchAll(/([+\-~])(-?[\d.]+)/g)) {
    const v = Number(n);
    out = op === "+" ? mix(out, "#ffffff", v / 100) : op === "-" ? mix(out, "#000000", v / 100) : hue(out, v);
  }
  return out;
}

// ---- frames --------------------------------------------------------------------------------------

/** An affine matrix as SVG's [a b c d e f]: x' = a x + c y + e, y' = b x + d y + f. */
type M = [N, N, N, N, N, N];
const ID: M = [1, 0, 0, 1, 0, 0];
const mul = (m: M, n: M): M => [
  m[0] * n[0] + m[2] * n[1],
  m[1] * n[0] + m[3] * n[1],
  m[0] * n[2] + m[2] * n[3],
  m[1] * n[2] + m[3] * n[3],
  m[0] * n[4] + m[2] * n[5] + m[4],
  m[1] * n[4] + m[3] * n[5] + m[5],
];
const inv = (m: M): M => {
  const det = m[0] * m[3] - m[1] * m[2];
  return [m[3] / det, -m[1] / det, -m[2] / det, m[0] / det, (m[2] * m[5] - m[3] * m[4]) / det, (m[1] * m[4] - m[0] * m[5]) / det];
};
const move = (x: N, y: N): M => [1, 0, 0, 1, x, y];
const turn = (deg: N): M => {
  const a = (deg * Math.PI) / 180;
  return [Math.cos(a), Math.sin(a), -Math.sin(a), Math.cos(a), 0, 0];
};
const size = (sx: N, sy: N = sx): M => [sx, 0, 0, sy, 0, 0];
const pinMatrix = (p: Pin): M => mul(mul(move(p[0], p[1]), turn(p[2])), size(p[3]));
const isId = (m: M) => m.every((v, i) => Math.abs(v - ID[i]!) < 1e-9);
const svgMatrix = (m: M) => `matrix(${m.map((v) => +v.toFixed(5)).join(" ")})`;
const scaleOf = (m: M) => Math.sqrt(Math.abs(m[0] * m[3] - m[1] * m[2]));

/** The layer's own frame in its parent's: at a pin, then moved, turned and scaled. */
function frameOf(l: Layer, parent: M, form: FormFit): M {
  let m = parent;
  if (l.at) {
    const p = form.pins[l.at];
    if (p && typeof p[0] === "number") m = pinMatrix(p as Pin);
  }
  if (l.t) m = mul(m, move(l.t[0], l.t[1]));
  if (l.r) m = mul(m, turn(l.r));
  if (l.s !== undefined) m = mul(m, typeof l.s === "number" ? size(l.s) : size(l.s[0], l.s[1]));
  return m;
}

// ---- drawing -------------------------------------------------------------------------------------

export interface Look {
  formId: string;
  form: FormFit;
  inks: Inks;
  /** The worn eyes' spots, for what sits on them. */
  spots: [N, N, N][];
  /** Ids unique to this drawing (gradients, clips). */
  uid: string;
}

const DEFAULT_SPOTS: [N, N, N][] = [
  [-22, 0, 1],
  [22, 0, 1],
];
export const spotsOf = (e: EyeStyle) => e.spots ?? DEFAULT_SPOTS;

function formPath(form: FormFit, s: FormShape): string {
  return s === "head" ? (form.head ?? form.body) : s === "ears" ? (form.ears ?? "") : form.body;
}

/** Draws layers into SVG, as ui/Wardrobe.kt draws them on a Canvas. */
export function drawLayers(layers: Layer[] | undefined, look: Look, frame: M, key: string): ReactNode {
  if (!layers?.length) return null;
  return layers.map((l, i) => drawLayer(l, look, frame, `${key}.${i}`));
}

/**
 * One layer, its placement composed onto `base`. `ctx` is the frame of the SVG element it goes in (the same, except
 * where a pair of pins moves the base without a group of its own).
 */
function drawLayer(l: Layer, look: Look, base: M, key: string, ctx: M = base): ReactNode {
  if ((l.forms && !l.forms.includes(look.formId)) || l.except?.includes(look.formId)) return null;
  if (l.each === "eye") {
    const spots = look.spots;
    if (l.pair && spots.length < 2) return null;
    const out: ReactNode[] = [];
    spots.forEach(([x, y, s], i) => {
      const right = x > 0;
      if ((l.side === "R" && x < 0) || (l.side === "L" && right)) return;
      const at = mul(frameOf({ ...l, each: undefined, mirror: undefined }, base, look.form), mul(move(x, y), size(right ? -s : s, s)));
      const kids = l.g ?? [];
      const flip = mul(at, size(-1, 1));
      const kid = (k: Layer, j: number) =>
        right && k.fixed ? group(flip, at, drawLayer(k, look, flip, `${key}.e${i}.${j}`), `${key}.e${i}.f${j}`) : drawLayer(k, look, at, `${key}.e${i}.${j}`);
      out.push(group(at, ctx, kids.map(kid), `${key}.e${i}`));
      // One eye in the middle: what's on its outer side goes on both sides of it.
      const outer = x === 0 ? kids.filter((k) => k.outer) : [];
      if (outer.length) out.push(group(flip, ctx, outer.map((k, j) => drawLayer(k, look, flip, `${key}.o${i}.${j}`)), `${key}.o${i}`));
    });
    return out;
  }
  // A pair of pins (its sides, its cheeks): once at each, the right one mirrored.
  const pair = l.at ? look.form.pins[l.at] : undefined;
  if (pair && typeof pair[0] !== "number") {
    return (pair as Pin[]).map((p, i) => drawLayer({ ...l, at: undefined }, look, mul(pinMatrix(p), size(i ? -1 : 1, 1)), `${key}.p${i}`, ctx));
  }
  const m = frameOf(l, base, look.form);
  const copies = l.mirror ? [m, mul(m, size(-1, 1))] : [m];
  return copies.map((f, ci) => {
    const k = `${key}${ci ? "m" : ""}`;
    const body = l.g ? l.g.map((c, j) => drawLayer(c, look, f, `${k}.${j}`)) : shapeOf(l, look, f, k);
    const inner = l.opacity !== undefined && l.g ? <g opacity={l.opacity}>{body}</g> : body;
    return group(f, ctx, clipped(l.clip, look, f, k, inner), k);
  });
}

function group(m: M, parent: M, content: ReactNode, key: string): ReactElement {
  const rel = mul(inv(parent), m);
  return isId(rel) ? <g key={key}>{content}</g> : <g key={key} transform={svgMatrix(rel)}>{content}</g>;
}

/** The content, inside each of the clips (nested), in the layer's frame. */
function clipped(clip: Layer["clip"], look: Look, m: M, key: string, content: ReactNode): ReactNode {
  if (!clip) return content;
  const list = Array.isArray(clip) ? clip : [clip];
  return list.reduceRight<ReactNode>((inner, c, i) => {
    const id = `${look.uid}-${key}-k${i}`.replace(/[^\w-]/g, "_");
    if (typeof c === "string") {
      // A form's outline: `head`, `!head` (outside it), `head+3` (pushed out by 3).
      const [, out, name, grow] = /^(!?)(head|body|ears)(?:\+([\d.]+))?$/.exec(c) ?? [];
      const d = formPath(look.form, (name ?? "body") as FormShape);
      const tf = svgMatrix(inv(m));
      if (out || grow) {
        // A mask: a clip can't be pushed out, and an even-odd one would open holes where a form's parts overlap.
        const w = grow ? Number(grow) * 2 * scaleOf(m) : undefined;
        const ink = out ? "#000" : "#fff";
        return (
          <g>
            <mask id={id} maskUnits="userSpaceOnUse" maskContentUnits="userSpaceOnUse" x="-2000" y="-2000" width="4000" height="4000">
              {out && <rect x="-2000" y="-2000" width="4000" height="4000" fill="#fff" />}
              <path d={d} transform={tf} fill={ink} stroke={w ? ink : undefined} strokeWidth={w} strokeLinejoin="round" />
            </mask>
            <g mask={`url(#${id})`}>{inner}</g>
          </g>
        );
      }
      return (
        <g>
          <clipPath id={id} clipPathUnits="userSpaceOnUse">
            <path d={d} transform={tf} />
          </clipPath>
          <g clipPath={`url(#${id})`}>{inner}</g>
        </g>
      );
    }
    // Its own placement, in the layer's frame (or at a pin).
    const local = c.at ? mul(inv(m), frameOf(c, ID, look.form)) : frameOf(c, ID, look.form);
    return (
      <g>
        <clipPath id={id} clipPathUnits="userSpaceOnUse">
          {primitive(c, look, local, {})}
        </clipPath>
        <g clipPath={`url(#${id})`}>{inner}</g>
      </g>
    );
  }, content);
}

/** The shape's element with these attributes, in the frame (`m`, for a form's outline). */
function primitive(l: Layer, look: Look, m: M, attrs: Record<string, unknown>): ReactElement | null {
  const tf = isId(m) ? undefined : svgMatrix(m);
  if (l.shape) return <path d={formPath(look.form, l.shape)} transform={tf} {...attrs} />;
  if (l.d) return <path d={l.d} transform={tf} {...attrs} />;
  if (l.circle) return <circle cx={l.circle[0]} cy={l.circle[1]} r={l.circle[2]} transform={tf} {...attrs} />;
  if (l.ellipse) return <ellipse cx={l.ellipse[0]} cy={l.ellipse[1]} rx={l.ellipse[2]} ry={l.ellipse[3]} transform={tf} {...attrs} />;
  if (l.rect) {
    const [x, y, w, h, r = 0] = l.rect;
    return <rect x={x} y={y} width={w} height={h} rx={r} transform={tf} {...attrs} />;
  }
  return null;
}

/** A shape, filled and stroked. A form's outline is drawn in the form's box; its paint stays in the layer's frame. */
function shapeOf(l: Layer, look: Look, m: M, key: string): ReactNode {
  const defs: ReactElement[] = [];
  // A form's outline is drawn back in the form's box; gradients and widths are kept in the layer's frame.
  const back = l.shape ? inv(m) : ID;
  const k = l.shape ? scaleOf(m) : 1;
  const paint = (p: Paint | undefined, which: string): string | undefined => {
    if (p === undefined) return undefined;
    if (typeof p === "string") return p === "none" ? "none" : colorSpec(p, look.inks);
    const id = `${look.uid}-${key}-${which}`.replace(/[^\w-]/g, "_");
    const n = p.stops.length;
    const stops = p.stops.map((s, i) => {
      const [at, col] = typeof s === "string" ? [n === 1 ? 0 : i / (n - 1), s] : s;
      return <stop key={i} offset={at} stopColor={colorSpec(col, look.inks)} />;
    });
    const gt = l.shape ? svgMatrix(m) : undefined;
    defs.push(
      "lin" in p ? (
        <linearGradient key={id} id={id} gradientUnits="userSpaceOnUse" gradientTransform={gt} x1={p.lin[0]} y1={p.lin[1]} x2={p.lin[2]} y2={p.lin[3]}>
          {stops}
        </linearGradient>
      ) : (
        <radialGradient key={id} id={id} gradientUnits="userSpaceOnUse" gradientTransform={gt} cx={p.rad[0]} cy={p.rad[1]} r={p.rad[2]}>
          {stops}
        </radialGradient>
      ),
    );
    return `url(#${id})`;
  };
  const fill = paint(l.fill, "f") ?? "none";
  let stroke = paint(l.stroke, "s");
  let width = l.width;
  if (l.grow && fill !== "none") {
    stroke = fill;
    width = l.grow * 2;
  }
  const el = primitive(l, look, back, {
    fill,
    stroke,
    strokeWidth: width !== undefined ? width * k : undefined,
    strokeLinecap: stroke ? (l.cap ?? "round") : undefined,
    strokeLinejoin: stroke ? (l.join ?? "round") : undefined,
    strokeDasharray: l.dash ? l.dash.map((v) => v * k).join(" ") : undefined,
    opacity: l.opacity,
  });
  return (
    <g key={key}>
      {defs}
      {el}
    </g>
  );
}

/** A pin's frame on a form (the form's box when it has no such pin). */
export function pinOf(form: FormFit, name: string): M {
  const p = form.pins[name];
  return p && typeof p[0] === "number" ? pinMatrix(p as Pin) : ID;
}

/** A pin's point in the form's box. */
export function pinPoint(form: FormFit, name: string): [N, N] {
  const m = pinOf(form, name);
  return [m[4], m[5]];
}

/** Layers at a pin, in an SVG drawn in the form's box. */
export function drawAt(layers: Layer[] | undefined, look: Look, pin: string, key: string): ReactNode {
  if (!layers?.length) return null;
  const m = pinOf(look.form, pin);
  return (
    <g key={key} transform={svgMatrix(m)}>
      {drawLayers(layers, look, m, key)}
    </g>
  );
}

/** The pin a slot's items sit at. */
export const slotPin = (slot: string) => SLOTS.find((s) => s.id === slot)?.pin ?? "face";
