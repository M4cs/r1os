// The site's look, as the companion follows the R1's (companion/src/theme.ts): dark or light, and an accent from the
// R1's own palette, kept for the next visit (index.html applies it before the first paint).

import { createContext, createElement, type ReactNode, useCallback, useContext, useEffect, useMemo, useState } from "react";
import { ACCENTS } from "@companion/theme";

export type Mode = "system" | "dark" | "light";
export interface Look {
  mode: Mode;
  accent: string;
}

const KEY = "r1os-look";
const DEFAULT: Look = { mode: "system", accent: ACCENTS.orange! };

function stored(): Look {
  try {
    return { ...DEFAULT, ...JSON.parse(localStorage.getItem(KEY) ?? "{}") };
  } catch {
    return DEFAULT;
  }
}

const prefersLight = () => typeof matchMedia !== "undefined" && matchMedia("(prefers-color-scheme: light)").matches;

interface LookContext {
  look: Look;
  /** What the mode came to: the system's, or the one picked. */
  dark: boolean;
  setLook: (patch: Partial<Look>) => void;
}

const Ctx = createContext<LookContext | null>(null);

export function LookProvider({ children }: { children: ReactNode }) {
  const [look, set] = useState<Look>(stored);
  const [systemLight, setSystemLight] = useState(prefersLight);
  useEffect(() => {
    const mq = matchMedia("(prefers-color-scheme: light)");
    const on = () => setSystemLight(mq.matches);
    mq.addEventListener("change", on);
    return () => mq.removeEventListener("change", on);
  }, []);
  const dark = look.mode === "system" ? !systemLight : look.mode === "dark";
  useEffect(() => {
    const root = document.documentElement;
    root.dataset.mode = dark ? "dark" : "light";
    root.style.setProperty("--accent", look.accent);
    document.querySelector('meta[name="theme-color"]')?.setAttribute("content", dark ? "#000000" : "#f3f2ef");
  }, [dark, look.accent]);
  const setLook = useCallback((patch: Partial<Look>) => {
    set((l) => {
      const next = { ...l, ...patch };
      try {
        localStorage.setItem(KEY, JSON.stringify(next));
      } catch {
        // No storage: the look lasts this visit.
      }
      return next;
    });
  }, []);
  const value = useMemo(() => ({ look, dark, setLook }), [look, dark, setLook]);
  return createElement(Ctx.Provider, { value }, children);
}

export function useLook(): LookContext {
  const c = useContext(Ctx);
  if (!c) throw new Error("useLook outside LookProvider");
  return c;
}

export { ACCENTS };
