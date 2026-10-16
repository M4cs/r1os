// The hero's turning words: a wheel of phrases that springs a detent at a time, as the R1's wheel does. The column
// overshoots a little and settles; while it moves, a vertical blur grows with its speed (an SVG filter fed by the spring's
// velocity), so the words smear like a reel spun past the eye. The phrase on the detent sits in the accent's fill,
// centred and springing to its width; inside the fill an ink copy of the column turns in step, like a picker's lens.
// With a `delay`, its first detent comes that long after it's drawn: it turns in from the phrase before, its fill
// opening from nothing. It tells its line how wide its widest phrase is, so the line can shrink to fit a phone. Its
// rows draw their words from `data-text` (home.css), so the heading's own text is just the sentence a screen reader
// hears, not every phrase three times over.

import { animate, motion, useMotionValue, useMotionValueEvent, useReducedMotion, useTransform, useVelocity, type MotionValue } from "motion/react";
import { useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import { bouncy } from "../site/motion";

export interface ReelItem {
  text: string;
  color: string;
}

const mod = (i: number, n: number) => ((i % n) + n) % n;

/** The fill's padding either side of its phrase, in row heights. */
const PAD = 0.26;

export function Reel({
  items,
  index,
  delay,
  onWidest,
}: {
  items: ReelItem[];
  index: number;
  /** Seconds before its first detent (none: it's on it from the start). */
  delay?: number;
  /** How wide the widest phrase is in its fill, in ems of the line's type. */
  onWidest?: (em: number) => void;
}) {
  const reduce = useReducedMotion();
  const intro = delay !== undefined && !reduce;
  const [born] = useState(() => performance.now());
  /** What's left of the wait for the first detent (s): whatever moves before it waits for it. */
  const wait = () => (intro ? Math.max(0, (delay ?? 0) - (performance.now() - born) / 1000) : 0);
  const pos = useMotionValue(intro ? index - 1 : index);
  const velocity = useVelocity(pos);
  const id = useId().replace(/:/g, "");
  const blur = useRef<SVGFEGaussianBlurElement>(null);
  const layers = useRef<HTMLDivElement[]>([]);
  const measure = useRef<HTMLDivElement>(null);
  const [h, setH] = useState(80);
  const [widths, setWidths] = useState<number[] | null>(null);
  const told = useRef(0);

  // The row's height and each phrase's width, in the font as drawn: at each turn (so a stale reading never lasts), once
  // fonts load, and when the page is resized.
  const read = () => {
    const el = measure.current;
    if (!el) return;
    const spans = Array.from(el.children) as HTMLElement[];
    const next = spans.map((s) => s.getBoundingClientRect().width);
    const row = spans[0]?.getBoundingClientRect().height || 80;
    setWidths((w) => (w && w.length === next.length && w.every((x, i) => Math.abs(x - next[i]!) < 0.5) ? w : next));
    setH(row);
    const em = (Math.max(...next) + 2 * PAD * row) / (parseFloat(getComputedStyle(el).fontSize) || 1);
    if (onWidest && Number.isFinite(em) && Math.abs(em - told.current) > 0.01) {
      told.current = em;
      onWidest(em);
    }
  };
  useLayoutEffect(read, [items, index]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => {
    const el = measure.current;
    if (!el) return;
    const ro = new ResizeObserver(read);
    ro.observe(el);
    window.addEventListener("resize", read);
    document.fonts?.ready.then(read);
    return () => {
      ro.disconnect();
      window.removeEventListener("resize", read);
    };
  }, [items]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    if (reduce) pos.jump(index);
    else animate(pos, index, { ...bouncy, delay: wait() });
  }, [index, reduce, pos]); // eslint-disable-line react-hooks/exhaustive-deps

  // The fill's width, sprung to the phrase on the detent (set at once the first time it's known, unless it opens with
  // the first detent).
  const pad = PAD * h;
  const fillW = useMotionValue(0);
  const fillX = useTransform(fillW, (w) => -w / 2);
  const placed = useRef(false);
  const target = widths ? (widths[mod(index, items.length)] ?? 0) + 2 * pad : 0;
  useEffect(() => {
    if (!target) return;
    if (reduce || (!placed.current && !intro)) fillW.jump(target);
    else animate(fillW, target, { ...bouncy, delay: wait() });
    placed.current = true;
  }, [target, reduce, fillW]); // eslint-disable-line react-hooks/exhaustive-deps

  // Motion blur from speed: rows a second × row height = px a second.
  useMotionValueEvent(velocity, "change", (v) => {
    const px = Math.min(Math.abs(v) * h * 0.022, 16);
    blur.current?.setAttribute("stdDeviation", `0 ${px.toFixed(2)}`);
    const f = px > 0.35 ? `url(#${id})` : "none";
    for (const l of layers.current) if (l && l.style.filter !== f) l.style.filter = f;
  });

  const current = items[mod(index, items.length)]!;
  const near = [-2, -1, 0, 1, 2].map((d) => index + d);
  return (
    <motion.span
      className="reel"
      style={{ ["--h" as string]: `${h}px` }}
      initial={intro ? { opacity: 0 } : false}
      animate={{ opacity: 1 }}
      transition={{ duration: 0.2, delay: delay ?? 0 }}
      aria-hidden
    >
      <svg width="0" height="0" style={{ position: "absolute" }} focusable="false">
        <filter id={id} x="-5%" y="-60%" width="110%" height="220%">
          <feGaussianBlur ref={blur} stdDeviation="0 0" />
        </filter>
      </svg>
      <div className="reel-measure" ref={measure}>
        {items.map((it) => (
          <span key={it.text} data-text={it.text} />
        ))}
      </div>
      <div className="reel-rows" ref={(el) => void (el && (layers.current[0] = el))}>
        {near.map((i) => (
          <Row key={i} i={i} pos={pos} h={h} text={items[mod(i, items.length)]!.text} faded />
        ))}
      </div>
      <motion.span className="reel-fill" style={{ width: fillW, x: fillX }} initial={false} animate={{ backgroundColor: current.color }} transition={{ duration: reduce ? 0 : 0.35 }}>
        <div className="reel-rows ink" ref={(el) => void (el && (layers.current[1] = el))}>
          {near.map((i) => (
            <Row key={i} i={i} pos={pos} h={h} text={items[mod(i, items.length)]!.text} />
          ))}
        </div>
      </motion.span>
    </motion.span>
  );
}

function Row({ i, pos, h, text, faded }: { i: number; pos: MotionValue<number>; h: number; text: string; faded?: boolean }) {
  const y = useTransform(pos, (p) => (i - p) * h);
  const opacity = useTransform(pos, (p) => (faded ? Math.max(0, 0.3 - 0.2 * Math.max(0, Math.abs(i - p) - 1)) : 1));
  return (
    <motion.span className="reel-row" data-text={text} style={{ x: "-50%", y, opacity }} />
  );
}
