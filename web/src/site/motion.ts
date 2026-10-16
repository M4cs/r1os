// The site's motion: the R1's springs (retargetable, a little overshoot, settling fast; companion/src/motion.ts), and a
// livelier one for the hero, whose reel lands like the wheel's detents. The hero is the page's one big moment; past it,
// parts rise a short way, once, as they scroll in. Everything here moves only transform and opacity, and with reduced
// motion the parts are simply there (parts.tsx, Root.tsx).

import type { Variants } from "motion/react";

export const spring = { type: "spring", stiffness: 420, damping: 34, mass: 0.9 } as const;
export const soft = { type: "spring", stiffness: 260, damping: 28 } as const;
export const bouncy = { type: "spring", stiffness: 190, damping: 20, mass: 1 } as const;

/** A section's parts arriving as they scroll in. */
export const rise = {
  initial: { opacity: 0, y: 14 },
  whileInView: { opacity: 1, y: 0 },
  viewport: { once: true, margin: "-80px" },
  transition: soft,
} as const;

/** On a parent: its children's `hidden` → `show` variants play once, as it scrolls in. */
export const reveal = {
  initial: "hidden",
  whileInView: "show",
  viewport: { once: true, margin: "-80px" },
} as const;

/** A part that rises in `delay` seconds after its parent is shown. */
export const after = (delay: number, y = 10): Variants => ({
  hidden: { opacity: 0, y },
  show: { opacity: 1, y: 0, transition: { ...soft, delay } },
});

/** A small mark that settles in `delay` seconds after its parent is shown (`custom`: its place, to stagger them). */
export const popIn = (delay: number): Variants => ({
  hidden: { opacity: 0, scale: 0.85 },
  show: (i: number = 0) => ({ opacity: 1, scale: 1, transition: { ...soft, delay: delay + i * 0.05 } }),
});

/** A page arriving when the route changes (Root.tsx): a soft rise, no exit, so hash links and scroll restoring keep
 *  working. */
export const arrive = { initial: { opacity: 0, y: 10 }, animate: { opacity: 1, y: 0 }, transition: soft } as const;
