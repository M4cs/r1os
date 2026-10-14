// Desktop simulator frame (r1d --sim --harness; tools/sim.sh).
//
// Renders the shell at the R1's CSS viewport (480×640 px at density 180 → 427×569 CSS px) and
// maps the keyboard and mouse wheel to r1d's harness, so simulated input goes through the same
// gesture engine as the real wheel and side button.

import { useEffect, useState, type ReactNode } from "react";
import { api } from "./r1";
import { useSimMic } from "./voice";

const MIC_KEY = "r1os.simMic";

export function SimFrame({ children }: { children: ReactNode }) {
  // Push-to-talk with the PC's mic is opt-in: otherwise every simulated press would ask for it.
  const [mic, setMic] = useState(() => {
    try {
      return localStorage.getItem(MIC_KEY) === "1";
    } catch {
      return false;
    }
  });
  useSimMic(mic);
  useEffect(() => {
    let spaceDown = false;
    // Typing in a field (New app's brief) is typing, not the wheel or the button.
    const typing = (e: KeyboardEvent) =>
      e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement || e.target instanceof HTMLSelectElement;
    const keydown = (e: KeyboardEvent) => {
      if (typing(e)) return;
      if (e.key === "ArrowUp" || e.key === "ArrowDown") {
        e.preventDefault();
        void api.simWheel(e.key === "ArrowUp" ? -1 : 1);
      } else if (e.code === "Space" && !spaceDown) {
        e.preventDefault();
        spaceDown = true;
        void api.simButton("down");
      }
    };
    const keyup = (e: KeyboardEvent) => {
      if (typing(e) && !spaceDown) return;
      if (e.code === "Space" && spaceDown) {
        e.preventDefault();
        spaceDown = false;
        void api.simButton("up");
      }
    };
    window.addEventListener("keydown", keydown);
    window.addEventListener("keyup", keyup);
    return () => {
      window.removeEventListener("keydown", keydown);
      window.removeEventListener("keyup", keyup);
    };
  }, []);

  return (
    <div className="sim">
      <div
        className="sim-device"
        onWheel={(e) => {
          if (Math.abs(e.deltaY) >= 1) void api.simWheel(Math.sign(e.deltaY));
        }}
      >
        {children}
      </div>
      <aside className="sim-help">
        <h1>r1os simulator</h1>
        <p>
          <kbd>↑</kbd> <kbd>↓</kbd> or mouse wheel: the scroll wheel
        </p>
        <p>
          <kbd>Space</kbd>: the side button (tap, double-tap, hold)
        </p>
        <p className="muted">Input goes through r1d's gesture engine, the same as on the R1.</p>
        <label className="sim-mic">
          <input
            type="checkbox"
            checked={mic}
            onChange={(e) => {
              setMic(e.target.checked);
              try {
                localStorage.setItem(MIC_KEY, e.target.checked ? "1" : "0");
              } catch {
                /* private window: just this session */
              }
            }}
          />
          Use my mic for push-to-talk (hold <kbd>Space</kbd> and talk)
        </label>
      </aside>
    </div>
  );
}
