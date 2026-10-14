// One mini-app on its own page (ADR-0017 decision 4): the native shell opens `app.html#<id>` in a
// WebView per app, under its own header. The page keeps no socket: the shell passes it the wheel,
// clicks and the app's change events through `window.r1Event`. Double tap and hold stay the shell's.

import { StrictMode, useCallback } from "react";
import { createRoot } from "react-dom/client";
import { AppView } from "./apps/AppView";
import { AppInputContext, type InputHandler } from "./input";
import { emit, native, type R1Event } from "./r1";
import "./styles.css";
import "./app.css";

const id = decodeURIComponent(location.hash.slice(1));

// The shell draws at a larger scale than Android's (ADR-0021: 213 dpi to the system's 180), and the
// WebView lays out at the system's. The shell passes the ratio so the app matches it, applied before
// the first render so nothing jumps.
const params = new URLSearchParams(location.search);
const zoom = Number(params.get("zoom"));
if (zoom > 0 && zoom !== 1) document.documentElement.style.zoom = String(zoom);

// The device's look (r1os ADR-0043): light mode, and its font (the R1's system fonts, by their names).
const FONTS: Record<string, string> = {
  roboto: "Roboto, sans-serif",
  flex: '"Roboto Flex", Roboto, sans-serif',
  source: '"Source Sans Pro", sans-serif',
  serif: '"Noto Serif", serif',
  mono: '"Droid Sans Mono", monospace',
  typewriter: '"Cutive Mono", monospace',
  casual: '"Coming Soon", casual, cursive',
  script: '"Dancing Script", cursive',
  smallcaps: '"Carrois Gothic SC", sans-serif',
};
if (params.get("mode") === "light") document.documentElement.dataset.mode = "light";
const font = FONTS[params.get("font") ?? ""];
if (font) document.documentElement.style.setProperty("--r1-font", font);
let handler: InputHandler | null = null;

// Back lit up in the shell's header (ADR-0014/0021): the app's focus ring steps aside, and comes
// back on its first control when Back goes out.
(window as unknown as { r1Focus: (on: boolean) => void }).r1Focus = (on) => {
  window.dispatchEvent(new CustomEvent("r1focus", { detail: on }));
};

// The shell takes the wheel back from a component that has it (ADR-0038): the app was hidden.
(window as unknown as { r1Release: () => void }).r1Release = () => {
  window.dispatchEvent(new CustomEvent("r1release"));
};

(window as unknown as { r1Event: (ev: R1Event) => void }).r1Event = (ev) => {
  emit(ev);
  // The button's edges come only while a component has the wheel (a game).
  if (ev.type === "wheel" || ev.type === "gesture" || ev.type === "button") handler?.(ev);
};

function AppPage() {
  const setHandler = useCallback((h: InputHandler | null) => {
    handler = h;
  }, []);
  return (
    <div className="screen">
      <div className="app">
        <AppInputContext.Provider value={setHandler}>{id ? <AppView id={id} /> : <div className="center muted">No app</div>}</AppInputContext.Provider>
      </div>
    </div>
  );
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <AppPage />
  </StrictMode>,
);
native?.ready();
