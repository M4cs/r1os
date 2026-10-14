// The R1's look, on the computer (docs/specs/companion.md §5): the same accents, mode and backgrounds as the R1's
// own (ADR-0043), applied as CSS variables, and following the R1 as it changes.

/** The R1's named accents (host/…/ui/Theme.kt COLORS). */
export const ACCENTS: Record<string, string> = {
  orange: "#ff8a3d", amber: "#f5b83d", yellow: "#f0d04a", lime: "#a3d65c",
  green: "#3ddc84", teal: "#2fc4a8", cyan: "#3cc8e0", blue: "#4ea1ff",
  indigo: "#8b8cf8", purple: "#b07cf6", pink: "#f06aa8", red: "#f2555a",
};

/** The soft gradients' palettes (crate::look PALETTES). */
export const PALETTES: Record<string, string[]> = {
  sunset: ["#ff8a3d", "#f06aa8", "#6b4cff"], lagoon: ["#2fc4a8", "#3cc8e0", "#4e6bff"],
  aurora: ["#3ddc84", "#8b8cf8", "#1d4ed8"], peach: ["#ffb38a", "#ff6f91", "#ffc75f"],
  dusk: ["#4ea1ff", "#b07cf6", "#f06aa8"], forest: ["#a3d65c", "#2f8f5b", "#f5b83d"],
  berry: ["#b07cf6", "#f2555a", "#ff8a3d"], ink: ["#3a4a6b", "#7a5c99", "#1f2a44"],
};

export function colorOf(name?: string | null): string {
  if (!name) return ACCENTS.orange;
  if (/^#[0-9a-f]{6}$/i.test(name)) return name;
  return ACCENTS[name] ?? ACCENTS.orange;
}

export interface Look {
  mode?: "dark" | "light";
  accent?: string;
  font?: string;
  home?: { background?: Background };
  glance?: {
    background?: Background;
    /** The clock's face (crate::look CLOCKS). */
    clock?: string;
    align?: "left" | "center";
    /** Where the agent peeks up: center, left, right, hidden. */
    agent?: string;
    /** What the glance screen leaves out (crate::look PARTS). */
    hide?: string[];
  };
}

/** A change to the look, as `PUT /api/look` takes it: only what changes, `null` putting a value back. */
export type LookPatch = { [k: string]: LookPatch | string | number | boolean | string[] | null };

/** A change merged into a look as r1d merges it (crate::look::merge): objects merged, `null` removed. */
export function mergeLook<T extends object>(base: T, patch: LookPatch): T {
  const out: Record<string, unknown> = { ...(base as Record<string, unknown>) };
  for (const [k, v] of Object.entries(patch)) {
    if (v === null) delete out[k];
    else if (typeof v === "object" && !Array.isArray(v) && out[k] && typeof out[k] === "object" && !Array.isArray(out[k])) out[k] = mergeLook(out[k] as object, v);
    else out[k] = v;
  }
  return out as T;
}

/** Two changes as one (the later wins; its `null`s kept, to be sent). */
export function mergePatch(a: LookPatch, b: LookPatch): LookPatch {
  const out: LookPatch = { ...a };
  for (const [k, v] of Object.entries(b)) {
    const had = out[k];
    out[k] = v && typeof v === "object" && !Array.isArray(v) && had && typeof had === "object" && !Array.isArray(had) ? mergePatch(had, v) : v;
  }
  return out;
}

export interface Background {
  kind?: string;
  color?: string;
  colors?: string[];
  style?: string;
  grain?: number;
  palette?: string;
  /** It drifts with the wheel on the R1 (ADR-0043's flow). */
  flow?: boolean;
  /** A photo background's media id. */
  photo?: string;
  /** How much a photo or a soft gradient is darkened (lightened in light mode): 0–0.85. */
  dim?: number;
}

/** A background's colours: a soft gradient's own (or its palette's), else the accent's. */
export function colorsOf(bg: Background | undefined, accent: string): string[] {
  if (bg?.colors?.length) return bg.colors.map(colorOf);
  if (bg?.palette && PALETTES[bg.palette]) return PALETTES[bg.palette];
  if (bg?.color) return [colorOf(bg.color)];
  return [accent];
}

/** Sets the window's colours from the R1's look. */
export function applyLook(look: Look | null) {
  const root = document.documentElement;
  const light = look?.mode === "light";
  const accent = colorOf(look?.accent);
  const v: Record<string, string> = light
    ? { bg: "#f3f2ef", fg: "#16171a", muted: "#6c717a", panel: "#ffffff", panel2: "#e9e8e4", line: "#dcdcd8" }
    : { bg: "#0b0b0c", fg: "#f2f2f2", muted: "#8a8f98", panel: "#141519", panel2: "#1b1c21", line: "#24262c" };
  for (const [k, val] of Object.entries(v)) root.style.setProperty(`--${k}`, val);
  root.style.setProperty("--accent", accent);
  root.style.setProperty("--accent-soft", `${accent}2e`);
  root.dataset.mode = light ? "light" : "dark";
}
