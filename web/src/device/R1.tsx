// The R1, drawn (styles/device.css), and its screen in the R1's own 360×480 dp, scaled to fit as the companion's small
// R1 is (companion/src/r1screen/R1Screen.tsx). A screen turns the wheel and presses the button through `useDevice`, so
// the R1 on the page moves as the screen's story asks: the ribs roll a tick at a time, the button sinks while held.
// Docked, the body turns 90° clockwise onto its side, the wheel and button underneath, as it stands in dock mode
// (T-103); a `LandscapeScreen` lays its children out in 480×360 dp so they read upright on it.

import { animate, motion, type MotionValue, useMotionValue, useReducedMotion, useTransform } from "motion/react";
import { createContext, type CSSProperties, type ReactNode, useCallback, useContext, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import "../styles/device.css";

interface Device {
  /** Turns the wheel by `ticks` (negative is up). */
  roll: (ticks: number) => void;
  /** Holds the button down, or lets it go. */
  hold: (down: boolean) => void;
}

const Ctx = createContext<Device>({ roll: () => {}, hold: () => {} });
export const useDevice = () => useContext(Ctx);

/** How much the screen's dp are scaled to fit the page. */
const ScaleCtx = createContext(1);
export const useScale = () => useContext(ScaleCtx);

/** Setting it on its side: a little weight, a little settle. */
const SIDE = { type: "spring", stiffness: 140, damping: 19, mass: 1.1 } as const;

/** `turn`: the wheel's position from outside (the page turning it), on top of the screens' own turns. `docked`: on its
 *  side (`true` springs it there, `false` back upright), or how far over it is (0 upright, 1 on its side) for a turn
 *  the page drives, such as one linked to scrolling. */
export function R1({
  children,
  glow,
  turn = 0,
  docked,
  style,
  className,
  onWheelClick,
}: {
  children: ReactNode;
  glow?: string;
  turn?: number;
  docked?: boolean | MotionValue<number>;
  style?: CSSProperties;
  className?: string;
  onWheelClick?: () => void;
}) {
  // The wheel: a position in ticks, sprung so a turn settles into its detent.
  const ticks = useMotionValue(0);
  const ribs = useTransform(ticks, (t) => `0 ${t * 3.2}cqw`);
  const edge = useTransform(ticks, (t) => `0 ${t * 1.6}cqw`);
  const target = useRef(0);
  const [down, setDown] = useState(false);
  const reduce = useReducedMotion();
  const roll = useCallback(
    (n: number) => {
      target.current += n;
      if (reduce) ticks.jump(target.current);
      else animate(ticks, target.current, { type: "spring", stiffness: 520, damping: 26 });
    },
    [ticks, reduce],
  );
  const device = useMemo(() => ({ roll, hold: setDown }), [roll]);
  const turned = useRef(turn);
  useEffect(() => {
    if (turn !== turned.current) roll(turn - turned.current);
    turned.current = turn;
  }, [turn, roll]);

  // On its side: 0 upright, 1 turned 90° clockwise. The body turns; its shadow stays on the floor, and its light stays
  // where the room's is (device.css counter-turns the sheen by `--tilt`).
  const side = useMotionValue(typeof docked === "object" ? docked.get() : docked ? 1 : 0);
  useEffect(() => {
    if (typeof docked === "object") {
      side.set(docked.get());
      return docked.on("change", (v) => side.set(v));
    }
    const to = docked ? 1 : 0;
    if (reduce) {
      side.set(to);
      return;
    }
    const a = animate(side, to, SIDE);
    return () => a.stop();
  }, [docked, side, reduce]);
  const rotate = useTransform(side, (s) => s * 90);
  const tilt = useTransform(side, (s) => `${Math.round(s * 90)}deg`);

  return (
    <Ctx.Provider value={device}>
      <div className={`r1-device ${className ?? ""}`} style={{ ...style, ["--glow" as string]: glow }}>
        <div className="r1-shadow" />
        <motion.div className="r1-body" style={{ rotate, ["--tilt" as string]: tilt }}>
          <div className="r1-glass">{children}</div>
          <div className="r1-cam">
            <div className="r1-barrel">
              <div className="r1-lens" />
            </div>
          </div>
          <div className="r1-slot" onClick={onWheelClick} style={onWheelClick ? { cursor: "pointer" } : undefined} title={onWheelClick ? "turn the wheel" : undefined}>
            <div className="r1-roller">
              <motion.div className="r1-ribs" style={{ backgroundPosition: ribs }} />
            </div>
          </div>
          <div className="r1-edge">
            <motion.div className="r1-ribs" style={{ backgroundPosition: edge }} />
          </div>
          <motion.div
            className="r1-button"
            animate={{ scaleX: down ? 0.35 : 1, boxShadow: down ? `0 0 14px ${glow ?? "#ff8a3d"}` : "1px 0 2px rgba(0,0,0,.3)" }}
            transition={{ type: "spring", stiffness: 600, damping: 30 }}
          />
        </motion.div>
      </div>
    </Ctx.Provider>
  );
}

/** The screen's box, and the scale that fits the R1's 360 dp across it. */
function useFit() {
  const box = useRef<HTMLDivElement>(null);
  const [k, setK] = useState(0.8);
  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setK(el.clientWidth / 360));
    ro.observe(el);
    setK(el.clientWidth / 360);
    return () => ro.disconnect();
  }, []);
  return [box, k] as const;
}

/** The R1's screen: its children laid out in dp. */
export function Screen({ children, bg = "#000", style }: { children: ReactNode; bg?: string; style?: CSSProperties }) {
  const [box, k] = useFit();
  return (
    <div className="r1-screen" ref={box} style={{ background: bg }}>
      <div className="r1-dp" style={{ transform: `scale(${k})`, ...style }}>
        <ScaleCtx.Provider value={k}>{children}</ScaleCtx.Provider>
      </div>
    </div>
  );
}

/** The screen on its side (dock mode): its children laid out in 480×360 dp and turned a quarter left inside the
 *  upright glass, so they read upright on an R1 that's `docked`. The scale is the upright screen's: 360 dp across. */
export function LandscapeScreen({ children, bg = "#000", style }: { children: ReactNode; bg?: string; style?: CSSProperties }) {
  const [box, k] = useFit();
  return (
    <div className="r1-screen" ref={box} style={{ background: bg }}>
      <div className="r1-dp r1-dp-side" style={{ transform: `rotate(-90deg) scale(${k})`, ...style }}>
        <ScaleCtx.Provider value={k}>{children}</ScaleCtx.Provider>
      </div>
    </div>
  );
}
