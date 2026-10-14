// Routing r1d input to whatever is on screen. App.tsx gives the open app's handler first look at every
// event; an unconsumed double tap goes home.

import { createContext, useContext, useEffect, useRef } from "react";
import type { R1Event } from "./r1";

/** Returns true when the app consumed the event. */
export type InputHandler = (ev: R1Event) => boolean;
export const AppInputContext = createContext<(h: InputHandler | null) => void>(() => {});

/** Registers the open app's input handler (latest closure wins). */
export function useAppInput(handler: InputHandler) {
  const set = useContext(AppInputContext);
  const ref = useRef(handler);
  ref.current = handler;
  useEffect(() => {
    set((ev) => ref.current(ev));
    return () => set(null);
  }, [set]);
}
