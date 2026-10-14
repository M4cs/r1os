// Generated app tiles: every app gets its own gradient and finish, derived from its id (stable across launches) and
// biased toward its `accent`. Colors come from one OKLCH band (similar lightness and chroma, varied hue), so any set of
// tiles looks like it belongs together and white glyphs stay legible.

import type { CSSProperties } from "react";

import type { IconWeight } from "./icons";

export type TileEffect = "gloss" | "mesh" | "glow" | "grain";

export interface AppTile {
  /** Hue of the top-left stop (OKLCH degrees). */
  hue: number;
  effect: TileEffect;
  /** Glyph weight that suits the effect. */
  iconWeight: Extract<IconWeight, "fill" | "duotone">;
  /** Styles for the tile's square (background layers and a soft edge). */
  style: CSSProperties;
}

/** Named accents (compiled-app `app.accent`) → OKLCH hue. */
const ACCENT_HUE: Record<string, number> = {
  red: 25,
  orange: 50,
  amber: 72,
  yellow: 95,
  lime: 125,
  green: 150,
  teal: 185,
  cyan: 210,
  blue: 255,
  indigo: 278,
  purple: 300,
  pink: 345,
};

const EFFECTS: readonly TileEffect[] = ["gloss", "mesh", "glow", "grain"];

/** FNV-1a, 32-bit. */
function hash(s: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return h >>> 0;
}

/** mulberry32: a small, well-mixed PRNG seeded by the hash. */
function rng(seed: number): () => number {
  let a = seed;
  return () => {
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const r2 = (n: number) => Math.round(n * 100) / 100;
const oklch = (l: number, c: number, h: number, alpha = 1) =>
  `oklch(${r2(l)} ${r2(c)} ${Math.round(((h % 360) + 360) % 360)}${alpha < 1 ? ` / ${alpha}` : ""})`;

/** A faint fractal-noise texture (data URI, allowed by the shell's `img-src data:`). */
const GRAIN =
  "url(\"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='96' height='96'%3E%3Cfilter id='n'%3E%3CfeTurbulence type='fractalNoise' baseFrequency='0.9' numOctaves='2' stitchTiles='stitch'/%3E%3CfeColorMatrix values='0 0 0 0 1 0 0 0 0 1 0 0 0 0 1 0 0 0 0.55 0'/%3E%3C/filter%3E%3Crect width='96' height='96' filter='url(%23n)'/%3E%3C/svg%3E\")";

export function appTile(id: string, accent?: string): AppTile {
  const rand = rng(hash(id));
  const accentHue = accent ? ACCENT_HUE[accent] : undefined;
  // Apps that share an accent fan out to either side of it instead of clustering on it.
  const h1 =
    accentHue === undefined ? rand() * 360 : accentHue + (rand() < 0.5 ? -1 : 1) * (8 + rand() * 26);
  let turn = (rand() < 0.5 ? -1 : 1) * (22 + rand() * 26);
  // Never end a gradient in the yellow-green band, which turns olive at the deeper stop.
  const norm = (h: number) => ((h % 360) + 360) % 360;
  if (norm(h1 + turn) > 85 && norm(h1 + turn) < 130 && !(norm(h1) > 70 && norm(h1) < 125)) turn = -turn;
  const h2 = h1 + turn;
  // Lighter top-left stop, deeper bottom-right stop. Yellows stay bright (darkening them reads as brown); the glyph's
  // drop shadow keeps white legible on them.
  const yellowish = Math.abs(((h1 % 360) + 360) % 360 - 95) < 30;
  const l1 = (yellowish ? 0.82 : 0.7) + rand() * 0.05;
  const l2 = (yellowish ? 0.7 : 0.54) + rand() * 0.06;
  const c1 = 0.13 + rand() * 0.05;
  const c2 = 0.16 + rand() * 0.05;
  const angle = 135 + Math.round((rand() - 0.5) * 40);
  const effect = EFFECTS[Math.floor(rand() * EFFECTS.length)] ?? "gloss";
  const base = `linear-gradient(${angle}deg, ${oklch(l1, c1, h1)}, ${oklch(l2, c2, h2)})`;

  const layers: Record<TileEffect, string[]> = {
    gloss: ["radial-gradient(120% 85% at 22% 0%, rgb(255 255 255 / 0.38), transparent 58%)", base],
    mesh: [
      `radial-gradient(75% 75% at 88% 92%, ${oklch(l2 - 0.04, c2 + 0.02, h2 + turn * 0.4, 0.95)}, transparent 72%)`,
      `radial-gradient(65% 65% at 12% 8%, ${oklch(l1 + 0.06, c1, h1 - turn * 0.4, 0.85)}, transparent 70%)`,
      base,
    ],
    glow: [`radial-gradient(90% 70% at 50% 115%, ${oklch(0.86, 0.12, h2, 0.6)}, transparent 70%)`, base],
    grain: [GRAIN, "radial-gradient(110% 80% at 20% 0%, rgb(255 255 255 / 0.22), transparent 60%)", base],
  };

  return {
    hue: Math.round(((h1 % 360) + 360) % 360),
    effect,
    iconWeight: effect === "glow" || effect === "mesh" ? "duotone" : "fill",
    style: {
      backgroundImage: layers[effect].join(", "),
      backgroundSize: effect === "grain" ? "96px 96px, 100% 100%, 100% 100%" : undefined,
      boxShadow: "inset 0 1px 0 rgb(255 255 255 / 0.22), inset 0 -1px 0 rgb(0 0 0 / 0.18)",
    },
  };
}
