// The R1's backgrounds, painted as the R1 paints them (host/…/shell/Backdrop.kt, ui/Noise.kt, ui/Theme.kt): the same
// colours, gradients, glows, grain and flow, on a canvas in the R1's own 360×480 dp, for the look page's small R1.

import { type Background, colorOf } from "../theme";

export type RGB = [number, number, number];

/** The R1's screen, in dp. */
export const W = 360;
export const H = 480;

/** The R1's colours (ui/Theme.kt), dark and light. */
export interface Tokens {
  bg: RGB;
  fg: string;
  muted: string;
  card: string;
  focus: string;
  surface: string;
  subtle: string;
  bad: string;
}
export const DARK: Tokens = { bg: [0, 0, 0], fg: "#f2f2f2", muted: "#8a8f98", card: "#1a1c21", focus: "#15171b", surface: "rgba(24,26,31,.96)", subtle: "#2a2d33", bad: "#ff5a5a" };
export const LIGHT: Tokens = { bg: [0xf3, 0xf2, 0xef], fg: "#16171a", muted: "#6c717a", card: "#ffffff", focus: "#e7e6e2", surface: "rgba(255,255,255,.98)", subtle: "#c9c8c3", bad: "#ff5a5a" };

export function rgb(hex: string): RGB {
  const n = Number.parseInt(hex.slice(1, 7), 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}
export function css([r, g, b]: RGB, a = 1): string {
  return a >= 1 ? `rgb(${r},${g},${b})` : `rgba(${r},${g},${b},${a})`;
}
/** Theme.mix: each channel `c + (to − c)·t`, truncated. */
export function mix(c: RGB, to: RGB, t: number): RGB {
  return [0, 1, 2].map((i) => Math.trunc(c[i] + (to[i] - c[i]) * t)) as RGB;
}
/** Theme.hue: the hue turned by `deg`, saturation and lightness kept. */
export function hue([r, g, b]: RGB, deg: number): RGB {
  const [rf, gf, bf] = [r / 255, g / 255, b / 255];
  const max = Math.max(rf, gf, bf);
  const min = Math.min(rf, gf, bf);
  const d = max - min;
  const l = (max + min) / 2;
  let h = 0;
  let s = 0;
  if (d > 0) {
    s = d / (1 - Math.abs(2 * l - 1));
    h = max === rf ? ((gf - bf) / d) % 6 : max === gf ? (bf - rf) / d + 2 : (rf - gf) / d + 4;
    h *= 60;
  }
  h = (((h + deg) % 360) + 360) % 360;
  const c = (1 - Math.abs(2 * l - 1)) * s;
  const x = c * (1 - Math.abs(((h / 60) % 2) - 1));
  const m = l - c / 2;
  const [r1, g1, b1] = h < 60 ? [c, x, 0] : h < 120 ? [x, c, 0] : h < 180 ? [0, c, x] : h < 240 ? [0, x, c] : h < 300 ? [x, 0, c] : [c, 0, x];
  return [Math.round((r1 + m) * 255), Math.round((g1 + m) * 255), Math.round((b1 + m) * 255)];
}

/** A background with what its kind implies filled in (crate::look's defaults). */
export function normal(bg: Background | undefined): Required<Pick<Background, "kind">> & Background {
  const kind = bg?.kind ?? "plain";
  const grain = { mesh: 0.5, texture: 0.7, gradient: 0.3, tint: 0 }[kind];
  return {
    ...bg,
    kind,
    grain: grain === undefined ? undefined : (bg?.grain ?? grain),
    style: kind === "gradient" ? (bg?.style ?? "glow") : kind === "texture" ? (bg?.style ?? "grain") : undefined,
    dim: kind === "photo" ? (bg?.dim ?? 0.45) : kind === "mesh" ? (bg?.dim ?? 0.4) : undefined,
    flow: kind === "mesh" || kind === "gradient" ? !!bg?.flow : false,
  };
}

interface Glow {
  shader: (ctx: CanvasRenderingContext2D) => CanvasGradient;
  cx: number;
  cy: number;
  weight: number;
  swirl: number;
  x: number;
  y: number;
  vx: number;
  vy: number;
}

// Flow's springs (Backdrop.kt): soft, a little under-damped.
const STIFF = 40;
const DAMP = 2 * 0.45 * Math.sqrt(STIFF);
const STEP = 1 / 240;
const KICK = 700;
const MAX_DRIFT = 180;
const SQUEEZE = 1.4;

/** One R1 screen's background on a canvas: set it up, draw it, and (flow) let the wheel and the button move it. */
export class Painter {
  private ctx: CanvasRenderingContext2D;
  private bg = normal(undefined);
  private t: Tokens = DARK;
  private base: RGB = [0, 0, 0];
  private glows: Glow[] = [];
  private photo: HTMLImageElement | null = null;
  private q = 0;
  private vq = 0;
  private frame = 0;
  private last = 0;

  constructor(private canvas: HTMLCanvasElement) {
    this.ctx = canvas.getContext("2d")!;
  }

  set(bg: Background | undefined, light: boolean, accent: string, photo: HTMLImageElement | null) {
    this.bg = normal(bg);
    this.t = light ? LIGHT : DARK;
    this.photo = photo;
    const BG = this.t.bg;
    const toward = (c: RGB, t: number) => mix(c, BG, t);
    const c = rgb(colorOf(this.bg.color ?? accent));
    this.glows = [];
    this.base = BG;
    const kind = this.bg.kind;
    if (kind === "tint" || kind === "texture") this.base = toward(c, light ? 0.7 : 0.82);
    if (kind === "gradient") {
      const stops = (g: CanvasGradient, s: [RGB, number][]) => {
        for (const [col, at] of s) g.addColorStop(at, css(col));
        return g;
      };
      const shaders: Record<string, (x: CanvasRenderingContext2D) => CanvasGradient> = {
        dusk: (x: CanvasRenderingContext2D) => stops(x.createLinearGradient(0, 0, 0, H), [[mix(hue(c, -30), BG, 0.55), 0], [toward(c, 0.8), 0.55], [BG, 1]]),
        diagonal: (x: CanvasRenderingContext2D) => stops(x.createLinearGradient(0, 0, W, H), [[toward(c, 0.5), 0], [mix(hue(c, 60), BG, 0.78), 0.55], [BG, 1]]),
        horizon: (x: CanvasRenderingContext2D) => stops(x.createLinearGradient(0, H, 0, 0), [[toward(c, 0.45), 0], [toward(c, 0.82), 0.4], [BG, 1]]),
        glow: (x: CanvasRenderingContext2D) => stops(x.createRadialGradient(W / 2, H * 0.12, 0, W / 2, H * 0.12, H * 0.8), [[toward(c, 0.5), 0], [toward(c, 0.8), 0.45], [BG, 1]]),
      };
      const shader = shaders[this.bg.style ?? "glow"] ?? shaders.glow;
      this.glows.push({ shader, cx: W / 2, cy: H / 2, weight: 1, swirl: 0.5, x: 0, y: 0, vx: 0, vy: 0 });
    }
    if (kind === "mesh") {
      const raw = (this.bg.colors?.length ? this.bg.colors : [accent]).map((x) => rgb(colorOf(x)));
      const dim = Math.min(0.85, Math.max(0, this.bg.dim ?? 0.4));
      const cs = (raw.length === 1 ? [raw[0], mix(hue(raw[0], 28), BG, 0.25)] : raw).map((x) => toward(x, dim));
      const avg = cs.reduce((x, y) => mix(x, y, 0.5));
      this.base = toward(avg, 0.32);
      const spots =
        cs.length === 2
          ? [[0.12, 0.06, 1.0, 1, 0.6], [0.92, 0.92, 0.95, 0.7, -0.8]]
          : [[0.1, 0.04, 0.85, 1, 0.6], [0.98, 0.42, 0.78, 0.65, -0.8], [0.28, 1.02, 0.88, 1.3, 0.4]];
      cs.forEach((col, i) => {
        const [fx, fy, fr, weight, swirl] = spots[i];
        const cx = W * fx;
        const cy = H * fy;
        const shader = (x: CanvasRenderingContext2D) => {
          const g = x.createRadialGradient(cx, cy, 0, cx, cy, H * fr);
          g.addColorStop(0, css(col));
          g.addColorStop(0.45, css(col, 0.55));
          g.addColorStop(1, css(col, 0));
          return g;
        };
        this.glows.push({ shader, cx, cy, weight, swirl, x: 0, y: 0, vx: 0, vy: 0 });
      });
    }
    this.draw();
  }

  /** Paints it, sized to the canvas (which is the screen at whatever size it's shown). */
  draw() {
    const { ctx, canvas } = this;
    const k = canvas.width / W;
    ctx.setTransform(k, 0, 0, k, 0, 0);
    ctx.globalCompositeOperation = "source-over";
    ctx.globalAlpha = 1;
    const kind = this.bg.kind;
    if (kind === "photo") {
      ctx.fillStyle = css(this.t.bg);
      ctx.fillRect(0, 0, W, H);
      const p = this.photo;
      if (p?.naturalWidth) {
        const s = Math.max(W / p.naturalWidth, H / p.naturalHeight);
        ctx.imageSmoothingEnabled = true;
        ctx.imageSmoothingQuality = "high";
        ctx.drawImage(p, (W - p.naturalWidth * s) / 2, (H - p.naturalHeight * s) / 2, p.naturalWidth * s, p.naturalHeight * s);
      }
      ctx.fillStyle = css(this.t.bg, Math.min(0.85, Math.max(0, this.bg.dim ?? 0.45)));
      ctx.fillRect(0, 0, W, H);
      return;
    }
    ctx.fillStyle = css(this.base);
    ctx.fillRect(0, 0, W, H);
    const s = 1 + 0.1 * this.q;
    for (const g of this.glows) {
      ctx.save();
      ctx.translate(g.x + g.cx, g.y + g.cy);
      ctx.scale(s, s);
      ctx.translate(-g.cx, -g.cy);
      ctx.fillStyle = g.shader(ctx);
      ctx.fillRect(-W * 2, -H * 2, W * 5, H * 5);
      ctx.restore();
    }
    const amount = this.bg.grain ?? 0;
    if (kind !== "plain" && amount > 0) {
      // Over the screen in its own pixels: soft light, as strong as the grain is.
      const noise = this.bg.style === "noise";
      const tile = grainTile(noise);
      ctx.setTransform(1, 0, 0, 1, 0, 0);
      ctx.imageSmoothingEnabled = noise;
      const pattern = ctx.createPattern(tile, "repeat")!;
      if (noise) pattern.setTransform(new DOMMatrix().scale(2));
      // The R1's grain is finer than this canvas's pixels (1.331 of them a dp): shown a pixel a grain, it's as strong as
      // the R1's grains would be blended together at this size (though never so faint it can't be seen).
      const fine = Math.max(0.35, Math.min(1, canvas.width / W / 1.331));
      ctx.globalCompositeOperation = "soft-light";
      ctx.globalAlpha = amount * fine;
      ctx.fillStyle = pattern;
      ctx.fillRect(0, 0, canvas.width, canvas.height);
      ctx.globalCompositeOperation = "source-over";
      ctx.globalAlpha = 1;
    }
  }

  get flows() {
    return !!this.bg.flow && this.glows.length > 0;
  }

  /** A wheel tick: the glows drift with it, each its own way (Backdrop.nudge). */
  nudge(delta: number) {
    if (!this.flows) return;
    const d = Math.sign(delta);
    for (const g of this.glows) {
      g.vy -= d * KICK * g.weight;
      g.vx += d * KICK * 0.8 * g.swirl;
    }
    this.vq += SQUEEZE * 0.35;
    this.start();
  }

  /** The button: the glows draw in while it's down and spring out when it's let go. */
  press(down: boolean) {
    if (!this.flows) return;
    this.vq += down ? -SQUEEZE : SQUEEZE * 0.8;
    this.start();
  }

  private start() {
    if (this.frame) return;
    this.last = 0;
    this.frame = requestAnimationFrame(this.tick);
  }

  private tick = (now: number) => {
    const dt = this.last ? Math.min(1 / 20, Math.max(0, (now - this.last) / 1000)) : 1 / 60;
    this.last = now;
    for (let left = dt; left > 0; left -= STEP) {
      const step = Math.min(STEP, left);
      for (const g of this.glows) {
        g.vx += (-STIFF * g.x - DAMP * g.vx) * step;
        g.vy += (-STIFF * g.y - DAMP * g.vy) * step;
        g.x = Math.max(-MAX_DRIFT, Math.min(MAX_DRIFT, g.x + g.vx * step));
        g.y = Math.max(-MAX_DRIFT, Math.min(MAX_DRIFT, g.y + g.vy * step));
      }
      this.vq += (-STIFF * this.q - DAMP * this.vq) * step;
      this.q += this.vq * step;
    }
    const resting = this.glows.every((g) => Math.abs(g.x) < 0.3 && Math.abs(g.y) < 0.3 && Math.abs(g.vx) < 2 && Math.abs(g.vy) < 2) && Math.abs(this.q) < 0.002 && Math.abs(this.vq) < 0.01;
    if (resting) {
      for (const g of this.glows) Object.assign(g, { x: 0, y: 0, vx: 0, vy: 0 });
      this.q = 0;
      this.vq = 0;
    }
    this.draw();
    this.frame = resting ? 0 : requestAnimationFrame(this.tick);
  };

  stop() {
    cancelAnimationFrame(this.frame);
    this.frame = 0;
  }
}

/** Noise.kt's tiles: each pixel white or black, `t = U + U − 1`, alpha `|t|^1.35 · amp` (grain 256 at 190; noise 160 at
 *  230, drawn twice as big and smoothed). Seeded, so it's the same every time. */
const tiles = new Map<boolean, HTMLCanvasElement>();
function grainTile(noise: boolean): HTMLCanvasElement {
  const had = tiles.get(noise);
  if (had) return had;
  const n = noise ? 160 : 256;
  const amp = noise ? 230 : 190;
  const c = document.createElement("canvas");
  c.width = n;
  c.height = n;
  const x = c.getContext("2d")!;
  const img = x.createImageData(n, n);
  let seed = noise ? 0x9e3779b9 : 0x2545f491;
  const rnd = () => {
    seed ^= seed << 13;
    seed ^= seed >>> 17;
    seed ^= seed << 5;
    return (seed >>> 0) / 4294967296;
  };
  for (let i = 0; i < n * n; i++) {
    const t = rnd() + rnd() - 1;
    const v = t > 0 ? 255 : 0;
    img.data.set([v, v, v, Math.round(Math.abs(t) ** 1.35 * amp)], i * 4);
  }
  x.putImageData(img, 0, 0);
  tiles.set(noise, c);
  return c;
}
