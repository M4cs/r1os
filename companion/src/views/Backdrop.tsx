// The R1's backgrounds, drawn with modgrad (a gradient library: mesh, grain, dither, drift; CSS/SVG, no canvas):
// behind the window, the R1's home background, softly animated and leaning toward the pointer; and the look page's
// small R1. Its soft gradients are modgrad meshes; its styled gradients, linear or radial ones.

import { Gradient, type GradientProps } from "modgrad";
import { type Background, type Look, colorOf, colorsOf } from "../theme";

/** Where the blobs sit: the R1's soft gradients glow from the top left, the right and below. */
const PLACES = [
  { x: 8, y: 6, size: 75 },
  { x: 96, y: 28, size: 70 },
  { x: 38, y: 104, size: 80 },
];

/** modgrad's props for one of the R1's backgrounds (ADR-0043's kinds). */
export function gradientOf(bg: Background | undefined, look: Look | null): GradientProps {
  const accent = colorOf(look?.accent);
  const light = look?.mode === "light";
  const base = light ? "#f3f2ef" : "#0b0b0c";
  const grain = 0.06 + 0.3 * (bg?.grain ?? 0.5);
  const common: GradientProps = { theme: light ? "light" : "dark", background: base, grain, grainScale: 140 };
  switch (bg?.kind) {
    case "gradient": {
      const c = colorOf(bg.color ?? look?.accent);
      if (bg.style === "glow") return { ...common, variant: "radial", colors: [c, base] };
      const angle = bg.style === "horizon" ? 180 : bg.style === "dusk" ? 160 : 135;
      return { ...common, variant: "linear", angle, colors: [c, base] };
    }
    case "plain":
      return { ...common, colors: [], grain: 0.05 };
    default: {
      // Soft gradients, tints, textures, photos: their colours (or the accent's) as a mesh.
      const colors = colorsOf(bg, accent);
      return { ...common, variant: "mesh", colors: colors.map((color, i) => ({ color, ...PLACES[i % PLACES.length] })) };
    }
  }
}

/** Behind the whole window. */
export function Backdrop({ look }: { look: Look | null }) {
  const g = gradientOf(look?.home?.background, look);
  return (
    <div className="backdrop" aria-hidden>
      <Gradient {...g} blur={36} animate={{ speed: 0.4 }} interactive opacity={look?.mode === "light" ? 0.6 : 0.55} />
      <div className="backdrop-fade" />
    </div>
  );
}
