// Whether a part of the page is on screen in a visible tab. Anything that loops (a screen's story, a counter, the
// characters' CSS animations) runs only while it is, as on the R1: battery first, even on a web page.

import { type RefObject, useEffect, useState } from "react";

/** True while `ref` is at least `amount` in view and the tab is visible. */
export function useSeen(ref: RefObject<Element | null>, amount = 0.15): boolean {
  const [inView, setInView] = useState(false);
  const [visible, setVisible] = useState(() => typeof document === "undefined" || document.visibilityState === "visible");
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    // Some browsers count any overlap as intersecting, so the ratio is checked too.
    const io = new IntersectionObserver(([e]) => setInView(!!e && e.isIntersecting && e.intersectionRatio >= amount), { threshold: amount > 0 ? [0, amount] : 0 });
    io.observe(el);
    const vis = () => setVisible(document.visibilityState === "visible");
    document.addEventListener("visibilitychange", vis);
    return () => {
      io.disconnect();
      document.removeEventListener("visibilitychange", vis);
    };
  }, [ref, amount]);
  return inView && visible;
}

/** Spread on a section: its CSS animations pause while it's out of sight (base.css `[data-rest]`). */
export const rest = (seen: boolean) => ({ "data-rest": seen ? undefined : "" });
