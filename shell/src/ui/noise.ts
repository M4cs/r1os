// Grain and noise textures (the characters' finishes, the frosted home bar). Each is made once on a
// small canvas and reused as an image, so nothing is computed per frame: SVG turbulence filters
// would be re-run on every repaint of an animating character, which the R1's A53 can't afford.

export type NoiseKind = "grain" | "noise" | "frost";

const SPEC: Record<NoiseKind, { size: number; seed: number; alpha: (v: number) => number; light?: boolean }> = {
  // Fine film grain: soft light and dark specks.
  grain: { size: 96, seed: 7, alpha: (v) => v * 64 },
  // Coarser and stronger: drawn scaled up, pixelated.
  noise: { size: 64, seed: 13, alpha: (v) => v ** 1.7 * 105 },
  // Frosted glass: light specks only, faint, and drawn at half a CSS pixel each (screens.css
  // .home-bar), so they blend into a soft frost rather than salt-and-pepper.
  frost: { size: 128, seed: 29, alpha: (v) => v ** 2 * 16, light: true },
};

const cache = new Map<NoiseKind, string>();

/** A PNG data URL of `kind`'s texture: transparent, with white and black specks. */
export function noiseUrl(kind: NoiseKind): string {
  const hit = cache.get(kind);
  if (hit !== undefined) return hit;
  const { size, seed, alpha, light } = SPEC[kind];
  let url = "";
  const canvas = document.createElement("canvas");
  canvas.width = canvas.height = size;
  const ctx = canvas.getContext("2d");
  if (ctx) {
    const img = ctx.createImageData(size, size);
    // A fixed-seed generator, so a texture looks the same on every load.
    let s = seed;
    const rand = () => {
      s = (Math.imul(s, 1664525) + 1013904223) >>> 0;
      return s / 4294967296;
    };
    for (let i = 0; i < size * size; i++) {
      const v = rand();
      const shade = light || v > 0.5 ? 255 : 0;
      img.data[i * 4] = img.data[i * 4 + 1] = img.data[i * 4 + 2] = shade;
      img.data[i * 4 + 3] = Math.round(alpha(Math.abs(v - 0.5) * 2));
    }
    ctx.putImageData(img, 0, 0);
    url = canvas.toDataURL("image/png");
  }
  cache.set(kind, url);
  return url;
}

/** The size a texture is drawn at, in the character's 200-unit view box. */
export function noiseTile(kind: NoiseKind): number {
  return kind === "noise" ? SPEC.noise.size * 2 : SPEC[kind].size;
}
