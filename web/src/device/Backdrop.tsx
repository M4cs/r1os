// A screen's background, painted as the R1 paints it (companion/src/r1screen/paint.ts): plain, a tint, a styled
// gradient or a soft mesh with grain.

import { useEffect, useRef } from "react";
import type { Background } from "@companion/theme";
import { Painter } from "@companion/r1screen/paint";
import { useScale } from "./R1";

export function Backdrop({ bg, accent, light = false }: { bg: Background; accent: string; light?: boolean }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const k = useScale();
  const key = JSON.stringify(bg);
  useEffect(() => {
    const c = canvas.current!;
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    c.width = Math.max(1, Math.round(360 * k * dpr));
    c.height = Math.max(1, Math.round(480 * k * dpr));
    const p = new Painter(c);
    p.set(bg, light, accent, null);
    return () => p.stop();
  }, [key, accent, light, k]); // eslint-disable-line react-hooks/exhaustive-deps
  return <canvas ref={canvas} className="r1-canvas" />;
}
